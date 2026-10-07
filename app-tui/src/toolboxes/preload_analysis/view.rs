//! Rendering for the Preload Analysis toolbox. Wide terminals show the
//! editable field list beside a live-updating readout of every derived
//! result; narrow terminals stack the two vertically - same responsive
//! rule and `fields_required_width` technique every other toolbox in this
//! crate uses.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::theme::{StatusTone, Theme};

use fastened_joint_solver::solve::{JointSolution, SolverStatus, StressSection};

use super::model::{self, PreloadModel};
use super::PreloadAnalysisState;

const MIN_READOUT_WIDTH: u16 = 44;

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, state: &PreloadAnalysisState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(" Preload Analysis - Space/Enter: toggle/edit \u{b7} t: joint templates \u{b7} d: details \u{b7} e: export ");
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
    draw_readout(frame, readout_area, theme, &state.model, &state.fe, state.show_numbers, state.results_scroll, state.message.as_deref());

    if state.bolt_picker.open {
        super::bolt_picker::render(frame, area, theme, &state.bolt_picker, regions);
    }
    if state.template_picker.open {
        super::template_picker::render(frame, area, theme, &state.template_picker, &state.model, regions);
    }
}

fn compute_label_width(rows: &[model::FieldRow]) -> u16 {
    rows.iter()
        .filter(|r| !matches!(r, model::FieldRow::Header(_)))
        .map(|r| match r {
            model::FieldRow::Number(target) => target.label().len(),
            other => model::row_label(*other).len(),
        })
        .max()
        .unwrap_or(0) as u16
        + 2
}

fn display_value(model: &PreloadModel, row: model::FieldRow) -> String {
    match row {
        model::FieldRow::Header(_) => String::new(),
        model::FieldRow::ToggleMode => model.mode.label().to_string(),
        model::FieldRow::OpenBoltPicker => model.matching_bolt().map(|b| b.designation.to_string()).unwrap_or_else(|| "Custom".to_string()),
        model::FieldRow::OpenTemplatePicker => format!("{} member(s) - Enter to choose", model.members.len()),
        model::FieldRow::ToggleTighteningFrom => model::label_tightening_from(model.tightening_from).to_string(),
        model::FieldRow::ToggleBearingModel => model::label_bearing_model(model.bearing_model).to_string(),
        model::FieldRow::ToggleExternalLoadEnabled => bool_label(model.external_load_enabled),
        model::FieldRow::ToggleSlipEnabled => bool_label(model.slip_enabled),
        model::FieldRow::ToggleStrengthLimitsEnabled => bool_label(model.strength_limits_enabled),
        model::FieldRow::ToggleAddMember => format!("Enter ({}/{})", model.members.len(), model::MAX_MEMBERS),
        model::FieldRow::ToggleRemoveMember => "Enter".to_string(),
        model::FieldRow::ToggleUncertaintyEnabled => bool_label(model.uncertainty_enabled),
        model::FieldRow::ToggleMonteCarloEnabled => bool_label(model.monte_carlo_enabled),
        model::FieldRow::ToggleThreadLoadDistributionEnabled => bool_label(model.thread_load_distribution_enabled),
        model::FieldRow::Number(target) => target.format_value(model.number_value(target)),
    }
}

fn bool_label(b: bool) -> String {
    if b { "Enabled" } else { "Disabled" }.to_string()
}

fn fields_required_width(state: &PreloadAnalysisState) -> u16 {
    let rows = model::field_rows(&state.model);
    let label_width = compute_label_width(&rows);
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
        .unwrap_or(0) as u16;
    2 + label_width + max_value_width + 2
}

fn draw_fields(frame: &mut Frame, area: Rect, theme: &Theme, state: &PreloadAnalysisState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
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
    let label_width = compute_label_width(&rows) as usize;

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            if let model::FieldRow::Header(text) = row {
                return ListItem::new(Line::from(Span::styled(format!("-- {text} --"), theme.title_style(false).add_modifier(Modifier::BOLD))));
            }
            let selected = focused && i == state.selected;
            let value = if selected && state.editing { state.edit_buffer.with_cursor() } else { display_value(&state.model, *row) };
            let marker = if selected { "> " } else { "  " };
            let label = match row {
                model::FieldRow::Number(target) => target.label(),
                other => model::row_label(*other).to_string(),
            };
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            ListItem::new(Line::from(Span::styled(format!("{marker}{label:<label_width$}{value}"), style)))
        })
        .collect();

    let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(state.selected));
    regions.preload_analysis_rows.extend(crate::mouse::list_row_regions(list_area, offset, rows.len()));

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

