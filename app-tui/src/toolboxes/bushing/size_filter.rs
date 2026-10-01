//! Filter text matching shared by the reamer and Bushing ID pickers.
//!
//! Typing digits/`.` in a picker starts filtering at once (no `/` needed) and
//! the text is matched against each size's decimal inches (4 places), so
//! `0.26` lists 0.2600, 0.2624, 0.2652, 0.2689... as you type. Anything with
//! other characters (`#6`, `1/4`, `common`) is matched as text by the
//! picker's own label rules.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// True when `needle` is purely a number being typed (digits and `.`).
pub fn is_numeric(needle: &str) -> bool {
    !needle.is_empty() && needle.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Does `value` (inches) match the numeric filter `needle`?
pub fn decimal_matches(needle: &str, value: f64) -> bool {
    format!("{value:.4}").contains(needle)
}

/// A digit or `.` typed with no modifier - what starts numeric filtering.
pub fn numeric_start_char(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) && (c.is_ascii_digit() || c == '.') => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_zero_point_26_matches_the_26xx_sizes_only() {
        for v in [0.2600, 0.2624, 0.2652, 0.2689] {
            assert!(decimal_matches("0.26", v), "{v}");
        }
        for v in [0.2500, 0.2700, 0.0260, 0.3260, 0.1260] {
            assert!(!decimal_matches("0.26", v), "{v}");
        }
        assert!(decimal_matches(".26", 0.2624));
    }

    #[test]
    fn only_digits_and_dots_are_numeric() {
        assert!(is_numeric("0.26") && is_numeric(".5") && is_numeric("12"));
        assert!(!is_numeric("") && !is_numeric("#6") && !is_numeric("1/4") && !is_numeric("common"));
    }
}
