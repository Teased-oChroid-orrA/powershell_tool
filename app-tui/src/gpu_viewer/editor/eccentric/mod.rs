//! The Eccentric Bushing workbench in the native window: spin capacity of a bushing whose bore is offset from its outer
//! diameter, and the largest offset or pin load the interference fit still holds. The analysis, its runs, queue,
//! history and report are the terminal toolbox's own (`toolboxes::eccentric_bushing`, over the `eccentric-bushing`
//! solver); the bore, ID, interference, friction, length, load, materials and edge distance are read from a Bushing case
//! file, so nothing is entered twice. Before an analysis the picture is the housing, the offset bushing and the pin load
//! as an FEA problem (`fe`); after it, the solved model of the analysis itself (`vtu`): the housing, bushing and pin with
//! their stresses.
mod cards;
pub mod fe;
pub mod file;
pub mod vtu;

use super::{
    authoring::{Authoring, Card, Overlay},
    bushing::file::BushingFile,
    camera::Camera,
    handoff,
    session::{EditorAction, Key, KeyResult, Mods},
    solid::{SolidAction, SolidSession},
};
use crate::app::Effect;
use crate::gpu_viewer::{
    bench::Bench,
    menu::{Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode},
    Scene,
};
use crate::toolboxes::bushing::model::BushingModel;
use crate::toolboxes::eccentric_bushing::{
    model::{self, FieldRow, Inputs, Output, Task},
    EccentricState,
};
use file::EccentricFile;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// What a click in the Eccentric Bushing workbench asks for (carried by `EditorAction::Eccentric`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EccentricAct {
    /// An input row: edit the number, flip the choice, open or close the advanced rows, or start a run.
    Row(FieldRow),
    Step(&'static str),
    /// Analyse this offset (the solved model is drawn afterwards).
    Analyse,
    Cancel,
    ReloadBushing,
    OpenBushing,
    /// Open the case in the FEA workbench (saved first); it comes back through the FEA workbench's Send.
    OpenInFea,
    ToggleProfile,
    ToggleResult,
    ExportReport,
    ExportCsv,
    ExportFields,
    Undo,
    Redo,
    Save,
}

struct Prompt {
    row: FieldRow,
    text: String,
    fresh: bool,
}

type Answer = Result<Output, String>;

pub struct EccentricSession {
    st: EccentricState,
    bushing: BushingModel,
    /// Where the bushing inputs came from, or why the defaults are in use.
    bushing_note: String,
    bushing_case: String,
    fea: SolidSession,
    path: PathBuf,
    name: String,
    dirty: bool,
    undo: Vec<model::EccentricUi>,
    redo: Vec<model::EccentricUi>,
    prompt: Option<Prompt>,
    status: String,
    fe_note: Option<String>,
    fe_key: Option<fea_problem::Problem>,
    mesh_wanted: bool,
    jobs: Vec<(u64, Receiver<Answer>)>,
    /// The `.vtu` text of the last analysis and the inputs it was solved for.
    vtu: Option<(Inputs, String)>,
    /// Show the solved model rather than the picture of the inputs (when the solved one is current).
    show_result: bool,
    showing_result: bool,
    /// The next field export is also written to a file.
    export_fields: bool,
    version: u64,
    built: Option<u64>,
    step: &'static str,
    /// Modification times of the case file and of the Bushing case when this window last read or wrote them.
    stamp: Option<std::time::SystemTime>,
    saved_revision: Option<String>,
    bushing_stamp: Option<std::time::SystemTime>,
    beat_at: std::time::Instant,
    watch_at: std::time::Instant,
}

impl Drop for EccentricSession {
    fn drop(&mut self) {
        handoff::clear_beat(&self.path);
    }
}

const UNDO_LIMIT: usize = 100;

impl EccentricSession {
    pub fn new(file: &EccentricFile, path: PathBuf) -> Self {
        let mut st = EccentricState::default();
        st.ui = file.ui();
        let blank = fea_problem::templates::templates().remove(4).1;
        let mut fea = SolidSession::new(blank, path.with_extension("fe.json"));
        fea.highlight_bushings = true;
        fea.do_action(SolidAction::ToggleRefineShow);
        let mut s = Self {
            st,
            bushing: BushingModel::default(),
            bushing_note: String::new(),
            bushing_case: file.bushing_case.clone(),
            fea,
            name: if file.name.is_empty() { path.file_stem().and_then(|n| n.to_str()).unwrap_or("Eccentric bushing").to_string() } else { file.name.clone() },
            path,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            prompt: None,
            status: String::new(),
            fe_note: None,
            fe_key: None,
            mesh_wanted: false,
            jobs: Vec::new(),
            vtu: None,
            show_result: true,
            showing_result: false,
            export_fields: false,
            version: 1,
            built: None,
            step: "Inputs",
            stamp: None,
            saved_revision: None,
            bushing_stamp: None,
            beat_at: std::time::Instant::now(),
            watch_at: std::time::Instant::now(),
        };
        s.stamp = handoff::stamp(&s.path);
        s.saved_revision = crate::persistence::revision(&s.path);
        s.reload_bushing();
        s
    }

