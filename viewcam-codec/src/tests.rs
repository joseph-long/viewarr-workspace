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

/// The bit-shuffle written out bit by bit, as the module docs lay it out:
/// slow and plain, for the fast one to be checked against.
fn naive_bitshuffle(input: &[u8], s: usize) -> Vec<u8> {
    let n = input.len() / s;
    let groups = n / 8;
    let mut out = vec![0u8; input.len()];
    for i in 0..groups * 8 {
        for b in 0..s {
            for k in 0..8 {
                let bit = (input[i * s + b] >> k) & 1;
                out[(8 * b + k) * groups + i / 8] |= bit << (i % 8);
            }
        }
    }
    out[groups * 8 * s..].copy_from_slice(&input[groups * 8 * s..]);
    out
}

/// Deterministic bytes, no dependency.
fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

#[test]
fn the_transpose_moves_each_bit_across_the_diagonal() {
    for seed in 1..200u32 {
        let bytes = pseudo_random(8, seed);
        let x = u64::from_le_bytes(bytes.clone().try_into().unwrap());
        let y = transpose8(x);
        for r in 0..8 {
            for c in 0..8 {
                assert_eq!(
                    (y >> (8 * c + r)) & 1,
                    (x >> (8 * r + c)) & 1,
                    "x {x:#018x}"
                );
            }
        }
        assert_eq!(transpose8(y), x, "its own inverse");
    }
}

#[test]
fn bit_planes_are_laid_out_as_documented() {
    // Eight u16 elements, then three left over. Element i is i + 2^(9+i%3):
    // bits 0..3 count up, and one of bits 9..11 is set by turns.
    let input: Vec<u8> = (0..11u16)
        .flat_map(|i| (i + (1 << (9 + i % 3))).to_le_bytes())
        .collect();
    let mut out = vec![0u8; input.len()];
    bitshuffle_into(&input, &mut out, 2).unwrap();
    assert_eq!(out, naive_bitshuffle(&input, 2));
    // One group of eight: sixteen planes of one byte each.
    // Plane 0 is bit 0 of elements 0..8: 0,1,0,1,... from the lowest bit up.
    assert_eq!(out[0], 0b1010_1010);
    // Plane 1 is bit 1: elements 2, 3, 6, 7.
    assert_eq!(out[1], 0b1100_1100);
    // Plane 9 is bit 1 of the high byte, bit 9 overall: elements 0, 3, 6.
    assert_eq!(out[9], 0b0100_1001);
    // Planes 12..16, bits 12..16, are empty.
    assert_eq!(&out[12..16], &[0; 4]);
    // The three left over follow as they were.
    assert_eq!(&out[16..], &input[16..]);
}

#[test]
fn the_bit_shuffle_round_trips_for_every_element_size_and_length() {
    for s in [1, 2, 4, 8] {
        for n in [0, 1, 7, 8, 9, 15, 16, 17, 64, 100, 1000] {
            let data = pseudo_random(n * s, (n * 31 + s) as u32);
            let mut shuffled = vec![0; data.len()];
            let mut back = vec![0; data.len()];
            bitshuffle_into(&data, &mut shuffled, s).unwrap();
            assert_eq!(shuffled, naive_bitshuffle(&data, s), "{n} of {s} bytes");
            bitunshuffle_into(&shuffled, &mut back, s).unwrap();
            assert_eq!(back, data, "{n} of {s} bytes");
        }
    }
}

/// What `encode` makes, built from the dev-dependency so that the tests
/// needn't enable the `encode-zstd` feature.
fn zstd_payload(plane: &[u8], element_size: usize) -> Vec<u8> {
    zstd::bulk::compress(&bitshuffle(plane, element_size).unwrap(), ZSTD_LEVEL).unwrap()
}

#[cfg(feature = "encode-zstd")]
#[test]
fn encode_is_bitshuffle_then_zstd() {
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

    // And a mask, one bit of each byte used: smaller than the bits packed
    // eight to a byte, and than zstd makes of it alone.
    let mask: Vec<u8> = plane.chunks_exact(2).map(|p| (p[0] > 250) as u8).collect();
    let encoded = zstd_payload(&mask, 1).len();
    let plain = zstd::bulk::compress(&mask, ZSTD_LEVEL).unwrap().len();
    assert!(encoded < mask.len() / 8, "{encoded} bytes");
    assert!(encoded < plain, "{encoded} bytes vs plain {plain}");
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

/// Planes encoded here, `<name>.raw` beside `<name>.e<element size>.zst`,
/// for the Python extension's and the WASM module's tests to check that
/// they run this same code: that they decode these, and (Python) encode the
/// raw planes to these exact bytes. Run with `VIEWCAM_REGEN_VECTORS=1` to
/// rewrite them after a deliberate change to the format.
#[test]
fn encoded_vectors_are_current() {
    let camera = camera_plane(32, 24);
    let values = || {
        camera
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
    };
    let planes: [(&str, usize, Vec<u8>); 5] = [
        ("camera_u16", 2, camera.clone()),
        ("mask_u8", 1, values().map(|v| (v > 1150) as u8).collect()),
        (
            "float_f32",
            4,
            values()
                .flat_map(|v| (v as f32 / 7.0).to_le_bytes())
                .collect(),
        ),
        (
            "signed_i32",
            4,
            values()
                .flat_map(|v| (v as i32 - 1200).to_le_bytes())
                .collect(),
        ),
        (
            "dark_f64",
            8,
            values()
                .flat_map(|v| (v as f64 / 3.0).to_le_bytes())
                .collect(),
        ),
    ];
    let regen = std::env::var_os("VIEWCAM_REGEN_VECTORS").is_some();
    if regen {
        std::fs::create_dir_all(vectors_dir()).unwrap();
    }
    for (name, size, plane) in planes {
        let encoded = zstd_payload(&plane, size);
        assert_eq!(
            decode(&encoded, size, plane.len()).unwrap(),
            plane,
            "{name}"
        );
        let raw_path = vectors_dir().join(format!("{name}.raw"));
        let zst_path = vectors_dir().join(format!("{name}.e{size}.zst"));
        if regen {
            std::fs::write(&raw_path, &plane).unwrap();
            std::fs::write(&zst_path, &encoded).unwrap();
        } else {
            assert_eq!(std::fs::read(&raw_path).unwrap(), plane, "{name}");
            assert_eq!(std::fs::read(&zst_path).unwrap(), encoded, "{name}");
        }
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

#[test]
fn the_threshold_falls_between_the_levels_code_and_the_one_below() {
    let level = 16383.0;
    let threshold = CAMERA.threshold(level);
    let code = CAMERA.code(level) as i64;
    assert!(threshold < CAMERA.value(code));
    assert!(threshold > CAMERA.value(code - 1));
    // Everything at or above the level comes back at or above the threshold.
    for value in [level, level + 1.0, level + 500.0] {
        assert!(
            CAMERA.value(CAMERA.code(value) as i64) >= threshold,
            "{value}"
        );
    }
}
