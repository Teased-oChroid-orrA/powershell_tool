//! Filterable material picker + "add new material" sub-form, mirroring
//! `toolboxes/search/extension_picker.rs`'s pattern (a toolbox-local
//! overlay, not a global `ModalState`, since only this one toolbox and
//! Search Files' extension catalog need one): `/` narrows the list by
//! substring, Up/Down navigates it, Enter selects and closes. `n` opens a
//! blank-template add-material form instead of narrowing/selecting -
//! Name/E/Sy/Ftu/Poisson's ratio/Thermal expansion, each edited the same
//! select-row/Enter-to-edit way every other field list in this crate
//! already works, plus a `[Save]` action row that validates and commits
//! all six fields at once.
//!
//! Deliberately reads the model's live `material_catalog()` on every
//! keystroke/render rather than holding its own snapshot copy (unlike
//! `ExtensionPicker::available`, which Search Files' folder scan seeds
//! once) - a material added mid-session via `n` must appear in the very
//! same list without a second synchronization step.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph};
use ratatui::Frame;

use crate::theme::{StatusTone, Theme};
use crate::widgets::empty_state;

use super::model::PressureVesselModel;
use crate::app::Effect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddMaterialField {
    Name,
    E,
    Sy,
    Ftu,
    Nu,
    Alpha,
    Save,
}

pub const ADD_MATERIAL_FIELDS: [AddMaterialField; 7] =
    [AddMaterialField::Name, AddMaterialField::E, AddMaterialField::Sy, AddMaterialField::Ftu, AddMaterialField::Nu, AddMaterialField::Alpha, AddMaterialField::Save];

impl AddMaterialField {
    pub fn label(self) -> &'static str {
        match self {
            AddMaterialField::Name => "Name",
            AddMaterialField::E => "E (ksi)",
            AddMaterialField::Sy => "Yield Strength Sy (ksi)",
            AddMaterialField::Ftu => "Ultimate Strength Ftu (ksi)",
            AddMaterialField::Nu => "Poisson's Ratio",
            AddMaterialField::Alpha => "Thermal Expansion (\u{b5}strain/\u{b0}F)",
            AddMaterialField::Save => "[Save]",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AddMaterialForm {
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: String,
    pub name: String,
    pub e_ksi: String,
    pub sy_ksi: String,
    pub ftu_ksi: String,
    pub nu: String,
    pub alpha_u_f: String,
    pub error: Option<String>,
}

impl Default for AddMaterialForm {
    fn default() -> Self {
        Self {
            selected: 0,
            editing: false,
            edit_buffer: String::new(),
            name: String::new(),
            e_ksi: String::new(),
            sy_ksi: String::new(),
            ftu_ksi: String::new(),
            nu: String::new(),
            alpha_u_f: String::new(),
            error: None,
        }
    }
}

impl AddMaterialForm {
    fn field_text(&self, field: AddMaterialField) -> &str {
        match field {
            AddMaterialField::Name => &self.name,
            AddMaterialField::E => &self.e_ksi,
            AddMaterialField::Sy => &self.sy_ksi,
            AddMaterialField::Ftu => &self.ftu_ksi,
            AddMaterialField::Nu => &self.nu,
            AddMaterialField::Alpha => &self.alpha_u_f,
            AddMaterialField::Save => "",
        }
    }

    fn set_field_text(&mut self, field: AddMaterialField, text: String) {
        match field {
            AddMaterialField::Name => self.name = text,
            AddMaterialField::E => self.e_ksi = text,
            AddMaterialField::Sy => self.sy_ksi = text,
            AddMaterialField::Ftu => self.ftu_ksi = text,
            AddMaterialField::Nu => self.nu = text,
            AddMaterialField::Alpha => self.alpha_u_f = text,
            AddMaterialField::Save => {}
        }
    }

    /// Parses every field and checks it against a physically sane range -
    /// `Err` leaves the form open with the message shown inline, `Ok`
    /// returns the six values ready for `PressureVesselModel::add_custom_material`.
    /// Deliberately permissive on exact bounds (this is a user-entered
    /// material, not a catalog entry curated against a spec sheet) but
    /// strict on "this text isn't even a usable number" and on signs/ranges
    /// that would silently produce nonsense elsewhere (e.g. Poisson's ratio
    /// outside its physically possible range would make
    /// `pressure-vessel-solver`'s own formulas divide by a near-zero or
    /// negative quantity).
    fn validate(&self) -> Result<(String, f64, f64, f64, f64, f64), String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Name must not be blank".to_string());
        }
        let e_ksi = parse_positive(&self.e_ksi, "E")?;
        let sy_ksi = parse_positive(&self.sy_ksi, "Yield strength")?;
        let ftu_ksi = parse_positive(&self.ftu_ksi, "Ultimate strength")?;
        let nu: f64 = self.nu.trim().parse().map_err(|_| "Poisson's ratio must be a number".to_string())?;
        if !(0.0..0.5).contains(&nu) {
            return Err("Poisson's ratio must be between 0 and 0.5".to_string());
        }
        let alpha_u_f: f64 = self.alpha_u_f.trim().parse().map_err(|_| "Thermal expansion must be a number".to_string())?;
        if !alpha_u_f.is_finite() || alpha_u_f < 0.0 {
            return Err("Thermal expansion must be \u{2265} 0".to_string());
        }
        Ok((name.to_string(), e_ksi, sy_ksi, ftu_ksi, nu, alpha_u_f))
    }
}

