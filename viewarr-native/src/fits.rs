//! A small, dependency-free FITS reader for image HDUs.
//!
//! Only what the viewer needs: walk the HDU chain, describe each image HDU, and
//! read one 2-D plane at a time out of an N-D cube. The file is memory-mapped
//! and planes are decoded on demand, which is what lets the viewer scrub through
//! a multi-gigabyte cube without loading it.
//!
//! Scope: uncompressed image HDUs (`BITPIX` 8/16/32/64/-32/-64), including the
//! `BZERO`/`BSCALE` linear scaling that encodes unsigned integers. Tile-compressed
//! images (`ZIMAGE = T` inside a `BINTABLE`) are detected and reported, not read.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use memmap2::Mmap;

/// FITS files are a whole number of 2880-byte blocks; both headers and data
/// units are padded out to a multiple of this.
const BLOCK: usize = 2880;
/// Header cards are fixed-width 80-byte records, 36 to a block.
const CARD: usize = 80;

// =============================================================================
// Pixel types
// =============================================================================

/// The storage type of a FITS data array, as given by `BITPIX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitPix {
    U8,
    I16,
    I32,
    I64,
    F32,
    F64,
}

impl BitPix {
    fn from_code(code: i64) -> Result<Self> {
        Ok(match code {
            8 => BitPix::U8,
            16 => BitPix::I16,
            32 => BitPix::I32,
            64 => BitPix::I64,
            -32 => BitPix::F32,
            -64 => BitPix::F64,
            other => bail!("unsupported BITPIX {other}"),
        })
    }

    /// Bytes per stored element.
    pub fn width(self) -> usize {
        match self {
            BitPix::U8 => 1,
            BitPix::I16 => 2,
            BitPix::I32 | BitPix::F32 => 4,
            BitPix::I64 | BitPix::F64 => 8,
        }
    }

    fn is_integer(self) -> bool {
        !matches!(self, BitPix::F32 | BitPix::F64)
    }

    /// Rust-style type name, matching the `array_type` strings viewarr's WASM
    /// API already uses.
    pub fn name(self) -> &'static str {
        match self {
            BitPix::U8 => "u8",
            BitPix::I16 => "i16",
            BitPix::I32 => "i32",
            BitPix::I64 => "i64",
            BitPix::F32 => "f32",
            BitPix::F64 => "f64",
        }
    }

    /// Decode one big-endian element. Integers come back exactly; `BLANK`
    /// substitution is the caller's job since it only applies to integer types.
    fn decode(self, bytes: &[u8]) -> (f64, Option<i64>) {
        match self {
            BitPix::U8 => {
                let v = bytes[0] as i64;
                (v as f64, Some(v))
            }
            BitPix::I16 => {
                let v = i16::from_be_bytes([bytes[0], bytes[1]]) as i64;
                (v as f64, Some(v))
            }
            BitPix::I32 => {
                let v = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64;
                (v as f64, Some(v))
            }
            BitPix::I64 => {
                let v = i64::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                (v as f64, Some(v))
            }
            BitPix::F32 => {
                let v = f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                (v as f64, None)
            }
            BitPix::F64 => {
                let v = f64::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                (v, None)
            }
        }
    }
}

// =============================================================================
// Header card values
// =============================================================================

/// The subset of FITS value types this reader cares about.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl Value {
    /// How the header viewer shows it: strings quoted the way the file has
    /// them, so a numeric-looking string is not mistaken for a number.
    pub fn display(&self) -> String {
        match self {
            Value::Int(v) => v.to_string(),
            Value::Float(v) => v.to_string(),
            Value::Bool(v) => if *v { "T" } else { "F" }.to_string(),
            Value::Str(v) => format!("'{v}'"),
        }
    }

    fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            // Some writers emit integral keywords in floating point form.
            Value::Float(v) if v.fract() == 0.0 => Some(*v as i64),
            _ => None,
        }
    }

    fn as_float(&self) -> Option<f64> {
        match self {
            Value::Int(v) => Some(*v as f64),
            Value::Float(v) => Some(*v),
            _ => None,
        }
    }
}

