//! viewcam-native: watch a viewcam camera stream in a native window.
//!
//! The window hosts the same `ViewcamWidget` the browser does, fed by the
//! same protocol, so the two can be compared directly; `--headless` streams
//! without a window and reports throughput, for working on the streaming
//! itself.
//!
//! - `catalog`: a camera's feeds, found from its data feed's URL
//! - `feed`: one feed's WebSocket, acks, pacing and decoding
//! - `decode`: a FRAME message to pixels
//! - `app`: the window
//! - `headless`: the throughput report

pub mod app;
pub mod catalog;
pub mod decode;
pub mod feed;
pub mod headless;

use viewcam::status::FeedRole;
use viewcam_proto::DEFAULT_TARGET_IN_FLIGHT_SEC;

use catalog::{Companions, FeedUrl};
use feed::Pacing;

/// Frames allowed in flight on each feed with turbo on. Six keeps 30 fps
/// going over a 150 ms round trip; the dark changes so rarely that one is
/// plenty. See "Choosing a window" in viewcam-proto/PROTOCOL.md. The same
/// as sup's camera page.
pub fn turbo_window(role: FeedRole) -> u32 {
    match role {
        FeedRole::Data | FeedRole::Saturation => 6,
        FeedRole::Dark => 1,
    }
}

/// Until turbo is switched on, each feed gets at most this many frames a
/// second, with a window of one frame, which is plenty at that rate and
/// keeps a burst of frames from coming through ahead of the pacing.
pub const CAPPED_FPS: f64 = 1.0;

pub fn pacing(role: FeedRole, turbo: bool) -> Pacing {
    if turbo {
        Pacing {
            window_frames: turbo_window(role),
            max_fps: 0.0,
            target_in_flight: DEFAULT_TARGET_IN_FLIGHT_SEC,
        }
    } else {
        Pacing {
            window_frames: 1,
            max_fps: CAPPED_FPS,
            target_in_flight: DEFAULT_TARGET_IN_FLIGHT_SEC,
        }
    }
}

/// One feed to open: its role in the camera view, its stream and URL.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedSpec {
    pub role: FeedRole,
    pub name: String,
    pub url: String,
}

/// The feeds for a camera: its data, and its dark and saturation mask if it
/// has them.
pub fn plan_feeds(url: &FeedUrl, companions: &Companions) -> Vec<FeedSpec> {
    let mut specs = vec![FeedSpec {
        role: FeedRole::Data,
        name: url.name.clone(),
        url: url.sibling(&url.name),
    }];
    for (role, name) in [
        (FeedRole::Dark, &companions.dark),
        (FeedRole::Saturation, &companions.saturation),
    ] {
        if let Some(name) = name {
            specs.push(FeedSpec {
                role,
                name: name.clone(),
                url: url.sibling(name),
            });
        }
    }
    specs
}

pub fn role_name(role: FeedRole) -> &'static str {
    match role {
        FeedRole::Data => "data",
        FeedRole::Dark => "dark",
        FeedRole::Saturation => "saturation",
    }
}
