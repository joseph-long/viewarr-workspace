//! One open file, with its own viewer state.
//!
//! Each tab owns a full `ArrayViewerWidget`, so zoom, stretch, colormap and
//! slice position are per-file and survive switching away and back.

use std::path::Path;

use anyhow::Result;
use viewarr::widget::ArrayViewerWidget;

use crate::fits::FitsFile;

pub struct Tab {
    fits: FitsFile,
    hdu_index: usize,
    widget: ArrayViewerWidget,
    /// Short label for the tab bar: the file's name.
    title: String,
    /// Set when a slice read fails, so the failure is visible in the window
    /// rather than only on stderr.
    error: Option<String>,
}

impl Tab {
    /// Open `path` and load its first plane, so the tab has something to show
    /// the moment it appears.
    pub fn open(path: &Path, requested_hdu: Option<&str>) -> Result<Self> {
        let fits = FitsFile::open(path)?;
        let hdu_index = fits.resolve_hdu(requested_hdu)?;

        let mut widget = ArrayViewerWidget::new();

        let hdu = &fits.hdus[hdu_index];
        let dims = hdu.slice_dims();
        let indices = vec![0usize; dims.len()];
        widget.set_cube(dims);

        let pixels = fits.read_plane(hdu, &indices)?;
        widget.receive_slice(
            indices,
            pixels,
            hdu.width() as u32,
            hdu.height() as u32,
            hdu.is_integer(),
            hdu.value_decimals(),
        );

        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());

        Ok(Self {
            fits,
            hdu_index,
            widget,
            title,
            error: None,
        })
    }

    /// Label for this tab in the tab bar.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Full path, shown on hover where the short title is ambiguous.
    pub fn path_string(&self) -> String {
        self.fits.path.display().to_string()
    }

    /// The HDU's name, as `EXTNAME` gives it (or `PRIMARY`).
    pub fn hdu_name(&self) -> &str {
        &self.fits.hdus[self.hdu_index].name
    }

    /// Shape in FITS axis order, e.g. `200x120x8`.
    pub fn shape_string(&self) -> String {
        self.fits.hdus[self.hdu_index].shape_string()
    }

    /// Storage type name, e.g. `f32`.
    pub fn dtype_name(&self) -> &'static str {
        self.fits.hdus[self.hdu_index].bitpix.name()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether this file has sliceable axes needing the slice-control bar.
    pub fn has_slices(&self) -> bool {
        self.widget.has_slices()
    }

    pub fn show_slice_controls(&mut self, ui: &mut egui::Ui) {
        self.widget.show_slice_controls(ui);
    }

    pub fn show_viewer(&mut self, ui: &mut egui::Ui) {
        let container_size = ui.available_size();
        self.widget.show(ui, container_size);
    }

    /// The image's own size, used to pick a sensible initial window size.
    pub fn image_size(&self) -> (usize, usize) {
        let hdu = &self.fits.hdus[self.hdu_index];
        (hdu.width(), hdu.height())
    }

    /// How many sliceable axes this file has, i.e. how many rows the slice bar
    /// will occupy.
    pub fn slice_axis_count(&self) -> usize {
        self.fits.hdus[self.hdu_index].slice_dims().len()
    }

    /// Fulfil whatever slice the widget is asking for. Reads are from a mapped
    /// file and so complete in-frame; the loop converges because the widget
    /// coalesces to at most one outstanding request, and the bound just
    /// guarantees we always hand control back to the frame.
    pub fn serve_slice_requests(&mut self) {
        for _ in 0..8 {
            let Some(indices) = self.widget.take_slice_request() else {
                return;
            };

            let (result, width, height, is_integer, decimals) = {
                let hdu = &self.fits.hdus[self.hdu_index];
                (
                    self.fits.read_plane(hdu, &indices),
                    hdu.width() as u32,
                    hdu.height() as u32,
                    hdu.is_integer(),
                    hdu.value_decimals(),
                )
            };

            match result {
                Ok(pixels) => {
                    self.error = None;
                    self.widget
                        .receive_slice(indices, pixels, width, height, is_integer, decimals);
                }
                Err(err) => {
                    // Tell the widget the request died, or its coalescing would
                    // wait forever on a reply that is never coming and the
                    // scrubber would stop responding. The previous plane stays
                    // on screen; we just surface why the new one never arrived.
                    let message = format!("could not read slice {indices:?}: {err:#}");
                    eprintln!("viewarr: {message}");
                    self.widget.fail_slice_request(&indices);
                    self.error = Some(message);
                    return;
                }
            }
        }
    }
}
