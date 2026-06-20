//! ArrayViewerWidget - A self-contained egui widget for viewing 2D arrays/images
//!
//! This widget encapsulates all state and rendering logic for displaying array data,
//! including pan/zoom, stretch functions, colormaps, and overlays. Multiple instances
//! can be used side-by-side without sharing state.

use egui::{Color32, ColorImage, Key, PointerButton, Response, TextureHandle, TextureOptions, Ui, Vec2};
use egui_phosphor::regular as phosphor;

use crate::colormap::Colormap;
use crate::transform::{self, ViewTransform};

/// Default contrast value (DS9 default)
const DEFAULT_CONTRAST: f64 = 1.0;
/// Default bias value (DS9 default)
const DEFAULT_BIAS: f64 = 0.5;
/// Maximum contrast value (DS9 uses 0-10 range)
const MAX_CONTRAST: f64 = 10.0;
/// Minimum contrast value
const MIN_CONTRAST: f64 = 0.0;
/// Log stretch exponent (DS9 default for optical images)
const LOG_EXPONENT: f64 = 1000.0;
/// Color bar width in pixels
const COLORBAR_WIDTH: f32 = 32.0;
/// Maximum color bar height in pixels
const COLORBAR_MAX_HEIGHT: f32 = 300.0;
/// Color bar margin from edge
const COLORBAR_MARGIN: f32 = 10.0;
/// Duration to show zoom level overlay after zooming
const ZOOM_OVERLAY_DURATION: f64 = 0.5;
/// Default play interval in seconds (¼ s)
const DEFAULT_PLAY_INTERVAL: f64 = 0.25;
/// Selectable play intervals: (seconds, dropdown label, compact glyph for the
/// square selector button).
const PLAY_SPEEDS: [(f64, &str, &str); 4] = [
    (0.125, "1/8 s", "⅛"),
    (0.25, "1/4 s", "¼"),
    (0.5, "1/2 s", "½"),
    (1.0, "1 s", "1"),
];

/// Actions returned from zoom controls overlay
#[derive(Clone, Copy, Debug, PartialEq)]
enum ZoomAction {
    None,
    ZoomIn,
    ZoomOut,
    Reset,
    ResetRotation,
    RotateBy(f32),        // Rotate by delta degrees
    TogglePivotMarker,    // Toggle pivot marker visibility
    ResetPivot,           // Reset pivot to image center
    CenterOnPoint(f32, f32), // Center view on image point (x, y)
}

/// Actions returned from stretch controls overlay
#[derive(Clone, Copy, Debug, PartialEq)]
enum StretchAction {
    None,
    SetLinear,
    SetLog,
    SetDiverging,
    SetColormap(Colormap),
    ToggleReverse,
    ResetStretch,
}

/// Actions returned from the slice / play controls overlay
#[derive(Clone, Debug, PartialEq)]
enum SliceAction {
    None,
    /// Make the given axis the "live" one (radio selection).
    SetActive(usize),
    /// Manually set the index along the given axis (e.g. slider drag).
    SetIndex(usize, usize),
    /// Start playing the given axis.
    Play(usize),
    /// Stop playback.
    Pause,
    /// Change the play interval (seconds).
    SetSpeed(f64),
}

/// A prefetched slice held until it is time to display it (play mode).
struct PendingSlice {
    indices: Vec<usize>,
    pixels: Vec<f64>,
    width: u32,
    height: u32,
    is_integer: bool,
    value_decimals: usize,
}

/// Stretch function type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StretchType {
    Linear,
    Log,
}

impl Default for StretchType {
    fn default() -> Self {
        Self::Linear
    }
}

/// Contrast and bias settings for a stretch mode
#[derive(Clone, Copy, Debug)]
pub struct ContrastBias {
    pub contrast: f64,
    pub bias: f64,
}

impl Default for ContrastBias {
    fn default() -> Self {
        Self {
            contrast: DEFAULT_CONTRAST,
            bias: DEFAULT_BIAS,
        }
    }
}

impl ContrastBias {
    pub fn is_default(&self) -> bool {
        (self.contrast - DEFAULT_CONTRAST).abs() < 0.001
            && (self.bias - DEFAULT_BIAS).abs() < 0.001
    }
}

/// A self-contained widget for viewing 2D array/image data.
///
/// This widget owns all its state and can be embedded in any egui application.
/// Multiple instances can coexist without sharing state.
pub struct ArrayViewerWidget {
    // === Image data ===
    /// Raw pixel data as f64 values
    pixels: Option<Vec<f64>>,
    /// Image width in pixels
    width: u32,
    /// Image height in pixels
    height: u32,
    /// Computed min value for scaling
    min_val: f64,
    /// Computed max value for scaling
    max_val: f64,
    /// Original auto-computed min value (for reset)
    original_min_val: f64,
    /// Original auto-computed max value (for reset)
    original_max_val: f64,
    /// Whether the source data is integer-typed (for display formatting)
    is_integer: bool,
    /// Decimal places for floating hover values (based on source dtype)
    value_decimals: usize,

    // === View transformation ===
    /// Pan/zoom/rotation transformation state
    transform: ViewTransform,

    // === Rotation UI state ===
    /// Text buffer for rotation angle input (only applied on enter/defocus)
    rotation_input_text: String,
    /// Whether the rotation input field is currently focused
    rotation_input_focused: bool,

    // === Colorbar UI state ===
    /// Text buffer for min limit input (only applied on enter/defocus)
    min_limit_input_text: String,
    /// Text buffer for max limit input (only applied on enter/defocus)
    max_limit_input_text: String,

    // === Stretch settings ===
    /// Current stretch type (Linear or Log)
    stretch_type: StretchType,
    /// Contrast/bias settings for Linear mode
    linear_cb: ContrastBias,
    /// Contrast/bias settings for Log mode
    log_cb: ContrastBias,
    /// Contrast settings for Symmetric mode (bias is ignored)
    symmetric_cb: ContrastBias,
    /// Whether user is currently dragging to adjust contrast/bias
    is_adjusting_stretch: bool,

    // === Colormap ===
    /// Current colormap for standard (Lin/Log) modes
    standard_colormap: Colormap,
    /// Current colormap for symmetric/diverging mode
    diverging_colormap: Colormap,
    /// Symmetric mode (scale around zero)
    symmetric_mode: bool,
    /// Whether colormap is reversed
    colormap_reversed: bool,
    /// Keep current display limits when loading new arrays.
    colorbar_locked: bool,

    // === Rendering state ===
    /// Flag indicating texture needs rebuild
    texture_dirty: bool,
    /// Cached hover information: (image_x, image_y, raw_value), where x/y are
    /// continuous coordinates and integer values are pixel centers.
    hover_info: Option<(f64, f64, f64)>,
    /// Main image texture
    texture: Option<TextureHandle>,
    /// Colorbar texture
    colorbar_texture: Option<TextureHandle>,
    /// Track if right mouse button started a drag (for contrast/bias adjustment)
    stretch_drag_active: bool,
    /// Track when zoom was last changed (for overlay display)
    zoom_changed_time: Option<f64>,
    /// Previous zoom level to detect changes
    prev_zoom_level: f32,
    /// Whether to show build info overlay (debug)
    show_build_info: bool,
    /// Optional overlay message shown at the bottom of the viewer.
    overlay_message: String,
    /// Latest shift-click event in data coordinates, consumed by app callback code.
    pending_shift_click: Option<(f64, f64)>,
    /// Marker positions in continuous image coordinates (x, y).
    markers: Vec<(f32, f32)>,

    // === Cube / slice state ===
    /// Lengths of the sliceable leading axes (outer→inner). Empty = plain 2D.
    slice_dims: Vec<usize>,
    /// Target index along each sliceable axis — the scrubber handle / play
    /// position the user is pointing at (may lead [`displayed_indices`] while a
    /// slice is being fetched).
    current_indices: Vec<usize>,
    /// Indices of the slice actually shown on screen. Lags `current_indices`
    /// during a high-latency scrub; the loading overlay shows while they differ.
    displayed_indices: Vec<usize>,
    /// The "live" axis whose controls are enabled (selected via the radio).
    active_axis: usize,
    /// Which sliceable axis is currently playing, if any.
    playing_axis: Option<usize>,
    /// Play interval in seconds.
    play_interval: f64,
    /// egui monotonic time (seconds) at which the current frame was displayed.
    /// The play interval for the next frame is measured from this instant.
    display_time: f64,
    /// Indices currently requested from the host (one prefetch in flight).
    requested_indices: Option<Vec<usize>>,
    /// Prefetched frame held until the interval elapses (play mode only).
    pending_slice: Option<PendingSlice>,
    /// Slice-index request to emit to JS, drained each frame by the app shell.
    pending_slice_request: Option<Vec<usize>>,
}

impl Default for ArrayViewerWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl ArrayViewerWidget {
    fn format_limit_value(value: f64, is_int: bool) -> String {
        if is_int {
            format!("{}", value as i64)
        } else {
            format_scientific(value)
        }
    }

    /// Compute auto display limits from pixel data, ignoring NaNs/Infs.
    fn auto_limits_from_pixels(pixels: &[f64]) -> (f64, f64) {
        let mut min_val = f64::INFINITY;
        let mut max_val = f64::NEG_INFINITY;

        for &v in pixels {
            if v.is_finite() {
                if v < min_val {
                    min_val = v;
                }
                if v > max_val {
                    max_val = v;
                }
            }
        }

        if !min_val.is_finite() {
            min_val = 0.0;
        }
        if !max_val.is_finite() {
            max_val = 1.0;
        }
        if (max_val - min_val).abs() < f64::EPSILON {
            max_val = min_val + 1.0;
        }
        (min_val, max_val)
    }

    fn apply_limits(&mut self, min_val: f64, max_val: f64, update_original: bool) {
        self.min_val = min_val;
        self.max_val = max_val;
        if update_original {
            self.original_min_val = min_val;
            self.original_max_val = max_val;
        }
        self.min_limit_input_text = Self::format_limit_value(self.min_val, self.is_integer);
        self.max_limit_input_text = Self::format_limit_value(self.max_val, self.is_integer);
    }

    /// Create a new empty widget
    pub fn new() -> Self {
        Self {
            pixels: None,
            width: 0,
            height: 0,
            min_val: 0.0,
            max_val: 1.0,
            original_min_val: 0.0,
            original_max_val: 1.0,
            is_integer: false,
            value_decimals: 6,
            transform: ViewTransform::new(),
            rotation_input_text: "0".to_string(),
            rotation_input_focused: false,
            min_limit_input_text: "0".to_string(),
            max_limit_input_text: "1".to_string(),
            stretch_type: StretchType::default(),
            linear_cb: ContrastBias::default(),
            log_cb: ContrastBias::default(),
            symmetric_cb: ContrastBias::default(),
            is_adjusting_stretch: false,
            standard_colormap: Colormap::default(),
            diverging_colormap: Colormap::RdBu,
            symmetric_mode: false,
            colormap_reversed: false,
            colorbar_locked: false,
            texture_dirty: false,
            hover_info: None,
            texture: None,
            colorbar_texture: None,
            stretch_drag_active: false,
            zoom_changed_time: None,
            prev_zoom_level: 1.0,
            show_build_info: false,
            overlay_message: String::new(),
            pending_shift_click: None,
            markers: Vec::new(),
            slice_dims: Vec::new(),
            current_indices: Vec::new(),
            displayed_indices: Vec::new(),
            active_axis: 0,
            playing_axis: None,
            play_interval: DEFAULT_PLAY_INTERVAL,
            display_time: 0.0,
            requested_indices: None,
            pending_slice: None,
            pending_slice_request: None,
        }
    }

