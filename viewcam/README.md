# viewcam

A camera-frame viewer for the browser, built on [viewarr](../viewarr). It
takes up to three images per view and does the per-pixel work in WASM before
handing the result to an embedded viewarr viewer:

- **data**: the science frame.
- **dark**: subtracted from the data while **Subtract dark** is on. The
  viewer is given the difference, so hover values, auto limits and the
  colorbar histogram all describe the dark-subtracted image. The arithmetic is
  in `f64`, so a `u16` frame minus a brighter dark goes negative instead of
  wrapping.
- **saturation**: any nonzero pixel is painted over the image in the
  saturation colour (red by default, adjustable from the toolbar) while
  **Saturation** is on, and is transparent elsewhere. A camera without a
  saturation image can give a **saturation level** instead: each data
  frame's pixels at or above it are painted. The level is compared with the
  data as sent, before dark subtraction, since that is what the detector
  clipped. A level and an image replace each other.

The toolbar above the viewer holds the two toggles. Many cameras have no
dark or no saturation image, and that is not a fault: each toggle appears
only once its image has been set, and with neither set there is no toolbar.
A toggle whose image does not match the data's shape is greyed out, with the
reason on hover. A dark or saturation image is kept until it is replaced or
cleared, so a stream of data frames needs the dark sent only once, and an
image that arrives before data of its shape applies as soon as that data does.

The status bar below the viewer lists each feed a camera's images come
from, with its connection, and once frames arrive their shape, dtype,
sequence number and rate; then the bandwidth of all of them together, and a
**turbo** toggle. A feed whose newest frame is overdue for replacing (nothing
newer for 2.5 of its usual gaps between frames, and at least 1.5 s) is
flagged with that frame's age, from when it was acquired if the server said. viewcam doesn't receive the feeds itself, so the bar shows
only what it is told: whatever does receive them names them with `setFeed`
and reports with `setFeedConnection`, `noteFeedFrame`, `noteFeedEmpty` and
`noteReceived`. What turbo does is up to it too (sup's camera views lift a
one-frame-a-second cap); `onStateChange` reports the toggle. Until a feed is
named there is no status bar.

Camera specifics stay out of viewarr itself. The one hook viewcam needs, a
solid-colour mask overlay (`ArrayViewerWidget::set_mask_overlay`), is generic.

## Building

Same prerequisites as viewarr (Rust with the `wasm32-unknown-unknown` target,
and `wasm-pack`):

```bash
npm run build          # -> pkg/ (wrapper.js is the entry point)
cargo test             # the arithmetic and toolbar logic, natively
```

## Demo

```bash
npm run build && npm run demo     # serves this directory on :8000
open http://localhost:8000/demo/
```

The demo streams synthetic 512×512 `u16` frames: a fixed-pattern dark (an
offset, a gradient and hot columns), drifting stars, and saturation where
they clip at 65535.

## JavaScript API

```javascript
import viewcam from 'viewcam';

await viewcam.createViewer('cam');            // container must have an ID

viewcam.setDark('cam', dark.buffer, w, h, 'u16');        // once
viewcam.setData('cam', frame.buffer, w, h, 'u16');       // per frame
viewcam.setSaturation('cam', sat.buffer, w, h, 'u8');    // nonzero = saturated
viewcam.setSaturationLevel('cam', 65535);     // or: data >= level = saturated

viewcam.setDarkSubtraction('cam', false);     // the toolbar toggles
viewcam.setSaturationOverlay('cam', true);
viewcam.setSaturationColor('cam', '#00ff00');

viewcam.onStateChange('cam', ({ darkSubtraction, saturationOverlay, saturationColor }) => {});

viewcam.clearDark('cam');
viewcam.clearSaturation('cam');               // the image or the level
viewcam.destroyViewer('cam');
```

dtypes are the same as viewarr's `setImageData`. See `js/index.d.ts` for the
full surface.

## License

MIT. Builds also contain third-party code under their own licenses;
[THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt) reproduces them, and
`licenses/generate.py` at the repository root keeps it current.
