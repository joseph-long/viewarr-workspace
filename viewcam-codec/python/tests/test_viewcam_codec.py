"""Test the Python extension against an independent reference: numpy for the
shuffle and the C libzstd (through the ``zstandard`` package) for compression.

    maturin develop -m viewcam-codec/python/Cargo.toml
    python -m pytest viewcam-codec/python/tests

The reference-encoded planes are also stored in ../../tests/vectors/ for the
Rust and WASM tests; set VIEWCAM_REGEN_VECTORS=1 to rewrite them.
"""

import os
import threading
import unittest
from pathlib import Path

import numpy as np
import zstandard

import viewcam_codec as vc

VECTORS = Path(__file__).resolve().parents[2] / "tests" / "vectors"


def shuffled(plane):
    return plane.view(np.uint8).reshape(-1, plane.itemsize).T.tobytes()


def reference_encode(plane):
    return zstandard.ZstdCompressor(level=1).compress(shuffled(plane))


def reference_decode(encoded, plane_like):
    data = zstandard.ZstdDecompressor().decompress(encoded, max_output_size=plane_like.nbytes)
    n = plane_like.size
    data = np.frombuffer(data, np.uint8).reshape(plane_like.itemsize, n).T.tobytes()
    return np.frombuffer(data, plane_like.dtype).reshape(plane_like.shape)


def planes():
    rng = np.random.default_rng(1)
    y, x = np.mgrid[0:24, 0:32]
    camera = (1000 + 4 * y + 2 * x + rng.integers(0, 64, size=y.shape)).astype("<u2")
    return {
        "py_camera_u16": camera,
        "py_mask_u8": (camera > 1150).astype(np.uint8),
        "py_float_f32": (camera / 7.0).astype("<f4"),
        "py_signed_i32": (camera.astype("<i4") - 1200),
        "py_dark_f64": (camera / 3.0).astype("<f8"),
    }


class TestAgainstReference(unittest.TestCase):
    def test_round_trip(self):
        for name, plane in planes().items():
            back = vc.decode(vc.encode(plane), plane.dtype, plane.shape)
            np.testing.assert_array_equal(back, plane, err_msg=name)

    def test_reference_decodes_ours(self):
        # Our payloads are standard zstd frames of the shuffled bytes.
        for name, plane in planes().items():
            np.testing.assert_array_equal(reference_decode(vc.encode(plane), plane), plane, err_msg=name)

    def test_we_decode_the_reference(self):
        for name, plane in planes().items():
            back = vc.decode(reference_encode(plane), plane.dtype, plane.shape)
            np.testing.assert_array_equal(back, plane, err_msg=name)

    def test_shuffle_layout(self):
        # Low bytes first, then high bytes, as in the Rust test.
        plane = np.array([0x0201, 0x0403, 0x0605], "<u2")
        data = zstandard.ZstdDecompressor().decompress(vc.encode(plane))
        self.assertEqual(data, bytes([1, 3, 5, 2, 4, 6]))


class TestInputs(unittest.TestCase):
    def test_strided_views_are_made_contiguous(self):
        plane = planes()["py_camera_u16"]
        view = plane[:, ::2]
        back = vc.decode(vc.encode(view), view.dtype, view.shape)
        np.testing.assert_array_equal(back, view)

    def test_big_endian_is_refused(self):
        with self.assertRaises(ValueError):
            vc.encode(planes()["py_camera_u16"].astype(">u2"))

    def test_plain_buffers_work(self):
        data = bytes(range(200))
        self.assertEqual(zstandard.ZstdDecompressor().decompress(vc.encode(data)), data)

    def test_corrupt_input_raises(self):
        with self.assertRaises(ValueError):
            vc.decode(b"\x01\x02\x03", "<u2", (8,))
        encoded = vc.encode(planes()["py_camera_u16"])
        with self.assertRaises(ValueError):
            vc.decode(encoded, "<u2", (24, 33))
        with self.assertRaises(ValueError):
            vc.decode(encoded[:-3], "<u2", (24, 32))

    def test_threads_encode_concurrently(self):
        # Mostly a check that releasing the GIL does not break anything.
        plane = np.tile(planes()["py_camera_u16"], (20, 20))
        results = [None] * 4

        def work(i):
            results[i] = vc.encode(plane)

        threads = [threading.Thread(target=work, args=(i,)) for i in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertTrue(all(r == results[0] for r in results))


# A wavefront sensor's noise: read noise 14 ADU above a bias of 432, and
# photon noise, amplified, above that.
MODEL = dict(bias=432.0, read_noise=14.0, noise_slope=8.0, step=0.25)


def model_noise(values):
    return np.sqrt(MODEL["read_noise"] ** 2 + MODEL["noise_slope"] * np.maximum(values - MODEL["bias"], 0))


class TestQuantize(unittest.TestCase):
    def test_error_is_within_half_a_step_of_the_local_noise(self):
        values = np.linspace(300, 16383, 5000)
        codes, offset = vc.quantize(values, **MODEL)
        back = vc.dequantize(codes, offset, **MODEL)
        # float32 output, and the curvature across a step, allow a little over half.
        self.assertTrue(np.all(np.abs(back - values) <= 0.5 * 0.25 * model_noise(values) * 1.05 + 1e-3))

    def test_codes_are_as_small_as_they_can_be(self):
        codes, offset = vc.quantize(np.full((3, 4), 432.0) + np.arange(12).reshape(3, 4), **MODEL)
        self.assertEqual((codes.dtype, codes.shape, codes.min()), (np.uint8, (3, 4), 0))
        codes, _ = vc.quantize(np.array([0.0, 16383.0]), **MODEL)
        self.assertEqual(codes.dtype, np.uint16)

    def test_any_numeric_image(self):
        image = (432 + np.arange(20) * 37).astype("<u2").reshape(4, 5)
        codes, offset = vc.quantize(image, **MODEL)
        np.testing.assert_allclose(
            vc.dequantize(codes, offset, **MODEL), image, atol=0.5 * 0.25 * model_noise(image).max() * 1.05)

    def test_bad_input_raises(self):
        with self.assertRaises(ValueError):
            vc.quantize(np.array([1.0, np.nan]), **MODEL)
        with self.assertRaises(ValueError):
            vc.quantize(np.array([1.0]), **{**MODEL, "read_noise": 0.0})


class TestVectors(unittest.TestCase):
    """Reference-encoded planes the Rust and WASM tests decode
    (``<name>.e<element size>.zst``), and a Rust-encoded plane decoded here."""

    def test_reference_vectors_are_current(self):
        for name, plane in planes().items():
            raw_path = VECTORS / f"{name}.raw"
            path = VECTORS / f"{name}.e{plane.itemsize}.zst"
            if os.environ.get("VIEWCAM_REGEN_VECTORS"):
                VECTORS.mkdir(parents=True, exist_ok=True)
                raw_path.write_bytes(plane.tobytes())
                path.write_bytes(reference_encode(plane))
            else:
                self.assertEqual(raw_path.read_bytes(), plane.tobytes(), name)
                back = vc.decode(path.read_bytes(), plane.dtype, plane.shape)
                np.testing.assert_array_equal(back, plane, err_msg=path.name)

    def test_decodes_rust_encoded_vector(self):
        plane = np.frombuffer((VECTORS / "rs_camera_u16.raw").read_bytes(), "<u2")
        encoded = (VECTORS / "rs_camera_u16.e2.zst").read_bytes()
        np.testing.assert_array_equal(vc.decode(encoded, "<u2", plane.shape), plane)
        np.testing.assert_array_equal(reference_decode(encoded, plane), plane)


if __name__ == "__main__":
    unittest.main()
