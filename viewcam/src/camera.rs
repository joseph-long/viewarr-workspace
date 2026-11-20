//! Camera-frame arithmetic, independent of any UI.
//!
//! A camera hands over up to three images per view: the science `data`, a
//! `dark` frame to subtract from it, and a `saturation` image flagging pixels
//! that hit the detector's ceiling. A camera without a saturation image can
//! give a saturation level instead, and each data frame's pixels at or above
//! it are flagged. [`CameraFrames`] holds the latest of each and derives what
//! the viewer should show from them. Each input is kept as it arrived, so
//! toggling dark subtraction off and on again needs no resend.

pub use viewcam_codec::Quantizer;

/// One image as handed to the viewer: `f64` pixels, row-major, plus the
/// display metadata viewarr derives from the source dtype.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub pixels: Vec<f64>,
    pub width: u32,
    pub height: u32,
    /// Whether the source dtype was an integer type (hover shows no decimals).
    pub is_integer: bool,
    /// Decimal places for floating hover values.
    pub value_decimals: usize,
    /// For a quantized frame (viewcam-proto/PROTOCOL.md, "Quantization"),
    /// the quantizer whose rounded values `pixels` are. A saturation level
    /// is compared with them as its threshold, since rounding can bring a
    /// saturated pixel back a little under the level itself.
    pub quantization: Option<Quantizer>,
}

impl Frame {
    fn shape(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Whether an auxiliary image (dark or saturation) can be applied to the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// Loaded, and either matches the data's shape or there is no data yet.
    Ready,
    /// Never loaded, or cleared.
    Missing,
    /// Loaded, but its shape differs from the data's. Kept rather than
    /// dropped: data of the matching shape may be on its way.
    ShapeMismatch { aux: (u32, u32), data: (u32, u32) },
}

impl Availability {
    pub fn is_ready(self) -> bool {
        self == Availability::Ready
    }
}

/// A boolean saturation mask, `true` where a pixel saturated.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub mask: Vec<bool>,
    pub width: u32,
    pub height: u32,
}

/// The latest data, dark and saturation images for one view.
#[derive(Default)]
pub struct CameraFrames {
    data: Option<Frame>,
    dark: Option<Frame>,
    /// From a saturation image, or derived from the data at the saturation level.
    saturation: Option<Mask>,
    /// When set, `saturation` is derived from each data frame rather than given.
    saturation_level: Option<f64>,
}

impl CameraFrames {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn data(&self) -> Option<&Frame> {
        self.data.as_ref()
    }

    pub fn set_data(&mut self, frame: Frame) {
        self.data = Some(frame);
        if self.saturation_level.is_some() {
            self.derive_saturation();
        }
    }

    pub fn dark(&self) -> Option<&Frame> {
        self.dark.as_ref()
    }

    /// Set (or with `None`, clear) the dark frame.
    pub fn set_dark(&mut self, frame: Option<Frame>) {
        self.dark = frame;
    }

    pub fn saturation(&self) -> Option<&Mask> {
        self.saturation.as_ref()
    }

    /// Set (or with `None`, clear) the saturation image. Any nonzero pixel
    /// counts as saturated; NaN does not, since it says nothing either way.
    /// Replaces any saturation level.
    pub fn set_saturation(&mut self, frame: Option<Frame>) {
        self.saturation_level = None;
        self.saturation = frame.map(|f| Mask {
            mask: f.pixels.iter().map(|&v| v > 0.0 || v < 0.0).collect(),
            width: f.width,
            height: f.height,
        });
    }

    pub fn saturation_level(&self) -> Option<f64> {
        self.saturation_level
    }

    /// Set (or with `None`, clear) the saturation level: from now on, each
    /// data frame's pixels at or above it are saturated. It is compared with
    /// the data as the camera sent it, before any dark is subtracted, since
    /// that is what the detector clipped. Replaces any saturation image.
    pub fn set_saturation_level(&mut self, level: Option<f64>) {
        self.saturation_level = level;
        self.saturation = None;
        if level.is_some() {
            self.derive_saturation();
        }
    }