fn draw_readout(frame: &mut Frame, area: Rect, theme: &Theme, model: &PreloadModel, fe: &super::FeCheckState, show_numbers: bool, scroll: u16, message: Option<&str>) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(" Results ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let lines = readout_lines(theme, model, fe, show_numbers, message);
    crate::widgets::scroll_paragraph::render(frame, inner, theme, lines, scroll);
}

/// The clamped stack as a picture (head, each member, nut) - shown above the
/// numbers so a template's or hand-built stack-up is visible at a glance.
fn stack_lines<'a>(theme: &Theme, model: &PreloadModel) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from(Span::styled("Joint Stack", theme.title_style(false)))];
    for row in super::joint_templates::stack_diagram(model) {
        let style = if row.is_washer { theme.disabled_style() } else { Style::default() };
        lines.push(Line::from(vec![Span::styled(row.bar, style), Span::raw("  "), Span::raw(row.caption)]));
    }
    lines.push(Line::from(""));
    lines
}

fn readout_lines<'a>(theme: &'a Theme, model: &'a PreloadModel, fe: &super::FeCheckState, show_numbers: bool, message: Option<&str>) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    if let Some(m) = message {
        lines.push(Line::from(Span::styled(format!("\u{2713} {m}"), theme.status_style(StatusTone::Success))));
    }
    let solution = match &model.output {
        Ok(s) => s,
        Err(e) => {
            lines.push(Line::from(Span::styled(format!("Invalid input: {e:?}"), theme.status_style(StatusTone::Danger))));
            return lines;
        }
    };

    let (status_text, status_tone) = match solution.status {
        SolverStatus::Converged => ("CONVERGED", StatusTone::Success),
        SolverStatus::PrevailingTorqueExceedsApplied => ("PREVAILING TORQUE EXCEEDS APPLIED", StatusTone::Warning),
        SolverStatus::NotBracketed => ("NOT BRACKETED", StatusTone::Danger),
        SolverStatus::MaxIterationsExceeded => ("MAX ITERATIONS EXCEEDED", StatusTone::Danger),
    };
    lines.push(Line::from(vec![
        Span::styled("Solver: ", Style::default()),
        Span::styled(status_text, theme.status_style(status_tone).add_modifier(Modifier::BOLD)),
        Span::raw(format!("   Residual: {:.4}", solution.torque.residual)),
    ]));
    lines.push(Line::from(""));
    lines.extend(stack_lines(theme, model));

    lines.push(Line::from(Span::styled("Preload / Torque Breakdown", theme.title_style(false))));
    lines.push(Line::from(format!("  Applied torque       {:.4}", solution.torque.applied)));
    lines.push(Line::from(format!(
        "  Thread torque        {:.4}  ({:.1}%)",
        solution.torque.thread,
        solution.torque.thread_fraction() * 100.0
    )));
    lines.push(Line::from(format!(
        "  Bearing torque       {:.4}  ({:.1}%)",
        solution.torque.bearing,
        solution.torque.bearing_fraction() * 100.0
    )));
    lines.push(Line::from(format!(
        "  Prevailing torque    {:.4}  ({:.1}%)",
        solution.torque.prevailing,
        solution.torque.prevailing_fraction() * 100.0
    )));
    lines.push(Line::from(Span::styled(format!("  Solved preload       {:.4}", solution.preload), theme.title_style(false))));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Deformation", theme.title_style(false))));
    lines.push(Line::from(format!("  Fastener elongation  {:.6}", solution.deformation.fastener_elongation)));
    lines.push(Line::from(format!("  Member compression   {:.6}", solution.deformation.member_compression)));
    lines.push(Line::from(format!("  Total closure        {:.6}", solution.deformation.total_closure)));
    lines.push(Line::from(""));

    lines.extend(fe_lines(theme, model, fe));

    lines.push(Line::from(Span::styled("Nut Rotation", theme.title_style(false))));
    lines.push(Line::from(format!("  Relative rotation    {:.4} rev  ({:.2} deg)", solution.rotation.revolutions, solution.rotation.degrees)));
    lines.push(Line::from(format!("  Fastener twist       {:.3} deg", solution.rotation.torsional_twist_degrees)));
    lines.push(Line::from(format!("  Est. tool rotation   {:.2} deg", solution.rotation.estimated_tool_rotation_degrees)));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Stress (section: axial / shear / von Mises / yield MS / ult MS)", theme.title_style(false))));
    lines.push(stress_line("Shank", &solution.stress.shank));
    lines.push(stress_line("Tensile area", &solution.stress.tensile_stress_area));
    lines.push(stress_line("Thread root", &solution.stress.thread_root));
    lines.push(Line::from(format!("  Bearing contact pressure {:.2}", solution.bearing_contact_pressure)));
    lines.push(Line::from(format!("  Member avg. compressive stress {:.2}", solution.member_average_compressive_stress)));
    if let Some(margin) = solution.thread_shear_margin {
        lines.push(Line::from(vec![Span::raw("  Thread shear margin      "), Span::styled(fmt_margin(margin), Style::default().fg(tone_color(tone_for_margin(margin))))]));
    }
    lines.push(Line::from(""));

    if let Some(service) = &solution.service_load {
        lines.push(Line::from(Span::styled("Service Load", theme.title_style(false))));
        lines.push(Line::from(format!("  External load        {:.4}", service.external_load)));
        lines.push(Line::from(format!("  Resulting bolt load  {:.4}", service.bolt_load)));
        lines.push(Line::from(format!("  Remaining clamp load {:.4}", service.member_load)));
        lines.push(Line::from(format!("  Separation load      {:.4}", service.separation_load)));
        lines.push(Line::from(vec![
            Span::raw("  Separation margin    "),
            Span::styled(fmt_margin(service.separation_margin), Style::default().fg(tone_color(tone_for_margin(service.separation_margin)))),
            if service.separated { Span::styled("  SEPARATED", theme.status_style(StatusTone::Danger)) } else { Span::raw("") },
        ]));
        lines.push(Line::from(""));
    }

    if let (Some(capacity), Some(margin)) = (solution.slip_capacity, solution.slip_margin) {
        lines.push(Line::from(Span::styled("Slip Resistance", theme.title_style(false))));
        lines.push(Line::from(format!("  Slip capacity        {capacity:.4}")));
        lines.push(Line::from(vec![Span::raw("  Slip margin          "), Span::styled(fmt_margin(margin), Style::default().fg(tone_color(tone_for_margin(margin))))]));
        lines.push(Line::from(""));
    }

    if let Some(u) = solution.uncertainty {
        lines.push(Line::from(Span::styled("Uncertainty (Worst Case)", theme.title_style(false))));
        lines.push(Line::from(format!("  Preload range        {:.1} .. {:.1} .. {:.1} lbf  (min / nominal / max)", u.preload_min, u.preload_nominal, u.preload_max)));
        lines.push(Line::from(""));
    }
    if let Some(mc) = solution.monte_carlo {
        lines.push(Line::from(Span::styled(format!("Uncertainty (Monte Carlo, seed {})", mc.seed), theme.title_style(false))));
        lines.push(Line::from(format!("  {} samples converged", mc.samples)));
        lines.push(Line::from(format!("  Preload mean +/- std dev  {:.1} +/- {:.1} lbf", mc.preload_mean, mc.preload_std_dev)));
        lines.push(Line::from(format!("  Preload min / max    {:.1} / {:.1} lbf", mc.preload_min, mc.preload_max)));
        lines.push(Line::from(""));
    }

    if let Some(tld) = &solution.thread_load_distribution {
        lines.push(Line::from(Span::styled("Thread Load Distribution (Advanced)", theme.title_style(false))));
        lines.push(Line::from(format!("  First engaged thread load  {:.1} lbf", tld.first_thread_load)));
        lines.push(Line::from(format!("  Maximum thread load        {:.1} lbf", tld.max_thread_load)));
        let shown = tld.per_thread_loads.len().min(20);
        for (i, load) in tld.per_thread_loads.iter().take(shown).enumerate() {
            lines.push(Line::from(format!("  Thread {:>3}                 {:.1} lbf", i + 1, load)));
        }
        if tld.per_thread_loads.len() > shown {
            lines.push(Line::from(format!("  ... {} more threads not shown", tld.per_thread_loads.len() - shown)));
        }
        lines.push(Line::from(""));
    }

    if !solution.warnings.is_empty() {
        lines.push(Line::from(Span::styled("Warnings", theme.title_style(false))));
        for w in &solution.warnings {
            lines.push(Line::from(Span::styled(format!("  \u{26a0} {w}"), theme.status_style(StatusTone::Warning))));
        }
        lines.push(Line::from(""));
    }

    if show_numbers {
        lines.extend(numbers_panel_lines(theme, solution));
    }

    lines
}

