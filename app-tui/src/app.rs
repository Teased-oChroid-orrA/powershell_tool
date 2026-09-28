//! Central application state and the single synchronous event reducer.
//! `handle_event` never touches the terminal, the async runtime, or the OS
//! directly - it only mutates `AppState` and returns `Effect`s for
//! `main.rs` to execute (spawn a search task, write a report, open a path,
//! ...). This keeps the whole decision layer testable without a real
//! terminal or tokio runtime, and gives the app exactly one mutation point
//! for state that background tasks also touch - a narrower surface than
//! either existing GUI head's own pattern (see the migration plan's Event
//! and State Model section).

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use search_core::models::{SearchRunResult, SearchSettings};
use search_core::orchestrator::OrchestratorError;

use crate::command_palette::{Command, CommandPalette};
use crate::modal::{ConfirmAction, ConfirmDialog, ModalState};
use crate::nav::{FocusArea, FocusState, NavigationState, ToolId};
use crate::notifications::NotificationQueue;
use crate::theme::{StatusTone, Theme};
use crate::toolboxes::search::{self, SearchToolState};

pub struct AppState {
    pub theme: Theme,
    pub nav: NavigationState,
    pub focus: FocusState,
    pub modal: ModalState,
    pub notifications: NotificationQueue,
    pub should_quit: bool,
    pub search: SearchToolState,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            nav: NavigationState::default(),
            focus: FocusState::default(),
            modal: ModalState::default(),
            notifications: NotificationQueue::default(),
            should_quit: false,
            search: SearchToolState::default(),
        }
    }
}

impl AppState {
    /// Number of focusable panes in the active toolbox's workspace, for
    /// Tab-cycling.
    pub fn workspace_pane_count(&self) -> u8 {
        match self.nav.active_tool {
            ToolId::Search => search::PANE_COUNT,
            _ => 0,
        }
    }

    /// A running search should ask for confirmation before Quit - the
    /// active toolbox's own running flag is the source of truth, not a
    /// separate app-level "busy" bit that could drift out of sync with it.
    pub fn is_busy(&self) -> bool {
        self.search.run.is_running
    }
}

pub enum AppEvent {
    Terminal(Event),
    Tick,
    SearchProgress(search_core::models::SearchProgressReport),
    SearchFinished(Result<SearchRunResult, OrchestratorError>),
    ReportWritten(Option<String>),
    IndexBuildProgress(search_core::native_index::CorpusIndexProgress),
    IndexBuildFinished(native_search::error::NsResult<search_core::native_index::CorpusIndexOutcome>),
    ExtensionsScanned(Result<Vec<String>, String>),
    Quit,
}

/// Side effects `handle_event` asks `main.rs` to perform - anything that
/// needs the async runtime or the outside world (spawning a search,
/// writing a report, opening a file, touching the clipboard) rather than a
/// plain state mutation. Kept as a real enum (not executed inline) so
/// `handle_event` stays a pure function callable from a unit test with no
/// terminal, no tokio runtime, and no filesystem/OS access.
#[derive(Debug)]
pub enum Effect {
    StartSearch { roots: Vec<String>, settings: SearchSettings, index: search::indexing::IndexSettings },
    /// `write_html` comes from `SearchToolConfig.export_html` - a TUI-only
    /// toggle, not a `SearchSettings` field (`SearchSettings` only carries
    /// `export_csv`/`export_json`/`open_report_when_done`; HTML export is
    /// gated purely at the UI layer in both existing heads too).
    WriteReport { settings: SearchSettings, run_result: SearchRunResult, write_html: bool },
    OpenPath(String),
    CopyToClipboard(String),
    /// Writes `contents` to `path`, creating the parent directory if
    /// needed, then opens it - used by the "export this file's hits"
    /// result action.
    WriteTextFileAndOpen { path: String, contents: String },
    PersistSearchSettings,
    BuildIndex { settings: SearchSettings, index_dir: std::path::PathBuf, force_rebuild: bool },
    ScanExtensions { root: String, exclude_folders: Vec<String>, include_hidden: bool },
}

