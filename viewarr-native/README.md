# viewarr-native

A native desktop front-end for [viewarr](../viewarr): open an array file from
disk and get a window.

```bash
cargo run --release -- path/to/image.fits
```

Or install it onto your `PATH`:

```bash
cargo install --path .
viewarr path/to/image.fits        # one file
viewarr a.fits b.fits c.fits      # one tab each
viewarr                           # empty window; open files from the toolbar
```

There is no macOS app bundle yet — this is a plain CLI binary that opens a
window.

## The window

Chrome stacks above the image rather than floating on top of it, so the canvas
stays pure image. Each bar is subordinate to the one above:

```
┌ button bar ───────── Open… / Open in Tab — act on whichever tab is active
├ tab bar ──────────── which files are open
├ file info ────────── name · HDU picker · shape · type, for the active tab
├ slice controls ───── only for files with sliceable axes
└ viewer canvas
```

- **Open…** replaces the contents of the current tab. With nothing open yet, it
  opens the first one.
- **Open in Tab** adds a tab and switches to it.
- **Cmd+Shift+←/→** (Ctrl+Shift+←/→ off macOS) moves between tabs, wrapping at
  both ends. **✕** on a tab closes it.
- The **HDU** in the file-info bar is a picker whenever the file holds more
  than one HDU. It lists every HDU with its shape and type; ones this build
  cannot display are greyed out with the reason on hover, so a file whose image
  sits in a compressed extension explains itself rather than looking empty.

Every tab holds its own viewer state, so zoom, stretch, colormap and slice
position are per-file and survive switching away and back. Within a tab the
same is true per HDU: each one keeps the view you left it at.

## What it shows

The same `ArrayViewerWidget` the browser and JupyterLab front-ends embed, so
pan/zoom, stretch, colormaps, the colorbar and the hover readout all behave
identically. N-dimensional files get viewarr's per-axis slice bar with scrubbing
and playback.

Planes are read on demand from the memory-mapped file rather than loaded up
front, so scrubbing through a cube larger than RAM works.

## Options

```
viewarr [--hdu <INDEX|EXTNAME>] [--list] [PATHS]...
```

- `--list` prints each named file's HDUs and exits, without opening a window.
- `--hdu` picks which HDU to display, by index (`--hdu 1`) or by `EXTNAME`
  (`--hdu SCI`); it applies to every file named on the command line. The default
  is the first HDU holding a displayable image, which skips the dataless primary
  HDU that multi-extension files usually start with. Files opened from the
  toolbar start on that default; the file-info bar's HDU picker reaches the
  rest without reopening the file.
- A file that will not open is reported on stderr and skipped; the rest still
  open. If every named file fails, no window opens and the exit status is 1.

## Supported formats

FITS image HDUs, read by a small built-in parser (`src/fits.rs`) with no
system-library dependency:

- `BITPIX` 8, 16, 32, 64, -32 and -64, big-endian as the standard requires.
- `BZERO`/`BSCALE` linear scaling, including the `BZERO = 32768` convention for
  unsigned 16-bit data.
- `BLANK` undefined-pixel values, mapped to NaN so they drop out of auto-scaling.
- Any number of axes. `NAXIS1`/`NAXIS2` are the image; anything beyond becomes a
  slice axis.

**Precision is capped at float64.** Pixels are carried to the viewer as `f64`,
which is what the viewer's pixel pipe takes everywhere — it matches JavaScript's
native number type, so the WASM front-ends hand egui a `Float64Array` with no
per-pixel conversion. That is exact for `BITPIX` 8, 16 and 32 (and for the
`BZERO = 32768` unsigned-16 convention), since a 53-bit mantissa holds all of
them with room to spare. Only `BITPIX = 64` can lose anything, and only past
±2^53; values that large are vanishingly rare in image data, and matching the
browser front-ends is worth more than the corner case.

**Not yet supported:** tile-compressed images (`ZIMAGE = T` in a `BINTABLE`,
i.e. RICE/GZIP-compressed HDUs, which many archives ship). These are detected
and reported rather than misread — adding them means pulling in a decompressor
or linking cfitsio.

## Tests

```bash
cargo test
```

`tests/fits.rs` checks the reader against fixtures in `tests/data/` that were
written by `astropy.io.fits`, with expected pixel values taken from reading them
back with astropy. The fixtures encode each pixel's own coordinates in its
value, so a transposed, flipped, or mis-strided read fails loudly.

The tab bookkeeping in `src/app.rs` is unit-tested against those same fixtures —
switching, wrapping, closing, and the difference between Open and Open in Tab —
since none of that is visible to the FITS tests and all of it is easy to get
subtly wrong. `src/tab.rs` does the same for HDU switching, including that each
HDU keeps its own view state and that an unviewable one is refused without
disturbing the tab. What the tests cannot cover is the drawing itself or the
native file dialog; those need a human looking at the window.