fn stress_line(label: &str, s: &StressSection) -> Line<'static> {
    Line::from(format!(
        "  {label:<14} {:>10.2} / {:>8.2} / {:>10.2} / {} / {}",
        s.axial,
        s.torsional_shear,
        s.von_mises,
        fmt_margin(s.yield_margin),
        fmt_margin(s.ultimate_margin)
    ))
}

fn numbers_panel_lines(theme: &Theme, solution: &JointSolution) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled("Numbers (d to hide)", theme.title_style(false)))];
    lines.push(Line::from(format!(
        "  Compliance: C_b={:.3e}  C_m={:.3e}  k_b={:.3e}  k_m={:.3e}  C={:.4}",
        solution.compliance.c_b, solution.compliance.c_m, solution.compliance.k_b, solution.compliance.k_m, solution.compliance.load_fraction
    )));
    lines.push(Line::from(format!(
        "  Energy: fastener strain U_b={:.4}  member strain U_m={:.4}  tightening work W={:.4}",
        solution.energy.fastener_strain_energy, solution.energy.member_strain_energy, solution.energy.tightening_work
    )));
    let p = solution.stress.thread_root.principal;
    lines.push(Line::from(format!("  Thread-root principal stresses: sigma1={:.2} sigma2={:.2} max_shear={:.2}", p.sigma1, p.sigma2, p.max_shear)));
    lines
}

