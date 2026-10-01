//! Per-extension checkbox catalog picker for the Search Files Settings
//! view - a capability neither `app/` nor `app-egui/` has: instead of only
//! offering a static built-in extension list, this scans the actual search
//! folder and shows only the extensions genuinely present as options, which
//! can then be further filtered/tailored down (`/`) or extended with an
//! extension the scan didn't find (`Enter` while filtering).
//!
//! `scan_extensions` mirrors `search-core::file_reader::enumerate_files_safely`'s
//! walk semantics (hidden-file/dir skipping via the same `is_hidden`
//! platform split, folder pruning, cycle avoidance via a visited-set) and
//! `search-core::orchestrator::file_extension_lower`'s extension
//! convention (dot-prefixed, lowercased, e.g. `.txt`) closely enough that
//! this picker shows the same universe of extensions a real run would
//! actually see - but it is a fresh, independent walk (search-core exposes
//! no public "just enumerate, don't extract" API shaped for this), not a
//! call into search-core itself.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph};

use crate::theme::{StatusTone, Theme};
use crate::widgets::empty_state;

/// Recursively walks `root`, returning the distinct file extensions found
/// (dot-prefixed, lowercased, sorted alphabetically) - matching
/// `search_core::orchestrator::file_extension_lower`'s convention exactly,
/// so a value from here drops straight into `SearchToolConfig.selected_extensions`
/// with no translation.
///
/// A nonexistent/unreadable root, or any individual unreadable
/// subdirectory, is not an error here - it just contributes nothing to the
/// result, matching `enumerate_files_safely`'s own "a bad entry is skipped,
/// never fails the whole walk" philosophy. Symlinks are skipped entirely
/// (never followed), which both avoids symlink cycles and matches
/// `enumerate_files_safely`'s own file-type match (only `is_dir()`/`is_file()`
/// branches are handled there too - a symlink's `file_type()` reports the
/// link itself, not its target, under `read_dir`, so it falls through to
/// neither branch and is naturally skipped the same way here).
pub fn scan_extensions(root: &Path, exclude_folders: &[String], include_hidden: bool) -> Vec<String> {
    let mut found: HashSet<String> = HashSet::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let resolved = match std::fs::canonicalize(&dir) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let key = resolved.to_string_lossy().to_lowercase();
        if !visited.insert(key) {
            continue; // already visited this real directory - breaks any cycle
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else { continue };

            if file_type.is_dir() {
                if !include_hidden && is_hidden(&path) {
                    continue;
                }
                let dir_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                if is_excluded_directory(&dir_name, exclude_folders) {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() {
                if !include_hidden && is_hidden(&path) {
                    continue;
                }
                if let Some(ext) = path.extension() {
                    found.insert(format!(".{}", ext.to_string_lossy().to_lowercase()));
                }
            }
            // Symlinks (neither is_dir() nor is_file() under read_dir's
            // ReadDir::file_type, which does not follow links) are skipped.
        }
    }

    let mut result: Vec<String> = found.into_iter().collect();
    result.sort();
    result
}

#[cfg(windows)]
fn is_hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    std::fs::metadata(path).map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0).unwrap_or(false)
}

/// Non-Windows fallback, same convention as `search-core::file_reader::is_hidden`
/// - this app's shipped target is win-x64 only, so this path only matters
/// for local development/testing off-Windows.
#[cfg(not(windows))]
fn is_hidden(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with('.')).unwrap_or(false)
}

/// Whole-name, case-insensitive match against `exclude_folders` - a
/// simplified sibling of `search-core::file_reader::is_excluded_directory`
/// (that function additionally supports multi-segment sub-path excludes
/// like `"src/bin"` and is private to `search-core`, not reusable here).
/// Covers the common case (a bare folder name like `"node_modules"`).
fn is_excluded_directory(directory_name: &str, exclude_folders: &[String]) -> bool {
    exclude_folders.iter().any(|ex| ex.eq_ignore_ascii_case(directory_name))
}

