/**
 * Reference web worker for decoding viewcam frames off the main thread.
 *
 * Start it as a module worker from the page:
 *
 *   const worker = new Worker(
 *     new URL('viewcam-codec/decode-worker.js', import.meta.url),
 *     { type: 'module' },
 *   );
 *
 * Request (page → worker), with the encoded buffers in the transfer list:
 *
 *   worker.postMessage({
 *     id,                        // anything; echoed back
 *     planes: [{ encoded, elementSize, decodedLength }, ...],
 *     meta,                      // anything; echoed back (width, dtype, seq, ...)
 *   }, planes.map((p) => p.encoded));
 *
 * Reply (worker → page), with the decoded buffers transferred, not copied:
 *
 *   { type: 'decoded', id, planes: [ArrayBuffer, ...], meta }
 *   { type: 'error',   id, message, meta }
 *
 * Requests are decoded in arrival order. Keeping the queue short (e.g. only
 * posting the newest frame while one is in flight) is the page's job; this
 * worker does not drop anything on its own.
 */

import init, { Decoder } from './viewcam_codec.js';

// Messages can arrive before the WASM module has loaded; each waits on this.
const ready = init().then(() => new Decoder());

self.onmessage = async (event) => {
  const { id, planes, meta } = event.data;
  const decoder = await ready;
  try {
    const decoded = planes.map(({ encoded, elementSize, decodedLength }) =>
      decoder.decode(new Uint8Array(encoded), elementSize, decodedLength).buffer,
    );
    self.postMessage({ type: 'decoded', id, planes: decoded, meta }, decoded);
  } catch (error) {
    self.postMessage({ type: 'error', id, message: error.message ?? String(error), meta });
  }
};
