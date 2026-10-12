//! Native authoring of meshed solids (`fea_problem::Problem`): the same model, JSON file and row
//! editing as the terminal FEA Workbench (`fea_workbench::model` is the shared editing core), driven
//! from the CAD chrome. A background worker meshes the geometry (preview) and solves; surfaces
//! (edges, holes, faces) are picked in the viewport by their names.
//!
//! Colours of the preview scene use the same scalar-code trick as the member editor: 0 body,
//! 1 support, 2 load, 3 selected / being edited.
use super::{
    camera::Camera,
    session::{Key, KeyResult, Mods},
    worker::Worker,
};
use crate::{
    gpu_viewer::{controller::Command, scene::SceneSource, Scene},
    toolboxes::fea_workbench::model::{self, EditCommit, EditKind, FieldRow},
};
use crate::gpu_viewer::bench::Bench;
use fea_core::Mesh;
use fea_problem::{solve::Field, templates::templates, Geometry, Problem, Solved};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

mod bench_support;
mod sketch;
mod advising;
mod guidance;
mod stepops;
mod inspect;
mod corpus;
mod models;
mod refining;
mod sketching;
mod study;
mod ui;
pub use sketch::SolidTool;
pub use advising::AdvisorOp;
pub use sketching::{FeatureParam, PlaneChoice, SketchAction};
pub use study::{Study, StudyKind};

/// Undo depth.
const UNDO_LIMIT: usize = 100;
/// Pixels within which a click picks a surface.
const PICK_RADIUS: f32 = 10.0;

/// Boundary condition added to the selected surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    Fixed,
    Roller(usize),
    Pressure,
}

/// What the ribbon, menus, tree and property rows ask the solid session to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SolidAction {
    /// Click a property row: focus it and edit or activate it.
    Focus(usize),
    /// Perform a row's action (cycle, add, remove, toggle) without needing it focused.
    Activate(FieldRow),
    /// Focus the first row of a model feature (tree click).
    FocusFeature(Feature),
    /// Select a named surface (tree click).
    SelectSurface(usize),
    ClearSelection,
    /// Give the focused support / load the selected surface.
    AssignSelection,
    /// Remove the entry (hole, support, load, bushing) the focused row belongs to.
    RemoveFocused,
    /// Add a fixed support / a pressure load on the selected surface.
    SupportOnSelected,
    /// Roller on the selected surfaces: only the displacement component is held (the surface slides in the others).
    RollerOnSelected(usize),
    LoadOnSelected,
    Undo,
    Redo,
    Save,
    Load,
    NextTemplate,
    /// File > New: an empty part, sketch open.
    NewPart,
    /// Load template `i` of `fea_problem::templates`.
    OpenTemplate(usize),
    /// Ask for a path and open that problem or mesh file.
    OpenFile,
    /// Remove all geometry, boundary conditions and the feature history.
    ClearModel,
    Solve,
    CancelSolve,
    ShowResults,
    /// Mesh the geometry (the only thing that starts a mesh).
    GenerateMesh,
    /// Apply refinement level `level` (0 removes the zone) to region `index` of the mesh-density hints.
    Refine(usize, u8),
    RefineAll,
    ClearRefine,
    ToggleRefineShow,
    SetTool(SolidTool),
    /// Feature history / Sketch mode commands.
    Sketch(SketchAction),
    /// Apply recommendation `i` of `fea_problem::advice::recommend`.
    Advice(usize),
    /// Apply remedy `.1` of finding `.0` of `fea_problem::diagnose` (failure guidance, ADR-020).
    Remedy(usize, usize),
    /// Ask for the size of a hole / boss / fillet on the STEP solid.
    SolidOp(stepops::PromptOp),
    /// Answer question `.1` of finding `.0` with its option `.2`.
    Answer(usize, usize, usize),
    /// Add this kind of support / load on the selected surfaces.
    BoundaryKind(fea_problem::advice::BcKind),
    Advisor(AdvisorOp),
    RunStudy(StudyKind),
    ShowStudy,
    StudyStep(bool),
}

/// A model feature as the tree lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Problem,
    Material,
    Geometry,
    Hole(usize),
    Mesh,
    Support(usize),
    Load(usize),
    Bushing(usize),
}

/// The feature a property row belongs to.
pub fn feature_of(row: FieldRow) -> Feature {
    use FieldRow::*;
    match row {
        Header(_) | Template | Analysis | Thickness => Feature::Problem,
        Material | Young | Poisson | Alpha | Density | Yield | DeltaT => Feature::Material,
        Source | ImportPath | OuterKind | Outer(_) | PolyPoint(..) | PolyAdd | PolyRemove | HoleAdd | Depth | Layers => Feature::Geometry,
        HoleKind(h) | Hole(h, _) | HoleRemove(h) => Feature::Hole(h),
        Element | MeshSize | HoleFactor | AdaptPasses | TargetError => Feature::Mesh,
        SupportKind(s) | SupportEdge(s) | SupportCoord(s, _) | SupportComp(s, _) | SupportRemove(s) => Feature::Support(s),
        SupportAdd => Feature::Support(usize::MAX),
        LoadKind(l) | LoadEdge(l) | LoadParam(l, _) | LoadRemove(l) => Feature::Load(l),
        LoadAdd => Feature::Load(usize::MAX),
        BushingParam(b, _) | BushingRemove(b) => Feature::Bushing(b),
        BushingAdd => Feature::Bushing(usize::MAX),
    }
}

/// The row that removes the entry `row` belongs to.
pub fn removal_of(row: FieldRow) -> Option<FieldRow> {
    match feature_of(row) {
        Feature::Hole(h) if h != usize::MAX => Some(FieldRow::HoleRemove(h)),
        Feature::Support(s) if s != usize::MAX => Some(FieldRow::SupportRemove(s)),
        Feature::Load(l) if l != usize::MAX => Some(FieldRow::LoadRemove(l)),
        Feature::Bushing(b) if b != usize::MAX => Some(FieldRow::BushingRemove(b)),
        _ => None,
    }
}