    fn derive_saturation(&mut self) {
        let (Some(level), Some(data)) = (self.saturation_level, &self.data) else {
            return;
        };
        let level = data.quantization.map_or(level, |q| q.threshold(level));
        self.saturation = Some(Mask {
            mask: data.pixels.iter().map(|&v| v >= level).collect(),
            width: data.width,
            height: data.height,
        });
    }

    pub fn dark_status(&self) -> Availability {
        self.status_of(self.dark.as_ref().map(Frame::shape))
    }

    pub fn saturation_status(&self) -> Availability {
        self.status_of(self.saturation.as_ref().map(|m| (m.width, m.height)))
    }

    fn status_of(&self, aux: Option<(u32, u32)>) -> Availability {
        match (aux, self.data.as_ref().map(Frame::shape)) {
            (None, _) => Availability::Missing,
            (Some(aux), Some(data)) if aux != data => Availability::ShapeMismatch { aux, data },
            (Some(_), _) => Availability::Ready,
        }
    }

    /// The image to display: the data, minus the dark frame when
    /// `subtract_dark` is set and the dark fits. `None` until data arrives.
    pub fn display_image(&self, subtract_dark: bool) -> Option<Frame> {
        let data = self.data.as_ref()?;
        let dark = match &self.dark {
            Some(dark) if subtract_dark && dark.shape() == data.shape() => dark,
            _ => return Some(data.clone()),
        };
        // In f64, so integer data minus a brighter dark goes negative instead
        // of wrapping around as it would in the camera's unsigned type.
        let pixels = data
            .pixels
            .iter()
            .zip(&dark.pixels)
            .map(|(&d, &k)| d - k)
            .collect();
        Some(Frame {
            pixels,
            width: data.width,
            height: data.height,
            // A float dark (e.g. an average of many) makes the difference
            // fractional, so it takes on the dark's precision.
            is_integer: data.is_integer && dark.is_integer,
            value_decimals: data.value_decimals.max(dark.value_decimals),
            quantization: None,
        })
    }

