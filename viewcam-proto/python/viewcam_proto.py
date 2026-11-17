"""Reference encoder/decoder for the viewcam streaming protocol (server side).

Standard library only, so an instrument server can vendor this one file.
See ../PROTOCOL.md for the specification; the Rust crate in this directory's
parent is the client side, and both are checked against ../tests/vectors/.

A server writes ``encode_frame(...)`` / ``encode_dark(...)`` /
``encode_clear_dark()`` to a fresh unidirectional WebTransport stream per
message, finishing the stream after it, and reads client credit datagrams
with ``decode_credit``.
"""

from __future__ import annotations

import struct

MAGIC = b"VCAM"
VERSION = 1

KIND_FRAME = 1
KIND_DARK = 2
KIND_CLEAR_DARK = 3
KIND_CREDIT = 0x81

# dtype code -> (numpy-style name, bytes per element)
DTYPES = {
    1: ("u1", 1),
    2: ("i1", 1),
    3: ("<u2", 2),
    4: ("<i2", 2),
    5: ("<u4", 4),
    6: ("<i4", 4),
    7: ("<u8", 8),
    8: ("<i8", 8),
    9: ("<f4", 4),
    10: ("<f8", 8),
}

_PREFIX = struct.Struct("<4sBBH")  # magic, version, kind, reserved
_FRAME = struct.Struct("<4sBBHQIIBBH")  # prefix, seq, width, height, data, sat, reserved
_DARK = struct.Struct("<4sBBHIIB3x")  # prefix, width, height, dtype, reserved
_CREDIT = struct.Struct("<4sBBHQ")  # prefix, limit

FRAME_HEADER_LEN = _FRAME.size  # 28
DARK_HEADER_LEN = _DARK.size  # 20
CREDIT_LEN = _CREDIT.size  # 16


class ProtocolError(ValueError):
    pass


def dtype_code(dtype) -> int:
    """The protocol code for a numpy dtype (or anything with a ``.str``, or a
    string like ``"<u2"``). Big-endian data must be byteswapped first."""
    name = getattr(dtype, "str", dtype)
    # numpy spells single-byte types with "|"
    name = name.replace("|", "")
    for code, (n, _) in DTYPES.items():
        if name == n:
            return code
    raise ProtocolError(f"dtype {name!r} has no protocol code (little-endian only)")


def _plane_bytes(plane, width: int, height: int, code: int) -> bytes:
    data = bytes(plane) if not hasattr(plane, "tobytes") else plane.tobytes(order="C")
    expected = width * height * DTYPES[code][1]
    if len(data) != expected:
        raise ProtocolError(f"plane is {len(data)} bytes, expected {expected}")
    return data


def encode_frame(
    seq: int,
    width: int,
    height: int,
    data,
    data_dtype: int,
    saturation=None,
    sat_dtype: int = 0,
) -> bytes:
    """A FRAME message: header, data plane, optional saturation plane.

    ``data``/``saturation`` are bytes-like or C-contiguous numpy arrays whose
    rows run from y = 0 upward; ``*_dtype`` are protocol codes (see
    ``dtype_code``). Pass ``saturation=None`` for a frame without one.
    """
    if (saturation is None) != (sat_dtype == 0):
        raise ProtocolError("give saturation and sat_dtype together, or neither")
    parts = [
        _FRAME.pack(MAGIC, VERSION, KIND_FRAME, 0, seq, width, height, data_dtype, sat_dtype, 0),
        _plane_bytes(data, width, height, data_dtype),
    ]
    if saturation is not None:
        parts.append(_plane_bytes(saturation, width, height, sat_dtype))
    return b"".join(parts)


def encode_dark(width: int, height: int, data, dtype: int) -> bytes:
    """A DARK message."""
    return _DARK.pack(MAGIC, VERSION, KIND_DARK, 0, width, height, dtype) + _plane_bytes(
        data, width, height, dtype
    )


def encode_clear_dark() -> bytes:
    """A CLEAR_DARK message."""
    return _PREFIX.pack(MAGIC, VERSION, KIND_CLEAR_DARK, 0)


def encode_credit(limit: int) -> bytes:
    """A CREDIT datagram (normally sent by clients; here for tests)."""
    return _CREDIT.pack(MAGIC, VERSION, KIND_CREDIT, 0, limit)


def decode_credit(datagram: bytes) -> int:
    """The ``limit`` from a client's CREDIT datagram."""
    if len(datagram) != CREDIT_LEN:
        raise ProtocolError(f"credit datagram is {len(datagram)} bytes, expected {CREDIT_LEN}")
    magic, version, kind, _, limit = _CREDIT.unpack(datagram)
    if magic != MAGIC:
        raise ProtocolError("not a viewcam message (bad magic)")
    if version != VERSION:
        raise ProtocolError(f"unsupported protocol version {version}")
    if kind != KIND_CREDIT:
        raise ProtocolError(f"expected a credit datagram, got kind {kind:#04x}")
    return limit


class CreditGate:
    """The server's side of the credit window, for one session.

    Call ``on_credit`` with each datagram's limit, ``may_send`` before opening
    a FRAME stream, and ``sent`` after opening one.
    """

    def __init__(self) -> None:
        self.limit = 0
        self.opened = 0

    def on_credit(self, limit: int) -> None:
        # Datagrams can arrive out of order; credit only ever grows.
        self.limit = max(self.limit, limit)

    def may_send(self) -> bool:
        return self.opened < self.limit

    def sent(self) -> None:
        self.opened += 1
