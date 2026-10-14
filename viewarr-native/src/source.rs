//! A format-agnostic model of what a source holds, for the array picker.
//!
//! FITS hands back a flat HDU chain; a Zarr store would hand back a tree of
//! groups and arrays. Both are [`Tree`]s here, so the picker renders either
//! from one model and a new source means producing [`Node`]s — never drawing
//! UI. That split is not cosmetic: the JupyterLab front-end draws the same
//! choice in HTML, so a source that owned its own widget would be unusable
//! there.
//!
//! This lives in `viewarr-native` while FITS is the only source. It moves to a
//! shared crate the moment a second front-end wants to read it.

use std::fmt;

/// Identity of one array within its source, as a `/`-separated path.
///
/// FITS uses the HDU's position (`"1"`) rather than its `EXTNAME`, because
/// position is what actually addresses an HDU and `EXTNAME` need not be
/// unique. A Zarr store would use the array's path inside it,
/// `"calib/darks/master"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArrayId(String);

impl ArrayId {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// The identity a FITS HDU gets. The one place the encoding is spelled
    /// out, so resolving an id back to an HDU can just compare against it.
    pub fn for_hdu(index: usize) -> Self {
        Self(index.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Path segments, outermost first.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for ArrayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One array a source can offer, whether or not this build can display it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrayMeta {
    pub id: ArrayId,
    /// How the source names it, e.g. `[1] SCI`.
    pub name: String,
    /// Shape and type, shown after the name: `4x3  f64`.
    pub detail: String,
    /// Why it cannot be shown, or `None` if it can be selected.
    pub unviewable_reason: Option<&'static str>,
}

impl ArrayMeta {
    /// One-line label, for a picker showing a flat list.
    pub fn label(&self) -> String {
        if self.detail.is_empty() {
            self.name.clone()
        } else {
            format!("{}  {}", self.name, self.detail)
        }
    }

    pub fn is_selectable(&self) -> bool {
        self.unviewable_reason.is_none()
    }
}

/// A node in a source's tree: an array, or a group holding more nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Array(ArrayMeta),
    Group { name: String, children: Children },
}

/// What a group contains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Children {
    /// Already enumerated. The only case FITS produces: one header scan yields
    /// the whole chain, so there is nothing to defer.
    Loaded(Vec<Node>),
    /// Known to exist but not fetched. A remote store with many groups cannot
    /// be walked up front without a request storm, so the picker shows these
    /// collapsed rather than blocking on them. No source produces this yet —
    /// the variant is here so that adding one does not have to reshape this
    /// type, every match on it, and the picker all at once.
    Unexpanded,
}

/// Everything one source holds, as a forest (a FITS file's HDUs are all
/// top-level, so its forest is one flat row of [`Node::Array`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    nodes: Vec<Node>,
    noun: &'static str,
}

impl Tree {
    /// `noun` is what this source calls one of these, singular and lowercase
    /// unless it is an acronym: `"HDU"` for FITS, `"array"` for Zarr. UI that
    /// has to name the choice asks for it rather than guessing, so a generic
    /// picker can still speak the format's own language.
    pub fn new(nodes: Vec<Node>, noun: &'static str) -> Self {
        Self { nodes, noun }
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn noun(&self) -> &'static str {
        self.noun
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Whether this is a plain one-level list of arrays, i.e. whether the
    /// picker can use a flat menu rather than nested submenus. Presentation is
    /// the picker's call, taken from the shape of the tree rather than from
    /// which source produced it.
    pub fn is_flat(&self) -> bool {
        self.nodes.iter().all(|node| matches!(node, Node::Array(_)))
    }

    /// Every array in the forest, depth-first. Unexpanded groups contribute
    /// nothing, since by definition their contents are not known yet.
    pub fn arrays(&self) -> Vec<&ArrayMeta> {
        let mut found = Vec::new();
        collect(&self.nodes, &mut found);
        found
    }

    pub fn find(&self, id: &ArrayId) -> Option<&ArrayMeta> {
        self.arrays().into_iter().find(|meta| meta.id == *id)
    }

    /// How many arrays this build could actually display. The picker is only
    /// worth showing when there is more than one thing to choose between.
    pub fn selectable_count(&self) -> usize {
        self.arrays()
            .iter()
            .filter(|meta| meta.is_selectable())
            .count()
    }
}

fn collect<'a>(nodes: &'a [Node], out: &mut Vec<&'a ArrayMeta>) {
    for node in nodes {
        match node {
            Node::Array(meta) => out.push(meta),
            Node::Group {
                children: Children::Loaded(kids),
                ..
            } => collect(kids, out),
            Node::Group {
                children: Children::Unexpanded,
                ..
            } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn array(id: &str, name: &str) -> Node {
        Node::Array(ArrayMeta {
            id: ArrayId::new(id),
            name: name.to_string(),
            detail: "4x3  f32".to_string(),
            unviewable_reason: None,
        })
    }

    fn tree(nodes: Vec<Node>) -> Tree {
        Tree::new(nodes, "array")
    }

    fn group(name: &str, children: Vec<Node>) -> Node {
        Node::Group {
            name: name.to_string(),
            children: Children::Loaded(children),
        }
    }

    /// A FITS file's shape: everything top-level, nothing nested.
    #[test]
    fn a_flat_forest_is_flat() {
        let tree = tree(vec![array("0", "PRIMARY"), array("1", "SCI")]);
        assert!(tree.is_flat());
        assert_eq!(tree.arrays().len(), 2);
    }

    /// A Zarr store's shape: the picker has to know it cannot use a flat menu.
    #[test]
    fn a_forest_holding_a_group_is_not_flat() {
        let tree = tree(vec![
            array("raw", "raw"),
            group("calib", vec![array("calib/dark", "dark")]),
        ]);
        assert!(!tree.is_flat());
    }

    #[test]
    fn arrays_are_collected_depth_first_through_groups() {
        let tree = tree(vec![
            array("a", "a"),
            group(
                "g",
                vec![array("g/b", "b"), group("g/h", vec![array("g/h/c", "c")])],
            ),
            array("d", "d"),
        ]);
        let ids: Vec<&str> = tree.arrays().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "g/b", "g/h/c", "d"]);
    }

