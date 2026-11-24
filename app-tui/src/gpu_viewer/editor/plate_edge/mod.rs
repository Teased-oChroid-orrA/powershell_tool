//! The Plate Edge workbench in the native window: a pin or fastener in a hole near a free edge of a plate. The person gives the
//! plate material, the fastener and the fit, and the window returns the load each failure mode allows (bearing onset and
//! ultimate, shear-out, tear-out, the pin's shear, bearing and bending) from the handbook closed forms and the elastic stress
//! field (`edge_check::modes`), and the 2D and 3D finite-element bearing response (`lug_solver::bracket`) beside them.
//!
//! The window is a boxed three-pane workbench (`Authoring::workbench`): inputs on the left (a click types a number or picks
//! one of a few choices), the plate or the solved model in the centre with the load-travel plot under it, the results on the
//! right. The viewport shows the plate, then the solved model coloured by equivalent plastic strain. Nothing here computes mechanics:
//! `inputs.rs` turns the case into the two solvers' inputs, `run.rs` runs them on a worker, `cards.rs` shows the answer.
mod cards;
pub mod inputs;
pub mod run;
mod scene;

use super::{
    authoring::{Authoring, Overlay, OverlayKind},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
};
use crate::gpu_viewer::{
    bench::Bench,
    controller::StandardView,
    menu::{Item, Menu},
    ui::{DockItem, RibbonTab, TreeNode, Workbench},
    Scene,
};
use inputs::{Field, FitKind, MeshMode, PinSource, PlateEdgeInputs, Source};

/// The caption of the picture: the solve is on half the plate, the drawing mirrors it.
/// The result's caption: what the picture shows of the pin and the hole.
pub(crate) const PIN_TRAVEL: &str = "pin travel 0.1 D: the hole elongates toward the edge";
pub(crate) const HALF_MODEL: &str = "solved on half, mirrored about the load line";
use std::path::{Path, PathBuf};

/// What a click in the Plate Edge workbench asks for (carried by `EditorAction::PlateEdge`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlateEdgeAct {
    /// A numeric input: type the number.
    Field(Field),
    /// A choice of many (fastener class, nominal size): step to the next.
    Cycle(Choice),
    /// A choice of a few, one segment of a card's switch.
    Set(Setting),
    /// The centre view: 2D or 3D model.
    View(Dim),
    /// What the centre view shows of the model.
    Layer(Layer),
    /// Type part of a library material's name to pick it.
    Search(Pick),
    Run(run::Kind),
    Cancel,
    Undo,
    Redo,
    Save,
    Export,
}

/// A choice row that steps through many values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    PinClass,
    Size,
}

/// One segment of a card's switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Plate(Source),
    Pin(PinSource),
    Fit(FitKind),
    Mesh(MeshMode),
}

/// Which FE model the centre view draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dim {
    Two,
    Three,
}

/// What the centre view shows: the plate as drawn, or the solved model coloured by plastic strain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Geometry,
    Strain,
}

/// Which library material a search picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Plate,
    Pin,
}

/// What is being typed.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Field(Field),
    Search(Pick),
}

struct Prompt {
    target: Target,
    text: String,
    /// Nothing typed yet: the first character replaces the value instead of extending it.
    fresh: bool,
}

pub struct PlateEdgeSession {
    inputs: PlateEdgeInputs,
    path: PathBuf,
    name: String,
    dirty: bool,
    undo: Vec<PlateEdgeInputs>,
    redo: Vec<PlateEdgeInputs>,
    prompt: Option<Prompt>,
    status: String,
    /// The centre view: which FE model, and what of it.
    dim: Dim,
    layer: Layer,
    saved_revision: Option<String>,
    /// The picture is rebuilt after an edit or a result.
    drawn: bool,
    /// The last finished run and the one going.
    outcome: Option<run::Outcome>,
    job: Option<run::Job>,
    /// The quick allowables follow every edit: this is the one waiting for the worker to start after the last keystroke.
    quick_due: bool,
    /// Where the next search for a library material resumes (it cycles through the matches).
    last_search: Option<(Pick, String, usize)>,
}

const UNDO_LIMIT: usize = 100;

impl PlateEdgeSession {
    pub fn new(inputs: PlateEdgeInputs, name: String, path: PathBuf) -> Self {
        let name = if name.is_empty() { path.file_stem().and_then(|n| n.to_str()).unwrap_or("Plate edge").to_string() } else { name };
        let mut s = Self {
            inputs,
            saved_revision: crate::persistence::revision(&path),
            path,
            name,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            prompt: None,
            status: String::new(),
            dim: Dim::Two,
            layer: Layer::Geometry,
            drawn: false,
            outcome: None,
            job: None,
            quick_due: true,
            last_search: None,
        };
        s.start_quick();
        s
    }

