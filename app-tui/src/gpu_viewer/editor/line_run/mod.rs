//! The Line Run workbench in the native window: a supported pressure tube (tube, material, supports, point loads, load
//! cases) checked by `pressure_vessel_solver::line_run`, with the margins, the moment and axial-force stations and the
//! reactions in the inspector, and the line drawn along the vertical axis coloured by bending moment.
//!
//! Inputs are typed in the inspector (a click types a number, toggles a flag, selects a case); every edit re-solves
//! (`LineRunModel::recompute`). Nothing here computes an action or a stress: the solver does.
mod cards;
pub mod drawing;
pub mod file;

use super::{
    authoring::{Authoring, Card},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
};
use crate::gpu_viewer::{
    bench::Bench,
    controller::StandardView,
    menu::{Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
    Scene,
};
use crate::toolboxes::line_run::{
    model::{format_for_edit, Field, Inputs, LineRunModel},
    report,
};
use file::LineRunFile;
use std::path::{Path, PathBuf};

/// What a click in the Line Run workbench asks for (carried by `EditorAction::LineRun`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineRunAct {
    /// Type one number.
    Edit(Field),
    /// Bring the tools of a step (the tree).
    Step(&'static str),
    ToggleClass,
    ToggleUnits,
    Anchor(usize),
    Clamp(usize),
    AddSupport,
    RemoveSupport(usize),
    AddLoad,
    RemoveLoad(usize),
    Thrust(usize),
    /// Cycle which case a load acts in.
    Scope(usize),
    EndLoad,
    SelectCase(usize),
    AddCase,
    /// Remove the selected case.
    RemoveCase,
    /// Type the selected case's name.
    RenameCase,
    Undo,
    Redo,
    Save,
    /// Write the report and the station table beside the case file.
    Export,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Field(Field),
    CaseName,
}

struct Prompt {
    target: Target,
    text: String,
    /// Nothing typed yet: the first character replaces the value instead of extending it.
    fresh: bool,
}

pub struct LineRunSession {
    model: LineRunModel,
    path: PathBuf,
    name: String,
    dirty: bool,
    undo: Vec<Inputs>,
    redo: Vec<Inputs>,
    prompt: Option<Prompt>,
    status: String,
    step: &'static str,
    saved_revision: Option<String>,
    /// The picture is rebuilt only after an edit.
    drawn: bool,
}

const UNDO_LIMIT: usize = 100;

impl LineRunSession {
    pub fn new(file: &LineRunFile, path: PathBuf) -> Self {
        let mut model = LineRunModel::default();
        file.apply(&mut model);
        Self {
            model,
            name: if file.name.is_empty() { path.file_stem().and_then(|n| n.to_str()).unwrap_or("Line run").to_string() } else { file.name.clone() },
            saved_revision: crate::persistence::revision(&path),
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            prompt: None,
            status: String::new(),
            step: "Inputs",
            drawn: false,
        }
    }

    /// A session on `path` when it holds a line run case, else on the workbench defaults (saved there by Ctrl/Cmd+S).
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = if path.exists() { LineRunFile::from_json(&std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?)? } else { LineRunFile::capture(&LineRunModel::default(), "") };
        Ok(Self::new(&file, path.to_path_buf()))
    }

    pub fn model(&self) -> &LineRunModel {
        &self.model
    }

    // ------------------------------------------------------------------ editing

    /// Record the state before an edit so Undo can return to it.
    fn checkpoint(&mut self) {
        self.undo.push(self.model.inputs.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn changed(&mut self) {
        self.dirty = true;
        self.drawn = false;
    }
    /// One undoable edit of the inputs.
    fn edit(&mut self, f: impl FnOnce(&mut LineRunModel)) {
        self.checkpoint();
        f(&mut self.model);
        self.changed();
    }
    fn restore(&mut self, inputs: Inputs) {
        self.model.set_inputs(inputs);
        self.prompt = None;
        self.changed();
    }
    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(self.model.inputs.clone());
            self.restore(prev);
        }
    }
    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(self.model.inputs.clone());
            self.restore(next);
        }
    }
    fn snapshot(&self) -> LineRunFile {
        LineRunFile::capture(&self.model, &self.name)
    }
    fn save(&mut self) {
        if crate::persistence::revision(&self.path) != self.saved_revision {
            self.status = "Save conflict: file changed externally; reopen before saving.".into();
            return;
        }
        match crate::persistence::write(&self.path, self.snapshot().to_json()) {
            Ok(()) => {
                self.dirty = false;
                self.saved_revision = crate::persistence::revision(&self.path);
                self.status = format!("Saved {}.", self.path.display());
            }
            Err(e) => self.status = format!("Could not save {}: {e}", self.path.display()),
        }
    }
    /// The plain-text report (`name.report.txt`) and the station table (`name.stations.csv`), beside the case file.
    fn export(&mut self) {
        let (text, csv) = (self.path.with_extension("report.txt"), self.path.with_extension("stations.csv"));
        let table = report::stations_csv(&self.model);
        let written = crate::persistence::write(&text, report::build_report_text(&self.model)).and_then(|()| table.as_ref().map_or(Ok(()), |t| crate::persistence::write(&csv, t.clone())));
        self.status = match (written, table) {
            (Ok(()), Ok(_)) => format!("Report and station table written to {} and {}.", text.display(), csv.display()),
            (Ok(()), Err(_)) => format!("Report written to {} (no station table: there is no result).", text.display()),
            (Err(e), _) => format!("Could not write the report: {e}"),
        };
    }

    // ------------------------------------------------------------------ prompts

    fn open_prompt(&mut self, target: Target) {
        let text = match target {
            Target::Field(f) => self.model.shown(f).map(format_for_edit).unwrap_or_default(),
            Target::CaseName => self.model.selected_case().name.clone(),
        };
        self.prompt = Some(Prompt { target, text, fresh: true });
    }
    fn commit_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        match p.target {
            Target::CaseName => {
                let name = p.text.trim().to_string();
                if !name.is_empty() && name != self.model.selected_case().name {
                    self.edit(|m| m.rename_case(&name));
                }
            }
            Target::Field(f) => match p.text.trim().parse::<f64>() {
                // Typing the value it already has is not an edit: no undo step.
                Ok(v) if v.is_finite() && self.model.same(f, v) => {}
                Ok(v) if v.is_finite() && self.model.exists(f) => self.edit(|m| {
                    m.set(f, v);
                }),
                _ => self.status = format!("{} not changed: {:?} is not a number.", f.label(), p.text.trim()),
            },
        }
    }
    fn prompt_char(&mut self, c: char) {
        let Some(p) = &mut self.prompt else { return };
        let ok = match p.target {
            Target::Field(_) => c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'),
            Target::CaseName => !c.is_control(),
        };
        if ok && p.text.chars().count() < 40 {
            if std::mem::take(&mut p.fresh) {
                p.text.clear();
            }
            p.text.push(c);
        }
    }

    fn do_act(&mut self, a: LineRunAct) {
        // A message stays until the next thing the person does.
        self.status.clear();
        match a {
            LineRunAct::Edit(f) => self.open_prompt(Target::Field(f)),
            LineRunAct::RenameCase => self.open_prompt(Target::CaseName),
            LineRunAct::Step(s) => self.step = s,
            LineRunAct::SelectCase(k) => {
                self.prompt = None;
                self.model.select_case(k);
                self.drawn = false;
            }
            LineRunAct::Undo => self.undo(),
            LineRunAct::Redo => self.redo(),
            LineRunAct::Save => self.save(),
            LineRunAct::Export => self.export(),
            other => {
                self.prompt = None;
                self.edit(|m| match other {
                    LineRunAct::ToggleClass => m.toggle_class(),
                    LineRunAct::ToggleUnits => m.toggle_units(),
                    LineRunAct::Anchor(k) => m.toggle_anchor(k),
                    LineRunAct::Clamp(k) => m.toggle_clamp(k),
                    LineRunAct::AddSupport => m.add_support(),
                    LineRunAct::RemoveSupport(k) => m.remove_support(k),
                    LineRunAct::AddLoad => m.add_load(),
                    LineRunAct::RemoveLoad(k) => m.remove_load(k),
                    LineRunAct::Thrust(k) => m.toggle_thrust(k),
                    LineRunAct::Scope(k) => m.cycle_scope(k),
                    LineRunAct::EndLoad => m.toggle_end_load(),
                    LineRunAct::AddCase => m.add_case(),
                    LineRunAct::RemoveCase => m.remove_case(m.inputs.selected),
                    _ => {}
                });
            }
        }
    }
}

