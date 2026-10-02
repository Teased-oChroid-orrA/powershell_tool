//! Search Files toolbox integration: wires the pure `model`/`persistence`
//! modules to the shell's event/render loop.
//!
//! Toolbox-local keyboard routing (text-field editing, result-list
//! navigation) lives here rather than behind a generic `Toolbox` trait -
//! per the migration plan's "don't abstract prematurely" rule, that
//! abstraction is worth adding once a second real toolbox needs the same
//! treatment, not before.

pub mod extension_picker;
pub mod index_view;
pub mod indexing;
pub mod model;
pub mod persistence;
pub mod preview;
pub mod run_view;
pub mod runner;
pub mod settings_view;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio_util::sync::CancellationToken;

use crate::app::Effect;
use crate::notifications::NotificationQueue;
use crate::theme::StatusTone;
use model::{SearchRunState, SearchToolConfig};
use persistence::{PersistedFile, PersistedSearchSettings, RecentSearch, SavedPreset};
use settings_view::SettingsView;

/// Which screen the toolbox's workspace shows - a compact live-run view by
/// default, or the full settings form (`s` to enter, `Esc` to leave).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolboxScreen {
    #[default]
    Run,
    Settings,
}

/// Workspace pane indices for `FocusState`/Tab-cycling. Small, toolbox-local
/// constants rather than an enum shared with the shell, since only this
/// module needs to interpret them.
pub const PANE_PATH: u8 = 0;
pub const PANE_FILTERS: u8 = 1;
pub const PANE_RESULTS: u8 = 2;
pub const PANE_COUNT: u8 = 3;

pub struct SearchToolState {
    pub config: SearchToolConfig,
    pub run: SearchRunState,
    pub recent_searches: Vec<RecentSearch>,
    pub saved_presets: Vec<SavedPreset>,
    /// Most-recently-selected extensions from the extension picker,
    /// independent of `config.selected_extensions` (the current run's
    /// active filter) - see `persistence::remember_recent_extensions`.
    pub recent_extensions: Vec<String>,
    pub cancel_token: Option<CancellationToken>,
    pub selected_result: usize,
    pub screen: ToolboxScreen,
    pub settings: SettingsView,
    pub index_run: indexing::IndexRunState,
    pub index_cancel: Option<CancellationToken>,
    pub extension_picker: extension_picker::ExtensionPicker,
}

impl Default for SearchToolState {
    fn default() -> Self {
        // `mut` is only exercised by the `cfg(not(test))` block below -
        // harmless "unused mut" in test builds, where that block is
        // compiled out entirely.
        #[cfg_attr(test, allow(unused_mut))]
        let mut config = SearchToolConfig::default();
        #[cfg_attr(test, allow(unused_mut))]
        let mut recent_searches = Vec::new();
        #[cfg_attr(test, allow(unused_mut))]
        let mut saved_presets = Vec::new();
        #[cfg_attr(test, allow(unused_mut))]
        let mut recent_extensions = Vec::new();
        // Never read the real on-disk settings file from a test build - a
        // dev machine's actual `settings-tui.json` (written by a real,
        // interactive run) would otherwise leak into `cargo test`, making
        // "default = empty" a false assumption every test here relies on.
        // Production `main.rs` gets the real load via this same `Default`
        // impl, unaffected - `cfg(test)` only applies within this crate's
        // own test builds.
        #[cfg(not(test))]
        if let Some(file) = persistence::load() {
            if let Some(settings) = &file.settings {
                settings.apply_to(&mut config);
            }
            recent_searches = file.recent_searches;
            saved_presets = file.saved_presets;
            recent_extensions = file.recent_extensions;
        }
        Self {
            config,
            run: SearchRunState::default(),
            recent_searches,
            saved_presets,
            recent_extensions,
            cancel_token: None,
            selected_result: 0,
            screen: ToolboxScreen::default(),
            settings: SettingsView::default(),
            index_run: indexing::IndexRunState::default(),
            index_cancel: None,
            extension_picker: extension_picker::ExtensionPicker::default(),
        }
    }
}

