//! Centered placeholder for "nothing to show yet" states - reusable by any
//! toolbox (never searched / zero results / a not-yet-migrated toolbox),
//! not just Search Files.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;

/// Renders `message` (and optional `hint`) vertically and horizontally
/// centered within `area`. Guards degenerate (zero/near-zero) areas
/// instead of assuming space exists - small-terminal behavior must never
/// panic, only degrade.
pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, message: &str, hint: Option<&str>) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let line_count = if hint.is_some() { 2 } else { 1 };
    // Never ask Layout for more vertical space than `area` actually has -
    // `Constraint::Length` beyond the area is handled gracefully by
    // ratatui's layout solver, but centering math below assumes the
    // block's height is at most `area.height`, so clamp explicitly.
    let block_height = line_count.min(area.height);

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(block_height),
            Constraint::Min(0),
        ])
        .split(area);
    let centered = vertical[1];

    let mut lines = vec![Line::from(message).alignment(Alignment::Center).style(theme.fg_subtle)];
    if let Some(hint) = hint {
        lines.push(Line::from(hint).alignment(Alignment::Center).style(theme.disabled_style()));
    }

    frame.render_widget(Paragraph::new(lines), centered);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn renders_without_panicking_at_normal_size() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &Theme::default_palette(), "No results yet", Some("Press Enter to search")))
            .unwrap();
    }

    #[test]
    fn renders_without_panicking_at_degenerate_sizes() {
        for (w, h) in [(0, 0), (1, 1), (0, 5), (5, 0)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let area = Rect::new(0, 0, w, h);
            terminal
                .draw(|f| render(f, area, &Theme::default_palette(), "No results yet", None))
                .unwrap();
        }
    }
}
