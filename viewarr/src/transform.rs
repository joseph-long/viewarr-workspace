//! Coordinate transformation logic for pan/zoom/rotation functionality
//!
//! This module contains pure coordinate transformation logic that can be
//! easily unit tested without egui dependencies.

use egui::{Pos2, Rect, Vec2};

/// Zoom step multiplier for zoom in/out operations (buttons/keyboard)
pub const ZOOM_STEP: f32 = 1.25;

/// Zoom step multiplier for scroll wheel (smaller for finer control)
pub const SCROLL_ZOOM_STEP: f32 = 1.08;

/// Minimum zoom level (10% of fit-to-view)
pub const MIN_ZOOM: f32 = 0.1;

/// Maximum zoom level (5000% of fit-to-view)
pub const MAX_ZOOM: f32 = 50.0;

/// Rotation step for +/- buttons (in degrees)
pub const ROTATION_STEP: f32 = 15.0;

/// View transformation state for pan, zoom, and rotation
#[derive(Clone, Debug)]
pub struct ViewTransform {
    /// Zoom level: 1.0 = fit-to-view, >1 = zoomed in, <1 = zoomed out
    pub zoom: f32,
    /// Pan offset in screen coordinates (pixels)
    pub pan_offset: Vec2,
    /// Rotation angle in degrees (counter-clockwise, math convention)
    pub rotation_degrees: f32,
    /// Pivot point for rotation in image coordinates (0..width-1, 0..height-1)
    /// Default is image center: ((width-1)/2, (height-1)/2)
    pub pivot_point: (f32, f32),
    /// Whether the pivot marker should be shown
    pub show_pivot_marker: bool,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan_offset: Vec2::ZERO,
            rotation_degrees: 0.0,
            pivot_point: (0.0, 0.0), // Will be set to image center when image is loaded
            show_pivot_marker: false,
        }
    }
}

impl ViewTransform {
    /// Create a new transform at default zoom (fit-to-view) with no pan or rotation
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset to fit-to-view state (zoom, pan, and rotation)
    /// Pivot point is kept at its current position (or image center if not set)
    pub fn reset(&mut self) {
        self.zoom = 1.0;
        self.pan_offset = Vec2::ZERO;
        self.rotation_degrees = 0.0;
    }

    /// Reset only pan offset, keeping current zoom level and rotation
    pub fn reset_pan(&mut self) {
        self.pan_offset = Vec2::ZERO;
    }

    /// Reset zoom and pan, keeping rotation and pivot
    pub fn reset_zoom_and_pan(&mut self) {
        self.zoom = 1.0;
        self.pan_offset = Vec2::ZERO;
    }

    /// Check if transform is at default state (for showing/hiding reset button)
    pub fn is_default(&self) -> bool {
        (self.zoom - 1.0).abs() < 0.001 
            && self.pan_offset.length() < 0.5
            && self.rotation_degrees.abs() < 0.001
    }

    /// Get rotation angle in degrees
    pub fn rotation(&self) -> f32 {
        self.rotation_degrees
    }

    /// Set rotation angle in degrees (counter-clockwise)
    pub fn set_rotation(&mut self, degrees: f32) {
        // Normalize to -180..180 range
        let mut normalized = degrees % 360.0;
        if normalized > 180.0 {
            normalized -= 360.0;
        } else if normalized < -180.0 {
            normalized += 360.0;
        }
        self.rotation_degrees = normalized;
    }

    /// Rotate by a delta amount (in degrees)
    pub fn rotate_by(&mut self, delta_degrees: f32) {
        self.set_rotation(self.rotation_degrees + delta_degrees);
    }

    /// Get pivot point in image coordinates
    pub fn pivot_point(&self) -> (f32, f32) {
        self.pivot_point
    }

    /// Set pivot point in image coordinates
    pub fn set_pivot_point(&mut self, x: f32, y: f32) {
        self.pivot_point = (x, y);
    }

    /// Initialize pivot point to image center
    pub fn set_pivot_to_center(&mut self, image_width: u32, image_height: u32) {
        self.pivot_point = (
            (image_width as f32 - 1.0) / 2.0,
            (image_height as f32 - 1.0) / 2.0,
        );
    }

