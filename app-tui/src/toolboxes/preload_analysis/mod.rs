//! Fastened Joint Preload Analysis toolbox: models a threaded fastener
//! tightened to a specified installation torque and solves the coupled
//! mechanical state of the fastener and clamped joint over
//! `fastened-joint-solver` (thread + bearing friction torque equilibrium,
//! preload, fastener/member elastic compliance, nut rotation, installation
//! stress state, service-load/separation behavior - see that crate's own
//! `lib.rs` doc comment for the full engineering scope and its two
//! documented, spec-labeled-optional cuts).
//!
//! Split the same way `toolboxes/pressure_vessel/` and `toolboxes/bushing/`
//! are: a `model.rs` bridging the pure solver crate into UI-facing state,
//! this `mod.rs` owning toolbox state + toolbox-local key routing, and
//! `view.rs` for rendering. Single workspace pane (`PANE_MAIN`) containing
//! one scrollable field list, same as every other toolbox in this crate
//! beyond Search Files.
//!
//! No cross-section sketch or load-vs-deformation chart exists here at all
//! (unlike Pressure Vessel Analyzer/Bushing Workbench, which at least have
//! a GUI-only sketch to explicitly *not* port) - this toolbox is new to
//! every GUI head in this repository, so there is no existing visual
//! presentation to compare against or deliberately omit.

pub mod bolt_picker;
pub mod fe_check;
pub mod joint_templates;
pub mod template_picker;
pub mod model;
pub mod persistence;
pub mod view;

use crossterm::event::{KeyCode, KeyEvent};

use bolt_picker::BoltPickerState;
use template_picker::TemplatePickerState;
use crate::app::Effect;
use model::{FieldRow, PreloadModel};

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

pub struct PreloadAnalysisState {
    pub model: PreloadModel,
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: crate::widgets::number_edit::EditBuffer,
    pub bolt_picker: BoltPickerState,
    pub template_picker: TemplatePickerState,
    /// What the last applied joint template was, shown atop Results.
    pub message: Option<String>,
    /// `d` toggles a text-only panel breaking down torque work vs. elastic
    /// strain energy and the full stress-section table (a partial
    /// substitute for spec section 84's optional Load-Preload curve, which
    /// has no meaningful terminal-chart equivalent at this phase).
    pub show_numbers: bool,
    /// PageUp/PageDown-adjusted scroll offset into the Results pane -
    /// clamped on every render by `widgets::scroll_paragraph::render`.
    pub results_scroll: u16,
    /// The finite-element member-compliance cross-check (`fe_check.rs`).
    pub fe: FeCheckState,
}

/// State of the member-compliance cross-check: its last result, the job running, and the debounce.
#[derive(Default)]
pub struct FeCheckState {
    pub result: Option<(fe_check::FeInput, Result<fe_check::FeResult, String>)>,
    job: Option<(u64, fe_check::FeInput, std::time::Instant)>,
    seen: Option<(fe_check::FeInput, std::time::Instant)>,
    next_job: u64,
}

impl FeCheckState {
    /// The result for exactly these inputs.
    pub fn current(&self, input: &fe_check::FeInput) -> Option<&Result<fe_check::FeResult, String>> {
        self.result.as_ref().filter(|(i, _)| i == input).map(|(_, r)| r)
    }

    pub fn running(&self) -> bool {
        self.job.is_some()
    }
}

/// Inputs must sit unchanged this long before the cross-check starts.
const FE_DEBOUNCE_MS: u128 = 400;

impl Default for PreloadAnalysisState {
    fn default() -> Self {
        let mut state =
            Self { model: PreloadModel::default(), selected: 0, editing: false, edit_buffer: Default::default(), bolt_picker: BoltPickerState::default(), template_picker: TemplatePickerState::default(), message: None, show_numbers: false, results_scroll: 0, fe: FeCheckState::default() };
        // Row 0 is always a `Header` - land on the first real field instead
        // of an unselectable row.
        state.clamp_selection();
        state
    }
}

impl PreloadAnalysisState {
    /// Clamps `selected` into range, then nudges off a `Header` row if it
    /// landed on one (a section can shrink out from under the current
    /// selection, e.g. toggling Uncertainty Analysis off).
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

    /// Steps `delta` rows at a time, skipping non-selectable `Header` rows -
    /// same technique `toolboxes/bushing/mod.rs::BushingState::move_selection`
    /// uses.
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

