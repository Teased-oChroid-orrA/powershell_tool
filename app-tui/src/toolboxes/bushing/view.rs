//! Rendering for the Bushing Workbench toolbox. Wide terminals show the
//! editable field list beside a live-updating readout of every derived
//! result; narrow terminals stack the two vertically - same responsive
//! rule and `fields_required_width` technique `toolboxes/pressure_vessel/view.rs`
//! and `toolboxes/fastener_hole/view.rs` both use.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::theme::{StatusTone, Theme};

use bushing_solver::tolerance::ToleranceStatus;

use super::advice::{self, CheckKind, Severity};
use super::model::{self, BushingModel, FieldRow};
use super::{AdviceTab, BushingAction, BushingState};

const MIN_READOUT_WIDTH: u16 = 40;

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, state: &BushingState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(" Bushing Workbench - Space/Enter: toggle/pick/edit \u{b7} d: details \u{b7} e: export ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let required_fields_width = fields_required_width(state);
    let (fields_area, readout_area) = if inner.width >= required_fields_width + MIN_READOUT_WIDTH {
        let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Length(required_fields_width), Constraint::Min(MIN_READOUT_WIDTH)]).split(inner);
        (cols[0], cols[1])
    } else {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(55), Constraint::Percentage(45)]).split(inner);
        (rows[0], rows[1])
    };

    regions.workspace_panes.push((area, super::PANE_MAIN));
    draw_fields(frame, fields_area, theme, state, focused, regions);
    draw_readout(frame, readout_area, theme, state, regions);

    if state.material_picker.open {
        super::material_picker::render(frame, area, theme, &state.material_picker, &state.model, regions);
    }
    if state.reamer_picker.open {
        super::reamer_picker::render(frame, area, theme, &state.reamer_picker, &state.model, regions);
    }
    if state.friction_picker.open {
        super::friction_picker::render(frame, area, theme, &state.friction_picker, regions);
    }
    if state.bushing_id_picker.open {
        super::bushing_id_picker::render(frame, area, theme, &state.bushing_id_picker, &state.model, regions);
    }
    if state.advice.open {
        draw_advice_window(frame, area, theme, state, regions);
    }
    draw_edge_tip(frame, area, theme, state);
}

/// Tooltip for a cross-check: shown while the mouse is over its name, or
/// pinned by a click (Esc / another click closes it).
fn draw_edge_tip(frame: &mut Frame, area: Rect, theme: &Theme, state: &BushingState) {
    use ratatui::widgets::Clear;

    if state.advice.open || area.width < 24 || area.height < 8 {
        return;
    }
    let (topic, ax, ay) = match (state.edge_info_pinned, state.edge_hover) {
        (Some(t), Some((_, x, y))) => (t, x, y),
        (Some(t), None) => (t, area.x + area.width / 2, area.y + area.height / 3),
        (None, Some((t, x, y))) => (t, x, y),
        (None, None) => return,
    };
    let tip = super::edge_check::tip(topic);
    let width = area.width.saturating_sub(2).min(72);
    let mut lines: Vec<Line> = Vec::new();
    // The numbers behind this table row first, then the static discussion.
    lines.push(Line::from(Span::styled("This run", theme.title_style(true).add_modifier(Modifier::BOLD))));
    for detail in super::edge_check::detail_lines(topic, state.edge_check.as_ref(), &state.model) {
        lines.push(Line::from(detail));
    }
    for (heading, body) in tip.sections {
        lines.push(Line::from(Span::styled(*heading, theme.title_style(true).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(*body));
    }
    let inner_w = width.saturating_sub(2).max(1);
    let height = (crate::widgets::scroll_paragraph::wrapped_height(&lines, inner_w) + 2).min(area.height.saturating_sub(1)).max(3);
    let x = ax.min(area.x + area.width - width).max(area.x);
    // Prefer below the pointer; flip above when it would run off the pane.
    let below = ay + 1;
    let y = if below + height <= area.y + area.height { below } else { ay.saturating_sub(height).max(area.y) };
    let rect = Rect { x, y, width, height };
    frame.render_widget(Clear, rect);
    let hint = if state.edge_info_pinned.is_some() { " (Esc/click closes) " } else { " (click to pin) " };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(format!(" {} ", tip.title)).title_bottom(hint);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// Label of a row; tolerance rows read "... Tol" or "... Limits" by entry mode.
fn row_text(mode: model::ToleranceMode, row: FieldRow) -> &'static str {
    match row {
        FieldRow::Tol(group) => group.label(mode),
        other => model::row_label(other),
    }
}

fn compute_label_width(mode: model::ToleranceMode, rows: &[FieldRow]) -> u16 {
    rows.iter().filter(|r| !matches!(r, FieldRow::Header(_))).map(|r| row_text(mode, *r).len()).max().unwrap_or(0) as u16 + 2
}

fn display_value(model: &BushingModel, row: FieldRow) -> String {
    match row {
        FieldRow::Header(_) => String::new(),
        FieldRow::ToggleFitType => model::label_fit_type(model.fit_type).to_string(),
        FieldRow::ToggleToleranceMode => model.tolerance_mode.label().to_string(),
        FieldRow::Tol(group) => model.tolerance_display(group),
        FieldRow::ToggleBushingType => model::label_bushing_type(model.bushing_type).to_string(),
        FieldRow::ToggleIdType => model::label_id_type(model.id_type).to_string(),
        FieldRow::ToggleEndConstraint => model::label_end_constraint(model.end_constraint).to_string(),
        FieldRow::ToggleCsMode => model::label_cs_mode(model.cs_mode).to_string(),
        FieldRow::ToggleExtCsMode => model::label_cs_mode(model.ext_cs_mode).to_string(),
        FieldRow::ToggleEnforcementEnabled => if model.enforcement_enabled { "Enabled" } else { "Disabled" }.to_string(),
        FieldRow::ToggleLockBore => bool_label(model.lock_bore),
        FieldRow::TogglePreserveBoreNominal => bool_label(model.preserve_bore_nominal),
        FieldRow::ToggleAllowBoreNominalShift => bool_label(model.allow_bore_nominal_shift),
        FieldRow::ToggleAssemblyThermalEnabled => if model.assembly_thermal_enabled { "Enabled" } else { "Disabled" }.to_string(),
        FieldRow::OpenHousingMaterialPicker => model.housing_material().name.to_string(),
        FieldRow::OpenBushingMaterialPicker => model.bushing_material().name.to_string(),
        FieldRow::Number(target) => target.format_value(model.number_value(target)),
    }
}

fn bool_label(b: bool) -> String {
    if b { "Yes" } else { "No" }.to_string()
}

fn fields_required_width(state: &BushingState) -> u16 {
    let rows = model::field_rows(&state.model);
    let label_width = compute_label_width(state.model.tolerance_mode, &rows);
    let max_value_width = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            if state.editing && i == state.selected {
                state.edit_buffer.chars().count() + 1
            } else {
                display_value(&state.model, *row).chars().count()
            }
        })
        .max()
        .unwrap_or(0)
        .min(crate::widgets::scroll_list::VALUE_CAP) as u16;
    2 + label_width + max_value_width + 2
}

