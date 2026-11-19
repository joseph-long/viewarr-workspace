# viewcam streaming protocol, version 1 (draft)

How an instrument server streams images to viewcam clients (the native app
and the browser) over WebSockets. Still a draft: nothing consumes it yet, so
it changes in place.

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
feed, so a client can pair a saturation map with its data frame when it
wants to.

Each feed is independent: its own connection, `seq` order, window and
newest-frame-wins. Separate connections also keep a large data frame from
delaying the small saturation map queued behind it, as it would on one shared
TCP connection.

## Design

On TCP a lost packet stalls everything behind it until it is retransmitted,
and an outage of a few seconds stalls the connection for those seconds. What
matters is what happens next. If the server had kept writing frames into the
connection, they would be sitting in its socket buffers and the network, and
the client would have to receive all of them, stale, before anything current.
The longer the stall, the longer the flood after it.

So the server writes frames only as the client's acknowledgements allow,
much as TCP itself does with its ACKs and receive window:

1. **The client acknowledges what it has received.** It reports how many
   frames and bytes it has received, and how many it lets be in flight
   beyond that (its *window*). The server counts what it has written, so it
   knows exactly how much is in flight: written to the connection and not yet
   received, wherever it is (the server's buffers, the kernel's, the network).
   A send completing locally says nothing about that, which is why the
   window has to run from the client's acknowledgements, not from the
   server's own bookkeeping.
2. **Newest frame wins.** While the window is full, the server holds only
   the newest frame for the feed, replacing any older one still waiting. It
   never queues.
3. **The window bounds the catch-up.** After a stall, the client first
   receives what was already in flight, at most one window of frames and
   bytes, all of it from before the stall. The next frame is then the newest
   the server has. The cost of a stall, once the link is back, is one window.

