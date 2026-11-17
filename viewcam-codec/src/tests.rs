use std::path::PathBuf;

use super::*;

/// A camera-like `u16` plane: a smooth gradient plus a little noise, so the
/// high bytes are predictable and the low bytes are not.
fn camera_plane(width: usize, height: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_u32;
    let mut noise = move || {
        // xorshift: deterministic, no dependency
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state % 64
    };
    (0..width * height)
        .flat_map(|i| {
            let (x, y) = (i % width, i / width);
            let v = 1000 + 4 * y as u32 + 2 * x as u32 + noise();
            (v as u16).to_le_bytes()
        })
        .collect()
}

#[test]
fn shuffle_layout_matches_blosc() {
    // Three u16 elements 0x0201, 0x0403, 0x0605: low bytes first, then high.
    let input = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
    let mut out = [0u8; 6];
    shuffle_into(&input, &mut out, 2).unwrap();
    assert_eq!(out, [0x01, 0x03, 0x05, 0x02, 0x04, 0x06]);

    // Two 4-byte elements.
    let input = [1, 2, 3, 4, 5, 6, 7, 8];
    let mut out = [0u8; 8];
    shuffle_into(&input, &mut out, 4).unwrap();
    assert_eq!(out, [1, 5, 2, 6, 3, 7, 4, 8]);
}

#[test]
fn shuffle_round_trips_for_every_element_size() {
    let data: Vec<u8> = (0..240u32).map(|i| (i * 31 % 251) as u8).collect();
    for size in [1, 2, 4, 8] {
        let mut shuffled = vec![0; data.len()];
        let mut back = vec![0; data.len()];
        shuffle_into(&data, &mut shuffled, size).unwrap();
        unshuffle_into(&shuffled, &mut back, size).unwrap();
        assert_eq!(back, data, "element size {size}");
    }
}

#[test]
fn codec_round_trips_and_compresses_camera_data() {
    let plane = camera_plane(256, 256);
    let encoded = encode(&plane, 2).unwrap();
    assert_eq!(decode(&encoded, 2, plane.len()).unwrap(), plane);

    // The point of shuffling: it should beat plain LZ4 on this data.
    let plain = lz4_flex::block::compress(&plane).len();
    assert!(
        encoded.len() < plain,
        "shuffled {} bytes vs plain {plain}",
        encoded.len()
    );
}

#[test]
fn scratch_is_reused_across_frames() {
    let mut scratch = Vec::new();
    let plane = camera_plane(64, 64);
    let encoded = encode(&plane, 2).unwrap();
    let mut out = vec![0; plane.len()];
    decode_into(&encoded, 2, &mut scratch, &mut out).unwrap();
    let capacity = scratch.capacity();
    decode_into(&encoded, 2, &mut scratch, &mut out).unwrap();
    assert_eq!(scratch.capacity(), capacity);
    assert_eq!(out, plane);
}

#[test]
fn wrong_lengths_and_corruption_are_errors() {
    let plane = camera_plane(16, 16);
    let encoded = encode(&plane, 2).unwrap();

    // Expecting more than the block holds.
    assert!(matches!(
        decode(&encoded, 2, plane.len() + 2),
        Err(CodecError::ShortOutput { .. })
    ));
    // Expecting less: the block overflows the output.
    assert!(matches!(
        decode(&encoded, 2, plane.len() - 2),
        Err(CodecError::Corrupt(_))
    ));
    // Not a whole number of elements.
    assert!(matches!(
        decode(&encoded, 2, plane.len() - 1),
        Err(CodecError::PartialElement { .. })
    ));
    assert_eq!(
        decode(&encoded, 0, plane.len()),
        Err(CodecError::ZeroElementSize)
    );
    // Truncated input never panics.
    for cut in 0..encoded.len() {
        assert!(decode(&encoded[..cut], 2, plane.len()).is_err());
    }
}

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vectors")
}

/// Planes the Python reference (`python/viewcam_codec.py`) encoded decode to
/// the raw planes stored beside them. `<name>.e<size>.lz4` holds the encoded
/// plane, `<name>.raw` the expected bytes.
#[test]
fn decodes_python_encoded_vectors() {
    let mut seen = 0;
    for entry in std::fs::read_dir(vectors_dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let Some(stem) = name.strip_suffix(".lz4") else {
            continue;
        };
        if !stem.starts_with("py_") {
            continue;
        }
        let (base, size) = stem.rsplit_once(".e").unwrap();
        let size: usize = size.parse().unwrap();
        let raw = std::fs::read(vectors_dir().join(format!("{base}.raw"))).unwrap();
        let encoded = std::fs::read(&path).unwrap();
        assert_eq!(decode(&encoded, size, raw.len()).unwrap(), raw, "{name}");
        seen += 1;
    }
    assert!(
        seen >= 3,
        "expected Python vectors in {}",
        vectors_dir().display()
    );
}

/// Writes planes encoded here for the Python test to decode. Run with
/// `VIEWCAM_REGEN_VECTORS=1`.
#[test]
fn rust_encoded_vectors_are_current() {
    let plane = camera_plane(32, 24);
    let encoded = encode(&plane, 2).unwrap();
    let raw_path = vectors_dir().join("rs_camera_u16.raw");
    let lz4_path = vectors_dir().join("rs_camera_u16.e2.lz4");
    if std::env::var_os("VIEWCAM_REGEN_VECTORS").is_some() {
        std::fs::create_dir_all(vectors_dir()).unwrap();
        std::fs::write(&raw_path, &plane).unwrap();
        std::fs::write(&lz4_path, &encoded).unwrap();
    } else {
        assert_eq!(std::fs::read(&raw_path).unwrap(), plane);
        assert_eq!(std::fs::read(&lz4_path).unwrap(), encoded);
    }
}
