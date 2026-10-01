//! A single-row text-input line (`label` + editable `text` + cursor) that
//! always keeps the *end* of the text - where the cursor is - visible.
//!
//! A bare `Paragraph` of `label + text + "_"` silently clips at the right
//! edge, so a long path (an import file location, a pasted folder) pushes the
//! cursor and the characters being typed out of the box. Here the label
//! yields first (dropped entirely on very narrow rows) and the text is
//! shown as `…` + its tail, so what the user just typed is always on screen.

use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Width of the text the caller wants shown, in `char`s (the terminal cell
/// width of the glyphs this app prints here - paths and filter text - is 1).
fn chars(s: &str) -> usize {
    s.chars().count()
}

/// The last `max` characters of `text`, prefixed with `…` when anything was
/// cut. `max == 0` yields an empty string.
fn tail(text: &str, max: usize) -> String {
    let len = chars(text);
    if len <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let keep = max - 1;
    let skip = len - keep;
    format!("…{}", text.chars().skip(skip).collect::<String>())
}

/// Builds the line for a row `width` cells wide. `cursor` is the glyph drawn
/// after the text (`"_"` while editing, `""` otherwise).
pub fn line<'a>(theme: &Theme, label: &str, text: &str, cursor: &str, width: u16) -> Line<'a> {
    let width = width as usize;
    let cursor_len = chars(cursor);
    let label_len = chars(label);
    // Keep at least 24 cells (or the whole text, if shorter) for the text;
    // if the label would squeeze it below that, drop the label - what the
    // user is typing matters more than its caption.
    let show_label = width >= label_len + cursor_len + chars(text).min(24);
    let room = width.saturating_sub(cursor_len + if show_label { label_len } else { 0 });
    let mut spans = Vec::with_capacity(3);
    if show_label {
        spans.push(Span::styled(label.to_string(), theme.title_style(true)));
    }
    spans.push(Span::raw(tail(text, room)));
    if !cursor.is_empty() {
        spans.push(Span::styled(cursor.to_string(), theme.title_style(true)));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn short_text_is_shown_whole_with_label_and_cursor() {
        let l = line(&Theme::default_palette(), "Path: ", "abc", "_", 40);
        assert_eq!(flat(&l), "Path: abc_");
    }

    #[test]
    fn long_text_keeps_its_tail_and_the_cursor_inside_the_width() {
        let text = "C:\\Users\\someone\\Documents\\reamers\\a-very-long-folder-name\\reamer-library.json";
        for width in [20u16, 30, 46, 60] {
            let l = line(&Theme::default_palette(), "Import from (Enter to confirm, Esc to cancel): ", text, "_", width);
            let rendered = flat(&l);
            assert!(rendered.chars().count() <= width as usize, "{} > {width}: {rendered}", rendered.chars().count());
            assert!(rendered.ends_with("library.json_"), "tail and cursor must stay visible: {rendered}");
        }
    }

    #[test]
    fn label_is_dropped_before_the_text_is_squeezed_too_small() {
        let l = line(&Theme::default_palette(), "A long label: ", "0123456789", "_", 20);
        assert!(!flat(&l).contains("label"));
        assert_eq!(flat(&l), "0123456789_");
        assert!(flat(&l).ends_with("_"));
    }

    #[test]
    fn degenerate_widths_do_not_panic() {
        for w in 0..4 {
            let _ = line(&Theme::default_palette(), "L: ", "text", "_", w);
        }
    }

    #[test]
    fn multibyte_text_is_cut_on_char_boundaries() {
        let l = line(&Theme::default_palette(), "", "ééééééééééé", "_", 6);
        assert_eq!(flat(&l).chars().count(), 6);
    }
}
