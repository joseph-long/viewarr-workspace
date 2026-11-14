//! The native eframe shell: a button bar, a tab bar, and per-tab content.
//!
//! Chrome that belongs to the application (open buttons, which files are open)
//! lives above the viewer rather than on top of it, so the canvas stays pure
//! image. The layout nests outward-in:
//!
//! ```text
//! ┌ button bar ────────── application: acts on whichever tab is active
//! │                       (on macOS only to report a failed open; the menu
//! │                       bar has the Open commands)
//! ├ tab bar ───────────── application: which files are open
//! ├ file info ─────────── per tab: swaps when the active tab changes
//! ├ slice controls ───── per tab: only for files with sliceable axes
//! └ viewer canvas ────── per tab
//! ```

use std::path::{Path, PathBuf};

use egui::Color32;
use egui_phosphor::regular as phosphor;

use crate::source::{ArrayId, Children, Node};
use crate::tab::Tab;
use crate::watch::FileWatcher;

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

/// Cmd/Ctrl+1…9 jumps straight to a tab. It stops at nine because the viewer
/// already uses a bare `0` to reset zoom, and because borrowing the browsers'
/// habit of making the last key mean "last tab" would leave one key meaning
/// something different from its eight neighbours.
const TAB_KEYS: [egui::Key; 9] = [
    egui::Key::Num1,
    egui::Key::Num2,
    egui::Key::Num3,
    egui::Key::Num4,
    egui::Key::Num5,
    egui::Key::Num6,
    egui::Key::Num7,
    egui::Key::Num8,
    egui::Key::Num9,
];

/// What a file-picking button asked for.
enum OpenMode {
    /// Replace the active tab's contents, as a plain "Open" does.
    Replace,
    /// Add a tab and switch to it.
    NewTab,
}

/// Something asked of the app as a whole, the same whether it came from a
/// keyboard shortcut or, on macOS, the menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppCommand {
    /// Pick a file and show it in the current tab.
    Open,
    /// Pick a file and show it in a new tab.
    OpenInTab,
    /// Close whichever window has focus if it is a header window, otherwise
    /// the current tab.
    Close,
}

pub struct NativeViewerApp {
    tabs: Vec<Tab>,
    active: usize,
    /// Why the last open attempt failed. Shown in the button bar, since at that
    /// point there may be no tab to attach it to.
    open_error: Option<String>,
    /// Follows open files on disk so their tabs reload when they change.
    /// Absent in tests, and wherever the platform will not provide one.
    watcher: Option<FileWatcher>,
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

        // winit has set up the menu bar by the time the app is created.
        #[cfg(target_os = "macos")]
        crate::macos::install_menus();