Several frames in flight keep throughput independent of the round trip
(one at a time would cap a 150 ms link near 6 fps), so the window trades
throughput against how much stale data a stall can trap. See
[Choosing a window](#choosing-a-window).

## Transport

Each feed is a WebSocket at a URL of the server's choosing, such as
`wss://instrument.example/viewcam/camwfs_dark`. Every protocol message is one
binary WebSocket message, and every WebSocket message is one protocol
message. A text message, or a binary one that isn't a valid message of a kind
expected from that side, is a protocol error: close the connection with code
1002 (protocol error), or 1003 for a text message.

Servers should:

- disable Nagle's algorithm (`TCP_NODELAY`), so an ack or the tail of a
  frame isn't held back waiting for more to send;
- not negotiate `permessage-deflate` on a feed: LZ4 payloads don't deflate
  further, and the CPU it costs is latency.

## Encoding

All integers are little-endian. Every message starts with the same 8-byte
prefix:

| Offset | Size | Field      | Value                    |
| -----: | ---: | ---------- | ------------------------ |
|      0 |    4 | `magic`    | ASCII `VCAM`             |
|      4 |    1 | `version`  | `1`                      |
|      5 |    1 | `kind`     | `1` = FRAME, `0x81` = ACK |
|      6 |    2 | `reserved` | `0`; receivers ignore it |

A receiver rejects a message with the wrong magic or an unknown version or
kind.

### `FRAME` (kind 1, server → client)

72-byte header, then `encoded_len` bytes of payload, and nothing after it:

| Offset | Size | Field          | Notes                                     |
| -----: | ---: | -------------- | ----------------------------------------- |
|      0 |    8 | prefix         | `kind = 1`                                |
|      8 |    8 | `seq`          | u64, strictly increasing within a feed    |
|     16 |    4 | `width`        | u32                                       |
|     20 |    4 | `height`       | u32                                       |
|     24 |    1 | `dtype`        | element type, below                       |
|     25 |    1 | `codec`        | payload encoding, below                   |
|     26 |    1 | `quantization` | `0` = exact, `1` = noise-matched, below   |
|     27 |    1 | `reserved`     | `0`                                       |
|     28 |    4 | `encoded_len`  | u32, payload length in bytes              |
|     32 |    8 | `code_offset`  | i64; quantized frames only, else `0`      |
|     40 |    8 | `bias`         | f64; quantized frames only, else `0`      |
|     48 |    8 | `read_noise`   | f64; quantized frames only, else `0`      |
|     56 |    8 | `noise_slope`  | f64; quantized frames only, else `0`      |
|     64 |    8 | `step`         | f64; quantized frames only, else `0`      |

The decoded image is `width × height` elements of `dtype`, little-endian,
row-major, with no padding; the first row is `y = 0`, the bottom row as
displayed (FITS convention). Its length, `width × height × size(dtype)`, is
the **decoded length**.

A frame with `width × height = 0` means "no image on this feed", e.g. a
dark that has been withdrawn. Its `encoded_len` is 0.

`seq` strictly increases on a connection; gaps are fine, and are what a
client sees of the frames the server skipped. It can be the camera's own
frame counter, but if that counter restarts (a camera restarted, a stream
recreated) the server must keep `seq` increasing, e.g. by offsetting the new
counter past the last `seq` it sent.

#### Dtypes

| Code | Type | Bytes | | Code | Type | Bytes |
| ---: | ---- | ----: |-| ---: | ---- | ----: |
|    1 | u8   |     1 | |    6 | i32  |     4 |
|    2 | i8   |     1 | |    7 | u64  |     8 |
|    3 | u16  |     2 | |    8 | i64  |     8 |
|    4 | i16  |     2 | |    9 | f32  |     4 |
|    5 | u32  |     4 | |   10 | f64  |     8 |

#### Codecs

| Code | Codec          | Payload                                                         |
| ---: | -------------- | --------------------------------------------------------------- |
|    0 | `raw`          | the decoded image; `encoded_len` = decoded length               |
|    1 | `lz4`          | a raw LZ4 block (no frame, no size prefix) of the decoded image |
|    2 | `shuffle-lz4`  | a raw LZ4 block of the image's bytes shuffled by element        |
|    3 | `shuffle-zstd` | a zstd frame of the image's bytes shuffled by element           |

*Shuffled* means all the elements' byte 0, then all their byte 1, and so on
(Blosc's shuffle). It groups each pixel's slowly varying high byte with its
neighbours', where LZ4 finds the repetition the noisy low bytes hide. In
numpy: `img.view(np.uint8).reshape(-1, img.itemsize).T.tobytes()`.

The decompressed size is the decoded length, which the receiver computes
from the header instead of trusting a stored size, and a payload that
decompresses to any other length is an error. `encoded_len` may not exceed
the codec's worst case for a decoded length `n` (integer division
throughout):

- LZ4: `n + n / 255 + 16`;
- zstd: `n + n / 256 + (n < 131072 ? (131072 − n) / 2048 : 0)`, zstd's
  `ZSTD_COMPRESSBOUND`, which includes the frame header. The zstd frame
  must be a single frame, and a receiver needn't check content checksums.

Which codec suits a feed is the server's call: `raw` costs no CPU,
`shuffle-lz4` costs little and suits images with repetitive bytes (a mostly
zero saturation map compresses enormously with either LZ4 codec), and
`shuffle-zstd` costs more CPU but, having entropy coding, also compresses
noise, which is most of a camera frame. Compression also shrinks what a
stall can trap, since the window counts encoded bytes.

#### Quantization

A display can't use precision finer than an image's noise, and noise is
what compresses worst. So a server may round a frame to steps a fixed
fraction of its noise, and send the steps' integer **codes**
(`quantization = 1`). The noise model is one where a pixel at `value` has
noise variance

    read_noise² + noise_slope × max(value − bias, 0)

which covers read noise alone (`noise_slope = 0`) and photon noise, which
grows with the signal, on top of it. Values are first mapped to `y`, in
units of that noise:

    s = value − bias
    y = s / read_noise                                                if s ≤ 0 or noise_slope = 0
    y = (2 / noise_slope) (√(noise_slope × s + read_noise²) − read_noise)   otherwise

linear at and below `bias`, and above it the square root that holds the
noise at one unit, the generalized Anscombe transform. The two meet with
the same slope at `bias`. The code is `round(y / step)`, and each payload
element is a code less `code_offset`, so it can be stored unsigned in as
few bytes as the frame's codes need: `dtype` must be `u8`, `u16` or `u32`.

To decode, `y = (element + code_offset) × step`, and

    value = bias + y × read_noise                                     if y ≤ 0 or noise_slope = 0
    value = bias + ((noise_slope × y / 2 + read_noise)² − read_noise²) / noise_slope   otherwise

A quantized frame decodes to `f32` values; `width`, `height` and the codec
describe the codes, as for any frame. `read_noise` and `step` must be
positive, `noise_slope` not negative, and all four finite. An exact frame
(`quantization = 0`) has all five fields zero. Any other `quantization` is
an error.

A value comes back within half a step of its noise: at a step of ¼, an
error a fraction of the noise that adds ~2% to it in quadrature. Where the
noise grows with the signal, steps are finest at the faint end, where a
display stretched for faint detail needs them, and coarse only for bright
pixels, which are noisier anyway. Rounding is monotonic, so a level (a
saturation level, say) compares with quantized values as the value of its
own code. `viewcam-codec`'s `Quantizer` implements both directions.

### `ACK` (kind 0x81, client → server)

32 bytes:

| Offset | Size | Field           | Notes                                         |
| -----: | ---: | --------------- | --------------------------------------------- |
|      0 |    8 | prefix          | `kind = 0x81`                                 |
|      8 |    8 | `acked_frames`  | u64, `FRAME`s received on this connection     |
|     16 |    8 | `acked_bytes`   | u64, their total length, headers included     |
|     24 |    4 | `window_frames` | u32, frames allowed in flight; `0` pauses     |
|     28 |    4 | `window_bytes`  | u32, bytes allowed in flight; `0` = no limit  |

The `acked_*` counts are cumulative, so they never decrease, and can never
exceed what the server has sent; an ack that breaks either rule is a
protocol error. The window fields replace the previous ack's, so a client
can resize its window, or pause a feed it isn't showing, at any time.

## Server behaviour

For each connection the server counts `sent_frames` and `sent_bytes`, the
`FRAME`s it has written and their total length. With the latest ack:

    in_flight_frames = sent_frames - acked_frames
    in_flight_bytes  = sent_bytes  - acked_bytes

It may write a frame of `n` bytes (header included) when

    window_frames > 0
    and in_flight_frames < window_frames
    and (window_bytes = 0 or in_flight_frames = 0
         or in_flight_bytes + n <= window_bytes)

The `in_flight_frames = 0` case lets a frame larger than the byte window go
alone, rather than never.

- Write nothing until the first ack arrives: the initial window is zero.
- Keep only the newest frame waiting on each feed. When a new one arrives
  before the waiting one was written, replace it.
- Write the waiting frame as soon as the rule above allows, which is either
  when it arrives or when an ack frees room.
- When a feed opens, send its current frame, so a rarely changing feed (the
  dark) needs nothing else until it changes. If the feed has no image, send
  an empty frame so the client knows.
- When a feed's image goes away, send an empty frame.

## Client behaviour

- Send an ack as soon as the connection opens (`acked_* = 0`, with the
  window), again after every `FRAME` received, and whenever the window
  changes. Acknowledge a frame when it arrives, before decoding it: decoding
  time shouldn't count against the window, and the client can drop frames
  it hasn't the time to decode itself.
- Check a frame's header before trusting its payload. Reject headers that are
  inconsistent (a `raw` payload not exactly the decoded length, an LZ4 payload
  over the bound, a nonzero payload for an empty frame, a message longer or
  shorter than its header says) and bound what it accepts (e.g. at most 2²⁸
  pixels), so a corrupt header cannot ask it to allocate gigabytes.
- Frames arrive in `seq` order, but a client that decodes off its main
  thread can finish them out of order: show a frame only if its `seq` is
  newer than the newest shown on that feed, and drop it otherwise.
- Decoding the LZ4 and zstd codecs, and quantized frames' codes, is up to
  the client; the `viewcam-codec` crate does all three in Rust and WASM.

`js/viewcam_proto.js` is a browser client that does all of this:
`ViewcamFeed` handles one feed, and decodes in `viewcam-codec`'s web worker
when given `workerDecoder(worker)`.

### Choosing a window

A window of `F` frames and `B` bytes, over a round trip `rtt`, allows at most
`F / rtt` frames and `B / rtt` bytes per second, and a stall can leave at most
`F` frames and `max(B, one frame)` bytes to receive before anything fresh.

- `window_frames` of about `rtt × fps` plus one or two reaches a frame rate
  `fps` (6 covers 30 fps over 150 ms). Larger buys nothing but a longer
  catch-up.
- `window_bytes` matters when frames are large compared with the link: set
  it to about what the link carries in a round trip, and the catch-up after
  a stall is about one round trip's worth of data. `0` leaves the frame count
  as the only bound.
- A rarely changing feed like the dark needs a window of 1.
- `window_frames = 0` pauses a feed (a hidden tab, a collapsed panel) without
  closing it; reopening the window gets the newest frame straight away.

## Test vectors

`tests/vectors/` holds encoded examples of each message, a quantized frame
among them. The Rust crate, the
Python reference (`python/viewcam_proto.py`) and the browser client
(`js/viewcam_proto.js`) are all tested against them, byte for byte.