pub fn handle_event(state: &mut AppState, event: AppEvent) -> Vec<Effect> {
    match event {
        AppEvent::Terminal(Event::Key(key)) if key.kind == KeyEventKind::Press => {
            handle_key(state, key)
        }
        AppEvent::Terminal(_) => Vec::new(),
        AppEvent::Tick => {
            state.notifications.expire();
            Vec::new()
        }
        AppEvent::SearchProgress(report) => {
            search::model::apply_progress(&mut state.search.run, report);
            Vec::new()
        }
        AppEvent::SearchFinished(result) => handle_search_finished(state, result),
        AppEvent::ReportWritten(path) => {
            state.search.run.last_report_path = path;
            Vec::new()
        }
        AppEvent::IndexBuildProgress(progress) => {
            state.search.index_run.is_building = true;
            state.search.index_run.status_text = if progress.total_files > 0 {
                format!("Indexing {} of {}: {}", progress.files_processed, progress.total_files, progress.current_file)
            } else {
                progress.current_file
            };
            Vec::new()
        }
        AppEvent::IndexBuildFinished(result) => {
            state.search.index_run.is_building = false;
            match result {
                Ok(outcome) => {
                    state.search.index_run.status_text = format!(
                        "Indexed {} file(s) ({} skipped, {} failed)",
                        outcome.indexed_count, outcome.skipped_count, outcome.failed_count
                    );
                    state.search.index_run.last_error = None;
                    state.notifications.push("Index build finished", StatusTone::Success);
                }
                Err(e) => {
                    state.search.index_run.last_error = Some(e.to_string());
                    state.notifications.push("Index build failed", StatusTone::Danger);
                }
            }
            Vec::new()
        }
        AppEvent::ExtensionsScanned(result) => {
            state.search.extension_picker = match result {
                Ok(available) => {
                    search::extension_picker::ExtensionPicker::open_with(available, state.search.config.selected_extensions.as_deref())
                }
                Err(e) => search::extension_picker::ExtensionPicker::open_with_error(e),
            };
            Vec::new()
        }
        AppEvent::Quit => {
            request_quit(state);
            Vec::new()
        }
    }
}

fn handle_search_finished(state: &mut AppState, result: Result<SearchRunResult, OrchestratorError>) -> Vec<Effect> {
    state.search.run.is_running = false;
    state.search.run.in_flight_files.clear();
    state.search.cancel_token = None;

    match result {
        Ok(run_result) => {
            state.search.run.results_summary_text = search::model::summarize(&run_result.summary);
            state.notifications.push("Search finished", StatusTone::Success);
            let write_html = state.search.config.export_html;
            let settings = search::model::build_settings(&state.search.config);
            vec![Effect::WriteReport { settings, run_result, write_html }]
        }
        Err(OrchestratorError::Cancelled) => {
            state.search.run.status_text = "Cancelled.".to_string();
            state.notifications.push("Search cancelled", StatusTone::Info);
            Vec::new()
        }
        Err(e) => {
            state.search.run.status_text = format!("Error: {e}");
            state.notifications.push("Search failed", StatusTone::Danger);
            Vec::new()
        }
    }
}

fn request_quit(state: &mut AppState) {
    if state.is_busy() {
        state.modal = ModalState::Confirm(ConfirmDialog {
            title: "Quit?".into(),
            message: "A search is still running. Quit anyway?".into(),
            on_confirm: ConfirmAction::Quit,
        });
    } else {
        state.should_quit = true;
    }
}

