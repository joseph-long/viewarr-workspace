//! Byte-shuffle + LZ4: lossless, cheap-to-decode compression for camera frames.
//!
//! Raw detector pixels compress poorly: the low byte of each `u16` is mostly
//! noise, and interleaved with the slowly varying high byte it hides what
//! little structure there is from LZ4. Shuffling first, all the elements'
//! byte 0, then all their byte 1, and so on (Blosc's "shuffle" filter), puts
//! the predictable bytes next to each other, where LZ4 finds them.
//!
//! ## Format
//!
//! An encoded plane is a raw LZ4 *block* (no frame, no size prefix) of the
//! shuffled bytes. The decoded length is not stored: the receiver already
//! knows it from the image's width, height and dtype, and taking it from the
//! stream would let a corrupt length ask for an arbitrary allocation.
//!
//! Server side, with numpy and the `lz4` package:
//!
//! ```python
//! shuffled = plane.view(np.uint8).reshape(-1, plane.itemsize).T.tobytes()
//! encoded = lz4.block.compress(shuffled, store_size=False)
//! ```
//!
//! The same plane decodes with [`decode`] here, or with the WASM
//! [`Decoder`](wasm::Decoder) in a browser worker.
//!
//! These are the payloads of the viewcam protocol's `shuffle-lz4` codec. Its
//! plain `lz4` codec (no shuffle) decodes with the same functions and an
//! element size of 1, since shuffling single bytes changes nothing.

use std::fmt;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// Element size of zero.
    ZeroElementSize,
    /// A plane whose length is not a whole number of elements.
    PartialElement { len: usize, element_size: usize },
    /// The LZ4 block is corrupt, or decodes to more than the expected length.
    Corrupt(String),
    /// The LZ4 block decoded to fewer bytes than expected.
    ShortOutput { expected: usize, got: usize },
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::ZeroElementSize => write!(f, "element size must be at least 1"),
            CodecError::PartialElement { len, element_size } => write!(
                f,
                "{len} bytes is not a whole number of {element_size}-byte elements"
            ),
            CodecError::Corrupt(why) => write!(f, "corrupt LZ4 block: {why}"),
            CodecError::ShortOutput { expected, got } => {
                write!(
                    f,
                    "LZ4 block decoded to {got} of the expected {expected} bytes"
                )
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

/// LZ4-compress bytes into a raw block, no size prefix. On its own this is
/// the protocol's plain `lz4` codec; [`encode`] shuffles first.
pub fn compress_block(data: &[u8]) -> Vec<u8> {
    lz4_flex::block::compress(data)
}

/// Shuffle and LZ4-compress a plane of `element_size`-byte elements.
pub fn encode(plane: &[u8], element_size: usize) -> Result<Vec<u8>, CodecError> {
    Ok(compress_block(&shuffle(plane, element_size)?))
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
    // buffer has grown to the frame size; LZ4 overwrites every byte it
    // reports, and anything short of the full length is rejected below.
    scratch.resize(out.len(), 0);
    let got = lz4_flex::block::decompress_into(encoded, scratch)
        .map_err(|e| CodecError::Corrupt(e.to_string()))?;
    if got != out.len() {
        return Err(CodecError::ShortOutput {
            expected: out.len(),
            got,
        });
    }
    unshuffle_into(scratch, out, element_size)
}

#[cfg(test)]
mod tests;
