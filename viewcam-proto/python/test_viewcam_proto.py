"""Check the Python encoder against the shared test vectors, byte for byte.

    python3 -m unittest discover viewcam-proto/python
"""

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

    def test_frame_u16_raw(self):
        self.check("frame_u16_raw", vp.encode_frame(0x0102030405060708, 3, 2, 3, ramp(12)))

    def test_frame_f32_lz4(self):
        self.check("frame_f32_lz4", vp.encode_frame(7, 2, 2, 9, ramp(9), vp.CODEC_LZ4))

    def test_frame_u16_shuffle_lz4(self):
        self.check(
            "frame_u16_shuffle_lz4",
            vp.encode_frame(8, 3, 2, 3, ramp(5), vp.CODEC_SHUFFLE_LZ4),
        )

    def test_frame_empty(self):
        self.check("frame_empty", vp.encode_empty_frame(9, 10))

    def test_credit(self):
        self.check("credit", vp.encode_credit(0x1122334455667788))
        self.assertEqual(vp.decode_credit(vp.encode_credit(42)), 42)


class TestValidation(unittest.TestCase):
    def test_raw_payload_must_be_the_image(self):
        with self.assertRaises(vp.ProtocolError):
            vp.encode_frame(0, 2, 2, 3, b"\0" * 7)

    def test_lz4_payload_bounds(self):
        # 4 x 4 u16 = 32 bytes decoded; LZ4 allows 1 to 32 + 0 + 16 bytes.
        vp.encode_frame(0, 4, 4, 3, b"\0", vp.CODEC_LZ4)
        vp.encode_frame(0, 4, 4, 3, b"\0" * 48, vp.CODEC_SHUFFLE_LZ4)
        for bad in (b"", b"\0" * 49):
            with self.assertRaises(vp.ProtocolError):
                vp.encode_frame(0, 4, 4, 3, bad, vp.CODEC_LZ4)

    def test_unknown_codes(self):
        with self.assertRaises(vp.ProtocolError):
            vp.encode_frame(0, 1, 1, 3, b"\0\0", codec=3)
        with self.assertRaises(vp.ProtocolError):
            vp.encode_frame(0, 1, 1, 11, b"\0")

    def test_bad_credit_rejected(self):
        with self.assertRaises(vp.ProtocolError):
            vp.decode_credit(vp.encode_empty_frame(0, 1)[:16])

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
