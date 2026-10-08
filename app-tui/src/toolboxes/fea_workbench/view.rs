//! Rendering for the FEA Workbench: the field list beside a canvas (mesh preview or result contour)
//! and a results readout.

use fea_problem::raster::rasterize;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::canvas;
use super::model::{self, EditKind, FieldRow};
use super::{FeaWorkbenchState, RasterKey};
use crate::theme::{StatusTone, Theme};

const MIN_READOUT_WIDTH: u16 = 44;

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, state: &FeaWorkbenchState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(" FEA Workbench - r: solve \u{b7} v: field \u{b7} m: mesh \u{b7} x: deform \u{b7} d: details \u{b7} j: save \u{b7} o: open \u{b7} e: report \u{b7} p: .vtu \u{b7} c: .csv ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let required = fields_required_width(state);
    let (fields_area, right_area) = if inner.width >= required + MIN_READOUT_WIDTH {
        let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Length(required), Constraint::Min(MIN_READOUT_WIDTH)]).split(inner);
        (cols[0], cols[1])
    } else {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(45), Constraint::Percentage(55)]).split(inner);
        (rows[0], rows[1])
    };
    regions.workspace_panes.push((area, super::PANE_MAIN));
    draw_fields(frame, fields_area, theme, state, focused, regions);
    let split = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(58), Constraint::Percentage(42)]).split(right_area);
    draw_canvas(frame, split[0], theme, state);
    draw_readout(frame, split[1], theme, state);

    if let Some(browser) = &state.material_browser {
        let popup = crate::widgets::popup::centered_rect(92, 86, area);
        frame.render_widget(Clear, popup);
        crate::toolboxes::material_lookup::view::draw(frame, popup, theme, browser, true, regions);
    }
    if let Some(prompt) = &state.prompt {
        let w = area.width.min(90);
        let popup = Rect { x: area.x + area.width.saturating_sub(w) / 2, y: area.y + area.height / 2, width: w, height: 3.min(area.height) };
        frame.render_widget(Clear, popup);
        let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(format!(" {} - Enter: open \u{b7} Esc: cancel ", prompt.label));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        if inner.width > 0 && inner.height > 0 {
            frame.render_widget(Paragraph::new(crate::widgets::input_line::line(theme, "Path: ", &prompt.buffer, "_", inner.width)), inner);
        }
    }
}

fn value_of(state: &FeaWorkbenchState, row: FieldRow) -> String {
    model::row_value(&state.problem, &state.names(), state.import.as_ref().map(|(_, t)| t.len()), row)
}

fn label_width(state: &FeaWorkbenchState, rows: &[FieldRow]) -> usize {
    rows.iter().filter(|r| !matches!(r, FieldRow::Header(_))).map(|r| model::row_label(&state.problem, *r).chars().count()).max().unwrap_or(0) + 2
}

fn fields_required_width(state: &FeaWorkbenchState) -> u16 {
    let rows = state.rows();
    let lw = label_width(state, &rows);
    let max_value = rows
        .iter()
        .enumerate()
        .map(|(i, r)| if state.edit.is_some() && i == state.selected { state.edit_buffer.chars().count() + 1 } else { value_of(state, *r).chars().count() }.min(crate::widgets::scroll_list::VALUE_CAP))
        .max()
        .unwrap_or(0);
    (2 + lw + max_value + 2) as u16
}

fn draw_fields(frame: &mut Frame, area: Rect, theme: &Theme, state: &FeaWorkbenchState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(focused)).title(" Problem ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows = state.rows();
    let hint = rows.get(state.selected).map(|r| model::field_hint(*r)).unwrap_or("");
    let max_hint_lines = inner.height.saturating_sub(4).max(1);
    let hint_height = crate::widgets::hint_panel::hint_panel_height(hint, inner.width, max_hint_lines);
    let (list_area, hint_area) = if inner.height > hint_height + 1 {
        let split = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(hint_height)]).split(inner);
        (split[0], Some(split[1]))
    } else {
        (inner, None)
    };
    let lw = label_width(state, &rows);
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
            let editing = selected && state.edit.is_some();
            let value = if editing { state.edit_buffer.with_cursor() } else { value_of(state, *row) };
            let marker = if selected { "> " } else { "  " };
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            let action = matches!(model::edit_kind(*row), EditKind::Action) && value == "Enter";
            let style = if action && !selected { theme.disabled_style() } else { style };
            let (item, h) = crate::widgets::scroll_list::field_item(marker, &model::row_label(&state.problem, *row), lw, &value, list_area.width, style, None);
            heights.push(h);
            item
        })
        .collect();
    let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(state.selected));
    regions.fea_workbench_rows.extend(crate::mouse::list_row_regions_var(list_area, offset, &heights));
    if let Some(hint_area) = hint_area {
        frame.render_widget(Paragraph::new(Line::from(Span::styled(hint, theme.disabled_style()))).wrap(Wrap { trim: true }), hint_area);
    }
}

