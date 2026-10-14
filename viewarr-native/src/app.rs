//! The native eframe shell: a button bar, a tab bar, and per-tab content.
//!
//! Chrome that belongs to the application (open buttons, which files are open)
//! lives above the viewer rather than on top of it, so the canvas stays pure
//! image. The layout nests outward-in:
//!
//! ```text
//! ┌ button bar ────────── application: acts on whichever tab is active
//! ├ tab bar ───────────── application: which files are open
//! ├ file info ─────────── per tab: swaps when the active tab changes
//! ├ slice controls ───── per tab: only for files with sliceable axes
//! └ viewer canvas ────── per tab
//! ```

use std::path::{Path, PathBuf};

use egui_phosphor::regular as phosphor;

use crate::fits::Card;
use crate::source::{ArrayId, Children, Node};
use crate::tab::Tab;

/// Render a header as the file has it: one row per card, in file order, with
/// nothing merged, sorted or hidden. A FITS header is as much a document as it
/// is data -- the order of the cards and the comments beside them are how it
/// explains itself -- so the only thing the viewer does to it is filter.
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

/// Render one level of the picker's tree, recursing into groups.
///
/// A FITS file never gets past the first arm — every HDU is top-level — but a
/// Zarr store's groups would, and the picker should not need to know which
/// source it is drawing.
fn show_array_nodes(
    ui: &mut egui::Ui,
    nodes: &[Node],
    current: &ArrayId,
    chosen: &mut Option<ArrayId>,
) {
    for node in nodes {
        match node {
            Node::Array(meta) => match meta.unviewable_reason {
                None => {
                    if ui
                        .selectable_label(meta.id == *current, meta.label())
                        .clicked()
                    {
                        *chosen = Some(meta.id.clone());
                    }
                }
                // Shown but unselectable, so the source's full contents stay
                // visible and the reason is one hover away.
                Some(reason) => {
                    ui.add_enabled(false, egui::Button::selectable(false, meta.label()))
                        .on_hover_text(reason);
                }
            },
            Node::Group { name, children } => match children {
                Children::Loaded(kids) => {
                    ui.menu_button(name, |ui| show_array_nodes(ui, kids, current, chosen));
                }
                // Its contents are not known yet, so there is nothing to
                // offer and nothing to pretend about.
                Children::Unexpanded => {
                    ui.add_enabled(false, egui::Button::new(format!("{name}  …")))
                        .on_hover_text("Not loaded yet");
                }
            },
        }
    }
}

/// What a file-picking button asked for.
enum OpenMode {
    /// Replace the active tab's contents, as a plain "Open" does.
    Replace,
    /// Add a tab and switch to it.
    NewTab,
}

pub struct NativeViewerApp {
    tabs: Vec<Tab>,
    active: usize,
    /// Whether the header window is up. It always shows the active tab's
    /// current array, so it follows the picker without any state of its own.
    header_open: bool,
    /// Filter text for that window, kept across opens.
    header_filter: String,
    /// Why the last open attempt failed. Shown in the button bar, since at that
    /// point there may be no tab to attach it to.
    open_error: Option<String>,
}

impl NativeViewerApp {
    /// Build the app around whatever files the command line named. An empty
    /// list is fine — the window opens with the Open button and nothing else.
    pub fn new(cc: &eframe::CreationContext<'_>, tabs: Vec<Tab>) -> Self {
        // The widget labels its controls with Phosphor glyphs; without this font
        // every button renders as tofu.
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        Self::with_tabs(tabs)
    }

