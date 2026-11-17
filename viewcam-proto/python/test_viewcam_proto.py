"""Check the Python encoder against the shared test vectors, byte for byte.

    python3 -m unittest discover viewcam-proto/python
"""

import struct
import unittest
from pathlib import Path

import viewcam_proto as vp

VECTORS = Path(__file__).resolve().parent.parent / "tests" / "vectors"


def ramp(n):
    return bytes((i * 37 + 11) % 256 for i in range(n))


class TestVectors(unittest.TestCase):
    """The same messages as the Rust crate's `vectors()`."""

    def check(self, name, encoded):
        self.assertEqual(encoded, (VECTORS / f"{name}.bin").read_bytes(), name)

    def test_frame_u16_with_saturation(self):
        self.check(
            "frame_u16_with_saturation",
            vp.encode_frame(
                0x0102030405060708, 3, 2, ramp(12), 3, bytes([0, 1, 0, 0, 0, 1]), 1
            ),
        )

    def test_frame_f32_no_saturation(self):
        data = struct.pack("<4f", 1.5, -2.0, 0.0, 65535.0)
        self.check("frame_f32_no_saturation", vp.encode_frame(7, 2, 2, data, 9))

    def test_dark_f64(self):
        self.check("dark_f64", vp.encode_dark(2, 1, struct.pack("<2d", 1000.25, -0.5), 10))

    def test_clear_dark(self):
        self.check("clear_dark", vp.encode_clear_dark())

    def test_credit(self):
        self.check("credit", vp.encode_credit(0x1122334455667788))
        self.assertEqual(vp.decode_credit(vp.encode_credit(42)), 42)


class TestValidation(unittest.TestCase):
    def test_plane_length_is_checked(self):
        with self.assertRaises(vp.ProtocolError):
            vp.encode_frame(0, 2, 2, b"\0" * 7, 3)

    def test_saturation_needs_its_dtype(self):
        with self.assertRaises(vp.ProtocolError):
            vp.encode_frame(0, 1, 1, b"\0", 1, saturation=b"\0")

    def test_bad_credit_rejected(self):
        with self.assertRaises(vp.ProtocolError):
            vp.decode_credit(vp.encode_clear_dark() + b"\0" * 8)

    def test_dtype_codes(self):
        self.assertEqual(vp.dtype_code("<u2"), 3)
        self.assertEqual(vp.dtype_code("|u1"), 1)
        with self.assertRaises(vp.ProtocolError):
            vp.dtype_code(">u2")


class TestCreditGate(unittest.TestCase):
    def test_gate(self):
        gate = vp.CreditGate()
        self.assertFalse(gate.may_send())
        gate.on_credit(2)
        gate.on_credit(1)  # late, reordered datagram: ignored
        gate.sent()
        gate.sent()
        self.assertFalse(gate.may_send())
        gate.on_credit(3)
        self.assertTrue(gate.may_send())


if __name__ == "__main__":
    unittest.main()
