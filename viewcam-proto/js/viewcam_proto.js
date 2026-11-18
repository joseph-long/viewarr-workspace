/**
 * Reference client for the viewcam streaming protocol, for browsers.
 *
 * See ../PROTOCOL.md for the specification. `decodeFrame` and `encodeAck` are
 * the wire format, checked against ../tests/vectors/ like the Rust crate and
 * the Python reference. `ViewcamFeed` is a client for one feed: it opens the
 * WebSocket, acknowledges every frame as it arrives, decodes off the main
 * thread and hands over only frames newer than the last one shown.
 *
 *   import { ViewcamFeed, workerDecoder } from './viewcam_proto.js';
 *
 *   const decode = workerDecoder(new Worker(
 *     new URL('viewcam-codec/decode-worker.js', import.meta.url), { type: 'module' }));
 *   const feed = new ViewcamFeed('wss://host/ws/viewcam/camwfs', {
 *     decode,
 *     onFrame: ({ seq, width, height, dtype, pixels }) => { ... },  // pixels: ArrayBuffer
 *     onEmpty: () => { ... },          // the feed has no image
 *     onStatus: (status) => { ... },   // 'connecting' | 'open' | 'closed'
 *   });
 *   feed.setWindow(0);   // pause while hidden; feed.setWindow(6) to resume
 *   feed.close();
 *
 * Standard JavaScript only (a WebSocket, a DataView), so it runs in browsers
 * and in Node.
 */

export const MAGIC = 'VCAM';
export const VERSION = 1;

export const KIND_FRAME = 1;
export const KIND_ACK = 0x81;

export const CODEC_RAW = 0;
export const CODEC_LZ4 = 1;
export const CODEC_SHUFFLE_LZ4 = 2;

/** dtype code -> [viewarr dtype name, bytes per element] */
export const DTYPES = {
  1: ['u8', 1],
  2: ['i8', 1],
  3: ['u16', 2],
  4: ['i16', 2],
  5: ['u32', 4],
  6: ['i32', 4],
  7: ['u64', 8],
  8: ['i64', 8],
  9: ['f32', 4],
  10: ['f64', 8],
};

export const FRAME_HEADER_LEN = 32;
export const ACK_LEN = 32;
/** The most pixels a frame may claim before it is rejected unread. */
export const DEFAULT_MAX_PIXELS = 2 ** 28;

/** Close codes PROTOCOL.md asks for. */
export const CLOSE_PROTOCOL_ERROR = 1002;
export const CLOSE_UNSUPPORTED_DATA = 1003;

export class ProtocolError extends Error {
  constructor(message) {
    super(message);
    this.name = 'ProtocolError';
  }
}

/** The largest payload `codec` can produce for a `decoded`-byte image. */
export function maxEncodedLen(codec, decoded) {
  if (codec === CODEC_RAW) return decoded;
  return decoded + Math.floor(decoded / 255) + 16; // LZ4's worst case
}

function checkPrefix(view, kind) {
  for (let i = 0; i < 4; i++) {
    if (view.getUint8(i) !== MAGIC.charCodeAt(i)) {
      throw new ProtocolError('not a viewcam message (bad magic)');
    }
  }
  const version = view.getUint8(4);
  if (version !== VERSION) throw new ProtocolError(`unsupported protocol version ${version}`);
  const got = view.getUint8(5);
  if (got !== kind) {
    throw new ProtocolError(`expected kind 0x${kind.toString(16)}, got 0x${got.toString(16)}`);
  }
}

/**
 * Check a FRAME message (an ArrayBuffer) and return its header and payload.
 *
 * `seq` is a BigInt, since it is a u64. `payload` is a Uint8Array view of
 * the message, still encoded with `codec`. An empty frame ("no image on
 * this feed") has `width * height === 0`. Throws ProtocolError for anything
 * inconsistent, before the payload is looked at, so a corrupt header can't
 * ask for a huge allocation.
 */