    /// Bookkeeping after the template window applied something (or not).
    /// Called every `Tick` while this toolbox is on screen: starts the FE member-compliance
    /// cross-check once the joint has stopped changing (the very first run starts at once).
    pub fn tick(&mut self) -> Vec<Effect> {
        self.sync_fe_angle();
        let Some(input) = self.model.fe_input() else {
            self.fe.seen = None;
            return Vec::new();
        };
        if self.fe.seen.as_ref().is_none_or(|(s, _)| *s != input) {
            self.fe.seen = Some((input.clone(), std::time::Instant::now()));
        }
        let settled = self.fe.seen.as_ref().is_some_and(|(_, at)| at.elapsed().as_millis() >= FE_DEBOUNCE_MS);
        let first = self.fe.result.is_none();
        if self.fe.job.is_some() || self.fe.current(&input).is_some() || !(settled || first) {
            return Vec::new();
        }
        self.fe.next_job += 1;
        let id = self.fe.next_job;
        self.fe.job = Some((id, input.clone(), std::time::Instant::now()));
        vec![Effect::RunMemberFe { id, input: Box::new(input) }]
    }

    /// Keeps `model.fe_angle_deg` equal to the FE-equivalent cone angle of exactly the current joint
    /// (cleared while the FE result is pending, failed, or for other inputs). Called from `tick`, the
    /// stiffness toggle and `finish_fe`; `FeInput` excludes the solver angle, so there is no feedback.
    pub fn sync_fe_angle(&mut self) {
        let angle = self
            .model
            .fe_input()
            .and_then(|i| self.fe.current(&i).and_then(|r| r.as_ref().ok().and_then(|r| r.equivalent_angle_deg)));
        self.model.set_fe_angle(angle);
    }

    /// A worker finished job `id`; a result for older inputs is kept only as "stale" (it never matches `current`).
    pub fn finish_fe(&mut self, id: u64, result: Result<fe_check::FeResult, String>) {
        let Some((_, input, _)) = self.fe.job.take_if(|(j, _, _)| *j == id) else { return };
        self.fe.result = Some((input, result));
        self.sync_fe_angle();
    }

    pub fn after_template(&mut self, applied: Option<String>) {
        if let Some(name) = applied {
            self.message = Some(format!("Joint template applied: {name}"));
            self.clamp_selection();
        }
    }

    fn activate_selected(&mut self) {
        let rows = model::field_rows(&self.model);
        match rows.get(self.selected).copied() {
            Some(FieldRow::ToggleMode) => self.model.toggle_mode(),
            Some(FieldRow::OpenBoltPicker) => self.bolt_picker = BoltPickerState::open_for(&self.model),
            Some(FieldRow::OpenTemplatePicker) => self.template_picker = TemplatePickerState::open(),
            Some(FieldRow::ToggleTighteningFrom) => self.model.toggle_tightening_from(),
            Some(FieldRow::AdvancedSection) => self.model.advanced_open = !self.model.advanced_open,
            Some(FieldRow::ToggleBearingModel) => self.model.toggle_bearing_model(),
            Some(FieldRow::ToggleMemberStiffness) => {
                self.model.toggle_member_stiffness();
                self.sync_fe_angle();
            }
            Some(FieldRow::ToggleExternalLoadEnabled) => self.model.toggle_external_load_enabled(),
            Some(FieldRow::ToggleSlipEnabled) => self.model.toggle_slip_enabled(),
            Some(FieldRow::ToggleStrengthLimitsEnabled) => self.model.toggle_strength_limits_enabled(),
            Some(FieldRow::ToggleAddMember) => self.model.add_member(),
            Some(FieldRow::ToggleRemoveMember) => self.model.remove_member(),
            Some(FieldRow::ToggleUncertaintyEnabled) => self.model.toggle_uncertainty_enabled(),
            Some(FieldRow::ToggleMonteCarloEnabled) => self.model.toggle_monte_carlo_enabled(),
            Some(FieldRow::ToggleThreadLoadDistributionEnabled) => self.model.toggle_thread_load_distribution_enabled(),
            Some(FieldRow::Header(_)) | Some(FieldRow::Number(_)) | None => return,
        }
        self.clamp_selection();
    }
}

