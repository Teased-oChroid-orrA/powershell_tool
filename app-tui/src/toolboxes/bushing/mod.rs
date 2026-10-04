//! Bushing Workbench toolbox: straight/flanged/countersunk interference-fit
//! bushing analysis over `bushing-solver`. Split the same way
//! `toolboxes/pressure_vessel/` is: a `model.rs` bridging the pure solver
//! crate into UI-facing state, this `mod.rs` owning toolbox state +
//! toolbox-local key routing, `view.rs` for rendering, plus this toolbox's
//! own `material_picker.rs` (housing/bushing material selection) and
//! `reamer_picker.rs` (bore-diameter reamer catalog lookup).
//!
//! Like Fastener Holes and Pressure Vessel Analyzer, this toolbox uses a
//! single workspace pane (`PANE_MAIN`) containing one scrollable field
//! list.
//!
//! **Deliberately not ported** from `app`'s/`app-egui`'s own Bushing
//! Workbench: the axial cross-section sketch
//! (`app-egui/src/sketches.rs::bushing_cross_section`,
//! `app/src/bushing_visualizer.rs`) - visual presentation with no terminal
//! equivalent (same call `pressure_vessel/mod.rs`'s own doc comment already
//! made for its own sketches). Every numeric result either GUI head
//! computes is still exposed here, including the full hoop/radial/axial
//! stress field (`d` numbers panel) and the aircraft reamer catalog
//! (`reamer_picker.rs`).

pub mod advice;
pub mod bushing_id_persistence;
pub mod bushing_id_picker;
pub mod edge_check;
pub mod friction_picker;
pub mod material_persistence;
pub mod material_picker;
pub mod model;
pub mod persistence;
pub mod reamer_persistence;
pub mod reamer_picker;
pub mod size_filter;
pub mod view;

use crossterm::event::{KeyCode, KeyEvent};

use bushing_id_picker::BushingIdPickerState;
use crate::app::Effect;
use friction_picker::FrictionPickerState;
use material_picker::{MaterialPickerState, MaterialTarget};
use model::{BushingModel, FieldRow, NumberTarget};
use reamer_picker::ReamerPickerState;

/// The two tabs of the Fixes window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdviceTab {
    Fixes,
    Explain,
}

/// Everything clickable in the Results pane and the Fixes window. One enum so
/// the mouse and the keyboard run exactly the same `BushingState::perform`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BushingAction {
    OpenFixes,
    OpenFixesFor(advice::CheckKind),
    OpenExplain,
    ToggleNumbers,
    /// Run the independent edge-distance cross-check (`c`).
    EdgeCheck,
    /// Same, plus the bushing + housing contact FE (`C`, ~1-3 s).
    EdgeCheckDeep,
    /// Pin/unpin the tooltip for one cross-check (also shown on hover).
    EdgeInfo(edge_check::EdgeTopic),
    Export,
    AdviceTab(AdviceTab),
    AdviceRow(usize),
    AdviceApply,
    AdviceClose,
}

/// The pop-up window listing recommended fixes and the OD-clamp explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdviceWindow {
    pub open: bool,
    pub tab: AdviceTab,
    pub scroll: u16,
}

impl Default for AdviceWindow {
    fn default() -> Self {
        Self { open: false, tab: AdviceTab::Fixes, scroll: 0 }
    }
}

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

pub struct BushingState {
    pub model: BushingModel,
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: crate::widgets::number_edit::EditBuffer,
    pub material_picker: MaterialPickerState,
    pub reamer_picker: ReamerPickerState,
    pub friction_picker: FrictionPickerState,
    pub bushing_id_picker: BushingIdPickerState,
    /// `d` toggles a text-only panel (Lamé constants + full per-radius
    /// hoop/radial/axial stress field breakdown) - same reasoning as
    /// `PressureVesselState::show_numbers`.
    pub show_numbers: bool,
    /// PageUp/PageDown-adjusted scroll offset into the Results pane -
    /// clamped on every render by `widgets::scroll_paragraph::render`, so
    /// it's safe to let this grow past the actual content height (e.g.
    /// after toggling `show_numbers` off shrinks the content).
    pub results_scroll: u16,
    /// Which recommendation `a` applies (`r` cycles it).
    pub rec_selected: usize,
    /// What the last applied recommendation changed - shown atop Results
    /// until the next edit.
    pub last_applied: Option<String>,
    pub advice: AdviceWindow,
    /// Why the last tolerance edit was rejected; shown atop Results until the next edit.
    pub input_error: Option<String>,
    /// The last edge-distance cross-check (key `c`) and the input it ran for.
    pub edge_check: Option<edge_check::EdgeCheckRun>,
    /// Tooltip topic under the mouse (and where), set from `Moved` events.
    pub edge_hover: Option<(edge_check::EdgeTopic, u16, u16)>,
    /// Tooltip pinned by a click (for terminals without mouse motion); Esc closes.
    pub edge_info_pinned: Option<edge_check::EdgeTopic>,
    /// The cross-check running on a worker thread, if any.
    pub edge_job: Option<EdgeJob>,
    /// The persistent tooltip about the last run (finished, failed or
    /// cancelled); closed by Esc, a click, or after [`EDGE_NOTICE_SECS`].
    pub edge_notice: Option<EdgeNotice>,
    next_edge_job: u64,
    /// Whether `c` (false) or `C` (true) ran last: the automatic re-runs use the same set.
    edge_deep_pref: bool,
    /// The live input signature last seen and when it last changed (debounce).
    edge_seen: Option<(edge_check::EdgeSig, std::time::Instant)>,
    /// A change was seen after start-up: automatic runs are on (also on once the user has run a check).
    edge_armed: bool,
}

/// How long the completion tooltip stays up unless dismissed.
pub const EDGE_NOTICE_SECS: u64 = 20;

/// A cross-check running off the UI thread (see [`BushingState::start_edge_check`]).
#[derive(Debug, Clone)]
pub struct EdgeJob {
    pub id: u64,
    pub deep: bool,
    /// Started by the app because the inputs changed, not by the user.
    pub auto: bool,
    pub started: std::time::Instant,
    /// What the job was started for; a later change cancels it.
    pub sig: edge_check::EdgeSig,
}

/// How long the inputs must stay unchanged before an automatic re-run starts.
pub const EDGE_AUTO_DEBOUNCE_MS: u64 = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeNoticeKind {
    Done,
    Short,
    Failed,
    Info,
}

/// Content of the completion tooltip.
#[derive(Debug, Clone)]
pub struct EdgeNotice {
    pub kind: EdgeNoticeKind,
    pub title: String,
    pub lines: Vec<String>,
    pub at: std::time::Instant,
}