/// One header card, kept whole.
///
/// Everything here is what a reader would otherwise throw away: the comment,
/// the valueless `COMMENT`/`HISTORY` cards, and the order and duplication of
/// both. The viewer exists to show a header as the file actually has it, so
/// none of that can be normalised away.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// Keyword, or empty for a card that has only free text.
    pub key: String,
    /// `None` for `COMMENT`, `HISTORY`, and other cards carrying no value.
    pub value: Option<Value>,
    /// The `/ ...` comment, or for a valueless card, its free text.
    pub comment: Option<String>,
}

impl Card {
    /// Whether this card matches a filter box's text. The needle must already
    /// be lowercased -- the caller does it once rather than this doing it per
    /// card, per frame.
    pub fn matches(&self, lowercase_needle: &str) -> bool {
        if lowercase_needle.is_empty() {
            return true;
        }
        let hit = |text: &str| text.to_ascii_lowercase().contains(lowercase_needle);
        hit(&self.key)
            || self.value.as_ref().is_some_and(|v| hit(&v.display()))
            || self.comment.as_deref().is_some_and(hit)
    }
}

/// One parsed FITS header, in card order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Header {
    pub cards: Vec<Card>,
}

impl Header {
    /// The first value for `key`. Duplicates are kept in `cards` but the
    /// keywords this reader interprets are all single-valued.
    fn get(&self, key: &str) -> Option<&Value> {
        self.cards
            .iter()
            .find(|card| card.key == key)
            .and_then(|card| card.value.as_ref())
    }

    fn int(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Value::as_int)
    }

    fn float(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Value::as_float)
    }

    fn string(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Value::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        }
    }
}

/// Split a card's value field from its trailing comment, respecting quoted
/// strings (a `/` inside `'...'` is data, not a comment marker). The comment
/// is kept rather than dropped — it is often the only thing explaining what a
/// keyword means.
fn split_value_and_comment(field: &str) -> (&str, Option<&str>) {
    let value = split_comment(field);
    let comment = field[value.len()..]
        .strip_prefix('/')
        .map(str::trim)
        .filter(|c| !c.is_empty());
    (value, comment)
}

fn split_comment(field: &str) -> &str {
    let bytes = field.as_bytes();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                // Inside a string, '' is an escaped single quote.
                if in_string && bytes.get(i + 1) == Some(&b'\'') {
                    i += 1;
                } else {
                    in_string = !in_string;
                }
            }
            b'/' if !in_string => return &field[..i],
            _ => {}
        }
        i += 1;
    }
    field
}

fn parse_value(field: &str) -> Option<Value> {
    let text = split_comment(field).trim();
    if text.is_empty() {
        return None;
    }
    if let Some(rest) = text.strip_prefix('\'') {
        // Quoted string: ends at the first unescaped quote; '' means a literal '.
        let mut out = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    out.push('\'');
                } else {
                    break;
                }
            } else {
                out.push(c);
            }
        }
        return Some(Value::Str(out.trim_end().to_string()));
    }
    match text {
        "T" => return Some(Value::Bool(true)),
        "F" => return Some(Value::Bool(false)),
        _ => {}
    }
    if let Ok(v) = text.parse::<i64>() {
        return Some(Value::Int(v));
    }
    // FITS permits Fortran-style `D` exponents (1.5D3) alongside `E`.
    let normalized = text.replace(['D', 'd'], "E");
    normalized.parse::<f64>().ok().map(Value::Float)
}