/// Toolbox-local key routing, called from `app.rs::handle_key` whenever
/// this toolbox's workspace pane has focus - same `(consumed, effects)`
/// contract as every other toolbox in this crate.
pub fn handle_key(state: &mut PreloadAnalysisState, key: KeyEvent) -> (bool, Vec<Effect>) {
    if state.bolt_picker.open {
        return bolt_picker::handle_key(&mut state.bolt_picker, &mut state.model, key);
    }
    if state.template_picker.open {
        let (consumed, applied) = template_picker::handle_key(&mut state.template_picker, &mut state.model, key);
        state.after_template(applied);
        return (consumed, Vec::new());
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

    // Typing a digit/'.'/'-' directly on an already-selected `Number` row
    // starts editing immediately, buffer seeded from that character - see
    // `bushing::handle_key`'s own comment for the interaction rationale.
    // `OpenBoltPicker` is its own `FieldRow` variant (not `Number`), so no
    // exclusion is needed here.
    if let Some(c) = crate::widgets::number_edit::number_char(&key) {
        if matches!(model::field_rows(&state.model).get(state.selected).copied(), Some(FieldRow::Number(_))) {
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
        KeyCode::Char(' ') => {
            state.activate_selected();
            (true, Vec::new())
        }
        KeyCode::Enter => {
            let rows = model::field_rows(&state.model);
            match rows.get(state.selected).copied() {
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
        KeyCode::Char('t' | 'T') => {
            state.template_picker = TemplatePickerState::open();
            (true, Vec::new())
        }
        KeyCode::Char('e' | 'E') => (true, vec![Effect::ExportPreloadAnalysisReport(view::build_report_text(&state.model, &state.fe))]),
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

fn commit_edit(state: &mut PreloadAnalysisState) {
    let rows = model::field_rows(&state.model);
    if let Some(FieldRow::Number(target)) = rows.get(state.selected).copied() {
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

    #[test]
    fn up_down_navigation_wraps() {
        // Row 0 is always a `Header` (non-selectable), so the default
        // selection lands on row 1, the first real field - not row 0.
        let mut state = PreloadAnalysisState::default();
        let first_real_row = state.selected;
        assert_eq!(first_real_row, 1);
        handle_key(&mut state, key(KeyCode::Up));
        let last = model::field_rows(&state.model).len() - 1;
        assert_eq!(state.selected, last);
        handle_key(&mut state, key(KeyCode::Down));
        assert_eq!(state.selected, first_real_row);
    }

    #[test]
    fn space_on_mode_toggle_cycles_it_without_entering_edit_mode() {
        let mut state = PreloadAnalysisState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::ToggleMode).unwrap();
        handle_key(&mut state, key(KeyCode::Char(' ')));
        assert_eq!(state.model.mode, model::Mode::PreloadControlled);
        assert!(!state.editing);
    }

    #[test]
    fn typing_a_digit_on_a_number_row_starts_editing_from_just_that_digit() {
        let mut state = PreloadAnalysisState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::AppliedTorque)).unwrap();
        handle_key(&mut state, key(KeyCode::Char('9')));
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "9", "buffer must start fresh from the typed digit, not prefilled with the old value");
    }

    #[test]
    fn arrow_keys_move_the_cursor_and_delete_removes_one_character_while_editing() {
        let mut state = PreloadAnalysisState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::AppliedTorque)).unwrap();
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
        let mut state = PreloadAnalysisState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::AppliedTorque)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "360");
    }

    #[test]
    fn editing_and_committing_updates_the_model_and_recomputes() {
        let mut state = PreloadAnalysisState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::AppliedTorque)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        state.edit_buffer.clear();
        for c in "60".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)));
        }
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.editing);
        assert_eq!(state.model.applied_torque, 60.0);
    }

    #[test]
    fn esc_cancels_an_edit_without_committing() {
        let mut state = PreloadAnalysisState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::AppliedTorque)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        handle_key(&mut state, key(KeyCode::Char('9')));
        handle_key(&mut state, key(KeyCode::Esc));
        assert!(!state.editing);
        assert_eq!(state.model.applied_torque, 360.0);
    }

    #[test]
    fn adding_a_member_via_space_grows_the_row_list() {
        let mut state = PreloadAnalysisState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::ToggleAddMember).unwrap();
        let before = model::field_rows(&state.model).len();
        handle_key(&mut state, key(KeyCode::Char(' ')));
        assert_eq!(state.model.members.len(), 2);
        assert!(model::field_rows(&state.model).len() > before);
    }

    #[test]
    fn toggling_mode_clamps_selection_when_row_list_shrinks() {
        let mut state = PreloadAnalysisState::default();
        state.model.mode = model::Mode::RotationControlled;
        state.selected = model::field_rows(&state.model).len() - 1;
        state.model.mode = model::Mode::TorqueControlled;
        state.clamp_selection();
        assert!(state.selected < model::field_rows(&state.model).len());
    }

    #[test]
    fn ctrl_modified_char_is_not_consumed_while_editing() {
        let mut state = PreloadAnalysisState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::AppliedTorque)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        let mut ctrl_p = key(KeyCode::Char('p'));
        ctrl_p.modifiers = KeyModifiers::CONTROL;
        let (consumed, _) = handle_key(&mut state, ctrl_p);
        assert!(!consumed);
    }

    #[test]
    fn d_toggles_the_numbers_panel() {
        let mut state = PreloadAnalysisState::default();
        assert!(!state.show_numbers);
        handle_key(&mut state, key(KeyCode::Char('d')));
        assert!(state.show_numbers);
    }

    #[test]
    fn uppercase_d_and_e_from_caps_lock_still_work() {
        let mut state = PreloadAnalysisState::default();
        assert!(!state.show_numbers);
        handle_key(&mut state, key(KeyCode::Char('D')));
        assert!(state.show_numbers);
        let (consumed, effects) = handle_key(&mut state, key(KeyCode::Char('E')));
        assert!(consumed);
        match effects.as_slice() {
            [Effect::ExportPreloadAnalysisReport(text)] => assert!(!text.is_empty()),
            other => panic!("expected exactly one ExportPreloadAnalysisReport effect, got {other:?}"),
        }
    }

    #[test]
    fn e_returns_an_export_effect_with_nonempty_report_text() {
        let mut state = PreloadAnalysisState::default();
        let (consumed, effects) = handle_key(&mut state, key(KeyCode::Char('e')));
        assert!(consumed);
        match effects.as_slice() {
            [Effect::ExportPreloadAnalysisReport(text)] => assert!(!text.is_empty()),
            other => panic!("expected exactly one ExportPreloadAnalysisReport effect, got {other:?}"),
        }
    }

    #[test]
    fn the_fe_member_check_runs_once_per_joint_and_again_after_an_edit() {
        let mut s = PreloadAnalysisState::default();
        let e = s.tick();
        let [Effect::RunMemberFe { id, input }] = e.as_slice() else { panic!("{e:?}") };
        assert!(s.tick().is_empty(), "one job at a time");
        s.finish_fe(*id, fe_check::run(input));
        let current = s.model.fe_input().unwrap();
        let r = s.fe.current(&current).expect("result for these inputs").as_ref().expect("a default joint solves");
        assert!(r.fe.compliance > 0.0 && r.cone > 0.0);
        assert!(s.tick().is_empty(), "nothing to do for an unchanged joint");
        // Edit a member thickness: the result no longer matches, and the check re-runs after the debounce.
        s.model.members[0].thickness *= 1.5;
        s.model.recompute();
        assert!(s.fe.current(&s.model.fe_input().unwrap()).is_none());
        assert!(s.tick().is_empty(), "debounce");
        s.fe.seen = s.fe.seen.take().map(|(i, _)| (i, std::time::Instant::now() - std::time::Duration::from_secs(5)));
        let e = s.tick();
        assert!(matches!(e.as_slice(), [Effect::RunMemberFe { .. }]), "{e:?}");
    }

    #[test]
    fn a_result_for_older_inputs_is_never_shown_as_current() {
        let mut s = PreloadAnalysisState::default();
        let e = s.tick();
        let [Effect::RunMemberFe { id, input }] = e.as_slice() else { panic!() };
        s.model.members[0].hole_diameter *= 1.1;
        s.model.recompute();
        s.finish_fe(*id, fe_check::run(input));
        assert!(s.fe.current(&s.model.fe_input().unwrap()).is_none());
    }

    #[test]
    fn finite_element_stiffness_makes_the_solver_use_the_fe_compliance() {
        let mut s = PreloadAnalysisState::default();
        let cone_c_m = s.model.output.as_ref().unwrap().compliance.c_m;
        let e = s.tick();
        let [Effect::RunMemberFe { id, input }] = e.as_slice() else { panic!("{e:?}") };
        // Cone mode: the FE result is only a cross-check, the solution does not move.
        s.finish_fe(*id, fe_check::run(input));
        assert_eq!(s.model.output.as_ref().unwrap().compliance.c_m, cone_c_m);
        assert!(!s.model.uses_fe_stiffness());
        // Finite Element mode: C_m is the FE compliance (to the angle bisection tolerance).
        assert!(!model::field_rows(&s.model).contains(&FieldRow::ToggleMemberStiffness), "advanced: hidden until the section opens");
        s.model.advanced_open = true;
        s.selected = model::field_rows(&s.model).iter().position(|r| *r == FieldRow::ToggleMemberStiffness).unwrap();
        handle_key(&mut s, key(KeyCode::Char(' ')));
        assert!(s.model.uses_fe_stiffness());
        let fe_c = s.fe.current(&s.model.fe_input().unwrap()).unwrap().as_ref().unwrap().fe.compliance;
        let c_m = s.model.output.as_ref().unwrap().compliance.c_m;
        assert!((c_m / fe_c - 1.0).abs() < 1e-6, "{c_m:e} vs FE {fe_c:e}");
        assert_ne!(c_m, cone_c_m);
        // Editing the joint drops the (now stale) FE angle at once: no FE value is applied to other inputs.
        s.model.members[0].thickness *= 1.5;
        s.model.recompute();
        s.tick();
        assert!(!s.model.uses_fe_stiffness());
        assert_eq!(s.model.solver_cone_angle_deg(), s.model.cone_half_angle_deg);
    }
}