impl Default for BushingState {
    fn default() -> Self {
        let mut state = Self {
            model: BushingModel::default(),
            selected: 0,
            editing: false,
            edit_buffer: Default::default(),
            material_picker: MaterialPickerState::default(),
            reamer_picker: ReamerPickerState::default(),
            friction_picker: FrictionPickerState::default(),
            bushing_id_picker: BushingIdPickerState::default(),
            show_numbers: false,
            results_scroll: 0,
            rec_selected: 0,
            last_applied: None,
            advice: AdviceWindow::default(),
            input_error: None,
            edge_check: None,
            edge_hover: None,
            edge_info_pinned: None,
            edge_job: None,
            edge_notice: None,
            next_edge_job: 1,
            edge_deep_pref: false,
            edge_seen: None,
            edge_armed: false,
        };
        // Row 0 is always a `Header` (the first section divider) - land on
        // the first real field instead of an unselectable row.
        state.clamp_selection();
        state
    }
}

impl BushingState {
    pub(crate) fn clamp_selection(&mut self) {
        let rows = model::field_rows(&self.model);
        if rows.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = self.selected.min(rows.len() - 1);
        if matches!(rows[self.selected], FieldRow::Header(_)) {
            self.move_selection(1);
        }
    }

    /// Steps `delta` rows at a time, skipping over non-selectable `Header`
    /// rows (bounded by `rows.len()` steps so an all-header list, which
    /// never actually occurs, still can't loop forever).
    fn move_selection(&mut self, delta: i32) {
        let rows = model::field_rows(&self.model);
        if rows.is_empty() {
            self.selected = 0;
            return;
        }
        let len = rows.len() as i32;
        let mut next = self.selected as i32;
        for _ in 0..rows.len() {
            next = (next + delta).rem_euclid(len);
            if !matches!(rows[next as usize], FieldRow::Header(_)) {
                break;
            }
        }
        self.selected = next as usize;
    }

    /// Runs one Results-pane / Fixes-window action - the single code path for
    /// both mouse clicks and keys.
    pub fn perform(&mut self, action: BushingAction) -> Vec<Effect> {
        match action {
            BushingAction::OpenFixes => {
                if !self.model.recommendations.is_empty() {
                    self.open_advice(AdviceTab::Fixes);
                } else if advice::explain_tolerance(&self.model).is_some() {
                    self.open_advice(AdviceTab::Explain);
                } else {
                    self.last_applied = Some("nothing to fix - no failing check".to_string());
                }
            }
            BushingAction::OpenFixesFor(kind) => {
                if let Some(i) = self.model.recommendations.iter().position(|r| r.fixes == kind) {
                    self.rec_selected = i;
                    self.open_advice(AdviceTab::Fixes);
                } else if kind == advice::CheckKind::Tolerance && advice::explain_tolerance(&self.model).is_some() {
                    self.open_advice(AdviceTab::Explain);
                } else {
                    self.last_applied = Some(format!("{}: no automatic fix available", kind.label()));
                }
            }
            BushingAction::OpenExplain => {
                if advice::explain_tolerance(&self.model).is_some() {
                    self.open_advice(AdviceTab::Explain);
                }
            }
            BushingAction::ToggleNumbers => self.show_numbers = !self.show_numbers,
            BushingAction::EdgeCheck => return self.start_edge_check(false),
            BushingAction::EdgeCheckDeep => return self.start_edge_check(true),
            BushingAction::EdgeInfo(topic) => self.edge_info_pinned = if self.edge_info_pinned == Some(topic) { None } else { Some(topic) },
            BushingAction::Export => return vec![Effect::ExportBushingReport(self.report_text())],
            BushingAction::AdviceTab(tab) => {
                self.advice.tab = tab;
                self.advice.scroll = 0;
            }
            BushingAction::AdviceRow(i) => self.rec_selected = i.min(self.model.recommendations.len().saturating_sub(1)),
            BushingAction::AdviceApply => {
                self.apply_selected_recommendation();
                if self.model.recommendations.is_empty() && advice::explain_tolerance(&self.model).is_none() {
                    self.advice.open = false;
                }
            }
            BushingAction::AdviceClose => self.advice.open = false,
        }
        Vec::new()
    }

    /// Starts the edge-distance cross-check for the current inputs on a
    /// worker thread (the `RunEdgeCheck` effect); the UI stays responsive and
    /// shows a persistent "running" tooltip until
    /// [`finish_edge_check`](Self::finish_edge_check). A second request while
    /// one runs is refused with a note rather than queued.
    pub fn start_edge_check(&mut self, deep: bool) -> Vec<Effect> {
        if let Some(job) = &self.edge_job {
            self.set_notice(EdgeNoticeKind::Info, "Edge check already running", vec![format!("The {} run started {:.1} s ago is still going; wait for it or press Esc to cancel it.", if job.deep { "contact FE" } else { "quick" }, job.started.elapsed().as_secs_f64())]);
            return Vec::new();
        }
        self.edge_deep_pref = deep;
        match self.launch_edge_check(deep, false) {
            Ok(effects) => effects,
            Err(why) => {
                self.input_error = Some(format!("edge-distance check not run: {why}"));
                Vec::new()
            }
        }
    }

    fn launch_edge_check(&mut self, deep: bool, auto: bool) -> Result<Vec<Effect>, String> {
        let (input, cfg) = edge_check::prepare(&self.model, deep)?;
        let id = self.next_edge_job;
        self.next_edge_job += 1;
        self.edge_job = Some(EdgeJob { id, deep, auto, started: std::time::Instant::now(), sig: (input.clone(), cfg.bushing) });
        if !auto {
            self.edge_notice = None;
            self.last_applied = None;
            self.input_error = None;
        }
        Ok(vec![Effect::RunEdgeCheck { id, input, cfg, deep }])
    }

    /// Called every `Tick` while this toolbox is on screen: when an input has
    /// changed (anything the check depends on, the bushing included) and then
    /// stayed unchanged for `EDGE_AUTO_DEBOUNCE_MS`, re-runs the check
    /// (the same set as the last manual run) so the table is never stale for
    /// long. A change cancels a run for older inputs. Does nothing at
    /// start-up (until a value changes or the user has run a check once) and
    /// never reports an invalid input as an error (the manual run does).
    pub fn auto_edge_check(&mut self) -> Vec<Effect> {
        let Some(sig) = edge_check::live_signature(&self.model, self.edge_deep_pref) else { return Vec::new() };
        match &self.edge_seen {
            None => self.edge_seen = Some((sig.clone(), std::time::Instant::now())),
            Some((seen, _)) if *seen != sig => {
                self.edge_seen = Some((sig.clone(), std::time::Instant::now()));
                self.edge_armed = true;
                if self.edge_job.as_ref().is_some_and(|j| j.sig != sig) {
                    self.edge_job = None; // its result would be stale on arrival; restart once settled
                }
            }
            _ => {}
        }
        let settled = self.edge_seen.as_ref().is_some_and(|(_, at)| at.elapsed().as_millis() as u64 >= EDGE_AUTO_DEBOUNCE_MS);
        let armed = self.edge_armed || self.edge_check.is_some();
        let up_to_date = self.edge_check.as_ref().is_some_and(|run| (run.input.clone(), run.bushing) == sig);
        if !settled || !armed || up_to_date || self.edge_job.is_some() {
            return Vec::new();
        }
        self.launch_edge_check(self.edge_deep_pref, true).unwrap_or_default()
    }

