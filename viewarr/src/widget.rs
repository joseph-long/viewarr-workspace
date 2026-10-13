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
const COLORBAR_BAR_WIDTH: f32 = 16.0;
/// Maximum color bar height in pixels
const COLORBAR_MAX_HEIGHT: f32 = 300.0;
/// Color bar margin from edge
const COLORBAR_MARGIN: f32 = 10.0;
/// Vertical inset of the bar from the widget's top.
///
/// Deeper than the side margin by half a value box: a limit sitting at the very
/// top of the bar centres its box on the bar's top edge, so half of it hangs
/// above. Buying that room here is what lets every box stay centred on its own
/// arrow instead of being nudged back inside and trailing a sloped leader.
const COLORBAR_VERTICAL_MARGIN: f32 = COLORBAR_MARGIN + COLORBAR_BOX_HEIGHT * 0.5;
/// Length of the arrow that points from a limit handle at the bar.
const COLORBAR_ARROW_W: f32 = 6.0;
/// Half-height of that arrow, i.e. how blunt its base is.
const COLORBAR_ARROW_HALF_H: f32 = 5.0;
/// Gap between the arrow's base and the gripped tab.
const COLORBAR_STEM_W: f32 = 5.0;
/// Width of the gripped tab a limit is dragged by.
const COLORBAR_GRIP_W: f32 = 12.0;
/// Height of that tab.
const COLORBAR_GRIP_H: f32 = 16.0;
/// Horizontal distance from the bar's right edge to the value boxes, leaving
/// room for the arrow, stem and grip in between.
const COLORBAR_CONNECTOR_W: f32 =
    COLORBAR_ARROW_W + COLORBAR_STEM_W + COLORBAR_GRIP_W + COLORBAR_STEM_W;
/// How far the tallest histogram bin reaches out from the bar. Bins are
/// normalised to this, so the shape stays readable whatever the pixel counts.
const COLORBAR_HISTOGRAM_W: f32 = 90.0;
/// Height of a limit value box.
const COLORBAR_BOX_HEIGHT: f32 = 20.0;
/// Width of a limit value box.
const COLORBAR_BOX_WIDTH: f32 = 70.0;
/// The box's full footprint including the text edit's own margins, used to
/// decide when the pointer counts as being on the colorbar.
const COLORBAR_BOX_OUTER_W: f32 = COLORBAR_BOX_WIDTH + 14.0;
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

/// A value histogram binned to the colorbar's own extent, cached because it
/// costs a full pass over the image and the bars do not change while the user
/// is simply moving the pointer around.
struct ColorbarHistogram {
    /// The value span the bins were computed for; a different span needs a
    /// different histogram.
    span: (f64, f64),
    /// Log-scaled bin heights normalised to the tallest bin, bottom bin first.
    /// Normalising keeps the drawn width independent of how many pixels landed
    /// in the mode; the log keeps the sparse tail visible, which for the images
    /// this viewer is for is most of the interesting part.
    bins: Vec<f32>,
}

/// Which of the two colorbar limits a handle drives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LimitHandle {
    Min,
    Max,
}

/// An in-progress drag of a colorbar limit handle.
///
/// The value span is captured when the drag starts and held for its duration:
/// the span is derived from the very limits being dragged, so recomputing it
/// each frame would rescale the bar under the pointer and leave the value
/// chasing itself.
#[derive(Clone, Copy)]
struct ColorbarDrag {
    handle: LimitHandle,
    span: (f64, f64),
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
    /// The colorbar limit handle currently being dragged, if any.
    colorbar_drag: Option<ColorbarDrag>,
    /// Cached value histogram drawn over the colorbar on hover/drag.
    colorbar_histogram: Option<ColorbarHistogram>,
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
            colorbar_drag: None,
            colorbar_histogram: None,
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
        // New pixels, so the cached histogram describes the wrong image.
        self.colorbar_histogram = None;
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

