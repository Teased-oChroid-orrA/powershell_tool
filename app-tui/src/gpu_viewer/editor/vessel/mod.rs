//! The Pressure Vessel workbench in the native window: the thick-wall (Lamé) cylinder of `toolboxes::pressure_vessel`
//! (the same model, form rows, checks, minimum-thickness solve and report), with the vessel drawn to scale and coloured
//! by its stress through the wall.
//!
//! Inputs live in the inspector (a click types a number, cycles the end condition or opens the material picker), the
//! checks follow every edit in their own cards, the ribbon has the steps Inputs, Results and Report. Nothing here
//! computes a stress: `pressure-vessel-solver` does, through `PressureVesselModel::commit_number` / `recompute`.
mod cards;
mod page;
pub mod drawing;
pub mod file;
mod picker;

use super::{
    authoring::{Authoring, Card},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
};
use crate::gpu_viewer::{
    bench::Bench,
    controller::StandardView,
    menu::{Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode, ValueCard},
    Scene,
};
use crate::toolboxes::pressure_vessel::{
    edit::Edit,
    model::{FieldRow, PressureVesselModel},
    persistence, report,
    walls::WallOutcome,
};
use super::worker::Worker;
use file::VesselFile;
use std::path::{Path, PathBuf};

/// What a click in the Pressure Vessel workbench asks for (carried by `EditorAction::Vessel`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VesselAct {
    /// An input row: type the number, flip the end condition, or open the material picker.
    Row(FieldRow),
    /// Bring the tools of a step (the tree).
    Step(&'static str),
    /// The `i`th material of the catalog (`material_catalog` order).
    PickMaterial(usize),
    /// The picker's form for a material of one's own, and back to the list.
    NewMaterial,
    ListMaterials,
    /// Check the form and add its material to the catalog.
    AddMaterial,
    ClosePicker,
    /// Type into a field of the requirements table, the allowables or the line run.
    Edit(Edit),
    /// The next criterion of a requirement row; add or remove a row.
    CycleRequirement(usize),
    AddRequirement,
    RemoveRequirement(usize),
    /// The material class (ductile / brittle) the rows follow.
    ToggleKind,
    ToggleLineRun,
    SelectCase(usize),
    AddCase,
    AddSupport,
    AddLoad,
    RemoveCase(usize),
    RemoveSupport(usize),
    RemoveLoad(usize),
    /// A support's axial anchor (`false`) or rotation fixity (`true`), and a case's end condition.
    ToggleSupport(usize, bool),
    ToggleCaseEnds(usize),
    Undo,
    Redo,
    Save,
    Export,
}

struct Prompt {
    edit: Edit,
    text: String,
    /// Nothing typed yet: the first character replaces the value instead of extending it.
    fresh: bool,
}

pub struct VesselSession {
    model: PressureVesselModel,
    path: PathBuf,
    name: String,
    dirty: bool,
    undo: Vec<VesselFile>,
    redo: Vec<VesselFile>,
    prompt: Option<Prompt>,
    picker: Option<picker::Picker>,
    /// Where materials added here are kept between runs (`toolboxes::pressure_vessel::persistence`; `None`: nowhere).
    library: Option<PathBuf>,
    status: String,
    step: &'static str,
    saved_revision: Option<String>,
    /// The picture is rebuilt only after an edit.
    drawn: bool,
    /// The line run's minimum walls, solved off the window's thread (tag: `PressureVesselModel::walls_revision`).
    walls: Worker<Vec<WallOutcome>>,
}

const UNDO_LIMIT: usize = 100;

impl VesselSession {
    pub fn new(file: &VesselFile, path: PathBuf) -> Self {
        let mut model = PressureVesselModel::default();
        let skipped = file.apply(&mut model);
        let status = if skipped.is_empty() { String::new() } else { format!("Not applied from the file: {}.", skipped.join("; ")) };
        Self {
            model,
            name: if file.name.is_empty() { path.file_stem().and_then(|n| n.to_str()).unwrap_or("Pressure vessel").to_string() } else { file.name.clone() },
            saved_revision: crate::persistence::revision(&path),
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            prompt: None,
            picker: None,
            library: persistence::config_path(),
            status,
            step: "Inputs",
            drawn: false,
            walls: Worker::new("vessel-walls"),
        }
    }

