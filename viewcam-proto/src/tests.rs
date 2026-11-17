use std::path::PathBuf;

use super::*;

/// Every message kind, with planes whose bytes are distinct enough that a
/// misplaced byte shows. These are written to `tests/vectors/` and the Python
/// reference encoder is checked against the same files.
fn vectors() -> Vec<(&'static str, Vec<u8>)> {
    let ramp = |n: usize| (0..n).map(|i| (i * 37 + 11) as u8).collect::<Vec<u8>>();
    vec![
        (
            "frame_u16_with_saturation",
            Message::Frame {
                header: FrameHeader {
                    seq: 0x0102_0304_0506_0708,
                    width: 3,
                    height: 2,
                    data_dtype: DType::U16,
                    sat_dtype: Some(DType::U8),
                },
                data: ramp(3 * 2 * 2),
                saturation: Some(vec![0, 1, 0, 0, 0, 1]),
            }
            .encode(),
        ),
        (
            "frame_f32_no_saturation",
            Message::Frame {
                header: FrameHeader {
                    seq: 7,
                    width: 2,
                    height: 2,
                    data_dtype: DType::F32,
                    sat_dtype: None,
                },
                data: [1.5f32, -2.0, 0.0, 65535.0]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
                saturation: None,
            }
            .encode(),
        ),
        (
            "dark_f64",
            Message::Dark {
                header: DarkHeader {
                    width: 2,
                    height: 1,
                    dtype: DType::F64,
                },
                data: [1000.25f64, -0.5]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            }
            .encode(),
        ),
        ("clear_dark", Message::ClearDark.encode()),
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
    assert_eq!(frame[4], 1);
    assert_eq!(frame[5], 1);
    assert_eq!(&frame[8..16], &0x0102_0304_0506_0708u64.to_le_bytes());
    assert_eq!(&frame[16..20], &3u32.to_le_bytes());
    assert_eq!(&frame[20..24], &2u32.to_le_bytes());
    assert_eq!((frame[24], frame[25]), (3, 1));
    assert_eq!(frame.len(), FRAME_HEADER_LEN + 12 + 6);
}

/// Every vector reassembles to the message it came from, whatever the chunk
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
        .unwrap();
    assert!(matches!(header, Some(Header::Frame(h)) if h.seq == 0x0102_0304_0506_0708));
    assert_eq!(r.push(&bytes[FRAME_HEADER_LEN..]).unwrap(), None);
}

#[test]
fn frame_planes_split_correctly() {
    let mut r = StreamReader::new(DEFAULT_MAX_PIXELS);
    r.push(&vectors().remove(0).1).unwrap();
    let Message::Frame {
        data, saturation, ..
    } = r.finish().unwrap()
    else {
        panic!("not a frame");
    };
    assert_eq!(data.len(), 12);
    assert_eq!(saturation, Some(vec![0, 1, 0, 0, 0, 1]));
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

    let mut bad = frame.clone();
    bad[4] = 2;
    assert_eq!(
        Header::decode(&bad, DEFAULT_MAX_PIXELS),
        Err(ProtoError::UnsupportedVersion(2))
    );
    let mut bad = frame.clone();
    bad[5] = 9;
    assert_eq!(
        Header::decode(&bad, DEFAULT_MAX_PIXELS),
        Err(ProtoError::UnknownKind(9))
    );
    let mut bad = frame.clone();
    bad[24] = 11;
    assert_eq!(
        Header::decode(&bad, DEFAULT_MAX_PIXELS),
        Err(ProtoError::UnknownDtype(11))
    );

    // A credit belongs in a datagram, not on a stream.
    assert_eq!(
        Header::decode(&encode_credit(1), DEFAULT_MAX_PIXELS),
        Err(ProtoError::WrongChannel(0x81))
    );
}

#[test]
fn oversized_planes_are_refused_before_allocating() {
    let header = Header::Frame(FrameHeader {
        seq: 0,
        width: 100_000,
        height: 100_000,
        data_dtype: DType::F64,
        sat_dtype: None,
    })
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
        decode_credit(&Message::ClearDark.encode()),
        Err(ProtoError::WrongChannel(3))
    );
}

#[test]
fn dtypes_convert_little_endian() {
    assert_eq!(
        DType::U16.to_f64(&[0x34, 0x12, 0xff, 0xff]),
        vec![4660.0, 65535.0]
    );
    assert_eq!(DType::I16.to_f64(&[0xff, 0xff]), vec![-1.0]);
    assert_eq!(DType::F32.to_f64(&1.5f32.to_le_bytes()), vec![1.5]);
    for d in DType::ALL {
        assert_eq!(DType::from_code(d.code()), Ok(d));
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
