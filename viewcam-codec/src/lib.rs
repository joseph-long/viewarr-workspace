//! Byte-shuffle + zstd, and noise-matched quantization: compression for
//! camera frames that decodes cheaply in a browser.
//!
//! Raw detector pixels compress poorly: the low byte of each `u16` is mostly
//! noise, and interleaved with the slowly varying high byte it hides what
//! little structure there is. Shuffling first, all the elements' byte 0,
//! then all their byte 1, and so on (Blosc's "shuffle" filter), puts the
//! predictable bytes next to each other. zstd then compresses the result,
//! its entropy coding spending on the noise only the bits it needs.
//!
//! ## Format
//!
//! An encoded plane is a zstd frame (level 1) of the shuffled bytes: the
//! payload of the viewcam protocol's `shuffle-zstd` codec. Decoding is into
//! a buffer of the length the receiver expects from the image's width,
//! height and dtype, never a length the stream gives, so a corrupt stream
//! cannot ask for an arbitrary allocation.
//!
//! Server side, with numpy and the `zstandard` package:
//!
//! ```python
//! shuffled = plane.view(np.uint8).reshape(-1, plane.itemsize).T.tobytes()
//! encoded = zstandard.ZstdCompressor(level=1).compress(shuffled)
//! ```
//!
//! The same plane decodes with [`decode`] here, or with the WASM
//! [`Decoder`](wasm::Decoder) in a browser worker. Decoding is pure Rust
//! ([`ruzstd`]), so it builds for wasm32; encoding ([`encode`]) uses the C
//! library, behind the `encode-zstd` feature, since only servers encode.
//!
//! ## Noise-matched quantization
//!
//! A [`Quantizer`] rounds an image to steps a fixed fraction of its noise,
//! which is all of its precision a display can use. Where noise grows with
//! the signal, as photon noise does, it first maps values through the
//! square root that makes the noise the same everywhere (the generalized
//! Anscombe transform), so the steps are fine at the faint end and coarse
//! only where bright pixels are noisy anyway.

use std::fmt;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// Element size of zero.
    ZeroElementSize,
    /// A plane whose length is not a whole number of elements.
    PartialElement { len: usize, element_size: usize },
    /// zstd failed: a corrupt frame, or one that decodes to more than the
    /// expected length.
    Zstd(String),
    /// Fewer bytes than expected: from a zstd frame, or quantized codes.
    ShortOutput { expected: usize, got: usize },
    /// Quantizer parameters that don't describe a noise model: a read noise
    /// or step that isn't positive, a negative slope, or anything not finite.
    BadQuantizer,
    /// An image value a quantizer can't round: NaN or infinite.
    NotFinite,
    /// Codes spanning more than a `u32` holds.
    CodeRangeTooLarge,
    /// Quantized codes are stored in 1, 2 or 4 bytes.
    UnsupportedCodeSize(usize),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::ZeroElementSize => write!(f, "element size must be at least 1"),
            CodecError::PartialElement { len, element_size } => write!(
                f,
                "{len} bytes is not a whole number of {element_size}-byte elements"
            ),
            CodecError::Zstd(why) => write!(f, "zstd: {why}"),
            CodecError::ShortOutput { expected, got } => {
                write!(f, "{got} bytes where {expected} were expected")
            }
            CodecError::BadQuantizer => write!(f, "quantizer parameters are not a noise model"),
            CodecError::NotFinite => write!(f, "can't quantize a value that isn't finite"),
            CodecError::CodeRangeTooLarge => write!(f, "codes span more than 32 bits"),
            CodecError::UnsupportedCodeSize(size) => {
                write!(f, "codes are stored in 1, 2 or 4 bytes, not {size}")
            }
        }
    }
}

impl std::error::Error for CodecError {}

fn check_layout(len: usize, element_size: usize) -> Result<usize, CodecError> {
    if element_size == 0 {
        return Err(CodecError::ZeroElementSize);
    }
    if !len.is_multiple_of(element_size) {
        return Err(CodecError::PartialElement { len, element_size });
    }
    Ok(len / element_size)
}

