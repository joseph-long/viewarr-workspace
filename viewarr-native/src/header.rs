//! One tab's header window, as its own OS window.
//!
//! A real viewport rather than an egui window drawn inside the canvas: a
//! header is reference material you want beside the image, on a second
//! monitor if you have one, and two of them side by side when comparing
//! files. That is also why there is one of these per tab rather than one that
//! follows the selection — content changing because you clicked a tab in a
//! *different* window is surprising once the window is genuinely separate.

use crate::fits::{Card, Value};

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
            .with_inner_size([560.0, 420.0])
            // Without a floor, dragging the window narrow grinds the columns
            // down to a character apiece.
            .with_min_inner_size([MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT]);

        // Immediate rather than deferred: a deferred viewport's callback must
        // be `Send + Sync + 'static`, which would mean cloning every card out
        // of the open file each frame. An immediate one borrows the parsed
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
                    .min_width(MIN_WINDOW_WIDTH)
                    .min_height(MIN_WINDOW_HEIGHT)
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

/// Gap between columns.
const COL_GAP: f32 = 12.0;
/// A keyword column may grow to this share of the window before its keywords
/// start wrapping inside their cell. Past that it would starve the other two.
const KEY_SHARE: f32 = 1.0 / 3.0;
/// Floors, in characters, for the two columns that share what the keyword
/// column leaves. The window's minimum size keeps both reachable.
const MIN_VALUE_CH: f32 = 12.0;
const MIN_COMMENT_CH: f32 = 24.0;
/// Width below which the window will not shrink, so dragging it narrow cannot
/// grind the columns down to a character apiece.
pub const MIN_WINDOW_WIDTH: f32 = 420.0;
pub const MIN_WINDOW_HEIGHT: f32 = 200.0;

/// How wide each column should be, and which of them have to wrap to fit.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Columns {
    key: f32,
    value: f32,
    comment: f32,
    wrap_key: bool,
    wrap_value: bool,
}

impl Columns {
    /// Share `avail` between the three columns.
    ///
    /// Keywords grow to fit what is actually there, capped at a third of the
    /// window. Values are the column worth keeping on one line — a wrapped
    /// number or path is markedly harder to read than a wrapped sentence — so
    /// the value takes its natural width and the comment wraps into what is
    /// left. Only when that would squeeze the comment below its floor does
    /// the value wrap too.
    ///
    /// Widths come from character counts rather than laid-out text: the two
    /// columns measured here are monospace, so a count is exact, and a header
    /// of several thousand cards would be costly to lay out twice a frame.
    fn lay_out(
        avail: f32,
        char_width: f32,
        key_chars: usize,
        value_chars: usize,
        has_comments: bool,
    ) -> Self {
        let usable = (avail - 2.0 * COL_GAP).max(0.0);
        let natural_key = key_chars as f32 * char_width;
        let natural_value = value_chars as f32 * char_width;

        let key = natural_key.min(usable * KEY_SHARE);
        let rest = (usable - key).max(0.0);

        let value = if has_comments {
            // Never reserve more for the comment than half of what is left,
            // or a narrow window would leave the value nothing.
            let min_comment = (MIN_COMMENT_CH * char_width).min(rest * 0.5);
            if natural_value + min_comment <= rest {
                natural_value
            } else {
                (rest - min_comment).max((MIN_VALUE_CH * char_width).min(rest))
            }
        } else {
            // Nothing to share with.
            rest
        };
        let comment = (rest - value).max(0.0);

        // Half a pixel of slack, so a column that fits exactly is not told to
        // wrap by floating-point noise.
        Self {
            key,
            value,
            comment,
            wrap_key: natural_key > key + 0.5,
            wrap_value: natural_value > value + 0.5,
        }
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

            let values: Vec<String> = matching
                .iter()
                .map(|c| c.value.as_ref().map(Value::display).unwrap_or_default())
                .collect();

            let mono = egui::TextStyle::Monospace.resolve(ui.style());
            // Any glyph will do in a monospace face; '0' is always present.
            let char_width = ui.ctx().fonts_mut(|f| f.glyph_width(&mono, '0'));
            let key_chars = matching
                .iter()
                .map(|c| c.key.chars().count())
                .max()
                .unwrap_or(0);
            let value_chars = values.iter().map(|v| v.chars().count()).max().unwrap_or(0);
            let has_comments = matching.iter().any(|c| c.comment.is_some());

            let full_width = ui.available_width();
            let cols =
                Columns::lay_out(full_width, char_width, key_chars, value_chars, has_comments);

            let stripe = ui.visuals().faint_bg_color;
            for (row, (card, value)) in matching.iter().zip(&values).enumerate() {
                // Reserve the stripe before the row draws, so it lands behind
                // the text rather than over it.
                let background = ui.painter().add(egui::Shape::Noop);
                let rect = ui
                    .horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = COL_GAP;
                        cell(
                            ui,
                            cols.key,
                            cols.wrap_key,
                            egui::RichText::new(&card.key).monospace().strong(),
                        );
                        cell(
                            ui,
                            cols.value,
                            cols.wrap_value,
                            egui::RichText::new(value).monospace(),
                        );
                        // The comment is prose and always wraps: it is the
                        // column that suffers least from it.
                        cell(
                            ui,
                            cols.comment,
                            true,
                            egui::RichText::new(card.comment.as_deref().unwrap_or_default()).weak(),
                        );
                    })
                    .response
                    .rect;