export function decodeFrame(message, maxPixels = DEFAULT_MAX_PIXELS) {
  if (message.byteLength < FRAME_HEADER_LEN) {
    throw new ProtocolError(`frame message is ${message.byteLength} bytes, shorter than its header`);
  }
  const view = new DataView(message);
  checkPrefix(view, KIND_FRAME);
  const seq = view.getBigUint64(8, true);
  const width = view.getUint32(16, true);
  const height = view.getUint32(20, true);
  const dtypeCode = view.getUint8(24);
  const codec = view.getUint8(25);
  const encodedLen = view.getUint32(28, true);
  if (!(dtypeCode in DTYPES)) throw new ProtocolError(`unknown dtype code ${dtypeCode}`);
  if (codec !== CODEC_RAW && codec !== CODEC_LZ4 && codec !== CODEC_SHUFFLE_LZ4) {
    throw new ProtocolError(`unknown codec code ${codec}`);
  }
  const pixels = width * height;
  if (pixels > maxPixels) {
    throw new ProtocolError(`${width}x${height} frame is over the limit of ${maxPixels} pixels`);
  }
  const [dtype, elementSize] = DTYPES[dtypeCode];
  const decodedLen = pixels * elementSize;
  let ok;
  if (codec === CODEC_RAW) {
    ok = encodedLen === decodedLen;
  } else if (decodedLen === 0) {
    ok = encodedLen === 0;
  } else {
    ok = encodedLen > 0 && encodedLen <= maxEncodedLen(codec, decodedLen);
  }
  if (!ok) {
    throw new ProtocolError(
      `codec ${codec} payload of ${encodedLen} bytes cannot decode to ${decodedLen} bytes`);
  }
  if (message.byteLength !== FRAME_HEADER_LEN + encodedLen) {
    throw new ProtocolError(
      `frame message is ${message.byteLength} bytes, its header says ${FRAME_HEADER_LEN + encodedLen}`);
  }
  return {
    seq, width, height, dtype, dtypeCode, elementSize, codec, decodedLen,
    payload: new Uint8Array(message, FRAME_HEADER_LEN, encodedLen),
  };
}

/**
 * An ACK message, as an ArrayBuffer.
 *
 * `ackedFrames` and `ackedBytes` count the FRAME messages received on this
 * connection and their total length; `windowFrames` (0 pauses) and
 * `windowBytes` (0 means no limit) are how much more may be in flight.
 */
export function encodeAck({ ackedFrames, ackedBytes, windowFrames, windowBytes = 0 }) {
  const message = new ArrayBuffer(ACK_LEN);
  const view = new DataView(message);
  for (let i = 0; i < 4; i++) view.setUint8(i, MAGIC.charCodeAt(i));
  view.setUint8(4, VERSION);
  view.setUint8(5, KIND_ACK);
  view.setBigUint64(8, BigInt(ackedFrames), true);
  view.setBigUint64(16, BigInt(ackedBytes), true);
  view.setUint32(24, windowFrames, true);
  view.setUint32(28, windowBytes, true);
  return message;
}

/**
 * A `decode` function for ViewcamFeed that uses viewcam-codec's
 * decode-worker.js, so LZ4 decoding stays off the main thread.
 */
export function workerDecoder(worker) {
  let nextId = 0;
  const waiting = new Map();
  worker.addEventListener('message', (event) => {
    const { type, id, planes, message } = event.data;
    const pending = waiting.get(id);
    if (!pending) return;
    waiting.delete(id);
    if (type === 'decoded') pending.resolve(planes[0]);
    else pending.reject(new Error(message));
  });
  return (encoded, elementSize, decodedLength) => new Promise((resolve, reject) => {
    const id = nextId++;
    waiting.set(id, { resolve, reject });
    worker.postMessage({ id, planes: [{ encoded, elementSize, decodedLength }] }, [encoded]);
  });
}

/**
 * A client for one feed: a WebSocket that reconnects, acknowledges each
 * frame on arrival, and keeps the decoding queue to one frame in progress
 * plus the newest waiting, so a slow decoder skips frames instead of
 * falling behind.
 *
 * Options:
 *  - `decode(encoded: ArrayBuffer, elementSize, decodedLength) -> Promise<ArrayBuffer>`
 *    for the LZ4 codecs (shuffled when elementSize > 1); `raw` frames need none.
 *  - `onFrame({ seq, width, height, dtype, pixels })`, pixels an ArrayBuffer
 *    of the decoded image, row-major from the bottom row.
 *  - `onEmpty({ seq })` when the feed has no image.
 *  - `onStatus(status)`: 'connecting', 'open' or 'closed'.
 *  - `onReceive(bytes)` for every message received, decoded or skipped,
 *    with its length: what the feed costs in bandwidth.
 *  - `onError(error)`, for frames that fail to decode.
 *  - `windowFrames` (default 6) and `windowBytes` (default 0, no limit);
 *    see PROTOCOL.md, "Choosing a window".
 *  - `reconnectDelayMs` (default 1000), doubling to `maxReconnectDelayMs`
 *    (default 10000) while connecting keeps failing.
 *  - `WebSocket`, a constructor to use instead of the global one.
 */
