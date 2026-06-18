# fitsview

[![Github Actions Status](https://github.com/joseph-long/jupyterlab-fitsview/workflows/Build/badge.svg)](https://github.com/joseph-long/jupyterlab-fitsview/actions/workflows/build.yml)
[![Binder](https://mybinder.org/badge_logo.svg)](https://mybinder.org/v2/gh/joseph-long/jupyterlab-fitsview/main?urlpath=%2Fdoc%2Ftree%2Fexample.fits)

A JupyterLab extension for viewing FITS (Flexible Image Transport System) files without downloading entire files to the browser. This is essential for working with large astronomical datasets.

![Screenshot of a JupyterLab interface with a notebook next to a FITS viewer](./screenshot.png)

## Features

- **Lazy loading**: Opens FITS files without downloading the full content to the browser
- **Handles FITS quirks**: Uses the battle-tested `astropy.io.fits` on the backend
- **Metadata display**: View HDU headers, shapes, and data types
- **Data slicing**: Request specific regions of data on demand
- **Multiple HDU support**: Browse all extensions in a FITS file

## Requirements

- JupyterLab >= 4.0.0
- Python >= 3.10
- astropy >= 5.0
- numpy

## Install

To install the extension, execute:

```bash
pip install fitsview
```

The server extension will be automatically enabled. Verify the installation:

```bash
# Check the lab extension
jupyter labextension list

# Check the server extension
jupyter server extension list
```

Both should show `fitsview` as enabled.

## Usage

1. Open JupyterLab
2. Navigate to a `.fits` file in the file browser
3. Double-click to open with the FITS Viewer
4. View HDU metadata and test data slicing

## Uninstall

To remove the extension, execute:

```bash
pip uninstall fitsview
```

## API Endpoints

The extension provides two REST API endpoints (namespaced under `/fitsview/`):

### GET `/fitsview/metadata`

Returns metadata for a FITS file.

**Parameters:**

- `path` (required): Path to the FITS file relative to the Jupyter server root

**Response:** JSON object with:

- `path`: File path
- `hdus`: Array of HDU info (index, name, type, header, shape, dtype)

### GET `/fitsview/slice`

Returns a data slice as raw bytes in little-endian byte order, preserving the original data type. Supports N-dimensional data (2D images, 3D cubes, 4D hypercubes, etc.).

**Parameters:**

- `path` (required): Path to the FITS file
- `hdu` (optional, default=0): HDU index
- `slices` (required): Comma-separated slice ranges for each axis in NumPy order.
  - Format: `start:stop,start:stop,...` with one range per axis
  - Uses Python/NumPy conventions: 0-indexed, half-open intervals `[start, stop)`
  - Axis order matches NumPy (for 2D: `row,col` or `y,x`; for 3D: `z,y,x`)
  - Examples:
    - 2D image: `slices=0:100,50:150` → rows 0-99, columns 50-149
    - 3D cube: `slices=0:10,0:100,50:150` → planes 0-9, rows 0-99, columns 50-149

**Response:** Binary data (`application/octet-stream`) with headers:

- `X-FITS-Shape`: JSON array of dimensions
- `X-FITS-Type`: Rust-style type name (e.g. `f64` or `u16`)

**Errors:** Returns 400 for out-of-bounds requests or dimension mismatches

## Contributing

### Development Setup

#### Prerequisites

- Python >= 3.10
- Node.js >= 18
- JupyterLab >= 4.0.0
- Rust toolchain (for building viewarr)

#### Building the viewarr Viewer Component

This extension uses [viewarr](https://github.com/joseph-long/viewarr), a WebAssembly image viewer built with Rust and egui. It is included as a git submodule and must be built before the extension.

**One-time setup:**

```bash
# Install wasm-pack if not already installed
cargo install wasm-pack

# Build the WebAssembly package (from the fitsview directory)
cd viewarr
npm run build
cd ..
```

**Rebuilding after viewarr changes:**

```bash
# From the fitsview directory, use the convenience script:
jlpm rebuild:viewarr
```

This runs `cargo clean -p viewarr && npm run build` in viewarr, copies the built package to `node_modules/viewarr`, and rebuilds the extension.

#### Initial Setup

```bash
# Clone the repository with submodules
git clone --recurse-submodules <repository-url>
cd fitsview

# Or if already cloned without submodules:
git submodule update --init --recursive

# Create a virtual environment
python -m venv .venv
# Install the package in development mode with all dependencies
.venv/bin/pip install -e '.[test]'
# activate virtual environment
source .venv/bin/activate  # On Windows: .venv\Scripts\activate

# Check to make sure jlpm is coming from inside the virtualenv (should end with '.venv/bin/jlpm')
which jlpm

# Install JavaScript dependencies
jlpm install

# Build the extension
jlpm build

# Link the extension to JupyterLab (creates symlink)
jupyter labextension develop . --overwrite

# Verify the labextension is 'enabled OK' and points to the .venv folder
jupyter labextension list

# Verify the server extension is enabled and coming from the .venv folder too
jupyter server extension list
```

#### Development with Auto-Rebuild

For the best development experience, run these commands in separate terminals:

**Terminal 1 - Watch TypeScript changes:**

```bash
source .venv/bin/activate
jlpm watch
```

**Terminal 2 - Run JupyterLab:**

```bash
source .venv/bin/activate
jupyter lab --no-browser
```

With this setup:

- TypeScript changes are automatically rebuilt when you save files
- Refresh your browser to see frontend changes
- Python (server extension) changes require restarting `jupyter lab`

#### Quick Iteration Workflow

1. Make changes to TypeScript files in `src/`
2. Wait for `jlpm watch` to rebuild (watch terminal output)
3. Refresh your browser (Cmd+Shift+R / Ctrl+Shift+R to hard refresh)

For Python changes in `fitsview/`:

1. Make changes to Python files
2. Restart the `jupyter lab` process
3. Refresh your browser

#### Debugging

**Frontend debugging:**

- Open browser DevTools (F12)
- Check Console for errors and extension logs
- Use Network tab to inspect API calls to `/fitsview/`

**Backend debugging:**

- Server logs appear in the terminal running `jupyter lab`
- Add `server_app.log.info("message")` for custom logging
- Use `--debug` flag: `jupyter lab --debug`

### Development Uninstall

```bash
# Uninstall the package
pip uninstall fitsview

# Remove the symlink (find location with `jupyter labextension list`)
# Then remove the fitsview symlink from the labextensions directory
```

### Testing the extension

#### Frontend tests (TypeScript/JavaScript)

This extension uses [Jest](https://jestjs.io/) for JavaScript testing:

```bash
jlpm test
```

#### Backend tests (Python)

This extension uses [pytest](https://pytest.org/) with [pytest-jupyter](https://github.com/jupyter-server/pytest-jupyter) for testing the server extension:

```bash
# Install test dependencies
pip install -e ".[test]"

# Run tests
python -m pytest fitsview/tests -v

# Run with coverage
python -m pytest fitsview/tests -v --cov=fitsview --cov-report=term-missing

# Run with coverage HTML report
python -m pytest fitsview/tests -v --cov=fitsview --cov-report=html
```

The Python tests cover:

- Metadata retrieval from FITS files
- Data slice extraction with dtype preservation
- Error handling for invalid paths, HDU indices, and out-of-bounds requests

#### Integration tests (End-to-End)

This extension uses [Playwright](https://playwright.dev/) with [Galata](https://github.com/jupyterlab/jupyterlab/tree/master/galata) for integration tests.

More information in [ui-tests/README.md](./ui-tests/README.md).

## Architecture

```
fitsview/
├── fitsview/               # Python server extension
│   ├── __init__.py         # Extension entry points
│   ├── handlers.py         # REST API handlers
│   └── tests/              # Python unit tests
├── src/                    # TypeScript frontend
│   ├── index.ts            # Plugin registration
│   ├── widget.ts           # FITS viewer widget
│   └── handler.ts          # API client utilities
├── ui-tests/               # Playwright integration tests
└── jupyter-config/         # Auto-enable configuration

viewarr/ (external)         # WebAssembly image viewer
├── src/                    # Rust source
│   ├── app.rs              # egui App with pan/zoom/render
│   ├── transform.rs        # Coordinate transforms
│   └── lib.rs              # WASM bindings
└── pkg/                    # Built wasm-pack output
```

The extension uses a **Content Provider** pattern to prevent JupyterLab from downloading full file contents. Instead, the frontend widget makes targeted API calls to fetch only the metadata and data slices needed.

The **viewarr** component is a Rust/WebAssembly viewer that provides:

- Hardware-accelerated rendering via WebGL (through egui)
- Pan and zoom with mouse/trackpad
- Support for multiple data types (uint8, uint16, int16, float32, float64)

## AI Coding Assistant Support

This project includes an `AGENTS.md` file with coding standards and best practices for JupyterLab extension development. See [AGENTS.md](AGENTS.md) for details.