    /// Abandon an in-flight slice request the host could not fulfil.
    ///
    /// Hosts must call this when a fetch fails, because the coalescing in
    /// [`apply_slice_action`] only issues a new request while none is in
    /// flight: a dropped request would leave `requested_indices` set forever
    /// and the scrubber silently dead. Playback stops too — a slice that cannot
    /// be fetched cannot be played through. The currently displayed image is
    /// left alone, so the viewer keeps showing the last good slice.
    pub fn fail_slice_request(&mut self, indices: &[usize]) {
        // Only the request actually in flight can be failed. A stale or
        // duplicate report — for a slice that already arrived, or one this
        // widget never asked for — must not stop playback as a side effect.
        if self.requested_indices.as_deref() != Some(indices) {
            return;
        }
        self.requested_indices = None;
        self.playing_axis = None;
        self.pending_slice = None;
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

        // Shares ramp_color with the saturated end bands, so the bands and the
        // gradient they abut are guaranteed to be the same colour.
        let pixels: Vec<Color32> = (0..height)
            .rev() // Reverse so high values are at top
            .map(|y| self.ramp_color(y as f64 / (height - 1) as f64))
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

    /// Data values the colorbar spans, bottom edge to top edge.
    ///
    /// The bar always covers the whole data range, so a limit pulled inside it
    /// leaves a flat band at the end it moved away from. Those bands are not
    /// decoration: every pixel out there really is drawn in that one colour.
    /// A limit pushed *past* the data stretches the bar instead, because then
    /// nothing saturates and there is no band to show.
    fn colorbar_span(&self) -> (f64, f64) {
        let (vmin, vmax) = self.scaling_range();

        // Symmetric mode ties vmin to -vmax, so the bar has to be symmetric
        // about zero too. Spanning the raw data instead would put zero off
        // centre, and then the two handles -- which are always mirror images --
        // would reach the ends of the bar at different moments: the derived
        // vmin bottoming out while vmax still had room, and dragging vmax
        // further would go on moving vmin with nowhere left to show it. Half
        // the bar per sign, scaled to whichever magnitude is larger, keeps the
        // pair in step and makes the gradient, the bands and the histogram all
        // agree about where zero is.
        if self.symmetric_mode {
            let extent = vmax
                .abs()
                .max(self.original_min_val.abs())
                .max(self.original_max_val.abs());
            if !extent.is_finite() || extent < f64::EPSILON {
                return (-1.0, 1.0);
            }
            return (-extent, extent);
        }

        let lo = vmin.min(self.original_min_val);
        let hi = vmax.max(self.original_max_val);
        if !lo.is_finite() || !hi.is_finite() {
            return (0.0, 1.0);
        }
        if hi - lo < f64::EPSILON {
            return (lo, lo + 1.0);
        }
        (lo, hi)
    }

    /// Colour at `t` along the ramp with the current stretch, contrast/bias and
    /// reversal applied — the same chain [`Self::rebuild_colorbar_texture`]
    /// uses, so the saturated bands meet the gradient without a seam.
    fn ramp_color(&self, t: f64) -> Color32 {
        let cb = self.current_contrast_bias();
        let stretched = apply_stretch(t, self.stretch_type);
        let mut adjusted = apply_contrast_bias(stretched, cb.contrast, cb.bias);
        if self.colormap_reversed {
            adjusted = 1.0 - adjusted;
        }
        self.colormap().map(adjusted)
    }

    /// The painter every non-interactive part of the colorbar draws into. One
    /// shared layer keeps the bar, arrows and leader lines in a known order.
    fn colorbar_painter(ctx: &egui::Context) -> egui::Painter {
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("colorbar_paint"),
        ))
    }

    /// Bin the image's values to the colorbar's extent, reusing the cache when
    /// the span and resolution are unchanged.
    ///
    /// Binning is a full pass over the pixels, so this must not run per frame;
    /// the span is stable while the pointer merely hovers, and frozen outright
    /// while a handle is dragged, so the cache holds for the whole interaction.
    fn colorbar_histogram_bins(&mut self, span: (f64, f64), bin_count: usize) -> &[f32] {
        let stale = match &self.colorbar_histogram {
            Some(hist) => hist.bins.len() != bin_count || hist.span != span,
            None => true,
        };

        if stale {
            let mut bins = vec![0.0_f32; bin_count];
            if let Some(pixels) = self.pixels.as_deref() {
                let (lo, hi) = span;
                let scale = if hi > lo {
                    bin_count as f64 / (hi - lo)
                } else {
                    0.0
                };
                for &v in pixels {
                    // NaN is "no value" here exactly as it is for auto-scaling,
                    // so it belongs in no bin.
                    if !v.is_finite() {
                        continue;
                    }
                    let idx = (((v - lo) * scale) as isize).clamp(0, bin_count as isize - 1);
                    bins[idx as usize] += 1.0;
                }
            }
            // Sky frames put almost every pixel into a handful of bins near the
            // background level, so on a linear scale everything else -- the
            // sources, the tail the limits are usually set against -- collapses
            // to an invisible sliver. log1p keeps that tail legible while still
            // showing which bins dominate, and maps an empty bin to exactly
            // zero so the outline sits flat on the bar where there is no data.
            for bin in &mut bins {
                *bin = bin.ln_1p();
            }
            let peak = bins.iter().copied().fold(0.0_f32, f32::max);
            if peak > 0.0 {
                for bin in &mut bins {
                    *bin /= peak;
                }
            }
            self.colorbar_histogram = Some(ColorbarHistogram { span, bins });
        }

        &self.colorbar_histogram.as_ref().expect("just populated").bins
    }

    /// Trace the histogram beside the bar as a stepped hairline.
    ///
    /// Only the outline is drawn — the top of each bin and the sides where
    /// neighbouring bins differ — so the shape reads without hiding the ramp
    /// or the image underneath. It is stroked twice, a dark halo under a light
    /// hairline, which keeps it legible over both the colour ramp and whatever
    /// part of the image lies beyond it. (egui has no inverting blend mode
    /// short of a custom paint callback, so a halo stands in for one.)
    fn paint_colorbar_histogram(&self, ctx: &egui::Context, bar: egui::Rect, bins: &[f32]) {
        if bins.is_empty() || bar.height() <= 0.0 {
            return;
        }

        let bin_height = bar.height() / bins.len() as f32;
        let mut points = Vec::with_capacity(bins.len() * 2 + 2);
        // Start and finish on the bar's edge so the outline closes against it
        // rather than floating.
        points.push(egui::pos2(bar.max.x, bar.max.y));
        for (i, &value) in bins.iter().enumerate() {
            // Bin 0 holds the lowest values, which sit at the bottom of the bar.
            let top = bar.max.y - (i + 1) as f32 * bin_height;
            let bottom = bar.max.y - i as f32 * bin_height;
            let x = bar.max.x + value * COLORBAR_HISTOGRAM_W;
            points.push(egui::pos2(x, bottom));
            points.push(egui::pos2(x, top));
        }
        points.push(egui::pos2(bar.max.x, bar.min.y));

        let painter = Self::colorbar_painter(ctx);
        painter.add(egui::Shape::line(
            points.clone(),
            egui::Stroke::new(3.0, Color32::from_black_alpha(160)),
        ));
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(1.0, Color32::WHITE),
        ));
    }

    /// Render colorbar overlay at top-left of widget with draggable limits.
    fn render_colorbar(&mut self, ctx: &egui::Context, widget_rect: egui::Rect) {
        if !self.has_image() {
            return;
        }

        let is_int = self.is_integer;
        let is_symmetric = self.is_symmetric();
        let bar_height = COLORBAR_MAX_HEIGHT.min(widget_rect.height() * 0.5);
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(
                widget_rect.min.x + COLORBAR_MARGIN,
                widget_rect.min.y + COLORBAR_VERTICAL_MARGIN,
            ),
            egui::vec2(COLORBAR_BAR_WIDTH, bar_height),
        );

        // Hold the span still for the length of a drag; see ColorbarDrag.
        let span = match self.colorbar_drag {
            Some(drag) => drag.span,
            None => self.colorbar_span(),
        };
        let (vmin, vmax) = self.scaling_range();
        // Each box is centred on its own handle, so an arrow and its value are
        // always on the same line.
        let max_y = colorbar_value_to_y(bar_rect, span, vmax);
        let min_y = colorbar_value_to_y(bar_rect, span, vmin);

        // Everything painted into the shared layer goes down first, so the
        // interactive grips and the value boxes land on top of it.
        self.paint_colorbar_ramp(ctx, bar_rect, max_y, min_y);

        // The histogram is a hint about where the data actually lives, so it
        // appears only while the user is working the colorbar -- hovering it or
        // dragging a limit -- and stays out of the way otherwise.
        //
        // It goes into the shared layer before the connectors, and the grips
        // and value boxes are separate Areas drawn after it, so it always
        // passes *under* the controls rather than scribbling over them.
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        // The whole group counts as "the colorbar", value boxes included: the
        // boxes are what a limit is read off, so reaching for one is reaching
        // for the limit. The boxes hang half a box above and below the bar's
        // ends when a limit is parked there, hence the vertical slack.
        let hover_zone = egui::Rect::from_min_max(
            egui::pos2(bar_rect.min.x, bar_rect.min.y - COLORBAR_BOX_HEIGHT * 0.5),
            egui::pos2(
                bar_rect.max.x + COLORBAR_CONNECTOR_W + COLORBAR_BOX_OUTER_W,
                bar_rect.max.y + COLORBAR_BOX_HEIGHT * 0.5,
            ),
        )
        .expand(4.0);
        // The same condition reveals the grips: the overlay stays quiet until
        // the user actually reaches for it.
        let show_histogram = self.colorbar_drag.is_some()
            || pointer.is_some_and(|p| hover_zone.contains(p));
        if show_histogram {
            let bin_count = (bar_rect.height().round() as usize).clamp(16, 512);
            // Borrow ends before painting, which needs &self.
            let bins = self.colorbar_histogram_bins(span, bin_count).to_vec();
            self.paint_colorbar_histogram(ctx, bar_rect, &bins);
        }

        self.paint_limit_connector(ctx, bar_rect, max_y, true, show_histogram);
        self.paint_limit_connector(ctx, bar_rect, min_y, !is_symmetric, show_histogram);

        self.limit_grip(ctx, LimitHandle::Max, bar_rect, span, max_y, show_histogram);
        // No tab for a limit that is not the user's to set. Beyond being
        // pointless, a disabled tab still swallows the pointer, and with vmin
        // mirroring vmax the two sit on top of each other whenever vmax is
        // small -- so the dead tab would cover the live one exactly when the
        // user most needs to drag it.
        if !is_symmetric {
            self.limit_grip(ctx, LimitHandle::Min, bar_rect, span, min_y, show_histogram);
        }

        self.limit_value_box(ctx, LimitHandle::Max, bar_rect, max_y, is_int, is_symmetric);
        self.limit_value_box(ctx, LimitHandle::Min, bar_rect, min_y, is_int, is_symmetric);

        self.render_colorbar_buttons(ctx, bar_rect);
    }

    /// Paint the bar: the gradient between the limits, and a flat band of the
    /// end colour beyond each one.
    fn paint_colorbar_ramp(
        &self,
        ctx: &egui::Context,
        bar: egui::Rect,
        max_y: f32,
        min_y: f32,
    ) {
        let painter = Self::colorbar_painter(ctx);

        // Order the two so an inverted pair of limits still paints forwards.
        let upper = max_y.min(min_y).clamp(bar.min.y, bar.max.y);
        let lower = max_y.max(min_y).clamp(bar.min.y, bar.max.y);

        if upper > bar.min.y {
            painter.rect_filled(
                egui::Rect::from_min_max(bar.min, egui::pos2(bar.max.x, upper)),
                0.0,
                self.ramp_color(1.0),
            );
        }
        if lower < bar.max.y {
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(bar.min.x, lower), bar.max),
                0.0,
                self.ramp_color(0.0),
            );
        }
        if let Some(texture) = &self.colorbar_texture {
            if lower > upper {
                painter.image(
                    texture.id(),
                    egui::Rect::from_min_max(
                        egui::pos2(bar.min.x, upper),
                        egui::pos2(bar.max.x, lower),
                    ),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }

        painter.rect_stroke(
            bar.expand(1.0),
            0.0,
            egui::Stroke::new(1.0, Color32::GRAY),
            egui::StrokeKind::Outside,
        );
    }

    /// Paint one handle's arrow and the straight run out to its value box.
    ///
    /// With the grip hidden the run is a single unbroken line; with it showing,
    /// the line stops either side so the tab is not struck through.
    fn paint_limit_connector(
        &self,
        ctx: &egui::Context,
        bar: egui::Rect,
        y: f32,
        enabled: bool,
        engaged: bool,
    ) {
        let painter = Self::colorbar_painter(ctx);
        let (halo, core) = overlay_line_colors(ctx.style().visuals.dark_mode);
        let core = if enabled {
            core
        } else {
            core.gamma_multiply(0.45)
        };

        // Arrow: tip on the bar's edge, pointing back at the value it marks.
        // Outlined in the halo colour so it survives a bright image under it.
        let tip = egui::pos2(bar.max.x, y);
        let base_x = bar.max.x + COLORBAR_ARROW_W;
        painter.add(egui::Shape::convex_polygon(
            vec![
                tip,
                egui::pos2(base_x, y - COLORBAR_ARROW_HALF_H),
                egui::pos2(base_x, y + COLORBAR_ARROW_HALF_H),
            ],
            core,
            egui::Stroke::new(1.5, halo),
        ));

        let box_edge = bar.max.x + COLORBAR_CONNECTOR_W;
        // Halo under core, same reason as the arrow's outline.
        let mut run = |from: egui::Pos2, to: egui::Pos2| {
            painter.line_segment([from, to], egui::Stroke::new(3.5, halo));
            painter.line_segment([from, to], egui::Stroke::new(1.5, core));
        };
        if engaged {
            // Stop either side of the tab rather than drawing through it.
            run(
                egui::pos2(base_x, y),
                egui::pos2(base_x + COLORBAR_STEM_W, y),
            );
            run(
                egui::pos2(base_x + COLORBAR_STEM_W + COLORBAR_GRIP_W, y),
                egui::pos2(box_edge, y),
            );
        } else {
            run(egui::pos2(base_x, y), egui::pos2(box_edge, y));
        }
    }

    /// The gripped tab a limit is dragged by: parallel lines to say "grab me",
    /// and the drag handling that moves the limit with the pointer.
    fn limit_grip(
        &mut self,
        ctx: &egui::Context,
        handle: LimitHandle,
        bar: egui::Rect,
        span: (f64, f64),
        y: f32,
        engaged: bool,
    ) {
        let grip_min = egui::pos2(
            bar.max.x + COLORBAR_ARROW_W + COLORBAR_STEM_W,
            y - COLORBAR_GRIP_H * 0.5,
        );
        let area_id = egui::Id::new(match handle {
            LimitHandle::Max => "colorbar_max_grip",
            LimitHandle::Min => "colorbar_min_grip",
        });

        // Foreground, not Middle: egui draws every Area in an order before the
        // plain layers in that same order, so a Middle grip would end up
        // *under* the histogram that is painted into the shared Middle layer.
        egui::Area::new(area_id)
            .fixed_pos(grip_min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(COLORBAR_GRIP_W, COLORBAR_GRIP_H),
                    egui::Sense::drag(),
                );

                // The widget palette is meant for chrome sitting on the UI's
                // own background; over the image its fills are near-invisible
                // (a pale grey tab on a bright frame). Use the overlay colours,
                // which carry a backdrop of their own.
                let backdrop = get_overlay_bg(ui);
                let ink = get_overlay_text_color(ui);
                let fill = if response.dragged() || response.hovered() {
                    backdrop.to_opaque()
                } else {
                    backdrop
                };
                let line = ink;

                // The tab is drawn only once the user has reached for the
                // colorbar. It still senses drags the rest of the time, but
                // reaching it means entering the hover zone, which shows it --
                // so there is never an invisible thing to grab.
                if engaged {
                    let painter = ui.painter();
                    painter.rect_filled(rect, 2.0, fill);
                    painter.rect_stroke(
                        rect,
                        2.0,
                        egui::Stroke::new(1.0, line),
                        egui::StrokeKind::Inside,
                    );
                    // Parallel lines across the tab: the conventional "drag me"
                    // texture, horizontal because the tab travels vertically.
                    let inset = 3.0;
                    for i in 0..3 {
                        let ly = rect.center().y + (i as f32 - 1.0) * 4.0;
                        painter.line_segment(
                            [
                                egui::pos2(rect.min.x + inset, ly),
                                egui::pos2(rect.max.x - inset, ly),
                            ],
                            egui::Stroke::new(1.0, line),
                        );
                    }
                }

                if response.drag_started() {
                    self.colorbar_drag = Some(ColorbarDrag { handle, span });
                }
                if response.dragged() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let value = colorbar_y_to_value(bar, span, pos.y);
                        // Keep a sliver between the limits so they cannot cross
                        // and invert the ramp.
                        let epsilon = (span.1 - span.0) * 1e-6;
                        match handle {
                            LimitHandle::Max => {
                                // With vmin derived as -vmax there is no lower
                                // limit to stay clear of, only zero: dragging
                                // past it would mirror the handle back up.
                                let floor = if self.symmetric_mode {
                                    epsilon
                                } else {
                                    self.min_val + epsilon
                                };
                                self.set_max_val(value.max(floor))
                            }
                            LimitHandle::Min => {
                                self.set_min_val(value.min(self.max_val - epsilon))
                            }
                        }
                    }
                }
                if response.drag_stopped() {
                    self.colorbar_drag = None;
                }

                response
                    .on_hover_cursor(egui::CursorIcon::ResizeVertical)
                    .on_hover_text(match handle {
                        LimitHandle::Max => "Drag to set the maximum display value",
                        LimitHandle::Min => "Drag to set the minimum display value",
                    });
            });
    }

    /// One editable limit box, parked beside its handle.
    fn limit_value_box(
        &mut self,
        ctx: &egui::Context,
        handle: LimitHandle,
        bar: egui::Rect,
        y: f32,
        is_int: bool,
        is_symmetric: bool,
    ) {
        let pos = egui::pos2(
            bar.max.x + COLORBAR_CONNECTOR_W,
            y - COLORBAR_BOX_HEIGHT * 0.5,
        );
        let area_id = egui::Id::new(match handle {
            LimitHandle::Max => "colorbar_max_input",
            LimitHandle::Min => "colorbar_min_input",
        });

        // Foreground for the same reason as the grips: the histogram sweeps
        // across this box's x range and must pass beneath it.
        egui::Area::new(area_id)
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                style_limit_edit(ui);

                // In symmetric mode vmin is not the user's to set; keep it
                // showing the value actually in force.
                if handle == LimitHandle::Min && is_symmetric {
                    self.min_limit_input_text =
                        Self::format_limit_value(-self.max_val.abs(), is_int);
                }

                let buffer = match handle {
                    LimitHandle::Max => &mut self.max_limit_input_text,
                    LimitHandle::Min => &mut self.min_limit_input_text,
                };
                // Left-aligned so the digits start the same distance from the
                // arrow whatever the value's length; centring made a short
                // value drift away from the mark it belongs to.
                let edit = egui::TextEdit::singleline(buffer)
                    .desired_width(COLORBAR_BOX_WIDTH)
                    .horizontal_align(egui::Align::Min)
                    .text_color(text_color)
                    .font(egui::FontId::proportional(13.0));

                let editable = handle == LimitHandle::Max || !is_symmetric;
                let response = ui.add_enabled(editable, edit);
                let committed = response.lost_focus()
                    || (response.has_focus() && ui.input(|i| i.key_pressed(Key::Enter)));

                if editable && committed {
                    match handle {
                        LimitHandle::Max => {
                            if let Ok(parsed) = self.max_limit_input_text.trim().parse::<f64>() {
                                let parsed = if is_symmetric { parsed.abs() } else { parsed };
                                self.set_max_val(parsed);
                                self.max_limit_input_text =
                                    Self::format_limit_value(parsed, is_int);
                            } else {
                                let shown = if is_symmetric {
                                    self.max_val.abs()
                                } else {
                                    self.max_val
                                };
                                self.max_limit_input_text =
                                    Self::format_limit_value(shown, is_int);
                            }
                        }
                        LimitHandle::Min => {
                            if let Ok(parsed) = self.min_limit_input_text.trim().parse::<f64>() {
                                self.set_min_val(parsed);
                            } else {
                                self.min_limit_input_text =
                                    Self::format_limit_value(self.min_val, is_int);
                            }
                        }
                    }
                }

                response.on_hover_text(match handle {
                    LimitHandle::Max => "Maximum display value",
                    LimitHandle::Min if is_symmetric => {
                        "Symmetric mode: vmin is derived from -vmax"
                    }
                    LimitHandle::Min => "Minimum display value",
                });
            });
    }

    /// Lock and reset buttons, stacked under the bar.
    fn render_colorbar_buttons(&mut self, ctx: &egui::Context, bar_rect: egui::Rect) {
        let spacing = 4.0_f32;
        let button_size = COLORBAR_BAR_WIDTH;

        let lock_button_pos = egui::pos2(bar_rect.min.x, bar_rect.max.y + spacing);
        egui::Area::new(egui::Id::new("colorbar_lock_button"))
            .fixed_pos(lock_button_pos)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let bg_color = get_overlay_bg(ui);
                let is_locked = self.is_colorbar_locked();

                ui.style_mut().spacing.button_padding = egui::vec2(2.0, 2.0);
                let btn_icon = egui::RichText::new(if is_locked {
                    phosphor::LOCK
                } else {
                    phosphor::LOCK_OPEN
                })
                .color(if is_locked {
                    text_color
                } else {
                    text_color.gamma_multiply(0.4)
                })
                .size(12.0);
                let btn = egui::Button::new(btn_icon)
                    .fill(bg_color)
                    .min_size(egui::vec2(button_size, button_size));
                let response = ui.add(btn);
                if response.clicked() {
                    self.toggle_colorbar_lock();
                }
                response.on_hover_text(if is_locked {
                    "Limits stay put when a new array is loaded"
                } else {
                    "Limits are recomputed when a new array is loaded"
                });
            });

        let reset_button_pos =
            egui::pos2(bar_rect.min.x, bar_rect.max.y + spacing + button_size + spacing);
        egui::Area::new(egui::Id::new("colorbar_reset_button"))
            .fixed_pos(reset_button_pos)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let text_color = get_overlay_text_color(ui);
                let bg_color = get_overlay_bg(ui);
                let is_modified = self.is_display_modified() || self.is_colorbar_locked();

                ui.style_mut().spacing.button_padding = egui::vec2(2.0, 2.0);
                let btn_icon = egui::RichText::new(phosphor::ARROW_COUNTER_CLOCKWISE)
                    .color(if is_modified {
                        text_color
                    } else {
                        text_color.gamma_multiply(0.4)
                    })
                    .size(12.0);
                let btn = egui::Button::new(btn_icon)
                    .fill(bg_color)
                    .min_size(egui::vec2(button_size, button_size));
                let response = ui.add_enabled(is_modified, btn);
                if response.clicked() {
                    // Puts both limits back on the ends of the bar, which is
                    // the only way out of a stretch dragged somewhere useless.
                    self.reset_display();
                }
                response.on_hover_text("Reset contrast/bias and restore min/max limits");
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
// =============================================================================
// Colorbar geometry
//
// Free functions so the mapping between data values and bar positions can be
// tested without standing up a widget or a window.
// =============================================================================

