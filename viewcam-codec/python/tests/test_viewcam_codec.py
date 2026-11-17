"""Test the Python extension against an independent reference: numpy for the
shuffle and the C liblz4 (through the ``lz4`` package) for compression.

    maturin develop -m viewcam-codec/python/Cargo.toml
    python -m pytest viewcam-codec/python/tests

The reference-encoded planes are also stored in ../../tests/vectors/ for the
Rust and WASM tests; set VIEWCAM_REGEN_VECTORS=1 to rewrite them.
"""

import os
import threading
import unittest
from pathlib import Path

import lz4.block
import numpy as np

import viewcam_codec as vc

VECTORS = Path(__file__).resolve().parents[2] / "tests" / "vectors"


def reference_encode(plane, shuffle=True):
    data = plane.view(np.uint8).reshape(-1, plane.itemsize).T.tobytes() if shuffle else plane.tobytes()
    return lz4.block.compress(data, store_size=False)


def reference_decode(encoded, plane_like, shuffle=True):
    data = lz4.block.decompress(encoded, uncompressed_size=plane_like.nbytes)
    if shuffle:
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
            for shuffle in (True, False):
                back = vc.decode(vc.encode(plane, shuffle), plane.dtype, plane.shape, shuffle)
                np.testing.assert_array_equal(back, plane, err_msg=f"{name} {shuffle=}")

    def test_reference_decodes_ours(self):
        # Our payloads are standard LZ4 blocks of the shuffled bytes.
        for name, plane in planes().items():
            for shuffle in (True, False):
                back = reference_decode(vc.encode(plane, shuffle), plane, shuffle)
                np.testing.assert_array_equal(back, plane, err_msg=f"{name} {shuffle=}")

    def test_we_decode_the_reference(self):
        for name, plane in planes().items():
            for shuffle in (True, False):
                back = vc.decode(reference_encode(plane, shuffle), plane.dtype, plane.shape, shuffle)
                np.testing.assert_array_equal(back, plane, err_msg=f"{name} {shuffle=}")

    def test_shuffle_layout(self):
        # Low bytes first, then high bytes, as in the Rust test.
        plane = np.array([0x0201, 0x0403, 0x0605], "<u2")
        data = lz4.block.decompress(vc.encode(plane), uncompressed_size=6)
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
        self.assertEqual(lz4.block.decompress(vc.encode(data, shuffle=False), uncompressed_size=200), data)

    def test_corrupt_input_raises(self):
        with self.assertRaises(ValueError):
            vc.decode(b"\x01\x02\x03", "<u2", (8,))
        encoded = vc.encode(planes()["py_camera_u16"])
        with self.assertRaises(ValueError):
            vc.decode(encoded, "<u2", (24, 33))

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


class TestVectors(unittest.TestCase):
    """Reference-encoded planes the Rust and WASM tests decode
    (``<name>.e<element size>.lz4``; element size 1 is plain lz4), and a
    Rust-encoded plane decoded here."""

    def test_reference_vectors_are_current(self):
        camera = planes()["py_camera_u16"]
        entries = [(name, plane, True) for name, plane in planes().items()]
        entries.append(("py_camera_u16", camera, False))
        for name, plane, shuffle in entries:
            raw_path = VECTORS / f"{name}.raw"
            lz4_path = VECTORS / f"{name}.e{plane.itemsize if shuffle else 1}.lz4"
            if os.environ.get("VIEWCAM_REGEN_VECTORS"):
                VECTORS.mkdir(parents=True, exist_ok=True)
                raw_path.write_bytes(plane.tobytes())
                lz4_path.write_bytes(reference_encode(plane, shuffle))
            else:
                self.assertEqual(raw_path.read_bytes(), plane.tobytes(), name)
                back = vc.decode(lz4_path.read_bytes(), plane.dtype, plane.shape, shuffle)
                np.testing.assert_array_equal(back, plane, err_msg=lz4_path.name)

    def test_decodes_rust_encoded_vector(self):
        plane = np.frombuffer((VECTORS / "rs_camera_u16.raw").read_bytes(), "<u2")
        encoded = (VECTORS / "rs_camera_u16.e2.lz4").read_bytes()
        np.testing.assert_array_equal(vc.decode(encoded, "<u2", plane.shape), plane)
        np.testing.assert_array_equal(reference_decode(encoded, plane), plane)


if __name__ == "__main__":
    unittest.main()
