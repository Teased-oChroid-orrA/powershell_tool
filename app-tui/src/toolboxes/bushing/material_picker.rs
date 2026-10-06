//! Filterable material picker, mirroring
//! `toolboxes/pressure_vessel/material_picker.rs`'s pattern (an "add new
//! material" sub-form plus a `crate::library`-backed custom-material
//! catalog with import/export/duplicate-pruning/conflict-resolution/
//! labeling - see that module's own doc comment for the JSON schema).
//! `bushing_solver::solve::BushingInputs` takes a resolved
//! `mechanics_core::materials::Material` value directly (not a `&str` id
//! looked up internally), so a custom material flows into the solver
//! exactly like a built-in one. One picker instance is reused for both the
//! housing and the bushing material rows - `target` says which
//! `BushingModel` field Enter commits into.
//!
//! Extended beyond the Pressure Vessel version with the two fields that
//! toolbox's own add-material form zeroes because it never reads them
//! (`fbru_ksi`/`fsu_ksi`, bearing/shear ultimate) - this toolbox's
//! `compute()` does read both.

use crate::widgets::popup::centered_rect;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::library::{ConflictQueue, LibraryItem};
use crate::theme::{StatusTone, Theme};
use crate::widgets::empty_state;

use super::material_persistence::{self, PersistedMaterial};
use super::model::BushingModel;
use crate::app::Effect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialTarget {
    Housing,
    Bushing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddMaterialField {
    Name,
    E,
    Sy,
    Fbru,
    FbruE15,
    Fsu,
    Ftu,
    Nu,
    Alpha,
    Save,
}

pub const ADD_MATERIAL_FIELDS: [AddMaterialField; 10] = [
    AddMaterialField::Name,
    AddMaterialField::E,
    AddMaterialField::Sy,
    AddMaterialField::Fbru,
    AddMaterialField::FbruE15,
    AddMaterialField::Fsu,
    AddMaterialField::Ftu,
    AddMaterialField::Nu,
    AddMaterialField::Alpha,
    AddMaterialField::Save,
];

impl AddMaterialField {
    pub fn label(self) -> &'static str {
        match self {
            AddMaterialField::Name => "Name",
            AddMaterialField::E => "E (ksi)",
            AddMaterialField::Sy => "Yield Strength Sy (ksi)",
            AddMaterialField::Fbru => "Bearing Ult. Fbru e/D=2.0 (ksi)",
            AddMaterialField::FbruE15 => "Bearing Ult. Fbru e/D=1.5 (ksi)",
            AddMaterialField::Fsu => "Shear Ultimate Fsu (ksi)",
            AddMaterialField::Ftu => "Ultimate Strength Ftu (ksi)",
            AddMaterialField::Nu => "Poisson's Ratio",
            AddMaterialField::Alpha => "Thermal Expansion (\u{b5}strain/\u{b0}F)",
            AddMaterialField::Save => "[Save]",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AddMaterialForm {
    pub selected: usize,
    pub editing: bool,
    pub edit_buffer: crate::widgets::number_edit::EditBuffer,
    pub name: String,
    pub e_ksi: String,
    pub sy_ksi: String,
    pub fbru_ksi: String,
    pub fbru_e15_ksi: String,
    pub fsu_ksi: String,
    pub ftu_ksi: String,
    pub nu: String,
    pub alpha_u_f: String,
    pub error: Option<String>,
}

impl AddMaterialForm {
    fn field_text(&self, field: AddMaterialField) -> &str {
        match field {
            AddMaterialField::Name => &self.name,
            AddMaterialField::E => &self.e_ksi,
            AddMaterialField::Sy => &self.sy_ksi,
            AddMaterialField::Fbru => &self.fbru_ksi,
            AddMaterialField::FbruE15 => &self.fbru_e15_ksi,
            AddMaterialField::Fsu => &self.fsu_ksi,
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
            AddMaterialField::Fbru => self.fbru_ksi = text,
            AddMaterialField::FbruE15 => self.fbru_e15_ksi = text,
            AddMaterialField::Fsu => self.fsu_ksi = text,
            AddMaterialField::Ftu => self.ftu_ksi = text,
            AddMaterialField::Nu => self.nu = text,
            AddMaterialField::Alpha => self.alpha_u_f = text,
            AddMaterialField::Save => {}
        }
    }

    /// Same validation discipline as
    /// `pressure_vessel::material_picker::AddMaterialForm::validate` -
    /// permissive on exact bounds (user-entered, not a curated catalog
    /// entry) but strict on unusable text and on ranges that would make
    /// downstream formulas divide by a near-zero/negative quantity.
    /// `Fbru`/`Fsu` are allowed to be blank/zero (this toolbox's own
    /// `compute()` falls back to `Sy` when either is zero, same as the
    /// built-in materials that don't set them).
    fn validate(&self) -> Result<PersistedMaterial, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Name must not be blank".to_string());
        }
        let e_ksi = parse_positive(&self.e_ksi, "E")?;
        let sy_ksi = parse_positive(&self.sy_ksi, "Yield strength")?;
        let fbru_ksi = parse_non_negative_or_blank(&self.fbru_ksi, "Bearing ultimate")?;
        let fbru_e15_ksi = parse_non_negative_or_blank(&self.fbru_e15_ksi, "Bearing ultimate at e/D 1.5")?;
        if fbru_e15_ksi > 0.0 && fbru_ksi > 0.0 && fbru_e15_ksi > fbru_ksi {
            return Err("Fbru at e/D 1.5 cannot exceed Fbru at e/D 2.0".to_string());
        }
        let fsu_ksi = parse_non_negative_or_blank(&self.fsu_ksi, "Shear ultimate")?;
        let ftu_ksi = parse_positive(&self.ftu_ksi, "Ultimate strength")?;
        let nu: f64 = self.nu.trim().parse().map_err(|_| "Poisson's ratio must be a number".to_string())?;
        if !(0.0..0.5).contains(&nu) {
            return Err("Poisson's ratio must be between 0 and 0.5".to_string());
        }
        let alpha_u_f: f64 = self.alpha_u_f.trim().parse().map_err(|_| "Thermal expansion must be a number".to_string())?;
        if !alpha_u_f.is_finite() || alpha_u_f < 0.0 {
            return Err("Thermal expansion must be \u{2265} 0".to_string());
        }
        Ok(PersistedMaterial { name: name.to_string(), e_ksi, sy_ksi, fbru_ksi, fbru_e15_ksi, fsu_ksi, ftu_ksi, nu, alpha_u_f })
    }
}

fn parse_positive(text: &str, label: &str) -> Result<f64, String> {
    let value: f64 = text.trim().parse().map_err(|_| format!("{label} must be a number"))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("{label} must be > 0"));
    }
    Ok(value)
}