/// Parse the header starting at `offset`, returning it along with the offset of
/// the data unit that follows (i.e. just past the block containing `END`).
fn parse_header(buf: &[u8], offset: usize) -> Result<(Header, usize)> {
    let mut cards = Vec::new();
    let mut pos = offset;
    loop {
        if pos + BLOCK > buf.len() {
            bail!("header at byte {offset} runs past the end of the file (truncated?)");
        }
        let block = &buf[pos..pos + BLOCK];
        pos += BLOCK;
        // BLOCK is a whole number of CARDs (36 of them), so nothing is left over.
        let (cards_in_block, _) = block.as_chunks::<CARD>();
        for card in cards_in_block {
            // Headers are restricted to printable ASCII, so this is lossless in
            // practice and merely defensive for malformed files.
            let text = String::from_utf8_lossy(card);
            let keyword = text[..8].trim();
            if keyword == "END" {
                return Ok((Header { cards }, pos));
            }
            // CONTINUE carries the next piece of a long string: no value
            // indicator, just a quoted segment and perhaps more comment. A
            // HIERARCH keyword leaves so little room that its value and even
            // its comment routinely spill onto these, so folding them back
            // together is what makes such a card readable at all.
            if keyword == "CONTINUE" {
                let (value_field, comment) = split_value_and_comment(&text[8..]);
                let segment = parse_value(value_field);
                let mut folded = false;
                if let Some(previous) = cards.last_mut() {
                    if let Some(Value::Str(accumulated)) = previous.value.as_mut() {
                        // Only now is a trailing '&' known to be a
                        // continuation marker rather than part of the value:
                        // a string that merely ends in '&' and is never
                        // continued keeps it, as astropy does.
                        if accumulated.ends_with('&') {
                            accumulated.pop();
                        }
                        if let Some(Value::Str(piece)) = &segment {
                            accumulated.push_str(piece);
                        }
                        // The standard concatenates the comments too, which
                        // is how a comment too long for its own card arrives.
                        if let Some(extra) = comment {
                            match previous.comment.as_mut() {
                                Some(existing) => {
                                    existing.push(' ');
                                    existing.push_str(extra);
                                }
                                None => previous.comment = Some(extra.to_string()),
                            }
                        }
                        folded = true;
                    }
                }
                if !folded {
                    // Nothing to continue. astropy rejects the whole file
                    // here; a viewer is more useful showing the card than
                    // refusing to open the image beside it.
                    cards.push(Card {
                        key: keyword.to_string(),
                        value: segment,
                        comment: comment.map(str::to_string),
                    });
                }
                continue;
            }

            // A value is only present when columns 9-10 are exactly "= ".
            // Anything else -- COMMENT, HISTORY, a bare keyword -- is free
            // text, which the viewer still shows.
            if &text[8..10] == "= " {
                let (value_field, comment) = split_value_and_comment(&text[10..]);
                cards.push(Card {
                    key: keyword.to_string(),
                    value: parse_value(value_field),
                    comment: comment.map(str::to_string),
                });
                continue;
            }
            // The HIERARCH convention puts a multi-word keyword where a
            // standard card has its value indicator, so the `= ` is further
            // along and everything left of it is the keyword. Restricted to
            // HIERARCH on purpose: COMMENT and HISTORY text routinely
            // contains `=`, and hunting for one there would turn prose into
            // a bogus keyword/value pair.
            if keyword == "HIERARCH" {
                if let Some(eq) = text.find('=') {
                    // The keyword is what follows the marker, as the standard
                    // defines it and astropy reports it: HIERARCH announces
                    // the convention, it is not part of the name.
                    let key = text[..eq]
                        .trim()
                        .strip_prefix("HIERARCH")
                        .unwrap_or_default()
                        .trim();
                    let (value_field, comment) = split_value_and_comment(&text[eq + 1..]);
                    cards.push(Card {
                        key: key.to_string(),
                        value: parse_value(value_field),
                        comment: comment.map(str::to_string),
                    });
                    continue;
                }
            }

            let free_text = text[8..].trim_end();
            if keyword.is_empty() && free_text.trim().is_empty() {
                // Padding between the last card and END.
                continue;
            }
            cards.push(Card {
                key: keyword.to_string(),
                value: None,
                comment: Some(free_text.trim_start().to_string()).filter(|t| !t.is_empty()),
            });
        }
    }
}

// =============================================================================
// HDUs
// =============================================================================