/// Plain-text export - mirrors `pressure_vessel/view.rs::build_report_text`'s
/// pattern: pure and synchronous, no filesystem access.
pub fn build_report_text(model: &PreloadModel, fe: &super::FeCheckState) -> String {
    let mut s = String::new();
    s.push_str("Preload Analysis Report\n");
    s.push_str("========================\n\n");
    s.push_str(&format!("Mode: {}\n", model.mode.label()));
    s.push_str(&format!("Tightening from: {}\n\n", model::label_tightening_from(model.tightening_from)));

    let solution = match &model.output {
        Ok(s) => s,
        Err(e) => {
            s.push_str(&format!("Invalid input: {e:?}\n"));
            return s;
        }
    };

    s.push_str(&format!("Solver status: {:?}\n", solution.status));
    s.push_str(&format!("Solved preload: {:.4}\n", solution.preload));
    s.push_str(&format!(
        "Torque breakdown: applied {:.4}, thread {:.4} ({:.1}%), bearing {:.4} ({:.1}%), prevailing {:.4} ({:.1}%), residual {:.4}\n\n",
        solution.torque.applied,
        solution.torque.thread,
        solution.torque.thread_fraction() * 100.0,
        solution.torque.bearing,
        solution.torque.bearing_fraction() * 100.0,
        solution.torque.prevailing,
        solution.torque.prevailing_fraction() * 100.0,
        solution.torque.residual
    ));
    s.push_str(&format!(
        "Deformation: fastener elongation {:.6}, member compression {:.6}, total {:.6}\n",
        solution.deformation.fastener_elongation, solution.deformation.member_compression, solution.deformation.total_closure
    ));
    s.push_str(&format!(
        "Nut rotation: {:.4} rev ({:.2} deg), fastener twist {:.3} deg, est. tool rotation {:.2} deg\n\n",
        solution.rotation.revolutions, solution.rotation.degrees, solution.rotation.torsional_twist_degrees, solution.rotation.estimated_tool_rotation_degrees
    ));
    for (label, section) in [("Shank", &solution.stress.shank), ("Tensile area", &solution.stress.tensile_stress_area), ("Thread root", &solution.stress.thread_root)] {
        s.push_str(&format!(
            "{label}: axial {:.2}, shear {:.2}, von Mises {:.2}, yield MS {}, ult MS {}\n",
            section.axial,
            section.torsional_shear,
            section.von_mises,
            fmt_margin(section.yield_margin),
            fmt_margin(section.ultimate_margin)
        ));
    }
    if let Some(service) = &solution.service_load {
        s.push_str(&format!(
            "\nService load: external {:.4}, bolt load {:.4}, clamp load {:.4}, separation load {:.4}, margin {}\n",
            service.external_load,
            service.bolt_load,
            service.member_load,
            service.separation_load,
            fmt_margin(service.separation_margin)
        ));
    }
    if let (Some(capacity), Some(margin)) = (solution.slip_capacity, solution.slip_margin) {
        s.push_str(&format!("Slip capacity {capacity:.4}, margin {}\n", fmt_margin(margin)));
    }
    if let Some(u) = solution.uncertainty {
        s.push_str(&format!("\nUncertainty (worst case): preload {:.1} .. {:.1} .. {:.1} lbf (min/nominal/max)\n", u.preload_min, u.preload_nominal, u.preload_max));
    }
    if let Some(mc) = solution.monte_carlo {
        s.push_str(&format!(
            "Uncertainty (Monte Carlo, seed {}, {} samples): preload mean {:.1} +/- {:.1} lbf, range {:.1} .. {:.1} lbf\n",
            mc.seed, mc.samples, mc.preload_mean, mc.preload_std_dev, mc.preload_min, mc.preload_max
        ));
    }
    if !solution.warnings.is_empty() {
        s.push_str("\nWarnings:\n");
        for w in &solution.warnings {
            s.push_str(&format!("  - {w}\n"));
        }
    }
    if let Some(Ok(r)) = model.fe_input().and_then(|i| fe.current(&i)) {
        s.push_str(&format!(
            "\nMember compliance cross-check (FE, axisymmetric): cone C_m {:.4e}, FE C_m {:.4e} ({:+.1}%); joint load fraction cone {:.3}, FE {:.3}{}\n",
            r.cone,
            r.fe.compliance,
            r.difference() * 100.0,
            r.fraction_cone,
            r.fraction_fe,
            r.equivalent_angle_deg.map(|a| format!("; equivalent cone half angle {a:.1} deg")).unwrap_or_default()
        ));
    }
    s
}

