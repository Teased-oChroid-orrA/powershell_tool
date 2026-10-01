//! Filter text matching shared by the reamer and Bushing ID pickers.
//!
//! Typing digits/`.` in a picker starts filtering at once (no `/` needed) and
//! the text is matched against each size's decimal inches (4 places), so
//! `0.26` lists 0.2600, 0.2624, 0.2652, 0.2689... as you type. On top of
//! those matches the list always includes the 3 sizes just below and the 3
//! just above the typed value (the "neighbours"), even when nothing matches
//! exactly, and the cursor lands on the closest one. Anything with other
//! characters (`#6`, `1/4`, `common`) is matched as text by the picker's own
//! label rules.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// True when `needle` is purely a number being typed (digits and `.`).
pub fn is_numeric(needle: &str) -> bool {
    !needle.is_empty() && needle.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Does `value` (inches) match the numeric filter `needle`?
pub fn decimal_matches(needle: &str, value: f64) -> bool {
    format!("{value:.4}").contains(needle)
}

/// How many neighbours are kept on each side of the typed value.
pub const NEIGHBOURS: usize = 3;

/// The typed value, when the filter text is a complete number (`0.26`, `.5`).
pub fn typed_value(needle: &str) -> Option<f64> {
    if !is_numeric(needle) {
        return None;
    }
    needle.parse::<f64>().ok()
}

/// Numeric-filter selection over `items` (kept in their original order):
/// every item whose decimal contains `needle`, plus the [`NEIGHBOURS`]
/// nearest items strictly below and the [`NEIGHBOURS`] nearest at or above the
/// typed value.
pub fn select_numeric<T>(items: Vec<T>, needle: &str, value: impl Fn(&T) -> f64) -> Vec<T> {
    let target = typed_value(needle);
    let mut keep: Vec<bool> = items.iter().map(|it| decimal_matches(needle, value(it))).collect();
    if let Some(t) = target {
        let mut below: Vec<usize> = (0..items.len()).filter(|&i| value(&items[i]) < t).collect();
        below.sort_by(|&a, &b| value(&items[b]).partial_cmp(&value(&items[a])).unwrap_or(std::cmp::Ordering::Equal));
        let mut above: Vec<usize> = (0..items.len()).filter(|&i| value(&items[i]) >= t).collect();
        above.sort_by(|&a, &b| value(&items[a]).partial_cmp(&value(&items[b])).unwrap_or(std::cmp::Ordering::Equal));
        for i in below.into_iter().take(NEIGHBOURS).chain(above.into_iter().take(NEIGHBOURS)) {
            keep[i] = true;
        }
    }
    items.into_iter().zip(keep).filter_map(|(it, k)| k.then_some(it)).collect()
}

/// Index of the item whose value is closest to the typed number (cursor start).
pub fn nearest_index<T>(items: &[T], needle: &str, value: impl Fn(&T) -> f64) -> usize {
    let Some(t) = typed_value(needle) else { return 0 };
    items.iter().enumerate().min_by(|(_, a), (_, b)| (value(a) - t).abs().partial_cmp(&(value(b) - t).abs()).unwrap_or(std::cmp::Ordering::Equal)).map(|(i, _)| i).unwrap_or(0)
}

/// True for a listed size that is only there as a neighbour (no decimal match).
pub fn is_neighbour(needle: &str, value: f64) -> bool {
    is_numeric(needle) && !decimal_matches(needle, value)
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
    fn neighbours_are_added_below_and_above_even_with_no_exact_hit() {
        let sizes = vec![0.10, 0.12, 0.14, 0.16, 0.18, 0.22, 0.24, 0.26, 0.28, 0.30];
        // "0.19" matches nothing as text, yet the 3 below and 3 above appear.
        let got = select_numeric(sizes.clone(), "0.19", |v| *v);
        assert_eq!(got, vec![0.14, 0.16, 0.18, 0.22, 0.24, 0.26]);
        // Exact/substring hits stay, neighbours are added around the typed value.
        let got = select_numeric(sizes.clone(), "0.2", |v| *v);
        assert!(got.contains(&0.22) && got.contains(&0.18) && got.contains(&0.16) && got.contains(&0.14), "{got:?}");
        // At the ends there are simply fewer neighbours.
        assert_eq!(select_numeric(sizes.clone(), "0.05", |v| *v), vec![0.10, 0.12, 0.14]);
        assert_eq!(select_numeric(sizes, "0.9", |v| *v), vec![0.26, 0.28, 0.30]);
    }

    #[test]
    fn nearest_index_points_at_the_closest_size() {
        let sizes = [0.14, 0.16, 0.18, 0.22];
        assert_eq!(nearest_index(&sizes, "0.19", |v| *v), 2);
        assert_eq!(nearest_index(&sizes, "0.21", |v| *v), 3);
        assert_eq!(nearest_index(&sizes, "abc", |v| *v), 0);
    }

    #[test]
    fn only_digits_and_dots_are_numeric() {
        assert!(is_numeric("0.26") && is_numeric(".5") && is_numeric("12"));
        assert!(!is_numeric("") && !is_numeric("#6") && !is_numeric("1/4") && !is_numeric("common"));
    }
}