impl Authoring for LineRunSession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::LineRun(a) = action {
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
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
    // The picture is the result drawn; a click on it picks nothing.
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        self.drawn = true;
        // A model with no result shows an empty viewport; the Result card says why.
        Ok(drawing::scene(&self.model, &self.name).unwrap_or_else(|_| drawing::empty(&self.name)))
    }
    fn needs_scene(&self) -> bool {
        !self.drawn
    }
    fn poll(&mut self) -> bool {
        false
    }
    fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
    fn solving(&self) -> bool {
        false
    }
    fn status(&self) -> String {
        if !self.status.is_empty() {
            return self.status.clone();
        }
        self.verdict()
    }
    fn set_status(&mut self, text: String) {
        self.status = text;
    }
    fn title(&self) -> String {
        format!("Line Run | {}{}", self.name, if self.dirty { " *" } else { "" })
    }
    fn initial_view(&self) -> Option<StandardView> {
        Some(StandardView::Isometric)
    }
    // The colours are the member actions along the line: a legend in the shown units, not a colour key.
    fn shows_result(&self) -> bool {
        true
    }
    fn geometry_view(&self) -> bool {
        false
    }
    fn field_unit(&self, label: &str) -> String {
        let u = self.model.units();
        if label.contains("moment") {
            u.label(crate::toolboxes::line_run::model::Qty::Moment).into()
        } else if label.contains("force") {
            u.label(crate::toolboxes::line_run::model::Qty::Force).into()
        } else {
            String::new()
        }
    }
    fn restore_recovery(&mut self) -> Result<(), String> {
        let text = std::fs::read_to_string(self.path.with_extension("recovery.json")).map_err(|e| e.to_string())?;
        let f = LineRunFile::from_json(&text)?;
        self.undo.push(self.model.inputs.clone());
        self.restore(f.inputs);
        Ok(())
    }
    fn recovery_json(&self) -> Result<String, String> {
        Ok(self.snapshot().to_json())
    }
    fn dirty(&self) -> bool {
        self.dirty
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
            "Inputs" if self.model.outcome.is_ok() => Done,
            "Results" if self.all_pass() => Done,
            "Report" if !self.dirty && self.path.exists() => Done,
            "Inputs" | "Results" | "Report" => Todo,
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
