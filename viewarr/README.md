# viewarr

An array/image viewer built with Rust, WebAssembly, and egui.

## Features

- Re-stretches images with linear, log, or symmetric linear scales
- Adjusts contrast and bias interactively by right-clicking and dragging
- Shows original pixel values on hover
- Supports multiple independent viewer instances per page
- Accepts all JavaScript TypedArray types (Int8, Uint8, Int16, Uint16, Int32, Uint32, BigInt64, BigUint64, Float32, Float64)
- Clean vanilla JS API with integration points for reactive frameworks

## Building

### Prerequisites

- [Rust](https://rustup.rs/) with `wasm32-unknown-unknown` target
- [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/)

### Build

```bash
# Install deps
npm install

# Build the WASM module and JS wrapper
npm run build

# Or for development (faster, larger, with debug symbols)
npm run build:dev
```

The built package will be in `pkg/`.

## Usage

### Installation

For development with a local copy:

```json
{
  "dependencies": {
    "viewarr": "file:../viewarr/pkg"
  }
}
```

### JavaScript API

```javascript
import { createViewer, setImageData, destroyViewer } from 'viewarr';

// Create a viewer in a container element
// The container must have an ID
await createViewer('my-container-id');

// Load image data
// buffer: ArrayBuffer with raw pixel data
// width, height: image dimensions
// dtype: numpy dtype string ("f4", "f8", "i2", "u1", etc.)
setImageData('my-container-id', buffer, width, height, dtype);

// Clean up when done
destroyViewer('my-container-id');
```

### Marker coordinates API

Markers are specified in continuous image coordinates `(x, y)`:

```javascript
import { setMarkers, getMarkers } from 'viewarr';

setMarkers('my-container-id', [
  [10.5, 20.25],
  [100.0, 42.0]
]);

const markers = getMarkers('my-container-id');
```

Markers are rendered as fixed-size plus signs in screen space and follow pan/zoom/rotation.

### Shift-click callback API

Use `onClick(...)` to receive continuous data-space coordinates from shift-click events:

```javascript
import { onClick } from 'viewarr';

onClick('my-container-id', ({ x, y }) => {
  console.log(`Shift-click at x=${x.toFixed(3)}, y=${y.toFixed(3)}`);
});
```

In `pyviewarr`, there is a notebook demo that combines this callback with marker updates:
`notebooks/shift_click_callback_demo.ipynb`.

### Container Requirements

- The container element **must have an ID** - this ID is used to identify the viewer instance
- The container should have defined dimensions (width and height)
- A `ResizeObserver` is automatically attached to handle dynamic resizing

### Multiple Viewers

Each container ID creates an independent viewer with its own state:

```javascript
await createViewer('viewer-1');
await createViewer('viewer-2');

setImageData('viewer-1', buffer1, 100, 100, 'u16');
setImageData('viewer-2', buffer2, 200, 200, 'f64');
```

## Integration with JupyterLab

This package is designed to be used as the image viewer backend for [jupyterlab-fitsview](https://github.com/joseph-long/jupyterlab-fitsview). It can also be embedded as a widget within a notebook using [pyviewarr](https://github.com/joseph-long/pyviewarr).

## Changelog

### Since last release

- Added shift-click event callbacks with continuous (fractional) data-space coordinates via `onClick(...)`.
- Added marker coordinate APIs: `getMarkers(...)` and `setMarkers(...)`.
- Added generic overlay text APIs: `getOverlayMessage(...)` and `setOverlayMessage(...)`.
- Added `overlayMessage` support in `setViewerState(...)` for bulk config application.
- Renamed overlay API from shift-click-specific names to generic names (`getOverlayMessage`/`setOverlayMessage`, `overlayMessage` in state config).
- Removed the default overlay text. Overlay rendering is now opt-in via explicit message.
- Updated overlay layout to compute safe positioning from live overlay bounds so it sits between the hover readout and zoom controls without overlap.
- Fixed diverging/symmetric colorbar limit behavior: `vmax` is now the sole editable limit, coerced positive, while `vmin` is disabled and displayed as `-vmax` without mutating underlying `min_val`.
- Fixed reported `vmin`/`vmax` values from state callbacks and `getValueRange()` to reflect effective display limits in diverging/symmetric mode.
- Fixed colorbar `vmin` textbox refresh when leaving diverging/symmetric mode so it correctly restores the underlying non-symmetric minimum value text.
- Added bulk state application via `setViewerState(...)` in the JS API for applying partial viewer configuration in one call.
- Added explicit JS/TS APIs for zoom and colormap control: `getZoom`/`setZoom`, `setColormap`, and `setColormapReversed`.
- Improved TypeScript typings with `StretchMode` and `ViewerStateConfig` to make state sync calls type-safe.
- Added Rust-side colormap setters exposed to WASM bindings (`setColormap`, `setColormapReversed`).
- Standardized colormap names and parsing (including common aliases like `grayscale`/`greyscale`) for more robust interop between Python/JS/Rust layers.
- Added direct widget support for setting colormap reversal without cycling through toggle paths.

## License

MIT