/// The part of the inputs the mesh depends on (supports, loads and material values do not).
fn preview_key(p: &Problem) -> Problem {
    Problem { supports: Vec::new(), loads: Vec::new(), delta_t: 0.0, name: String::new(), ..p.clone() }
}

struct Prompt {
    kind: EditKind,
    label: String,
    text: String,
    /// `None`: free text (the material search, or a file path when `open`).
    row: Option<FieldRow>,
    open: bool,
    /// A solid operation waiting for its number(s) (hole diameter, ...); `row` is `None`.
    op: Option<stepops::PromptOp>,
}

struct Preview {
    key: Problem,
    mesh: Arc<Mesh>,
    names: Vec<String>,
}
struct PreviewOut {
    key: Problem,
    mesh: Mesh,
    names: Vec<String>,
}

pub struct SolidSession {
    pub problem: Problem,
    pub path: PathBuf,
    pub focus: usize,
    pub status: String,
    pub dirty: bool,
    pub show_results: bool,
    /// The picked surface name.
    pub selected: Option<String>,
    /// Further selected surfaces (Shift-click, box select); `selected` is the last picked one.
    pub multi: BTreeSet<String>,
    /// A surface click asks for the list of applicable boundary conditions (taken by the window).
    popup: bool,
    prompt: Option<Prompt>,
    saved_geometry: Option<Geometry>,
    /// `(path, text)` of an imported mesh file.
    import: Option<(String, String)>,
    template: usize,
    preview: Option<Preview>,
    preview_error: Option<String>,
    preview_worker: Worker<PreviewOut>,
    solve_worker: Worker<Solved>,
    pub tool: SolidTool,
    pub bench: Bench,
    study_worker: Worker<Study>,
    study: Option<Study>,
    study_index: usize,
    show_study: bool,
    solved: Option<Arc<Solved>>,
    /// Zones the last solve's ZZ error estimate asks for; folded into the model at the next Generate mesh or Solve
    /// (adding them at once would make the result just shown look stale).
    error_zones: Vec<fea_problem::RefineRegion>,
    undo: Vec<Problem>,
    redo: Vec<Problem>,
    /// The user asked for a mesh (Generate mesh); nothing meshes on its own.
    mesh_requested: bool,
    /// Draw the refinement hints as rings in the viewport.
    show_refine: bool,
    version: u64,
    built: Option<u64>,
    next_tag: u64,
    /// Scene-space node positions of the last built preview scene (picking).
    pick_positions: Vec<[f32; 3]>,
    /// Sketch mode (editing one sketch of the feature history).
    sketching: Option<sketching::Sketching>,
    /// A number being typed for a dimension or a feature parameter.
    typing: Option<sketching::Typing>,
    selected_feature: Option<fea_problem::sketch::Id>,
    /// Camera orientation to ask the window for once.
    want_view: Option<crate::gpu_viewer::controller::StandardView>,
    advisors: advising::Advisors,
    /// The solver's message of the last failed solve, until the model changes (failure guidance reads it).
    failure: Option<String>,
    findings_cache: std::cell::RefCell<Option<guidance::Findings>>,
}

fn is_edit_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')
}

impl SolidSession {
    pub fn new(mut problem: Problem, path: PathBuf) -> Self {
        // A file written with a feature history is rebuilt from it, so the geometry always matches the recipe.
        let history_error = problem.apply_history().err();
        let template = templates().iter().position(|(_, p)| p.name == problem.name).unwrap_or(0);
        Self {
            problem,
            path,
            focus: 1,
            status: history_error.map(|e| format!("The feature history cannot be rebuilt: {e}")).unwrap_or_default(),
            dirty: false,
            show_results: false,
            selected: None,
            multi: BTreeSet::new(),
            popup: false,
            prompt: None,
            saved_geometry: None,
            import: None,
            template,
            preview: None,
            preview_error: None,
            preview_worker: Worker::new("solid-preview"),
            solve_worker: Worker::new("solid-solve"),
            tool: SolidTool::Select,
            bench: Bench::Static,
            study_worker: Worker::new("solid-study"),
            study: None,
            study_index: 0,
            show_study: false,
            solved: None,
            error_zones: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            mesh_requested: false,
            show_refine: true,
            version: 1,
            built: None,
            next_tag: 1,
            pick_positions: Vec::new(),
            sketching: None,
            typing: None,
            selected_feature: None,
            want_view: None,
            advisors: advising::Advisors::new(),
            failure: None,
            findings_cache: std::cell::RefCell::new(None),
        }
    }

    // ------------------------------------------------------------------ queries