fn parse_positive(text: &str, label: &str) -> Result<f64, String> {
    let value: f64 = text.trim().parse().map_err(|_| format!("{label} must be a number"))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{label} must be > 0"));
    }
    Ok(value)
}

#[derive(Debug, Clone, Default)]
pub struct MaterialPickerState {
    pub open: bool,
    /// Index into the *visible* (filtered) list, not directly into
    /// `PressureVesselModel::material_catalog()`.
    pub cursor: usize,
    pub filter_text: String,
    pub filtering: bool,
    pub add_form: Option<AddMaterialForm>,
}

impl MaterialPickerState {
    pub fn open_now() -> Self {
        Self { open: true, ..Default::default() }
    }

    fn visible_indices(&self, model: &PressureVesselModel) -> Vec<usize> {
        let needle = self.filter_text.trim().to_lowercase();
        let catalog = model.material_catalog();
        if needle.is_empty() {
            (0..catalog.len()).collect()
        } else {
            catalog.iter().enumerate().filter(|(_, m)| m.name.to_lowercase().contains(&needle)).map(|(i, _)| i).collect()
        }
    }

    fn move_cursor(&mut self, model: &PressureVesselModel, delta: i32) {
        let len = self.visible_indices(model).len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as i32 + delta).rem_euclid(len as i32) as usize;
    }
}

/// `(consumed, effects)` - same contract as every other `handle_key` in
/// this crate. Mutates `model` directly (selecting a material, or adding
/// a new one) rather than staging a result for the caller to apply -
/// consistent with how this crate's toolbox-local overlays already work
/// (e.g. `extension_picker::handle_key` mutates its own picker in place).
pub fn handle_key(picker: &mut MaterialPickerState, model: &mut PressureVesselModel, key: KeyEvent) -> (bool, Vec<Effect>) {
    if picker.add_form.is_some() {
        return handle_add_form_key(picker, model, key);
    }

    if picker.filtering {
        return match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                picker.filtering = false;
                picker.cursor = 0;
                (true, Vec::new())
            }
            KeyCode::Backspace => {
                picker.filter_text.pop();
                picker.cursor = 0;
                (true, Vec::new())
            }
            KeyCode::Delete => {
                picker.filter_text.clear();
                picker.cursor = 0;
                (true, Vec::new())
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                picker.filter_text.push(c);
                picker.cursor = 0;
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        };
    }

    match key.code {
        KeyCode::Char('/') => {
            picker.filtering = true;
            (true, Vec::new())
        }
        KeyCode::Char('n' | 'N') => {
            picker.add_form = Some(AddMaterialForm::default());
            (true, Vec::new())
        }
        KeyCode::Up => {
            picker.move_cursor(model, -1);
            (true, Vec::new())
        }
        KeyCode::Down => {
            picker.move_cursor(model, 1);
            (true, Vec::new())
        }
        KeyCode::Enter => {
            if let Some(&idx) = picker.visible_indices(model).get(picker.cursor) {
                model.select_material(idx);
            }
            picker.open = false;
            (true, Vec::new())
        }
        KeyCode::Esc => {
            if !picker.filter_text.is_empty() {
                picker.filter_text.clear();
                picker.cursor = 0;
            } else {
                picker.open = false;
            }
            (true, Vec::new())
        }
        _ => (false, Vec::new()),
    }
}

