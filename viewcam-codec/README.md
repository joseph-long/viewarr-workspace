# viewcam-codec

Compression for camera frames that is cheap to decode in a browser: a
**bit shuffle** followed by **zstd**, and **noise-matched quantization**
that rounds a floating-point frame to a fraction of its noise first. The
decoder builds to a WASM module with no dependency on viewarr or egui, meant
to be loaded on its own in a web worker, so frames are decoded off the main
thread that draws the viewer.

Bit-shuffling first, all the elements' bit 0, then all their bit 1, and so
on (as HDF5's bitshuffle filter does), turns each bit a camera barely uses,
a 14-bit detector's top two or a mask's top seven, into a plane of nearly
constant bits that zstd all but removes. zstd's entropy coding then spends
on the noisy low planes only the bits their noise needs, where a plain
LZ-style compressor, finding no repeats in noise, saves nothing on them.

## Format

An encoded plane is a **zstd frame** (level 1) of the bit-shuffled bytes:
for `n` elements of `s` bytes, the first `8 × ⌊n / 8⌋` make `8s` planes of
`⌊n / 8⌋` bytes, plane `8b + k` holding bit `k` (0 the least significant)
of byte `b` of each element, element `i` at bit `i mod 8` of the plane's
byte `⌊i / 8⌋`; the last `n mod 8` elements follow as they are. The
decoded length is not taken from the stream: the receiver supplies it from
the image's width × height × element size, and decodes into a buffer of
exactly that length, so a corrupt stream cannot ask for an arbitrary
allocation.

Planes must be little-endian.

These are the payloads of the viewcam protocol's `bitshuffle-zstd` codec
(`viewcam-proto/PROTOCOL.md`). Its `raw` codec needs no decoder.

Decoding is pure Rust ([`ruzstd`](https://crates.io/crates/ruzstd)), so the
WASM build needs no C toolchain; encoding uses the C libzstd, behind the
`encode-zstd` feature, which only the Python extension enables.

### Noise-matched quantization

`Quantizer` rounds an image to steps a fraction of its noise, given a noise
model: variance `read_noise² + noise_slope × max(value − bias, 0)`. Above
`bias` values go through the square root that evens the noise out (the
generalized Anscombe transform), so the steps are finest at the faint end;
at and below it, they are linear. Codes are stored unsigned in as few bytes
as they need, less an offset. See "Quantization" in `PROTOCOL.md`.

## Python (servers)

[`python/`](python/) builds this same Rust code as a Python extension with
[maturin](https://www.maturin.rs/) and PyO3, so the server encodes with the
code the browser decodes with: one implementation, not two to keep in step.

```python
import viewcam_codec

payload = viewcam_codec.encode(frame)                  # bitshuffle-zstd
image = viewcam_codec.decode(payload, "<u2", (h, w))   # needs numpy

# Round to a quarter of the noise; codes are the smallest unsigned type that fits.
codes, offset = viewcam_codec.quantize(frame, bias, read_noise, noise_slope, 0.25)
values = viewcam_codec.dequantize(codes, offset, bias, read_noise, noise_slope, 0.25)
```

`encode` takes any C-contiguous little-endian buffer (numpy arrays included;
strided views are copied to contiguous first) and reads the element size
from it. It releases the GIL while compressing, so several threads can
encode at once. Wheels are `abi3`: one per platform covers Python 3.11 and
later.

On an Apple Silicon Mac, bit-shuffling and compressing a 1024×1024 `u16`
frame takes about 7 ms, and quantizing a floating-point frame first about
9 ms more.

## Web worker

[`js/decode-worker.js`](js/decode-worker.js), shipped in `pkg/`, is a reference
worker the web UI can use as-is or adapt. Feeding it is the page's job:

```javascript
const worker = new Worker(
  new URL('viewcam-codec/decode-worker.js', import.meta.url),
  { type: 'module' },
);

worker.onmessage = ({ data }) => {
  if (data.type === 'decoded') {
    const [frame, saturation] = data.planes;   // ArrayBuffers, transferred
    viewcam.setData('cam', frame, data.meta.width, data.meta.height, 'f32');
    viewcam.setSaturation('cam', saturation, data.meta.width, data.meta.height, 'u8');
  } else {
    console.error(data.message);
  }
};

// Encoded buffers are transferred, not copied; they are unusable afterwards.
worker.postMessage({
  id: seq,
  planes: [
    // Quantized codes, one byte each, decoding to float32 values.
    { encoded: dataBytes, elementSize: 1, decodedLength: w * h, quantized },
    { encoded: satBytes, elementSize: 1, decodedLength: w * h },
  ],
  meta: { width: w, height: h },
}, [dataBytes, satBytes]);
```

`quantized` is `{ codeOffset, bias, readNoise, noiseSlope, step }`, from the
frame's header; a plane without it decodes to its bytes. A plane with
`codec: 'raw'` is not decompressed. The worker decodes requests in arrival
order and drops nothing. To stay current under load, post only the newest
frame while one is being decoded.

The module can also be used directly: `new Decoder().decode(encoded,
elementSize, decodedLength)` returns a `Uint8Array` that owns its own buffer
(not a view into WASM memory), so it can be transferred onwards. A `Decoder`
reuses one scratch buffer across calls. `dequantize(stored, elementSize,
codeOffset, bias, readNoise, noiseSlope, step)` turns quantized codes into a
`Float32Array` of values. On an Apple Silicon Mac under Node, a 1024×1024
frame of one-byte codes takes about 6 ms to decompress and 6 ms to
dequantize.

## Building and testing

```bash
npm run build          # -> pkg/ (wasm-pack, --target web)
npm run test:wasm      # Node: decode the vectors, check dequantize
cargo test             # Rust: layout, round trips, corruption, vectors

cd python              # the Python extension, in a virtualenv:
pip install maturin numpy pytest
maturin develop --release
pytest tests
```

The format is specified and tested here, in Rust: the bit shuffle against
a plain bit-by-bit one, round trips for every element size and length, and
compression. The Rust tests also write `tests/vectors/`, planes and their
encodings, which the WASM module and the Python extension must decode, and
the extension encode to the same bytes, showing they are this same code
rather than versions of it. Regenerate with `VIEWCAM_REGEN_VECTORS=1 cargo
test` after a deliberate change to the format.

## License

MIT. Builds also contain `ruzstd` and `wasm-bindgen` code under their own
licenses; [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt) reproduces
them, and `licenses/generate.py` at the repository root keeps it current.
