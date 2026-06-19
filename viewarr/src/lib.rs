//! viewarr - A browser-based array/image viewer using Rust, WASM, and egui
//!
//! This library provides a WebAssembly-based image viewer that can be embedded
//! in web applications. It accepts typed array data from JavaScript and renders
//! it with colormap support, showing pixel values on hover.
//!
//! ## Architecture
//!
//! - `ArrayViewerWidget`: Self-contained egui widget with all viewing state
//! - `ViewerApp`: Thin eframe App shell that hosts the widget
//! - `ViewerHandle`: WASM interface for JavaScript to control the viewer

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
mod colormap;
mod colormap_luts;
mod transform;
mod widget;

#[cfg(target_arch = "wasm32")]
use app::ViewerApp;
#[cfg(target_arch = "wasm32")]
use colormap::Colormap;
#[cfg(target_arch = "wasm32")]
use widget::ArrayViewerWidget;

/// Callbacks that can be registered from JavaScript
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct ViewerCallbacks {
    /// Called when viewer state changes (contrast, bias, zoom, pan, etc.)
    pub on_state_change: Option<js_sys::Function>,
    /// Called when the user shift-clicks on the image (continuous image coordinates)
    pub on_click: Option<js_sys::Function>,
    /// Called when the widget needs a cube slice fetched (slider drag / play loop).
    /// Receives the requested slice indices as a JS number array.
    pub on_slice_request: Option<js_sys::Function>,
}

/// Callbacks that can be registered from JavaScript
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub struct ViewerCallbacks {}

/// A handle to a viewer instance. Each handle manages its own canvas and state.
///
/// This struct is exposed to JavaScript and provides methods to control the viewer.
/// It holds an Rc to the widget so it can call methods on it, and also stores
/// the eframe runner for the application lifecycle.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct ViewerHandle {
    /// The widget instance (shared with ViewerApp)
    widget: Rc<RefCell<ArrayViewerWidget>>,
    /// Callbacks registered from JavaScript
    callbacks: Rc<RefCell<ViewerCallbacks>>,
    /// The eframe runner (kept alive to maintain the render loop)
    #[allow(dead_code)]
    runner: eframe::WebRunner,
}

// #[wasm_bindgen(start)]
// fn init_logging() {
//     // Initialize the logger
//     console_log::init_with_level(log::Level::Debug).expect("error initializing logger");
// }


