# viewcam-codec

Lossless, cheap-to-decode compression for camera frames: a Blosc-style
**byte shuffle** followed by **LZ4** (via the pure-Rust
[`lz4_flex`](https://crates.io/crates/lz4_flex)). It builds to a 28 KB WASM
module with no dependency on viewarr or egui, meant to be loaded on its own
in a web worker, so frames are decoded off the main thread that draws the
viewer.

Shuffling first, all the elements' byte 0, then all their byte 1, puts each
pixel's slowly varying high byte next to its neighbours' high bytes, where
LZ4 can find the repetition the noisy low bytes hide. This is the same idea
as the bitshuffle/LZ4 compression used for X-ray detector streams, at byte
rather than bit granularity.

## Format

An encoded plane is a **raw LZ4 block** (no frame, no size prefix) of the
shuffled bytes. The decoded length is not stored: the receiver supplies it
from the image's width × height × element size, so a corrupt stream cannot
ask for an arbitrary allocation.

Planes must be little-endian.

These are the payloads of the viewcam protocol's `shuffle-lz4` codec
(`viewcam-proto/PROTOCOL.md`). Its plain `lz4` codec is the same without the
shuffle, and decodes here with an element size of 1, since shuffling single
bytes changes nothing. The `raw` codec needs no decoder.

## Python (servers)

[`python/`](python/) builds this same Rust code as a Python extension with
[maturin](https://www.maturin.rs/) and PyO3, so the server encodes with the
code the browser decodes with: one implementation, not two to keep in step.

```python
import viewcam_codec

payload = viewcam_codec.encode(frame)                  # shuffle-lz4
payload = viewcam_codec.encode(satmap, shuffle=False)  # lz4
image = viewcam_codec.decode(payload, "<u2", (h, w))   # needs numpy
```

`encode` takes any C-contiguous little-endian buffer (numpy arrays included;
strided views are copied to contiguous first) and reads the element size
from it. It releases the GIL while compressing, so several threads can
encode at once. Wheels are `abi3`: one per platform covers Python 3.11 and
later.

On an Apple Silicon Mac, `shuffle-lz4` encodes a 1024×1024 `u16` camera-like
frame in about 0.4 ms, against about 1.1 ms for numpy's shuffle followed by
the C liblz4, with an identical compression ratio (1.9× on that frame; plain
LZ4 managed 1.0×, which is why the shuffle is there).

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
    viewcam.setData('cam', frame, data.meta.width, data.meta.height, 'u16');
    viewcam.setSaturation('cam', saturation, data.meta.width, data.meta.height, 'u8');
  } else {
    console.error(data.message);
  }
};

// Encoded buffers are transferred, not copied; they are unusable afterwards.
worker.postMessage({
  id: seq,
  planes: [
    { encoded: dataBytes, elementSize: 2, decodedLength: w * h * 2 },
    { encoded: satBytes, elementSize: 1, decodedLength: w * h },
  ],
  meta: { width: w, height: h },
}, [dataBytes, satBytes]);
```

The worker decodes requests in arrival order and drops nothing. To stay
current under load, post only the newest frame while one is being decoded.

The module can also be used directly: `new Decoder().decode(encoded,
elementSize, decodedLength)` returns a `Uint8Array` that owns its own buffer
(not a view into WASM memory), so it can be transferred onwards. A `Decoder`
reuses one scratch buffer across calls.

## Building and testing

```bash
npm run build          # -> pkg/ (wasm-pack, --target web)
npm run test:wasm      # Node: decode the reference vectors, time a 1024x1024 frame
cargo test             # Rust: layout, round trips, corruption, reference vectors

cd python              # the Python extension, in a virtualenv:
pip install maturin numpy lz4 pytest
maturin develop --release
pytest tests
```

The Python tests check the extension against an independent reference:
numpy for the shuffle and the C liblz4 (through the `lz4` package) for
compression, in both directions, which also proves the payloads are standard
LZ4. `tests/vectors/` holds planes that reference encoded (`py_*`, decoded by
the Rust and WASM tests) and one the Rust code encoded (`rs_*`, decoded by
the reference). Regenerate with `VIEWCAM_REGEN_VECTORS=1` on either suite.

On an Apple Silicon Mac under Node, a 1024×1024 `u16` frame decodes in about
1.4 ms (~1.4 GiB/s). The compression ratio depends entirely on the data; the
1.75× the smoke test prints is for synthetic gradient-plus-noise, not a real
camera.

## License

MIT. Builds also contain `lz4_flex` and `wasm-bindgen` code under their own
licenses; [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt) reproduces
them, and `licenses/generate.py` at the repository root keeps it current.
