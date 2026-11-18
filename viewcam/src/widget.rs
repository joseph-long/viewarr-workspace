//! ViewcamWidget - a viewarr viewer with a camera toolbar above it
//!
//! Holds the camera inputs in a [`CameraFrames`] and keeps the embedded
//! [`ArrayViewerWidget`] showing what they add up to: the data (dark-subtracted
//! while that toggle is on) as the image, and the saturation mask as a
//! mask overlay in a colour the colormap does not use. Because the viewer is
//! handed the subtracted pixels, its hover readout, limits and histogram all
//! describe the dark-subtracted image too.

use egui::{Color32, Response, Ui, Vec2};
use viewarr::widget::ArrayViewerWidget;

use crate::camera::{Availability, CameraFrames, Frame};

/// Default colour for saturated pixels: red sits outside the default (gray)
/// colormap and reads as "bad" at a glance.
pub const DEFAULT_SATURATION_COLOR: Color32 = Color32::RED;

pub struct ViewcamWidget {
    viewer: ArrayViewerWidget,
    frames: CameraFrames,
    /// Whether the user wants the dark subtracted. Stays set while no usable
    /// dark is loaded, so it takes effect as soon as one is.
    dark_subtraction: bool,
    /// Whether the user wants saturated pixels shown; held the same way.
    saturation_overlay: bool,
}

impl Default for ViewcamWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewcamWidget {
    pub fn new() -> Self {
        let mut viewer = ArrayViewerWidget::new();
        viewer.set_mask_overlay_color(DEFAULT_SATURATION_COLOR);
        Self {
            viewer,
            frames: CameraFrames::new(),
            dark_subtraction: true,
            saturation_overlay: true,
        }
    }

    /// The embedded viewer, for everything that is not camera-specific.
    pub fn viewer(&self) -> &ArrayViewerWidget {
        &self.viewer
    }

    pub fn viewer_mut(&mut self) -> &mut ArrayViewerWidget {
        &mut self.viewer
    }

    pub fn frames(&self) -> &CameraFrames {
        &self.frames
    }

    // =========================================================================
    // Inputs
    // =========================================================================

    /// Show a new data frame, dark-subtracted if that is on.
    pub fn set_data(&mut self, frame: Frame) {
        let old_shape = self.frames.data().map(|d| (d.width, d.height));
        let new_shape = (frame.width, frame.height);
        self.frames.set_data(frame);
        self.refresh_image();
        // Whether the saturation mask fits depends on the data's shape, so it
        // only needs revisiting when that changes. Skipping it otherwise keeps
        // a stream of frames from re-uploading an unchanged mask every time.
        if old_shape != Some(new_shape) {
            self.refresh_mask();
        }
    }

    /// Set (or with `None`, clear) the dark frame.
    pub fn set_dark(&mut self, frame: Option<Frame>) {
        self.frames.set_dark(frame);
        if self.dark_subtraction {
            self.refresh_image();
        }
    }

    /// Set (or with `None`, clear) the saturation image; nonzero = saturated.
    pub fn set_saturation(&mut self, frame: Option<Frame>) {
        self.frames.set_saturation(frame);
        self.refresh_mask();
    }

    // =========================================================================
    // Toggles
    // =========================================================================

    pub fn dark_subtraction(&self) -> bool {
        self.dark_subtraction
    }

    pub fn set_dark_subtraction(&mut self, enabled: bool) {
        if enabled != self.dark_subtraction {
            self.dark_subtraction = enabled;
            self.refresh_image();
        }
    }

    pub fn saturation_overlay(&self) -> bool {
        self.saturation_overlay
    }

    pub fn set_saturation_overlay(&mut self, enabled: bool) {
        if enabled != self.saturation_overlay {
            self.saturation_overlay = enabled;
            self.refresh_mask();
        }
    }

    pub fn saturation_color(&self) -> Color32 {
        self.viewer.mask_overlay_color()
    }

    pub fn set_saturation_color(&mut self, color: Color32) {
        self.viewer.set_mask_overlay_color(color);
    }

    // =========================================================================
    // Keeping the viewer in step
    // =========================================================================

    fn refresh_image(&mut self) {
        if let Some(frame) = self.frames.display_image(self.dark_subtraction) {
            self.viewer.set_image(
                frame.pixels,
                frame.width,
                frame.height,
                frame.is_integer,
                frame.value_decimals,
            );
        }
    }

    fn refresh_mask(&mut self) {
        match self.frames.applicable_saturation() {
            Some(sat) if self.saturation_overlay => {
                self.viewer
                    .set_mask_overlay(sat.mask.clone(), sat.width, sat.height)
            }
            _ => self.viewer.clear_mask_overlay(),
        }
    }

    // =========================================================================
    // Rendering
    // =========================================================================

    /// Whether there is a toolbar to show. A camera with no dark or no
    /// saturation image is normal, not a fault, so a toggle appears only once
    /// its image has arrived, and with neither there is no toolbar at all.
    pub fn has_toolbar(&self) -> bool {
        self.frames.dark_status() != Availability::Missing
            || self.frames.saturation_status() != Availability::Missing
    }

