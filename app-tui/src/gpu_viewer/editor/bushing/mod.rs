//! The Bushing workbench in the native window: the press-fit analysis of `toolboxes::bushing` (the same inputs, solver,
//! pass / fail checks, verified fixes and edge-distance cross-check) with the housing and the bushing drawn, meshed and
//! solved by the FEA kernel. The inputs and checks are `BushingState`'s; the picture is a `SolidSession` over a `Problem`
//! derived from them (`fe`), shown read-only: the inputs are the model, the FE view follows them.
//!
//! Inputs live in the inspector (a click edits a number, cycles a choice or opens a picker as a value card), the tree
//! lists the groups and checks, the ribbon has the steps Inputs, Checks and Simulation. Nothing here computes
//! engineering results: `bushing-solver`, `edge-check` and the FE kernel do.
mod cards;
pub mod fe;
pub mod file;
mod picker;

use super::{
    authoring::{Authoring, Card, Overlay},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
    solid::{SolidAction, SolidSession},
};
use crate::app::Effect;
use crate::gpu_viewer::{
    bench::Bench,
    menu::{Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode, ValueCard},
    Scene,
};
use crate::toolboxes::bushing::{
    advice::{CheckKind, Severity},
    edge_check::{self, EdgeCheckRun},
    model::{self, FieldRow, NumberTarget},
    BushingState,
};
use file::BushingFile;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// What a click in the Bushing workbench asks for (carried by `EditorAction::Bushing`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BushingAct {
    /// An input row: edit the number, cycle the choice or open the picker behind it.
    Row(FieldRow),
    /// Bring the tools of a step (the tree).
    Step(&'static str),
    /// A material of the catalog (`.0` true: the bushing's, else the housing's).
    PickMaterial(bool, usize),
    /// An entry of the bore (reamer) picker, the bushing ID picker, the friction picker.
    PickBore(usize),
    PickId(usize),
    PickFriction(usize),
    /// Leave the picker for typing the number.
    TypeValue(NumberTarget),
    ClosePicker,
    /// Run the edge-distance cross-check (`.0`: with the contact FE).
    EdgeCheck(bool),
    CancelEdgeCheck,
    /// Show the fixes for a failing check, or all of them.
    ShowFixes(CheckKind),
    AllFixes,
    SelectFix(usize),
    ApplyFix(usize),
    Undo,
    Redo,
    Save,
    Export,
    GenerateMesh,
    Solve,
    CancelSolve,
    ToggleResults,
}

struct Prompt {
    row: FieldRow,
    text: String,
    /// Nothing typed yet: the first character replaces the value instead of extending it.
    fresh: bool,
}

pub struct BushingSession {
    st: BushingState,
    fea: SolidSession,
    path: PathBuf,
    name: String,
    dirty: bool,
    undo: Vec<BushingFile>,
    redo: Vec<BushingFile>,
    prompt: Option<Prompt>,
    picker: Option<picker::Picker>,
    status: String,
    /// Why there is no FE picture (the inputs describe no bushing in a housing yet).
    fe_note: Option<String>,
    /// The problem last handed to the FE session.
    fe_key: Option<fea_problem::Problem>,
    mesh_wanted: bool,
    last_fea_status: String,
    edge_jobs: Vec<(u64, Receiver<Option<EdgeCheckRun>>)>,
    step: &'static str,
    /// Only the fixes of this check are listed (`None`: all of them).
    fix_kind: Option<CheckKind>,
}

const UNDO_LIMIT: usize = 100;

