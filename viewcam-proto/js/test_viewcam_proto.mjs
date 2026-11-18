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
  assert.deepEqual([raw.width, raw.height, raw.dtype, raw.codec], [3, 2, 'u16', vp.CODEC_RAW]);
  assert.deepEqual(raw.payload, ramp(12));

  const lz4 = vp.decodeFrame(vector('frame_f32_lz4'));
  assert.deepEqual([lz4.seq, lz4.dtype, lz4.codec, lz4.decodedLen], [7n, 'f32', vp.CODEC_LZ4, 16]);
  assert.deepEqual(lz4.payload, ramp(9));

  const shuffled = vp.decodeFrame(vector('frame_u16_shuffle_lz4'));
  assert.deepEqual([shuffled.seq, shuffled.codec, shuffled.elementSize], [8n, vp.CODEC_SHUFFLE_LZ4, 2]);

  const empty = vp.decodeFrame(vector('frame_empty'));
  assert.deepEqual([empty.seq, empty.width * empty.height, empty.dtype, empty.payload.length], [9n, 0, 'f64', 0]);
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
    'unknown dtype': corrupt('frame_u16_raw', 24, 'setUint8', 11),
    'unknown codec': corrupt('frame_u16_raw', 25, 'setUint8', 3),
    'raw of the wrong length': corrupt('frame_u16_raw', 16, 'setUint32', 4),
    'LZ4 over its bound': corrupt('frame_u16_shuffle_lz4', 16, 'setUint32', 0),
    'too many pixels': corrupt('frame_u16_raw', 20, 'setUint32', 2 ** 31),
    'a header longer than its message': corrupt('frame_u16_raw', 28, 'setUint32', 13),
    'truncated': vector('frame_u16_raw').slice(0, 40),
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
}

function frameMessage(seq, value) {
  const message = new ArrayBuffer(vp.FRAME_HEADER_LEN + 2);
  new Uint8Array(message).set(new Uint8Array(vector('frame_u16_raw')).subarray(0, vp.FRAME_HEADER_LEN));
  const view = new DataView(message);
  view.setBigUint64(8, BigInt(seq), true);
  view.setUint32(16, 1, true);
  view.setUint32(20, 1, true);
  view.setUint32(28, 2, true);
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
  const decodes = [];
  const received = [];
  const feed = new vp.ViewcamFeed('ws://test/feed', {
    WebSocket: FakeWebSocket,
    windowFrames: 4,
    onReceive: (bytes) => received.push(bytes),
    decode: (encoded) => {
      const d = deferred();
      decodes.push(d);
      return d.promise.then(() => encoded);
    },
    onFrame: ({ seq, pixels }) => shown.push([seq, new DataView(pixels).getUint16(0, true)]),
  });
  const ws = FakeWebSocket.last;
  ws.open();
  // A raw frame needs no decoder; make these LZ4 by flipping the codec.
  const lz4 = (seq, value) => {
    const message = frameMessage(seq, value);
    new DataView(message).setUint8(25, vp.CODEC_LZ4);
    return message;
  };
  ws.deliver(lz4(1, 10));
  ws.deliver(lz4(2, 20));
  ws.deliver(lz4(3, 30));
  // Every frame was acknowledged as it arrived, decoded or not.
  assert.deepEqual(ws.acks(), [[0, 0, 4], [1, 34, 4], [2, 68, 4], [3, 102, 4]]);
  // Counted as received whether they will be shown or not.
  assert.deepEqual(received, [34, 34, 34]);
  assert.equal(decodes.length, 1);
  decodes[0].resolve();
  await new Promise((r) => setTimeout(r));
  // Frame 2 was replaced while waiting, so 3 is decoded next.
  assert.equal(decodes.length, 2);
  decodes[1].resolve();
  await new Promise((r) => setTimeout(r));
  assert.deepEqual(shown, [[1n, 10], [3n, 30]]);

  feed.setWindow(0);
  assert.deepEqual(ws.acks().at(-1), [3, 102, 0]);
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
