//! One open file, with its own viewer state.
//!
//! Each tab owns a full `ArrayViewerWidget`, so zoom, stretch, colormap and
//! slice position are per-file and survive switching away and back. A tab
//! showing a multi-extension file keeps one viewer per HDU it has visited, so
//! the same is true of switching HDU within a file.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use viewarr::widget::ArrayViewerWidget;

use crate::fits::FitsFile;

/// One row of the HDU picker.
pub struct HduChoice {
    pub index: usize,
    /// `[1] SCI  200x120  f32`
    pub label: String,
    /// Why this HDU cannot be shown, or `None` if it can be selected.
    pub unviewable_reason: Option<&'static str>,
}

pub struct Tab {
    fits: FitsFile,
    hdu_index: usize,
    widget: ArrayViewerWidget,
    /// Viewers for HDUs this tab showed earlier, kept so switching back
    /// restores that HDU's own zoom, stretch and slice position. This is the
    /// viewer-per-HDU the JupyterLab front-end also keeps, except built on
    /// demand: an HDU never visited costs nothing.
    parked: HashMap<usize, ArrayViewerWidget>,
    /// Short label for the tab bar: the file's name.
    title: String,
    /// Set when a slice read fails, so the failure is visible in the window
    /// rather than only on stderr.
    error: Option<String>,
}

impl Tab {
    /// Open `path` and load its first plane, so the tab has something to show
    /// the moment it appears.
    pub fn open(path: &Path, requested_hdu: Option<&str>) -> Result<Self> {
        let fits = FitsFile::open(path)?;
        let hdu_index = fits.resolve_hdu(requested_hdu)?;
        let widget = Self::load_viewer(&fits, hdu_index)?;

        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());