/// Shuffle `input` into `output`: byte `j` of element `i` moves to
/// `j * n + i`. Both must be the same length, a multiple of `element_size`.
pub fn shuffle_into(
    input: &[u8],
    output: &mut [u8],
    element_size: usize,
) -> Result<(), CodecError> {
    let n = check_layout(input.len(), element_size)?;
    assert_eq!(input.len(), output.len());
    match element_size {
        1 => output.copy_from_slice(input),
        // The common camera case, written out (as in `unshuffle_into`) so it
        // compiles to a tight split of one run into two.
        2 => {
            let (lo, hi) = output.split_at_mut(n);
            for ((pair, l), h) in input.chunks_exact(2).zip(lo).zip(hi) {
                *l = pair[0];
                *h = pair[1];
            }
        }
        // Walk the input contiguously; each element scatters one byte into
        // each of the `element_size` output runs.
        _ => {
            for (i, element) in input.chunks_exact(element_size).enumerate() {
                for (j, &byte) in element.iter().enumerate() {
                    output[j * n + i] = byte;
                }
            }
        }
    }
    Ok(())
}

/// Undo [`shuffle_into`].
pub fn unshuffle_into(
    input: &[u8],
    output: &mut [u8],
    element_size: usize,
) -> Result<(), CodecError> {
    let n = check_layout(input.len(), element_size)?;
    assert_eq!(input.len(), output.len());
    match element_size {
        1 => output.copy_from_slice(input),
        // The common camera case, written out so it compiles to a tight
        // interleave of two contiguous runs.
        2 => {
            let (lo, hi) = input.split_at(n);
            for ((pair, &l), &h) in output.chunks_exact_mut(2).zip(lo).zip(hi) {
                pair[0] = l;
                pair[1] = h;
            }
        }
        _ => {
            for (i, element) in output.chunks_exact_mut(element_size).enumerate() {
                for (j, byte) in element.iter_mut().enumerate() {
                    *byte = input[j * n + i];
                }
            }
        }
    }
    Ok(())
}

/// [`shuffle_into`] a new buffer.
pub fn shuffle(input: &[u8], element_size: usize) -> Result<Vec<u8>, CodecError> {
    let mut shuffled = vec![0u8; input.len()];
    shuffle_into(input, &mut shuffled, element_size)?;
    Ok(shuffled)
}

/// The zstd level servers use: the fastest, which on noisy frames compresses
/// nearly as well as any.
pub const ZSTD_LEVEL: i32 = 1;

/// Shuffle and zstd-compress a plane of `element_size`-byte elements: the
/// protocol's `shuffle-zstd` payload.
#[cfg(feature = "encode-zstd")]
pub fn encode(plane: &[u8], element_size: usize) -> Result<Vec<u8>, CodecError> {
    let shuffled = shuffle(plane, element_size)?;
    zstd::bulk::compress(&shuffled, ZSTD_LEVEL).map_err(|e| CodecError::Zstd(e.to_string()))
}

/// Decompress and unshuffle a plane that should decode to `decoded_len` bytes.
pub fn decode(
    encoded: &[u8],
    element_size: usize,
    decoded_len: usize,
) -> Result<Vec<u8>, CodecError> {
    let mut scratch = Vec::new();
    let mut out = vec![0u8; decoded_len];
    decode_into(encoded, element_size, &mut scratch, &mut out)?;
    Ok(out)
}

/// [`decode`] into `out`, whose length is the expected decoded length, using
/// `scratch` for the shuffled intermediate. Reusing one scratch buffer across
/// frames saves an allocation of a whole plane per frame.
pub fn decode_into(
    encoded: &[u8],
    element_size: usize,
    scratch: &mut Vec<u8>,
    out: &mut [u8],
) -> Result<(), CodecError> {
    check_layout(out.len(), element_size)?;
    // Resizing within the existing capacity costs nothing once the scratch
    // buffer has grown to the frame size. Decoding into a buffer of exactly
    // the expected length: a frame that would write more is refused, not
    // allowed to grow it, and one that writes less is rejected below.
    scratch.resize(out.len(), 0);
    let got = ruzstd::decoding::FrameDecoder::new()
        .decode_all(encoded, scratch)
        .map_err(|e| CodecError::Zstd(e.to_string()))?;
    if got != out.len() {
        return Err(CodecError::ShortOutput {
            expected: out.len(),
            got,
        });
    }
    unshuffle_into(scratch, out, element_size)
}

// =============================================================================
// Noise-matched quantization
// =============================================================================

