// Smoke test of the built WASM package under Node: decodes the Python-encoded
// vectors and times a camera-sized frame. Run after `npm run build`:
//
//   npm run test:wasm

import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

import init, { Decoder, encode } from '../pkg/viewcam_codec.js';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));

// `--target web` would fetch the .wasm; Node has no fetch for file: URLs.
await init({ module_or_path: readFileSync(here('../pkg/viewcam_codec_bg.wasm')) });
const decoder = new Decoder();

const vectors = here('../tests/vectors/');
let checked = 0;
for (const name of readdirSync(vectors)) {
  const match = name.match(/^(py_.*)\.e(\d+)\.lz4$/);
  if (!match) continue;
  const raw = readFileSync(vectors + `${match[1]}.raw`);
  const out = decoder.decode(readFileSync(vectors + name), Number(match[2]), raw.length);
  assert.deepEqual(Buffer.from(out), raw, name);
  checked++;
}
assert.ok(checked >= 3, 'no vectors found');
console.log(`decoded ${checked} Python-encoded vectors`);

assert.throws(() => decoder.decode(new Uint8Array([1, 2, 3]), 2, 64), Error);
console.log('corrupt input throws');

// A 1024x1024 u16 frame shaped like camera data: gradient plus noise.
const W = 1024, H = 1024;
const frame = new Uint16Array(W * H);
for (let i = 0; i < frame.length; i++) {
  frame[i] = 1000 + 4 * Math.floor(i / W) + 2 * (i % W) + Math.floor(Math.random() * 64);
}
const bytes = new Uint8Array(frame.buffer);
const encoded = encode(bytes, 2);
assert.deepEqual(decoder.decode(encoded, 2, bytes.length), bytes);

const runs = 50;
const t0 = performance.now();
for (let i = 0; i < runs; i++) decoder.decode(encoded, 2, bytes.length);
const ms = (performance.now() - t0) / runs;
console.log(
  `1024x1024 u16: ${(bytes.length / 2 ** 20).toFixed(1)} MiB -> ` +
    `${(encoded.length / 2 ** 20).toFixed(2)} MiB ` +
    `(${(bytes.length / encoded.length).toFixed(2)}x), decode ${ms.toFixed(2)} ms ` +
    `(${(bytes.length / 2 ** 20 / (ms / 1000)).toFixed(0)} MiB/s)`,
);