/// Whether the canvas draws the solved contour (else the mesh preview).
fn showing_result(state: &FeaWorkbenchState) -> bool {
    state.solved.is_some() && (!state.stale() || state.preview.is_none())
}

fn draw_canvas(frame: &mut Frame, area: Rect, theme: &Theme, state: &FeaWorkbenchState) {
    let result = showing_result(state);
    let title = if result { format!(" {} ", state.field.label()) } else { " Mesh preview ".to_string() };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 4 || inner.height < 3 {
        return;
    }
    let mesh = match (result, &state.solved, &state.preview) {
        (true, Some(s), _) => Some(&s.model.mesh),
        (_, _, Some(p)) => Some(&p.mesh),
        _ => None,
    };
    let Some(mesh) = mesh else {
        let msg = state.preview_error.clone().unwrap_or_else(|| "Building the mesh...".to_string());
        let tone = if state.preview_error.is_some() { StatusTone::Danger } else { StatusTone::Info };
        frame.render_widget(Paragraph::new(Line::from(Span::styled(msg, theme.status_style(tone)))).wrap(Wrap { trim: true }), inner);
        return;
    };
    let canvas_area = Rect { height: inner.height - 1, ..inner };
    let legend_area = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
    let (w, h) = canvas::pixel_size(canvas_area);
    let key = RasterKey { w, h, field: result.then_some(state.field), deform: state.deform && result, source: state.source };
    let mut cache = state.raster.borrow_mut();
    if cache.as_ref().is_none_or(|(k, _)| *k != key) {
        let raster = match (result, &state.solved) {
            (true, Some(s)) => {
                let values = s.node_values(state.field);
                let deform = key.deform.then(|| {
                    let (lo, hi) = mesh.nodes.iter().fold(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), |(lo, hi), x| (std::array::from_fn(|i| lo[i].min(x[i])), std::array::from_fn(|i| hi[i].max(x[i]))));
                    let extent = (0..mesh.dim()).map(|i| hi[i] - lo[i]).fold(0.0f64, f64::max);
                    let umax = s.summary.max_displacement.value;
                    (&s.u[..], if umax > 0.0 { 0.1 * extent / umax } else { 0.0 })
                });
                rasterize(mesh, Some(&values), deform, w, h)
            }
            _ => rasterize(mesh, None, None, w, h),
        };
        *cache = Some((key, raster));
    }
    let Some((_, raster)) = cache.as_ref() else { return };
    canvas::draw(frame, canvas_area, raster, result, state.show_mesh);
    if result {
        let n = (legend_area.width as usize).min(24);
        let mut spans: Vec<Span> = vec![Span::raw(format!("{:.4e} ", raster.min))];
        spans.extend(canvas::legend(n).into_iter().map(|c| Span::styled("\u{2588}", Style::default().fg(c))));
        spans.push(Span::raw(format!(" {:.4e}", raster.max)));
        if state.deform {
            spans.push(Span::styled("  (shape deformed, scaled)", theme.disabled_style()));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), legend_area);
    } else if let Some(p) = &state.preview {
        let text = format!("{} nodes, {} elements{}", p.mesh.nodes.len(), p.mesh.n_elems(), if state.meshing() { "  (rebuilding...)" } else { "" });
        frame.render_widget(Paragraph::new(Line::from(Span::styled(text, theme.disabled_style()))), legend_area);
    }
    let _ = Color::Reset;
}

fn draw_readout(frame: &mut Frame, area: Rect, theme: &Theme, state: &FeaWorkbenchState) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(" Results ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    crate::widgets::scroll_paragraph::render(frame, inner, theme, readout_lines(theme, state), state.results_scroll);
}

