//! Material Lookup toolbox: a searchable, filterable, sortable browser over
//! every built-in material (the curated typical values plus all 1,375
//! MIL-HDBK-5J conditions in `mechanics-core`), with a property panel and a
//! side-by-side comparison of up to four marked materials.
//!
//! It owns no engineering data and no formulas - `model.rs` only filters,
//! sorts and formats `mechanics_core::materials`. One workspace pane
//! (`PANE_MAIN`); like the material pickers, every printable key types into
//! the search box and the commands are Ctrl chords and function keys, so
//! searching never collides with a shortcut (Tab leaves the pane).

pub mod model;
pub mod view;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::Effect;
use model::{Query, SortKey, MAX_COMPARE};

pub const PANE_MAIN: u8 = 0;
pub const PANE_COUNT: u8 = 1;

/// Rows moved by PageUp / PageDown.
const PAGE: usize = 10;

pub struct MaterialLookupState {
    pub query: Query,
    /// Catalog indices matching `query`, in display order.
    pub hits: Vec<usize>,
    /// Position within `hits`.
    pub cursor: usize,
    /// Catalog indices marked for comparison, oldest first (at most `MAX_COMPARE`).
    pub marked: Vec<usize>,
    pub compare: bool,
    /// Opened by another toolbox to choose a material (Enter chooses instead of marking).
    pub picking: bool,
    /// Scroll offset of the property / comparison panel.
    pub detail_scroll: u16,
}

impl Default for MaterialLookupState {
    fn default() -> Self {
        let query = Query::default();
        let hits = model::filter_sort(&query);
        Self { query, hits, cursor: 0, marked: Vec::new(), compare: false, picking: false, detail_scroll: 0 }
    }
}

impl MaterialLookupState {
    /// Catalog index under the cursor.
    pub fn current(&self) -> Option<usize> {
        self.hits.get(self.cursor).copied()
    }

    /// Re-runs the query, keeping the highlighted material if it is still listed.
    fn refilter(&mut self) {
        let keep = self.current();
        self.hits = model::filter_sort(&self.query);
        self.cursor = keep.and_then(|k| self.hits.iter().position(|&h| h == k)).unwrap_or(0);
        self.detail_scroll = 0;
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.hits.is_empty() {
            self.cursor = 0;
            return;
        }
        let last = self.hits.len() - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last as isize) as usize;
        self.detail_scroll = 0;
    }

    /// Marks / unmarks the highlighted material for comparison; the oldest mark
    /// is dropped when a fifth would be added.
    pub fn toggle_mark(&mut self) {
        let Some(i) = self.current() else { return };
        if let Some(p) = self.marked.iter().position(|&m| m == i) {
            self.marked.remove(p);
        } else {
            if self.marked.len() == MAX_COMPARE {
                self.marked.remove(0);
            }
            self.marked.push(i);
        }
    }

    fn cycle_group(&mut self, step: isize) {
        let n = model::groups().len() as isize;
        self.query.group = (self.query.group as isize + step).rem_euclid(n) as usize;
        self.refilter();
    }

    fn cycle_sort(&mut self) {
        self.query.sort = self.query.sort.next();
        // A property sort starts with the strongest/largest first; name starts A-Z.
        self.query.descending = self.query.sort != SortKey::Name;
        self.refilter();
    }

    pub fn report_text(&self) -> String {
        model::report_text(&self.query, &self.hits, &self.marked)
    }
}

fn export_effect(state: &MaterialLookupState) -> Vec<Effect> {
    match crate::paths::app_data_dir() {
        Some(dir) => vec![Effect::WriteTextFileAndOpen { path: dir.join("reports").join("material-lookup.txt").to_string_lossy().into_owned(), contents: state.report_text() }],
        None => Vec::new(),
    }
}

