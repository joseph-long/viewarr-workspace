//! The viewcam frame-streaming protocol, without I/O.
//!
//! [PROTOCOL.md](../PROTOCOL.md) is the specification; this crate is its
//! reference implementation. It turns bytes into messages and messages into
//! bytes, and tracks the client's side of the credit window and the
//! newest-frame-wins rule. Reading streams and sending datagrams is left to the
//! caller (the native client on `wtransport`, the browser client on the
//! WebTransport JS API), so the same logic, and the same tests, serve both.

use std::fmt;

pub const MAGIC: [u8; 4] = *b"VCAM";
pub const VERSION: u8 = 1;

/// Length of the prefix every message starts with.
pub const PREFIX_LEN: usize = 8;
pub const FRAME_HEADER_LEN: usize = 28;
pub const DARK_HEADER_LEN: usize = 20;
pub const CREDIT_LEN: usize = 16;

const KIND_FRAME: u8 = 1;
const KIND_DARK: u8 = 2;
const KIND_CLEAR_DARK: u8 = 3;
const KIND_CREDIT: u8 = 0x81;

/// Largest plane a client accepts by default, in pixels. Bounds what a corrupt
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
    /// A plane larger than the receiver's limit.
    TooLarge {
        pixels: u64,
        max: u64,
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
    /// A message that may not travel the way it arrived (e.g. a credit on a
    /// stream).
    WrongChannel(u8),
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtoError::BadMagic => write!(f, "not a viewcam message (bad magic)"),
            ProtoError::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            ProtoError::UnknownKind(k) => write!(f, "unknown message kind {k:#04x}"),
            ProtoError::UnknownDtype(d) => write!(f, "unknown dtype code {d}"),
            ProtoError::TooLarge { pixels, max } => {
                write!(f, "plane of {pixels} pixels exceeds the limit of {max}")
            }
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
// Dtypes
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

    /// Convert a little-endian plane to `f64`, the viewer's pixel type.
    /// Trailing bytes short of a whole element are ignored.
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

// =============================================================================
// Headers
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub data_dtype: DType,
    /// `None` when the frame carries no saturation plane.
    pub sat_dtype: Option<DType>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DarkHeader {
    pub width: u32,
    pub height: u32,
    pub dtype: DType,
}

/// The header of a server-to-client stream message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Header {
    Frame(FrameHeader),
    Dark(DarkHeader),
    ClearDark,
}

fn pixels(width: u32, height: u32) -> u64 {
    width as u64 * height as u64
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

impl Header {
    pub fn len(&self) -> usize {
        match self {
            Header::Frame(_) => FRAME_HEADER_LEN,
            Header::Dark(_) => DARK_HEADER_LEN,
            Header::ClearDark => PREFIX_LEN,
        }
    }

    /// Bytes that follow the header.
    pub fn payload_len(&self) -> usize {
        match self {
            Header::Frame(h) => {
                let n = pixels(h.width, h.height) as usize;
                n * h.data_dtype.size() + n * h.sat_dtype.map_or(0, DType::size)
            }
            Header::Dark(h) => pixels(h.width, h.height) as usize * h.dtype.size(),
            Header::ClearDark => 0,
        }
    }

    /// Header plus payload.
    pub fn message_len(&self) -> usize {
        self.len() + self.payload_len()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len());
        match self {
            Header::Frame(h) => {
                out.extend_from_slice(&prefix(KIND_FRAME));
                out.extend_from_slice(&h.seq.to_le_bytes());
                out.extend_from_slice(&h.width.to_le_bytes());
                out.extend_from_slice(&h.height.to_le_bytes());
                out.push(h.data_dtype.code());
                out.push(h.sat_dtype.map_or(0, DType::code));
                out.extend_from_slice(&[0, 0]);
            }
            Header::Dark(h) => {
                out.extend_from_slice(&prefix(KIND_DARK));
                out.extend_from_slice(&h.width.to_le_bytes());
                out.extend_from_slice(&h.height.to_le_bytes());
                out.push(h.dtype.code());
                out.extend_from_slice(&[0, 0, 0]);
            }
            Header::ClearDark => out.extend_from_slice(&prefix(KIND_CLEAR_DARK)),
        }
        out
    }

    /// Decode a header from the start of a stream. `Ok(None)` means more bytes
    /// are needed; `max_pixels` bounds the planes it will accept.
    pub fn decode(buf: &[u8], max_pixels: u64) -> Result<Option<Header>, ProtoError> {
        let Some(kind) = read_prefix(buf)? else {
            return Ok(None);
        };
        let check = |w: u32, h: u32| {
            let n = pixels(w, h);
            if n > max_pixels {
                Err(ProtoError::TooLarge {
                    pixels: n,
                    max: max_pixels,
                })
            } else {
                Ok(())
            }
        };
        match kind {
            KIND_FRAME => {
                if buf.len() < FRAME_HEADER_LEN {
                    return Ok(None);
                }
                let (width, height) = (read_u32(buf, 16), read_u32(buf, 20));
                check(width, height)?;
                let sat_dtype = match buf[25] {
                    0 => None,
                    code => Some(DType::from_code(code)?),
                };
                Ok(Some(Header::Frame(FrameHeader {
                    seq: read_u64(buf, 8),
                    width,
                    height,
                    data_dtype: DType::from_code(buf[24])?,
                    sat_dtype,
                })))
            }
            KIND_DARK => {
                if buf.len() < DARK_HEADER_LEN {
                    return Ok(None);
                }
                let (width, height) = (read_u32(buf, 8), read_u32(buf, 12));
                check(width, height)?;
                Ok(Some(Header::Dark(DarkHeader {
                    width,
                    height,
                    dtype: DType::from_code(buf[16])?,
                })))
            }
            KIND_CLEAR_DARK => Ok(Some(Header::ClearDark)),
            KIND_CREDIT => Err(ProtoError::WrongChannel(kind)),
            other => Err(ProtoError::UnknownKind(other)),
        }
    }
}

