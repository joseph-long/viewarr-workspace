//! One feed: a WebSocket to a viewcam stream, read on a thread of its own.
//!
//! The same client behaviour as the browser's `ViewcamFeed` (see
//! viewcam-proto/PROTOCOL.md, "Client behaviour"): send an ack as soon as
//! the connection opens and again for every frame as it arrives, before
//! decoding it; decode off the socket's thread, keeping only the newest
//! frame waiting; reconnect with a growing delay when the connection drops.
//! And the same cap on the frame rate: with `max_fps` set, acks go out at
//! most that often, so a window of one frame lets that many frames through.
//! And the same byte window, sized from the rate frames arrive at so that
//! about `target_in_flight` seconds of them are on the way at once.

use std::io;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::CloseFrame;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};
use viewcam::status::Connection;
use viewcam_proto::{Ack, DeliveryRate};

use crate::decode::{Decoded, DecodedFrame, Decoder};

const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(10);
/// Longest the reader waits on the socket before looking at its commands
/// and any paced ack that has come due.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// How many frames may be in flight, and how often acks may go out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pacing {
    pub window_frames: u32,
    /// Acks a second at most; 0 for no limit.
    pub max_fps: f64,
    /// Seconds of the delivery rate to keep in flight, which sets the byte
    /// window (PROTOCOL.md, "Choosing a window"); 0 for no byte limit.
    pub target_in_flight: f64,
}

pub enum FeedEvent {
    Connection(Connection),
    /// A message arrived, decoded or not: what the feed costs in bandwidth,
    /// and the byte window it leaves (0 for none).
    Received {
        bytes: usize,
        at: Instant,
        window_bytes: u32,
    },
    /// A message was replaced by a newer one before it could be decoded.
    Superseded,
    Frame(DecodedFrame),
    Empty { seq: u64 },
    Error(String),
}

type Handler = Arc<dyn Fn(FeedEvent) + Send + Sync>;

/// A running feed. Dropping it stops its threads.
pub struct Feed {
    commands: Sender<Pacing>,
    stop: Arc<AtomicBool>,
    mailbox: Arc<Mailbox>,
}

