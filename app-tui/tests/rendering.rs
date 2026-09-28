//! Rendering/resize regression tests using ratatui's `TestBackend`.
//!
//! These exercise the actual render path (`widgets::shell::draw`,
//! `toolboxes::search::run_view::draw`) at a range of terminal sizes,
//! including degenerate ones, per the migration plan's "graceful
//! degradation at small terminal sizes, never a panic" requirement. Pure
//! unit tests already cover the underlying state/logic (`nav`, `app`,
//! `toolboxes::search::model`, etc.) - this file is specifically about
//! whether rendering itself can panic, which those tests can't see.

use chrono::Local;
use crossterm::event::Event;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use app_tui::app::{handle_event, AppEvent, AppState};
use app_tui::command_palette::CommandPalette;
use app_tui::modal::{ConfirmAction, ConfirmDialog, ModalState};
use app_tui::toolboxes::search::model::SearchRunState;
use app_tui::toolboxes::search::{run_view, SearchToolState, PANE_RESULTS};
use app_tui::widgets::shell;

/// Flattens a rendered buffer into one string (rows joined by '\n') so
/// tests can assert on visible text content, not just "didn't panic".
fn buffer_text(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buffer[(x + area.x, y + area.y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn render_shell(state: &AppState, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width.max(1), height.max(1));
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| shell::draw(frame, state, 0)).unwrap();
    terminal.backend().buffer().clone()
}

fn hit_result(name: &str) -> search_core::models::FileSearchResult {
    search_core::models::FileSearchResult {
        full_name: name.to_string(),
        status: search_core::models::FileSearchStatus::Hit,
        hits: vec![search_core::models::LineHit {
            line_number: 1,
            before: None,
            after: None,
            match_line: "needle found here".to_string(),
            matched_filters: vec!["needle".to_string()],
        }],
        created: Local::now(),
        modified: Local::now(),
        file_length: 1234,
        lines_cache: Vec::new(),
        total_line_count: 1,
        proximity_min_range: None,
        low_confidence_pdf: false,
        error_message: None,
    }
}

// ---------------------------------------------------------------------
// 1. shell::draw at each documented breakpoint
// ---------------------------------------------------------------------

#[test]
fn shell_renders_at_narrow_breakpoint_without_the_rail() {
    let state = AppState::default();
    let buffer = render_shell(&state, 60, 24);
    let text = buffer_text(&buffer);
    // Narrow (<70 cols) drops the persistent rail entirely per the plan's
    // responsive design - "Search Files" (a rail label) should not appear,
    // but the topbar's breadcrumb (built from the same tool title) does,
    // so assert on a rail-only marker instead: the "Bushing" rail entry
    // text, which only the rail (not the topbar/workspace) ever renders.
    assert!(!text.contains("Bushing"), "narrow layout should have no persistent rail:\n{text}");
}

#[test]
fn shell_renders_at_medium_breakpoint_with_a_collapsed_rail() {
    let state = AppState::default();
    let buffer = render_shell(&state, 85, 30);
    let text = buffer_text(&buffer);
    assert!(text.contains("Ctrl+P"), "topbar hint should still render:\n{text}");
}

#[test]
fn shell_renders_at_wide_breakpoint_with_a_full_rail() {
    let state = AppState::default();
    let buffer = render_shell(&state, 120, 40);
    let text = buffer_text(&buffer);
    assert!(text.contains("Bushing"), "wide layout should show full rail labels:\n{text}");
    assert!(text.contains("GS Toolbench"), "topbar breadcrumb should render:\n{text}");
}

// ---------------------------------------------------------------------
// 1b. Status bar's contextual hint - the only always-visible indication
// that "s" opens Settings, or that "Esc" backs out of it; previously
// discoverable only via the `?` help overlay, a separate modal.
// ---------------------------------------------------------------------

#[test]
fn status_bar_hints_s_settings_only_when_results_pane_is_focused_on_run_screen() {
    let mut state = AppState::default();
    state.nav.activate(app_tui::nav::ToolId::Search);
    state.focus.area = app_tui::nav::FocusArea::Workspace(PANE_RESULTS);

    let buffer = render_shell(&state, 100, 30);
    let text = buffer_text(&buffer);
    assert!(text.contains("s Settings"), "Results pane focused on Run screen should hint \"s Settings\":\n{text}");
}

#[test]
fn status_bar_does_not_hint_s_settings_when_a_different_pane_is_focused() {
    let mut state = AppState::default();
    state.nav.activate(app_tui::nav::ToolId::Search);
    state.focus.area = app_tui::nav::FocusArea::Workspace(app_tui::toolboxes::search::PANE_PATH);

    let buffer = render_shell(&state, 100, 30);
    let text = buffer_text(&buffer);
    assert!(!text.contains("s Settings"), "\"s\" only opens Settings from the Results pane, not Path:\n{text}");
}

#[test]
fn status_bar_hints_esc_back_while_on_the_settings_screen() {
    let mut state = AppState::default();
    state.nav.activate(app_tui::nav::ToolId::Search);
    state.search.screen = app_tui::toolboxes::search::ToolboxScreen::Settings;

    let buffer = render_shell(&state, 100, 30);
    let text = buffer_text(&buffer);
    assert!(text.contains("Esc Back"), "Settings screen should hint how to back out:\n{text}");
}

#[test]
fn status_bar_has_no_search_specific_hint_outside_the_search_toolbox() {
    // Placeholder toolboxes (e.g. Bushing) have no bindings of their own
    // yet - the contextual hint slot must stay empty, not stale from
    // whatever Search-toolbox state happens to be sitting around.
    let mut state = AppState::default();
    state.nav.activate(app_tui::nav::ToolId::Search);
    state.focus.area = app_tui::nav::FocusArea::Workspace(PANE_RESULTS);
    // Bushing is disabled, so `activate` alone won't switch to it - flip
    // the field directly to exercise the render path regardless.
    state.nav.active_tool = app_tui::nav::ToolId::Bushing;

    let buffer = render_shell(&state, 100, 30);
    let text = buffer_text(&buffer);
    assert!(!text.contains("s Settings"), "non-Search toolbox must not show the Search-only hint:\n{text}");
}

// ---------------------------------------------------------------------
// 2. Degenerate / small-terminal sizes - must never panic
// ---------------------------------------------------------------------

#[test]
fn shell_does_not_panic_at_degenerate_sizes() {
    let state = AppState::default();
    for (w, h) in [(0, 0), (0, 24), (60, 0), (1, 1), (69, 24), (70, 24), (99, 24), (100, 24)] {
        let _ = render_shell(&state, w, h);
    }
}

#[test]
fn shell_does_not_panic_at_degenerate_sizes_with_search_active_and_results() {
    let mut state = AppState::default();
    state.nav.activate(app_tui::nav::ToolId::Search);
    state.search.run.results = vec![hit_result("a.txt"), hit_result("b.txt")];
    state.search.run.in_flight_files = vec![search_core::models::InFlightFileStatus {
        file_name: "c.pdf".to_string(),
        status_text: "Extracting PDF text - 2 stream(s) scanned".to_string(),
        elapsed_seconds: 4.2,
    }];
    state.search.run.is_running = true;
    for (w, h) in [(0, 0), (1, 1), (5, 3), (69, 10), (70, 10), (120, 40)] {
        let _ = render_shell(&state, w, h);
    }
}

// ---------------------------------------------------------------------
// 3. Modal overlays render without panicking, at small sizes included
// ---------------------------------------------------------------------

#[test]
fn palette_overlay_does_not_panic_at_any_size() {
    let mut state = AppState::default();
    state.modal = ModalState::Palette(CommandPalette::default());
    for (w, h) in [(0, 0), (1, 1), (10, 5), (120, 40)] {
        let _ = render_shell(&state, w, h);
    }
}

#[test]
fn help_overlay_does_not_panic_at_any_size() {
    let mut state = AppState::default();
    state.modal = ModalState::Help;
    for (w, h) in [(0, 0), (1, 1), (10, 5), (120, 40)] {
        let _ = render_shell(&state, w, h);
    }
}

#[test]
fn confirm_overlay_does_not_panic_at_any_size() {
    let mut state = AppState::default();
    state.modal = ModalState::Confirm(ConfirmDialog {
        title: "Quit?".to_string(),
        message: "A search is still running. Quit anyway?".to_string(),
        on_confirm: ConfirmAction::Quit,
    });
    for (w, h) in [(0, 0), (1, 1), (10, 5), (120, 40)] {
        let _ = render_shell(&state, w, h);
    }
}

#[test]
fn palette_overlay_renders_its_input_prompt_at_a_normal_size() {
    let mut state = AppState::default();
    state.modal = ModalState::Palette(CommandPalette::default());
    let buffer = render_shell(&state, 120, 40);
    let text = buffer_text(&buffer);
    assert!(text.contains("Commands"), "expected the palette's own border title:\n{text}");
}

// ---------------------------------------------------------------------
// 4. run_view::draw in each meaningful state
// ---------------------------------------------------------------------

fn render_run_view(tool: &SearchToolState, focused_pane: Option<u8>, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width.max(1), height.max(1));
    let mut terminal = Terminal::new(backend).unwrap();
    let theme = app_tui::theme::Theme::default();
    terminal
        .draw(|frame| run_view::draw(frame, frame.area(), &theme, tool, focused_pane, 0))
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn run_view_empty_state_shows_no_search_run_yet() {
    let tool = SearchToolState::default();
    let buffer = render_run_view(&tool, None, 100, 30);
    let text = buffer_text(&buffer);
    assert!(text.contains("No search run yet"), "expected the empty-results message:\n{text}");
}

#[test]
fn run_view_running_state_shows_in_flight_file_and_gauge() {
    let mut tool = SearchToolState::default();
    tool.run = SearchRunState {
        is_running: true,
        progress_percent: 42.0,
        status_text: "3 of 7 file(s) - 1 hit(s) so far".to_string(),
        in_flight_files: vec![search_core::models::InFlightFileStatus {
            file_name: "report.pdf".to_string(),
            status_text: "Extracting PDF text - 1 stream(s) scanned".to_string(),
            elapsed_seconds: 1.5,
        }],
        ..Default::default()
    };
    let buffer = render_run_view(&tool, Some(PANE_RESULTS), 100, 30);
    let text = buffer_text(&buffer);
    assert!(text.contains("report.pdf"), "expected the in-flight file name:\n{text}");
    assert!(text.contains("42%"), "expected the gauge label:\n{text}");
}

#[test]
fn run_view_results_state_shows_hit_count_and_preview() {
    let mut tool = SearchToolState::default();
    tool.run.results = vec![hit_result("notes.txt")];
    tool.run.has_results = true;
    let buffer = render_run_view(&tool, Some(PANE_RESULTS), 100, 30);
    let text = buffer_text(&buffer);
    assert!(text.contains("notes.txt"), "expected the result row's file name:\n{text}");
    assert!(text.contains("needle"), "expected the preview pane's highlighted match text:\n{text}");
}

#[test]
fn run_view_does_not_panic_at_degenerate_sizes_in_any_state() {
    let mut empty = SearchToolState::default();
    let mut running = SearchToolState::default();
    running.run.is_running = true;
    running.run.in_flight_files =
        vec![search_core::models::InFlightFileStatus { file_name: "x".to_string(), status_text: "Reading...".to_string(), elapsed_seconds: 0.1 }];
    let mut with_results = SearchToolState::default();
    with_results.run.results = vec![hit_result("a.txt"), hit_result("b.txt")];

    for tool in [&mut empty, &mut running, &mut with_results] {
        for pane in [None, Some(PANE_RESULTS)] {
            for (w, h) in [(0, 0), (1, 1), (5, 2), (39, 6), (40, 6)] {
                let _ = render_run_view(tool, pane, w, h);
            }
        }
    }
}

// ---------------------------------------------------------------------
// 5. A resize sequence through handle_event, crossing breakpoints
// ---------------------------------------------------------------------

#[test]
fn resize_events_through_handle_event_never_panic_on_the_next_render() {
    let mut state = AppState::default();
    // Deliberately walks across all three documented breakpoints
    // (Narrow <70, Medium 70-99, Wide >=100) plus a couple of degenerate
    // sizes, exactly as a user dragging a terminal window narrower/wider
    // would produce - a real "user resizes their terminal" path, not just
    // calling shell::draw directly at assorted sizes.
    for (w, h) in [(120, 40), (85, 30), (60, 20), (1, 1), (0, 0), (100, 24), (69, 24), (70, 24)] {
        let effects = handle_event(&mut state, AppEvent::Terminal(Event::Resize(w, h)));
        assert!(effects.is_empty(), "a resize event should never itself produce an Effect");
        let _ = render_shell(&state, w, h);
    }
}
