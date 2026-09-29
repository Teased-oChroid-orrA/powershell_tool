//! Pressure Vessel Analyzer toolbox: cylindrical-vessel stress, failure-mode,
//! buckling, thermal-stress and minimum-thickness analysis over
//! `pressure-vessel-solver`. Split the same way `toolboxes/fastener_hole/`
//! is: a `model.rs` bridging the pure solver crate into UI-facing state,
//! this `mod.rs` owning toolbox state + toolbox-local key routing, and
//! `view.rs` for rendering. `material_picker.rs` and `persistence.rs` are
//! this toolbox's own additions - a filterable material catalog (mirroring
//! `toolboxes/search/extension_picker.rs`'s pattern) plus an "add new
//! material" form, and cross-relaunch persistence for materials added that
//! way.
//!
//! Like Fastener Holes (and unlike Search Files), this toolbox uses a single
//! workspace pane (`PANE_MAIN`) containing one scrollable field list -
//! internal Up/Down/Enter navigation happens inside this one pane.
//!
//! **Deliberately not ported** from `app`'s/`app-egui`'s own Pressure Vessel
//! Analyzer: the head-on/side/isometric cross-section sketches
//! (`app-egui/src/sketches.rs`'s `pv_head_on`/`pv_side_view`/`pv_isometric`)
//! and the 8-step KaTeX-rendered derivation view (`app/src/pressure_vessel_workbench.rs`'s
//! `PV_FORMULAS`/`pv_derivation_value`) - both are visual presentation, not
//! engineering functionality, and neither has a meaningful terminal
//! equivalent. A text-only partial substitute for the derivation view - the
//! Lamé constants and per-surface stress breakdown, as plain numbers - is
//! provided instead via the `d` details toggle (`view.rs::numbers_panel_lines`).

pub mod material_picker;
pub mod model;
pub mod persistence;
pub mod view;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::Effect;
use material_picker::MaterialPickerState;
use model::{FieldRow, PressureVesselModel};

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

pub struct PressureVesselState {
    pub model: PressureVesselModel,
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub material_picker: MaterialPickerState,
    /// `d` toggles a text-only panel (Lamé constants + per-surface stress
    /// breakdown) appended below the Results readout - see this module's
    /// own top doc comment for why this exists instead of the derivation
    /// view.
    pub show_numbers: bool,
}

impl Default for PressureVesselState {
    fn default() -> Self {
        let mut state = Self {
            model: PressureVesselModel::default(),
            selected: 0,
            editing: false,
            edit_buffer: String::new(),
            material_picker: MaterialPickerState::default(),
            show_numbers: false,
        };
        // Row 0 is always a `Header` - land on the first real field instead
        // of an unselectable row (same technique `toolboxes/bushing/mod.rs`
        // uses).
        state.clamp_selection();
        state
    }
}

impl PressureVesselState {
    pub(crate) fn clamp_selection(&mut self) {
        let rows = model::field_rows();
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
        let rows = model::field_rows();
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
        match model::field_rows()[self.selected] {
            FieldRow::ToggleEndCondition => self.model.toggle_end_condition(),
            FieldRow::OpenMaterialPicker => self.material_picker = MaterialPickerState::open_now(),
            FieldRow::Header(_) | FieldRow::Number(_) => {}
        }
    }
}

/// Toolbox-local key routing, called from `app.rs::handle_key` whenever this
/// toolbox's workspace pane has focus - same `(consumed, effects)` contract
/// as `toolboxes::fastener_hole::handle_key`.
pub fn handle_key(state: &mut PressureVesselState, key: KeyEvent) -> (bool, Vec<Effect>) {
    // The material picker overlay takes priority over the main field list
    // while open - same layering `app::handle_key`/`handle_click` already
    // give Search Files' `extension_picker`.
    if state.material_picker.open {
        return material_picker::handle_key(&mut state.material_picker, &mut state.model, key);
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
            KeyCode::Backspace => {
                state.edit_buffer.pop();
                (true, Vec::new())
            }
            // Only characters a floating-point literal can contain are
            // accepted - malformed text never reaches the domain layer at
            // all, not merely "ignored on commit" (same discipline as
            // `fastener_hole::handle_key`).
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
            state.activate_selected();
            (true, Vec::new())
        }
        KeyCode::Enter => match model::field_rows().get(state.selected).copied() {
            Some(FieldRow::Number(target)) => {
                state.editing = true;
                state.edit_buffer = model::format_for_edit(state.model.number_value(target));
                (true, Vec::new())
            }
            Some(FieldRow::ToggleEndCondition) | Some(FieldRow::OpenMaterialPicker) => {
                state.activate_selected();
                (true, Vec::new())
            }
            Some(FieldRow::Header(_)) | None => (false, Vec::new()),
        },
        KeyCode::Char('d') => {
            state.show_numbers = !state.show_numbers;
            (true, Vec::new())
        }
        KeyCode::Char('e') => (true, vec![Effect::ExportPressureVesselReport(view::build_report_text(&state.model))]),
        _ => (false, Vec::new()),
    }
}

