//! Lug Analysis toolbox: a pin-loaded lug solved with the rigid-pin contact
//! finite-element model in `lug-solver` (mapped, graded Q9 mesh; the pin is a
//! rigid analytic circle with Gauss-point contact and Coulomb friction),
//! compared with the material allowables.
//!
//! Like the other engineering toolboxes it is a single workspace pane
//! (`PANE_MAIN`) with a field list and a live readout. The analysis runs on a
//! worker thread (`Effect::RunLugAnalysis`) and re-runs automatically a moment
//! after any input stops changing; the condensed model is cached between runs
//! so editing only the pin or the load costs milliseconds.
//!
//! The lug material is chosen in the Material Lookup browser, which opens here
//! as an overlay (`material_browser`).

pub mod mesh_test;
pub mod model;
pub mod view;

use crossterm::event::{KeyCode, KeyEvent};

use crate::app::Effect;
use crate::toolboxes::material_lookup::{self, MaterialLookupState};
use mesh_test::MeshAdvice;
use model::{CachedModel, FieldRow, LugInput, LugRun, LugUiModel};

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

/// Inputs must sit unchanged this long before a re-run starts.
const AUTO_DEBOUNCE_MS: u128 = 250;

/// Which material the open browser is choosing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickTarget {
    #[default]
    Lug,
    Bushing,
}

/// An analysis running on a worker thread.
pub struct LugJob {
    pub id: u64,
    pub sig: LugInput,
    pub started: std::time::Instant,
}

/// A mesh-size test running on a worker thread.
pub struct MeshJob {
    pub id: u64,
    pub sig: String,
    pub started: std::time::Instant,
}

pub struct LugAnalysisState {
    pub model: LugUiModel,
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: crate::widgets::number_edit::EditBuffer,
    /// The material browser while it is open as an overlay.
    pub material_browser: Option<MaterialLookupState>,
    pub browser_target: PickTarget,
    /// `d` adds the bore stress / pressure profile below the results.
    pub show_numbers: bool,
    pub results_scroll: u16,
    pub run: Option<LugRun>,
    /// Why the current inputs cannot be analysed (or the solver's message).
    pub error: Option<String>,
    pub job: Option<LugJob>,
    next_job: u64,
    /// The inputs last seen and when they last changed (for the debounce).
    pub(crate) seen: Option<(LugInput, std::time::Instant)>,
    cache: Option<CachedModel>,
    /// The inputs the last failed analysis was for: they are not retried until an edit.
    failed_sig: Option<LugInput>,
    /// The last mesh-size test with the inputs (`mesh_test::signature`) it was made for, the one running, and why the
    /// last one failed.
    pub mesh_advice: Option<(String, MeshAdvice)>,
    pub mesh_job: Option<MeshJob>,
    pub mesh_error: Option<String>,
}

impl Default for LugAnalysisState {
    fn default() -> Self {
        let mut state = Self {
            model: LugUiModel::default(),
            selected: 0,
            editing: false,
            edit_buffer: Default::default(),
            material_browser: None,
            browser_target: PickTarget::Lug,
            show_numbers: false,
            results_scroll: 0,
            run: None,
            error: None,
            job: None,
            next_job: 1,
            seen: None,
            cache: None,
            failed_sig: None,
            mesh_advice: None,
            mesh_job: None,
            mesh_error: None,
        };
        state.clamp_selection();
        state
    }
}

impl LugAnalysisState {
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

    /// Steps `delta` rows, skipping the non-selectable headers.
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

    /// The selected row (for the mouse handler).
    pub fn selected_row_for_click(&self) -> Option<FieldRow> {
        self.selected_row()
    }

    fn selected_row(&self) -> Option<FieldRow> {
        model::field_rows(&self.model).get(self.selected).copied()
    }