// =============================================================================
// Messages
// =============================================================================

/// A complete server-to-client message. Planes are kept as the raw
/// little-endian bytes that arrived; [`DType::to_f64`] converts them.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Frame {
        header: FrameHeader,
        data: Vec<u8>,
        /// Present exactly when `header.sat_dtype` is.
        saturation: Option<Vec<u8>>,
    },
    Dark {
        header: DarkHeader,
        data: Vec<u8>,
    },
    ClearDark,
}

impl Message {
    /// Encode a whole message, as a server writes it to its stream.
    ///
    /// Panics if a plane's length does not match the header, which is a bug
    /// in the caller rather than something to put on the wire.
    pub fn encode(&self) -> Vec<u8> {
        let header = self.header();
        let mut out = header.encode();
        out.reserve(header.payload_len());
        match self {
            Message::Frame {
                data, saturation, ..
            } => {
                out.extend_from_slice(data);
                if let Some(sat) = saturation {
                    out.extend_from_slice(sat);
                }
            }
            Message::Dark { data, .. } => out.extend_from_slice(data),
            Message::ClearDark => {}
        }
        assert_eq!(
            out.len(),
            header.message_len(),
            "plane length does not match header"
        );
        out
    }

    pub fn header(&self) -> Header {
        match self {
            Message::Frame { header, .. } => Header::Frame(*header),
            Message::Dark { header, .. } => Header::Dark(*header),
            Message::ClearDark => Header::ClearDark,
        }
    }

    /// Build a message from its header and the bytes that followed it, which
    /// must be exactly `header.payload_len()` long.
    fn from_parts(header: Header, mut payload: Vec<u8>) -> Message {
        match header {
            Header::Frame(h) => {
                let data_len = pixels(h.width, h.height) as usize * h.data_dtype.size();
                let saturation = h.sat_dtype.map(|_| payload.split_off(data_len));
                Message::Frame {
                    header: h,
                    data: payload,
                    saturation,
                }
            }
            Header::Dark(h) => Message::Dark {
                header: h,
                data: payload,
            },
            Header::ClearDark => Message::ClearDark,
        }
    }
}

/// Reassembles one server-to-client stream into a [`Message`].
///
/// Feed it chunks as they arrive with [`push`](Self::push); it reports the
/// header as soon as it is complete, so a client can abandon a stale frame
/// before the rest of it arrives. Call [`finish`](Self::finish) when the stream
/// ends.
pub struct StreamReader {
    max_pixels: u64,
    buf: Vec<u8>,
    header: Option<Header>,
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
    pub fn header(&self) -> Option<&Header> {
        self.header.as_ref()
    }

    /// Append a chunk. Returns the header the first time it becomes complete.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Option<Header>, ProtoError> {
        self.buf.extend_from_slice(chunk);
        if self.header.is_some() {
            return Ok(None);
        }
        let Some(header) = Header::decode(&self.buf, self.max_pixels)? else {
            return Ok(None);
        };
        // Now the full size is known, reserve it once instead of growing by
        // doubling through a multi-megabyte frame.
        self.buf
            .reserve(header.message_len().saturating_sub(self.buf.len()));
        self.header = Some(header);
        Ok(Some(header))
    }

    /// The stream ended: return the message, or why it was malformed.
    pub fn finish(self) -> Result<Message, ProtoError> {
        let got = self.buf.len();
        let Some(header) = self.header else {
            // Short of a full header: report the header length this kind
            // needs, or just the prefix if not even the kind arrived.
            let expected = match self.buf.get(5) {
                Some(&KIND_FRAME) => FRAME_HEADER_LEN,
                Some(&KIND_DARK) => DARK_HEADER_LEN,
                _ => PREFIX_LEN,
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
        let payload = buf.split_off(header.len());
        Ok(Message::from_parts(header, payload))
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
    match read_prefix(buf)? {
        Some(KIND_CREDIT) if buf.len() == CREDIT_LEN => Ok(read_u64(buf, 8)),
        Some(KIND_CREDIT) => Err(ProtoError::Truncated {
            expected: CREDIT_LEN,
            got: buf.len(),
        }),
        Some(kind) => Err(ProtoError::WrongChannel(kind)),
        None => Err(ProtoError::Truncated {
            expected: CREDIT_LEN,
            got: buf.len(),
        }),
    }
}

/// The client's side of flow control and newest-frame-wins, for one session.
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
