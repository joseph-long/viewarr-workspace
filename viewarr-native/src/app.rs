//! The native eframe shell hosting an `ArrayViewerWidget`.
//!
//! This is the desktop counterpart to viewarr's WASM `ViewerApp`: same layout
//! (a slice-control bar above a pannable image), but where the web build routes
//! slice requests out to JavaScript, here we serve them straight from the
//! memory-mapped FITS file.

use anyhow::Result;
use egui_phosphor as phosphor;

use viewarr::widget::ArrayViewerWidget;

use crate::fits::FitsFile;

pub struct NativeViewerApp {
    fits: FitsFile,
    hdu_index: usize,
    widget: ArrayViewerWidget,
    /// Set when a slice read fails, so the failure is visible in the window
    /// rather than only on stderr.
    error: Option<String>,
}

impl NativeViewerApp {
    /// Build the app and load the first plane, so the window has something to
    /// show on its very first frame.
    pub fn new(cc: &eframe::CreationContext<'_>, fits: FitsFile, hdu_index: usize) -> Result<Self> {
        // The widget labels its controls with Phosphor glyphs; without this font
        // every button renders as tofu.
        let mut fonts = egui::FontDefinitions::default();
        phosphor::add_to_fonts(&mut fonts, phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

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

        let file_label = fits
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| fits.path.display().to_string());
        widget.set_overlay_message(&format!("{file_label} · {} · {}", hdu.name, hdu.shape_string()));

        Ok(Self {
            fits,
            hdu_index,
            widget,
            error: None,
        })
    }

    /// Fulfil whatever slice the widget is asking for. Reads are from a mapped
    /// file and so complete in-frame; the loop converges because the widget
    /// coalesces to at most one outstanding request, and the bound just
    /// guarantees we always hand control back to the frame.
    fn serve_slice_requests(&mut self) {
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

impl eframe::App for NativeViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Slice/play controls sit in their own bar above the image, a level up
        // from the image's own overlays. Only shown for cubes.
        if self.widget.has_slices() {
            let slice_frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 2,
                bottom: 3,
            });
            egui::TopBottomPanel::top("slice_controls")
                .frame(slice_frame)
                .show(ctx, |ui| {
                    self.widget.show_slice_controls(ui);
                });
        }

        if let Some(error) = &self.error {
            egui::TopBottomPanel::bottom("error_bar").show(ctx, |ui| {
                ui.colored_label(ui.visuals().error_fg_color, error);
            });
        }

        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(0.0);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let container_size = ui.available_size();
            self.widget.show(ui, container_size);
        });

        // Serve after rendering, so a request raised this frame is satisfied
        // before the next one draws.
        self.serve_slice_requests();

        // Match the web shell: repaint continuously so playback, the zoom
        // indicator fade, and drag feedback all animate smoothly.
        ctx.request_repaint();
    }
}