    pub fn rows(&self) -> Vec<FieldRow> {
        model::field_rows(&self.problem)
    }
    pub fn focused_row(&self) -> Option<FieldRow> {
        self.rows().get(self.focus).copied()
    }
    /// Names supports and loads can refer to (sketch edges, or the imported mesh's surfaces).
    pub fn names(&self) -> Vec<String> {
        match &self.problem.geometry {
            Geometry::Sketch { .. } | Geometry::Solid { .. } => self.problem.edge_names(),
            Geometry::Imported { .. } => self.preview.as_ref().map(|p| p.names.clone()).unwrap_or_default(),
        }
    }
    pub fn import_len(&self) -> Option<usize> {
        let Geometry::Imported { path } = &self.problem.geometry else { return None };
        self.import.as_ref().filter(|(p, _)| p == path).map(|(_, t)| t.len())
    }
    fn import_text(&self) -> Option<String> {
        let Geometry::Imported { path } = &self.problem.geometry else { return None };
        self.import.as_ref().filter(|(p, _)| p == path).map(|(_, t)| t.clone())
    }
    pub fn current_solution(&self) -> Option<&Arc<Solved>> {
        self.solved.as_ref().filter(|s| s.problem == self.problem)
    }
    pub fn prompting(&self) -> bool {
        self.prompt.is_some() || self.typing_active()
    }
    pub fn prompt_text(&self) -> Option<(&str, &str)> {
        self.prompt.as_ref().map(|p| (p.label.as_str(), p.text.as_str()))
    }
    pub fn solving(&self) -> bool {
        self.solve_worker.is_running() || self.study_worker.is_running()
    }
    pub fn solve_elapsed(&self) -> Option<Duration> {
        self.solve_worker.elapsed().or_else(|| self.study_worker.elapsed()).filter(|_| self.solving())
    }
    pub fn meshing(&self) -> bool {
        self.preview_worker.is_running()
    }
    pub fn preview_summary(&self) -> Result<(usize, usize), String> {
        match (&self.preview, &self.preview_error) {
            (Some(p), _) if p.key == preview_key(&self.problem) => Ok((p.mesh.nodes.len(), p.mesh.n_elems())),
            (_, Some(e)) => Err(e.clone()),
            _ if self.meshing() => Err("meshing...".into()),
            (Some(_), _) => Err("out of date (Generate mesh)".into()),
            _ => Err("not generated (Generate)".into()),
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Why a solve cannot start now (`Ok` when it can).
    pub fn solve_availability(&self) -> Result<(), String> {
        if self.sketch_mode() {
            return Err("finish the sketch first: the model is rebuilt from it when you do".into());
        }
        if self.solving() {
            return Err("a solve is already running (Esc cancels it)".into());
        }
        self.problem.validate()?;
        if matches!(self.problem.geometry, Geometry::Imported { .. }) && self.import_text().is_none() {
            return Err("read the mesh file first (Enter on the Mesh File row)".into());
        }
        if self.current_solution().is_some() {
            return Err("results are already current for this model".into());
        }
        Ok(())
    }
    pub fn results_availability(&self) -> Result<(), String> {
        self.current_solution().map(|_| ()).ok_or_else(|| "solve the model first".into())
    }
    pub fn remove_availability(&self) -> Result<(), String> {
        match self.focused_row().and_then(removal_of) {
            Some(_) => Ok(()),
            None => Err("focus a row of a hole, support, load or bushing first".into()),
        }
    }
    pub fn assign_availability(&self) -> Result<(), String> {
        let Some(name) = &self.selected else { return Err("pick a surface in the viewport or the tree first".into()) };
        match self.focused_row().map(feature_of) {
            Some(Feature::Support(s)) if s != usize::MAX && matches!(self.problem.supports.get(s), Some(fea_problem::Support::Edge { .. })) => Ok(()),
            Some(Feature::Load(l)) if l != usize::MAX && self.problem.loads.get(l).is_some_and(|l| l.edge().is_some()) => Ok(()),
            _ => Err(format!("focus a support or load that acts on an edge to assign '{name}' to it")),
        }
    }
    pub fn save_availability(&self) -> Result<(), String> {
        if self.dirty { Ok(()) } else { Err("no changes since the last save".into()) }
    }

    // ------------------------------------------------------------------ editing

    fn touch(&mut self) {
        self.version += 1;
    }
    fn clamp_focus(&mut self) {
        let rows = self.rows();
        self.focus = self.focus.min(rows.len().saturating_sub(1));
        if matches!(rows.get(self.focus), Some(FieldRow::Header(_))) {
            self.focus = model::move_selection(&rows, self.focus, 1);
        }
    }
    /// Apply `f` to the problem as one undoable edit (a no-op change records nothing).
    fn edit(&mut self, f: impl FnOnce(&mut Problem)) {
        let before = self.problem.clone();
        f(&mut self.problem);
        if self.problem != before {
            self.failure = None;
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.dirty = true;
            self.touch();
        }
        self.clamp_focus();
        // A sketch open in Sketch mode may have been replaced along with the problem.
        self.drop_sketch_mode_if_orphaned();
    }
    fn replace(&mut self, problem: Problem) {
        self.saved_geometry = None;
        self.edit(|p| *p = problem);
        self.clear_selection();
    }
    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.problem, prev));
            self.after_history();
        }
    }
    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.problem, next));
            self.after_history();
        }
    }
    fn after_history(&mut self) {
        self.drop_sketch_mode_if_orphaned();
        self.dirty = true;
        self.clear_selection();
        self.touch();
        self.clamp_focus();
    }

    /// Perform the action of `row` (cycle, add, remove, toggle, open a prompt).
    pub fn activate(&mut self, row: FieldRow, forward: bool) {
        use FieldRow::*;
        let names = self.names();
        match row {
            Template => {
                let all = templates();
                let n = all.len();
                let i = if forward { (self.template + 1) % n } else { (self.template + n - 1) % n };
                if let Some((title, problem)) = all.into_iter().nth(i) {
                    self.template = i;
                    self.replace(problem);
                    self.status = format!("Started from template '{title}' (undoable).");
                }
            }
            Material => {
                self.prompt = Some(Prompt { kind: EditKind::Text, label: "material search".into(), text: String::new(), row: None, open: false, op: None });
                self.status = "Type part of a material name (e.g. 7075 T6), Enter picks the first match.".into();
            }
            Source => {
                let import = self.import.as_ref().map(|(p, _)| p.clone());
                let mut saved = self.saved_geometry.take();
                self.edit(|p| model::toggle_source(p, &mut saved, import));
                self.saved_geometry = saved;
            }
            ImportPath | Header(_) => {}
            _ => self.edit(|p| {
                model::apply_row_action(p, &names, row, forward);
            }),
        }
        self.touch();
    }
    fn focus_row(&mut self, i: usize) {
        let rows = self.rows();
        if i < rows.len() && !matches!(rows[i], FieldRow::Header(_)) {
            self.focus = i;
        }
        self.touch();
    }
    /// Enter / click on the focused row: edit a value or perform an action.
    fn enter_row(&mut self) {
        let Some(row) = self.focused_row() else { return };
        match model::edit_kind(row) {
            EditKind::Action => self.activate(row, true),
            kind => self.begin_edit(row, kind, None),
        }
    }
    fn begin_edit(&mut self, row: FieldRow, kind: EditKind, first: Option<char>) {
        let text = model::edit_start_text(&self.problem, row, kind, first);
        self.prompt = Some(Prompt { kind, label: model::row_label(&self.problem, row), text, row: Some(row), open: false, op: None });
        self.touch();
    }
    fn commit_prompt(&mut self) {
        let Some(prompt) = self.prompt.take() else { return };
        match prompt.row {
            None if prompt.op.is_some() => self.run_prompt_op(prompt.op.unwrap_or(stepops::PromptOp::Fillet), &prompt.text),
            None if prompt.open => self.open_path(&prompt.text),
            None => self.apply_material_search(&prompt.text),
            Some(row) => {
                let mut read = None;
                let kind = prompt.kind;
                self.edit(|p| {
                    if let EditCommit::ReadMesh(path) = model::commit_edit(p, row, kind, &prompt.text) {
                        read = Some(path);
                    }
                });
                if let Some(path) = read {
                    self.read_mesh(&path);
                }
            }
        }
        self.touch();
    }
    fn apply_material_search(&mut self, query: &str) {
        let tokens: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if tokens.is_empty() {
            return;
        }
        let hits: Vec<_> = crate::toolboxes::material_lookup::model::catalog().iter().filter(|m| tokens.iter().all(|t| m.name.to_lowercase().contains(t))).collect();
        match hits.first() {
            Some(m) => {
                let m = **m;
                self.edit(|p| model::apply_material(p, m));
                self.status = format!("Material {} ({} match{})", m.name, hits.len(), if hits.len() == 1 { "" } else { "es" });
            }
            None => self.status = format!("No material matches '{}'.", query.trim()),
        }
    }
    fn read_mesh(&mut self, path: &str) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.import = Some((path.to_string(), text));
                self.status = format!("Read mesh file {path}");
            }
            Err(e) => self.status = format!("Could not read {path}: {e}"),
        }
    }
    fn cycle_focused(&mut self, forward: bool) {
        use FieldRow::*;
        if let Some(r @ (Template | Analysis | Element | SupportEdge(_) | LoadEdge(_))) = self.focused_row() {
            self.activate(r, forward);
        }
    }
    fn remove_focused(&mut self) {
        match self.remove_availability() {
            Ok(()) => {
                if let Some(r) = self.focused_row().and_then(removal_of) {
                    self.activate(r, true);
                }
            }
            Err(e) => self.status = e,
        }
    }
    /// Add a support (fixed) or a load (pressure) on the selected surface and focus its first row.
    /// Every selected surface name, the last picked one first.
    pub fn all_selected(&self) -> Vec<String> {
        self.selected.iter().cloned().chain(self.multi.iter().filter(|n| Some(*n) != self.selected.as_ref()).cloned()).collect()
    }
    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.multi.clear();
    }
    /// Shift-click: add the surface to the selection, or remove it when already selected.
    fn toggle_selected(&mut self, name: String) {
        if self.multi.remove(&name) {
            if self.selected.as_ref() == Some(&name) {
                self.selected = self.multi.iter().next_back().cloned();
            }
            return;
        }
        if self.selected.as_ref() == Some(&name) {
            self.selected = self.multi.iter().next_back().cloned();
            return;
        }
        if let Some(prev) = self.selected.replace(name) {
            self.multi.insert(prev);
        }
    }
    /// One support / load per selected surface (one undo step).
    fn add_on_selected(&mut self, kind: Boundary) {
        let names = self.all_selected();
        if names.is_empty() {
            self.status = "Pick a surface in the viewport or the tree first.".into();
            return;
        }
        self.edit(|p| {
            for name in &names {
                match kind {
                    Boundary::Fixed => p.supports.push(fea_problem::Support::fixed(name)),
                    Boundary::Roller(c) => p.supports.push(fea_problem::Support::roller(name, c)),
                    Boundary::Pressure => p.loads.push(model::default_load(name)),
                }
            }
        });
        let support = kind != Boundary::Pressure;
        let target = self.rows().iter().position(|r| match r {
            FieldRow::SupportKind(s) => support && *s + 1 == self.problem.supports.len(),
            FieldRow::LoadKind(l) => !support && *l + 1 == self.problem.loads.len(),
            _ => false,
        });
        if let Some(i) = target {
            self.focus = i;
        }
        let what = match kind {
            Boundary::Fixed => "fixed support".to_string(),
            Boundary::Roller(c) => format!("roller (holds {}, slides in the others)", ["x", "y", "z"][c]),
            Boundary::Pressure => "pressure load".to_string(),
        };
        self.status = format!("Added a {what} on {}.", names.iter().map(|n| format!("'{n}'")).collect::<Vec<_>>().join(", "));
    }
    fn assign_selection(&mut self) {
        if let Err(e) = self.assign_availability() {
            self.status = e;
            return;
        }
        let (Some(name), Some(row)) = (self.selected.clone(), self.focused_row()) else { return };
        match feature_of(row) {
            Feature::Support(s) => self.edit(|p| {
                if let Some(sup) = p.supports.get_mut(s) {
                    sup.set_edge(&name);
                }
            }),
            Feature::Load(l) => self.edit(|p| {
                if let Some(load) = p.loads.get_mut(l) {
                    load.set_edge(&name);
                }
            }),
            _ => {}
        }
        self.status = format!("Assigned '{name}'.");
    }

    // ------------------------------------------------------------------ files

    pub fn save(&mut self) {
        let result = self.problem.to_json().and_then(|json| {
            if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&self.path, json).map_err(|e| e.to_string())
        });
        match result {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}", self.path.display());
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }
    pub fn load(&mut self) {
        match read_problem(&self.path) {
            Ok(mut p) => {
                // The history is authoritative over the geometry stored beside it.
                let history_error = p.apply_history().err();
                self.replace(p);
                self.dirty = false;
                self.status = match history_error {
                    Some(e) => format!("Loaded {}, but its feature history cannot be rebuilt: {e}", self.path.display()),
                    None => format!("Loaded {}", self.path.display()),
                };
            }
            Err(e) => self.status = format!("Load failed: {e}"),
        }
    }

    // ------------------------------------------------------------------ jobs

    fn take_tag(&mut self) -> u64 {
        self.next_tag += 1;
        self.next_tag
    }
    pub fn solve(&mut self) {
        if let Err(e) = self.solve_availability() {
            self.status = e;
            return;
        }
        self.fold_error_zones();
        let (problem, import) = (self.problem.clone(), self.import_text());
        let tag = self.take_tag();
        match self.solve_worker.start(tag, move || fea_problem::solve(&problem, import.as_deref())) {
            Ok(()) => self.status = "Solving...".into(),
            Err(e) => self.status = e,
        }
    }
    pub fn cancel_solve(&mut self) {
        if self.study_worker.cancel() {
            self.status = "Study cancelled (its answer is ignored).".into();
        }
        if self.solve_worker.cancel() {
            self.status = "Solve cancelled (its answer is ignored).".into();
        }
    }
    fn preview_current(&self) -> bool {
        self.preview.as_ref().is_some_and(|p| p.key == preview_key(&self.problem))
    }
    /// Why a mesh cannot be generated now (`Ok` when it can).
    pub fn mesh_availability(&self) -> Result<(), String> {
        if self.sketch_mode() {
            return Err("finish the sketch first: the mesh is built from the finished model".into());
        }
        if self.meshing() || self.mesh_requested {
            return Err("a mesh is already being generated".into());
        }
        if self.preview_current() {
            return Err("the mesh is already current for this model".into());
        }
        self.mesh_inputs()
    }
    /// The model itself can be meshed (a valid body; for an import, the file has been read).
    fn mesh_inputs(&self) -> Result<(), String> {
        self.problem.validate_geometry()?;
        if matches!(self.problem.geometry, Geometry::Imported { .. }) && self.import_text().is_none() {
            return Err("read the mesh file first (Enter on the Mesh File row)".into());
        }
        Ok(())
    }
    fn generate_mesh(&mut self) {
        match self.mesh_availability() {
            Ok(()) => {
                let added = self.fold_error_zones();
                self.mesh_requested = true;
                self.preview_error = None;
                self.status = if added > 0 { format!("Generating the mesh with {added} zone(s) from the error estimate...") } else { "Generating the mesh...".into() };
            }
            Err(e) => {
                // A model that cannot be meshed says why in the viewport and the tree, not only in the status line.
                if self.mesh_inputs().is_err() && !self.sketch_mode() {
                    self.preview_error = Some(e.clone());
                }
                self.status = e;
            }
        }
        self.touch();
    }
    /// Add the zones the last solve's error estimate asked for, as one undoable edit; how many were added.
    fn fold_error_zones(&mut self) -> usize {
        let zones = std::mem::take(&mut self.error_zones);
        if zones.is_empty() {
            return 0;
        }
        let mut added = 0;
        self.edit(|p| added = fea_problem::advice::add_zones(p, &zones));
        added
    }
    /// Collect finished jobs and start a requested mesh.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some((_, answer)) = self.preview_worker.poll() {
            self.mesh_requested = false;
            match answer {
                Ok(out) => {
                    self.status = format!("Mesh generated: {} nodes, {} elements.", out.mesh.nodes.len(), out.mesh.n_elems());
                    self.preview = Some(Preview { key: out.key, mesh: Arc::new(out.mesh), names: out.names });
                    self.preview_error = None;
                }
                Err(e) => {
                    self.status = format!("Mesh failed: {e}");
                    self.preview_error = Some(e);
                }
            }
            self.touch();
            changed = true;
        }
        if let Some((_, answer)) = self.solve_worker.poll() {
            match answer {
                Ok(solved) => {
                    let s = &solved.summary;
                    self.failure = None;
                    self.status = format!("Solved: max von Mises {:.4e}, equilibrium error {:.1e}", s.max_von_mises.value, s.equilibrium_error);
                    let current = solved.problem == self.problem;
                    self.error_zones = if current { fea_problem::advice::error_zones(&solved) } else { Vec::new() };
                    if !self.error_zones.is_empty() {
                        self.status.push_str(&format!("; the error estimate adds {} refinement zone(s) at the next mesh or solve", self.error_zones.len()));
                    }
                    self.solved = Some(Arc::new(solved));
                    self.show_results = current;
                    if current {
                        self.show_study = false;
                    }
                }
                Err(e) => {
                    self.status = format!("Solve failed: {e}");
                    self.failure = Some(e);
                }
            }
            self.touch();
            changed = true;
        }
        changed |= self.poll_study();
        changed | (self.mesh_requested && self.maybe_start_preview())
    }
    fn maybe_start_preview(&mut self) -> bool {
        if self.sketching.is_some() || self.preview_worker.is_busy() {
            return false;
        }
        if self.preview_current() {
            self.mesh_requested = false;
            return false;
        }
        if let Err(e) = self.mesh_inputs() {
            self.mesh_requested = false;
            self.status = e.clone();
            self.preview_error = Some(e);
            self.touch();
            return true;
        }
        let import = self.import_text();
        let key = preview_key(&self.problem);
        let (tag, job_key) = (self.take_tag(), key.clone());
        let started = self.preview_worker.start(tag, move || {
            let mesh = fea_problem::solve::preview(&job_key, import.as_deref())?;
            let names = fea_problem::build::mesh_names(&mesh);
            Ok(PreviewOut { key: job_key, mesh, names })
        });
        started.is_ok()
    }

    // ------------------------------------------------------------------ scene and picking

    fn nodes_of(mesh: &Mesh, name: &str) -> BTreeSet<usize> {
        let mut out: BTreeSet<usize> = mesh.node_sets.get(name).into_iter().flatten().copied().collect();
        out.extend(mesh.surfaces.get(name).into_iter().flatten().flatten().copied());
        out
    }
    /// Per-node state codes of the preview scene.
    fn node_codes(&self, mesh: &Mesh) -> Vec<f64> {
        let mut codes = vec![0.0_f64; mesh.nodes.len()];
        let mut mark = |name: &str, code: f64| {
            for n in Self::nodes_of(mesh, name) {
                if n < codes.len() {
                    codes[n] = codes[n].max(code);
                }
            }
        };
        for s in &self.problem.supports {
            if let Some(e) = s.edge() {
                mark(e, 1.0);
            }
        }
        for l in &self.problem.loads {
            if let Some(e) = l.edge() {
                mark(e, 2.0);
            }
        }
        // The surface being edited or picked.
        let focused = self.focused_row().map(feature_of).and_then(|f| match f {
            Feature::Support(s) => self.problem.supports.get(s).and_then(|s| s.edge().map(str::to_string)),
            Feature::Load(l) => self.problem.loads.get(l).and_then(|l| l.edge().map(str::to_string)),
            Feature::Hole(h) => Some(format!("hole{}", h + 1)),
            _ => None,
        });
        for name in focused.iter().chain(self.all_selected().iter()) {
            mark(name, 3.0);
        }
        codes
    }
    fn placeholder(label: String) -> Scene {
        Scene {
            label,
            positions: vec![[-0.3, -0.3, 0.0], [0.3, -0.3, 0.0], [0.0, 0.3, 0.0]],
            triangles: vec![0, 1, 2],
            samples: vec![[0.0; 4]; 3],
            times: vec![0.0],
            range: [0.0, 1.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: "no mesh".into(),
            fields: Vec::new(),
        }
    }
    /// Build the scene for the current state. A failure is reported once per change, not retried
    /// every frame: the version is marked built either way.
    pub fn build_scene(&mut self) -> Result<Scene, String> {
        let scene = self.build_scene_inner();
        self.built = Some(self.version);
        scene
    }
    fn build_scene_inner(&mut self) -> Result<Scene, String> {
        if self.sketching.is_some() {
            self.pick_positions.clear();
            return self.sketch_scene();
        }
        let showing = self.current_solution().filter(|_| self.show_results).cloned();
        let scene = if let Some(study_scene) = self.study_scene() {
            self.pick_positions.clear();
            study_scene
        } else if let Some(solved) = showing {
            self.pick_positions.clear();
            SceneSource::from_solved(&solved, Field::VonMises).build()?
        } else if let Some(p) = self.preview.as_ref().filter(|p| p.key == preview_key(&self.problem)) {
            let mesh = p.mesh.clone();
            let codes = self.node_codes(&mesh);
            let zero = vec![0.0; mesh.nodes.len() * mesh.dim()];
            let label = format!("{} | geometry: blue body, teal support, yellow load, red selected", self.problem.name);
            let mut scene = Scene::from_frames(&mesh, &[zero], vec![0.0], &codes, label)?;
            scene.range = [0.0, 3.0];
            scene.field_label = "entity state (see color key)".into();
            self.pick_positions = scene.positions.clone();
            self.add_rings(&mut scene, refining::mesh_to_scene(&mesh), refining::ring_code);
            scene
        } else {
            self.pick_positions.clear();
            let why = match (&self.preview_error, self.meshing()) {
                (Some(e), _) => format!("no mesh: {e}"),
                (None, true) => "meshing...".to_string(),
                _ if self.preview.is_some() => "mesh out of date: generate it again".to_string(),
                _ => "mesh not generated: use Mesh > Generate mesh".to_string(),
            };
            self.outline_scene(&why).unwrap_or_else(|| Self::placeholder(why))
        };
        Ok(scene)
    }
    pub fn needs_scene(&self) -> bool {
        self.built != Some(self.version)
    }

    /// Name of the surface under a viewport pixel (nearest within `PICK_RADIUS`, front-most on ties).
    pub fn pick(&self, cam: &Camera, px: [f32; 2], size: [f32; 2]) -> Option<String> {
        let mesh = &self.preview.as_ref()?.mesh;
        pick_surface(mesh, &self.pick_positions, cam, px, size, PICK_RADIUS)
    }

    // ------------------------------------------------------------------ input

    pub fn do_action(&mut self, action: SolidAction) {
        match action {
            SolidAction::Focus(i) => {
                self.focus_row(i);
                self.enter_row();
            }
            SolidAction::Activate(row) => {
                if let Some(i) = self.rows().iter().position(|r| *r == row) {
                    self.focus = i;
                }
                match model::edit_kind(row) {
                    EditKind::Action => self.activate(row, true),
                    kind => self.begin_edit(row, kind, None),
                }
            }
            SolidAction::FocusFeature(f) => {
                if let Some(i) = self.rows().iter().position(|r| !matches!(r, FieldRow::Header(_)) && feature_of(*r) == f) {
                    self.focus_row(i);
                }
            }
            SolidAction::SelectSurface(i) => {
                self.clear_selection();
                self.selected = self.names().get(i).cloned();
                self.touch();
            }
            SolidAction::ClearSelection => {
                self.clear_selection();
                self.touch();
            }
            SolidAction::AssignSelection => self.assign_selection(),
            SolidAction::RemoveFocused => self.remove_focused(),
            SolidAction::SupportOnSelected => self.add_on_selected(Boundary::Fixed),
            SolidAction::RollerOnSelected(c) => self.add_on_selected(Boundary::Roller(c)),
            SolidAction::LoadOnSelected => self.add_on_selected(Boundary::Pressure),
            SolidAction::Undo => self.undo(),
            SolidAction::Redo => self.redo(),
            SolidAction::Save => self.save(),
            SolidAction::Load => self.load(),
            SolidAction::NextTemplate => self.activate(FieldRow::Template, true),
            SolidAction::NewPart => self.new_part(),
            SolidAction::OpenTemplate(i) => self.open_template(i),
            SolidAction::OpenFile => self.begin_open_file(),
            SolidAction::ClearModel => self.clear_model(),
            SolidAction::Solve => match Self::bench_study(self.bench) {
                Some(kind) => self.run_study(kind),
                None => self.solve(),
            },
            SolidAction::CancelSolve => self.cancel_solve(),
            SolidAction::ShowResults => match self.results_availability() {
                Ok(()) => {
                    self.show_results = !self.show_results;
                    if self.show_results {
                        self.show_study = false;
                    }
                    self.touch();
                }
                Err(e) => self.status = e,
            },
            SolidAction::SetTool(t) => self.set_tool(t),
            SolidAction::Sketch(a) => self.do_sketch_action(a),
            SolidAction::Advice(i) => self.do_advice(i),
            SolidAction::Remedy(f, r) => self.do_remedy(f, r),
            SolidAction::SolidOp(op) => self.begin_solid_op(op),
            SolidAction::Answer(f, q, o) => self.do_answer(f, q, o),
            SolidAction::BoundaryKind(k) => self.add_boundary_kind(k),
            SolidAction::Advisor(op) => self.do_advisor_op(op),
            SolidAction::RunStudy(kind) => self.run_study(kind),
            SolidAction::ShowStudy => self.toggle_study(),
            SolidAction::StudyStep(forward) => self.study_step(forward),
            SolidAction::GenerateMesh => self.generate_mesh(),
            SolidAction::Refine(i, level) => self.do_refine(i, level),
            SolidAction::RefineAll => self.refine_all(),
            SolidAction::ClearRefine => self.clear_refine(),
            SolidAction::ToggleRefineShow => {
                self.show_refine = !self.show_refine;
                self.touch();
            }
        }
    }

    pub fn handle_key(&mut self, key: Key, mods: Mods) -> KeyResult {
        if self.prompt.is_none() {
            if let Some(r) = self.sketch_key(key, mods) {
                return r;
            }
        }
        if let Some(prompt) = &mut self.prompt {
            match key {
                Key::Escape => {
                    self.prompt = None;
                    self.status.clear();
                }
                Key::Backspace => {
                    prompt.text.pop();
                }
                Key::Enter => self.commit_prompt(),
                Key::Char(c) => {
                    let ok = match prompt.kind {
                        EditKind::Number => is_edit_char(c),
                        EditKind::Component => is_edit_char(c) || c.is_ascii_alphabetic(),
                        _ => c.is_ascii_graphic() || c == ' ',
                    };
                    if ok && prompt.text.len() < 200 {
                        prompt.text.push(c);
                    }
                }
                _ => {}
            }
            self.touch();
            return KeyResult::Handled;
        }
        let ctrl = mods.command;
        let row = self.focused_row();
        match key {
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'z') => self.do_action(if mods.shift { SolidAction::Redo } else { SolidAction::Undo }),
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'y') => self.do_action(SolidAction::Redo),
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'s') => self.do_action(SolidAction::Save),
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'o') => self.do_action(SolidAction::OpenFile),
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'n') => self.do_action(SolidAction::NewPart),
            Key::Enter if ctrl => self.do_action(SolidAction::Solve),
            Key::Char(_) if ctrl => return KeyResult::Unhandled,
            Key::Char(c) if matches!(row.map(model::edit_kind), Some(EditKind::Number | EditKind::Component)) && is_edit_char(c) && !c.eq_ignore_ascii_case(&'e') => {
                if let Some(r) = row {
                    self.begin_edit(r, model::edit_kind(r), Some(c));
                }
            }
            Key::Char(c) if c.eq_ignore_ascii_case(&'e') => self.enter_row(),
            Key::Char(c) if c.eq_ignore_ascii_case(&'v') => self.do_action(if Self::bench_study(self.bench).is_some() { SolidAction::ShowStudy } else { SolidAction::ShowResults }),
            Key::Char('[') => self.do_action(SolidAction::StudyStep(false)),
            Key::Char(']') => self.do_action(SolidAction::StudyStep(true)),
            Key::Char(' ') => match row.map(model::edit_kind) {
                Some(EditKind::Component | EditKind::Action) => {
                    if let Some(r) = row {
                        self.activate(r, true);
                    }
                }
                Some(EditKind::Text) => self.enter_row(),
                _ => {}
            },
            Key::Enter => self.enter_row(),
            Key::Up | Key::Down => {
                let rows = self.rows();
                self.focus = model::move_selection(&rows, self.focus, if key == Key::Up { -1 } else { 1 });
                self.touch();
            }
            Key::Left | Key::Right => self.cycle_focused(key == Key::Right),
            Key::Delete | Key::Backspace => self.do_action(SolidAction::RemoveFocused),
            Key::Escape => {
                if self.solving() {
                    self.cancel_solve();
                } else if self.tool != SolidTool::Select {
                    self.set_tool(SolidTool::Select);
                } else if self.selected.is_some() || !self.multi.is_empty() {
                    self.do_action(SolidAction::ClearSelection);
                }
            }
            Key::Tab => return KeyResult::Command(Command::CycleField { forward: !mods.shift }),
            Key::Char(_) => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }

    /// Left click in the viewport: assign the picked surface to the focused support/load edge when
    /// that is what is being edited, otherwise select it.
    pub fn click(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool {
        self.click_with(px, size, cam, false)
    }
    pub fn click_with(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) -> bool {
        if self.sketching.is_some() {
            self.sketch_click(px, size, cam, additive);
            return true;
        }
        if self.prompt.is_some() {
            return true;
        }
        if self.show_results && self.current_solution().is_some() {
            self.status = "Showing results: press V to return to the model before editing.".into();
            return true;
        }
        if self.tool == SolidTool::Hole {
            self.place_hole(cam, px, size);
            self.touch();
            return true;
        }
        let picked = self.pick(cam, px, size);
        if additive {
            if let Some(name) = picked {
                self.toggle_selected(name);
                self.status = format!("{} surface(s) selected.", self.all_selected().len());
            }
            self.touch();
            return true;
        }
        match picked {
            Some(name) => {
                self.multi.clear();
                self.selected = Some(name.clone());
                if self.assign_availability().is_ok() {
                    self.assign_selection();
                } else {
                    self.status = format!("Selected '{name}'.");
                    self.popup = true;
                }
            }
            None => self.clear_selection(),
        }
        self.touch();
        true
    }
    /// Select every surface whose centre falls inside the pixel box (Shift-drag in the viewport).
    pub fn box_select(&mut self, a: [f32; 2], b: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) {
        if self.sketching.is_some() {
            self.sketch_box(a, b, size, cam, additive);
            return;
        }
        if self.prompt.is_some() || self.show_results || self.show_study {
            return;
        }
        let Some(p) = self.preview.as_ref().filter(|p| p.key == preview_key(&self.problem)) else { return };
        let found = surfaces_in_box(&p.mesh, &self.pick_positions, cam, a, b, size);
        if !additive {
            self.clear_selection();
        }
        for name in found {
            if self.selected.is_some() && self.selected.as_ref() != Some(&name) {
                if let Some(prev) = self.selected.take() {
                    self.multi.insert(prev);
                }
            }
            self.selected = Some(name);
        }
        self.status = format!("{} surface(s) selected by box.", self.all_selected().len());
        self.touch();
    }
    /// The boundary-condition list a surface click asked for (taken once).
    pub fn take_popup_flag(&mut self) -> bool {
        std::mem::take(&mut self.popup)
    }
    pub fn select_at(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) {
        if self.sketching.is_some() {
            self.sketch_select_at(px, size, cam);
            return;
        }
        if self.prompt.is_some() || self.show_results {
            return;
        }
        if let Some(name) = self.pick(cam, px, size) {
            self.selected = Some(name);
            self.touch();
        }
    }
}