export class ViewcamFeed {
  constructor(url, options = {}) {
    this.url = url;
    this.decode = options.decode ?? null;
    this.onFrame = options.onFrame ?? (() => {});
    this.onEmpty = options.onEmpty ?? (() => {});
    this.onStatus = options.onStatus ?? (() => {});
    this.onReceive = options.onReceive ?? (() => {});
    this.onError = options.onError ?? ((error) => console.error(`viewcam feed ${url}:`, error));
    this.windowFrames = options.windowFrames ?? 6;
    this.windowBytes = options.windowBytes ?? 0;
    this.reconnectDelayMs = options.reconnectDelayMs ?? 1000;
    this.maxReconnectDelayMs = options.maxReconnectDelayMs ?? 10000;
    this.WebSocket = options.WebSocket ?? globalThis.WebSocket;
    this.maxPixels = options.maxPixels ?? DEFAULT_MAX_PIXELS;

    this.ws = null;
    this.closed = false;
    this.retryDelayMs = this.reconnectDelayMs;
    this.retryTimer = null;
    // seq of the newest frame handed over. A reconnection is a new
    // connection, whose seq may start anywhere, so this resets with it.
    this.shownSeq = -1n;
    this.decoding = false;
    this.waiting = null;
    this.connect();
  }

  /** Resize the window, or pause the feed with `windowFrames = 0`. */
  setWindow(windowFrames, windowBytes = this.windowBytes) {
    this.windowFrames = windowFrames;
    this.windowBytes = windowBytes;
    this.sendAck();
  }

  close() {
    this.closed = true;
    clearTimeout(this.retryTimer);
    if (this.ws) this.ws.close();
    this.ws = null;
  }

  connect() {
    if (this.closed) return;
    this.onStatus('connecting');
    const ws = new this.WebSocket(this.url);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;
    this.ackedFrames = 0;
    this.ackedBytes = 0;
    this.shownSeq = -1n;
    this.waiting = null;
    ws.onopen = () => {
      if (ws !== this.ws) return;
      this.retryDelayMs = this.reconnectDelayMs;
      this.sendAck();
      this.onStatus('open');
    };
    ws.onmessage = (event) => {
      if (ws !== this.ws) return;
      this.receive(ws, event.data);
    };
    ws.onclose = () => {
      if (ws !== this.ws) return;
      this.ws = null;
      this.onStatus('closed');
      if (this.closed) return;
      this.retryTimer = setTimeout(() => this.connect(), this.retryDelayMs);
      this.retryDelayMs = Math.min(2 * this.retryDelayMs, this.maxReconnectDelayMs);
    };
  }

  sendAck() {
    const ws = this.ws;
    if (!ws || ws.readyState !== this.WebSocket.OPEN) return;
    ws.send(encodeAck({
      ackedFrames: this.ackedFrames,
      ackedBytes: this.ackedBytes,
      windowFrames: this.windowFrames,
      windowBytes: this.windowBytes,
    }));
  }

  receive(ws, message) {
    this.onReceive(typeof message === 'string' ? message.length : message.byteLength);
    if (typeof message === 'string') {
      ws.close(CLOSE_UNSUPPORTED_DATA);
      return;
    }
    let frame;
    try {
      frame = decodeFrame(message, this.maxPixels);
    } catch (error) {
      this.onError(error);
      ws.close(CLOSE_PROTOCOL_ERROR);
      return;
    }
    // Acknowledge on arrival, before decoding: decoding time shouldn't count
    // against the window.
    this.ackedFrames += 1;
    this.ackedBytes += message.byteLength;
    this.sendAck();
    if (this.decoding) {
      this.waiting = frame; // newest wins
    } else {
      this.process(frame);
    }
  }

  async process(frame) {
    this.decoding = true;
    try {
      while (frame) {
        await this.show(frame);
        frame = this.waiting;
        this.waiting = null;
      }
    } finally {
      this.decoding = false;
    }
  }

  async show(frame) {
    const { seq, width, height, dtype, codec, elementSize, decodedLen } = frame;
    if (seq <= this.shownSeq) return;
    if (width * height === 0) {
      this.shownSeq = seq;
      this.onEmpty({ seq });
      return;
    }
    // A copy of just the payload: it may be transferred to a worker, and
    // the image handed on should not hold the whole message.
    const encoded = frame.payload.slice().buffer;
    let pixels;
    try {
      if (codec === CODEC_RAW) {
        pixels = encoded;
      } else {
        if (!this.decode) throw new Error(`no decoder for codec ${codec}`);
        pixels = await this.decode(encoded, codec === CODEC_SHUFFLE_LZ4 ? elementSize : 1, decodedLen);
      }
    } catch (error) {
      this.onError(error);
      return;
    }
    // Decoding is asynchronous, so check again that nothing newer got here first.
    if (seq <= this.shownSeq) return;
    this.shownSeq = seq;
    this.onFrame({ seq, width, height, dtype, pixels });
  }
}