    /// The camera toolbar: one toggle per correction whose image has arrived,
    /// disabled (with the reason on hover) while that image is the wrong
    /// shape for the data.
    pub fn show_toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let dark = self.frames.dark_status();
            if dark != Availability::Missing {
                let resp = ui
                    .add_enabled(
                        dark.is_ready(),
                        egui::Button::selectable(
                            self.dark_subtraction && dark.is_ready(),
                            "Subtract dark",
                        ),
                    )
                    .on_hover_text("Subtract the dark frame from the image (hover values too)")
                    .on_disabled_hover_text(unavailable_reason("dark frame", dark));
                if resp.clicked() {
                    self.set_dark_subtraction(!self.dark_subtraction);
                }
            }

            let sat = self.frames.saturation_status();
            if sat == Availability::Missing {
                return;
            }
            if dark != Availability::Missing {
                ui.separator();
            }
            let resp = ui
                .add_enabled(
                    sat.is_ready(),
                    egui::Button::selectable(
                        self.saturation_overlay && sat.is_ready(),
                        "Saturation",
                    ),
                )
                .on_hover_text("Paint saturated pixels in the overlay colour")
                .on_disabled_hover_text(unavailable_reason("saturation image", sat));
            if resp.clicked() {
                self.set_saturation_overlay(!self.saturation_overlay);
            }

            ui.add_enabled_ui(sat.is_ready(), |ui| {
                let mut color = self.saturation_color();
                if egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut color,
                    egui::color_picker::Alpha::Opaque,
                )
                .on_hover_text("Saturation overlay colour")
                .changed()
                {
                    self.set_saturation_color(color);
                }
            });
        });
    }

    /// The embedded viewer, filling `container_size`.
    pub fn show_viewer(&mut self, ui: &mut Ui, container_size: Vec2) -> Response {
        self.viewer.show(ui, container_size)
    }
}

fn unavailable_reason(what: &str, status: Availability) -> String {
    match status {
        Availability::Ready => String::new(),
        Availability::Missing => format!("No {what} loaded"),
        Availability::ShapeMismatch { aux, data } => format!(
            "The {what} is {}×{} but the image is {}×{}",
            aux.0, aux.1, data.0, data.1
        ),
    }
}

/// Parse a CSS-style `#rrggbb` colour (the `#` is optional).
pub fn parse_hex_color(text: &str) -> Option<Color32> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Color32::from_rgb(channel(0)?, channel(2)?, channel(4)?))
}

/// Format a colour as CSS-style `#rrggbb`, dropping alpha.
pub fn format_hex_color(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_round_trip() {
        assert_eq!(parse_hex_color("#ff0000"), Some(Color32::RED));
        assert_eq!(
            parse_hex_color("00FF7f"),
            Some(Color32::from_rgb(0, 255, 127))
        );
        assert_eq!(format_hex_color(Color32::from_rgb(0, 255, 127)), "#00ff7f");
        assert_eq!(parse_hex_color("#f00"), None);
        assert_eq!(parse_hex_color("#gg0000"), None);
    }

    fn frame(pixels: &[f64], width: u32, height: u32) -> Frame {
        Frame {
            pixels: pixels.to_vec(),
            width,
            height,
            is_integer: true,
            value_decimals: 0,
        }
    }

    #[test]
    fn toggling_dark_reaches_the_hover_values_source() {
        let mut w = ViewcamWidget::new();
        w.set_dark(Some(frame(&[1.0; 4], 2, 2)));
        w.set_data(frame(&[10.0; 4], 2, 2));
        assert_eq!(w.viewer().value_range(), (9.0, 10.0));

        w.set_dark_subtraction(false);
        assert_eq!(w.viewer().value_range(), (10.0, 11.0));
    }

    #[test]
    fn mask_follows_toggle_and_shape() {
        let mut w = ViewcamWidget::new();
        w.set_data(frame(&[0.0; 4], 2, 2));
        w.set_saturation(Some(frame(&[0.0, 1.0, 0.0, 0.0], 2, 2)));
        assert!(w.viewer().has_mask_overlay());

        w.set_saturation_overlay(false);
        assert!(!w.viewer().has_mask_overlay());
        w.set_saturation_overlay(true);
        assert!(w.viewer().has_mask_overlay());

        // New data of another shape: the mask no longer fits, so it goes.
        w.set_data(frame(&[0.0; 6], 3, 2));
        assert!(!w.viewer().has_mask_overlay());
    }

    #[test]
    fn toolbar_appears_once_a_dark_or_saturation_image_does() {
        let mut w = ViewcamWidget::new();
        w.set_data(frame(&[0.0; 4], 2, 2));
        assert!(!w.has_toolbar());

        w.set_saturation(Some(frame(&[0.0; 4], 2, 2)));
        assert!(w.has_toolbar());
        w.set_saturation(None);
        assert!(!w.has_toolbar());

        // One of the wrong shape still shows, disabled, to say why.
        w.set_dark(Some(frame(&[0.0; 6], 3, 2)));
        assert!(w.has_toolbar());
    }
}
