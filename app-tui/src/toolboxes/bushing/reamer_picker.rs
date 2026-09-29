//! Bore-diameter reamer picker popup - the primary way to set Bore
//! Diameter (see `mod.rs`'s Enter routing on that row), not a secondary
//! "nearest suggestion" alongside free numeric entry. A real installed
//! bushing bore is reamed to a real tool size, not an arbitrary decimal -
//! this picker's full, filterable catalog (`bushing_solver::reamers::all_reamers`,
//! the same sourced Pan American Tool/Rock River Tool/Omega Technologies
//! aircraft reamer data `bushing-solver/AGENTS.md` documents) is opened
//! pre-positioned at the closest real size to whatever value is already
//! entered, mirroring `material_picker.rs`'s filter/list pattern. `m`
//! (handled one level up, in `mod.rs`, since it needs to hand control back
//! to the toolbox's own text-edit mode) is the escape hatch to a plain
//! numeric value when a non-catalog bore is genuinely needed.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph};
use ratatui::Frame;

use bushing_solver::reamers::{self, AvailabilityTier, ReamerEntry};

use crate::theme::Theme;
use crate::widgets::empty_state;

use super::model::BushingModel;
use crate::app::Effect;

#[derive(Debug, Clone, Default)]
pub struct ReamerPickerState {
    pub open: bool,
    /// Index into the *visible* (filtered) list, not directly into
    /// `all_reamers()` - same convention `MaterialPickerState::cursor` uses.
    pub cursor: usize,
    pub filter_text: String,
    pub filtering: bool,
}

impl ReamerPickerState {
    /// Opens with the cursor pre-positioned at the catalog entry closest to
    /// `model.bore_dia` - the picker starts where the user's current value
    /// already is, not at the top of a ~48-entry list.
    pub fn open_near(model: &BushingModel) -> Self {
        let target = model.bore_dia;
        let all = reamers::all_reamers();
        let cursor = all
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (a.nominal_in - target).abs().partial_cmp(&(b.nominal_in - target).abs()).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        Self { open: true, cursor, filter_text: String::new(), filtering: false }
    }

    fn visible(&self) -> Vec<&'static ReamerEntry> {
        let needle = self.filter_text.trim().to_lowercase();
        let all = reamers::all_reamers();
        if needle.is_empty() {
            all
        } else {
            all.into_iter().filter(|e| e.size_label.to_lowercase().contains(&needle)).collect()
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as i32 + delta).rem_euclid(len as i32) as usize;
    }
}

pub fn handle_key(picker: &mut ReamerPickerState, model: &mut BushingModel, key: KeyEvent) -> (bool, Vec<Effect>) {
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
            picker.move_cursor(-1);
            (true, Vec::new())
        }
        KeyCode::Down => {
            picker.move_cursor(1);
            (true, Vec::new())
        }
        KeyCode::Enter => {
            if let Some(entry) = picker.visible().get(picker.cursor) {
                model.select_reamer(entry);
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

fn tier_tag(tier: AvailabilityTier) -> &'static str {
    match tier {
        AvailabilityTier::Preferred => " [preferred]",
        AvailabilityTier::Common => "",
        AvailabilityTier::Special => " [special]",
    }
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, picker: &ReamerPickerState, model: &BushingModel, regions: &mut crate::mouse::MouseRegions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = centered_rect(64, 70, area);
    frame.render_widget(Clear, popup);

    let visible = picker.visible();
    let total = reamers::all_reamers().len();
    let title = if picker.filter_text.is_empty() {
        format!(" Reamer for Bore Diameter (currently {:.4} in, {total} sizes) - m: type exact value ", model.bore_dia)
    } else {
        format!(" Reamer ({} of {total}) - m: type exact value ", visible.len())
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
        empty_state::render(frame, list_area, theme, "No reamer sizes match the filter", Some("Esc to clear the filter"));
    } else {
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let selected = i == picker.cursor;
                let marker = if selected { "> " } else { "  " };
                let style = if selected { theme.selected_row_style() } else { Style::default() };
                ListItem::new(Line::from(Span::styled(
                    format!("{marker}{:<12} {:.4} in (+{:.4}/-{:.4}){}", entry.size_label, entry.nominal_in, entry.tool_tolerance_plus_in, entry.tool_tolerance_minus_in, tier_tag(entry.availability_tier)),
                    style,
                )))
            })
            .collect();
        let offset = crate::widgets::scroll_list::render(frame, list_area, items, Some(picker.cursor));
        regions.reamer_rows.extend(crate::mouse::list_row_regions(list_area, offset, visible.len()));
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
    fn open_near_positions_the_cursor_at_the_closest_real_size() {
        let model = BushingModel::default(); // bore_dia = 0.5
        let picker = ReamerPickerState::open_near(&model);
        assert!(picker.open);
        let all = reamers::all_reamers();
        let nearest = all[picker.cursor];
        for e in &all {
            assert!((nearest.nominal_in - 0.5).abs() <= (e.nominal_in - 0.5).abs());
        }
    }

    #[test]
    fn enter_sets_bore_dia_to_the_selected_reamer_and_closes() {
        let mut model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let target = picker.visible()[picker.cursor].nominal_in;
        handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(!picker.open);
        assert_eq!(model.bore_dia, target);
    }

    #[test]
    fn esc_closes_without_changing_bore_dia() {
        let mut model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let before = model.bore_dia;
        handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(!picker.open);
        assert_eq!(model.bore_dia, before);
    }

    #[test]
    fn slash_then_typing_narrows_the_visible_list() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        let mut model = model;
        handle_key(&mut picker, &mut model, key(KeyCode::Char('/')));
        for c in "1/4".chars() {
            handle_key(&mut picker, &mut model, key(KeyCode::Char(c)));
        }
        let visible = picker.visible();
        assert!(!visible.is_empty());
        assert!(visible.iter().all(|e| e.size_label.to_lowercase().contains("1/4")));
    }

    #[test]
    fn up_down_navigation_wraps() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState { open: true, cursor: 0, filter_text: String::new(), filtering: false };
        let mut model = model;
        let len = picker.visible().len();
        handle_key(&mut picker, &mut model, key(KeyCode::Up));
        assert_eq!(picker.cursor, len - 1);
        handle_key(&mut picker, &mut model, key(KeyCode::Down));
        assert_eq!(picker.cursor, 0);
    }

    #[test]
    fn render_does_not_panic_at_normal_or_degenerate_sizes() {
        let model = BushingModel::default();
        let picker = ReamerPickerState::open_near(&model);
        for (w, h) in [(100, 30), (0, 0), (1, 1)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| render(f, Rect { x: 0, y: 0, width: w, height: h }, &Theme::default_palette(), &picker, &model, &mut regions)).unwrap();
        }
    }

    #[test]
    fn render_with_filter_active_does_not_panic() {
        let model = BushingModel::default();
        let mut picker = ReamerPickerState::open_near(&model);
        picker.filtering = true;
        picker.filter_text = "3/8".to_string();
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| render(f, f.area(), &Theme::default_palette(), &picker, &model, &mut regions)).unwrap();
    }
}
