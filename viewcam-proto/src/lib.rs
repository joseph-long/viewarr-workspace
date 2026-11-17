//! The viewcam frame-streaming protocol, without I/O.
//!
//! [PROTOCOL.md](../PROTOCOL.md) is the specification; this crate is its
//! reference implementation. It turns bytes into frames and frames into
//! bytes, and tracks a client's side of the credit window and the
//! newest-frame-wins rule for one feed. Reading streams and sending datagrams
//! is left to the caller, so the same logic, and the same tests, serve every
//! client and transport.
//!
//! Payloads are carried as they arrived, still encoded; decoding the LZ4
//! codecs is the `viewcam-codec` crate's job, so this one stays
//! dependency-free.

use std::fmt;

pub const MAGIC: [u8; 4] = *b"VCAM";
pub const VERSION: u8 = 1;

/// Length of the prefix every message starts with.
pub const PREFIX_LEN: usize = 8;
pub const FRAME_HEADER_LEN: usize = 32;
pub const CREDIT_LEN: usize = 16;

const KIND_FRAME: u8 = 1;
const KIND_CREDIT: u8 = 0x81;

/// Largest image a client accepts by default, in pixels. Bounds what a corrupt
/// header can make it allocate.
pub const DEFAULT_MAX_PIXELS: u64 = 1 << 28;

/// How often a client re-sends its credit even when nothing changed, since
/// datagrams can be lost.
pub const CREDIT_RESEND_INTERVAL_MS: u64 = 100;

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
    /// The stream ended before the message was complete.
    Truncated {
        expected: usize,
        got: usize,
    },
    /// The stream carried bytes past the end of its message.
    TrailingBytes {
        expected: usize,
        got: usize,
    },
    /// A message that may not travel the way it arrived (a credit on a
    /// stream, a frame in a datagram).
    WrongChannel(u8),
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::BadMagic => write!(f, "not a viewcam message (bad magic)"),
            ProtoError::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            ProtoError::UnknownKind(k) => write!(f, "unknown message kind {k:#04x}"),
            ProtoError::UnknownDtype(d) => write!(f, "unknown dtype code {d}"),
            ProtoError::UnknownCodec(c) => write!(f, "unknown codec code {c}"),
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
                write!(f, "stream ended after {got} of {expected} bytes")
            }
            ProtoError::TrailingBytes { expected, got } => {
                write!(
                    f,
                    "stream carried {got} bytes for a {expected}-byte message"
                )
            }
            ProtoError::WrongChannel(k) => write!(f, "message kind {k:#04x} on the wrong channel"),
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
    /// A raw LZ4 block of the image.
    Lz4,
    /// A raw LZ4 block of the image's bytes shuffled by element.
    ShuffleLz4,
}

impl Codec {
    pub const ALL: [Codec; 3] = [Codec::Raw, Codec::Lz4, Codec::ShuffleLz4];

    pub fn code(self) -> u8 {
        match self {
            Codec::Raw => 0,
            Codec::Lz4 => 1,
            Codec::ShuffleLz4 => 2,
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
            Codec::Lz4 => "lz4",
            Codec::ShuffleLz4 => "shuffle-lz4",
        }
    }

    /// The largest payload this codec can produce for a `decoded`-byte image.
    pub fn max_encoded_len(self, decoded: u64) -> u64 {
        match self {
            Codec::Raw => decoded,
            // LZ4's worst case for incompressible input.
            Codec::Lz4 | Codec::ShuffleLz4 => decoded + decoded / 255 + 16,
        }
    }
}

// =============================================================================
// Frames
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub dtype: DType,
    pub codec: Codec,
    /// Payload length in bytes.
    pub encoded_len: u32,
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
            // one byte of LZ4 block.
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
        Ok(())
    }

    pub fn encode(&self) -> [u8; FRAME_HEADER_LEN] {
        let mut out = [0u8; FRAME_HEADER_LEN];
        out[..PREFIX_LEN].copy_from_slice(&prefix(KIND_FRAME));
        out[8..16].copy_from_slice(&self.seq.to_le_bytes());
        out[16..20].copy_from_slice(&self.width.to_le_bytes());
        out[20..24].copy_from_slice(&self.height.to_le_bytes());
        out[24] = self.dtype.code();
        out[25] = self.codec.code();
        out[28..32].copy_from_slice(&self.encoded_len.to_le_bytes());
        out
    }

    /// Decode and validate a header from the start of a stream. `Ok(None)`
    /// means more bytes are needed.
    pub fn decode(buf: &[u8], max_pixels: u64) -> Result<Option<FrameHeader>, ProtoError> {
        match read_prefix(buf)? {
            None => return Ok(None),
            Some(KIND_FRAME) => {}
            Some(KIND_CREDIT) => return Err(ProtoError::WrongChannel(KIND_CREDIT)),
            Some(kind) => return Err(ProtoError::UnknownKind(kind)),
        }
        if buf.len() < FRAME_HEADER_LEN {
            return Ok(None);
        }
        let header = FrameHeader {
            seq: read_u64(buf, 8),
            width: read_u32(buf, 16),
            height: read_u32(buf, 20),
            dtype: DType::from_code(buf[24])?,
            codec: Codec::from_code(buf[25])?,
            encoded_len: read_u32(buf, 28),
        };
        header.validate(max_pixels)?;
        Ok(Some(header))
    }
}