    /// A session on `path` when it holds a pressure vessel case, else on the workbench defaults (saved there by
    /// Ctrl/Cmd+S).
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = if path.exists() { VesselFile::from_json(&std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?)? } else { VesselFile::capture(&PressureVesselModel::default(), "") };
        Ok(Self::new(&file, path.to_path_buf()))
    }

    pub fn model(&self) -> &PressureVesselModel {
        &self.model
    }

    /// Start the line run's wall solve when the model waits for one and the worker holds no job for this revision. An
    /// older job is told to stop, its answer is dropped (`set_line_walls`), and `poll` starts the new one once it ends.
    fn kick_walls(&mut self) {
        if self.model.walls.is_some() || !self.model.line.enabled {
            return;
        }
        match self.walls.tag() {
            Some(t) if t == self.model.walls_revision => return,
            Some(_) => {
                self.walls.cancel();
                return;
            }
            None => {}
        }
        if self.walls.is_busy() {
            return;
        }
        if let Some((revision, job)) = self.model.line_wall_job() {
            let _ = self.walls.start(revision, move || job.run());
        }
    }

    // ------------------------------------------------------------------ editing

    fn snapshot(&self) -> VesselFile {
        VesselFile::capture(&self.model, &self.name)
    }
    /// Record the state before an edit so Undo can return to it.
    fn checkpoint(&mut self) {
        let before = self.snapshot();
        self.checkpoint_with(before);
    }
    fn checkpoint_with(&mut self, before: VesselFile) {
        self.undo.push(before);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn changed(&mut self) {
        self.dirty = true;
        self.drawn = false;
        self.kick_walls();
    }
    /// One undo step around a model edit that has no typed text: a switch, a row added or removed.
    fn edit_model(&mut self, f: impl FnOnce(&mut PressureVesselModel)) {
        self.prompt = None;
        self.checkpoint();
        f(&mut self.model);
        self.changed();
    }
    fn set_from(&mut self, file: &VesselFile) {
        let skipped = file.apply(&mut self.model);
        if !skipped.is_empty() {
            self.status = format!("Not applied: {}.", skipped.join("; "));
        }
        self.prompt = None;
        self.picker = None;
        self.changed();
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
    /// The plain-text report, beside the case file (`name.report.txt`).
    fn export(&mut self) {
        let target = self.path.with_extension("report.txt");
        match crate::persistence::write(&target, report::build_report_text(&self.model)) {
            Ok(()) => self.status = format!("Report written to {}.", target.display()),
            Err(e) => self.status = format!("Could not write {}: {e}", target.display()),
        }
    }

    // ------------------------------------------------------------------ rows and prompts

    /// A click (or Enter) on an input row.
    fn activate_row(&mut self, row: FieldRow) {
        match row {
            FieldRow::Header(_) => {}
            FieldRow::Number(target) => self.begin_edit(Edit::Number(target)),
            FieldRow::ToggleEndCondition => {
                self.prompt = None;
                self.checkpoint();
                self.model.toggle_end_condition();
                self.changed();
            }
            FieldRow::ToggleHeadComponent => {
                self.prompt = None;
                self.checkpoint();
                self.model.toggle_head_component();
                self.changed();
            }
            FieldRow::OpenMaterialPicker => self.open_picker(),
        }
    }
    /// Open the typed edit of a field; a number starts selected (the first character replaces it), a name does not.
    fn begin_edit(&mut self, edit: Edit) {
        self.picker = None;
        if let Some(text) = self.model.edit_text(edit) {
            self.prompt = Some(Prompt { edit, text, fresh: !edit.is_text() });
        }
    }
    fn commit_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        let before = self.snapshot();
        match self.model.apply_edit(p.edit, &p.text) {
            // Typing the value it already has is not an edit: no undo step.
            Ok(false) => {}
            Ok(true) => {
                self.checkpoint_with(before);
                self.changed();
            }
            Err(e) => self.status = e,
        }
    }
    fn prompt_char(&mut self, c: char) {
        let Some(p) = &mut self.prompt else { return };
        let typed = if p.edit.is_text() { !c.is_control() } else { c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E') };
        if typed && p.text.len() < 40 {
            if std::mem::take(&mut p.fresh) {
                p.text.clear();
            }
            p.text.push(c);
        }
    }

    fn do_act(&mut self, a: VesselAct) {
        // A message stays until the next thing the person does.
        self.status.clear();
        match a {
            VesselAct::Row(r) => self.activate_row(r),
            VesselAct::Step(s) => self.step = s,
            VesselAct::PickMaterial(i) => self.pick_material(i),
            VesselAct::NewMaterial => self.new_material(true),
            VesselAct::ListMaterials => self.new_material(false),
            VesselAct::AddMaterial => self.add_material(),
            VesselAct::ClosePicker => self.picker = None,
            VesselAct::Edit(e) => self.begin_edit(e),
            VesselAct::CycleRequirement(i) => self.edit_model(|m| m.cycle_requirement(i)),
            VesselAct::AddRequirement => self.edit_model(|m| m.add_requirement()),
            VesselAct::RemoveRequirement(i) => self.edit_model(|m| m.remove_requirement(i)),
            VesselAct::ToggleKind => self.edit_model(|m| m.toggle_kind()),
            VesselAct::ToggleLineRun => self.edit_model(|m| m.toggle_line_run()),
            VesselAct::AddCase => self.edit_model(|m| m.add_case()),
            VesselAct::AddSupport => self.edit_model(|m| m.add_support()),
            VesselAct::AddLoad => self.edit_model(|m| m.add_load()),
            VesselAct::RemoveCase(i) => self.edit_model(|m| m.remove_case(i)),
            VesselAct::RemoveSupport(i) => self.edit_model(|m| m.remove_support(i)),
            VesselAct::RemoveLoad(i) => self.edit_model(|m| m.remove_load(i)),
            VesselAct::ToggleSupport(i, rotation) => self.edit_model(|m| m.toggle_support(i, rotation)),
            VesselAct::ToggleCaseEnds(i) => self.edit_model(|m| m.toggle_case_ends(i)),
            // Which case is drawn is not an edit: no undo step, the case file keeps it.
            VesselAct::SelectCase(i) => {
                self.model.select_case(i);
                self.changed();
            }
            VesselAct::Undo => self.undo(),
            VesselAct::Redo => self.redo(),
            VesselAct::Save => self.save(),
            VesselAct::Export => self.export(),
        }
    }
}

impl Authoring for VesselSession {
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
        if let EditorAction::Vessel(a) = action {
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
        // The picker's fields take the typing; Esc leaves the form, then the picker.
        if let Some(p) = &mut self.picker {
            if key == Key::Escape {
                if p.add.is_some() {
                    p.add = None;
                } else {
                    self.picker = None;
                }
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
        // Inputs that make no vessel show an empty viewport; the drawing card says why.
        Ok(drawing::scene(&self.model, &self.name).unwrap_or_else(|_| drawing::empty(&self.name)))
    }
    fn needs_scene(&self) -> bool {
        !self.drawn
    }
    fn poll(&mut self) -> bool {
        let answered = match self.walls.poll() {
            Some((revision, answer)) => self.model.set_line_walls(revision, answer),
            None => false,
        };
        self.kick_walls();
        answered
    }
    fn page(&self) -> Vec<DockItem> {
        self.page_items()
    }
    fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
    fn solving(&self) -> bool {
        self.walls.is_running()
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
        format!("Pressure Vessel | {}{}", self.name, if self.dirty { " *" } else { "" })
    }
    fn initial_view(&self) -> Option<StandardView> {
        Some(StandardView::Isometric)
    }
    // The colours are the stress through the wall: a legend in psi, not a colour key.
    fn shows_result(&self) -> bool {
        true
    }
    fn geometry_view(&self) -> bool {
        false
    }
    fn field_unit(&self, label: &str) -> String {
        if label.to_lowercase().contains("stress") { "psi".into() } else { String::new() }
    }
    fn value_card(&self) -> Option<ValueCard> {
        self.picker_card()
    }
    fn card_field(&mut self, index: usize, text: String) {
        self.picker_field(index, text);
    }
    fn restore_recovery(&mut self) -> Result<(), String> {
        let text = std::fs::read_to_string(self.path.with_extension("recovery.json")).map_err(|e| e.to_string())?;
        let f = VesselFile::from_json(&text)?;
        self.undo.push(self.snapshot());
        self.set_from(&f);
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
            "Inputs" if self.model.geometry.is_ok() && self.model.pressure.is_ok() => Done,
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