impl SearchToolState {
    pub fn can_run(&self) -> bool {
        !self.run.is_running && !self.config.search_path.trim().is_empty()
    }

    /// Snapshot suitable for `persistence::save` - called on quit.
    pub fn to_persisted_file(&self) -> PersistedFile {
        PersistedFile {
            settings: Some(PersistedSearchSettings::from(&self.config)),
            recent_searches: self.recent_searches.clone(),
            saved_presets: self.saved_presets.clone(),
            recent_extensions: self.recent_extensions.clone(),
        }
    }

    fn roots(&self) -> Vec<String> {
        let mut roots = vec![self.config.search_path.trim().to_string()];
        roots.extend(
            self.config
                .search_paths_extra
                .iter()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty()),
        );
        roots
    }

    pub fn move_selection(&mut self, delta: i32) {
        let len = self.run.results.len();
        if len == 0 {
            self.selected_result = 0;
            return;
        }
        let current = self.selected_result as i32;
        self.selected_result = (current + delta).rem_euclid(len as i32) as usize;
    }
}

/// Starts a run if possible: builds settings, records the recent search,
/// resets live-run state, and returns the `Effect` `main.rs` executes to
/// actually spawn the background task. Never touches the async runtime
/// itself - see the plan's Event and State Model section on why
/// `handle_event`'s whole call graph must stay synchronous.
pub fn start_run(state: &mut SearchToolState) -> Vec<Effect> {
    if !state.can_run() {
        return Vec::new();
    }
    let settings = model::build_settings(&state.config);
    let roots = state.roots();
    persistence::remember_recent_search(
        &mut state.recent_searches,
        state.config.search_path.clone(),
        state.config.filters_text.clone(),
    );
    state.run = SearchRunState { is_running: true, started: Some(std::time::Instant::now()), ..SearchRunState::default() };
    state.cancel_token = None;
    vec![Effect::StartSearch { roots, settings, index: state.config.index }]
}

pub fn request_cancel(state: &SearchToolState) {
    if let Some(token) = &state.cancel_token {
        token.cancel();
    }
}

/// Starts a fast re-search index build/rebuild if one isn't already running
/// and a search path is set. Mirrors `start_run`'s shape: builds the
/// `SearchSettings`/index directory, marks live state, and returns the
/// `Effect` `main.rs` executes - `handle_event`'s call graph never touches
/// the async runtime directly.
pub fn start_index_build(state: &mut SearchToolState) -> Vec<Effect> {
    if state.index_run.is_building || state.config.search_path.trim().is_empty() {
        return Vec::new();
    }
    let settings = model::build_settings(&state.config);
    let index_dir =
        indexing::index_directory(state.config.index.location, &state.config.search_path, &state.config.output_folder);
    state.index_run.begin();
    let cancel = CancellationToken::new();
    state.index_cancel = Some(cancel.clone());
    vec![Effect::BuildIndex { settings, index_dir, cancel }]
}

