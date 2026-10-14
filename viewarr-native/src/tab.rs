//! One open file, with its own viewer state.
//!
//! Each tab owns a full `ArrayViewerWidget`, so zoom, stretch, colormap and
//! slice position are per-file and survive switching away and back. A tab
//! showing a multi-extension file keeps one viewer per HDU it has visited, so
//! the same is true of switching HDU within a file.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use viewarr::widget::ArrayViewerWidget;

use crate::fits::{Card, FitsFile};
use crate::header::HeaderWindow;
use crate::source::{ArrayId, ArrayMeta, Node, Tree};

/// Source of the per-tab identity below. Position cannot serve: tabs close,
/// and a window must not start showing a different file because the tab to
/// its left went away.
static NEXT_UID: AtomicU64 = AtomicU64::new(0);

pub struct Tab {
    /// Stable identity, so this tab's header window keeps the same viewport
    /// as other tabs come and go.
    uid: u64,
    /// This tab's own header window. One per tab rather than one that follows
    /// the selection, so two files' headers can sit side by side.
    header: HeaderWindow,
    fits: FitsFile,
    hdu_index: usize,
    widget: ArrayViewerWidget,
    /// Viewers for HDUs this tab showed earlier, kept so switching back
    /// restores that HDU's own zoom, stretch and slice position. This is the
    /// viewer-per-HDU the JupyterLab front-end also keeps, except built on
    /// demand: an HDU never visited costs nothing.
    parked: HashMap<ArrayId, ArrayViewerWidget>,
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
            uid: NEXT_UID.fetch_add(1, Ordering::Relaxed),
            header: HeaderWindow::default(),
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

    /// The active array's metadata, for the header window. A borrow, not a
    /// copy: the header is parsed once when the file is opened and kept, so
    /// the window renders straight from it every frame.
    ///
    /// `None` means this source has no metadata to show at all, as opposed to
    /// an empty header. FITS never returns it — every HDU has a header — but
    /// the window has to be able to say so for sources that do not, and that
    /// is cheaper to carry now than to thread through later.
    pub fn metadata(&self) -> Option<&[Card]> {
        Some(&self.fits.hdus[self.hdu_index].header.cards)
    }

    pub fn header_is_open(&self) -> bool {
        self.header.is_open()
    }

    pub fn toggle_header(&mut self) {
        self.header.toggle();
    }

    /// Draw this tab's header window if it is open. Called for every tab, not
    /// just the active one, so a background tab's window stays up.
    pub fn show_header_window(&mut self, ctx: &egui::Context) {
        if !self.header.is_open() {
            return;
        }
        // Built before the borrows below so they do not overlap the mutable
        // borrow of `header`.
        let hdu = &self.fits.hdus[self.hdu_index];
        let title = format!("{} — {}", self.title, hdu.name);
        let id = egui::ViewportId::from_hash_of(("viewarr-header", self.uid));
        let cards = Some(hdu.header.cards.as_slice());
        self.header.show(ctx, id, &title, cards);
    }

    /// Which array of the source this tab is showing.
    pub fn array_id(&self) -> ArrayId {
        ArrayId::for_hdu(self.hdu_index)
    }

    /// Whether there is enough here to be worth offering a choice at all. A
    /// source holding one array gets a plain label instead of a picker.
    pub fn has_array_choice(&self) -> bool {
        self.fits.hdus.len() > 1
    }

    /// Everything this source holds, as the picker's model. A FITS file is a
    /// flat forest — every HDU is top-level — so a Zarr store's groups would
    /// be the first thing here to nest. The HDUs this reader cannot display
    /// are included, carrying their reason, so a file whose image sits in a
    /// compressed extension explains itself rather than looking empty.
    pub fn arrays(&self) -> Tree {
        let nodes = self
            .fits
            .hdus
            .iter()
            .map(|hdu| {
                Node::Array(ArrayMeta {
                    id: ArrayId::for_hdu(hdu.index),
                    name: format!("[{}] {}", hdu.index, hdu.name),
                    // A storage type means something for an image and
                    // nothing for a table, whose BITPIX is 8 because its
                    // rows are bytes.
                    detail: if hdu.is_viewable_image() {
                        format!("{}  {}", hdu.shape_label(), hdu.bitpix.name())
                    } else {
                        hdu.shape_label()
                    },
                    unviewable_reason: hdu.unviewable_reason(),
                })
            })
            .collect();
        Tree::new(nodes, "HDU")
    }