    /// A worker finished job `id`. Results of a cancelled or superseded job
    /// are dropped.
    pub fn finish_edge_check(&mut self, id: u64, run: edge_check::EdgeCheckRun) {
        let Some(job) = self.edge_job.take_if(|j| j.id == id) else { return };
        let secs = job.started.elapsed().as_secs_f64();
        let lines = edge_check::completion_lines(&run);
        let short = lines.iter().any(|l| l.contains("SHORT"));
        let title = format!("Edge check finished in {secs:.1} s ({})", if job.deep { "quick set + contact FE" } else { "quick set" });
        self.edge_check = Some(run);
        // A quick automatic re-run is silent (the table just updates); a manual or deep one says what it found.
        if !job.auto || job.deep {
            self.set_notice(if short { EdgeNoticeKind::Short } else { EdgeNoticeKind::Done }, &title, lines);
        }
    }

    /// Esc while a run is going: forget it (the worker finishes on its own;
    /// its result is dropped).
    pub fn cancel_edge_check(&mut self) -> bool {
        match self.edge_job.take() {
            Some(job) => {
                self.set_notice(EdgeNoticeKind::Info, "Edge check cancelled", vec![format!("The {} run was discarded after {:.1} s; the Results table is unchanged.", if job.deep { "contact FE" } else { "quick" }, job.started.elapsed().as_secs_f64())]);
                true
            }
            None => false,
        }
    }

    fn set_notice(&mut self, kind: EdgeNoticeKind, title: &str, lines: Vec<String>) {
        self.edge_notice = Some(EdgeNotice { kind, title: title.to_string(), lines, at: std::time::Instant::now() });
    }

    /// Called once per `Tick`: drops an expired completion tooltip.
    pub fn expire_edge_notice(&mut self) {
        if self.edge_notice.as_ref().is_some_and(|n| n.at.elapsed().as_secs() >= EDGE_NOTICE_SECS) {
            self.edge_notice = None;
        }
    }

    /// Runs the cross-check synchronously for the current inputs (tests and
    /// scripted use; the UI uses [`start_edge_check`](Self::start_edge_check)).
    pub fn run_edge_check(&mut self, deep: bool) {
        match edge_check::run_check(&self.model, deep) {
            Ok(run) => {
                self.edge_check = Some(run);
                self.last_applied = None;
                self.input_error = None;
            }
            Err(why) => self.input_error = Some(format!("edge-distance check not run: {why}")),
        }
    }

    /// The exported report: the model's own text plus the cross-check, if it
    /// has been run for the current inputs.
    pub fn report_text(&self) -> String {
        let mut text = view::build_report_text(&self.model);
        if let Some(run) = &self.edge_check {
            text.push_str(&edge_check::report_text(run));
        }
        text
    }

    fn open_advice(&mut self, tab: AdviceTab) {
        self.advice = AdviceWindow { open: true, tab, scroll: 0 };
        self.last_applied = None;
        self.rec_selected = self.rec_selected.min(self.model.recommendations.len().saturating_sub(1));
    }

    /// Keys while the Fixes window is open (it is modal).
    fn handle_advice_key(&mut self, key: KeyEvent) -> (bool, Vec<Effect>) {
        let has_explain = advice::explain_tolerance(&self.model).is_some();
        match key.code {
            KeyCode::Esc => return (true, self.perform(BushingAction::AdviceClose)),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right if has_explain && !self.model.recommendations.is_empty() => {
                let next = if self.advice.tab == AdviceTab::Fixes { AdviceTab::Explain } else { AdviceTab::Fixes };
                return (true, self.perform(BushingAction::AdviceTab(next)));
            }
            KeyCode::Up => match self.advice.tab {
                AdviceTab::Fixes => {
                    let n = self.model.recommendations.len();
                    if n > 0 {
                        self.rec_selected = (self.rec_selected + n - 1) % n;
                    }
                }
                AdviceTab::Explain => self.advice.scroll = self.advice.scroll.saturating_sub(1),
            },
            KeyCode::Down => match self.advice.tab {
                AdviceTab::Fixes => {
                    let n = self.model.recommendations.len();
                    if n > 0 {
                        self.rec_selected = (self.rec_selected + 1) % n;
                    }
                }
                AdviceTab::Explain => self.advice.scroll = self.advice.scroll.saturating_add(1),
            },
            KeyCode::PageUp => self.advice.scroll = self.advice.scroll.saturating_sub(crate::widgets::scroll_paragraph::SCROLL_STEP),
            KeyCode::PageDown => self.advice.scroll = self.advice.scroll.saturating_add(crate::widgets::scroll_paragraph::SCROLL_STEP),
            KeyCode::Enter | KeyCode::Char('a' | 'A') if self.advice.tab == AdviceTab::Fixes => {
                return (true, self.perform(BushingAction::AdviceApply));
            }
            _ => {}
        }
        (true, Vec::new())
    }

    /// Applies the selected recommendation to the real input fields, moves
    /// the field cursor to the first field it changed, and records what
    /// happened for the Results pane.
    fn apply_selected_recommendation(&mut self) {
        let index = self.rec_selected.min(self.model.recommendations.len().saturating_sub(1));
        let Some(rec) = self.model.recommendations.get(index).cloned() else {
            self.last_applied = Some("nothing to apply - no failing check".to_string());
            return;
        };
        match self.model.apply_recommendation(index) {
            Some(summary) => {
                self.last_applied = Some(summary);
                self.rec_selected = 0;
                if let Some(first) = rec.edits.first() {
                    if let Some(row) = model::field_rows(&self.model).iter().position(|r| *r == FieldRow::Number(first.target)) {
                        self.selected = row;
                    }
                }
            }
            None => self.last_applied = Some(format!("manual change needed: {}", rec.summary)),
        }
    }

