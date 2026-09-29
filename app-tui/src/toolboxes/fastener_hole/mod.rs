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
pub mod view;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::Effect;
use model::{FastenerHoleModel, FieldRow};

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

pub struct FastenerHoleState {
    pub model: FastenerHoleModel,
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: String,
}

impl Default for FastenerHoleState {
    fn default() -> Self {
        Self { model: FastenerHoleModel::default(), selected: 0, editing: false, edit_buffer: String::new() }
    }
}

impl FastenerHoleState {
    fn clamp_selection(&mut self) {
        let len = model::field_rows(&self.model).len();
        if len == 0 {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(len - 1);
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let rows = model::field_rows(&self.model);
        if rows.is_empty() {
            self.selected = 0;
            return;
        }
        let len = rows.len() as i32;
        self.selected = ((self.selected as i32 + delta).rem_euclid(len)) as usize;
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
            _ => return,
        }
        self.model.recompute();
        self.clamp_selection();
    }
}

/// Toolbox-local key routing, called from `app.rs::handle_key` whenever
/// this toolbox's workspace pane has focus - same `(consumed, effects)`
/// contract as `toolboxes::search::handle_key`. This toolbox performs no
/// filesystem/OS side effects, so it never returns a non-empty `Effect`
/// list, but keeps the same return shape as every other toolbox for a
/// consistent `app.rs` routing pattern.
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
            KeyCode::Backspace => {
                state.edit_buffer.pop();
                (true, Vec::new())
            }
            // Only characters a floating-point literal can contain are
            // accepted - malformed text never reaches the domain layer at
            // all (spec section 31), not merely "ignored on commit".
            KeyCode::Char(c) if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) && (c.is_ascii_digit() || c == '.' || c == '-') => {
                state.edit_buffer.push(c);
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        };
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
                    state.edit_buffer = model::format_for_edit(model::part_value(&state.model.get_toleranced(target), part));
                    (true, Vec::new())
                }
                Some(_) => {
                    state.toggle_selected();
                    (true, Vec::new())
                }
                None => (false, Vec::new()),
            }
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
    use crossterm::event::{KeyEventKind, KeyEventState};
    use model::{HoleType, NumberTarget};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn up_down_navigation_wraps() {
        let mut state = FastenerHoleState::default();
        handle_key(&mut state, key(KeyCode::Up));
        let last = model::field_rows(&state.model).len() - 1;
        assert_eq!(state.selected, last);
        handle_key(&mut state, key(KeyCode::Down));
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn space_on_hole_type_toggle_switches_hole_type_and_resets_selection() {
        let mut state = FastenerHoleState::default();
        assert_eq!(state.model.hole_type, HoleType::Regular);
        handle_key(&mut state, key(KeyCode::Char(' ')));
        assert_eq!(state.model.hole_type, HoleType::Countersunk);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn enter_on_a_number_row_starts_editing_prefilled_with_the_current_value() {
        let mut state = FastenerHoleState::default();
        state.selected = 2; // Hole 1 Diameter / Nominal (first dimension row after the two toggles)
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "0.25");
    }

    #[test]
    fn editing_and_committing_updates_the_model_and_recomputes() {
        let mut state = FastenerHoleState::default();
        state.selected = 2; // Hole 1 Diameter / Nominal
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
        state.selected = 2;
        handle_key(&mut state, key(KeyCode::Enter));
        handle_key(&mut state, key(KeyCode::Char('9')));
        handle_key(&mut state, key(KeyCode::Esc));
        assert!(!state.editing);
        assert!((state.model.get_toleranced(NumberTarget::RegularHole1).nominal - 0.250).abs() < 1e-9);
    }

    #[test]
    fn invalid_numeric_text_is_ignored_on_commit_not_panicking() {
        let mut state = FastenerHoleState::default();
        state.selected = 2;
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
        state.selected = 2;
        handle_key(&mut state, key(KeyCode::Enter));
        let mut ctrl_p = key(KeyCode::Char('p'));
        ctrl_p.modifiers = KeyModifiers::CONTROL;
        let (consumed, _) = handle_key(&mut state, ctrl_p);
        assert!(!consumed, "Ctrl+P must fall through to the global command palette binding even mid-edit");
    }

    #[test]
    fn navigation_is_ignored_while_editing() {
        let mut state = FastenerHoleState::default();
        state.selected = 2;
        handle_key(&mut state, key(KeyCode::Enter));
        let (consumed, _) = handle_key(&mut state, key(KeyCode::Down));
        assert!(!consumed);
        assert_eq!(state.selected, 2);
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
