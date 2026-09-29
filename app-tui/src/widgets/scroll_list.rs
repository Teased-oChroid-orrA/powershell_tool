//! Renders a selectable list that scrolls to keep the selected row visible.
//!
//! Every selectable list in this crate (Results, Settings' Fields/Recents/
//! Presets, the extension picker's checkbox catalog, the `Ctrl+P` command
//! palette) used to render through ratatui's *stateless* path directly
//! (`frame.render_widget(List::new(items), area)`, or `.block(...)`
//! chained onto it for the palette) - stateless rendering always starts
//! drawing at item 0 and never scrolls. Each call site already tracked its
//! own `selected` index and
//! styled that row manually (a "> " marker + `selected_row_style`), so
//! moving the selection past the bottom of a short viewport kept changing
//! `selected` (and whatever pane reads it - e.g. the Results list driving
//! the Preview pane) while the rendered list itself never moved, leaving
//! the highlighted row scrolled off-screen with no visual trace.
//!
//! Fix: render through ratatui's *stateful* `List`/`ListState` path instead.
//! `ListState::select` plus `render_stateful_widget` makes ratatui compute a
//! scroll offset that keeps the selected index within the visible height -
//! this crate never sets `List::highlight_style`/`highlight_symbol`, so the
//! existing manual per-row styling is completely unaffected; only the
//! scroll offset changes.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{List, ListItem, ListState};

/// `items` must already be styled exactly as the caller wants each row to
/// look (including any "selected" marker/highlight) - this only handles
/// scrolling, not row appearance. `selected` is the same 0-based index into
/// `items` the caller used to decide which row got that styling; `None`
/// renders the list with no scroll-to-selection (equivalent to the old
/// stateless behavior, for an empty or no-selection case).
///
/// Returns the real scroll offset ratatui settled on (the index of the
/// first visible item) - callers with clickable rows feed this straight
/// into `mouse::list_row_regions` so hit-test geometry can never drift from
/// what was actually painted (there is no independent offset computation
/// anywhere else).
pub fn render(frame: &mut Frame, area: Rect, items: Vec<ListItem>, selected: Option<usize>) -> usize {
    let list = List::new(items);
    let mut state = ListState::default();
    state.select(selected);
    frame.render_stateful_widget(list, area, &mut state);
    state.offset()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::text::Line;

    fn items(n: usize) -> Vec<ListItem<'static>> {
        (0..n).map(|i| ListItem::new(Line::from(format!("row {i}")))).collect()
    }

    #[test]
    fn renders_without_panicking_at_normal_and_degenerate_sizes() {
        for (w, h) in [(40, 10), (0, 0), (1, 1), (40, 0), (0, 10)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let area = Rect::new(0, 0, w, h);
            terminal.draw(|f| { render(f, area, items(50), Some(49)); }).unwrap();
        }
    }

    #[test]
    fn a_selection_past_the_visible_height_scrolls_it_into_view() {
        // 50 rows in a 5-row-tall viewport (3 usable after a 2-row border
        // elsewhere is irrelevant here - this widget gets a raw inner Rect):
        // selecting the last row must move ratatui's internal scroll offset
        // so that row is actually painted somewhere in the buffer, not just
        // tracked as a number nobody renders.
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 20, 5);
        terminal.draw(|f| { render(f, area, items(50), Some(49)); }).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let rendered: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("row 49"), "selected row must be scrolled into the visible buffer:\n{rendered}");
    }
}