    /// The tab bookkeeping on its own, with no egui context — which is also
    /// what lets the tab behaviour below be tested without a window.
    fn with_tabs(tabs: Vec<Tab>) -> Self {
        Self {
            tabs,
            active: 0,
            header_open: false,
            header_filter: String::new(),
            open_error: None,
        }
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    /// Ask for a file and open it. A cancelled dialog is not an error and
    /// leaves everything as it was.
    fn pick_and_open(&mut self, mode: OpenMode) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open array file")
            .add_filter("FITS", &["fits", "fit", "fts"])
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            return;
        };
        self.open_path(&path, mode);
    }

    /// Open `path`, reporting a failure in the button bar rather than closing
    /// or disturbing whatever is already open.
    fn open_path(&mut self, path: &Path, mode: OpenMode) {
        match Tab::open(path, None) {
            Ok(tab) => {
                self.open_error = None;
                match mode {
                    OpenMode::Replace if !self.tabs.is_empty() => {
                        self.tabs[self.active] = tab;
                    }
                    // With nothing open yet, "Open" has no tab to replace, so
                    // it opens the first one.
                    _ => {
                        self.tabs.push(tab);
                        self.active = self.tabs.len() - 1;
                    }
                }
            }
            Err(err) => {
                let message = format!("{err:#}");
                eprintln!("viewarr: {message}");
                self.open_error = Some(message);
            }
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        // Keep the selection on a real tab, preferring the one that slid into
        // the closed tab's place.
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }

    /// Step the active tab, wrapping around at both ends.
    fn switch_tab(&mut self, forward: bool) {
        let count = self.tabs.len();
        if count < 2 {
            return;
        }
        self.active = if forward {
            (self.active + 1) % count
        } else {
            (self.active + count - 1) % count
        };
    }

    /// Cmd+Shift+Arrow on macOS, Ctrl+Shift+Arrow elsewhere — `COMMAND` is
    /// egui's per-platform spelling of exactly that. Consumed so the keystroke
    /// does not also reach the viewer widget.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let shortcut = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let (prev, next) = ctx.input_mut(|i| {
            (
                i.consume_key(shortcut, egui::Key::ArrowLeft),
                i.consume_key(shortcut, egui::Key::ArrowRight),
            )
        });
        if prev {
            self.switch_tab(false);
        }
        if next {
            self.switch_tab(true);
        }
    }

    /// Application chrome: acts on whichever tab is active.
    fn show_button_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("button_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button(format!("{} Open…", phosphor::FOLDER_OPEN))
                    .on_hover_text("Replace the contents of the current tab")
                    .clicked()
                {
                    self.pick_and_open(OpenMode::Replace);
                }
                if ui
                    .button(format!("{} Open in Tab", phosphor::PLUS))
                    .on_hover_text("Open a file in a new tab")
                    .clicked()
                {
                    self.pick_and_open(OpenMode::NewTab);
                }

                if let Some(error) = &self.open_error {
                    ui.separator();
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
        });
    }

    /// Which files are open. Hidden entirely when there is nothing open, so an
    /// empty window is not cluttered by an empty bar.
    fn show_tab_bar(&mut self, ctx: &egui::Context) {
        if self.tabs.is_empty() {
            return;
        }

        egui::TopBottomPanel::top("tab_bar").show(ctx, |ui| {
            // Many open files should scroll rather than squeeze every tab down
            // to an unreadable sliver.
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut selected = self.active;
                    let mut to_close = None;

                    for (index, tab) in self.tabs.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if ui
                                .selectable_label(index == self.active, tab.title())
                                .on_hover_text(tab.path_string())
                                .clicked()
                            {
                                selected = index;
                            }
                            if ui
                                .small_button(phosphor::X)
                                .on_hover_text("Close this tab")
                                .clicked()
                            {
                                to_close = Some(index);
                            }
                        });
                        ui.separator();
                    }

                    self.active = selected;
                    if let Some(index) = to_close {
                        self.close_tab(index);
                    }
                });
            });
        });
    }

    /// Per-tab chrome, subordinate to the tab bar: what this tab is showing.
    /// Rebuilt from the active tab each frame, so it swaps with the selection.
    fn show_file_info(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let (title, path, hdu, shape, dtype) = (
            tab.title().to_string(),
            tab.path_string(),
            tab.hdu_name().to_string(),
            tab.shape_string(),
            tab.dtype_name(),
        );
        let error = tab.error().map(str::to_string);
        let current = tab.array_id();
        // Only built for a source that has a choice to offer; `None` keeps the
        // plain label a single-array source had before.
        let arrays = tab.has_array_choice().then(|| tab.arrays());

        let mut chosen = None;

        egui::TopBottomPanel::top("file_info").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).strong())
                    .on_hover_text(path);
                ui.separator();
                match &arrays {
                    Some(arrays) => {
                        egui::ComboBox::from_id_salt("hdu_picker")
                            .selected_text(hdu.as_str())
                            // egui otherwise caps a combo menu at
                            // `spacing.combo_height` (200pt) and scrolls the
                            // rest, which an HDU list hits while there is still
                            // plenty of window to grow into. Allow the whole
                            // window height; egui clamps the popup to what
                            // actually fits on screen.
                            .height(ui.ctx().content_rect().height())
                            .show_ui(ui, |ui| {
                                show_array_nodes(ui, arrays.nodes(), &current, &mut chosen);
                            })
                            .response
                            .on_hover_text(format!(
                                "Which {} of this file to display",
                                arrays.noun()
                            ));
                    }
                    None => {
                        ui.label(hdu.as_str());
                    }
                }
                ui.separator();
                ui.label(shape);
                ui.separator();
                ui.label(dtype);

                ui.separator();
                if ui
                    .selectable_label(self.header_open, phosphor::INFO)
                    .on_hover_text("Show this HDU's header")
                    .clicked()
                {
                    self.header_open = !self.header_open;
                }

                if let Some(error) = error {
                    ui.separator();
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
        });

        if let Some(id) = chosen {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.set_array(&id);
            }
        }
    }
}

