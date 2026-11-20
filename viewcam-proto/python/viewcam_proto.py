"""Reference encoder/decoder for the viewcam streaming protocol (server side).

Standard library only, so an instrument server can vendor this one file.
See ../PROTOCOL.md for the specification; the Rust crate in this directory's
parent is the client side, and both are checked against ../tests/vectors/.

Every feed (data, dark, saturation) is its own WebSocket carrying the same
FRAME messages, one per WebSocket message. A server sends
``encode_frame(...)`` as a binary message whenever its ``SendWindow`` allows,
and passes each binary message from the client through ``decode_ack``.
Payloads are passed in already encoded; for the ``bitshuffle-zstd`` codec, the
``viewcam_codec`` extension (``viewcam-codec/python``) produces them, and for
a quantized frame its ``quantize`` makes the codes, whose model goes in
``Quantization``.
"""

from __future__ import annotations

import math
import struct
from typing import NamedTuple

MAGIC = b"VCAM"
VERSION = 1

KIND_FRAME = 1
KIND_ACK = 0x81

CODEC_RAW = 0
CODEC_BITSHUFFLE_ZSTD = 1
CODECS = (CODEC_RAW, CODEC_BITSHUFFLE_ZSTD)

# dtype codes quantized codes may have: u8, u16, u32
CODE_DTYPES = (1, 3, 5)

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

# prefix (magic, version, kind, reserved), seq, acq_time_ns, width, height,
# dtype, codec, quantization, reserved, encoded_len, code_offset, bias,
# read_noise, noise_slope, step
_FRAME = struct.Struct("<4sBBHQqIIBBBBIqdddd")
# prefix, acked_frames, acked_bytes, window_frames, window_bytes
_ACK = struct.Struct("<4sBBHQQII")

FRAME_HEADER_LEN = _FRAME.size  # 80
ACK_LEN = _ACK.size  # 32


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
    small = 128 << 10  # zstd's worst case, ZSTD_COMPRESSBOUND
    return decoded + (decoded >> 8) + ((small - decoded) >> 11 if decoded < small else 0)


class Quantization(NamedTuple):
    """How a quantized frame's codes stand for values (PROTOCOL.md,
    "Quantization"): each payload element is a code less ``code_offset``,
    and code ``c`` is the value ``step * c`` units of noise from ``bias``,
    for noise of variance ``read_noise**2 + noise_slope * max(value - bias,
    0)``."""

    code_offset: int
    bias: float
    read_noise: float
    noise_slope: float
    step: float


def encode_frame(
    seq: int, width: int, height: int, dtype: int, payload, codec: int = CODEC_RAW,
    quantization: Quantization | None = None, acq_time_ns: int = 0,
) -> bytes:
    """A FRAME message: the header, then ``payload``.

    ``payload`` is bytes-like (or a C-contiguous numpy array, for ``raw``),
    already encoded with ``codec``. Images are little-endian, row-major, with
    row 0 at the bottom. ``width = height = 0`` with an empty payload means
    "no image on this feed".

    With ``quantization``, the image is quantized codes of ``dtype`` (u8, u16
    or u32), which the client turns into float32 values.

    ``acq_time_ns`` is when the image was acquired, in nanoseconds since the
    Unix epoch; 0 if not known.
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
    if quantization is None:
        flag, params = 0, (0, 0.0, 0.0, 0.0, 0.0)
    else:
        if dtype not in CODE_DTYPES:
            raise ProtocolError(f"quantized codes are u8, u16 or u32, not dtype {dtype}")
        q = quantization
        model = (q.bias, q.read_noise, q.noise_slope, q.step)
        if not all(math.isfinite(v) for v in model):
            raise ProtocolError("a quantization parameter is not finite")
        if not (q.read_noise > 0 and q.step > 0 and q.noise_slope >= 0):
            raise ProtocolError("read noise and step must be positive, and the slope not negative")
        flag, params = 1, (int(q.code_offset), *map(float, model))
    header = _FRAME.pack(
        MAGIC, VERSION, KIND_FRAME, 0, seq, int(acq_time_ns), width, height, dtype, codec, flag, 0, encoded, *params
    )
    return header + payload


def encode_empty_frame(seq: int, dtype: int) -> bytes:
    """A frame meaning "no image on this feed", e.g. a withdrawn dark."""
    return encode_frame(seq, 0, 0, dtype, b"")


class Ack(NamedTuple):
    """What a client has received on its connection, and how much more it
    lets be in flight."""

    acked_frames: int
    acked_bytes: int
    window_frames: int  # 0 pauses the feed
    window_bytes: int  # 0 means no byte limit


def encode_ack(ack: Ack) -> bytes:
    """An ACK message (normally sent by clients; here for tests)."""
    return _ACK.pack(MAGIC, VERSION, KIND_ACK, 0, *ack)


def decode_ack(message: bytes) -> Ack:
    """The ack in a client's message."""
    if len(message) != ACK_LEN:
        raise ProtocolError(f"ack message is {len(message)} bytes, expected {ACK_LEN}")
    magic, version, kind, _, *fields = _ACK.unpack(message)
    if magic != MAGIC:
        raise ProtocolError("not a viewcam message (bad magic)")
    if version != VERSION:
        raise ProtocolError(f"unsupported protocol version {version}")
    if kind != KIND_ACK:
        raise ProtocolError(f"expected an ack message, got kind {kind:#04x}")
    return Ack(*fields)


class SendWindow:
    """The server's side of the window, for one connection.

    Call ``on_ack`` with each ack the client sends, ``may_send`` with a
    frame's message length before sending it, and ``sent`` after.

    In flight is everything sent that the client hasn't reported receiving,
    wherever it is: the server's buffers, the kernel's, the network. Holding
    that to the client's window is what bounds the stale frames a stalled
    connection delivers once it recovers.
    """

    def __init__(self) -> None:
        self.sent_frames = 0
        self.sent_bytes = 0
        # Nothing may be sent before the first ack.
        self.ack = Ack(0, 0, 0, 0)

    def on_ack(self, ack: Ack) -> None:
        if (
            ack.acked_frames < self.ack.acked_frames
            or ack.acked_bytes < self.ack.acked_bytes
            or ack.acked_frames > self.sent_frames
            or ack.acked_bytes > self.sent_bytes
        ):
            raise ProtocolError(
                f"ack for {ack.acked_frames} frames, {ack.acked_bytes} bytes; "
                f"previously {self.ack.acked_frames}, {self.ack.acked_bytes}, "
                f"with {self.sent_frames}, {self.sent_bytes} sent"
            )
        self.ack = ack

    @property
    def in_flight_frames(self) -> int:
        return self.sent_frames - self.ack.acked_frames

    @property
    def in_flight_bytes(self) -> int:
        return self.sent_bytes - self.ack.acked_bytes

    def may_send(self, message_len: int) -> bool:
        window_frames, window_bytes = self.ack.window_frames, self.ack.window_bytes
        if self.in_flight_frames >= window_frames:
            return False
        # Alone, a frame may exceed the byte window; otherwise it never goes.
        return (
            window_bytes == 0
            or self.in_flight_frames == 0
            or self.in_flight_bytes + message_len <= window_bytes
        )

    def sent(self, message_len: int) -> None:
        self.sent_frames += 1
        self.sent_bytes += message_len
