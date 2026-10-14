//! One tab's header window, as its own OS window.
//!
//! A real viewport rather than an egui window drawn inside the canvas: a
//! header is reference material you want beside the image, on a second
//! monitor if you have one, and two of them side by side when comparing
//! files. That is also why there is one of these per tab rather than one that
//! follows the selection — content changing because you clicked a tab in a
//! *different* window is surprising once the window is genuinely separate.

use crate::fits::Card;

/// The header window belonging to one tab.
#[derive(Default)]
pub struct HeaderWindow {
    open: bool,
    /// Kept across opens, so re-opening lands on the same filter.
    filter: String,
}

impl HeaderWindow {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    /// Draw the window, if it is open.
    ///
    /// `cards` is `None` for a source with no metadata to show. That gets a
    /// message rather than a closed window or an empty table: closing would
    /// throw away wherever the user had put the window, and an empty table
    /// reads like a bug rather than an answer.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        id: egui::ViewportId,
        title: &str,
        cards: Option<&[Card]>,
    ) {
        if !self.open {
            return;
        }

        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([560.0, 420.0]);

        // Immediate rather than deferred: a deferred viewport's callback must
        // be `Send + Sync + 'static`, which would mean cloning every card out
        // of the mapped file each frame. An immediate one borrows the parsed
        // header where it already sits.
        ctx.show_viewport_immediate(id, builder, |ctx, class| {
            if class == egui::ViewportClass::Embedded {
                // No real windows on this platform, so egui is drawing us
                // inside the parent. Fall back to an egui window rather than
                // painting over the whole app.
                let mut open = true;
                egui::Window::new(title)
                    .id(id.into())
                    .default_width(560.0)
                    .default_height(420.0)
                    .vscroll(false)
                    .open(&mut open)
                    .show(ctx, |ui| self.contents(ui, cards));
                self.open = open;
            } else {
                egui::CentralPanel::default().show(ctx, |ui| self.contents(ui, cards));
                // The OS close button closes this window, not the app.
                if ctx.input(|i| i.viewport().close_requested()) {
                    self.open = false;
                }
            }
        });
    }

    fn contents(&mut self, ui: &mut egui::Ui, cards: Option<&[Card]>) {
        let Some(cards) = cards else {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                ui.weak("This file holds no header metadata to show.");
            });
            return;
        };

        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("keyword, value or comment")
                    .desired_width(f32::INFINITY),
            );
        });
        show_header_cards(ui, cards, &self.filter);
    }
}

/// Render a header as the file has it: one row per card, in file order, with
/// nothing merged, sorted or hidden. A FITS header is as much a document as it
/// is data — the order of the cards and the comments beside them are how it
/// explains itself — so the only thing the viewer does to it is filter.
fn show_header_cards(ui: &mut egui::Ui, cards: &[Card], filter: &str) {
    let needle = filter.trim().to_ascii_lowercase();
    let matching: Vec<&Card> = cards.iter().filter(|c| c.matches(&needle)).collect();

    ui.horizontal(|ui| {
        ui.label(if needle.is_empty() {
            format!("{} cards", cards.len())
        } else {
            format!("{} of {} cards", matching.len(), cards.len())
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if matching.is_empty() {
                ui.weak("No card matches that.");
                return;
            }
            egui::Grid::new("header_cards")
                .num_columns(3)
                .striped(true)
                .spacing([12.0, 2.0])
                .show(ui, |ui| {
                    for card in matching {
                        // Selectable so values can be copied out, which is
                        // most of what anyone opens a header to do.
                        let cell = |ui: &mut egui::Ui, text: egui::RichText| {
                            ui.add(
                                egui::Label::new(text)
                                    .selectable(true)
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        };
                        cell(ui, egui::RichText::new(&card.key).monospace().strong());
                        match &card.value {
                            Some(value) => {
                                cell(ui, egui::RichText::new(value.display()).monospace())
                            }
                            // COMMENT/HISTORY carry no value; their text goes
                            // in the comment column, so leave this blank
                            // rather than filling it with a placeholder.
                            None => {
                                ui.label("");
                            }
                        }
                        match &card.comment {
                            Some(comment) => cell(ui, egui::RichText::new(comment).weak()),
                            None => {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_window_starts_closed_and_toggles() {
        let mut window = HeaderWindow::default();
        assert!(
            !window.is_open(),
            "opening is the user's call, not a default"
        );
        window.toggle();
        assert!(window.is_open());
        window.toggle();
        assert!(!window.is_open());
    }

    #[test]
    fn the_filter_survives_closing_and_reopening() {
        let mut window = HeaderWindow {
            filter: "naxis".to_string(),
            ..Default::default()
        };
        window.toggle();
        window.toggle();
        assert_eq!(window.filter, "naxis");
    }
}