#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl ViewerHandle {
    /// Create a new viewer instance attached to the given canvas element.
    /// Returns a promise that resolves to a ViewerHandle when initialization completes.
    /// 
    /// Use this static factory method instead of a constructor since async constructors
    /// are deprecated in wasm-bindgen.
    #[wasm_bindgen]
    pub async fn create(canvas: HtmlCanvasElement) -> Result<ViewerHandle, JsValue> {
        // Initialize logging for debug builds
        #[cfg(debug_assertions)]
        {
            eframe::WebLogger::init(log::LevelFilter::Debug).ok();
        }
        #[cfg(not(debug_assertions))]
        {
            eframe::WebLogger::init(log::LevelFilter::Warn).ok();
        }

        // Create the widget that will be shared between the handle and the app
        let widget = Rc::new(RefCell::new(ArrayViewerWidget::new()));
        let widget_for_app = widget.clone();

        // Create callbacks container
        let callbacks = Rc::new(RefCell::new(ViewerCallbacks::default()));
        let callbacks_for_app = callbacks.clone();

        let web_options = eframe::WebOptions::default();
        let runner = eframe::WebRunner::new();

        runner
            .start(
                canvas,
                web_options,
                Box::new(move |cc| Ok(Box::new(ViewerApp::new(cc, widget_for_app.clone(), callbacks_for_app.clone())))),
            )
            .await?;

        Ok(ViewerHandle { widget, callbacks, runner })
    }

    /// Set the image data to display.
    ///
    /// # Arguments
    /// * `buffer` - ArrayBuffer containing the raw pixel data
    /// * `width` - Image width in pixels
    /// * `height` - Image height in pixels
    /// * `array_type` - Rust-style type specifier:
    ///   - "i8" (signed 8-bit integer)
    ///   - "u8" (unsigned 8-bit integer)
    ///   - "i16" (signed 16-bit integer)
    ///   - "u16" (unsigned 16-bit integer)
    ///   - "i32" (signed 32-bit integer)
    ///   - "u32" (unsigned 32-bit integer)
    ///   - "i64" (signed 64-bit integer)
    ///   - "u64" (unsigned 64-bit integer)
    ///   - "f32" (32-bit float)
    ///   - "f64" (64-bit float, default)
    #[wasm_bindgen(js_name = setImageData)]
    pub fn set_image_data(
        &self,
        buffer: &js_sys::ArrayBuffer,
        width: u32,
        height: u32,
        array_type: &str,
    ) -> Result<(), JsValue> {
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

        let mut widget = self.widget.borrow_mut();
        widget.set_image(pixels, width, height, is_integer, value_decimals);

        Ok(())
    }

    /// Declare the sliceable leading axes of an N-D cube.
    ///
    /// `dims` lists the lengths of the leading (sliceable) axes in outer→inner
    /// order; an empty array means a plain 2D image (no slice controls). The
    /// widget shows a slider + play control per axis and emits slice requests via
    /// the `onSliceRequest` callback; the host serves each slice on demand and
    /// delivers it with `setSliceData`.
    #[wasm_bindgen(js_name = setCube)]
    pub fn set_cube(&self, dims: Vec<u32>) {
        let dims = dims.into_iter().map(|d| d as usize).collect();
        self.widget.borrow_mut().set_cube(dims);
    }

    /// Set image data for a specific cube slice.
    ///
    /// Like `setImageData` but tagged with the slice `indices` it corresponds to,
    /// so the widget can sync slider positions and correlate play-mode prefetch
    /// responses. `indices` may be empty for a plain 2D image.
    #[wasm_bindgen(js_name = setSliceData)]
    pub fn set_slice_data(
        &self,
        buffer: &js_sys::ArrayBuffer,
        width: u32,
        height: u32,
        array_type: &str,
        indices: Vec<u32>,
    ) -> Result<(), JsValue> {
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
        let indices = indices.into_iter().map(|i| i as usize).collect();

        let mut widget = self.widget.borrow_mut();
        widget.receive_slice(indices, pixels, width, height, is_integer, value_decimals);

        Ok(())
    }

    /// End event loop and release resources
    #[wasm_bindgen(js_name = destroy)]
    pub fn destroy(&self) {
        self.runner.destroy();
    }

    /// Zoom in by one step (1.25x)
    #[wasm_bindgen(js_name = zoomIn)]
    pub fn zoom_in(&self) {
        let mut widget = self.widget.borrow_mut();
        // Use a default viewport center - the actual center will be calculated in the UI
        let center = egui::pos2(400.0, 300.0);
        widget.zoom_in(None, center);
    }

    /// Zoom out by one step (1/1.25x)
    #[wasm_bindgen(js_name = zoomOut)]
    pub fn zoom_out(&self) {
        let mut widget = self.widget.borrow_mut();
        let center = egui::pos2(400.0, 300.0);
        widget.zoom_out(None, center);
    }

    /// Reset zoom and pan to fit-to-view
    #[wasm_bindgen(js_name = zoomToFit)]
    pub fn zoom_to_fit(&self) {
        let mut widget = self.widget.borrow_mut();
        widget.zoom_to_fit();
    }

    /// Set zoom level directly (1.0 = fit to view)
    #[wasm_bindgen(js_name = setZoom)]
    pub fn set_zoom(&self, level: f32) {
        let mut widget = self.widget.borrow_mut();
        let transform = widget.transform_mut();
        transform.zoom = level.clamp(transform::MIN_ZOOM, transform::MAX_ZOOM);
    }

    /// Get current zoom level (1.0 = fit to view)
    #[wasm_bindgen(js_name = getZoom)]
    pub fn get_zoom(&self) -> f32 {
        self.widget.borrow().zoom_level()
    }

    // =========================================================================
    // Rotation getters and setters
    // =========================================================================

    /// Get current rotation angle in degrees (counter-clockwise)
    #[wasm_bindgen(js_name = getRotation)]
    pub fn get_rotation(&self) -> f32 {
        self.widget.borrow().rotation()
    }

    /// Set rotation angle in degrees (counter-clockwise)
    #[wasm_bindgen(js_name = setRotation)]
    pub fn set_rotation(&self, degrees: f32) {
        self.widget.borrow_mut().set_rotation(degrees);
    }

    /// Get pivot point as [x, y] in image coordinates
    #[wasm_bindgen(js_name = getPivotPoint)]
    pub fn get_pivot_point(&self) -> js_sys::Float32Array {
        let (x, y) = self.widget.borrow().pivot_point();
        let result = js_sys::Float32Array::new_with_length(2);
        result.copy_from(&[x, y]);
        result
    }

    /// Set pivot point in image coordinates
    #[wasm_bindgen(js_name = setPivotPoint)]
    pub fn set_pivot_point(&self, x: f32, y: f32) {
        self.widget.borrow_mut().set_pivot_point(x, y);
    }

    /// Get whether the pivot marker is visible
    #[wasm_bindgen(js_name = getShowPivotMarker)]
    pub fn get_show_pivot_marker(&self) -> bool {
        self.widget.borrow().show_pivot_marker()
    }

    /// Set whether to show the pivot marker
    #[wasm_bindgen(js_name = setShowPivotMarker)]
    pub fn set_show_pivot_marker(&self, show: bool) {
        self.widget.borrow_mut().set_show_pivot_marker(show);
    }

    /// Get the overlay message shown at the bottom of the viewer.
    #[wasm_bindgen(js_name = getOverlayMessage)]
    pub fn get_overlay_message(&self) -> String {
        self.widget.borrow().overlay_message().to_string()
    }

    /// Set the overlay message shown at the bottom of the viewer.
    #[wasm_bindgen(js_name = setOverlayMessage)]
    pub fn set_overlay_message(&self, message: &str) {
        self.widget.borrow_mut().set_overlay_message(message);
    }

    /// Get point markers as a flat [x0, y0, x1, y1, ...] float array.
    #[wasm_bindgen(js_name = getMarkers)]
    pub fn get_markers(&self) -> js_sys::Float32Array {
        let widget = self.widget.borrow();
        let markers = widget.markers();
        let mut flat = Vec::with_capacity(markers.len() * 2);
        for &(x, y) in markers {
            flat.push(x);
            flat.push(y);
        }
        let result = js_sys::Float32Array::new_with_length(flat.len() as u32);
        result.copy_from(&flat);
        result
    }

    /// Set point markers from a flat [x0, y0, x1, y1, ...] float array.
    #[wasm_bindgen(js_name = setMarkers)]
    pub fn set_markers(&self, flat_points: Vec<f32>) {
        if flat_points.len() % 2 != 0 {
            return;
        }
        let markers: Vec<(f32, f32)> = flat_points
            .chunks_exact(2)
            .filter_map(|xy| {
                let x = xy[0];
                let y = xy[1];
                if x.is_finite() && y.is_finite() {
                    Some((x, y))
                } else {
                    None
                }
            })
            .collect();
        self.widget.borrow_mut().set_markers(markers);
    }

    // =========================================================================
    // Contrast/Bias/Stretch getters and setters
    // =========================================================================

    /// Get current contrast value (0.0 to 10.0, default 1.0)
    #[wasm_bindgen(js_name = getContrast)]
    pub fn get_contrast(&self) -> f64 {
        self.widget.borrow().current_contrast_bias().contrast
    }

    /// Set contrast value (0.0 to 10.0)
    #[wasm_bindgen(js_name = setContrast)]
    pub fn set_contrast(&self, contrast: f64) {
        self.widget.borrow_mut().set_contrast(contrast);
    }

    /// Get current bias value (0.0 to 1.0, default 0.5)
    #[wasm_bindgen(js_name = getBias)]
    pub fn get_bias(&self) -> f64 {
        self.widget.borrow().current_contrast_bias().bias
    }

    /// Set bias value (0.0 to 1.0)
    #[wasm_bindgen(js_name = setBias)]
    pub fn set_bias(&self, bias: f64) {
        self.widget.borrow_mut().set_bias(bias);
    }

    /// Get current stretch mode as string: "linear", "log", or "symmetric"
    #[wasm_bindgen(js_name = getStretchMode)]
    pub fn get_stretch_mode(&self) -> String {
        let widget = self.widget.borrow();
        if widget.is_symmetric() {
            "symmetric".to_string()
        } else {
            match widget.stretch_type() {
                widget::StretchType::Linear => "linear".to_string(),
                widget::StretchType::Log => "log".to_string(),
            }
        }
    }

    /// Set stretch mode: "linear", "log", or "symmetric"
    #[wasm_bindgen(js_name = setStretchMode)]
    pub fn set_stretch_mode(&self, mode: &str) {
        let mut widget = self.widget.borrow_mut();
        match mode {
            "linear" => {
                widget.set_symmetric(false);
                widget.set_stretch_type(widget::StretchType::Linear);
            }
            "log" => {
                widget.set_symmetric(false);
                widget.set_stretch_type(widget::StretchType::Log);
            }
            "symmetric" => {
                widget.set_stretch_type(widget::StretchType::Linear);
                widget.set_symmetric(true);
            }
            _ => {} // Ignore invalid modes
        }
    }

    /// Get visible image bounds as [xmin, xmax, ymin, ymax] in pixel coordinates.
    /// Returns the portion of the image currently visible in the viewport.
    /// If no image is loaded or bounds cannot be computed, returns [0, 0, 0, 0].
    #[wasm_bindgen(js_name = getViewBounds)]
    pub fn get_view_bounds(&self, viewport_width: f32, viewport_height: f32) -> js_sys::Float64Array {
        let widget = self.widget.borrow();
        let result = js_sys::Float64Array::new_with_length(4);
        
        if !widget.has_image() {
            result.copy_from(&[0.0, 0.0, 0.0, 0.0]);
            return result;
        }

        let (img_width, img_height) = widget.dimensions();
        let viewport_size = egui::vec2(viewport_width, viewport_height);
        let viewport_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, viewport_size);

        // Calculate base display size (fit-to-view)
        let img_aspect = img_width as f32 / img_height as f32;
        let viewport_aspect = viewport_width / viewport_height;
        let base_display_size = if img_aspect > viewport_aspect {
            egui::vec2(viewport_width, viewport_width / img_aspect)
        } else {
            egui::vec2(viewport_height * img_aspect, viewport_height)
        };

        // Get the image rect with current transform
        let image_rect = widget.transform().calculate_image_rect(viewport_rect, base_display_size);

        // Intersect with viewport to get visible region in screen coords
        let visible_screen = viewport_rect.intersect(image_rect);
        
        if visible_screen.width() <= 0.0 || visible_screen.height() <= 0.0 {
            result.copy_from(&[0.0, 0.0, 0.0, 0.0]);
            return result;
        }

        // Convert screen coords to image coords
        // Note: Y is flipped for FITS convention (Y=0 at bottom)
        let rel_x_min = (visible_screen.min.x - image_rect.min.x) / image_rect.width();
        let rel_x_max = (visible_screen.max.x - image_rect.min.x) / image_rect.width();
        // Flip Y: screen Y increases downward, but image Y=0 is at bottom
        let rel_y_max = 1.0 - (visible_screen.min.y - image_rect.min.y) / image_rect.height();
        let rel_y_min = 1.0 - (visible_screen.max.y - image_rect.min.y) / image_rect.height();

        let img_x_min = (rel_x_min * img_width as f32).max(0.0) as f64;
        let img_x_max = (rel_x_max * img_width as f32).min(img_width as f32) as f64;
        let img_y_min = (rel_y_min * img_height as f32).max(0.0) as f64;
        let img_y_max = (rel_y_max * img_height as f32).min(img_height as f32) as f64;

        result.copy_from(&[img_x_min, img_x_max, img_y_min, img_y_max]);
        result
    }

    /// Set view to show specific image bounds [xmin, xmax, ymin, ymax] in pixel coordinates.
    /// This adjusts zoom and pan to display the specified region.
    #[wasm_bindgen(js_name = setViewBounds)]
    pub fn set_view_bounds(
        &self,
        xmin: f64,
        xmax: f64,
        ymin: f64,
        ymax: f64,
        viewport_width: f32,
        viewport_height: f32,
    ) {
        let mut widget = self.widget.borrow_mut();
        
        if !widget.has_image() {
            return;
        }

        let (img_width, img_height) = widget.dimensions();
        let viewport_size = egui::vec2(viewport_width, viewport_height);

        // Calculate the requested region size in image pixels
        let region_width = (xmax - xmin) as f32;
        let region_height = (ymax - ymin) as f32;

        if region_width <= 0.0 || region_height <= 0.0 {
            return;
        }

        // Calculate base display size (fit-to-view)
        let img_aspect = img_width as f32 / img_height as f32;
        let viewport_aspect = viewport_width / viewport_height;
        let base_display_size = if img_aspect > viewport_aspect {
            egui::vec2(viewport_width, viewport_width / img_aspect)
        } else {
            egui::vec2(viewport_height * img_aspect, viewport_height)
        };

        // Calculate zoom needed to fit the region
        let region_aspect = region_width / region_height;
        let zoom_x = (img_width as f32 / region_width) * (base_display_size.x / viewport_width);
        let zoom_y = (img_height as f32 / region_height) * (base_display_size.y / viewport_height);
        let zoom = zoom_x.min(zoom_y).clamp(transform::MIN_ZOOM, transform::MAX_ZOOM);

        // Calculate center of the region in image coords
        let center_x = (xmin + xmax) as f32 / 2.0;
        let center_y = (ymin + ymax) as f32 / 2.0;

        // Set zoom and center on the region
        let transform = widget.transform_mut();
        transform.zoom = zoom;
        transform.center_on_image_point(
            egui::pos2(center_x, center_y),
            egui::vec2(img_width as f32, img_height as f32),
            viewport_size,
            egui::Rect::from_center_size(viewport_size.to_pos2() / 2.0, base_display_size),
        );
    }

    /// Get the colormap name
    #[wasm_bindgen(js_name = getColormap)]
    pub fn get_colormap(&self) -> String {
        self.widget.borrow().colormap().name().to_string()
    }

    /// Set the colormap by name.
    ///
    /// Accepted values include: Gray, Inferno, Magma, RdBu, RdYlBu.
    /// Invalid names are ignored.
    #[wasm_bindgen(js_name = setColormap)]
    pub fn set_colormap(&self, name: &str) {
        let Some(colormap) = Colormap::from_name(name) else {
            return;
        };
        self.widget.borrow_mut().set_colormap(colormap);
    }

    /// Get whether the colormap is reversed
    #[wasm_bindgen(js_name = getColormapReversed)]
    pub fn get_colormap_reversed(&self) -> bool {
        self.widget.borrow().is_reversed()
    }

    /// Set whether the colormap is reversed
    #[wasm_bindgen(js_name = setColormapReversed)]
    pub fn set_colormap_reversed(&self, reversed: bool) {
        self.widget.borrow_mut().set_reversed(reversed);
    }

    /// Get the image value range (vmin, vmax) as [min, max]
    #[wasm_bindgen(js_name = getValueRange)]
    pub fn get_value_range(&self) -> js_sys::Float64Array {
        let widget = self.widget.borrow();
        let (min_val, max_val) = widget.display_value_range();
        let result = js_sys::Float64Array::new_with_length(2);
        result.copy_from(&[min_val, max_val]);
        result
    }

    /// Set the image value range (vmin, vmax) for display scaling
    #[wasm_bindgen(js_name = setValueRange)]
    pub fn set_value_range(&self, min_val: f64, max_val: f64) {
        self.widget.borrow_mut().set_value_range(min_val, max_val);
    }

    // =========================================================================
    // Callback registration
    // =========================================================================

    /// Register a callback to be called when viewer state changes.
    /// The callback receives an object with the current state:
    /// { contrast, bias, stretchMode, zoom, xlim, ylim, colormap, colormapReversed, vmin, vmax }
    #[wasm_bindgen(js_name = onStateChange)]
    pub fn on_state_change(&self, callback: js_sys::Function) {
        self.callbacks.borrow_mut().on_state_change = Some(callback);
    }

    /// Register a callback to be called when the user shift-clicks on the image.
    /// The callback receives: { x, y } in continuous image coordinates.
    #[wasm_bindgen(js_name = onClick)]
    pub fn on_click(&self, callback: js_sys::Function) {
        self.callbacks.borrow_mut().on_click = Some(callback);
    }

    /// Register a callback invoked when the widget needs a cube slice fetched.
    /// The callback receives the requested slice indices as a number array.
    #[wasm_bindgen(js_name = onSliceRequest)]
    pub fn on_slice_request(&self, callback: js_sys::Function) {
        self.callbacks.borrow_mut().on_slice_request = Some(callback);
    }

    /// Clear all registered callbacks.
    #[wasm_bindgen(js_name = clearCallbacks)]
    pub fn clear_callbacks(&self) {
        let mut callbacks = self.callbacks.borrow_mut();
        callbacks.on_state_change = None;
        callbacks.on_click = None;
        callbacks.on_slice_request = None;
    }
}

