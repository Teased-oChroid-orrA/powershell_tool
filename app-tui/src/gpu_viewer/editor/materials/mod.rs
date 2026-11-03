//! The Material Lookup workbench in the native window: the shared state (`toolboxes::material_lookup`:
//! `MaterialLookupState` over `model::{catalog, groups, Query, filter_sort, compare_rows, report_text}`) behind the
//! window chrome. Like Search it is a page bench (`Authoring::document`): a search bar with the basis choice, the
//! results table sorted by a click on a column, and a card with the selected material's properties or the side-by-side
//! comparison of the marked ones. Nothing here filters, sorts or formats a value the toolbox does not.
mod view;
#[cfg(test)]
mod tests;

use super::{
    authoring::{Authoring, Card, StepState},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
};
use crate::gpu_viewer::{
    bench::Bench,
    menu::{Item, Menu},
    ui::{DockItem, Document, RibbonTab, TreeNode},
    Scene,
};
use crate::toolboxes::material_lookup::{
    model::{Basis, Query, SortKey},
    MaterialLookupState,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// The search input of the page.
pub(crate) const INPUT_SEARCH: u8 = 0;

/// What a click in the Material Lookup workbench asks for (carried by `EditorAction::Materials`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialsAct {
    /// Highlight row `.0` of the listed materials.
    Select(usize),
    /// Mark / unmark catalog material `.0` for comparison (a row's double click).
    ToggleMark(usize),
    /// Mark / unmark the highlighted material.
    MarkCurrent,
    ClearMarks,
    /// Show the comparison of the marked materials instead of the highlighted one's properties, or back.
    Compare,
    /// Show only group `.0` of `model::groups()` (0 = All).
    Group(usize),
    Basis(Basis),
    /// Sort by a column; the sorted column again reverses it.
    SortBy(SortKey),
    ClearSearch,
    /// Write the plain-text report (filter, comparison, every listed material) and open it.
    Export,
}

pub(crate) fn act(a: MaterialsAct) -> crate::gpu_viewer::controller::UiAction {
    crate::gpu_viewer::controller::UiAction::Editor(EditorAction::Materials(a))
}

pub struct MaterialsSession {
    st: MaterialLookupState,
    rows_cache: RefCell<Option<view::RowsCache>>,
    /// Where Export writes (`<app data>/reports/material-lookup.txt`).
    report: Option<PathBuf>,
    status: String,
}

impl MaterialsSession {
    /// The window's session; the argument of `--gpu-materials` is not used (the lookup has no file).
    pub fn open(_path: Option<&Path>) -> Result<Self, String> {
        Ok(Self::with_report(crate::paths::app_data_dir().map(|d| d.join("reports").join("material-lookup.txt"))))
    }

    pub(crate) fn with_report(report: Option<PathBuf>) -> Self {
        Self { st: MaterialLookupState::default(), rows_cache: RefCell::new(None), report, status: String::new() }
    }

    pub fn state(&self) -> &MaterialLookupState {
        &self.st
    }

    fn comparing(&self) -> bool {
        self.st.compare && !self.st.marked.is_empty()
    }

    fn act_materials(&mut self, a: MaterialsAct) {
        // A message (Export's) stays until the next thing the person does.
        self.status.clear();
        let st = &mut self.st;
        match a {
            MaterialsAct::Select(row) if row < st.hits.len() => {
                st.cursor = row;
                // A click on a row shows that material, also while comparing.
                st.compare = false;
            }
            MaterialsAct::Select(_) => {}
            MaterialsAct::ToggleMark(i) => {
                if let Some(row) = st.hits.iter().position(|&h| h == i) {
                    st.cursor = row;
                }
                if st.current() == Some(i) {
                    st.toggle_mark();
                } else if let Some(p) = st.marked.iter().position(|&m| m == i) {
                    st.marked.remove(p);
                }
            }
            MaterialsAct::MarkCurrent => st.toggle_mark(),
            MaterialsAct::ClearMarks => {
                st.marked.clear();
                st.compare = false;
            }
            MaterialsAct::Compare => st.compare = !st.compare && !st.marked.is_empty(),
            MaterialsAct::Group(g) if g < crate::toolboxes::material_lookup::model::groups().len() => {
                st.query.group = g;
                st.refilter();
            }
            MaterialsAct::Group(_) => {}
            MaterialsAct::Basis(b) => {
                st.query.basis = b;
                st.refilter();
            }
            MaterialsAct::SortBy(k) => {
                if st.query.sort == k {
                    st.query.descending = !st.query.descending;
                } else {
                    st.query.sort = k;
                    // A property starts with the largest first, a name A-Z.
                    st.query.descending = k != SortKey::Name;
                }
                st.refilter();
            }
            MaterialsAct::ClearSearch => {
                st.query.text.clear();
                st.refilter();
            }
            MaterialsAct::Export => self.export(),
        }
        if self.st.marked.is_empty() {
            self.st.compare = false;
        }
    }

