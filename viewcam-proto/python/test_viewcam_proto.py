"""Check the Python encoder against the shared test vectors, byte for byte.

    python3 -m unittest discover viewcam-proto/python
"""

import collections
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

    def test_frame_u8_quantized_zstd(self):
        q = vp.Quantization(code_offset=-3, bias=432.0, read_noise=14.25, noise_slope=8.5, step=0.25)
        self.check(
            "frame_u8_quantized_zstd",
            vp.encode_frame(12, 3, 2, 1, ramp(4), vp.CODEC_SHUFFLE_ZSTD, quantization=q),
        )

    def test_ack(self):
        ack = vp.Ack(0x1122334455667788, 0x0102030405060708, 6, 0x00400000)
        self.check("ack", vp.encode_ack(ack))
        self.assertEqual(vp.decode_ack(vp.encode_ack(ack)), ack)


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

    def test_zstd_payload_bounds(self):
        # 4 x 4 u16 = 32 bytes decoded; zstd's bound is 32 + 0 + 63.
        vp.encode_frame(0, 4, 4, 3, b"\0" * 95, vp.CODEC_SHUFFLE_ZSTD)
        with self.assertRaises(vp.ProtocolError):
            vp.encode_frame(0, 4, 4, 3, b"\0" * 96, vp.CODEC_SHUFFLE_ZSTD)

    def test_quantized_frames_need_unsigned_codes_and_a_model(self):
        q = vp.Quantization(0, 432.0, 14.0, 8.0, 0.25)
        vp.encode_frame(0, 2, 2, 3, b"\0", vp.CODEC_SHUFFLE_ZSTD, quantization=q)
        bad = [
            (9, q),  # f32 codes
            (1, q._replace(read_noise=0.0)),
            (1, q._replace(step=float("nan"))),
            (1, q._replace(noise_slope=-1.0)),
        ]
        for dtype, quantization in bad:
            with self.assertRaises(vp.ProtocolError):
                vp.encode_frame(0, 2, 2, dtype, b"\0", vp.CODEC_SHUFFLE_ZSTD, quantization=quantization)

    def test_unknown_codes(self):
        with self.assertRaises(vp.ProtocolError):
            vp.encode_frame(0, 1, 1, 3, b"\0\0", codec=4)
        with self.assertRaises(vp.ProtocolError):
            vp.encode_frame(0, 1, 1, 11, b"\0")

    def test_bad_ack_rejected(self):
        with self.assertRaises(vp.ProtocolError):
            vp.decode_ack(vp.encode_empty_frame(0, 1)[:32])  # a frame cut to an ack's length
        with self.assertRaises(vp.ProtocolError):
            vp.decode_ack(vp.encode_ack(vp.Ack(0, 0, 1, 0))[:31])

    def test_dtype_codes(self):
        self.assertEqual(vp.dtype_code("<u2"), 3)
        self.assertEqual(vp.dtype_code("|u1"), 1)
        with self.assertRaises(vp.ProtocolError):
            vp.dtype_code(">u2")


class TestSendWindow(unittest.TestCase):
    def test_nothing_goes_before_the_first_ack(self):
        self.assertFalse(vp.SendWindow().may_send(100))

    def test_window_in_frames(self):
        send_window = vp.SendWindow()
        send_window.on_ack(vp.Ack(0, 0, 2, 0))
        for _ in range(2):
            self.assertTrue(send_window.may_send(100))
            send_window.sent(100)
        self.assertFalse(send_window.may_send(100))
        send_window.on_ack(vp.Ack(1, 100, 2, 0))
        self.assertTrue(send_window.may_send(100))

    def test_window_in_bytes(self):
        send_window = vp.SendWindow()
        send_window.on_ack(vp.Ack(0, 0, 10, 250))
        send_window.sent(100)
        self.assertTrue(send_window.may_send(150))
        self.assertFalse(send_window.may_send(151))
        send_window.on_ack(vp.Ack(1, 100, 10, 250))
        # Nothing in flight: a frame bigger than the whole byte window still goes.
        self.assertTrue(send_window.may_send(1000))

    def test_zero_frames_pauses(self):
        send_window = vp.SendWindow()
        send_window.on_ack(vp.Ack(0, 0, 0, 0))
        self.assertFalse(send_window.may_send(1))
        send_window.on_ack(vp.Ack(0, 0, 1, 0))
        self.assertTrue(send_window.may_send(1))

    def test_impossible_ack_is_a_protocol_error(self):
        send_window = vp.SendWindow()
        send_window.on_ack(vp.Ack(0, 0, 4, 0))
        send_window.sent(10)
        send_window.sent(10)
        send_window.on_ack(vp.Ack(2, 20, 4, 0))
        for ack in (
            vp.Ack(1, 10, 4, 0),  # acknowledges less than before
            vp.Ack(3, 30, 4, 0),  # acknowledges more than was sent
        ):
            with self.assertRaises(vp.ProtocolError):
                send_window.on_ack(ack)


