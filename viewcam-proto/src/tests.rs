use std::path::PathBuf;

use super::*;

/// Every codec, plus an empty frame and an ack. Payloads are fixed bytes,
/// not real LZ4 output, since this crate only frames them; that keeps the
/// Python reference able to reproduce them exactly. These are written to
/// `tests/vectors/` and `python/test_viewcam_proto.py` checks the same files.
fn vectors() -> Vec<(&'static str, Vec<u8>)> {
    let ramp = |n: usize| (0..n).map(|i| (i * 37 + 11) as u8).collect::<Vec<u8>>();
    let frame = |seq, w, h, dtype, codec, payload| {
        Frame::new(seq, w, h, dtype, codec, payload)
            .unwrap()
            .encode()
    };
    vec![
        (
            "frame_u16_raw",
            frame(
                0x0102_0304_0506_0708,
                3,
                2,
                DType::U16,
                Codec::Raw,
                ramp(12),
            ),
        ),
        (
            "frame_f32_lz4",
            frame(7, 2, 2, DType::F32, Codec::Lz4, ramp(9)),
        ),
        (
            "frame_u16_shuffle_lz4",
            frame(8, 3, 2, DType::U16, Codec::ShuffleLz4, ramp(5)),
        ),
        ("frame_empty", Frame::empty(9, DType::F64).encode()),
        (
            "ack",
            Ack {
                acked_frames: 0x1122_3344_5566_7788,
                acked_bytes: 0x0102_0304_0506_0708,
                window_frames: 6,
                window_bytes: 0x0040_0000,
            }
            .encode()
            .to_vec(),
        ),
    ]
}

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vectors")
}

