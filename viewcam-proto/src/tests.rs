use std::path::PathBuf;

use super::*;

/// Every codec, plus an empty frame and a credit. Payloads are fixed bytes,
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
        ("credit", encode_credit(0x1122_3344_5566_7788).to_vec()),
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

/// Every vector reassembles to the frame it came from, whatever the chunk
/// boundaries: QUIC delivers a stream in pieces of any size.
#[test]
fn reassembles_across_every_split() {
    for (name, bytes) in vectors() {
        if name == "credit" {
            continue;
        }
        let whole = {
            let mut r = StreamReader::new(DEFAULT_MAX_PIXELS);
            r.push(&bytes).unwrap();
            r.finish().unwrap()
        };
        assert_eq!(whole.encode(), bytes);
        for split in 0..=bytes.len() {
            let mut r = StreamReader::new(DEFAULT_MAX_PIXELS);
            r.push(&bytes[..split]).unwrap();
            r.push(&bytes[split..]).unwrap();
            assert_eq!(r.finish().unwrap(), whole, "{name} split at {split}");
        }
    }
}

#[test]
fn header_is_reported_once_and_before_the_payload() {
    let bytes = vectors().remove(0).1;
    let mut r = StreamReader::new(DEFAULT_MAX_PIXELS);
    assert_eq!(r.push(&bytes[..FRAME_HEADER_LEN - 1]).unwrap(), None);
    let header = r
        .push(&bytes[FRAME_HEADER_LEN - 1..FRAME_HEADER_LEN])
        .unwrap()
        .unwrap();
    assert_eq!(header.seq, 0x0102_0304_0506_0708);
    assert_eq!(header.decoded_len(), 12);
    assert_eq!(r.push(&bytes[FRAME_HEADER_LEN..]).unwrap(), None);
}

#[test]
fn empty_frames_mean_no_image() {
    let mut r = StreamReader::new(DEFAULT_MAX_PIXELS);
    r.push(&Frame::empty(3, DType::U16).encode()).unwrap();
    let frame = r.finish().unwrap();
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
fn malformed_streams_are_rejected() {
    let frame = vectors().remove(0).1;

    let mut r = StreamReader::new(DEFAULT_MAX_PIXELS);
    r.push(&frame[..frame.len() - 1]).unwrap();
    assert!(matches!(r.finish(), Err(ProtoError::Truncated { .. })));

    let mut r = StreamReader::new(DEFAULT_MAX_PIXELS);
    r.push(&frame).unwrap();
    r.push(&[0]).unwrap();
    assert!(matches!(r.finish(), Err(ProtoError::TrailingBytes { .. })));

    let mut r = StreamReader::new(DEFAULT_MAX_PIXELS);
    r.push(&frame[..10]).unwrap();
    assert_eq!(
        r.finish(),
        Err(ProtoError::Truncated {
            expected: FRAME_HEADER_LEN,
            got: 10
        })
    );

    // A bad magic is caught from the first wrong byte.
    assert_eq!(
        StreamReader::new(DEFAULT_MAX_PIXELS).push(b"VX"),
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

    // A credit belongs in a datagram, not on a stream.
    assert_eq!(
        FrameHeader::decode(&encode_credit(1), DEFAULT_MAX_PIXELS),
        Err(ProtoError::WrongChannel(0x81))
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
    assert!(matches!(
        StreamReader::new(DEFAULT_MAX_PIXELS).push(&header),
        Err(ProtoError::TooLarge { .. })
    ));
}

#[test]
fn credit_round_trips_and_rejects_other_kinds() {
    assert_eq!(decode_credit(&encode_credit(42)), Ok(42));
    assert!(decode_credit(&encode_credit(42)[..15]).is_err());
    assert_eq!(
        decode_credit(&Frame::empty(0, DType::U8).encode()),
        Err(ProtoError::WrongChannel(1))
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
fn client_state_tracks_credit_and_newest_frame() {
    let mut s = ClientState::new(6);
    assert_eq!(s.credit_limit(), 6);

    // Frames complete out of order: 3 first, then 2 (late, dropped), then 5.
    assert!(s.accept(3));
    assert_eq!(s.frame_stream_finished(), 7);
    assert!(s.is_stale(2));
    assert!(!s.accept(2));
    assert_eq!(s.frame_stream_finished(), 8);
    assert!(!s.is_stale(5));
    assert!(s.accept(5));
    assert!(!s.accept(5), "the same frame twice is not newer");

    // Abandoned and broken streams still return their credit.
    assert_eq!(s.frame_stream_finished(), 9);
    assert_eq!(s.frames_finished(), 3);
}
