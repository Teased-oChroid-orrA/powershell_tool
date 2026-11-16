//! Data for a workbench's page: the pieces the Pressure Vessel window draws over the viewport (case pills, the supported
//! line drawn to scale with its margin curve, the "Minimum wall by requirement" table). Plain data like `ui`; `gui/page.rs`
//! draws it and reports clicks as the `action` of the piece clicked.
use super::{controller::UiAction, editor::session::Tone};

/// A rounded chip: a state when `action` is `None`, a button or a choice (`selected`) when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Pill {
    pub text: String,
    pub tone: Tone,
    pub action: Option<UiAction>,
    pub selected: bool,
}
impl Pill {
    pub fn new(text: impl Into<String>, tone: Tone, action: Option<UiAction>, selected: bool) -> Self {
        Self { text: text.into(), tone, action, selected }
    }
}

/// A supported line to scale: x from 0 to `length`, the tube as a bar, supports under it, loads above it, and the
/// margin curve along x (zero marked) with its worst point.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LineDrawing {
    pub length: f64,
    pub supports: Vec<(f64, String)>,
    /// Position, label and the load's text (a force, a weight).
    pub loads: Vec<(f64, String, String)>,
    /// (x, margin above the requirement); zero is "just met".
    pub curve: Vec<[f64; 2]>,
    pub worst: Option<[f64; 2]>,
    pub curve_label: String,
    /// A reason there is no curve (the solve is running, the run failed).
    pub note: String,
}

/// One cell of a table row.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    /// Text; a click (when `action` is set) edits or cycles it, and `editing` draws it as an edit box.
    Text { text: String, tone: Tone, action: Option<UiAction>, editing: bool },
    /// A wall: the number, a bar from zero to `value` against the scale `max`, and a tick at the current wall.
    Wall { text: String, value: f64, current: f64, max: f64, tone: Tone },
    /// A met / short state.
    Chip { text: String, tone: Tone },
    /// A small button (remove the row).
    Button { text: String, action: UiAction },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub title: String,
    /// Relative width.
    pub weight: f32,
    /// Left out when the table is narrow.
    pub optional: bool,
    pub right: bool,
}
impl Column {
    pub fn new(title: &str, weight: f32) -> Self {
        Self { title: title.into(), weight, optional: false, right: false }
    }
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
    pub fn right(mut self) -> Self {
        self.right = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    pub cells: Vec<Cell>,
    /// Tinted: the row that sets the wall.
    pub governing: bool,
}

/// A table with fixed row heights, a footer line and an add row.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DockTable {
    pub columns: Vec<Column>,
    pub rows: Vec<TableRow>,
    pub add: Option<(String, UiAction)>,
    pub footer: String,
    pub footer_tone: Option<Tone>,
    /// A dim line under the table.
    pub hint: String,
}