/// Display metadata for a dtype: (is_integer, value_decimals for hover readout).
#[cfg(target_arch = "wasm32")]
fn dtype_meta(array_type: &str) -> (bool, usize) {
    let is_integer = matches!(
        array_type,
        "i8" | "u8" | "i16" | "u16" | "i32" | "u32" | "i64" | "u64"
    );
    let value_decimals = match array_type {
        "f32" => 4,
        "f64" => 6,
        _ => 0,
    };
    (is_integer, value_decimals)
}

/// Convert a JavaScript ArrayBuffer to Vec<f64> based on ArrayType string.
/// ArrayType values are Rust-style type specifiers (i8, u8, i16, etc.).
#[cfg(target_arch = "wasm32")]
fn convert_buffer_to_f64(buffer: &js_sys::ArrayBuffer, array_type: &str) -> Result<Vec<f64>, JsValue> {
    match array_type {
        "i8" => {
            let view = js_sys::Int8Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "u8" => {
            let view = js_sys::Uint8Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "i16" => {
            let view = js_sys::Int16Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "u16" => {
            let view = js_sys::Uint16Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "i32" => {
            let view = js_sys::Int32Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "u32" => {
            let view = js_sys::Uint32Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "i64" => {
            let view = js_sys::BigInt64Array::new(buffer);
            let len = view.length() as usize;
            let mut result = Vec::with_capacity(len);
            for i in 0..len {
                let val = view.get_index(i as u32);
                // i64 converts to f64 directly (may lose precision for very large values)
                result.push(val as f64);
            }
            Ok(result)
        }
        "u64" => {
            let view = js_sys::BigUint64Array::new(buffer);
            let len = view.length() as usize;
            let mut result = Vec::with_capacity(len);
            for i in 0..len {
                let val = view.get_index(i as u32);
                // u64 converts to f64 directly (may lose precision for very large values)
                result.push(val as f64);
            }
            Ok(result)
        }
        "f32" => {
            let view = js_sys::Float32Array::new(buffer);
            Ok(view.to_vec().into_iter().map(|v| v as f64).collect())
        }
        "f64" | _ => {
            // Default to Float64
            let view = js_sys::Float64Array::new(buffer);
            Ok(view.to_vec())
        }
    }
}
