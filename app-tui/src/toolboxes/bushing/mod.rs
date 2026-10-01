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
pub mod friction_picker;
pub mod material_persistence;
pub mod material_picker;
pub mod model;
pub mod persistence;
pub mod reamer_persistence;
pub mod reamer_picker;
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
    pub edit_buffer: String,
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
            friction_picker: FrictionPickerState::default(),
            bushing_id_picker: BushingIdPickerState::default(),
            show_numbers: false,
            results_scroll: 0,
            rec_selected: 0,
            last_applied: None,
            advice: AdviceWindow::default(),
            input_error: None,
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
            BushingAction::Export => return vec![Effect::ExportBushingReport(view::build_report_text(&self.model))],
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
                state.edit_buffer = model::format_for_edit(state.model.bore_dia);
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
            state.edit_buffer = model::format_for_edit(state.model.friction);
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
            state.edit_buffer = model::format_for_edit(state.model.id_bushing);
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
            state.edit_buffer = c.to_string();
            return (true, Vec::new());
        }
    }
    if let Some(c) = crate::widgets::number_edit::number_char(&key) {
        if let Some(FieldRow::Number(target)) = model::field_rows(&state.model).get(state.selected).copied() {
            if !matches!(target, NumberTarget::BoreDia | NumberTarget::Friction | NumberTarget::IdBushing) {
                state.editing = true;
                state.edit_buffer = c.to_string();
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
                    state.edit_buffer = state.model.tolerance_edit_text(group);
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
        KeyCode::Char('d' | 'D') => {
            state.show_numbers = !state.show_numbers;
            (true, Vec::new())
        }
        KeyCode::Char('f' | 'F' | 'a' | 'A') => (true, state.perform(BushingAction::OpenFixes)),
        KeyCode::Char('w' | 'W') => (true, state.perform(BushingAction::OpenExplain)),
        KeyCode::Char('e' | 'E') => (true, vec![Effect::ExportBushingReport(view::build_report_text(&state.model))]),
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
fn tol_buffer_key(buffer: &mut String, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Backspace => {
            buffer.pop();
            true
        }
        KeyCode::Delete => {
            buffer.clear();
            true
        }
        KeyCode::Char(c) if (key.modifiers.is_empty() || key.modifiers == crossterm::event::KeyModifiers::SHIFT) && (c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | ' ' | '/' | ',' | '\u{b1}')) => {
            buffer.push(c);
            true
        }
        _ => false,
    }
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
    fn delete_clears_the_edit_buffer_while_editing() {
        let mut state = BushingState::default();
        state.selected = model::field_rows(&state.model).iter().position(|r| *r == FieldRow::Number(NumberTarget::HousingLen)).unwrap();
        handle_key(&mut state, key(KeyCode::Enter));
        assert!(!state.edit_buffer.is_empty());
        handle_key(&mut state, key(KeyCode::Delete));
        assert_eq!(state.edit_buffer, "");
        assert!(state.editing, "Delete clears the buffer but stays in edit mode");
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
        state.edit_buffer = "0.4995 0.5005".to_string();
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
        state.edit_buffer = "abc".to_string();
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