fn handle_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    // Modal input takes priority over every global binding while open.
    if state.modal.is_open() {
        return handle_modal_key(state, key);
    }

    // Toolbox-local key routing (text-field editing, list navigation)
    // takes priority over global bindings while a workspace pane is
    // focused, so e.g. typing "quit" into the search path field never
    // triggers the global `q` quit binding. See `toolboxes::search::handle_key`.
    if let FocusArea::Workspace(pane) = state.focus.area {
        if state.nav.active_tool == ToolId::Search {
            let (consumed, effects) =
                search::handle_key(&mut state.search, &mut state.notifications, pane, key);
            if consumed {
                return effects;
            }
        }
    }

    match key.code {
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.modal = ModalState::Palette(CommandPalette::default());
        }
        KeyCode::Char('?') => state.modal = ModalState::Help,
        KeyCode::Char('q') => request_quit(state),
        KeyCode::Tab => {
            let panes = state.workspace_pane_count();
            state.focus.cycle_forward(panes);
        }
        KeyCode::BackTab => {
            let panes = state.workspace_pane_count();
            state.focus.cycle_backward(panes);
        }
        KeyCode::Left | KeyCode::Right if state.focus.area == FocusArea::Rail => {
            let idx = ToolId::ALL.iter().position(|t| *t == state.nav.active_tool).unwrap_or(0);
            let len = ToolId::ALL.len();
            let next = if key.code == KeyCode::Left {
                (idx + len - 1) % len
            } else {
                (idx + 1) % len
            };
            state.nav.activate(ToolId::ALL[next]);
        }
        _ => {}
    }
    Vec::new()
}

fn handle_modal_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match &mut state.modal {
        ModalState::Palette(palette) => match key.code {
            KeyCode::Esc => state.modal.close(),
            KeyCode::Enter => {
                let picked = palette.picked();
                state.modal.close();
                if let Some(cmd) = picked {
                    return execute_command(state, cmd);
                }
            }
            KeyCode::Up => palette.move_selection(-1),
            KeyCode::Down => palette.move_selection(1),
            KeyCode::Backspace => palette.backspace(),
            KeyCode::Char(c) => palette.push_char(c),
            _ => {}
        },
        ModalState::Help => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                state.modal.close();
            }
        }
        ModalState::Confirm(dialog) => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let action = dialog.on_confirm;
                state.modal.close();
                match action {
                    ConfirmAction::Quit => state.should_quit = true,
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => state.modal.close(),
            _ => {}
        },
        ModalState::None => {}
    }
    Vec::new()
}

