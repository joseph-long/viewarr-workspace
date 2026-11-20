//! StatusBar - what a camera's feeds are doing, below the image
//!
//! One entry per feed (data, dark, saturation): its connection, and once
//! frames arrive their shape, dtype, sequence number and rate; then the
//! bandwidth of all of them together, and the turbo toggle. The bar only
//! shows what it is told: whatever receives the feeds (the browser's
//! viewcam-proto client, or a native one) reports connections, frames and
//! bytes as they happen. What turbo means is up to it too; the bar only
//! holds the toggle.

use std::collections::VecDeque;

use egui::{Color32, RichText, Ui};
use egui_phosphor as phosphor;

/// Over how many frames a feed's frame rate is averaged.
const FPS_FRAMES: usize = 10;
/// Over how long the bandwidth is averaged.
const BANDWIDTH_WINDOW_SEC: f64 = 5.0;
/// How often the bandwidth shown is updated, so it can be read.
const BANDWIDTH_UPDATE_SEC: f64 = 1.0;

/// Feed names, in the blue sup uses for names.
const NAME_COLOR: Color32 = Color32::from_rgb(0x3d, 0xae, 0xe9);

/// Seconds on a monotonic clock, for timing what the feeds report.
#[cfg(target_arch = "wasm32")]
pub fn now_secs() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or_else(js_sys::Date::now, |p| p.now())
        / 1000.0
}

/// Seconds on a monotonic clock, for timing what the feeds report.
#[cfg(not(target_arch = "wasm32"))]
pub fn now_secs() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FeedRole {
    Data,
    Dark,
    Saturation,
}

impl FeedRole {
    /// In the order the bar shows them.
    pub const ALL: [FeedRole; 3] = [FeedRole::Data, FeedRole::Dark, FeedRole::Saturation];

    /// From "data", "dark" or "saturation".
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "data" => Some(FeedRole::Data),
            "dark" => Some(FeedRole::Dark),
            "saturation" => Some(FeedRole::Saturation),
            _ => None,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Connection {
    Connecting,
    Open,
    Closed,
}

impl Connection {
    /// From "connecting", "open" or "closed".
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "connecting" => Some(Connection::Connecting),
            "open" => Some(Connection::Open),
            "closed" => Some(Connection::Closed),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Connection::Connecting => "connecting",
            Connection::Open => "open",
            Connection::Closed => "closed",
        }
    }
}

/// The latest frame shown from a feed.
#[derive(Clone, PartialEq, Debug)]
pub struct FrameInfo {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub dtype: String,
}

#[derive(Clone, Debug)]
pub struct FeedStatus {
    pub name: String,
    pub connection: Connection,
    /// The feed says it has no image.
    pub empty: bool,
    pub frame: Option<FrameInfo>,
    /// When the last few frames were shown, oldest first.
    arrivals: VecDeque<f64>,
}

impl FeedStatus {
    fn new(name: String) -> Self {
        Self {
            name,
            connection: Connection::Connecting,
            empty: false,
            frame: None,
            arrivals: VecDeque::new(),
        }
    }

    /// Frames a second over the last few shown, or 0 before there are two.
    pub fn fps(&self) -> f64 {
        match (self.arrivals.front(), self.arrivals.back()) {
            (Some(first), Some(last)) if last > first => {
                (self.arrivals.len() - 1) as f64 / (last - first)
            }
            _ => 0.0,
        }
    }

    /// A dark or saturation image is optional: many cameras have neither,
    /// so those feeds are listed only while they have an image. The data
    /// feed is always listed, as the one that says whether the camera is there.
    fn listed(&self, role: FeedRole) -> bool {
        role == FeedRole::Data || (!self.empty && self.frame.is_some())
    }

    fn detail(&self, role: FeedRole) -> String {
        if self.connection != Connection::Open {
            return self.connection.label().to_owned();
        }
        if self.empty {
            return "no image".to_owned();
        }
        let Some(frame) = &self.frame else {
            return "waiting".to_owned();
        };
        let mut detail = format!(
            "{}×{} {} · #{}",
            frame.width, frame.height, frame.dtype, frame.seq
        );
        // The dark changes too rarely for a rate to mean anything.
        if role != FeedRole::Dark {
            detail.push_str(&format!(" · {:.1} fps", self.fps()));
        }
        detail
    }
}

pub struct StatusBar {
    feeds: [Option<FeedStatus>; 3],
    /// (time, bytes) of each message received on any feed, oldest first.
    received: VecDeque<(f64, usize)>,
    opened_at: f64,
    bandwidth: f64,
    bandwidth_at: f64,
    turbo: bool,
}

