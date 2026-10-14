//! `viewarr` — open array data from disk in a native window.
//!
//! Currently reads FITS; the viewer itself is the same egui widget the browser
//! and Jupyter front-ends embed.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use viewarr_native::app::{open_paths, NativeViewerApp};
use viewarr_native::fits::FitsFile;
use viewarr_native::tab::Tab;

#[derive(Parser)]
#[command(
    name = "viewarr",
    version,
    about = "View array data from disk",
    long_about = "Open array files in a viewarr window. FITS image HDUs are supported, \
                  including N-dimensional cubes, which get per-axis slice and playback \
                  controls. Several files open as tabs; more can be opened from the window \
                  itself, and with no file named the window opens empty."
)]
struct Cli {
    /// Array files to open (FITS), one tab each
    paths: Vec<PathBuf>,

    /// HDU to display, by index (0 is the primary HDU) or by EXTNAME.
    /// Applies to every file named on the command line.
    #[arg(long)]
    hdu: Option<String>,

    /// List the HDUs in the given files and exit without opening a window
    #[arg(long)]
    list: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("viewarr: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    if cli.list {
        if cli.paths.is_empty() {
            anyhow::bail!("--list needs at least one file to list");
        }
        for path in &cli.paths {
            print_hdu_list(&FitsFile::open(path)?);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let (tabs, errors) = open_paths(&cli.paths, cli.hdu.as_deref());
    for message in &errors {
        eprintln!("viewarr: {message}");
    }
    // Every named file failing is a command-line error worth an exit code, not
    // an empty window the user has to work out the meaning of.
    if tabs.is_empty() && !cli.paths.is_empty() {
        return Ok(ExitCode::FAILURE);
    }

    let title = window_title(&tabs);
    let window_size = initial_window_size(tabs.first());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size(window_size)
            .with_min_inner_size([520.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| Ok(Box::new(NativeViewerApp::new(cc, tabs)))),
    )
    .map_err(|err| anyhow::anyhow!("could not open a window: {err}"))?;

    // A file that failed to open still deserves a non-zero exit, but only once
    // the user has finished with the window.
    Ok(if errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Name the window after the single file it opened with, or just the app when
/// it is showing several (or none) — the tab bar names them individually.
fn window_title(tabs: &[Tab]) -> String {
    match tabs {
        [only] => format!("viewarr — {}", only.title()),
        _ => "viewarr".to_string(),
    }
}

fn print_hdu_list(fits: &FitsFile) {
    println!("{}", fits.path.display());
    for hdu in &fits.hdus {
        let note = match hdu.unviewable_reason() {
            Some(reason) => format!("  ({reason})"),
            None => String::new(),
        };
        println!(
            "  [{}] {:<20} {:<20} {}{note}",
            hdu.index,
            hdu.name,
            hdu.shape_label(),
            hdu.bitpix.name()
        );
    }
}

/// Open at roughly the first image's own size, kept within something that fits
/// on a laptop screen, with room for the chrome stacked above it.
fn initial_window_size(first: Option<&Tab>) -> [f32; 2] {
    let Some(tab) = first else {
        return [900.0, 600.0];
    };
    let (image_width, image_height) = tab.image_size();
    // Button bar, tab bar and file-info bar, plus one row per slice axis.
    let chrome = 3.0 * 30.0 + 28.0 * tab.slice_axis_count() as f32;
    let width = (image_width as f32).clamp(640.0, 1400.0);
    let height = (image_height as f32).clamp(480.0, 1000.0) + chrome;
    [width, height]
}
