//! Turning a FRAME message into pixels the widget can show.
//!
//! The same steps as the browser's decode worker: check the header,
//! decompress (`shuffle-zstd`), and turn a quantized frame's codes back into
//! values; then widen to `f64`, which is what `ViewcamWidget` takes.

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use viewcam::camera::Frame;
use viewcam_codec::Quantizer;
use viewcam_proto::{Codec, FrameHeader, Quantization, DEFAULT_MAX_PIXELS, FRAME_HEADER_LEN};

/// A decoded frame, with what the status bar and the throughput report want
/// to know about it.
pub struct DecodedFrame {
    pub seq: u64,
    pub frame: Frame,
    /// The dtype as shown: "f32" for a quantized frame, whose codes decode to
    /// `f32` values, as the browser reports it.
    pub dtype: &'static str,
    pub quantization: Option<Quantization>,
    /// When the image was acquired, in seconds since the Unix epoch, if the
    /// server said.
    pub acquired: Option<f64>,
    /// Length of the whole message, and of its image once decoded.
    pub message_len: usize,
    pub decoded_len: usize,
    pub decode_time: Duration,
}

pub enum Decoded {
    Frame(DecodedFrame),
    /// The feed has no image.
    Empty { seq: u64 },
}

/// Decodes one feed's frames, reusing its buffers from one frame to the next.
#[derive(Default)]
pub struct Decoder {
    /// The shuffled intermediate zstd decodes into.
    scratch: Vec<u8>,
    /// The decompressed plane.
    plane: Vec<u8>,
    /// A quantized frame's values.
    values: Vec<f32>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decode(&mut self, message: &[u8]) -> Result<Decoded> {
        let started = Instant::now();
        let header = FrameHeader::decode(message, DEFAULT_MAX_PIXELS)?
            .context("a message too short for a frame header")?;
        if message.len() != header.message_len() {
            bail!(
                "a frame message of {} bytes, but its header says {}",
                message.len(),
                header.message_len()
            );
        }
        if header.is_empty() {
            return Ok(Decoded::Empty { seq: header.seq });
        }
        let payload = &message[FRAME_HEADER_LEN..];
        let element_size = header.dtype.size();
        let decoded_len = header.decoded_len();

        let plane: &[u8] = match header.codec {
            Codec::Raw => payload,
            Codec::ShuffleZstd => {
                self.plane.resize(decoded_len, 0);
                viewcam_codec::decode_into(
                    payload,
                    element_size,
                    &mut self.scratch,
                    &mut self.plane,
                )?;
                &self.plane
            }
        };

        let (pixels, dtype) = match header.quantization {
            Some(q) => {
                self.values.resize(header.pixels() as usize, 0.0);
                quantizer(&q).dequantize_into(plane, element_size, q.code_offset, &mut self.values)?;
                (self.values.iter().map(|&v| v as f64).collect(), "f32")
            }
            None => (header.dtype.to_f64(plane), header.dtype.name()),
        };
        let (is_integer, value_decimals) = viewarr::buffer::dtype_meta(dtype);
        Ok(Decoded::Frame(DecodedFrame {
            seq: header.seq,
            frame: Frame {
                pixels,
                width: header.width,
                height: header.height,
                is_integer,
                value_decimals,
            },
            dtype,
            quantization: header.quantization,
            acquired: (header.acq_time_ns != 0).then(|| header.acq_time_ns as f64 / 1e9),
            message_len: message.len(),
            decoded_len,
            decode_time: started.elapsed(),
        }))
    }
}

fn quantizer(q: &Quantization) -> Quantizer {
    Quantizer {
        bias: q.bias,
        read_noise: q.read_noise,
        noise_slope: q.noise_slope,
        step: q.step,
    }
}

/// A quantized frame's values are rounded, so a saturated pixel can come
/// back a little under the saturation level: the threshold to compare with
/// is the one between the level's code and the one below it. As
/// `quantizedThreshold` in the browser client.
pub fn quantized_threshold(level: f64, q: &Quantization) -> f64 {
    let quantizer = quantizer(q);
    let code = (quantizer.forward(level) / q.step).round();
    (quantizer.inverse(code * q.step) + quantizer.inverse((code - 1.0) * q.step)) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewcam_proto::{DType, Frame as WireFrame};

    #[test]
    fn a_raw_frame_decodes_to_its_values() {
        let pixels: Vec<u8> = [1u16, 2, 3, 65535]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let message = WireFrame::new(7, 2, 2, DType::U16, Codec::Raw, pixels)
            .unwrap()
            .acquired_at(1_800_000_000_500_000_000)
            .encode();
        let Decoded::Frame(f) = Decoder::new().decode(&message).unwrap() else {
            panic!("expected a frame");
        };
        assert_eq!(f.seq, 7);
        assert_eq!(f.frame.pixels, vec![1.0, 2.0, 3.0, 65535.0]);
        assert_eq!((f.frame.width, f.frame.height), (2, 2));
        assert_eq!(f.dtype, "u16");
        assert!(f.frame.is_integer);
        assert_eq!(f.acquired, Some(1_800_000_000.5));
    }

    #[test]
    fn an_empty_frame_says_so() {
        let message = WireFrame::empty(3, DType::U8).encode();
        assert!(matches!(
            Decoder::new().decode(&message).unwrap(),
            Decoded::Empty { seq: 3 }
        ));
    }

    #[test]
    fn a_short_message_is_an_error() {
        let message = WireFrame::new(1, 1, 1, DType::U16, Codec::Raw, vec![0, 0])
            .unwrap()
            .encode();
        assert!(Decoder::new().decode(&message[..message.len() - 1]).is_err());
    }

    #[test]
    fn the_threshold_falls_between_the_levels_code_and_the_one_below() {
        let q = Quantization {
            code_offset: 0,
            bias: 100.0,
            read_noise: 4.0,
            noise_slope: 1.0,
            step: 0.25,
        };
        let level = 16383.0;
        let t = quantized_threshold(level, &q);
        let quantizer = quantizer(&q);
        let code = (quantizer.forward(level) / q.step).round();
        assert!(t < quantizer.inverse(code * q.step));
        assert!(t > quantizer.inverse((code - 1.0) * q.step));
    }
}
