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
    /// `hover`: `(on, off)` actions sent when the pointer enters / leaves the row (a preview of what a click would do).
    Line { text: String, tone: Tone, action: Option<UiAction>, focused: bool, hover: Option<(UiAction, UiAction)> },
    /// A label with its value; a click (when `action` is set) edits it.
    Field { label: String, value: String, tone: Tone, action: Option<UiAction>, focused: bool },
    Gap,
    /// Contour colour bar with its value range.
    Legend { lo: f32, hi: f32 },
}
impl DockItem {
    pub fn text(text: impl Into<String>, tone: Tone) -> Self {
        DockItem::Line { text: text.into(), tone, action: None, focused: false, hover: None }
    }
    pub fn row(text: impl Into<String>, tone: Tone, action: Option<UiAction>, focused: bool) -> Self {
        DockItem::Line { text: text.into(), tone, action, focused, hover: None }
    }
    /// A clickable row that also previews its effect while the pointer is on it (`on`), and stops when it leaves (`off`).
    pub fn row_with_preview(text: impl Into<String>, tone: Tone, action: UiAction, on: UiAction, off: UiAction) -> Self {
        DockItem::Line { text: text.into(), tone, action: Some(action), focused: false, hover: Some((on, off)) }
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

/// One row of a value card.
#[derive(Clone, Debug, PartialEq)]
pub enum CardRow {
    /// A dropdown that dispatches the selected typed command.
    Select { label: String, selected: String, options: Vec<(String, super::controller::UiAction)> },
    /// A switch (a held direction, a support kept in a case).
    Toggle { label: String, on: bool, action: super::controller::UiAction },
    /// A text field; edits are reported with `index` (`Authoring::card_field`).
    Field { label: String, text: String, index: usize, unit: String },
    /// A line of explanation.
    Text(String),
    /// A pickable line (a material of the library): `label`, a dim `detail`, and what choosing it does.
    Choice { label: String, detail: String, action: super::controller::UiAction, current: bool },
}

/// A small form the window shows next to the face it concerns: a constraint's directions, a load's numbers, a
/// load case's switches. `apply` runs on the button or Enter, `cancel` on the button or Esc.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueCard {
    pub title: String,
    /// Width in points.
    pub width: f32,
    /// Small caption beside the title (the Nastran card the form writes).
    pub tag: String,
    pub rows: Vec<CardRow>,
    pub apply_label: String,
    pub apply: super::controller::UiAction,
    pub cancel: super::controller::UiAction,
}

/// What a bench without a 3D model (Search) shows where the model would be: a bar of text inputs and buttons,
/// quick switches, a notice, live progress, a few figures, a table and the detail of its selected row. Plain data
/// like the rest of the chrome; `gui/document.rs` draws it and reports edits as `Outcome::DocField`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Document {
    pub precision: Option<super::editor::stackup::PrecisionPage>,
    pub policy: Option<DocTable>,
    pub inputs: Vec<DocInput>,
    /// A second row of inputs under the first, the bar's full width (Exclude, while its switch is on).
    pub extra_inputs: Vec<DocInput>,
    /// Beside the inputs; the first is the main action (drawn in the accent colour).
    pub buttons: Vec<Entry>,
    /// Switches under the bar (`Entry::checked`).
    pub chips: Vec<Entry>,
    pub notice: Option<DocNotice>,
    pub progress: Option<DocProgress>,
    /// Figures shown as small tiles: label and value.
    pub stats: Vec<(String, String)>,
    pub table: DocTable,
    pub detail: Option<DocDetail>,
    /// Buttons in the bar that open a popover of option cards (Options, Index, Report): the page has no docks, so the
    /// settings take room only while they are open.
    pub panels: Vec<DocPanel>,
    /// A card the page must answer before going on (the file types to search), centred over a dimmed page.
    pub modal: Option<DocModal>,
}

