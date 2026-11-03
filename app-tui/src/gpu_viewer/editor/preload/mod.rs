//! The Preload Analysis workbench in the native window: the fastened-joint preload of `toolboxes::preload_analysis`
//! (the same model, form rows, solve, bolt catalog, joint templates and report), with the clamped joint drawn to scale
//! as the finite-element model of its members sees it.
//!
//! Inputs live in the inspector (a click types a number, flips a switch or opens the bolt or template picker), the
//! results follow every edit in their own cards, the ribbon has the steps Inputs, Results and Report. Nothing here
//! computes a preload or a stress: `fastened-joint-solver` does through `PreloadModel`, and the member stack is solved
//! on the general kernel (`fe_check::run_with_field`, on a worker a moment after the joint stops changing). That one
//! solve gives the member-compliance cross-check, the FE-equivalent cone angle the solver can use, and the stress the
//! picture is coloured by.
mod cards;
pub mod drawing;
pub mod file;
mod picker;

use super::{
    authoring::{Authoring, Card},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
    worker::Worker,
};
use crate::gpu_viewer::{
    bench::Bench,
    controller::StandardView,
    menu::{Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode, ValueCard},
    Scene,
};
use crate::toolboxes::preload_analysis::{
    fe_check::{self, FeInput, FeResult},
    model::{self, FieldRow, PreloadModel},
    report,
};
use fea_problem::joint::MemberField;
use file::PreloadFile;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What a click in the Preload Analysis workbench asks for (carried by `EditorAction::Preload`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreloadAct {
    /// An input row: type the number, flip the switch, or open a picker.
    Row(FieldRow),
    /// Bring the tools of a step (the tree).
    Step(&'static str),
    /// Entry `entry` of catalog section `section` (`model::BOLT_SECTIONS`).
    PickBolt(usize, usize),
    /// The `i`th preset of `joint_templates::presets`.
    ApplyTemplate(usize),
    /// Apply the next random joint; the picker stays open for another.
    RandomJoint,
    ClosePicker,
    Undo,
    Redo,
    Save,
    Export,
}

struct Prompt {
    row: FieldRow,
    text: String,
    /// Nothing typed yet: the first character replaces the value instead of extending it.
    fresh: bool,
}

/// One solve of the member stack: the cross-check and the field it measured.
type FeAnswer = Result<(FeResult, MemberField), String>;

/// The member-stack FE: the job running, the last answer and the joint it is for, and the wait before a new job.
struct FeState {
    worker: Worker<(FeResult, MemberField)>,
    job: Option<(u64, FeInput)>,
    result: Option<(FeInput, FeAnswer)>,
    /// The joint last seen and since when, so a job starts only once the inputs have settled.
    seen: Option<(FeInput, Instant)>,
    next: u64,
}

/// Inputs must sit unchanged this long before the member FE runs again (the first run starts at once).
const FE_WAIT: Duration = Duration::from_millis(400);

pub struct PreloadSession {
    model: PreloadModel,
    path: PathBuf,
    name: String,
    dirty: bool,
    undo: Vec<PreloadFile>,
    redo: Vec<PreloadFile>,
    prompt: Option<Prompt>,
    picker: Option<picker::Picker>,
    fe: FeState,
    /// Random joints: the sequence's base (a clock reading) and how many were drawn.
    random_base: u64,
    randoms: u64,
    status: String,
    step: &'static str,
    saved_revision: Option<String>,
    /// The picture is rebuilt only after an edit or a new FE answer.
    drawn: bool,
    /// Where the members' surfaces begin in the drawn scene's triangles (`drawing::scene`); `None` with nothing drawn.
    translucent_from: Option<u32>,
}

const UNDO_LIMIT: usize = 100;

impl PreloadSession {
    pub fn new(file: &PreloadFile, path: PathBuf) -> Self {
        let mut model = PreloadModel::default();
        let skipped = file.apply(&mut model);
        let status = if skipped.is_empty() { String::new() } else { format!("Not applied from the file: {}.", skipped.join("; ")) };
        let mut s = Self {
            model,
            name: if file.name.is_empty() { path.file_stem().and_then(|n| n.to_str()).unwrap_or("Preload").to_string() } else { file.name.clone() },
            saved_revision: crate::persistence::revision(&path),
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            prompt: None,
            picker: None,
            fe: FeState { worker: Worker::new("preload-member-fe"), job: None, result: None, seen: None, next: 0 },
            random_base: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64),
            randoms: 0,
            status,
            step: "Inputs",
            drawn: false,
            translucent_from: None,
        };
        // The first solve of the members starts at once.
        s.start_fe();
        s
    }

    /// A session on `path` when it holds a preload case, else on the workbench defaults (saved there by Ctrl/Cmd+S).
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = if path.exists() { PreloadFile::from_json(&std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?)? } else { PreloadFile::capture(&PreloadModel::default(), "") };
        Ok(Self::new(&file, path.to_path_buf()))
    }

    pub fn model(&self) -> &PreloadModel {
        &self.model
    }

    // ------------------------------------------------------------------ the member FE

    /// The member FE answer for exactly the joint shown, when there is one.
    pub(crate) fn fe_current(&self) -> Option<&FeAnswer> {
        let input = self.model.fe_input()?;
        self.fe.result.as_ref().filter(|(i, _)| *i == input).map(|(_, r)| r)
    }
    /// The cross-check for exactly the joint shown, as the report takes it.
    fn fe_check_current(&self) -> Option<Result<FeResult, String>> {
        self.fe_current().map(|r| r.as_ref().map(|(f, _)| *f).map_err(Clone::clone))
    }
    /// The member FE is running for the joint shown (or one being typed).
    pub(super) fn fe_running(&self) -> bool {
        self.fe.worker.is_busy()
    }
    /// Keep the model's FE-equivalent angle that of exactly the joint shown (cleared while the answer is pending or
    /// failed); returns whether it changed. `FeInput` excludes the solver's angle, so this cannot feed back.
    fn sync_fe_angle(&mut self) -> bool {
        let angle = self.fe_current().and_then(|r| r.as_ref().ok().and_then(|(r, _)| r.equivalent_angle_deg));
        self.model.set_fe_angle(angle)
    }
    /// Start the member FE for the joint shown once its inputs have settled (at once when there is no answer yet).
    fn start_fe(&mut self) {
        let Some(input) = self.model.fe_input() else {
            self.fe.seen = None;
            return;
        };
        if self.fe.seen.as_ref().is_none_or(|(s, _)| *s != input) {
            self.fe.seen = Some((input.clone(), Instant::now()));
        }
        let settled = self.fe.seen.as_ref().is_some_and(|(_, at)| at.elapsed() >= FE_WAIT);
        if self.fe.worker.is_busy() || self.fe_current().is_some() || !(settled || self.fe.result.is_none()) {
            return;
        }
        self.fe.next += 1;
        let tag = self.fe.next;
        let job = input.clone();
        match self.fe.worker.start(tag, move || fe_check::run_with_field(&job)) {
            Ok(()) => self.fe.job = Some((tag, input)),
            Err(e) => self.status = format!("The member finite-element model did not start: {e}."),
        }
    }

    // ------------------------------------------------------------------ editing

    fn snapshot(&self) -> PreloadFile {
        PreloadFile::capture(&self.model, &self.name)
    }
    /// Record the state before an edit so Undo can return to it.
    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    fn changed(&mut self) {
        self.dirty = true;
        self.drawn = false;
        // An answer already in hand for the new joint (an undo back to it) applies at once.
        self.sync_fe_angle();
    }
    fn set_from(&mut self, file: &PreloadFile) {
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
    /// The plain-text report (with the member cross-check for this joint), beside the case file.
    fn export(&mut self) {
        let target = self.path.with_extension("report.txt");
        let fe = self.fe_check_current();
        match crate::persistence::write(&target, report::build_report_text(&self.model, fe.as_ref())) {
            Ok(()) => self.status = format!("Report written to {}.", target.display()),
            Err(e) => self.status = format!("Could not write {}: {e}", target.display()),
        }
    }

    // ------------------------------------------------------------------ rows and prompts

    /// A click (or Enter) on an input row.
    fn activate_row(&mut self, row: FieldRow) {
        let edit: fn(&mut PreloadModel) = match row {
            FieldRow::Header(_) => return,
            FieldRow::Number(target) => {
                self.picker = None;
                self.prompt = Some(Prompt { row, text: model::format_for_edit(self.model.number_value(target)), fresh: true });
                return;
            }
            FieldRow::OpenBoltPicker => return self.open_picker(picker::Picker::Bolt { query: String::new() }),
            FieldRow::OpenTemplatePicker => return self.open_picker(picker::Picker::Template { last: None }),
            // Showing more rows is not an edit of the joint.
            FieldRow::AdvancedSection => {
                self.model.advanced_open = !self.model.advanced_open;
                return;
            }
            FieldRow::ToggleMode => PreloadModel::toggle_mode,
            FieldRow::ToggleTighteningFrom => PreloadModel::toggle_tightening_from,
            FieldRow::ToggleBearingModel => PreloadModel::toggle_bearing_model,
            FieldRow::ToggleMemberStiffness => PreloadModel::toggle_member_stiffness,
            FieldRow::ToggleExternalLoadEnabled => PreloadModel::toggle_external_load_enabled,
            FieldRow::ToggleSlipEnabled => PreloadModel::toggle_slip_enabled,
            FieldRow::ToggleStrengthLimitsEnabled => PreloadModel::toggle_strength_limits_enabled,
            FieldRow::ToggleAddMember => PreloadModel::add_member,
            FieldRow::ToggleRemoveMember => PreloadModel::remove_member,
            FieldRow::ToggleUncertaintyEnabled => PreloadModel::toggle_uncertainty_enabled,
            FieldRow::ToggleMonteCarloEnabled => PreloadModel::toggle_monte_carlo_enabled,
            FieldRow::ToggleThreadLoadDistributionEnabled => PreloadModel::toggle_thread_load_distribution_enabled,
        };
        self.prompt = None;
        self.checkpoint();
        edit(&mut self.model);
        self.changed();
    }
    fn commit_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        let FieldRow::Number(target) = p.row else { return };
        match p.text.trim().parse::<f64>() {
            // Typing the value it already has is not an edit: no undo step.
            Ok(v) if v.is_finite() && v == self.model.number_value(target) => {}
            Ok(v) if v.is_finite() => {
                self.checkpoint();
                self.model.commit_number(target, v);
                self.changed();
            }
            _ => self.status = format!("{} not changed: {:?} is not a number.", target.label(), p.text.trim()),
        }
    }
    fn prompt_char(&mut self, c: char) {
        let Some(p) = &mut self.prompt else { return };
        if (c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')) && p.text.len() < 40 {
            if std::mem::take(&mut p.fresh) {
                p.text.clear();
            }
            p.text.push(c);
        }
    }

    fn do_act(&mut self, a: PreloadAct) {
        // A message stays until the next thing the person does.
        self.status.clear();
        match a {
            PreloadAct::Row(r) => self.activate_row(r),
            PreloadAct::Step(s) => self.step = s,
            PreloadAct::PickBolt(section, entry) => self.pick_bolt(section, entry),
            PreloadAct::ApplyTemplate(i) => self.apply_preset(i),
            PreloadAct::RandomJoint => self.random_joint(),
            PreloadAct::ClosePicker => self.picker = None,
            PreloadAct::Undo => self.undo(),
            PreloadAct::Redo => self.redo(),
            PreloadAct::Save => self.save(),
            PreloadAct::Export => self.export(),
        }
    }
}