fn draw_fields(frame: &mut Frame, area: Rect, theme: &Theme, state: &BushingState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(focused)).title(" Inputs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = model::field_rows(&state.model);
    let hint = rows.get(state.selected).map(|r| model::field_hint(*r)).unwrap_or("");
    // Reserve at least 4 rows for the field list itself - the hint panel
    // may grow to fit a long wrapped hint, but never past the point of
    // crushing the list to nothing.
    let max_hint_lines = inner.height.saturating_sub(4).max(1);
    let hint_height = crate::widgets::hint_panel::hint_panel_height(hint, inner.width, max_hint_lines);
    let (list_area, hint_area) = if inner.height > hint_height + 1 {
        let split = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(hint_height)]).split(inner);
        (split[0], Some(split[1]))
    } else {
        (inner, None)
    };

    let label_width = compute_label_width(state.model.tolerance_mode, &rows) as usize;

    let mut heights: Vec<u16> = Vec::with_capacity(rows.len());
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            if let FieldRow::Header(text) = row {
                heights.push(1);
                return ListItem::new(Line::from(Span::styled(format!("-- {text} --"), theme.title_style(false).add_modifier(Modifier::BOLD))));
            }
            let selected = focused && i == state.selected;
            let value = if selected && state.editing { state.edit_buffer.with_cursor() } else { display_value(&state.model, *row) };
            let marker = if selected { "> " } else { "  " };
            let label = row_text(state.model.tolerance_mode, *row);
            let failing = state.model.checks.iter().any(|c| {
                c.severity == Severity::Fail
                    && match row {
                        FieldRow::Number(t) => c.kind.related_inputs().contains(t),
                        FieldRow::Tol(g) => {
                            let (_, plus, minus) = g.targets();
                            c.kind.related_inputs().contains(&plus) || c.kind.related_inputs().contains(&minus)
                        }
                        _ => false,
                    }
            });
            let style = if selected {
                theme.selected_row_style()
            } else if failing {
                theme.status_style(StatusTone::Danger)
            } else {
                Style::default()
            };
            let (item, h) = crate::widgets::scroll_list::field_item(marker, label, label_width, &value, list_area.width, style, None);
            heights.push(h);
            item
        })
        .collect();

    let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(state.selected));
    regions.bushing_rows.extend(crate::mouse::list_row_regions_var(list_area, offset, &heights));

    if let Some(hint_area) = hint_area {
        frame.render_widget(Paragraph::new(Line::from(Span::styled(hint, theme.disabled_style()))).wrap(Wrap { trim: true }), hint_area);
    }
}

fn tone_for_margin(margin: f64) -> StatusTone {
    if !margin.is_finite() {
        StatusTone::Neutral
    } else if margin < 0.0 {
        StatusTone::Danger
    } else if margin < 0.15 {
        StatusTone::Warning
    } else {
        StatusTone::Success
    }
}

fn fmt_margin(margin: f64) -> String {
    if margin.is_infinite() { "\u{2014}".to_string() } else { format!("{margin:+.2}") }
}

/// Maps a [`StatusTone`] to a concrete `ratatui` color without borrowing a
/// `Theme`, kept in sync with `Theme::status_style`'s own color choice by
/// construction (both switch on the same enum) - same helper
/// `pressure_vessel/view.rs::tone_color` provides for the same reason.
fn tone_color(tone: StatusTone) -> ratatui::style::Color {
    use ratatui::style::Color;
    match tone {
        StatusTone::Neutral => Color::Reset,
        StatusTone::Success => Color::Green,
        StatusTone::Warning => Color::Yellow,
        StatusTone::Danger => Color::Red,
        StatusTone::Info => Color::Blue,
    }
}

fn draw_readout(frame: &mut Frame, area: Rect, theme: &Theme, state: &BushingState, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(" Results ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    // Fixed action bar on top (never scrolls), scrolling readout below.
    let (bar_area, body_area) = if inner.height > 2 {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(1)]).split(inner);
        (Some(rows[0]), rows[1])
    } else {
        (None, inner)
    };
    // While the Fixes window is open it owns the mouse: publish only its regions.
    let interactive = !state.advice.open;
    if let Some(bar) = bar_area {
        draw_action_bar(frame, bar, theme, state, interactive.then_some(&mut *regions));
    }
    let stale = state.edge_check.as_ref().is_some_and(|run| super::edge_check::build_input(&state.model).as_ref() != Ok(&run.input));
    let edge_section = super::edge_check::section_lines(theme, state.edge_check.as_ref(), stale, &state.model);
    let advisories = super::edge_check::advisories(state.edge_check.as_ref(), stale);
    let (lines, tags) = readout_lines(theme, &state.model, state.show_numbers, state.last_applied.as_deref(), state.input_error.as_deref(), edge_section, advisories);
    let clicks = crate::widgets::scroll_paragraph::render_interactive(frame, body_area, theme, lines, state.results_scroll, &tags);
    if interactive {
        regions.bushing_actions.extend(clicks);
        regions.bushing_results = Some(body_area);
    }
}