    // =========================================================================
    // Public API (called from outside, e.g., from JS via ViewerHandle)
    // =========================================================================

    /// Set new image data, computing min/max for auto-scaling.
    /// Pan is reset if dimensions change; zoom is always preserved.
    pub fn set_image(
        &mut self,
        pixels: Vec<f64>,
        width: u32,
        height: u32,
        is_integer: bool,
        value_decimals: usize,
    ) {
        // Check if dimensions changed
        let dimensions_changed = width != self.width || height != self.height;

        let (computed_min_val, computed_max_val) = Self::auto_limits_from_pixels(&pixels);

        self.pixels = Some(pixels);
        self.width = width;
        self.height = height;
        if !self.colorbar_locked {
            self.apply_limits(computed_min_val, computed_max_val, true);
        }
        self.texture_dirty = true;
        self.is_integer = is_integer;
        self.value_decimals = value_decimals;

        // Only reset pan if dimensions changed; always keep zoom
        if dimensions_changed {
            self.transform.reset_pan();
            // Initialize pivot point to image center when dimensions change
            self.transform.set_pivot_to_center(width, height);
        }
    }

    /// Check if we have image data
    pub fn has_image(&self) -> bool {
        self.pixels.is_some()
    }

    /// Get image dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Zoom in by one step
    pub fn zoom_in(&mut self, center: Option<egui::Pos2>, viewport_center: egui::Pos2) {
        self.transform.zoom_in(center, viewport_center);
    }

    /// Zoom out by one step
    pub fn zoom_out(&mut self, center: Option<egui::Pos2>, viewport_center: egui::Pos2) {
        self.transform.zoom_out(center, viewport_center);
    }

    /// Reset zoom and pan to fit-to-view (preserves rotation and pivot)
    pub fn zoom_to_fit(&mut self) {
        self.transform.reset_zoom_and_pan();
    }

    /// Get current zoom level (1.0 = fit to view)
    pub fn zoom_level(&self) -> f32 {
        self.transform.zoom
    }

    /// Get mutable reference to transform
    pub fn transform_mut(&mut self) -> &mut ViewTransform {
        &mut self.transform
    }

    /// Get reference to transform
    pub fn transform(&self) -> &ViewTransform {
        &self.transform
    }

    /// Check if view is at default state
    pub fn is_default_view(&self) -> bool {
        self.transform.is_default()
    }

    // =========================================================================
    // Rotation API
    // =========================================================================

    /// Get rotation angle in degrees (counter-clockwise)
    pub fn rotation(&self) -> f32 {
        self.transform.rotation()
    }

    /// Set rotation angle in degrees (counter-clockwise)
    pub fn set_rotation(&mut self, degrees: f32) {
        self.transform.set_rotation(degrees);
        // Only update the input text if user is not currently editing
        if !self.rotation_input_focused {
            self.rotation_input_text = format!("{:.1}", self.transform.rotation());
        }
    }

    /// Get pivot point in image coordinates
    pub fn pivot_point(&self) -> (f32, f32) {
        self.transform.pivot_point()
    }

    /// Set pivot point in image coordinates
    pub fn set_pivot_point(&mut self, x: f32, y: f32) {
        self.transform.set_pivot_point(x, y);
    }

    /// Get whether pivot marker is shown
    pub fn show_pivot_marker(&self) -> bool {
        self.transform.show_pivot_marker
    }

    /// Set whether pivot marker is shown
    pub fn set_show_pivot_marker(&mut self, show: bool) {
        self.transform.show_pivot_marker = show;
    }

    /// Set the overlay text shown at bottom-center.
    pub fn set_overlay_message(&mut self, message: &str) {
        self.overlay_message = message.to_string();
    }

    /// Get the current overlay text.
    pub fn overlay_message(&self) -> &str {
        &self.overlay_message
    }

    /// Consume and return the latest shift-click event (if any).
    pub fn take_shift_click_event(&mut self) -> Option<(f64, f64)> {
        self.pending_shift_click.take()
    }

    // =========================================================================
    // Cube / slice navigation
    // =========================================================================

    /// Declare the sliceable leading axes of an N-D cube (their lengths, in
    /// outer→inner order). An empty list means plain 2D — no slice controls.
    ///
    /// If the dimensions are unchanged this is a no-op (current indices and any
    /// in-flight playback are preserved). Otherwise indices reset to zero and
    /// playback stops. The host is responsible for pushing the initial slice via
    /// [`receive_slice`]; this method never requests data itself.
    pub fn set_cube(&mut self, dims: Vec<usize>) {
        if dims == self.slice_dims {
            return;
        }
        self.slice_dims = dims;
        self.current_indices = vec![0; self.slice_dims.len()];
        self.displayed_indices = self.current_indices.clone();
        // Default the live axis to the innermost (conventional spectral/time axis).
        self.active_axis = self.slice_dims.len().saturating_sub(1);
        self.playing_axis = None;
        self.requested_indices = None;
        self.pending_slice = None;
        self.pending_slice_request = None;
    }

    /// Deliver pixel data tagged with the slice `indices` it represents.
    ///
    /// While playing, a slice matching the in-flight prefetch request is held in
    /// `pending_slice` and shown later by [`update_playback`] once the interval
    /// elapses. Otherwise (manual navigation, or a non-cube image) it is
    /// displayed immediately.
    pub fn receive_slice(
        &mut self,
        indices: Vec<usize>,
        pixels: Vec<f64>,
        width: u32,
        height: u32,
        is_integer: bool,
        value_decimals: usize,
    ) {
        let is_prefetch = self.playing_axis.is_some()
            && self.requested_indices.as_ref() == Some(&indices);
        if is_prefetch {
            self.pending_slice = Some(PendingSlice {
                indices,
                pixels,
                width,
                height,
                is_integer,
                value_decimals,
            });
            return;
        }
        // Manual / unsolicited slice: display now.
        let fulfilled = self.requested_indices.as_ref() == Some(&indices);
        if fulfilled {
            self.requested_indices = None;
        }
        // An unsolicited slice (initial load / programmatic) also moves the
        // handle; a fulfilled scrub response must NOT, or it would snap the
        // handle back from where the user has dragged to.
        if !fulfilled && !indices.is_empty() {
            self.current_indices = indices.clone();
        }
        self.displayed_indices = indices;
        self.set_image(pixels, width, height, is_integer, value_decimals);

        // Coalesced follow-up: if the handle moved past this slice while it was
        // loading, request the latest target now. This keeps at most one scrub
        // request in flight — local connections see the intermediate frames,
        // high-latency ones skip straight to where the handle ended up.
        if self.playing_axis.is_none()
            && self.requested_indices.is_none()
            && !self.current_indices.is_empty()
            && self.current_indices != self.displayed_indices
        {
            self.requested_indices = Some(self.current_indices.clone());
            self.pending_slice_request = Some(self.current_indices.clone());
        }
    }

    /// Advance play-mode state. Called once per frame before texture rebuild.
    ///
    /// Display cadence is `max(interval, fetch_time)`: a prefetched frame is only
    /// shown once both the configured interval has elapsed *and* the data has
    /// arrived, then the next prefetch is triggered immediately.
    fn update_playback(&mut self, now: f64) {
        let Some(axis) = self.playing_axis else {
            return;
        };
        if axis >= self.slice_dims.len() {
            self.playing_axis = None;
            return;
        }

        // Promote a ready prefetched frame once the interval window has passed.
        let ready = self
            .pending_slice
            .as_ref()
            .map(|p| self.requested_indices.as_ref() == Some(&p.indices))
            .unwrap_or(false);
        let interval_elapsed = now - self.display_time >= self.play_interval;
        if ready && interval_elapsed {
            let p = self.pending_slice.take().unwrap();
            self.current_indices = p.indices.clone();
            self.displayed_indices = p.indices;
            self.requested_indices = None;
            self.display_time = now;
            self.set_image(p.pixels, p.width, p.height, p.is_integer, p.value_decimals);
        }

        // With no request in flight, trigger the next prefetch right away.
        if self.requested_indices.is_none() {
            let len = self.slice_dims[axis];
            let mut next = self.current_indices.clone();
            if len > 0 {
                next[axis] = (next[axis] + 1) % len; // loop back at the end
            }
            self.requested_indices = Some(next.clone());
            self.pending_slice_request = Some(next);
        }
    }

    /// Consume and return a pending slice-index request (if any) to be emitted
    /// to the host. Mirrors [`take_shift_click_event`].
    pub fn take_slice_request(&mut self) -> Option<Vec<usize>> {
        self.pending_slice_request.take()
    }

    /// Get the current marker list in continuous image coordinates.
    pub fn markers(&self) -> &[(f32, f32)] {
        &self.markers
    }

    /// Replace the marker list with points in continuous image coordinates.
    pub fn set_markers(&mut self, markers: Vec<(f32, f32)>) {
        self.markers = markers;
    }

    /// Check if pivot point is at the image center
    pub fn is_pivot_at_center(&self) -> bool {
        let center_x = (self.width as f32 - 1.0) / 2.0;
        let center_y = (self.height as f32 - 1.0) / 2.0;
        let (pivot_x, pivot_y) = self.transform.pivot_point();
        (pivot_x - center_x).abs() < 0.5 && (pivot_y - center_y).abs() < 0.5
    }

    // =========================================================================
    // Stretch / Colormap API
    // =========================================================================

    /// Get current stretch type
    pub fn stretch_type(&self) -> StretchType {
        self.stretch_type
    }

    /// Toggle between Linear and Log stretch
    pub fn toggle_stretch_type(&mut self) {
        self.stretch_type = match self.stretch_type {
            StretchType::Linear => StretchType::Log,
            StretchType::Log => StretchType::Linear,
        };
        self.texture_dirty = true;
    }

    /// Set stretch type directly (used by selectable labels)
    pub fn set_stretch_type(&mut self, stretch_type: StretchType) {
        if self.stretch_type != stretch_type {
            self.stretch_type = stretch_type;
            // If switching to log, disable symmetric mode (log doesn't work well with negative values)
            if stretch_type == StretchType::Log && self.symmetric_mode {
                self.symmetric_mode = false;
            }
            self.texture_dirty = true;
        }
    }

    /// Get current contrast/bias for the active stretch mode
    pub fn current_contrast_bias(&self) -> ContrastBias {
        if self.symmetric_mode {
            // In symmetric mode, always use default bias (0.5) to keep it centered
            ContrastBias {
                contrast: self.symmetric_cb.contrast,
                bias: DEFAULT_BIAS,
            }
        } else {
            match self.stretch_type {
                StretchType::Linear => self.linear_cb,
                StretchType::Log => self.log_cb,
            }
        }
    }

