//! `viewcam` — watch a viewcam camera stream in a native window.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;

use viewcam_native::app::CameraApp;
use viewcam_native::catalog::{fetch_companions, Companions, FeedUrl};
use viewcam_native::feed::Pacing;
use viewcam_native::{headless, pacing, plan_feeds, turbo_window};
use viewcam_proto::DEFAULT_TARGET_IN_FLIGHT_SEC;

#[derive(Parser)]
#[command(
    name = "viewcam",
    version,
    about = "Watch a viewcam camera stream",
    long_about = "Open a camera's viewcam feed in a window, with its dark and saturation \
                  mask, which are looked up in the server's stream catalog. Frames come \
                  at one a second until turbo is switched on in the status bar. With \
                  --headless, stream without a window and report throughput instead."
)]
struct Cli {
    /// The camera's data feed, e.g. wss://exao1.magao-x.org/ws/viewcam/camsci2
    url: String,

    /// The dark's stream, instead of the catalog's
    #[arg(long, value_name = "NAME")]
    dark: Option<String>,

    /// The saturation mask's stream, instead of the catalog's
    #[arg(long, value_name = "NAME")]
    saturation: Option<String>,

    /// For a camera without a saturation mask: data at or above this is
    /// saturated. Instead of the catalog's
    #[arg(long, value_name = "LEVEL")]
    saturation_level: Option<f64>,

    /// Don't look the camera up in the stream catalog
    #[arg(long)]
    no_catalog: bool,

    /// Start with turbo on
    #[arg(long)]
    turbo: bool,

    /// Stream without a window and print throughput each second, then a summary
    #[arg(long)]
    headless: bool,

    /// With --headless: how long to stream, in seconds
    #[arg(long, default_value_t = 10.0, value_name = "SECONDS")]
    duration: f64,

    /// With --headless: frames in flight on the data feed, instead of turbo's
    #[arg(long, value_name = "FRAMES")]
    window: Option<u32>,

    /// With --headless: acknowledge at most this many times a second, as
    /// with turbo off (window 1 unless --window), instead of streaming flat out
    #[arg(long, value_name = "FPS")]
    max_fps: Option<f64>,

    /// With --headless: only receive and decode, without what the window does
    /// with each frame
    #[arg(long)]
    no_display: bool,

    /// With --headless: size the byte window to keep this many seconds of the
    /// delivery rate in flight (0 for no byte limit)
    #[arg(long, default_value_t = DEFAULT_TARGET_IN_FLIGHT_SEC, value_name = "SECONDS")]
    target_in_flight: f64,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("viewcam: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let url = FeedUrl::parse(&cli.url)?;
    let mut companions = if cli.no_catalog {
        Companions::default()
    } else {
        // A camera with no catalog entry, or a server without a catalog,
        // still has its data to show.
        fetch_companions(&url).unwrap_or_else(|e| {
            eprintln!("viewcam: {e:#}; showing the data alone");
            Companions::default()
        })
    };
    if cli.dark.is_some() {
        companions.dark = cli.dark;
    }
    if cli.saturation.is_some() {
        companions.saturation = cli.saturation;
    }
    if cli.saturation_level.is_some() {
        companions.saturation_level = cli.saturation_level;
    }
    let specs = plan_feeds(&url, &companions);

    if cli.headless {
        let (window, max_fps, target) = (
            cli.window,
            cli.max_fps.unwrap_or(0.0),
            cli.target_in_flight,
        );
        let options = headless::Options {
            duration: Duration::from_secs_f64(cli.duration),
            pacing: Box::new(move |role| {
                let mut p = if max_fps > 0.0 {
                    Pacing {
                        window_frames: 1,
                        max_fps,
                        target_in_flight: target,
                    }
                } else {
                    pacing(role, true)
                };
                p.target_in_flight = target;
                if role == viewcam::status::FeedRole::Data {
                    p.window_frames = window.unwrap_or(p.window_frames);
                } else if max_fps == 0.0 {
                    p.window_frames = turbo_window(role);
                }
                p
            }),
            display: !cli.no_display,
            viewport: egui::vec2(720.0, 680.0),
        };
        return headless::run(&specs, &options);
    }

    let title = format!("{} — viewcam", url.name);
    let saturation_level = companions.saturation_level;
    let turbo = cli.turbo;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([720.0, 760.0])
            .with_min_inner_size([400.0, 300.0]),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| Ok(Box::new(CameraApp::new(cc, &specs, saturation_level, turbo)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