    /// The saturation mask, when one is loaded that fits the data.
    pub fn applicable_saturation(&self) -> Option<&Mask> {
        if self.saturation_status().is_ready() {
            self.saturation.as_ref()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int_frame(pixels: &[f64], width: u32, height: u32) -> Frame {
        Frame {
            pixels: pixels.to_vec(),
            width,
            height,
            is_integer: true,
            value_decimals: 0,
            quantization: None,
        }
    }

    #[test]
    fn subtracts_dark_and_goes_negative_rather_than_wrapping() {
        let mut frames = CameraFrames::new();
        frames.set_data(int_frame(&[10.0, 20.0, 5.0, 65535.0], 2, 2));
        frames.set_dark(Some(int_frame(&[3.0, 3.0, 8.0, 0.0], 2, 2)));

        let shown = frames.display_image(true).unwrap();
        assert_eq!(shown.pixels, vec![7.0, 17.0, -3.0, 65535.0]);
        assert!(shown.is_integer);

        // Off again: the raw data, untouched by the earlier subtraction.
        assert_eq!(
            frames.display_image(false).unwrap().pixels,
            vec![10.0, 20.0, 5.0, 65535.0]
        );
    }

    #[test]
    fn float_dark_makes_the_difference_fractional() {
        let mut frames = CameraFrames::new();
        frames.set_data(int_frame(&[10.0, 20.0], 2, 1));
        frames.set_dark(Some(Frame {
            pixels: vec![0.5, 1.25],
            width: 2,
            height: 1,
            is_integer: false,
            value_decimals: 4,
            quantization: None,
        }));

        let shown = frames.display_image(true).unwrap();
        assert_eq!(shown.pixels, vec![9.5, 18.75]);
        assert!(!shown.is_integer);
        assert_eq!(shown.value_decimals, 4);
    }

    #[test]
    fn mismatched_dark_is_kept_but_not_applied() {
        let mut frames = CameraFrames::new();
        frames.set_dark(Some(int_frame(&[1.0; 4], 2, 2)));
        // A dark that arrives before any data is ready to use.
        assert_eq!(frames.dark_status(), Availability::Ready);

        frames.set_data(int_frame(&[5.0; 6], 3, 2));
        assert_eq!(
            frames.dark_status(),
            Availability::ShapeMismatch {
                aux: (2, 2),
                data: (3, 2)
            }
        );
        assert_eq!(frames.display_image(true).unwrap().pixels, vec![5.0; 6]);

        // Data of the dark's shape arrives: it applies again, no resend needed.
        frames.set_data(int_frame(&[5.0; 4], 2, 2));
        assert_eq!(frames.display_image(true).unwrap().pixels, vec![4.0; 4]);
    }

    #[test]
    fn saturation_is_any_nonzero_pixel() {
        let mut frames = CameraFrames::new();
        assert_eq!(frames.saturation_status(), Availability::Missing);
        frames.set_saturation(Some(Frame {
            pixels: vec![0.0, 1.0, -1.0, f64::NAN, 0.25, -0.0],
            width: 3,
            height: 2,
            is_integer: false,
            value_decimals: 6,
            quantization: None,
        }));
        assert_eq!(
            frames.saturation().unwrap().mask,
            vec![false, true, true, false, true, false]
        );

        frames.set_data(int_frame(&[0.0; 4], 2, 2));
        assert!(frames.applicable_saturation().is_none());
        frames.set_saturation(None);
        assert_eq!(frames.saturation_status(), Availability::Missing);
    }

    #[test]
    fn saturation_level_flags_each_frame_before_dark_subtraction() {
        let mut frames = CameraFrames::new();
        frames.set_saturation_level(Some(100.0));
        // Nothing to flag until data arrives.
        assert_eq!(frames.saturation_status(), Availability::Missing);

        frames.set_dark(Some(int_frame(&[10.0; 4], 2, 2)));
        frames.set_data(int_frame(&[99.0, 100.0, 101.0, f64::NAN], 2, 2));
        assert_eq!(
            frames.applicable_saturation().unwrap().mask,
            vec![false, true, true, false]
        );

        // Each new frame, whatever its shape, brings its own mask.
        frames.set_data(int_frame(&[100.0, 0.0, 0.0], 3, 1));
        let mask = frames.applicable_saturation().unwrap();
        assert_eq!(mask.mask, vec![true, false, false]);
        assert_eq!((mask.width, mask.height), (3, 1));
    }

    #[test]
    fn a_quantized_frame_is_compared_with_the_level_as_its_threshold() {
        let q = Quantizer {
            bias: 100.0,
            read_noise: 4.0,
            noise_slope: 1.0,
            step: 0.25,
        };
        let level = 1000.0;
        // The value a pixel at the level comes back as, rounded down a little.
        let just_under = q.threshold(level) + 1e-6;
        assert!(just_under < level);
        let mut frames = CameraFrames::new();
        frames.set_saturation_level(Some(level));
        frames.set_data(Frame {
            quantization: Some(q),
            ..int_frame(&[0.0, just_under], 2, 1)
        });
        assert_eq!(frames.saturation().unwrap().mask, vec![false, true]);
        // Exact, the same value is under the level.
        frames.set_data(int_frame(&[0.0, just_under], 2, 1));
        assert_eq!(frames.saturation().unwrap().mask, vec![false, false]);
    }

    #[test]
    fn saturation_level_and_image_replace_each_other() {
        let mut frames = CameraFrames::new();
        frames.set_data(int_frame(&[0.0, 200.0], 2, 1));
        frames.set_saturation(Some(int_frame(&[1.0, 0.0], 2, 1)));

        frames.set_saturation_level(Some(100.0));
        assert_eq!(frames.saturation().unwrap().mask, vec![false, true]);

        frames.set_saturation(Some(int_frame(&[1.0, 0.0], 2, 1)));
        assert_eq!(frames.saturation_level(), None);
        frames.set_data(int_frame(&[0.0, 300.0], 2, 1));
        assert_eq!(frames.saturation().unwrap().mask, vec![true, false]);

        frames.set_saturation_level(Some(100.0));
        frames.set_saturation_level(None);
        assert_eq!(frames.saturation_status(), Availability::Missing);
    }
}