/// Toolbox-local key routing for whichever workspace pane currently has
/// focus. Returns `(consumed, effects)` - `consumed = false` lets
/// `app.rs`'s global bindings (Ctrl+P, `?`, Tab, ...) still apply, which is
/// exactly how Ctrl+P keeps opening the command palette even while a text
/// field has focus (a plain `Char('p')` arm here would otherwise steal it).
pub fn handle_key(
    state: &mut SearchToolState,
    notifications: &mut NotificationQueue,
    pane: u8,
    key: KeyEvent,
) -> (bool, Vec<Effect>) {
    // The extension picker is a toolbox-local overlay (not a global
    // `ModalState`, since only this toolbox has one) - it takes priority
    // over everything else while open, same as a modal would.
    if state.extension_picker.open {
        let consumed = extension_picker::handle_key(&mut state.extension_picker, key);
        if consumed && !state.extension_picker.open {
            let mut selected: Vec<String> = state.extension_picker.selected.iter().cloned().collect();
            selected.sort();
            persistence::remember_recent_extensions(&mut state.recent_extensions, &selected);
            state.config.selected_extensions = if selected.is_empty() { None } else { Some(selected.clone()) };
            return (consumed, if selected.is_empty() { Vec::new() } else { vec![Effect::PersistSearchSettings] });
        }
        return (consumed, Vec::new());
    }

    if state.screen == ToolboxScreen::Settings {
        // Esc backs out of the whole Settings screen only when nothing is
        // being edited/named/renamed right now - otherwise Esc means
        // "cancel this field edit / preset name" instead (handled inside
        // `settings_view::handle_key`).
        if key.code == KeyCode::Esc
            && !state.settings.editing
            && !state.settings.naming_preset
            && !state.settings.renaming_preset
        {
            state.screen = ToolboxScreen::Run;
            return (true, Vec::new());
        }
        return settings_view::handle_key(
            &mut state.settings,
            &mut state.config,
            &state.recent_searches,
            &mut state.saved_presets,
            key,
        );
    }

    match pane {
        PANE_PATH => match key.code {
            KeyCode::Enter => (true, run_or_notify(state, notifications)),
            _ => match edit_buffer_key(&mut state.config.search_path, key) {
                Some(consumed) => (consumed, Vec::new()),
                None => (false, Vec::new()),
            },
        },
        PANE_FILTERS => match key.code {
            KeyCode::Enter => (true, run_or_notify(state, notifications)),
            _ => match edit_buffer_key(&mut state.config.filters_text, key) {
                Some(consumed) => (consumed, Vec::new()),
                None => (false, Vec::new()),
            },
        },
        PANE_RESULTS => match key.code {
            KeyCode::Up => {
                state.move_selection(-1);
                (true, Vec::new())
            }
            KeyCode::Down => {
                state.move_selection(1);
                (true, Vec::new())
            }
            KeyCode::Char('c' | 'C') if state.run.is_running && is_plain_char(key) => {
                request_cancel(state);
                (true, Vec::new())
            }
            KeyCode::Char('s' | 'S') if is_plain_char(key) => {
                state.screen = ToolboxScreen::Settings;
                (true, Vec::new())
            }
            KeyCode::Enter => match state.run.results.get(state.selected_result) {
                Some(result) => (true, vec![Effect::OpenPath(result.full_name.clone())]),
                None => (true, Vec::new()),
            },
            KeyCode::Char('y' | 'Y') if is_plain_char(key) => match state.run.results.get(state.selected_result) {
                Some(result) => (true, vec![Effect::CopyToClipboard(result.full_name.clone())]),
                None => (true, Vec::new()),
            },
            // Reveal containing folder - this app has no "reveal AND
            // select the file" primitive available in a terminal any more
            // than `app/`'s own `open::that(parent)` did on the desktop
            // (see the plan's parity notes); it just opens the parent
            // folder in the OS default handler.
            KeyCode::Char('r' | 'R') if is_plain_char(key) => match state.run.results.get(state.selected_result) {
                Some(result) => {
                    let parent = std::path::Path::new(&result.full_name)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned());
                    match parent {
                        Some(dir) => (true, vec![Effect::OpenPath(dir)]),
                        None => (true, Vec::new()),
                    }
                }
                None => (true, Vec::new()),
            },
            // Export this file's hits to a standalone text file, then open
            // it - ported behavior from `app/`'s per-row "Export hits"
            // action (`hits_as_text`), now a keybinding since there's no
            // mouse/context-menu in this phase.
            KeyCode::Char('e' | 'E') if is_plain_char(key) => match state.run.results.get(state.selected_result) {
                Some(result) => {
                    let contents = model::hits_as_text(result);
                    let stem = std::path::Path::new(&result.full_name)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "result".to_string());
                    let file_name = format!("{}_hits.txt", model::sanitize_file_name(&stem));
                    let output_dir = if state.config.output_folder.trim().is_empty() {
                        std::path::Path::new(&result.full_name).parent().map(|p| p.to_path_buf()).unwrap_or_default()
                    } else {
                        std::path::PathBuf::from(state.config.output_folder.trim())
                    };
                    let path = output_dir.join(file_name).to_string_lossy().into_owned();
                    (true, vec![Effect::WriteTextFileAndOpen { path, contents }])
                }
                None => (true, Vec::new()),
            },
            _ => (false, Vec::new()),
        },
        _ => (false, Vec::new()),
    }
}

