# viewarr workspace

A monorepo for the `viewarr` Rust/WASM array viewer and the two packages that build on it.

| Path                  | What it is                                                            |
| --------------------- | --------------------------------------------------------------------- |
| `viewarr/`            | The Rust/WASM array+image viewer (egui). **Single source of truth.**  |
| `pyviewarr/`          | `pyviewarr` anywidget — PyPI package `pyviewarr`.                      |
| `jupyterlab-fitsview/`| FITS viewer JupyterLab extension — PyPI package `fitsview`.            |

## Single-sourced viewarr

There is exactly one copy of `viewarr`, at the top level. Each downstream package has a
`viewarr` **symlink → `../viewarr`**, so their existing `file:`-based dependencies resolve to
the one real directory. Build viewarr once and both packages consume the same
`viewarr/pkg/` output. `viewarr/pkg/` is a build artifact (gitignored), not committed.

## Local development

```bash
# 1. Build viewarr once (Rust + wasm-pack required)
cd viewarr && npm install && npm run build      # -> viewarr/pkg/viewarr_bg.wasm

# 2a. pyviewarr
cd ../pyviewarr && npm install && npm run build:all
pip install -e .

# 2b. jupyterlab-fitsview
cd ../jupyterlab-fitsview && jlpm install && jlpm build
pip install -e .
jupyter labextension list 2>&1 | grep -i fitsview   # should show "OK"
```

After changing viewarr's Rust source, rebuild it once (step 1) and both packages pick up the
new `viewarr/pkg/`.

## CI / releases

A single workflow (`.github/workflows/ci.yml`) builds viewarr once, then builds and tests both
packages against it. On a published GitHub release it publishes **both** `fitsview` and
`pyviewarr` to PyPI — so a viewarr change ships into both packages from one release.

> **Before the first release from this repo:** update the PyPI Trusted Publisher config for
> both `fitsview` and `pyviewarr` to point at this repository and `ci.yml`.

## Prerequisites

- Rust toolchain + [`wasm-pack`](https://rustwasm.github.io/wasm-pack/) (`cargo install wasm-pack`)
- Node.js 18+
- Python 3.10+
