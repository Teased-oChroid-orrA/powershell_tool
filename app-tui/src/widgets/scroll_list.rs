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

/// Longest value (in characters) a field list asks its pane to make room
/// for. Longer values (a 60-character material name) wrap onto a second
/// row instead of widening the pane and squeezing the Results out.
pub const VALUE_CAP: usize = 20;

/// One `label  value` row. When it fits `width` it is a single line;
/// otherwise the label stays on the first line and the value moves to
/// wrapped, indented lines (up to three, then cut with `...`). Returns the
/// item and its height in rows. `extra` (a trailing hint span) goes on the
/// last line.
pub fn field_item(marker: &str, label: &str, label_width: usize, value: &str, width: u16, style: ratatui::style::Style, extra: Option<ratatui::text::Span<'static>>) -> (ListItem<'static>, u16) {
    use ratatui::text::{Line, Span};
    let width = width as usize;
    let one = format!("{marker}{label:<label_width$}{value}");
    let extra_len = extra.as_ref().map_or(0, |s| s.content.chars().count());
    if one.chars().count() + extra_len <= width || width < 12 {
        let mut spans = vec![Span::styled(one, style)];
        spans.extend(extra);
        return (ListItem::new(Line::from(spans)), 1);
    }
    let indent = 4usize;
    let room = width.saturating_sub(indent + extra_len).max(1);
    // Word-wrap the value into at most three indented lines, cutting the
    // last with "..." only if even that is not enough.
    let mut wrapped: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in value.split(' ') {
        let need = if cur.is_empty() { word.chars().count() } else { cur.chars().count() + 1 + word.chars().count() };
        if need > room && !cur.is_empty() {
            wrapped.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
        while cur.chars().count() > room {
            // A single over-long word: hard-split.
            let head: String = cur.chars().take(room).collect();
            cur = cur.chars().skip(room).collect();
            wrapped.push(head);
        }
    }
    if !cur.is_empty() {
        wrapped.push(cur);
    }
    if wrapped.len() > 3 {
        wrapped.truncate(3);
        let last = wrapped[2].chars().take(room.saturating_sub(3)).collect::<String>();
        wrapped[2] = format!("{last}...");
    }
    let first = Line::from(Span::styled(format!("{marker}{}", label.trim_end()), style));
    let mut lines = vec![first];
    let n = wrapped.len();
    for (i, w) in wrapped.into_iter().enumerate() {
        let mut spans = vec![Span::styled(format!("{:indent$}{w}", ""), style)];
        if i + 1 == n {
            spans.extend(extra.clone());
        }
        lines.push(Line::from(spans));
    }
    let h = lines.len() as u16;
    (ListItem::new(lines), h)
}

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

    fn text_of(item: &ListItem) -> Vec<String> {
        // ListItem exposes its height only; render it to read the lines back.
        let mut t = Terminal::new(TestBackend::new(60, 6)).unwrap();
        t.draw(|f| f.render_widget(List::new(vec![item.clone()]), f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        (0..item.height() as u16).map(|y| (0..60).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn a_short_value_stays_on_one_row_and_a_long_one_wraps_under_its_label() {
        let st = ratatui::style::Style::default();
        let (item, h) = field_item("> ", "Housing Material", 18, "Al 7075-T6", 40, st, None);
        assert_eq!(h, 1);
        assert_eq!(text_of(&item), vec!["> Housing Material  Al 7075-T6"]);

        let long = "7075 Aluminum Alloy T651 Plate 0.250-0.499 in [A] (T3.7.6.0b1)";
        let (item, h) = field_item("> ", "Housing Material", 18, long, 36, st, None);
        let lines = text_of(&item);
        assert!(h >= 2 && h <= 4 && lines.len() == h as usize, "{lines:?}");
        assert_eq!(lines[0], "> Housing Material");
        assert!(lines.iter().all(|l| l.chars().count() <= 36), "{lines:?}");
        assert!(lines[1..].iter().all(|l| l.starts_with("    ")));
        assert!(lines[1..].join(" ").replace("    ", "").contains("T3.7.6.0b1") || lines.last().unwrap().ends_with("..."), "{lines:?}");
    }

    #[test]
    fn a_trailing_hint_goes_on_the_last_line_and_absurd_values_are_cut_with_dots() {
        let st = ratatui::style::Style::default();
        let hint = ratatui::text::Span::raw("  !");
        let (item, h) = field_item("  ", "Material", 10, &"word ".repeat(60), 30, st, Some(hint));
        let lines = text_of(&item);
        assert_eq!(h, 4, "label + at most three wrapped lines");
        assert!(lines[3].ends_with("...  !"), "{lines:?}");
        for w in 0..14 {
            let _ = field_item("", "L", 1, "value that is long", w, st, None); // degenerate widths must not panic
        }
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
