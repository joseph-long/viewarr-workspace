//! The viewcam frame-streaming protocol, without I/O.
//!
//! [PROTOCOL.md](../PROTOCOL.md) is the specification; this crate is its
//! reference implementation. It turns messages into frames and frames into
//! messages, and tracks a client's side of the window and the
//! newest-frame-wins rule for one feed. Reading and writing WebSocket
//! messages is left to the caller, so the same logic, and the same tests,
//! serve the native client and the browser.
//!
//! Payloads are carried as they arrived, still encoded; decompressing them,
//! and turning quantized codes into values, is the `viewcam-codec` crate's
//! job, so this one stays dependency-free.

use std::collections::VecDeque;
use std::fmt;

pub const MAGIC: [u8; 4] = *b"VCAM";
pub const VERSION: u8 = 1;

/// Length of the prefix every message starts with.
pub const PREFIX_LEN: usize = 8;
pub const FRAME_HEADER_LEN: usize = 80;
pub const ACK_LEN: usize = 32;

const KIND_FRAME: u8 = 1;
const KIND_ACK: u8 = 0x81;

/// Largest image a client accepts by default, in pixels. Bounds what a corrupt
/// header can make it allocate.
pub const DEFAULT_MAX_PIXELS: u64 = 1 << 28;

// =============================================================================
// Errors
// =============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtoError {
    BadMagic,
    UnsupportedVersion(u8),
    UnknownKind(u8),
    UnknownDtype(u8),
    UnknownCodec(u8),
    /// A quantization other than none (0) or noise-matched (1).
    UnknownQuantization(u8),
    /// A quantized frame that can't be one: codes that aren't an unsigned
    /// integer type, or a noise model that isn't one; or an unquantized
    /// frame with quantization parameters.
    BadQuantization(&'static str),
    /// An image larger than the receiver's limit.
    TooLarge {
        pixels: u64,
        max: u64,
    },
    /// An `encoded_len` its codec cannot produce for the image's size.
    BadEncodedLength {
        codec: Codec,
        encoded: u64,
        decoded: u64,
    },
    /// The message ended before its header said it would.
    Truncated {
        expected: usize,
        got: usize,
    },
    /// The message ran on past the length its header gave.
    TrailingBytes {
        expected: usize,
        got: usize,
    },
    /// A message of a kind the other side doesn't send (an ack from the
    /// server, a frame from the client).
    UnexpectedKind(u8),
    /// An ack for fewer frames or bytes than an earlier one, or for more than were sent.
    BadAck,
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::BadMagic => write!(f, "not a viewcam message (bad magic)"),
            ProtoError::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            ProtoError::UnknownKind(k) => write!(f, "unknown message kind {k:#04x}"),
            ProtoError::UnknownDtype(d) => write!(f, "unknown dtype code {d}"),
            ProtoError::UnknownCodec(c) => write!(f, "unknown codec code {c}"),
            ProtoError::UnknownQuantization(q) => write!(f, "unknown quantization {q}"),
            ProtoError::BadQuantization(why) => write!(f, "bad quantized frame: {why}"),
            ProtoError::TooLarge { pixels, max } => {
                write!(f, "image of {pixels} pixels exceeds the limit of {max}")
            }
            ProtoError::BadEncodedLength {
                codec,
                encoded,
                decoded,
            } => write!(
                f,
                "{} payload of {encoded} bytes cannot decode to {decoded} bytes",
                codec.name()
            ),
            ProtoError::Truncated { expected, got } => {
                write!(f, "message ended after {got} of {expected} bytes")
            }
            ProtoError::TrailingBytes { expected, got } => {
                write!(f, "{got}-byte message where {expected} bytes were expected")
            }
            ProtoError::UnexpectedKind(k) => {
                write!(f, "message kind {k:#04x} is not expected from this side")
            }
            ProtoError::BadAck => write!(
                f,
                "ack for less than an earlier one, or for more than was sent"
            ),
        }
    }
}

impl std::error::Error for ProtoError {}

// =============================================================================
// Dtypes and codecs
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
}

