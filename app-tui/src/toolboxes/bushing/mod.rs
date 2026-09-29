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

pub mod material_picker;
pub mod model;
pub mod persistence;
pub mod reamer_picker;
pub mod view;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::Effect;
use material_picker::{MaterialPickerState, MaterialTarget};
use model::{BushingModel, FieldRow, NumberTarget};
use reamer_picker::ReamerPickerState;

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

pub struct BushingState {
    pub model: BushingModel,
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub material_picker: MaterialPickerState,
    pub reamer_picker: ReamerPickerState,
    /// `d` toggles a text-only panel (Lamé constants + full per-radius
    /// hoop/radial/axial stress field breakdown) - same reasoning as
    /// `PressureVesselState::show_numbers`.
    pub show_numbers: bool,
}

impl Default for BushingState {
    fn default() -> Self {
        let mut state = Self {
            model: BushingModel::default(),
            selected: 0,
            editing: false,
            edit_buffer: String::new(),
            material_picker: MaterialPickerState::default(),
            reamer_picker: ReamerPickerState::default(),
            show_numbers: false,
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

    fn activate_selected(&mut self) {
        let rows = model::field_rows(&self.model);
        match rows.get(self.selected).copied() {
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
            Some(FieldRow::Header(_)) | Some(FieldRow::Number(_)) | None => return,
        }
        self.clamp_selection();
    }
}

/// Toolbox-local key routing, called from `app.rs::handle_key` whenever
/// this toolbox's workspace pane has focus - same `(consumed, effects)`
/// contract as every other toolbox in this crate.
pub fn handle_key(state: &mut BushingState, key: KeyEvent) -> (bool, Vec<Effect>) {
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
            if let KeyCode::Char('m') = key.code {
                state.reamer_picker.open = false;
                state.editing = true;
                state.edit_buffer = model::format_for_edit(state.model.bore_dia);
                return (true, Vec::new());
            }
        }
        return reamer_picker::handle_key(&mut state.reamer_picker, &mut state.model, key);
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
        KeyCode::Char('d') => {
            state.show_numbers = !state.show_numbers;
            (true, Vec::new())
        }
        KeyCode::Char('e') => (true, vec![Effect::ExportBushingReport(view::build_report_text(&state.model))]),
        _ => (false, Vec::new()),
    }
}

fn commit_edit(state: &mut BushingState) {
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
    use crossterm::event::{KeyEventKind, KeyEventState};
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
    fn m_while_filtering_the_reamer_picker_types_into_the_filter_instead() {
        let mut state = BushingState::default();
        state.reamer_picker.open = true;
        state.reamer_picker.filtering = true;
        handle_key(&mut state, key(KeyCode::Char('m')));
        assert!(state.reamer_picker.open, "'m' must filter, not exit to manual entry, while the filter text field has focus");
        assert_eq!(state.reamer_picker.filter_text, "m");
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
    fn e_returns_an_export_effect_with_nonempty_report_text() {
        let mut state = BushingState::default();
        let (consumed, effects) = handle_key(&mut state, key(KeyCode::Char('e')));
        assert!(consumed);
        match effects.as_slice() {
            [Effect::ExportBushingReport(text)] => assert!(!text.is_empty()),
            other => panic!("expected exactly one ExportBushingReport effect, got {other:?}"),
        }
    }
}
