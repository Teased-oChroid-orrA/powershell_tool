//! Busy-indicator glyph. Renders one frame of a rotor from a tick counter
//! owned by the caller (this module has no timing/state of its own - it's
//! a pure `tick -> glyph` function, redrawn each time the app's periodic
//! `Tick` event fires).

use ratatui::text::Span;

use crate::theme::{StatusTone, Theme};

/// Braille spinner - the common modern-TUI choice (used by e.g. many
/// `indicatif`-style CLIs). Some terminals/fonts render these poorly -
/// notably flagged for Windows ConHost in the migration plan - so
/// `spinner_span` falls back to a plain ASCII rotor when
/// `theme.reduced_color` is set, on the assumption that a user who opted
/// into (or was auto-detected into) the reduced-color fallback is also on
/// a terminal least likely to render fancy Unicode glyphs correctly.
pub const BRAILLE_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
pub const ASCII_FRAMES: &[char] = &['|', '/', '-', '\\'];

pub fn frame(tick: u64) -> char {
    BRAILLE_FRAMES[(tick as usize) % BRAILLE_FRAMES.len()]
}

pub fn ascii_frame(tick: u64) -> char {
    ASCII_FRAMES[(tick as usize) % ASCII_FRAMES.len()]
}

pub fn spinner_span(tick: u64, theme: &Theme, tone: StatusTone) -> Span<'static> {
    let glyph = if theme.reduced_color { ascii_frame(tick) } else { frame(tick) };
    Span::styled(glyph.to_string(), theme.status_style(tone))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_cycles_through_all_braille_glyphs_and_wraps() {
        let seen: Vec<char> = (0..BRAILLE_FRAMES.len() as u64).map(frame).collect();
        assert_eq!(seen, BRAILLE_FRAMES.to_vec());
        assert_eq!(frame(BRAILLE_FRAMES.len() as u64), BRAILLE_FRAMES[0]);
    }

    #[test]
    fn ascii_frame_cycles_and_wraps() {
        let seen: Vec<char> = (0..ASCII_FRAMES.len() as u64).map(ascii_frame).collect();
        assert_eq!(seen, ASCII_FRAMES.to_vec());
        assert_eq!(ascii_frame(ASCII_FRAMES.len() as u64), ASCII_FRAMES[0]);
    }

    #[test]
    fn reduced_color_theme_uses_ascii_glyphs() {
        let theme = Theme::reduced_palette();
        let span = spinner_span(0, &theme, StatusTone::Info);
        assert_eq!(span.content.to_string(), ASCII_FRAMES[0].to_string());
    }

    #[test]
    fn full_color_theme_uses_braille_glyphs() {
        let theme = Theme::default_palette();
        let span = spinner_span(0, &theme, StatusTone::Info);
        assert_eq!(span.content.to_string(), BRAILLE_FRAMES[0].to_string());
    }
}
