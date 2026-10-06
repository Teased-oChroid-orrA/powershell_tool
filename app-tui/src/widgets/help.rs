//! Keybinding discoverability: a persistent one-line context-bar hint
//! strip (`render_status_hints`, always visible in the status bar) plus a
//! full keybinding-reference overlay (`render_overlay`, toggled by `?`).
//! Together with the command palette (which doubles as a searchable
//! command list), this is the three-layer discoverability the migration
//! plan calls for.

use crate::widgets::popup::centered_rect;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy)]
pub struct KeyHint {
    pub key: &'static str,
    pub label: &'static str,
}

/// Full keybinding reference, grouped into `(section_title, hints)` pairs.
pub fn render_overlay(frame: &mut Frame, area: Rect, theme: &Theme, sections: &[(&str, &[KeyHint])]) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let popup = centered_rect(70, 70, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(true))
        .title(" Help (? to close) ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::new();
    for (title, hints) in sections {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(*title, theme.title_style(true))));
        for hint in *hints {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<10}", hint.key), accent_style(theme)),
                Span::styled(hint.label, ratatui::style::Style::default().fg(theme.fg)),
            ]));
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn accent_style(theme: &Theme) -> ratatui::style::Style {
    ratatui::style::Style::default().fg(theme.accent)
}

/// A single-line, space-separated `"key label"` strip for the bottom
/// status bar - always visible, not an overlay. Truncates gracefully
/// (ratatui's `Paragraph` clips at the area boundary rather than
/// panicking) instead of assuming enough width exists.
pub fn render_status_hints(frame: &mut Frame, area: Rect, theme: &Theme, hints: &[KeyHint]) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let mut spans: Vec<Span> = Vec::new();
    for (i, hint) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(hint.key, accent_style(theme)));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(hint.label, ratatui::style::Style::default().fg(theme.fg_subtle)));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const SECTIONS: &[(&str, &[KeyHint])] = &[(
        "Global",
        &[
            KeyHint { key: "Ctrl+P", label: "Commands" },
            KeyHint { key: "q", label: "Quit" },
        ],
    )];

    #[test]
    fn overlay_renders_without_panicking_at_normal_size() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_overlay(f, f.area(), &Theme::default_palette(), SECTIONS)).unwrap();
    }

    #[test]
    fn overlay_renders_without_panicking_at_small_size() {
        for (w, h) in [(1, 1), (5, 3), (0, 0)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let area = Rect::new(0, 0, w, h);
            terminal.draw(|f| render_overlay(f, area, &Theme::default_palette(), SECTIONS)).unwrap();
        }
    }

    #[test]
    fn status_hints_render_without_panicking_when_too_narrow() {
        let backend = TestBackend::new(3, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_status_hints(f, f.area(), &Theme::default_palette(), SECTIONS[0].1))
            .unwrap();
    }
}
