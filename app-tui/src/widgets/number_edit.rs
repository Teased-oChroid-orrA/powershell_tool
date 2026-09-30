//! Shared numeric edit-buffer key handling - factors out the "digit/'.'/'-'
//! typed on a selected `Number` row starts editing immediately (rather than
//! requiring `Enter` first), `Delete` clears the whole buffer, `Backspace`
//! pops one character" behavior that used to be hand-copied, nearly
//! identically, into every toolbox's own `mod.rs`. `Enter` still exists too
//! (starts editing prefilled with the current value, for tweaking rather
//! than retyping) - both paths coexist, this module only owns the buffer-
//! editing half, not the "how editing starts" half (that stays toolbox-
//! specific, since which row is selected and what it means differs per
//! toolbox).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The character `key` would contribute to a numeric edit buffer, if any -
/// a digit, decimal point, or leading minus, with no unexpected modifier
/// held. Same character/modifier rule every toolbox's own edit-buffer
/// handling already enforced ad hoc before this module existed.
pub fn number_char(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) && (c.is_ascii_digit() || c == '.' || c == '-') => Some(c),
        _ => None,
    }
}

/// Handles one key while a `Number` row's edit buffer is active. Returns
/// `true` if the key was consumed by buffer editing. `Enter`/`Esc`
/// (commit/cancel) are deliberately not handled here - those call into
/// toolbox-specific commit logic, so the caller keeps those as its own
/// match arms ahead of this fallback.
pub fn handle_buffer_key(buffer: &mut String, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Backspace => {
            buffer.pop();
            true
        }
        KeyCode::Delete => {
            buffer.clear();
            true
        }
        _ => {
            if let Some(c) = number_char(key) {
                buffer.push(c);
                true
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn number_char_accepts_digits_dot_and_minus() {
        assert_eq!(number_char(&key(KeyCode::Char('7'))), Some('7'));
        assert_eq!(number_char(&key(KeyCode::Char('.'))), Some('.'));
        assert_eq!(number_char(&key(KeyCode::Char('-'))), Some('-'));
    }

    #[test]
    fn number_char_rejects_letters_and_ctrl_modified_keys() {
        assert_eq!(number_char(&key(KeyCode::Char('a'))), None);
        let mut ctrl_digit = key(KeyCode::Char('5'));
        ctrl_digit.modifiers = KeyModifiers::CONTROL;
        assert_eq!(number_char(&ctrl_digit), None, "Ctrl+<digit> must fall through, e.g. to a global binding, not type into the buffer");
    }

    #[test]
    fn handle_buffer_key_backspace_pops_one_character() {
        let mut buf = "12.5".to_string();
        assert!(handle_buffer_key(&mut buf, &key(KeyCode::Backspace)));
        assert_eq!(buf, "12.");
    }

    #[test]
    fn handle_buffer_key_delete_clears_the_whole_buffer() {
        let mut buf = "12.5".to_string();
        assert!(handle_buffer_key(&mut buf, &key(KeyCode::Delete)));
        assert_eq!(buf, "");
    }

    #[test]
    fn handle_buffer_key_appends_a_valid_character() {
        let mut buf = "1".to_string();
        assert!(handle_buffer_key(&mut buf, &key(KeyCode::Char('2'))));
        assert_eq!(buf, "12");
    }

    #[test]
    fn handle_buffer_key_ignores_an_invalid_character() {
        let mut buf = "1".to_string();
        assert!(!handle_buffer_key(&mut buf, &key(KeyCode::Char('x'))));
        assert_eq!(buf, "1");
    }
}