/// One header-data unit, described well enough to read planes out of it.
pub struct Hdu {
    /// Position in the file's HDU chain, 0 being the primary HDU.
    pub index: usize,
    /// `EXTNAME` if present, else a synthesized label.
    pub name: String,
    pub bitpix: BitPix,
    /// Axis lengths in FITS order: `axes[0]` is `NAXIS1`, the fastest-varying
    /// (i.e. the opposite of the C/NumPy convention).
    pub axes: Vec<usize>,
    bzero: f64,
    bscale: f64,
    blank: Option<i64>,
    data_offset: usize,
    data_bytes: usize,
    /// A tile-compressed image masquerading as a binary table.
    pub compressed: bool,
    /// The HDU's header, whole, for the header viewer. A few keywords are
    /// read out of it above; the rest is kept because showing it is the
    /// point.
    pub header: Header,
}

impl Hdu {
    /// Whether this HDU holds a 2-D-or-higher image this reader can display.
    pub fn is_viewable_image(&self) -> bool {
        self.axes.len() >= 2 && !self.compressed && self.data_bytes > 0
    }

    /// Why this HDU cannot be displayed, or `None` if it can. Phrased to read
    /// after the HDU's name, in a listing or a tooltip.
    pub fn unviewable_reason(&self) -> Option<&'static str> {
        if self.compressed {
            Some("tile-compressed, not yet supported")
        } else if !self.is_viewable_image() {
            Some("no 2-D-or-larger image data")
        } else {
            None
        }
    }

    /// Shape for display, which unlike [`shape_string`](Self::shape_string)
    /// has something to say about a dataless HDU.
    pub fn shape_label(&self) -> String {
        if self.axes.is_empty() {
            "no data".to_string()
        } else {
            self.shape_string()
        }
    }

    pub fn width(&self) -> usize {
        self.axes.first().copied().unwrap_or(0)
    }

    pub fn height(&self) -> usize {
        self.axes.get(1).copied().unwrap_or(0)
    }

    /// The sliceable (non-image) axes in outer-to-inner order, which is the
    /// order `ArrayViewerWidget::set_cube` expects. For a FITS cube this is
    /// `[NAXISn, ..., NAXIS3]` — the reverse of `axes[2..]`.
    pub fn slice_dims(&self) -> Vec<usize> {
        self.axes.iter().skip(2).rev().copied().collect()
    }

    /// Whether displayed values are whole numbers, i.e. an integer `BITPIX`
    /// whose scaling keeps it integral (the `BZERO = 32768` unsigned-16 trick
    /// does; an arbitrary `BSCALE` does not).
    pub fn is_integer(&self) -> bool {
        self.bitpix.is_integer() && self.bscale == 1.0 && self.bzero.fract() == 0.0
    }

    /// Digits to show in the hover readout, matching the WASM side's choices.
    pub fn value_decimals(&self) -> usize {
        if self.is_integer() {
            0
        } else if self.bitpix == BitPix::F32 {
            4
        } else {
            6
        }
    }

    /// Human-readable shape, in FITS axis order.
    pub fn shape_string(&self) -> String {
        self.axes
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("x")
    }
}

// =============================================================================
// File
// =============================================================================

pub struct FitsFile {
    pub path: PathBuf,
    pub hdus: Vec<Hdu>,
    mmap: Mmap,
}

impl FitsFile {
    /// Memory-map `path` and walk its HDU chain. No pixel data is read here.
    pub fn open(path: &Path) -> Result<Self> {
        let file =
            File::open(path).with_context(|| format!("could not open {}", path.display()))?;
        // SAFETY: the usual mmap caveat — another process truncating the file
        // while we hold the map would fault. Acceptable for a desktop viewer.
        let mmap = unsafe { Mmap::map(&file) }
            .with_context(|| format!("could not memory-map {}", path.display()))?;

        if mmap.len() < BLOCK || !mmap.starts_with(b"SIMPLE  =") {
            bail!(
                "{} does not look like a FITS file (no SIMPLE record)",
                path.display()
            );
        }

        let hdus = scan_hdus(&mmap)
            .with_context(|| format!("could not read the structure of {}", path.display()))?;

        Ok(Self {
            path: path.to_path_buf(),
            hdus,
            mmap,
        })
    }

