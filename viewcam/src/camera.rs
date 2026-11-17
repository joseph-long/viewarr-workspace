//! Camera-frame arithmetic, independent of any UI.
//!
//! A camera hands over up to three images per view: the science `data`, a
//! `dark` frame to subtract from it, and a `saturation` image flagging pixels
//! that hit the detector's ceiling. [`CameraFrames`] holds the latest of each
//! and derives what the viewer should show from them. Each input is kept as it
//! arrived, so toggling dark subtraction off and on again needs no resend.

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
    saturation: Option<Mask>,
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
    pub fn set_saturation(&mut self, frame: Option<Frame>) {
        self.saturation = frame.map(|f| Mask {
            mask: f.pixels.iter().map(|&v| v > 0.0 || v < 0.0).collect(),
            width: f.width,
            height: f.height,
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
}
