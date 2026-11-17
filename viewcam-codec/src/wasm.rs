//! The WASM interface: what a web worker calls.
//!
//! Errors surface in JS as thrown `Error`s carrying the [`CodecError`] text.

use wasm_bindgen::prelude::*;

use crate::{decode_into, encode as encode_plane};

/// Decodes byte-shuffled LZ4 planes. Holds one scratch buffer that grows to
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

/// Shuffle and LZ4-compress a plane. The server normally does this (in
/// Python); it is here for tests and for a client that ever needs to send
/// frames.
#[wasm_bindgen]
pub fn encode(plane: &[u8], element_size: usize) -> Result<Vec<u8>, JsError> {
    Ok(encode_plane(plane, element_size)?)
}
