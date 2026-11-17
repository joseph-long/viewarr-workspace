"""Reference encoder/decoder for the viewcam streaming protocol (server side).

Standard library only, so an instrument server can vendor this one file.
See ../PROTOCOL.md for the specification; the Rust crate in this directory's
parent is the client side, and both are checked against ../tests/vectors/.

Every feed (data, dark, saturation) carries the same FRAME messages. A server
writes ``encode_frame(...)`` to a fresh unidirectional stream per frame,
finishing the stream after it, and reads client credit datagrams with
``decode_credit``. Payloads are passed in already encoded; for the LZ4
codecs, the ``viewcam_codec`` extension (``viewcam-codec/python``)
produces them.
"""

from __future__ import annotations

import struct

MAGIC = b"VCAM"
VERSION = 1

KIND_FRAME = 1
KIND_CREDIT = 0x81

CODEC_RAW = 0
CODEC_LZ4 = 1
CODEC_SHUFFLE_LZ4 = 2
CODECS = (CODEC_RAW, CODEC_LZ4, CODEC_SHUFFLE_LZ4)

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

# prefix (magic, version, kind, reserved), seq, width, height, dtype, codec,
# reserved, encoded_len
_FRAME = struct.Struct("<4sBBHQIIBBHI")
_CREDIT = struct.Struct("<4sBBHQ")  # prefix, limit

FRAME_HEADER_LEN = _FRAME.size  # 32
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


def max_encoded_len(codec: int, decoded: int) -> int:
    """The largest payload ``codec`` can produce for a ``decoded``-byte image."""
    if codec == CODEC_RAW:
        return decoded
    return decoded + decoded // 255 + 16  # LZ4's worst case


def encode_frame(seq: int, width: int, height: int, dtype: int, payload, codec: int = CODEC_RAW) -> bytes:
    """A FRAME message: the header, then ``payload``.

    ``payload`` is bytes-like (or a C-contiguous numpy array, for ``raw``),
    already encoded with ``codec``. Images are little-endian, row-major, with
    row 0 at the bottom. ``width = height = 0`` with an empty payload means
    "no image on this feed".
    """
    if dtype not in DTYPES:
        raise ProtocolError(f"unknown dtype code {dtype}")
    if codec not in CODECS:
        raise ProtocolError(f"unknown codec code {codec}")
    payload = payload.tobytes(order="C") if hasattr(payload, "tobytes") else bytes(payload)
    decoded = width * height * DTYPES[dtype][1]
    encoded = len(payload)
    if codec == CODEC_RAW:
        ok = encoded == decoded
    elif decoded == 0:
        ok = encoded == 0
    else:
        ok = 0 < encoded <= max_encoded_len(codec, decoded)
    if not ok:
        raise ProtocolError(
            f"codec {codec} payload of {encoded} bytes cannot decode to {decoded} bytes"
        )
    header = _FRAME.pack(MAGIC, VERSION, KIND_FRAME, 0, seq, width, height, dtype, codec, 0, encoded)
    return header + payload


def encode_empty_frame(seq: int, dtype: int) -> bytes:
    """A frame meaning "no image on this feed", e.g. a withdrawn dark."""
    return encode_frame(seq, 0, 0, dtype, b"")


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
    """The server's side of the credit window, for one feed.

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