    fn export(&mut self) {
        let Some(path) = self.report.clone() else {
            self.status = "No folder for reports: the app data folder is not available.".into();
            return;
        };
        match crate::persistence::write(&path, self.st.report_text()) {
            Ok(()) => {
                self.status = format!("Report written to {}.", path.display());
                #[cfg(not(test))]
                let _ = open::that(&path);
            }
            Err(e) => self.status = format!("Cannot write the report: {e}"),
        }
    }
}

impl Authoring for MaterialsSession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn path(&self) -> &Path {
        self.report.as_deref().unwrap_or(Path::new("material-lookup.txt"))
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::Materials(a) = action {
            self.act_materials(a);
        }
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        match key {
            Key::Up => self.st.move_cursor(-1),
            Key::Down => self.st.move_cursor(1),
            Key::Enter => self.act_materials(MaterialsAct::MarkCurrent),
            Key::Escape if self.comparing() => self.st.compare = false,
            Key::Escape if !self.st.query.text.is_empty() => self.act_materials(MaterialsAct::ClearSearch),
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'e') => self.act_materials(MaterialsAct::Export),
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        Ok(Scene { label: "materials".into(), positions: vec![[0.0; 3]; 3], triangles: vec![0, 1, 2], samples: vec![[0.0; 4]; 3], times: vec![0.0], range: [0.0, 1.0], gain: 1.0, harmonic_period: None, field_label: String::new(), fields: Vec::new() })
    }
    fn needs_scene(&self) -> bool {
        false
    }
    fn poll(&mut self) -> bool {
        false
    }
    fn prompting(&self) -> bool {
        false
    }
    fn solving(&self) -> bool {
        false
    }
    fn status(&self) -> String {
        if self.status.is_empty() {
            return "Type to search; click a column to sort; double-click a row to mark it for Compare.".into();
        }
        self.status.clone()
    }
    fn set_status(&mut self, text: String) {
        self.status = text;
    }
    fn title(&self) -> String {
        "Material Lookup".into()
    }
    fn dirty(&self) -> bool {
        false
    }
    fn geometry_view(&self) -> bool {
        false
    }
    fn tree(&self) -> Vec<TreeNode> {
        view::tree(self)
    }
    fn properties(&self) -> Vec<DockItem> {
        crate::gpu_viewer::editor::authoring::flatten(self.inspector())
    }
    fn inspector(&self) -> Vec<Card> {
        Vec::new()
    }
    fn focus_step(&self) -> Option<&'static str> {
        Some(if self.comparing() { "Compare" } else { "Browse" })
    }
    fn ribbon(&self) -> Vec<RibbonTab> {
        view::ribbon(self)
    }
    fn step_state(&self, tab: &str) -> Option<StepState> {
        match tab {
            "Browse" => Some(StepState::Done),
            "Compare" => Some(if self.st.marked.len() >= 2 { StepState::Done } else { StepState::Todo }),
            _ => None,
        }
    }
    fn menus(&self) -> Vec<Menu> {
        view::menus(self)
    }
    fn context_menu(&self) -> Vec<Item> {
        view::context_menu(self)
    }
    fn document(&self) -> Option<Document> {
        Some(view::document(self))
    }
    fn has_document(&self) -> bool {
        true
    }
    fn document_field(&mut self, id: u8, text: String) {
        if id == INPUT_SEARCH && text != self.st.query.text {
            self.st.query.text = text;
            // A changed search starts at the best match.
            self.st.refilter_from_top();
        }
    }
}
