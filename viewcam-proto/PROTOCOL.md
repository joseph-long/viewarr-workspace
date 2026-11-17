# viewcam streaming protocol, version 1

How an instrument server streams camera frames to viewcam clients (the
native app and the browser) over [WebTransport]. The goal is video that does
not stall on a lossy, high-latency link. Three choices get there:

1. **One stream per frame.** A lost packet holds up only the frame it belongs
   to; later frames keep arriving on their own streams. (Over TCP, one loss
   blocks everything behind it until it is retransmitted.)
2. **Newest frame wins.** A client shows a frame only if it is newer than the
   one on screen, and abandons a frame stream once a newer frame has been
   shown.
3. **A credit window.** The server keeps at most a client-granted number of
   frames in flight. It never builds a backlog behind a slow link; it sends
   whatever is newest when credit frees up. Several frames in flight means
   throughput does not depend on the round trip time (one-at-a-time request/reply would cap a 150 ms link
   near 6 fps).

[WebTransport]: https://www.w3.org/TR/webtransport/

## Session

The client opens a WebTransport session on a URL the server chooses, e.g.
`https://instrument.example:4433/viewcam`. Everything below happens inside
that session, which must have datagrams enabled.

Servers on private networks without a publicly trusted certificate can use a
self-signed one and give clients its SHA-256 hash to pin, the browser's
`serverCertificateHashes` mechanism. Browsers accept such a certificate only
if it is ECDSA (P-256) and valid for **at most 14 days**, so it must be rotated.

## Encoding

All integers are little-endian. Pixel planes are row-major, `width × height`
elements of their dtype, with no padding, and the first row is `y = 0` (the
bottom row as displayed, FITS convention).

Every message starts with the same 8-byte prefix:

| Offset | Size | Field      | Value                       |
| -----: | ---: | ---------- | --------------------------- |
|      0 |    4 | `magic`    | ASCII `VCAM`                |
|      4 |    1 | `version`  | `1`                         |
|      5 |    1 | `kind`     | message kind, below         |
|      6 |    2 | `reserved` | `0`; receivers ignore it    |

A receiver rejects a message with the wrong magic or an unknown version or
kind.

### Dtype codes

| Code | Type | Bytes | | Code | Type | Bytes |
| ---: | ---- | ----: |-| ---: | ---- | ----: |
|    1 | u8   |     1 | |    6 | i32  |     4 |
|    2 | i8   |     1 | |    7 | u64  |     8 |
|    3 | u16  |     2 | |    8 | i64  |     8 |
|    4 | i16  |     2 | |    9 | f32  |     4 |
|    5 | u32  |     4 | |   10 | f64  |     8 |

## Server → client: one message per unidirectional stream

The server opens a new unidirectional stream for each message, writes the
message, and finishes the stream. A stream that ends before its full length
arrived, or carries bytes past it, is an error and is discarded.

### `FRAME` (kind 1)

A camera frame with an optional saturation plane. 28-byte header:

| Offset | Size | Field       | Notes                                        |
| -----: | ---: | ----------- | -------------------------------------------- |
|      0 |    8 | prefix      | `kind = 1`                                   |
|      8 |    8 | `seq`       | u64, strictly increasing within a session    |
|     16 |    4 | `width`     | u32                                          |
|     20 |    4 | `height`    | u32                                          |
|     24 |    1 | `data_dtype`| dtype code                                   |
|     25 |    1 | `sat_dtype` | dtype code, or `0` for no saturation plane   |
|     26 |    2 | `reserved`  | `0`                                          |

Then the data plane, then the saturation plane if `sat_dtype ≠ 0`. In the
saturation plane any nonzero value marks a saturated pixel. A frame without
one clears the saturation overlay: it describes that frame only.

`seq` can be the camera's own frame counter; gaps are fine.

### `DARK` (kind 2)

The dark frame clients subtract from subsequent `FRAME`s. 20-byte header:

| Offset | Size | Field      | Notes          |
| -----: | ---: | ---------- | -------------- |
|      0 |    8 | prefix     | `kind = 2`     |
|      8 |    4 | `width`    | u32            |
|     12 |    4 | `height`   | u32            |
|     16 |    1 | `dtype`    | dtype code     |
|     17 |    3 | `reserved` | `0`            |

Then the plane. The server sends one when a session opens (if it has one)
and whenever it changes. `DARK` messages do not use credit.

### `CLEAR_DARK` (kind 3)

The 8-byte prefix alone, `kind = 3`: forget the dark frame.

## Client → server: datagrams

### `CREDIT` (kind 0x81)

16 bytes: the prefix with `kind = 0x81`, then `limit`, a u64 at offset 8.

`limit` is the total number of `FRAME` streams the server may have opened in
this session so far. It only ever grows, so it is safe to lose or reorder: the
server keeps the largest `limit` it has seen and ignores the rest. A client
sends one:

- as soon as the session opens (`limit = window`),
- each time a `FRAME` stream finishes for it, however it ended (`limit =
  frames finished + window`),
- and every 100 ms regardless, since datagrams can be lost.

A client picks `window` from the round trip time and the frame rate it wants:
about `rtt × fps`, plus a little slack (6 covers 30 fps over 150 ms).

## Server behaviour

- Keep only the newest camera frame waiting to be sent. When a new one
  arrives before the waiting one was sent, replace it.
- Send the waiting frame as soon as `frames opened < limit`.
- On `STOP_SENDING` for a frame stream, reset it (`RESET_STREAM`); the client
  no longer wants that frame.
- A server may also reset an unfinished older frame stream itself once it
  has sent a newer one, to save bandwidth.

## Client behaviour

- Read each stream's header as soon as it arrives. If it is a `FRAME` whose
  `seq` is not newer than the newest frame shown, send `STOP_SENDING` and
  move on.
- When a `FRAME` completes, show it only if it is still the newest; otherwise
  drop it.
- Count every `FRAME` stream that ends, however it ends (completed, stopped,
  reset, malformed), towards "frames finished" for credit.
- Bound what it will accept (e.g. at most 2²⁸ pixels per plane) so a corrupt
  header cannot ask it to allocate gigabytes.

## Test vectors

`tests/vectors/` holds encoded examples of every message. The Rust crate and
the Python reference encoder (`python/viewcam_proto.py`) are both tested
against them, byte for byte.
