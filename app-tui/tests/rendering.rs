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
    let mut regions = app_tui::mouse::MouseRegions::default();
    render_shell_with_regions(state, width, height, &mut regions)
}

fn render_shell_with_regions(state: &AppState, width: u16, height: u16, regions: &mut app_tui::mouse::MouseRegions) -> Buffer {
    let backend = TestBackend::new(width.max(1), height.max(1));
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| shell::draw(frame, state, 0, regions)).unwrap();
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
    // Contextual hints must not leak across toolboxes.
    let mut state = AppState::default();
    state.nav.activate(app_tui::nav::ToolId::Search);
    state.focus.area = app_tui::nav::FocusArea::Workspace(PANE_RESULTS);
    state.nav.activate(app_tui::nav::ToolId::EccentricBushing);

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
    let mut regions = app_tui::mouse::MouseRegions::default();
    terminal
        .draw(|frame| run_view::draw(frame, frame.area(), &theme, tool, focused_pane, 0, &mut regions))
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
fn filter_group_summary_stays_visible_after_a_run_completes() {
    let mut tool = SearchToolState::default();
    tool.config.filters_text = "house ; floor, two [near 3] ; draft [not]".to_string();
    tool.run = SearchRunState {
        is_running: false,
        progress_percent: 100.0,
        status_text: "7 of 7 file(s) - 3 hit(s) so far".to_string(),
        files_completed: 7,
        total_files: 7,
        started: Some(std::time::Instant::now()),
        elapsed: Some(std::time::Duration::from_secs(2)),
        ..Default::default()
    };
    let text = buffer_text(&render_run_view(&tool, None, 120, 30));
    assert!(text.contains("floor, two [within 3 lines]"), "group summary must not be replaced by run status:\n{text}");
    assert!(text.contains("draft [exclude file]"), "{text}");
    assert!(text.contains("100%") && text.contains("7 of 7 file(s)") && text.contains("elapsed"), "{text}");
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

// ---------------------------------------------------------------------
// 6. Mouse hit-test round trip: render for real, then click at the exact
// geometry the render just published. Catches drift between rendered
// layout and hit-test math directly, rather than assuming they agree (see
// the mouse navigation plan's verification section).
// ---------------------------------------------------------------------

fn click_at(rect: ratatui::layout::Rect) -> Event {
    Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: crossterm::event::KeyModifiers::NONE,
    })
}

#[test]
fn clicking_the_rendered_fastener_hole_rail_entry_switches_to_it() {
    let state = AppState::default();
    let mut regions = app_tui::mouse::MouseRegions::default();
    let _ = render_shell_with_regions(&state, 120, 40, &mut regions);

    let rect = regions
        .rail
        .iter()
        .find(|(_, tool)| *tool == app_tui::nav::ToolId::FastenerHole)
        .map(|(rect, _)| *rect)
        .expect("wide layout must publish a rail region for Fastener Holes");

    let mut state = state;
    if let Event::Mouse(mouse_event) = click_at(rect) {
        app_tui::app::handle_mouse(&mut state, &regions, mouse_event);
    }
    assert_eq!(state.nav.active_tool, app_tui::nav::ToolId::FastenerHole);
}

#[test]
fn clicking_the_rendered_second_result_row_selects_it() {
    let mut state = AppState::default();
    state.nav.activate(app_tui::nav::ToolId::Search);
    state.focus.area = app_tui::nav::FocusArea::Workspace(PANE_RESULTS);
    state.search.run.results = vec![hit_result("a.txt"), hit_result("b.txt")];

    let mut regions = app_tui::mouse::MouseRegions::default();
    let _ = render_shell_with_regions(&state, 120, 40, &mut regions);

    let rect = regions
        .results_rows
        .iter()
        .find(|(_, idx)| *idx == 1)
        .map(|(rect, _)| *rect)
        .expect("results list must publish a region for the second row");

    if let Event::Mouse(mouse_event) = click_at(rect) {
        app_tui::app::handle_mouse(&mut state, &regions, mouse_event);
    }
    assert_eq!(state.search.selected_result, 1);
}

#[test]
fn clicking_outside_every_published_region_never_panics() {
    let state = AppState::default();
    let mut regions = app_tui::mouse::MouseRegions::default();
    let _ = render_shell_with_regions(&state, 120, 40, &mut regions);

    let mut state = state;
    let mouse_event = crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 5000,
        row: 5000,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };
    let effects = app_tui::app::handle_mouse(&mut state, &regions, mouse_event);
    assert!(effects.is_empty());
}

// ---------------------------------------------------------------------
// Post-search diagnostics summary
// ---------------------------------------------------------------------

fn finished_tool_with_diagnostics() -> SearchToolState {
    use app_tui::toolboxes::search::diagnostics::{IndexHealth, SearchDiagnostics};
    let mut diagnostics = SearchDiagnostics { roots: 1, verified: 1_397, matched_files: 17, verify_ms: 380, total_ms: 610, ..Default::default() };
    diagnostics.record_narrowed(
        &search_core::native_index::NarrowOutcome { scannable: 104_331, walk_ms: 100, freshness_ms: 60, query_ms: 42, ..Default::default() },
        1_382,
    );
    diagnostics.health = Some(IndexHealth { docs: 104_331, segments: 4, size_bytes: 2048, semantic_version: 2, ..Default::default() });
    let mut tool = SearchToolState::default();
    tool.run = SearchRunState { started: Some(std::time::Instant::now()), diagnostics: Some(diagnostics), ..SearchRunState::default() };
    tool
}

