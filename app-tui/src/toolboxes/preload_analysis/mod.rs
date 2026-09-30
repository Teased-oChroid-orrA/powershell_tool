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
pub mod model;
pub mod persistence;
pub mod view;

use crossterm::event::{KeyCode, KeyEvent};

use bolt_picker::BoltPickerState;
use crate::app::Effect;
use model::{FieldRow, PreloadModel};

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

pub struct PreloadAnalysisState {
    pub model: PreloadModel,
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub bolt_picker: BoltPickerState,
    /// `d` toggles a text-only panel breaking down torque work vs. elastic
    /// strain energy and the full stress-section table (a partial
    /// substitute for spec section 84's optional Load-Preload curve, which
    /// has no meaningful terminal-chart equivalent at this phase).
    pub show_numbers: bool,
    /// PageUp/PageDown-adjusted scroll offset into the Results pane -
    /// clamped on every render by `widgets::scroll_paragraph::render`.
    pub results_scroll: u16,
}

impl Default for PreloadAnalysisState {
    fn default() -> Self {
        let mut state =
            Self { model: PreloadModel::default(), selected: 0, editing: false, edit_buffer: String::new(), bolt_picker: BoltPickerState::default(), show_numbers: false, results_scroll: 0 };
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

    fn activate_selected(&mut self) {
        let rows = model::field_rows(&self.model);
        match rows.get(self.selected).copied() {
            Some(FieldRow::ToggleMode) => self.model.toggle_mode(),
            Some(FieldRow::OpenBoltPicker) => self.bolt_picker = BoltPickerState::open_for(&self.model),
            Some(FieldRow::ToggleTighteningFrom) => self.model.toggle_tightening_from(),
            Some(FieldRow::ToggleBearingModel) => self.model.toggle_bearing_model(),
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
            state.edit_buffer = c.to_string();
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
                    state.edit_buffer = model::format_for_edit(state.model.number_value(target));
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
        KeyCode::Char('e' | 'E') => (true, vec![Effect::ExportPreloadAnalysisReport(view::build_report_text(&state.model))]),
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
    fn delete_clears_the_edit_buffer_while_editing() {
        let mut state = PreloadAnalysisState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::AppliedTorque)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.edit_buffer.is_empty());
        handle_key(&mut state, key(KeyCode::Delete));
        assert_eq!(state.edit_buffer, "");
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
}
