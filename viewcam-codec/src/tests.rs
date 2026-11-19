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

/// What `encode` makes, built from the dev-dependency so that the tests
/// needn't enable the `encode-zstd` feature.
fn zstd_payload(plane: &[u8], element_size: usize) -> Vec<u8> {
    zstd::bulk::compress(&shuffle(plane, element_size).unwrap(), ZSTD_LEVEL).unwrap()
}

#[cfg(feature = "encode-zstd")]
#[test]
fn encode_is_shuffle_then_zstd() {
    let plane = camera_plane(64, 64);
    assert_eq!(encode(&plane, 2).unwrap(), zstd_payload(&plane, 2));
}

#[test]
fn codec_round_trips_and_compresses_camera_data() {
    let plane = camera_plane(256, 256);
    let encoded = zstd_payload(&plane, 2);
    assert_eq!(decode(&encoded, 2, plane.len()).unwrap(), plane);

    // The point of shuffling: it should beat zstd alone on this data.
    let plain = zstd::bulk::compress(&plane, ZSTD_LEVEL).unwrap().len();
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
    let encoded = zstd_payload(&plane, 2);
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
    let encoded = zstd_payload(&plane, 2);

    // Expecting more than the frame holds.
    assert!(matches!(
        decode(&encoded, 2, plane.len() + 2),
        Err(CodecError::ShortOutput { .. })
    ));
    // Expecting less: the frame overflows the output.
    assert!(matches!(
        decode(&encoded, 2, plane.len() - 2),
        Err(CodecError::Zstd(_))
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

/// Planes the independent reference (numpy + the C libzstd, through the
/// `zstandard` package, in `python/tests/`) encoded decode to the raw planes
/// stored beside them. `<name>.e<size>.zst` holds the encoded plane,
/// `<name>.raw` the expected bytes.
#[test]
fn decodes_python_encoded_vectors() {
    let mut seen = 0;
    for entry in std::fs::read_dir(vectors_dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let Some(stem) = name.strip_suffix(".zst") else {
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

/// Writes a plane encoded here for the Python test to decode. Run with
/// `VIEWCAM_REGEN_VECTORS=1`.
#[test]
fn rust_encoded_vectors_are_current() {
    let plane = camera_plane(32, 24);
    let encoded = zstd_payload(&plane, 2);
    let raw_path = vectors_dir().join("rs_camera_u16.raw");
    let zst_path = vectors_dir().join("rs_camera_u16.e2.zst");
    if std::env::var_os("VIEWCAM_REGEN_VECTORS").is_some() {
        std::fs::create_dir_all(vectors_dir()).unwrap();
        std::fs::write(&raw_path, &plane).unwrap();
        std::fs::write(&zst_path, &encoded).unwrap();
    } else {
        assert_eq!(std::fs::read(&raw_path).unwrap(), plane);
        assert_eq!(std::fs::read(&zst_path).unwrap(), encoded);
    }
}

/// Read noise 14, with photon-like noise above a bias of 432: an EMCCD wavefront sensor's.
const CAMERA: Quantizer = Quantizer {
    bias: 432.0,
    read_noise: 14.0,
    noise_slope: 8.0,
    step: 0.25,
};

#[test]
fn quantizing_errs_by_at_most_half_a_step_of_the_local_noise() {
    for value in [300.0, 420.0, 432.0, 433.0, 450.0, 600.0, 2000.0, 16383.0] {
        let back = CAMERA.value(CAMERA.code(value) as i64);
        let noise = (14.0f64.powi(2) + 8.0 * (value - 432.0f64).max(0.0)).sqrt();
        // Half a step of the noise, give or take the curvature across a step.
        assert!(
            (back - value).abs() <= 0.5 * 0.25 * noise * 1.05,
            "{value} came back as {back}"
        );
    }
    // Fine at the faint end, coarse only where the noise is large.
    let step_at =
        |v: f64| CAMERA.value(CAMERA.code(v) as i64 + 1) - CAMERA.value(CAMERA.code(v) as i64);
    assert!(step_at(432.0) < 4.0);
    assert!(step_at(16000.0) > 40.0);
}

#[test]
fn the_transform_is_smooth_through_the_bias() {
    let below = CAMERA.forward(431.999);
    let above = CAMERA.forward(432.001);
    assert!((above - below - 0.002 / 14.0).abs() < 1e-6);
    for y in [-3.0, -0.1, 0.0, 0.1, 3.0, 80.0] {
        assert!((CAMERA.forward(CAMERA.inverse(y)) - y).abs() < 1e-9, "{y}");
    }
}

#[test]
fn without_a_slope_quantizing_is_linear() {
    let linear = Quantizer {
        bias: 0.0,
        read_noise: 0.04,
        noise_slope: 0.0,
        step: 0.25,
    };
    assert_eq!(linear.code(1.0), 100.0);
    assert_eq!(linear.code(-1.0), -100.0);
    assert!((linear.value(100) - 1.0).abs() < 1e-12);
}

#[test]
fn codes_are_stored_in_the_fewest_bytes_that_hold_them() {
    let values: Vec<f64> = (0..100).map(|i| 400.0 + i as f64).collect();
    let q = CAMERA.quantize(&values).unwrap();
    assert_eq!(q.element_size, 1);
    let mut out = vec![0f32; values.len()];
    CAMERA
        .dequantize_into(&q.stored, q.element_size, q.code_offset, &mut out)
        .unwrap();
    for (v, o) in values.iter().zip(&out) {
        let noise = (14.0f64.powi(2) + 8.0 * (v - 432.0f64).max(0.0)).sqrt();
        assert!(
            (*o as f64 - v).abs() <= 0.5 * 0.25 * noise * 1.05,
            "{v} -> {o}"
        );
    }
    // The full 14-bit range at this noise needs two bytes.
    assert_eq!(CAMERA.quantize(&[0.0, 16383.0]).unwrap().element_size, 2);
}

#[test]
fn quantizer_inputs_are_checked() {
    assert_eq!(CAMERA.quantize(&[f64::NAN]), Err(CodecError::NotFinite));
    let bad = Quantizer {
        read_noise: 0.0,
        ..CAMERA
    };
    assert_eq!(bad.quantize(&[1.0]), Err(CodecError::BadQuantizer));
    let bad = Quantizer {
        noise_slope: -1.0,
        ..CAMERA
    };
    assert_eq!(bad.check(), Err(CodecError::BadQuantizer));
    let mut out = [0f32; 2];
    assert_eq!(
        CAMERA.dequantize_into(&[0; 6], 3, 0, &mut out),
        Err(CodecError::UnsupportedCodeSize(3))
    );
    assert!(matches!(
        CAMERA.dequantize_into(&[0; 6], 2, 0, &mut out),
        Err(CodecError::ShortOutput { .. })
    ));
}