    /// Zoom in by one step, centered on the given screen position
    pub fn zoom_in(&mut self, center: Option<Pos2>, viewport_center: Pos2) {
        let center = center.unwrap_or(viewport_center);
        self.zoom_around_point(ZOOM_STEP, center, viewport_center);
    }

    /// Zoom out by one step, centered on the given screen position
    pub fn zoom_out(&mut self, center: Option<Pos2>, viewport_center: Pos2) {
        let center = center.unwrap_or(viewport_center);
        self.zoom_around_point(1.0 / ZOOM_STEP, center, viewport_center);
    }

    /// Apply a zoom delta centered on a specific screen position.
    /// This preserves the point under the cursor while zooming.
    /// 
    /// The math: pan_offset is defined relative to viewport_center (see calculate_image_rect).
    /// To keep screen_pos showing the same image content after zoom:
    ///   pan_new = (screen_pos - viewport_center) * (1 - zoom_ratio) + pan_old * zoom_ratio
    pub fn zoom_around_point(&mut self, zoom_delta: f32, screen_pos: Pos2, viewport_center: Pos2) {
        if zoom_delta == 1.0 {
            return;
        }

        let old_zoom = self.zoom;
        let new_zoom = (old_zoom * zoom_delta).clamp(MIN_ZOOM, MAX_ZOOM);

        if (new_zoom - old_zoom).abs() < 0.0001 {
            return; // No change after clamping
        }

        let zoom_ratio = new_zoom / old_zoom;
        
        // d = position relative to viewport center
        let d = screen_pos - viewport_center;
        
        // New pan offset to keep the point under cursor fixed
        self.pan_offset = d * (1.0 - zoom_ratio) + self.pan_offset * zoom_ratio;
        self.zoom = new_zoom;
    }

    /// Apply a pan delta (in screen coordinates)
    pub fn pan_by(&mut self, delta: Vec2) {
        self.pan_offset += delta;
    }

    /// Center the view on a specific image position
    pub fn center_on_image_point(
        &mut self,
        image_pos: Pos2,
        image_size: Vec2,
        viewport_size: Vec2,
        base_image_rect: Rect,
    ) {
        // Match image_to_screen[_rotated] conventions:
        // - integer image coordinates are pixel centers (+0.5)
        // - FITS Y axis is flipped (Y=0 at bottom)
        let rel_x = (image_pos.x + 0.5) / image_size.x;
        let rel_y = 1.0 - (image_pos.y + 0.5) / image_size.y;

        // Position within the zoomed image
        let zoomed_size = base_image_rect.size() * self.zoom;
        let image_screen_pos = Vec2::new(rel_x * zoomed_size.x, rel_y * zoomed_size.y);

        // We want this point to be at viewport center
        let viewport_center = viewport_size / 2.0;

        // Calculate the required offset
        let zoomed_center_offset = (viewport_size - zoomed_size) / 2.0;

        self.pan_offset = viewport_center - image_screen_pos - zoomed_center_offset;
    }

    /// Calculate the display rect for the image given viewport and base image sizes.
    /// Returns the rect where the image should be drawn in screen coordinates.
    pub fn calculate_image_rect(&self, viewport_rect: Rect, base_display_size: Vec2) -> Rect {
        let zoomed_size = base_display_size * self.zoom;

        // Base position centers the image in the viewport
        let base_offset = (viewport_rect.size() - base_display_size) / 2.0;

        // Apply zoom offset (keeping center fixed) and pan
        let zoom_offset = (base_display_size - zoomed_size) / 2.0;
        let final_offset = base_offset + zoom_offset + self.pan_offset;

        Rect::from_min_size(viewport_rect.min + final_offset, zoomed_size)
    }

    /// Convert screen position to image coordinates
    /// Note: Y is flipped for FITS convention (Y=0 at bottom of displayed image)
    pub fn screen_to_image(
        &self,
        screen_pos: Pos2,
        image_rect: Rect,
        image_size: (u32, u32),
    ) -> Option<(u32, u32)> {
        if !image_rect.contains(screen_pos) {
            return None;
        }

        let rel_x = (screen_pos.x - image_rect.min.x) / image_rect.width();
        // Flip Y: screen Y increases downward, but image Y=0 is at bottom
        let rel_y = 1.0 - (screen_pos.y - image_rect.min.y) / image_rect.height();

        // Clamp to [0, 1) to handle boundary conditions
        let rel_x = rel_x.clamp(0.0, 0.9999999);
        let rel_y = rel_y.clamp(0.0, 0.9999999);

        let img_x = (rel_x * image_size.0 as f32).floor() as i32;
        let img_y = (rel_y * image_size.1 as f32).floor() as i32;

        if img_x >= 0 && img_x < image_size.0 as i32 && img_y >= 0 && img_y < image_size.1 as i32 {
            Some((img_x as u32, img_y as u32))
        } else {
            None
        }
    }