/// True for a character keypress with no Ctrl/Alt held. Shift alone must
/// still count as "plain" - crossterm reports Shift+s (or a physical key
/// typed with Caps Lock on) as `Char('S')` with `KeyModifiers::SHIFT` set,
/// not as `Char('s')` with empty modifiers, so a bare `.is_empty()` guard
/// silently drops every single-key shortcut whenever Shift/Caps Lock is
/// involved. Ctrl/Alt combinations are still excluded so e.g. Ctrl+P keeps
/// reaching the global command-palette binding instead of being consumed
/// here.
pub(crate) fn is_plain_char(key: KeyEvent) -> bool {
    key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT
}

/// Shared "plain character edits this buffer, everything else falls
/// through to global bindings" behavior for the two single-line text
/// fields (Enter is handled by the caller, since it submits rather than
/// edits). `Ctrl`/`Alt`-modified characters are deliberately NOT consumed
/// here (the modifiers guard) so e.g. Ctrl+P still reaches the global
/// command-palette binding instead of inserting a literal "p" into
/// whatever field has focus.
fn edit_buffer_key(buffer: &mut String, key: KeyEvent) -> Option<bool> {
    match key.code {
        KeyCode::Backspace => {
            buffer.pop();
            Some(true)
        }
        // Clears the whole buffer in one press ("start fresh") - same
        // convention every other text/number buffer in this crate now
        // follows (see `widgets/number_edit.rs`'s own doc comment).
        KeyCode::Delete => {
            buffer.clear();
            Some(true)
        }
        KeyCode::Char(c) if is_plain_char(key) => {
            buffer.push(c);
            Some(true)
        }
        _ => None,
    }
}