        Self {
            watcher: FileWatcher::new(),
            ..Self::with_tabs(tabs)
        }
    }

    /// The tab bookkeeping on its own, with no egui context — which is also
    /// what lets the tab behaviour below be tested without a window.
    fn with_tabs(tabs: Vec<Tab>) -> Self {
        Self {
            tabs,
            active: 0,
            open_error: None,
            watcher: None,
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
            .add_filter("FITS", &["fits"])
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            return;
        };
        self.open_path(&path, mode);
    }

    /// Open `path`, reporting a failure in the button bar rather than closing
    /// or disturbing whatever is already open. A file that already has a tab
    /// is switched to instead, whichever way it was asked for, so the same
    /// file never ends up in two tabs with diverging views.
    fn open_path(&mut self, path: &Path, mode: OpenMode) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.shows_file(path)) {
            self.open_error = None;
            self.active = index;
            return;
        }
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

    /// Open files that arrived from outside the window's own buttons — dropped
    /// onto it, or on macOS sent by Finder — each in a tab of its own, as
    /// several files named on the command line would be.
    fn open_incoming_files(&mut self, ctx: &egui::Context) {
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut paths: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        #[cfg(target_os = "macos")]
        paths.extend(crate::macos::take_opened_files());
        for path in paths {
            self.open_path(&path, OpenMode::NewTab);
        }
    }

    /// Reload every tab whose file has changed on disk and settled, then
    /// bring the watched folders up to date with whatever tabs are now open.
    fn reload_changed_files(&mut self) {
        let Some(watcher) = &mut self.watcher else {
            return;
        };
        for path in watcher.settled() {
            for tab in &mut self.tabs {
                if tab.disk_path() == path {
                    tab.reload();
                }
            }
        }
        watcher.follow(self.tabs.iter().map(Tab::disk_path));
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

    fn run_command(&mut self, ctx: &egui::Context, command: AppCommand) {
        match command {
            AppCommand::Open => self.pick_and_open(OpenMode::Replace),
            AppCommand::OpenInTab => self.pick_and_open(OpenMode::NewTab),
            AppCommand::Close => match focused_secondary_viewport(ctx) {
                // A header window in front is what the user means to close,
                // as Cmd+W would close any other document window.
                Some(id) => ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Close),
                None => self.close_active_tab(),
            },
        }
    }

    /// Close whichever tab is showing. With nothing open this does nothing:
    /// the empty window stays up until the app is quit, as it does when the
    /// last tab is closed with its button.
    fn close_active_tab(&mut self) {
        self.close_tab(self.active);
    }

    /// Jump to a tab by position. Out of range does nothing, so Cmd+7 in a
    /// three-tab window is inert rather than landing somewhere arbitrary.
    fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
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

    /// Cmd+Shift+Arrow to step tabs, Cmd+N to jump to one, Cmd+W to close the
    /// current one and Cmd+O / Cmd+T to open a file in it or a new one, on
    /// macOS;
    /// Ctrl+ the same elsewhere — `COMMAND` is egui's per-platform spelling of
    /// exactly that. All consumed so the keystrokes do not also reach the
    /// viewer widget.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let step = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let (prev, next, jump, command) = ctx.input_mut(|i| {
            let prev = i.consume_key(step, egui::Key::ArrowLeft);
            let next = i.consume_key(step, egui::Key::ArrowRight);
            // On macOS these three are the menu bar's key equivalents, and
            // AppKit hands them to the menu before the window ever sees them;
            // they arrive here only on the other platforms.
            let command = [
                (egui::Key::O, AppCommand::Open),
                (egui::Key::T, AppCommand::OpenInTab),
                (egui::Key::W, AppCommand::Close),
            ]
            .into_iter()
            .filter(|(key, _)| i.consume_key(egui::Modifiers::COMMAND, *key))
            .map(|(_, command)| command)
            .last();
            // Every pressed digit is consumed, not just the first, so a second
            // one cannot leak through to the viewer; the first still wins.
            let mut jump = None;
            for (index, key) in TAB_KEYS.iter().enumerate() {
                if i.consume_key(egui::Modifiers::COMMAND, *key) && jump.is_none() {
                    jump = Some(index);
                }
            }
            (prev, next, jump, command)
        });
        if prev {
            self.switch_tab(false);
        }
        if next {
            self.switch_tab(true);
        }
        if let Some(index) = jump {
            self.select_tab(index);
        }
        if let Some(command) = command {
            self.run_command(ctx, command);
        }
    }

    /// Application chrome: acts on whichever tab is active.
    ///
    /// On macOS the Open commands live in the menu bar, so the bar is only put
    /// up to report a failed open, which otherwise would have nowhere to go.
    fn show_button_bar(&mut self, ctx: &egui::Context) {
        let show_buttons = !cfg!(target_os = "macos");
        if !show_buttons && self.open_error.is_none() {
            return;
        }
        egui::TopBottomPanel::top("button_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if show_buttons {
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
                }

                if let Some(error) = &self.open_error {
                    if show_buttons {
                        ui.separator();
                    }
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    if ui
                        .small_button(phosphor::X)
                        .on_hover_text("Dismiss")
                        .clicked()
                    {
                        self.open_error = None;
                    }
                }
            });
        });
    }

    /// Which files are open, as a strip of browser-style tabs. Hidden entirely
    /// when there is nothing open, so an empty window is not cluttered by an
    /// empty bar.
    ///
    /// The active tab is filled with the panel colour of the file-info bar
    /// beneath it and the strip is drawn a shade darker, so the selected tab
    /// reads as the top of the content it heads. A tab's close button shows on
    /// the active tab and on hover, in a slot that is always reserved so the
    /// titles do not shift; a middle-click closes too.
    fn show_tab_bar(&mut self, ctx: &egui::Context) {
        if self.tabs.is_empty() {
            return;
        }

        let visuals = ctx.style().visuals.clone();
        let strip_fill = visuals
            .panel_fill
            .lerp_to_gamma(Color32::BLACK, if visuals.dark_mode { 0.35 } else { 0.08 });
        let frame = egui::Frame::side_top_panel(&ctx.style())
            .fill(strip_fill)
            .inner_margin(egui::Margin {
                left: 6,
                right: 6,
                top: 5,
                bottom: 0,
            });

        egui::TopBottomPanel::top("tab_bar")
            .frame(frame)
            // The baseline is painted below instead, broken by the active tab.
            .show_separator_line(false)
            .show(ctx, |ui| {
                let strip = ui.max_rect();
                let mut active_rect = None;

                // Many open files should scroll rather than squeeze every tab
                // down to an unreadable sliver.
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let mut selected = self.active;
                        let mut to_close = None;

                        for (index, tab) in self.tabs.iter().enumerate() {
                            let is_active = index == self.active;
                            // A divider between two inactive tabs; beside the
                            // active one its own edges do the separating.
                            let divide = index + 1 < self.tabs.len()
                                && !is_active
                                && index + 1 != self.active;
                            let (clicked, rect) =
                                tab_button(ui, tab, is_active, divide, strip_fill);
                            if is_active {
                                active_rect = Some(rect);
                            }
                            match clicked {
                                TabClick::Select => selected = index,
                                TabClick::Close => to_close = Some(index),
                                TabClick::None => {}
                            }
                        }

                        ui.add_space(4.0);
                        if ui
                            .add(egui::Button::new(phosphor::PLUS).frame(false))
                            .on_hover_text("Open a file in a new tab")
                            .clicked()
                        {
                            self.pick_and_open(OpenMode::NewTab);
                        }

                        self.active = selected;
                        if let Some(index) = to_close {
                            self.close_tab(index);
                        }
                    });
                });

                // The strip's bottom edge, with a gap where the active tab
                // opens onto the panel below. Painted here rather than under
                // the tabs because the scroll area clips them to the strip.
                let stroke = visuals.widgets.noninteractive.bg_stroke;
                let y = strip.bottom() - 0.5;
                let gap = active_rect
                    .map(|r| r.x_range().intersection(strip.x_range()))
                    .filter(|r| r.span() > 0.0);
                match gap {
                    Some(gap) => {
                        ui.painter().hline(strip.left()..=gap.min, y, stroke);
                        ui.painter().hline(gap.max..=strip.right(), y, stroke);
                    }
                    None => {
                        ui.painter().hline(strip.x_range(), y, stroke);
                    }
                }
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
        let header_open = tab.header_is_open();
        let mut toggle_header = false;
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
                    .selectable_label(header_open, phosphor::INFO)
                    .on_hover_text("Show this HDU's header in its own window")
                    .clicked()
                {
                    toggle_header = true;
                }

                if let Some(error) = error {
                    ui.separator();
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
        });

        if let Some(tab) = self.tabs.get_mut(self.active) {
            if toggle_header {
                tab.toggle_header();
            }
            if let Some(id) = chosen {
                tab.set_array(&id);
            }
        }
    }
}