impl BushingSession {
    pub fn new(file: &BushingFile, path: PathBuf) -> Self {
        let mut st = BushingState::default();
        let skipped = file.apply(&mut st.model);
        let status = if skipped.is_empty() { String::new() } else { format!("Not applied from the file: {}.", skipped.join("; ")) };
        let blank = fea_problem::templates::templates().remove(4).1;
        let mut fea = SolidSession::new(blank, path.with_extension("fe.json"));
        fea.highlight_bushings = true;
        // The refinement hints are for editing a mesh; here the mesh follows the inputs.
        fea.do_action(SolidAction::ToggleRefineShow);
        let mut s = Self {
            st,
            fea,
            name: if file.name.is_empty() { path.file_stem().and_then(|n| n.to_str()).unwrap_or("Bushing").to_string() } else { file.name.clone() },
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            prompt: None,
            picker: None,
            status,
            fe_note: None,
            fe_key: None,
            mesh_wanted: false,
            last_fea_status: String::new(),
            edge_jobs: Vec::new(),
            step: "Inputs",
            fix_kind: None,
        };
        s.refresh_fea();
        s
    }

    /// A session on `path` when it holds a bushing file, else on the workbench defaults (saved there by Ctrl/Cmd+S).
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = if path.exists() { BushingFile::from_json(&std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?)? } else { BushingFile::capture(&model::BushingModel::default(), "") };
        Ok(Self::new(&file, path.to_path_buf()))
    }

    pub fn model(&self) -> &model::BushingModel {
        &self.st.model
    }
    pub fn fea(&self) -> &SolidSession {
        &self.fea
    }

    // ------------------------------------------------------------------ editing

    fn snapshot(&self) -> BushingFile {
        BushingFile::capture(&self.st.model, &self.name)
    }
    /// Record the state before an edit so Undo can return to it.
    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    /// The inputs changed: the FE picture follows, and what the last fix did is no longer news.
    fn changed(&mut self) {
        self.dirty = true;
        self.st.last_applied = None;
        self.st.input_error = None;
        self.refresh_fea();
    }
    /// Hand the FE session the problem for the current inputs, and ask for its mesh.
    fn refresh_fea(&mut self) {
        match fe::housing_problem(&self.st.model) {
            Ok(p) => {
                self.fe_note = None;
                if self.fe_key.as_ref() != Some(&p) {
                    self.fea.set_derived(p.clone());
                    self.fe_key = Some(p);
                    self.mesh_wanted = true;
                }
            }
            Err(why) => self.fe_note = Some(why),
        }
    }
    fn set_from(&mut self, file: &BushingFile) {
        let skipped = file.apply(&mut self.st.model);
        if !skipped.is_empty() {
            self.status = format!("Not applied: {}.", skipped.join("; "));
        }
        self.dirty = true;
        self.refresh_fea();
    }
    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.set_from(&prev);
        }
    }
    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.set_from(&next);
        }
    }
    fn save(&mut self) {
        match std::fs::write(&self.path, self.snapshot().to_json()) {
            Ok(()) => {
                self.dirty = false;
                self.status = format!("Saved {}.", self.path.display());
            }
            Err(e) => self.status = format!("Could not save {}: {e}", self.path.display()),
        }
    }
    /// The report text beside the file (`name.report.txt`).
    fn export(&mut self) {
        let target = self.path.with_extension("report.txt");
        match std::fs::write(&target, self.st.report_text()) {
            Ok(()) => self.status = format!("Report written to {}.", target.display()),
            Err(e) => self.status = format!("Could not write {}: {e}", target.display()),
        }
    }

    // ------------------------------------------------------------------ rows and prompts

    fn rows(&self) -> Vec<FieldRow> {
        model::field_rows(&self.st.model)
    }
    /// A click (or Enter) on an input row.
    fn activate_row(&mut self, row: FieldRow) {
        match row {
            FieldRow::Header(_) => {}
            FieldRow::Number(NumberTarget::BoreDia) => self.open_picker(picker::Kind::Bore),
            FieldRow::Number(NumberTarget::IdBushing) => self.open_picker(picker::Kind::Id),
            FieldRow::Number(NumberTarget::Friction) => self.open_picker(picker::Kind::Friction),
            FieldRow::OpenHousingMaterialPicker => self.open_picker(picker::Kind::Housing),
            FieldRow::OpenBushingMaterialPicker => self.open_picker(picker::Kind::Bushing),
            FieldRow::Number(t) => self.begin_prompt(row, model::format_for_edit(self.st.model.number_value(t))),
            FieldRow::Tol(g) => self.begin_prompt(row, self.st.model.tolerance_edit_text(g)),
            _ => {
                let Some(i) = self.rows().iter().position(|r| *r == row) else { return };
                self.checkpoint();
                self.st.selected = i;
                self.st.activate_selected();
                self.changed();
            }
        }
    }
    fn begin_prompt(&mut self, row: FieldRow, text: String) {
        self.picker = None;
        self.prompt = Some(Prompt { row, text, fresh: true });
    }
    fn commit_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        match p.row {
            FieldRow::Number(t) => match p.text.trim().parse::<f64>() {
                Ok(v) if v.is_finite() => {
                    // Typing the value it already has is not an edit: no undo step, no recompute, no new mesh.
                    if v != self.st.model.number_value(t) {
                        self.checkpoint();
                        self.st.model.commit_number(t, v);
                        self.changed();
                    }
                }
                _ => self.status = format!("{} not changed: {:?} is not a number.", t.label(), p.text.trim()),
            },
            FieldRow::Tol(g) => {
                self.checkpoint();
                match self.st.model.commit_tolerance_text(g, &p.text) {
                    Ok(()) => self.changed(),
                    Err(why) => {
                        self.undo.pop();
                        self.status = format!("Tolerance not changed: {why}");
                    }
                }
            }
            _ => {}
        }
    }
    fn prompt_char(&mut self, c: char) {
        let Some(p) = &mut self.prompt else { return };
        let ok = match p.row {
            FieldRow::Tol(_) => c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | ' ' | '/' | ',' | '\u{b1}'),
            _ => c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'),
        };
        if ok && p.text.len() < 40 {
            if std::mem::take(&mut p.fresh) {
                p.text.clear();
            }
            p.text.push(c);
        }
    }

    // ------------------------------------------------------------------ checks, fixes, edge check

    fn run_effects(&mut self, effects: Vec<Effect>) {
        for e in effects {
            if let Effect::RunEdgeCheck { id, input, cfg, .. } = e {
                let (tx, rx) = channel();
                let spawned = std::thread::Builder::new().name("bushing-edge-check".into()).spawn(move || {
                    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| edge_check::execute(input, &cfg))).ok();
                    let _ = tx.send(run);
                });
                match spawned {
                    Ok(_) => self.edge_jobs.push((id, rx)),
                    Err(e) => {
                        self.st.edge_job = None;
                        self.status = format!("Could not start the edge check: {e}");
                    }
                }
            }
        }
    }
    fn start_edge_check(&mut self, deep: bool) {
        let effects = self.st.start_edge_check(deep);
        self.run_effects(effects);
        if effects_started(&self.st) {
            self.status.clear();
        } else if let Some(why) = self.st.input_error.take() {
            self.status = why;
        }
    }
    fn apply_fix(&mut self, i: usize) {
        self.checkpoint();
        self.st.rec_selected = i;
        match self.st.model.apply_recommendation(i) {
            Some(summary) => {
                self.status = format!("Applied: {summary}");
                self.st.rec_selected = 0;
                self.changed();
                self.st.last_applied = Some(summary);
            }
            None => {
                self.undo.pop();
                self.status = "That fix needs a manual change: see its note.".into();
            }
        }
    }
    fn failing(&self) -> usize {
        self.st.model.checks.iter().filter(|c| c.severity == Severity::Fail).count()
    }
    fn edge_current(&self) -> bool {
        self.st.edge_check.as_ref().is_some_and(|r| !edge_check::is_stale(&self.st.model, r))
    }

    // ------------------------------------------------------------------ dispatch

    fn do_act(&mut self, a: BushingAct) {
        match a {
            BushingAct::Row(r) => self.activate_row(r),
            BushingAct::Step(s) => self.step = s,
            BushingAct::PickMaterial(bushing, i) => {
                self.checkpoint();
                if bushing {
                    self.st.model.select_bushing_material(i);
                } else {
                    self.st.model.select_housing_material(i);
                }
                self.picker = None;
                self.changed();
            }
            BushingAct::PickBore(i) => self.pick_bore(i),
            BushingAct::PickId(i) => self.pick_id(i),
            BushingAct::PickFriction(i) => self.pick_friction(i),
            BushingAct::TypeValue(t) => self.begin_prompt(FieldRow::Number(t), model::format_for_edit(self.st.model.number_value(t))),
            BushingAct::ClosePicker => self.picker = None,
            BushingAct::EdgeCheck(deep) => self.start_edge_check(deep),
            BushingAct::CancelEdgeCheck => {
                self.st.cancel_edge_check();
                self.edge_jobs.clear();
            }
            BushingAct::ShowFixes(k) => {
                self.fix_kind = Some(k);
                self.step = "Checks";
                self.st.rec_selected = self.st.model.recommendations.iter().position(|r| r.fixes == k).unwrap_or(0);
            }
            BushingAct::AllFixes => {
                self.fix_kind = None;
                self.step = "Checks";
                self.st.rec_selected = 0;
            }
            BushingAct::SelectFix(i) => self.st.rec_selected = i,
            BushingAct::ApplyFix(i) => self.apply_fix(i),
            BushingAct::Undo => self.undo(),
            BushingAct::Redo => self.redo(),
            BushingAct::Save => self.save(),
            BushingAct::Export => self.export(),
            BushingAct::GenerateMesh => {
                self.status.clear();
                self.fea.do_action(SolidAction::GenerateMesh);
            }
            BushingAct::Solve => {
                self.status.clear();
                self.fea.do_action(SolidAction::Solve);
            }
            BushingAct::CancelSolve => self.fea.do_action(SolidAction::CancelSolve),
            BushingAct::ToggleResults => self.fea.do_action(SolidAction::ShowResults),
        }
    }
}