impl NativeViewerApp {
    /// The header window: a free-floating panel rather than another bar, so a
    /// header long enough to be worth reading does not squeeze the image.
    fn show_header_window(&mut self, ctx: &egui::Context) {
        if !self.header_open {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            // Nothing open to have a header; don't leave an empty window up.
            self.header_open = false;
            return;
        };
        let title = format!("{} — {}", tab.title(), tab.hdu_name());
        let cards = tab.header();

        let mut open = true;
        egui::Window::new(title)
            // Pinned, because the title changes with the tab and the picker
            // and the window should stay the same window.
            .id(egui::Id::new("header_window"))
            .default_width(560.0)
            .default_height(420.0)
            .vscroll(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Filter");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.header_filter)
                            .hint_text("keyword, value or comment")
                            .desired_width(f32::INFINITY),
                    );
                });
                show_header_cards(ui, cards, &self.header_filter);
            });
        self.header_open = open;
    }
}

impl eframe::App for NativeViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);

        self.show_button_bar(ctx);
        self.show_tab_bar(ctx);
        self.show_file_info(ctx);

        // Slice/play controls sit in their own bar directly above the image,
        // the innermost chrome because they choose which image is shown.
        if self.tabs.get(self.active).is_some_and(Tab::has_slices) {
            let slice_frame =
                egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin {
                    left: 8,
                    right: 8,
                    top: 2,
                    bottom: 3,
                });
            egui::TopBottomPanel::top("slice_controls")
                .frame(slice_frame)
                .show(ctx, |ui| {
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        tab.show_slice_controls(ui);
                    }
                });
        }

        self.show_header_window(ctx);

        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(0.0);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            match self.tabs.get_mut(self.active) {
                Some(tab) => tab.show_viewer(ui),
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.label("Open a file to begin.");
                    });
                }
            }
        });

        // Serve after rendering, so a request raised this frame is satisfied
        // before the next one draws.
        if let Some(tab) = self.active_tab_mut() {
            tab.serve_slice_requests();
        }

        // Match the web shell: repaint continuously so playback, the zoom
        // indicator fade, and drag feedback all animate smoothly.
        ctx.request_repaint();
    }
}