impl DType {
    pub const ALL: [DType; 10] = [
        DType::U8,
        DType::I8,
        DType::U16,
        DType::I16,
        DType::U32,
        DType::I32,
        DType::U64,
        DType::I64,
        DType::F32,
        DType::F64,
    ];

    pub fn code(self) -> u8 {
        match self {
            DType::U8 => 1,
            DType::I8 => 2,
            DType::U16 => 3,
            DType::I16 => 4,
            DType::U32 => 5,
            DType::I32 => 6,
            DType::U64 => 7,
            DType::I64 => 8,
            DType::F32 => 9,
            DType::F64 => 10,
        }
    }

    pub fn from_code(code: u8) -> Result<Self, ProtoError> {
        DType::ALL
            .into_iter()
            .find(|d| d.code() == code)
            .ok_or(ProtoError::UnknownDtype(code))
    }

    /// Bytes per element.
    pub fn size(self) -> usize {
        match self {
            DType::U8 | DType::I8 => 1,
            DType::U16 | DType::I16 => 2,
            DType::U32 | DType::I32 | DType::F32 => 4,
            DType::U64 | DType::I64 | DType::F64 => 8,
        }
    }

    /// The type specifier viewarr's `setImageData` takes ("u16", "f32", ...).
    pub fn name(self) -> &'static str {
        match self {
            DType::U8 => "u8",
            DType::I8 => "i8",
            DType::U16 => "u16",
            DType::I16 => "i16",
            DType::U32 => "u32",
            DType::I32 => "i32",
            DType::U64 => "u64",
            DType::I64 => "i64",
            DType::F32 => "f32",
            DType::F64 => "f64",
        }
    }

    /// Convert a decoded little-endian image to `f64`, the viewer's pixel
    /// type. Trailing bytes short of a whole element are ignored.
    pub fn to_f64(self, bytes: &[u8]) -> Vec<f64> {
        macro_rules! convert {
            ($t:ty) => {
                bytes
                    .chunks_exact(std::mem::size_of::<$t>())
                    .map(|c| <$t>::from_le_bytes(c.try_into().unwrap()) as f64)
                    .collect()
            };
        }
        match self {
            DType::U8 => convert!(u8),
            DType::I8 => convert!(i8),
            DType::U16 => convert!(u16),
            DType::I16 => convert!(i16),
            DType::U32 => convert!(u32),
            DType::I32 => convert!(i32),
            DType::U64 => convert!(u64),
            DType::I64 => convert!(i64),
            DType::F32 => convert!(f32),
            DType::F64 => convert!(f64),
        }
    }
}

/// How a frame's payload encodes its image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// The image itself.
    Raw,
    /// A zstd frame of the image's bits regrouped into bit planes.
    BitshuffleZstd,
}

impl Codec {
    pub const ALL: [Codec; 2] = [Codec::Raw, Codec::BitshuffleZstd];

    pub fn code(self) -> u8 {
        match self {
            Codec::Raw => 0,
            Codec::BitshuffleZstd => 1,
        }
    }

    pub fn from_code(code: u8) -> Result<Self, ProtoError> {
        Codec::ALL
            .into_iter()
            .find(|c| c.code() == code)
            .ok_or(ProtoError::UnknownCodec(code))
    }

    pub fn name(self) -> &'static str {
        match self {
            Codec::Raw => "raw",
            Codec::BitshuffleZstd => "bitshuffle-zstd",
        }
    }

    /// The largest payload this codec can produce for a `decoded`-byte image.
    pub fn max_encoded_len(self, decoded: u64) -> u64 {
        match self {
            Codec::Raw => decoded,
            // zstd's worst case, ZSTD_COMPRESSBOUND, frame header included.
            Codec::BitshuffleZstd => {
                let small = 128 << 10;
                let margin = if decoded < small {
                    (small - decoded) >> 11
                } else {
                    0
                };
                decoded + (decoded >> 8) + margin
            }
        }
    }
}

// =============================================================================
// Frames
// =============================================================================

