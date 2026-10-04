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

use std::fmt;
use std::ops::Deref;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A single-line edit buffer with a movable cursor: `Left`/`Right`/`Home`/
/// `End` move it, typed characters insert at it, `Backspace` removes the
/// character before it and `Delete` the one under it - so one digit in the
/// middle of a value can be fixed without retyping the rest. `cursor` is a
/// `char` index in `0..=len` (never a byte offset), kept in range by every
/// method; the text is read through `Deref<Target = str>`.
#[derive(Debug, Clone, Default)]
pub struct EditBuffer {
    text: String,
    cursor: usize,
}

impl EditBuffer {
    /// Replaces the text and puts the cursor after its last character.
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.chars().count();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Takes the text out (cursor reset), leaving the buffer empty.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text.char_indices().nth(char_idx).map_or(self.text.len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte_at(self.cursor);
            self.text.remove(at);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.chars().count() {
            let at = self.byte_at(self.cursor);
            self.text.remove(at);
        }
    }

    /// The text with `_` inserted at the cursor - what an edit row shows.
    /// At the end of the text this is the familiar trailing `_`.
    pub fn with_cursor(&self) -> String {
        let at = self.byte_at(self.cursor);
        format!("{}_{}", &self.text[..at], &self.text[at..])
    }

    /// Handles a cursor-movement or deletion key (`Left`/`Right`/`Home`/
    /// `End`/`Backspace`/`Delete`). Returns `true` if consumed.
    pub fn handle_edit_key(&mut self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.text.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.chars().count(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            _ => return false,
        }
        true
    }

    /// `handle_edit_key`, plus inserting `key`'s character at the cursor when
    /// `accept` allows it and no unexpected modifier (anything but `SHIFT`)
    /// is held - a `Ctrl`-modified character must fall through unconsumed so
    /// global bindings keep working while a field has focus.
    pub fn handle_key(&mut self, key: &KeyEvent, accept: impl Fn(char) -> bool) -> bool {
        if self.handle_edit_key(key) {
            return true;
        }
        match key.code {
            KeyCode::Char(c) if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) && accept(c) => {
                self.insert(c);
                true
            }
            _ => false,
        }
    }
}

impl Deref for EditBuffer {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for EditBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl PartialEq<&str> for EditBuffer {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

/// The character `key` would contribute to a numeric edit buffer, if any -
/// a digit, decimal point, or leading minus, with no unexpected modifier
/// held. Same character/modifier rule every toolbox's own edit-buffer
/// handling already enforced ad hoc before this module existed.
pub fn number_char(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) && is_number_char(c) => Some(c),
        _ => None,
    }
}

pub fn is_number_char(c: char) -> bool {
    c.is_ascii_digit() || c == '.' || c == '-'
}

/// Handles one key while a `Number` row's edit buffer is active. Returns
/// `true` if the key was consumed by buffer editing. `Enter`/`Esc`
/// (commit/cancel) are deliberately not handled here - those call into
/// toolbox-specific commit logic, so the caller keeps those as its own
/// match arms ahead of this fallback.
pub fn handle_buffer_key(buffer: &mut EditBuffer, key: &KeyEvent) -> bool {
    buffer.handle_key(key, is_number_char)
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

    fn buf(text: &str) -> EditBuffer {
        let mut b = EditBuffer::default();
        b.set(text);
        b
    }

    #[test]
    fn handle_buffer_key_backspace_removes_the_character_before_the_cursor() {
        let mut b = buf("12.5");
        assert!(handle_buffer_key(&mut b, &key(KeyCode::Backspace)));
        assert_eq!(b, "12.");
    }

    #[test]
    fn handle_buffer_key_appends_a_valid_character_at_the_end() {
        let mut b = buf("1");
        assert!(handle_buffer_key(&mut b, &key(KeyCode::Char('2'))));
        assert_eq!(b, "12");
    }

    #[test]
    fn handle_buffer_key_ignores_an_invalid_character() {
        let mut b = buf("1");
        assert!(!handle_buffer_key(&mut b, &key(KeyCode::Char('x'))));
        assert_eq!(b, "1");
    }

    #[test]
    fn left_right_move_the_cursor_so_a_middle_digit_can_be_replaced() {
        let mut b = buf("0.2500");
        // Cursor at end; move left 3 -> between "0.2" and "500".
        for _ in 0..3 {
            assert!(handle_buffer_key(&mut b, &key(KeyCode::Left)));
        }
        assert_eq!(b.with_cursor(), "0.2_500");
        handle_buffer_key(&mut b, &key(KeyCode::Backspace));
        handle_buffer_key(&mut b, &key(KeyCode::Char('4')));
        assert_eq!(b, "0.4500");
        // Cursor stays after the inserted digit, not at the end.
        assert_eq!(b.with_cursor(), "0.4_500");
    }

    #[test]
    fn delete_removes_the_character_under_the_cursor_not_the_whole_buffer() {
        let mut b = buf("0.2500");
        handle_buffer_key(&mut b, &key(KeyCode::Home));
        handle_buffer_key(&mut b, &key(KeyCode::Delete));
        assert_eq!(b, ".2500");
        handle_buffer_key(&mut b, &key(KeyCode::End));
        handle_buffer_key(&mut b, &key(KeyCode::Delete));
        assert_eq!(b, ".2500", "Delete at the end is a no-op");
    }

    #[test]
    fn cursor_is_clamped_at_both_ends() {
        let mut b = buf("12");
        for _ in 0..5 {
            handle_buffer_key(&mut b, &key(KeyCode::Right));
        }
        assert_eq!(b.with_cursor(), "12_");
        for _ in 0..5 {
            handle_buffer_key(&mut b, &key(KeyCode::Left));
        }
        assert_eq!(b.with_cursor(), "_12");
        handle_buffer_key(&mut b, &key(KeyCode::Backspace));
        assert_eq!(b, "12", "Backspace at the start is a no-op");
    }

    #[test]
    fn editing_is_char_indexed_for_multibyte_text() {
        let mut b = buf("aéb");
        handle_buffer_key(&mut b, &key(KeyCode::Left));
        b.handle_key(&key(KeyCode::Backspace), |_| true);
        assert_eq!(b, "ab");
        b.handle_key(&key(KeyCode::Char('ü')), |_| true);
        assert_eq!(b, "aüb");
    }
}
