# viewcam-codec

Compression for camera frames that is cheap to decode in a browser: a
Blosc-style **byte shuffle** followed by **zstd**, and **noise-matched
quantization** that rounds a frame to a fraction of its noise first. The
decoder builds to a WASM module with no dependency on viewarr or egui, meant
to be loaded on its own in a web worker, so frames are decoded off the main
thread that draws the viewer.

Shuffling first, all the elements' byte 0, then all their byte 1, puts each
pixel's slowly varying high byte next to its neighbours' high bytes. zstd
then compresses the result: its entropy coding spends on the noisy low bytes
only the bits their noise needs, where a plain LZ-style compressor, finding
no repeats in noise, saves nothing on them.

## Format

An encoded plane is a **zstd frame** (level 1) of the shuffled bytes. The
decoded length is not taken from the stream: the receiver supplies it from
the image's width × height × element size, and decodes into a buffer of
exactly that length, so a corrupt stream cannot ask for an arbitrary
allocation.

Planes must be little-endian.

These are the payloads of the viewcam protocol's `shuffle-zstd` codec
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

payload = viewcam_codec.encode(frame)                  # shuffle-zstd
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

On an Apple Silicon Mac, quantizing and compressing a 1024×1024 `u16` frame
takes about 7 ms.

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
npm run test:wasm      # Node: decode the reference vectors, check dequantize
cargo test             # Rust: layout, round trips, corruption, reference vectors

cd python              # the Python extension, in a virtualenv:
pip install maturin numpy zstandard pytest
maturin develop --release
pytest tests
```

The Python tests check the extension against an independent reference:
numpy for the shuffle and the C libzstd (through the `zstandard` package)
for compression, in both directions, which also proves the payloads are
standard zstd frames. `tests/vectors/` holds planes that reference encoded
(`py_*`, decoded by the Rust and WASM tests) and one the Rust code encoded
(`rs_*`, decoded by the reference). Regenerate with `VIEWCAM_REGEN_VECTORS=1`
on either suite.

## License

MIT. Builds also contain `ruzstd` and `wasm-bindgen` code under their own
licenses; [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt) reproduces
them, and `licenses/generate.py` at the repository root keeps it current.