/// Toolbox-local key routing - same `(consumed, effects)` contract as the
/// other toolboxes. Letters are matched in both cases (Windows Caps Lock can
/// report an uppercase letter with no Shift held).
pub fn handle_key(state: &mut MaterialLookupState, key: KeyEvent) -> (bool, Vec<Effect>) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Up => state.move_cursor(-1),
        KeyCode::Down => state.move_cursor(1),
        KeyCode::PageUp if ctrl => state.detail_scroll = state.detail_scroll.saturating_sub(crate::widgets::scroll_paragraph::SCROLL_STEP),
        KeyCode::PageDown if ctrl => state.detail_scroll = state.detail_scroll.saturating_add(crate::widgets::scroll_paragraph::SCROLL_STEP),
        KeyCode::PageUp => state.move_cursor(-(PAGE as isize)),
        KeyCode::PageDown => state.move_cursor(PAGE as isize),
        KeyCode::Home => {
            state.cursor = 0;
            state.detail_scroll = 0;
        }
        KeyCode::End => {
            state.cursor = state.hits.len().saturating_sub(1);
            state.detail_scroll = 0;
        }
        KeyCode::Left => state.cycle_group(-1),
        KeyCode::Right => state.cycle_group(1),
        KeyCode::Enter => state.toggle_mark(),
        KeyCode::F(2) => state.compare = !state.compare,
        KeyCode::F(3) => return (true, export_effect(state)),
        KeyCode::Esc => {
            if state.query.text.is_empty() {
                return (false, Vec::new());
            }
            state.query.text.clear();
            state.refilter();
        }
        KeyCode::Char(c) if ctrl => match c {
            'b' | 'B' => {
                state.query.basis = state.query.basis.next();
                state.refilter();
            }
            's' | 'S' => state.cycle_sort(),
            'r' | 'R' => {
                state.query.descending = !state.query.descending;
                state.refilter();
            }
            'o' | 'O' => state.toggle_mark(),
            'x' | 'X' => state.marked.clear(),
            'v' | 'V' => state.compare = !state.compare,
            'e' | 'E' => return (true, export_effect(state)),
            _ => return (false, Vec::new()),
        },
        _ => {
            if crate::widgets::material_detail::search_edit(&mut state.query.text, &key) {
                state.refilter_from_top();
            } else {
                return (false, Vec::new());
            }
        }
    }
    (true, Vec::new())
}