impl eframe::App for NativeViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.open_incoming_files(ctx);
        self.reload_changed_files();
        self.handle_shortcuts(ctx);
        #[cfg(target_os = "macos")]
        for command in crate::macos::take_menu_commands() {
            self.run_command(ctx, command);
        }

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

        for tab in &mut self.tabs {
            tab.show_header_window(ctx);
        }

        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(0.0);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            match self.tabs.get_mut(self.active) {
                Some(tab) => tab.show_viewer(ui),
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.weak(if cfg!(target_os = "macos") {
                            "Choose File ▸ Open… (⌘O), or drop a file here."
                        } else {
                            "Open a file, or drop one here."
                        });
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

/// Tab widths: wide enough for a short filename, narrow enough that a long
/// one does not crowd out its neighbours (it is truncated with an ellipsis,
/// and the full path is on hover).
const TAB_MIN_WIDTH: f32 = 110.0;
const TAB_MAX_WIDTH: f32 = 220.0;
const TAB_HEIGHT: f32 = 26.0;
const TAB_PADDING: f32 = 10.0;
/// Room kept at a tab's right for its close button, shown or not.
const TAB_CLOSE_SLOT: f32 = 18.0;

/// What happened to one tab this frame.
enum TabClick {
    None,
    Select,
    Close,
}

/// Draw one tab of the tab bar and report what was done to it.
fn tab_button(
    ui: &mut egui::Ui,
    tab: &Tab,
    is_active: bool,
    divide: bool,
    strip_fill: Color32,
) -> (TabClick, egui::Rect) {
    let visuals = ui.visuals().clone();
    let font = egui::TextStyle::Button.resolve(ui.style());
    let max_text = TAB_MAX_WIDTH - 2.0 * TAB_PADDING - TAB_CLOSE_SLOT;
    let title_width = ui
        .painter()
        .layout_no_wrap(tab.title().to_string(), font.clone(), Color32::PLACEHOLDER)
        .size()
        .x;
    let width = (title_width.min(max_text) + 2.0 * TAB_PADDING + TAB_CLOSE_SLOT)
        .clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);

    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, TAB_HEIGHT), egui::Sense::click());
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(
            rect.right() - TAB_PADDING / 2.0 - TAB_CLOSE_SLOT / 2.0 + 2.0,
            rect.center().y,
        ),
        egui::vec2(TAB_CLOSE_SLOT, TAB_CLOSE_SLOT),
    );
    let close = ui.interact(close_rect, response.id.with("close"), egui::Sense::click());
    let hovered = response.hovered() || close.hovered();

    let painter = ui.painter();
    let corner = egui::CornerRadius {
        nw: 6,
        ne: 6,
        sw: 0,
        se: 0,
    };
    if is_active {
        painter.rect_filled(rect, corner, visuals.panel_fill);
        // Outlined on three sides: the bottom edge is painted back over in
        // the fill, leaving the tab open onto the panel below.
        let edge = visuals.widgets.noninteractive.bg_stroke;
        painter.rect_stroke(rect, corner, edge, egui::StrokeKind::Inside);
        painter.hline(
            rect.x_range().shrink(edge.width),
            rect.bottom() - edge.width / 2.0,
            egui::Stroke::new(edge.width, visuals.panel_fill),
        );
    } else if hovered {
        let hover_fill = strip_fill.lerp_to_gamma(visuals.panel_fill, 0.5);
        painter.rect_filled(rect.shrink2(egui::vec2(2.0, 2.0)), 5.0, hover_fill);
    }
    if divide && !hovered {
        let x = rect.right() - 0.5;
        painter.vline(
            x,
            egui::Rangef::new(rect.top() + 7.0, rect.bottom() - 7.0),
            visuals.widgets.noninteractive.bg_stroke,
        );
    }

    let text_color = if is_active {
        visuals.strong_text_color()
    } else if hovered {
        visuals.text_color()
    } else {
        visuals.weak_text_color()
    };
    let mut job = egui::text::LayoutJob::single_section(
        tab.title().to_string(),
        egui::TextFormat::simple(font, text_color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_text);
    let galley = painter.layout_job(job);
    let text_pos = egui::pos2(
        rect.left() + TAB_PADDING,
        rect.center().y - galley.size().y / 2.0,
    );
    painter.galley(text_pos, galley, text_color);

    if is_active || hovered {
        if close.hovered() {
            painter.rect_filled(
                close_rect.shrink(1.0),
                4.0,
                visuals.widgets.hovered.weak_bg_fill,
            );
        }
        painter.text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            phosphor::X,
            egui::FontId::proportional(12.0),
            if close.hovered() {
                visuals.strong_text_color()
            } else {
                text_color
            },
        );
    }

    let close_clicked = close.on_hover_text("Close this tab").clicked();
    let response = response.on_hover_text(tab.path_string());
    let click = if close_clicked || response.middle_clicked() {
        TabClick::Close
    } else if response.clicked() {
        TabClick::Select
    } else {
        TabClick::None
    };
    (click, rect)
}

