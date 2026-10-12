# viewarr-native

A native desktop front-end for [viewarr](../viewarr): open an array file from
disk and get a window.

```bash
cargo run --release -- path/to/image.fits
```

Or install it onto your `PATH`:

```bash
cargo install --path .
viewarr path/to/image.fits
```

There is no macOS app bundle yet — this is a plain CLI binary that opens a
window.

## What it shows

The same `ArrayViewerWidget` the browser and JupyterLab front-ends embed, so
pan/zoom, stretch, colormaps, the colorbar and the hover readout all behave
identically. N-dimensional files get viewarr's per-axis slice bar with scrubbing
and playback.

Planes are read on demand from the memory-mapped file rather than loaded up
front, so scrubbing through a cube larger than RAM works.

## Options

```
viewarr [--hdu <INDEX|EXTNAME>] [--list] <PATH>
```

- `--list` prints the file's HDUs and exits, without opening a window.
- `--hdu` picks which HDU to display, by index (`--hdu 1`) or by `EXTNAME`
  (`--hdu SCI`). The default is the first HDU holding a displayable image, which
  skips the dataless primary HDU that multi-extension files usually start with.

## Supported formats

FITS image HDUs, read by a small built-in parser (`src/fits.rs`) with no
system-library dependency:

- `BITPIX` 8, 16, 32, 64, -32 and -64, big-endian as the standard requires.
- `BZERO`/`BSCALE` linear scaling, including the `BZERO = 32768` convention for
  unsigned 16-bit data.
- `BLANK` undefined-pixel values, mapped to NaN so they drop out of auto-scaling.
- Any number of axes. `NAXIS1`/`NAXIS2` are the image; anything beyond becomes a
  slice axis.

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