/// How a quantized frame's codes stand for values: rounded to `step` units
/// of a noise whose variance is `read_noise² + noise_slope × max(value −
/// bias, 0)`, linearly at and below `bias` and through the square root that
/// evens out the noise above it. The payload's elements are each a code
/// less `code_offset`. See PROTOCOL.md, "Quantization", and
/// `viewcam_codec::Quantizer`, which turns codes back into values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantization {
    pub code_offset: i64,
    pub bias: f64,
    pub read_noise: f64,
    pub noise_slope: f64,
    pub step: f64,
}

impl Quantization {
    fn validate(&self) -> Result<(), ProtoError> {
        let model = [self.bias, self.read_noise, self.noise_slope, self.step];
        if !model.iter().all(|v| v.is_finite()) {
            return Err(ProtoError::BadQuantization("a parameter is not finite"));
        }
        if !(self.read_noise > 0.0 && self.step > 0.0 && self.noise_slope >= 0.0) {
            return Err(ProtoError::BadQuantization(
                "read noise and step must be positive, and the slope not negative",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameHeader {
    pub seq: u64,
    /// When the image was acquired, in nanoseconds since the Unix epoch
    /// (UTC, as the server's clock had it); 0 if the server doesn't know.
    pub acq_time_ns: i64,
    pub width: u32,
    pub height: u32,
    pub dtype: DType,
    pub codec: Codec,
    /// Payload length in bytes.
    pub encoded_len: u32,
    /// For a quantized frame, how its codes (of `dtype`, an unsigned integer
    /// type) stand for values; it decodes to `f32` values.
    pub quantization: Option<Quantization>,
}

fn prefix(kind: u8) -> [u8; PREFIX_LEN] {
    let mut p = [0u8; PREFIX_LEN];
    p[..4].copy_from_slice(&MAGIC);
    p[4] = VERSION;
    p[5] = kind;
    p
}

fn read_u32(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
}

fn read_u64(buf: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(buf[at..at + 8].try_into().unwrap())
}

/// Check the prefix and return the message kind, or `None` if `buf` is too
/// short to tell yet.
fn read_prefix(buf: &[u8]) -> Result<Option<u8>, ProtoError> {
    // Reject a bad magic as early as its first bytes allow, rather than
    // waiting for a full prefix of garbage.
    let n = buf.len().min(4);
    if buf[..n] != MAGIC[..n] {
        return Err(ProtoError::BadMagic);
    }
    if buf.len() < PREFIX_LEN {
        return Ok(None);
    }
    if buf[4] != VERSION {
        return Err(ProtoError::UnsupportedVersion(buf[4]));
    }
    Ok(Some(buf[5]))
}

impl FrameHeader {
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// A frame of no pixels: "no image on this feed".
    pub fn is_empty(&self) -> bool {
        self.pixels() == 0
    }

    /// Length of the image once decoded.
    pub fn decoded_len(&self) -> usize {
        self.pixels() as usize * self.dtype.size()
    }

    /// Header plus payload.
    pub fn message_len(&self) -> usize {
        FRAME_HEADER_LEN + self.encoded_len as usize
    }

    /// Check the header is self-consistent and within `max_pixels`.
    pub fn validate(&self, max_pixels: u64) -> Result<(), ProtoError> {
        let pixels = self.pixels();
        if pixels > max_pixels {
            return Err(ProtoError::TooLarge {
                pixels,
                max: max_pixels,
            });
        }
        let decoded = pixels * self.dtype.size() as u64;
        let encoded = self.encoded_len as u64;
        let fits = match self.codec {
            Codec::Raw => encoded == decoded,
            // An empty image has an empty payload; any other needs at least
            // one byte of compressed data.
            _ if decoded == 0 => encoded == 0,
            codec => encoded > 0 && encoded <= codec.max_encoded_len(decoded),
        };
        if !fits {
            return Err(ProtoError::BadEncodedLength {
                codec: self.codec,
                encoded,
                decoded,
            });
        }
        if let Some(q) = &self.quantization {
            if !matches!(self.dtype, DType::U8 | DType::U16 | DType::U32) {
                return Err(ProtoError::BadQuantization("codes are u8, u16 or u32"));
            }
            q.validate()?;
        }
        Ok(())
    }

    pub fn encode(&self) -> [u8; FRAME_HEADER_LEN] {
        let mut out = [0u8; FRAME_HEADER_LEN];
        out[..PREFIX_LEN].copy_from_slice(&prefix(KIND_FRAME));
        out[8..16].copy_from_slice(&self.seq.to_le_bytes());
        out[16..24].copy_from_slice(&self.acq_time_ns.to_le_bytes());
        out[24..28].copy_from_slice(&self.width.to_le_bytes());
        out[28..32].copy_from_slice(&self.height.to_le_bytes());
        out[32] = self.dtype.code();
        out[33] = self.codec.code();
        out[36..40].copy_from_slice(&self.encoded_len.to_le_bytes());
        if let Some(q) = &self.quantization {
            out[34] = 1;
            out[40..48].copy_from_slice(&q.code_offset.to_le_bytes());
            out[48..56].copy_from_slice(&q.bias.to_le_bytes());
            out[56..64].copy_from_slice(&q.read_noise.to_le_bytes());
            out[64..72].copy_from_slice(&q.noise_slope.to_le_bytes());
            out[72..80].copy_from_slice(&q.step.to_le_bytes());
        }
        out
    }

    /// Decode and validate the header at the start of a message, so it can
    /// be checked before the payload is trusted. `Ok(None)` means `buf` is
    /// too short to hold one.
    pub fn decode(buf: &[u8], max_pixels: u64) -> Result<Option<FrameHeader>, ProtoError> {
        match read_prefix(buf)? {
            None => return Ok(None),
            Some(KIND_FRAME) => {}
            Some(KIND_ACK) => return Err(ProtoError::UnexpectedKind(KIND_ACK)),
            Some(kind) => return Err(ProtoError::UnknownKind(kind)),
        }
        if buf.len() < FRAME_HEADER_LEN {
            return Ok(None);
        }
        let read_f64 = |at| f64::from_bits(read_u64(buf, at));
        let quantization = match buf[34] {
            0 if buf[40..80].iter().all(|&b| b == 0) => None,
            0 => {
                return Err(ProtoError::BadQuantization(
                    "an exact frame has no parameters",
                ))
            }
            1 => Some(Quantization {
                code_offset: read_u64(buf, 40) as i64,
                bias: read_f64(48),
                read_noise: read_f64(56),
                noise_slope: read_f64(64),
                step: read_f64(72),
            }),
            other => return Err(ProtoError::UnknownQuantization(other)),
        };
        let header = FrameHeader {
            seq: read_u64(buf, 8),
            acq_time_ns: read_u64(buf, 16) as i64,
            width: read_u32(buf, 24),
            height: read_u32(buf, 28),
            dtype: DType::from_code(buf[32])?,
            codec: Codec::from_code(buf[33])?,
            encoded_len: read_u32(buf, 36),
            quantization,
        };
        header.validate(max_pixels)?;
        Ok(Some(header))
    }
}

/// A complete frame, its payload still encoded.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}

impl Frame {
    /// Build a frame from an already-encoded payload, checking the result
    /// is one a client will accept.
    pub fn new(
        seq: u64,
        width: u32,
        height: u32,
        dtype: DType,
        codec: Codec,
        payload: Vec<u8>,
    ) -> Result<Frame, ProtoError> {
        Frame::quantized(seq, width, height, dtype, codec, payload, None)
    }

    /// As [`new`](Self::new), for a frame whose payload holds codes
    /// quantized as `quantization` says.
    pub fn quantized(
        seq: u64,
        width: u32,
        height: u32,
        dtype: DType,
        codec: Codec,
        payload: Vec<u8>,
        quantization: Option<Quantization>,
    ) -> Result<Frame, ProtoError> {
        let encoded_len =
            u32::try_from(payload.len()).map_err(|_| ProtoError::BadEncodedLength {
                codec,
                encoded: payload.len() as u64,
                decoded: width as u64 * height as u64 * dtype.size() as u64,
            })?;
        let header = FrameHeader {
            seq,
            acq_time_ns: 0,
            width,
            height,
            dtype,
            codec,
            encoded_len,
            quantization,
        };
        header.validate(DEFAULT_MAX_PIXELS)?;
        Ok(Frame { header, payload })
    }

    /// The same frame, acquired at `acq_time_ns` (nanoseconds since the
    /// Unix epoch).
    pub fn acquired_at(mut self, acq_time_ns: i64) -> Frame {
        self.header.acq_time_ns = acq_time_ns;
        self
    }

    /// "No image on this feed".
    pub fn empty(seq: u64, dtype: DType) -> Frame {
        Frame::new(seq, 0, 0, dtype, Codec::Raw, Vec::new()).unwrap()
    }

    /// Encode the whole message, as a server sends it.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.header.message_len());
        out.extend_from_slice(&self.header.encode());
        out.extend_from_slice(&self.payload);
        out
    }

    /// Decode one whole `FRAME` message, checking that its length is the
    /// one its header gives.
    pub fn decode(message: &[u8], max_pixels: u64) -> Result<Frame, ProtoError> {
        let Some(header) = FrameHeader::decode(message, max_pixels)? else {
            let expected = if message.len() < PREFIX_LEN {
                PREFIX_LEN
            } else {
                FRAME_HEADER_LEN
            };
            return Err(ProtoError::Truncated {
                expected,
                got: message.len(),
            });
        };
        let expected = header.message_len();
        let got = message.len();
        if got < expected {
            return Err(ProtoError::Truncated { expected, got });
        }
        if got > expected {
            return Err(ProtoError::TrailingBytes { expected, got });
        }
        Ok(Frame {
            header,
            payload: message[FRAME_HEADER_LEN..].to_vec(),
        })
    }
}

// =============================================================================
// Ack
// =============================================================================

/// A client's acknowledgement: what it has received on the connection, and how much
/// more it lets be in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ack {
    /// `FRAME`s received on this connection.
    pub acked_frames: u64,
    /// Their total length, headers included.
    pub acked_bytes: u64,
    /// Frames allowed in flight beyond those received; `0` pauses the feed.
    pub window_frames: u32,
    /// Bytes allowed in flight beyond those received; `0` means no limit.
    pub window_bytes: u32,
}

impl Ack {
    pub fn encode(&self) -> [u8; ACK_LEN] {
        let mut out = [0u8; ACK_LEN];
        out[..PREFIX_LEN].copy_from_slice(&prefix(KIND_ACK));
        out[8..16].copy_from_slice(&self.acked_frames.to_le_bytes());
        out[16..24].copy_from_slice(&self.acked_bytes.to_le_bytes());
        out[24..28].copy_from_slice(&self.window_frames.to_le_bytes());
        out[28..32].copy_from_slice(&self.window_bytes.to_le_bytes());
        out
    }

    pub fn decode(message: &[u8]) -> Result<Ack, ProtoError> {
        let got = message.len();
        match read_prefix(message)? {
            Some(KIND_ACK) if got == ACK_LEN => Ok(Ack {
                acked_frames: read_u64(message, 8),
                acked_bytes: read_u64(message, 16),
                window_frames: read_u32(message, 24),
                window_bytes: read_u32(message, 28),
            }),
            Some(KIND_ACK) if got > ACK_LEN => Err(ProtoError::TrailingBytes {
                expected: ACK_LEN,
                got,
            }),
            Some(KIND_ACK) | None => Err(ProtoError::Truncated {
                expected: ACK_LEN,
                got,
            }),
            Some(kind) => Err(ProtoError::UnexpectedKind(kind)),
        }
    }
}

/// A client's side of flow control and newest-frame-wins, for one feed.
#[derive(Clone, Debug)]
pub struct ClientState {
    acked_frames: u64,
    acked_bytes: u64,
    window_frames: u32,
    window_bytes: u32,
    newest_shown: Option<u64>,
}

impl ClientState {
    /// A new connection's state, with the window to ask for. Send
    /// [`ack`](Self::ack) as soon as the connection opens: the server
    /// sends nothing until it has one.
    pub fn new(window_frames: u32, window_bytes: u32) -> Self {
        Self {
            acked_frames: 0,
            acked_bytes: 0,
            window_frames,
            window_bytes,
            newest_shown: None,
        }
    }

    /// The ack describing the current state.
    pub fn ack(&self) -> Ack {
        Ack {
            acked_frames: self.acked_frames,
            acked_bytes: self.acked_bytes,
            window_frames: self.window_frames,
            window_bytes: self.window_bytes,
        }
    }

    /// A `FRAME` message of `message_len` bytes arrived, however it then
    /// fares (shown, dropped as stale, or malformed). Returns the ack to
    /// send straight away.
    pub fn frame_received(&mut self, message_len: usize) -> Ack {
        self.acked_frames += 1;
        self.acked_bytes += message_len as u64;
        self.ack()
    }

    /// Change the window, e.g. to `0` frames to pause a feed that isn't
    /// shown. Returns the ack to send straight away.
    pub fn set_window(&mut self, window_frames: u32, window_bytes: u32) -> Ack {
        self.window_frames = window_frames;
        self.window_bytes = window_bytes;
        self.ack()
    }

    /// Whether a frame with this `seq` is already superseded by one shown.
    pub fn is_stale(&self, seq: u64) -> bool {
        self.newest_shown.is_some_and(|newest| seq <= newest)
    }

    /// A frame is ready to show (decoded, perhaps off the main thread and
    /// out of order): `true` if it should be shown, being the newest yet, in
    /// which case it becomes the newest shown.
    pub fn accept(&mut self, seq: u64) -> bool {
        if self.is_stale(seq) {
            return false;
        }
        self.newest_shown = Some(seq);
        true
    }
}

// =============================================================================
// Sizing the byte window
// =============================================================================

/// How long a client aims to have in flight, by default: the byte window is
/// this much of the measured delivery rate. See PROTOCOL.md, "Choosing a window".
pub const DEFAULT_TARGET_IN_FLIGHT_SEC: f64 = 0.5;

/// Over how many of the latest messages, and how long, the delivery rate is
/// measured.
const RATE_MESSAGES: usize = 16;
const RATE_SPAN_SEC: f64 = 2.0;

/// The rate a feed's messages are arriving at, for sizing its byte window.
#[derive(Clone, Debug, Default)]
pub struct DeliveryRate {
    /// (seconds, bytes) of the latest arrivals, oldest first.
    arrivals: VecDeque<(f64, usize)>,
}

impl DeliveryRate {
    pub fn new() -> Self {
        Self::default()
    }

    /// A message of `bytes` arrived at `now`, in seconds on any steady clock.
    pub fn note(&mut self, now: f64, bytes: usize) {
        self.arrivals.push_back((now, bytes));
        while self.arrivals.len() > RATE_MESSAGES
            || self
                .arrivals
                .front()
                .is_some_and(|&(t, _)| now - t > RATE_SPAN_SEC)
        {
            self.arrivals.pop_front();
        }
    }

    /// Forget what arrived, as for a new connection.
    pub fn clear(&mut self) {
        self.arrivals.clear();
    }

    /// Bytes a second: all but the first recent message's bytes, over the
    /// time since it arrived. `None` until two have arrived close enough
    /// together to say.
    pub fn bytes_per_sec(&self) -> Option<f64> {
        let (&(t0, _), &(t1, _)) = (self.arrivals.front()?, self.arrivals.back()?);
        let bytes: usize = self.arrivals.iter().skip(1).map(|&(_, n)| n).sum();
        (t1 > t0).then(|| bytes as f64 / (t1 - t0))
    }

    /// The byte window that keeps `target_sec` of the delivery rate in
    /// flight, but never less than room for two of the largest recent
    /// messages: with only one in flight, a link that takes longer than a
    /// round trip to carry a frame sits idle for a round trip after each, and
    /// the rate measured that way is too low ever to make room for a second.
    /// Two and a half, in fact, since the next frame may be a little larger
    /// than any yet; three still won't fit.
    /// Before there is a rate, 1, which (the server always letting one frame
    /// go when none is in flight) means a frame at a time.
    pub fn window_bytes(&self, target_sec: f64) -> u32 {
        let Some(rate) = self.bytes_per_sec() else {
            return 1;
        };
        let largest = self.arrivals.iter().map(|&(_, n)| n).max().unwrap_or(0);
        (rate * target_sec)
            .round()
            .max(2.5 * largest as f64)
            .clamp(1.0, u32::MAX as f64) as u32
    }
}

#[cfg(test)]
mod tests;