    /// Get mutable reference to current contrast/bias
    fn current_contrast_bias_mut(&mut self) -> &mut ContrastBias {
        if self.symmetric_mode {
            &mut self.symmetric_cb
        } else {
            match self.stretch_type {
                StretchType::Linear => &mut self.linear_cb,
                StretchType::Log => &mut self.log_cb,
            }
        }
    }

    /// Set contrast value directly (clamped to valid range)
    pub fn set_contrast(&mut self, contrast: f64) {
        let cb = self.current_contrast_bias_mut();
        cb.contrast = contrast.clamp(MIN_CONTRAST, MAX_CONTRAST);
        self.texture_dirty = true;
    }

    /// Set bias value directly (clamped to valid range)
    /// Note: In symmetric mode, bias is ignored (always 0.5)
    pub fn set_bias(&mut self, bias: f64) {
        if !self.symmetric_mode {
            let cb = self.current_contrast_bias_mut();
            cb.bias = bias.clamp(0.0, 1.0);
            self.texture_dirty = true;
        }
    }

    /// Adjust contrast/bias based on mouse drag delta
    pub fn adjust_contrast_bias(&mut self, dx: f32, dy: f32, viewport_size: Vec2) {
        // Check symmetric mode before borrowing mutably
        let is_symmetric = self.symmetric_mode;
        let cb = self.current_contrast_bias_mut();

        // In symmetric mode, only adjust contrast (ignore bias to keep scaling centered)
        if !is_symmetric {
            // Map horizontal to bias (0 to 1)
            cb.bias = (cb.bias + (dx as f64) / (viewport_size.x as f64)).clamp(0.0, 1.0);
        }

        // Map vertical to contrast (0 to MAX_CONTRAST)
        // Negate dy because screen Y increases downward but we want drag-up to increase contrast
        cb.contrast = (cb.contrast - (dy as f64) / (viewport_size.y as f64) * MAX_CONTRAST)
            .clamp(MIN_CONTRAST, MAX_CONTRAST);

        self.texture_dirty = true;
    }

    /// Reset contrast/bias for current stretch mode to defaults
    pub fn reset_current_stretch(&mut self) {
        *self.current_contrast_bias_mut() = ContrastBias::default();
        self.texture_dirty = true;
    }

    /// Reset all stretch settings (both modes) to defaults
    pub fn reset_all_stretch(&mut self) {
        self.linear_cb = ContrastBias::default();
        self.log_cb = ContrastBias::default();
        self.symmetric_cb = ContrastBias::default();
        self.stretch_type = StretchType::Linear;
        self.texture_dirty = true;
    }

    /// Check if current stretch mode has non-default contrast/bias
    pub fn is_stretch_modified(&self) -> bool {
        if self.symmetric_mode {
            // In symmetric mode, only contrast matters
            (self.symmetric_cb.contrast - DEFAULT_CONTRAST).abs() >= 0.001
        } else {
            !self.current_contrast_bias().is_default()
        }
    }

    /// Check if min/max limits have been modified from original values
    pub fn is_limits_modified(&self) -> bool {
        (self.min_val - self.original_min_val).abs() > 1e-10
            || (self.max_val - self.original_max_val).abs() > 1e-10
    }

    /// Check if any display settings (stretch or limits) have been modified
    pub fn is_display_modified(&self) -> bool {
        self.is_stretch_modified() || self.is_limits_modified()
    }

    /// Reset limits to original auto-computed values
    pub fn reset_limits(&mut self) {
        self.apply_limits(self.original_min_val, self.original_max_val, false);
        self.texture_dirty = true;
    }

    /// Reset display settings.
    /// If colorbar is locked, unlock and recompute limits from current image.
    pub fn reset_display(&mut self) {
        self.reset_current_stretch();
        if self.colorbar_locked {
            self.colorbar_locked = false;
            if let Some(pixels) = self.pixels.as_deref() {
                let (min_val, max_val) = Self::auto_limits_from_pixels(pixels);
                self.apply_limits(min_val, max_val, true);
                self.texture_dirty = true;
            } else {
                self.reset_limits();
            }
        } else {
            self.reset_limits();
        }
    }

    /// Set whether user is currently adjusting stretch
    pub fn set_adjusting_stretch(&mut self, adjusting: bool) {
        self.is_adjusting_stretch = adjusting;
    }

    /// Check if user is currently adjusting stretch
    pub fn is_adjusting_stretch(&self) -> bool {
        self.is_adjusting_stretch
    }

    /// Get current colormap (based on current mode)
    pub fn colormap(&self) -> Colormap {
        if self.symmetric_mode {
            self.diverging_colormap
        } else {
            self.standard_colormap
        }
    }

    /// Set colormap (stores in appropriate field based on colormap type)
    pub fn set_colormap(&mut self, colormap: Colormap) {
        if colormap.is_diverging() {
            self.diverging_colormap = colormap;
        } else {
            self.standard_colormap = colormap;
        }
        self.texture_dirty = true;
    }

    /// Check if symmetric mode is enabled
    pub fn is_symmetric(&self) -> bool {
        self.symmetric_mode
    }

    /// Enable symmetric/diverging mode
    pub fn set_symmetric(&mut self, enabled: bool) {
        if self.symmetric_mode != enabled {
            self.symmetric_mode = enabled;
            if enabled {
                self.max_limit_input_text =
                    Self::format_limit_value(self.max_val.abs(), self.is_integer);
                self.min_limit_input_text =
                    Self::format_limit_value(-self.max_val.abs(), self.is_integer);
            } else {
                self.max_limit_input_text =
                    Self::format_limit_value(self.max_val, self.is_integer);
                self.min_limit_input_text =
                    Self::format_limit_value(self.min_val, self.is_integer);
            }
            self.texture_dirty = true;
        }
    }

    /// Toggle symmetric mode
    pub fn toggle_symmetric(&mut self) {
        self.set_symmetric(!self.symmetric_mode);
    }

    /// Check if colormap is reversed
    pub fn is_reversed(&self) -> bool {
        self.colormap_reversed
    }

    /// Check whether colorbar limits are locked across array changes.
    pub fn is_colorbar_locked(&self) -> bool {
        self.colorbar_locked
    }

    /// Toggle colorbar lock state.
    pub fn toggle_colorbar_lock(&mut self) {
        self.colorbar_locked = !self.colorbar_locked;
    }

    /// Toggle colormap reversal
    pub fn toggle_reverse(&mut self) {
        self.colormap_reversed = !self.colormap_reversed;
        self.texture_dirty = true;
    }

    /// Set colormap reversal directly
    pub fn set_reversed(&mut self, reversed: bool) {
        if self.colormap_reversed != reversed {
            self.colormap_reversed = reversed;
            self.texture_dirty = true;
        }
    }

    // =========================================================================
    // Internal helpers
    // =========================================================================

    /// Get the scaling range based on symmetric mode
    fn scaling_range(&self) -> (f64, f64) {
        if self.symmetric_mode {
            let abs_max = self.max_val.abs();
            (-abs_max, abs_max)
        } else {
            (self.min_val, self.max_val)
        }
    }

    /// Get raw pixel value at image coordinates
    fn get_pixel_value(&self, x: u32, y: u32) -> Option<f64> {
        let pixels = self.pixels.as_ref()?;
        if x < self.width && y < self.height {
            let idx = (y as usize) * (self.width as usize) + (x as usize);
            pixels.get(idx).copied()
        } else {
            None
        }
    }

    /// Get min/max values
    pub fn value_range(&self) -> (f64, f64) {
        (self.min_val, self.max_val)
    }

    /// Get effective display limits used by rendering.
    pub fn display_value_range(&self) -> (f64, f64) {
        self.scaling_range()
    }

    /// Set the min value for scaling (marks texture dirty)
    pub fn set_min_val(&mut self, min_val: f64) {
        if (self.min_val - min_val).abs() > 1e-15 {
            self.min_val = min_val;
            self.min_limit_input_text = Self::format_limit_value(min_val, self.is_integer);
            self.texture_dirty = true;
        }
    }

    /// Set the max value for scaling (marks texture dirty)
    pub fn set_max_val(&mut self, mut max_val: f64) {
        if self.symmetric_mode {
            max_val = max_val.abs();
        }
        if (self.max_val - max_val).abs() > 1e-15 {
            self.max_val = max_val;
            self.max_limit_input_text = Self::format_limit_value(max_val, self.is_integer);
            self.texture_dirty = true;
        }
    }

    /// Set both min and max values at once
    pub fn set_value_range(&mut self, min_val: f64, max_val: f64) {
        if !self.symmetric_mode {
            self.set_min_val(min_val);
        }
        self.set_max_val(max_val);
    }

    /// Check if source data is integer-typed
    pub fn is_integer(&self) -> bool {
        self.is_integer
    }

    /// Get current hover info
    pub fn hover_info(&self) -> Option<(f64, f64, f64)> {
        self.hover_info
    }

    /// Apply full stretch pipeline to a single value
    /// Returns a value in 0-1 range suitable for colormap lookup
    fn apply_full_stretch(
        &self,
        v: f64,
        scale_min: f64,
        scale_max: f64,
        cb: ContrastBias,
        stretch_type: StretchType,
    ) -> f64 {
        // Step 1: Normalize to 0-1
        let range = scale_max - scale_min;
        let normalized = if v.is_finite() && range.abs() > f64::EPSILON {
            ((v - scale_min) / range).clamp(0.0, 1.0)
        } else {
            0.0 // NaN/Inf -> black
        };

        // Step 2: Apply stretch function
        let stretched = apply_stretch(normalized, stretch_type);

        // Step 3: Apply contrast/bias (DS9 formula)
        apply_contrast_bias(stretched, cb.contrast, cb.bias)
    }

    /// Build a ColorImage from the current pixel data using colormap
    fn build_color_image(&self) -> Option<ColorImage> {
        let pixels = self.pixels.as_ref()?;

        let (scale_min, scale_max) = self.scaling_range();
        let cb = self.current_contrast_bias();
        let stretch_type = self.stretch_type;
        let colormap = self.colormap();
        let reversed = self.colormap_reversed;

        let rgba: Vec<Color32> = pixels
            .iter()
            .map(|&v| {
                let mut adjusted = self.apply_full_stretch(v, scale_min, scale_max, cb, stretch_type);
                if reversed {
                    adjusted = 1.0 - adjusted;
                }
                colormap.map(adjusted)
            })
            .collect();

        Some(ColorImage {
            size: [self.width as usize, self.height as usize],
            pixels: rgba,
            source_size: egui::Vec2::new(self.width as f32, self.height as f32),
        })
    }

    /// Rebuild the main image texture
    fn rebuild_texture(&mut self, ctx: &egui::Context) {
        if let Some(color_image) = self.build_color_image() {
            self.texture = Some(ctx.load_texture(
                "image",
                color_image,
                TextureOptions::NEAREST,
            ));
        }
        // Also rebuild colorbar
        self.rebuild_colorbar_texture(ctx);
    }