        Ok(Self {
            fits,
            hdu_index,
            widget,
            parked: HashMap::new(),
            title,
            error: None,
        })
    }

    /// A viewer showing `hdu_index`'s first plane, with its cube axes declared.
    /// Reading that plane here is what lets both opening a tab and switching
    /// HDU either arrive with an image on screen or fail having changed
    /// nothing.
    fn load_viewer(fits: &FitsFile, hdu_index: usize) -> Result<ArrayViewerWidget> {
        let mut widget = ArrayViewerWidget::new();

        let hdu = &fits.hdus[hdu_index];
        let dims = hdu.slice_dims();
        let indices = vec![0usize; dims.len()];
        widget.set_cube(dims);

        let pixels = fits.read_plane(hdu, &indices)?;
        widget.receive_slice(
            indices,
            pixels,
            hdu.width() as u32,
            hdu.height() as u32,
            hdu.is_integer(),
            hdu.value_decimals(),
        );

        Ok(widget)
    }

    /// Label for this tab in the tab bar.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Full path, shown on hover where the short title is ambiguous.
    pub fn path_string(&self) -> String {
        self.fits.path.display().to_string()
    }

    /// The HDU's name, as `EXTNAME` gives it (or `PRIMARY`).
    pub fn hdu_name(&self) -> &str {
        &self.fits.hdus[self.hdu_index].name
    }

    /// Which HDU of the file this tab is showing.
    pub fn hdu_index(&self) -> usize {
        self.hdu_index
    }

    /// Whether this file holds enough HDUs to be worth offering a choice at
    /// all. A single-HDU file gets a plain label instead of a picker.
    pub fn has_hdu_choice(&self) -> bool {
        self.fits.hdus.len() > 1
    }

    /// Every HDU in the file, in file order. The ones this reader cannot
    /// display are listed too, carrying their reason, so a file whose image
    /// sits in a compressed extension explains itself rather than looking
    /// like it holds nothing.
    pub fn hdu_choices(&self) -> Vec<HduChoice> {
        self.fits
            .hdus
            .iter()
            .map(|hdu| HduChoice {
                index: hdu.index,
                label: format!(
                    "[{}] {}  {}  {}",
                    hdu.index,
                    hdu.name,
                    hdu.shape_label(),
                    hdu.bitpix.name()
                ),
                unviewable_reason: hdu.unviewable_reason(),
            })
            .collect()
    }

    /// Show a different HDU of the same file. The outgoing viewer is parked
    /// rather than dropped, so switching away and back is free and lands on
    /// the view you left. A plane that will not read leaves the tab on the HDU
    /// it was already showing, with the reason surfaced in the window.
    pub fn set_hdu(&mut self, index: usize) {
        if index == self.hdu_index || index >= self.fits.hdus.len() {
            return;
        }

        // The picker greys these out, but this is the API an HDU arrives
        // through, so it does not get to depend on the UI having done that.
        if let Some(reason) = self.fits.hdus[index].unviewable_reason() {
            self.error = Some(format!(
                "cannot show HDU {index} ({}): {reason}",
                self.fits.hdus[index].name
            ));
            return;
        }

        // Reclaim or build the replacement before touching any state, so a
        // failed read is a no-op rather than a half-switched tab.
        let widget = match self.parked.remove(&index) {
            Some(widget) => widget,
            None => match Self::load_viewer(&self.fits, index) {
                Ok(widget) => widget,
                Err(err) => {
                    let message = format!("could not open HDU {index}: {err:#}");
                    eprintln!("viewarr: {message}");
                    self.error = Some(message);
                    return;
                }
            },
        };

        let previous = std::mem::replace(&mut self.widget, widget);
        self.parked.insert(self.hdu_index, previous);
        self.hdu_index = index;
        self.error = None;
    }

    /// Shape in FITS axis order, e.g. `200x120x8`.
    pub fn shape_string(&self) -> String {
        self.fits.hdus[self.hdu_index].shape_string()
    }

    /// Storage type name, e.g. `f32`.
    pub fn dtype_name(&self) -> &'static str {
        self.fits.hdus[self.hdu_index].bitpix.name()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether this file has sliceable axes needing the slice-control bar.
    pub fn has_slices(&self) -> bool {
        self.widget.has_slices()
    }

    pub fn show_slice_controls(&mut self, ui: &mut egui::Ui) {
        self.widget.show_slice_controls(ui);
    }

    pub fn show_viewer(&mut self, ui: &mut egui::Ui) {
        let container_size = ui.available_size();
        self.widget.show(ui, container_size);
    }

    /// The image's own size, used to pick a sensible initial window size.
    pub fn image_size(&self) -> (usize, usize) {
        let hdu = &self.fits.hdus[self.hdu_index];
        (hdu.width(), hdu.height())
    }

    /// How many sliceable axes this file has, i.e. how many rows the slice bar
    /// will occupy.
    pub fn slice_axis_count(&self) -> usize {
        self.fits.hdus[self.hdu_index].slice_dims().len()
    }

    /// Fulfil whatever slice the widget is asking for. Reads are from a mapped
    /// file and so complete in-frame; the loop converges because the widget
    /// coalesces to at most one outstanding request, and the bound just
    /// guarantees we always hand control back to the frame.
    pub fn serve_slice_requests(&mut self) {
        for _ in 0..8 {
            let Some(indices) = self.widget.take_slice_request() else {
                return;
            };

            let (result, width, height, is_integer, decimals) = {
                let hdu = &self.fits.hdus[self.hdu_index];
                (
                    self.fits.read_plane(hdu, &indices),
                    hdu.width() as u32,
                    hdu.height() as u32,
                    hdu.is_integer(),
                    hdu.value_decimals(),
                )
            };

            match result {
                Ok(pixels) => {
                    self.error = None;
                    self.widget
                        .receive_slice(indices, pixels, width, height, is_integer, decimals);
                }
                Err(err) => {
                    // Tell the widget the request died, or its coalescing would
                    // wait forever on a reply that is never coming and the
                    // scrubber would stop responding. The previous plane stays
                    // on screen; we just surface why the new one never arrived.
                    let message = format!("could not read slice {indices:?}: {err:#}");
                    eprintln!("viewarr: {message}");
                    self.widget.fail_slice_request(&indices);
                    self.error = Some(message);
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use viewarr::colormap::Colormap;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(name)
    }

    /// `mef.fits` is a dataless PRIMARY followed by SCI (f64) and ERR (f32) —
    /// the shape that makes a picker worth having, and one where every HDU is
    /// distinguishable by dtype.
    fn mef() -> Tab {
        Tab::open(&fixture("mef.fits"), None).expect("the fixture should open")
    }

    #[test]
    fn a_tab_opens_on_the_first_viewable_hdu_not_the_dataless_primary() {
        let tab = mef();
        assert_eq!(tab.hdu_index(), 1);
        assert_eq!(tab.hdu_name(), "SCI");
    }

    #[test]
    fn switching_hdu_changes_what_the_tab_shows() {
        let mut tab = mef();

        tab.set_hdu(2);
        assert_eq!(tab.hdu_index(), 2);
        assert_eq!(tab.hdu_name(), "ERR");
        assert_eq!(tab.dtype_name(), "f32", "the info bar follows the HDU");
        assert!(tab.error().is_none());

        tab.set_hdu(1);
        assert_eq!(tab.hdu_name(), "SCI");
        assert_eq!(tab.dtype_name(), "f64");
    }

    #[test]
    fn each_hdu_keeps_its_own_view_state_across_a_switch() {
        let mut tab = mef();

        // Leave a mark on SCI's viewer that ERR's should not inherit...
        tab.widget.set_colormap(Colormap::Inferno);

        tab.set_hdu(2);
        assert_eq!(
            tab.widget.colormap(),
            Colormap::Grayscale,
            "a freshly visited HDU starts from the default, not the last HDU's state"
        );
        tab.widget.set_colormap(Colormap::Magma);

        // ...and both marks should still be there on the way back.
        tab.set_hdu(1);
        assert_eq!(tab.widget.colormap(), Colormap::Inferno);
        tab.set_hdu(2);
        assert_eq!(tab.widget.colormap(), Colormap::Magma);
    }

    #[test]
    fn a_revisited_hdu_is_reclaimed_rather_than_rebuilt() {
        let mut tab = mef();
        assert!(tab.parked.is_empty(), "nothing is parked before a switch");

        tab.set_hdu(2);
        assert_eq!(
            tab.parked.keys().collect::<Vec<_>>(),
            vec![&1],
            "the outgoing viewer is parked, not dropped"
        );

        tab.set_hdu(1);
        assert_eq!(tab.parked.keys().collect::<Vec<_>>(), vec![&2]);
    }

    #[test]
    fn switching_to_an_unviewable_hdu_is_refused_without_disturbing_the_tab() {
        let mut tab = mef();

        // HDU 0 is the dataless primary; it has nothing to show.
        tab.set_hdu(0);

        assert_eq!(tab.hdu_index(), 1, "the tab stays on the HDU it had");
        assert_eq!(tab.hdu_name(), "SCI");
        let error = tab.error().expect("the refusal is surfaced in the window");
        assert!(
            error.contains("no 2-D-or-larger image data"),
            "the reason should survive to the UI: {error}"
        );

        // A good switch afterwards clears it.
        tab.set_hdu(2);
        assert!(tab.error().is_none());
    }

    #[test]
    fn switching_nowhere_is_a_no_op() {
        let mut tab = mef();

        tab.set_hdu(1); // already there
        tab.set_hdu(99); // past the end
        assert_eq!(tab.hdu_index(), 1);
        assert!(tab.parked.is_empty());
        assert!(
            tab.error().is_none(),
            "neither is a failure worth reporting"
        );
    }

    #[test]
    fn hdu_choices_list_every_hdu_with_the_reason_it_cannot_be_shown() {
        let tab = mef();
        let choices = tab.hdu_choices();

        assert_eq!(choices.len(), 3, "unviewable HDUs are listed, not hidden");
        assert_eq!(
            choices[0].unviewable_reason,
            Some("no 2-D-or-larger image data")
        );
        assert!(choices[1].unviewable_reason.is_none());
        assert!(choices[2].unviewable_reason.is_none());

        // The label is what the picker shows, so it should name the HDU and
        // say enough to tell two extensions apart.
        assert!(choices[1].label.contains("SCI"), "{}", choices[1].label);
        assert!(choices[1].label.contains("4x3"), "{}", choices[1].label);
        assert!(choices[1].label.contains("f64"), "{}", choices[1].label);
        assert!(choices[0].label.contains("no data"), "{}", choices[0].label);
    }

    #[test]
    fn a_single_hdu_file_offers_no_choice() {
        let tab = Tab::open(&fixture("cube3d.fits"), None).unwrap();
        assert!(!tab.has_hdu_choice());
        assert!(mef().has_hdu_choice());
    }

    #[test]
    fn a_compressed_extension_is_listed_with_its_reason() {
        // This file cannot be opened at all (its only image is compressed), so
        // the picker's reason text is checked through the reader instead.
        let fits = crate::fits::FitsFile::open(&fixture("compressed.fits")).unwrap();
        assert_eq!(
            fits.hdus[1].unviewable_reason(),
            Some("tile-compressed, not yet supported")
        );
    }
}