/// Rounds an image to steps of `step` times its noise, given a noise model:
/// a pixel at `value` has noise variance
/// `read_noise² + noise_slope × max(value − bias, 0)`.
///
/// Values are first mapped to units of noise, `y`: linearly, `(value −
/// bias) / read_noise`, at and below `bias` (and everywhere when
/// `noise_slope` is 0), and above it through the square root that keeps the
/// noise at one unit, `(2 / noise_slope)(√(noise_slope × s + read_noise²) −
/// read_noise)` for `s = value − bias`. The two meet smoothly at `bias`. The
/// code is `round(y / step)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantizer {
    pub bias: f64,
    pub read_noise: f64,
    pub noise_slope: f64,
    pub step: f64,
}

/// A quantized image: codes stored as unsigned integers of `element_size`
/// (1, 2 or 4) bytes, little-endian, each `code − code_offset`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quantized {
    pub stored: Vec<u8>,
    pub element_size: usize,
    pub code_offset: i64,
}

impl Quantizer {
    pub fn check(&self) -> Result<(), CodecError> {
        let finite = [self.bias, self.read_noise, self.noise_slope, self.step]
            .iter()
            .all(|v| v.is_finite());
        if finite && self.read_noise > 0.0 && self.step > 0.0 && self.noise_slope >= 0.0 {
            Ok(())
        } else {
            Err(CodecError::BadQuantizer)
        }
    }

    /// A value in units of noise.
    pub fn forward(&self, value: f64) -> f64 {
        let s = value - self.bias;
        if self.noise_slope > 0.0 && s > 0.0 {
            let a = self.read_noise * self.read_noise;
            2.0 / self.noise_slope * ((self.noise_slope * s + a).sqrt() - self.read_noise)
        } else {
            s / self.read_noise
        }
    }

    /// The value at `y` units of noise.
    pub fn inverse(&self, y: f64) -> f64 {
        if self.noise_slope > 0.0 && y > 0.0 {
            let r = self.noise_slope * y / 2.0 + self.read_noise;
            self.bias + (r * r - self.read_noise * self.read_noise) / self.noise_slope
        } else {
            self.bias + y * self.read_noise
        }
    }

    pub fn code(&self, value: f64) -> f64 {
        (self.forward(value) / self.step).round()
    }

    /// The value a code stands for.
    pub fn value(&self, code: i64) -> f64 {
        self.inverse(code as f64 * self.step)
    }

    /// Quantize `values`, storing the codes in the fewest bytes that hold
    /// their range.
    pub fn quantize(&self, values: &[f64]) -> Result<Quantized, CodecError> {
        self.check()?;
        let mut codes = Vec::with_capacity(values.len());
        for &v in values {
            if !v.is_finite() {
                return Err(CodecError::NotFinite);
            }
            codes.push(self.code(v) as i64);
        }
        let code_offset = codes.iter().copied().min().unwrap_or(0);
        let range = codes.iter().copied().max().unwrap_or(0) - code_offset;
        let element_size = match range {
            0..=0xff => 1,
            0x100..=0xffff => 2,
            0x1_0000..=0xffff_ffff => 4,
            _ => return Err(CodecError::CodeRangeTooLarge),
        };
        let mut stored = Vec::with_capacity(codes.len() * element_size);
        for code in codes {
            let c = (code - code_offset) as u32;
            stored.extend_from_slice(&c.to_le_bytes()[..element_size]);
        }
        Ok(Quantized {
            stored,
            element_size,
            code_offset,
        })
    }

    /// Turn stored codes back into values, into `out`, one per code.
    pub fn dequantize_into(
        &self,
        stored: &[u8],
        element_size: usize,
        code_offset: i64,
        out: &mut [f32],
    ) -> Result<(), CodecError> {
        self.check()?;
        if !matches!(element_size, 1 | 2 | 4) {
            return Err(CodecError::UnsupportedCodeSize(element_size));
        }
        let n = check_layout(stored.len(), element_size)?;
        if n != out.len() {
            return Err(CodecError::ShortOutput {
                expected: out.len(),
                got: n,
            });
        }
        for (chunk, o) in stored.chunks_exact(element_size).zip(out.iter_mut()) {
            let mut bytes = [0u8; 4];
            bytes[..element_size].copy_from_slice(chunk);
            let code = u32::from_le_bytes(bytes) as i64 + code_offset;
            *o = self.value(code) as f32;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