    /// Rebuild the colorbar texture
    fn rebuild_colorbar_texture(&mut self, ctx: &egui::Context) {
        let height = 256;
        let width = 1;

        let cb = self.current_contrast_bias();
        let stretch_type = self.stretch_type;
        let colormap = self.colormap();
        let reversed = self.colormap_reversed;

        let pixels: Vec<Color32> = (0..height)
            .rev() // Reverse so high values are at top
            .map(|y| {
                let t = y as f64 / (height - 1) as f64;
                let stretched = apply_stretch(t, stretch_type);
                let mut adjusted = apply_contrast_bias(stretched, cb.contrast, cb.bias);
                if reversed {
                    adjusted = 1.0 - adjusted;
                }
                colormap.map(adjusted)
            })
            .collect();

        let color_image = ColorImage {
            size: [width, height],
            pixels,
            source_size: egui::Vec2::new(width as f32, height as f32),
        };

        self.colorbar_texture = Some(ctx.load_texture(
            "colorbar",
            color_image,
            TextureOptions::LINEAR,
        ));
    }

    // =========================================================================
    // Main rendering - called via egui::Widget trait
    // =========================================================================

    /// Show the widget, rendering into the given UI with a specified container size.
    ///
    /// The container_size determines how large the widget should render itself.
    /// This is typically the available space from the parent layout or window.
    pub fn show(&mut self, ui: &mut Ui, container_size: Vec2) -> Response {
        let ctx = ui.ctx().clone();

        // Advance play-mode state (may promote a prefetched slice into view).
        let now = ctx.input(|i| i.time);
        self.update_playback(now);

        // Check if texture needs rebuilding
        if self.texture_dirty {
            self.texture_dirty = false;
            self.rebuild_texture(&ctx);
        }

        // Handle keyboard shortcuts
        self.handle_keyboard_input(&ctx);

        // Allocate space for the widget
        let (rect, response) = ui.allocate_exact_size(container_size, egui::Sense::click_and_drag());

        if !self.has_image() {
            // Draw "no image" message
            let painter = ui.painter_at(rect);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No image loaded",
                egui::FontId::default(),
                ui.style().visuals.text_color(),
            );
            return response;
        }

        let (img_width, img_height) = self.dimensions();

        // Calculate base image display size (fit-to-view size)
        let available_size = container_size;
        let img_aspect = img_width as f32 / img_height as f32;
        let available_aspect = available_size.x / available_size.y;

        let base_display_size = if img_aspect > available_aspect {
            egui::vec2(available_size.x, available_size.x / img_aspect)
        } else {
            egui::vec2(available_size.y * img_aspect, available_size.y)
        };

        // Calculate the actual image rect with zoom and pan applied
        let viewport_rect = rect;
        let image_rect = self.transform.calculate_image_rect(viewport_rect, base_display_size);
        let viewport_center = viewport_rect.center();

