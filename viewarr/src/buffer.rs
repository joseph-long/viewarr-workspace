//! Typed-array plumbing shared by the WASM front-ends.
//!
//! Pixel buffers arrive from JavaScript as an `ArrayBuffer` plus a dtype
//! string; these turn them into the `f64` pixels the widget carries. Public so
//! wrappers built on viewarr (e.g. `viewcam`) accept exactly the same dtypes.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;

/// Display metadata for a dtype: (is_integer, value_decimals for hover readout).
pub fn dtype_meta(array_type: &str) -> (bool, usize) {
    let is_integer = matches!(
        array_type,
        "i8" | "u8" | "i16" | "u16" | "i32" | "u32" | "i64" | "u64"
    );
    let value_decimals = match array_type {
        "f32" => 4,
        "f64" => 6,
        _ => 0,
    };
    (is_integer, value_decimals)
}

/// Convert a JavaScript ArrayBuffer to Vec<f64> based on ArrayType string.
/// ArrayType values are Rust-style type specifiers (i8, u8, i16, etc.).
#[cfg(target_arch = "wasm32")]
pub fn convert_buffer_to_f64(
    buffer: &js_sys::ArrayBuffer,
    array_type: &str,
) -> Result<Vec<f64>, JsValue> {
    match array_type {
        "i8" => {
            let view = js_sys::Int8Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "u8" => {
            let view = js_sys::Uint8Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "i16" => {
            let view = js_sys::Int16Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "u16" => {
            let view = js_sys::Uint16Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "i32" => {
            let view = js_sys::Int32Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "u32" => {
            let view = js_sys::Uint32Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "i64" => {
            let view = js_sys::BigInt64Array::new(buffer);
            let len = view.length() as usize;
            let mut result = Vec::with_capacity(len);
            for i in 0..len {
                let val = view.get_index(i as u32);
                // i64 converts to f64 directly (may lose precision for very large values)
                result.push(val as f64);
            }
            Ok(result)
        }
        "u64" => {
            let view = js_sys::BigUint64Array::new(buffer);
            let len = view.length() as usize;
            let mut result = Vec::with_capacity(len);
            for i in 0..len {
                let val = view.get_index(i as u32);
                // u64 converts to f64 directly (may lose precision for very large values)
                result.push(val as f64);
            }
            Ok(result)
        }
        "f32" => {
            let view = js_sys::Float32Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "f64" | _ => {
            // Default to Float64
            let view = js_sys::Float64Array::new(buffer);
            Ok(view.to_vec())
        }
    }
}