/// An edge check is running for the state (the request was accepted).
fn effects_started(st: &BushingState) -> bool {
    st.edge_job.is_some()
}

impl Authoring for BushingSession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn key_labels(&self) -> [&'static str; 4] {
        ["Housing", "Support", "Load", "Bushing"]
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::Bushing(a) = action {
            self.do_act(a);
        }
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        if self.prompt.is_some() {
            match key {
                Key::Escape => self.prompt = None,
                Key::Enter => self.commit_prompt(),
                Key::Backspace => {
                    if let Some(p) = &mut self.prompt {
                        if std::mem::take(&mut p.fresh) {
                            p.text.clear();
                        } else {
                            p.text.pop();
                        }
                    }
                }
                Key::Char(c) => self.prompt_char(c),
                _ => {}
            }
            return KeyResult::Handled;
        }
        if self.picker.is_some() {
            if key == Key::Escape {
                self.picker = None;
            }
            return KeyResult::Handled;
        }
        match key {
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'s') => self.save(),
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'z') => {
                if mods.shift {
                    self.redo()
                } else {
                    self.undo()
                }
            }
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'y') => self.redo(),
            Key::Enter if mods.command => self.do_act(BushingAct::Solve),
            Key::Escape if self.st.edge_job.is_some() => self.do_act(BushingAct::CancelEdgeCheck),
            Key::Char(_) if mods.command => return KeyResult::Unhandled,
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
    // The FE picture is read-only: the inputs are the model, so a click on it picks nothing.
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        // The fit moves things by thousandths of an inch: at the window's automatic exaggeration the bodies drift apart, so
        // a result opens at its true size (View > Larger deformation exaggerates it again).
        self.fea.scene().map(|mut s| {
            if self.fea.shows_result() {
                s.gain = 1.0;
            }
            s
        })
    }
    fn needs_scene(&self) -> bool {
        self.fea.needs_scene()
    }
    fn poll(&mut self) -> bool {
        let mut changed = self.fea.poll();
        // The mesh follows the inputs a moment after they settle: one job at a time.
        if self.mesh_wanted && !self.fea.meshing() {
            self.mesh_wanted = false;
            if self.fea.mesh_availability().is_ok() {
                self.fea.do_action(SolidAction::GenerateMesh);
            }
            changed = true;
        }
        // Finished edge checks, and the automatic re-run once the inputs have stopped changing.
        let mut done = Vec::new();
        self.edge_jobs.retain(|(id, rx)| match rx.try_recv() {
            Ok(run) => {
                done.push((*id, run));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => {
                done.push((*id, None));
                false
            }
        });
        for (id, run) in done {
            changed = true;
            match run {
                Some(run) => self.st.finish_edge_check(id, run),
                None => {
                    self.st.edge_job = None;
                    self.status = "The edge check stopped unexpectedly.".into();
                }
            }
        }
        let effects = self.st.auto_edge_check();
        if !effects.is_empty() {
            self.run_effects(effects);
            changed = true;
        }
        self.st.expire_edge_notice();
        if self.fea.status != self.last_fea_status {
            self.last_fea_status = self.fea.status.clone();
            self.status.clear();
            changed = true;
        }
        changed
    }
    fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
    fn solving(&self) -> bool {
        self.fea.solving()
    }
    fn solving_for(&self) -> Option<std::time::Duration> {
        self.fea.solve_elapsed()
    }
    fn status(&self) -> String {
        if !self.status.is_empty() {
            return self.status.clone();
        }
        if let Some(j) = &self.st.edge_job {
            return format!("Edge check running ({:.0} s)...", j.started.elapsed().as_secs_f32());
        }
        if let Some(why) = &self.fe_note {
            return format!("No FE picture yet: {why}.");
        }
        self.fea.status.clone()
    }
    fn set_status(&mut self, text: String) {
        self.status = text;
    }
    fn title(&self) -> String {
        format!("Bushing | {}{}", self.name, if self.dirty { " *" } else { "" })
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    // The bodies read by colour; the mesh lines are one key press away (M).
    fn default_mesh_lines(&self) -> bool {
        false
    }
    fn geometry_view(&self) -> bool {
        self.fea.geometry_view()
    }
    fn shows_result(&self) -> bool {
        self.fea.shows_result()
    }
    fn shows_body_only(&self) -> bool {
        self.fea.shows_body_only()
    }
    fn field_unit(&self, label: &str) -> String {
        self.fea.field_unit(label)
    }
    fn set_view(&mut self, cam: &Camera, size: [f32; 2]) {
        self.fea.set_view(cam, size);
    }
    fn overlay(&self, cam: &Camera, size: [f32; 2]) -> Vec<Overlay> {
        self.fea.overlay(cam, size)
    }
    fn value_card(&self) -> Option<ValueCard> {
        self.picker_card()
    }
    fn card_field(&mut self, index: usize, text: String) {
        self.picker_field(index, text);
    }
    fn focus_step(&self) -> Option<&'static str> {
        Some(self.step)
    }
    fn tree(&self) -> Vec<TreeNode> {
        self.tree_nodes()
    }
    fn properties(&self) -> Vec<DockItem> {
        super::authoring::flatten(self.inspector())
    }
    fn inspector(&self) -> Vec<Card> {
        self.inspector_cards()
    }
    fn ribbon(&self) -> Vec<RibbonTab> {
        self.ribbon_tabs()
    }
    fn step_state(&self, tab: &str) -> Option<super::authoring::StepState> {
        use super::authoring::StepState::{Done, Todo};
        Some(match tab {
            "Inputs" => Done,
            "Checks" if self.failing() == 0 && self.edge_current() => Done,
            "Simulation" if self.fea.current_solution().is_some_and(|s| s.problem == self.fea.problem) => Done,
            "Checks" | "Simulation" => Todo,
            _ => return None,
        })
    }
    fn menus(&self) -> Vec<Menu> {
        self.menu_bar()
    }
    fn context_menu(&self) -> Vec<Item> {
        self.menu_bar().into_iter().find(|m| m.title == "Tools").map(|m| m.items).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