        // Draw the image with rotation
        if let Some(texture) = &self.texture {
            let painter = ui.painter_at(rect);
            
            if self.transform.rotation().abs() < 0.001 {
                // No rotation - use simple image draw (faster)
                // Flip Y-axis for FITS convention: Y=0 at bottom
                painter.image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0)),
                    egui::Color32::WHITE,
                );
            } else {
                // With rotation - use mesh with rotated vertices
                let pivot_screen = self.transform.pivot_to_screen(image_rect, (img_width, img_height));
                // Screen +Y points downward; negate so positive rotation stays CCW
                // in image/math convention.
                let rotation_rad = -self.transform.rotation().to_radians();
                let cos_r = rotation_rad.cos();
                let sin_r = rotation_rad.sin();
                
                // Helper to rotate a point around pivot
                let rotate = |p: egui::Pos2| -> egui::Pos2 {
                    let dx = p.x - pivot_screen.x;
                    let dy = p.y - pivot_screen.y;
                    egui::pos2(
                        pivot_screen.x + dx * cos_r - dy * sin_r,
                        pivot_screen.y + dx * sin_r + dy * cos_r,
                    )
                };
                
                // Calculate rotated corners
                let tl = rotate(image_rect.left_top());
                let tr = rotate(image_rect.right_top());
                let br = rotate(image_rect.right_bottom());
                let bl = rotate(image_rect.left_bottom());
                
                // UV coordinates (Y-flipped for FITS convention)
                let uv_tl = egui::pos2(0.0, 1.0);
                let uv_tr = egui::pos2(1.0, 1.0);
                let uv_br = egui::pos2(1.0, 0.0);
                let uv_bl = egui::pos2(0.0, 0.0);
                
                // Build mesh with two triangles using Vertex struct
                let mut mesh = egui::Mesh::with_texture(texture.id());
                let color = egui::Color32::WHITE;
                mesh.vertices.push(egui::epaint::Vertex { pos: tl, uv: uv_tl, color });
                mesh.vertices.push(egui::epaint::Vertex { pos: tr, uv: uv_tr, color });
                mesh.vertices.push(egui::epaint::Vertex { pos: br, uv: uv_br, color });
                mesh.vertices.push(egui::epaint::Vertex { pos: bl, uv: uv_bl, color });
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                
                painter.add(egui::Shape::mesh(mesh));
            }
            
            // Draw pivot marker whenever the pivot is off-center, or while in pivot-placement mode.
            if self.transform.show_pivot_marker || !self.is_pivot_at_center() {
                let pivot_screen = self.transform.pivot_to_screen(image_rect, (img_width, img_height));
                self.render_pivot_marker(&painter, pivot_screen);
            }

            self.render_markers(
                &painter,
                image_rect,
                (img_width, img_height),
                ui.visuals().dark_mode,
            );
        }

        // Handle mouse wheel zoom
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if zoom_delta != 1.0 {
            if let Some(pointer_pos) = ui.input(|i| i.pointer.latest_pos()) {
                if response.rect.contains(pointer_pos) {
                    self.transform.zoom_around_point(zoom_delta, pointer_pos, viewport_center);
                }
            }
        }

        // Handle scroll wheel (for zoom when not using native zoom gesture)
        let scroll_delta = ui.input(|i| i.raw_scroll_delta);
        if scroll_delta.y != 0.0 && zoom_delta == 1.0 {
            if let Some(pointer_pos) = ui.input(|i| i.pointer.latest_pos()) {
                if response.rect.contains(pointer_pos) {
                    let zoom_factor = if scroll_delta.y > 0.0 {
                        transform::SCROLL_ZOOM_STEP
                    } else {
                        1.0 / transform::SCROLL_ZOOM_STEP
                    };
                    self.transform.zoom_around_point(zoom_factor, pointer_pos, viewport_center);
                }
            }
        }

        // Handle pan via drag
        let should_pan = response.dragged_by(PointerButton::Primary)
            || response.dragged_by(PointerButton::Middle);

        if should_pan {
            let drag_delta = response.drag_delta();
            if drag_delta != Vec2::ZERO {
                self.transform.pan_by(drag_delta);
            }
        }

        // Handle contrast/bias adjustment via right-click drag (DS9 style)
        if response.drag_started_by(PointerButton::Secondary) {
            self.stretch_drag_active = true;
            self.is_adjusting_stretch = true;
        }

        if self.stretch_drag_active && response.dragged_by(PointerButton::Secondary) {
            let drag_delta = response.drag_delta();
            if drag_delta != Vec2::ZERO {
                self.adjust_contrast_bias(drag_delta.x, drag_delta.y, available_size);
            }
        }

        if response.drag_stopped_by(PointerButton::Secondary) {
            self.stretch_drag_active = false;
            self.is_adjusting_stretch = false;
        }

        // Handle modifier+click interactions:
        // - Shift+click: emit callback in continuous data coordinates
        // - Cmd/Ctrl+click: center view on clicked point
        // - Cmd/Ctrl+Shift+click: set rotation pivot point
        let modifiers = ui.input(|i| i.modifiers);
        let has_cmd_or_ctrl = modifiers.command || modifiers.ctrl;

        if response.clicked() && modifiers.shift && !has_cmd_or_ctrl {
            if let Some(click_pos) = response.interact_pointer_pos() {
                if let Some((img_x, img_y)) = self.transform.screen_to_image_continuous_rotated(
                    click_pos,
                    image_rect,
                    (img_width, img_height),
                ) {
                    self.pending_shift_click = Some((img_x as f64, img_y as f64));
                }
            }
        }
        
        if response.clicked() && has_cmd_or_ctrl {
            if let Some(click_pos) = response.interact_pointer_pos() {
                if modifiers.shift {
                    // Cmd/Ctrl+Shift+click: set pivot point
                    if let Some((img_x, img_y)) = self.transform.screen_to_image_for_pivot(
                        click_pos,
                        image_rect,
                        (img_width, img_height),
                    ) {
                        self.transform.set_pivot_point(img_x as f32, img_y as f32);
                        // Pivot has been set; exit pivot-placement mode.
                        self.transform.show_pivot_marker = false;
                    }
                } else {
                    // Cmd/Ctrl+click: center view on point
                    if let Some((img_x, img_y)) = self.transform.screen_to_image_rotated(
                        click_pos,
                        image_rect,
                        (img_width, img_height),
                    ) {
                        self.transform.center_on_image_point(
                            egui::pos2(img_x as f32, img_y as f32),
                            egui::vec2(img_width as f32, img_height as f32),
                            available_size,
                            egui::Rect::from_center_size(viewport_center, base_display_size),
                        );
                    }
                }
            }
        }

        // Handle hover to show pixel value (using rotation-aware conversion)
        if let Some(hover_pos) = response.hover_pos() {
            if let Some((img_x, img_y)) = self.transform.screen_to_image_continuous_rotated(
                hover_pos,
                image_rect,
                (img_width, img_height),
            ) {
                let px = (img_x + 0.5).floor() as u32;
                let py = (img_y + 0.5).floor() as u32;
                if let Some(value) = self.get_pixel_value(px, py) {
                    self.hover_info = Some((img_x as f64, img_y as f64, value));
                } else {
                    self.hover_info = None;
                }
            } else {
                self.hover_info = None;
            }
        } else {
            self.hover_info = None;
        }

        // Track zoom changes for overlay display
        let current_zoom = self.zoom_level();
        let current_time = ctx.input(|i| i.time);
        if (current_zoom - self.prev_zoom_level).abs() > 0.001 {
            self.zoom_changed_time = Some(current_time);
            self.prev_zoom_level = current_zoom;
        }

        // Render overlays using Areas (they render at screen coordinates)
        // We collect actions from overlays and apply them after rendering
        let (control_action, zoom_controls_rect) = self.render_zoom_controls(&ctx, viewport_center, rect);
        let stretch_action = self.render_stretch_controls(&ctx, rect);
        self.render_colorbar(&ctx, rect);
        self.render_stretch_info_overlay(&ctx, rect);
        self.render_slice_scrub_overlay(&ctx, rect);
        self.render_zoom_info_overlay(&ctx, rect, current_time);
        self.render_pivot_hint_overlay(&ctx, rect);
        let hover_overlay_rect = self.render_hover_overlay(&ctx, rect);
        self.render_overlay_message(&ctx, rect, hover_overlay_rect, zoom_controls_rect);
        self.render_build_info(&ctx, rect);

        // Apply collected actions from bottom controls
        match control_action {
            ZoomAction::None => {}
            ZoomAction::ZoomIn => self.zoom_in(None, viewport_center),
            ZoomAction::ZoomOut => self.zoom_out(None, viewport_center),
            ZoomAction::Reset => self.zoom_to_fit(),
            ZoomAction::ResetRotation => {
                let current = self.transform.rotation();
                if current.abs() > 0.001 {
                    self.transform.set_rotation(0.0);
                    if !self.rotation_input_focused {
                        self.rotation_input_text = "0.0".to_string();
                    }
                }
            }
            ZoomAction::RotateBy(delta) => {
                self.transform.rotate_by(delta);
                // Only update text if user is not currently editing
                if !self.rotation_input_focused {
                    self.rotation_input_text = format!("{:.1}", self.transform.rotation());
                }
            }
            ZoomAction::TogglePivotMarker => {
                self.transform.show_pivot_marker = !self.transform.show_pivot_marker;
            }
            ZoomAction::ResetPivot => {
                self.transform.set_pivot_to_center(self.width, self.height);
            }
            ZoomAction::CenterOnPoint(x, y) => {
                self.transform.center_on_image_point(
                    egui::pos2(x, y),
                    egui::vec2(self.width as f32, self.height as f32),
                    egui::vec2(0.0, 0.0), // Will be computed
                    egui::Rect::NOTHING,
                );
            }
        }

        match stretch_action {
            StretchAction::None => {}
            StretchAction::SetLinear => {
                self.set_symmetric(false);
                self.set_stretch_type(StretchType::Linear);
            }
            StretchAction::SetLog => {
                self.set_symmetric(false);
                self.set_stretch_type(StretchType::Log);
            }
            StretchAction::SetDiverging => {
                self.set_stretch_type(StretchType::Linear);
                self.set_symmetric(true);
            }
            StretchAction::SetColormap(cmap) => self.set_colormap(cmap),
            StretchAction::ToggleReverse => self.toggle_reverse(),
            StretchAction::ResetStretch => self.reset_current_stretch(),
        }

        response
    }

    /// Apply an action collected from the slice / play controls.
    fn apply_slice_action(&mut self, action: SliceAction, now: f64) {
        match action {
            SliceAction::None => {}
            SliceAction::SetActive(axis) => {
                // Switching the live axis pauses any playback and does NOT start
                // the newly selected axis (the displayed frame is kept).
                self.playing_axis = None;
                self.requested_indices = None;
                self.pending_slice = None;
                self.active_axis = axis;
            }
            SliceAction::SetIndex(axis, index) => {
                // Manual scrubbing on the playing axis stops playback.
                if self.playing_axis == Some(axis) {
                    self.playing_axis = None;
                    self.pending_slice = None;
                }
                if let Some(slot) = self.current_indices.get_mut(axis) {
                    *slot = index;
                }
                // Coalesce: only fetch if nothing is already in flight. If a
                // request is pending, the follow-up in `receive_slice` will pick
                // up wherever the handle ends up — so a fast drag across many
                // frames never floods the backend.
                if self.requested_indices.is_none() {
                    self.requested_indices = Some(self.current_indices.clone());
                    self.pending_slice_request = Some(self.current_indices.clone());
                }
            }
            SliceAction::Play(axis) => {
                self.playing_axis = Some(axis);
                self.requested_indices = None;
                self.pending_slice = None;
                // Anchor the interval window at the moment play starts.
                self.display_time = now;
            }
            SliceAction::Pause => {
                self.playing_axis = None;
                self.requested_indices = None;
                self.pending_slice = None;
            }
            SliceAction::SetSpeed(interval) => {
                self.play_interval = interval;
            }
        }
    }

    /// Handle keyboard shortcuts for zoom
    fn handle_keyboard_input(&mut self, ctx: &egui::Context) {
        // Don't process keyboard shortcuts when any text input has focus
        let anything_focused = ctx.memory(|m| m.focused().is_some());
        if anything_focused {
            return;
        }

        let viewport_center = ctx.viewport(|vp| vp.this_pass.available_rect.center());

        ctx.input(|i| {
            // Zoom in: = or + (numpad)
            if i.key_pressed(Key::Equals) || i.key_pressed(Key::Plus) {
                self.zoom_in(None, viewport_center);
            }
            // Zoom out: - (minus)
            if i.key_pressed(Key::Minus) {
                self.zoom_out(None, viewport_center);
            }
            // Reset: 0
            if i.key_pressed(Key::Num0) {
                self.zoom_to_fit();
            }
            // Debug toggle
            if i.key_pressed(Key::F1) {
                self.show_build_info = !self.show_build_info;
            }
        });
    }

    /// Render bottom controls (rotation + zoom) as one container at bottom-right.
    /// Returns an action to be applied after rendering.
    fn render_zoom_controls(&mut self, ctx: &egui::Context, _viewport_center: egui::Pos2, _widget_rect: egui::Rect) -> (ZoomAction, egui::Rect) {
        let button_size = egui::vec2(28.0, 28.0);
        let small_button_size = egui::vec2(24.0, 28.0);
        let margin = 10.0;
        let spacing = 4.0;

        let mut action = ZoomAction::None;

        let area_response = egui::Area::new(egui::Id::new("zoom_controls"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-margin, -margin))
            .show(ctx, |ui| {
                // Get themed colors
                let frame_style = overlay_frame(ui);
                let text_color = get_overlay_text_color(ui);

                frame_style.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = spacing;

                        // Rotation controls
                        let pivot_label = if self.transform.show_pivot_marker {
                            phosphor::GPS_SLASH
                        } else {
                            phosphor::GPS
                        };
                        let pivot_btn = egui::Button::new(
                            egui::RichText::new(pivot_label).color(text_color).size(16.0)
                        ).fill(Color32::TRANSPARENT);
                        let pivot_response = ui.add_sized(button_size, pivot_btn);
                        if pivot_response.clicked() {
                            action = ZoomAction::TogglePivotMarker;
                        }
                        pivot_response.on_hover_text("Toggle rotation pivot marker (Cmd/Ctrl+Shift+click to set)");

                        let pivot_at_center = self.is_pivot_at_center();
                        let reset_pivot_btn = egui::Button::new(
                            egui::RichText::new(phosphor::GPS_FIX).color(if pivot_at_center {
                                text_color.gamma_multiply(0.4)
                            } else {
                                text_color
                            })
                        ).fill(Color32::TRANSPARENT);
                        let reset_response = ui.add_sized(small_button_size, reset_pivot_btn);
                        if !pivot_at_center && reset_response.clicked() {
                            action = ZoomAction::ResetPivot;
                        }
                        reset_response.on_hover_text(if pivot_at_center {
                            "Pivot is at image center"
                        } else {
                            "Reset pivot to image center"
                        });

                        let rotation_zero = self.transform.rotation().abs() < 0.001;
                        let reset_rotation_btn = egui::Button::new(
                            egui::RichText::new(phosphor::VECTOR_TWO).color(if rotation_zero {
                                text_color.gamma_multiply(0.4)
                            } else {
                                text_color
                            })
                        ).fill(Color32::TRANSPARENT);
                        let reset_rotation_response = ui.add_sized(small_button_size, reset_rotation_btn);
                        if !rotation_zero && reset_rotation_response.clicked() {
                            action = ZoomAction::ResetRotation;
                        }
                        reset_rotation_response.on_hover_text(if rotation_zero {
                            "Rotation is already zero"
                        } else {
                            "Reset rotation"
                        });

                        let ccw_btn = egui::Button::new(
                            egui::RichText::new(phosphor::ARROW_COUNTER_CLOCKWISE).color(text_color)
                        ).fill(Color32::TRANSPARENT);
                        if ui.add_sized(small_button_size, ccw_btn).on_hover_text("Rotate 15° CCW").clicked() {
                            action = ZoomAction::RotateBy(transform::ROTATION_STEP);
                        }

                        let text_edit = egui::TextEdit::singleline(&mut self.rotation_input_text)
                            .desired_width(50.0)
                            .horizontal_align(egui::Align::Center)
                            .font(egui::FontId::proportional(14.0));
                        let response = ui.add(text_edit);

                        if response.gained_focus() {
                            self.rotation_input_focused = true;
                        }
                        if response.lost_focus() || (self.rotation_input_focused && ui.input(|i| i.key_pressed(Key::Enter))) {
                            self.rotation_input_focused = false;
                            if let Ok(degrees) = self.rotation_input_text.parse::<f32>() {
                                let current = self.transform.rotation();
                                if (degrees - current).abs() > 0.001 {
                                    action = ZoomAction::RotateBy(degrees - current);
                                }
                            } else {
                                self.rotation_input_text = format!("{:.1}", self.transform.rotation());
                            }
                        }
                        response.on_hover_text("Rotation angle in degrees (CCW)");

                        let cw_btn = egui::Button::new(
                            egui::RichText::new(phosphor::ARROW_CLOCKWISE).color(text_color)
                        ).fill(Color32::TRANSPARENT);
                        if ui.add_sized(small_button_size, cw_btn).on_hover_text("Rotate 15° CW").clicked() {
                            action = ZoomAction::RotateBy(-transform::ROTATION_STEP);
                        }

                        ui.separator();

                        // Always show reset button, but disable when at default view
                        let can_reset = !self.is_default_view();
                        let reset_color = if can_reset { text_color } else { text_color.gamma_multiply(0.3) };
                        let reset_btn = egui::Button::new(
                            egui::RichText::new(phosphor::ARROWS_OUT)
                                .color(reset_color)
                        ).fill(Color32::TRANSPARENT);
                        let reset_response = ui.add_sized(button_size, reset_btn);
                        if can_reset && reset_response.clicked() {
                            action = ZoomAction::Reset;
                        }
                        reset_response.on_hover_text("Reset zoom to fit");

                        let minus_btn = egui::Button::new(
                            egui::RichText::new(phosphor::MINUS).color(text_color)
                        ).fill(Color32::TRANSPARENT);
                        if ui.add_sized(button_size, minus_btn).on_hover_text("Zoom out").clicked() {
                            action = ZoomAction::ZoomOut;
                        }

                        let plus_btn = egui::Button::new(
                            egui::RichText::new(phosphor::PLUS).color(text_color)
                        ).fill(Color32::TRANSPARENT);
                        if ui.add_sized(button_size, plus_btn).on_hover_text("Zoom in").clicked() {
                            action = ZoomAction::ZoomIn;
                        }
                    });
                });
            });

        (action, area_response.response.rect)
    }

    /// Render stretch controls at top-right of widget.
    /// Returns an action to be applied after rendering.
    fn render_stretch_controls(&self, ctx: &egui::Context, widget_rect: egui::Rect) -> StretchAction {
        let margin = 10.0;

        let mut action = StretchAction::None;

        egui::Area::new(egui::Id::new("stretch_controls"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-margin, margin))
            // Keep this overlay within the image area, below any slice bar above.
            .constrain_to(widget_rect)
            .show(ctx, |ui| {
                let stretch_type = self.stretch_type();
                let colormap = self.colormap();
                let symmetric = self.is_symmetric();
                let reversed = self.is_reversed();

                // Get themed colors for overlay
                let frame_style = overlay_frame(ui);
                let text_color = get_overlay_text_color(ui);

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    // Colormaps group with Rev toggle
                    frame_style.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if symmetric {
                                // Diverging colormaps for symmetric mode
                                for &cmap in Colormap::diverging_colormaps() {
                                    let selected = colormap == cmap;
                                    let label = egui::RichText::new(cmap.name()).color(text_color);
                                    if ui.selectable_label(selected, label).clicked() {
                                        action = StretchAction::SetColormap(cmap);
                                    }
                                }
                            } else {
                                // Standard colormaps for Lin/Log modes
                                for &cmap in Colormap::standard_colormaps() {
                                    let selected = colormap == cmap;
                                    let label = egui::RichText::new(cmap.name()).color(text_color);
                                    if ui.selectable_label(selected, label).clicked() {
                                        action = StretchAction::SetColormap(cmap);
                                    }
                                }
                            }

                            ui.separator();

                            // Reverse toggle
                            let rev_label = egui::RichText::new(phosphor::ARROWS_DOWN_UP).color(text_color);
                            if ui.selectable_label(reversed, rev_label).on_hover_text("Reverse colormap").clicked() {
                                action = StretchAction::ToggleReverse;
                            }
                        });
                    });

                    // Stretch modes group
                    frame_style.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let lin_label = egui::RichText::new("Lin").color(text_color);
                            if ui.selectable_label(stretch_type == StretchType::Linear && !symmetric, lin_label).on_hover_text("Linear stretch").clicked() {
                                action = StretchAction::SetLinear;
                            }
                            let log_label = egui::RichText::new("Log").color(text_color);
                            if ui.selectable_label(stretch_type == StretchType::Log, log_label).on_hover_text("Logarithmic stretch").clicked() {
                                action = StretchAction::SetLog;
                            }
                            let div_label = egui::RichText::new("±").color(text_color);
                            if ui.selectable_label(symmetric, div_label).on_hover_text("Symmetric scaling (diverging)").clicked() {
                                action = StretchAction::SetDiverging;
                            }
                        });
                    });
                });
            });

        action
    }

    /// Whether this viewer has sliceable cube axes (i.e. should show the slice bar).
    pub fn has_slices(&self) -> bool {
        !self.slice_dims.is_empty()
    }

    /// Render the cube slice + play controls into a dedicated bar (an egui panel
    /// above the pannable image, not an overlay on it). Every axis gets an
    /// identical row — radio (which axis is "live"), square play button, square
    /// speed selector, a full-width scrubber, and a fixed-width `index / total`
    /// readout. Only the live axis's controls are enabled. The radio column is
    /// omitted for a single axis (there's nothing to choose). No-op for plain 2D.
    pub fn show_slice_controls(&mut self, ui: &mut Ui) {
        if self.slice_dims.is_empty() {
            return;
        }
        let now = ui.input(|i| i.time);
        let mut action = SliceAction::None;

        // Square side shared by the play button and speed selector.
        let side = 22.0;
        let multi = self.slice_dims.len() > 1;

        let glyph = PLAY_SPEEDS
            .iter()
            .find(|(v, _, _)| (*v - self.play_interval).abs() < 1e-9)
            .map(|(_, _, g)| *g)
            .unwrap_or("¼");

        for axis in 0..self.slice_dims.len() {
            let len = self.slice_dims[axis];
            let cur = self.current_indices.get(axis).copied().unwrap_or(0);
            let max = len.saturating_sub(1);
            // Fixed width for each number cell: enough to hold the largest value
            // so the slash and totals never shift as the index changes.
            let num_w = (format!("{len}").len() as f32) * 8.0 + 4.0;
            let is_active = self.active_axis == axis;

            ui.push_id(axis, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    // Radio to pick the live axis (omitted when there's only one).
                    if multi && ui.radio(is_active, "").clicked() {
                        action = SliceAction::SetActive(axis);
                    }

                    // Everything else is identical per row, but only enabled for
                    // the live axis.
                    ui.add_enabled_ui(is_active, |ui| {
                        let playing = self.playing_axis == Some(axis);
                        let icon = if playing { phosphor::PAUSE } else { phosphor::PLAY };
                        let play_resp = ui.add_sized(
                            egui::vec2(side, side),
                            egui::Button::new(egui::RichText::new(icon).size(13.0)),
                        );
                        if play_resp.clicked() {
                            action = if playing {
                                SliceAction::Pause
                            } else {
                                SliceAction::Play(axis)
                            };
                        }
                        play_resp.on_hover_text(if playing { "Pause" } else { "Play" });

                        // Square speed selector; compact glyph on the button,
                        // full "N s" labels in the dropdown.
                        ui.spacing_mut().interact_size = egui::vec2(side, side);
                        ui.spacing_mut().button_padding = egui::vec2(2.0, 2.0);
                        egui::ComboBox::from_id_salt("slice_speed")
                            .selected_text(glyph)
                            .width(side)
                            .show_ui(ui, |ui| {
                                for (interval, label, _) in PLAY_SPEEDS {
                                    let selected = (self.play_interval - interval).abs() < 1e-9;
                                    if ui.selectable_label(selected, label).clicked() {
                                        action = SliceAction::SetSpeed(interval);
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Seconds per frame");

                        // Right side, laid out right-to-left: `index / total`
                        // with fixed-width cells, then the scrubber fills the gap.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_sized(
                                egui::vec2(num_w, side),
                                egui::Label::new(format!("{len}")).selectable(false),
                            );
                            ui.label("/");
                            ui.add_sized(
                                egui::vec2(num_w, side),
                                egui::Label::new(format!("{}", cur + 1)).selectable(false),
                            );

                            let mut idx = cur;
                            ui.spacing_mut().slider_width = (ui.available_width() - 4.0).max(40.0);
                            let resp =
                                ui.add(egui::Slider::new(&mut idx, 0..=max).show_value(false));
                            if resp.changed() && idx != cur {
                                action = SliceAction::SetIndex(axis, idx);
                            }
                        });
                    });
                });
            });
        }

        self.apply_slice_action(action, now);
    }

    /// Render colorbar overlay at top-left of widget with editable limit values
    fn render_colorbar(&mut self, ctx: &egui::Context, widget_rect: egui::Rect) {
        if !self.has_image() {
            return;
        }

        let is_int = self.is_integer;
        let bar_height = COLORBAR_MAX_HEIGHT.min(widget_rect.height() * 0.5);
        let bar_width = 16.0_f32;
        let bar_stroke_width = 1.0_f32;
        let bar_stroke_offset = 1.0_f32;
        let text_input_width = 70.0_f32;
        let spacing = 4.0_f32;
        let text_input_height = 20.0_f32;
        
        // Calculate positions
        let bar_pos = egui::pos2(widget_rect.min.x + COLORBAR_MARGIN, widget_rect.min.y + COLORBAR_MARGIN);
        let bar_rect = egui::Rect::from_min_size(bar_pos, egui::vec2(bar_width, bar_height));
        
        // Paint colorbar directly to the screen (no interaction, no Area)
        if let Some(texture) = &self.colorbar_texture {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("colorbar_paint")));
            painter.rect_stroke(
                bar_rect.expand(bar_stroke_offset),
                0.0,
                egui::Stroke::new(bar_stroke_width, Color32::GRAY),
                egui::StrokeKind::Outside,
            );
            painter.image(
                texture.id(),
                bar_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        
        // Max value text input - separate Area just for this widget
        let max_input_pos = egui::pos2(bar_rect.max.x + spacing, bar_rect.min.y);
        egui::Area::new(egui::Id::new("colorbar_max_input"))
            .fixed_pos(max_input_pos)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let is_dark = ui.visuals().dark_mode;
                let is_symmetric = self.is_symmetric();
                
                let edit_bg = if is_dark {
                    egui::Color32::from_black_alpha(180)
                } else {
                    egui::Color32::from_white_alpha(220)
                };
                let edit_bg_hover = if is_dark {
                    egui::Color32::from_black_alpha(200)
                } else {
                    egui::Color32::from_white_alpha(240)
                };
                let edit_bg_active = if is_dark {
                    egui::Color32::from_black_alpha(220)
                } else {
                    egui::Color32::from_white_alpha(255)
                };
                ui.style_mut().visuals.extreme_bg_color = edit_bg;
                ui.style_mut().visuals.widgets.inactive.bg_fill = edit_bg;
                ui.style_mut().visuals.widgets.hovered.bg_fill = edit_bg_hover;
                ui.style_mut().visuals.widgets.active.bg_fill = edit_bg_active;
                
                let max_edit = egui::TextEdit::singleline(&mut self.max_limit_input_text)
                    .desired_width(text_input_width)
                    .horizontal_align(egui::Align::Center)
                    .text_color(text_color)
                    .font(egui::FontId::proportional(13.0));
                let max_response = ui.add(max_edit);
                
                if max_response.lost_focus() || (max_response.has_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                    if let Ok(new_val) = self.max_limit_input_text.trim().parse::<f64>() {
                        let parsed = if is_symmetric { new_val.abs() } else { new_val };
                        self.set_max_val(parsed);
                        self.max_limit_input_text = Self::format_limit_value(parsed, is_int);
                    } else {
                        self.max_limit_input_text = if is_symmetric {
                            Self::format_limit_value(self.max_val.abs(), is_int)
                        } else {
                            Self::format_limit_value(self.max_val, is_int)
                        };
                    }
                }
                max_response.on_hover_text("Maximum display value");
            });
        
        // Min value text input - separate Area just for this widget
        let min_input_pos = egui::pos2(bar_rect.max.x + spacing, bar_rect.max.y - text_input_height);
        egui::Area::new(egui::Id::new("colorbar_min_input"))
            .fixed_pos(min_input_pos)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let is_dark = ui.visuals().dark_mode;
                let is_symmetric = self.is_symmetric();
                if is_symmetric {
                    self.min_limit_input_text =
                        Self::format_limit_value(-self.max_val.abs(), is_int);
                }
                
                let edit_bg = if is_dark {
                    egui::Color32::from_black_alpha(180)
                } else {
                    egui::Color32::from_white_alpha(220)
                };
                let edit_bg_hover = if is_dark {
                    egui::Color32::from_black_alpha(200)
                } else {
                    egui::Color32::from_white_alpha(240)
                };
                let edit_bg_active = if is_dark {
                    egui::Color32::from_black_alpha(220)
                } else {
                    egui::Color32::from_white_alpha(255)
                };
                ui.style_mut().visuals.extreme_bg_color = edit_bg;
                ui.style_mut().visuals.widgets.inactive.bg_fill = edit_bg;
                ui.style_mut().visuals.widgets.hovered.bg_fill = edit_bg_hover;
                ui.style_mut().visuals.widgets.active.bg_fill = edit_bg_active;
                
                let min_edit = egui::TextEdit::singleline(&mut self.min_limit_input_text)
                    .desired_width(text_input_width)
                    .horizontal_align(egui::Align::Center)
                    .text_color(text_color)
                    .font(egui::FontId::proportional(13.0));
                let min_response = ui.add_enabled(!is_symmetric, min_edit);
                
                if !is_symmetric
                    && (min_response.lost_focus()
                        || (min_response.has_focus() && ui.input(|i| i.key_pressed(Key::Enter))))
                {
                    if let Ok(new_val) = self.min_limit_input_text.trim().parse::<f64>() {
                        self.set_min_val(new_val);
                    } else {
                        self.min_limit_input_text = Self::format_limit_value(self.min_val, is_int);
                    }
                }
                min_response.on_hover_text(if is_symmetric {
                    "Symmetric mode: vmin is derived from -vmax"
                } else {
                    "Minimum display value"
                });
            });
        
        // Lock button below the colorbar - compact with theme background
        let lock_button_pos = egui::pos2(bar_rect.min.x, bar_rect.max.y + spacing);
        egui::Area::new(egui::Id::new("colorbar_lock_button"))
            .fixed_pos(lock_button_pos)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let bg_color = get_overlay_bg(ui);

                ui.style_mut().spacing.button_padding =
                    egui::vec2(bar_stroke_offset + bar_stroke_width, bar_stroke_offset + bar_stroke_width);

                let is_locked = self.is_colorbar_locked();
                let icon = if is_locked {
                    phosphor::LOCK
                } else {
                    phosphor::LOCK_OPEN
                };
                let btn_icon = egui::RichText::new(icon).color(text_color).size(12.0);
                let active_stroke = ui.visuals().widgets.active.bg_stroke;
                let locked_stroke = egui::Stroke::new(active_stroke.width, active_stroke.color);
                let btn = egui::Button::new(btn_icon)
                    .fill(bg_color)
                    .min_size(egui::vec2(bar_width, bar_width))
                    .stroke(if is_locked {
                        locked_stroke
                    } else {
                        egui::Stroke::NONE
                    });
                let response = ui.add(btn);
                if response.clicked() {
                    self.toggle_colorbar_lock();
                }
                response.on_hover_text(if is_locked {
                    "Colorbar locked (new arrays keep current limits)"
                } else {
                    "Colorbar unlocked (new arrays auto-reset limits)"
                });
            });

        // Reset button below lock button - compact with theme background
        let reset_button_pos = egui::pos2(bar_rect.min.x, bar_rect.max.y + spacing + bar_width + spacing);
        egui::Area::new(egui::Id::new("colorbar_reset_button"))
            .fixed_pos(reset_button_pos)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let bg_color = get_overlay_bg(ui);
                let is_modified = self.is_display_modified() || self.is_colorbar_locked();
                
                // Minimal styling with no padding to keep width tight
                ui.style_mut().spacing.button_padding = egui::vec2(bar_stroke_offset + bar_stroke_width, bar_stroke_offset + bar_stroke_width);
                
                let btn_icon = egui::RichText::new(phosphor::ARROW_COUNTER_CLOCKWISE)
                    .color(if is_modified { text_color } else { text_color.gamma_multiply(0.4) })
                    .size(12.0);
                let btn = egui::Button::new(btn_icon)
                    .fill(bg_color)
                    .min_size(egui::vec2(bar_width , bar_width));
                let response = ui.add_enabled(is_modified, btn);
                if response.clicked() {
                    self.reset_display();
                }
                response.on_hover_text("Reset contrast/bias and limits");
            });
    }

    /// Render contrast/bias values while adjusting
    fn render_stretch_info_overlay(&self, ctx: &egui::Context, widget_rect: egui::Rect) {
        if !self.is_adjusting_stretch() {
            return;
        }

        let cb = self.current_contrast_bias();
        let stretch_type = self.stretch_type();

        egui::Area::new(egui::Id::new("stretch_info_overlay"))
            .fixed_pos(egui::pos2(widget_rect.center().x - 80.0, widget_rect.min.y + 50.0))
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let bg = get_overlay_bg(ui);
                egui::Frame::popup(ui.style())
                    .fill(bg)
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        let mode_str = match stretch_type {
                            StretchType::Linear => "Linear",
                            StretchType::Log => "Log",
                        };
                        ui.label(
                            egui::RichText::new(format!(
                                "{} | Contrast: {:.2} | Bias: {:.2}",
                                mode_str, cb.contrast, cb.bias
                            ))
                            .color(text_color),
                        );
                    });
            });
    }

    /// Render a big centered index overlay while a scrubbed slice is still
    /// loading (the shown frame lags the handle). The frame draws over it once
    /// it arrives; on a high-latency scrub the number just keeps updating.
    fn render_slice_scrub_overlay(&self, ctx: &egui::Context, widget_rect: egui::Rect) {
        if self.slice_dims.is_empty() || self.current_indices == self.displayed_indices {
            return;
        }
        let axis = self
            .active_axis
            .min(self.slice_dims.len().saturating_sub(1));
        let cur = self.current_indices.get(axis).copied().unwrap_or(0);

        // Center on the image area (offset from screen center by the panel above).
        let offset = widget_rect.center() - ctx.screen_rect().center();
        egui::Area::new(egui::Id::new("slice_scrub_overlay"))
            .anchor(egui::Align2::CENTER_CENTER, offset)
            .interactable(false)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let bg = get_overlay_bg(ui);
                egui::Frame::NONE
                    .fill(bg)
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(24, 16))
                    .show(ui, |ui| {
                        // Extend (don't wrap) so the box sizes to the number.
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.label(
                            egui::RichText::new(format!("{}", cur + 1))
                                .color(text_color)
                                .size(48.0)
                                .strong(),
                        );
                    });
            });
    }

    /// Render zoom level overlay while zooming (similar to contrast adjustment overlay)
    fn render_zoom_info_overlay(&self, ctx: &egui::Context, widget_rect: egui::Rect, current_time: f64) {
        // Check if we should show the overlay (during and shortly after zoom changes)
        let should_show = if let Some(changed_time) = self.zoom_changed_time {
            (current_time - changed_time) < ZOOM_OVERLAY_DURATION
        } else {
            false
        };

        if !should_show {
            return;
        }

        let zoom_level = self.zoom_level();
        let zoom_text = format_zoom_multiple(zoom_level);

        egui::Area::new(egui::Id::new("zoom_info_overlay"))
            .fixed_pos(egui::pos2(widget_rect.center().x - 50.0, widget_rect.center().y - 20.0))
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let bg = get_overlay_bg(ui);
                egui::Frame::popup(ui.style())
                    .fill(bg)
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(16, 8))
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.label(
                            egui::RichText::new(zoom_text)
                                .color(text_color)
                                .size(24.0),
                        );
                    });
            });
    }

    /// Render build info at bottom-center of widget (debug toggle)
    fn render_build_info(&self, ctx: &egui::Context, widget_rect: egui::Rect) {
        if !self.show_build_info {
            return;
        }

        let margin = 10.0;

        egui::Area::new(egui::Id::new("build_info"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -margin))
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let bg = get_overlay_bg(ui);
                egui::Frame::popup(ui.style())
                    .fill(bg)
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(10, 4))
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.label(
                                egui::RichText::new(env!("BUILD_TIMESTAMP"))
                                    .color(text_color)
                                    .size(12.0),
                            );
                        });
                    });
            });
    }

    /// Render hint for pivot-point placement mode.
    fn render_pivot_hint_overlay(&self, ctx: &egui::Context, widget_rect: egui::Rect) {
        if !self.transform.show_pivot_marker {
            return;
        }

        let max_width = widget_rect.width() * 0.5;
        let top_left = egui::pos2(
            widget_rect.center().x - (max_width * 0.5),
            widget_rect.bottom() - (widget_rect.height() / 6.0),
        );
        egui::Area::new(egui::Id::new("pivot_hint_overlay"))
            .fixed_pos(top_left)
            .interactable(false)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let frame_style = overlay_frame(ui);
                frame_style.show(ui, |ui| {
                    ui.set_max_width(max_width);
                    ui.add_sized(
                        [max_width, 0.0],
                        egui::Label::new(
                            egui::RichText::new("Ctrl/Cmd+Shift+click to set rotation pivot point")
                                .color(text_color)
                                .size(16.0),
                        )
                        .wrap(),
                    );
                });
            });
    }

    /// Render overlay message at the bottom-center of the widget.
    fn render_overlay_message(
        &self,
        ctx: &egui::Context,
        widget_rect: egui::Rect,
        hover_overlay_rect: egui::Rect,
        zoom_controls_rect: egui::Rect,
    ) {
        if self.overlay_message.is_empty() {
            return;
        }

        let margin = 10.0;
        let safe_left = hover_overlay_rect.right() + margin;
        let safe_right = zoom_controls_rect.left() - margin;
        if safe_right <= safe_left + 40.0 {
            return;
        }
        let safe_width = safe_right - safe_left;
        let safe_center_x = (safe_left + safe_right) * 0.5;

        egui::Area::new(egui::Id::new("shift_click_hint_overlay"))
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(safe_center_x - widget_rect.center().x, -margin),
            )
            .interactable(false)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let frame_style = overlay_frame(ui);
                frame_style.show(ui, |ui| {
                    ui.set_max_width(safe_width);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(self.overlay_message.as_str())
                                .color(text_color)
                                .size(14.0),
                        )
                        .wrap(),
                    );
                });
            });
    }

    /// Render the rotation pivot marker at the given screen position
    fn render_pivot_marker(&self, painter: &egui::Painter, screen_pos: egui::Pos2) {
        let size = 12.0;
        let stroke_color = egui::Color32::from_rgba_unmultiplied(255, 100, 100, 200);
        let stroke = egui::Stroke::new(2.0, stroke_color);
        
        // Draw crosshair
        painter.line_segment(
            [
                egui::pos2(screen_pos.x - size, screen_pos.y),
                egui::pos2(screen_pos.x + size, screen_pos.y),
            ],
            stroke,
        );
        painter.line_segment(
            [
                egui::pos2(screen_pos.x, screen_pos.y - size),
                egui::pos2(screen_pos.x, screen_pos.y + size),
            ],
            stroke,
        );
        
        // Draw circle around crosshair
        painter.circle_stroke(screen_pos, size * 0.7, stroke);
    }

    /// Render point markers as fixed-size plus signs with contrasting outlines.
    fn render_markers(
        &self,
        painter: &egui::Painter,
        image_rect: egui::Rect,
        image_size: (u32, u32),
        dark_mode: bool,
    ) {
        if self.markers.is_empty() {
            return;
        }

        let outline_color = if dark_mode {
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 128)
        } else {
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 128)
        };
        let marker_color = egui::Color32::from_rgb(0, 255, 255);
        let half_size = 6.0;
        let outline = egui::Stroke::new(4.0, outline_color);
        let fill = egui::Stroke::new(2.0, marker_color);

        for &(x, y) in &self.markers {
            let screen_pos = self
                .transform
                .image_to_screen_continuous_rotated((x, y), image_rect, image_size);
            let h0 = egui::pos2(screen_pos.x - half_size, screen_pos.y);
            let h1 = egui::pos2(screen_pos.x + half_size, screen_pos.y);
            let v0 = egui::pos2(screen_pos.x, screen_pos.y - half_size);
            let v1 = egui::pos2(screen_pos.x, screen_pos.y + half_size);

            painter.line_segment([h0, h1], outline);
            painter.line_segment([v0, v1], outline);
            painter.line_segment([h0, h1], fill);
            painter.line_segment([v0, v1], fill);
        }
    }

    /// Render compact hover info overlay at bottom-left with fixed-width fields.
    fn render_hover_overlay(&self, ctx: &egui::Context, _widget_rect: egui::Rect) -> egui::Rect {
        let margin = 10.0;
        let value_chars = self.overlay_value_char_width();
        let (x_value, y_value, z_value) = match self.hover_info() {
            Some((x, y, value)) => {
                let x_txt = format!("{:>width$.2}", x as f64, width = value_chars);
                let y_txt = format!("{:>width$.2}", y as f64, width = value_chars);
                let z_txt = format!("{:>width$}", self.format_hover_value(value), width = value_chars);
                (x_txt, y_txt, z_txt)
            }
            None => (
                format!("{:>width$}", "--.--", width = value_chars),
                format!("{:>width$}", "--.--", width = value_chars),
                format!("{:>width$}", "--", width = value_chars),
            ),
        };

        let area_response = egui::Area::new(egui::Id::new("hover_overlay_compact"))
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(margin, -margin))
            .interactable(false)
            .show(ctx, |ui| {
                ui.style_mut().interaction.selectable_labels = false;
                let frame_style = overlay_frame(ui);
                let text_color = get_overlay_text_color(ui);
                let label_width = 18.0;

                frame_style.show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let mut row = |label: &str, value: &str| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.add_sized(
                                    [label_width, 18.0],
                                    egui::Label::new(egui::RichText::new(label).monospace().color(text_color))
                                        .selectable(false)
                                        .sense(egui::Sense::hover()),
                                );
                                ui.add(
                                    egui::Label::new(egui::RichText::new(value).monospace().color(text_color))
                                        .selectable(false)
                                        .sense(egui::Sense::hover()),
                                );
                            });
                        };

                        row("x:", &x_value);
                        row("y:", &y_value);
                        row("z:", &z_value);
                    });
                });
            });
        area_response.response.rect
    }

    fn format_hover_value(&self, value: f64) -> String {
        if !value.is_finite() {
            return value.to_string();
        }
        if self.is_integer() {
            format!("{}", value as i64)
        } else {
            let abs = value.abs();
            if abs >= 100_000.0 || (value != 0.0 && abs < 0.00001) {
                format!("{:.*e}", self.value_decimals, value)
            } else {
                format!("{:.*}", self.value_decimals, value)
            }
        }
    }

    /// Compute minimal stable width for hover value column (in characters),
    /// based on image dimensions and the current value formatting mode/range.
    fn overlay_value_char_width(&self) -> usize {
        let max_x = self.width.saturating_sub(1) as f64;
        let max_y = self.height.saturating_sub(1) as f64;
        let x_chars = format!("{:.2}", max_x).len();
        let y_chars = format!("{:.2}", max_y).len();

        let v0 = self.format_hover_value(self.min_val).len();
        let v1 = self.format_hover_value(self.max_val).len();
        let v2 = self.format_hover_value(0.0).len();
        let v_chars = v0.max(v1).max(v2).max(2); // "--" fallback

        x_chars.max(y_chars).max(v_chars)
    }
}

