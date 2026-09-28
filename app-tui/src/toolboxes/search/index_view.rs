//! Rendering for fast re-search indexing status. Pure functions of plain
//! parameters (no `AppState`/`SearchToolState` coupling) - same convention
//! as `widgets/*` and `preview.rs`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::{StatusTone, Theme};

use super::indexing::{IndexLocation, IndexRunState};

/// One status line meant to slot into the Run view's existing vertical
/// layout as one more `Constraint::Length(1)` row (the parent session owns
/// that wiring). Quiet when idle with no error, so it doesn't add visual
/// noise for users who never enable fast re-search.
pub fn render_status_line(frame: &mut Frame, area: Rect, theme: &Theme, tick: u64, state: &IndexRunState) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let line = if state.is_building {
        Line::from(vec![
            crate::widgets::spinner::spinner_span(tick, theme, StatusTone::Info),
            Span::raw(" "),
            Span::styled(state.status_text.clone(), theme.status_style(StatusTone::Info)),
        ])
    } else if let Some(error) = &state.last_error {
        Line::from(Span::styled(format!("Index error: {error}"), theme.status_style(StatusTone::Danger)))
    } else if !state.status_text.is_empty() {
        Line::from(Span::styled(state.status_text.clone(), theme.disabled_style()))
    } else {
        Line::from(Span::styled("Index: not built", theme.disabled_style()))
    };

    frame.render_widget(Paragraph::new(line), area);
}

/// Display strings for the Settings screen's index fields - the parent
/// session wires these into `settings_view.rs`'s existing `FIELDS`/
/// `apply_toggle`/`display_value` per-field-index match pattern using
/// `IndexLocation::cycle`/`label` (in `indexing.rs`) plus these two
/// helpers, exactly like every other bool/enum field there.
pub fn display_enabled(enabled: bool) -> &'static str {
    if enabled {
        "[x]"
    } else {
        "[ ]"
    }
}

pub fn display_location(location: IndexLocation) -> &'static str {
    location.label()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_at(width: u16, height: u16, state: &IndexRunState) {
        let backend = TestBackend::new(width.max(1), height.max(1));
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect { x: 0, y: 0, width, height };
                render_status_line(frame, area, &Theme::default(), 0, state);
            })
            .unwrap();
    }

    #[test]
    fn idle_state_renders_without_panicking() {
        render_at(40, 1, &IndexRunState::default());
    }

    #[test]
    fn building_state_renders_without_panicking() {
        let state = IndexRunState { is_building: true, status_text: "Indexing 3 of 10: a.txt".to_string(), last_error: None };
        render_at(40, 1, &state);
    }

    #[test]
    fn error_state_renders_without_panicking() {
        let state = IndexRunState { is_building: false, status_text: String::new(), last_error: Some("disk full".to_string()) };
        render_at(40, 1, &state);
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        render_at(0, 0, &IndexRunState::default());
        render_at(1, 1, &IndexRunState::default());
    }

    #[test]
    fn display_helpers_match_expected_strings() {
        assert_eq!(display_enabled(true), "[x]");
        assert_eq!(display_enabled(false), "[ ]");
        assert_eq!(display_location(IndexLocation::SearchFolder), "Search folder");
        assert_eq!(display_location(IndexLocation::OutputFolder), "Output folder");
    }
}