    /// Resolve a picker identity back to an HDU. Comparing against
    /// `ArrayId::for_hdu` keeps the encoding in that one function rather than
    /// parsing it back out here.
    fn hdu_for(&self, id: &ArrayId) -> Option<usize> {
        (0..self.fits.hdus.len()).find(|&index| ArrayId::for_hdu(index) == *id)
    }

    /// Show a different array of the same source. The outgoing viewer is
    /// parked rather than dropped, so switching away and back is free and
    /// lands on the view you left. A plane that will not read leaves the tab
    /// on what it was already showing, with the reason surfaced in the window.
    pub fn set_array(&mut self, id: &ArrayId) {
        let Some(index) = self.hdu_for(id) else {
            self.error = Some(format!("this file has no array {id}"));
            return;
        };
        if index == self.hdu_index {
            return;
        }

        // The picker greys these out, but this is the API a selection arrives
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
        let widget = match self.parked.remove(id) {
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
        self.parked.insert(self.array_id(), previous);
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

    /// The picker identity of an HDU, which is what `set_array` takes.
    fn hdu(index: usize) -> ArrayId {
        ArrayId::for_hdu(index)
    }

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
        assert_eq!(tab.array_id(), hdu(1));
        assert_eq!(tab.hdu_name(), "SCI");
    }

    #[test]
    fn each_tab_gets_its_own_identity_for_its_header_window() {
        let a = mef();
        let b = mef();
        assert_ne!(
            a.uid, b.uid,
            "two tabs sharing a uid would share one header window"
        );

        // Dropping one leaves the other's identity alone, which is the whole
        // reason this is a counter rather than the tab's position: closing a
        // tab must not make another tab's window start showing a new file.
        let b_uid = b.uid;
        drop(a);
        assert_eq!(b.uid, b_uid);
    }

    #[test]
    fn a_header_window_starts_closed_and_follows_the_selected_hdu() {
        let mut tab = mef();
        assert!(!tab.header_is_open());
        tab.toggle_header();
        assert!(tab.header_is_open());

        // SCI's header, then ERR's: the window belongs to the tab, but shows
        // whichever HDU the picker has selected within it.
        let sci = tab.metadata().expect("FITS always has a header").to_vec();
        tab.set_array(&hdu(2));
        let err = tab.metadata().expect("FITS always has a header");
        assert_ne!(sci, err);
        assert!(err.iter().any(|c| {
            c.key == "EXTNAME"
                && c.value
                    .as_ref()
                    .is_some_and(|v| v.display().contains("ERR"))
        }));

        // Switching HDU does not close the window out from under the user.
        assert!(tab.header_is_open());
    }

    #[test]
    fn switching_hdu_changes_what_the_tab_shows() {
        let mut tab = mef();

        tab.set_array(&hdu(2));
        assert_eq!(tab.array_id(), hdu(2));
        assert_eq!(tab.hdu_name(), "ERR");
        assert_eq!(tab.dtype_name(), "f32", "the info bar follows the HDU");
        assert!(tab.error().is_none());

        tab.set_array(&hdu(1));
        assert_eq!(tab.hdu_name(), "SCI");
        assert_eq!(tab.dtype_name(), "f64");
    }

    #[test]
    fn each_hdu_keeps_its_own_view_state_across_a_switch() {
        let mut tab = mef();

        // Leave a mark on SCI's viewer that ERR's should not inherit...
        tab.widget.set_colormap(Colormap::Inferno);

        tab.set_array(&hdu(2));
        assert_eq!(
            tab.widget.colormap(),
            Colormap::Grayscale,
            "a freshly visited HDU starts from the default, not the last HDU's state"
        );
        tab.widget.set_colormap(Colormap::Magma);

        // ...and both marks should still be there on the way back.
        tab.set_array(&hdu(1));
        assert_eq!(tab.widget.colormap(), Colormap::Inferno);
        tab.set_array(&hdu(2));
        assert_eq!(tab.widget.colormap(), Colormap::Magma);
    }

    #[test]
    fn a_revisited_hdu_is_reclaimed_rather_than_rebuilt() {
        let mut tab = mef();
        assert!(tab.parked.is_empty(), "nothing is parked before a switch");

        tab.set_array(&hdu(2));
        assert_eq!(
            tab.parked.keys().collect::<Vec<_>>(),
            vec![&hdu(1)],
            "the outgoing viewer is parked, not dropped"
        );

        tab.set_array(&hdu(1));
        assert_eq!(tab.parked.keys().collect::<Vec<_>>(), vec![&hdu(2)]);
    }

    #[test]
    fn switching_to_an_unviewable_hdu_is_refused_without_disturbing_the_tab() {
        let mut tab = mef();

        // HDU 0 is the dataless primary; it has nothing to show.
        tab.set_array(&hdu(0));

        assert_eq!(tab.array_id(), hdu(1), "the tab stays on the HDU it had");
        assert_eq!(tab.hdu_name(), "SCI");
        let error = tab.error().expect("the refusal is surfaced in the window");
        assert!(
            error.contains("no 2-D-or-larger image data"),
            "the reason should survive to the UI: {error}"
        );

        // A good switch afterwards clears it.
        tab.set_array(&hdu(2));
        assert!(tab.error().is_none());
    }

    #[test]
    fn switching_nowhere_is_a_no_op() {
        let mut tab = mef();

        tab.set_array(&hdu(1)); // already there
        assert_eq!(tab.array_id(), hdu(1));
        assert!(tab.parked.is_empty());
        assert!(
            tab.error().is_none(),
            "selecting what is shown is not a failure"
        );

        // An id this source does not hold is reported rather than ignored —
        // with a tree model it is a real possibility, not just a bad index.
        tab.set_array(&ArrayId::new("calib/darks/master"));
        assert_eq!(tab.array_id(), hdu(1), "the tab stays where it was");
        assert!(tab
            .error()
            .expect("an unknown array is reported")
            .contains("calib/darks/master"));
    }

    #[test]
    fn the_array_tree_lists_every_hdu_with_the_reason_it_cannot_be_shown() {
        let tab = mef();
        let tree = tab.arrays();
        let arrays = tree.arrays();

        assert!(
            tree.is_flat(),
            "a FITS file has no groups, so the picker stays a flat menu"
        );
        assert_eq!(arrays.len(), 3, "unviewable HDUs are listed, not hidden");
        assert_eq!(
            arrays[0].unviewable_reason,
            Some("no 2-D-or-larger image data")
        );
        assert!(arrays[1].is_selectable());
        assert!(arrays[2].is_selectable());
        assert_eq!(tree.selectable_count(), 2);

        // The label is what the picker shows, so it should name the HDU and
        // say enough to tell two extensions apart.
        let label = arrays[1].label();
        assert!(label.contains("SCI"), "{label}");
        assert!(label.contains("4x3"), "{label}");
        assert!(label.contains("f64"), "{label}");
        assert!(
            arrays[0].label().contains("no data"),
            "{}",
            arrays[0].label()
        );

        // Identity round-trips through the tree, which is what the picker
        // hands back to `set_array`.
        assert_eq!(tree.find(&hdu(2)).unwrap().name, "[2] ERR");
    }

    /// Tables reach the picker, but greyed out and named for what they are:
    /// hiding them would make a file full of them look empty.
    #[test]
    fn the_picker_offers_tables_but_will_not_select_them() {
        let tab = Tab::open(&fixture("table.fits"), None).unwrap();
        let tree = tab.arrays();
        let arrays = tree.arrays();

        assert_eq!(arrays.len(), 4, "every HDU is listed");
        assert_eq!(tree.selectable_count(), 2, "only the two images");

        let find = |name: &str| {
            arrays
                .iter()
                .find(|a| a.name.contains(name))
                .unwrap_or_else(|| panic!("no entry for {name}"))
        };
        assert_eq!(
            find("EVENTS").unviewable_reason,
            Some("a binary table, not an image")
        );
        assert_eq!(
            find("EVENTS").detail,
            "binary table",
            "no bogus shape or dtype"
        );
        assert_eq!(find("CATALOG").detail, "ASCII table");
        assert!(find("SCI").is_selectable());
        assert_eq!(find("SCI").detail, "3x2  f32");

        // And a table cannot be selected even by asking directly.
        let mut tab = tab;
        let events = find("EVENTS").id.clone();
        tab.set_array(&events);
        assert_eq!(tab.array_id(), hdu(0), "the tab stays on the image");
        assert!(tab.error().unwrap().contains("binary table"));
    }

    #[test]
    fn a_single_hdu_file_offers_no_choice() {
        let tab = Tab::open(&fixture("cube3d.fits"), None).unwrap();
        assert!(!tab.has_array_choice());
        assert!(mef().has_array_choice());
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