/// `[ Fixes (3) ] [ Why OD clamped? ] [ Numbers: off ] [ Export ]` - each a
/// mouse button (and each has a key: `f`/`a`, `w`, `d`, `e`).
fn draw_action_bar(frame: &mut Frame, area: Rect, theme: &Theme, state: &BushingState, mut regions: Option<&mut crate::mouse::MouseRegions>) {
    let model = &state.model;
    let any_fail = model.checks.iter().any(|c| c.severity == Severity::Fail);
    let n = model.recommendations.len();
    let explain = advice::explain_tolerance(model).is_some();
    let mut buttons: Vec<(String, BushingAction, Style)> = Vec::new();
    let active = |tone: StatusTone| theme.status_style(tone).add_modifier(Modifier::REVERSED | Modifier::BOLD);
    buttons.push((
        format!(" Fixes ({n}) "),
        BushingAction::OpenFixes,
        if n > 0 { active(if any_fail { StatusTone::Danger } else { StatusTone::Warning }) } else { theme.disabled_style() },
    ));
    if explain {
        let label = if model.output.tolerance_status == ToleranceStatus::Infeasible { " Why infeasible? " } else { " Why OD clamped? " };
        buttons.push((label.to_string(), BushingAction::OpenExplain, active(StatusTone::Warning)));
    }
    buttons.push((format!(" Numbers: {} ", if state.show_numbers { "on" } else { "off" }), BushingAction::ToggleNumbers, Style::default().add_modifier(Modifier::REVERSED)));
    buttons.push((" Edge check ".to_string(), BushingAction::EdgeCheck, Style::default().add_modifier(Modifier::REVERSED)));
    buttons.push((" +Contact FE ".to_string(), BushingAction::EdgeCheckDeep, Style::default().add_modifier(Modifier::REVERSED)));
    buttons.push((" Export ".to_string(), BushingAction::Export, Style::default().add_modifier(Modifier::REVERSED)));

    let mut spans = Vec::new();
    let mut x = area.x;
    let right = area.x + area.width;
    for (label, action, style) in buttons {
        let w = label.chars().count() as u16;
        if x + w > right {
            break;
        }
        if let Some(r) = regions.as_deref_mut() {
            r.bushing_actions.push((Rect { x, y: area.y, width: w, height: 1 }, action));
        }
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
        x += w + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Centered pop-up: tab 1 lists the verified fixes (click a row to select,
/// double-click or `Apply` to apply it), tab 2 explains *why* the OD
/// tolerance was clamped/infeasible with the user's own numbers. Every
/// element is a mouse region; `Esc`/`Close` dismisses it.
fn draw_advice_window(frame: &mut Frame, area: Rect, theme: &Theme, state: &BushingState, regions: &mut crate::mouse::MouseRegions) {
    use ratatui::widgets::Clear;

    if area.width < 30 || area.height < 9 {
        return;
    }
    let width = area.width.saturating_sub(4).min(100);
    // Fit the window to its content on the Fixes tab (tab row + list + detail + buttons + border);
    // the Explain tab is long-form text and takes the full height.
    let wanted = if state.advice.tab == AdviceTab::Fixes {
        (state.model.recommendations.len().clamp(1, 10) as u16) + 18
    } else {
        28
    };
    let height = area.height.saturating_sub(2).min(wanted.max(12));
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(true))
        .title(" Fixes & explanations \u{b7} Up/Down select \u{b7} Enter apply \u{b7} Tab switch \u{b7} Esc close ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height < 5 || inner.width < 10 {
        return;
    }
    regions.advice_window = Some(popup);

    let model = &state.model;
    let explanation = advice::explain_tolerance(model);
    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).split(inner);
    let (tab_row, body, button_row) = (rows[0], rows[1], rows[2]);

    // Tabs.
    let mut tabs: Vec<(String, AdviceTab)> = vec![(format!(" Recommended fixes ({}) ", model.recommendations.len()), AdviceTab::Fixes)];
    if explanation.is_some() {
        let label = if model.output.tolerance_status == ToleranceStatus::Infeasible { " Why is the tolerance infeasible? " } else { " Why is the OD clamped? " };
        tabs.push((label.to_string(), AdviceTab::Explain));
    }
    let mut spans = Vec::new();
    let mut x = tab_row.x;
    for (label, tab) in tabs {
        let w = label.chars().count() as u16;
        if x + w > tab_row.x + tab_row.width {
            break;
        }
        let style = if state.advice.tab == tab { theme.selected_row_style().add_modifier(Modifier::BOLD) } else { theme.disabled_style() };
        regions.bushing_actions.push((Rect { x, y: tab_row.y, width: w, height: 1 }, BushingAction::AdviceTab(tab)));
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
        x += w + 1;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), tab_row);

    let tab = if explanation.is_none() { AdviceTab::Fixes } else { state.advice.tab };
    match tab {
        AdviceTab::Fixes => {
            if model.recommendations.is_empty() {
                frame.render_widget(Paragraph::new(Line::from(Span::styled("No failing checks - nothing to fix.", theme.status_style(StatusTone::Success)))), body);
            } else {
                let detail_height = 13.min(body.height.saturating_sub(2));
                let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(detail_height)]).split(body);
                let selected = state.rec_selected.min(model.recommendations.len() - 1);
                let items: Vec<ListItem> = model
                    .recommendations
                    .iter()
                    .enumerate()
                    .map(|(i, rec)| {
                        let marker = if i == selected { "> " } else { "  " };
                        let style = if i == selected { theme.selected_row_style() } else { Style::default() };
                        let bore = if !rec.is_applicable() { "" } else if rec.touches_bore { "  [CHANGES BORE]" } else { "  [bore unchanged]" };
                        ListItem::new(Line::from(Span::styled(format!("{marker}{}. [{}] {}{bore}{}", i + 1, rec.fixes.label(), rec.summary, if rec.is_applicable() { "" } else { "  (manual)" }), style)))
                    })
                    .collect();
                let offset = crate::widgets::scroll_list::render(frame, parts[0], items, Some(selected));
                for (rect, i) in crate::mouse::list_row_regions(parts[0], offset, model.recommendations.len()) {
                    regions.bushing_actions.push((rect, BushingAction::AdviceRow(i)));
                }
                let rec = &model.recommendations[selected];
                let mut detail = vec![
                    Line::from(Span::styled(format!("Fixes: {}", rec.fixes.label()), theme.title_style(false))),
                    Line::from(format!("Change: {}", rec.summary)),
                    Line::from(format!("Solver result with this change: {}", rec.outcome)),
                ];
                if let Some(note) = &rec.note {
                    detail.push(Line::from(Span::styled(format!("Note: {note}"), theme.status_style(StatusTone::Info))));
                }
                if !rec.impact.is_empty() {
                    detail.push(Line::from(Span::styled("Impact of applying it:", theme.title_style(false))));
                    for line in &rec.impact {
                        let style = if line.starts_with("Warning") { theme.status_style(StatusTone::Warning) } else { Style::default() };
                        detail.push(Line::from(Span::styled(format!("  \u{2022} {line}"), style)));
                    }
                }
                if !rec.is_applicable() {
                    detail.push(Line::from(Span::styled("Manual change - cannot be applied automatically.", theme.status_style(StatusTone::Warning))));
                }
                if let Some(applied) = &state.last_applied {
                    detail.push(Line::from(Span::styled(format!("\u{2713} Applied: {applied}"), theme.status_style(StatusTone::Success))));
                }
                frame.render_widget(Paragraph::new(detail).wrap(Wrap { trim: true }), parts[1]);
            }
        }
        AdviceTab::Explain => {
            let mut lines: Vec<Line> = Vec::new();
            for (i, para) in explanation.unwrap_or_default().into_iter().enumerate() {
                // Paragraphs alternate heading / body (see `explain_tolerance`).
                if i % 2 == 0 {
                    if i > 0 {
                        lines.push(Line::from(""));
                    }
                    lines.push(Line::from(Span::styled(para, theme.title_style(true).add_modifier(Modifier::BOLD))));
                } else {
                    lines.push(Line::from(para));
                }
            }
            crate::widgets::scroll_paragraph::render(frame, body, theme, lines, state.advice.scroll);
        }
    }

    // Buttons.
    let mut buttons: Vec<(&str, BushingAction, Style)> = Vec::new();
    let can_apply = tab == AdviceTab::Fixes && model.recommendations.get(state.rec_selected.min(model.recommendations.len().saturating_sub(1))).map(|r| r.is_applicable()).unwrap_or(false);
    if can_apply {
        buttons.push((" Apply selected ", BushingAction::AdviceApply, theme.status_style(StatusTone::Success).add_modifier(Modifier::REVERSED | Modifier::BOLD)));
    }
    buttons.push((" Close ", BushingAction::AdviceClose, Style::default().add_modifier(Modifier::REVERSED)));
    let mut spans = Vec::new();
    let mut x = button_row.x;
    for (label, action, style) in buttons {
        let w = label.chars().count() as u16;
        if x + w > button_row.x + button_row.width {
            break;
        }
        regions.bushing_actions.push((Rect { x, y: button_row.y, width: w, height: 1 }, action));
        spans.push(Span::styled(label, style));
        spans.push(Span::raw("  "));
        x += w + 2;
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), button_row);
}