fn parse_non_negative_or_blank(text: &str, label: &str) -> Result<f64, String> {
    if text.trim().is_empty() {
        return Ok(0.0);
    }
    let value: f64 = text.trim().parse().map_err(|_| format!("{label} must be a number (or blank)"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{label} must be \u{2265} 0"));
    }
    Ok(value)
}

enum PathPromptKind {
    Import,
    Export,
}

struct PathPrompt {
    kind: PathPromptKind,
    buffer: String,
}

pub struct MaterialPickerState {
    pub open: bool,
    pub target: Option<MaterialTarget>,
    pub cursor: usize,
    pub filter_text: String,
    pub add_form: Option<AddMaterialForm>,
    /// User-added/imported materials, merged into `BushingModel`'s own
    /// combined catalog once added via the form or an import - this
    /// picker's own view of the same library, kept for import/export/
    /// conflict handling independent of which of the two target fields is
    /// currently open.
    pub library: Vec<LibraryItem<PersistedMaterial>>,
    pub pending_conflicts: Option<ConflictQueue<PersistedMaterial>>,
    path_prompt: Option<PathPrompt>,
}

impl Default for MaterialPickerState {
    fn default() -> Self {
        Self { open: false, target: None, cursor: 0, filter_text: String::new(), add_form: None, library: Vec::new(), pending_conflicts: None, path_prompt: None }
    }
}

impl MaterialPickerState {
    pub fn open_for(target: MaterialTarget) -> Self {
        Self { open: true, target: Some(target), library: material_persistence::load(), ..Default::default() }
    }

    fn visible_indices(&self, model: &BushingModel) -> Vec<usize> {
        let needle = self.filter_text.trim().to_lowercase();
        let catalog = model.material_catalog();
        if needle.is_empty() {
            (0..catalog.len()).collect()
        } else {
            catalog.iter().enumerate().filter(|(_, m)| crate::widgets::material_detail::matches(m, &needle)).map(|(i, _)| i).collect()
        }
    }

    /// Moves by `delta` rows, clamped (no wrap): paging a long list.
    fn jump_cursor(&mut self, model: &BushingModel, delta: i32) {
        let len = self.visible_indices(model).len() as i32;
        self.cursor = if len == 0 { 0 } else { (self.cursor as i32 + delta).clamp(0, len - 1) as usize };
    }

    fn move_cursor(&mut self, model: &BushingModel, delta: i32) {
        let len = self.visible_indices(model).len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as i32 + delta).rem_euclid(len as i32) as usize;
    }
}