/// A bar button and the cards its popover shows.
#[derive(Clone, Debug, PartialEq)]
pub struct DocPanel {
    pub label: String,
    /// A short state shown on the button (the index is on).
    pub badge: Option<String>,
    /// Text inputs drawn above the cards (the output folder, with its Browse button).
    pub inputs: Vec<DocInput>,
    pub cards: Vec<crate::gpu_viewer::editor::authoring::Card>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DocModal {
    pub card: crate::gpu_viewer::editor::authoring::Card,
    /// What closing it (Esc, a click outside, Done) does.
    pub close: UiAction,
}

/// How to write what goes into an input, opened from a link beside its label. A row with an `example` puts that
/// text into the input when clicked.
#[derive(Clone, Debug, PartialEq)]
pub struct DocHelp {
    pub link: String,
    pub title: String,
    pub intro: String,
    pub sections: Vec<HelpSection>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HelpSection {
    pub title: String,
    pub rows: Vec<HelpRow>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HelpRow {
    pub code: String,
    pub text: String,
    pub example: Option<String>,
}

/// A text input of a document. Each edit is reported with `id`; Enter runs `submit`.
#[derive(Clone, Debug, PartialEq)]
pub struct DocInput {
    pub id: u8,
    pub label: String,
    pub text: String,
    pub hint: String,
    /// Shown under the input in the warning colour (an invalid regex).
    pub error: Option<String>,
    /// A line under the input that explains what it will do (the parsed filter groups).
    pub note: Option<String>,
    pub submit: Option<UiAction>,
    /// Share of the bar's width (inputs split the room left by the buttons in these proportions).
    pub grow: f32,
    /// A button drawn inside the input's right end (Browse).
    pub trailing: Option<Entry>,
    /// A guide to what can be typed, behind a link beside the label.
    pub help: Option<DocHelp>,
    /// A segmented choice right of the input (how an exclusion applies); the checked entry is the current one.
    pub choices: Vec<Entry>,
}

/// A banner over the table with its actions (the fast index is out of date: Update / Not now).
#[derive(Clone, Debug, PartialEq)]
pub struct DocNotice {
    pub text: String,
    pub tone: Tone,
    pub actions: Vec<Entry>,
}

/// A computation in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct DocProgress {
    /// `None`: not measurable yet (scanning folders); the bar then moves without a value.
    pub fraction: Option<f32>,
    pub headline: String,
    /// Rate, time left and elapsed.
    pub detail: String,
    /// What is being worked on right now: name and its own status.
    pub active: Vec<(String, String)>,
    pub cancel: Option<Entry>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DocTable {
    pub title: String,
    /// A filter over the rows, drawn in the table's header.
    pub filter: Option<DocInput>,
    pub columns: Vec<DocColumn>,
    pub rows: Vec<DocRow>,
    /// Shown instead of the rows when there are none.
    pub empty_title: String,
    pub empty_body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DocColumn {
    pub title: String,
    /// Width in points; 0 takes what the other columns leave.
    pub width: f32,
    /// Right-aligned (numbers).
    pub right: bool,
    /// A click on the header sorts by this column.
    pub sort: Option<UiAction>,
    /// The table is sorted by this column: `Some(true)` ascending.
    pub sorted: Option<bool>,
}

/// One row: its cells (the first may have a dim second line, `sub`), what a click and a double click do.
#[derive(Clone, Debug, PartialEq)]
pub struct DocRow {
    pub cells: Vec<String>,
    pub sub: String,
    pub selected: bool,
    pub select: UiAction,
    pub open: Option<UiAction>,
    /// A small badge after the first cell (the file type).
    pub badge: Option<String>,
}

/// The selected row in full: a title, a subtitle, actions and the text blocks that matter (the matching lines).
#[derive(Clone, Debug, PartialEq)]
pub struct DocDetail {
    pub drawing: Option<DocDrawing>,
    pub title: String,
    pub subtitle: String,
    pub actions: Vec<Entry>,
    pub blocks: Vec<DocBlock>,
    /// Shown when there are no blocks.
    pub empty: String,
}

/// A numbered line with optional context lines around it; `marks` are byte ranges of `text` to highlight.
#[derive(Clone, Debug, PartialEq)]
pub struct DocBlock {
    pub number: i32,
    pub before: Option<String>,
    pub text: String,
    pub after: Option<String>,
    pub marks: Vec<(usize, usize)>,
    /// Small tags beside the number (the filters that matched).
    pub tags: Vec<String>,
}

/// Engineering-coordinate section, transformed once by the native painter.
#[derive(Clone, Debug, PartialEq)]
pub struct DocDrawing { pub bounds: [[f64;2];2], pub paths: Vec<DocPath> }
#[derive(Clone, Debug, PartialEq)]
pub struct DocPath { pub points: Vec<[f64;2]>, pub label: String, pub kind: u8, pub selected: bool, pub select: Option<UiAction> }