/// Run with `VIEWCAM_REGEN_VECTORS=1` to rewrite the files after a deliberate
/// protocol change.
#[test]
fn encoding_matches_test_vectors() {
    let regen = std::env::var_os("VIEWCAM_REGEN_VECTORS").is_some();
    for (name, bytes) in vectors() {
        let path = vectors_dir().join(format!("{name}.bin"));
        if regen {
            std::fs::create_dir_all(vectors_dir()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        } else {
            let expected =
                std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(
                bytes,
                expected,
                "{name} no longer matches {}",
                path.display()
            );
        }
    }
}

#[test]
fn header_layout_is_as_specified() {
    let frame = vectors().remove(0).1;
    assert_eq!(&frame[..4], b"VCAM");
    assert_eq!((frame[4], frame[5]), (1, 1));
    assert_eq!(&frame[8..16], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(&frame[16..20], &3u32.to_le_bytes());
    assert_eq!(&frame[20..24], &2u32.to_le_bytes());
    assert_eq!((frame[24], frame[25]), (3, 0)); // u16, raw
    assert_eq!(&frame[28..32], &12u32.to_le_bytes());
    assert_eq!(frame.len(), FRAME_HEADER_LEN + 12);

    let shuffled = &vectors()[2].1;
    assert_eq!(shuffled[25], 2);
    assert_eq!(&shuffled[28..32], &5u32.to_le_bytes());
}

/// Every frame vector decodes to the frame it came from, and no shorter or
/// longer message does.
#[test]
fn frames_decode_from_whole_messages_only() {
    for (name, bytes) in vectors() {
        if name == "ack" {
            continue;
        }
        let frame = Frame::decode(&bytes, DEFAULT_MAX_PIXELS).unwrap();
        assert_eq!(frame.encode(), bytes, "{name}");
        for len in 0..bytes.len() {
            assert!(
                matches!(
                    Frame::decode(&bytes[..len], DEFAULT_MAX_PIXELS),
                    Err(ProtoError::Truncated { .. })
                ),
                "{name} cut to {len} bytes"
            );
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(matches!(
            Frame::decode(&longer, DEFAULT_MAX_PIXELS),
            Err(ProtoError::TrailingBytes { .. })
        ));
    }
}

#[test]
fn header_can_be_checked_before_the_payload() {
    let bytes = vectors().remove(0).1;
    assert_eq!(
        FrameHeader::decode(&bytes[..FRAME_HEADER_LEN - 1], DEFAULT_MAX_PIXELS),
        Ok(None)
    );
    let header = FrameHeader::decode(&bytes[..FRAME_HEADER_LEN], DEFAULT_MAX_PIXELS)
        .unwrap()
        .unwrap();
    assert_eq!(header.seq, 0x0102_0304_0506_0708);
    assert_eq!(header.decoded_len(), 12);
    assert_eq!(header.message_len(), bytes.len());
}

#[test]
fn empty_frames_mean_no_image() {
    let frame = Frame::decode(&Frame::empty(3, DType::U16).encode(), DEFAULT_MAX_PIXELS).unwrap();
    assert!(frame.header.is_empty());
    assert!(frame.payload.is_empty());
    // An empty image cannot carry a payload, whatever the codec.
    assert!(Frame::new(0, 0, 5, DType::U8, Codec::Lz4, vec![1]).is_err());
    assert!(Frame::new(0, 0, 0, DType::U8, Codec::Lz4, vec![]).is_ok());
}

#[test]
fn encoded_lengths_are_checked_per_codec() {
    let bad = |codec, len| {
        matches!(
            Frame::new(0, 4, 4, DType::U16, codec, vec![0; len]),
            Err(ProtoError::BadEncodedLength { .. })
        )
    };
    // Raw must be exactly the image: 4 x 4 x 2 = 32 bytes.
    assert!(!bad(Codec::Raw, 32));
    assert!(bad(Codec::Raw, 31));
    assert!(bad(Codec::Raw, 33));
    // LZ4 may be anything from one byte up to its worst case, 32 + 0 + 16.
    for codec in [Codec::Lz4, Codec::ShuffleLz4] {
        assert!(bad(codec, 0));
        assert!(!bad(codec, 1));
        assert!(!bad(codec, 48));
        assert!(bad(codec, 49));
    }
}

#[test]
fn malformed_messages_are_rejected() {
    let frame = vectors().remove(0).1;

    assert_eq!(
        Frame::decode(&frame[..10], DEFAULT_MAX_PIXELS),
        Err(ProtoError::Truncated {
            expected: FRAME_HEADER_LEN,
            got: 10
        })
    );

    // A bad magic is caught from the first wrong byte.
    assert_eq!(
        Frame::decode(b"VX", DEFAULT_MAX_PIXELS),
        Err(ProtoError::BadMagic)
    );

    let decode = |at: usize, value: u8| {
        let mut bad = frame.clone();
        bad[at] = value;
        FrameHeader::decode(&bad, DEFAULT_MAX_PIXELS)
    };
    assert_eq!(decode(4, 2), Err(ProtoError::UnsupportedVersion(2)));
    assert_eq!(decode(5, 9), Err(ProtoError::UnknownKind(9)));
    assert_eq!(decode(24, 11), Err(ProtoError::UnknownDtype(11)));
    assert_eq!(decode(25, 3), Err(ProtoError::UnknownCodec(3)));
    // A raw frame whose length field disagrees with its dimensions.
    assert!(matches!(
        decode(28, 13),
        Err(ProtoError::BadEncodedLength { .. })
    ));

    // Servers don't send ack.
    let ack = ClientState::new(1, 0).ack().encode();
    assert_eq!(
        Frame::decode(&ack, DEFAULT_MAX_PIXELS),
        Err(ProtoError::UnexpectedKind(0x81))
    );
}

#[test]
fn oversized_images_are_refused_before_allocating() {
    let header = FrameHeader {
        seq: 0,
        width: 100_000,
        height: 100_000,
        dtype: DType::U8,
        codec: Codec::Lz4,
        encoded_len: 1,
    }
    .encode();
    // Refused from the header alone, before any payload is looked at.
    assert!(matches!(
        FrameHeader::decode(&header, DEFAULT_MAX_PIXELS),
        Err(ProtoError::TooLarge { .. })
    ));
}

#[test]
fn ack_round_trips_and_rejects_other_kinds() {
    let ack = Ack {
        acked_frames: 3,
        acked_bytes: 4096,
        window_frames: 6,
        window_bytes: 0,
    };
    let bytes = ack.encode();
    assert_eq!(Ack::decode(&bytes), Ok(ack));
    assert_eq!(&bytes[24..28], &6u32.to_le_bytes());
    assert!(matches!(
        Ack::decode(&bytes[..ACK_LEN - 1]),
        Err(ProtoError::Truncated { .. })
    ));
    assert!(matches!(
        Ack::decode(&[&bytes[..], &[0]].concat()),
        Err(ProtoError::TrailingBytes { .. })
    ));
    // Clients don't send frames.
    assert_eq!(
        Ack::decode(&Frame::empty(0, DType::U8).encode()),
        Err(ProtoError::UnexpectedKind(1))
    );
}

#[test]
fn dtypes_and_codecs_round_trip_their_codes() {
    assert_eq!(
        DType::U16.to_f64(&[0x34, 0x12, 0xff, 0xff]),
        vec![4660.0, 65535.0]
    );
    assert_eq!(DType::I16.to_f64(&[0xff, 0xff]), vec![-1.0]);
    assert_eq!(DType::F32.to_f64(&1.5f32.to_le_bytes()), vec![1.5]);
    for d in DType::ALL {
        assert_eq!(DType::from_code(d.code()), Ok(d));
    }
    for c in Codec::ALL {
        assert_eq!(Codec::from_code(c.code()), Ok(c));
    }
}

#[test]
fn client_state_acknowledges_what_it_received() {
    let mut s = ClientState::new(6, 1 << 20);
    let opening = s.ack();
    assert_eq!((opening.acked_frames, opening.acked_bytes), (0, 0));
    assert_eq!((opening.window_frames, opening.window_bytes), (6, 1 << 20));

    let ack = s.frame_received(100);
    assert_eq!((ack.acked_frames, ack.acked_bytes), (1, 100));
    let ack = s.frame_received(40);
    assert_eq!((ack.acked_frames, ack.acked_bytes), (2, 140));

    // Pausing changes only the window; the counts carry on.
    let paused = s.set_window(0, 0);
    assert_eq!((paused.acked_frames, paused.window_frames), (2, 0));
}

#[test]
fn client_state_shows_only_the_newest_frame() {
    // Decoded off the main thread, frames can finish out of order:
    // 3 first, then 2 (late, dropped), then 5.
    let mut s = ClientState::new(6, 0);
    assert!(s.accept(3));
    assert!(s.is_stale(2));
    assert!(!s.accept(2));
    assert!(!s.is_stale(5));
    assert!(s.accept(5));
    assert!(!s.accept(5), "the same frame twice is not newer");
}