impl StatusBar {
    /// A bar with no feeds yet, its bandwidth averaged from `now`.
    pub fn new(now: f64) -> Self {
        Self {
            feeds: [None, None, None],
            received: VecDeque::new(),
            opened_at: now,
            bandwidth: 0.0,
            bandwidth_at: f64::NEG_INFINITY,
            turbo: false,
        }
    }

    /// Whether there is anything to show: a bar with no feeds has none.
    pub fn has_feeds(&self) -> bool {
        self.feeds.iter().any(Option::is_some)
    }

    pub fn feed(&self, role: FeedRole) -> Option<&FeedStatus> {
        self.feeds[role.index()].as_ref()
    }

    /// Name the shmim (or whatever) a role's feed comes from, connecting;
    /// `None` removes the feed.
    pub fn set_feed(&mut self, role: FeedRole, name: Option<String>) {
        self.feeds[role.index()] = name.map(FeedStatus::new);
    }

    /// A feed's connection changed. Its rate starts over, since frames from
    /// before the change say nothing of the rate after it.
    pub fn set_connection(&mut self, role: FeedRole, connection: Connection) {
        if let Some(feed) = &mut self.feeds[role.index()] {
            feed.connection = connection;
            feed.arrivals.clear();
        }
    }

    /// A feed's frame was shown at `now`.
    pub fn note_frame(&mut self, role: FeedRole, frame: FrameInfo, now: f64) {
        if let Some(feed) = &mut self.feeds[role.index()] {
            feed.empty = false;
            feed.frame = Some(frame);
            feed.arrivals.push_back(now);
            while feed.arrivals.len() > FPS_FRAMES {
                feed.arrivals.pop_front();
            }
        }
    }

    /// A feed said it has no image.
    pub fn note_empty(&mut self, role: FeedRole) {
        if let Some(feed) = &mut self.feeds[role.index()] {
            feed.empty = true;
            feed.frame = None;
        }
    }

    /// A message of `bytes` arrived at `now` on any feed, whether it was
    /// shown or not: what the feeds cost in bandwidth.
    pub fn note_received(&mut self, bytes: usize, now: f64) {
        self.received.push_back((now, bytes));
        self.forget_before(now - BANDWIDTH_WINDOW_SEC);
    }

    fn forget_before(&mut self, since: f64) {
        while self.received.front().is_some_and(|&(t, _)| t < since) {
            self.received.pop_front();
        }
    }

    /// Bytes a second received over the last few seconds, updated at most
    /// once a second. It is averaged over the whole window, or since the
    /// bar was made if sooner, so a quiet spell counts as quiet rather than
    /// being skipped over.
    pub fn bandwidth(&mut self, now: f64) -> f64 {
        if now - self.bandwidth_at >= BANDWIDTH_UPDATE_SEC {
            self.forget_before(now - BANDWIDTH_WINDOW_SEC);
            let span = BANDWIDTH_WINDOW_SEC.min(now - self.opened_at);
            let bytes: usize = self.received.iter().map(|&(_, n)| n).sum();
            self.bandwidth = if span > 0.0 { bytes as f64 / span } else { 0.0 };
            self.bandwidth_at = now;
        }
        self.bandwidth
    }

    pub fn turbo(&self) -> bool {
        self.turbo
    }

    pub fn set_turbo(&mut self, turbo: bool) {
        self.turbo = turbo;
    }

    /// The bar: the feeds from the left, turbo and the bandwidth at the right.
    pub fn show(&mut self, ui: &mut Ui, now: f64) {
        let bandwidth = self.bandwidth(now);
        ui.horizontal(|ui| {
            for role in FeedRole::ALL {
                let Some(feed) = self.feed(role) else {
                    continue;
                };
                if !feed.listed(role) {
                    continue;
                }
                let color = if feed.connection == Connection::Open {
                    ui.visuals().weak_text_color()
                } else {
                    ui.visuals().warn_fg_color
                };
                ui.label(mono(&feed.name).color(NAME_COLOR));
                ui.label(mono(&feed.detail(role)).color(color));
                ui.add_space(12.0);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(mono(&format_rate(bandwidth)).color(ui.visuals().weak_text_color()))
                    .on_hover_text("Received over the last few seconds, all feeds together");
                ui.label(mono("total").color(NAME_COLOR));
                ui.add_space(12.0);
                let turbo = egui::Button::selectable(
                    self.turbo,
                    mono(&format!("{} turbo", phosphor::regular::LIGHTNING)),
                );
                let hover = if self.turbo {
                    "Back to the slower frame rate"
                } else {
                    "As many frames as the link and server allow"
                };
                if ui.add(turbo).on_hover_text(hover).clicked() {
                    self.turbo = !self.turbo;
                }
            });
        });
    }
}

