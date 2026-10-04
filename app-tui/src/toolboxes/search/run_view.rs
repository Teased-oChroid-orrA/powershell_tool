//! Search Files "Run" workspace: compact config summary, live progress, the
//! in-flight ticker, results list, and preview pane.
//!
//! The fuller scrollable Settings form (every `SearchToolConfig` field) is
//! later-phase work - this view deliberately keeps the always-visible
//! config surface to just Path/Filters rather than crowding every setting
//! onto one screen (see the migration plan's rationale for splitting
//! Run/Settings into two views instead of one desktop-mockup-shaped strip).

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph};

use crate::mouse::MouseRegions;
use crate::theme::{StatusTone, Theme};
use crate::widgets::progress_bar::{self, BarState, ProgressView};
use crate::widgets::{empty_state, scroll_list};

use super::{index_view, preview, SearchToolState, PANE_FILTERS, PANE_PATH, PANE_RESULTS};

#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    state: &SearchToolState,
    focused_pane: Option<u8>,
    tick: u64,
    regions: &mut MouseRegions,
) {
    let summary = finished_summary(state);
    // Room for the whole summary only when the terminal is tall enough to keep
    // a usable results list; otherwise the box stays 2 lines and shows the first two.
    let ticker_height = match &summary {
        Some(lines) if area.height >= 26 => lines.len() as u16 + 2,
        _ => 4,
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // path field
            Constraint::Length(1), // filters field
            Constraint::Length(1), // run/cancel hint + status
            Constraint::Length(2), // progress bar + live stats
            Constraint::Length(2), // fast re-search index status (wraps; quiet when idle/disabled)
            Constraint::Length(ticker_height), // in-flight ticker, or the finished-run summary
            Constraint::Min(3),    // results + preview
        ])
        .split(area);

    draw_field(frame, rows[0], theme, "Path", &state.config.search_path, focused_pane == Some(PANE_PATH), PANE_PATH, regions);
    draw_field(frame, rows[1], theme, "Filters", &state.config.filters_text, focused_pane == Some(PANE_FILTERS), PANE_FILTERS, regions);
    // "s" is only bound while the Results pane has focus (see
    // `toolboxes::search::handle_key`) - typing "s" into Path/Filters must
    // insert a literal character, not open Settings.
    draw_run_hint(frame, rows[2], theme, state, focused_pane == Some(PANE_FILTERS));
    draw_progress(frame, rows[3], theme, state, tick);
    index_view::render_status_line(frame, rows[4], theme, tick, &state.index_run);
    match summary {
        Some(lines) => draw_summary(frame, rows[5], theme, &lines),
        None => draw_in_flight(frame, rows[5], theme, state),
    }
    draw_results(frame, rows[6], theme, state, focused_pane == Some(PANE_RESULTS), regions);
}

fn draw_field(frame: &mut Frame, area: Rect, theme: &Theme, label: &str, value: &str, focused: bool, pane: u8, regions: &mut MouseRegions) {
    // Tail-keeping: a long path must never push the cursor off the row.
    let line = crate::widgets::input_line::line(theme, &format!("{label}: "), value, if focused { "_" } else { "" }, area.width);
    frame.render_widget(Paragraph::new(line), area);
    regions.workspace_panes.push((area, pane));
}

