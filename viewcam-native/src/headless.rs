//! `--headless`: stream a camera without a window and report throughput.
//!
//! Each feed runs exactly as it does behind the window. Decoded frames are
//! then put through what the window does with them on its own thread (the
//! widget's `set_data` and friends, and drawing the viewer, colormap and
//! texture upload included, in an offscreen egui context) unless
//! `--no-display`, so the report can say where a frame's time goes:
//!
//! - **fps**: frames that arrived; **camera**: the rate the camera's own
//!   counter advanced at over the same frames, so the two differ by what
//!   the server never sent;
//! - **MB/s** and the size of a frame on the wire, with its compression;
//! - **decode** and **display**: mean and longest time per frame, on the
//!   feed's decoder thread and the window's thread;
//! - **age**: the median time from when frames were acquired (by the server's
//!   clock) to when they arrived (by this one's), if the server says;
//! - **superseded**: frames that arrived but were replaced by a newer one
//!   before the decoder got to them;
//! - **window**: the byte window last asked for, sized from the delivery
//!   rate, with how many of these frames it lets be in flight at once.
//!   While the link is what limits the rate, that is how many frames are
//!   on their way, and so how far behind the newest each one is.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use viewcam::status::{wall_secs, FeedRole};
use viewcam::widget::ViewcamWidget;

use crate::decode::DecodedFrame;
use crate::feed::{Feed, FeedEvent, Pacing};
use crate::{role_name, FeedSpec};

pub struct Options {
    pub duration: Duration,
    /// Per role; the caller decides between turbo, a cap, or a set window.
    pub pacing: Box<dyn Fn(FeedRole) -> Pacing>,
    pub display: bool,
    /// The viewport the offscreen viewer is drawn into.
    pub viewport: egui::Vec2,
}

/// What a feed did over some span of time.
#[derive(Default, Clone)]
struct Tally {
    messages: u64,
    bytes: u64,
    superseded: u64,
    decoded: u64,
    decoded_bytes: u64,
    decoded_message_bytes: u64,
    decode_total: Duration,
    decode_max: Duration,
    displayed: u64,
    display_total: Duration,
    display_max: Duration,
    /// (seq, when decoded) of the first and last frames.
    first: Option<(u64, Instant)>,
    last: Option<(u64, Instant)>,
    /// Gaps between successive messages' arrivals.
    gaps: Vec<Duration>,
    errors: u64,
    /// The byte window last asked for; 0 for none.
    window_bytes: Option<u32>,
    window_frames: u32,
    /// Seconds from acquisition to arrival, of frames whose acquisition
    /// time the server gave.
    ages: Vec<f64>,
}

impl Tally {
    fn fps(&self, span: Duration) -> f64 {
        self.messages as f64 / span.as_secs_f64()
    }

    /// Frames a second the camera's counter advanced by, between the first
    /// and last frames decoded.
    fn camera_fps(&self) -> Option<f64> {
        let ((s0, t0), (s1, t1)) = (self.first?, self.last?);
        let dt = t1.duration_since(t0).as_secs_f64();
        (s1 > s0 && dt > 0.0).then(|| (s1 - s0) as f64 / dt)
    }

    fn summary(&self, span: Duration, display: bool) -> String {
        let mut line = format!("{:5.1} fps", self.fps(span));
        match self.camera_fps() {
            Some(fps) => line.push_str(&format!("  camera {fps:5.1} fps")),
            None => line.push_str("  camera     - fps"),
        }
        line.push_str(&format!(
            "  {:6.2} MB/s",
            self.bytes as f64 / span.as_secs_f64() / 1e6
        ));
        if self.decoded > 0 {
            let per_frame = self.decoded_message_bytes as f64 / self.decoded as f64;
            let ratio = self.decoded_bytes as f64 / self.decoded_message_bytes as f64;
            line.push_str(&format!(
                "  {:6.1} kB/frame ({ratio:.1}x)  decode {}",
                per_frame / 1e3,
                mean_max(self.decode_total, self.decode_max, self.decoded)
            ));
        }
        if display && self.displayed > 0 {
            line.push_str(&format!(
                "  display {}",
                mean_max(self.display_total, self.display_max, self.displayed)
            ));
        }
        if let (Some(bytes), true) = (self.window_bytes, self.decoded > 0) {
            if bytes == 0 {
                line.push_str("  window -");
            } else {
                let per_frame = self.decoded_message_bytes as f64 / self.decoded as f64;
                // The server always lets one frame go when none is in flight.
                let frames = ((bytes as f64 / per_frame).floor() as u32)
                    .clamp(1, self.window_frames.max(1));
                line.push_str(&format!(
                    "  window {:.0} kB ({frames} frame{})",
                    bytes as f64 / 1e3,
                    if frames == 1 { "" } else { "s" }
                ));
            }
        }
        if !self.ages.is_empty() {
            let mut ages = self.ages.clone();
            ages.sort_by(f64::total_cmp);
            line.push_str(&format!("  age {:.2} s", ages[ages.len() / 2]));
        }
        if self.superseded > 0 {
            line.push_str(&format!("  superseded {}", self.superseded));
        }
        if self.errors > 0 {
            line.push_str(&format!("  errors {}", self.errors));
        }
        line
    }
}