impl Feed {
    pub fn spawn(
        url: String,
        pacing: Pacing,
        on_event: impl Fn(FeedEvent) + Send + Sync + 'static,
    ) -> Feed {
        let handler: Handler = Arc::new(on_event);
        let (commands, command_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let mailbox = Arc::new(Mailbox::default());
        {
            let (handler, stop, mailbox) = (handler.clone(), stop.clone(), mailbox.clone());
            thread::Builder::new()
                .name(format!("feed {url}"))
                .spawn(move || read_loop(&url, pacing, &command_rx, &stop, &mailbox, &handler))
                .expect("spawning a feed's reader");
        }
        {
            let mailbox = mailbox.clone();
            thread::Builder::new()
                .name("feed decoder".to_owned())
                .spawn(move || decode_loop(&mailbox, &handler))
                .expect("spawning a feed's decoder");
        }
        Feed {
            commands,
            stop,
            mailbox,
        }
    }

    /// Resize the window or change the cap; takes effect straight away.
    pub fn set_pacing(&self, pacing: Pacing) {
        let _ = self.commands.send(pacing);
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.mailbox.close();
    }
}

// =============================================================================
// Reading
// =============================================================================

fn read_loop(
    url: &str,
    mut pacing: Pacing,
    commands: &Receiver<Pacing>,
    stop: &AtomicBool,
    mailbox: &Mailbox,
    handler: &Handler,
) {
    let mut delay = RECONNECT_DELAY;
    while !stop.load(Ordering::Relaxed) {
        handler(FeedEvent::Connection(Connection::Connecting));
        match connect(url) {
            Ok(ws) => {
                delay = RECONNECT_DELAY;
                handler(FeedEvent::Connection(Connection::Open));
                let mut conn = Conn::new(ws, pacing);
                if let Err(e) = conn.run(commands, stop, mailbox, handler) {
                    handler(FeedEvent::Error(format!("{url}: {e}")));
                }
                pacing = conn.pacing;
            }
            Err(e) => handler(FeedEvent::Error(format!("{url}: {e}"))),
        }
        handler(FeedEvent::Connection(Connection::Closed));
        // Sleep in steps, so dropping the feed isn't held up by a long delay.
        let until = Instant::now() + delay;
        while !stop.load(Ordering::Relaxed) && Instant::now() < until {
            thread::sleep(POLL_INTERVAL);
        }
        delay = (delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

fn connect(url: &str) -> tungstenite::Result<Socket> {
    let (ws, _) = tungstenite::connect(url)?;
    if let Some(tcp) = tcp_of(&ws) {
        // PROTOCOL.md: an ack or a frame's tail shouldn't wait for more to send.
        tcp.set_nodelay(true)?;
    }
    Ok(ws)
}

fn tcp_of(ws: &Socket) -> Option<&TcpStream> {
    match ws.get_ref() {
        MaybeTlsStream::Plain(tcp) => Some(tcp),
        MaybeTlsStream::Rustls(tls) => Some(&tls.sock),
        _ => None,
    }
}

/// One connection's side of the window.
struct Conn {
    ws: Socket,
    pacing: Pacing,
    received_frames: u64,
    received_bytes: u64,
    acked_frames: u64,
    acked_bytes: u64,
    last_ack_at: Option<Instant>,
    rate: DeliveryRate,
    opened_at: Instant,
}

impl Conn {
    fn new(ws: Socket, pacing: Pacing) -> Self {
        Self {
            ws,
            pacing,
            received_frames: 0,
            received_bytes: 0,
            acked_frames: 0,
            acked_bytes: 0,
            last_ack_at: None,
            rate: DeliveryRate::new(),
            opened_at: Instant::now(),
        }
    }

    fn window_bytes(&self) -> u32 {
        if self.pacing.target_in_flight > 0.0 {
            self.rate.window_bytes(self.pacing.target_in_flight)
        } else {
            0
        }
    }

    fn run(
        &mut self,
        commands: &Receiver<Pacing>,
        stop: &AtomicBool,
        mailbox: &Mailbox,
        handler: &Handler,
    ) -> tungstenite::Result<()> {
        self.send_ack()?;
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = self.ws.close(None);
                return Ok(());
            }
            loop {
                match commands.try_recv() {
                    Ok(pacing) => {
                        self.pacing = pacing;
                        // Lifting or loosening the cap may make a held ack due.
                        if !self.acknowledge()? {
                            self.send_ack()?;
                        }
                    }
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            self.acknowledge()?;

            let timeout = self.ack_due_in().map_or(POLL_INTERVAL, |d| d.min(POLL_INTERVAL));
            if let Some(tcp) = tcp_of(&self.ws) {
                tcp.set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
            }
            match self.ws.read() {
                Ok(Message::Binary(data)) => {
                    let at = Instant::now();
                    self.rate
                        .note(at.duration_since(self.opened_at).as_secs_f64(), data.len());
                    handler(FeedEvent::Received {
                        bytes: data.len(),
                        at,
                        window_bytes: self.window_bytes(),
                    });
                    // Acknowledge on arrival, before decoding: decoding time
                    // shouldn't count against the window.
                    self.received_frames += 1;
                    self.received_bytes += data.len() as u64;
                    self.acknowledge()?;
                    if mailbox.put(data.to_vec()) {
                        handler(FeedEvent::Superseded);
                    }
                }
                Ok(Message::Text(_)) => {
                    let _ = self.ws.close(Some(CloseFrame {
                        code: CloseCode::Unsupported,
                        reason: "viewcam feeds are binary".into(),
                    }));
                    return Err(tungstenite::Error::Io(io::Error::other(
                        "a text message on a viewcam feed",
                    )));
                }
                Ok(Message::Close(_)) => return Ok(()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return Ok(())
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// How long until a paced ack may go, if one is waiting.
    fn ack_due_in(&self) -> Option<Duration> {
        if self.received_frames == self.acked_frames {
            return None;
        }
        Some(self.next_ack_at().saturating_duration_since(Instant::now()))
    }

    fn next_ack_at(&self) -> Instant {
        match self.last_ack_at {
            Some(last) if self.pacing.max_fps > 0.0 => {
                last + Duration::from_secs_f64(1.0 / self.pacing.max_fps)
            }
            _ => Instant::now(),
        }
    }

    /// Acknowledge what has arrived, if there is anything and it is time.
    /// Returns whether an ack went out.
    fn acknowledge(&mut self) -> tungstenite::Result<bool> {
        if self.received_frames == self.acked_frames || Instant::now() < self.next_ack_at() {
            return Ok(false);
        }
        self.last_ack_at = Some(Instant::now());
        self.acked_frames = self.received_frames;
        self.acked_bytes = self.received_bytes;
        self.send_ack()?;
        Ok(true)
    }

    fn send_ack(&mut self) -> tungstenite::Result<()> {
        let ack = Ack {
            acked_frames: self.acked_frames,
            acked_bytes: self.acked_bytes,
            window_frames: self.pacing.window_frames,
            window_bytes: self.window_bytes(),
        };
        self.ws.send(Message::Binary(ack.encode().to_vec().into()))
    }
}

// =============================================================================
// Decoding
// =============================================================================

/// The newest message waiting to be decoded; a newer one replaces it.
#[derive(Default)]
struct Mailbox {
    slot: Mutex<Slot>,
    ready: Condvar,
}

#[derive(Default)]
struct Slot {
    message: Option<Vec<u8>>,
    closed: bool,
}

impl Mailbox {
    /// Leave `message` for the decoder; returns whether it replaced one.
    fn put(&self, message: Vec<u8>) -> bool {
        let replaced = self.slot.lock().unwrap().message.replace(message).is_some();
        self.ready.notify_one();
        replaced
    }

    /// The next message, or `None` once closed.
    fn take(&self) -> Option<Vec<u8>> {
        let mut slot = self.slot.lock().unwrap();
        loop {
            if slot.closed {
                return None;
            }
            if let Some(message) = slot.message.take() {
                return Some(message);
            }
            slot = self.ready.wait(slot).unwrap();
        }
    }

    fn close(&self) {
        self.slot.lock().unwrap().closed = true;
        self.ready.notify_all();
    }
}

fn decode_loop(mailbox: &Mailbox, handler: &Handler) {
    let mut decoder = Decoder::new();
    while let Some(message) = mailbox.take() {
        match decoder.decode(&message) {
            Ok(Decoded::Frame(frame)) => handler(FeedEvent::Frame(frame)),
            Ok(Decoded::Empty { seq }) => handler(FeedEvent::Empty { seq }),
            Err(e) => handler(FeedEvent::Error(format!("undecodable frame: {e:#}"))),
        }
    }
}
