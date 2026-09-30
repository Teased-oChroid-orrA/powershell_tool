//! Scrollable, wrap-aware `Paragraph` rendering for a toolbox's Results/
//! readout pane - shared by every toolbox with a variable-length results
//! readout (bushing/pressure_vessel/preload_analysis/fastener_hole), fixing
//! the same "fixed space regardless of actual content" bug class the bottom
//! Hint panel (`widgets/hint_panel.rs`) was already fixed for, this time on
//! the results side: `Paragraph::new(lines)` rendered with no `.wrap()` and
//! no scroll silently character-truncates any line wider than the pane, and
//! silently drops any line past the pane's height with no way to reach it
//! or even know it exists.
//!
//! Reuses `hint_panel::wrapped_line_count`'s tested greedy word-wrap counter
//! (rather than a new approximation) to compute exactly how tall `lines`
//! renders once wrapped, so the scroll clamp and "more below" indicator
//! agree with what `Wrap { trim: true }` actually does.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::theme::Theme;
use crate::widgets::hint_panel::wrapped_line_count;

/// Rows scrolled per PageUp/PageDown press in every toolbox's Results pane -
/// the actual scroll position is clamped every render by [`render`], so this
/// only needs to be "a reasonable page size", not an exact fit for any one
/// terminal height.
pub const SCROLL_STEP: u16 = 10;

fn line_plain_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Total rendered height of `lines` word-wrapped to `width` columns - the
/// same wrapping `Wrap { trim: true }` performs. A blank `Line` (used
/// throughout every toolbox's readout as a section spacer) still claims one
/// row, matching how ratatui renders an empty line.
pub fn wrapped_height(lines: &[Line], width: u16) -> u16 {
    lines.iter().map(|l| wrapped_line_count(&line_plain_text(l), width).max(1)).fold(0u16, |acc, n| acc.saturating_add(n))
}

/// Renders `lines` into `area` with word-wrap, clamping `scroll` to the
/// deepest position that still fills the pane so it can never scroll past
/// the end - even after content shrinks between renders (e.g. toggling a
/// Numbers panel off) - and reserving the last row for a "N more below"
/// indicator whenever the content doesn't fit at once, so overflow is
/// discoverable without already knowing to scroll. The indicator row is
/// reserved (but left blank) once scrolled all the way to the bottom, so
/// the layout doesn't jump between scroll positions.
pub fn render<'a>(frame: &mut Frame, area: Rect, theme: &Theme, lines: Vec<Line<'a>>, scroll: u16) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let total_height = wrapped_height(&lines, area.width);

    let (body_area, indicator_area) = if total_height > area.height && area.height > 1 {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(area);
        (rows[0], Some(rows[1]))
    } else {
        (area, None)
    };

    let max_scroll = total_height.saturating_sub(body_area.height);
    let applied = scroll.min(max_scroll);
    let remaining = total_height.saturating_sub(applied).saturating_sub(body_area.height);

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }).scroll((applied, 0)), body_area);

    if let Some(indicator_area) = indicator_area {
        if remaining > 0 {
            let text = format!("\u{2193} {remaining} more line(s) below \u{2014} PageDown to scroll");
            frame.render_widget(Paragraph::new(Line::styled(text, theme.disabled_style())), indicator_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn lines(n: usize) -> Vec<Line<'static>> {
        (0..n).map(|i| Line::from(format!("line {i}"))).collect()
    }

    #[test]
    fn wrapped_height_counts_one_row_per_short_line() {
        assert_eq!(wrapped_height(&lines(5), 40), 5);
    }

    #[test]
    fn wrapped_height_counts_extra_rows_for_a_wrapped_line() {
        let long = vec![Line::from("aaaa bbbb cccc dddd")];
        let narrow = wrapped_height(&long, 9);
        let wide = wrapped_height(&long, 100);
        assert!(narrow > wide);
    }

    #[test]
    fn render_does_not_panic_at_degenerate_or_overflowing_sizes() {
        for (w, h) in [(0, 0), (1, 1), (40, 3), (40, 100)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let area = Rect::new(0, 0, w, h);
            terminal.draw(|f| render(f, area, &Theme::default_palette(), lines(50), 0)).unwrap();
        }
    }

    #[test]
    fn scroll_past_the_end_is_clamped_not_blank() {
        let width = 40u16;
        let height = 10u16;
        let content = lines(30);

        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        terminal.draw(|f| render(f, area, &Theme::default_palette(), content, u16::MAX)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let rendered: String = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n").collect();
        assert!(rendered.contains("line 29"), "an out-of-range scroll must clamp to the last valid position, not render blank:\n{rendered}");
    }
}