    /// Convert image coordinates to screen position
    /// Note: Y is flipped for FITS convention (Y=0 at bottom of displayed image)
    pub fn image_to_screen(&self, image_pos: (u32, u32), image_rect: Rect, image_size: (u32, u32)) -> Pos2 {
        let rel_x = (image_pos.0 as f32 + 0.5) / image_size.0 as f32;
        // Flip Y: image Y=0 is at bottom, but screen Y increases downward
        let rel_y = 1.0 - (image_pos.1 as f32 + 0.5) / image_size.1 as f32;

        Pos2::new(
            image_rect.min.x + rel_x * image_rect.width(),
            image_rect.min.y + rel_y * image_rect.height(),
        )
    }

    /// Convert screen position to image coordinates for setting the pivot point.
    /// 
    /// This is similar to screen_to_image but does NOT account for current rotation.
    /// When setting a new pivot point via alt-click, we want the marker to appear
    /// exactly where the user clicked, regardless of current rotation state.
    /// 
    /// The key difference from screen_to_image_rotated: we don't unrotate the
    /// screen position before converting.
    pub fn screen_to_image_for_pivot(
        &self,
        screen_pos: Pos2,
        image_rect: Rect,
        image_size: (u32, u32),
    ) -> Option<(u32, u32)> {
        // Check bounds against the unrotated image rect
        if !image_rect.contains(screen_pos) {
            return None;
        }

        let rel_x = (screen_pos.x - image_rect.min.x) / image_rect.width();
        // Flip Y: screen Y increases downward, but image Y=0 is at bottom
        let rel_y = 1.0 - (screen_pos.y - image_rect.min.y) / image_rect.height();

        // Clamp to [0, 1) to handle boundary conditions
        let rel_x = rel_x.clamp(0.0, 0.9999999);
        let rel_y = rel_y.clamp(0.0, 0.9999999);

        let img_x = (rel_x * image_size.0 as f32).floor() as i32;
        let img_y = (rel_y * image_size.1 as f32).floor() as i32;

        if img_x >= 0 && img_x < image_size.0 as i32 && img_y >= 0 && img_y < image_size.1 as i32 {
            Some((img_x as u32, img_y as u32))
        } else {
            None
        }
    }

    /// Clamp pan offset to keep at least part of the image visible
    pub fn clamp_pan_offset(&mut self, viewport_size: Vec2, zoomed_image_size: Vec2) {
        // Allow panning until only 10% of image is visible
        let margin = 0.1;
        let min_visible = zoomed_image_size * margin;

        // Calculate bounds for pan offset
        let max_pan_x = zoomed_image_size.x - min_visible.x;
        let max_pan_y = zoomed_image_size.y - min_visible.y;
        let min_pan_x = -(viewport_size.x - min_visible.x);
        let min_pan_y = -(viewport_size.y - min_visible.y);

        self.pan_offset.x = self.pan_offset.x.clamp(min_pan_x, max_pan_x);
        self.pan_offset.y = self.pan_offset.y.clamp(min_pan_y, max_pan_y);
    }

    /// Rotate a point around a center point
    /// angle_degrees: counter-clockwise rotation in degrees
    fn rotate_point(point: Pos2, center: Pos2, angle_degrees: f32) -> Pos2 {
        // Screen coordinates have +Y downward; negate angle so positive degrees
        // are still counter-clockwise in image/math convention.
        let angle_rad = -angle_degrees.to_radians();
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();
        
        let dx = point.x - center.x;
        let dy = point.y - center.y;
        
        Pos2::new(
            center.x + dx * cos_a - dy * sin_a,
            center.y + dx * sin_a + dy * cos_a,
        )
    }