    fn activate_selected(&mut self) {
        self.last_applied = None;
        self.input_error = None;
        let rows = model::field_rows(&self.model);
        match rows.get(self.selected).copied() {
            Some(FieldRow::ToggleFitType) => self.model.toggle_fit_type(),
            Some(FieldRow::ToggleToleranceMode) => self.model.toggle_tolerance_mode(),
            Some(FieldRow::ToggleBushingType) => self.model.toggle_bushing_type(),
            Some(FieldRow::ToggleIdType) => self.model.toggle_id_type(),
            Some(FieldRow::ToggleEndConstraint) => self.model.toggle_end_constraint(),
            Some(FieldRow::ToggleCsMode) => self.model.toggle_cs_mode(),
            Some(FieldRow::ToggleExtCsMode) => self.model.toggle_ext_cs_mode(),
            Some(FieldRow::ToggleEnforcementEnabled) => self.model.toggle_enforcement_enabled(),
            Some(FieldRow::ToggleLockBore) => self.model.toggle_lock_bore(),
            Some(FieldRow::TogglePreserveBoreNominal) => self.model.toggle_preserve_bore_nominal(),
            Some(FieldRow::ToggleAllowBoreNominalShift) => self.model.toggle_allow_bore_nominal_shift(),
            Some(FieldRow::ToggleAssemblyThermalEnabled) => self.model.toggle_assembly_thermal_enabled(),
            Some(FieldRow::OpenHousingMaterialPicker) => self.material_picker = MaterialPickerState::open_for(MaterialTarget::Housing),
            Some(FieldRow::OpenBushingMaterialPicker) => self.material_picker = MaterialPickerState::open_for(MaterialTarget::Bushing),
            Some(FieldRow::Header(_)) | Some(FieldRow::Number(_)) | Some(FieldRow::Tol(_)) | None => return,
        }
        self.clamp_selection();
    }
}

/// Toolbox-local key routing, called from `app.rs::handle_key` whenever
/// this toolbox's workspace pane has focus - same `(consumed, effects)`
/// contract as every other toolbox in this crate.
pub fn handle_key(state: &mut BushingState, key: KeyEvent) -> (bool, Vec<Effect>) {
    if key.code == KeyCode::Esc {
        if state.edge_info_pinned.is_some() {
            state.edge_info_pinned = None;
            return (true, Vec::new());
        }
        if state.cancel_edge_check() {
            return (true, Vec::new());
        }
        if state.edge_notice.take().is_some() {
            return (true, Vec::new());
        }
    }
    if state.advice.open {
        return state.handle_advice_key(key);
    }
    if state.material_picker.open {
        return material_picker::handle_key(&mut state.material_picker, &mut state.model, key);
    }
    if state.reamer_picker.open {
        // 'm' drops out of the reamer catalog into the ordinary numeric
        // text-edit mode for Bore Diameter, prefilled with its current
        // value - the "manual entry" escape hatch so a real reamer size is
        // never mandatory. Checked before delegating (not inside
        // `reamer_picker::handle_key`, which knows nothing about the
        // toolbox's text-edit mode) and only when the catalog list isn't
        // itself being text-filtered, so typing "m" while filtering still
        // filters.
        if !state.reamer_picker.filtering {
            if let KeyCode::Char('m' | 'M') = key.code {
                state.reamer_picker.open = false;
                state.editing = true;
                state.edit_buffer.set(model::format_for_edit(state.model.bore_dia));
                return (true, Vec::new());
            }
        }
        return reamer_picker::handle_key(&mut state.reamer_picker, &mut state.model, key);
    }
    if state.friction_picker.open {
        // Same 'm' manual-entry escape hatch as the reamer picker.
        if let KeyCode::Char('m' | 'M') = key.code {
            state.friction_picker.open = false;
            state.editing = true;
            state.edit_buffer.set(model::format_for_edit(state.model.friction));
            return (true, Vec::new());
        }
        return friction_picker::handle_key(&mut state.friction_picker, &mut state.model, key);
    }
    if state.bushing_id_picker.open {
        // Typing "m" while the filter has focus must filter, not leave for manual entry
        // (same rule as the reamer picker above).
        if let (false, KeyCode::Char('m' | 'M')) = (state.bushing_id_picker.filtering, key.code) {
            state.bushing_id_picker.open = false;
            state.editing = true;
            state.edit_buffer.set(model::format_for_edit(state.model.id_bushing));
            return (true, Vec::new());
        }
        return bushing_id_picker::handle_key(&mut state.bushing_id_picker, &mut state.model, key);
    }

    if state.editing {
        return match key.code {
            KeyCode::Enter => {
                commit_edit(state);
                (true, Vec::new())
            }
            KeyCode::Esc => {
                state.editing = false;
                state.edit_buffer.clear();
                (true, Vec::new())
            }
            _ if matches!(model::field_rows(&state.model).get(state.selected), Some(FieldRow::Tol(_))) && tol_buffer_key(&mut state.edit_buffer, &key) => (true, Vec::new()),
            _ if crate::widgets::number_edit::handle_buffer_key(&mut state.edit_buffer, &key) => (true, Vec::new()),
            _ => (false, Vec::new()),
        };
    }

    // Typing a digit/'.'/'-' directly on an already-selected `Number` row
    // starts editing immediately, buffer seeded from that character (not
    // prefilled) - "once you hover it and start typing it updates
    // accordingly". `Enter` (below) still exists too, prefilled with the
    // current value, for tweaking rather than retyping. Picker-backed rows
    // (Bore Diameter/Friction/Bushing ID) are excluded - `Enter` still
    // activates their selection process exactly as before; a bare typed
    // character on those rows is a no-op, matching every other non-`Number`
    // row.
    // A tolerance row starts editing on a digit, '.', '-' or '+'.
    if let KeyCode::Char(c @ ('+' | '-' | '.' | '0'..='9')) = key.code {
        if (key.modifiers.is_empty() || key.modifiers == crossterm::event::KeyModifiers::SHIFT) && matches!(model::field_rows(&state.model).get(state.selected), Some(FieldRow::Tol(_))) {
            state.editing = true;
            state.edit_buffer.set(c.to_string());
            return (true, Vec::new());
        }
    }
    if let Some(c) = crate::widgets::number_edit::number_char(&key) {
        if let Some(FieldRow::Number(target)) = model::field_rows(&state.model).get(state.selected).copied() {
            if !matches!(target, NumberTarget::BoreDia | NumberTarget::Friction | NumberTarget::IdBushing) {
                state.editing = true;
                state.edit_buffer.set(c.to_string());
                return (true, Vec::new());
            }
        }
    }

    match key.code {
        KeyCode::Up => {
            state.move_selection(-1);
            (true, Vec::new())
        }
        KeyCode::Down => {
            state.move_selection(1);
            (true, Vec::new())
        }
        KeyCode::Char(' ') => {
            state.activate_selected();
            (true, Vec::new())
        }
        KeyCode::Enter => {
            let rows = model::field_rows(&state.model);
            match rows.get(state.selected).copied() {
                // Bore Diameter's Enter opens the reamer catalog rather
                // than the ordinary numeric text-edit mode - a real
                // installed bore size almost always comes from a reamer,
                // not an arbitrary decimal. `reamer_picker::open_near`
                // pre-positions the cursor/filter at the closest real size
                // to the current value; 'm' inside it (handled above)
                // reaches the plain numeric editor when a non-catalog value
                // is genuinely needed.
                Some(FieldRow::Number(NumberTarget::BoreDia)) => {
                    state.reamer_picker = ReamerPickerState::open_near(&state.model);
                    (true, Vec::new())
                }
                // Friction's own Enter opens the typical-coefficient picker
                // rather than the ordinary numeric text-edit mode, same
                // "pick a real reference value first, free-type only when
                // genuinely needed" pattern as Bore Diameter's reamer
                // picker - 'm' (handled above) reaches the plain numeric
                // editor.
                Some(FieldRow::Number(NumberTarget::Friction)) => {
                    state.friction_picker = FrictionPickerState::open_now();
                    (true, Vec::new())
                }
                // Bushing ID's own Enter opens its user library picker
                // (same selection/library functionality as Bore Diameter's
                // reamer picker) rather than the ordinary numeric text-edit
                // mode - 'm' (handled above) reaches the plain numeric
                // editor.
                Some(FieldRow::Number(NumberTarget::IdBushing)) => {
                    state.bushing_id_picker = BushingIdPickerState::open_near(&state.model);
                    (true, Vec::new())
                }
                Some(FieldRow::Tol(group)) => {
                    state.editing = true;
                    state.edit_buffer.set(state.model.tolerance_edit_text(group));
                    (true, Vec::new())
                }
                Some(FieldRow::Number(target)) => {
                    state.editing = true;
                    state.edit_buffer.set(model::format_for_edit(state.model.number_value(target)));
                    (true, Vec::new())
                }
                Some(_) => {
                    state.activate_selected();
                    (true, Vec::new())
                }
                None => (false, Vec::new()),
            }
        }
        KeyCode::Char('d' | 'D') => {
            state.show_numbers = !state.show_numbers;
            (true, Vec::new())
        }
        KeyCode::Char('f' | 'F' | 'a' | 'A') => (true, state.perform(BushingAction::OpenFixes)),
        KeyCode::Char('w' | 'W') => (true, state.perform(BushingAction::OpenExplain)),
        KeyCode::Char('c') => (true, state.start_edge_check(false)),
        KeyCode::Char('C') => (true, state.start_edge_check(true)),
        KeyCode::Char('e' | 'E') => (true, vec![Effect::ExportBushingReport(state.report_text())]),
        KeyCode::PageUp => {
            state.results_scroll = state.results_scroll.saturating_sub(crate::widgets::scroll_paragraph::SCROLL_STEP);
            (true, Vec::new())
        }
        KeyCode::PageDown => {
            state.results_scroll = state.results_scroll.saturating_add(crate::widgets::scroll_paragraph::SCROLL_STEP);
            (true, Vec::new())
        }
        _ => (false, Vec::new()),
    }
}