pub fn handle_key(picker: &mut MaterialPickerState, model: &mut BushingModel, key: KeyEvent) -> (bool, Vec<Effect>) {
    if let Some(queue) = &mut picker.pending_conflicts {
        let (consumed, _) = super::conflict_prompt::handle_key(queue, &mut picker.library, key);
        if !consumed {
            return (false, Vec::new());
        }
        model.sync_custom_materials_from_library(&picker.library);
        return (true, vec![Effect::PersistBushingMaterialLibrary]);
    }

    if let Some(prompt) = &mut picker.path_prompt {
        return match key.code {
            KeyCode::Enter => {
                let path = prompt.buffer.trim().to_string();
                let kind_is_export = matches!(prompt.kind, PathPromptKind::Export);
                picker.path_prompt = None;
                if path.is_empty() {
                    return (true, Vec::new());
                }
                if kind_is_export {
                    let mut items: Vec<LibraryItem<PersistedMaterial>> = mechanics_core::materials::MATERIALS
                        .iter()
                        .map(|m| LibraryItem::new(PersistedMaterial { name: m.name.to_string(), e_ksi: m.e_ksi, sy_ksi: m.sy_ksi, fbru_ksi: m.fbru_ksi, fbru_e15_ksi: m.fbru_e15_ksi, fsu_ksi: m.fsu_ksi, ftu_ksi: m.ftu_ksi, nu: m.nu, alpha_u_f: m.alpha_u_f }))
                        .collect();
                    items.extend(picker.library.iter().cloned());
                    let contents = crate::library::export_json(&items);
                    (true, vec![Effect::ExportBushingMaterialLibraryFile { path, contents }])
                } else {
                    (true, vec![Effect::ImportBushingMaterialLibraryFile(path)])
                }
            }
            KeyCode::Esc => {
                picker.path_prompt = None;
                (true, Vec::new())
            }
            KeyCode::Delete => {
                prompt.buffer.clear();
                (true, Vec::new())
            }
            KeyCode::Backspace => {
                prompt.buffer.pop();
                (true, Vec::new())
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                prompt.buffer.push(c);
                (true, Vec::new())
            }
            _ => (false, Vec::new()),
        };
    }

    if picker.add_form.is_some() {
        return handle_add_form_key(picker, model, key);
    }

    // Always-on search: printable keys type into the search box (so the
    // action keys are Ctrl chords); the list narrows as you type.
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('n') if ctrl => {
            picker.add_form = Some(AddMaterialForm::default());
            (true, Vec::new())
        }
        KeyCode::Char('l') if ctrl => {
            picker.path_prompt = Some(PathPrompt { kind: PathPromptKind::Import, buffer: default_material_library_path() });
            (true, Vec::new())
        }
        KeyCode::Char('e') if ctrl => {
            picker.path_prompt = Some(PathPrompt { kind: PathPromptKind::Export, buffer: default_material_library_path() });
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
        KeyCode::PageUp => {
            picker.jump_cursor(model, -10);
            (true, Vec::new())
        }
        KeyCode::PageDown => {
            picker.jump_cursor(model, 10);
            (true, Vec::new())
        }
        KeyCode::Home => {
            picker.cursor = 0;
            (true, Vec::new())
        }
        KeyCode::End => {
            picker.cursor = picker.visible_indices(model).len().saturating_sub(1);
            (true, Vec::new())
        }
        KeyCode::Enter => {
            if let Some(&idx) = picker.visible_indices(model).get(picker.cursor) {
                match picker.target {
                    Some(MaterialTarget::Housing) => model.select_housing_material(idx),
                    Some(MaterialTarget::Bushing) => model.select_bushing_material(idx),
                    None => {}
                }
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
        _ => {
            if crate::widgets::material_detail::search_edit(&mut picker.filter_text, &key) {
                picker.cursor = 0;
                (true, Vec::new())
            } else {
                (false, Vec::new())
            }
        }
    }
}

fn default_material_library_path() -> String {
    crate::paths::app_data_dir().unwrap_or_default().join("bushing-material-library.json").to_string_lossy().into_owned()
}

fn move_add_form_selection(form: &mut AddMaterialForm, delta: i32) {
    let len = ADD_MATERIAL_FIELDS.len() as i32;
    form.selected = ((form.selected as i32 + delta).rem_euclid(len)) as usize;
}

fn handle_add_form_key(picker: &mut MaterialPickerState, model: &mut BushingModel, key: KeyEvent) -> (bool, Vec<Effect>) {
    let editing = picker.add_form.as_ref().is_some_and(|f| f.editing);
    if editing {
        let form = picker.add_form.as_mut().expect("editing is only true when add_form is Some");
        return match key.code {
            KeyCode::Enter => {
                let field = ADD_MATERIAL_FIELDS[form.selected];
                let text = form.edit_buffer.take();
                form.set_field_text(field, text);
                form.editing = false;
                (true, Vec::new())
            }
            KeyCode::Esc => {
                form.editing = false;
                form.edit_buffer.clear();
                (true, Vec::new())
            }
            _ if form.edit_buffer.handle_key(&key, |_| true) => (true, Vec::new()),
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
                    Ok(persisted) => {
                        let Some(target) = picker.target else { return (true, Vec::new()) };
                        model.add_custom_material(target, persisted.clone());
                        picker.library.push(LibraryItem::new(persisted));
                        picker.add_form = None;
                        (true, vec![Effect::PersistBushingMaterialLibrary])
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
                    form.edit_buffer.set(form.field_text(field).to_string());
                }
                (true, Vec::new())
            }
        }
        _ => (false, Vec::new()),
    }
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, picker: &MaterialPickerState, model: &BushingModel, regions: &mut crate::mouse::MouseRegions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = centered_rect(64, 64, area);
    frame.render_widget(Clear, popup);

    if let Some(queue) = &picker.pending_conflicts {
        super::conflict_prompt::render(frame, popup, theme, "Material import conflict", queue, |i| (format!("Material \"{}\" already exists with different data.", i.name), format!("Imported: E={:.0} ksi  Sy={:.0} ksi  Ftu={:.0} ksi", i.e_ksi, i.sy_ksi, i.ftu_ksi)));
        return;
    }

    if let Some(form) = &picker.add_form {
        render_add_form(frame, popup, theme, form);
        return;
    }

    let catalog = model.material_catalog();
    let visible = picker.visible_indices(model);
    let which = match picker.target {
        Some(MaterialTarget::Housing) => "Housing",
        Some(MaterialTarget::Bushing) => "Bushing",
        None => "",
    };
    let title = format!(" {which} Material ({} of {}) \u{b7} Enter select \u{b7} PgUp/PgDn \u{b7} Esc clear/close ", visible.len(), catalog.len());
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(title);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (list_area, bottom_area, detail_area) = detail_split(inner, visible.get(picker.cursor).map(|&i| catalog[i]));
    if let (Some(area), Some(&i)) = (detail_area, visible.get(picker.cursor)) {
        frame.render_widget(Paragraph::new(crate::widgets::material_detail::lines(theme, catalog[i])).wrap(Wrap { trim: true }), area);
    }

    if visible.is_empty() {
        empty_state::render(frame, list_area, theme, "No materials match the filter", Some("Esc to clear the filter"));
    } else {
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(display_i, &idx)| {
                let material = catalog[idx];
                let is_custom = idx >= mechanics_core::materials::builtin_len();
                let labels = if is_custom { picker.library.iter().find(|li| li.item.name == material.name).map(|li| li.labels.clone()).unwrap_or_default() } else { Vec::new() };
                let tag = if is_custom { " [custom]".to_string() } else { String::new() };
                let labels_tag = if labels.is_empty() { String::new() } else { format!(" {{{}}}", labels.join(", ")) };
                let selected_row = display_i == picker.cursor;
                let marker = if selected_row { "> " } else { "  " };
                let style = if selected_row { theme.selected_row_style() } else { Style::default() };
                ListItem::new(Line::from(Span::styled(format!("{marker}{}{tag}{labels_tag}", material.name), style)))
            })
            .collect();
        let offset = crate::widgets::scroll_list::render(frame, list_area, items, Some(picker.cursor));
        regions.material_rows.extend(crate::mouse::list_row_regions(list_area, offset, visible.len()));
    }

    if let Some(area) = bottom_area {
        if let Some(prompt) = &picker.path_prompt {
            let label = match prompt.kind {
                PathPromptKind::Import => "Import from (Enter to confirm, Esc to cancel): ",
                PathPromptKind::Export => "Export to (Enter to confirm, Esc to cancel): ",
            };
            let line = crate::widgets::input_line::line(theme, label, prompt.buffer.as_str(), "_", area.width);
            frame.render_widget(Paragraph::new(line), area);
        } else {
            frame.render_widget(Paragraph::new(crate::widgets::material_detail::search_line(theme, &picker.filter_text, "Ctrl+N add \u{b7} Ctrl+L import \u{b7} Ctrl+E export", area.width)), area);
        }
    }
}