/// Apply stretch function to a normalized value (0-1)
fn apply_stretch(x: f64, stretch_type: StretchType) -> f64 {
    match stretch_type {
        StretchType::Linear => x,
        StretchType::Log => {
            (LOG_EXPONENT * x + 1.0).log10() / LOG_EXPONENT.log10()
        }
    }
}

/// Apply DS9-style contrast/bias transformation
fn apply_contrast_bias(x: f64, contrast: f64, bias: f64) -> f64 {
    ((x - bias) * contrast + 0.5).clamp(0.0, 1.0)
}

/// Format a float in scientific notation for compact display
fn format_scientific(v: f64) -> String {
    if v == 0.0 {
        "0".to_string()
    } else if v.abs() >= 1e4 || v.abs() < 1e-2 {
        format!("{:.2e}", v)
    } else {
        format!("{:.2}", v)
    }
}

/// Get a translucent background color appropriate for light/dark mode
fn get_overlay_bg(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_black_alpha(180)
    } else {
        Color32::from_white_alpha(220)
    }
}

/// Get text color appropriate for light/dark mode overlays
fn get_overlay_text_color(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::WHITE
    } else {
        Color32::from_gray(30)
    }
}

/// Create a frame style for overlay controls that adapts to light/dark mode
fn overlay_frame(ui: &Ui) -> egui::Frame {
    let bg = get_overlay_bg(ui);
    egui::Frame::NONE
        .fill(bg)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 4))
}

