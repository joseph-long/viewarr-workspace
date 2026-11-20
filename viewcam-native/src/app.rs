//! The window: a ViewcamWidget fed by the camera's feeds.
//!
//! Laid out like viewcam's browser shell: the camera toolbar in a panel
//! above, the status bar in one below, the viewer filling the rest. The
//! feeds run on threads of their own; what they report reaches the window
//! through a channel, except decoded frames, which wait in one slot per feed
//! so that a window that falls behind shows the newest frame rather than
//! working through a queue.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui_phosphor as phosphor;
use viewcam::status::{now_secs, FeedRole, FrameInfo};
use viewcam::widget::ViewcamWidget;

use crate::decode::{quantized_threshold, DecodedFrame};
use crate::feed::{Feed, FeedEvent};
use crate::{pacing, FeedSpec};

/// How often the window redraws with nothing new, to keep the status bar's
/// bandwidth current.
const IDLE_REPAINT: Duration = Duration::from_millis(250);

/// The newest decoded frame from each feed, by role.
type Latest = [Mutex<Option<DecodedFrame>>; 3];

pub struct CameraApp {
    widget: ViewcamWidget,
    feeds: Vec<(FeedRole, Feed)>,
    events: Receiver<(FeedRole, FeedEvent)>,
    latest: Arc<Latest>,
    turbo: bool,
    /// For a camera without a saturation mask: data at or above this is saturated.
    saturation_level: Option<f64>,
    /// The level last given the widget, which for a quantized frame is the
    /// threshold between codes rather than the level itself.
    applied_level: Option<f64>,
}

impl CameraApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        specs: &[FeedSpec],
        saturation_level: Option<f64>,
        turbo: bool,
    ) -> Self {
        // The viewer draws its controls with Phosphor glyphs.
        let mut fonts = egui::FontDefinitions::default();
        phosphor::add_to_fonts(&mut fonts, phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        let mut widget = ViewcamWidget::new();
        widget.status_mut().set_turbo(turbo);
        let has_saturation_feed = specs.iter().any(|s| s.role == FeedRole::Saturation);
        let saturation_level = saturation_level.filter(|_| !has_saturation_feed);
        if let Some(level) = saturation_level {
            widget.set_saturation_level(Some(level));
        }

        let (tx, events) = mpsc::channel();
        let latest: Arc<Latest> = Arc::new(Default::default());
        let feeds = specs
            .iter()
            .map(|spec| {
                widget.status_mut().set_feed(spec.role, Some(spec.name.clone()));
                let role = spec.role;
                let (ctx, tx, latest) = (cc.egui_ctx.clone(), tx.clone(), latest.clone());
                let feed = Feed::spawn(spec.url.clone(), pacing(role, turbo), move |event| {
                    match event {
                        FeedEvent::Frame(frame) => {
                            *latest[role.index()].lock().unwrap() = Some(frame);
                        }
                        FeedEvent::Superseded => return,
                        other => {
                            let _ = tx.send((role, other));
                        }
                    }
                    ctx.request_repaint();
                });
                (role, feed)
            })
            .collect();

        Self {
            widget,
            feeds,
            events,
            latest,
            turbo,
            saturation_level,
            applied_level: saturation_level,
        }
    }

    /// Bring the widget up to date with what the feeds have reported.
    fn take_feed_news(&mut self) {
        let now = now_secs();
        while let Ok((role, event)) = self.events.try_recv() {
            let status = self.widget.status_mut();
            match event {
                FeedEvent::Connection(connection) => status.set_connection(role, connection),
                FeedEvent::Received { bytes, at, .. } => {
                    status.note_received(bytes, now - at.elapsed().as_secs_f64())
                }
                FeedEvent::Empty { .. } => {
                    status.note_empty(role);
                    match role {
                        FeedRole::Data => {}
                        FeedRole::Dark => self.widget.set_dark(None),
                        FeedRole::Saturation => self.widget.set_saturation(None),
                    }
                }
                FeedEvent::Error(message) => eprintln!("viewcam: {message}"),
                FeedEvent::Frame(_) | FeedEvent::Superseded => {}
            }
        }
        for role in FeedRole::ALL {
            let frame = self.latest[role.index()].lock().unwrap().take();
            if let Some(frame) = frame {
                self.show(role, frame, now);
            }
        }
    }

    fn show(&mut self, role: FeedRole, decoded: DecodedFrame, now: f64) {
        let info = FrameInfo {
            seq: decoded.seq,
            width: decoded.frame.width,
            height: decoded.frame.height,
            dtype: decoded.dtype.to_owned(),
            acquired: decoded.acquired,
        };
        match role {
            FeedRole::Data => {
                self.follow_saturation_level(&decoded);
                self.widget.set_data(decoded.frame);
            }
            FeedRole::Dark => self.widget.set_dark(Some(decoded.frame)),
            FeedRole::Saturation => self.widget.set_saturation(Some(decoded.frame)),
        }
        self.widget.status_mut().note_frame(role, info, now);
    }

    /// A quantized frame's values are rounded, so compare a saturation level
    /// with the threshold between its code and the one below.
    fn follow_saturation_level(&mut self, decoded: &DecodedFrame) {
        let Some(level) = self.saturation_level else {
            return;
        };
        let level = decoded
            .quantization
            .map_or(level, |q| quantized_threshold(level, &q));
        if self.applied_level != Some(level) {
            self.applied_level = Some(level);
            self.widget.set_saturation_level(Some(level));
        }
    }

    fn apply_turbo(&mut self) {
        let turbo = self.widget.status().turbo();
        if turbo == self.turbo {
            return;
        }
        self.turbo = turbo;
        for (role, feed) in &self.feeds {
            feed.set_pacing(pacing(*role, turbo));
        }
    }
}

impl eframe::App for CameraApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.take_feed_news();

        // Same frames as viewcam's browser shell.
        let toolbar_frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 2,
            bottom: 3,
        });
        if self.widget.has_toolbar() {
            egui::TopBottomPanel::top("camera_toolbar")
                .frame(toolbar_frame)
                .show(ctx, |ui| self.widget.show_toolbar(ui));
        }
        let status_frame = toolbar_frame.inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 3,
            bottom: 2,
        });
        egui::TopBottomPanel::bottom("camera_status")
            .frame(status_frame)
            .show(ctx, |ui| self.widget.show_status_bar(ui));
        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(0.0);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let size = ui.available_size();
            self.widget.show_viewer(ui, size);
        });

        self.apply_turbo();
        ctx.request_repaint_after(IDLE_REPAINT);
    }
}