    /// The HDU a bare `viewarr file.fits` should show: the first one holding a
    /// displayable image.
    pub fn default_image_hdu(&self) -> Option<usize> {
        self.hdus.iter().position(Hdu::is_viewable_image)
    }

    /// Resolve which HDU to display: the caller's choice by index or `EXTNAME`,
    /// or the first displayable image when they did not name one.
    ///
    /// Lives here rather than in the CLI so that files opened through the GUI's
    /// file dialog pick their HDU — and explain a refusal — the same way.
    pub fn resolve_hdu(&self, requested: Option<&str>) -> Result<usize> {
        let Some(requested) = requested else {
            return self
                .default_image_hdu()
                .ok_or_else(|| self.no_image_error());
        };

        let index = if let Ok(index) = requested.parse::<usize>() {
            if index >= self.hdus.len() {
                bail!(
                    "{} has no HDU {index} (it has {})",
                    self.path.display(),
                    self.hdus.len()
                );
            }
            index
        } else {
            self.hdus
                .iter()
                .position(|h| h.name.eq_ignore_ascii_case(requested))
                .ok_or_else(|| {
                    anyhow!(
                        "{} has no HDU named {requested:?}; run with --list to see what it does have",
                        self.path.display()
                    )
                })?
        };

        let hdu = &self.hdus[index];
        if hdu.compressed {
            bail!(
                "HDU {index} ({}) is a tile-compressed image ({}), which this build cannot decompress yet",
                hdu.name,
                hdu.shape_string()
            );
        }
        if !hdu.is_viewable_image() {
            bail!(
                "HDU {index} ({}) holds no 2-D-or-larger image data; run with --list to see what the file contains",
                hdu.name
            );
        }
        Ok(index)
    }

    /// Explain what went wrong when nothing in the file can be displayed,
    /// naming compressed HDUs specifically since those are a common and
    /// fixable surprise.
    fn no_image_error(&self) -> anyhow::Error {
        let compressed = self.hdus.iter().filter(|h| h.compressed).count();
        if compressed > 0 {
            anyhow!(
                "{} has no uncompressed image HDU ({compressed} tile-compressed image(s) found, \
                 which this build cannot decompress yet)",
                self.path.display()
            )
        } else {
            anyhow!(
                "{} has no image HDU with 2 or more axes; run with --list to see what it contains",
                self.path.display()
            )
        }
    }

    /// Read a single 2-D plane as `f64`, in the row-major, bottom-row-first
    /// order the widget expects — which is exactly FITS storage order, so no
    /// flipping is needed here.
    ///
    /// `indices` selects a position along each sliceable axis in outer-to-inner
    /// order, matching [`Hdu::slice_dims`]; it is empty for a plain 2-D image.
    pub fn read_plane(&self, hdu: &Hdu, indices: &[usize]) -> Result<Vec<f64>> {
        let dims = hdu.slice_dims();
        if indices.len() != dims.len() {
            bail!(
                "expected {} slice indices for this HDU, got {}",
                dims.len(),
                indices.len()
            );
        }

        // Fold outer-to-inner to get the plane's position in the data array.
        // This mirrors C ordering, which is what FITS storage amounts to once
        // the axes are read back-to-front.
        let mut plane = 0usize;
        for (axis, (&len, &index)) in dims.iter().zip(indices).enumerate() {
            if index >= len {
                bail!("slice index {index} is out of range for axis of length {len} (axis {axis})");
            }
            plane = plane * len + index;
        }

        let plane_pixels = hdu
            .width()
            .checked_mul(hdu.height())
            .ok_or_else(|| anyhow!("image dimensions overflow"))?;
        let elem = hdu.bitpix.width();
        let start = hdu.data_offset + plane * plane_pixels * elem;
        let end = start + plane_pixels * elem;
        if end > self.mmap.len() {
            bail!(
                "data for this slice ends at byte {end} but the file is only {} bytes (truncated?)",
                self.mmap.len()
            );
        }

        let raw = &self.mmap[start..end];
        let mut out = Vec::with_capacity(plane_pixels);
        for chunk in raw.chunks_exact(elem) {
            let (value, as_int) = hdu.bitpix.decode(chunk);
            // BLANK marks undefined integer pixels; NaN is how the widget's
            // auto-scaling and hover readout already represent "no value".
            if let (Some(blank), Some(v)) = (hdu.blank, as_int) {
                if v == blank {
                    out.push(f64::NAN);
                    continue;
                }
            }
            out.push(hdu.bzero + hdu.bscale * value);
        }
        Ok(out)
    }
}