#[derive(Debug, Clone, Default)]
pub struct ExtensionPicker {
    pub open: bool,
    pub available: Vec<String>,
    pub selected: HashSet<String>,
    /// Index into the *visible* (filtered) list, not directly into
    /// `available` - see `visible_indices`.
    pub cursor: usize,
    pub scan_error: Option<String>,
    /// Distinguishes "never scanned" from "scanned, found nothing" for the
    /// empty state - `available.is_empty()` alone can't tell those apart.
    pub scanned: bool,
    /// Live filter text (`/` to start typing) - narrows `available` to
    /// entries containing this text (case-insensitive substring), and also
    /// doubles as the source for "add as custom extension" (`Enter` while
    /// `filtering`), mirroring `app/`'s/`app-egui/`'s extension-filter text
    /// box. Stays active (still narrowing the list) after `filtering` goes
    /// back to `false`, until cleared.
    pub filter_text: String,
    /// Whether `filter_text` is currently being typed into. Mutually
    /// exclusive with the list-navigation keys (space/a/n/arrows) the same
    /// way `SettingsView::naming_preset` is with its own field list.
    pub filtering: bool,
}

impl ExtensionPicker {
    /// Opens the picker pre-seeded with a scan result, pre-checking
    /// whatever was already selected (case-insensitive) so re-opening
    /// after a previous selection doesn't lose it.
    pub fn open_with(available: Vec<String>, previously_selected: Option<&[String]>) -> Self {
        let selected = match previously_selected {
            Some(prev) => {
                let prev_lower: HashSet<String> = prev.iter().map(|e| e.to_lowercase()).collect();
                available.iter().filter(|e| prev_lower.contains(&e.to_lowercase())).cloned().collect()
            }
            None => HashSet::new(),
        };
        Self {
            open: true,
            available,
            selected,
            cursor: 0,
            scan_error: None,
            scanned: true,
            filter_text: String::new(),
            filtering: false,
        }
    }

    /// Opens the picker in an error state (e.g. the configured search path
    /// doesn't exist / isn't readable) - no scan result to show.
    pub fn open_with_error(error: impl Into<String>) -> Self {
        Self { open: true, scan_error: Some(error.into()), scanned: true, ..Default::default() }
    }

    /// Positions within `available` whose extension contains `filter_text`
    /// (case-insensitive substring) - every position when the filter is
    /// empty. What the list actually renders/navigates always goes through
    /// this, never `available` directly, so filtering and cursor movement
    /// can't drift out of sync.
    fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter_text.trim().to_lowercase();
        if needle.is_empty() {
            (0..self.available.len()).collect()
        } else {
            self.available.iter().enumerate().filter(|(_, e)| e.to_lowercase().contains(&needle)).map(|(i, _)| i).collect()
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        let visible_len = self.visible_indices().len();
        if visible_len == 0 {
            self.cursor = 0;
            return;
        }
        let len = visible_len as i32;
        self.cursor = (self.cursor as i32 + delta).rem_euclid(len) as usize;
    }
}

/// Trims, lowercases, and ensures a leading `.` - `None` for blank input.
/// Matches `app-egui/src/search.rs::AppState::add_custom_extension`'s
/// normalization exactly.
fn normalize_extension(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let with_dot = if trimmed.starts_with('.') { trimmed.to_string() } else { format!(".{trimmed}") };
    Some(with_dot.to_lowercase())
}

