"""Bit-shuffle + zstd for viewcam frames, and noise-matched quantization.

The same Rust code (the ``viewcam-codec`` crate) that browsers run as WASM to
decode frames, built as a Python extension so servers encode with it.

``encode`` produces the payload of the viewcam protocol's
``bitshuffle-zstd`` codec: a zstd frame of the image, first regrouped into
bit planes, all elements' bit 0, then all their bit 1, and so on (see the
crate's docs for the exact layout). The ``raw`` codec needs no encoding.

``quantize`` rounds an image to a fraction of its noise, given a noise model
(read noise, and how fast noise variance grows above a bias), for the
protocol's noise-matched quantization; ``dequantize`` turns the codes back
into values.
"""

from __future__ import annotations

from ._native import decode as _decode
from ._native import dequantize as _dequantize
from ._native import encode as _encode
from ._native import quantize as _quantize

__all__ = ["encode", "decode", "quantize", "dequantize"]


def encode(plane) -> bytes:
    """Compress an image: a C-contiguous little-endian numpy array, or any
    such buffer. Big-endian arrays must be byteswapped first, since
    little-endian is what clients decode to.

    Releases the GIL while compressing.
    """
    try:
        return _encode(plane)
    except BufferError:
        if not hasattr(plane, "__array__"):
            raise
        # A strided numpy view (a slice, a transpose): compress a contiguous
        # copy of it instead.
        import numpy as np

        return _encode(np.ascontiguousarray(plane))


def decode(encoded: bytes, dtype, shape):
    """Inverse of ``encode``, given the image's dtype and shape. Returns a
    read-only numpy array. Raises ``ValueError`` on corrupt input."""
    import numpy as np

    dtype = np.dtype(dtype)
    n = int(np.prod(shape))
    data = _decode(encoded, dtype.itemsize, n * dtype.itemsize)
    return np.frombuffer(data, dtype).reshape(shape)


_CODE_DTYPES = {1: "u1", 2: "<u2", 4: "<u4"}


def quantize(image, bias: float, read_noise: float, noise_slope: float, step: float):
    """Round ``image`` to ``step`` units of its noise, where a pixel at
    ``value`` has noise variance ``read_noise**2 + noise_slope * max(value -
    bias, 0)``. Returns ``(codes, code_offset)``: ``codes``, of ``image``'s
    shape, are unsigned integers of the fewest bytes that hold them, each
    the code less ``code_offset``. Raises ``ValueError`` for values that
    aren't finite, or a model that isn't one."""
    import numpy as np

    values = np.ascontiguousarray(image, dtype=np.float64)
    stored, element_size, code_offset = _quantize(values, bias, read_noise, noise_slope, step)
    return np.frombuffer(stored, _CODE_DTYPES[element_size]).reshape(values.shape), code_offset


def dequantize(codes, code_offset: int, bias: float, read_noise: float, noise_slope: float, step: float):
    """The float32 values ``quantize``'s codes stand for."""
    import numpy as np

    codes = np.ascontiguousarray(codes)
    data = _dequantize(codes.tobytes(), codes.itemsize, code_offset, bias, read_noise, noise_slope, step)
    return np.frombuffer(data, "<f4").reshape(codes.shape)