/// The header window that has keyboard focus, if one does. `None` means the
/// main window is in front (or nothing is).
fn focused_secondary_viewport(ctx: &egui::Context) -> Option<egui::ViewportId> {
    ctx.input(|i| {
        i.raw
            .viewports
            .iter()
            .find(|(id, info)| **id != egui::ViewportId::ROOT && info.focused == Some(true))
            .map(|(id, _)| *id)
    })
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
    fn jumping_selects_by_position_and_ignores_what_is_not_there() {
        let mut app = app_with(&["image2d.fits", "cube3d.fits", "cube4d.fits"]);

        app.select_tab(2);
        assert_eq!(app.active, 2, "Cmd+3 lands on the third tab");
        app.select_tab(0);
        assert_eq!(app.active, 0);

        // Cmd+7 with three tabs open: nothing to go to, so nothing moves.
        app.select_tab(6);
        assert_eq!(app.active, 0);
        app.select_tab(TAB_KEYS.len() - 1);
        assert_eq!(app.active, 0);

        // The keys only ever reach the first nine, so a tenth tab is only
        // reachable by stepping or clicking.
        assert_eq!(TAB_KEYS.len(), 9);
    }

    #[test]
    fn jumping_is_a_no_op_with_no_tabs_open() {
        let mut app = app_with(&[]);
        app.select_tab(0);
        assert_eq!(app.active, 0, "an empty app still has a valid index");
        assert!(app.tabs.is_empty());
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
    fn cmd_w_closes_the_active_tab_and_nothing_once_empty() {
        let mut app = app_with(&["image2d.fits", "cube3d.fits", "cube4d.fits"]);
        let ctx = egui::Context::default();
        let press_cmd_w = |app: &mut NativeViewerApp| {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::W,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }],
                    ..Default::default()
                },
                |ctx| app.handle_shortcuts(ctx),
            );
        };

        app.active = 1;
        press_cmd_w(&mut app);
        assert_eq!(app.tabs.len(), 2);
        assert!(
            app.tabs
                .iter()
                .all(|t| !t.shows_file(&fixture("cube3d.fits"))),
            "the active tab, not some other, is the one closed"
        );

        press_cmd_w(&mut app);
        press_cmd_w(&mut app);
        assert!(app.tabs.is_empty());

        // Nothing left: the keystroke is swallowed and the app stays up.
        press_cmd_w(&mut app);
        assert!(app.tabs.is_empty());
        assert_eq!(app.active, 0);
    }

    #[test]
    fn close_goes_to_a_focused_header_window_instead_of_a_tab() {
        let mut app = app_with(&["image2d.fits", "cube3d.fits"]);
        let ctx = egui::Context::default();
        let header = egui::ViewportId::from_hash_of("some header window");
        let run = |app: &mut NativeViewerApp, header_focused: bool| {
            let mut raw = egui::RawInput::default();
            raw.viewports.insert(
                header,
                egui::ViewportInfo {
                    focused: Some(header_focused),
                    ..Default::default()
                },
            );
            let _ = ctx.run(raw, |ctx| app.run_command(ctx, AppCommand::Close));
        };

        run(&mut app, true);
        assert_eq!(app.tabs.len(), 2, "the header window, not a tab, is closed");

        run(&mut app, false);
        assert_eq!(
            app.tabs.len(),
            1,
            "with the main window in front, the tab goes"
        );
    }

    #[test]
    fn the_tab_bar_is_a_single_row_of_tabs() {
        let mut app = app_with(&["image2d.fits", "cube3d.fits", "cube4d.fits"]);
        app.active = 1;
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.show_tab_bar(ctx));
        }
        // A tab's height plus the strip's top margin, not a stack of rows.
        let height = ctx.used_rect().height();
        assert!(height < TAB_HEIGHT + 12.0, "tab bar is {height} tall");
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
    fn opening_a_file_that_already_has_a_tab_selects_it() {
        let mut app = app_with(&["image2d.fits", "cube3d.fits"]);
        app.active = 1;

        app.open_path(&fixture("image2d.fits"), OpenMode::NewTab);
        assert_eq!(app.tabs.len(), 2, "no second tab for the same file");
        assert_eq!(app.active, 0);

        // "Open" switches too, rather than duplicating the file over the
        // active tab.
        app.active = 1;
        app.open_path(&fixture("image2d.fits"), OpenMode::Replace);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 0);
        assert_eq!(app.tabs[1].title(), "cube3d.fits", "nothing was replaced");
    }

    #[test]
    fn the_same_file_by_another_path_is_still_the_same_file() {
        let mut app = app_with(&["image2d.fits", "cube3d.fits"]);

        let roundabout = fixture("image2d.fits")
            .parent()
            .unwrap()
            .join("../data/./image2d.fits");
        app.open_path(&roundabout, OpenMode::NewTab);

        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active, 0);
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