/// A result line for a check: plain when it passes, otherwise the whole line
/// (name and value) in the failure/warning colour with a leading marker that
/// keeps the column alignment of the two-space indent it replaces.
fn flag_line<'a>(theme: &Theme, severity: Severity, text: String) -> Line<'a> {
    let body = text.strip_prefix("  ").unwrap_or(&text).to_string();
    match severity {
        Severity::Fail => Line::from(Span::styled(format!("\u{2717} {body}"), theme.status_style(StatusTone::Danger).add_modifier(Modifier::BOLD))),
        Severity::Warn => Line::from(Span::styled(format!("! {body}"), theme.status_style(StatusTone::Warning))),
        Severity::Pass => Line::from(text),
    }
}

/// Pushes a (possibly flagged) check line; a flagged one is also made
/// clickable (`action`) and gets a trailing `▸` hint.
fn push_check<'a>(theme: &Theme, lines: &mut Vec<Line<'a>>, tags: &mut Vec<(usize, BushingAction)>, severity: Severity, action: BushingAction, text: String) {
    if severity == Severity::Pass {
        lines.push(flag_line(theme, severity, text));
        return;
    }
    tags.push((lines.len(), action));
    lines.push(flag_line(theme, severity, format!("{text}  \u{25b8}")));
}

/// The scrollable readout plus, for each clickable line, `(line index, action)`.
fn readout_lines<'a>(
    theme: &'a Theme,
    model: &'a BushingModel,
    show_numbers: bool,
    last_applied: Option<&str>,
    input_error: Option<&str>,
    edge_section: (Vec<Line<'a>>, Vec<(usize, super::edge_check::EdgeTopic)>),
    advisories: Vec<super::edge_check::Advisory>,
) -> (Vec<Line<'a>>, Vec<(usize, BushingAction)>) {
    let out = &model.output;
    let mut lines = Vec::new();
    let mut tags: Vec<(usize, BushingAction)> = Vec::new();
    let explain = advice::explain_tolerance(model).is_some();
    let fixes_for = |kind: CheckKind| BushingAction::OpenFixesFor(kind);
    let tolerance_action = if explain { BushingAction::OpenExplain } else { BushingAction::OpenFixesFor(CheckKind::Tolerance) };
    let sev = |kind: CheckKind| advice::severity_of(&model.checks, kind);
    let failing: Vec<&advice::Check> = model.checks.iter().filter(|c| c.severity == Severity::Fail).collect();

    let (headline, tone) = if failing.is_empty() { ("PASS", StatusTone::Success) } else { ("REVIEW", StatusTone::Danger) };
    lines.push(Line::from(vec![
        Span::styled(headline, theme.status_style(tone).add_modifier(Modifier::BOLD)),
        Span::raw(format!("  governing: {} ({})", out.governing.name, fmt_margin(out.governing.margin))),
        if failing.is_empty() { Span::raw("") } else { Span::styled(format!("  {} check(s) failing", failing.len()), theme.status_style(StatusTone::Danger)) },
    ]));
    // The Tolerance check has its own (clickable) line further down.
    for check in model.checks.iter().filter(|c| c.severity != Severity::Pass && c.kind != CheckKind::Tolerance) {
        push_check(theme, &mut lines, &mut tags, check.severity, fixes_for(check.kind), format!("  {}: {}", check.kind.label(), check.detail));
    }
    for adv in &advisories {
        push_check(theme, &mut lines, &mut tags, Severity::Warn, BushingAction::EdgeInfo(adv.topic), format!("  {}", adv.text));
    }
    if let Some(err) = input_error {
        lines.push(Line::from(Span::styled(format!("\u{2717} {err}"), theme.status_style(StatusTone::Danger))));
    }
    if let Some(applied) = last_applied {
        lines.push(Line::from(Span::styled(format!("\u{2713} Applied: {applied}"), theme.status_style(StatusTone::Success))));
    }
    push_check(
        theme,
        &mut lines,
        &mut tags,
        sev(CheckKind::Tolerance),
        tolerance_action,
        format!(
            "  Tolerance: {} ({} note(s))",
            match out.tolerance_status {
                ToleranceStatus::Ok => "OK",
                ToleranceStatus::Clamped => "Clamped",
                ToleranceStatus::Infeasible => "INFEASIBLE",
            },
            out.tolerance_notes.len()
        ),
    );
    for note in &out.tolerance_notes {
        if explain {
            tags.push((lines.len(), BushingAction::OpenExplain));
        }
        lines.push(Line::from(Span::styled(format!("  \u{26a0} {note}{}", if explain { "  \u{25b8} why?" } else { "" }), theme.status_style(StatusTone::Warning))));
    }
    if matches!(model.fit_type, model::FitType::Clearance | model::FitType::Slip) && model.interference > 0.0 {
        lines.push(Line::from(Span::styled(
            format!("  \u{26a0} Fit Type is {} but Target Interference is positive ({:.4} in) - informational only, not blocked.", model::label_fit_type(model.fit_type), model.interference),
            theme.status_style(StatusTone::Warning),
        )));
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Fit", theme.title_style(false))));
    lines.push(Line::from(format!("  OD installed        {:.4} in  (range {:.4}..{:.4})", out.od_installed, out.od_tol.lower, out.od_tol.upper)));
    lines.push(Line::from(format!(
        "  Interference        target {:.4} in, achieved {:.4} in (range {:.4}..{:.4})",
        model.interference, out.delta_total, out.achieved_interference_tol.lower, out.achieved_interference_tol.upper
    )));
    // Interference change caused by temperature (differential expansion of the two materials).
    lines.push(Line::from(format!(
        "  \u{394} interference, service \u{394}T {:+.1} \u{b0}F  {:+.5} in  \u{2192} in service {:.5} in",
        model.delta_t, out.delta_thermal + 0.0, out.delta_total
    )));
    if model.assembly_thermal_enabled {
        lines.push(Line::from(format!(
            "  \u{394} interference, install thermal assist  {:+.5} in  \u{2192} at install {:.5} in",
            out.assembly_thermal_delta + 0.0, out.install_delta
        )));
    }
    push_check(
        theme,
        &mut lines,
        &mut tags,
        sev(CheckKind::StraightWall),
        fixes_for(CheckKind::StraightWall),
        format!(
            "  Straight wall       {:.4} in  ({}, range {:.4}..{:.4})",
            out.wall_straight,
            if out.fail_straight { "FAIL vs min" } else { "OK" },
            out.wall_straight_range.min,
            out.wall_straight_range.max
        ),
    );
    push_check(theme, &mut lines, &mut tags, sev(CheckKind::NeckWall), fixes_for(CheckKind::NeckWall), format!("  Neck wall           {:.4} in  ({})", out.wall_neck, if out.fail_neck { "FAIL vs min" } else { "OK" }));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Contact / Stress", theme.title_style(false))));
    lines.push(Line::from(format!("  Contact pressure    {:.0} psi  (range {:.0}..{:.0})", out.pressure, out.pressure_range.min, out.pressure_range.max)));
    for (kind, name, stress, ms) in [
        (CheckKind::HousingStress, "Housing", out.stress_hoop_housing, out.housing_ms),
        (CheckKind::BushingStress, "Bushing", out.stress_hoop_bushing, out.bushing_ms),
    ] {
        if sev(kind) == Severity::Fail {
            push_check(theme, &mut lines, &mut tags, Severity::Fail, fixes_for(kind), format!("  {name} hoop stress {stress:>10.0} psi  MS {}", fmt_margin(ms)));
        } else {
            lines.push(Line::from(vec![
                Span::raw(format!("  {name} hoop stress {stress:>10.0} psi  MS ")),
                Span::styled(fmt_margin(ms), Style::default().fg(tone_color(tone_for_margin(ms)))),
            ]));
        }
    }
    if out.axial_constraint_factor > 0.0 {
        lines.push(Line::from(format!("  Housing axial stress {:>9.0} psi", out.stress_axial_housing)));
        lines.push(Line::from(format!("  Bushing axial stress {:>9.0} psi", out.stress_axial_bushing)));
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Install", theme.title_style(false))));
    lines.push(Line::from(format!("  Install force       {:.1} lbf  (range {:.1}..{:.1})", out.install_force, out.install_force_range.min, out.install_force_range.max)));
    lines.push(Line::from(format!("  Retained (in-service) force {:.1} lbf", out.retained_install_force)));
    lines.push(Line::from(""));

    tags.push((lines.len(), BushingAction::EdgeInfo(super::edge_check::EdgeTopic::Legacy)));
    lines.push(Line::from(Span::styled("Edge Distance (legacy check; hover for details)", theme.title_style(false))));
    lines.push(Line::from(format!("  Actual e/D          {:.3}", out.ed_actual)));
    for (kind, label, min, margin) in [
        (CheckKind::EdgeSequencing, "Sequencing margin ", out.ed_min_sequence, out.sequence_margin),
        (CheckKind::EdgeStrength, "Strength margin   ", out.ed_min_strength, out.strength_margin),
    ] {
        if sev(kind) == Severity::Fail {
            push_check(theme, &mut lines, &mut tags, Severity::Fail, fixes_for(kind), format!("  {label} min {min:.3}  actual/min {}", fmt_margin(margin)));
        } else {
            lines.push(Line::from(vec![
                Span::raw(format!("  {label} min {min:.3}  actual/min ")),
                Span::styled(fmt_margin(margin), Style::default().fg(tone_color(tone_for_margin(margin)))),
            ]));
        }
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Governing Candidates", theme.title_style(false))));
    for c in &out.candidates {
        let kind_sev = CheckKind::from_candidate_name(c.name).map(sev).unwrap_or(Severity::Pass);
        if kind_sev == Severity::Fail {
            if let Some(k) = CheckKind::from_candidate_name(c.name) {
                push_check(theme, &mut lines, &mut tags, Severity::Fail, fixes_for(k), format!("  {:<28}{}", c.name, fmt_margin(c.margin)));
            }
        } else {
            lines.push(Line::from(vec![
                Span::raw(format!("  {:<28}", c.name)),
                Span::styled(fmt_margin(c.margin), Style::default().fg(tone_color(tone_for_margin(c.margin)))),
            ]));
        }
    }

    lines.push(Line::from(""));
    let (edge_lines, edge_tags) = edge_section;
    let base = lines.len();
    lines.extend(edge_lines);
    tags.extend(edge_tags.into_iter().map(|(i, t)| (base + i, BushingAction::EdgeInfo(t))));

    if show_numbers {
        lines.push(Line::from(""));
        lines.extend(numbers_panel_lines(theme, model));
    }

    (lines, tags)
}

/// Text-only partial substitute for the cross-section sketch neither GUI
/// head's own port carries over here (see this toolbox's `mod.rs` doc
/// comment) - the full per-radius hoop/radial/axial stress field for both
/// the bushing and the housing, toggled by `d`.
fn numbers_panel_lines<'a>(theme: &'a Theme, model: &'a BushingModel) -> Vec<Line<'a>> {
    let out = &model.output;
    let mut lines = vec![Line::from(Span::styled("Numbers (d to hide)", theme.title_style(false)))];
    lines.push(Line::from(format!(
        "  psi (finite-plate factor) = {:.4}, lambda = {:.4}, d_equivalent = {:.4} in",
        out.psi, out.lambda, out.d_equivalent
    )));
    lines.push(Line::from(format!("  term_b (bushing compliance) = {:.3e} in/psi, term_h (housing compliance) = {:.3e} in/psi", out.term_b, out.term_h)));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(format!("Bushing stress field ({} samples)", out.bushing_stress_field.len()), theme.title_style(false))));
    lines.extend(field_sample_lines(&out.bushing_stress_field, 4));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!("Housing stress field ({} samples)", out.housing_stress_field.len()), theme.title_style(false))));
    lines.extend(field_sample_lines(&out.housing_stress_field, 4));

    lines
}

