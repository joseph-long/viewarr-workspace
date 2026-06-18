//! Thin application shell for the array viewer
//!
//! This module contains the eframe App implementation that hosts an ArrayViewerWidget.
//! The app is responsible for:
//! - Hosting the widget in a CentralPanel
//! - Passing available size from egui's layout to the widget
//! - Continuous repaint requests for smooth updates
//! - Tracking state changes and calling JavaScript callbacks

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use crate::widget::ArrayViewerWidget;
use crate::ViewerCallbacks;
use wasm_bindgen::JsValue;

use eframe::WebLogger;
use egui_phosphor as phosphor;

/// Cached state for detecting changes
#[derive(Clone, Default)]
struct CachedState {
    contrast: f64,
    bias: f64,
    stretch_mode: String,
    zoom: f32,
    symmetric: bool,
    colormap: String,
    colormap_reversed: bool,
    vmin: f64,
    vmax: f64,
    pan_x: f32,
    pan_y: f32,
    rotation: f32,
    pivot_x: f32,
    pivot_y: f32,
    show_pivot_marker: bool,
}

impl CachedState {
    fn from_widget(widget: &ArrayViewerWidget) -> Self {
        let cb = widget.current_contrast_bias();
        let (vmin, vmax) = widget.display_value_range();
        let transform = widget.transform();
        let (pivot_x, pivot_y) = widget.pivot_point();
        Self {
            contrast: cb.contrast,
            bias: cb.bias,
            stretch_mode: if widget.is_symmetric() {
                "symmetric".to_string()
            } else {
                match widget.stretch_type() {
                    crate::widget::StretchType::Linear => "linear".to_string(),
                    crate::widget::StretchType::Log => "log".to_string(),
                }
            },
            zoom: widget.zoom_level(),
            symmetric: widget.is_symmetric(),
            colormap: widget.colormap().name().to_string(),
            colormap_reversed: widget.is_reversed(),
            vmin,
            vmax,
            pan_x: transform.pan_offset.x,
            pan_y: transform.pan_offset.y,
            rotation: widget.rotation(),
            pivot_x,
            pivot_y,
            show_pivot_marker: widget.show_pivot_marker(),
        }
    }

    fn differs_from(&self, other: &CachedState) -> bool {
        (self.contrast - other.contrast).abs() > 0.001
            || (self.bias - other.bias).abs() > 0.001
            || self.stretch_mode != other.stretch_mode
            || (self.zoom - other.zoom).abs() > 0.001
            || self.symmetric != other.symmetric
            || self.colormap != other.colormap
            || self.colormap_reversed != other.colormap_reversed
            || (self.vmin - other.vmin).abs() > 1e-10
            || (self.vmax - other.vmax).abs() > 1e-10
            || (self.pan_x - other.pan_x).abs() > 0.5
            || (self.pan_y - other.pan_y).abs() > 0.5
            || (self.rotation - other.rotation).abs() > 0.01
            || (self.pivot_x - other.pivot_x).abs() > 0.01
            || (self.pivot_y - other.pivot_y).abs() > 0.01
            || self.show_pivot_marker != other.show_pivot_marker
    }
}

/// The eframe application shell for the viewer.
///
/// This is a thin wrapper that hosts a single ArrayViewerWidget and
/// manages the application lifecycle. The widget itself contains all
/// viewing state and rendering logic.
pub struct ViewerApp {
    /// The widget instance (shared with ViewerHandle for external control)
    widget: Rc<RefCell<ArrayViewerWidget>>,
    /// Callbacks registered from JavaScript
    callbacks: Rc<RefCell<ViewerCallbacks>>,
    /// Cached state for detecting changes
    cached_state: CachedState,
    /// Last viewport size (for bounds calculation)
    last_viewport_size: egui::Vec2,
}

