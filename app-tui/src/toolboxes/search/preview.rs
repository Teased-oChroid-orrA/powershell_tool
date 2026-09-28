//! Match-highlighting for the Search Files preview pane.
//!
//! `highlight_ranges` ports `app/src/preview.rs::highlighted_line`'s
//! range-computation core verbatim (same case-insensitive substring scan +
//! overlap-merge algorithm, same tolerance documented on the original:
//! "close enough for a preview highlight, not required to be pixel-identical
//! to the HTML report's own highlighter" - the report's own highlighter in
//! `search_core::report` stays the source of truth for the saved report).
//! `render_highlighted_line` is new - there is no ratatui equivalent to
//! port, since the original builds a Dioxus `Element` with `mark { }` tags.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use search_core::models::FileSearchResult;

/// Case-insensitive substring scan for every filter in `matched_filters`
/// against `line`, merged into a sorted, non-overlapping list of byte
/// ranges. Ported 1:1 from `app/src/preview.rs::highlighted_line`'s first
/// half (the range computation, before it starts building UI elements).
pub fn highlight_ranges(line: &str, matched_filters: &[String]) -> Vec<(usize, usize)> {
    let lower_line = line.to_lowercase();
    let mut ranges: Vec<(usize, usize)> = Vec::new();

    for f in matched_filters {
        if f.is_empty() {
            continue;
        }
        let lower_f = f.to_lowercase();
        let mut search_from = 0usize;
        while search_from <= lower_line.len() {
            let Some(rel_pos) = lower_line[search_from..].find(&lower_f) else { break };
            let start = search_from + rel_pos;
            let end = start + lower_f.len();
            ranges.push((start, end));
            search_from = end.max(start + 1);
        }
    }

    ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in ranges {
        if let Some(last) = merged.last_mut() {
            if r.0 <= last.1 {
                if r.1 > last.1 {
                    last.1 = r.1;
                }
                continue;
            }
        }
        merged.push(r);
    }
    merged
}

/// Renders one matched line as a styled `Line`, with `highlight_style`
/// applied to every range `highlight_ranges` finds and normal styling
/// everywhere else. The caller controls the exact highlight color/weight
/// (the theme's accent/danger style, typically) rather than this module
/// hardcoding one, so it stays reusable across light/dark/reduced-color
/// themes.
pub fn render_highlighted_line(
    line: &str,
    matched_filters: &[String],
    highlight_style: Style,
) -> Line<'static> {
    let merged = highlight_ranges(line, matched_filters);

    let mut pos = 0usize;
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (start, end) in merged {
        if start > pos && line.is_char_boundary(pos) && line.is_char_boundary(start) {
            spans.push(Span::raw(line[pos..start].to_string()));
        }
        if line.is_char_boundary(start) && line.is_char_boundary(end) {
            spans.push(Span::styled(line[start..end].to_string(), highlight_style));
            pos = end;
        }
    }
    if pos < line.len() && line.is_char_boundary(pos) {
        spans.push(Span::raw(line[pos..].to_string()));
    }

    Line::from(spans)
}

/// One-line metadata summary for the preview pane's header: hit count,
/// file size, and modified date. `search_core::models::FileSearchResult`
/// has no `file_name`-only field (only `full_name`, the full path) - the
/// caller is expected to already have a shorter display name if it wants
/// one; this only summarizes hits/size/dates, which is all this module's
/// directive scope needs.
pub fn metadata_summary(result: &FileSearchResult) -> String {
    let hits = result.hits.len();
    let hit_word = if hits == 1 { "hit" } else { "hits" };
    format!(
        "{hits} {hit_word} · {size} · modified {modified}",
        size = crate::format::format_bytes(result.file_length),
        modified = result.modified.format("%Y-%m-%d %H:%M"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    fn hl_style() -> Style {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    }

    #[test]
    fn no_match_yields_no_ranges() {
        assert!(highlight_ranges("hello world", &["zzz".to_string()]).is_empty());
    }

    #[test]
    fn single_match_yields_correct_byte_range() {
        let ranges = highlight_ranges("hello world", &["world".to_string()]);
        assert_eq!(ranges, vec![(6, 11)]);
    }

    #[test]
    fn overlapping_matches_are_merged() {
        // "hello" is (0, 5); "lo wor" starts at index 3 (inside "hello")
        // and runs 6 bytes to (3, 9) - the two ranges overlap and merge
        // into one spanning the earliest start to the latest end: (0, 9).
        let ranges = highlight_ranges("hello world", &["hello".to_string(), "lo wor".to_string()]);
        assert_eq!(ranges, vec![(0, 9)]);
    }

    #[test]
    fn matching_is_case_insensitive() {
        let ranges = highlight_ranges("HELLO world", &["hello".to_string()]);
        assert_eq!(ranges, vec![(0, 5)]);
    }

    #[test]
    fn filter_not_present_contributes_nothing() {
        let ranges = highlight_ranges("hello world", &["hello".to_string(), "nope".to_string()]);
        assert_eq!(ranges, vec![(0, 5)]);
    }

    #[test]
    fn empty_filter_string_is_skipped() {
        let ranges = highlight_ranges("hello world", &[String::new()]);
        assert!(ranges.is_empty());
    }

    #[test]
    fn render_produces_three_spans_for_one_interior_match() {
        let line = render_highlighted_line("hello world today", &["world".to_string()], hl_style());
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content, "hello ");
        assert_eq!(line.spans[1].content, "world");
        assert_eq!(line.spans[1].style, hl_style());
        assert_eq!(line.spans[2].content, " today");
    }

    #[test]
    fn render_with_no_match_is_a_single_plain_span() {
        let line = render_highlighted_line("nothing matches here", &["zzz".to_string()], hl_style());
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "nothing matches here");
        assert_ne!(line.spans[0].style, hl_style());
    }

    #[test]
    fn render_match_at_very_start_has_no_leading_plain_span() {
        let line = render_highlighted_line("world hello", &["world".to_string()], hl_style());
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content, "world");
        assert_eq!(line.spans[0].style, hl_style());
    }
}