fn commit_edit(state: &mut PressureVesselState) {
    if let Some(FieldRow::Number(target)) = model::field_rows().get(state.selected).copied() {
        if let Ok(raw) = state.edit_buffer.trim().parse::<f64>() {
            state.model.commit_number(target, raw);
        }
        // A parse failure or a non-finite result both leave the previous
        // value in place - see `PressureVesselModel::commit_number`.
    }
    state.editing = false;
    state.edit_buffer.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn up_down_navigation_wraps() {
        let mut state = PressureVesselState::default();
        let first_real_row = state.selected;
        handle_key(&mut state, key(KeyCode::Up));
        assert_eq!(state.selected, model::field_rows().len() - 1);
        handle_key(&mut state, key(KeyCode::Down));
        assert_eq!(state.selected, first_real_row);
    }

    #[test]
    fn space_on_end_condition_toggle_flips_it_without_entering_edit_mode() {
        let mut state = PressureVesselState::default();
        state.selected = model::field_rows().iter().position(|r| *r == FieldRow::ToggleEndCondition).unwrap();
        assert!(state.model.closed_ends);
        handle_key(&mut state, key(KeyCode::Char(' ')));
        assert!(!state.model.closed_ends);
        assert!(!state.editing);
    }

    #[test]
    fn enter_on_the_material_row_opens_the_picker() {
        let mut state = PressureVesselState::default();
        state.selected = model::field_rows().iter().position(|r| *r == FieldRow::OpenMaterialPicker).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.material_picker.open);
    }

    #[test]
    fn key_routing_is_handed_to_the_material_picker_while_it_is_open() {
        let mut state = PressureVesselState::default();
        state.material_picker = MaterialPickerState::open_now();
        state.material_picker.cursor = 2; // steel
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.material_picker.open);
        assert_eq!(state.model.material().id, "steel");
    }

    #[test]
    fn enter_on_a_number_row_starts_editing_prefilled_with_the_current_value() {
        let mut state = PressureVesselState::default();
        state.selected = model::field_rows().iter().position(|r| *r == FieldRow::Number(model::NumberTarget::OuterDiameter)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(state.editing);
        assert_eq!(state.edit_buffer, "6");
    }

    #[test]
    fn editing_and_committing_updates_the_model_and_recomputes() {
        let mut state = PressureVesselState::default();
        state.selected = model::field_rows().iter().position(|r| *r == FieldRow::Number(model::NumberTarget::WallThickness)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        state.edit_buffer.clear();
        for c in "0.500".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)));
        }
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.editing);
        assert!((state.model.wall_thickness - 0.500).abs() < 1e-9);
    }

    #[test]
    fn esc_cancels_an_edit_without_committing() {
        let mut state = PressureVesselState::default();
        state.selected = model::field_rows().iter().position(|r| *r == FieldRow::Number(model::NumberTarget::WallThickness)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        handle_key(&mut state, key(KeyCode::Char('9')));
        handle_key(&mut state, key(KeyCode::Esc));
        assert!(!state.editing);
        assert!((state.model.wall_thickness - 1.0).abs() < 1e-9);
    }

    #[test]
    fn invalid_numeric_text_is_ignored_on_commit_not_panicking() {
        let mut state = PressureVesselState::default();
        state.selected = model::field_rows().iter().position(|r| *r == FieldRow::Number(model::NumberTarget::WallThickness)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        state.edit_buffer.clear();
        for c in "not-a-number".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)));
        }
        handle_key(&mut state, key(KeyCode::Enter));
        assert!((state.model.wall_thickness - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ctrl_modified_char_is_not_consumed_while_editing() {
        let mut state = PressureVesselState::default();
        state.selected = model::field_rows().iter().position(|r| *r == FieldRow::Number(model::NumberTarget::WallThickness)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        let mut ctrl_p = key(KeyCode::Char('p'));
        ctrl_p.modifiers = KeyModifiers::CONTROL;
        let (consumed, _) = handle_key(&mut state, ctrl_p);
        assert!(!consumed, "Ctrl+P must fall through to the global command palette binding even mid-edit");
    }

    #[test]
    fn navigation_is_ignored_while_editing() {
        let mut state = PressureVesselState::default();
        state.selected = model::field_rows().iter().position(|r| *r == FieldRow::Number(model::NumberTarget::WallThickness)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        let (consumed, _) = handle_key(&mut state, key(KeyCode::Down));
        assert!(!consumed);
    }

    #[test]
    fn d_toggles_the_numbers_panel() {
        let mut state = PressureVesselState::default();
        assert!(!state.show_numbers);
        handle_key(&mut state, key(KeyCode::Char('d')));
        assert!(state.show_numbers);
        handle_key(&mut state, key(KeyCode::Char('d')));
        assert!(!state.show_numbers);
    }

    #[test]
    fn e_returns_an_export_effect_with_nonempty_report_text() {
        let mut state = PressureVesselState::default();
        let (consumed, effects) = handle_key(&mut state, key(KeyCode::Char('e')));
        assert!(consumed);
        match effects.as_slice() {
            [Effect::ExportPressureVesselReport(text)] => assert!(!text.is_empty()),
            other => panic!("expected exactly one ExportPressureVesselReport effect, got {other:?}"),
        }
    }
}