/// Read a problem file written by this editor or the terminal Workbench.
pub fn read_problem(path: &Path) -> Result<Problem, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    Problem::from_json(&text)
}

/// Scene-space corner nodes of a boundary face (quadratic faces list mid-side nodes after the corners).
fn corners(face: &[usize]) -> &[usize] {
    let n = match face.len() {
        6 => 3,
        8 | 9 => 4,
        n => n,
    };
    &face[..n.min(face.len())]
}

/// Depth (shader depth, smaller is nearer) of the polygon `pts` at pixel `px`, when the pixel is inside it:
/// interpolated over a triangle fan, so a small face in front beats a large face behind it.
fn depth_at(pts: &[([f32; 2], f32)], px: [f32; 2]) -> Option<f32> {
    for k in 1..pts.len().saturating_sub(1) {
        let (a, b, c) = (pts[0], pts[k], pts[k + 1]);
        let d = (b.0[1] - c.0[1]) * (a.0[0] - c.0[0]) + (c.0[0] - b.0[0]) * (a.0[1] - c.0[1]);
        if d.abs() < 1e-9 {
            continue;
        }
        let w0 = ((b.0[1] - c.0[1]) * (px[0] - c.0[0]) + (c.0[0] - b.0[0]) * (px[1] - c.0[1])) / d;
        let w1 = ((c.0[1] - a.0[1]) * (px[0] - c.0[0]) + (a.0[0] - c.0[0]) * (px[1] - c.0[1])) / d;
        let w2 = 1.0 - w0 - w1;
        if w0 >= -1e-5 && w1 >= -1e-5 && w2 >= -1e-5 {
            return Some(w0 * a.1 + w1 * b.1 + w2 * c.1);
        }
    }
    None
}

