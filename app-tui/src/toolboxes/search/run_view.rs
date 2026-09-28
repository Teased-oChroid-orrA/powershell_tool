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

use crate::theme::{StatusTone, Theme};
use crate::widgets::{empty_state, gauge_row, scroll_list};

use super::{index_view, preview, SearchToolState, PANE_FILTERS, PANE_PATH, PANE_RESULTS};

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState, focused_pane: Option<u8>, tick: u64) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // path field
            Constraint::Length(1), // filters field
            Constraint::Length(1), // run/cancel hint + status
            Constraint::Length(3), // progress gauge
            Constraint::Length(1), // fast re-search index status (quiet when idle/disabled)
            Constraint::Length(4), // in-flight ticker
            Constraint::Min(3),    // results + preview
        ])
        .split(area);

    draw_field(frame, rows[0], theme, "Path", &state.config.search_path, focused_pane == Some(PANE_PATH));
    draw_field(frame, rows[1], theme, "Filters", &state.config.filters_text, focused_pane == Some(PANE_FILTERS));
    // "s" is only bound while the Results pane has focus (see
    // `toolboxes::search::handle_key`) - typing "s" into Path/Filters must
    // insert a literal character, not open Settings.
    draw_run_hint(frame, rows[2], theme, state);
    draw_gauge(frame, rows[3], theme, state);
    index_view::render_status_line(frame, rows[4], theme, tick, &state.index_run);
    draw_in_flight(frame, rows[5], theme, state);
    draw_results(frame, rows[6], theme, state, focused_pane == Some(PANE_RESULTS));
}

fn draw_field(frame: &mut Frame, area: Rect, theme: &Theme, label: &str, value: &str, focused: bool) {
    let cursor = if focused { Span::styled("_", theme.title_style(true)) } else { Span::raw("") };
    let line = Line::from(vec![Span::styled(format!("{label}: "), theme.title_style(focused)), Span::raw(value), cursor]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_run_hint(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState) {
    let hint = if state.run.is_running { "c Cancel" } else { "Enter Run" };
    let tone = if state.run.status_text.starts_with("Error") {
        StatusTone::Danger
    } else if state.run.is_running {
        StatusTone::Info
    } else {
        StatusTone::Neutral
    };
    let line = Line::from(vec![
        Span::styled(hint, theme.disabled_style()),
        Span::raw("   "),
        Span::styled(state.run.status_text.clone(), theme.status_style(tone)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_gauge(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState) {
    let tone = if state.run.is_running {
        gauge_row::GaugeTone::Running
    } else if state.run.status_text.starts_with("Error") {
        gauge_row::GaugeTone::Danger
    } else {
        gauge_row::GaugeTone::Success
    };
    let label = format!("{:.0}%", state.run.progress_percent);
    gauge_row::render(frame, area, theme, state.run.progress_percent, &label, tone);
}

fn draw_in_flight(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState) {
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

fn draw_results(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState, focused: bool) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    draw_results_list(frame, cols[0], theme, state, focused);
    draw_preview(frame, cols[1], theme, state);
}

fn draw_results_list(frame: &mut Frame, area: Rect, theme: &Theme, state: &SearchToolState, focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(format!(" Results ({}) ", state.run.results.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

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
    scroll_list::render(frame, inner, items, Some(state.selected_result));
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