fn mono(text: &str) -> RichText {
    RichText::new(text).monospace().small()
}

/// A rate in bytes a second, as bits a second.
pub fn format_rate(bytes_per_sec: f64) -> String {
    let bits = 8.0 * bytes_per_sec;
    if bits >= 1e6 {
        format!("{:.1} Mbit/s", bits / 1e6)
    } else if bits >= 1e3 {
        format!("{:.1} kbit/s", bits / 1e3)
    } else {
        format!("{bits:.0} bit/s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u64) -> FrameInfo {
        FrameInfo {
            seq,
            width: 4,
            height: 2,
            dtype: "u16".to_owned(),
        }
    }

    #[test]
    fn fps_is_over_the_last_few_frames_and_restarts_with_the_connection() {
        let mut bar = StatusBar::new(0.0);
        bar.set_feed(FeedRole::Data, Some("camwfs".to_owned()));
        bar.set_connection(FeedRole::Data, Connection::Open);
        bar.note_frame(FeedRole::Data, frame(1), 0.0);
        assert_eq!(bar.feed(FeedRole::Data).unwrap().fps(), 0.0);
        // Slow at first, then 20 a second: only the last FPS_FRAMES count.
        for i in 0..FPS_FRAMES {
            bar.note_frame(FeedRole::Data, frame(2 + i as u64), 10.0 + 0.05 * i as f64);
        }
        assert!((bar.feed(FeedRole::Data).unwrap().fps() - 20.0).abs() < 1e-9);
        assert_eq!(
            bar.feed(FeedRole::Data).unwrap().detail(FeedRole::Data),
            "4×2 u16 · #11 · 20.0 fps"
        );

        bar.set_connection(FeedRole::Data, Connection::Closed);
        assert_eq!(bar.feed(FeedRole::Data).unwrap().fps(), 0.0);
        assert_eq!(
            bar.feed(FeedRole::Data).unwrap().detail(FeedRole::Data),
            "closed"
        );
    }

    #[test]
    fn a_dark_or_mask_is_listed_only_while_it_has_an_image() {
        let mut bar = StatusBar::new(0.0);
        assert!(!bar.has_feeds());
        bar.set_feed(FeedRole::Data, Some("camwfs".to_owned()));
        bar.set_feed(FeedRole::Dark, Some("camwfs_dark".to_owned()));
        assert!(bar.has_feeds());
        let listed = |bar: &StatusBar, role| bar.feed(role).unwrap().listed(role);
        assert!(listed(&bar, FeedRole::Data));
        assert!(!listed(&bar, FeedRole::Dark));

        bar.set_connection(FeedRole::Dark, Connection::Open);
        bar.note_frame(FeedRole::Dark, frame(1), 1.0);
        assert!(listed(&bar, FeedRole::Dark));
        assert_eq!(
            bar.feed(FeedRole::Dark).unwrap().detail(FeedRole::Dark),
            "4×2 u16 · #1"
        );
        bar.note_empty(FeedRole::Dark);
        assert!(!listed(&bar, FeedRole::Dark));

        bar.note_empty(FeedRole::Data);
        bar.set_connection(FeedRole::Data, Connection::Open);
        assert_eq!(
            bar.feed(FeedRole::Data).unwrap().detail(FeedRole::Data),
            "no image"
        );
    }

    #[test]
    fn bandwidth_averages_the_window_and_updates_once_a_second() {
        let mut bar = StatusBar::new(0.0);
        bar.note_received(1000, 0.5);
        // Averaged since the bar was made, being under the window.
        assert_eq!(bar.bandwidth(2.0), 500.0);
        bar.note_received(1000, 2.2);
        // Not a second since the last update: unchanged.
        assert_eq!(bar.bandwidth(2.5), 500.0);
        assert_eq!(bar.bandwidth(4.0), 500.0);
        // The first message has left the window.
        assert_eq!(bar.bandwidth(6.0), 200.0);
        assert_eq!(bar.bandwidth(10.0), 0.0);
    }

    #[test]
    fn rates_read_in_bits() {
        assert_eq!(format_rate(0.0), "0 bit/s");
        assert_eq!(format_rate(1500.0), "12.0 kbit/s");
        assert_eq!(format_rate(2.5e6), "20.0 Mbit/s");
    }
}