fn move_add_form_selection(form: &mut AddMaterialForm, delta: i32) {
    let len = ADD_MATERIAL_FIELDS.len() as i32;
    form.selected = ((form.selected as i32 + delta).rem_euclid(len)) as usize;
}

/// Takes `picker` (not a bare `&mut AddMaterialForm`) specifically so the
/// "close the whole form" cases (Esc on the form itself, a successful
/// Save) can clear `picker.add_form` directly - a function scoped to just
/// the form couldn't tell its own owner to drop it.
fn handle_add_form_key(picker: &mut MaterialPickerState, model: &mut PressureVesselModel, key: KeyEvent) -> (bool, Vec<Effect>) {
    let editing = picker.add_form.as_ref().is_some_and(|f| f.editing);
    if editing {
        let form = picker.add_form.as_mut().expect("editing is only true when add_form is Some");
        return match key.code {
            KeyCode::Enter => {
                let field = ADD_MATERIAL_FIELDS[form.selected];
                let text = std::mem::take(&mut form.edit_buffer);
                form.set_field_text(field, text);
                form.editing = false;
                (true, Vec::new())
            }
            KeyCode::Esc => {
                form.editing = false;
                form.edit_buffer.clear();
                (true, Vec::new())
            }
            KeyCode::Backspace => {
                form.edit_buffer.pop();
                (true, Vec::new())
            }
            KeyCode::Delete => {
                form.edit_buffer.clear();
                (true, Vec::new())
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                form.edit_buffer.push(c);
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        };
    }

    match key.code {
        KeyCode::Up => {
            if let Some(form) = picker.add_form.as_mut() {
                move_add_form_selection(form, -1);
            }
            (true, Vec::new())
        }
        KeyCode::Down => {
            if let Some(form) = picker.add_form.as_mut() {
                move_add_form_selection(form, 1);
            }
            (true, Vec::new())
        }
        // Cancels the add-form, back to the material list - not the whole
        // picker (same layered-Esc convention as the picker's own filter,
        // and as every other sub-form in this crate).
        KeyCode::Esc => {
            picker.add_form = None;
            (true, Vec::new())
        }
        KeyCode::Enter => {
            let Some(field) = picker.add_form.as_ref().map(|f| ADD_MATERIAL_FIELDS[f.selected]) else {
                return (false, Vec::new());
            };
            if field == AddMaterialField::Save {
                let validation = picker.add_form.as_ref().expect("checked above").validate();
                match validation {
                    Ok((name, e_ksi, sy_ksi, ftu_ksi, nu, alpha_u_f)) => {
                        model.add_custom_material(name, e_ksi, sy_ksi, ftu_ksi, nu, alpha_u_f);
                        picker.add_form = None;
                        (true, vec![Effect::PersistPressureVesselMaterials])
                    }
                    Err(e) => {
                        if let Some(form) = picker.add_form.as_mut() {
                            form.error = Some(e);
                        }
                        (true, Vec::new())
                    }
                }
            } else {
                if let Some(form) = picker.add_form.as_mut() {
                    form.editing = true;
                    form.edit_buffer = form.field_text(field).to_string();
                }
                (true, Vec::new())
            }
        }
        _ => (false, Vec::new()),
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage((100 - percent_y) / 2), Constraint::Percentage(percent_y), Constraint::Percentage((100 - percent_y) / 2)])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage((100 - percent_x) / 2), Constraint::Percentage(percent_x), Constraint::Percentage((100 - percent_x) / 2)])
        .split(vertical[1])[1]
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, picker: &MaterialPickerState, model: &PressureVesselModel, regions: &mut crate::mouse::MouseRegions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);

    if let Some(form) = &picker.add_form {
        render_add_form(frame, popup, theme, form);
        return;
    }

    let catalog = model.material_catalog();
    let visible = picker.visible_indices(model);
    let title = if picker.filter_text.is_empty() {
        format!(" Material ({} available) - n: add new ", catalog.len())
    } else {
        format!(" Material ({} shown of {}) - n: add new ", visible.len(), catalog.len())
    };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(title);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (list_area, filter_area) = if (picker.filtering || !picker.filter_text.is_empty()) && inner.height > 1 {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
        (rows[0], Some(rows[1]))
    } else {
        (inner, None)
    };

    if visible.is_empty() {
        empty_state::render(frame, list_area, theme, "No materials match the filter", Some("Esc to clear the filter"));
    } else {
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(display_i, &idx)| {
                let material = catalog[idx];
                let is_custom = idx >= mechanics_core::materials::MATERIALS.len();
                let selected_row = display_i == picker.cursor;
                let marker = if selected_row { "> " } else { "  " };
                let tag = if is_custom { " [custom]" } else { "" };
                let style = if selected_row { theme.selected_row_style() } else { Style::default() };
                ListItem::new(Line::from(Span::styled(format!("{marker}{}{tag}", material.name), style)))
            })
            .collect();
        let offset = crate::widgets::scroll_list::render(frame, list_area, items, Some(picker.cursor));
        regions.material_rows.extend(crate::mouse::list_row_regions(list_area, offset, visible.len()));
    }

    if let Some(area) = filter_area {
        let label = if picker.filtering { "Filter (Enter/Esc to stop): " } else { "Filter: " };
        let cursor_glyph = if picker.filtering { "_" } else { "" };
        let line = crate::widgets::input_line::line(theme, label, picker.filter_text.as_str(), cursor_glyph, area.width);
        frame.render_widget(Paragraph::new(line), area);
    }
}