    /// A session on `path` when it holds an eccentric case, else on the defaults (saved there by Ctrl/Cmd+S).
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = if path.exists() {
            EccentricFile::from_json(&std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?)?
        } else {
            EccentricFile::new("", crate::gpu_viewer::EditorKind::Bushing.default_file(), &model::EccentricUi::default())
        };
        Ok(Self::new(&file, path.to_path_buf()))
    }

    pub fn ui(&self) -> &model::EccentricUi {
        &self.st.ui
    }
    pub fn state(&self) -> &EccentricState {
        &self.st
    }
    pub fn bushing(&self) -> &BushingModel {
        &self.bushing
    }
    pub fn fea(&self) -> &SolidSession {
        &self.fea
    }

    // ------------------------------------------------------------------ the Bushing case

    fn case_file(&self) -> EccentricFile {
        EccentricFile::new(&self.name, &self.bushing_case, &self.st.ui)
    }
    fn case_path(&self) -> PathBuf {
        self.case_file().case_path(&self.path)
    }
    /// Read the Bushing case the inputs come from; without one the Bushing workbench's defaults stand, and the note says so.
    fn reload_bushing(&mut self) {
        let path = self.case_path();
        let mut model = BushingModel::default();
        self.bushing_note = match std::fs::read_to_string(&path) {
            Ok(text) => match BushingFile::from_json(&text) {
                Ok(f) => {
                    let skipped = f.apply(&mut model);
                    if skipped.is_empty() { format!("from {}", path.display()) } else { format!("from {} (not applied: {})", path.display(), skipped.join("; ")) }
                }
                Err(why) => format!("{} is not usable ({why}); the Bushing defaults stand", path.display()),
            },
            Err(_) => format!("no Bushing case at {}; the Bushing defaults stand", path.display()),
        };
        self.bushing = model;
        self.bushing_stamp = handoff::stamp(&path);
        self.inputs_changed();
    }
    /// Hand the case to the FEA workbench: saved first (what comes back goes into it), the derived model written.
    fn open_in_fea(&mut self) {
        if self.dirty || !self.path.exists() {
            self.save();
            if self.dirty {
                return;
            }
        }
        self.status = match fe::preview_problem(&self.bushing, &self.st.ui).and_then(|p| handoff::open_in_fea(&self.path, crate::gpu_viewer::EditorKind::Eccentric, &p)) {
            Ok(msg) => msg,
            Err(e) => format!("Could not open the FEA workbench: {e}"),
        };
    }
    /// Make the next `poll` look at the files (tests; otherwise it looks once a second).
    #[cfg(test)]
    pub(crate) fn make_watch_due(&mut self) {
        self.watch_at = std::time::Instant::now() - std::time::Duration::from_secs(5);
    }
    /// Once a second: keep the heartbeat while this case is linked to an FEA file, and take in what the FEA workbench's Send
    /// wrote: the Bushing case it read is simply reloaded, this case's own inputs unless there are unsaved edits.
    fn watch(&mut self) -> bool {
        if self.watch_at.elapsed() < std::time::Duration::from_secs(1) {
            return false;
        }
        self.watch_at = std::time::Instant::now();
        if handoff::Link::exists(&handoff::fea_path(&self.path)) && self.beat_at.elapsed() >= std::time::Duration::from_secs(2) {
            self.beat_at = std::time::Instant::now();
            handoff::beat(&self.path);
        }
        let mut changed = false;
        let own = handoff::stamp(&self.path);
        if own != self.stamp {
            self.stamp = own;
            changed = true;
            let text = own.and_then(|_| std::fs::read_to_string(&self.path).ok());
            if self.dirty {
                self.status = "The case file changed on disk (the FEA workbench sent a model); this window has unsaved edits: save to keep them, or reopen the file for the new inputs.".into();
            } else if let Some(text) = text {
                match EccentricFile::from_json(&text) {
                    Ok(f) => {
                        self.undo.push(self.st.ui.clone());
                        self.redo.clear();
                        self.st.ui = f.ui();
                        self.bushing_case = f.bushing_case;
                        self.reload_bushing();
                        self.dirty = false;
                        self.saved_revision = crate::persistence::revision(&self.path);
                        self.status = "Updated from the FEA workbench.".into();
                    }
                    Err(why) => {
                        self.status = format!("The case file changed on disk but cannot be read: {why}")
                    }
                }
            }
        }
        if handoff::stamp(&self.case_path()) != self.bushing_stamp {
            changed = true;
            self.reload_bushing();
            self.status = "The Bushing case changed on disk: reloaded.".into();
        }
        changed
    }
    fn open_bushing(&mut self) {
        let case = self.case_path();
        let spawned = handoff::launch(crate::gpu_viewer::EditorKind::Bushing, &case);
        self.status = match spawned {
            Ok(()) => format!("Opened {} in the Bushing workbench; save it there, then reload here.", case.display()),
            Err(e) => format!("Could not open the Bushing workbench: {e}"),
        };
    }

    // ------------------------------------------------------------------ editing

    fn push_undo(&mut self, before: model::EccentricUi) {
        self.undo.push(before);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    /// An input changed (ours or the Bushing case's): the picture follows, a stored result is stale by its inputs.
    fn inputs_changed(&mut self) {
        self.version += 1;
        match fe::preview_problem(&self.bushing, &self.st.ui) {
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
    fn edited(&mut self) {
        self.dirty = true;
        self.st.error = None;
        self.inputs_changed();
    }
    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.st.ui, prev));
            self.edited();
        }
    }
    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.st.ui, next));
            self.edited();
        }
    }
    fn save(&mut self) {
        if crate::persistence::revision(&self.path) != self.saved_revision {
            self.status = "Save conflict: file changed externally; reopen before saving.".into();
            return;
        }
        match crate::persistence::write(&self.path, self.case_file().to_json()) {
            Ok(()) => {
                self.dirty = false;
                self.stamp = handoff::stamp(&self.path);
                self.saved_revision = crate::persistence::revision(&self.path);
                self.status = format!("Saved {}.", self.path.display());
            }
            Err(e) => self.status = format!("Could not save {}: {e}", self.path.display()),
        }
    }
    fn write(&mut self, target: PathBuf, text: Option<String>, what: &str) {
        match text {
            None => self.status = format!("Nothing to export: run the {what} first."),
            Some(t) => match crate::persistence::write(&target, t) {
                Ok(()) => self.status = format!("Wrote {}.", target.display()),
                Err(e) => self.status = format!("Could not write {}: {e}", target.display()),
            },
        }
    }

    fn rows(&self) -> Vec<FieldRow> {
        model::field_rows(self.st.ui.advanced_open)
    }
    fn begin_prompt(&mut self, row: FieldRow, text: String) {
        self.prompt = Some(Prompt { row, text, fresh: true });
    }
    fn activate_row(&mut self, row: FieldRow) {
        match row {
            FieldRow::Header(_) => {}
            FieldRow::Number(t) => self.begin_prompt(row, model::format_for_edit(self.st.ui.number_value(t))),
            FieldRow::Run(Task::Analyze) => self.analyse(),
            FieldRow::Run(task) => {
                let effects = self.st.start(&self.bushing, task);
                self.after_start(effects);
            }
            // The switches work whether or not the Advanced rows are open (the ribbon offers some of them).
            switch => {
                let before = self.st.ui.clone();
                let ui = &mut self.st.ui;
                match switch {
                    FieldRow::TogglePlane => ui.plane_strain = !ui.plane_strain,
                    FieldRow::ToggleHousing => ui.edge_limited = !ui.edge_limited,
                    FieldRow::ToggleDirectOnset => ui.direct_onset = !ui.direct_onset,
                    FieldRow::TogglePinCredit => ui.credit_pin_load = !ui.credit_pin_load,
                    FieldRow::AdvancedSection => ui.advanced_open = !ui.advanced_open,
                    _ => {}
                }
                if self.st.ui != before {
                    self.push_undo(before);
                    self.edited();
                }
            }
        }
    }
    fn commit_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        let FieldRow::Number(t) = p.row else { return };
        match p.text.trim().parse::<f64>() {
            Ok(v) if v.is_finite() => {
                let before = self.st.ui.clone();
                self.st.ui.commit_number(t, v);
                // The value a field clamps to may be the one it had: that is no edit.
                if self.st.ui != before {
                    self.push_undo(before);
                    self.edited();
                }
            }
            _ => self.status = format!("{} not changed: {:?} is not a number.", t.label(), p.text.trim()),
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

    // ------------------------------------------------------------------ runs

    /// Analyse the offset: one solve gives the analysis and the solved model the window draws.
    fn analyse(&mut self) {
        let effects = self.st.start(&self.bushing, Task::Fields);
        self.after_start(effects);
    }
    fn after_start(&mut self, effects: Vec<Effect>) {
        self.status.clear();
        self.run_effects(effects);
    }
    fn run_effects(&mut self, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::RunEccentric { id, task, input, control } => {
                    let (tx, rx) = channel();
                    let spawned = std::thread::Builder::new().name("eccentric-bushing".into()).spawn(move || {
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| model::run_controlled(task, &input, &control))).unwrap_or_else(|_| Err("the solver stopped unexpectedly".into()));
                        let _ = tx.send(r);
                    });
                    match spawned {
                        Ok(_) => self.jobs.push((id, rx)),
                        Err(e) => {
                            self.st.job = None;
                            self.status = format!("Could not start the solver thread: {e}");
                        }
                    }
                }
                // The field export a finished analysis hands back: written only when it was asked for.
                Effect::WriteTextFile { path, contents } if self.export_fields => {
                    self.export_fields = false;
                    let target = self.path.with_extension("vtu");
                    let _ = path;
                    self.write(target, Some(contents), "analysis");
                }
                _ => {}
            }
        }
    }
    fn fresh_input(&self) -> Option<Inputs> {
        model::build_input(&self.bushing, &self.st.ui).ok()
    }
    fn result_is_current(&self) -> bool {
        self.fresh_input().is_some_and(|i| self.vtu.as_ref().is_some_and(|(s, _)| *s == i))
    }
    fn analysis_is_current(&self) -> bool {
        self.fresh_input().is_some_and(|i| self.st.analysis.as_ref().is_some_and(|(s, _)| *s == i))
    }

    fn do_act(&mut self, a: EccentricAct) {
        // A message stays until the next thing the person does.
        self.status.clear();
        match a {
            EccentricAct::Row(r) => self.activate_row(r),
            EccentricAct::Step(s) => self.step = s,
            EccentricAct::Analyse => self.analyse(),
            EccentricAct::Cancel => self.st.cancel(),
            EccentricAct::ReloadBushing => self.reload_bushing(),
            EccentricAct::OpenBushing => self.open_bushing(),
            EccentricAct::OpenInFea => self.open_in_fea(),
            EccentricAct::ToggleProfile => self.st.show_profile = !self.st.show_profile,
            EccentricAct::ToggleResult => {
                self.show_result = !self.show_result;
                self.version += 1;
            }
            EccentricAct::ExportReport => {
                let text = self.st.report_text(&self.bushing);
                self.write(self.path.with_extension("report.txt"), text, "analysis");
            }
            EccentricAct::ExportCsv => {
                let text = self.fresh_input().and_then(|i| {
                    let a = self.st.analysis.as_ref().filter(|(s, _)| *s == i).map(|(_, a)| a);
                    let sweep = self.st.sweep.as_ref().filter(|(s, _)| *s == i).map(|(_, p)| p.as_slice());
                    (a.is_some() || sweep.is_some()).then(|| model::csv_text(&i, a, sweep))
                });
                self.write(self.path.with_extension("csv"), text, "analysis");
            }
            EccentricAct::ExportFields => {
                if self.result_is_current() {
                    let text = self.vtu.as_ref().map(|(_, t)| t.clone());
                    self.write(self.path.with_extension("vtu"), text, "analysis");
                } else {
                    self.export_fields = true;
                    self.analyse();
                }
            }
            EccentricAct::Undo => self.undo(),
            EccentricAct::Redo => self.redo(),
            EccentricAct::Save => self.save(),
        }
    }
}

