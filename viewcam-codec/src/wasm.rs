//! The WASM interface: what a web worker calls.
//!
//! Errors surface in JS as thrown `Error`s carrying the [`CodecError`] text.

use wasm_bindgen::prelude::*;

use crate::{decode_into, Quantizer};

/// Decodes byte-shuffled zstd planes. Holds one scratch buffer that grows to
/// the largest plane seen and is reused after that, so a worker decoding a
/// stream of same-sized frames allocates only each frame's output.
#[wasm_bindgen]
pub struct Decoder {
    scratch: Vec<u8>,
}

#[wasm_bindgen]
impl Decoder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Decoder {
        Decoder {
            scratch: Vec::new(),
        }
    }

    /// Decode `encoded` into a new `Uint8Array` of exactly `decodedLength`
    /// bytes. The result is an ordinary JS buffer, not a view into WASM
    /// memory, so it can be transferred to another thread with `postMessage`.
    ///
    /// Throws on corrupt input or a length that does not match.
    pub fn decode(
        &mut self,
        encoded: &[u8],
        element_size: usize,
        decoded_length: usize,
    ) -> Result<Vec<u8>, JsError> {
        let mut out = vec![0u8; decoded_length];
        decode_into(encoded, element_size, &mut self.scratch, &mut out)?;
        Ok(out)
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// The values of a quantized image: `stored` holds its codes, unsigned
/// integers of `element_size` (1, 2 or 4) bytes, each the code less
/// `code_offset`, and the other arguments are the [`Quantizer`]'s. Returns
/// a new `Float32Array`, one value per code.
#[wasm_bindgen]
pub fn dequantize(
    stored: &[u8],
    element_size: usize,
    code_offset: f64,
    bias: f64,
    read_noise: f64,
    noise_slope: f64,
    step: f64,
) -> Result<Vec<f32>, JsError> {
    let quantizer = Quantizer {
        bias,
        read_noise,
        noise_slope,
        step,
    };
    let mut out = vec![0f32; stored.len() / element_size.max(1)];
    quantizer.dequantize_into(stored, element_size, code_offset as i64, &mut out)?;
    Ok(out)
}