impl Authoring for PreloadSession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn key_labels(&self) -> [&'static str; 4] {
        drawing::KEY_LABELS
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::Preload(a) = action {
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
        // The picker's search field takes the typing; Esc closes it.
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
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
    // The picture is the joint drawn; a click on it picks nothing.
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        self.drawn = true;
        let solved = self.fe_current().and_then(|r| r.as_ref().ok()).zip(self.model.output.as_ref().ok()).map(|((fe, field), solution)| drawing::Solved { field, fe, solution });
        // Members that make no joint show an empty viewport; the drawing card says why.
        let (scene, translucent_from) = match drawing::scene(&self.model, &self.name, solved) {
            Ok((scene, first)) => (scene, Some(first)),
            Err(_) => (drawing::empty(&self.name), None),
        };
        self.translucent_from = translucent_from;
        Ok(scene)
    }
    fn needs_scene(&self) -> bool {
        !self.drawn
    }
    // The members around the bolt, so it shows through them.
    fn translucent(&self) -> Option<(u32, f32)> {
        self.translucent_from.map(|first| (first, drawing::MEMBER_OPACITY))
    }
    /// Take the member FE's answer when it arrives, keep the solver's FE angle in step, and start the next solve once
    /// the joint has settled. The window polls at least every 250 ms.
    fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some((tag, answer)) = self.fe.worker.poll() {
            if let Some((_, input)) = self.fe.job.take_if(|(t, _)| *t == tag) {
                self.fe.result = Some((input, answer));
                changed = true;
            }
        }
        changed |= self.sync_fe_angle();
        self.start_fe();
        if changed {
            self.drawn = false;
        }
        changed
    }
    fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
    // The member FE runs in the background; nothing waits on it.
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
        format!("Preload Analysis | {}{}", self.name, if self.dirty { " *" } else { "" })
    }
    fn initial_view(&self) -> Option<StandardView> {
        Some(StandardView::Isometric)
    }
    // Coloured by the member FE's stress once it is in for this joint; until then the parts in the colour key.
    fn shows_result(&self) -> bool {
        self.fe_current().is_some_and(|r| r.is_ok())
    }
    fn geometry_view(&self) -> bool {
        !self.shows_result()
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
        let f = PreloadFile::from_json(&text)?;
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
            "Inputs" if self.model.output.is_ok() => Done,
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
