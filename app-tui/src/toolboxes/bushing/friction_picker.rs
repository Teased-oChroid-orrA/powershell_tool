//! Friction-coefficient picker - `Enter` on the Friction row opens a small
//! list of typical installation friction coefficients
//! (`bushing_solver::friction::FRICTION_TYPICALS`), each with a tooltip-
//! style note on appropriate/inappropriate use shown for the highlighted
//! entry. Selecting one copies the value into `model.friction` as an
//! ordinary editable number - never locked, same as every other `Number`
//! row in this crate. `m` (mirroring `reamer_picker.rs`'s own escape hatch)
//! drops straight to plain numeric entry when none of the typical values
//! apply.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use bushing_solver::friction::FRICTION_TYPICALS;

use crate::app::Effect;
use crate::theme::Theme;

use super::model::BushingModel;

#[derive(Debug, Clone, Copy, Default)]
pub struct FrictionPickerState {
    pub open: bool,
    pub cursor: usize,
}

impl FrictionPickerState {
    pub fn open_now() -> Self {
        Self { open: true, cursor: 0 }
    }

    fn move_cursor(&mut self, delta: i32) {
        let len = FRICTION_TYPICALS.len() as i32;
        self.cursor = (self.cursor as i32 + delta).rem_euclid(len) as usize;
    }
}

pub fn handle_key(picker: &mut FrictionPickerState, model: &mut BushingModel, key: KeyEvent) -> (bool, Vec<Effect>) {
    match key.code {
        KeyCode::Up => {
            picker.move_cursor(-1);
            (true, Vec::new())
        }
        KeyCode::Down => {
            picker.move_cursor(1);
            (true, Vec::new())
        }
        KeyCode::Enter => {
            if let Some(entry) = FRICTION_TYPICALS.get(picker.cursor) {
                model.friction = entry.value;
                model.recompute();
            }
            picker.open = false;
            (true, Vec::new())
        }
        KeyCode::Esc => {
            picker.open = false;
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

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, picker: &FrictionPickerState, regions: &mut crate::mouse::MouseRegions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = centered_rect(64, 60, area);
    frame.render_widget(Clear, popup);

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(" Friction Coefficient - m: type exact value ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let note_height = 4u16.min(inner.height.saturating_sub(FRICTION_TYPICALS.len() as u16).max(1));
    let (list_area, note_area) = if inner.height > note_height {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(note_height)]).split(inner);
        (rows[0], Some(rows[1]))
    } else {
        (inner, None)
    };

    let items: Vec<ListItem> = FRICTION_TYPICALS
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let selected = i == picker.cursor;
            let marker = if selected { "> " } else { "  " };
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            ListItem::new(Line::from(Span::styled(format!("{marker}{:<38} {:.3}", entry.label, entry.value), style)))
        })
        .collect();
    let offset = crate::widgets::scroll_list::render(frame, list_area, items, Some(picker.cursor));
    regions.friction_rows.extend(crate::mouse::list_row_regions(list_area, offset, FRICTION_TYPICALS.len()));

    if let Some(area) = note_area {
        if let Some(entry) = FRICTION_TYPICALS.get(picker.cursor) {
            frame.render_widget(Paragraph::new(Line::from(entry.note)).wrap(Wrap { trim: true }).style(theme.disabled_style()), area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn up_down_navigation_wraps() {
        let mut picker = FrictionPickerState::open_now();
        let mut model = BushingModel::default();
        let len = FRICTION_TYPICALS.len();
        handle_key(&mut picker, &mut model, key(KeyCode::Up));
        assert_eq!(picker.cursor, len - 1);
        handle_key(&mut picker, &mut model, key(KeyCode::Down));
        assert_eq!(picker.cursor, 0);
    }

    #[test]
    fn enter_sets_friction_to_the_selected_typical_value_and_closes() {
        let mut picker = FrictionPickerState::open_now();
        let mut model = BushingModel::default();
        picker.cursor = 2;
        let expected = FRICTION_TYPICALS[2].value;
        handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(!picker.open);
        assert_eq!(model.friction, expected);
    }

    #[test]
    fn esc_closes_without_changing_friction() {
        let mut picker = FrictionPickerState::open_now();
        let mut model = BushingModel::default();
        let before = model.friction;
        handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(!picker.open);
        assert_eq!(model.friction, before);
    }

    #[test]
    fn friction_stays_editable_after_selection() {
        // "still updatable after selection like all other inputs" - once
        // picked, the value is just an ordinary f64 field; committing a new
        // number through the normal edit path must work exactly as before.
        let mut model = BushingModel::default();
        model.friction = FRICTION_TYPICALS[0].value;
        model.commit_number(super::super::model::NumberTarget::Friction, 0.42);
        assert_eq!(model.friction, 0.42);
    }

    #[test]
    fn render_does_not_panic_at_normal_or_degenerate_sizes() {
        let picker = FrictionPickerState::open_now();
        for (w, h) in [(100, 30), (0, 0), (1, 1)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| render(f, Rect { x: 0, y: 0, width: w, height: h }, &Theme::default_palette(), &picker, &mut regions)).unwrap();
        }
    }
}