impl MaterialLookupState {
    /// A changed search starts at the best match, not wherever the old cursor was.
    fn refilter_from_top(&mut self) {
        self.hits = model::filter_sort(&self.query);
        self.cursor = 0;
        self.detail_scroll = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn plain(code: KeyCode) -> KeyEvent {
        key(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_text(state: &mut MaterialLookupState, text: &str) {
        for c in text.chars() {
            handle_key(state, plain(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_filters_the_list_and_esc_clears_it() {
        let mut s = MaterialLookupState::default();
        let all = s.hits.len();
        type_text(&mut s, "7075 t6");
        assert!(s.hits.len() < all && !s.hits.is_empty());
        assert_eq!(s.query.text, "7075 t6");
        handle_key(&mut s, plain(KeyCode::Esc));
        assert!(s.query.text.is_empty());
        assert_eq!(s.hits.len(), all);
    }

    #[test]
    fn q_and_other_global_shortcuts_type_into_the_search_instead() {
        let mut s = MaterialLookupState::default();
        let (consumed, _) = handle_key(&mut s, plain(KeyCode::Char('q')));
        assert!(consumed, "a letter is search text, never a quit");
        assert_eq!(s.query.text, "q");
        let (consumed, _) = handle_key(&mut s, plain(KeyCode::Tab));
        assert!(!consumed, "Tab is left to the shell so focus can leave the pane");
    }

    #[test]
    fn esc_with_an_empty_search_is_left_to_the_shell() {
        let mut s = MaterialLookupState::default();
        assert!(!handle_key(&mut s, plain(KeyCode::Esc)).0);
    }

    #[test]
    fn up_down_clamp_and_page_keys_jump() {
        let mut s = MaterialLookupState::default();
        handle_key(&mut s, plain(KeyCode::Up));
        assert_eq!(s.cursor, 0);
        handle_key(&mut s, plain(KeyCode::PageDown));
        assert_eq!(s.cursor, PAGE);
        handle_key(&mut s, plain(KeyCode::End));
        assert_eq!(s.cursor, s.hits.len() - 1);
        handle_key(&mut s, plain(KeyCode::Down));
        assert_eq!(s.cursor, s.hits.len() - 1);
        handle_key(&mut s, plain(KeyCode::Home));
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn left_right_cycle_the_group_filter_and_wrap() {
        let mut s = MaterialLookupState::default();
        let n = model::groups().len();
        handle_key(&mut s, plain(KeyCode::Right));
        assert_eq!(s.query.group, 1);
        assert!(s.hits.len() < model::catalog().len());
        handle_key(&mut s, plain(KeyCode::Left));
        assert_eq!(s.query.group, 0);
        handle_key(&mut s, plain(KeyCode::Left));
        assert_eq!(s.query.group, n - 1, "wraps to the last group");
    }

    #[test]
    fn ctrl_chords_cycle_basis_and_sort_and_reverse() {
        let mut s = MaterialLookupState::default();
        let all = s.hits.len();
        handle_key(&mut s, ctrl('b'));
        assert_eq!(s.query.basis, model::Basis::A);
        assert!(s.hits.len() < all);
        handle_key(&mut s, ctrl('S'));
        assert_eq!(s.query.sort, SortKey::Ftu);
        assert!(s.query.descending, "a property sort starts strongest first");
        let top = s.hits[0];
        handle_key(&mut s, ctrl('r'));
        assert!(!s.query.descending);
        assert_ne!(s.hits[0], top, "reversing changes the first row");
    }

    #[test]
    fn enter_marks_for_comparison_and_the_fifth_mark_drops_the_oldest() {
        let mut s = MaterialLookupState::default();
        let first = s.current().unwrap();
        for _ in 0..MAX_COMPARE {
            handle_key(&mut s, plain(KeyCode::Enter));
            handle_key(&mut s, plain(KeyCode::Down));
        }
        assert_eq!(s.marked.len(), MAX_COMPARE);
        assert!(s.marked.contains(&first));
        handle_key(&mut s, plain(KeyCode::Enter));
        assert_eq!(s.marked.len(), MAX_COMPARE);
        assert!(!s.marked.contains(&first), "the oldest mark was dropped");
        handle_key(&mut s, ctrl('x'));
        assert!(s.marked.is_empty());
    }

    #[test]
    fn marking_twice_unmarks() {
        let mut s = MaterialLookupState::default();
        handle_key(&mut s, plain(KeyCode::Enter));
        assert_eq!(s.marked.len(), 1);
        handle_key(&mut s, plain(KeyCode::Enter));
        assert!(s.marked.is_empty());
    }

    #[test]
    fn f2_and_ctrl_v_toggle_the_comparison_view() {
        let mut s = MaterialLookupState::default();
        handle_key(&mut s, plain(KeyCode::F(2)));
        assert!(s.compare);
        handle_key(&mut s, ctrl('V'));
        assert!(!s.compare);
    }

    #[test]
    fn export_writes_a_report_under_the_app_data_folder() {
        let mut s = MaterialLookupState::default();
        type_text(&mut s, "4340");
        let (consumed, effects) = handle_key(&mut s, plain(KeyCode::F(3)));
        assert!(consumed);
        match effects.as_slice() {
            [Effect::WriteTextFileAndOpen { path, contents }] => {
                assert!(path.ends_with("material-lookup.txt"), "{path}");
                assert!(contents.contains("4340"));
            }
            [] => {} // no app-data dir in this environment
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_new_search_starts_at_the_first_match_but_a_filter_change_keeps_the_highlight() {
        let mut s = MaterialLookupState::default();
        type_text(&mut s, "7075");
        handle_key(&mut s, plain(KeyCode::Down));
        handle_key(&mut s, plain(KeyCode::Down));
        let here = s.current().unwrap();
        handle_key(&mut s, ctrl('b')); // basis filter: keeps the highlight when it still matches
        if s.hits.contains(&here) {
            assert_eq!(s.current(), Some(here));
        }
        type_text(&mut s, " t6");
        assert_eq!(s.cursor, 0);
    }
}
