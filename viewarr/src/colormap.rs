//! Colormap definitions and utilities
//!
//! Contains matplotlib-compatible colormaps exported as lookup tables.

use egui::Color32;

// Import generated lookup tables
use crate::colormap_luts::{INFERNO_LUT, MAGMA_LUT, RDBU_LUT, RDYLBU_LUT};

/// Available colormap types
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Colormap {
    #[default]
    Grayscale,
    Inferno,
    Magma,
    /// Diverging colormap, only available in symmetric mode
    RdBu,
    /// Diverging colormap, only available in symmetric mode
    RdYlBu,
}

impl Colormap {
    /// Parse a colormap name from JavaScript/user input.
    ///
    /// Accepts canonical names used by the UI/state callbacks, with a few
    /// common aliases.
    pub fn from_name(name: &str) -> Option<Self> {
        let normalized = name.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "gray" | "grayscale" | "greyscale" => Some(Colormap::Grayscale),
            "inferno" => Some(Colormap::Inferno),
            "magma" => Some(Colormap::Magma),
            "rdbu" => Some(Colormap::RdBu),
            "rdylbu" => Some(Colormap::RdYlBu),
            _ => None,
        }
    }

    /// Get display name for UI
    pub fn name(&self) -> &'static str {
        match self {
            Colormap::Grayscale => "gray",
            Colormap::Inferno => "inferno",
            Colormap::Magma => "magma",
            Colormap::RdBu => "RdBu",
            Colormap::RdYlBu => "RdYlBu",
        }
    }

    /// Check if this is a diverging colormap (requires symmetric mode)
    pub fn is_diverging(&self) -> bool {
        matches!(self, Colormap::RdBu | Colormap::RdYlBu)
    }

    /// Get all non-diverging colormaps
    pub fn standard_colormaps() -> &'static [Colormap] {
        &[Colormap::Grayscale, Colormap::Inferno, Colormap::Magma]
    }

    /// Get diverging colormaps (for symmetric mode)
    pub fn diverging_colormaps() -> &'static [Colormap] {
        &[Colormap::RdBu, Colormap::RdYlBu]
    }

    /// Map a normalized value (0-1) to a color
    pub fn map(&self, t: f64) -> Color32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Colormap::Grayscale => {
                let v = (t * 255.0) as u8;
                Color32::from_rgb(v, v, v)
            }
            Colormap::Inferno => sample_lut(&INFERNO_LUT, t),
            Colormap::Magma => sample_lut(&MAGMA_LUT, t),
            Colormap::RdBu => sample_lut(&RDBU_LUT, t),
            Colormap::RdYlBu => sample_lut(&RDYLBU_LUT, t),
        }
    }
}

/// Sample a 256-entry lookup table
fn sample_lut(lut: &[[u8; 3]; 256], t: f64) -> Color32 {
    let idx = (t * 255.0) as usize;
    let idx = idx.min(255);
    let rgb = lut[idx];
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}
