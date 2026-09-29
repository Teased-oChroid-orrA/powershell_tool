//! Filterable material picker, mirroring
//! `toolboxes/pressure_vessel/material_picker.rs`'s pattern minus the
//! "add new material" sub-form and cross-relaunch persistence -
//! `bushing_solver::solve::compute` looks a material up by a fixed `&str`
//! id from `mechanics_core::materials::MATERIALS` only
//! (`get_material`'s own doc comment: falls back to `al7075` on an unknown
//! id), with no custom-material concept anywhere in its public API, so
//! adding one here would be new functionality this toolbox's underlying
//! solver has no way to honor - not a scope cut, a faithful match to what
//! actually exists.
//!
//! One picker instance is reused for both the housing and the bushing
//! material rows - `target` says which `BushingModel` field Enter commits
//! into.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph};
use ratatui::Frame;

use crate::theme::Theme;
use crate::widgets::empty_state;

use super::model::BushingModel;
use crate::app::Effect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialTarget {
    Housing,
    Bushing,
}

#[derive(Debug, Clone, Default)]
pub struct MaterialPickerState {
    pub open: bool,
    pub target: Option<MaterialTarget>,
    pub cursor: usize,
    pub filter_text: String,
    pub filtering: bool,
}

impl MaterialPickerState {
    pub fn open_for(target: MaterialTarget) -> Self {
        Self { open: true, target: Some(target), ..Default::default() }
    }

    fn visible_indices(&self, model: &BushingModel) -> Vec<usize> {
        let needle = self.filter_text.trim().to_lowercase();
        let catalog = model.material_catalog();
        if needle.is_empty() {
            (0..catalog.len()).collect()
        } else {
            catalog.iter().enumerate().filter(|(_, m)| m.name.to_lowercase().contains(&needle)).map(|(i, _)| i).collect()
        }
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

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, picker: &MaterialPickerState, model: &BushingModel, regions: &mut crate::mouse::MouseRegions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);

    let catalog = model.material_catalog();
    let visible = picker.visible_indices(model);
    let which = match picker.target {
        Some(MaterialTarget::Housing) => "Housing",
        Some(MaterialTarget::Bushing) => "Bushing",
        None => "",
    };
    let title = format!(" {which} Material ({} of {}) ", visible.len(), catalog.len());
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
                let selected_row = display_i == picker.cursor;
                let marker = if selected_row { "> " } else { "  " };
                let style = if selected_row { theme.selected_row_style() } else { Style::default() };
                ListItem::new(Line::from(Span::styled(format!("{marker}{}", material.name), style)))
            })
            .collect();
        let offset = crate::widgets::scroll_list::render(frame, list_area, items, Some(picker.cursor));
        regions.material_rows.extend(crate::mouse::list_row_regions(list_area, offset, visible.len()));
    }

    if let Some(area) = filter_area {
        let label = if picker.filtering { "Filter (Enter/Esc to stop): " } else { "Filter: " };
        let cursor_glyph = if picker.filtering { "_" } else { "" };
        let line = Line::from(vec![Span::styled(label, theme.title_style(true)), Span::raw(picker.filter_text.as_str()), Span::styled(cursor_glyph, theme.title_style(true))]);
        frame.render_widget(Paragraph::new(line), area);
    }
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
    fn slash_then_typing_narrows_the_visible_list() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        let mut model = BushingModel::default();
        handle_key(&mut picker, &mut model, key(KeyCode::Char('/')));
        for c in "steel".chars() {
            handle_key(&mut picker, &mut model, key(KeyCode::Char(c)));
        }
        let visible = picker.visible_indices(&model);
        assert!(!visible.is_empty());
        assert!(visible.iter().all(|&i| model.material_catalog()[i].name.to_lowercase().contains("steel")));
    }

    #[test]
    fn esc_closes_when_no_filter_is_active() {
        let mut picker = MaterialPickerState::open_for(MaterialTarget::Housing);
        let mut model = BushingModel::default();
        handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(!picker.open);
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
}