/// The named surface under a pixel. 2D edges are polylines (nearest within `radius`); 3D faces are
/// polygons: the front-most face whose projection contains the pixel wins (depth interpolated at the
/// pixel, not a face average), and near misses within `radius` fall back to the nearest face outline.
/// Scene-space `positions` match the node indices of `mesh`.
pub fn pick_surface(mesh: &Mesh, positions: &[[f32; 3]], cam: &Camera, px: [f32; 2], size: [f32; 2], radius: f32) -> Option<String> {
    if positions.len() != mesh.nodes.len() {
        return None;
    }
    let screen = |i: usize| {
        let (n, d) = cam.project(positions[i]);
        (Camera::pixel(n, size[0], size[1]), d)
    };
    let solid = mesh.dim() == 3;
    // (inside?, outline distance, depth): inside faces rank by depth, outlines by distance then depth.
    let mut best: Option<((u8, f32, f32), &String)> = None;
    for (name, faces) in &mesh.surfaces {
        for face in faces {
            if face.iter().any(|&n| n >= positions.len()) || face.is_empty() {
                continue;
            }
            let pts: Vec<([f32; 2], f32)> = corners(face).iter().map(|&n| screen(n)).collect();
            let mut dist = f32::INFINITY;
            let mut depth = f32::INFINITY;
            let closed = solid && pts.len() >= 3;
            let segments = if closed { pts.len() } else { pts.len().saturating_sub(1) }.max(1);
            for k in 0..segments {
                let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
                let ab = [b.0[0] - a.0[0], b.0[1] - a.0[1]];
                let len2 = ab[0] * ab[0] + ab[1] * ab[1];
                let t = if len2 > 0.0 { (((px[0] - a.0[0]) * ab[0] + (px[1] - a.0[1]) * ab[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let c = [a.0[0] + t * ab[0], a.0[1] + t * ab[1]];
                let d = ((c[0] - px[0]).powi(2) + (c[1] - px[1]).powi(2)).sqrt();
                if d < dist {
                    dist = d;
                    depth = a.1 + t * (b.1 - a.1);
                }
            }
            let mut rank = 1u8;
            if solid && pts.len() >= 3 {
                if let Some(d) = depth_at(&pts, px) {
                    (rank, dist, depth) = (0, 0.0, d);
                }
            }
            if dist <= radius && best.is_none_or(|(b, _)| (rank, dist, depth) < b) {
                best = Some(((rank, dist, depth), name));
            }
        }
    }
    best.map(|(_, n)| n.clone())
}

/// Names of the surfaces with a face whose centre projects into the pixel box `a`..`b`.
pub fn surfaces_in_box(mesh: &Mesh, positions: &[[f32; 3]], cam: &Camera, a: [f32; 2], b: [f32; 2], size: [f32; 2]) -> BTreeSet<String> {
    let (lo, hi) = ([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]);
    let mut out = BTreeSet::new();
    if positions.len() != mesh.nodes.len() {
        return out;
    }
    for (name, faces) in &mesh.surfaces {
        let hit = faces.iter().any(|face| {
            let c = corners(face);
            if c.is_empty() || c.iter().any(|&n| n >= positions.len()) {
                return false;
            }
            let (mut sx, mut sy) = (0.0, 0.0);
            for &n in c {
                let (ndc, _) = cam.project(positions[n]);
                let p = Camera::pixel(ndc, size[0], size[1]);
                sx += p[0];
                sy += p[1];
            }
            let (cx, cy) = (sx / c.len() as f32, sy / c.len() as f32);
            cx >= lo[0] && cx <= hi[0] && cy >= lo[1] && cy <= hi[1]
        });
        if hit {
            out.insert(name.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod sketching_tests;
#[cfg(test)]
mod models_tests;
#[cfg(test)]
mod corpus_tests;
#[cfg(test)]
mod refining_tests;
#[cfg(test)]
mod advising_tests;
#[cfg(test)]
mod guidance_tests;
