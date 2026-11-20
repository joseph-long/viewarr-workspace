"""Test the Python extension: that it runs the Rust crate's code, and that
its numpy-facing API handles the buffers servers hand it.

    maturin develop -m viewcam-codec/python/Cargo.toml
    python -m pytest viewcam-codec/python/tests

The format itself is specified and tested in the crate (src/tests.rs),
which also writes the planes in ../../tests/vectors/ that these tests,
like the WASM module's, check the extension encodes and decodes the same.
"""

import threading
import unittest
from pathlib import Path

import numpy as np

import viewcam_codec as vc

VECTORS = Path(__file__).resolve().parents[2] / "tests" / "vectors"


def planes():
    rng = np.random.default_rng(1)
    y, x = np.mgrid[0:24, 0:32]
    camera = (1000 + 4 * y + 2 * x + rng.integers(0, 64, size=y.shape)).astype("<u2")
    return {
        "camera_u16": camera,
        "mask_u8": (camera > 1150).astype(np.uint8),
        "float_f32": (camera / 7.0).astype("<f4"),
        "signed_i32": (camera.astype("<i4") - 1200),
        "dark_f64": (camera / 3.0).astype("<f8"),
    }


class TestVectors(unittest.TestCase):
    """The planes the Rust tests encoded (``<name>.e<element size>.zst``,
    beside ``<name>.raw``): the extension must decode them, and encode the
    raw planes to the very same bytes, being the same code."""

    def vectors(self):
        found = sorted(VECTORS.glob("*.zst"))
        self.assertGreaterEqual(len(found), 5, f"expected vectors in {VECTORS}")
        for path in found:
            name, size = path.name.removesuffix(".zst").rsplit(".e", 1)
            yield path.name, int(size), (VECTORS / f"{name}.raw").read_bytes(), path.read_bytes()

    def test_decodes_what_rust_encoded(self):
        for name, size, raw, encoded in self.vectors():
            dtype = {1: "u1", 2: "<u2", 4: "<u4", 8: "<u8"}[size]
            back = vc.decode(encoded, dtype, (len(raw) // size,))
            self.assertEqual(back.tobytes(), raw, name)

    def test_encodes_as_rust_does(self):
        for name, size, raw, encoded in self.vectors():
            dtype = {1: "u1", 2: "<u2", 4: "<u4", 8: "<u8"}[size]
            self.assertEqual(vc.encode(np.frombuffer(raw, dtype)), encoded, name)

    def test_round_trip(self):
        for name, plane in planes().items():
            back = vc.decode(vc.encode(plane), plane.dtype, plane.shape)
            np.testing.assert_array_equal(back, plane, err_msg=name)


class TestInputs(unittest.TestCase):
    def test_strided_views_are_made_contiguous(self):
        plane = planes()["camera_u16"]
        view = plane[:, ::2]
        back = vc.decode(vc.encode(view), view.dtype, view.shape)
        np.testing.assert_array_equal(back, view)

    def test_big_endian_is_refused(self):
        with self.assertRaises(ValueError):
            vc.encode(planes()["camera_u16"].astype(">u2"))

    def test_plain_buffers_work(self):
        data = bytes(range(200))
        self.assertEqual(vc.decode(vc.encode(data), "u1", (200,)).tobytes(), data)

    def test_corrupt_input_raises(self):
        with self.assertRaises(ValueError):
            vc.decode(b"\x01\x02\x03", "<u2", (8,))
        encoded = vc.encode(planes()["camera_u16"])
        with self.assertRaises(ValueError):
            vc.decode(encoded, "<u2", (24, 33))
        with self.assertRaises(ValueError):
            vc.decode(encoded[:-3], "<u2", (24, 32))

    def test_threads_encode_concurrently(self):
        # Mostly a check that releasing the GIL does not break anything.
        plane = np.tile(planes()["camera_u16"], (20, 20))
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


if __name__ == "__main__":
    unittest.main()
