# AGENTS.md - Build and Development Guide

This document provides guidance for AI agents working on the pyviewarr codebase.

## Project Structure

This package lives in the `viewarr` monorepo. `viewarr/` here is a **symlink to the
single shared `../viewarr`** copy at the repo root — not a submodule or a private copy.
Building it updates the one shared `viewarr/pkg/` that `jupyterlab-fitsview` also uses.

```
pyviewarr/
├── viewarr/                 # symlink -> ../viewarr (single shared Rust/WASM viewer)
│   ├── src/                 # Rust source code
│   │   ├── lib.rs          # WASM bindings
│   │   ├── widget.rs       # Main viewer widget
│   │   ├── transform.rs    # Pan/zoom/rotation logic
│   │   ├── colormap.rs     # Colormap implementation
│   │   └── app.rs          # eframe app shell
│   ├── js/                  # JavaScript API wrapper
│   │   ├── index.js        # JS API for the viewer
│   │   └── index.d.ts      # TypeScript definitions
│   ├── pkg/                 # WASM build output (generated)
│   └── Cargo.toml
├── js/
│   └── widget.ts           # Jupyter widget frontend (TypeScript)
├── src/pyviewarr/
│   ├── __init__.py         # Python widget implementation
│   └── static/             # Built widget assets (generated)
│       ├── widget.js
│       └── widget.css
├── build.mjs               # esbuild script for widget bundling
└── package.json
```

## Build Sequence

### Standard build (recommended)
```bash
npm run build:all
```

This single command handles everything:
1. Builds the WASM module (`wasm-pack build`)
2. Copies wrapper files (`postbuild:wasm`) - copies `js/index.js` to `pkg/wrapper.js`
3. Builds the JavaScript widget

### Clean rebuild (if things are broken)
```bash
cd viewarr && npm run clean && cd .. && npm run build:all
```

### Development build (faster, unoptimized)
```bash
npm run build:dev
```

### Manual steps (for reference)
If you need to run steps individually:
```bash
# Step 1: Build WASM and copy wrapper in the shared viewarr
cd viewarr && npm run build && cd ..

# Step 2: Build the JavaScript widget
npm run build
```

## Running Tests

Rust tests can run on the native target (not WASM):
```bash
cd viewarr
cargo test
```

Note: The code uses `#[cfg(target_arch = "wasm32")]` gating for WASM-specific code to allow tests to run natively.

## Key Files to Know

### Rust/WASM Layer (viewarr/)
- **src/widget.rs**: Main viewer widget with UI controls, mouse handling, rendering
- **src/transform.rs**: Coordinate transformations for pan/zoom/rotation (has unit tests)
- **src/lib.rs**: WASM bindings and `ViewerHandle` API exposed to JavaScript
- **js/index.js**: JavaScript API that wraps the WASM viewer

### Python/Jupyter Layer (pyviewarr/)
- **js/widget.ts**: TypeScript frontend for the Jupyter widget (anywidget)
- **src/pyviewarr/__init__.py**: Python widget class with traitlets for state sync
- **build.mjs**: esbuild configuration that bundles widget.ts + WASM into static/widget.js

## Common Issues

### "Could not resolve 'viewarr'" during npm build
The `pkg/wrapper.js` file is missing. This happens if you ran `wasm-pack` directly instead of using the npm scripts. Fix with:
```bash
npm run build:all
```

### Changes not appearing after rebuild
1. Make sure you ran `npm run build:all` (not just `npm run build`)
2. Restart the Jupyter kernel
3. For a clean slate: `cd viewarr && npm run clean && cd .. && npm run build:all`

### Unicode/special characters not rendering
egui may not have all Unicode glyphs. Use ASCII-safe alternatives or common Unicode symbols that are widely supported.

## viewarr is shared (monorepo)

`viewarr/` is a symlink to `../viewarr`, the single shared copy at the repo root. It is no
longer a submodule — edit `../viewarr/src/...` directly and rebuild. The same `viewarr/pkg/`
is consumed by `jupyterlab-fitsview`, so a viewarr change affects both packages; build and
test both before committing. Everything is committed in one repo (no submodule pointer to
bump), and a single release publishes both packages with the new viewarr (see the root
`README.md` and `.github/workflows/ci.yml`).