/// Shows a small, evenly-spaced subset (inner/outer boundary plus interior
/// points) rather than every one of ~41 raw samples - the full field is
/// meant for numerical/engineering inspection, not a wall of nearly-identical
/// rows in a fixed-height terminal pane.
fn field_sample_lines(field: &[mechanics_core::lame::LameSample], count: usize) -> Vec<Line<'static>> {
    if field.is_empty() {
        return vec![Line::from("  (no samples)".to_string())];
    }
    let step = ((field.len() - 1).max(1)) as f64 / (count.max(1) - 1).max(1) as f64;
    (0..count.max(1))
        .map(|i| {
            let idx = ((i as f64 * step).round() as usize).min(field.len() - 1);
            let s = &field[idx];
            Line::from(format!("  r={:.4} in  sigma_r={:>9.0}  sigma_theta={:>9.0}  sigma_axial={:>9.0} psi", s.r, s.sigma_r, s.sigma_theta, s.sigma_axial))
        })
        .collect()
}

/// Plain-text export - mirrors `pressure_vessel/view.rs::build_report_text`'s
/// pattern: pure and synchronous, no filesystem access, so `mod.rs`'s `e`
/// key can build it directly and hand the string to `main.rs` via
/// `Effect::ExportBushingReport`.
pub fn build_report_text(model: &BushingModel) -> String {
    let out = &model.output;
    let mut s = String::new();
    s.push_str("Bushing Workbench Report\n");
    s.push_str("========================\n\n");
    s.push_str(&format!("OD geometry:      {}\n", model::label_bushing_type(model.bushing_type)));
    s.push_str(&format!("ID geometry:      {}\n", model::label_id_type(model.id_type)));
    s.push_str(&format!("Housing material: {}\n", model.housing_material().name));
    s.push_str(&format!("Bushing material: {}\n", model.bushing_material().name));
    s.push_str(&format!("Bore diameter:    {:.4} in\n", model.bore_dia));
    s.push_str(&format!("Bushing ID:       {:.4} in\n", model.id_bushing));
    s.push_str(&format!("Target interference: {:.4} in\n", model.interference));
    s.push_str(&format!("Delta interference, service temp change {:+.1} F: {:+.5} in (in service {:.5} in)\n", model.delta_t, out.delta_thermal, out.delta_total));
    if model.assembly_thermal_enabled {
        s.push_str(&format!("Delta interference, install thermal assist: {:+.5} in (at install {:.5} in)\n", out.assembly_thermal_delta, out.install_delta));
    }
    s.push('\n');

    s.push_str(&format!("OD installed:     {:.4} in\n", out.od_installed));
    s.push_str(&format!("Straight wall:    {:.4} in ({})\n", out.wall_straight, if out.fail_straight { "FAIL" } else { "OK" }));
    s.push_str(&format!("Neck wall:        {:.4} in ({})\n", out.wall_neck, if out.fail_neck { "FAIL" } else { "OK" }));
    s.push_str(&format!("Contact pressure: {:.0} psi\n", out.pressure));
    s.push_str(&format!("Housing hoop stress: {:.0} psi, MS {}\n", out.stress_hoop_housing, fmt_margin(out.housing_ms)));
    s.push_str(&format!("Bushing hoop stress: {:.0} psi, MS {}\n", out.stress_hoop_bushing, fmt_margin(out.bushing_ms)));
    s.push_str(&format!("Install force:    {:.1} lbf\n", out.install_force));
    s.push_str(&format!("Retained force:   {:.1} lbf\n\n", out.retained_install_force));

    s.push_str(&format!("Governing: {} ({})\n", out.governing.name, fmt_margin(out.governing.margin)));
    s.push_str("Candidates:\n");
    for c in &out.candidates {
        s.push_str(&format!("  {:<28} {}\n", c.name, fmt_margin(c.margin)));
    }
    s.push('\n');
    let failing: Vec<_> = model.checks.iter().filter(|c| c.severity != Severity::Pass).collect();
    if !failing.is_empty() {
        s.push_str("Checks needing attention:\n");
        for c in failing {
            s.push_str(&format!("  [{}] {}: {}\n", if c.severity == Severity::Fail { "FAIL" } else { "WARN" }, c.kind.label(), c.detail));
        }
        for (i, r) in model.recommendations.iter().enumerate() {
            s.push_str(&format!("  recommendation {}: {} ({})\n", i + 1, r.summary, r.outcome));
        }
        s.push('\n');
    }
    s.push_str(&format!(
        "Tolerance status: {:?}\n",
        out.tolerance_status
    ));
    for note in &out.tolerance_notes {
        s.push_str(&format!("  note: {note}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn draw_at(width: u16, height: u16, state: &BushingState) {
        let backend = TestBackend::new(width.max(1), height.max(1));
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, area, &Theme::default_palette(), state, true, &mut regions)).unwrap();
    }

    #[test]
    fn draw_does_not_panic_at_normal_width() {
        draw_at(160, 40, &BushingState::default());
    }

    #[test]
    fn the_edge_check_section_renders_before_and_after_a_run_at_any_width() {
        for width in [40u16, 60, 100, 160] {
            let mut state = BushingState::default();
            draw_at(width, 40, &state); // "not run" hint
            state.run_edge_check(false);
            assert!(state.edge_check.is_some());
            draw_at(width, 40, &state);
            state.model.commit_number(model::NumberTarget::EdgeDist, 1.0); // now stale
            draw_at(width, 40, &state);
        }
    }

    #[test]
    fn the_results_text_shows_the_cross_check_and_flags_it_stale_after_an_edit() {
        let theme = Theme::default_palette();
        let flat = |lines: &[Line]| lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect::<Vec<_>>().join("\n");
        let mut state = BushingState::default();
        assert!(flat(&super::super::edge_check::section_lines(&theme, None, false, &state.model).0).contains("Not run"));
        state.run_edge_check(false);
        let fresh = flat(&super::super::edge_check::section_lines(&theme, state.edge_check.as_ref(), false, &state.model).0);
        assert!(fresh.contains("Superposition") && fresh.contains("Allowables") && fresh.contains("Legacy (solver)"), "{fresh}");
        assert!(!fresh.contains("inputs changed"));
        let stale = flat(&super::super::edge_check::section_lines(&theme, state.edge_check.as_ref(), true, &state.model).0);
        assert!(stale.contains("inputs changed"));
    }

    #[test]
    fn hovering_or_pinning_a_check_name_shows_its_strengths_weaknesses_and_restrictions() {
        use super::super::edge_check::EdgeTopic;
        let mut state = BushingState::default();
        state.run_edge_check(true);
        assert!(!rendered_text(&state, 160, 50).contains("Weaknesses"), "no tooltip until hovered");
        state.edge_hover = Some((EdgeTopic::ContactFe, 100, 20));
        let text = rendered_text(&state, 160, 50);
        for needle in ["Contact FE", "Strengths", "Weaknesses", "Restrictions", "plane strain"] {
            assert!(text.contains(needle), "hover tooltip lacks {needle:?}");
        }
        state.edge_hover = None;
        state.perform(BushingAction::EdgeInfo(EdgeTopic::Legacy));
        assert!(rendered_text(&state, 160, 50).contains("Legacy edge-distance check"));
        state.perform(BushingAction::EdgeInfo(EdgeTopic::Legacy));
        assert!(state.edge_info_pinned.is_none(), "clicking again unpins");
    }

    #[test]
    fn every_topic_tooltip_has_the_four_sections_and_fits_a_small_pane() {
        use super::super::edge_check::{tip, EdgeTopic};
        for t in [EdgeTopic::Legacy, EdgeTopic::StressSuperposition, EdgeTopic::Allowables, EdgeTopic::PlasticFe, EdgeTopic::ContactFe] {
            let headings: Vec<&str> = tip(t).sections.iter().map(|(h, _)| *h).collect();
            assert_eq!(headings, ["What it does", "Strengths", "Weaknesses", "Restrictions"], "{t:?}");
            let mut state = BushingState::default();
            state.edge_info_pinned = Some(t);
            for (w, h) in [(30u16, 12u16), (60, 20), (160, 50)] {
                let _ = rendered_text(&state, w, h); // must not panic at any size
            }
        }
    }

    #[test]
    fn a_long_material_name_wraps_instead_of_widening_the_inputs_pane() {
        let base = BushingState::default();
        let mut long = BushingState::default();
        let idx = long.model.material_catalog().iter().position(|m| m.name.len() > 55).expect("handbook has long names");
        long.model.housing_material_index = idx;
        long.model.recompute();
        assert_eq!(fields_required_width(&long), fields_required_width(&base), "pane width must not depend on the longest material name");
        let text = rendered_text(&long, 150, 45);
        let name = long.model.housing_material().name;
        let first_word = name.split(' ').next().unwrap();
        assert!(text.contains(first_word), "the name is shown (wrapped):\n{text}");
        // The label sits on its own row with the value on the rows below it.
        let rows: Vec<&str> = text.lines().collect();
        let i = rows.iter().position(|r| r.contains("Housing Material")).expect("label row");
        assert!(!rows[i].contains(first_word), "value moved off the label row");
        assert!(rows[i + 1].contains(first_word), "value starts on the next row");
    }

    #[test]
    fn clicking_the_wrapped_second_row_of_a_field_still_selects_that_field() {
        let mut state = BushingState::default();
        let idx = state.model.material_catalog().iter().position(|m| m.name.len() > 55).unwrap();
        state.model.housing_material_index = idx;
        state.model.recompute();
        let backend = TestBackend::new(150, 45);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, Rect::new(0, 0, 150, 45), &Theme::default_palette(), &state, true, &mut regions)).unwrap();
        let rows = model::field_rows(&state.model);
        let target = rows.iter().position(|r| *r == FieldRow::OpenHousingMaterialPicker).unwrap();
        let (rect, _) = regions.bushing_rows.iter().find(|(_, i)| *i == target).unwrap();
        assert!(rect.height >= 2, "the wrapped row owns every row it occupies: {rect:?}");
        assert_eq!(crate::mouse::hit(&regions.bushing_rows, rect.x + 3, rect.y + rect.height - 1), Some(target));
    }

    #[test]
    fn the_action_bar_offers_an_edge_check_button() {
        let backend = TestBackend::new(160, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        let state = BushingState::default();
        terminal.draw(|f| draw(f, Rect::new(0, 0, 160, 40), &Theme::default_palette(), &state, true, &mut regions)).unwrap();
        assert!(regions.bushing_actions.iter().any(|(_, a)| *a == BushingAction::EdgeCheck));
    }

    /// Regression test for the hint-truncation bug: the bottom Hint panel
    /// used to be a hardcoded `Constraint::Length(2)` regardless of content,
    /// silently clipping any hint whose wrapped text needed more than two
    /// lines. Selects the field with the single longest `field_hint()`
    /// string and asserts every word of it appears somewhere in the
    /// rendered buffer, across a range of widths including narrow ones.
    #[test]
    fn the_longest_hint_is_never_truncated_across_a_range_of_widths() {
        let rows = model::field_rows(&BushingState::default().model);
        let (longest_index, longest_hint) = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (i, model::field_hint(*r)))
            .max_by_key(|(_, hint)| hint.len())
            .expect("at least one field row");
        assert!(!longest_hint.is_empty());

        for width in [40u16, 50, 60, 84, 98, 140] {
            let mut state = BushingState::default();
            state.selected = longest_index;
            let backend = TestBackend::new(width, 40);
            let mut terminal = Terminal::new(backend).unwrap();
            let area = Rect::new(0, 0, width, 40);
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| draw(f, area, &Theme::default_palette(), &state, true, &mut regions)).unwrap();

            let buffer = terminal.backend().buffer().clone();
            let rendered: String = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n").collect();

            for word in longest_hint.split_whitespace() {
                assert!(rendered.contains(word), "word `{word}` from the longest hint missing at width {width}:\n{rendered}");
            }
        }
    }

    #[test]
    fn draw_does_not_panic_at_narrow_width() {
        draw_at(50, 40, &BushingState::default());
    }

    #[test]
    fn draw_does_not_panic_at_degenerate_sizes() {
        for (w, h) in [(0, 0), (1, 1), (40, 0), (0, 10)] {
            draw_at(w, h, &BushingState::default());
        }
    }

    #[test]
    fn draw_does_not_panic_with_countersink_geometry_and_numbers_panel() {
        let mut state = BushingState::default();
        state.model.id_type = bushing_solver::geometry::IdType::Countersink;
        state.model.bushing_type = bushing_solver::geometry::BushingType::Countersink;
        state.model.recompute();
        state.show_numbers = true;
        draw_at(160, 50, &state);
        draw_at(50, 50, &state);
    }

    #[test]
    fn draw_does_not_panic_with_the_material_or_reamer_picker_open() {
        let mut state = BushingState::default();
        state.material_picker = super::super::material_picker::MaterialPickerState::open_for(super::super::material_picker::MaterialTarget::Housing);
        draw_at(120, 40, &state);
        state.material_picker.open = false;
        state.reamer_picker.open = true;
        draw_at(120, 40, &state);
    }

    /// Regression test for the Results-pane clipping bug: `draw_readout`
    /// used to render via a bare `Paragraph::new(lines)` with no `.wrap()`
    /// and no scroll, so content taller than the pane (e.g. the Numbers
    /// panel's full stress-field breakdown) was silently dropped off the
    /// bottom with no way to reach it. Forces a short terminal height with
    /// the Numbers panel on, scrolls to the bottom via a very large
    /// PageDown-equivalent offset (which `scroll_paragraph::render` must
    /// clamp, not blank out), and asserts the last readout line is visible.
    #[test]
    fn scrolling_the_results_pane_reaches_content_past_a_short_pane_height() {
        let mut state = BushingState::default();
        state.show_numbers = true;
        state.results_scroll = u16::MAX;
        draw_at(160, 12, &state);
        // Re-render into a buffer we can inspect directly.
        let backend = TestBackend::new(160, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 160, 12);
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, area, &Theme::default_palette(), &state, true, &mut regions)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rendered: String = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n").collect();
        assert!(rendered.contains("Housing stress field"), "last section of the readout must be reachable by scrolling, not silently dropped:\n{rendered}");
    }

    #[test]
    fn clearance_fit_with_positive_interference_shows_a_non_blocking_warning() {
        let mut state = BushingState::default();
        state.model.fit_type = model::FitType::Clearance;
        state.model.recompute();
        let backend = TestBackend::new(160, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 160, 40);
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, area, &Theme::default_palette(), &state, true, &mut regions)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rendered: String = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n").collect();
        assert!(rendered.contains("Clearance Fit"), "warning must name the selected fit type:\n{rendered}");
    }

    #[test]
    fn build_report_text_mentions_the_governing_mode() {
        let model = BushingModel::default();
        let text = build_report_text(&model);
        assert!(text.contains("Governing:"));
        assert!(text.contains(model.output.governing.name));
    }

    fn rendered_text(state: &BushingState, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, area, &Theme::default_palette(), state, true, &mut regions)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n").collect()
    }

    #[test]
    fn a_failing_check_is_marked_and_the_fixes_button_is_shown() {
        let mut state = BushingState::default();
        state.model.commit_number(model::NumberTarget::MinWallStraight, 0.2);
        let text = rendered_text(&state, 170, 60);
        assert!(text.contains("\u{2717} Straight wall"), "failing straight wall line must carry the fail marker:\n{text}");
        assert!(text.contains("REVIEW"));
        assert!(text.contains("Fixes ("), "action bar shows the Fixes button:\n{text}");
        assert!(!text.contains("Recommendations"), "recommendations live in the pop-up window, not the results pane:\n{text}");
        assert!(text.contains("\u{25b8}"), "failing lines carry the click hint");
    }

    #[test]
    fn failing_check_name_and_value_are_drawn_in_the_danger_colour() {
        let mut state = BushingState::default();
        state.model.commit_number(model::NumberTarget::MinWallStraight, 0.2);
        let backend = TestBackend::new(170, 60);
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, 170, 60);
        let mut regions = crate::mouse::MouseRegions::default();
        let theme = Theme::default_palette();
        terminal.draw(|f| draw(f, area, &theme, &state, true, &mut regions)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let danger = theme.status_style(StatusTone::Danger).fg;
        let mut found = false;
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect();
            if let Some(col) = row.find("Straight wall       ") {
                // `find` yields a byte index; this row is ASCII up to the marker column's neighbours.
                let x = row[..col].chars().count() as u16;
                let name_fg = buffer[(x, y)].fg;
                let value_fg = buffer[(x + 20, y)].fg;
                assert_eq!(Some(name_fg), danger, "name must be highlighted");
                assert_eq!(Some(value_fg), danger, "value must be highlighted");
                found = true;
            }
        }
        assert!(found, "straight wall result line not found");
    }

    #[test]
    fn passing_results_carry_no_failure_marker() {
        let mut state = BushingState::default();
        state.model.commit_number(model::NumberTarget::EdgeDist, 5.0);
        let text = rendered_text(&state, 170, 60);
        assert!(!text.contains('\u{2717}'));
        assert!(text.contains("PASS"));
    }

    #[test]
    fn the_fixes_window_lists_recommendations_and_the_selected_ones_detail() {
        let mut state = BushingState::default();
        state.model.commit_number(model::NumberTarget::MinWallStraight, 0.2);
        state.perform(BushingAction::OpenFixes);
        let text = rendered_text(&state, 150, 50);
        assert!(text.contains("Fixes & explanations"));
        assert!(text.contains("Recommended fixes ("));
        assert!(text.contains("Bushing ID"));
        assert!(text.contains("Solver result with this change"));
        assert!(text.contains("Apply selected") && text.contains("Close"));
    }

    #[test]
    fn the_explain_tab_describes_why_the_od_is_clamped_with_the_users_numbers() {
        let mut state = BushingState::default();
        state.model.commit_number(model::NumberTarget::BoreTolPlus, 0.002);
        state.model.commit_number(model::NumberTarget::InterferenceTolPlus, 0.001);
        state.model.commit_number(model::NumberTarget::InterferenceTolMinus, 0.001);
        state.perform(BushingAction::OpenExplain);
        let text = rendered_text(&state, 150, 50);
        assert!(text.contains("Why is the OD clamped?"));
        assert!(text.contains("Why it is CLAMPED"));
        assert!(text.contains("lopsided toward the large side"), "{text}");
        assert!(text.contains("Inputs that cause it"));
    }

    #[test]
    fn results_action_bar_regions_exist_and_only_the_window_owns_the_mouse_while_open() {
        let mut state = BushingState::default();
        state.model.commit_number(model::NumberTarget::MinWallStraight, 0.2);
        let backend = TestBackend::new(170, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &state, true, &mut regions)).unwrap();
        for wanted in [BushingAction::OpenFixes, BushingAction::ToggleNumbers, BushingAction::Export, BushingAction::OpenFixesFor(CheckKind::StraightWall)] {
            assert!(regions.bushing_actions.iter().any(|(_, a)| *a == wanted), "{wanted:?} must be clickable");
        }
        state.perform(BushingAction::OpenFixes);
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &state, true, &mut regions)).unwrap();
        assert!(regions.advice_window.is_some());
        assert!(regions.bushing_actions.iter().all(|(_, a)| matches!(a, BushingAction::AdviceTab(_) | BushingAction::AdviceRow(_) | BushingAction::AdviceApply | BushingAction::AdviceClose)));
    }

    #[test]
    fn the_fixes_window_survives_degenerate_sizes() {
        let mut state = BushingState::default();
        state.model.commit_number(model::NumberTarget::MinWallStraight, 0.2);
        state.perform(BushingAction::OpenFixes);
        for (w, h) in [(0, 0), (20, 6), (40, 12), (60, 20), (200, 80)] {
            draw_at(w, h, &state);
        }
    }

    #[test]
    fn results_show_delta_interference_from_service_temperature_and_install_thermal_assist() {
        let mut state = BushingState::default();
        state.model.commit_number(model::NumberTarget::DeltaT, 100.0);
        let text = rendered_text(&state, 190, 60);
        assert!(text.contains("service \u{394}T +100.0"), "{text}");
        assert!(!text.contains("install thermal assist"));
        state.model.toggle_assembly_thermal_enabled();
        state.model.commit_number(model::NumberTarget::AssemblyBushingTemp, -100.0);
        let text = rendered_text(&state, 190, 60);
        assert!(text.contains("install thermal assist"), "{text}");
        let report = build_report_text(&state.model);
        assert!(report.contains("Delta interference, service temp change") && report.contains("install thermal assist"));
    }

    #[test]
    fn the_fixes_window_shows_impact_bore_tag_and_catalog_note() {
        let mut state = BushingState::default();
        state.model.commit_number(model::NumberTarget::MinWallStraight, 0.2);
        state.perform(BushingAction::OpenFixes);
        let text = rendered_text(&state, 190, 60);
        assert!(text.contains("[bore unchanged]"), "{text}");
        assert!(text.contains("Impact of applying it"), "{text}");
        assert!(text.contains("Bore unchanged - no re-reaming"));
    }
}