    #[test]
    fn an_unexpanded_group_contributes_nothing_until_it_is_loaded() {
        let tree = tree(vec![
            array("a", "a"),
            Node::Group {
                name: "remote".to_string(),
                children: Children::Unexpanded,
            },
        ]);
        // Not an error and not a guess: its contents are simply not known, so
        // nothing in it can be found or counted yet.
        assert_eq!(tree.arrays().len(), 1);
        assert!(tree.find(&ArrayId::new("remote/anything")).is_none());
        assert!(!tree.is_flat(), "a group means nested presentation");
    }

    #[test]
    fn find_locates_a_nested_array_by_its_path() {
        let tree = tree(vec![group("g", vec![array("g/b", "b")])]);
        assert_eq!(tree.find(&ArrayId::new("g/b")).unwrap().name, "b");
        assert!(tree.find(&ArrayId::new("g/nope")).is_none());
    }

    #[test]
    fn selectable_count_ignores_what_cannot_be_shown() {
        let mut dead = ArrayMeta {
            id: ArrayId::new("2"),
            name: "COMPRESSED".to_string(),
            detail: "6x4  f32".to_string(),
            unviewable_reason: Some("tile-compressed, not yet supported"),
        };
        let with_dead = tree(vec![array("1", "SCI"), Node::Array(dead.clone())]);
        assert_eq!(with_dead.selectable_count(), 1);

        dead.unviewable_reason = None;
        let all_live = tree(vec![array("1", "SCI"), Node::Array(dead)]);
        assert_eq!(all_live.selectable_count(), 2);
    }

    #[test]
    fn a_label_joins_the_name_and_detail_the_way_the_picker_shows_them() {
        let meta = ArrayMeta {
            id: ArrayId::for_hdu(1),
            name: "[1] SCI".to_string(),
            detail: "4x3  f64".to_string(),
            unviewable_reason: None,
        };
        assert_eq!(meta.label(), "[1] SCI  4x3  f64");

        let bare = ArrayMeta {
            detail: String::new(),
            ..meta
        };
        assert_eq!(bare.label(), "[1] SCI", "no detail, no trailing spaces");
    }

    #[test]
    fn a_tree_reports_what_its_source_calls_an_array() {
        // So the picker can say "Which HDU…" for FITS without knowing it is
        // drawing FITS.
        assert_eq!(Tree::new(Vec::new(), "HDU").noun(), "HDU");
    }

    #[test]
    fn an_id_splits_into_path_segments() {
        let id = ArrayId::new("calib/darks/master");
        assert_eq!(
            id.segments().collect::<Vec<_>>(),
            vec!["calib", "darks", "master"]
        );
        // A FITS id is a single segment, which is still a valid path.
        assert_eq!(
            ArrayId::for_hdu(3).segments().collect::<Vec<_>>(),
            vec!["3"]
        );
    }
}