fn render_add_form(frame: &mut Frame, popup: Rect, theme: &Theme, form: &AddMaterialForm) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(" Add Material - blank template ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let label_width = ADD_MATERIAL_FIELDS.iter().map(|f| f.label().len()).max().unwrap_or(0);
    let mut lines: Vec<Line> = ADD_MATERIAL_FIELDS
        .iter()
        .enumerate()
        .map(|(i, &field)| {
            let selected = i == form.selected;
            let value = if selected && form.editing { format!("{}_", form.edit_buffer) } else { form.field_text(field).to_string() };
            let marker = if selected { "> " } else { "  " };
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            let label = field.label();
            Line::from(Span::styled(format!("{marker}{label:<label_width$}  {value}"), style))
        })
        .collect();

    lines.push(Line::from(""));
    if let Some(error) = &form.error {
        lines.push(Line::from(Span::styled(error.as_str(), theme.status_style(StatusTone::Danger))));
    } else {
        lines.push(Line::from(Span::styled("Up/Down select \u{b7} Enter edit/save \u{b7} Esc cancel field or close form", theme.disabled_style())));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn open_now_starts_open_with_no_filter() {
        let picker = MaterialPickerState::open_now();
        assert!(picker.open);
        assert!(picker.filter_text.is_empty());
    }

    #[test]
    fn enter_selects_the_cursor_and_closes() {
        let mut picker = MaterialPickerState::open_now();
        let mut model = PressureVesselModel::default();
        picker.cursor = 2; // steel is index 2 in MATERIALS
        handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(!picker.open);
        assert_eq!(model.material().id, "steel");
    }

    #[test]
    fn esc_closes_when_no_filter_is_active() {
        let mut picker = MaterialPickerState::open_now();
        let mut model = PressureVesselModel::default();
        handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(!picker.open);
    }

    #[test]
    fn slash_then_typing_narrows_the_visible_list() {
        let mut picker = MaterialPickerState::open_now();
        let mut model = PressureVesselModel::default();
        handle_key(&mut picker, &mut model, key(KeyCode::Char('/')));
        for c in "steel".chars() {
            handle_key(&mut picker, &mut model, key(KeyCode::Char(c)));
        }
        let visible = picker.visible_indices(&model);
        assert!(visible.iter().all(|&i| model.material_catalog()[i].name.to_lowercase().contains("steel")));
        assert!(!visible.is_empty());
    }

    #[test]
    fn n_opens_the_add_material_form() {
        let mut picker = MaterialPickerState::open_now();
        let mut model = PressureVesselModel::default();
        handle_key(&mut picker, &mut model, key(KeyCode::Char('n')));
        assert!(picker.add_form.is_some());
    }

    #[test]
    fn uppercase_n_from_caps_lock_still_opens_the_add_material_form() {
        // Regression: crossterm's Windows backend reports Caps-Lock-typed
        // letters as uppercase with no Shift held - a bare-lowercase
        // pattern silently drops the binding on Windows only.
        let mut picker = MaterialPickerState::open_now();
        let mut model = PressureVesselModel::default();
        handle_key(&mut picker, &mut model, key(KeyCode::Char('N')));
        assert!(picker.add_form.is_some());
    }

    #[test]
    fn esc_on_the_add_form_cancels_it_without_closing_the_picker() {
        let mut picker = MaterialPickerState::open_now();
        let mut model = PressureVesselModel::default();
        handle_key(&mut picker, &mut model, key(KeyCode::Char('n')));
        assert!(picker.add_form.is_some());
        handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(picker.add_form.is_none());
        assert!(picker.open, "canceling the add-form must not close the whole picker");
    }

    #[test]
    fn add_form_enter_on_a_text_field_starts_editing_prefilled() {
        let mut picker = MaterialPickerState { add_form: Some(AddMaterialForm { name: "Steel-X".to_string(), ..Default::default() }), ..MaterialPickerState::open_now() };
        let mut model = PressureVesselModel::default();
        handle_add_form_key(&mut picker, &mut model, key(KeyCode::Enter));
        let form = picker.add_form.unwrap();
        assert!(form.editing);
        assert_eq!(form.edit_buffer, "Steel-X");
    }

    fn save_selected_form() -> AddMaterialForm {
        let mut form = AddMaterialForm::default();
        form.selected = ADD_MATERIAL_FIELDS.iter().position(|f| *f == AddMaterialField::Save).unwrap();
        form
    }

    #[test]
    fn add_form_save_with_blank_name_fails_validation() {
        let mut form = save_selected_form();
        form.e_ksi = "10000".to_string();
        form.sy_ksi = "50".to_string();
        form.ftu_ksi = "60".to_string();
        form.nu = "0.3".to_string();
        form.alpha_u_f = "6.5".to_string();
        let mut picker = MaterialPickerState { add_form: Some(form), ..MaterialPickerState::open_now() };
        let mut model = PressureVesselModel::default();
        let (consumed, effects) = handle_add_form_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(consumed);
        assert!(effects.is_empty());
        assert!(picker.add_form.unwrap().error.is_some());
    }

    #[test]
    fn add_form_save_with_valid_fields_adds_the_material_and_persists() {
        let mut form = save_selected_form();
        form.name = "Test Alloy".to_string();
        form.e_ksi = "12000".to_string();
        form.sy_ksi = "55".to_string();
        form.ftu_ksi = "65".to_string();
        form.nu = "0.31".to_string();
        form.alpha_u_f = "7.0".to_string();
        let mut picker = MaterialPickerState { add_form: Some(form), ..MaterialPickerState::open_now() };
        let mut model = PressureVesselModel::default();
        let (consumed, effects) = handle_add_form_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(consumed);
        assert_eq!(effects.len(), 1);
        assert!(picker.add_form.is_none(), "a successful save must close the add-form");
        assert_eq!(model.material().name, "Test Alloy");
    }

    #[test]
    fn add_form_rejects_an_out_of_range_poissons_ratio() {
        let mut form = save_selected_form();
        form.name = "Bad".to_string();
        form.e_ksi = "10000".to_string();
        form.sy_ksi = "50".to_string();
        form.ftu_ksi = "60".to_string();
        form.nu = "0.9".to_string();
        form.alpha_u_f = "6.5".to_string();
        let mut picker = MaterialPickerState { add_form: Some(form), ..MaterialPickerState::open_now() };
        let mut model = PressureVesselModel::default();
        handle_add_form_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(picker.add_form.unwrap().error.is_some());
    }

    fn render_at(picker: &MaterialPickerState, model: &PressureVesselModel, width: u16, height: u16) {
        let backend = TestBackend::new(width.max(1), height.max(1));
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, Rect { x: 0, y: 0, width, height }, &Theme::default_palette(), picker, model, &mut regions)).unwrap();
    }

    #[test]
    fn render_does_not_panic_at_normal_or_degenerate_sizes() {
        let picker = MaterialPickerState::open_now();
        let model = PressureVesselModel::default();
        render_at(&picker, &model, 80, 24);
        render_at(&picker, &model, 0, 0);
        render_at(&picker, &model, 1, 1);
    }

    #[test]
    fn render_of_add_form_does_not_panic() {
        let mut picker = MaterialPickerState::open_now();
        picker.add_form = Some(AddMaterialForm::default());
        let model = PressureVesselModel::default();
        render_at(&picker, &model, 80, 24);
        render_at(&picker, &model, 0, 0);
    }
}
