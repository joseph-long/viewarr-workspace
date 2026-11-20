//! Thin application shell for the camera viewer
//!
//! Hosts a ViewcamWidget: its toolbar in a panel above, its status bar in one
//! below, the viewer filling the rest. Also reports toolbar and status bar
//! settings to JavaScript when they change.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use egui_phosphor as phosphor;
use wasm_bindgen::JsValue;

use crate::widget::{format_hex_color, ViewcamWidget};
use crate::ViewcamCallbacks;

/// The settings JavaScript is told about when they change.
#[derive(Clone, PartialEq)]
struct ToolbarState {
    dark_subtraction: bool,
    saturation_overlay: bool,
    saturation_color: String,
    turbo: bool,
}

impl ToolbarState {
    fn from_widget(widget: &ViewcamWidget) -> Self {
        Self {
            dark_subtraction: widget.dark_subtraction(),
            saturation_overlay: widget.saturation_overlay(),
            saturation_color: format_hex_color(widget.saturation_color()),
            turbo: widget.status().turbo(),
        }
    }
}

pub struct ViewcamApp {
    widget: Rc<RefCell<ViewcamWidget>>,
    callbacks: Rc<RefCell<ViewcamCallbacks>>,
    /// Last state reported; `None` until the first frame, which reports it.
    reported: Option<ToolbarState>,
}

impl ViewcamApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        widget: Rc<RefCell<ViewcamWidget>>,
        callbacks: Rc<RefCell<ViewcamCallbacks>>,
    ) -> Self {
        // The embedded viewer draws its controls with Phosphor glyphs.
        let mut fonts = egui::FontDefinitions::default();
        phosphor::add_to_fonts(&mut fonts, phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);
        Self {
            widget,
            callbacks,
            reported: None,
        }
    }

    /// Call the state change callback if the toolbar settings changed.
    fn check_and_notify_state_change(&mut self) {
        // Read the state and drop the borrow before calling into JS, which may
        // call straight back into the handle.
        let current = ToolbarState::from_widget(&self.widget.borrow());
        if self.reported.as_ref() == Some(&current) {
            return;
        }
        if let Some(ref callback) = self.callbacks.borrow().on_state_change {
            let state = js_sys::Object::new();
            js_sys::Reflect::set(
                &state,
                &"darkSubtraction".into(),
                &current.dark_subtraction.into(),
            )
            .ok();
            js_sys::Reflect::set(
                &state,
                &"saturationOverlay".into(),
                &current.saturation_overlay.into(),
            )
            .ok();
            js_sys::Reflect::set(
                &state,
                &"saturationColor".into(),
                &current.saturation_color.clone().into(),
            )
            .ok();
            js_sys::Reflect::set(&state, &"turbo".into(), &current.turbo.into()).ok();
            let _ = callback.call1(&JsValue::NULL, &state);
        }
        self.reported = Some(current);
    }
}

impl eframe::App for ViewcamApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Same frame as viewarr's slice bar: the bottom margin is 1px deeper
        // because the panel's separator line eats 1px of it.
        let toolbar_frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 2,
            bottom: 3,
        });
        let has_toolbar = self.widget.borrow().has_toolbar();
        if has_toolbar {
            egui::TopBottomPanel::top("camera_toolbar")
                .frame(toolbar_frame)
                .show(ctx, |ui| {
                    self.widget.borrow_mut().show_toolbar(ui);
                });
        }

        // The status bar takes the same frame as the toolbar, mirrored.
        let status_frame = toolbar_frame.inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 3,
            bottom: 2,
        });
        let has_status_bar = self.widget.borrow().has_status_bar();
        if has_status_bar {
            egui::TopBottomPanel::bottom("camera_status")
                .frame(status_frame)
                .show(ctx, |ui| {
                    self.widget.borrow_mut().show_status_bar(ui);
                });
        }

        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(0.0);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let container_size = ui.available_size();
            self.widget.borrow_mut().show_viewer(ui, container_size);
        });

        self.check_and_notify_state_change();

        ctx.request_repaint();
    }
}