    fn activate_selected(&mut self) -> Vec<Effect> {
        match self.selected_row() {
            Some(FieldRow::ToggleHeadShape) => {
                self.model.toggle_head_shape();
                self.clamp_selection();
            }
            Some(FieldRow::ToggleMeshDensity) => {
                self.model.density = self.model.density.next();
                self.model.elements_around = None;
            }
            Some(FieldRow::MeshSection) => {
                self.model.mesh_open = !self.model.mesh_open;
                self.clamp_selection();
            }
            Some(FieldRow::ToggleAutoRefine) => self.model.auto_refine = !self.model.auto_refine,
            Some(FieldRow::ToggleSolver) => self.model.solver = self.model.solver.next(),
            Some(FieldRow::MeshAdvice) => return self.mesh_advice_action(),
            Some(FieldRow::TogglePlastic) => {
                self.model.plastic = !self.model.plastic;
                self.clamp_selection();
            }
            Some(FieldRow::ToggleFlowRule) => {
                self.model.flow_rule = self.model.flow_rule.next();
                self.clamp_selection();
            }
            Some(FieldRow::TogglePinBody) => self.model.pin_body = self.model.pin_body.next(),
            Some(FieldRow::ToggleFiniteStrain) => {
                self.model.finite_strain = !self.model.finite_strain;
                self.clamp_selection();
            }
            Some(FieldRow::ToggleSecondOrder) => self.model.second_order = !self.model.second_order,
            Some(FieldRow::TogglePinBending) => {
                self.model.pin_bending = !self.model.pin_bending;
                self.clamp_selection();
            }
            Some(FieldRow::ToggleBushing) => {
                self.model.toggle_bushing();
                self.clamp_selection();
            }
            Some(row @ (FieldRow::OpenMaterialPicker | FieldRow::OpenBushingMaterialPicker)) => {
                let target = if row == FieldRow::OpenMaterialPicker { PickTarget::Lug } else { PickTarget::Bushing };
                let current = if target == PickTarget::Lug { self.model.material_index } else { self.model.bushing_material_index };
                let mut browser = MaterialLookupState { picking: true, ..MaterialLookupState::default() };
                // Land on the current material.
                if let Some(p) = browser.hits.iter().position(|&i| i == current) {
                    browser.cursor = p;
                }
                self.browser_target = target;
                self.material_browser = Some(browser);
            }
            _ => {}
        }
        Vec::new()
    }

    /// Enter on the mesh test row: apply a current recommendation, else start the test.
    fn mesh_advice_action(&mut self) -> Vec<Effect> {
        let Ok(input) = self.model.input() else { return Vec::new() };
        let sig = mesh_test::signature(&input);
        if let Some((s, advice)) = &self.mesh_advice {
            if *s == sig {
                self.model.elements_around = Some(advice.recommended);
                return Vec::new();
            }
        }
        if self.mesh_job.is_some() {
            return Vec::new();
        }
        let id = self.next_job;
        self.next_job += 1;
        self.mesh_job = Some(MeshJob { id, sig, started: std::time::Instant::now() });
        self.mesh_error = None;
        vec![Effect::RunLugMeshTest { id, input: Box::new(input) }]
    }

    /// A worker finished mesh test `id`.
    pub fn finish_mesh_test(&mut self, id: u64, result: Result<MeshAdvice, String>) {
        let Some(job) = self.mesh_job.take_if(|j| j.id == id) else { return };
        match result {
            Ok(advice) => {
                self.mesh_advice = Some((job.sig, advice));
                self.mesh_error = None;
            }
            Err(why) => self.mesh_error = Some(why),
        }
    }

    /// The state of the mesh test row for the field list.
    pub fn mesh_advice_text(&self) -> String {
        if let Some(job) = &self.mesh_job {
            return format!("\u{2026} testing mesh sizes ({:.1} s)", job.started.elapsed().as_secs_f64());
        }
        if let Some(why) = &self.mesh_error {
            return format!("failed: {why}");
        }
        let current = self.model.input().ok().map(|i| mesh_test::signature(&i));
        match (&self.mesh_advice, current) {
            (Some((s, advice)), Some(cur)) if *s == cur => advice.summary(),
            (Some(_), _) => "inputs changed since the test (Enter runs it again)".to_string(),
            _ => "not run (Enter runs a brief test)".to_string(),
        }
    }

