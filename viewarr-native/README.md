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

## macOS app

The same binary also ships as `viewarr.app`, which registers itself as a
viewer for `.fits` files: double-click one in Finder,
pick viewarr from **Open With**, or drop files on its Dock icon. Files sent
to an already-running viewarr open as new tabs in it, or switch to the
file's tab if it is already open.

Each GitHub release has it attached as `viewarr-<version>-macos.zip`, a
universal build for Apple Silicon and Intel Macs running macOS 11 or later;
every CI run uploads one as a workflow artifact too. To build it yourself:

```bash
macos/bundle.sh               # universal; needs both targets:
                              #   rustup target add aarch64-apple-darwin x86_64-apple-darwin
macos/bundle.sh --host-only   # just this Mac's architecture
open target/bundle/viewarr.app
```

**It is not signed with a Developer ID**, only ad-hoc, so a copy downloaded
from GitHub is quarantined and macOS will refuse to open it the first time.
Either try to open it once, then go to **System Settings → Privacy & Security**
and click **Open Anyway**, or clear the quarantine flag after unzipping:

```bash
xattr -dr com.apple.quarantine /Applications/viewarr.app
```

A bundle you build yourself is never quarantined and opens directly.

Finder does not pass files on the command line. It sends an "open documents"
Apple Event, which `src/macos.rs` answers by queueing the paths for the app
to open as tabs. The file types Finder will offer come from
`macos/Info.plist`; that handler passes along whatever arrives, so a new
format needs only its reader and a document-type entry there (the plist's
header comment covers directory formats such as Zarr).

## The window

Chrome stacks above the image rather than floating on top of it, so the canvas
stays pure image. Each bar is subordinate to the one above:

```
┌ button bar ───────── Open… / Open in Tab — act on whichever tab is active
├ tab bar ──────────── which files are open
├ file info ────────── name · HDU picker · shape · type · header, for the active tab
├ slice controls ───── only for files with sliceable axes
└ viewer canvas
```

- **Open…** replaces the contents of the current tab. With nothing open yet, it
  opens the first one.
- **Open in Tab** adds a tab and switches to it.
- Files dropped onto the window open as new tabs, one each.
- A file that already has a tab is never opened twice: any way of opening it
  again — either button, a drop, or Finder — switches to its tab instead.
- **Cmd+Shift+←/→** (Ctrl+Shift+←/→ off macOS) moves between tabs, wrapping at
  both ends, and **Cmd+1…9** jumps straight to one. Nine is the limit: the
  viewer uses a bare `0` to reset zoom, and a tenth tab is reachable by
  stepping or clicking. **✕** on a tab closes it.
- The **HDU** in the file-info bar is a picker whenever the file holds more
  than one HDU. It lists every HDU with its shape and type; ones this build
  cannot display are greyed out with the reason on hover, so a file whose image
  sits in a compressed extension explains itself rather than looking empty.
- **ⓘ** opens the header of whatever HDU is showing, in a window of its own —
  a real OS window, so it can go on a second monitor and two files' headers
  can sit side by side. Each tab has its own, and it closes with its tab.
  Cards appear in file order with their comments beside them, and the filter
  box matches keyword, value or comment. Values are selectable, since copying
  one out is most of why anyone opens a header. Keywords grow to fit, up to a
  third of the window, then wrap in place; values are kept on one line where
  they can be, with comments wrapping into what is left, and both wrap when
  neither will fit otherwise. The window will not shrink past the width where
  that stops working.

Every tab holds its own viewer state, so zoom, stretch, colormap and slice
position are per-file and survive switching away and back. Within a tab the
same is true per HDU: each one keeps the view you left it at.

Open files are followed on disk. When one is rewritten, its tab reloads once
the writing has stopped for a moment. If the array's shape is unchanged —
same image size, same slice axes — the view is kept whole: zoom, pan,
rotation, stretch, colormap, slice position, and limits you set by hand
(limits left on automatic re-fit to the new data, as they do across slices).
If the shape changed, the view starts over. A file caught mid-write, or
deleted, leaves the tab showing what it had, with the reason in the
file-info bar, and it reloads on the next change. Files replaced by renaming
a new one over them, as many programs save, are followed just the same.

## What it shows

The same `ArrayViewerWidget` the browser and JupyterLab front-ends embed, so
pan/zoom, stretch, colormaps, the colorbar and the hover readout all behave
identically. N-dimensional files get viewarr's per-axis slice bar with scrubbing
and playback.

Planes are read from the file on demand rather than loaded up front, so
scrubbing through a cube larger than RAM works. They are read with ordinary
positioned reads, not a memory map: a mapped file that another program
empties in place crashes the reader on its next access, whereas a read just
comes back short and the tab shows an error until the rewrite settles and
reloads.

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
- Whole headers, kept as the file has them — comments, the valueless `COMMENT`
  and `HISTORY` cards, and the order and duplication of both — for the header
  window. `HIERARCH` keywords are read whole, minus the marker itself, the
  way the standard defines them and `astropy` reports them. `CONTINUE` cards
  are folded back into the card they continue, so a long value — and a comment
  too long to share its own card — read as one row rather than three; this
  matters most for `HIERARCH`, whose keyword can leave so little room that
  both spill. A `CONTINUE` with nothing to continue is shown as itself rather
  than rejected.
- Any number of axes. `NAXIS1`/`NAXIS2` are the image; anything beyond becomes a
  slice axis.
- Image HDUs only — the primary HDU and `XTENSION = 'IMAGE'`. Tables declare
  `NAXIS = 2` as well (`NAXIS1` is the row length in bytes, `NAXIS2` the row
  count), so shape alone cannot tell them apart from an image; `XTENSION` is
  what does. Tables are listed in the HDU picker, greyed out and named for
  what they are, rather than hidden — a file whose data is all tables should
  say so rather than look empty.

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
disturbing the tab. `tests/data/header.fits` carries the cards a pixel-only reader would
drop — comments, duplicate `HISTORY`, a `HIERARCH` keyword — so the header
window has something to be tested against. `tests/data/longcards.fits` is
generated by `astropy.io.fits` and its expected values are what astropy reads
back, so the handling of over-long lines is pinned to astropy rather than to
our own reading of the spec. Two hand-built fixtures cover what astropy will
not write: `eqcomment.fits` (an `=` inside comment prose is prose, not a value
indicator) and `orphan.fits` (a `CONTINUE` with nothing before it). `src/source.rs` holds the
format-agnostic model the picker reads — a tree of groups and arrays, of which a FITS file is the flat case —
and is tested against synthetic nested trees, since no source produces one yet. Reloading is tested
in `src/tab.rs` against scratch copies of the fixtures rewritten underneath an
open tab — same shape, new shape, half-written, and HDUs that vanish — and
`src/watch.rs` checks that a real rewrite is noticed by the platform's own
watcher.
What the tests cannot cover is the drawing itself, the native file dialog, or
files arriving from Finder; those need a human looking at the window.
