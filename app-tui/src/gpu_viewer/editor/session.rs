//! One editing session: state + background solve + file path + numeric prompt, driven by
//! window-independent key and click events. `window.rs` translates winit events into these calls
//! and draws what `scene()` / `panel()` return, so the whole interaction is testable headless.
use super::{
    camera::{Camera, Frame, Hit},
    caps::{self, Operation},
    job::{Outcome, Runner},
    scene::{self, EditView},
    state::{EditorState, PropertyKind, Tool},
};
use crate::gpu_viewer::{controller::Command, Scene};
use fea_problem::members::MemberProblem;
use std::path::PathBuf;

/// Keys the editor understands (letters arrive as typed so the numeric prompt can see `e`/`E`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Tab,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mods {
    /// Control, or Command on macOS.
    pub command: bool,
    pub shift: bool,
}
/// What the Advisor menu shows.
pub struct AdvisorMenu {
    pub suggestions: bool,
    pub recording: bool,
    pub learned: bool,
    pub learned_available: Result<(), String>,
    pub can_train: Result<(), String>,
    pub has_recordings: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyResult {
    Handled,
    Command(Command),
    /// Not an editor key: let the viewer controller interpret it.
    Unhandled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditorAction {
    SetTool(Tool),
    Run(Operation),
    Focus(usize),
    /// Reload the model from its file (Ctrl/Cmd+O).
    Load,
    /// Replace the model by the next template (undoable; Ctrl/Cmd+N).
    NextTemplate,
    CyclePlane,
    CycleGrid,
    FitView,
    /// Edit the focused property row (E).
    EditProperty,
    Advisor(super::learning::AdvisorAction),
    /// Select a node / member from the feature tree.
    SelectNode(u32),
    SelectMember(u32),
    /// Boundary conditions for the selected node (chosen from the list a Support / Load click opens).
    Support(super::state::SupportPreset),
    DefaultLoad,
    RemoveLoad,
    /// Prompt for one property of the selection.
    Edit(PropertyKind),
    /// Move the selected nodes (and the ends of selected members) along an axis by a typed distance.
    MoveGroup(usize),
    ToggleSupports,
    ToggleLoads,
    /// Solid-model editing (see `solid`).
    Solid(super::solid::SolidAction),
    /// The Bushing workbench (see `bushing`).
    Bushing(super::bushing::BushingAct),
    /// The Eccentric Bushing workbench (see `eccentric`).
    Eccentric(super::eccentric::EccentricAct),
    /// The Fastener Hole workbench (see `fastener`).
    Fastener(super::fastener::FastenerAct),
    /// The Search workbench (see `search`).
    Search(super::search::SearchAct),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Title,
    Normal,
    Dim,
    On,
    Off,
    Warn,
}
struct Prompt {
    kind: PropertyKind,
    label: String,
    text: String,
}

pub struct Session {
    pub state: EditorState,
    pub runner: Runner,
    pub path: PathBuf,
    /// Show the solved geometry/contours when a current result exists.
    pub show_results: bool,
    prompt: Option<Prompt>,
    /// Incremented whenever the scene must be rebuilt.
    version: u64,
    built: Option<u64>,
    /// Optional learned ranker; `None` means the rule-based baseline. See `advisory`.
    advisor: Option<Box<dyn super::advisory::Advisor>>,
    /// False when the user opted out (`FEA_EDITOR_ADVISOR=off`, or the menu): no ranked suggestions are shown.
    advice: bool,
    /// True when the environment forced the advisor off; the menu cannot override that.
    advice_locked: bool,
    /// Opt-in recording / training / model gate (`None` without an app-data folder or when locked off).
    learning: Option<super::learning::Learning>,
    trainer: super::worker::Worker<super::learning::Bundle>,
}

mod ui;
pub use ui::{operation as operation_entry, split_label};

const GRIDS: [f64; 6] = [0.5, 1.0, 2.5, 5.0, 10.0, 25.0];

impl Session {
    pub fn new(problem: MemberProblem, path: PathBuf) -> Self {
        let mut state = EditorState::new(problem);
        if let Some(i) = GRIDS.iter().position(|g| *g >= state.frame.span / 40.0) {
            state.grid = GRIDS[i];
        }
        Self { state, runner: Runner::default(), path, show_results: false, prompt: None, version: 1, built: None, advisor: None, advice: true, advice_locked: false, learning: None, trainer: super::worker::Worker::new("member-advisor-training") }
    }
    /// Apply the start-up advisory configuration (environment); a model that cannot be used is reported.
    pub fn configure_advice(&mut self, advisor: Option<Box<dyn super::advisory::Advisor>>, enabled: bool, note: Option<String>) {
        self.advisor = advisor;
        self.advice = enabled;
        self.advice_locked = !enabled;
        if let Some(n) = note {
            self.state.status = n;
        }
    }
    /// Attach the opt-in learning loop. Settings and the trained model are read from `store`; the
    /// learned ranking is switched on only if the user enabled it *and* the saved report still qualifies.
    /// Does nothing when the environment locked the advisor off.
    pub fn attach_learning(&mut self, store: super::learning::Store, session_id: u64) {
        if self.advice_locked {
            return;
        }
        let l = super::learning::Learning::new(store, session_id);
        self.advice = l.settings.suggestions;
        if l.settings.use_learned && self.advisor.is_none() {
            match (&l.bundle, l.learned_available()) {
                (Some(b), Ok(())) => self.advisor = Some(Box::new(b.model.clone())),
                (_, Err(why)) => self.state.status = format!("Learned ranking stays off ({why})."),
                _ => {}
            }
        }
        self.learning = Some(l);
    }
    /// Remember one user action (only when recording is on) with its context and the allowed set.
    fn note_action(&mut self, taken: super::advisory::Suggestion) {
        let Some(l) = self.learning.as_ref().filter(|l| l.settings.record) else { return };
        let context = super::advisory::Context::of(self);
        let allowed = super::advisory::allowed(self);
        if !allowed.contains(&taken) {
            return;
        }
        if let Err(e) = l.record(context, allowed, taken) {
            self.state.status = format!("Advisor recording failed ({e}); recording turned off.");
            if let Some(l) = &mut self.learning {
                l.settings.record = false;
            }
        }
    }
    pub fn touch(&mut self) {
        self.version += 1;
    }
    pub fn needs_scene(&self) -> bool {
        self.built != Some(self.version)
    }
    pub fn solving(&self) -> bool {
        self.runner.is_running()
    }
    pub fn prompting(&self) -> bool {
        self.prompt.is_some()
    }

    /// Scene for the current state (results only when current and enabled).
    pub fn scene(&mut self) -> Result<Scene, String> {
        let solved = if self.show_results { self.state.current_solution().cloned() } else { None };
        let sel = match self.state.selection {
            Some(Hit::Node(id) | Hit::Member(id)) => Some(id),
            None => None,
        };
        let group: Vec<u32> = self.state.group.iter().map(|h| match h { Hit::Node(id) | Hit::Member(id) => *id }).collect();
        let scene = scene::build(&self.state.problem, &EditView { frame: self.state.frame, selected: sel, group: &group, solved: solved.as_deref(), show_supports: self.state.show_supports, show_loads: self.state.show_loads })?;
        self.built = Some(self.version);
        Ok(scene)
    }

    /// Collect a finished background solve. Returns true when something changed on screen.
    pub fn poll(&mut self) -> bool {
        let mut trained = false;
        if let Some((_, answer)) = self.trainer.poll() {
            match answer {
                Ok(bundle) => if let Some(l) = &mut self.learning {
                    self.state.status = bundle.report.summary();
                    if let Err(e)=l.store.save_bundle(&bundle) { self.state.status = format!("Advisor save failed: {e}"); }
                    l.bundle=Some(bundle);
                    if l.learned_available().is_err() {
                        self.advisor=None;
                        l.settings.use_learned=false;
                        if let Err(e)=l.store.save_settings(&l.settings) { self.state.status=format!("Advisor settings save failed: {e}"); }
                    } else if l.settings.use_learned {
                        self.advisor=l.bundle.as_ref().map(|b|Box::new(b.model.clone()) as Box<dyn super::advisory::Advisor>);
                    }
                },
                Err(e) => self.state.status=format!("Advisor training failed: {e}"),
            }
            trained=true;
        }
        let Some(outcome) = self.runner.poll() else { return trained };
        match outcome {
            Outcome::Finished { problem, solved } => {
                let (u, name) = (solved.max_displacement(), problem.name.clone());
                self.state.accept_solution(*solved, *problem);
                if self.state.current_solution().is_some() {
                    self.show_results = true;
                    self.state.status = format!("Solved {name}: max displacement {u:.4e}, equilibrium accepted.");
                } else {
                    self.state.status = "The model changed while solving; that result was discarded. Press Enter to solve again.".into();
                }
            }
            Outcome::Failed(e) => self.state.status = format!("Solve failed: {e}"),
            Outcome::Cancelled => self.state.status = "Solve cancelled; its result was discarded.".into(),
        }
        self.touch();
        true
    }

    pub fn run(&mut self, op: Operation) {
        if let Err(reason) = caps::availability(&self.state, self.solving(), op) {
            self.state.status = reason;
            return;
        }
        self.note_action(super::advisory::Suggestion::Op(op));
        match op {
            Operation::Undo => {
                self.state.undo();
            }
            Operation::Redo => {
                self.state.redo();
            }
            Operation::Delete => {
                let _ = self.state.delete_selected();
            }
            Operation::Solve => match self.runner.start(self.state.problem.clone()) {
                Ok(()) => self.state.status = "Solving...".into(),
                Err(e) => self.state.status = e,
            },
            Operation::CancelSolve => {
                self.runner.cancel();
            }
            Operation::Save => match self.state.save(&self.path) {
                Ok(()) => self.state.status = format!("Saved {}", self.path.display()),
                Err(e) => self.state.status = format!("Save failed: {e}"),
            },
            Operation::ConvertToFrame => {
                let _ = self.state.convert_selected(true);
            }
            Operation::ConvertToTruss => {
                let _ = self.state.convert_selected(false);
            }
            Operation::ApplyMoment => self.begin_prompt_for(|k| matches!(k, PropertyKind::Moment(_))),
            Operation::FixRotations => {
                for k in 3..6 {
                    let _ = self.state.set_property(PropertyKind::Fixed(k), 1.0);
                }
            }
            Operation::ShowResults => self.show_results = !self.show_results,
            Operation::TogglePlanar => {
                let axis = if self.state.problem.plane.is_some() { None } else { Some(self.state.plane_axis) };
                if self.state.commit(fea_problem::members::Edit::SetPlane { axis }).is_ok() {
                    self.state.status = match axis {
                        Some(a) => format!("Planar analysis in the {} plane: out-of-plane motion is held.", ["yz", "xz", "xy"][a]),
                        None => "Full 3D analysis.".into(),
                    };
                }
            }
        }
        self.touch();
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.note_action(super::advisory::Suggestion::Tool(tool));
        self.state.set_tool(tool);
        self.touch();
    }

    pub fn act(&mut self, action: EditorAction) {
        match action {
            EditorAction::SetTool(t) => self.set_tool(t),
            EditorAction::Run(op) => self.run(op),
            EditorAction::Focus(i) => {
                self.state.focus = i;
                self.begin_prompt();
            }
            EditorAction::Load => self.load(),
            EditorAction::NextTemplate => self.next_template(),
            EditorAction::CyclePlane => self.cycle_plane(),
            EditorAction::CycleGrid => self.cycle_grid(),
            EditorAction::FitView => self.fit_view(),
            EditorAction::EditProperty => self.begin_prompt(),
            EditorAction::Advisor(a) => self.advisor_action(a),
            EditorAction::SelectNode(id) => self.select_entity(Hit::Node(id)),
            EditorAction::SelectMember(id) => self.select_entity(Hit::Member(id)),
            EditorAction::Support(preset) => self.with_selected_node(|st, n| st.apply_support(n, preset)),
            EditorAction::DefaultLoad => self.with_selected_node(|st, n| st.apply_default_load(n)),
            EditorAction::RemoveLoad => self.with_selected_node(|st, n| st.remove_load(n)),
            EditorAction::Edit(kind) => match self.state.properties().iter().position(|r| r.kind == kind) {
                Some(i) => {
                    self.state.focus = i;
                    self.begin_prompt();
                }
                None => self.state.status = "that property does not apply to the selection".into(),
            },
            EditorAction::MoveGroup(axis) => {
                if self.state.group_nodes().is_empty() {
                    self.state.status = "select nodes or members first".into();
                } else {
                    self.prompt = Some(Prompt { kind: PropertyKind::MoveGroup(axis), label: format!("move along {} by", ["x", "y", "z"][axis]), text: String::new() });
                }
            }
            EditorAction::ToggleSupports => self.state.show_supports = !self.state.show_supports,
            EditorAction::ToggleLoads => self.state.show_loads = !self.state.show_loads,
            // A member session never receives solid actions (its UI does not offer them).
            EditorAction::Solid(_) | EditorAction::Bushing(_) | EditorAction::Eccentric(_) | EditorAction::Fastener(_) | EditorAction::Search(_) => {}
        }
        self.touch();
    }

    fn with_selected_node(&mut self, f: impl FnOnce(&mut EditorState, u32) -> Result<(), String>) {
        let Some(Hit::Node(n)) = self.state.selection else {
            self.state.status = "select a node first".into();
            return;
        };
        if let Err(e) = f(&mut self.state, n) {
            self.state.status = e;
        }
    }

    /// State of the Advisor menu: `Err` carries why every entry is unavailable.
    pub fn advisor_menu(&self) -> Result<AdvisorMenu, String> {
        if self.advice_locked {
            return Err("the advisor is turned off by FEA_EDITOR_ADVISOR=off".into());
        }
        let l = self.learning.as_ref().ok_or("no app-data folder for advisor files on this system")?;
        Ok(AdvisorMenu {
            suggestions: self.advice,
            recording: l.settings.record,
            learned: self.advisor.is_some(),
            learned_available: l.learned_available(),
            can_train: if l.store.has_recordings() { Ok(()) } else { Err("nothing recorded yet: turn on recording first".into()) },
            has_recordings: l.store.has_recordings(),
        })
    }
    fn advisor_action(&mut self, action: super::learning::AdvisorAction) {
        use super::learning::AdvisorAction as A;
        let Some(l) = self.learning.as_mut() else {
            self.state.status = "The advisor is not available (no app-data folder, or turned off).".into();
            return;
        };
        let status = match action {
            A::ToggleSuggestions => {
                l.settings.suggestions = !l.settings.suggestions;
                self.advice = l.settings.suggestions;
                format!("Suggestions {}.", if self.advice { "shown" } else { "hidden" })
            }
            A::ToggleRecording => {
                l.settings.record = !l.settings.record;
                if l.settings.record {
                    format!("Recording on: coarse editor state and the action you take are saved locally in {} (no coordinates, names or values).", l.store.dir().display())
                } else {
                    "Recording off.".into()
                }
            }
            A::ToggleLearned => {
                if self.advisor.is_some() {
                    self.advisor = None;
                    l.settings.use_learned = false;
                    "Learned ranking off: suggestions come from the rules.".into()
                } else {
                    match (&l.bundle, l.learned_available()) {
                        (Some(b), Ok(())) => {
                            self.advisor = Some(Box::new(b.model.clone()));
                            l.settings.use_learned = true;
                            "Learned ranking on: it only reorders actions the rules allow and defers on unfamiliar situations.".into()
                        }
                        (_, Err(why)) => why,
                        _ => "No trained model.".into(),
                    }
                }
            }
            A::Train => {
                let store=l.store.clone();
                match self.trainer.start(0, move || super::learning::train(&store.read()?)) {
                    Ok(()) => "Advisor training in background; current ranking remains active.".into(),
                    Err(e) => e,
                }
            },
            A::DeleteRecordings => { self.trainer.cancel(); match l.store.delete_recordings() {
                Ok(()) => "Recordings deleted (a trained model, if any, is kept).".into(),
                Err(e) => format!("Could not delete recordings: {e}"),
            } },
        };
        if let Err(e) = l.store.save_settings(&l.settings) {
            self.state.status = format!("{status} (settings not saved: {e})");
        } else {
            self.state.status = status;
        }
    }

    fn load(&mut self) {
        match self.state.load(&self.path.clone()) {
            Ok(()) => {
                self.state.status = format!("Loaded {}", self.path.display());
                self.show_results = false;
            }
            Err(e) => self.state.status = format!("Load failed: {e}"),
        }
    }
    fn next_template(&mut self) {
        let mut all = vec![fea_problem::members::blank_planar()];
        all.extend(fea_problem::members::templates());
        let next = all.iter().position(|p| p.name == self.state.problem.name).map_or(0, |i| (i + 1) % all.len());
        self.state.replace(all.swap_remove(next), false);
        self.show_results = false;
        self.state.status = format!("Started from template '{}' (undoable).", self.state.problem.name);
    }
    fn cycle_plane(&mut self) {
        self.state.cycle_plane();
        self.state.status = format!("Working plane {} = {}", self.state.plane_name(), self.state.plane_value);
    }
    fn cycle_grid(&mut self) {
        let i = GRIDS.iter().position(|g| *g == self.state.grid).map_or(0, |i| (i + 1) % GRIDS.len());
        self.state.grid = GRIDS[i];
        self.state.status = format!("Grid {}", self.state.grid);
    }
    fn fit_view(&mut self) {
        self.state.frame = Frame::fit(&self.state.problem);
    }

    fn begin_prompt_for(&mut self, want: impl Fn(PropertyKind) -> bool) {
        let rows = self.state.properties();
        if let Some(i) = rows.iter().position(|r| want(r.kind)) {
            self.state.focus = i;
            self.begin_prompt();
        }
    }
    fn begin_prompt(&mut self) {
        let rows = self.state.properties();
        match rows.get(self.state.focus) {
            Some(r) => self.prompt = Some(Prompt { kind: r.kind, label: r.label.clone(), text: String::new() }),
            None => self.state.status = "select a node or member to edit its properties".into(),
        }
    }

    /// Left click at a window pixel (no drag). Returns true if the editor consumed it.
    pub fn click(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool {
        if self.prompt.is_some() {
            return true;
        }
        let hit = super::camera::pick(&self.state.problem, &self.state.frame, cam, px, size, super::camera::pick_radius(size));
        let plane = {
            let f: Frame = self.state.frame;
            let ndc = Camera::ndc(px[0] as f64, px[1] as f64, size[0], size[1]);
            let axis = self.state.plane_axis;
            let value = ((self.state.plane_value - f.center[axis]) / f.span) as f32;
            cam.unproject_to_plane(ndc, axis, value).map(|p| f.to_model(p))
        };
        let _ = self.state.click(hit, plane);
        self.touch();
        true
    }

    /// Shift-click: add the entity under the pixel to the selection, or drop it when already selected.
    pub fn click_additive(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool {
        if self.prompt.is_some() {
            return true;
        }
        if let Some(hit) = super::camera::pick(&self.state.problem, &self.state.frame, cam, px, size, super::camera::pick_radius(size)) {
            self.state.toggle(hit);
            self.state.status = format!("{} selected.", self.state.selected_all().len());
        }
        self.touch();
        true
    }
    /// Select the nodes inside the pixel box, and the members with both ends inside it.
    pub fn box_select(&mut self, a: [f32; 2], b: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) {
        if self.prompt.is_some() {
            return;
        }
        let hits = super::camera::pick_box(&self.state.problem, &self.state.frame, cam, a, b, size);
        self.state.status = format!("{} selected by box.", hits.len());
        self.state.select_many(hits, additive);
        self.touch();
    }
    /// Select the entity under a window pixel without applying the active tool (right-click).
    pub fn select_at(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) {
        if self.prompt.is_some() {
            return;
        }
        let hit = super::camera::pick(&self.state.problem, &self.state.frame, cam, px, size, super::camera::pick_radius(size));
        if hit.is_some() {
            self.state.select(hit);
            self.touch();
        }
    }

    pub fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        if let Some(prompt) = &mut self.prompt {
            match key {
                Key::Escape => self.prompt = None,
                Key::Backspace => {
                    prompt.text.pop();
                }
                Key::Char(c) if (c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')) && prompt.text.len() < 24 => prompt.text.push(c),
                Key::Enter => {
                    let (kind, text, label) = (prompt.kind, prompt.text.clone(), prompt.label.clone());
                    match text.trim().parse::<f64>() {
                        Ok(v) => match self.state.set_property(kind, v) {
                            Ok(()) => self.prompt = None,
                            Err(e) => self.state.status = format!("{label}: {e}"),
                        },
                        Err(_) => self.state.status = format!("{label}: '{text}' is not a number"),
                    }
                    self.touch();
                }
                _ => {}
            }
            return KeyResult::Handled;
        }
        let ctrl = mods.command;
        match key {
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'z') => self.run(if mods.shift { Operation::Redo } else { Operation::Undo }),
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'y') => self.run(Operation::Redo),
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'s') => self.run(Operation::Save),
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'o') => self.load(),
            Key::Char(c) if ctrl && c.eq_ignore_ascii_case(&'n') => self.next_template(),
            Key::Char(c) if ctrl => {
                let _ = c;
                return KeyResult::Unhandled;
            }
            Key::Char(c @ '1'..='6') => self.set_tool(Tool::ALL[c as usize - '1' as usize]),
            Key::Char(c) if c.eq_ignore_ascii_case(&'p') => self.cycle_plane(),
            Key::Char(c) if c.eq_ignore_ascii_case(&'g') => self.cycle_grid(),
            Key::Char(c) if c.eq_ignore_ascii_case(&'z') => self.fit_view(),
            Key::Char(c) if c.eq_ignore_ascii_case(&'b') => self.run(Operation::ConvertToFrame),
            Key::Char(c) if c.eq_ignore_ascii_case(&'t') => self.run(Operation::ConvertToTruss),
            Key::Char(c) if c.eq_ignore_ascii_case(&'e') => self.begin_prompt(),
            Key::Char(c) if c.eq_ignore_ascii_case(&'v') => self.run(Operation::ShowResults),
            Key::Char(c) if c.eq_ignore_ascii_case(&'k') => self.run(Operation::TogglePlanar),
            Key::Char(',') => return KeyResult::Command(Command::GainDown),
            Key::Char('.') => return KeyResult::Command(Command::GainUp),
            Key::Enter => self.run(Operation::Solve),
            Key::Delete | Key::Backspace => self.run(Operation::Delete),
            Key::Escape => {
                if self.solving() {
                    self.run(Operation::CancelSolve);
                } else if self.state.pending.is_some() {
                    self.state.pending = None;
                    self.state.status = "Member creation cancelled.".into();
                } else {
                    self.state.select(None);
                }
            }
            Key::Up => self.state.focus = self.state.focus.saturating_sub(1),
            Key::Down => self.state.focus = (self.state.focus + 1).min(self.state.properties().len().saturating_sub(1)),
            Key::Tab => return KeyResult::Command(Command::CycleField { forward: !mods.shift }),
            Key::Left | Key::Right | Key::Char(_) => return KeyResult::Unhandled,
        }
        self.touch();
        KeyResult::Handled
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::{controller::UiAction, ui::DockItem};
    use fea_problem::members::templates;
    use super::super::authoring::Authoring;
    use std::time::{Duration, Instant};

    /// Text of every property-manager line (headings included).
    fn texts(s: &Session) -> Vec<String> {
        Authoring::properties(s)
            .into_iter()
            .filter_map(|i| match i {
                DockItem::Heading(t) => Some(t),
                i @ (DockItem::Line { .. } | DockItem::Field { .. }) => i.plain_text(),
                _ => None,
            })
            .collect()
    }

    fn cam() -> Camera {
        Camera { yaw: 0.0, pitch: 0.0, zoom: 1.6, aspect: 1.5, pan: [0.0, 0.0] }
    }
    const SIZE: [f32; 2] = [900.0, 600.0];
    fn session(p: MemberProblem) -> Session {
        Session::new(p, std::env::temp_dir().join("never-written.members.json"))
    }
    fn screen(s: &Session, pos: [f64; 3]) -> [f32; 2] {
        let (n, _) = cam().project(s.state.frame.to_scene(pos));
        Camera::pixel(n, SIZE[0], SIZE[1])
    }
    fn wait(s: &mut Session) {
        let t = Instant::now();
        while !s.poll() {
            assert!(t.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn author_a_truss_from_scratch_solve_and_see_results() {
        let mut s = session(fea_problem::members::blank_planar());
        // Frame is centred on the origin with a 100-unit span for an empty model; the grid follows it.
        assert_eq!(s.key(Key::Char('3'), Mods::default()), KeyResult::Handled);
        let a = screen(&s, [-25.0, 0.0, 0.0]);
        let b = screen(&s, [25.0, 0.0, 0.0]);
        let c = screen(&s, [0.0, 30.0, 0.0]);
        s.click(a, SIZE, &cam());
        s.click(c, SIZE, &cam());
        s.click(b, SIZE, &cam());
        s.click(screen(&s, [0.0, 30.0, 0.0]), SIZE, &cam());
        assert_eq!((s.state.problem.nodes.len(), s.state.problem.members.len()), (3, 2), "{:?}", s.state.problem.nodes);
        s.key(Key::Char('5'), Mods::default());
        // Each click on a node offers the applicable list; picking an item applies it to that node.
        for x in [-25.0, 25.0] {
            s.click(screen(&s, [x, 0.0, 0.0]), SIZE, &cam());
            let items = s.take_popup().expect("a support list");
            assert!(items.iter().any(|i| matches!(i, crate::gpu_viewer::menu::Item::Entry(e) if e.label.starts_with("Roller"))));
            s.act(EditorAction::Support(crate::gpu_viewer::editor::state::SupportPreset::Pinned));
        }
        s.key(Key::Char('6'), Mods::default());
        s.click(screen(&s, [0.0, 30.0, 0.0]), SIZE, &cam());
        assert!(s.take_popup().is_some_and(|i| i.iter().any(|i| matches!(i, crate::gpu_viewer::menu::Item::Entry(e) if e.label == "Edit Fy..."))));
        s.act(EditorAction::DefaultLoad);
        assert_eq!((s.state.problem.supports.len(), s.state.problem.loads.len()), (2, 1));
        s.key(Key::Enter, Mods::default());
        assert!(s.solving());
        wait(&mut s);
        assert!(s.show_results && s.state.current_solution().is_some(), "{}", s.state.status);
        let sol = s.state.current_solution().unwrap();
        // Symmetric two-bar truss: closed form member force = P / (2 sin(theta)).
        let theta = (30.0_f64).atan2(25.0);
        let expect = 100.0 / (2.0 * theta.sin());
        assert!((sol.members[0].axial_force.abs() / expect - 1.0).abs() < 1e-9, "{} vs {expect}", sol.members[0].axial_force);
        let scene = s.scene().unwrap();
        assert!(!scene.fields.is_empty(), "solved scene exposes alternative fields");
    }
    #[test]
    fn editing_after_a_solve_makes_it_stale_and_hides_results() {
        let mut s = session(templates().remove(0));
        s.run(Operation::Solve);
        wait(&mut s);
        assert!(s.scene().unwrap().fields.len() == 2);
        s.state.select(Some(Hit::Node(4)));
        s.key(Key::Char('e'), Mods::default());
        assert!(s.prompting());
        for c in "-2500".chars() {
            s.key(Key::Char(c), Mods::default());
        }
        s.key(Key::Enter, Mods::default());
        assert!(!s.prompting());
        assert!(s.state.solution_is_stale());
        assert!(s.scene().unwrap().fields.is_empty(), "stale results are never drawn");
        assert!(texts(&s).iter().any(|l| l.contains("stale")));
    }
    #[test]
    fn result_arriving_for_an_old_model_is_discarded_not_shown() {
        let mut s = session(templates().remove(0));
        s.run(Operation::Solve);
        // edit while the worker runs
        s.state.select(Some(Hit::Node(4)));
        s.state.set_property(PropertyKind::Force(2), -1.0).unwrap();
        wait(&mut s);
        assert!(!s.show_results && s.state.current_solution().is_none());
        assert!(s.state.status.contains("changed while solving"));
    }
    #[test]
    fn prompt_validates_input_and_keeps_the_old_value_on_error() {
        let mut s = session(templates().remove(1));
        s.state.select(Some(Hit::Node(2)));
        s.key(Key::Char('e'), Mods::default());
        s.key(Key::Char('a'), Mods::default()); // ignored: not a numeric character
        s.key(Key::Char('1'), Mods::default());
        s.key(Key::Char('-'), Mods::default());
        s.key(Key::Enter, Mods::default());
        assert!(s.prompting() && s.state.status.contains("not a number"), "{}", s.state.status);
        s.key(Key::Backspace, Mods::default());
        s.key(Key::Escape, Mods::default());
        assert!(!s.prompting());
        assert_eq!(s.state.problem.nodes[1].pos, [50.0, 0.0, 0.0]);
        // invalid physical value: coincident node is explained by the problem, prompt stays open
        s.key(Key::Char('e'), Mods::default());
        for c in "0".chars() {
            s.key(Key::Char(c), Mods::default());
        }
        s.key(Key::Enter, Mods::default());
        assert!(s.prompting() && s.state.status.contains("already exists"), "{}", s.state.status);
        s.key(Key::Escape, Mods::default());
    }
    #[test]
    fn a_finished_background_refit_replaces_an_enabled_member_advisor() {
        use super::super::{advisory::{Context,Selected,Stage,Suggestion},learning::{self,Recorded,Store}};
        let dir=tempfile::tempdir().unwrap();
        let context=Context { stage: Stage::Empty, selected: Selected::Nothing, solving: false };
        let allowed=vec![Suggestion::Tool(Tool::Node),Suggestion::Tool(Tool::Truss),Suggestion::Tool(Tool::Frame)];
        let fit=|taken| {
            let events: Vec<_>=(0..60).flat_map(|id| (0..12).map({let allowed=allowed.clone(); move |_| Recorded {session:id,context,allowed:allowed.clone(),taken}})).collect();
            let bundle=learning::train(&events).unwrap();
            bundle.usable().unwrap();
            bundle
        };
        let old=fit(Suggestion::Tool(Tool::Truss));
        let new=fit(Suggestion::Tool(Tool::Frame));
        let mut s=session(fea_problem::members::blank_planar());
        s.attach_learning(Store::new(dir.path().to_path_buf()),1);
        s.advisor=Some(Box::new(old.model.clone()));
        let l=s.learning.as_mut().unwrap(); l.settings.use_learned=true; l.bundle=Some(old);
        s.trainer.start(0,move ||Ok(new)).unwrap();
        let until=std::time::Instant::now()+std::time::Duration::from_secs(10);
        while s.trainer.is_busy() { s.poll(); assert!(std::time::Instant::now()<until); std::thread::sleep(std::time::Duration::from_millis(2)); }
        assert_eq!(s.advisor.as_ref().unwrap().rank(&context,&allowed).unwrap()[0],Suggestion::Tool(Tool::Frame));
        assert!(s.learning.as_ref().unwrap().settings.use_learned);
    }

    #[test]
    fn advisor_loop_is_opt_in_records_trains_and_gates_the_learned_ranking() {
        use super::super::{advisory::Suggestion, learning::{AdvisorAction as A, Store}};
        let dir = std::env::temp_dir().join(format!("session-advisor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.clone());
        let mut s = session(fea_problem::members::blank_planar());
        // A session id that lands in the training split (about a fifth of ids are held out).
        let id = (1u64..).find(|i| !super::super::learning::is_held_out(*i)).unwrap();
        s.attach_learning(store.clone(), id);
        s.set_tool(Tool::Node);
        assert!(!store.has_recordings(), "recording is off by default");
        s.act(EditorAction::Advisor(A::ToggleRecording));
        assert!(s.state.status.contains("Recording on"), "{}", s.state.status);
        s.set_tool(Tool::Truss);
        s.run(Operation::Undo); // not available: refused, so not recorded
        let events = store.read().unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].taken, Suggestion::Tool(Tool::Truss));
        assert_eq!(events[0].context.stage, super::super::advisory::Stage::Empty);
        assert!(events[0].allowed.contains(&Suggestion::Tool(Tool::Truss)));
        // A learned ranking cannot be enabled before a model exists, or from too little data.
        s.act(EditorAction::Advisor(A::ToggleLearned));
        assert!(s.state.status.contains("no trained model"), "{}", s.state.status);
        s.act(EditorAction::Advisor(A::Train));
        let until = std::time::Instant::now()+std::time::Duration::from_secs(10);
        while s.trainer.is_busy() { s.poll(); assert!(std::time::Instant::now()<until); std::thread::sleep(std::time::Duration::from_millis(2)); }
        assert!(s.state.status.contains("not used: needs 20 recorded sessions"), "{}", s.state.status);
        s.act(EditorAction::Advisor(A::ToggleLearned));
        assert!(s.state.status.starts_with("not enabled"), "{}", s.state.status);
        assert!(!s.advisor_menu().unwrap().learned);
        // Choices persist across sessions; hiding suggestions removes them from the panel.
        s.act(EditorAction::Advisor(A::ToggleSuggestions));
        assert!(!texts(&s).iter().any(|l| l.starts_with("suggested")));
        let mut again = session(fea_problem::members::blank_planar());
        again.attach_learning(store.clone(), 2);
        let m = again.advisor_menu().unwrap();
        assert!(m.recording && !m.suggestions);
        again.act(EditorAction::Advisor(A::DeleteRecordings));
        assert!(!store.has_recordings());
        // The environment opt-out wins over everything.
        let mut off = session(fea_problem::members::blank_planar());
        off.configure_advice(None, false, None);
        off.attach_learning(store, 3);
        assert!(off.advisor_menu().is_err());
        off.act(EditorAction::Advisor(A::ToggleRecording));
        assert!(off.state.status.contains("not available"));
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn keyboard_shortcuts_route_to_state_viewer_or_nothing() {
        let mut s = session(templates().remove(1));
        assert_eq!(s.key(Key::Char('2'), Mods::default()), KeyResult::Handled);
        assert_eq!(s.state.tool, Tool::Node);
        assert_eq!(s.key(Key::Char(','), Mods::default()), KeyResult::Command(Command::GainDown));
        assert_eq!(s.key(Key::Tab, Mods { shift: true, ..Default::default() }), KeyResult::Command(Command::CycleField { forward: false }));
        assert_eq!(s.key(Key::Char('m'), Mods::default()), KeyResult::Unhandled);
        assert_eq!(s.key(Key::Char('q'), Mods { command: true, shift: false }), KeyResult::Unhandled);
        // undo/redo via shortcuts
        s.set_tool(Tool::Node);
        s.click(screen(&s, [10.0, 10.0, 0.0]), SIZE, &cam());
        let n = s.state.problem.nodes.len();
        s.key(Key::Char('z'), Mods { command: true, shift: false });
        assert_eq!(s.state.problem.nodes.len(), n - 1);
        s.key(Key::Char('Z'), Mods { command: true, shift: true });
        assert_eq!(s.state.problem.nodes.len(), n);
        s.key(Key::Char('g'), Mods::default());
        s.key(Key::Char('p'), Mods::default());
        assert_eq!(s.state.plane_name(), "x");
    }
    #[test]
    fn escape_cancels_pending_member_then_selection_and_a_running_solve() {
        let mut s = session(templates().remove(1));
        s.set_tool(Tool::Truss);
        s.state.click(Some(Hit::Node(1)), None).unwrap();
        assert!(s.state.pending.is_some());
        s.key(Key::Escape, Mods::default());
        assert!(s.state.pending.is_none());
        s.key(Key::Escape, Mods::default());
        assert!(s.state.selection.is_none());
        s.run(Operation::Solve);
        s.key(Key::Escape, Mods::default());
        wait(&mut s);
        assert!(!s.show_results && s.state.status.contains("cancelled"));
    }
    #[test]
    fn save_and_load_shortcuts_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::new(templates().remove(1), dir.path().join("m.json"));
        s.state.select(Some(Hit::Node(2)));
        s.state.set_property(PropertyKind::Force(1), 321.0).unwrap();
        s.key(Key::Char('s'), Mods { command: true, shift: false });
        assert!(s.state.status.starts_with("Saved") && !s.state.dirty);
        s.state.set_property(PropertyKind::Force(1), 5.0).unwrap();
        s.key(Key::Char('o'), Mods { command: true, shift: false });
        assert!(s.state.status.starts_with("Loaded"));
        assert_eq!(s.state.problem.load(2).unwrap().force[1], 321.0);
        std::fs::write(dir.path().join("m.json"), "garbage").unwrap();
        s.key(Key::Char('o'), Mods { command: true, shift: false });
        assert!(s.state.status.starts_with("Load failed"));
        assert_eq!(s.state.problem.load(2).unwrap().force[1], 321.0, "a failed load keeps the model");
    }
    #[test]
    fn property_manager_ribbon_and_tree_cover_selection_rows_operations_and_guidance() {
        let mut s = session(templates().remove(1));
        let t = texts(&s);
        // Nothing selected: no selection card (the tree lists the entities); and nothing the tree or the title already say.
        assert!(!Authoring::inspector(&s).iter().any(|c| c.id == "selection"));
        assert!(!t.iter().any(|l| l.starts_with("file ") || l.contains(" nodes, ") || l.contains("blue truss")), "{t:?}");
        assert!(t.iter().any(|l| l == "Next step"));
        // Operations live in the ribbon with their rule-derived availability and reasons.
        let ribbon = Authoring::ribbon(&s);
        let buttons: Vec<_> = ribbon.iter().flat_map(|t| &t.groups).flat_map(|g| &g.buttons).collect();
        assert_eq!(buttons.iter().filter(|b| matches!(b.action, UiAction::Editor(EditorAction::SetTool(_)))).count(), 6);
        assert!(buttons.iter().find(|b| b.label == "Solve").unwrap().enabled);
        let results = buttons.iter().find(|b| b.label == "Show results").unwrap();
        assert!(!results.enabled && results.reason.as_deref().is_some_and(|r| r.contains("solve the model first")), "{:?}", results.reason);
        s.state.select(Some(Hit::Node(2)));
        let items = Authoring::properties(&s);
        assert!(items.iter().any(|i| matches!(i, DockItem::Heading(h) if h == "Node 2")));
        assert_eq!(Authoring::inspector(&s)[0].relevance, crate::gpu_viewer::editor::authoring::Relevance::Primary, "the selection comes first");
        assert!(items.iter().any(|i| matches!(i, DockItem::Field { action: Some(UiAction::Editor(EditorAction::Focus(_))), .. })));
        // The feature tree lists the entities and marks the selection; clicking a row selects it.
        let tree = Authoring::tree(&s);
        let node2 = tree.iter().find(|n| n.label.starts_with("Node 2 ")).expect("node 2 row");
        assert!(node2.selected);
        assert_eq!(node2.action, Some(UiAction::Editor(EditorAction::SelectNode(2))));
        let member = s.state.problem.members[0].id;
        s.act(EditorAction::SelectMember(member));
        assert_eq!(s.state.selection, Some(Hit::Member(member)));
        assert!(Authoring::tree(&s).iter().any(|n| n.selected && n.glyph != 'o'));
    }
    #[test]
    fn acting_on_unavailable_operations_explains_instead_of_running() {
        let mut s = session(MemberProblem::new("e"));
        s.run(Operation::Solve);
        assert!(s.state.status.contains("member") && !s.solving());
        s.act(EditorAction::Run(Operation::Delete));
        assert!(s.state.status.contains("select"));
        // With nothing selected the only editable row is the working-plane offset.
        s.act(EditorAction::Focus(0));
        assert!(s.prompting() && s.prompt.as_ref().unwrap().kind == PropertyKind::PlaneOffset);
    }
    #[test]
    fn panel_suggests_the_rule_based_next_action_and_honours_opt_out() {
        let mut s = session(templates().remove(0));
        let text = |s: &Session| format!("{}\n{}", texts(s).join("\n"), Authoring::status(s));
        assert!(text(&s).contains("suggested [rules]") && text(&s).contains("Solve (Enter)"), "{}", text(&s));
        s.configure_advice(None, false, Some("Advisory model x not used".into()));
        assert!(!text(&s).contains("suggested"), "opt-out hides suggestions");
        assert!(text(&s).contains("Advisory model x not used"));
        // A learned model that has not seen this context defers: the label stays "rules".
        let model = crate::gpu_viewer::editor::advisory::CountModel::train(&crate::gpu_viewer::editor::advisory::Trace {
            version: crate::gpu_viewer::editor::advisory::ADVISORY_VERSION,
            provenance: "fixture".into(),
            sessions: 1,
            events: vec![crate::gpu_viewer::editor::advisory::Event {
                context: crate::gpu_viewer::editor::advisory::Context { stage: crate::gpu_viewer::editor::advisory::Stage::Empty, selected: crate::gpu_viewer::editor::advisory::Selected::Nothing, solving: false },
                taken: crate::gpu_viewer::editor::advisory::Suggestion::Tool(Tool::Node),
            }],
        })
        .unwrap();
        s.configure_advice(Some(Box::new(model)), true, None);
        assert!(text(&s).contains("suggested [rules]"));
    }
}