#[test]
fn run_view_shows_the_search_summary_after_a_run_finishes() {
    let text = buffer_text(&render_run_view(&finished_tool_with_diagnostics(), None, 140, 40));
    assert!(text.contains("Search summary"), "{text}");
    assert!(text.contains("Corpus 104,331 files") && text.contains("index candidates 1,382 (98.7% fewer)"), "{text}");
    assert!(text.contains("exact verification 380 ms") && text.contains("total 610 ms"), "{text}");
    assert!(text.contains("Index: 104,331 docs"), "health line missing:\n{text}");
}

#[test]
fn run_view_keeps_the_in_flight_box_while_running_and_shrinks_the_summary_on_short_terminals() {
    let mut tool = finished_tool_with_diagnostics();
    tool.run.is_running = true;
    let text = buffer_text(&render_run_view(&tool, None, 140, 40));
    assert!(text.contains("In-flight") && !text.contains("Search summary"), "{text}");

    let short = buffer_text(&render_run_view(&finished_tool_with_diagnostics(), None, 140, 20));
    assert!(short.contains("Search summary") && short.contains("Corpus 104,331 files"), "short terminal still shows the first lines:\n{short}");
}

#[test]
fn search_diagnostics_event_is_stored_on_the_run_state() {
    let mut state = AppState::default();
    let d = app_tui::toolboxes::search::diagnostics::SearchDiagnostics { matched_files: 5, ..Default::default() };
    handle_event(&mut state, AppEvent::SearchDiagnostics(Box::new(d)));
    assert_eq!(state.search.run.diagnostics.as_ref().map(|d| d.matched_files), Some(5));
}

// ---------------------------------------------------------------------
// FEA Workbench: the canvas paints a coloured contour, never panics
// ---------------------------------------------------------------------

fn fea_state_with_result() -> AppState {
    let mut state = AppState::default();
    state.nav.activate(app_tui::nav::ToolId::FeaWorkbench);
    let w = &mut state.fea_workbench;
    let problem = w.problem.clone();
    let mesh = fea_problem::solve::preview(&problem, None).unwrap();
    let solved = fea_problem::solve(&problem, None).unwrap();
    let _ = mesh;
    // Drive the same path the app does: tick -> preview job -> solve job.
    for e in w.tick() {
        if let app_tui::app::Effect::RunFeaPreview { id, problem, import_text } = e {
            w.finish_preview(id, fea_problem::solve::preview(&problem, import_text.as_deref()));
        }
    }
    w.finish_solve_for_test(solved);
    state
}

#[test]
fn fea_workbench_renders_the_field_list_and_a_colour_contour() {
    let state = fea_state_with_result();
    let buf = render_shell(&state, 160, 48);
    let text = buffer_text(&buf);
    assert!(text.contains("FEA Workbench"), "{text}");
    assert!(text.contains("von Mises stress"), "contour title");
    assert!(text.contains("Peak von Mises"), "results readout");
    assert!(text.contains("Plate with a hole"), "template row");
    // The canvas paints true-colour half blocks: both reds and blues from the ramp occur.
    let area = buf.area;
    let mut colours = std::collections::HashSet::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let c = &buf[(x, y)];
            if c.symbol() == "\u{2580}" || c.symbol() == "\u{2584}" {
                colours.insert(format!("{:?}", c.fg));
            }
        }
    }
    assert!(colours.len() > 8, "a contour uses many colours, found {}", colours.len());
}

#[test]
fn fea_workbench_draws_a_mode_shape_and_lists_the_frequencies() {
    let mut state = fea_state_with_result();
    let w = &mut state.fea_workbench;
    // The cantilever template has a clean first bending mode.
    let (_, beam) = fea_problem::templates::templates().into_iter().find(|(t, _)| *t == "Cantilever beam").unwrap();
    w.set_problem(beam.clone());
    let d = fea_problem::dynamics::run(fea_problem::dynamics::DynKind::Modal, &beam, None, 3).unwrap();
    w.finish_dynamic_for_test(d);
    let buf = render_shell(&state, 170, 50);
    let text = buffer_text(&buf);
    assert!(text.contains("Mode 1 - "), "contour title: {text}");
    assert!(text.contains("Natural frequencies"), "readout");
    assert!(text.contains(" Hz"), "frequencies listed");
    let area = buf.area;
    let mut colours = std::collections::HashSet::new();
    for y in 0..area.height {
        for x in 0..area.width {
            let c = &buf[(x, y)];
            if c.symbol() == "\u{2580}" || c.symbol() == "\u{2584}" {
                colours.insert(format!("{:?}", c.fg));
            }
        }
    }
    assert!(colours.len() > 8, "a mode contour uses many colours, found {}", colours.len());
}

#[test]
fn fea_workbench_does_not_panic_at_any_size_with_and_without_a_result() {
    let with = fea_state_with_result();
    let mut without = AppState::default();
    without.nav.activate(app_tui::nav::ToolId::FeaWorkbench);
    for state in [&with, &without] {
        for (w, h) in [(0, 0), (1, 1), (20, 6), (69, 10), (100, 24), (160, 48), (240, 70)] {
            let _ = render_shell(state, w, h);
        }
    }
}