fn draw_run_hint(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState, filters_focused: bool) {
    let hint = if state.run.is_running { "c Cancel" } else { "Enter Run" };
    // Always the group summary/tip - run status lives in the progress panel
    // below, so it can never hide this.
    let line = Line::from(vec![
        Span::styled(hint, theme.disabled_style()),
        Span::raw("   "),
        Span::styled(filter_group_hint(&state.config, filters_focused), theme.status_style(StatusTone::Neutral)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// "house [any line] + draft [exclude file]" once the Filters field has more
/// than one `;`-group; a one-line syntax tip while it is focused. Shown
/// whenever it applies (not only before the first run).
fn filter_group_hint(config: &super::model::SearchToolConfig, filters_focused: bool) -> String {
    use super::model::{effective_mode, mode_label, parse_filter_groups, GroupMode};
    let groups = parse_filter_groups(&config.filters_text);
    if groups.len() > 1 {
        let mut text = groups
            .iter()
            .map(|g| format!("{} [{}]", g.filters.join(", "), mode_label(effective_mode(g, config), config.proximity_lines)))
            .collect::<Vec<_>>()
            .join(" + ");
        if groups.iter().all(|g| matches!(g.mode, Some(GroupMode::Exclude(_)))) {
            text.push_str("  ⚠ add a filter that is not [not]");
        }
        return text;
    }
    if filters_focused {
        return "; adds a group: house ; floor, two [near 3] ; draft [not]   tags: any all near N not | not line".to_string();
    }
    String::new()
}

fn draw_progress(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState, tick: u64) {
    let view = if state.index_run.is_building {
        // An index build (manual, or started by a search that needed the
        // index) owns the panel while it runs.
        let building = state.index_run.percent > 0.0;
        ProgressView {
            percent: building.then_some(state.index_run.percent),
            state: BarState::Running,
            segments: vec![state.index_run.status_text.clone()],
            tick,
            tone: None,
        }
    } else {
        let run = &state.run;
        let failed = run.status_text.starts_with("Error");
        let cancelled = run.status_text == "Cancelled.";
        let bar_state = if run.is_running {
            BarState::Running
        } else if failed || cancelled {
            BarState::Failed
        } else if run.started.is_some() {
            BarState::Done
        } else {
            BarState::Idle
        };
        let scanning = run.is_running && run.total_files == 0 && run.progress_percent == 0.0;
        let elapsed = run.elapsed_secs();
        let mut segments = vec![run.status_text.clone()];
        if let Some(secs) = elapsed.filter(|s| *s >= 1.0 && run.files_completed > 0) {
            segments.push(format!("{:.1} files/s", f64::from(run.files_completed) / secs));
            if run.is_running && run.progress_percent >= 1.0 {
                segments.push(format!("ETA {}", super::indexing::format_eta(secs * (100.0 - run.progress_percent) / run.progress_percent)));
            }
        }
        if let Some(secs) = elapsed {
            segments.push(format!("{} elapsed", super::indexing::format_eta(secs)));
        }
        ProgressView {
            percent: if scanning { None } else { Some(if bar_state == BarState::Idle { 0.0 } else { run.progress_percent }) },
            state: bar_state,
            segments,
            tick,
            tone: failed.then_some(StatusTone::Danger),
        }
    };
    progress_bar::render(frame, area, theme, &view);
}

/// The post-search diagnostics, once a run has finished and nothing else (a
/// running search, an index build) owns the ticker box.
fn finished_summary(state: &SearchToolState) -> Option<Vec<String>> {
    if state.run.is_running || state.index_run.is_building {
        return None;
    }
    state.run.diagnostics.as_ref().map(|d| d.lines(state.index_run.last_build_failed))
}

fn draw_summary(frame: &mut Frame, area: Rect, theme: &Theme, lines: &[String]) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(false))
        .title(" Search summary ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    let text: Vec<Line> = lines.iter().map(|l| Line::from(l.as_str())).collect();
    frame.render_widget(Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: false }), inner);
}

fn draw_in_flight(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState) {
    if state.index_run.is_building && !state.index_run.current_file.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_style(false))
            .title(" Indexing ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height > 0 {
            // Wrapped so a long path is readable instead of cut off at the edge.
            let file = Paragraph::new(state.index_run.current_file.clone()).wrap(ratatui::widgets::Wrap { trim: false });
            frame.render_widget(file, inner);
        }
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(false))
        .title(format!(" In-flight ({}) ", state.run.in_flight_files.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if state.run.in_flight_files.is_empty() || inner.height == 0 {
        return;
    }
    let items: Vec<ListItem> = state
        .run
        .in_flight_files
        .iter()
        .map(|f| {
            ListItem::new(Line::from(format!(
                "{}  {}  {}",
                f.file_name,
                f.status_text,
                crate::format::format_elapsed(f.elapsed_seconds)
            )))
        })
        .collect();
    frame.render_widget(List::new(items), inner);
}

fn draw_results(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState, focused: bool, regions: &mut MouseRegions) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    draw_results_list(frame, cols[0], theme, state, focused, regions);
    draw_preview(frame, cols[1], theme, state);
}

fn draw_results_list(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState, focused: bool, regions: &mut MouseRegions) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(format!(" Results ({}) ", state.run.results.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    regions.workspace_panes.push((area, PANE_RESULTS));

    if state.run.results.is_empty() {
        let message = if state.run.is_running {
            "Searching..."
        } else if state.run.status_text.is_empty() {
            "No search run yet"
        } else {
            "0 hits"
        };
        empty_state::render(frame, inner, theme, message, Some("Enter Run to search"));
        return;
    }

    let items: Vec<ListItem> = state
        .run
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let name = std::path::Path::new(&r.full_name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| r.full_name.clone());
            let selected = i == state.selected_result;
            let marker = if selected { "> " } else { "  " };
            let style = if selected {
                theme.selected_row_style()
            } else {
                Style::default().fg(theme.fg)
            };
            let hit_word = if r.hits.len() == 1 { "hit" } else { "hits" };
            ListItem::new(Line::from(Span::styled(format!("{marker}{name}  {} {hit_word}", r.hits.len()), style)))
        })
        .collect();
    let offset = scroll_list::render(frame, inner, items, Some(state.selected_result));
    regions.results_rows.extend(crate::mouse::list_row_regions(inner, offset, state.run.results.len()));
}

fn draw_preview(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(false))
        .title(" Preview ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(result) = state.run.results.get(state.selected_result) else {
        empty_state::render(frame, inner, theme, "Select a result", None);
        return;
    };

    if inner.height == 0 {
        return;
    }

    let highlight_style = Style::default().fg(theme.danger).add_modifier(Modifier::BOLD);
    let mut lines = vec![Line::from(Span::styled(preview::metadata_summary(result), theme.disabled_style()))];
    let remaining = inner.height.saturating_sub(1) as usize;
    for hit in result.hits.iter().take(remaining) {
        lines.push(preview::render_highlighted_line(&hit.match_line, &hit.matched_filters, highlight_style));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
