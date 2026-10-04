//! Fastener Hole toolbox: defines and analyzes regular and countersunk
//! fastener holes. Split the same way `toolboxes/search/` is: a pure
//! `domain/` tree (zero `ratatui`/`crossterm` - independently testable and
//! reusable by a future frontend), a `model.rs` bridging domain results
//! into UI-facing state, this `mod.rs` owning toolbox state + toolbox-local
//! key routing, and `view.rs` for rendering.
//!
//! Unlike Search Files (three workspace panes: Path/Filters/Results), this
//! toolbox uses a single workspace pane (`PANE_MAIN`) containing one
//! scrollable field list - internal Up/Down/Enter navigation among fields
//! happens inside this one pane, the same self-contained navigation
//! `toolboxes/search/settings_view.rs`'s Fields section already uses
//! within its own screen. This avoids needing a second `AppState`-level
//! pane-focus concept for what is really one logical form.

pub mod domain;
pub mod model;
pub mod persistence;
pub mod view;

use crossterm::event::{KeyCode, KeyEvent};

use crate::app::Effect;
use model::{FastenerHoleModel, FieldRow};

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

pub struct FastenerHoleState {
    pub model: FastenerHoleModel,
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: crate::widgets::number_edit::EditBuffer,
    /// PageUp/PageDown-adjusted scroll offset into the Results pane -
    /// clamped on every render by `widgets::scroll_paragraph::render`.
    pub results_scroll: u16,
}

impl Default for FastenerHoleState {
    fn default() -> Self {
        let mut state = Self { model: FastenerHoleModel::default(), selected: 0, editing: false, edit_buffer: Default::default(), results_scroll: 0 };
        // Row 0 is always a `Header` - land on the first real field instead
        // of an unselectable row (same technique `toolboxes/bushing/mod.rs`
        // uses).
        state.clamp_selection();
        state
    }
}

impl FastenerHoleState {
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

    fn toggle_selected(&mut self) {
        let rows = model::field_rows(&self.model);
        match rows.get(self.selected).copied() {
            Some(FieldRow::ToggleHoleType) => {
                self.model.hole_type = self.model.hole_type.cycle();
                self.selected = 0;
            }
            Some(FieldRow::ToggleToleranceMode) => self.model.tolerance_mode = self.model.tolerance_mode.cycle(),
            Some(FieldRow::ToggleSolveFor) => self.model.countersink.solve_for = self.model.countersink.solve_for.cycle(),
            Some(FieldRow::ToggleSecondaryMethod) => self.model.countersink.secondary_method = self.model.countersink.secondary_method.cycle(),
            Some(FieldRow::Header(_)) | Some(FieldRow::Number(..)) | None => return,
        }
        self.model.recompute();
        self.clamp_selection();
    }
}