/// Edit-buffer keys for a tolerance row: digits, `.`, signs, separators.
fn tol_buffer_key(buffer: &mut crate::widgets::number_edit::EditBuffer, key: &KeyEvent) -> bool {
    buffer.handle_key(key, |c| c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | ' ' | '/' | ',' | '\u{b1}'))
}

fn commit_edit(state: &mut BushingState) {
    let rows = model::field_rows(&state.model);
    if let Some(FieldRow::Tol(group)) = rows.get(state.selected).copied() {
        match state.model.commit_tolerance_text(group, &state.edit_buffer) {
            Ok(()) => {
                state.last_applied = None;
                state.input_error = None;
            }
            Err(why) => state.input_error = Some(format!("tolerance not changed: {why}")),
        }
        state.editing = false;
        state.edit_buffer.clear();
        return;
    }
    if let Some(FieldRow::Number(target)) = rows.get(state.selected).copied() {
        if let Ok(raw) = state.edit_buffer.trim().parse::<f64>() {
            state.model.commit_number(target, raw);
            state.last_applied = None;
            state.input_error = None;
        }
    }
    state.editing = false;
    state.edit_buffer.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use model::NumberTarget;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn up_down_navigation_wraps() {
        // Row 0 is always a `Header` (non-selectable), so the default
        // selection lands on row 1, the first real field - not row 0.
        let mut state = BushingState::default();
        let first_real_row = state.selected;
        assert_eq!(first_real_row, 1);
        handle_key(&mut state, key(KeyCode::Up));
        let last = model::field_rows(&state.model).len() - 1;
        assert_eq!(state.selected, last);
        handle_key(&mut state, key(KeyCode::Down));
        assert_eq!(state.selected, first_real_row);
    }

    #[test]
    fn space_on_fit_type_toggle_cycles_it_without_entering_edit_mode() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::ToggleFitType).unwrap();
        handle_key(&mut state, key(KeyCode::Char(' ')));
        assert_eq!(state.model.fit_type, model::FitType::Shrink);
        assert!(!state.editing);
    }

    #[test]
    fn space_on_bushing_type_toggle_cycles_it_without_entering_edit_mode() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::ToggleBushingType).unwrap();
        handle_key(&mut state, key(KeyCode::Char(' ')));
        assert_eq!(state.model.bushing_type, bushing_solver::geometry::BushingType::Flanged);
        assert!(!state.editing);
    }

    #[test]
    fn enter_on_the_housing_material_row_opens_the_picker_targeting_housing() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::OpenHousingMaterialPicker).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.material_picker.open);
        assert_eq!(state.material_picker.target, Some(MaterialTarget::Housing));
    }

    #[test]
    fn key_routing_is_handed_to_the_material_picker_while_it_is_open() {
        let mut state = BushingState::default();
        state.material_picker = MaterialPickerState::open_for(MaterialTarget::Bushing);
        state.material_picker.cursor = 6; // ti6al4v
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.material_picker.open);
        assert_eq!(state.model.bushing_material().id, "ti6al4v");
    }

    #[test]
    fn enter_on_bore_diameter_opens_the_reamer_picker_instead_of_text_edit() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::BoreDia)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.reamer_picker.open);
        assert!(!state.editing, "Bore Diameter must not also enter plain text-edit mode");
    }

    #[test]
    fn key_routing_is_handed_to_the_reamer_picker_while_it_is_open() {
        let mut state = BushingState::default();
        state.reamer_picker.open = true;
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.reamer_picker.open);
    }

    #[test]
    fn m_inside_the_reamer_picker_switches_to_manual_numeric_entry() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::BoreDia)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.reamer_picker.open);
        handle_key(&mut state, key(KeyCode::Char('m')));
        assert!(!state.reamer_picker.open);
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "0.5");
    }

    #[test]
    fn uppercase_m_from_caps_lock_still_switches_to_manual_numeric_entry() {
        // Regression: crossterm's Windows backend reports Caps-Lock-typed
        // letters as uppercase even with no Shift held (see
        // `app-tui/AGENTS.md`'s Pitfalls) - a bare `'m'` pattern silently
        // drops this binding on Windows whenever Caps Lock is on.
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::BoreDia)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.reamer_picker.open);
        handle_key(&mut state, key(KeyCode::Char('M')));
        assert!(!state.reamer_picker.open);
        assert!(state.editing);
    }

    #[test]
    fn m_while_filtering_the_reamer_picker_types_into_the_filter_instead() {
        let mut state = BushingState::default();
        state.reamer_picker.open = true;
        state.reamer_picker.filtering = true;
        handle_key(&mut state, key(KeyCode::Char('m')));
        assert!(state.reamer_picker.open, "'m' must filter, not exit to manual entry, while the filter text field has focus");
        assert_eq!(state.reamer_picker.filter_text, "m");
    }

    #[test]
    fn typing_a_digit_on_a_number_row_starts_editing_from_just_that_digit() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::HousingLen)).unwrap();
        handle_key(&mut state, key(KeyCode::Char('7')));
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "7", "buffer must start fresh from the typed digit, not prefilled with the old value");
    }

    #[test]
    fn typing_a_digit_on_a_picker_backed_number_row_is_a_no_op() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::BoreDia)).unwrap();
        let (consumed, _) = handle_key(&mut state, key(KeyCode::Char('7')));
        assert!(!consumed, "Bore Diameter is picker-backed - Enter must still be required to reach it");
        assert!(!state.editing);
    }

    #[test]
    fn arrow_keys_move_the_cursor_and_delete_removes_one_character_while_editing() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::HousingLen)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        let before = state.edit_buffer.to_string();
        assert!(before.chars().count() >= 2, "prefilled value needs two characters for this test: {before}");
        // Home + Delete removes only the first character (not the whole buffer).
        handle_key(&mut state, key(KeyCode::Home));
        handle_key(&mut state, key(KeyCode::Delete));
        assert_eq!(state.edit_buffer.to_string(), before.chars().skip(1).collect::<String>());
        // Right then Backspace removes the new first character.
        handle_key(&mut state, key(KeyCode::Right));
        handle_key(&mut state, key(KeyCode::Backspace));
        assert_eq!(state.edit_buffer.to_string(), before.chars().skip(2).collect::<String>());
        // Left then typing inserts at the start, not the end.
        handle_key(&mut state, key(KeyCode::Left));
        handle_key(&mut state, key(KeyCode::Char('7')));
        assert!(state.edit_buffer.starts_with('7'));
        assert!(state.editing);
    }

    #[test]
    fn enter_on_a_number_row_starts_editing_prefilled_with_the_current_value() {
        // Bore Diameter is deliberately excluded here (its Enter opens the
        // reamer picker instead - see the dedicated test above) - use
        // Housing Length as an ordinary Number row.
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::HousingLen)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "0.5");
    }

    #[test]
    fn editing_and_committing_updates_the_model_and_recomputes() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::Interference)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        state.edit_buffer.clear();
        for c in "0.002".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)));
        }
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.editing);
        assert!((state.model.interference - 0.002).abs() < 1e-9);
    }

    #[test]
    fn esc_cancels_an_edit_without_committing() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::Interference)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        handle_key(&mut state, key(KeyCode::Char('9')));
        handle_key(&mut state, key(KeyCode::Esc));
        assert!(!state.editing);
        assert!((state.model.interference - 0.0015).abs() < 1e-9);
    }

    #[test]
    fn toggling_id_type_clamps_selection_when_row_list_shrinks() {
        let mut state = BushingState::default();
        state.model.id_type = bushing_solver::geometry::IdType::Countersink;
        state.selected = model::field_rows(&state.model).len() - 1;
        state.model.id_type = bushing_solver::geometry::IdType::Straight;
        state.clamp_selection();
        assert!(state.selected < model::field_rows(&state.model).len());
    }

    #[test]
    fn ctrl_modified_char_is_not_consumed_while_editing() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::Interference)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        let mut ctrl_p = key(KeyCode::Char('p'));
        ctrl_p.modifiers = KeyModifiers::CONTROL;
        let (consumed, _) = handle_key(&mut state, ctrl_p);
        assert!(!consumed);
    }

    #[test]
    fn d_toggles_the_numbers_panel() {
        let mut state = BushingState::default();
        assert!(!state.show_numbers);
        handle_key(&mut state, key(KeyCode::Char('d')));
        assert!(state.show_numbers);
    }

    #[test]
    fn page_down_and_page_up_adjust_the_results_scroll() {
        let mut state = BushingState::default();
        assert_eq!(state.results_scroll, 0);
        handle_key(&mut state, key(KeyCode::PageDown));
        assert_eq!(state.results_scroll, crate::widgets::scroll_paragraph::SCROLL_STEP);
        handle_key(&mut state, key(KeyCode::PageUp));
        assert_eq!(state.results_scroll, 0);
        // Never underflows past zero.
        handle_key(&mut state, key(KeyCode::PageUp));
        assert_eq!(state.results_scroll, 0);
    }

    #[test]
    fn uppercase_d_and_e_from_caps_lock_still_work() {
        let mut state = BushingState::default();
        assert!(!state.show_numbers);
        handle_key(&mut state, key(KeyCode::Char('D')));
        assert!(state.show_numbers);
        let (consumed, effects) = handle_key(&mut state, key(KeyCode::Char('E')));
        assert!(consumed);
        assert!(matches!(effects.as_slice(), [Effect::ExportBushingReport(text)] if !text.is_empty()));
    }

    #[test]
    fn e_returns_an_export_effect_with_nonempty_report_text() {
        let mut state = BushingState::default();
        let (consumed, effects) = handle_key(&mut state, key(KeyCode::Char('e')));
        assert!(consumed);
        match effects.as_slice() {
            [Effect::ExportBushingReport(text)] => assert!(!text.is_empty()),
            other => panic!("expected exactly one ExportBushingReport effect, got {other:?}"),
        }
    }

    fn failing_wall_state() -> BushingState {
        let mut state = BushingState::default();
        state.model.commit_number(NumberTarget::MinWallStraight, 0.2);
        state
    }

    #[test]
    fn a_opens_the_fixes_window_and_enter_applies_the_selected_fix_to_the_input_field() {
        let mut state = failing_wall_state();
        assert_eq!(advice::severity_of(&state.model.checks, advice::CheckKind::StraightWall), advice::Severity::Fail);
        state.rec_selected = state.model.recommendations.iter().position(|r| r.fixes == advice::CheckKind::StraightWall).unwrap();
        let before = state.model.id_bushing;
        handle_key(&mut state, key(KeyCode::Char('a')));
        assert!(state.advice.open, "a opens the window, it does not silently change inputs");
        assert_eq!(state.model.id_bushing, before);
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.model.id_bushing < before, "the Bushing ID input must have changed");
        assert_eq!(advice::severity_of(&state.model.checks, advice::CheckKind::StraightWall), advice::Severity::Pass);
        assert!(state.last_applied.as_deref().unwrap().contains("Bushing ID"));
        let rows = model::field_rows(&state.model);
        assert_eq!(rows[state.selected], FieldRow::Number(NumberTarget::IdBushing), "cursor jumps to the field that changed");
    }

    #[test]
    fn the_fixes_window_is_modal_and_navigable_by_keyboard() {
        let mut state = failing_wall_state();
        assert!(state.model.recommendations.len() >= 2);
        handle_key(&mut state, key(KeyCode::Char('F'))); // Caps Lock
        assert!(state.advice.open);
        handle_key(&mut state, key(KeyCode::Down));
        assert_eq!(state.rec_selected, 1);
        handle_key(&mut state, key(KeyCode::Up));
        assert_eq!(state.rec_selected, 0);
        let before = state.selected;
        handle_key(&mut state, key(KeyCode::Char('d'))); // must not leak to the field list
        assert_eq!(state.selected, before);
        assert!(!state.show_numbers);
        handle_key(&mut state, key(KeyCode::Esc));
        assert!(!state.advice.open);
    }

    #[test]
    fn opening_fixes_with_nothing_to_fix_says_so_instead_of_opening_an_empty_window() {
        let mut state = BushingState::default();
        state.model.commit_number(NumberTarget::EdgeDist, 5.0);
        assert!(state.model.recommendations.is_empty());
        handle_key(&mut state, key(KeyCode::Char('a')));
        assert!(!state.advice.open);
        assert!(state.last_applied.as_deref().unwrap().contains("nothing to fix"));
    }

    /// Plays the part of `main.rs`: runs the worker's job and delivers its result.
    fn run_effects(state: &mut BushingState, effects: Vec<Effect>) {
        for e in effects {
            if let Effect::RunEdgeCheck { id, input, cfg, .. } = e {
                let run = edge_check::execute(input, &cfg);
                state.finish_edge_check(id, run);
            }
        }
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn c_starts_the_edge_check_off_the_ui_thread_and_the_result_arrives_with_a_notice() {
        let mut state = BushingState::default();
        assert!(!state.report_text().contains("Edge-Distance Cross-Check"), "no cross-check before it is run");
        let (consumed, effects) = handle_key(&mut state, key(KeyCode::Char('c')));
        assert!(consumed);
        assert!(matches!(effects.as_slice(), [Effect::RunEdgeCheck { deep: false, .. }]), "{effects:?}");
        assert!(state.edge_job.is_some() && state.edge_check.is_none(), "running, nothing stored yet");
        run_effects(&mut state, effects);
        assert!(state.edge_job.is_none());
        let run = state.edge_check.as_ref().expect("the finished run is stored");
        assert_eq!(run.input, edge_check::build_input(&state.model).unwrap(), "the run records the input it was for");
        let notice = state.edge_notice.as_ref().expect("completion tooltip");
        assert!(notice.title.contains("finished") && notice.lines.iter().any(|l| l.starts_with("Superposition")), "{notice:?}");
        assert!(state.report_text().contains("Edge-Distance Cross-Check"));
        let effects = state.perform(BushingAction::Export);
        assert!(matches!(effects.as_slice(), [Effect::ExportBushingReport(t)] if t.contains("Stress superposition")));
    }

    #[test]
    fn capital_c_runs_the_deep_check_including_the_contact_model() {
        let mut state = BushingState::default();
        let (_, effects) = handle_key(&mut state, press(KeyCode::Char('C'), KeyModifiers::SHIFT));
        assert!(matches!(effects.as_slice(), [Effect::RunEdgeCheck { deep: true, .. }]));
        run_effects(&mut state, effects);
        assert!(state.edge_check.as_ref().unwrap().report.models.iter().any(|m| m.id == "contact"));
        let (_, effects) = handle_key(&mut state, key(KeyCode::Char('c')));
        run_effects(&mut state, effects);
        assert!(!state.edge_check.as_ref().unwrap().report.models.iter().any(|m| m.id == "contact"), "c re-runs the quick set");
    }

    #[test]
    fn a_second_request_while_running_is_refused_and_esc_cancels_the_run_and_drops_its_result() {
        let mut state = BushingState::default();
        let (_, first) = handle_key(&mut state, press(KeyCode::Char('C'), KeyModifiers::SHIFT));
        let (_, second) = handle_key(&mut state, key(KeyCode::Char('c')));
        assert!(second.is_empty(), "one run at a time");
        assert!(state.edge_notice.as_ref().unwrap().title.contains("already running"));
        let (consumed, _) = handle_key(&mut state, key(KeyCode::Esc));
        assert!(consumed && state.edge_job.is_none());
        assert!(state.edge_notice.as_ref().unwrap().title.contains("cancelled"));
        run_effects(&mut state, first); // the worker still finishes; its result must be dropped
        assert!(state.edge_check.is_none(), "a cancelled run's result is discarded");
        assert!(state.edge_notice.as_ref().unwrap().title.contains("cancelled"), "and it must not replace the notice");
        // Esc once more closes the notice itself.
        handle_key(&mut state, key(KeyCode::Esc));
        assert!(state.edge_notice.is_none());
    }

    #[test]
    fn a_result_for_an_older_job_is_ignored() {
        let mut state = BushingState::default();
        let (_, first) = handle_key(&mut state, key(KeyCode::Char('c')));
        state.cancel_edge_check();
        let (_, second) = handle_key(&mut state, key(KeyCode::Char('c')));
        run_effects(&mut state, first);
        assert!(state.edge_job.is_some() && state.edge_check.is_none(), "the stale job must not finish the new one");
        run_effects(&mut state, second);
        assert!(state.edge_job.is_none() && state.edge_check.is_some());
    }

    #[test]
    fn the_completion_notice_expires() {
        let mut state = BushingState::default();
        let (_, effects) = handle_key(&mut state, key(KeyCode::Char('c')));
        run_effects(&mut state, effects);
        state.expire_edge_notice();
        assert!(state.edge_notice.is_some(), "fresh notice stays");
        state.edge_notice.as_mut().unwrap().at -= std::time::Duration::from_secs(EDGE_NOTICE_SECS + 1);
        state.expire_edge_notice();
        assert!(state.edge_notice.is_none());
    }

    #[test]
    fn esc_closes_a_pinned_tooltip_before_anything_else() {
        let mut state = BushingState::default();
        state.perform(BushingAction::EdgeInfo(edge_check::EdgeTopic::Allowables));
        assert!(state.edge_info_pinned.is_some());
        let (consumed, _) = handle_key(&mut state, key(KeyCode::Esc));
        assert!(consumed && state.edge_info_pinned.is_none());
    }

    #[test]
    fn an_unusable_edge_distance_reports_why_instead_of_running() {
        let mut state = BushingState::default();
        state.model.commit_number(NumberTarget::EdgeDist, 0.01);
        state.perform(BushingAction::EdgeCheck);
        assert!(state.edge_check.is_none());
        assert!(state.input_error.as_deref().is_some_and(|m| m.contains("edge-distance check not run")));
    }

    #[test]
    fn perform_actions_match_their_keyboard_equivalents() {
        let mut state = failing_wall_state();
        state.perform(BushingAction::ToggleNumbers);
        assert!(state.show_numbers);
        let effects = state.perform(BushingAction::Export);
        assert!(matches!(effects.as_slice(), [Effect::ExportBushingReport(_)]));
        state.perform(BushingAction::OpenFixesFor(advice::CheckKind::StraightWall));
        assert!(state.advice.open && state.advice.tab == AdviceTab::Fixes);
        assert_eq!(state.model.recommendations[state.rec_selected].fixes, advice::CheckKind::StraightWall);
        state.perform(BushingAction::AdviceClose);
        assert!(!state.advice.open);
    }

    #[test]
    fn clamped_tolerance_offers_a_fix_and_an_explanation_tab() {
        let mut state = BushingState::default();
        state.model.commit_number(NumberTarget::BoreTolPlus, 0.002);
        state.model.commit_number(NumberTarget::InterferenceTolPlus, 0.001);
        state.model.commit_number(NumberTarget::InterferenceTolMinus, 0.001);
        state.perform(BushingAction::OpenFixesFor(advice::CheckKind::Tolerance));
        assert!(state.advice.open && state.advice.tab == AdviceTab::Fixes, "a verified fix exists, so Fixes opens first");
        handle_key(&mut state, key(KeyCode::Tab));
        assert_eq!(state.advice.tab, AdviceTab::Explain, "Tab reaches the explanation");
        state.perform(BushingAction::AdviceClose);
        handle_key(&mut state, key(KeyCode::Char('w')));
        assert!(state.advice.open && state.advice.tab == AdviceTab::Explain);
    }

    fn select(state: &mut BushingState, row: FieldRow) {
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == row).expect("row present");
    }

    fn type_text(state: &mut BushingState, text: &str) {
        for c in text.chars() {
            handle_key(state, key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_on_a_tolerance_row_edits_plus_and_minus_in_one_row() {
        let mut state = BushingState::default();
        select(&mut state, FieldRow::Tol(model::TolGroup::Bore));
        type_text(&mut state, "+0.0005 -0.0003");
        assert!(state.editing);
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.editing);
        assert_eq!((state.model.bore_tol_plus, state.model.bore_tol_minus), (0.0005, 0.0003));
        assert!(state.input_error.is_none());
    }

    #[test]
    fn min_max_mode_reads_two_limits_and_keeps_the_nominal_when_inside() {
        let mut state = BushingState::default();
        state.model.toggle_tolerance_mode();
        select(&mut state, FieldRow::Tol(model::TolGroup::Bore));
        handle_key(&mut state, key(KeyCode::Enter));
        state.edit_buffer.set("0.4995 0.5005".to_string());
        handle_key(&mut state, key(KeyCode::Enter));
        assert_eq!(state.model.bore_dia, 0.5, "nominal 0.5 lies inside the limits - kept");
        assert!((state.model.bore_tol_plus - 0.0005).abs() < 1e-12 && (state.model.bore_tol_minus - 0.0005).abs() < 1e-12);
        assert_eq!(state.model.tolerance_display(model::TolGroup::Bore), "0.4995 .. 0.5005");
    }

    #[test]
    fn an_unreadable_tolerance_is_rejected_with_a_reason_and_changes_nothing() {
        let mut state = BushingState::default();
        select(&mut state, FieldRow::Tol(model::TolGroup::Interference));
        handle_key(&mut state, key(KeyCode::Enter));
        state.edit_buffer.set("abc".to_string());
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.input_error.as_deref().unwrap().contains("not a number"));
        assert_eq!(state.model.interference_tol_plus, 0.0);
    }

    #[test]
    fn tolerance_entry_row_toggles_mode_without_changing_the_band() {
        let mut state = BushingState::default();
        state.model.commit_tolerance_text(model::TolGroup::Bore, "+0.001 -0.0005").unwrap();
        select(&mut state, FieldRow::ToggleToleranceMode);
        handle_key(&mut state, key(KeyCode::Char(' ')));
        assert_eq!(state.model.tolerance_mode, model::ToleranceMode::MinMax);
        assert_eq!(state.model.tolerance_display(model::TolGroup::Bore), "0.4995 .. 0.5010");
        handle_key(&mut state, key(KeyCode::Char(' ')));
        assert_eq!(state.model.tolerance_display(model::TolGroup::Bore), "+0.0010 / -0.0005");
    }

    #[test]
    fn tolerance_rows_replace_the_separate_plus_and_minus_rows() {
        let rows = model::field_rows(&BushingState::default().model);
        assert!(rows.contains(&FieldRow::Tol(model::TolGroup::Bore)) && rows.contains(&FieldRow::Tol(model::TolGroup::Interference)));
        assert!(!rows.iter().any(|r| matches!(r, FieldRow::Number(NumberTarget::BoreTolPlus | NumberTarget::BoreTolMinus | NumberTarget::InterferenceTolPlus | NumberTarget::InterferenceTolMinus))));
        assert!(!rows.contains(&FieldRow::Tol(model::TolGroup::CsDia)), "countersink tolerance rows only exist for countersunk geometry");
    }

    #[test]
    fn m_while_filtering_the_bushing_id_picker_types_into_the_filter() {
        let mut state = BushingState::default();
        state.bushing_id_picker.open = true;
        handle_key(&mut state, key(KeyCode::Char('/')));
        assert!(state.bushing_id_picker.filtering);
        for c in "common".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)));
        }
        assert!(state.bushing_id_picker.open && !state.editing, "the m in 'common' must not open manual entry");
        assert_eq!(state.bushing_id_picker.filter_text, "common");
        // Outside the filter, m still reaches manual entry.
        handle_key(&mut state, key(KeyCode::Esc));
        handle_key(&mut state, key(KeyCode::Char('m')));
        assert!(state.editing && !state.bushing_id_picker.open);
    }
}
