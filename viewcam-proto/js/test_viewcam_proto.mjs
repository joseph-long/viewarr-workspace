// Checks the JavaScript client against the shared test vectors, and the
// feed's acknowledging and newest-wins decoding against a fake WebSocket.
//
//   node --test js/test_viewcam_proto.mjs

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { test } from 'node:test';

import * as vp from './viewcam_proto.js';

const vector = (name) => {
  const bytes = readFileSync(fileURLToPath(new URL(`../tests/vectors/${name}.bin`, import.meta.url)));
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
};
const ramp = (n) => Uint8Array.from({ length: n }, (_, i) => (i * 37 + 11) & 0xff);

test('frames decode as the vectors were written', () => {
  const raw = vp.decodeFrame(vector('frame_u16_raw'));
  assert.equal(raw.seq, 0x0102030405060708n);
  assert.equal(raw.acqTimeNs, 1795208400123456789n);
  assert.deepEqual([raw.width, raw.height, raw.dtype, raw.codec], [3, 2, 'u16', vp.CODEC_RAW]);
  assert.deepEqual(raw.payload, ramp(12));

  const zstd = vp.decodeFrame(vector('frame_f32_zstd'));
  assert.deepEqual(
    [zstd.seq, zstd.dtype, zstd.codec, zstd.elementSize, zstd.decodedLen],
    [7n, 'f32', vp.CODEC_BITSHUFFLE_ZSTD, 4, 16]);
  assert.equal(zstd.acqTimeNs, 0n);
  assert.deepEqual(zstd.payload, ramp(9));

  const empty = vp.decodeFrame(vector('frame_empty'));
  assert.deepEqual([empty.seq, empty.width * empty.height, empty.dtype, empty.payload.length], [9n, 0, 'f64', 0]);
  assert.equal(empty.quantization, null);

  const quantized = vp.decodeFrame(vector('frame_u8_quantized_zstd'));
  assert.deepEqual([quantized.seq, quantized.dtype, quantized.codec], [12n, 'u8', vp.CODEC_BITSHUFFLE_ZSTD]);
  assert.deepEqual(quantized.quantization, { codeOffset: -3, bias: 432, readNoise: 14.25, noiseSlope: 8.5, step: 0.25 });
  assert.deepEqual(quantized.payload, ramp(4));
});

test('acks encode as the vector was written', () => {
  const ack = vp.encodeAck({
    ackedFrames: 0x1122334455667788n,
    ackedBytes: 0x0102030405060708n,
    windowFrames: 6,
    windowBytes: 0x00400000,
  });
  assert.deepEqual(new Uint8Array(ack), new Uint8Array(vector('ack')));
});

function corrupt(name, offset, setter, value) {
  const message = vector(name);
  new DataView(message)[setter](offset, value, true);
  return message;
}

test('inconsistent frames are rejected', () => {
  const bad = {
    'bad magic': corrupt('frame_u16_raw', 0, 'setUint8', 0x58),
    'unknown version': corrupt('frame_u16_raw', 4, 'setUint8', 2),
    'an ack': vector('ack'),
    'unknown dtype': corrupt('frame_u16_raw', 32, 'setUint8', 11),
    'unknown codec': corrupt('frame_u16_raw', 33, 'setUint8', 2),
    'unknown quantization': corrupt('frame_u16_raw', 34, 'setUint8', 2),
    'parameters on an exact frame': corrupt('frame_u16_raw', 56, 'setUint8', 1),
    'quantized f32 codes': corrupt('frame_u8_quantized_zstd', 32, 'setUint8', 9),
    'no read noise': corrupt('frame_u8_quantized_zstd', 56, 'setFloat64', 0),
    'raw of the wrong length': corrupt('frame_u16_raw', 24, 'setUint32', 4),
    'zstd over its bound': corrupt('frame_f32_zstd', 24, 'setUint32', 0),
    'too many pixels': corrupt('frame_u16_raw', 28, 'setUint32', 2 ** 31),
    'a header longer than its message': corrupt('frame_u16_raw', 36, 'setUint32', 13),
    'truncated': vector('frame_u16_raw').slice(0, 88),
  };
  for (const [what, message] of Object.entries(bad)) {
    assert.throws(() => vp.decodeFrame(message), vp.ProtocolError, what);
  }
});

