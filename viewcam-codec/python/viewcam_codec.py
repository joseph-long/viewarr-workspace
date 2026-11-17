"""Reference byte-shuffle + LZ4 codec for viewcam (server side).

Needs numpy and the ``lz4`` package. See ../src/lib.rs for the format: a raw
LZ4 block (no size prefix) of the plane with its bytes shuffled Blosc-style,
all elements' byte 0, then all their byte 1, and so on.
"""

from __future__ import annotations

import lz4.block
import numpy as np


def encode(plane: np.ndarray) -> bytes:
    """Shuffle and compress a plane. Big-endian data must be byteswapped to
    little-endian first, since that is what clients decode to."""
    plane = np.ascontiguousarray(plane)
    if plane.dtype.byteorder == ">":
        raise ValueError("encode little-endian data; byteswap first")
    shuffled = plane.view(np.uint8).reshape(-1, plane.itemsize).T.tobytes()
    return lz4.block.compress(shuffled, store_size=False)


def decode(encoded: bytes, dtype, shape) -> np.ndarray:
    """Inverse of ``encode``, given the plane's dtype and shape."""
    dtype = np.dtype(dtype)
    n = int(np.prod(shape))
    shuffled = lz4.block.decompress(encoded, uncompressed_size=n * dtype.itemsize)
    if len(shuffled) != n * dtype.itemsize:
        raise ValueError(f"decoded {len(shuffled)} bytes, expected {n * dtype.itemsize}")
    raw = np.frombuffer(shuffled, np.uint8).reshape(dtype.itemsize, n).T.tobytes()
    return np.frombuffer(raw, dtype).reshape(shape)
