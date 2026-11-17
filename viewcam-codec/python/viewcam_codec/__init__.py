"""LZ4 and byte-shuffle + LZ4 codecs for viewcam frames.

The same Rust code (the ``viewcam-codec`` crate) that browsers run as WASM to
decode frames, built as a Python extension so servers encode with it.

``encode`` produces the payload of the viewcam protocol's ``shuffle-lz4``
codec, or with ``shuffle=False`` its ``lz4`` codec: a raw LZ4 block (no size
prefix) of the image, its bytes first shuffled Blosc-style, all elements'
byte 0, then all their byte 1, and so on. The ``raw`` codec needs no
encoding.
"""

from __future__ import annotations

from ._native import decode as _decode
from ._native import encode as _encode

__all__ = ["encode", "decode"]


def encode(plane, shuffle: bool = True) -> bytes:
    """Compress an image: a C-contiguous little-endian numpy array, or any
    such buffer. Big-endian arrays must be byteswapped first, since
    little-endian is what clients decode to.

    Releases the GIL while compressing.
    """
    try:
        return _encode(plane, shuffle)
    except BufferError:
        if not hasattr(plane, "__array__"):
            raise
        # A strided numpy view (a slice, a transpose): compress a contiguous
        # copy of it instead.
        import numpy as np

        return _encode(np.ascontiguousarray(plane), shuffle)


def decode(encoded: bytes, dtype, shape, shuffle: bool = True):
    """Inverse of ``encode``, given the image's dtype and shape. Returns a
    read-only numpy array. Raises ``ValueError`` on corrupt input."""
    import numpy as np

    dtype = np.dtype(dtype)
    n = int(np.prod(shape))
    element_size = dtype.itemsize if shuffle else 1
    data = _decode(encoded, element_size, n * dtype.itemsize)
    return np.frombuffer(data, dtype).reshape(shape)