/// A complete frame, its payload still encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
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
        let encoded_len =
            u32::try_from(payload.len()).map_err(|_| ProtoError::BadEncodedLength {
                codec,
                encoded: payload.len() as u64,
                decoded: width as u64 * height as u64 * dtype.size() as u64,
            })?;
        let header = FrameHeader {
            seq,
            width,
            height,
            dtype,
            codec,
            encoded_len,
        };
        header.validate(DEFAULT_MAX_PIXELS)?;
        Ok(Frame { header, payload })
    }

    /// "No image on this feed".
    pub fn empty(seq: u64, dtype: DType) -> Frame {
        Frame::new(seq, 0, 0, dtype, Codec::Raw, Vec::new()).unwrap()
    }

    /// Encode the whole message, as a server writes it to its stream.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.header.message_len());
        out.extend_from_slice(&self.header.encode());
        out.extend_from_slice(&self.payload);
        out
    }
}

/// Reassembles one frame stream into a [`Frame`].
///
/// Feed it chunks as they arrive with [`push`](Self::push); it reports the
/// header as soon as it is complete, so a client can abandon a stale frame
/// before the rest of it arrives. Call [`finish`](Self::finish) when the
/// stream ends.
pub struct StreamReader {
    max_pixels: u64,
    buf: Vec<u8>,
    header: Option<FrameHeader>,
}

impl StreamReader {
    pub fn new(max_pixels: u64) -> Self {
        Self {
            max_pixels,
            buf: Vec::new(),
            header: None,
        }
    }

    /// The header, once enough of the stream has arrived.
    pub fn header(&self) -> Option<&FrameHeader> {
        self.header.as_ref()
    }

    /// Append a chunk. Returns the header the first time it becomes complete.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Option<FrameHeader>, ProtoError> {
        self.buf.extend_from_slice(chunk);
        if self.header.is_some() {
            return Ok(None);
        }
        let Some(header) = FrameHeader::decode(&self.buf, self.max_pixels)? else {
            return Ok(None);
        };
        // Now the full size is known, reserve it once instead of growing by
        // doubling through a multi-megabyte frame.
        self.buf
            .reserve(header.message_len().saturating_sub(self.buf.len()));
        self.header = Some(header);
        Ok(Some(header))
    }

    /// The stream ended: return the frame, or why it was malformed.
    pub fn finish(self) -> Result<Frame, ProtoError> {
        let got = self.buf.len();
        let Some(header) = self.header else {
            let expected = if got < PREFIX_LEN {
                PREFIX_LEN
            } else {
                FRAME_HEADER_LEN
            };
            return Err(ProtoError::Truncated { expected, got });
        };
        let expected = header.message_len();
        if got < expected {
            return Err(ProtoError::Truncated { expected, got });
        }
        if got > expected {
            return Err(ProtoError::TrailingBytes { expected, got });
        }
        let mut buf = self.buf;
        let payload = buf.split_off(FRAME_HEADER_LEN);
        Ok(Frame { header, payload })
    }
}

// =============================================================================
// Credit
// =============================================================================

pub fn encode_credit(limit: u64) -> [u8; CREDIT_LEN] {
    let mut out = [0u8; CREDIT_LEN];
    out[..PREFIX_LEN].copy_from_slice(&prefix(KIND_CREDIT));
    out[8..].copy_from_slice(&limit.to_le_bytes());
    out
}

pub fn decode_credit(buf: &[u8]) -> Result<u64, ProtoError> {
    let truncated = ProtoError::Truncated {
        expected: CREDIT_LEN,
        got: buf.len(),
    };
    match read_prefix(buf)? {
        Some(KIND_CREDIT) if buf.len() == CREDIT_LEN => Ok(read_u64(buf, 8)),
        Some(KIND_CREDIT) | None => Err(truncated),
        Some(kind) => Err(ProtoError::WrongChannel(kind)),
    }
}

/// A client's side of flow control and newest-frame-wins, for one feed.
#[derive(Clone, Debug)]
pub struct ClientState {
    window: u64,
    finished: u64,
    newest_shown: Option<u64>,
}

impl ClientState {
    pub fn new(window: u64) -> Self {
        Self {
            window: window.max(1),
            finished: 0,
            newest_shown: None,
        }
    }

    /// The credit limit to send: frames finished plus the window.
    pub fn credit_limit(&self) -> u64 {
        self.finished + self.window
    }

    /// Whether a frame with this `seq` is already superseded, so its stream
    /// should be abandoned without reading the rest.
    pub fn is_stale(&self, seq: u64) -> bool {
        self.newest_shown.is_some_and(|newest| seq <= newest)
    }

    /// A frame arrived complete: `true` if it should be shown (it is the
    /// newest yet), in which case it becomes the newest shown.
    pub fn accept(&mut self, seq: u64) -> bool {
        if self.is_stale(seq) {
            return false;
        }
        self.newest_shown = Some(seq);
        true
    }

    /// A frame stream ended, however it ended. Returns the new credit limit,
    /// to send straight away.
    pub fn frame_stream_finished(&mut self) -> u64 {
        self.finished += 1;
        self.credit_limit()
    }

    pub fn frames_finished(&self) -> u64 {
        self.finished
    }
}

#[cfg(test)]
mod tests;