/// Splits the popup into list / search row / property panel. The panel needs
/// room, so it is dropped on short terminals.
fn detail_split(inner: Rect, selected: Option<&mechanics_core::materials::Material>) -> (Rect, Option<Rect>, Option<Rect>) {
    let panel = if selected.is_some() && inner.height >= 16 { if selected.is_some_and(|m| m.extra.is_some()) { 8 } else { 2 } } else { 0 };
    let mut constraints = vec![Constraint::Min(1)];
    if panel > 0 {
        constraints.push(Constraint::Length(panel));
    }
    if inner.height > 1 {
        constraints.push(Constraint::Length(1));
    }
    let rows = Layout::default().direction(Direction::Vertical).constraints(constraints).split(inner);
    match (panel > 0, inner.height > 1) {
        (true, true) => (rows[0], Some(rows[2]), Some(rows[1])),
        (true, false) => (rows[0], None, Some(rows[1])),
        (false, true) => (rows[0], Some(rows[1]), None),
        (false, false) => (rows[0], None, None),
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
            let value = if selected && form.editing { form.edit_buffer.with_cursor() } else { form.field_text(field).to_string() };
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
        lines.push(Line::from(Span::styled("Up/Down select \u{b7} Enter edit/save \u{b7} Esc cancel field or close form \u{b7} Delete clears while editing", theme.disabled_style())));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent { code: KeyCode::Char(c), modifiers: KeyModifiers::CONTROL, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn open_for_starts_open_targeting_the_requested_field() {
        let picker = MaterialPickerState::open_for(MaterialTarget::Bushing);
        assert!(picker.open);
        assert_eq!(picker.target, Some(MaterialTarget::Bushing));
    }

    #[test]
    fn enter_selects_the_cursor_into_the_targeted_field_and_closes() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        let mut model = BushingModel::default();
        picker.cursor = 2; // "steel" is index 2 in MATERIALS
        handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(!picker.open);
        assert_eq!(model.housing_material().id, "steel");
        assert_ne!(model.bushing_material().id, "steel", "must not have touched the other field");
    }

    #[test]
    fn bushing_target_updates_only_the_bushing_material() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Bushing);
        let mut model = BushingModel::default();
        picker.cursor = 6; // "ti6al4v"
        handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert_eq!(model.bushing_material().id, "ti6al4v");
        assert_eq!(model.housing_material().id, "al7075", "housing must be untouched");
    }

    #[test]
    fn typing_narrows_the_visible_list_without_a_slash() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        let mut model = BushingModel::default();
        for c in "steel".chars() {
            handle_key(&mut picker, &mut model, key(KeyCode::Char(c)));
        }
        let visible = picker.visible_indices(&model);
        assert!(!visible.is_empty());
        // Matches the family too ("Low-Alloy Steels"), not just the name.
        assert!(visible.iter().all(|&i| crate::widgets::material_detail::matches(model.material_catalog()[i], "steel")));
        assert!(visible.len() > model.material_catalog().iter().filter(|m| m.name.to_lowercase().contains("steel")).count() / 2);
    }

    #[test]
    fn esc_closes_when_no_filter_is_active() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        let mut model = BushingModel::default();
        handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(!picker.open);
    }

    #[test]
    fn ctrl_n_opens_the_add_material_form() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        let mut model = BushingModel::default();
        handle_key(&mut picker, &mut model, ctrl_key('n'));
        assert!(picker.add_form.is_some());
    }

    fn save_selected_form() -> AddMaterialForm {
        let mut form = AddMaterialForm::default();
        form.selected = ADD_MATERIAL_FIELDS.iter().position(|f| *f == AddMaterialField::Save).unwrap();
        form
    }

    #[test]
    fn add_form_save_with_valid_fields_adds_the_material_persists_and_selects_it() {
        let mut form = save_selected_form();
        form.name = "Test Alloy".to_string();
        form.e_ksi = "12000".to_string();
        form.sy_ksi = "55".to_string();
        form.fbru_ksi = "120".to_string();
        form.fsu_ksi = "45".to_string();
        form.ftu_ksi = "65".to_string();
        form.nu = "0.31".to_string();
        form.alpha_u_f = "7.0".to_string();
        let mut picker = MaterialPickerState { add_form: Some(form), ..MaterialPickerState::open_for(MaterialTarget::Housing) };
        let mut model = BushingModel::default();
        let (consumed, effects) = handle_add_form_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(consumed);
        assert!(matches!(effects.as_slice(), [Effect::PersistBushingMaterialLibrary]));
        assert!(picker.add_form.is_none());
        assert_eq!(model.housing_material().name, "Test Alloy");
        assert_eq!(model.housing_material().fbru_ksi, 120.0);
        assert_eq!(picker.library.len(), 1);
    }

    #[test]
    fn add_form_stores_fbru_at_e_over_d_1p5_and_rejects_one_above_the_2p0_value() {
        let mut form = AddMaterialForm { name: "M".into(), e_ksi: "10000".into(), sy_ksi: "50".into(), ftu_ksi: "60".into(), nu: "0.3".into(), alpha_u_f: "6".into(), fbru_ksi: "120".into(), fbru_e15_ksi: "98".into(), ..AddMaterialForm::default() };
        assert_eq!(form.validate().unwrap().fbru_e15_ksi, 98.0);
        form.fbru_e15_ksi = "130".into();
        assert!(form.validate().unwrap_err().contains("cannot exceed"));
        form.fbru_e15_ksi = String::new();
        assert_eq!(form.validate().unwrap().fbru_e15_ksi, 0.0, "blank = not tabulated");
    }

    #[test]
    fn add_form_rejects_a_blank_name() {
        let mut form = save_selected_form();
        form.e_ksi = "10000".to_string();
        form.sy_ksi = "50".to_string();
        form.ftu_ksi = "60".to_string();
        form.nu = "0.3".to_string();
        form.alpha_u_f = "6.5".to_string();
        let mut picker = MaterialPickerState { add_form: Some(form), ..MaterialPickerState::open_for(MaterialTarget::Housing) };
        let mut model = BushingModel::default();
        let (consumed, effects) = handle_add_form_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(consumed);
        assert!(effects.is_empty());
        assert!(picker.add_form.unwrap().error.is_some());
    }

    #[test]
    fn add_form_allows_blank_fbru_and_fsu() {
        let mut form = save_selected_form();
        form.name = "Simple Alloy".to_string();
        form.e_ksi = "10000".to_string();
        form.sy_ksi = "50".to_string();
        form.ftu_ksi = "60".to_string();
        form.nu = "0.3".to_string();
        form.alpha_u_f = "6.5".to_string();
        let result = form.validate();
        assert!(result.is_ok(), "blank Fbru/Fsu must be accepted (falls back to Sy downstream): {result:?}");
        let material = result.unwrap();
        assert_eq!(material.fbru_ksi, 0.0);
        assert_eq!(material.fsu_ksi, 0.0);
    }

    #[test]
    fn ctrl_l_opens_an_import_prompt_prefilled_with_the_default_path() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        let mut model = BushingModel::default();
        handle_key(&mut picker, &mut model, ctrl_key('l'));
        assert!(picker.path_prompt.is_some());
    }

    #[test]
    fn conflict_resolution_overwrites_on_o() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        let existing = PersistedMaterial { name: "Custom X".to_string(), e_ksi: 10000.0, sy_ksi: 50.0, fbru_ksi: 0.0, fbru_e15_ksi: 0.0, fsu_ksi: 0.0, ftu_ksi: 60.0, nu: 0.3, alpha_u_f: 6.5 };
        picker.library.push(LibraryItem::new(existing.clone()));
        let incoming = PersistedMaterial { sy_ksi: 999.0, ..existing };
        let outcomes = crate::library::classify_import(&picker.library, vec![LibraryItem::new(incoming)], |m| m.name.clone());
        let (queue, _, _) = ConflictQueue::new(outcomes, &mut picker.library);
        picker.pending_conflicts = Some(queue);
        let mut model = BushingModel::default();
        let (consumed, _) = handle_key(&mut picker, &mut model, key(KeyCode::Char('o')));
        assert!(consumed);
        assert_eq!(picker.library[0].item.sy_ksi, 999.0);
    }

    fn render_at(picker: &MaterialPickerState, model: &BushingModel, width: u16, height: u16) {
        let backend = TestBackend::new(width.max(1), height.max(1));
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, Rect { x: 0, y: 0, width, height }, &Theme::default_palette(), picker, model, &mut regions)).unwrap();
    }

    #[test]
    fn render_does_not_panic_at_normal_or_degenerate_sizes() {
        let picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        let model = BushingModel::default();
        render_at(&picker, &model, 80, 24);
        render_at(&picker, &model, 0, 0);
        render_at(&picker, &model, 1, 1);
    }

    #[test]
    fn render_of_add_form_does_not_panic() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        picker.add_form = Some(AddMaterialForm::default());
        let model = BushingModel::default();
        render_at(&picker, &model, 80, 24);
    }
}