    /// Inverse rotate a point (rotate by negative angle)
    fn unrotate_point(point: Pos2, center: Pos2, angle_degrees: f32) -> Pos2 {
        Self::rotate_point(point, center, -angle_degrees)
    }

    /// Calculate the four corners of the rotated image in screen coordinates
    /// Returns corners in order: top-left, top-right, bottom-right, bottom-left
    pub fn calculate_rotated_corners(
        &self,
        image_rect: Rect,
        image_size: (u32, u32),
    ) -> [Pos2; 4] {
        // Get pivot point in screen coordinates
        let pivot_screen = self.pivot_to_screen(image_rect, image_size);
        
        let corners = [
            image_rect.left_top(),
            image_rect.right_top(),
            image_rect.right_bottom(),
            image_rect.left_bottom(),
        ];
        
        corners.map(|corner| Self::rotate_point(corner, pivot_screen, self.rotation_degrees))
    }

    /// Convert pivot point from image coordinates to screen coordinates
    pub fn pivot_to_screen(&self, image_rect: Rect, image_size: (u32, u32)) -> Pos2 {
        let rel_x = (self.pivot_point.0 + 0.5) / image_size.0 as f32;
        // Flip Y for FITS convention
        let rel_y = 1.0 - (self.pivot_point.1 + 0.5) / image_size.1 as f32;
        
        Pos2::new(
            image_rect.min.x + rel_x * image_rect.width(),
            image_rect.min.y + rel_y * image_rect.height(),
        )
    }

    /// Convert screen position to image coordinates, accounting for rotation
    /// Returns continuous image coordinates where integer values are pixel centers:
    /// pixel (0, 0) spans [-0.5, 0.5) in both axes.
    pub fn screen_to_image_continuous_rotated(
        &self,
        screen_pos: Pos2,
        image_rect: Rect,
        image_size: (u32, u32),
    ) -> Option<(f32, f32)> {
        // First, unrotate the screen position around the pivot
        let pivot_screen = self.pivot_to_screen(image_rect, image_size);
        let unrotated_pos = Self::unrotate_point(screen_pos, pivot_screen, self.rotation_degrees);

        let rel_x = (unrotated_pos.x - image_rect.min.x) / image_rect.width();
        let rel_y = 1.0 - (unrotated_pos.y - image_rect.min.y) / image_rect.height();

        // Continuous coordinates with pixel-center convention.
        let x = rel_x * image_size.0 as f32 - 0.5;
        let y = rel_y * image_size.1 as f32 - 0.5;

        if x >= -0.5 && x < image_size.0 as f32 - 0.5 && y >= -0.5 && y < image_size.1 as f32 - 0.5
        {
            Some((x, y))
        } else {
            None
        }
    }

    /// Convert screen position to image coordinates, accounting for rotation
    /// Note: Y is flipped for FITS convention (Y=0 at bottom of displayed image)
    pub fn screen_to_image_rotated(
        &self,
        screen_pos: Pos2,
        image_rect: Rect,
        image_size: (u32, u32),
    ) -> Option<(u32, u32)> {
        let (x, y) = self.screen_to_image_continuous_rotated(screen_pos, image_rect, image_size)?;
        let img_x = (x + 0.5).floor() as i32;
        let img_y = (y + 0.5).floor() as i32;
        Some((img_x as u32, img_y as u32))
    }

    /// Convert image coordinates to screen position, accounting for rotation
    pub fn image_to_screen_continuous_rotated(
        &self,
        image_pos: (f32, f32),
        image_rect: Rect,
        image_size: (u32, u32),
    ) -> Pos2 {
        let rel_x = (image_pos.0 + 0.5) / image_size.0 as f32;
        let rel_y = 1.0 - (image_pos.1 + 0.5) / image_size.1 as f32;

        let unrotated_pos = Pos2::new(
            image_rect.min.x + rel_x * image_rect.width(),
            image_rect.min.y + rel_y * image_rect.height(),
        );

        // Rotate around the pivot
        let pivot_screen = self.pivot_to_screen(image_rect, image_size);
        Self::rotate_point(unrotated_pos, pivot_screen, self.rotation_degrees)
    }

