//! `viewarr` — open array data from disk in a native window.
//!
//! Currently reads FITS; the viewer itself is the same egui widget the browser
//! and Jupyter front-ends embed.


use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::Parser;

use viewarr_native::app::NativeViewerApp;
use viewarr_native::fits::{FitsFile, Hdu};

#[derive(Parser)]
#[command(
    name = "viewarr",
    version,
    about = "View array data from disk",
    long_about = "Open an array file in a viewarr window. FITS image HDUs are supported, \
                  including N-dimensional cubes, which get per-axis slice and playback controls."
)]
struct Cli {
    /// Array file to open (FITS)
    path: PathBuf,

    /// HDU to display, by index (0 is the primary HDU) or by EXTNAME
    #[arg(long)]
    hdu: Option<String>,

    /// List the HDUs in the file and exit without opening a window
    #[arg(long)]
    list: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("viewarr: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let fits = FitsFile::open(&cli.path)?;

    if cli.list {
        print_hdu_list(&fits);
        return Ok(());
    }

    let hdu_index = resolve_hdu(&fits, cli.hdu.as_deref())?;
    let hdu = &fits.hdus[hdu_index];

    let title = format!(
        "viewarr — {}",
        cli.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| cli.path.display().to_string())
    );
    let window_size = initial_window_size(hdu);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size(window_size)
            .with_min_inner_size([480.0, 360.0]),
        ..Default::default()
    };

    // eframe wants its own error type here, and the app constructor can fail on
    // a bad read, so funnel anyhow's message through.
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            NativeViewerApp::new(cc, fits, hdu_index)
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
                .map_err(|err| format!("{err:#}").into())
        }),
    )
    .map_err(|err| anyhow::anyhow!("could not open a window: {err}"))
}

/// Pick the HDU to display, either the user's choice or the first viewable one.
fn resolve_hdu(fits: &FitsFile, requested: Option<&str>) -> Result<usize> {
    let Some(requested) = requested else {
        return fits.default_image_hdu().ok_or_else(|| no_image_error(fits));
    };

    let index = if let Ok(index) = requested.parse::<usize>() {
        if index >= fits.hdus.len() {
            bail!(
                "{} has no HDU {index} (it has {})",
                fits.path.display(),
                fits.hdus.len()
            );
        }
        index
    } else {
        fits.hdus
            .iter()
            .position(|h| h.name.eq_ignore_ascii_case(requested))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} has no HDU named {requested:?}; run with --list to see what it does have",
                    fits.path.display()
                )
            })?
    };

    let hdu = &fits.hdus[index];
    if hdu.compressed {
        bail!(
            "HDU {index} ({}) is a tile-compressed image ({}), which this build cannot decompress yet",
            hdu.name,
            hdu.shape_string()
        );
    }
    if !hdu.is_viewable_image() {
        bail!(
            "HDU {index} ({}) holds no 2-D-or-larger image data; run with --list to see what the file contains",
            hdu.name
        );
    }
    Ok(index)
}

/// Explain what went wrong when nothing in the file can be displayed, naming
/// compressed HDUs specifically since those are a common and fixable surprise.
fn no_image_error(fits: &FitsFile) -> anyhow::Error {
    let compressed = fits.hdus.iter().filter(|h| h.compressed).count();
    if compressed > 0 {
        anyhow::anyhow!(
            "{} has no uncompressed image HDU ({compressed} tile-compressed image(s) found, \
             which this build cannot decompress yet)",
            fits.path.display()
        )
    } else {
        anyhow::anyhow!(
            "{} has no image HDU with 2 or more axes; run with --list to see what it contains",
            fits.path.display()
        )
    }
}

fn print_hdu_list(fits: &FitsFile) {
    println!("{}", fits.path.display());
    for hdu in &fits.hdus {
        let shape = if hdu.axes.is_empty() {
            "no data".to_string()
        } else {
            hdu.shape_string()
        };
        let note = if hdu.compressed {
            "  (tile-compressed, not yet supported)"
        } else if hdu.is_viewable_image() {
            ""
        } else {
            "  (not a viewable image)"
        };
        println!(
            "  [{}] {:<20} {:<20} {}{note}",
            hdu.index,
            hdu.name,
            shape,
            hdu.bitpix.name()
        );
    }
}

/// Open at roughly the image's own size, kept within something that fits on a
/// laptop screen, with room for the slice bar when there is one.
fn initial_window_size(hdu: &Hdu) -> [f32; 2] {
    let slice_bar = if hdu.slice_dims().is_empty() {
        0.0
    } else {
        28.0 * hdu.slice_dims().len() as f32
    };
    let width = (hdu.width() as f32).clamp(640.0, 1400.0);
    let height = (hdu.height() as f32).clamp(480.0, 1000.0) + slice_bar;
    [width, height]
}