    /// A session on `path` when it holds a plate edge case, else on the defaults (saved there by Ctrl/Cmd+S).
    pub fn open(path: &Path) -> Result<Self, String> {
        let (inputs, name) = if path.exists() { PlateEdgeInputs::from_json(&std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?)? } else { (PlateEdgeInputs::default(), String::new()) };
        Ok(Self::new(inputs, name, path.to_path_buf()))
    }

    pub fn inputs(&self) -> &PlateEdgeInputs {
        &self.inputs
    }

    // ------------------------------------------------------------------ editing

    fn checkpoint(&mut self) {
        self.undo.push(self.inputs.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    /// Something the results depend on changed: the quick allowables rerun, the FE results are stale.
    fn changed(&mut self) {
        self.dirty = true;
        self.drawn = false;
        self.quick_due = true;
        self.start_quick();
    }
    fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.inputs, prev));
            self.prompt = None;
            self.changed();
        }
    }
    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.inputs, next));
            self.prompt = None;
            self.changed();
        }
    }
    fn save(&mut self) {
        if crate::persistence::revision(&self.path) != self.saved_revision {
            self.status = "Save conflict: file changed externally; reopen before saving.".into();
            return;
        }
        match crate::persistence::write(&self.path, self.inputs.to_json(&self.name)) {
            Ok(()) => {
                self.dirty = false;
                self.saved_revision = crate::persistence::revision(&self.path);
                self.status = format!("Saved {}.", self.path.display());
            }
            Err(e) => self.status = format!("Could not save {}: {e}", self.path.display()),
        }
    }
    /// The report text, beside the case file (`name.report.txt`).
    fn export(&mut self) {
        let target = self.path.with_extension("report.txt");
        match crate::persistence::write(&target, self.report_text()) {
            Ok(()) => self.status = format!("Report written to {}.", target.display()),
            Err(e) => self.status = format!("Could not write {}: {e}", target.display()),
        }
    }

    // ------------------------------------------------------------------ prompts

    fn commit_field(&mut self, f: Field, text: &str) {
        let Ok(v) = text.trim().parse::<f64>() else {
            self.status = format!("{} not changed: {:?} is not a number.", f.label(), text.trim());
            return;
        };
        if v == self.inputs.get(f) {
            return;
        }
        let mut next = self.inputs.clone();
        match next.set(f, v) {
            Ok(()) => {
                self.checkpoint();
                self.inputs = next;
                self.changed();
            }
            Err(why) => self.status = format!("{} not changed: {why}.", f.label()),
        }
    }

    /// Picks the first library material matching `text`, after the one the last identical search stopped at.
    fn commit_search(&mut self, pick: Pick, text: &str) {
        let found = inputs::search(text);
        if found.is_empty() {
            self.status = format!("No library material matches {:?}.", text.trim());
            return;
        }
        let at = match &self.last_search {
            Some((p, t, i)) if *p == pick && t == text.trim() => (*i + 1) % found.len(),
            _ => 0,
        };
        self.last_search = Some((pick, text.trim().to_string(), at));
        let m = found[at];
        self.checkpoint();
        match pick {
            Pick::Plate => self.inputs.plate_material = m.id.to_string(),
            Pick::Pin => self.inputs.pin_material = m.id.to_string(),
        }
        self.status = format!("{} of {}: {}", at + 1, found.len(), inputs::describe(m));
        self.changed();
    }

    fn commit_prompt(&mut self) {
        let Some(p) = self.prompt.take() else { return };
        match p.target {
            Target::Field(f) => self.commit_field(f, &p.text),
            Target::Search(pick) => {
                // Enter on the same text again steps to the next match, so keep the prompt's text for that.
                self.commit_search(pick, &p.text);
            }
        }
    }

    fn prompt_char(&mut self, c: char) {
        let Some(p) = &mut self.prompt else { return };
        let numeric = matches!(p.target, Target::Field(_));
        if (!numeric || c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')) && p.text.len() < 40 {
            if std::mem::take(&mut p.fresh) {
                p.text.clear();
            }
            p.text.push(c);
        }
    }

    // ------------------------------------------------------------------ runs

    /// Start the quick allowables (closed forms and the elastic field, about 0.3 s) unless a longer run is going.
    fn start_quick(&mut self) {
        if self.job.is_some() {
            return;
        }
        self.quick_due = false;
        match run::start(&self.inputs, run::Kind::Quick) {
            Ok(job) => self.job = Some(job),
            Err(why) => {
                self.outcome = None;
                self.status = format!("{why}.");
            }
        }
    }

    fn start(&mut self, kind: run::Kind) {
        if let Some(j) = &self.job {
            // The quick run that follows every edit gives way to a longer one the person asked for.
            if j.kind != run::Kind::Quick {
                return;
            }
            j.cancel();
            self.job = None;
        }
        match run::start(&self.inputs, kind) {
            Ok(job) => self.job = Some(job),
            Err(why) => self.status = format!("Not run: {why}."),
        }
    }

    /// The last run is for exactly the live inputs.
    fn current(&self) -> bool {
        self.outcome.as_ref().is_some_and(|o| o.inputs == self.inputs)
    }

    /// A segment of a card's switch; the one already chosen is not an edit.
    fn set(&mut self, setting: Setting) {
        let mut next = self.inputs.clone();
        match setting {
            Setting::Plate(v) => next.plate_source = v,
            Setting::Pin(v) => next.pin_source = v,
            Setting::Fit(k) if k != next.fit => {
                next.fit = k;
                next.fit_value = k.default_value();
            }
            Setting::Fit(_) => {}
            Setting::Mesh(v) => next.mesh = v,
        }
        if next != self.inputs {
            self.checkpoint();
            self.inputs = next;
            self.changed();
        }
    }

    /// A finished run draws its solved model: the 3D one when it ran, else the 2D.
    fn show_run(&mut self, outcome: &run::Outcome) {
        if scene::pick_fields(outcome, Dim::Three).is_some() {
            self.layer = Layer::Strain;
            self.dim = if outcome.lines.iter().any(|l| l.label.starts_with("3D") && l.fields.is_some()) { Dim::Three } else { Dim::Two };
        }
    }

    fn do_act(&mut self, a: PlateEdgeAct) {
        // A message stays until the next thing the person does.
        self.status.clear();
        match a {
            PlateEdgeAct::Field(f) => self.prompt = Some(Prompt { target: Target::Field(f), text: crate::toolboxes::fastener_hole::model::format_for_edit(self.inputs.get(f)), fresh: true }),
            PlateEdgeAct::Search(p) => self.prompt = Some(Prompt { target: Target::Search(p), text: String::new(), fresh: true }),
            PlateEdgeAct::Cycle(c) => {
                self.checkpoint();
                match c {
                    Choice::PinClass => self.inputs.pin_class = (self.inputs.pin_class + 1) % inputs::PIN_CLASSES.len(),
                    Choice::Size => self.inputs.cycle_size(),
                }
                self.changed();
            }
            PlateEdgeAct::Set(setting) => self.set(setting),
            PlateEdgeAct::View(d) => {
                self.dim = d;
                self.drawn = false;
            }
            PlateEdgeAct::Layer(l) => {
                self.layer = l;
                self.drawn = false;
            }
            PlateEdgeAct::Run(k) => self.start(k),
            PlateEdgeAct::Cancel => {
                if let Some(j) = &self.job {
                    j.cancel();
                    self.status = "Cancelling: the solver stops at its next step.".into();
                }
            }
            PlateEdgeAct::Undo => self.undo(),
            PlateEdgeAct::Redo => self.redo(),
            PlateEdgeAct::Save => self.save(),
            PlateEdgeAct::Export => self.export(),
        }
    }
}