fn mean_max(total: Duration, max: Duration, n: u64) -> String {
    format!(
        "{:5.1}/{:5.1} ms",
        total.as_secs_f64() * 1e3 / n as f64,
        max.as_secs_f64() * 1e3
    )
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

/// The current second's tallies and the whole run's, by role, and the newest
/// decoded frame from each feed waiting to be displayed.
#[derive(Default)]
struct Shared {
    interval: [Tally; 3],
    total: [Tally; 3],
    last_arrival: [Option<Instant>; 3],
    latest: [Option<DecodedFrame>; 3],
}

impl Shared {
    fn record(&mut self, role: FeedRole, event: FeedEvent) {
        let i = role.index();
        match event {
            FeedEvent::Received {
                bytes,
                at,
                window_bytes,
            } => {
                let gap = self.last_arrival[i].replace(at).map(|t| at.duration_since(t));
                for tally in [&mut self.interval[i], &mut self.total[i]] {
                    tally.messages += 1;
                    tally.bytes += bytes as u64;
                    tally.gaps.extend(gap);
                    tally.window_bytes = Some(window_bytes);
                }
            }
            FeedEvent::Superseded => {
                for tally in [&mut self.interval[i], &mut self.total[i]] {
                    tally.superseded += 1;
                }
            }
            FeedEvent::Frame(frame) => {
                let now = Instant::now();
                // Arrival was before decoding began.
                let age = frame
                    .acquired
                    .map(|t| wall_secs() - frame.decode_time.as_secs_f64() - t);
                for tally in [&mut self.interval[i], &mut self.total[i]] {
                    tally.decoded += 1;
                    tally.decoded_bytes += frame.decoded_len as u64;
                    tally.decoded_message_bytes += frame.message_len as u64;
                    tally.ages.extend(age);
                    tally.decode_total += frame.decode_time;
                    tally.decode_max = tally.decode_max.max(frame.decode_time);
                    tally.first.get_or_insert((frame.seq, now));
                    tally.last = Some((frame.seq, now));
                }
                self.latest[i] = Some(frame);
            }
            FeedEvent::Connection(connection) => {
                eprintln!("{}: {connection:?}", role_name(role));
                // Arrivals on a new connection don't follow on from the old one's.
                self.last_arrival[i] = None;
            }
            FeedEvent::Empty { seq } => eprintln!("{}: no image (#{seq})", role_name(role)),
            FeedEvent::Error(message) => {
                eprintln!("{}: {message}", role_name(role));
                for tally in [&mut self.interval[i], &mut self.total[i]] {
                    tally.errors += 1;
                }
            }
        }
    }

    fn record_display(&mut self, role: FeedRole, took: Duration) {
        let i = role.index();
        for tally in [&mut self.interval[i], &mut self.total[i]] {
            tally.displayed += 1;
            tally.display_total += took;
            tally.display_max = tally.display_max.max(took);
        }
    }
}

pub fn run(specs: &[FeedSpec], options: &Options) -> Result<()> {
    let mut shared = Shared::default();
    for spec in specs {
        let window_frames = (options.pacing)(spec.role).window_frames;
        shared.interval[spec.role.index()].window_frames = window_frames;
        shared.total[spec.role.index()].window_frames = window_frames;
    }
    let shared = Arc::new(Mutex::new(shared));
    for spec in specs {
        let pacing = (options.pacing)(spec.role);
        eprintln!(
            "{}: {} (window {} frames, {}{})",
            role_name(spec.role),
            spec.url,
            pacing.window_frames,
            if pacing.target_in_flight > 0.0 {
                format!("bytes for {} s in flight", pacing.target_in_flight)
            } else {
                "no byte limit".to_owned()
            },
            if pacing.max_fps > 0.0 {
                format!(", at most {} fps", pacing.max_fps)
            } else {
                String::new()
            }
        );
    }
    let feeds: Vec<Feed> = specs
        .iter()
        .map(|spec| {
            let (role, shared) = (spec.role, shared.clone());
            Feed::spawn(spec.url.clone(), (options.pacing)(role), move |event| {
                shared.lock().unwrap().record(role, event)
            })
        })
        .collect();

    let mut display = options.display.then(|| Display::new(options.viewport));
    let started = Instant::now();
    let mut next_report = started + Duration::from_secs(1);
    let roles: Vec<FeedRole> = specs.iter().map(|s| s.role).collect();
    while started.elapsed() < options.duration {
        // The window's thread: take the newest frames and show them.
        let frames: Vec<(FeedRole, DecodedFrame)> = {
            let mut shared = shared.lock().unwrap();
            roles
                .iter()
                .filter_map(|&role| shared.latest[role.index()].take().map(|f| (role, f)))
                .collect()
        };
        let idle = frames.is_empty();
        for (role, frame) in frames {
            if let Some(display) = &mut display {
                let took = display.show(role, frame);
                shared.lock().unwrap().record_display(role, took);
            }
        }

        if Instant::now() >= next_report {
            let mut shared = shared.lock().unwrap();
            let t = started.elapsed().as_secs_f64();
            for &role in &roles {
                let interval = &mut shared.interval[role.index()];
                let window_frames = interval.window_frames;
                let tally = std::mem::take(interval);
                interval.window_frames = window_frames;
                println!(
                    "{t:5.1}s {:<10} {}",
                    role_name(role),
                    tally.summary(Duration::from_secs(1), options.display)
                );
            }
            next_report += Duration::from_secs(1);
        }
        if idle {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    drop(feeds);

    let span = started.elapsed();
    let shared = shared.lock().unwrap();
    println!("\nover {:.1} s:", span.as_secs_f64());
    for (spec, &role) in specs.iter().zip(&roles) {
        let tally = &shared.total[role.index()];
        println!(
            "{:<10} {}  {}",
            role_name(role),
            spec.name,
            tally.summary(span, options.display)
        );
        let mut gaps = tally.gaps.clone();
        if !gaps.is_empty() {
            gaps.sort();
            let ms = |d: Duration| d.as_secs_f64() * 1e3;
            println!(
                "{:<10} between frames: p50 {:.1} ms  p90 {:.1} ms  p99 {:.1} ms  max {:.1} ms",
                "",
                ms(percentile(&gaps, 0.5)),
                ms(percentile(&gaps, 0.9)),
                ms(percentile(&gaps, 0.99)),
                ms(*gaps.last().unwrap())
            );
        }
        let mut ages = tally.ages.clone();
        if !ages.is_empty() {
            ages.sort_by(f64::total_cmp);
            let pick = |p: f64| ages[((ages.len() - 1) as f64 * p).round() as usize];
            println!(
                "{:<10} age on arrival: p50 {:.2} s  p90 {:.2} s  max {:.2} s",
                "",
                pick(0.5),
                pick(0.9),
                ages[ages.len() - 1]
            );
        }
    }
    Ok(())
}

/// What the window does with a frame, offscreen.
struct Display {
    widget: ViewcamWidget,
    ctx: egui::Context,
    viewport: egui::Vec2,
}

impl Display {
    fn new(viewport: egui::Vec2) -> Self {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        ctx.set_fonts(fonts);
        let mut display = Self {
            widget: ViewcamWidget::new(),
            ctx,
            viewport,
        };
        // The first frame lays out the fonts, which no later frame repeats.
        display.draw();
        display
    }

    fn show(&mut self, role: FeedRole, decoded: DecodedFrame) -> Duration {
        let started = Instant::now();
        match role {
            FeedRole::Data => self.widget.set_data(decoded.frame),
            FeedRole::Dark => self.widget.set_dark(Some(decoded.frame)),
            FeedRole::Saturation => self.widget.set_saturation(Some(decoded.frame)),
        }
        self.draw();
        started.elapsed()
    }

    fn draw(&mut self) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.viewport)),
            ..Default::default()
        };
        let widget = &mut self.widget;
        // The output has nowhere to go; what counts is the work of making
        // it, tessellation included, as the window would. Only handing it
        // to the GPU is left out.
        let output = self.ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let size = ui.available_size();
                widget.show_viewer(ui, size);
            });
        });
        let _ = self.ctx.tessellate(output.shapes, output.pixels_per_point);
    }
}