/// Map a data value to its y on the bar, clamped to the bar's extent.
/// `span` is (value at the bottom edge, value at the top edge).
fn colorbar_value_to_y(bar: egui::Rect, span: (f64, f64), value: f64) -> f32 {
    let (lo, hi) = span;
    let t = if hi > lo {
        ((value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    bar.max.y - t * bar.height()
}

/// Inverse of [`colorbar_value_to_y`]: where on the bar the pointer is, as a
/// data value. Positions outside the bar clamp to its ends.
fn colorbar_y_to_value(bar: egui::Rect, span: (f64, f64), y: f32) -> f64 {
    let (lo, hi) = span;
    let t = if bar.height() > 0.0 {
        ((bar.max.y - y) / bar.height()).clamp(0.0, 1.0) as f64
    } else {
        0.0
    };
    lo + t * (hi - lo)
}

/// Halo and core colours for the colorbar's arrows and leader lines.
///
/// This chrome floats on the image, not on the UI's own background, so a single
/// theme colour disappears the moment the pixels beneath it match — pale arrows
/// over a bright field. Every stroke is laid down twice, a contrasting halo
/// first and the core on top, which reads against anything either way round.
fn overlay_line_colors(dark_mode: bool) -> (Color32, Color32) {
    if dark_mode {
        (Color32::from_black_alpha(200), Color32::WHITE)
    } else {
        (Color32::from_white_alpha(230), Color32::from_gray(30))
    }
}

/// Give a limit value box the translucent, theme-aware chrome the rest of the
/// overlay uses.
fn style_limit_edit(ui: &mut Ui) {
    let is_dark = ui.visuals().dark_mode;
    let (bg, hover, active) = if is_dark {
        (
            egui::Color32::from_black_alpha(180),
            egui::Color32::from_black_alpha(200),
            egui::Color32::from_black_alpha(220),
        )
    } else {
        (
            egui::Color32::from_white_alpha(220),
            egui::Color32::from_white_alpha(240),
            egui::Color32::from_white_alpha(255),
        )
    };
    ui.style_mut().visuals.extreme_bg_color = bg;
    ui.style_mut().visuals.widgets.inactive.bg_fill = bg;
    ui.style_mut().visuals.widgets.hovered.bg_fill = hover;
    ui.style_mut().visuals.widgets.active.bg_fill = active;
}

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
    fn failed_request_frees_the_scrubber() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![4]);
        deliver(&mut w, vec![0]);

        w.apply_slice_action(SliceAction::SetIndex(0, 3), 1.0);
        assert_eq!(w.take_slice_request(), Some(vec![3]));

        // The host cannot serve it. Without clearing the in-flight marker the
        // next scrub below would be coalesced away and never requested.
        w.fail_slice_request(&[3]);
        assert_eq!(w.requested_indices, None);
        assert!(w.has_image(), "the last good slice stays on screen");

        w.apply_slice_action(SliceAction::SetIndex(0, 1), 2.0);
        assert_eq!(w.take_slice_request(), Some(vec![1]));
    }

    #[test]
    fn failing_a_stale_request_is_a_no_op() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![4]);
        deliver(&mut w, vec![0]);
        w.apply_slice_action(SliceAction::Play(0), 0.0);
        w.update_playback(0.0);
        assert_eq!(w.requested_indices, Some(vec![1]));

        // A late failure report for a slice that is no longer in flight must
        // not tear down playback that has already moved on.
        w.fail_slice_request(&[0]);
        assert_eq!(w.playing_axis, Some(0));
        assert_eq!(w.requested_indices, Some(vec![1]));
    }

    #[test]
    fn failed_request_stops_playback() {
        let mut w = ArrayViewerWidget::new();
        w.set_cube(vec![4]);
        deliver(&mut w, vec![0]);
        w.apply_slice_action(SliceAction::Play(0), 0.0);
        w.update_playback(0.0);
        assert_eq!(w.take_slice_request(), Some(vec![1]));

        w.fail_slice_request(&[1]);
        assert_eq!(w.playing_axis, None, "do not keep looping on unreadable data");
        assert_eq!(w.requested_indices, None);
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

#[cfg(test)]
mod colorbar_tests {
    use super::*;

    fn bar() -> egui::Rect {
        // 100pt tall, top at y=50 so "bar-relative" bugs cannot hide behind a
        // zero origin.
        egui::Rect::from_min_size(egui::pos2(10.0, 50.0), egui::vec2(16.0, 100.0))
    }

    fn widget_with(pixels: Vec<f64>, width: u32, height: u32) -> ArrayViewerWidget {
        let mut w = ArrayViewerWidget::new();
        w.set_image(pixels, width, height, false, 6);
        w
    }

    /// Run one frame of the colorbar with the pointer at `pointer`, returning
    /// every path egui emitted. Lets the overlay be exercised without a window.
    fn run_colorbar_frame(
        w: &mut ArrayViewerWidget,
        ctx: &egui::Context,
        pointer: Option<egui::Pos2>,
    ) -> Vec<egui::epaint::PathShape> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut events = Vec::new();
        if let Some(pos) = pointer {
            events.push(egui::Event::PointerMoved(pos));
        }
        return run_colorbar_events(w, ctx, events);
    }

    /// The colorbar's rect for the 800x600 screen the harness uses.
    fn harness_bar() -> egui::Rect {
        egui::Rect::from_min_size(
            egui::pos2(COLORBAR_MARGIN, COLORBAR_VERTICAL_MARGIN),
            egui::vec2(COLORBAR_BAR_WIDTH, COLORBAR_MAX_HEIGHT.min(600.0 * 0.5)),
        )
    }

    /// Centre of a limit handle's grip at bar position `y`.
    fn harness_grip(y: f32) -> egui::Pos2 {
        let bar = harness_bar();
        egui::pos2(
            bar.max.x + COLORBAR_ARROW_W + COLORBAR_STEM_W + COLORBAR_GRIP_W * 0.5,
            y,
        )
    }

    /// Run one frame of the colorbar with arbitrary input events.
    fn run_colorbar_events(
        w: &mut ArrayViewerWidget,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::PathShape> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            w.render_colorbar(ctx, screen);
        });

        collect_paths(&output.shapes)
    }

    /// Every shape egui emitted this frame, flattened but still in paint order.
    fn run_colorbar_shapes(
        w: &mut ArrayViewerWidget,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<egui::Shape> {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| {
            w.render_colorbar(ctx, screen);
        });

        fn flatten(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
            match shape {
                egui::Shape::Vec(v) => {
                    for s in v {
                        flatten(s, out);
                    }
                }
                other => out.push(other.clone()),
            }
        }
        let mut shapes = Vec::new();
        for clipped in &output.shapes {
            flatten(&clipped.shape, &mut shapes);
        }
        shapes
    }

    fn collect_paths(clipped: &[egui::epaint::ClippedShape]) -> Vec<egui::epaint::PathShape> {
        fn collect(shape: &egui::Shape, out: &mut Vec<egui::epaint::PathShape>) {
            match shape {
                egui::Shape::Path(p) => out.push(p.clone()),
                egui::Shape::Vec(v) => {
                    for s in v {
                        collect(s, out);
                    }
                }
                _ => {}
            }
        }
        let mut paths = Vec::new();
        for c in clipped {
            collect(&c.shape, &mut paths);
        }
        paths
    }

    /// The histogram outline: the white hairline, which no other part of the
    /// overlay draws as a long open path.
    fn histogram_paths(paths: &[egui::epaint::PathShape]) -> Vec<&egui::epaint::PathShape> {
        paths
            .iter()
            .filter(|p| {
                p.stroke.width == 1.0
                    && p.stroke.color == egui::epaint::ColorMode::Solid(Color32::WHITE)
                    && p.points.len() > 8
            })
            .collect()
    }

    #[test]
    fn dragging_the_max_handle_lowers_the_limit_and_follows_with_the_box() {
        let mut w = widget_with((0..400).map(|i| i as f64).collect(), 20, 20);
        let ctx = egui::Context::default();
        let bar = harness_bar();

        // Limits start on the data, so the max handle sits at the bar's top.
        run_colorbar_frame(&mut w, &ctx, None);
        let (vmin_before, vmax_before) = w.value_range();
        let grip = harness_grip(bar.min.y);

        // Press on the grip, then drag down to the middle of the bar.
        run_colorbar_frame(&mut w, &ctx, Some(grip));
        run_colorbar_events(
            &mut w,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: grip,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        let target = egui::pos2(grip.x, bar.center().y);
        run_colorbar_events(&mut w, &ctx, vec![egui::Event::PointerMoved(target)]);
        run_colorbar_events(&mut w, &ctx, vec![egui::Event::PointerMoved(target)]);

        let (vmin_after, vmax_after) = w.value_range();
        assert!(
            vmax_after < vmax_before,
            "dragging the max handle down should lower vmax: {vmax_before} -> {vmax_after}"
        );
        assert_eq!(vmin_before, vmin_after, "the other limit must not move");

        // Halfway down the bar is halfway through the span.
        let (lo, hi) = w.colorbar_span();
        let expected = lo + 0.5 * (hi - lo);
        assert!(
            (vmax_after - expected).abs() < 0.05 * (hi - lo),
            "vmax {vmax_after} should be near the bar's midpoint {expected}"
        );

        // The box shows the dragged value, not a stale one.
        assert_eq!(
            w.max_limit_input_text,
            ArrayViewerWidget::format_limit_value(vmax_after, false)
        );

        // And releasing ends the drag, so the span is free to move again.
        run_colorbar_events(
            &mut w,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        assert!(w.colorbar_drag.is_none(), "drag state should be cleared");
    }

    #[test]
    fn the_min_handle_is_inert_in_symmetric_mode() {
        let mut w = widget_with(vec![-8.0, -2.0, 4.0, 10.0], 2, 2);
        w.set_symmetric(true);
        let ctx = egui::Context::default();
        let bar = harness_bar();

        run_colorbar_frame(&mut w, &ctx, None);
        let before = w.value_range();

        // vmin is derived from vmax here, so its handle must not accept a drag.
        let grip = harness_grip(bar.max.y);
        run_colorbar_frame(&mut w, &ctx, Some(grip));
        run_colorbar_events(
            &mut w,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: grip,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        let target = egui::pos2(grip.x, bar.center().y);
        run_colorbar_events(&mut w, &ctx, vec![egui::Event::PointerMoved(target)]);
        run_colorbar_events(&mut w, &ctx, vec![egui::Event::PointerMoved(target)]);

        assert_eq!(w.value_range(), before, "symmetric vmin must stay derived");
        assert!(w.colorbar_drag.is_none(), "no drag should have started");
    }

    /// Locate the drag tabs in a frame's shapes: the only rounded rects out at
    /// the connector's x, the width of a grip.
    fn grip_count(shapes: &[egui::Shape]) -> usize {
        let bar = harness_bar();
        let grip_x = bar.max.x + COLORBAR_ARROW_W + COLORBAR_STEM_W;
        // A tab is drawn as a fill plus a stroke, so count distinct positions
        // rather than shapes.
        let mut centres: Vec<f32> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r)
                    if (r.rect.min.x - grip_x).abs() < 1.0
                        && (r.rect.width() - COLORBAR_GRIP_W).abs() < 1.0 =>
                {
                    Some(r.rect.center().y)
                }
                _ => None,
            })
            .collect();
        centres.sort_by(f32::total_cmp);
        centres.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        centres.len()
    }

    #[test]
    fn grips_appear_only_once_the_colorbar_is_reached_for() {
        let mut w = widget_with((0..400).map(|i| i as f64).collect(), 20, 20);
        let ctx = egui::Context::default();
        run_colorbar_frame(&mut w, &ctx, None);

        // Pointer elsewhere: the overlay stays quiet, just the bar, the arrows
        // and the value boxes.
        let away = run_colorbar_shapes(
            &mut w,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(700.0, 500.0))],
        );
        assert_eq!(grip_count(&away), 0, "tabs drawn without being reached for");

        // Pointer on the bar: both tabs show, alongside the histogram.
        let over = run_colorbar_shapes(
            &mut w,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(
                COLORBAR_MARGIN + 2.0,
                COLORBAR_VERTICAL_MARGIN + 20.0,
            ))],
        );
        assert_eq!(grip_count(&over), 2, "both tabs should show on hover");
        assert_eq!(
            histogram_paths(&collect_paths_from(&over)).len(),
            1,
            "the tabs and the histogram share one condition, so both or neither"
        );
    }

    /// Re-wrap flattened shapes so the histogram filter can be reused.
    fn collect_paths_from(shapes: &[egui::Shape]) -> Vec<egui::epaint::PathShape> {
        shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Path(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn symmetric_mode_centres_the_bar_on_zero() {
        // Data reaching much further below zero than above: the case that used
        // to put zero off centre and desynchronise the two handles.
        let mut w = widget_with(vec![-100.0, -40.0, 0.5, 1.0], 2, 2);
        w.set_symmetric(true);
        w.set_max_val(1.0);

        let (lo, hi) = w.colorbar_span();
        assert!((lo + hi).abs() < 1e-9, "bar must straddle zero: {lo}..{hi}");
        assert!(
            (hi - 100.0).abs() < 1e-9,
            "bar should reach the larger magnitude in the data, got {hi}"
        );

        // Zero lands exactly on the bar's midpoint, so the diverging colormap's
        // neutral colour sits where the data's zero is.
        let bar = harness_bar();
        assert!(
            (colorbar_value_to_y(bar, (lo, hi), 0.0) - bar.center().y).abs() < 1e-3,
            "zero is not at the centre of the bar"
        );
    }

    #[test]
    fn symmetric_handles_stay_mirrored_and_reach_the_ends_together() {
        let mut w = widget_with(vec![-100.0, -40.0, 0.5, 1.0], 2, 2);
        w.set_symmetric(true);
        w.set_max_val(1.0);

        let bar = harness_bar();
        let span = w.colorbar_span();

        // Whatever vmax is, the handles are equidistant from the centre.
        for vmax in [1.0, 25.0, 60.0, 100.0] {
            w.set_max_val(vmax);
            let (vmin, vmax) = w.scaling_range();
            let max_y = colorbar_value_to_y(bar, span, vmax);
            let min_y = colorbar_value_to_y(bar, span, vmin);
            let centre = bar.center().y;
            assert!(
                ((centre - max_y) - (min_y - centre)).abs() < 1e-3,
                "handles not mirrored at vmax={vmax}: {max_y} / {min_y}"
            );
        }

        // And they hit the ends at the same moment, rather than vmin bottoming
        // out early while vmax still has room.
        w.set_max_val(100.0);
        let (vmin, vmax) = w.scaling_range();
        assert!((colorbar_value_to_y(bar, span, vmax) - bar.min.y).abs() < 1e-3);
        assert!((colorbar_value_to_y(bar, span, vmin) - bar.max.y).abs() < 1e-3);
    }

    #[test]
    fn dragging_symmetric_max_to_the_top_takes_vmin_to_the_bottom() {
        // The reported case: max 1.0, min -100. Dragging the max handle to the
        // top of the bar should reach +100, putting the derived vmin at -100.
        let mut w = widget_with(vec![-100.0, -40.0, 0.5, 1.0], 2, 2);
        w.set_symmetric(true);
        w.set_max_val(1.0);

        let ctx = egui::Context::default();
        let bar = harness_bar();
        run_colorbar_frame(&mut w, &ctx, None);

        let span = w.colorbar_span();
        let start = colorbar_value_to_y(bar, span, 1.0);
        let grip = harness_grip(start);

        run_colorbar_frame(&mut w, &ctx, Some(grip));
        run_colorbar_events(
            &mut w,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: grip,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        // Drag above the bar's top; the handle pins to the end.
        let target = egui::pos2(grip.x, bar.min.y - 20.0);
        run_colorbar_events(&mut w, &ctx, vec![egui::Event::PointerMoved(target)]);
        run_colorbar_events(&mut w, &ctx, vec![egui::Event::PointerMoved(target)]);

        let (vmin, vmax) = w.scaling_range();
        assert!(
            (vmax - 100.0).abs() < 1.0,
            "vmax should have reached the data's magnitude, got {vmax}"
        );
        assert!(
            (vmin + 100.0).abs() < 1.0,
            "vmin should mirror it, got {vmin}"
        );
    }

    #[test]
    fn symmetric_max_cannot_be_dragged_through_zero() {
        let mut w = widget_with(vec![-10.0, -2.0, 3.0, 8.0], 2, 2);
        w.set_symmetric(true);
        w.set_max_val(8.0);

        let ctx = egui::Context::default();
        let bar = harness_bar();
        run_colorbar_frame(&mut w, &ctx, None);

        let span = w.colorbar_span();
        let grip = harness_grip(colorbar_value_to_y(bar, span, 8.0));
        run_colorbar_frame(&mut w, &ctx, Some(grip));
        run_colorbar_events(
            &mut w,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: grip,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        // Drag well below the centre, into negative values.
        let target = egui::pos2(grip.x, bar.max.y - 5.0);
        run_colorbar_events(&mut w, &ctx, vec![egui::Event::PointerMoved(target)]);
        run_colorbar_events(&mut w, &ctx, vec![egui::Event::PointerMoved(target)]);

        let (_, vmax) = w.scaling_range();
        assert!(
            vmax >= 0.0,
            "vmax must not cross zero and mirror back up, got {vmax}"
        );
    }

    #[test]
    fn grips_appear_when_hovering_a_value_box_too() {
        let mut w = widget_with((0..400).map(|i| i as f64).collect(), 20, 20);
        let ctx = egui::Context::default();
        let bar = harness_bar();
        run_colorbar_frame(&mut w, &ctx, None);

        // Over the middle of the max limit's box, well clear of the bar.
        let box_x = bar.max.x + COLORBAR_CONNECTOR_W + COLORBAR_BOX_WIDTH * 0.5;
        let shapes = run_colorbar_shapes(
            &mut w,
            &ctx,
            vec![egui::Event::PointerMoved(egui::pos2(box_x, bar.min.y))],
        );
        assert_eq!(
            grip_count(&shapes),
            2,
            "reaching for a value box should reveal the tabs"
        );
    }

    #[test]
    fn a_box_at_either_end_of_the_bar_still_fits_in_the_widget() {
        // The bar is inset by half a box so that a limit parked at the very top
        // can keep its box centred on its arrow without hanging off the widget.
        // If the box ever grows past the margin this has to be revisited.
        assert!(
            COLORBAR_VERTICAL_MARGIN - COLORBAR_BOX_HEIGHT * 0.5 >= 0.0,
            "a box at the top of the bar would be clipped by the widget edge"
        );
        assert!(
            COLORBAR_VERTICAL_MARGIN >= COLORBAR_BOX_HEIGHT * 0.5,
            "not enough room above the bar for half a value box"
        );
    }

    #[test]
    fn each_value_box_is_centred_on_its_own_arrow() {
        let mut w = widget_with((0..400).map(|i| i as f64).collect(), 20, 20);
        let ctx = egui::Context::default();
        let bar = harness_bar();

        run_colorbar_frame(&mut w, &ctx, None);
        // Put the limits somewhere asymmetric so a box that silently tracked
        // the wrong handle, or got nudged, would show up.
        w.set_value_range(100.0, 300.0);
        run_colorbar_frame(&mut w, &ctx, None);
        let shapes = run_colorbar_shapes(&mut w, &ctx, Vec::new());

        let span = w.colorbar_span();
        let box_x = bar.max.x + COLORBAR_CONNECTOR_W;
        let mut centres: Vec<f32> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r)
                    if (r.rect.min.x - box_x).abs() < 2.0 && r.rect.width() > 40.0 =>
                {
                    Some(r.rect.center().y)
                }
                _ => None,
            })
            .collect();
        centres.sort_by(f32::total_cmp);
        centres.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        assert_eq!(centres.len(), 2, "expected one box per limit: {centres:?}");

        for (value, centre) in [(300.0, centres[0]), (100.0, centres[1])] {
            let arrow_y = colorbar_value_to_y(bar, span, value);
            assert!(
                (centre - arrow_y).abs() < 1.5,
                "box for {value} centred at {centre}, arrow at {arrow_y}"
            );
        }
    }

    #[test]
    fn histogram_bins_are_log_scaled_so_sparse_bins_stay_visible() {
        // One lonely pixel against a thousand in the background bin -- the
        // shape of every image this viewer is actually pointed at.
        let mut pixels = vec![0.0; 1000];
        pixels.push(9.0);
        let count = pixels.len() as u32;
        let mut w = widget_with(pixels, count, 1);

        let span = w.colorbar_span();
        let bins = w.colorbar_histogram_bins(span, 10).to_vec();

        let crowded = bins[0];
        let lonely = bins[9];
        assert!((crowded - 1.0).abs() < 1e-6, "the mode normalises to 1.0");

        // Linear scaling would put the lone pixel at 1/1000 of the width: a
        // tenth of a pixel, i.e. invisible. The log has to do far better.
        let linear_would_be = 1.0 / 1000.0;
        assert!(
            lonely > 20.0 * linear_would_be,
            "sparse bin at {lonely} is barely better than linear ({linear_would_be})"
        );
        // ln(2)/ln(1001), give or take.
        assert!(
            (lonely - 0.1003).abs() < 0.01,
            "expected the log1p ratio, got {lonely}"
        );

        // Bins with nothing in them still sit flat on the bar.
        assert_eq!(bins[5], 0.0, "an empty bin must have no width");
    }

    #[test]
    fn histogram_is_drawn_under_the_handles_and_boxes() {
        let mut w = widget_with((0..400).map(|i| i as f64).collect(), 20, 20);
        let ctx = egui::Context::default();
        run_colorbar_frame(&mut w, &ctx, None);

        // Hover the bar so the histogram is up, then look at paint order.
        let shapes = run_colorbar_shapes(&mut w, &ctx, vec![egui::Event::PointerMoved(
            egui::pos2(COLORBAR_MARGIN + 2.0, COLORBAR_MARGIN + 20.0),
        )]);

        let histogram_at = shapes
            .iter()
            .position(|s| {
                matches!(s, egui::Shape::Path(p)
                    if p.stroke.width == 1.0
                        && p.stroke.color == egui::epaint::ColorMode::Solid(Color32::WHITE)
                        && p.points.len() > 8)
            })
            .expect("histogram should be drawn while hovering");

        // The grips are the only rounded rects out at the connector's x.
        let bar = harness_bar();
        let grip_x = bar.max.x + COLORBAR_ARROW_W + COLORBAR_STEM_W;
        let grip_at = shapes
            .iter()
            .position(|s| {
                matches!(s, egui::Shape::Rect(r)
                    if (r.rect.min.x - grip_x).abs() < 1.0
                        && (r.rect.width() - COLORBAR_GRIP_W).abs() < 1.0)
            })
            .expect("hovering should have drawn a grip");

        assert!(
            histogram_at < grip_at,
            "histogram (at {histogram_at}) must be painted before the grips (at {grip_at}) \
             so it passes under them"
        );
    }

    #[test]
    fn histogram_shows_only_while_the_colorbar_is_engaged() {
        let mut w = widget_with((0..400).map(|i| i as f64).collect(), 20, 20);
        let ctx = egui::Context::default();

        // Warm-up frame: egui needs one pass to lay the overlay out.
        run_colorbar_frame(&mut w, &ctx, None);

        // Pointer parked far from the colorbar: no histogram.
        let away = run_colorbar_frame(&mut w, &ctx, Some(egui::pos2(700.0, 500.0)));
        assert!(
            histogram_paths(&away).is_empty(),
            "histogram drawn while the pointer was nowhere near the colorbar"
        );

        // Pointer over the bar itself: histogram appears.
        let over = run_colorbar_frame(&mut w, &ctx, Some(egui::pos2(16.0, 60.0)));
        let drawn = histogram_paths(&over);
        assert_eq!(
            drawn.len(),
            1,
            "expected exactly one histogram hairline, got {}",
            drawn.len()
        );

        // It starts and ends on the bar's edge and never reaches past its
        // maximum width.
        let path = drawn[0];
        let bar_right = 10.0 + COLORBAR_BAR_WIDTH;
        for p in &path.points {
            assert!(
                p.x >= bar_right - 0.01 && p.x <= bar_right + COLORBAR_HISTOGRAM_W + 0.01,
                "histogram point {p:?} outside its allotted width"
            );
        }
        assert!(
            path.points.iter().any(|p| p.x > bar_right + 1.0),
            "histogram never left the bar edge, so nothing was actually drawn"
        );
    }

    #[test]
    fn histogram_stays_up_while_a_handle_is_dragged_away_from_the_pointer() {
        let mut w = widget_with((0..400).map(|i| i as f64).collect(), 20, 20);
        let ctx = egui::Context::default();
        run_colorbar_frame(&mut w, &ctx, None);

        // Pretend a drag is in flight, pointer well away from the bar.
        w.colorbar_drag = Some(ColorbarDrag {
            handle: LimitHandle::Max,
            span: w.colorbar_span(),
        });
        let during = run_colorbar_frame(&mut w, &ctx, Some(egui::pos2(700.0, 500.0)));
        assert_eq!(
            histogram_paths(&during).len(),
            1,
            "histogram should stay visible for the whole drag"
        );
    }

    #[test]
    fn the_overlay_renders_without_panicking_in_awkward_states() {
        let ctx = egui::Context::default();

        // Flat image, inverted limits, symmetric mode, and a limit pushed well
        // past the data -- each has its own branch in the paint code.
        let mut flat = widget_with(vec![3.0; 16], 4, 4);
        run_colorbar_frame(&mut flat, &ctx, Some(egui::pos2(16.0, 60.0)));

        let mut inverted = widget_with(vec![0.0, 1.0, 2.0, 3.0], 2, 2);
        inverted.set_max_val(0.5);
        inverted.set_min_val(2.5);
        run_colorbar_frame(&mut inverted, &ctx, Some(egui::pos2(16.0, 60.0)));

        let mut symmetric = widget_with(vec![-5.0, 0.0, 5.0, 10.0], 2, 2);
        symmetric.set_symmetric(true);
        run_colorbar_frame(&mut symmetric, &ctx, Some(egui::pos2(16.0, 60.0)));

        let mut wide = widget_with(vec![0.0, 1.0, 2.0, 3.0], 2, 2);
        wide.set_value_range(-1e6, 1e6);
        run_colorbar_frame(&mut wide, &ctx, Some(egui::pos2(16.0, 60.0)));

        let mut empty = ArrayViewerWidget::new();
        run_colorbar_frame(&mut empty, &ctx, Some(egui::pos2(16.0, 60.0)));
    }

    #[test]
    fn value_and_position_round_trip() {
        let bar = bar();
        let span = (-10.0, 30.0);

        // The span's ends land on the bar's ends, low at the bottom.
        assert!((colorbar_value_to_y(bar, span, -10.0) - bar.max.y).abs() < 1e-3);
        assert!((colorbar_value_to_y(bar, span, 30.0) - bar.min.y).abs() < 1e-3);
        assert!((colorbar_value_to_y(bar, span, 10.0) - bar.center().y).abs() < 1e-3);

        for value in [-10.0, -3.5, 0.0, 12.25, 30.0] {
            let y = colorbar_value_to_y(bar, span, value);
            let back = colorbar_y_to_value(bar, span, y);
            assert!((back - value).abs() < 1e-6, "{value} -> {y} -> {back}");
        }
    }

    #[test]
    fn positions_clamp_to_the_bar() {
        let bar = bar();
        let span = (0.0, 1.0);
        assert!((colorbar_value_to_y(bar, span, 99.0) - bar.min.y).abs() < 1e-3);
        assert!((colorbar_value_to_y(bar, span, -99.0) - bar.max.y).abs() < 1e-3);
        // Dragging past either end of the bar pins to that end's value.
        assert!((colorbar_y_to_value(bar, span, bar.min.y - 40.0) - 1.0).abs() < 1e-9);
        assert!((colorbar_y_to_value(bar, span, bar.max.y + 40.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn degenerate_span_does_not_divide_by_zero() {
        let bar = bar();
        let y = colorbar_value_to_y(bar, (5.0, 5.0), 5.0);
        assert!(y.is_finite());
        assert!(colorbar_y_to_value(bar, (5.0, 5.0), y).is_finite());
    }

    #[test]
    fn the_bar_spans_the_data_so_pulled_in_limits_leave_a_band() {
        let mut w = widget_with(vec![0.0, 10.0, 20.0, 30.0], 2, 2);
        assert_eq!(w.colorbar_span(), (0.0, 30.0));

        // Pulling a limit inside the data leaves the span alone, which is what
        // creates the saturated band.
        w.set_value_range(10.0, 20.0);
        assert_eq!(w.colorbar_span(), (0.0, 30.0));

        // Pushing one past the data stretches the bar instead: nothing
        // saturates out there, so there is no band to show.
        w.set_value_range(-5.0, 100.0);
        assert_eq!(w.colorbar_span(), (-5.0, 100.0));
    }

    #[test]
    fn a_flat_image_still_gives_a_usable_span() {
        let w = widget_with(vec![7.0; 4], 2, 2);
        let (lo, hi) = w.colorbar_span();
        assert!(hi > lo, "span must have width: {lo}..{hi}");
    }

    #[test]
    fn histogram_is_normalised_to_its_tallest_bin() {
        // Ten 5s and one each of 0 and 10, so the middle bin dominates.
        let mut pixels = vec![5.0; 10];
        pixels.push(0.0);
        pixels.push(10.0);
        let mut w = widget_with(pixels, 4, 3);

        let span = w.colorbar_span();
        let bins = w.colorbar_histogram_bins(span, 11).to_vec();

        assert_eq!(bins.len(), 11);
        let peak = bins.iter().copied().fold(0.0_f32, f32::max);
        assert!(
            (peak - 1.0).abs() < 1e-6,
            "tallest bin should be exactly 1.0, got {peak}"
        );
        // Bin 0 is the low end, and the counts land where the values are.
        assert!(bins[0] > 0.0, "the single 0.0 should be in the first bin");
        assert!(bins[10] > 0.0, "the single 10.0 should be in the last bin");
        assert!((bins[5] - 1.0).abs() < 1e-6, "the ten 5.0s are the mode");
    }

    #[test]
    fn histogram_ignores_nan() {
        let mut w = widget_with(vec![0.0, 1.0, f64::NAN, f64::NAN], 2, 2);
        let span = w.colorbar_span();
        let bins = w.colorbar_histogram_bins(span, 4).to_vec();
        let total: f32 = bins.iter().sum();
        // Two finite pixels, each alone in its bin, so both normalise to 1.0.
        assert!((total - 2.0).abs() < 1e-6, "NaN leaked into the bins: {bins:?}");
    }

    #[test]
    fn histogram_is_recomputed_when_the_span_or_resolution_changes() {
        let mut w = widget_with(vec![0.0, 1.0, 2.0, 3.0], 2, 2);
        let span = w.colorbar_span();

        assert_eq!(w.colorbar_histogram_bins(span, 8).len(), 8);
        // A different bin count must not hand back the old vector.
        assert_eq!(w.colorbar_histogram_bins(span, 16).len(), 16);

        // A span change invalidates too, even at the same resolution.
        w.set_value_range(-10.0, 10.0);
        let wider = w.colorbar_span();
        assert_ne!(wider, span);
        w.colorbar_histogram_bins(wider, 16);
        assert_eq!(
            w.colorbar_histogram.as_ref().map(|h| h.span),
            Some(wider),
            "cache should now describe the new span"
        );
    }

    #[test]
    fn a_new_image_drops_the_cached_histogram() {
        let mut w = widget_with(vec![0.0, 1.0, 2.0, 3.0], 2, 2);
        let span = w.colorbar_span();
        w.colorbar_histogram_bins(span, 8);
        assert!(w.colorbar_histogram.is_some());

        w.set_image(vec![10.0, 20.0, 30.0, 40.0], 2, 2, false, 6);
        assert!(
            w.colorbar_histogram.is_none(),
            "stale histogram would describe the previous image"
        );
    }

    #[test]
    fn reversing_the_colormap_swaps_the_ramp_ends() {
        let mut w = widget_with(vec![0.0, 1.0, 2.0, 3.0], 2, 2);
        let (bottom, top) = (w.ramp_color(0.0), w.ramp_color(1.0));
        w.set_reversed(true);
        // The end colours are what the saturated bands are painted with, so
        // they have to follow the ramp when it flips.
        assert_eq!(w.ramp_color(0.0), top);
        assert_eq!(w.ramp_color(1.0), bottom);
    }

    #[test]
    fn reset_restores_limits_dragged_away() {
        let mut w = widget_with(vec![0.0, 10.0, 20.0, 30.0], 2, 2);
        let (orig_min, orig_max) = w.value_range();

        // Stand in for a drag: the handles do nothing the setters do not.
        w.set_max_val(12.0);
        w.set_min_val(4.0);
        assert!(w.is_limits_modified());

        w.reset_display();
        assert_eq!(w.value_range(), (orig_min, orig_max));
        assert!(!w.is_limits_modified());
        // The boxes follow the limits, not the other way round.
        assert_eq!(w.max_limit_input_text, ArrayViewerWidget::format_limit_value(orig_max, false));
        assert_eq!(w.min_limit_input_text, ArrayViewerWidget::format_limit_value(orig_min, false));
    }
}
