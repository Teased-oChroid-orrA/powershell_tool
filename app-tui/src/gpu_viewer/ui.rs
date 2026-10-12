//! Data the window chrome shows, supplied by the viewer and by authoring sessions: feature-tree nodes,
//! ribbon tabs and property rows. Plain data, no drawing: the egui front end (`gui`) renders and routes it.
use super::{controller::UiAction, editor::session::Tone, menu::Entry};

/// One row of the feature tree.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    /// Stable within one tree; the expand/collapse state is keyed by it.
    pub id: u32,
    pub depth: u8,
    pub label: String,
    /// One character that says what kind of node this is (shown as a small coloured marker).
    pub glyph: char,
    /// The node owns the following deeper nodes and can be collapsed.
    pub collapsible: bool,
    pub selected: bool,
    pub tone: Tone,
    /// Click action (select the feature, show its properties).
    pub action: Option<UiAction>,
}
impl TreeNode {
    pub fn new(id: u32, depth: u8, glyph: char, label: impl Into<String>) -> Self {
        Self { id, depth, label: label.into(), glyph, collapsible: false, selected: false, tone: Tone::Normal, action: None }
    }
    pub fn folder(mut self) -> Self {
        self.collapsible = true;
        self
    }
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
    pub fn on_click(mut self, action: UiAction) -> Self {
        self.action = Some(action);
        self
    }
}

/// Nodes hidden because an ancestor folder in `collapsed` is closed.
pub fn visible_nodes<'a>(nodes: &'a [TreeNode], collapsed: &std::collections::HashSet<u32>) -> Vec<&'a TreeNode> {
    let mut out = Vec::new();
    let mut hide_below: Option<u8> = None;
    for n in nodes {
        if let Some(d) = hide_below {
            if n.depth > d {
                continue;
            }
            hide_below = None;
        }
        out.push(n);
        if n.collapsible && collapsed.contains(&n.id) {
            hide_below = Some(n.depth);
        }
    }
    out
}

/// A titled group of ribbon buttons.
#[derive(Clone, Debug, PartialEq)]
pub struct RibbonGroup {
    pub title: String,
    pub buttons: Vec<Entry>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RibbonTab {
    pub title: String,
    pub groups: Vec<RibbonGroup>,
}
impl RibbonTab {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), groups: Vec::new() }
    }
    pub fn group(mut self, title: impl Into<String>, buttons: Vec<Entry>) -> Self {
        self.groups.push(RibbonGroup { title: title.into(), buttons });
        self
    }
}

/// One line of a dock (property manager).
#[derive(Clone, Debug, PartialEq)]
pub enum DockItem {
    Heading(String),
    Line { text: String, tone: Tone, action: Option<UiAction>, focused: bool },
    /// A label with its value; a click (when `action` is set) edits it.
    Field { label: String, value: String, tone: Tone, action: Option<UiAction>, focused: bool },
    Gap,
    /// Contour colour bar with its value range.
    Legend { lo: f32, hi: f32 },
}
impl DockItem {
    pub fn text(text: impl Into<String>, tone: Tone) -> Self {
        DockItem::Line { text: text.into(), tone, action: None, focused: false }
    }
    pub fn row(text: impl Into<String>, tone: Tone, action: Option<UiAction>, focused: bool) -> Self {
        DockItem::Line { text: text.into(), tone, action, focused }
    }
    /// The row as plain text (labels and values joined), for tests and logs.
    pub fn plain_text(&self) -> Option<String> {
        match self {
            DockItem::Line { text, .. } => Some(text.clone()),
            DockItem::Field { label, value, .. } => Some(format!("{label}  {value}")),
            _ => None,
        }
    }
    pub fn field(label: impl Into<String>, value: impl Into<String>, tone: Tone, action: Option<UiAction>, focused: bool) -> Self {
        DockItem::Field { label: label.into(), value: value.into(), tone, action, focused }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn tree() -> Vec<TreeNode> {
        vec![
            TreeNode::new(1, 0, '+', "Geometry").folder(),
            TreeNode::new(2, 1, 'o', "Outer"),
            TreeNode::new(3, 1, 'o', "Hole 1"),
            TreeNode::new(4, 0, '+', "Loads").folder(),
            TreeNode::new(5, 1, '>', "Traction"),
            TreeNode::new(6, 0, '*', "Results"),
        ]
    }
    fn ids(v: &[&TreeNode]) -> Vec<u32> {
        v.iter().map(|n| n.id).collect()
    }

    #[test]
    fn collapsing_a_folder_hides_exactly_its_descendants() {
        let t = tree();
        assert_eq!(ids(&visible_nodes(&t, &HashSet::new())), [1, 2, 3, 4, 5, 6]);
        assert_eq!(ids(&visible_nodes(&t, &HashSet::from([1]))), [1, 4, 5, 6]);
        assert_eq!(ids(&visible_nodes(&t, &HashSet::from([4]))), [1, 2, 3, 4, 6]);
        assert_eq!(ids(&visible_nodes(&t, &HashSet::from([1, 4]))), [1, 4, 6]);
        // Collapsing a leaf (not a folder) does nothing.
        assert_eq!(ids(&visible_nodes(&t, &HashSet::from([6, 2]))), [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn nested_folders_collapse_with_their_parent() {
        let t = vec![
            TreeNode::new(1, 0, '+', "A").folder(),
            TreeNode::new(2, 1, '+', "B").folder(),
            TreeNode::new(3, 2, 'o', "C"),
            TreeNode::new(4, 1, 'o', "D"),
            TreeNode::new(5, 0, 'o', "E"),
        ];
        assert_eq!(ids(&visible_nodes(&t, &HashSet::from([2]))), [1, 2, 4, 5]);
        assert_eq!(ids(&visible_nodes(&t, &HashSet::from([1]))), [1, 5]);
    }
}