fn execute_command(state: &mut AppState, cmd: Command) -> Vec<Effect> {
    match cmd {
        Command::SwitchToSearch => {
            state.nav.activate(ToolId::Search);
            Vec::new()
        }
        Command::SwitchToBushing => {
            state.nav.activate(ToolId::Bushing);
            Vec::new()
        }
        Command::SwitchToPressureVessel => {
            state.nav.activate(ToolId::PressureVessel);
            Vec::new()
        }
        Command::SwitchToDupes => {
            state.nav.activate(ToolId::Dupes);
            Vec::new()
        }
        Command::SwitchToRename => {
            state.nav.activate(ToolId::Rename);
            Vec::new()
        }
        Command::SwitchToLogs => {
            state.nav.activate(ToolId::Logs);
            Vec::new()
        }
        Command::ToggleTheme => {
            state.theme = if state.theme.reduced_color {
                Theme::default_palette()
            } else {
                Theme::reduced_palette()
            };
            Vec::new()
        }
        Command::RunSearch => {
            state.search.screen = search::ToolboxScreen::Run;
            search::run_or_notify(&mut state.search, &mut state.notifications)
        }
        Command::CancelSearch => {
            state.search.screen = search::ToolboxScreen::Run;
            search::request_cancel(&state.search);
            Vec::new()
        }
        Command::OpenReport => match state.search.run.last_report_path.clone() {
            Some(path) => vec![Effect::OpenPath(path)],
            None => {
                state.notifications.push("No report yet", StatusTone::Info);
                Vec::new()
            }
        },
        Command::FocusPathField => {
            state.nav.activate(ToolId::Search);
            state.search.screen = search::ToolboxScreen::Run;
            state.focus.area = FocusArea::Workspace(search::PANE_PATH);
            Vec::new()
        }
        Command::ClearRecentSearches => {
            state.search.recent_searches.clear();
            vec![Effect::PersistSearchSettings]
        }
        Command::ToggleFastReSearchIndex => {
            state.search.config.index.enabled = !state.search.config.index.enabled;
            let tone = if state.search.config.index.enabled { StatusTone::Success } else { StatusTone::Info };
            let message = if state.search.config.index.enabled { "Fast re-search index enabled" } else { "Fast re-search index disabled" };
            state.notifications.push(message, tone);
            Vec::new()
        }
        Command::BuildIndex => search::start_index_build(&mut state.search, false),
        Command::RebuildIndex => search::start_index_build(&mut state.search, true),
        Command::Quit => {
            request_quit(state);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn press(code: KeyCode) -> AppEvent {
        AppEvent::Terminal(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn press_with(code: KeyCode, modifiers: KeyModifiers) -> AppEvent {
        let mut key = KeyEvent::new(code, modifiers);
        key.kind = KeyEventKind::Press;
        AppEvent::Terminal(Event::Key(key))
    }

    #[test]
    fn ctrl_p_opens_palette_and_esc_closes_it() {
        let mut state = AppState::default();
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(matches!(state.modal, ModalState::Palette(_)));
        handle_event(&mut state, press(KeyCode::Esc));
        assert!(!state.modal.is_open());
    }

    #[test]
    fn q_quits_immediately_when_not_busy() {
        let mut state = AppState::default();
        handle_event(&mut state, press(KeyCode::Char('q')));
        assert!(state.should_quit);
    }

    #[test]
    fn q_asks_for_confirmation_when_a_search_is_running() {
        let mut state = AppState::default();
        state.search.run.is_running = true;
        handle_event(&mut state, press(KeyCode::Char('q')));
        assert!(!state.should_quit);
        assert!(matches!(state.modal, ModalState::Confirm(_)));
        handle_event(&mut state, press(KeyCode::Char('y')));
        assert!(state.should_quit);
    }

    #[test]
    fn confirm_dialog_n_cancels_without_quitting() {
        let mut state = AppState::default();
        state.search.run.is_running = true;
        handle_event(&mut state, press(KeyCode::Char('q')));
        handle_event(&mut state, press(KeyCode::Char('n')));
        assert!(!state.should_quit);
        assert!(!state.modal.is_open());
    }

    #[test]
    fn help_toggles_open_and_closed() {
        let mut state = AppState::default();
        handle_event(&mut state, press(KeyCode::Char('?')));
        assert!(matches!(state.modal, ModalState::Help));
        handle_event(&mut state, press(KeyCode::Char('?')));
        assert!(!state.modal.is_open());
    }

    #[test]
    fn picking_switch_to_search_from_palette_activates_it() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        for c in "switch to: search".chars() {
            handle_event(&mut state, press(KeyCode::Char(c)));
        }
        handle_event(&mut state, press(KeyCode::Enter));
        assert!(!state.modal.is_open());
        assert_eq!(state.nav.active_tool, ToolId::Search);
    }

    #[test]
    fn run_search_command_without_a_path_shows_a_warning_toast_not_a_crash() {
        let mut state = AppState::default();
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        for c in "run search".chars() {
            handle_event(&mut state, press(KeyCode::Char(c)));
        }
        let effects = handle_event(&mut state, press(KeyCode::Enter));
        assert!(effects.is_empty());
        assert_eq!(state.notifications.visible().len(), 1);
    }

    #[test]
    fn typing_a_path_never_triggers_the_global_quit_binding() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        state.focus.area = FocusArea::Workspace(search::PANE_PATH);
        for c in "quit".chars() {
            handle_event(&mut state, press(KeyCode::Char(c)));
        }
        assert!(!state.should_quit);
        assert_eq!(state.search.config.search_path, "quit");
    }

    #[test]
    fn enter_on_path_field_with_a_path_starts_a_search() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        state.focus.area = FocusArea::Workspace(search::PANE_PATH);
        state.search.config.search_path = "/tmp".to_string();
        let effects = handle_event(&mut state, press(KeyCode::Enter));
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::StartSearch { .. }));
        assert!(state.search.run.is_running);
    }

    #[test]
    fn ctrl_p_still_opens_the_palette_while_a_text_field_is_focused() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        state.focus.area = FocusArea::Workspace(search::PANE_PATH);
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(matches!(state.modal, ModalState::Palette(_)));
        // And it must not have leaked a stray "p" into the path field.
        assert!(state.search.config.search_path.is_empty());
    }

    #[test]
    fn search_progress_event_updates_run_state() {
        let mut state = AppState::default();
        handle_event(
            &mut state,
            AppEvent::SearchProgress(search_core::models::SearchProgressReport {
                files_completed: 1,
                total_files: 2,
                ..Default::default()
            }),
        );
        assert_eq!(state.search.run.progress_percent, 50.0);
    }

    #[test]
    fn search_finished_ok_clears_running_and_requests_a_report_write() {
        let mut state = AppState::default();
        state.search.run.is_running = true;
        let effects = handle_event(&mut state, AppEvent::SearchFinished(Ok(SearchRunResult::default())));
        assert!(!state.search.run.is_running);
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::WriteReport { .. }));
    }

    #[test]
    fn search_finished_cancelled_does_not_request_a_report_write() {
        let mut state = AppState::default();
        state.search.run.is_running = true;
        let effects = handle_event(&mut state, AppEvent::SearchFinished(Err(OrchestratorError::Cancelled)));
        assert!(!state.search.run.is_running);
        assert!(effects.is_empty());
        assert_eq!(state.search.run.status_text, "Cancelled.");
    }

    #[test]
    fn extensions_scanned_ok_opens_the_picker_with_the_results() {
        let mut state = AppState::default();
        let effects = handle_event(&mut state, AppEvent::ExtensionsScanned(Ok(vec![".txt".to_string(), ".rs".to_string()])));
        assert!(effects.is_empty());
        assert!(state.search.extension_picker.open);
        assert_eq!(state.search.extension_picker.available, vec![".txt".to_string(), ".rs".to_string()]);
    }

    #[test]
    fn extensions_scanned_err_opens_the_picker_in_an_error_state() {
        let mut state = AppState::default();
        handle_event(&mut state, AppEvent::ExtensionsScanned(Err("not a directory".to_string())));
        assert!(state.search.extension_picker.open);
        assert_eq!(state.search.extension_picker.scan_error.as_deref(), Some("not a directory"));
    }

    #[test]
    fn toggle_fast_reindex_command_flips_the_setting() {
        let mut state = AppState::default();
        assert!(!state.search.config.index.enabled);
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        for c in "toggle fast".chars() {
            handle_event(&mut state, press(KeyCode::Char(c)));
        }
        handle_event(&mut state, press(KeyCode::Enter));
        assert!(state.search.config.index.enabled);
    }

    #[test]
    fn index_build_progress_and_finished_events_update_index_run_state() {
        let mut state = AppState::default();
        handle_event(
            &mut state,
            AppEvent::IndexBuildProgress(search_core::native_index::CorpusIndexProgress {
                files_processed: 3,
                total_files: 10,
                current_file: "a.txt".to_string(),
            }),
        );
        assert!(state.search.index_run.is_building);
        assert!(state.search.index_run.status_text.contains("3 of 10"));

        handle_event(
            &mut state,
            AppEvent::IndexBuildFinished(Ok(search_core::native_index::CorpusIndexOutcome {
                indexed_count: 10,
                skipped_count: 0,
                failed_count: 0,
                failed_files: Vec::new(),
            })),
        );
        assert!(!state.search.index_run.is_building);
        assert!(state.search.index_run.last_error.is_none());
    }
}