def simulate(windowed, window_frames=6, window_bytes=0, frame_len=10_000,
             fps=30, one_way_ms=75, bytes_per_ms=1_000, stall=(3_000, 5_000), end_ms=8_000):
    """Stream a camera over a TCP-like link: in order, one way delay, limited
    bandwidth, and during ``stall`` nothing moves in either direction. A windowed
    server sends only as its SendWindow allows, holding just the newest frame
    while it waits; an unwindowed one writes every frame straight into the link.

    Returns the frames the client received as (arrival ms, seq, made ms).
    """
    stall_start, stall_end = stall
    stalled = lambda t: stall_start <= t < stall_end
    send_window = vp.SendWindow()
    client = {"frames": 0, "bytes": 0}
    to_client = collections.deque()   # frames written but not yet transmitted
    in_transit = collections.deque()  # (arrival ms, seq, made ms), transmitted
    acks = collections.deque()     # (arrival ms, Ack)
    acks.append((one_way_ms, vp.Ack(0, 0, window_frames, window_bytes)))
    pending = None
    head_left = 0  # bytes of the queue's head still to transmit
    received = []
    seq = 0
    for t in range(end_ms):
        if t % (1000 // fps) == 0:
            seq += 1
            pending = (seq, t)
        if not stalled(t):
            while acks and acks[0][0] <= t:
                send_window.on_ack(acks.popleft()[1])
        if pending and (not windowed or send_window.may_send(frame_len)):
            send_window.sent(frame_len)
            to_client.append(pending)
            pending = None
        if stalled(t):
            continue
        budget = bytes_per_ms
        while to_client and budget:
            if head_left == 0:
                head_left = frame_len
            sent_now = min(budget, head_left)
            head_left -= sent_now
            budget -= sent_now
            if head_left == 0:
                frame_seq, made = to_client.popleft()
                in_transit.append((t + one_way_ms, frame_seq, made))
        while in_transit and in_transit[0][0] <= t:
            _, frame_seq, made = in_transit.popleft()
            received.append((t, frame_seq, made))
            client["frames"] += 1
            client["bytes"] += frame_len
            acks.append((t + one_way_ms, vp.Ack(
                client["frames"], client["bytes"], window_frames, window_bytes)))
    return received


def catch_up_after(received, stall_end):
    """Frames received after the stall that were made before it ended."""
    return [r for r in received if r[0] >= stall_end and r[2] < stall_end]


class TestStallRecovery(unittest.TestCase):
    """What the window is for: after a stall, a bounded catch-up, then fresh frames."""

    def test_window_bounds_the_stale_frames_after_a_stall(self):
        received = simulate(windowed=True, window_frames=6)
        stale = catch_up_after(received, 5_000)
        # What was in flight, plus the newest frame the server held back.
        self.assertLessEqual(len(stale), 6 + 1)
        # And within a few hundred ms of the link returning, frames are current.
        recovered = [r for r in received if r[0] >= 5_500]
        self.assertTrue(recovered)
        self.assertTrue(all(arrived - made < 300 for arrived, _, made in recovered))

    def test_byte_window_bounds_it_tighter(self):
        received = simulate(windowed=True, window_frames=6, window_bytes=20_000)
        self.assertLessEqual(len(catch_up_after(received, 5_000)), 2 + 1)

    def test_steady_state_keeps_up_despite_the_round_trip(self):
        received = simulate(windowed=True, window_frames=6)
        before_stall = [r for r in received if 1_000 <= r[0] < 3_000]
        # 30 fps for 2 s, though one frame at a time would manage 1 / 150 ms.
        self.assertGreaterEqual(len(before_stall), 55)

    def test_without_a_send_window_a_stall_floods_the_client(self):
        received = simulate(windowed=False)
        # Every frame made during the stall arrives, stale, before anything new.
        self.assertGreater(len(catch_up_after(received, 5_000)), 50)


if __name__ == "__main__":
    unittest.main()