pub fn readout_lines<'a>(theme: &Theme, state: &FeaWorkbenchState) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();
    let tone = |t: StatusTone, s: String| Line::from(Span::styled(s, theme.status_style(t)));
    if state.meshing() {
        lines.push(tone(StatusTone::Info, "\u{2026} meshing".to_string()));
    }
    if let Some(secs) = state.solving() {
        lines.push(tone(StatusTone::Info, format!("\u{2026} solving ({secs:.1} s)")));
    }
    if let Some(e) = &state.preview_error {
        lines.push(tone(StatusTone::Danger, format!("\u{2717} {e}")));
    }
    if let Some(e) = &state.solve_error {
        lines.push(tone(StatusTone::Danger, format!("\u{2717} solve failed: {e}")));
    }
    let Some(s) = &state.solved else {
        if lines.is_empty() {
            let hint = if state.auto_solve { "Solves automatically for a small model; r solves now." } else { "Press r to solve." };
            lines.push(Line::from(Span::styled(hint, theme.disabled_style())));
        }
        return lines;
    };
    if state.stale() {
        lines.push(tone(StatusTone::Warning, "The inputs changed since this result (r solves again).".to_string()));
    }
    let sm = &s.summary;
    let at = |e: &fea_problem::solve::Extreme| if s.model.mesh.dim() == 3 { format!("({:.3}, {:.3}, {:.3})", e.at[0], e.at[1], e.at[2]) } else { format!("({:.3}, {:.3})", e.at[0], e.at[1]) };
    if state.details {
        lines.extend(fea_problem::report::report(s).lines().map(|l| Line::from(l.to_string())));
    } else {
        lines.push(Line::from(format!("Peak von Mises   {:.1} at {}", sm.max_von_mises.value, at(&sm.max_von_mises))));
        lines.push(Line::from(format!("Max principal    {:.1}    min principal {:.1}", sm.max_principal.value, sm.min_principal.value)));
        lines.push(Line::from(format!("Max displacement {:.5e} at {}", sm.max_displacement.value, at(&sm.max_displacement))));
        if let Some(m) = sm.margin {
            lines.push(tone(if m >= 0.0 { StatusTone::Success } else { StatusTone::Danger }, format!("Yield margin     {m:+.2} (yield / peak von Mises - 1)")));
        }
        lines.push(Line::from(format!("Reactions        ({:.2}, {:.2}, {:.2})", sm.reaction[0], sm.reaction[1], sm.reaction[2])));
        lines.push(Line::from(Span::styled(format!("{} nodes, {} elements, {} dofs, solved in {:.0} ms ({} pass{})", sm.nodes, sm.elements, sm.dofs, sm.solve_ms, s.history.len(), if s.history.len() == 1 { "" } else { "es" }), theme.disabled_style())));
    }
    if let Some(b) = fea_problem::benchmark::check(s).filter(|_| !state.details) {
        let lead = b.lines().into_iter().next().unwrap_or_default();
        lines.push(tone(if b.passed() { StatusTone::Success } else { StatusTone::Warning }, format!("{} {lead}", if b.passed() { "\u{2713}" } else { "\u{26a0}" })));
        for c in &b.checks {
            lines.push(Line::from(format!("  {}: FE {:.5e} vs {:.5e} ({:+.2} %)", c.quantity, c.fe, c.reference, 100.0 * c.error())));
        }
    }
    for f in &sm.interfaces {
        lines.push(Line::from(format!("Bushing in hole {}: fit pressure {:.0} mean / {:.0} peak, friction torque capacity {:.2}{}", f.hole, f.mean_pressure, f.peak_pressure, f.torque_capacity, if f.open_arc_deg > 0.0 { format!(", open over {:.0} deg", f.open_arc_deg) } else { String::new() })));
    }
    for note in &sm.notes {
        lines.push(tone(StatusTone::Warning, format!("\u{26a0} {note}")));
    }
    // A contact solve balances to its Newton tolerance, a linear one to rounding.
    if sm.equilibrium_error > if sm.interfaces.is_empty() { 1e-6 } else { 1e-3 } {
        lines.push(tone(StatusTone::Warning, format!("\u{26a0} reactions and loads differ by {:.1e} (relative): check the supports and loads", sm.equilibrium_error)));
    }
    if sm.zz_error > 0.10 {
        lines.push(tone(StatusTone::Warning, format!("\u{26a0} estimated mesh error {:.0} %: use a smaller element size, more adaptive passes or a quadratic element", 100.0 * sm.zz_error)));
    } else {
        lines.push(Line::from(Span::styled(format!("Estimated mesh error {:.2} % of the energy norm", 100.0 * sm.zz_error), theme.disabled_style())));
    }
    lines
}