class FakeWebSocket {
  static OPEN = 1;
  static last = null;
  constructor(url) {
    this.url = url;
    this.readyState = 0;
    this.sent = [];
    FakeWebSocket.last = this;
  }
  send(message) { this.sent.push(message); }
  close(code) { this.closedWith = code; }
  open() { this.readyState = 1; this.onopen(); }
  deliver(message) { this.onmessage({ data: message }); }
  acks() {
    return this.sent.map((m) => {
      const v = new DataView(m);
      return [Number(v.getBigUint64(8, true)), Number(v.getBigUint64(16, true)), v.getUint32(24, true)];
    });
  }
  windowBytes() {
    return this.sent.map((m) => new DataView(m).getUint32(28, true));
  }
}

function frameMessage(seq, value) {
  const message = new ArrayBuffer(vp.FRAME_HEADER_LEN + 2);
  new Uint8Array(message).set(new Uint8Array(vector('frame_u16_raw')).subarray(0, vp.FRAME_HEADER_LEN));
  const view = new DataView(message);
  view.setBigUint64(8, BigInt(seq), true);
  view.setUint32(24, 1, true);
  view.setUint32(28, 1, true);
  view.setUint32(36, 2, true);
  view.setUint16(vp.FRAME_HEADER_LEN, value, true);
  return message;
}

function deferred() {
  let resolve;
  const promise = new Promise((r) => { resolve = r; });
  return { promise, resolve };
}

test('the feed acks on arrival, and shows only the newest of what arrived while decoding', async () => {
  const shown = [];
  const acquired = [];
  const decodes = [];
  const received = [];
  const feed = new vp.ViewcamFeed('ws://test/feed', {
    WebSocket: FakeWebSocket,
    windowFrames: 4,
    onReceive: (bytes) => received.push(bytes),
    decode: ({ encoded }) => {
      const d = deferred();
      decodes.push(d);
      return d.promise.then(() => encoded);
    },
    onFrame: ({ seq, pixels, acquiredMs }) => {
      shown.push([seq, new DataView(pixels).getUint16(0, true)]);
      acquired.push(acquiredMs);
    },
  });
  const ws = FakeWebSocket.last;
  ws.open();
  // A raw frame needs no decoder; make these zstd by flipping the codec.
  const zstd = (seq, value) => {
    const message = frameMessage(seq, value);
    new DataView(message).setUint8(33, vp.CODEC_BITSHUFFLE_ZSTD);
    return message;
  };
  ws.deliver(zstd(1, 10));
  ws.deliver(zstd(2, 20));
  ws.deliver(zstd(3, 30));
  // Every frame was acknowledged as it arrived, decoded or not.
  assert.deepEqual(ws.acks(), [[0, 0, 4], [1, 82, 4], [2, 164, 4], [3, 246, 4]]);
  // Counted as received whether they will be shown or not.
  assert.deepEqual(received, [82, 82, 82]);
  assert.equal(decodes.length, 1);
  decodes[0].resolve();
  await new Promise((r) => setTimeout(r));
  // Frame 2 was replaced while waiting, so 3 is decoded next.
  assert.equal(decodes.length, 2);
  decodes[1].resolve();
  await new Promise((r) => setTimeout(r));
  assert.deepEqual(shown, [[1n, 10], [3n, 30]]);
  // Copied from frame_u16_raw's header: 2026-11-20T21:00:00.123456789Z, in ms.
  assert.ok(Math.abs(acquired[0] - 1795208400123.4568) < 1e-3);

  feed.setWindow(0);
  assert.deepEqual(ws.acks().at(-1), [3, 246, 0]);
  feed.close();
});

test('a capped feed paces its acks, and acks at once when the cap is lifted', async () => {
  const feed = new vp.ViewcamFeed('ws://test/feed', {
    WebSocket: FakeWebSocket,
    windowFrames: 1,
    maxFps: 20,
  });
  const ws = FakeWebSocket.last;
  ws.open();
  ws.deliver(frameMessage(1, 10));
  // The first frame is acknowledged straight away: nothing was acked before it.
  assert.deepEqual(ws.acks(), [[0, 0, 1], [1, 82, 1]]);
  ws.deliver(frameMessage(2, 20));
  assert.equal(ws.acks().length, 2);
  await new Promise((r) => setTimeout(r, 80));
  // One ack, 50 ms after the last.
  assert.deepEqual(ws.acks(), [[0, 0, 1], [1, 82, 1], [2, 164, 1]]);
  ws.deliver(frameMessage(3, 30));
  ws.deliver(frameMessage(4, 40));
  assert.equal(ws.acks().length, 3);
  feed.setMaxFps(0);
  assert.deepEqual(ws.acks().at(-1), [4, 328, 1]);
  ws.deliver(frameMessage(5, 50));
  assert.deepEqual(ws.acks().at(-1), [5, 410, 1]);
  feed.close();
});

