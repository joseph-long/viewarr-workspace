//! viewcam - a camera-frame viewer built on viewarr
//!
//! Wraps viewarr's `ArrayViewerWidget` with the arithmetic a camera feed
//! needs before display: subtracting a dark frame, and overlaying a
//! saturation image in a colour outside the colormap. Doing it here, in WASM,
//! keeps the per-pixel work off the JavaScript thread's critical path and
//! keeps camera-specific features out of viewarr itself.
//!
//! ## Architecture
//!
//! - `camera::CameraFrames`: the inputs and the arithmetic, no UI
//! - `widget::ViewcamWidget`: toolbar + embedded viewer, usable in any egui app
//! - `ViewcamApp`: thin eframe shell hosting the widget in the browser
//! - `ViewcamHandle`: WASM interface for JavaScript to control it

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use web_sys::HtmlCanvasElement;

#[cfg(target_arch = "wasm32")]
mod app;
pub mod camera;
pub mod widget;

#[cfg(target_arch = "wasm32")]
use app::ViewcamApp;
#[cfg(target_arch = "wasm32")]
use camera::Frame;
#[cfg(target_arch = "wasm32")]
use viewarr::buffer::{convert_buffer_to_f64, dtype_meta};
#[cfg(target_arch = "wasm32")]
use widget::{format_hex_color, parse_hex_color, ViewcamWidget};

/// Callbacks that can be registered from JavaScript
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct ViewcamCallbacks {
    /// Called when a toolbar setting changes (from the UI or from JS).
    pub on_state_change: Option<js_sys::Function>,
}