/// Format zoom level as a nice multiple string with consistent decimal places
fn format_zoom_multiple(zoom: f32) -> String {
    format!("{:.3}x", zoom)
}

#[cfg(test)]
mod slice_tests {
    use super::*;

    /// A 2x2 f64 slice payload for a given index list.
    fn deliver(w: &mut ArrayViewerWidget, indices: Vec<usize>) {
        w.receive_slice(indices, vec![0.0, 1.0, 2.0, 3.0], 2, 2, false, 6);
    }

    #[test]
    fn set_cube_resets_indices_and_clears_play() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![4]);
        assert_eq!(w.current_indices, vec![0]);

        // Same dims is a no-op that preserves state.
        w.current_indices = vec![2];
        w.set_cube(vec![4]);
        assert_eq!(w.current_indices, vec![2]);

        // Changing dims resets indices and stops playback.
        w.playing_axis = Some(0);
        w.set_cube(vec![3, 5]);
        assert_eq!(w.current_indices, vec![0, 0]);
        assert_eq!(w.playing_axis, None);
    }

    #[test]
    fn manual_slice_displays_immediately() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![4]);
        deliver(&mut w, vec![2]);
        assert_eq!(w.current_indices, vec![2]);
        assert!(w.has_image());
        assert!(w.pending_slice.is_none());
    }

    #[test]
    fn manual_set_index_emits_request() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![4]);
        deliver(&mut w, vec![0]);
        w.apply_slice_action(SliceAction::SetIndex(0, 3), 1.0);
        assert_eq!(w.current_indices, vec![3]);
        assert_eq!(w.take_slice_request(), Some(vec![3]));
    }

    #[test]
    fn play_waits_for_interval_when_fetch_is_fast() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![4]);
        w.play_interval = 0.25;
        deliver(&mut w, vec![0]);

        // Start playing at t=0; first tick prefetches the next slice.
        w.apply_slice_action(SliceAction::Play(0), 0.0);
        w.update_playback(0.0);
        assert_eq!(w.requested_indices, Some(vec![1]));
        assert_eq!(w.take_slice_request(), Some(vec![1]));

        // Fast fetch: data arrives early and is held, not shown.
        deliver(&mut w, vec![1]);
        assert!(w.pending_slice.is_some());
        w.update_playback(0.1);
        assert_eq!(w.current_indices, vec![0], "must wait out the interval");

        // Once the interval elapses, the frame is shown and the next prefetched.
        w.update_playback(0.25);
        assert_eq!(w.current_indices, vec![1]);
        assert!(w.pending_slice.is_none());
        assert_eq!(w.requested_indices, Some(vec![2]));
    }

    #[test]
    fn play_shows_when_available_if_fetch_is_slow() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![4]);
        w.play_interval = 0.25;
        deliver(&mut w, vec![0]);

        w.apply_slice_action(SliceAction::Play(0), 0.0);
        w.update_playback(0.0);
        assert_eq!(w.take_slice_request(), Some(vec![1]));

        // Interval passes with no data yet: stay on the current frame.
        w.update_playback(0.3);
        assert_eq!(w.current_indices, vec![0]);
        assert_eq!(w.requested_indices, Some(vec![1]));

        // Slow fetch finally arrives; the next tick shows it right away.
        deliver(&mut w, vec![1]);
        w.update_playback(0.31);
        assert_eq!(w.current_indices, vec![1]);
        assert_eq!(w.requested_indices, Some(vec![2]));
    }

    #[test]
    fn play_loops_back_to_start_at_the_end() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![3]);
        w.play_interval = 0.1;
        deliver(&mut w, vec![2]); // start on the last slice

        w.apply_slice_action(SliceAction::Play(0), 0.0);
        w.update_playback(0.0);
        assert_eq!(w.requested_indices, Some(vec![0]), "wraps past the end");
    }

    #[test]
    fn set_cube_defaults_live_axis_to_innermost() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![3, 4]);
        assert_eq!(w.active_axis, 1);
        w.set_cube(vec![5]);
        assert_eq!(w.active_axis, 0);
    }

    #[test]
    fn scrubbing_coalesces_to_one_request_then_jumps_to_latest() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![100]);
        w.receive_slice(vec![0], vec![0.0, 1.0, 2.0, 3.0], 2, 2, false, 6); // showing 0

        // Fast drag 10 -> 20 -> 99: only the first emits a request; the rest
        // coalesce (one in flight).
        w.apply_slice_action(SliceAction::SetIndex(0, 10), 0.0);
        assert_eq!(w.take_slice_request(), Some(vec![10]));
        w.apply_slice_action(SliceAction::SetIndex(0, 20), 0.0);
        w.apply_slice_action(SliceAction::SetIndex(0, 99), 0.0);
        assert_eq!(w.current_indices, vec![99], "handle follows the drag");
        assert_eq!(w.take_slice_request(), None, "no extra requests while one is in flight");

        // The in-flight slice (10) arrives; the handle is now at 99, so it jumps
        // straight there (skipping 20).
        w.receive_slice(vec![10], vec![0.0, 1.0, 2.0, 3.0], 2, 2, false, 6);
        assert_eq!(w.displayed_indices, vec![10]);
        assert_eq!(w.take_slice_request(), Some(vec![99]));

        // 99 arrives -> caught up, no further requests.
        w.receive_slice(vec![99], vec![0.0, 1.0, 2.0, 3.0], 2, 2, false, 6);
        assert_eq!(w.displayed_indices, vec![99]);
        assert_eq!(w.take_slice_request(), None);
    }

    #[test]
    fn switching_active_axis_pauses_without_starting_new() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![3, 4]); // two axes; live axis defaults to 1
        w.receive_slice(vec![0, 0], vec![0.0, 1.0, 2.0, 3.0], 2, 2, false, 6);

        w.apply_slice_action(SliceAction::Play(1), 0.0);
        assert_eq!(w.playing_axis, Some(1));

        // Selecting another axis pauses playback and makes it live, but does not
        // begin playing the new axis.
        w.apply_slice_action(SliceAction::SetActive(0), 0.5);
        assert_eq!(w.active_axis, 0);
        assert_eq!(w.playing_axis, None);
        assert_eq!(w.requested_indices, None);
    }
}