/// Returns `true` if the key was consumed - same contract as this
/// toolbox's other `handle_key` functions (see `settings_view.rs`).
pub fn handle_key(picker: &mut ExtensionPicker, key: KeyEvent) -> bool {
    if picker.filtering {
        return match key.code {
            // Commits the typed text: an extension already in `available`
            // (case-insensitive) is just selected, otherwise it's appended
            // as a new custom entry and selected - matches
            // `app-egui/`'s `add_custom_extension`. Does not close the
            // picker (unlike the base `Enter` below).
            KeyCode::Enter => {
                if let Some(ext) = normalize_extension(&picker.filter_text) {
                    if let Some(existing) = picker.available.iter().find(|e| e.eq_ignore_ascii_case(&ext)).cloned() {
                        picker.selected.insert(existing);
                    } else {
                        picker.available.push(ext.clone());
                        picker.selected.insert(ext);
                    }
                }
                picker.filtering = false;
                picker.filter_text.clear();
                picker.cursor = 0;
                true
            }
            // Stops typing without adding anything - the filter itself
            // stays active, still narrowing the list.
            KeyCode::Esc => {
                picker.filtering = false;
                picker.cursor = 0;
                true
            }
            KeyCode::Backspace => {
                picker.filter_text.pop();
                picker.cursor = 0;
                true
            }
            KeyCode::Delete => {
                picker.filter_text.clear();
                picker.cursor = 0;
                true
            }
            KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
                picker.filter_text.push(c);
                picker.cursor = 0;
                true
            }
            _ => false,
        };
    }

    match key.code {
        KeyCode::Char('/') => {
            picker.filtering = true;
            true
        }
        KeyCode::Up => {
            picker.move_cursor(-1);
            true
        }
        KeyCode::Down => {
            picker.move_cursor(1);
            true
        }
        KeyCode::Char(' ') => {
            if let Some(&idx) = picker.visible_indices().get(picker.cursor) {
                let ext = picker.available[idx].clone();
                if !picker.selected.remove(&ext) {
                    picker.selected.insert(ext);
                }
            }
            true
        }
        // Bulk toggling, scoped to whatever the filter currently narrows
        // the list to (the whole catalog when no filter is active) - a
        // folder can easily have dozens of distinct extensions, and
        // toggling each one individually would be tedious.
        KeyCode::Char('a' | 'A') => {
            for idx in picker.visible_indices() {
                picker.selected.insert(picker.available[idx].clone());
            }
            true
        }
        KeyCode::Char('n' | 'N') => {
            for idx in picker.visible_indices() {
                picker.selected.remove(&picker.available[idx]);
            }
            true
        }
        KeyCode::Enter => {
            picker.open = false;
            true
        }
        // First Esc clears an active filter rather than closing the whole
        // picker, so narrowing the list down doesn't feel like a dead end;
        // a second Esc (filter already empty) closes it, unchanged from
        // before filtering existed.
        KeyCode::Esc => {
            if !picker.filter_text.is_empty() {
                picker.filter_text.clear();
                picker.cursor = 0;
            } else {
                picker.open = false;
            }
            true
        }
        _ => false,
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, picker: &ExtensionPicker, regions: &mut crate::mouse::MouseRegions) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);

    let visible = picker.visible_indices();
    let title = if picker.filter_text.is_empty() {
        format!(" Extensions ({}/{} selected) ", picker.selected.len(), picker.available.len())
    } else {
        format!(" Extensions ({}/{} selected, {} shown) ", picker.selected.len(), picker.available.len(), visible.len())
    };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(title);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if let Some(error) = &picker.scan_error {
        frame.render_widget(Paragraph::new(Line::from(Span::styled(error.as_str(), theme.status_style(StatusTone::Danger)))), inner);
        return;
    }

    if !picker.scanned {
        empty_state::render(frame, inner, theme, "Not scanned yet", None);
        return;
    }

    // A bottom row for the filter/add-custom-extension line, shown while
    // actively typing (`filtering`) or whenever a filter is narrowing the
    // list, so the active filter text is never silently invisible.
    let (list_area, filter_area) = if (picker.filtering || !picker.filter_text.is_empty()) && inner.height > 1 {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(1)]).split(inner);
        (rows[0], Some(rows[1]))
    } else {
        (inner, None)
    };

    if picker.available.is_empty() {
        empty_state::render(frame, list_area, theme, "No files with an extension found", Some("/ to add a custom extension"));
    } else if visible.is_empty() {
        empty_state::render(frame, list_area, theme, "No extensions match the filter", Some("Enter to add it as a custom extension"));
    } else {
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(display_i, &idx)| {
                let ext = &picker.available[idx];
                let checked = if picker.selected.contains(ext) { "[x]" } else { "[ ]" };
                let selected_row = display_i == picker.cursor;
                let marker = if selected_row { "> " } else { "  " };
                let style = if selected_row { theme.selected_row_style() } else { Style::default() };
                ListItem::new(Line::from(Span::styled(format!("{marker}{checked} {ext}"), style)))
            })
            .collect();
        let offset = crate::widgets::scroll_list::render(frame, list_area, items, Some(picker.cursor));
        regions.extension_rows.extend(crate::mouse::list_row_regions(list_area, offset, visible.len()));
    }

    if let Some(area) = filter_area {
        let label = if picker.filtering { "Filter/add (Enter to add): " } else { "Filter: " };
        let cursor_glyph = if picker.filtering { "_" } else { "" };
        let line = crate::widgets::input_line::line(theme, label, picker.filter_text.as_str(), cursor_glyph, area.width);
        frame.render_widget(Paragraph::new(line), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn scan_finds_distinct_lowercased_extensions_and_dedupes_case() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::write(dir.path().join("B.TXT"), "x").unwrap();
        std::fs::write(dir.path().join("c.rs"), "x").unwrap();
        std::fs::write(dir.path().join("no_extension"), "x").unwrap();

        let mut found = scan_extensions(dir.path(), &[], true);
        found.sort();
        assert_eq!(found, vec![".rs".to_string(), ".txt".to_string()]);
    }

    #[test]
    fn scan_prunes_excluded_folders_by_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("node_modules").join("b.json"), "x").unwrap();

        let found = scan_extensions(dir.path(), &["node_modules".to_string()], true);
        assert_eq!(found, vec![".txt".to_string()]);
    }

    #[test]
    fn scan_respects_include_hidden_toggle() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".hidden.conf"), "x").unwrap();
        std::fs::write(dir.path().join("visible.txt"), "x").unwrap();

        #[cfg(not(windows))]
        {
            let hidden_excluded = scan_extensions(dir.path(), &[], false);
            assert_eq!(hidden_excluded, vec![".txt".to_string()]);

            let hidden_included = scan_extensions(dir.path(), &[], true);
            assert_eq!(hidden_included, vec![".conf".to_string(), ".txt".to_string()]);
        }
        #[cfg(windows)]
        {
            // On Windows, "hidden" is a file attribute bit, not a dot-prefix
            // convention - a dot-prefixed file created on Windows is not
            // itself flagged hidden, so both calls see it. Just confirm the
            // visible file is always found and the walk doesn't panic.
            let result = scan_extensions(dir.path(), &[], false);
            assert!(result.contains(&".txt".to_string()));
        }
    }

    #[test]
    fn scan_walks_nested_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.md"), "x").unwrap();

        let found = scan_extensions(dir.path(), &[], true);
        assert_eq!(found, vec![".md".to_string()]);
    }

    #[test]
    fn scan_of_a_nonexistent_path_returns_empty_not_a_panic() {
        let found = scan_extensions(Path::new("/this/does/not/exist/anywhere"), &[], true);
        assert!(found.is_empty());
    }

    #[test]
    fn open_with_preselects_case_insensitively() {
        let picker = ExtensionPicker::open_with(
            vec![".txt".to_string(), ".rs".to_string()],
            Some(&[".TXT".to_string()]),
        );
        assert!(picker.selected.contains(".txt"));
        assert_eq!(picker.selected.len(), 1);
    }

    #[test]
    fn space_toggles_selection() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string()], None);
        assert!(handle_key(&mut picker, key(KeyCode::Char(' '))));
        assert!(picker.selected.contains(".txt"));
        handle_key(&mut picker, key(KeyCode::Char(' ')));
        assert!(!picker.selected.contains(".txt"));
    }

    #[test]
    fn select_all_and_select_none() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string(), ".rs".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('a')));
        assert_eq!(picker.selected.len(), 2);
        handle_key(&mut picker, key(KeyCode::Char('n')));
        assert!(picker.selected.is_empty());
    }

    #[test]
    fn select_all_and_select_none_work_with_uppercase_from_caps_lock() {
        // Regression: crossterm's Windows backend reports Caps-Lock-typed
        // letters as uppercase with no Shift held - a bare-lowercase
        // pattern silently drops the binding on Windows only.
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string(), ".rs".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('A')));
        assert_eq!(picker.selected.len(), 2);
        handle_key(&mut picker, key(KeyCode::Char('N')));
        assert!(picker.selected.is_empty());
    }

    #[test]
    fn cursor_wraps_both_directions() {
        let mut picker = ExtensionPicker::open_with(vec![".a".to_string(), ".b".to_string(), ".c".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Up));
        assert_eq!(picker.cursor, 2);
        handle_key(&mut picker, key(KeyCode::Down));
        assert_eq!(picker.cursor, 0);
    }

    #[test]
    fn enter_and_esc_both_close_without_panicking() {
        let mut picker = ExtensionPicker::open_with(vec![".a".to_string()], None);
        assert!(handle_key(&mut picker, key(KeyCode::Enter)));
        assert!(!picker.open);

        let mut picker2 = ExtensionPicker::open_with(vec![".a".to_string()], None);
        assert!(handle_key(&mut picker2, key(KeyCode::Esc)));
        assert!(!picker2.open);
    }

    #[test]
    fn slash_starts_filtering() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string()], None);
        assert!(!picker.filtering);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        assert!(picker.filtering);
    }

    #[test]
    fn typing_a_filter_narrows_the_visible_list() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string(), ".rs".to_string(), ".rtf".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        for c in "rt".chars() {
            handle_key(&mut picker, key(KeyCode::Char(c)));
        }
        let visible: Vec<&str> = picker.visible_indices().iter().map(|&i| picker.available[i].as_str()).collect();
        assert_eq!(visible, vec![".rtf"]);
    }

    #[test]
    fn enter_while_filtering_adds_a_new_extension_not_found_by_the_scan() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        for c in "log".chars() {
            handle_key(&mut picker, key(KeyCode::Char(c)));
        }
        assert!(handle_key(&mut picker, key(KeyCode::Enter)));
        assert!(!picker.filtering);
        assert!(picker.filter_text.is_empty(), "filter text clears after a successful add");
        assert!(picker.available.contains(&".log".to_string()));
        assert!(picker.selected.contains(".log"));
        assert!(picker.open, "adding a custom extension must not close the picker");
    }

    #[test]
    fn enter_while_filtering_normalizes_missing_leading_dot() {
        let mut picker = ExtensionPicker::open_with(Vec::new(), None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        for c in "log".chars() {
            handle_key(&mut picker, key(KeyCode::Char(c)));
        }
        handle_key(&mut picker, key(KeyCode::Enter));
        assert_eq!(picker.available, vec![".log".to_string()]);
    }

    #[test]
    fn enter_while_filtering_on_an_existing_extension_selects_it_instead_of_duplicating() {
        let mut picker = ExtensionPicker::open_with(vec![".TXT".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        for c in "txt".chars() {
            handle_key(&mut picker, key(KeyCode::Char(c)));
        }
        handle_key(&mut picker, key(KeyCode::Enter));
        assert_eq!(picker.available, vec![".TXT".to_string()], "must not push a case-variant duplicate");
        assert!(picker.selected.contains(".TXT"));
    }

    #[test]
    fn enter_while_filtering_with_blank_text_adds_nothing() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        handle_key(&mut picker, key(KeyCode::Enter));
        assert!(!picker.filtering);
        assert_eq!(picker.available, vec![".txt".to_string()]);
        assert!(picker.selected.is_empty());
    }

    #[test]
    fn esc_while_filtering_stops_typing_but_keeps_the_filter_active() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string(), ".rs".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        handle_key(&mut picker, key(KeyCode::Char('r')));
        handle_key(&mut picker, key(KeyCode::Esc));
        assert!(!picker.filtering);
        assert_eq!(picker.filter_text, "r");
        assert!(picker.open, "must not close the whole picker");
    }

    #[test]
    fn esc_with_an_active_filter_clears_it_before_closing_the_picker() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string(), ".rs".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        handle_key(&mut picker, key(KeyCode::Char('r')));
        handle_key(&mut picker, key(KeyCode::Esc)); // stop typing, filter stays "r"
        assert_eq!(picker.filter_text, "r");

        handle_key(&mut picker, key(KeyCode::Esc)); // first Esc on the list: clear the filter
        assert!(picker.filter_text.is_empty());
        assert!(picker.open);

        handle_key(&mut picker, key(KeyCode::Esc)); // second Esc: no filter left, closes
        assert!(!picker.open);
    }

    #[test]
    fn backspace_while_filtering_edits_the_filter_text() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        for c in "tx".chars() {
            handle_key(&mut picker, key(KeyCode::Char(c)));
        }
        handle_key(&mut picker, key(KeyCode::Backspace));
        assert_eq!(picker.filter_text, "t");
    }

    #[test]
    fn space_toggle_operates_on_the_filtered_list_not_the_raw_index() {
        let mut picker = ExtensionPicker::open_with(vec![".rs".to_string(), ".txt".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        for c in "txt".chars() {
            handle_key(&mut picker, key(KeyCode::Char(c)));
        }
        handle_key(&mut picker, key(KeyCode::Esc)); // stop typing, filter stays "txt" -> only .txt visible
        // cursor 0 in the filtered view must map to .txt, not .rs (raw index 0).
        handle_key(&mut picker, key(KeyCode::Char(' ')));
        assert!(picker.selected.contains(".txt"));
        assert!(!picker.selected.contains(".rs"));
    }

    #[test]
    fn select_all_and_select_none_are_scoped_to_the_visible_filtered_list() {
        let mut picker = ExtensionPicker::open_with(vec![".rs".to_string(), ".txt".to_string(), ".toml".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        for c in "t".chars() {
            handle_key(&mut picker, key(KeyCode::Char(c)));
        }
        handle_key(&mut picker, key(KeyCode::Esc)); // filter "t" -> .txt and .toml visible, .rs hidden
        handle_key(&mut picker, key(KeyCode::Char('a')));
        assert_eq!(picker.selected.len(), 2);
        assert!(!picker.selected.contains(".rs"), "select-all must not reach into a filtered-out entry");

        handle_key(&mut picker, key(KeyCode::Char('n')));
        assert!(picker.selected.is_empty());
    }

    #[test]
    fn navigation_on_an_empty_list_does_not_panic() {
        let mut picker = ExtensionPicker::default();
        handle_key(&mut picker, key(KeyCode::Down));
        handle_key(&mut picker, key(KeyCode::Up));
        handle_key(&mut picker, key(KeyCode::Char(' ')));
        assert_eq!(picker.cursor, 0);
    }

    fn render_at(picker: &ExtensionPicker, width: u16, height: u16) {
        let backend = TestBackend::new(width.max(1), height.max(1));
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal
            .draw(|frame| {
                let area = Rect { x: 0, y: 0, width, height };
                render(frame, area, &Theme::default(), picker, &mut regions);
            })
            .unwrap();
    }

    #[test]
    fn render_does_not_panic_at_normal_or_degenerate_sizes() {
        let picker = ExtensionPicker::open_with(vec![".txt".to_string(), ".rs".to_string()], Some(&[".rs".to_string()]));
        render_at(&picker, 80, 24);
        render_at(&picker, 0, 0);
        render_at(&picker, 1, 1);

        let empty_picker = ExtensionPicker::default();
        render_at(&empty_picker, 80, 24);

        let error_picker = ExtensionPicker::open_with_error("path does not exist");
        render_at(&error_picker, 80, 24);
    }

    #[test]
    fn render_with_an_active_filter_does_not_panic_at_normal_or_degenerate_sizes() {
        let mut picker = ExtensionPicker::open_with(vec![".txt".to_string(), ".rs".to_string()], None);
        handle_key(&mut picker, key(KeyCode::Char('/')));
        handle_key(&mut picker, key(KeyCode::Char('z'))); // filters everything out - exercises the "no match" empty state too
        render_at(&picker, 80, 24);
        render_at(&picker, 0, 0);
        render_at(&picker, 1, 1);
    }
}