/// A handle to a camera viewer instance, each with its own canvas and state.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct ViewcamHandle {
    widget: Rc<RefCell<ViewcamWidget>>,
    callbacks: Rc<RefCell<ViewcamCallbacks>>,
    /// The eframe runner (kept alive to maintain the render loop)
    runner: eframe::WebRunner,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl ViewcamHandle {
    /// Create a new camera viewer attached to the given canvas element.
    #[wasm_bindgen]
    pub async fn create(canvas: HtmlCanvasElement) -> Result<ViewcamHandle, JsValue> {
        #[cfg(debug_assertions)]
        {
            eframe::WebLogger::init(log::LevelFilter::Debug).ok();
        }
        #[cfg(not(debug_assertions))]
        {
            eframe::WebLogger::init(log::LevelFilter::Warn).ok();
        }

        let widget = Rc::new(RefCell::new(ViewcamWidget::new()));
        let widget_for_app = widget.clone();
        let callbacks = Rc::new(RefCell::new(ViewcamCallbacks::default()));
        let callbacks_for_app = callbacks.clone();

        let runner = eframe::WebRunner::new();
        runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    Ok(Box::new(ViewcamApp::new(
                        cc,
                        widget_for_app.clone(),
                        callbacks_for_app.clone(),
                    )))
                }),
            )
            .await?;

        Ok(ViewcamHandle {
            widget,
            callbacks,
            runner,
        })
    }

    /// Set the science frame. `array_type` takes the same specifiers as
    /// viewarr's `setImageData` ("u16", "f32", ...).
    #[wasm_bindgen(js_name = setData)]
    pub fn set_data(
        &self,
        buffer: &js_sys::ArrayBuffer,
        width: u32,
        height: u32,
        array_type: &str,
    ) -> Result<(), JsValue> {
        let frame = frame_from_buffer(buffer, width, height, array_type)?;
        self.widget.borrow_mut().set_data(frame);
        Ok(())
    }

    /// Set the dark frame subtracted from the data while that toggle is on.
    /// It is applied only while its shape matches the data's.
    #[wasm_bindgen(js_name = setDark)]
    pub fn set_dark(
        &self,
        buffer: &js_sys::ArrayBuffer,
        width: u32,
        height: u32,
        array_type: &str,
    ) -> Result<(), JsValue> {
        let frame = frame_from_buffer(buffer, width, height, array_type)?;
        self.widget.borrow_mut().set_dark(Some(frame));
        Ok(())
    }

    /// Remove the dark frame.
    #[wasm_bindgen(js_name = clearDark)]
    pub fn clear_dark(&self) {
        self.widget.borrow_mut().set_dark(None);
    }

    /// Set the saturation image: any nonzero pixel is drawn as saturated.
    /// It is drawn only while its shape matches the data's.
    #[wasm_bindgen(js_name = setSaturation)]
    pub fn set_saturation(
        &self,
        buffer: &js_sys::ArrayBuffer,
        width: u32,
        height: u32,
        array_type: &str,
    ) -> Result<(), JsValue> {
        let frame = frame_from_buffer(buffer, width, height, array_type)?;
        self.widget.borrow_mut().set_saturation(Some(frame));
        Ok(())
    }

    /// Remove the saturation image.
    #[wasm_bindgen(js_name = clearSaturation)]
    pub fn clear_saturation(&self) {
        self.widget.borrow_mut().set_saturation(None);
    }

    /// Whether dark subtraction is switched on (it applies only while a
    /// matching dark frame is loaded).
    #[wasm_bindgen(js_name = getDarkSubtraction)]
    pub fn get_dark_subtraction(&self) -> bool {
        self.widget.borrow().dark_subtraction()
    }

    #[wasm_bindgen(js_name = setDarkSubtraction)]
    pub fn set_dark_subtraction(&self, enabled: bool) {
        self.widget.borrow_mut().set_dark_subtraction(enabled);
    }

    /// Whether the saturation overlay is switched on (it shows only while a
    /// matching saturation image is loaded).
    #[wasm_bindgen(js_name = getSaturationOverlay)]
    pub fn get_saturation_overlay(&self) -> bool {
        self.widget.borrow().saturation_overlay()
    }

    #[wasm_bindgen(js_name = setSaturationOverlay)]
    pub fn set_saturation_overlay(&self, enabled: bool) {
        self.widget.borrow_mut().set_saturation_overlay(enabled);
    }

    /// The saturation overlay colour as CSS `#rrggbb`.
    #[wasm_bindgen(js_name = getSaturationColor)]
    pub fn get_saturation_color(&self) -> String {
        format_hex_color(self.widget.borrow().saturation_color())
    }

    /// Set the saturation overlay colour from CSS `#rrggbb`.
    #[wasm_bindgen(js_name = setSaturationColor)]
    pub fn set_saturation_color(&self, color: &str) -> Result<(), JsValue> {
        let color = parse_hex_color(color)
            .ok_or_else(|| JsValue::from_str(&format!("Not a #rrggbb colour: {color:?}")))?;
        self.widget.borrow_mut().set_saturation_color(color);
        Ok(())
    }

    /// Register a callback for toolbar setting changes. It receives
    /// `{ darkSubtraction, saturationOverlay, saturationColor }`.
    #[wasm_bindgen(js_name = onStateChange)]
    pub fn on_state_change(&self, callback: js_sys::Function) {
        self.callbacks.borrow_mut().on_state_change = Some(callback);
    }

    /// Clear all registered callbacks.
    #[wasm_bindgen(js_name = clearCallbacks)]
    pub fn clear_callbacks(&self) {
        self.callbacks.borrow_mut().on_state_change = None;
    }

    /// End event loop and release resources
    #[wasm_bindgen(js_name = destroy)]
    pub fn destroy(&self) {
        self.runner.destroy();
    }
}

#[cfg(target_arch = "wasm32")]
fn frame_from_buffer(
    buffer: &js_sys::ArrayBuffer,
    width: u32,
    height: u32,
    array_type: &str,
) -> Result<Frame, JsValue> {
    let pixels = convert_buffer_to_f64(buffer, array_type)?;
    let expected_len = (width as usize) * (height as usize);
    if pixels.len() != expected_len {
        return Err(JsValue::from_str(&format!(
            "Buffer size mismatch: expected {} pixels ({}x{}), got {}",
            expected_len,
            width,
            height,
            pixels.len()
        )));
    }
    let (is_integer, value_decimals) = dtype_meta(array_type);
    Ok(Frame {
        pixels,
        width,
        height,
        is_integer,
        value_decimals,
    })
}