/// Open every path the command line named, in order. A file that will not open
/// is reported but does not stop the rest from loading.
pub fn open_paths(paths: &[PathBuf], requested_hdu: Option<&str>) -> (Vec<Tab>, Vec<String>) {
    let mut tabs = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        match Tab::open(path, requested_hdu) {
            Ok(tab) => tabs.push(tab),
            Err(err) => errors.push(format!("{err:#}")),
        }
    }
    (tabs, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name)
    }

    fn app_with(names: &[&str]) -> NativeViewerApp {
        let paths: Vec<PathBuf> = names.iter().map(|n| fixture(n)).collect();
        let (tabs, errors) = open_paths(&paths, None);
        assert!(errors.is_empty(), "fixtures should open: {errors:?}");
        NativeViewerApp::with_tabs(tabs)
    }

    #[test]
    fn switching_wraps_in_both_directions() {
        let mut app = app_with(&["image2d.fits", "cube3d.fits", "cube4d.fits"]);
        assert_eq!(app.active, 0);

        app.switch_tab(true);
        assert_eq!(app.active, 1);
        app.switch_tab(true);
        app.switch_tab(true);
        assert_eq!(
            app.active, 0,
            "forward from the last tab wraps to the first"
        );

        app.switch_tab(false);
        assert_eq!(app.active, 2, "back from the first tab wraps to the last");
    }

    #[test]
    fn switching_is_a_no_op_without_somewhere_to_go() {
        let mut app = app_with(&[]);
        app.switch_tab(true);
        assert_eq!(app.active, 0, "no tabs: nothing to select");

        let mut app = app_with(&["image2d.fits"]);
        app.switch_tab(true);
        app.switch_tab(false);
        assert_eq!(app.active, 0, "one tab: it stays selected");
    }

    #[test]
    fn closing_keeps_the_selection_on_a_real_tab() {
        let mut app = app_with(&["image2d.fits", "cube3d.fits", "cube4d.fits"]);

        // Closing the active last tab must not leave `active` past the end.
        app.active = 2;
        app.close_tab(2);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 1);

        app.close_tab(0);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active, 0);

        app.close_tab(0);
        assert!(app.tabs.is_empty());
        assert_eq!(app.active, 0, "an empty app still has a valid index");
    }

    #[test]
    fn open_replaces_the_active_tab_in_place() {
        let mut app = app_with(&["image2d.fits", "cube3d.fits"]);
        app.active = 1;

        app.open_path(&fixture("cube4d.fits"), OpenMode::Replace);

        assert_eq!(app.tabs.len(), 2, "replacing must not add a tab");
        assert_eq!(app.active, 1);
        assert_eq!(
            app.tabs[0].title(),
            "image2d.fits",
            "the other tab is untouched"
        );
        assert_eq!(app.tabs[1].title(), "cube4d.fits");
    }

    #[test]
    fn open_in_tab_appends_and_selects_it() {
        let mut app = app_with(&["image2d.fits"]);

        app.open_path(&fixture("cube3d.fits"), OpenMode::NewTab);

        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 1, "a newly opened tab becomes the active one");
        assert_eq!(app.tabs[1].title(), "cube3d.fits");
    }

    #[test]
    fn open_with_nothing_open_yet_creates_the_first_tab() {
        // "Open" has no tab to replace when the window started empty, so it
        // has to open one rather than doing nothing.
        let mut app = app_with(&[]);
        app.open_path(&fixture("image2d.fits"), OpenMode::Replace);

        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active, 0);
    }

    #[test]
    fn a_failed_open_reports_without_disturbing_what_is_open() {
        let mut app = app_with(&["image2d.fits"]);

        app.open_path(&fixture("compressed.fits"), OpenMode::NewTab);

        assert_eq!(app.tabs.len(), 1, "the failed file must not become a tab");
        assert_eq!(app.tabs[0].title(), "image2d.fits");
        let error = app.open_error.as_deref().expect("the failure is reported");
        assert!(
            error.contains("tile-compressed"),
            "the reason should survive to the UI: {error}"
        );

        // A later good open clears the message.
        app.open_path(&fixture("cube3d.fits"), OpenMode::NewTab);
        assert!(app.open_error.is_none());
        assert_eq!(app.tabs.len(), 2);
    }
}
