// Smoke test of the built WASM package under Node: decodes the vectors the
// reference encoder made, and checks dequantizing. Run after `npm run build`:
//
//   npm run test:wasm

import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

import init, { Decoder, dequantize } from '../pkg/viewcam_codec.js';

const here = (p) => fileURLToPath(new URL(p, import.meta.url));

// `--target web` would fetch the .wasm; Node has no fetch for file: URLs.
await init({ module_or_path: readFileSync(here('../pkg/viewcam_codec_bg.wasm')) });
const decoder = new Decoder();

const vectors = here('../tests/vectors/');
let checked = 0;
for (const name of readdirSync(vectors)) {
  const match = name.match(/^(\w+)\.e(\d+)\.zst$/);
  if (!match) continue;
  const raw = readFileSync(vectors + `${match[1]}.raw`);
  const out = decoder.decode(readFileSync(vectors + name), Number(match[2]), raw.length);
  assert.deepEqual(Buffer.from(out), raw, name);
  checked++;
}
assert.ok(checked >= 6, 'no vectors found');
console.log(`decoded ${checked} vectors`);

assert.throws(() => decoder.decode(new Uint8Array([1, 2, 3]), 2, 64), Error);
console.log('corrupt input throws');

// Noise-matched codes back to values: read noise 14 above a bias of 432,
// slope 8, a quarter of the noise per step. Code 0 is the bias; the
// square root takes over above it.
const values = dequantize(new Uint8Array([0, 4, 8, 100]), 1, -4, 432, 14, 8, 0.25);
const expected = [-4, 0, 4, 96].map((code) => {
  const y = code * 0.25;
  if (y <= 0) return 432 + y * 14;
  const r = 8 * y / 2 + 14;
  return 432 + (r * r - 14 * 14) / 8;
});
values.forEach((v, i) => assert.ok(Math.abs(v - expected[i]) < 1e-3, `${v} vs ${expected[i]}`));
console.log('dequantize matches the noise model');