/// Walk the file block by block, parsing each header and stepping over its data
/// unit to find the next one.
fn scan_hdus(buf: &[u8]) -> Result<Vec<Hdu>> {
    let mut hdus = Vec::new();
    let mut offset = 0usize;
    let mut index = 0usize;

    while offset + BLOCK <= buf.len() {
        let (header, data_offset) = parse_header(buf, offset)?;

        let naxis = header.int("NAXIS").unwrap_or(0).max(0) as usize;
        let mut axes = Vec::with_capacity(naxis);
        for i in 1..=naxis {
            let len = header
                .int(&format!("NAXIS{i}"))
                .ok_or_else(|| anyhow!("HDU {index} declares NAXIS={naxis} but has no NAXIS{i}"))?;
            axes.push(len.max(0) as usize);
        }

        // Total data-unit size per the FITS standard, covering the PCOUNT/GCOUNT
        // used by tables and random-groups HDUs so we can step over them.
        let bitpix_code = header.int("BITPIX").unwrap_or(8);
        let elem_bits = bitpix_code.unsigned_abs() as usize;
        let pcount = header.int("PCOUNT").unwrap_or(0).max(0) as usize;
        let gcount = header.int("GCOUNT").unwrap_or(1).max(0) as usize;
        let axis_product: usize = if axes.is_empty() {
            0
        } else {
            axes.iter().product()
        };
        let data_bytes = if axis_product == 0 {
            0
        } else {
            (elem_bits / 8) * gcount * (pcount + axis_product)
        };

        let xtension = header.string("XTENSION").unwrap_or("").trim().to_string();
        let compressed = xtension == "BINTABLE" && header.bool("ZIMAGE").unwrap_or(false);

        // A tile-compressed image's real shape lives in ZNAXISn, not NAXISn
        // (which describe the table holding the compressed tiles). Report that
        // shape so the error message can be specific about what was skipped.
        let (axes, bitpix_code) = if compressed {
            let znaxis = header.int("ZNAXIS").unwrap_or(0).max(0) as usize;
            let mut zaxes = Vec::with_capacity(znaxis);
            for i in 1..=znaxis {
                zaxes.push(header.int(&format!("ZNAXIS{i}")).unwrap_or(0).max(0) as usize);
            }
            (zaxes, header.int("ZBITPIX").unwrap_or(bitpix_code))
        } else {
            (axes, bitpix_code)
        };

        let name = header
            .string("EXTNAME")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| {
                if index == 0 {
                    "PRIMARY".to_string()
                } else if xtension.is_empty() {
                    format!("HDU {index}")
                } else {
                    xtension.clone()
                }
            });

        // An unsupported BITPIX should not abort the whole file: other HDUs may
        // still be viewable, so record it as unreadable and carry on.
        let bitpix = BitPix::from_code(bitpix_code).unwrap_or(BitPix::U8);
        let readable = BitPix::from_code(bitpix_code).is_ok();

        hdus.push(Hdu {
            index,
            name,
            bitpix,
            axes: if readable { axes } else { Vec::new() },
            bzero: header.float("BZERO").unwrap_or(0.0),
            bscale: header.float("BSCALE").unwrap_or(1.0),
            blank: header.int("BLANK"),
            data_offset,
            data_bytes,
            compressed,
            header,
        });

        // Data units are padded out to a whole number of blocks.
        let padded = data_bytes.div_ceil(BLOCK) * BLOCK;
        offset = data_offset + padded;
        index += 1;
    }

    if hdus.is_empty() {
        bail!("no HDUs found");
    }
    Ok(hdus)
}