test('the delivery rate is over the latest arrivals, and sizes the byte window', () => {
  const rate = new vp.DeliveryRate();
  assert.equal(rate.bytesPerSec(), null);
  assert.equal(rate.windowBytes(500), 1);
  rate.note(10000, 1000);
  for (let i = 1; i <= 4; i++) rate.note(10000 + 100 * i, 1000);
  assert.equal(Math.round(rate.bytesPerSec()), 10000);
  assert.equal(rate.windowBytes(500), 5000);
  // Frames too large for the target alone still get two in flight.
  assert.equal(rate.windowBytes(100), 2500);
  // A pause longer than the span leaves one arrival: no rate again.
  rate.note(20000, 1000);
  assert.equal(rate.bytesPerSec(), null);
});

test('the feed asks for a byte window sized from what arrives', () => {
  let now = 0;
  const feed = new vp.ViewcamFeed('ws://test/feed', { WebSocket: FakeWebSocket, now: () => now });
  const ws = FakeWebSocket.last;
  ws.open();
  // A frame at a time until there is a rate to go by.
  assert.deepEqual(ws.windowBytes(), [1]);
  ws.deliver(frameMessage(1, 10));
  assert.deepEqual(ws.windowBytes(), [1, 1]);
  // 82 bytes every 100 ms is 820 B/s, so 410 bytes for half a second.
  now = 100;
  ws.deliver(frameMessage(2, 20));
  assert.equal(ws.windowBytes().at(-1), 410);

  // With the target off, the fixed byte window goes out.
  feed.targetInFlightMs = 0;
  feed.setWindow(6, 0);
  assert.equal(ws.windowBytes().at(-1), 0);
  feed.close();
});

test('the feed reports empty frames, and closes on a bad one', () => {
  const empties = [];
  const errors = [];
  const feed = new vp.ViewcamFeed('ws://test/feed', {
    WebSocket: FakeWebSocket,
    onEmpty: ({ seq }) => empties.push(seq),
    onError: (e) => errors.push(e),
  });
  const ws = FakeWebSocket.last;
  ws.open();
  ws.deliver(vector('frame_empty'));
  assert.deepEqual(empties, [9n]);
  ws.deliver(vector('ack'));
  assert.equal(ws.closedWith, vp.CLOSE_PROTOCOL_ERROR);
  assert.equal(errors.length, 1);
  feed.close();
});

test('a quantized frame is decoded to f32 values, with its quantization', async () => {
  const requests = [];
  const frames = [];
  const feed = new vp.ViewcamFeed('ws://test/feed', {
    WebSocket: FakeWebSocket,
    decode: (request) => {
      requests.push(request);
      return Promise.resolve(new Float32Array([1.5]).buffer);
    },
    onFrame: (frame) => frames.push(frame),
  });
  const ws = FakeWebSocket.last;
  ws.open();
  // One u8 code, quantized, sent raw: it still needs the decoder.
  const message = new ArrayBuffer(vp.FRAME_HEADER_LEN + 1);
  new Uint8Array(message).set(new Uint8Array(vector('frame_u8_quantized_zstd')).subarray(0, vp.FRAME_HEADER_LEN));
  const view = new DataView(message);
  view.setUint32(24, 1, true);
  view.setUint32(28, 1, true);
  view.setUint8(33, vp.CODEC_RAW);
  view.setUint32(36, 1, true);
  ws.deliver(message);
  await new Promise((r) => setTimeout(r));
  assert.equal(requests.length, 1);
  assert.deepEqual(
    [requests[0].codec, requests[0].elementSize, requests[0].decodedLength, requests[0].quantized.readNoise],
    ['raw', 1, 1, 14.25]);
  assert.equal(frames[0].dtype, 'f32');
  assert.equal(frames[0].quantization.step, 0.25);
  // This vector carries no acquisition time.
  assert.equal(frames[0].acquiredMs, null);
  feed.close();
});