                if row % 2 == 1 {
                    let band =
                        egui::Rect::from_min_size(rect.min, egui::vec2(full_width, rect.height()));
                    ui.painter().set(
                        background,
                        egui::Shape::rect_filled(band, egui::CornerRadius::ZERO, stripe),
                    );
                }
            }
        });
}

/// One table cell: a fixed width, and text that either wraps inside it or is
/// known to fit. Selectable, since copying a value out is most of what anyone
/// opens a header to do.
fn cell(ui: &mut egui::Ui, width: f32, wrap: bool, text: egui::RichText) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.set_width(width);
            ui.add(egui::Label::new(text).selectable(true).wrap_mode(if wrap {
                egui::TextWrapMode::Wrap
            } else {
                egui::TextWrapMode::Extend
            }));
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A monospace glyph is about this wide at the default text size; the
    /// exact figure does not matter, only that every width scales with it.
    const CH: f32 = 8.0;

    fn lay_out(avail: f32, key_chars: usize, value_chars: usize, comments: bool) -> Columns {
        Columns::lay_out(avail, CH, key_chars, value_chars, comments)
    }

    /// The ordinary case: everything fits, so nothing wraps and the keyword
    /// column is only as wide as the keywords actually are.
    #[test]
    fn columns_take_their_natural_width_when_there_is_room() {
        let cols = lay_out(900.0, 8, 20, true);
        assert_eq!(
            cols.key,
            8.0 * CH,
            "sized to the widest keyword, not padded out"
        );
        assert_eq!(cols.value, 20.0 * CH);
        assert!(!cols.wrap_key);
        assert!(!cols.wrap_value, "a value that fits stays on one line");
        assert!(cols.comment > 0.0, "the comment takes what is left");
    }

    /// A HIERARCH keyword can be long enough to crowd out everything else.
    #[test]
    fn a_long_keyword_stops_at_a_third_of_the_window_and_wraps() {
        let avail = 600.0;
        let cols = lay_out(avail, 200, 10, true);
        let usable = avail - 2.0 * COL_GAP;

        assert!(
            (cols.key - usable * KEY_SHARE).abs() < 0.001,
            "capped at a third: {}",
            cols.key
        );
        assert!(cols.wrap_key, "past the cap it wraps inside its cell");
        assert!(cols.value > 0.0 && cols.comment > 0.0, "neither is starved");
    }

    /// The stated preference: keep the value whole, let the comment wrap.
    #[test]
    fn a_wide_value_is_kept_whole_while_the_comment_wraps() {
        let cols = lay_out(900.0, 8, 60, true);
        assert_eq!(cols.value, 60.0 * CH);
        assert!(!cols.wrap_value);
        assert!(
            cols.comment >= MIN_COMMENT_CH * CH,
            "the comment keeps its floor"
        );
    }

    /// ...until keeping it whole would leave the comment unusable.
    #[test]
    fn a_value_too_wide_to_share_wraps_as_well() {
        let cols = lay_out(600.0, 8, 400, true);
        assert!(
            cols.wrap_value,
            "a 400-character value cannot stay on one line"
        );
        assert!(cols.comment > 0.0, "the comment is squeezed, not removed");
        assert!(cols.value > 0.0);
    }

    /// All three at once, which is the pathological case.
    #[test]
    fn everything_wraps_when_everything_is_too_long() {
        let avail = MIN_WINDOW_WIDTH;
        let cols = lay_out(avail, 200, 400, true);
        assert!(cols.wrap_key && cols.wrap_value);
        for width in [cols.key, cols.value, cols.comment] {
            assert!(width > 0.0, "no column collapses: {cols:?}");
        }
        assert!(
            cols.key + cols.value + cols.comment <= avail - 2.0 * COL_GAP + 0.001,
            "the row still fits the window: {cols:?}"
        );
    }

    /// With nothing in the comment column there is no reason to reserve it.
    #[test]
    fn the_value_takes_the_whole_rest_when_no_card_has_a_comment() {
        let cols = lay_out(600.0, 8, 400, false);
        let usable = 600.0 - 2.0 * COL_GAP;
        assert_eq!(cols.comment, 0.0);
        assert!((cols.key + cols.value - usable).abs() < 0.001);
    }

    /// Nothing may go negative or overflow however absurd the window.
    #[test]
    fn a_window_narrower_than_any_column_still_produces_sane_widths() {
        for avail in [0.0, 1.0, 20.0, COL_GAP * 2.0] {
            let cols = lay_out(avail, 50, 50, true);
            for width in [cols.key, cols.value, cols.comment] {
                assert!(width >= 0.0 && width.is_finite(), "{avail}: {cols:?}");
            }
            assert!(
                cols.key + cols.value + cols.comment <= (avail - 2.0 * COL_GAP).max(0.0) + 0.001,
                "{avail}: {cols:?}"
            );
        }
    }

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