impl Authoring for PlateEdgeSession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn key_labels(&self) -> [&'static str; 4] {
        scene::KEY_LABELS
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::PlateEdge(a) = action {
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
            Key::Escape if self.job.as_ref().is_some_and(|j| j.kind != run::Kind::Quick) => self.do_act(PlateEdgeAct::Cancel),
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
    // The picture is the inputs drawn; a click on it picks nothing.
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        self.drawn = true;
        scene::build(&self.inputs, &self.name, self.outcome.as_ref().filter(|_| self.current() && self.layer == Layer::Strain), self.dim)
    }
    fn needs_scene(&self) -> bool {
        !self.drawn
    }
    fn overlay(&self, cam: &Camera, size: [f32; 2]) -> Vec<Overlay> {
        let f = scene::Frame::of(&self.inputs);
        let px = |x: f64, y: f64| {
            let (ndc, _) = cam.project(f.point(x, y));
            Camera::pixel(ndc, size[0], size[1])
        };
        let below = |p: [f32; 2], dy: f32| [p[0], p[1] + dy];
        let beside = |p: [f32; 2], dx: f32| [p[0] + dx, p[1]];
        let r = 0.5 * f.d;
        // The pin as drawn: at its start before a result, at its travelled position after one.
        let cx = if self.shows_result() { scene::pin_disc(&f, self.inputs.pin_diameter()).0 } else { f.pin(self.inputs.pin_diameter()).1 };
        // The load: the pin is pulled along the load line toward the free edge.
        let reach = (r + 0.9 * f.d).min(0.85 * f.e);
        let mut out = Vec::new();
        if self.shows_result() {
            // Under the load arrow: the pin as a filled disc at its travelled position.
            let (centre, rp) = scene::pin_disc(&f, self.inputs.pin_diameter());
            out.push(Overlay { px: px(centre, 0.0), text: String::new(), kind: OverlayKind::Disc, tip: Some(px(centre + rp, 0.0)) });
        }
        out.push(Overlay { px: px(cx, 0.0), text: "P".into(), kind: OverlayKind::Load, tip: Some(px(-reach, 0.0)) });
        // The solve is on half the plate; the drawing mirrors it: say so.
        // The captions sit in the empty top of the view, clear of the colour legend that takes the bottom left.
        let top = px(-0.5 * f.e, 2.6 * f.d);
        out.push(Overlay { px: top, text: HALF_MODEL.into(), kind: OverlayKind::Dimension, tip: None });
        let boxed = |p: [f32; 2], text: String| Overlay { px: p, text, kind: OverlayKind::Dimension, tip: None };
        // The plate's own dimensions stay with the contour: the edge distance, the free edge and the shear-out planes.
        out.push(boxed(below(px(-0.5 * f.e, scene::dimension_y(f.d)), 22.0), format!("e = {:.3} in   e/D {:.2}", f.e, f.e / f.d)));
        out.push(boxed(below(px(-0.5 * f.e, r), -14.0), "shear-out planes".into()));
        // Inside the plate against the edge it names (a label is centred on its point).
        out.push(boxed(beside(px(-f.e, 2.0 * f.d), 48.0), "free edge".into()));
        if self.shows_result() {
            out.push(boxed(below(top, 30.0), PIN_TRAVEL.into()));
        } else {
            // The undeformed bore has a diameter to name.
            out.push(boxed(below(px(0.0, r), -20.0), format!("D {:.4} in", f.d)));
        }
        out
    }
    fn geometry_view(&self) -> bool {
        !self.shows_result()
    }
    fn shows_result(&self) -> bool {
        self.layer == Layer::Strain && self.current() && self.outcome.as_ref().is_some_and(|o| scene::pick_fields(o, self.dim).is_some())
    }
    fn field_unit(&self, _label: &str) -> String {
        String::new()
    }
    fn default_mesh_lines(&self) -> bool {
        true
    }
    fn poll(&mut self) -> bool {
        let Some(job) = &mut self.job else {
            if self.quick_due {
                self.start_quick();
                return self.job.is_some();
            }
            return false;
        };
        match job.poll() {
            None => !job.progress.is_empty() && job.kind != run::Kind::Quick,
            Some(done) => {
                let kind = job.kind;
                self.job = None;
                match done {
                    Some(outcome) => {
                        if let Some(note) = outcome.fe_note.as_ref().filter(|_| kind != run::Kind::Quick) {
                            self.status = format!("FE: {note}.");
                        }
                        if kind != run::Kind::Quick {
                            self.show_run(&outcome);
                        }
                        self.outcome = Some(outcome);
                    }
                    None => self.status = "The calculation stopped without a result.".into(),
                }
                self.drawn = false;
                if self.quick_due {
                    self.start_quick();
                }
                true
            }
        }
    }
    fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
    fn solving(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.kind != run::Kind::Quick)
    }
    fn solving_for(&self) -> Option<std::time::Duration> {
        self.job.as_ref().filter(|j| j.kind != run::Kind::Quick).map(|j| j.started.elapsed())
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
        format!("Plate Edge | {}{}", self.name, if self.dirty { " *" } else { "" })
    }
    fn initial_view(&self) -> Option<StandardView> {
        Some(StandardView::Isometric)
    }
    fn restore_recovery(&mut self) -> Result<(), String> {
        let text = std::fs::read_to_string(self.path.with_extension("recovery.json")).map_err(|e| e.to_string())?;
        let (inputs, _) = PlateEdgeInputs::from_json(&text)?;
        self.undo.push(std::mem::replace(&mut self.inputs, inputs));
        self.changed();
        Ok(())
    }
    fn recovery_json(&self) -> Result<String, String> {
        Ok(self.inputs.to_json(&self.name))
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn tree(&self) -> Vec<TreeNode> {
        Vec::new()
    }
    fn properties(&self) -> Vec<DockItem> {
        super::authoring::flatten([self.input_cards(), self.result_cards()].concat())
    }
    fn inspector(&self) -> Vec<super::authoring::Card> {
        Vec::new()
    }
    fn ribbon(&self) -> Vec<RibbonTab> {
        Vec::new()
    }
    fn workbench(&self) -> Option<Workbench> {
        Some(self.workbench_data())
    }
    fn has_workbench(&self) -> bool {
        true
    }
    fn menus(&self) -> Vec<Menu> {
        self.menu_bar()
    }
    fn context_menu(&self) -> Vec<Item> {
        self.menu_bar().into_iter().find(|m| m.title == "Run").map(|m| m.items).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests;