    /// Called every `Tick` while this toolbox is on screen: starts an analysis
    /// when the inputs have changed and then stayed unchanged for the debounce
    /// (the very first run starts at once), and never runs invalid inputs -
    /// those just report why. Results of a job for older inputs are dropped.
    pub fn tick(&mut self) -> Vec<Effect> {
        let sig = match self.model.input() {
            Ok(sig) => sig,
            Err(why) => {
                self.error = Some(why);
                self.job = None;
                self.seen = None;
                return Vec::new();
            }
        };
        let changed = self.seen.as_ref().is_none_or(|(s, _)| *s != sig);
        if changed {
            self.seen = Some((sig.clone(), std::time::Instant::now()));
            if self.job.as_ref().is_some_and(|j| j.sig != sig) {
                self.job = None; // its result would be stale on arrival
            }
        }
        if self.error.as_deref().is_some_and(|e| !e.starts_with("analysis failed")) {
            self.error = None; // the inputs are valid again
        }
        let first = self.run.is_none() && self.error.is_none();
        let settled = self.seen.as_ref().is_some_and(|(_, at)| at.elapsed().as_millis() >= AUTO_DEBOUNCE_MS);
        let up_to_date = self.run.as_ref().is_some_and(|r| r.input == sig);
        let failed_for_these = self.failed_sig.as_ref() == Some(&sig);
        if up_to_date || self.job.is_some() || failed_for_these || !(settled || first) {
            return Vec::new();
        }
        let id = self.next_job;
        self.next_job += 1;
        self.job = Some(LugJob { id, sig: sig.clone(), started: std::time::Instant::now() });
        vec![Effect::RunLugAnalysis { id, input: Box::new(sig), cache: self.cache.clone() }]
    }

    /// A worker finished job `id`. The cache it hands back is kept either
    /// way; a stale job's result is dropped.
    pub fn finish(&mut self, id: u64, result: Result<LugRun, String>, cache: Option<CachedModel>) {
        if cache.is_some() {
            self.cache = cache;
        }
        let Some(job) = self.job.take_if(|j| j.id == id) else { return };
        match result {
            Ok(run) => {
                self.run = Some(run);
                self.error = None;
                self.failed_sig = None;
            }
            Err(why) => {
                // Keep showing the last good result under the error.
                self.failed_sig = Some(job.sig);
                self.error = Some(format!("analysis failed: {why}"));
            }
        }
    }

    pub fn report_text(&self) -> Option<String> {
        self.run.as_ref().map(model::report_text)
    }

    #[cfg(test)]
    pub(crate) fn cache_for_test(&mut self, cache: Option<CachedModel>) {
        self.cache = cache;
    }
}

fn export_effect(state: &LugAnalysisState) -> Vec<Effect> {
    let (Some(contents), Some(dir)) = (state.report_text(), crate::paths::app_data_dir()) else { return Vec::new() };
    vec![Effect::WriteTextFileAndOpen { path: dir.join("reports").join("lug-analysis-report.txt").to_string_lossy().into_owned(), contents }]
}

