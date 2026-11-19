//! Python bindings for viewcam-codec.
//!
//! The server encodes with the same Rust the browser's WASM decoder runs, so
//! there is one implementation of the format, not two to keep in step.
//! `viewcam_codec/__init__.py` wraps these with the numpy-facing API.

use pyo3::buffer::PyUntypedBuffer;
use pyo3::exceptions::{PyBufferError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use viewcam_codec::{CodecError, Quantizer};

fn to_py(err: CodecError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// Compress `plane`, any C-contiguous little-endian buffer (a numpy array,
/// `bytes`, ...), into the payload of the protocol's `shuffle-lz4` codec, or
/// with `shuffle=False` its plain `lz4` codec, or with `zstd=True` its
/// `shuffle-zstd` codec. The element size comes from the buffer's own item
/// size.
///
/// The GIL is released while compressing, the expensive part, so several
/// threads can encode at once.
#[pyfunction]
#[pyo3(signature = (plane, shuffle = true, zstd = false))]
fn encode<'py>(
    py: Python<'py>,
    plane: &Bound<'py, PyAny>,
    shuffle: bool,
    zstd: bool,
) -> PyResult<Bound<'py, PyBytes>> {
    let buffer = PyUntypedBuffer::get(plane)?;
    if !buffer.is_c_contiguous() {
        return Err(PyBufferError::new_err(
            "buffer is not C-contiguous; use numpy.ascontiguousarray",
        ));
    }
    // Struct-module format codes: '>' and '!' are big-endian. Everything
    // else is native order, which is little-endian on every platform this
    // targets.
    if matches!(buffer.format().to_bytes().first(), Some(b'>' | b'!')) {
        return Err(PyValueError::new_err(
            "buffer is big-endian; byteswap to little-endian first",
        ));
    }
    // SAFETY: the buffer is C-contiguous, so its `len_bytes` bytes from
    // `buf_ptr` are one block, kept alive by `buffer` for this scope. It is
    // only read while the GIL is held, as any C extension reads one.
    let bytes =
        unsafe { std::slice::from_raw_parts(buffer.buf_ptr() as *const u8, buffer.len_bytes()) };
    // Copy (shuffling, if asked) under the GIL: one fast pass that leaves the
    // compression with data of its own, so it can run without the GIL and
    // without the caller's buffer.
    let data = if shuffle {
        viewcam_codec::shuffle(bytes, buffer.item_size()).map_err(to_py)?
    } else {
        bytes.to_vec()
    };
    drop(buffer);
    let encoded = if zstd {
        // Already shuffled (or not), so zstd sees it as single bytes.
        py.detach(|| viewcam_codec::encode_zstd(&data, 1))
            .map_err(to_py)?
    } else {
        py.detach(|| viewcam_codec::compress_block(&data))
    };
    Ok(PyBytes::new(py, &encoded))
}

/// Decode a `shuffle-lz4` payload (or with `element_size=1`, a plain `lz4`
/// one, or with `zstd=True` a `shuffle-zstd` one) to `decoded_len` bytes.
/// Raises `ValueError` on corrupt input or a length that does not match.
#[pyfunction]
#[pyo3(signature = (encoded, element_size, decoded_len, zstd = false))]
fn decode<'py>(
    py: Python<'py>,
    encoded: &[u8],
    element_size: usize,
    decoded_len: usize,
    zstd: bool,
) -> PyResult<Bound<'py, PyBytes>> {
    let encoded = encoded.to_vec();
    let decoded = py
        .detach(|| {
            if zstd {
                let mut out = vec![0u8; decoded_len];
                viewcam_codec::decode_zstd_into(&encoded, element_size, &mut Vec::new(), &mut out)
                    .map(|()| out)
            } else {
                viewcam_codec::decode(&encoded, element_size, decoded_len)
            }
        })
        .map_err(to_py)?;
    Ok(PyBytes::new(py, &decoded))
}

/// Quantize `values`, a C-contiguous buffer of float64, with a noise model
/// (see `viewcam_codec.Quantizer` in Rust). Returns `(stored, element_size,
/// code_offset)`: the codes less `code_offset`, as little-endian unsigned
/// integers of `element_size` bytes.
#[pyfunction]
fn quantize<'py>(
    py: Python<'py>,
    values: &Bound<'py, PyAny>,
    bias: f64,
    read_noise: f64,
    noise_slope: f64,
    step: f64,
) -> PyResult<(Bound<'py, PyBytes>, usize, i64)> {
    let buffer = pyo3::buffer::PyBuffer::<f64>::get(values)?;
    let values = buffer.to_vec(py)?;
    drop(buffer);
    let quantizer = Quantizer {
        bias,
        read_noise,
        noise_slope,
        step,
    };
    let q = py.detach(|| quantizer.quantize(&values)).map_err(to_py)?;
    Ok((PyBytes::new(py, &q.stored), q.element_size, q.code_offset))
}

/// The inverse of `quantize`: the values the stored codes stand for, as
/// little-endian float32 bytes.
#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn dequantize<'py>(
    py: Python<'py>,
    stored: &[u8],
    element_size: usize,
    code_offset: i64,
    bias: f64,
    read_noise: f64,
    noise_slope: f64,
    step: f64,
) -> PyResult<Bound<'py, PyBytes>> {
    let quantizer = Quantizer {
        bias,
        read_noise,
        noise_slope,
        step,
    };
    let mut out = vec![0f32; stored.len() / element_size.max(1)];
    quantizer
        .dequantize_into(stored, element_size, code_offset, &mut out)
        .map_err(to_py)?;
    let bytes: Vec<u8> = out.iter().flat_map(|v| v.to_le_bytes()).collect();
    Ok(PyBytes::new(py, &bytes))
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(encode, m)?)?;
    m.add_function(wrap_pyfunction!(decode, m)?)?;
    m.add_function(wrap_pyfunction!(quantize, m)?)?;
    m.add_function(wrap_pyfunction!(dequantize, m)?)?;
    Ok(())
}
