//! Sizes the bottom per-field "Hint" panel `bushing`/`preload_analysis` (and,
//! after the input-grouping rollout, every other toolbox) render below their
//! field list. The panel used to be a hardcoded `Constraint::Length(2)`,
//! which silently clipped any hint whose wrapped text needed more than two
//! lines at the current width - no overflow indicator, just missing text.
//!
//! `ratatui-widgets`'s own wrap-aware measurement (`Paragraph::line_count`)
//! exists but is gated behind the `unstable-rendered-line-info` Cargo
//! feature (not enabled in this crate - see `app-tui/AGENTS.md`). Since hint
//! text is always our own static data, not arbitrary content, a small local
//! greedy word-wrap counter is enough to size the area correctly without
//! taking a dependency on an explicitly-unstable upstream API.

/// Number of lines `text` occupies when greedily word-wrapped to `width`
/// columns - the same wrapping ratatui's `Wrap { trim: true }` performs
/// (break between words, never mid-word, leading/trailing whitespace on
/// each line dropped). Returns `0` for empty text so a blank hint claims no
/// space.
pub fn wrapped_line_count(text: &str, width: u16) -> u16 {
    if text.is_empty() {
        return 0;
    }
    let width = width.max(1) as usize;
    let mut lines: u32 = 1;
    let mut line_len = 0usize;
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if line_len == 0 {
            // First word on the line - it starts the line even if it alone
            // exceeds `width` (matches ratatui: an overlong word still gets
            // placed, not skipped).
            line_len = word_len;
        } else if line_len + 1 + word_len <= width {
            line_len += 1 + word_len;
        } else {
            lines += 1;
            line_len = word_len;
        }
    }
    lines.min(u16::MAX as u32) as u16
}

/// The `Constraint::Length` value the bottom Hint panel should use for
/// `hint` at the given inner width - the wrapped line count, clamped to
/// `1..=max_lines` so a blank hint still reserves one line (keeps the field
/// list from jumping height on every selection change) and one abnormally
/// long hint can never crush the field list to nothing.
pub fn hint_panel_height(hint: &str, width: u16, max_lines: u16) -> u16 {
    wrapped_line_count(hint, width).max(1).min(max_lines.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_needs_no_lines() {
        assert_eq!(wrapped_line_count("", 40), 0);
    }

    #[test]
    fn short_text_fits_on_one_line() {
        assert_eq!(wrapped_line_count("short hint", 40), 1);
    }

    #[test]
    fn text_wraps_at_the_expected_word_boundary() {
        // "aaaa bbbb cccc" at width 9: "aaaa bbbb" (9 chars) fits exactly,
        // "cccc" wraps to its own line.
        assert_eq!(wrapped_line_count("aaaa bbbb cccc", 9), 2);
    }

    #[test]
    fn a_single_word_longer_than_width_still_counts_as_one_line() {
        assert_eq!(wrapped_line_count("supercalifragilistic", 5), 1);
    }

    #[test]
    fn narrower_width_produces_more_lines() {
        let text = "Torque Controlled solves preload from applied torque, the physical installation process.";
        let narrow = wrapped_line_count(text, 20);
        let wide = wrapped_line_count(text, 100);
        assert!(narrow > wide, "narrow={narrow} wide={wide}");
    }

    #[test]
    fn height_clamps_within_the_requested_range() {
        assert_eq!(hint_panel_height("", 40, 4), 1);
        let very_long = "word ".repeat(200);
        assert_eq!(hint_panel_height(&very_long, 10, 4), 4);
    }
}