impl ViewerApp {
    /// Create a new application with the given widget instance.
    ///
    /// The widget is shared via Rc<RefCell<>> so that ViewerHandle can
    /// call methods on it from JavaScript.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        widget: Rc<RefCell<ArrayViewerWidget>>,
        callbacks: Rc<RefCell<ViewerCallbacks>>,
    ) -> Self {
        // Initialize logging (adjust level as needed: Error, Warn, Info, Debug, Trace)
        WebLogger::init(log::LevelFilter::Trace).ok();
        let mut fonts = egui::FontDefinitions::default();
        phosphor::add_to_fonts(&mut fonts, phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);
        Self {
            widget,
            callbacks,
            cached_state: CachedState::default(),
            last_viewport_size: egui::Vec2::ZERO,
        }
    }

    /// Call the state change callback if state has changed
    fn check_and_notify_state_change(&mut self) {
        // Collect all state data while holding the borrow, then drop it before calling JS
        let (current_state, bounds_data) = {
            let widget = self.widget.borrow();
            let state = CachedState::from_widget(&widget);
            
            // Calculate bounds data if we have an image
            let bounds = if self.last_viewport_size.x > 0.0 
                && self.last_viewport_size.y > 0.0 
                && widget.has_image() 
            {
                let (img_width, img_height) = widget.dimensions();
                let viewport_size = self.last_viewport_size;
                let viewport_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, viewport_size);

                // Calculate base display size (fit-to-view)
                let img_aspect = img_width as f32 / img_height as f32;
                let viewport_aspect = viewport_size.x / viewport_size.y;
                let base_display_size = if img_aspect > viewport_aspect {
                    egui::vec2(viewport_size.x, viewport_size.x / img_aspect)
                } else {
                    egui::vec2(viewport_size.y * img_aspect, viewport_size.y)
                };

                let image_rect = widget.transform().calculate_image_rect(viewport_rect, base_display_size);
                let visible_screen = viewport_rect.intersect(image_rect);

                if visible_screen.width() > 0.0 && visible_screen.height() > 0.0 {
                    let rel_x_min = (visible_screen.min.x - image_rect.min.x) / image_rect.width();
                    let rel_x_max = (visible_screen.max.x - image_rect.min.x) / image_rect.width();
                    let rel_y_max = 1.0 - (visible_screen.min.y - image_rect.min.y) / image_rect.height();
                    let rel_y_min = 1.0 - (visible_screen.max.y - image_rect.min.y) / image_rect.height();

                    Some((
                        (rel_x_min * img_width as f32).max(0.0) as f64,
                        (rel_x_max * img_width as f32).min(img_width as f32) as f64,
                        (rel_y_min * img_height as f32).max(0.0) as f64,
                        (rel_y_max * img_height as f32).min(img_height as f32) as f64,
                    ))
                } else {
                    None
                }
            } else {
                None
            };
            
            (state, bounds)
        }; // Widget borrow is dropped here

        if current_state.differs_from(&self.cached_state) {
            // State changed, call the callback (widget borrow already dropped)
            if let Some(ref callback) = self.callbacks.borrow().on_state_change {
                // Build the state object to pass to JavaScript
                let state = js_sys::Object::new();
                js_sys::Reflect::set(&state, &"contrast".into(), &current_state.contrast.into()).ok();
                js_sys::Reflect::set(&state, &"bias".into(), &current_state.bias.into()).ok();
                js_sys::Reflect::set(&state, &"stretchMode".into(), &current_state.stretch_mode.clone().into()).ok();
                js_sys::Reflect::set(&state, &"zoom".into(), &(current_state.zoom as f64).into()).ok();
                js_sys::Reflect::set(&state, &"colormap".into(), &current_state.colormap.clone().into()).ok();
                js_sys::Reflect::set(&state, &"colormapReversed".into(), &current_state.colormap_reversed.into()).ok();
                js_sys::Reflect::set(&state, &"vmin".into(), &current_state.vmin.into()).ok();
                js_sys::Reflect::set(&state, &"vmax".into(), &current_state.vmax.into()).ok();
                
                // Include rotation state
                js_sys::Reflect::set(&state, &"rotation".into(), &(current_state.rotation as f64).into()).ok();
                let pivot = js_sys::Array::new();
                pivot.push(&(current_state.pivot_x as f64).into());
                pivot.push(&(current_state.pivot_y as f64).into());
                js_sys::Reflect::set(&state, &"pivot".into(), &pivot).ok();
                js_sys::Reflect::set(&state, &"showPivotMarker".into(), &current_state.show_pivot_marker.into()).ok();

                // Include view bounds if available
                if let Some((x_min, x_max, y_min, y_max)) = bounds_data {
                    let xlim = js_sys::Array::new();
                    xlim.push(&x_min.into());
                    xlim.push(&x_max.into());
                    js_sys::Reflect::set(&state, &"xlim".into(), &xlim).ok();

                    let ylim = js_sys::Array::new();
                    ylim.push(&y_min.into());
                    ylim.push(&y_max.into());
                    js_sys::Reflect::set(&state, &"ylim".into(), &ylim).ok();
                }

                // Call the callback
                let this = JsValue::NULL;
                let _ = callback.call1(&this, &state);
            }

            // Update cached state
            self.cached_state = current_state;
        }
    }

    /// Call the click callback for any queued shift-click event.
    fn check_and_notify_shift_click(&mut self) {
        let click_event = self.widget.borrow_mut().take_shift_click_event();
        let Some((x, y)) = click_event else {
            return;
        };

        if let Some(ref callback) = self.callbacks.borrow().on_click {
            let event = js_sys::Object::new();
            js_sys::Reflect::set(&event, &"x".into(), &x.into()).ok();
            js_sys::Reflect::set(&event, &"y".into(), &y.into()).ok();
            let this = JsValue::NULL;
            let _ = callback.call1(&this, &event);
        }
    }
}

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Use a CentralPanel with no margin/padding
        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(0.0);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            // Use the actual available size from egui's layout system
            let container_size = ui.available_size();
            self.last_viewport_size = container_size;

            // Render the widget
            let mut widget = self.widget.borrow_mut();
            widget.show(ui, container_size);
        });

        // Check for state changes and notify JavaScript
        self.check_and_notify_state_change();
        self.check_and_notify_shift_click();

        // Request continuous repaints for smooth updates
        ctx.request_repaint();
    }
}
