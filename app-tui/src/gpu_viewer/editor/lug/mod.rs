//! The Lug Analysis workbench in the native window: the pin-loaded lug of `toolboxes::lug_analysis` (the same model, form
//! rows, solve, checks, mesh-size test and report), with the solved lug contoured on the kernel's own mesh.
//!
//! Inputs live in the inspector (a click types a number, flips a switch or opens the material picker), the results follow
//! every edit in their own cards, the ribbon has the steps Inputs, Results and Report. Nothing here computes a stress:
//! `lug-solver` does through `model::run`, on a worker a moment after the inputs stop changing, and the picture is the
//! nodal field that solve read its results from (`LugRun::field`).
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
use crate::toolboxes::lug_analysis::{
    mesh_test::{self, MeshAdvice},
    model::{self, CachedModel, FieldRow, LugInput, LugRun, LugUiModel},
};
use file::LugFile;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What a click in the Lug Analysis workbench asks for (carried by `EditorAction::Lug`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LugAct {
    /// An input row: type the number, flip the switch, open the material picker or run the mesh test.
    Row(FieldRow),
    /// Bring the tools of a step (the tree).
    Step(&'static str),
    /// Catalog entry `i` (`material_lookup::model::catalog`) for the material the picker is choosing.
    PickMaterial(usize),
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

/// What the analysis worker hands back: the run (or why it failed) and the cache, which returns either way.
type JobAnswer = (Result<LugRun, String>, Option<CachedModel>);

/// Inputs must sit unchanged this long before the analysis runs again (the first run starts at once).
const WAIT: Duration = Duration::from_millis(250);

pub struct LugSession {
    model: LugUiModel,
    path: PathBuf,
    name: String,
    dirty: bool,
    undo: Vec<LugFile>,
    redo: Vec<LugFile>,
    prompt: Option<Prompt>,
    picker: Option<picker::Picker>,
    /// The last completed analysis (kept under a newer edit or a failure until the next one is in).
    run: Option<LugRun>,
    /// The inputs the last analysis failed for, and why: not retried until an edit.
    failed: Option<(LugInput, String)>,
    /// The condensed / kernel models of the last geometry, handed to the next run.
    cache: Option<CachedModel>,
    job: Worker<JobAnswer>,
    job_input: Option<(u64, LugInput)>,
    /// The inputs last seen and since when, so a job starts only once they have settled.
    seen: Option<(LugInput, Instant)>,
    next: u64,
    mesh: Worker<MeshAdvice>,
    /// The mesh test running, with the inputs (`mesh_test::signature`) it is for.
    mesh_job: Option<(u64, String)>,
    mesh_advice: Option<(String, MeshAdvice)>,
    mesh_error: Option<String>,
    status: String,
    step: &'static str,
    saved_revision: Option<String>,
    /// The picture is rebuilt only after an edit or a new answer.
    drawn: bool,
}

const UNDO_LIMIT: usize = 100;

impl LugSession {
    pub fn new(file: &LugFile, path: PathBuf) -> Self {
        let mut model = LugUiModel::default();
        let skipped = file.apply(&mut model);
        let status = if skipped.is_empty() { String::new() } else { format!("Not applied from the file: {}.", skipped.join("; ")) };
        let mut s = Self {
            model,
            name: if file.name.is_empty() { path.file_stem().and_then(|n| n.to_str()).unwrap_or("Lug").to_string() } else { file.name.clone() },
            saved_revision: crate::persistence::revision(&path),
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            prompt: None,
            picker: None,
            run: None,
            failed: None,
            cache: None,
            job: Worker::new("lug-analysis"),
            job_input: None,
            seen: None,
            next: 0,
            mesh: Worker::new("lug-mesh-test"),
            mesh_job: None,
            mesh_advice: None,
            mesh_error: None,
            status,
            step: "Inputs",
            drawn: false,
        };
        // The first analysis starts at once.
        s.start_job();
        s
    }

    /// A session on `path` when it holds a lug case, else on the workbench defaults (saved there by Ctrl/Cmd+S).
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = if path.exists() { LugFile::from_json(&std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?)? } else { LugFile::capture(&LugUiModel::default(), "") };
        Ok(Self::new(&file, path.to_path_buf()))
    }

    pub fn model(&self) -> &LugUiModel {
        &self.model
    }

    // ------------------------------------------------------------------ the analysis

    /// The last completed analysis when it is for exactly the inputs shown.
    pub(crate) fn current_run(&self) -> Option<&LugRun> {
        let input = self.model.input().ok()?;
        self.run.as_ref().filter(|r| r.input == input)
    }
    /// The analysis is running (for the inputs shown or ones being typed).
    pub(super) fn running(&self) -> bool {
        self.job.is_busy()
    }
    /// Why there is no analysis of the inputs shown: they cannot be analysed, or the solver failed on them.
    pub(super) fn error(&self) -> Option<String> {
        match self.model.input() {
            Err(why) => Some(why),
            Ok(input) => self.failed.as_ref().filter(|(i, _)| *i == input).map(|(_, why)| format!("analysis failed: {why}")),
        }
    }
    /// Start the analysis of the inputs shown once they have settled (at once when nothing has been analysed yet). A
    /// job for older inputs is not cancelled: its cache is wanted, and the next job starts when it ends.
    fn start_job(&mut self) {
        let Ok(input) = self.model.input() else {
            self.seen = None;
            return;
        };
        if self.seen.as_ref().is_none_or(|(s, _)| *s != input) {
            self.seen = Some((input.clone(), Instant::now()));
        }
        let first = self.run.is_none() && self.failed.is_none();
        let settled = self.seen.as_ref().is_some_and(|(_, at)| at.elapsed() >= WAIT);
        let failed = self.failed.as_ref().is_some_and(|(i, _)| *i == input);
        if self.job.is_busy() || self.current_run().is_some() || failed || !(settled || first) {
            return;
        }
        self.next += 1;
        let tag = self.next;
        let (job, cache) = (input.clone(), self.cache.clone());
        match self.job.start(tag, move || Ok(model::run(&job, cache))) {
            Ok(()) => self.job_input = Some((tag, input)),
            Err(e) => self.status = format!("The analysis did not start: {e}."),
        }
    }
    /// Take the analysis worker's answer: the cache is kept either way; the run or the failure only when it is for
    /// the inputs shown (one for older inputs is superseded by the job that follows).
    fn take_answer(&mut self, tag: u64, answer: Result<JobAnswer, String>) -> bool {
        let Some((_, input)) = self.job_input.take_if(|(t, _)| *t == tag) else { return false };
        let (result, cache) = answer.unwrap_or_else(|why| (Err(why), None));
        if cache.is_some() {
            self.cache = cache;
        }
        if self.model.input().ok().as_ref() != Some(&input) {
            return false;
        }
        match result {
            Ok(run) => {
                self.run = Some(run);
                self.failed = None;
            }
            Err(why) => self.failed = Some((input, why)),
        }
        true
    }

    /// The mesh test row: apply a recommendation made for these inputs, else start the test.
    fn mesh_test(&mut self) {
        let Ok(input) = self.model.input() else {
            self.status = "The mesh test needs inputs that can be analysed.".into();
            return;
        };
        let sig = mesh_test::signature(&input);
        if let Some((s, advice)) = &self.mesh_advice {
            if *s == sig {
                let n = advice.recommended;
                if self.model.elements_around != Some(n) {
                    self.checkpoint();
                    self.model.elements_around = Some(n);
                    self.changed();
                }
                self.status = format!("{n} elements around the bore applied.");
                return;
            }
        }
        if self.mesh.is_busy() {
            return;
        }
        self.next += 1;
        let tag = self.next;
        match self.mesh.start(tag, move || mesh_test::run(&input)) {
            Ok(()) => {
                self.mesh_job = Some((tag, sig));
                self.mesh_error = None;
            }
            Err(e) => self.status = format!("The mesh test did not start: {e}."),
        }
    }
    /// The mesh test row's value: its progress, its recommendation, or how to run it.
    pub(super) fn mesh_advice_text(&self) -> String {
        if self.mesh_job.is_some() {
            return format!("Testing mesh sizes ({:.1} s)", self.mesh.elapsed().unwrap_or_default().as_secs_f64());
        }
        if let Some(why) = &self.mesh_error {
            return format!("Failed: {why}");
        }
        let current = self.model.input().ok().map(|i| mesh_test::signature(&i));
        match (&self.mesh_advice, current) {
            (Some((s, advice)), Some(cur)) if *s == cur => advice.summary(),
            (Some(_), _) => "Inputs changed since the test: click runs it again".into(),
            _ => "Not run: click runs a brief test".into(),
        }
    }

    // ------------------------------------------------------------------ editing

    fn snapshot(&self) -> LugFile {
        LugFile::capture(&self.model, &self.name)
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
    }
    fn set_from(&mut self, file: &LugFile) {
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
    /// The plain-text report (`model::report_text`) of the analysis of the inputs shown, beside the case file.
    fn export(&mut self) {
        let Some(run) = self.current_run() else {
            self.status = "Nothing to export yet: the analysis of these inputs has not finished.".into();
            return;
        };
        let target = self.path.with_extension("report.txt");
        match crate::persistence::write(&target, model::report_text(run)) {
            Ok(()) => self.status = format!("Report written to {}.", target.display()),
            Err(e) => self.status = format!("Could not write {}: {e}", target.display()),
        }
    }

    // ------------------------------------------------------------------ rows and prompts

    /// A click (or Enter) on an input row.
    fn activate_row(&mut self, row: FieldRow) {
        let edit: fn(&mut LugUiModel) = match row {
            FieldRow::Header(_) => return,
            FieldRow::Number(target) => {
                self.picker = None;
                self.prompt = Some(Prompt { row, text: model::format_for_edit(self.model.number_value(target)), fresh: true });
                return;
            }
            FieldRow::OpenMaterialPicker => return self.open_picker(picker::Target::Lug),
            FieldRow::OpenBushingMaterialPicker => return self.open_picker(picker::Target::Bushing),
            // Showing more rows is not an edit of the lug.
            FieldRow::MeshSection => {
                self.model.mesh_open = !self.model.mesh_open;
                return;
            }
            FieldRow::MeshAdvice => {
                self.prompt = None;
                return self.mesh_test();
            }
            FieldRow::ToggleHeadShape => LugUiModel::toggle_head_shape,
            FieldRow::ToggleBushing => LugUiModel::toggle_bushing,
            // A preset replaces a typed element count.
            FieldRow::ToggleMeshDensity => |m| {
                m.density = m.density.next();
                m.elements_around = None;
            },
            FieldRow::ToggleAutoRefine => |m| m.auto_refine = !m.auto_refine,
            FieldRow::ToggleSolver => |m| m.solver = m.solver.next(),
            FieldRow::TogglePlastic => |m| m.plastic = !m.plastic,
            FieldRow::ToggleFlowRule => |m| m.flow_rule = m.flow_rule.next(),
            FieldRow::TogglePinBody => |m| m.pin_body = m.pin_body.next(),
            FieldRow::ToggleFiniteStrain => |m| m.finite_strain = !m.finite_strain,
            FieldRow::ToggleSecondOrder => |m| m.second_order = !m.second_order,
            FieldRow::TogglePinBending => |m| m.pin_bending = !m.pin_bending,
        };
        self.prompt = None;
        self.checkpoint();
        edit(&mut self.model);
        self.changed();
    }
    fn commit_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        let FieldRow::Number(target) = p.row else { return };
        let before = self.model.number_value(target);
        match p.text.trim().parse::<f64>() {
            // Typing the value it already has is not an edit: no undo step.
            Ok(v) if v.is_finite() && v == before => {}
            Ok(v) if v.is_finite() => {
                // The model keeps the old value for one it refuses; say so instead of recording an edit.
                let mut edited = self.model.clone();
                edited.commit_number(target, v);
                if edited.number_value(target) == before {
                    self.status = format!("{} not changed: {v} is out of range.", target.label());
                    return;
                }
                self.checkpoint();
                self.model = edited;
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

    fn do_act(&mut self, a: LugAct) {
        // A message stays until the next thing the person does.
        self.status.clear();
        match a {
            LugAct::Row(r) => self.activate_row(r),
            LugAct::Step(s) => self.step = s,
            LugAct::PickMaterial(i) => self.pick_material(i),
            LugAct::ClosePicker => self.picker = None,
            LugAct::Undo => self.undo(),
            LugAct::Redo => self.redo(),
            LugAct::Save => self.save(),
            LugAct::Export => self.export(),
        }
    }
}

impl Authoring for LugSession {
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
        if let EditorAction::Lug(a) = action {
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
    // The picture is the lug drawn; a click on it picks nothing.
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        self.drawn = true;
        // Inputs that make no lug show an empty viewport; the drawing card says why.
        Ok(drawing::scene(&self.model, &self.name, self.current_run()).unwrap_or_else(|_| drawing::empty(&self.name)))
    }
    fn needs_scene(&self) -> bool {
        !self.drawn
    }
    /// Take the analysis and mesh-test answers when they arrive and start the next analysis once the inputs have
    /// settled. The window polls at least every 250 ms.
    fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some((tag, answer)) = self.job.poll() {
            changed |= self.take_answer(tag, answer);
        }
        if let Some((tag, answer)) = self.mesh.poll() {
            if let Some((_, sig)) = self.mesh_job.take_if(|(t, _)| *t == tag) {
                match answer {
                    Ok(advice) => self.mesh_advice = Some((sig, advice)),
                    Err(why) => self.mesh_error = Some(why),
                }
                changed = true;
            }
        }
        self.start_job();
        if changed {
            self.drawn = false;
        }
        // The mesh test's clock ticks in its row.
        changed || self.mesh_job.is_some()
    }
    fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
    // The analysis runs in the background; nothing waits on it.
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
        format!("Lug Analysis | {}{}", self.name, if self.dirty { " *" } else { "" })
    }
    // The lug is flat in the xy plane: the front view looks straight at it.
    fn initial_view(&self) -> Option<StandardView> {
        Some(StandardView::Front)
    }
    // Contoured once the analysis of these inputs is in with its field; until then the parts in the colour key.
    fn shows_result(&self) -> bool {
        self.current_run().is_some_and(|r| r.field.is_some())
    }
    fn geometry_view(&self) -> bool {
        !self.shows_result()
    }
    fn field_unit(&self, label: &str) -> String {
        let label = label.to_lowercase();
        if label.contains("stress") {
            "psi".into()
        } else if label.contains("displacement") {
            "in".into()
        } else {
            String::new()
        }
    }
    fn value_card(&self) -> Option<ValueCard> {
        self.picker_card()
    }
    fn card_field(&mut self, index: usize, text: String) {
        self.picker_field(index, text);
    }
    fn restore_recovery(&mut self) -> Result<(), String> {
        let text = std::fs::read_to_string(self.path.with_extension("recovery.json")).map_err(|e| e.to_string())?;
        let f = LugFile::from_json(&text)?;
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
            "Inputs" if self.model.input().is_ok() => Done,
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
