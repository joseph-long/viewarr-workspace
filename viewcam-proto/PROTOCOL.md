# viewcam streaming protocol, version 1 (draft)

How an instrument server streams images to viewcam clients (the native app
and the browser). Still a draft: nothing consumes it yet, so it changes in
place.

## Feeds

A **feed** is a sequence of frames of one kind. A camera viewer subscribes
to three, each on its own connection:

| Feed       | Typical rate              | Client uses it as                       |
| ---------- | ------------------------- | --------------------------------------- |
| data       | camera rate               | the image                               |
| dark       | rarely changes            | subtracted from the data                |
| saturation | camera rate, mostly zeros | nonzero pixels drawn as saturated       |

The protocol does not know which is which: every feed carries the same
`FRAME` messages, and a client combines them as it sees fit. A server should
give frames derived from the same camera exposure the same `seq` on every
feed (the camera's frame counter does this), so a client can pair a
saturation map with its data frame when it wants to.

Each feed is independent: its own `seq` order, its own credit window, its
own newest-frame-wins.

## Design

Three choices keep video from stalling on a lossy, high-latency link:

1. **One stream per frame.** A lost packet holds up only the frame it
   belongs to; later frames keep arriving on their own streams. (On a single
   TCP byte stream, one loss blocks everything behind it until it is
   retransmitted.)
2. **Newest frame wins.** A client shows a frame only if it is newer than the
   one on screen, and abandons a frame stream once a newer frame was shown.
3. **A credit window.** The server keeps at most a client-granted number of
   frames in flight, so it never builds a backlog behind a slow link, and
   several frames in flight means throughput does not hinge on the round
   trip (one-at-a-time request/reply would cap a 150 ms link near 6 fps).

## Transport

*Not settled.* The draft binding is WebTransport (HTTP/3): one session per
feed, at server-chosen URLs such as `https://instrument.example/viewcam/data`,
with datagrams enabled; each `FRAME` on its own server-initiated
unidirectional stream, finished after it; `CREDIT` as datagrams.

Every message is self-delimiting (its length is in its header), so a
byte-stream fallback such as a WebSocket can carry the same messages
back to back.

For servers on private networks, browsers can pin a self-signed certificate
by its SHA-256 hash (`serverCertificateHashes`), provided it is ECDSA P-256
and valid for at most 14 days.

## Encoding

All integers are little-endian. Every message starts with the same 8-byte
prefix:

| Offset | Size | Field      | Value                    |
| -----: | ---: | ---------- | ------------------------ |
|      0 |    4 | `magic`    | ASCII `VCAM`             |
|      4 |    1 | `version`  | `1`                      |
|      5 |    1 | `kind`     | `1` = FRAME, `0x81` = CREDIT |
|      6 |    2 | `reserved` | `0`; receivers ignore it |

A receiver rejects a message with the wrong magic or an unknown version or
kind.

### `FRAME` (kind 1, server → client)

32-byte header, then `encoded_len` bytes of payload:

| Offset | Size | Field         | Notes                                     |
| -----: | ---: | ------------- | ----------------------------------------- |
|      0 |    8 | prefix        | `kind = 1`                                |
|      8 |    8 | `seq`         | u64, strictly increasing within a feed    |
|     16 |    4 | `width`       | u32                                       |
|     20 |    4 | `height`      | u32                                       |
|     24 |    1 | `dtype`       | element type, below                       |
|     25 |    1 | `codec`       | payload encoding, below                   |
|     26 |    2 | `reserved`    | `0`                                       |
|     28 |    4 | `encoded_len` | u32, payload length in bytes              |

The decoded image is `width × height` elements of `dtype`, little-endian,
row-major, with no padding; the first row is `y = 0`, the bottom row as
displayed (FITS convention). Its length, `width × height × size(dtype)`, is
the **decoded length**.

A frame with `width × height = 0` means "no image on this feed", e.g. a
dark that has been withdrawn. Its `encoded_len` is 0.

`seq` can be the camera's own frame counter; gaps are fine.

#### Dtypes

| Code | Type | Bytes | | Code | Type | Bytes |
| ---: | ---- | ----: |-| ---: | ---- | ----: |
|    1 | u8   |     1 | |    6 | i32  |     4 |
|    2 | i8   |     1 | |    7 | u64  |     8 |
|    3 | u16  |     2 | |    8 | i64  |     8 |
|    4 | i16  |     2 | |    9 | f32  |     4 |
|    5 | u32  |     4 | |   10 | f64  |     8 |

#### Codecs

| Code | Codec         | Payload                                                         |
| ---: | ------------- | --------------------------------------------------------------- |
|    0 | `raw`         | the decoded image; `encoded_len` = decoded length               |
|    1 | `lz4`         | a raw LZ4 block (no frame, no size prefix) of the decoded image |
|    2 | `shuffle-lz4` | a raw LZ4 block of the image's bytes shuffled by element        |

*Shuffled* means all the elements' byte 0, then all their byte 1, and so on
(Blosc's shuffle). It groups each pixel's slowly varying high byte with its
neighbours', where LZ4 finds the repetition the noisy low bytes hide. In
numpy: `img.view(np.uint8).reshape(-1, img.itemsize).T.tobytes()`.

The LZ4 block's decompressed size is the decoded length, which the receiver
computes from the header instead of trusting a stored size. For the LZ4
codecs, `encoded_len` may not exceed LZ4's worst case for the decoded length,
`n + n / 255 + 16` (integer division).

Which codec suits a feed is the server's call: `raw` costs no CPU,
`shuffle-lz4` usually compresses multi-byte camera data best, and a mostly
zero saturation map compresses enormously with either LZ4 codec.

### `CREDIT` (kind 0x81, client → server)

16 bytes: the prefix with `kind = 0x81`, then `limit`, a u64 at offset 8.

`limit` is the total number of `FRAME` streams the server may have opened on
this feed so far. It only ever grows, so it is safe to lose or reorder: the
server keeps the largest `limit` it has seen and ignores the rest. A client
sends one:

- as soon as the feed opens (`limit = window`),
- each time a `FRAME` stream finishes for it, however it ended (`limit =
  frames finished + window`),
- and every 100 ms regardless, since datagrams can be lost.

A client picks `window` from the round trip and the frame rate it wants:
about `rtt × fps`, plus a little slack (6 covers 30 fps over 150 ms). A
rarely changing feed like the dark needs only 1 or 2.

## Server behaviour

- Keep only the newest frame waiting to be sent on each feed. When a new one
  arrives before the waiting one was sent, replace it.
- Send the waiting frame as soon as `frames opened < limit`.
- On `STOP_SENDING` for a frame stream, reset it (`RESET_STREAM`); the client
  no longer wants that frame.
- A server may also reset an unfinished older frame stream itself once it
  has sent a newer one, to save bandwidth.
- A rarely changing feed (the dark) sends its current frame when the feed
  opens, then only when it changes.

## Client behaviour

- Read each stream's header as soon as it arrives. If `seq` is not newer
  than the newest frame shown on that feed, send `STOP_SENDING` and move on.
- When a frame completes, show it only if it is still the newest; otherwise
  drop it.
- Count every `FRAME` stream that ends, however it ends (completed, stopped,
  reset, malformed), towards "frames finished" for credit.
- Reject headers that are inconsistent (a `raw` payload not exactly the
  decoded length, an LZ4 payload over the bound, a nonzero payload for an
  empty frame) and bound what it accepts (e.g. at most 2²⁸ pixels), so a
  corrupt header cannot ask it to allocate gigabytes.
- Decoding the LZ4 codecs is up to the client; the `viewcam-codec` crate does
  it in Rust and WASM.

## Test vectors

`tests/vectors/` holds encoded examples of each message. The Rust crate and
the Python reference encoder (`python/viewcam_proto.py`) are both tested
against them, byte for byte.