impl Authoring for EccentricSession {
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
        if let EditorAction::Eccentric(a) = action {
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
            Key::Enter if mods.command => self.analyse(),
            Key::Escape if self.st.job.is_some() => self.st.cancel(),
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
    // The picture is read-only: the inputs are the model.
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        self.built = Some(self.version);
        // Always let the picture of the inputs settle (it clears its own rebuild flag), then prefer the solved model.
        let geometry = self.fea.scene();
        if self.show_result && self.result_is_current() {
            if let (Some((_, text)), Some(i)) = (&self.vtu, self.fresh_input()) {
                let label = format!("Eccentric bushing | e = {:.4} in, {:.0} lbf at {:.0} deg | housing, bushing and pin", i.offset, i.load_lbf, i.load_angle_deg);
                // A fit moves things by thousandths of an inch: at the window's automatic exaggeration the bodies drift apart,
                // so the solved model opens at its true size (View > Larger deformation exaggerates it again).
                let scene = vtu::scene_source(text, &label, i.thickness).and_then(|s| s.build()).map(|mut s| {
                    s.gain = 1.0;
                    s
                });
                self.showing_result = scene.is_ok();
                return scene;
            }
        }
        self.showing_result = false;
        geometry
    }
    fn needs_scene(&self) -> bool {
        self.built != Some(self.version) || self.fea.needs_scene()
    }
    fn poll(&mut self) -> bool {
        let mut changed = self.fea.poll() | self.watch();
        if self.mesh_wanted && !self.fea.meshing() {
            self.mesh_wanted = false;
            if self.fea.mesh_availability().is_ok() {
                self.fea.do_action(SolidAction::GenerateMesh);
            }
            changed = true;
        }
        let mut done = Vec::new();
        self.jobs.retain(|(id, rx)| match rx.try_recv() {
            Ok(r) => {
                done.push((*id, r));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => {
                done.push((*id, Err("the solver stopped unexpectedly".into())));
                false
            }
        });
        for (id, result) in done {
            changed = true;
            if let (Ok(Output::Fields(_, text)), Some(job)) = (&result, self.st.job.as_ref().filter(|j| j.id == id)) {
                self.vtu = Some((job.sig, text.clone()));
            }
            let effects = self.st.finish(id, result, &self.bushing);
            self.version += 1;
            self.run_effects(effects);
        }
        changed
    }
    fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
    fn solving(&self) -> bool {
        self.st.job.is_some()
    }
    fn solving_for(&self) -> Option<std::time::Duration> {
        self.st.job.as_ref().map(|j| j.started.elapsed())
    }
    fn status(&self) -> String {
        if !self.status.is_empty() {
            return self.status.clone();
        }
        if let Some(j) = &self.st.job {
            let detail = j.control.progress.as_ref().map(|p| p.snapshot().detail).filter(|d| !d.is_empty()).map_or(String::new(), |d| format!(": {d}"));
            return format!("{} ({:.0} s){detail}", model::task_name(j.task), j.started.elapsed().as_secs_f32());
        }
        if let Some(e) = &self.st.error {
            return e.clone();
        }
        if let Some(why) = &self.fe_note {
            return format!("No picture yet: {why}.");
        }
        self.fea.status.clone()
    }
    fn set_status(&mut self, text: String) {
        self.status = text;
    }
    fn title(&self) -> String {
        format!("Eccentric bushing | {}{}", self.name, if self.dirty { " *" } else { "" })
    }
    fn restore_recovery(&mut self) -> Result<(), String> {
        let text = std::fs::read_to_string(self.path.with_extension("recovery.json"))
            .map_err(|e| e.to_string())?;
        let f = EccentricFile::from_json(&text)?;
        self.undo.push(self.st.ui.clone());
        self.st.ui = f.ui();
        self.bushing_case = f.bushing_case;
        self.reload_bushing();
        self.dirty = true;
        Ok(())
    }
    fn recovery_json(&self) -> Result<String, String> {
        Ok(self.case_file().to_json())
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn geometry_view(&self) -> bool {
        !self.showing_result && self.fea.geometry_view()
    }
    fn shows_result(&self) -> bool {
        self.showing_result
    }
    fn shows_body_only(&self) -> bool {
        !self.showing_result && self.fea.shows_body_only()
    }
    fn field_unit(&self, label: &str) -> String {
        let l = label.to_lowercase();
        if l.contains("stress") {
            "psi".into()
        } else if l.contains("displacement") {
            "in".into()
        } else {
            String::new()
        }
    }
    fn set_view(&mut self, cam: &Camera, size: [f32; 2]) {
        self.fea.set_view(cam, size);
    }
    fn overlay(&self, cam: &Camera, size: [f32; 2]) -> Vec<Overlay> {
        if self.showing_result {
            Vec::new()
        } else {
            self.fea.overlay(cam, size)
        }
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
            "Analysis" if self.analysis_is_current() => Done,
            "Analysis" => Todo,
            _ => return None,
        })
    }
    fn menus(&self) -> Vec<Menu> {
        self.menu_bar()
    }
    fn context_menu(&self) -> Vec<Item> {
        self.menu_bar().into_iter().find(|m| m.title == "Analysis").map(|m| m.items).unwrap_or_default()
    }
}


#[cfg(test)]
mod tests;