/// Toolbox-local key routing, called from `app.rs::handle_key` whenever
/// this toolbox's workspace pane has focus - same `(consumed, effects)`
/// contract as `toolboxes::search::handle_key`. `e` (export report) is the
/// only binding that returns a non-empty `Effect` list - everything else
/// is a plain state mutation, same as before this was added.
pub fn handle_key(state: &mut FastenerHoleState, key: KeyEvent) -> (bool, Vec<Effect>) {
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
    // No picker-backed `Number` row exists in this toolbox.
    if let Some(c) = crate::widgets::number_edit::number_char(&key) {
        if matches!(model::field_rows(&state.model).get(state.selected).copied(), Some(FieldRow::Number(..))) {
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
            state.toggle_selected();
            (true, Vec::new())
        }
        KeyCode::Enter => {
            let rows = model::field_rows(&state.model);
            match rows.get(state.selected).copied() {
                Some(FieldRow::Number(target, part, _)) => {
                    state.editing = true;
                    state.edit_buffer.set(model::format_for_edit(model::part_value(&state.model.get_toleranced(target), part)));
                    (true, Vec::new())
                }
                Some(_) => {
                    state.toggle_selected();
                    (true, Vec::new())
                }
                None => (false, Vec::new()),
            }
        }
        KeyCode::Char('e' | 'E') => (true, vec![Effect::ExportFastenerHoleReport(view::build_report_text(&state.model))]),
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

fn commit_edit(state: &mut FastenerHoleState) {
    let rows = model::field_rows(&state.model);
    if let Some(FieldRow::Number(target, part, _)) = rows.get(state.selected).copied() {
        if let Ok(raw) = state.edit_buffer.trim().parse::<f64>() {
            state.model.commit_number(target, part, raw);
        }
        // A parse failure or an invalid resulting TolerancedValue both
        // leave the previous value in place - malformed/impossible input
        // never reaches the domain layer's stored state.
    }
    state.editing = false;
    state.edit_buffer.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use model::{HoleType, NumberTarget};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn e_returns_an_export_effect_with_nonempty_report_text() {
        let mut state = FastenerHoleState::default();
        let (consumed, effects) = handle_key(&mut state, key(KeyCode::Char('e')));
        assert!(consumed);
        match effects.as_slice() {
            [Effect::ExportFastenerHoleReport(text)] => assert!(!text.is_empty()),
            other => panic!("expected exactly one ExportFastenerHoleReport effect, got {other:?}"),
        }
    }

    #[test]
    fn uppercase_e_from_caps_lock_still_exports() {
        let mut state = FastenerHoleState::default();
        let (consumed, effects) = handle_key(&mut state, key(KeyCode::Char('E')));
        assert!(consumed);
        assert!(matches!(effects.as_slice(), [Effect::ExportFastenerHoleReport(_)]));
    }

    #[test]
    fn build_report_text_contains_both_hole_types_content() {
        for hole_type in [HoleType::Regular, HoleType::Countersunk] {
            let mut model = model::FastenerHoleModel::default();
            model.hole_type = hole_type;
            model.recompute();
            let text = view::build_report_text(&model);
            assert!(text.contains("Fastener Holes Report"));
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn up_down_navigation_wraps() {
        let mut state = FastenerHoleState::default();
        let first_real_row = state.selected;
        handle_key(&mut state, key(KeyCode::Up));
        let last = model::field_rows(&state.model).len() - 1;
        assert_eq!(state.selected, last);
        handle_key(&mut state, key(KeyCode::Down));
        assert_eq!(state.selected, first_real_row);
    }

    #[test]
    fn space_on_hole_type_toggle_switches_hole_type_and_resets_selection() {
        let mut state = FastenerHoleState::default();
        assert_eq!(state.model.hole_type, HoleType::Regular);
        handle_key(&mut state, key(KeyCode::Char(' ')));
        assert_eq!(state.model.hole_type, HoleType::Countersunk);
        // Resets toward the top of the (now very different) row list, but
        // never onto the unselectable Header row 0.
        assert!(!matches!(model::field_rows(&state.model)[state.selected], model::FieldRow::Header(_)));
    }

    #[test]
    fn typing_a_digit_on_a_number_row_starts_editing_from_just_that_digit() {
        let mut state = FastenerHoleState::default();
        state.selected = 4; // Hole 1 Diameter / Nominal
        handle_key(&mut state, key(KeyCode::Char('9')));
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "9", "buffer must start fresh from the typed digit, not prefilled with the old value");
    }

    #[test]
    fn arrow_keys_move_the_cursor_and_delete_removes_one_character_while_editing() {
        let mut state = FastenerHoleState::default();
        state.selected = 4;
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
        let mut state = FastenerHoleState::default();
        state.selected = 4; // Hole 1 Diameter / Nominal (first dimension row after Hole Setup's header+toggles)
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "0.25");
    }

    #[test]
    fn editing_and_committing_updates_the_model_and_recomputes() {
        let mut state = FastenerHoleState::default();
        state.selected = 4; // Hole 1 Diameter / Nominal
        handle_key(&mut state, key(KeyCode::Enter));
        state.edit_buffer.clear();
        for c in "0.240".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)));
        }
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.editing);
        assert!((state.model.get_toleranced(NumberTarget::RegularHole1).nominal - 0.240).abs() < 1e-9);
    }

    #[test]
    fn esc_cancels_an_edit_without_committing() {
        let mut state = FastenerHoleState::default();
        state.selected = 4;
        handle_key(&mut state, key(KeyCode::Enter));
        handle_key(&mut state, key(KeyCode::Char('9')));
        handle_key(&mut state, key(KeyCode::Esc));
        assert!(!state.editing);
        assert!((state.model.get_toleranced(NumberTarget::RegularHole1).nominal - 0.250).abs() < 1e-9);
    }

    #[test]
    fn invalid_numeric_text_is_ignored_on_commit_not_panicking() {
        let mut state = FastenerHoleState::default();
        state.selected = 4;
        handle_key(&mut state, key(KeyCode::Enter));
        state.edit_buffer.clear();
        for c in "not-a-number".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)));
        }
        handle_key(&mut state, key(KeyCode::Enter));
        assert!((state.model.get_toleranced(NumberTarget::RegularHole1).nominal - 0.250).abs() < 1e-9);
    }

    #[test]
    fn ctrl_modified_char_is_not_consumed_while_editing() {
        let mut state = FastenerHoleState::default();
        state.selected = 4;
        handle_key(&mut state, key(KeyCode::Enter));
        let mut ctrl_p = key(KeyCode::Char('p'));
        ctrl_p.modifiers = KeyModifiers::CONTROL;
        let (consumed, _) = handle_key(&mut state, ctrl_p);
        assert!(!consumed, "Ctrl+P must fall through to the global command palette binding even mid-edit");
    }

    #[test]
    fn navigation_is_ignored_while_editing() {
        let mut state = FastenerHoleState::default();
        state.selected = 4;
        handle_key(&mut state, key(KeyCode::Enter));
        let (consumed, _) = handle_key(&mut state, key(KeyCode::Down));
        assert!(!consumed);
        assert_eq!(state.selected, 4);
    }

    #[test]
    fn toggling_solve_for_clamps_selection_when_the_row_list_shrinks() {
        let mut state = FastenerHoleState::default();
        state.model.hole_type = model::HoleType::Countersunk;
        state.selected = model::field_rows(&state.model).len() - 1; // last row: Secondary Hole Diameter's last part
        // Toggling hole type back to Regular drastically changes row count;
        // selection must never point past the end.
        handle_key(&mut state, key(KeyCode::Up)); // sanity: navigation still works before the toggle
        state.selected = model::field_rows(&state.model).len() - 1;
        state.model.hole_type = model::HoleType::Regular;
        state.clamp_selection();
        assert!(state.selected < model::field_rows(&state.model).len());
    }
}