/// Toolbox-local key routing - same `(consumed, effects)` contract as the
/// other toolboxes.
pub fn handle_key(state: &mut LugAnalysisState, key: KeyEvent) -> (bool, Vec<Effect>) {
    // The material browser takes every key while it is open.
    if let Some(browser) = state.material_browser.as_mut() {
        match key.code {
            KeyCode::Enter => {
                if let Some(i) = browser.current() {
                    match state.browser_target {
                        PickTarget::Lug => state.model.material_index = i,
                        PickTarget::Bushing => state.model.bushing_material_index = i,
                    }
                }
                state.material_browser = None;
                return (true, Vec::new());
            }
            KeyCode::Esc if browser.query.text.is_empty() => {
                state.material_browser = None;
                return (true, Vec::new());
            }
            _ => {
                let (consumed, effects) = material_lookup::handle_key(browser, key);
                // Tab etc. stay inside the overlay; only the browser's own keys matter.
                return (consumed || !matches!(key.code, KeyCode::Tab | KeyCode::BackTab), effects);
            }
        }
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
            _ if crate::widgets::number_edit::handle_buffer_key(&mut state.edit_buffer, &key) => (true, Vec::new()),
            _ => (false, Vec::new()),
        };
    }

    // Typing a digit straight onto a number row starts editing from that character.
    if let Some(c) = crate::widgets::number_edit::number_char(&key) {
        if matches!(state.selected_row(), Some(FieldRow::Number(_))) {
            state.editing = true;
            state.edit_buffer.set(c.to_string());
            return (true, Vec::new());
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
        KeyCode::Char(' ') => (true, state.activate_selected()),
        KeyCode::Enter => match state.selected_row() {
            Some(FieldRow::Number(target)) => {
                state.editing = true;
                state.edit_buffer.set(model::format_for_edit(state.model.number_value(target)));
                (true, Vec::new())
            }
            Some(FieldRow::Header(_)) | None => (false, Vec::new()),
            Some(_) => (true, state.activate_selected()),
        },
        KeyCode::Char('d' | 'D') => {
            state.show_numbers = !state.show_numbers;
            (true, Vec::new())
        }
        KeyCode::Char('e' | 'E') => (true, export_effect(state)),
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

fn commit_edit(state: &mut LugAnalysisState) {
    if let Some(FieldRow::Number(target)) = state.selected_row() {
        if let Ok(raw) = state.edit_buffer.trim().parse::<f64>() {
            state.model.commit_number(target, raw);
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

    fn select(state: &mut LugAnalysisState, row: FieldRow) {
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == row).unwrap();
    }

    fn finish_ok(state: &mut LugAnalysisState, effects: Vec<Effect>) {
        let [Effect::RunLugAnalysis { id, input, cache }] = effects.as_slice() else { panic!("{effects:?}") };
        let (res, cache2) = model::run(input, cache.clone());
        state.finish(*id, res, cache2);
    }

    #[test]
    fn the_bushing_toggle_shows_its_rows_and_resets_the_pin_to_the_new_bearing_surface() {
        let mut s = LugAnalysisState::default();
        let has = |s: &LugAnalysisState, t| model::field_rows(&s.model).contains(&FieldRow::Number(t));
        assert!(!has(&s, NumberTarget::BushingId));
        select(&mut s, FieldRow::ToggleBushing);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert!(s.model.bushing && has(&s, NumberTarget::BushingId) && has(&s, NumberTarget::BushingInterference));
        assert!((s.model.pin_dia - 0.374).abs() < 1e-9, "pin sits just under the bushing bore: {}", s.model.pin_dia);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert!(!s.model.bushing && (s.model.pin_dia - 0.499).abs() < 1e-9);
    }

    #[test]
    fn the_bushing_material_browser_sets_the_bushing_material_not_the_lug_material() {
        let mut s = LugAnalysisState::default();
        s.model.toggle_bushing();
        let lug_before = s.model.material_index;
        select(&mut s, FieldRow::OpenBushingMaterialPicker);
        handle_key(&mut s, key(KeyCode::Enter));
        assert_eq!(s.browser_target, PickTarget::Bushing);
        for c in "steel 4340".chars() {
            handle_key(&mut s, key(KeyCode::Char(c)));
        }
        let want = s.material_browser.as_ref().unwrap().current().unwrap();
        handle_key(&mut s, key(KeyCode::Enter));
        assert_eq!(s.model.bushing_material_index, want);
        assert_eq!(s.model.material_index, lug_before);
    }

    #[test]
    fn up_down_navigation_skips_headers_and_wraps() {
        let mut s = LugAnalysisState::default();
        let first = s.selected;
        assert!(!matches!(model::field_rows(&s.model)[first], FieldRow::Header(_)));
        handle_key(&mut s, key(KeyCode::Up));
        assert_eq!(s.selected, model::field_rows(&s.model).len() - 1);
        handle_key(&mut s, key(KeyCode::Down));
        assert_eq!(s.selected, first);
    }

    #[test]
    fn space_toggles_the_head_shape_and_the_mesh_density() {
        let mut s = LugAnalysisState::default();
        select(&mut s, FieldRow::ToggleHeadShape);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert!(!s.model.head_round);
        select(&mut s, FieldRow::MeshSection);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert!(s.model.mesh_open, "the mesh section opens");
        select(&mut s, FieldRow::ToggleMeshDensity);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert_eq!(s.model.density, model::MeshDensity::Fine);
        assert!(!s.editing);
    }

    #[test]
    fn the_material_row_opens_the_browser_and_enter_picks_the_highlighted_material() {
        let mut s = LugAnalysisState::default();
        select(&mut s, FieldRow::OpenMaterialPicker);
        handle_key(&mut s, key(KeyCode::Enter));
        assert!(s.material_browser.is_some());
        for c in "ti-6al".chars() {
            handle_key(&mut s, key(KeyCode::Char(c)));
        }
        let want = s.material_browser.as_ref().unwrap().current().unwrap();
        handle_key(&mut s, key(KeyCode::Enter));
        assert!(s.material_browser.is_none());
        assert_eq!(s.model.material_index, want);
        assert!(s.model.material().name.to_lowercase().contains("ti"), "{}", s.model.material().name);
    }

    #[test]
    fn esc_closes_the_browser_without_changing_the_material() {
        let mut s = LugAnalysisState::default();
        let before = s.model.material_index;
        select(&mut s, FieldRow::OpenMaterialPicker);
        handle_key(&mut s, key(KeyCode::Enter));
        handle_key(&mut s, key(KeyCode::Down));
        handle_key(&mut s, key(KeyCode::Esc));
        assert!(s.material_browser.is_none());
        assert_eq!(s.model.material_index, before);
    }

    #[test]
    fn typing_a_digit_on_a_number_row_edits_and_enter_commits() {
        let mut s = LugAnalysisState::default();
        select(&mut s, FieldRow::Number(NumberTarget::Load));
        handle_key(&mut s, key(KeyCode::Char('7')));
        assert!(s.editing);
        handle_key(&mut s, key(KeyCode::Char('5')));
        handle_key(&mut s, key(KeyCode::Char('0')));
        handle_key(&mut s, key(KeyCode::Char('0')));
        handle_key(&mut s, key(KeyCode::Enter));
        assert!(!s.editing);
        assert_eq!(s.model.load, 7500.0);
    }

    #[test]
    fn esc_cancels_an_edit_and_a_bad_value_keeps_the_old_one() {
        let mut s = LugAnalysisState::default();
        select(&mut s, FieldRow::Number(NumberTarget::Thickness));
        handle_key(&mut s, key(KeyCode::Enter));
        assert!(s.editing && s.edit_buffer.trim() == "0.25");
        handle_key(&mut s, key(KeyCode::Esc));
        assert!(!s.editing && s.model.thickness == 0.25);
        handle_key(&mut s, key(KeyCode::Char('-')));
        handle_key(&mut s, key(KeyCode::Enter));
        assert_eq!(s.model.thickness, 0.25, "'-' alone does not parse");
    }

    #[test]
    fn the_first_tick_starts_an_analysis_and_a_second_does_not_start_another() {
        let mut s = LugAnalysisState::default();
        let effects = s.tick();
        assert!(matches!(effects.as_slice(), [Effect::RunLugAnalysis { .. }]), "{effects:?}");
        assert!(s.tick().is_empty(), "a job is already running");
        finish_ok(&mut s, effects);
        assert!(s.run.is_some() && s.job.is_none());
        assert!(s.tick().is_empty(), "the result matches the inputs");
    }

    #[test]
    fn a_change_waits_for_the_debounce_then_reruns_reusing_the_model() {
        let mut s = LugAnalysisState::default();
        let e = s.tick();
        finish_ok(&mut s, e);
        s.model.commit_number(NumberTarget::Load, 3000.0);
        assert!(s.tick().is_empty(), "the edit is too recent");
        s.seen.as_mut().unwrap().1 = std::time::Instant::now() - std::time::Duration::from_millis(400);
        let e = s.tick();
        assert!(matches!(e.as_slice(), [Effect::RunLugAnalysis { cache: Some(_), .. }]), "the cached model is handed to the worker: {e:?}");
        finish_ok(&mut s, e);
        assert!(s.run.as_ref().unwrap().reused_model);
        assert_eq!(s.run.as_ref().unwrap().input.case.load_lbf, 3000.0);
    }

    #[test]
    fn a_job_for_older_inputs_is_dropped_when_the_inputs_change() {
        let mut s = LugAnalysisState::default();
        let old = s.tick();
        let old_id = s.job.as_ref().unwrap().id;
        s.model.commit_number(NumberTarget::Load, 2000.0);
        s.tick(); // sees the new inputs: the running job is stale and is replaced
        let new_job = s.job.as_ref().expect("a job for the new inputs");
        assert_ne!(new_job.id, old_id);
        assert_eq!(new_job.sig.case.load_lbf, 2000.0);
        let [Effect::RunLugAnalysis { id, input, cache }] = old.as_slice() else { panic!() };
        let (res, c) = model::run(input, cache.clone());
        s.finish(*id, res, c);
        assert!(s.run.is_none(), "a stale result is never shown");
        assert!(s.job.is_some(), "the job for the current inputs is still running");
    }

    #[test]
    fn invalid_geometry_reports_why_and_starts_nothing() {
        let mut s = LugAnalysisState::default();
        s.model.commit_number(NumberTarget::HoleDia, 1.8);
        assert!(s.tick().is_empty());
        assert!(s.error.as_deref().is_some_and(|e| e.contains("width")), "{:?}", s.error);
        s.model.commit_number(NumberTarget::HoleDia, 0.5);
        let e = s.tick();
        assert!(matches!(e.as_slice(), [Effect::RunLugAnalysis { .. }]));
        assert!(s.error.is_none());
    }

    #[test]
    fn a_solver_error_is_shown_once_and_not_retried_in_a_loop() {
        let mut s = LugAnalysisState::default();
        let e = s.tick();
        let [Effect::RunLugAnalysis { id, .. }] = e.as_slice() else { panic!() };
        s.finish(*id, Err("boom".into()), None);
        assert_eq!(s.error.as_deref(), Some("analysis failed: boom"));
        assert!(s.tick().is_empty(), "same inputs, same failure: do not spin");
        s.model.commit_number(NumberTarget::Load, 1234.0);
        s.tick();
        s.seen.as_mut().unwrap().1 = std::time::Instant::now() - std::time::Duration::from_millis(400);
        assert!(!s.tick().is_empty(), "an edit allows another attempt");
    }

    #[test]
    fn export_needs_a_result_and_writes_a_report() {
        let mut s = LugAnalysisState::default();
        assert!(handle_key(&mut s, key(KeyCode::Char('e'))).1.is_empty(), "nothing to export yet");
        let e = s.tick();
        finish_ok(&mut s, e);
        let (consumed, effects) = handle_key(&mut s, key(KeyCode::Char('E')));
        assert!(consumed);
        match effects.as_slice() {
            [Effect::WriteTextFileAndOpen { path, contents }] => {
                assert!(path.ends_with("lug-analysis-report.txt") && contents.contains("Lug Analysis"));
            }
            [] => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn uppercase_d_and_e_from_caps_lock_still_work() {
        let mut s = LugAnalysisState::default();
        handle_key(&mut s, key(KeyCode::Char('D')));
        assert!(s.show_numbers, "Caps Lock can report an uppercase letter with no Shift held");
        assert!(handle_key(&mut s, key(KeyCode::Char('E'))).0);
    }

    #[test]
    fn d_toggles_the_numbers_panel_and_page_keys_scroll() {
        let mut s = LugAnalysisState::default();
        handle_key(&mut s, key(KeyCode::Char('d')));
        assert!(s.show_numbers);
        handle_key(&mut s, key(KeyCode::PageDown));
        assert!(s.results_scroll > 0);
        handle_key(&mut s, key(KeyCode::PageUp));
        assert_eq!(s.results_scroll, 0);
    }

    #[test]
    fn the_mesh_section_is_collapsed_by_default_and_opens_with_enter_space_or_a_click() {
        let mut s = LugAnalysisState::default();
        let has = |s: &LugAnalysisState, r: FieldRow| model::field_rows(&s.model).contains(&r);
        assert!(has(&s, FieldRow::MeshSection) && !s.model.mesh_open);
        assert!(!has(&s, FieldRow::Number(NumberTarget::ElementsAround)) && !has(&s, FieldRow::MeshAdvice), "collapsed: no mesh rows");
        select(&mut s, FieldRow::MeshSection);
        handle_key(&mut s, key(KeyCode::Enter));
        assert!(s.model.mesh_open && has(&s, FieldRow::Number(NumberTarget::ElementsAround)) && has(&s, FieldRow::Number(NumberTarget::MaxGrowth)) && has(&s, FieldRow::Number(NumberTarget::FirstLayerAspect)) && has(&s, FieldRow::ToggleAutoRefine) && has(&s, FieldRow::MeshAdvice));
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert!(!s.model.mesh_open, "Space closes it again");
        // The selection stays valid when the rows under it disappear.
        s.model.mesh_open = true;
        select(&mut s, FieldRow::MeshAdvice);
        s.model.mesh_open = false;
        s.clamp_selection();
        assert!(s.selected < model::field_rows(&s.model).len());
    }

    #[test]
    fn typed_mesh_inputs_reach_the_solver_input_and_the_density_preset_clears_the_override() {
        let mut s = LugAnalysisState::default();
        s.model.mesh_open = true;
        s.model.commit_number(NumberTarget::ElementsAround, 60.0);
        s.model.commit_number(NumberTarget::MaxGrowth, 1.4);
        s.model.commit_number(NumberTarget::FirstLayerAspect, 0.8);
        let input = s.model.input().unwrap();
        assert_eq!((input.mesh.elements_around, input.mesh.max_growth, input.mesh.first_layer_aspect), (60, 1.4, 0.8));
        // Out of range values are ignored.
        s.model.commit_number(NumberTarget::ElementsAround, 5.0);
        s.model.commit_number(NumberTarget::MaxGrowth, 3.0);
        assert_eq!(s.model.effective_elements_around(), 60);
        assert_eq!(s.model.max_growth, 1.4);
        select(&mut s, FieldRow::ToggleMeshDensity);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert_eq!(s.model.elements_around, None);
        assert_eq!(s.model.effective_elements_around(), s.model.density.elements_around());
        select(&mut s, FieldRow::ToggleAutoRefine);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert!(!s.model.input().unwrap().auto_refine);
    }

    #[test]
    fn the_mesh_test_row_starts_a_test_then_applies_its_recommendation() {
        let mut s = LugAnalysisState::default();
        s.model.mesh_open = true;
        select(&mut s, FieldRow::MeshAdvice);
        assert!(s.mesh_advice_text().contains("not run"));
        let (_, effects) = handle_key(&mut s, key(KeyCode::Enter));
        let [Effect::RunLugMeshTest { id, input }] = effects.as_slice() else { panic!("{effects:?}") };
        assert!(s.mesh_job.is_some() && s.mesh_advice_text().contains("testing"));
        assert!(handle_key(&mut s, key(KeyCode::Enter)).1.is_empty(), "no second test while one runs");
        // A result for another job is ignored; the right one is stored and shown.
        s.finish_mesh_test(*id + 1, Err("stale".into()));
        assert!(s.mesh_job.is_some());
        let advice = mesh_test::MeshAdvice { rows: vec![], recommended: 54, converged: true, total_ms: 1.0 };
        let sig = mesh_test::signature(input);
        s.finish_mesh_test(*id, Ok(advice));
        assert!(s.mesh_job.is_none() && s.mesh_advice.as_ref().is_some_and(|(k, _)| *k == sig));
        assert!(s.mesh_advice_text().contains("Recommended 54"));
        handle_key(&mut s, key(KeyCode::Enter));
        assert_eq!(s.model.elements_around, Some(54), "Enter applies it");
        // Changing the geometry makes the result stale again.
        s.model.commit_number(NumberTarget::Thickness, 0.3);
        assert!(s.mesh_advice_text().contains("inputs changed"));
        // A failure is shown and does not leave a job behind.
        let (_, effects) = handle_key(&mut s, key(KeyCode::Enter));
        let [Effect::RunLugMeshTest { id, .. }] = effects.as_slice() else { panic!() };
        s.finish_mesh_test(*id, Err("boom".into()));
        assert!(s.mesh_advice_text().contains("boom") && s.mesh_job.is_none());
    }

    #[test]
    fn the_solver_row_cycles_kernel_legacy_compare() {
        let mut s = LugAnalysisState::default();
        assert_eq!(s.model.solver, model::SolverChoice::Kernel);
        select(&mut s, FieldRow::ToggleSolver);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert_eq!(s.model.solver, model::SolverChoice::Legacy);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert_eq!(s.model.solver, model::SolverChoice::Compare);
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert_eq!(s.model.solver, model::SolverChoice::Kernel);
        assert_eq!(s.model.input().unwrap().solver, model::SolverChoice::Kernel);
    }
}