/// The finite-element member-compliance cross-check (`fe_check.rs`): the solver's cone estimate
/// beside the axisymmetric FE value, the joint load fraction each gives, and the cone half angle that
/// would reproduce the FE compliance.
fn fe_lines<'a>(theme: &Theme, model: &PreloadModel, fe: &super::FeCheckState) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from(Span::styled("Member Compliance Cross-Check (FE, axisymmetric)", theme.title_style(false)))];
    let current = model.fe_input().and_then(|i| fe.current(&i));
    match current {
        Some(Ok(r)) => {
            let diff = r.difference();
            lines.push(Line::from(format!("  Pressure-cone C_m  {:.4e}   FE C_m  {:.4e}   FE vs cone {:+.1}%", r.cone, r.fe.compliance, diff * 100.0)));
            lines.push(Line::from(format!("  Joint load fraction  cone {:.3}   FE {:.3}   ({} elements, {:.0} ms)", r.fraction_cone, r.fraction_fe, r.fe.elements, r.fe.ms)));
            if let Some(a) = r.equivalent_angle_deg {
                if diff.abs() > 0.10 {
                    let tone = if diff.abs() > 0.25 { StatusTone::Warning } else { StatusTone::Info };
                    lines.push(Line::from(Span::styled(
                        format!("  The FE compliance corresponds to a cone half angle of {a:.1} deg (the joint uses {:.1} deg); members taken with Poisson's ratio {}.", model.cone_half_angle_deg, super::fe_check::MEMBER_NU),
                        theme.status_style(tone),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(format!("  Agrees with the cone model (equivalent half angle {a:.1} deg).") , theme.disabled_style())));
                }
            }
        }
        Some(Err(e)) => lines.push(Line::from(Span::styled(format!("  FE cross-check unavailable: {e}"), theme.disabled_style()))),
        None if fe.running() => lines.push(Line::from(Span::styled("  \u{2026} running the finite-element model", theme.status_style(StatusTone::Info)))),
        None => lines.push(Line::from(Span::styled("  (starts a moment after the joint stops changing)", theme.disabled_style()))),
    }
    lines.push(Line::from(""));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn draw_at(width: u16, height: u16, state: &PreloadAnalysisState) {
        let backend = TestBackend::new(width.max(1), height.max(1));
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, area, &Theme::default_palette(), state, true, &mut regions)).unwrap();
    }

    #[test]
    fn draw_does_not_panic_at_normal_width() {
        draw_at(180, 50, &PreloadAnalysisState::default());
    }

    /// Regression test for the hint-truncation bug (see `bushing/view.rs`'s
    /// twin test): selects the field with the single longest `field_hint()`
    /// string and asserts every word of it appears somewhere in the
    /// rendered buffer, across a range of widths including narrow ones.
    #[test]
    fn the_longest_hint_is_never_truncated_across_a_range_of_widths() {
        let mut everything_on = PreloadModel::default();
        everything_on.uncertainty_enabled = true;
        everything_on.monte_carlo_enabled = true;
        everything_on.external_load_enabled = true;
        everything_on.slip_enabled = true;
        everything_on.strength_limits_enabled = true;
        everything_on.thread_load_distribution_enabled = true;
        everything_on.tightening_from = fastened_joint_solver::solve::TighteningMember::BoltHead;
        let rows = model::field_rows(&everything_on);
        let (longest_index, longest_hint) = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (i, model::field_hint(*r)))
            .max_by_key(|(_, hint)| hint.len())
            .expect("at least one field row");
        assert!(!longest_hint.is_empty());

        for width in [40u16, 50, 60, 84, 98, 140] {
            let mut state = PreloadAnalysisState::default();
            state.model.uncertainty_enabled = true;
            state.model.monte_carlo_enabled = true;
            state.model.external_load_enabled = true;
            state.model.slip_enabled = true;
            state.model.strength_limits_enabled = true;
            state.model.thread_load_distribution_enabled = true;
            state.model.tightening_from = fastened_joint_solver::solve::TighteningMember::BoltHead;
            state.model.recompute();
            state.selected = longest_index;
            let backend = TestBackend::new(width, 50);
            let mut terminal = Terminal::new(backend).unwrap();
            let area = Rect::new(0, 0, width, 50);
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
        draw_at(50, 50, &PreloadAnalysisState::default());
    }

    #[test]
    fn draw_does_not_panic_at_degenerate_sizes() {
        for (w, h) in [(0, 0), (1, 1), (40, 0), (0, 10)] {
            draw_at(w, h, &PreloadAnalysisState::default());
        }
    }

    #[test]
    fn draw_does_not_panic_with_every_optional_section_enabled_and_numbers_shown() {
        let mut state = PreloadAnalysisState::default();
        state.model.toggle_external_load_enabled();
        state.model.toggle_slip_enabled();
        state.model.toggle_strength_limits_enabled();
        state.model.toggle_uncertainty_enabled();
        state.model.toggle_monte_carlo_enabled();
        state.model.toggle_thread_load_distribution_enabled();
        state.model.commit_number(model::NumberTarget::YieldStrength, 500.0);
        state.model.commit_number(model::NumberTarget::ThreadEngagementLength, 0.3);
        state.model.commit_number(model::NumberTarget::ThreadShearStrength, 60_000.0);
        state.model.add_member();
        state.show_numbers = true;
        draw_at(200, 60, &state);
        draw_at(50, 60, &state);
    }

    #[test]
    fn draw_does_not_panic_with_invalid_input() {
        let mut state = PreloadAnalysisState::default();
        state.model.commit_number(model::NumberTarget::ThreadRootDia, 50.0); // root > major, invalid
        draw_at(180, 50, &state);
    }

    #[test]
    fn build_report_text_mentions_the_solver_status_and_preload() {
        let model = PreloadModel::default();
        let text = build_report_text(&model, &super::super::FeCheckState::default());
        assert!(text.contains("Solver status:"));
        assert!(text.contains("Solved preload:"));
    }
}