/// Starts a run if possible, otherwise pushes a warning toast explaining
/// why not - the single entry point both the Path/Filters field's Enter
/// key and the command palette's "Run search" command go through, so the
/// two invocation paths can't drift into inconsistent failure behavior.
pub fn run_or_notify(state: &mut SearchToolState, notifications: &mut NotificationQueue) -> Vec<Effect> {
    if state.can_run() && state.config.index.enabled && state.index_run.is_building {
        // A search would try to build the same index concurrently (one
        // Tantivy writer per index folder).
        notifications.push("Fast index is still building - wait for it or stop it (Ctrl+P)", StatusTone::Warning);
        Vec::new()
    } else if state.can_run() {
        start_run(state)
    } else {
        notifications.push("Enter a search path first", StatusTone::Warning);
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::CONTROL, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn shift_key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::SHIFT, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    // Regression: crossterm reports Shift+s (or a physical `s` typed with
    // Caps Lock on) as `Char('S')` with `SHIFT` set, not `Char('s')` with
    // empty modifiers. `s`/`S` must both open Settings from the Results pane.
    #[test]
    fn shift_or_caps_s_opens_settings_same_as_plain_s() {
        let mut state = SearchToolState::default();
        let mut notifications = NotificationQueue::default();
        let (consumed, _) = handle_key(&mut state, &mut notifications, PANE_RESULTS, shift_key(KeyCode::Char('S')));
        assert!(consumed);
        assert_eq!(state.screen, ToolboxScreen::Settings);
    }

    #[test]
    fn closing_the_extension_picker_with_a_selection_remembers_it_and_persists() {
        let mut state = SearchToolState::default();
        let mut notifications = NotificationQueue::default();
        state.extension_picker = extension_picker::ExtensionPicker::open_with(vec![".txt".to_string()], None);
        // Space selects the sole visible row, Enter closes the picker.
        handle_key(&mut state, &mut notifications, PANE_PATH, key(KeyCode::Char(' ')));
        let (consumed, effects) = handle_key(&mut state, &mut notifications, PANE_PATH, key(KeyCode::Enter));
        assert!(consumed);
        assert!(!state.extension_picker.open);
        assert_eq!(state.recent_extensions, vec![".txt".to_string()]);
        assert!(matches!(effects.as_slice(), [Effect::PersistSearchSettings]));
    }

    #[test]
    fn closing_the_extension_picker_with_nothing_selected_does_not_persist() {
        let mut state = SearchToolState::default();
        let mut notifications = NotificationQueue::default();
        state.extension_picker = extension_picker::ExtensionPicker::open_with(vec![".txt".to_string()], None);
        let (consumed, effects) = handle_key(&mut state, &mut notifications, PANE_PATH, key(KeyCode::Enter));
        assert!(consumed);
        assert!(effects.is_empty());
        assert!(state.recent_extensions.is_empty());
    }

    #[test]
    fn typing_into_path_field_is_consumed_and_not_a_control_char() {
        let mut state = SearchToolState::default();
        let mut notifications = NotificationQueue::default();
        let (consumed, _) = handle_key(&mut state, &mut notifications, PANE_PATH, key(KeyCode::Char('a')));
        assert!(consumed);
        assert_eq!(state.config.search_path, "a");
    }

    #[test]
    fn ctrl_modified_char_is_not_consumed_by_a_text_field() {
        let mut state = SearchToolState::default();
        let mut notifications = NotificationQueue::default();
        let (consumed, _) = handle_key(&mut state, &mut notifications, PANE_PATH, ctrl_key(KeyCode::Char('p')));
        assert!(!consumed);
        assert!(state.config.search_path.is_empty());
    }

    #[test]
    fn enter_with_no_path_notifies_instead_of_starting() {
        let mut state = SearchToolState::default();
        let mut notifications = NotificationQueue::default();
        let (consumed, effects) = handle_key(&mut state, &mut notifications, PANE_PATH, key(KeyCode::Enter));
        assert!(consumed);
        assert!(effects.is_empty());
        assert_eq!(notifications.visible().len(), 1);
    }

    #[test]
    fn enter_with_a_path_starts_a_run() {
        let mut state = SearchToolState::default();
        state.config.search_path = "/tmp".to_string();
        let mut notifications = NotificationQueue::default();
        let (consumed, effects) = handle_key(&mut state, &mut notifications, PANE_PATH, key(KeyCode::Enter));
        assert!(consumed);
        assert_eq!(effects.len(), 1);
        assert!(state.run.is_running);
    }

    #[test]
    fn cancel_key_is_ignored_when_nothing_is_running() {
        let mut state = SearchToolState::default();
        let mut notifications = NotificationQueue::default();
        let (consumed, _) = handle_key(&mut state, &mut notifications, PANE_RESULTS, key(KeyCode::Char('c')));
        assert!(!consumed);
    }

    #[test]
    fn reveal_action_opens_the_parent_directory() {
        let mut state = SearchToolState::default();
        state.run.results = vec![dummy_result("/tmp/project/notes.txt")];
        let mut notifications = NotificationQueue::default();
        let (consumed, effects) = handle_key(&mut state, &mut notifications, PANE_RESULTS, key(KeyCode::Char('r')));
        assert!(consumed);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::OpenPath(path) => assert_eq!(path, "/tmp/project"),
            other => panic!("expected OpenPath, got {other:?}"),
        }
    }

    #[test]
    fn export_action_writes_hits_and_opens_the_file() {
        let mut state = SearchToolState::default();
        state.config.output_folder = "/tmp/reports".to_string();
        state.run.results = vec![dummy_result("/tmp/project/notes.txt")];
        let mut notifications = NotificationQueue::default();
        let (consumed, effects) = handle_key(&mut state, &mut notifications, PANE_RESULTS, key(KeyCode::Char('e')));
        assert!(consumed);
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::WriteTextFileAndOpen { path, contents } => {
                assert_eq!(path, "/tmp/reports/notes_hits.txt");
                assert!(contents.starts_with("/tmp/project/notes.txt"));
            }
            other => panic!("expected WriteTextFileAndOpen, got {other:?}"),
        }
    }

    #[test]
    fn extension_picker_routes_input_before_anything_else_and_commits_on_close() {
        let mut state = SearchToolState::default();
        let mut notifications = NotificationQueue::default();
        state.extension_picker = extension_picker::ExtensionPicker::open_with(
            vec![".txt".to_string(), ".md".to_string(), ".rs".to_string()],
            None,
        );

        // Move to ".md" and toggle it on.
        handle_key(&mut state, &mut notifications, PANE_RESULTS, key(KeyCode::Down));
        let (consumed, effects) = handle_key(&mut state, &mut notifications, PANE_RESULTS, key(KeyCode::Char(' ')));
        assert!(consumed);
        assert!(effects.is_empty());
        assert!(state.extension_picker.open, "still open after a toggle");

        // Closing commits the selection into config.selected_extensions -
        // note the key code (Enter) would otherwise mean "open selected
        // result" on PANE_RESULTS, proving the picker really does take
        // priority over normal pane routing while open.
        let (consumed, _) = handle_key(&mut state, &mut notifications, PANE_RESULTS, key(KeyCode::Enter));
        assert!(consumed);
        assert!(!state.extension_picker.open);
        assert_eq!(state.config.selected_extensions, Some(vec![".md".to_string()]));
    }

    #[test]
    fn extension_picker_commit_of_an_empty_selection_falls_back_to_default_catalog() {
        let mut state = SearchToolState::default();
        let mut notifications = NotificationQueue::default();
        state.config.selected_extensions = Some(vec![".txt".to_string()]);
        state.extension_picker = extension_picker::ExtensionPicker::open_with(vec![".txt".to_string()], Some(&[".txt".to_string()]));

        // Deselect the only (pre-selected) entry, then close.
        handle_key(&mut state, &mut notifications, PANE_RESULTS, key(KeyCode::Char(' ')));
        handle_key(&mut state, &mut notifications, PANE_RESULTS, key(KeyCode::Esc));

        assert_eq!(state.config.selected_extensions, None);
    }

    #[test]
    fn start_index_build_marks_building_and_returns_an_effect() {
        let mut state = SearchToolState::default();
        state.config.search_path = "/tmp/project".to_string();
        let effects = start_index_build(&mut state);
        assert!(state.index_run.is_building);
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::BuildIndex { .. }));
    }

    #[test]
    fn start_index_build_is_a_no_op_without_a_search_path() {
        let mut state = SearchToolState::default();
        let effects = start_index_build(&mut state);
        assert!(!state.index_run.is_building);
        assert!(effects.is_empty());
    }

    #[test]
    fn start_index_build_is_a_no_op_while_already_building() {
        let mut state = SearchToolState::default();
        state.config.search_path = "/tmp/project".to_string();
        state.index_run.is_building = true;
        let effects = start_index_build(&mut state);
        assert!(effects.is_empty());
    }

    #[test]
    fn move_selection_wraps_within_results() {
        let mut state = SearchToolState::default();
        state.run.results = vec![
            dummy_result("a"),
            dummy_result("b"),
            dummy_result("c"),
        ];
        state.move_selection(-1);
        assert_eq!(state.selected_result, 2);
        state.move_selection(1);
        assert_eq!(state.selected_result, 0);
    }

    fn dummy_result(name: &str) -> search_core::models::FileSearchResult {
        search_core::models::FileSearchResult {
            full_name: name.to_string(),
            status: search_core::models::FileSearchStatus::Hit,
            hits: Vec::new(),
            created: chrono::Local::now(),
            modified: chrono::Local::now(),
            file_length: 0,
            lines_cache: Vec::new(),
            total_line_count: 0,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }
    }
}
