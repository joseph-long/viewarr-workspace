"""Cross-check the Python codec with the Rust one through shared vectors.

    uv run --with numpy --with lz4 python -m unittest discover viewcam-codec/python

Set VIEWCAM_REGEN_VECTORS=1 to rewrite the Python-encoded vectors the Rust
tests decode.
"""

import os
import unittest
from pathlib import Path

import numpy as np

import viewcam_codec as vc

VECTORS = Path(__file__).resolve().parent.parent / "tests" / "vectors"


def python_planes():
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


class TestCodec(unittest.TestCase):
    def test_round_trip(self):
        for name, plane in python_planes().items():
            back = vc.decode(vc.encode(plane), plane.dtype, plane.shape)
            np.testing.assert_array_equal(back, plane, err_msg=name)

    def test_shuffle_layout(self):
        # Low bytes first, then high bytes, as in the Rust test.
        plane = np.array([0x0201, 0x0403, 0x0605], "<u2")
        shuffled = plane.view(np.uint8).reshape(-1, 2).T.tobytes()
        self.assertEqual(shuffled, bytes([1, 3, 5, 2, 4, 6]))

    def test_python_vectors_are_current(self):
        for name, plane in python_planes().items():
            raw_path = VECTORS / f"{name}.raw"
            lz4_path = VECTORS / f"{name}.e{plane.itemsize}.lz4"
            if os.environ.get("VIEWCAM_REGEN_VECTORS"):
                VECTORS.mkdir(parents=True, exist_ok=True)
                raw_path.write_bytes(plane.tobytes())
                lz4_path.write_bytes(vc.encode(plane))
            else:
                self.assertEqual(raw_path.read_bytes(), plane.tobytes(), name)
                # The stored encoding must decode correctly; byte equality
                # with a fresh encode is not required across lz4 versions.
                back = vc.decode(lz4_path.read_bytes(), plane.dtype, plane.shape)
                np.testing.assert_array_equal(back, plane, err_msg=name)

    def test_decodes_rust_encoded_vector(self):
        raw = (VECTORS / "rs_camera_u16.raw").read_bytes()
        plane = np.frombuffer(raw, "<u2")
        back = vc.decode((VECTORS / "rs_camera_u16.e2.lz4").read_bytes(), "<u2", plane.shape)
        np.testing.assert_array_equal(back, plane)


if __name__ == "__main__":
    unittest.main()