    /// Convert image coordinates to screen position, accounting for rotation
    pub fn image_to_screen_rotated(
        &self,
        image_pos: (u32, u32),
        image_rect: Rect,
        image_size: (u32, u32),
    ) -> Pos2 {
        self.image_to_screen_continuous_rotated(
            (image_pos.0 as f32, image_pos.1 as f32),
            image_rect,
            image_size,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_transform() {
        let t = ViewTransform::new();
        assert!((t.zoom - 1.0).abs() < 0.001);
        assert!(t.pan_offset.length() < 0.001);
        assert!(t.is_default());
    }

    #[test]
    fn test_reset() {
        let mut t = ViewTransform::new();
        t.zoom = 2.5;
        t.pan_offset = Vec2::new(100.0, 50.0);
        assert!(!t.is_default());

        t.reset();
        assert!(t.is_default());
    }

    #[test]
    fn test_zoom_in_out() {
        let mut t = ViewTransform::new();
        let center = Pos2::new(400.0, 300.0);

        t.zoom_in(Some(center), center);
        assert!((t.zoom - ZOOM_STEP).abs() < 0.001);

        t.zoom_out(Some(center), center);
        assert!((t.zoom - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_zoom_clamping() {
        let mut t = ViewTransform::new();
        let center = Pos2::new(400.0, 300.0);

        // Zoom way in
        for _ in 0..100 {
            t.zoom_in(Some(center), center);
        }
        assert!(t.zoom <= MAX_ZOOM);

        // Reset and zoom way out
        t.reset();
        for _ in 0..100 {
            t.zoom_out(Some(center), center);
        }
        assert!(t.zoom >= MIN_ZOOM);
    }

    #[test]
    fn test_zoom_around_point_preserves_center() {
        let mut t = ViewTransform::new();
        t.pan_offset = Vec2::new(50.0, 30.0);
        let viewport_center = Pos2::new(400.0, 300.0);
        let zoom_center = Pos2::new(200.0, 150.0);

        // Calculate what's under zoom_center before zoom
        // d = zoom_center - viewport_center
        let d = zoom_center - viewport_center;
        let old_offset = t.pan_offset;
        let old_zoom = t.zoom;

        t.zoom_around_point(2.0, zoom_center, viewport_center);

        // After zoom, the same relative position in image space should be under zoom_center
        // Using the formula: pan_new = d * (1 - zoom_ratio) + pan_old * zoom_ratio
        let zoom_ratio = 2.0;
        let expected_offset = d * (1.0 - zoom_ratio) + old_offset * zoom_ratio;

        assert!((t.pan_offset.x - expected_offset.x).abs() < 0.01);
        assert!((t.pan_offset.y - expected_offset.y).abs() < 0.01);
    }

    #[test]
    fn test_pan_by() {
        let mut t = ViewTransform::new();
        t.pan_by(Vec2::new(10.0, 20.0));
        assert!((t.pan_offset.x - 10.0).abs() < 0.001);
        assert!((t.pan_offset.y - 20.0).abs() < 0.001);

        t.pan_by(Vec2::new(-5.0, -10.0));
        assert!((t.pan_offset.x - 5.0).abs() < 0.001);
        assert!((t.pan_offset.y - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_screen_to_image_inside() {
        let t = ViewTransform::new();
        let image_rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        let image_size = (100, 100);

        // Center of image rect should map to center of image
        let center = Pos2::new(200.0, 200.0);
        let result = t.screen_to_image(center, image_rect, image_size);
        assert!(result.is_some(), "Center {:?} should be inside rect {:?}", center, image_rect);
        let (x, y) = result.unwrap();
        assert_eq!(x, 50);
        assert_eq!(y, 50); // Center is still center with Y-flip

        // Top-left corner of screen maps to bottom-left of image (FITS convention)
        let top_left = Pos2::new(100.0, 100.0);
        let result = t.screen_to_image(top_left, image_rect, image_size);
        assert!(result.is_some(), "Top-left {:?} should be inside rect {:?}, contains={}", top_left, image_rect, image_rect.contains(top_left));
        let (x, y) = result.unwrap();
        assert_eq!(x, 0);
        assert_eq!(y, 99); // Y is flipped: top of screen = Y=99 in image

        // Bottom-left corner of screen maps to top-left of image (FITS convention)
        let bottom_left = Pos2::new(100.0, 299.0);
        let result = t.screen_to_image(bottom_left, image_rect, image_size);
        assert!(result.is_some(), "Bottom-left {:?} should be inside rect {:?}", bottom_left, image_rect);
        let (x, y) = result.unwrap();
        assert_eq!(x, 0);
        assert_eq!(y, 0); // Y is flipped: bottom of screen = Y=0 in image
    }

    #[test]
    fn test_screen_to_image_outside() {
        let t = ViewTransform::new();
        let image_rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        let image_size = (100, 100);

        // Outside image rect
        let outside = Pos2::new(50.0, 50.0);
        let result = t.screen_to_image(outside, image_rect, image_size);
        assert!(result.is_none());
    }

    #[test]
    fn test_center_on_image_point_places_selected_pixel_at_viewport_center() {
        let mut t = ViewTransform::new();
        let viewport_size = Vec2::new(800.0, 600.0);
        let viewport_rect = Rect::from_min_size(Pos2::ZERO, viewport_size);
        let image_size = (100, 100);
        let base_size = Vec2::new(600.0, 600.0);
        let base_image_rect = Rect::from_center_size(viewport_rect.center(), base_size);

        let click_pos = Pos2::new(150.0, 120.0);
        let image_rect_before = t.calculate_image_rect(viewport_rect, base_size);
        let (img_x, img_y) = t
            .screen_to_image_rotated(click_pos, image_rect_before, image_size)
            .unwrap();

        t.center_on_image_point(
            Pos2::new(img_x as f32, img_y as f32),
            Vec2::new(image_size.0 as f32, image_size.1 as f32),
            viewport_size,
            base_image_rect,
        );

        let image_rect_after = t.calculate_image_rect(viewport_rect, base_size);
        let centered_pixel_pos = t.image_to_screen_rotated((img_x, img_y), image_rect_after, image_size);
        let viewport_center = viewport_rect.center();

        assert!((centered_pixel_pos.x - viewport_center.x).abs() < 0.01);
        assert!((centered_pixel_pos.y - viewport_center.y).abs() < 0.01);
    }

    #[test]
    fn test_calculate_image_rect_default_zoom() {
        let t = ViewTransform::new();
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let base_size = Vec2::new(400.0, 300.0);

        let result = t.calculate_image_rect(viewport, base_size);

        // Should be centered
        assert!((result.center().x - 400.0).abs() < 0.01);
        assert!((result.center().y - 300.0).abs() < 0.01);
        assert!((result.width() - 400.0).abs() < 0.01);
        assert!((result.height() - 300.0).abs() < 0.01);
    }

    #[test]
    fn test_calculate_image_rect_zoomed() {
        let mut t = ViewTransform::new();
        t.zoom = 2.0;
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let base_size = Vec2::new(400.0, 300.0);

        let result = t.calculate_image_rect(viewport, base_size);

        // Should be 2x size, still centered
        assert!((result.width() - 800.0).abs() < 0.01);
        assert!((result.height() - 600.0).abs() < 0.01);
        assert!((result.center().x - 400.0).abs() < 0.01);
        assert!((result.center().y - 300.0).abs() < 0.01);
    }

    /// Test that setting a pivot point via click and then displaying the marker
    /// results in the marker appearing at the click location.
    /// 
    /// This simulates the alt-click workflow:
    /// 1. User clicks at screen position
    /// 2. Convert screen position to image coordinates
    /// 3. Set that as the pivot point
    /// 4. Convert pivot back to screen coordinates for marker display
    /// 5. Marker should appear at original click location
    #[test]
    fn test_pivot_point_round_trip_no_rotation() {
        let t = ViewTransform::new();
        let image_rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        let image_size = (100u32, 100u32);

        // Click at center of the image on screen
        let click_pos = Pos2::new(200.0, 200.0);
        
        // Convert click to image coordinates (simulating alt-click)
        let img_coords = t.screen_to_image_rotated(click_pos, image_rect, image_size);
        assert!(img_coords.is_some(), "Click should be within image bounds");
        let (img_x, img_y) = img_coords.unwrap();
        
        // Set as pivot point
        let mut t2 = t.clone();
        t2.set_pivot_point(img_x as f32, img_y as f32);
        
        // Convert pivot back to screen coordinates
        let marker_pos = t2.pivot_to_screen(image_rect, image_size);
        
        // Marker should be at the click location (within 1 pixel tolerance due to discretization)
        let tolerance = image_rect.width() / image_size.0 as f32; // 1 image pixel in screen coords
        assert!(
            (marker_pos.x - click_pos.x).abs() < tolerance,
            "Marker X ({}) should be near click X ({}), diff={}",
            marker_pos.x, click_pos.x, (marker_pos.x - click_pos.x).abs()
        );
        assert!(
            (marker_pos.y - click_pos.y).abs() < tolerance,
            "Marker Y ({}) should be near click Y ({}), diff={}",
            marker_pos.y, click_pos.y, (marker_pos.y - click_pos.y).abs()
        );
    }

    /// Test pivot round-trip when there IS an existing rotation.
    /// When setting a new pivot via alt-click, we should use the DIRECT
    /// screen-to-image conversion (not rotation-aware), so the marker
    /// appears where the user clicked.
    #[test]
    fn test_pivot_point_round_trip_with_rotation() {
        let mut t = ViewTransform::new();
        t.rotation_degrees = 45.0; // 45 degree rotation
        // Initial pivot at image center
        t.pivot_point = (49.5, 49.5);
        
        let image_rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        let image_size = (100u32, 100u32);

        // User clicks at a specific screen position (upper-right area)
        let click_pos = Pos2::new(250.0, 150.0);
        
        // For setting a new pivot, we want the marker to appear WHERE the user clicked,
        // not where that point maps to in the rotated image coordinate system.
        // So we should use direct (non-rotated) screen-to-image conversion.
        let img_coords = t.screen_to_image_for_pivot(click_pos, image_rect, image_size);
        assert!(img_coords.is_some(), "Click should be within image bounds");
        let (img_x, img_y) = img_coords.unwrap();
        
        // Set as new pivot point
        t.set_pivot_point(img_x as f32, img_y as f32);
        
        // Convert pivot back to screen coordinates
        let marker_pos = t.pivot_to_screen(image_rect, image_size);
        
        // Marker should be at the click location
        let tolerance = image_rect.width() / image_size.0 as f32;
        assert!(
            (marker_pos.x - click_pos.x).abs() < tolerance,
            "Marker X ({}) should be near click X ({}), diff={}",
            marker_pos.x, click_pos.x, (marker_pos.x - click_pos.x).abs()
        );
        assert!(
            (marker_pos.y - click_pos.y).abs() < tolerance,
            "Marker Y ({}) should be near click Y ({}), diff={}",
            marker_pos.y, click_pos.y, (marker_pos.y - click_pos.y).abs()
        );
    }

    /// Test reset_zoom_and_pan preserves rotation and pivot
    #[test]
    fn test_reset_zoom_and_pan_preserves_rotation() {
        let mut t = ViewTransform::new();
        
        // Set up some state
        t.zoom = 2.5;
        t.pan_offset = Vec2::new(100.0, -50.0);
        t.rotation_degrees = 45.0;
        t.pivot_point = (25.0, 75.0);
        t.show_pivot_marker = true;
        
        // Reset zoom and pan only
        t.reset_zoom_and_pan();
        
        // Zoom and pan should be reset
        assert!((t.zoom - 1.0).abs() < 0.001, "Zoom should be reset to 1.0");
        assert!(t.pan_offset.length() < 0.001, "Pan should be reset to zero");
        
        // Rotation and pivot should be preserved
        assert!((t.rotation_degrees - 45.0).abs() < 0.001, "Rotation should be preserved");
        assert!((t.pivot_point.0 - 25.0).abs() < 0.001, "Pivot X should be preserved");
        assert!((t.pivot_point.1 - 75.0).abs() < 0.001, "Pivot Y should be preserved");
        assert!(t.show_pivot_marker, "Pivot marker visibility should be preserved");
    }

    /// Test full reset clears rotation as well
    #[test]
    fn test_full_reset_clears_rotation() {
        let mut t = ViewTransform::new();
        
        // Set up some state
        t.zoom = 2.5;
        t.pan_offset = Vec2::new(100.0, -50.0);
        t.rotation_degrees = 45.0;
        t.pivot_point = (25.0, 75.0);
        
        // Full reset
        t.reset();
        
        // Everything should be reset (except pivot which is preserved)
        assert!((t.zoom - 1.0).abs() < 0.001, "Zoom should be reset to 1.0");
        assert!(t.pan_offset.length() < 0.001, "Pan should be reset to zero");
        assert!(t.rotation_degrees.abs() < 0.001, "Rotation should be reset to 0");
        // Pivot is preserved even in full reset
        assert!((t.pivot_point.0 - 25.0).abs() < 0.001, "Pivot X should be preserved");
        assert!((t.pivot_point.1 - 75.0).abs() < 0.001, "Pivot Y should be preserved");
    }

    /// Test rotation angle normalization to -180..180 range
    #[test]
    fn test_rotation_normalization() {
        let mut t = ViewTransform::new();
        
        // Test positive overflow
        t.set_rotation(270.0);
        assert!((t.rotation() - (-90.0)).abs() < 0.001, "270° should normalize to -90°");
        
        // Test negative overflow
        t.set_rotation(-270.0);
        assert!((t.rotation() - 90.0).abs() < 0.001, "-270° should normalize to 90°");
        
        // Test large positive
        t.set_rotation(450.0);
        assert!((t.rotation() - 90.0).abs() < 0.001, "450° should normalize to 90°");
        
        // Test large negative
        t.set_rotation(-450.0);
        assert!((t.rotation() - (-90.0)).abs() < 0.001, "-450° should normalize to -90°");
        
        // Test exact boundaries
        t.set_rotation(180.0);
        assert!((t.rotation() - 180.0).abs() < 0.001, "180° should stay 180°");
        
        t.set_rotation(-180.0);
        assert!((t.rotation() - (-180.0)).abs() < 0.001, "-180° should stay -180°");
    }

    /// Test rotate_by accumulates correctly with normalization
    #[test]
    fn test_rotate_by() {
        let mut t = ViewTransform::new();
        
        t.rotate_by(45.0);
        assert!((t.rotation() - 45.0).abs() < 0.001);
        
        t.rotate_by(45.0);
        assert!((t.rotation() - 90.0).abs() < 0.001);
        
        // Rotate past 180 should wrap
        t.rotate_by(100.0);  // 90 + 100 = 190 -> -170
        assert!((t.rotation() - (-170.0)).abs() < 0.001, "190° should wrap to -170°");
    }

    /// Test boundary condition: clicking exactly at top-left corner of image rect
    /// should map to valid image coordinates (not out of bounds)
    #[test]
    fn test_screen_to_image_boundary_top_left() {
        let t = ViewTransform::new();
        let image_rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        let image_size = (100u32, 100u32);

        // Click exactly at top-left corner
        let corner = Pos2::new(100.0, 100.0);
        let result = t.screen_to_image(corner, image_rect, image_size);
        
        assert!(result.is_some(), "Top-left corner should be valid");
        let (x, y) = result.unwrap();
        assert_eq!(x, 0, "X should be 0");
        assert_eq!(y, 99, "Y should be 99 (FITS: top of screen = high Y)");
    }

    /// Test boundary condition for screen_to_image_for_pivot
    #[test]
    fn test_screen_to_image_for_pivot_boundary() {
        let t = ViewTransform::new();
        let image_rect = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 200.0));
        let image_size = (100u32, 100u32);

        // Click exactly at top-left corner
        let corner = Pos2::new(100.0, 100.0);
        let result = t.screen_to_image_for_pivot(corner, image_rect, image_size);
        
        assert!(result.is_some(), "Top-left corner should be valid for pivot");
        let (x, y) = result.unwrap();
        assert_eq!(x, 0, "X should be 0");
        assert_eq!(y, 99, "Y should be 99 (FITS convention)");
    }

    /// Test is_default correctly considers rotation
    #[test]
    fn test_is_default_with_rotation() {
        let mut t = ViewTransform::new();
        assert!(t.is_default(), "Fresh transform should be default");
        
        t.rotation_degrees = 15.0;
        assert!(!t.is_default(), "Rotated transform should not be default");
        
        t.rotation_degrees = 0.0;
        assert!(t.is_default(), "Zero rotation should be default again");
        
        t.pan_offset = Vec2::new(10.0, 0.0);
        assert!(!t.is_default(), "Panned transform should not be default");
    }
}
