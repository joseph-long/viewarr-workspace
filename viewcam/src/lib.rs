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
//! - `status::StatusBar`: what the feeds are doing, as whatever receives them reports it
//! - `widget::ViewcamWidget`: toolbar + embedded viewer + status bar, usable in any egui app
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
pub mod status;
pub mod widget;

#[cfg(target_arch = "wasm32")]
use app::ViewcamApp;
#[cfg(target_arch = "wasm32")]
use camera::Frame;
#[cfg(target_arch = "wasm32")]
use status::{now_secs, Connection, FeedRole, FrameInfo};
#[cfg(target_arch = "wasm32")]
use viewarr::buffer::{convert_buffer_to_f64, dtype_meta};
#[cfg(target_arch = "wasm32")]
use widget::{format_hex_color, parse_hex_color, ViewcamWidget};

/// Callbacks that can be registered from JavaScript
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct ViewcamCallbacks {
    /// Called when a toolbar setting or turbo changes (from the UI or from JS).
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
    /// viewarr's `setImageData` ("u16", "f32", ...). For a quantized frame,
    /// `quantization` is its `{ bias, readNoise, noiseSlope, step }` (as
    /// viewcam-proto's client hands it over), so that a saturation level is
    /// compared with its rounded values correctly; otherwise null.
    #[wasm_bindgen(js_name = setData)]
    pub fn set_data(
        &self,
        buffer: &js_sys::ArrayBuffer,
        width: u32,
        height: u32,
        array_type: &str,
        quantization: JsValue,
    ) -> Result<(), JsValue> {
        let mut frame = frame_from_buffer(buffer, width, height, array_type)?;
        frame.quantization = quantizer_from(&quantization)?;
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

    /// Remove the saturation image, or the saturation level.
    #[wasm_bindgen(js_name = clearSaturation)]
    pub fn clear_saturation(&self) {
        self.widget.borrow_mut().set_saturation(None);
    }

    /// For a camera without a saturation image: flag each data frame's
    /// pixels at or above `level` (in the data's units, before any dark is
    /// subtracted). Replaces any saturation image, as setting one replaces this.
    #[wasm_bindgen(js_name = setSaturationLevel)]
    pub fn set_saturation_level(&self, level: f64) -> Result<(), JsValue> {
        if level.is_nan() {
            return Err(JsValue::from_str("The saturation level must be a number"));
        }
        self.widget.borrow_mut().set_saturation_level(Some(level));
        Ok(())
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

    /// Name the stream a feed comes from, putting it in the status bar as
    /// connecting; `null` takes it out. `role` is "data", "dark" or "saturation".
    #[wasm_bindgen(js_name = setFeed)]
    pub fn set_feed(&self, role: &str, name: Option<String>) -> Result<(), JsValue> {
        let role = parse_role(role)?;
        self.widget.borrow_mut().status_mut().set_feed(role, name);
        Ok(())
    }

    /// A feed's connection is now "connecting", "open" or "closed".
    #[wasm_bindgen(js_name = setFeedConnection)]
    pub fn set_feed_connection(&self, role: &str, connection: &str) -> Result<(), JsValue> {
        let role = parse_role(role)?;
        let connection = Connection::parse(connection)
            .ok_or_else(|| JsValue::from_str(&format!("Not a connection state: {connection:?}")))?;
        self.widget
            .borrow_mut()
            .status_mut()
            .set_connection(role, connection);
        Ok(())
    }

    /// A feed's frame was shown: the status bar lists its shape, dtype and
    /// seq, and times it for the frame rate. `acquired_ms` is when the image
    /// was acquired, in milliseconds since the Unix epoch, if known; the
    /// status bar gives that age if the frame is left showing too long.
    #[wasm_bindgen(js_name = noteFeedFrame)]
    pub fn note_feed_frame(
        &self,
        role: &str,
        seq: u64,
        width: u32,
        height: u32,
        dtype: &str,
        acquired_ms: Option<f64>,
    ) -> Result<(), JsValue> {
        let role = parse_role(role)?;
        let frame = FrameInfo {
            seq,
            width,
            height,
            dtype: dtype.to_owned(),
            acquired: acquired_ms.map(|ms| ms / 1000.0),
        };
        self.widget
            .borrow_mut()
            .status_mut()
            .note_frame(role, frame, now_secs());
        Ok(())
    }

    /// A feed said it has no image.
    #[wasm_bindgen(js_name = noteFeedEmpty)]
    pub fn note_feed_empty(&self, role: &str) -> Result<(), JsValue> {
        let role = parse_role(role)?;
        self.widget.borrow_mut().status_mut().note_empty(role);
        Ok(())
    }

    /// A message of `bytes` arrived on any feed, shown or not, for the bandwidth.
    #[wasm_bindgen(js_name = noteReceived)]
    pub fn note_received(&self, bytes: f64) {
        self.widget
            .borrow_mut()
            .status_mut()
            .note_received(bytes as usize, now_secs());
    }

    /// Whether the status bar's turbo toggle is on. What it does is up to
    /// whatever receives the feeds.
    #[wasm_bindgen(js_name = getTurbo)]
    pub fn get_turbo(&self) -> bool {
        self.widget.borrow().status().turbo()
    }

    #[wasm_bindgen(js_name = setTurbo)]
    pub fn set_turbo(&self, turbo: bool) {
        self.widget.borrow_mut().status_mut().set_turbo(turbo);
    }

    /// Register a callback for toolbar setting and turbo changes. It receives
    /// `{ darkSubtraction, saturationOverlay, saturationColor, turbo }`.
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
fn parse_role(role: &str) -> Result<FeedRole, JsValue> {
    FeedRole::parse(role).ok_or_else(|| {
        JsValue::from_str(&format!(
            "Not a feed role (data, dark or saturation): {role:?}"
        ))
    })
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
        quantization: None,
    })
}

/// A quantizer from viewcam-proto's `{ bias, readNoise, noiseSlope, step }`,
/// or `None` from null or undefined.
#[cfg(target_arch = "wasm32")]
fn quantizer_from(value: &JsValue) -> Result<Option<camera::Quantizer>, JsValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    let field = |name: &str| -> Result<f64, JsValue> {
        js_sys::Reflect::get(value, &name.into())?
            .as_f64()
            .ok_or_else(|| JsValue::from_str(&format!("quantization.{name} is not a number")))
    };
    let quantizer = camera::Quantizer {
        bias: field("bias")?,
        read_noise: field("readNoise")?,
        noise_slope: field("noiseSlope")?,
        step: field("step")?,
    };
    quantizer
        .check()
        .map_err(|e| JsValue::from_str(&format!("quantization: {e}")))?;
    Ok(Some(quantizer))
}
