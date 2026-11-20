# viewcam-native

`viewcam`: watch a viewcam camera stream in a native window. It hosts the
same `ViewcamWidget` sup's camera page does (toolbar, viewer, status bar
with turbo), fed by the same protocol, so the browser and native clients
can be compared directly.

```bash
cargo run --release -- wss://exao1.magao-x.org/ws/viewcam/camsci2
```

The URL is the camera's data feed. Its dark, saturation mask and saturation
level are looked up in the server's stream catalog (`/ws/viewcam/`), as the
camera page does; `--dark`, `--saturation` and `--saturation-level`
override them, and `--no-catalog` skips the lookup. Frames come at one a
second until turbo is switched on in the status bar (`--turbo` starts with
it on).

## Headless throughput

`--headless` streams without a window for `--duration` seconds (10 by
default) and prints a line per feed each second, then a summary:

```
  3.0s data        24.0 fps  camera  24.1 fps   15.62 MB/s   649.9 kB/frame (3.2x)  decode   9.8/ 12.7 ms  display   5.5/  5.7 ms
```

- **fps**: frames that arrived. **camera**: how fast the camera's own frame
  counter advanced over those frames, so a gap between the two is frames the
  server never sent.
- **MB/s**, and a frame's size on the wire with its compression ratio.
- **decode** and **display**: mean/longest milliseconds per frame on the
  feed's decoder thread, and for what the window does with a frame (the
  widget's limits, colormap and texture, drawn in an offscreen egui
  context; only the GPU upload is left out). `--no-display` skips the latter.
- **superseded**: frames replaced by a newer one before the decoder got to
  them.
- **window**: the byte window last asked for, sized from the delivery rate
  (viewcam-proto/PROTOCOL.md, "Choosing a window"), and how many frames it
  lets be in flight at once. `--target-in-flight 0` turns it off, for
  comparison.

The summary adds the spread of gaps between frames. Headless runs flat out
(turbo's windows) unless told otherwise: `--window N` sets the data feed's
window, and `--max-fps F` paces acks as turbo-off does.

To try it without an instrument, run sup's fake camera and stream server
from a sup checkout:

```bash
export MILK_SHM_DIR=/tmp/shm && mkdir -p $MILK_SHM_DIR
python tests/fake_camera.py --name camsim --rate 24 --width 1024 --height 1024 &
sup-stream bind_port=19192 streaming.rtimv_config_dir=frontend/tests/fixtures/rtimv &
cargo run --release -- ws://127.0.0.1:19192/ws/viewcam/camsim --headless
```
