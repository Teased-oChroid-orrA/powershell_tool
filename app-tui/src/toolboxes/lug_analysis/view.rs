//! Rendering for Lug Analysis: the field list beside (or above) a live
//! readout of the contact, the stresses, the margins and, with `d`, the
//! contact-pressure / hoop-stress profile around the bore. The material
//! browser opens as an overlay.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::model::{self, Check, FieldRow, LugRun, Status};
use super::LugAnalysisState;
use crate::theme::{StatusTone, Theme};

const MIN_READOUT_WIDTH: u16 = 40;

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, state: &LugAnalysisState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(" Lug Analysis - Space/Enter: toggle/pick/edit \u{b7} d: bore profile \u{b7} e: export ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let required = fields_required_width(state);
    let (fields_area, readout_area) = if inner.width >= required + MIN_READOUT_WIDTH {
        let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Length(required), Constraint::Min(MIN_READOUT_WIDTH)]).split(inner);
        (cols[0], cols[1])
    } else {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(50), Constraint::Percentage(50)]).split(inner);
        (rows[0], rows[1])
    };

    regions.workspace_panes.push((area, super::PANE_MAIN));
    draw_fields(frame, fields_area, theme, state, focused, regions);
    draw_readout(frame, readout_area, theme, state);

    if let Some(browser) = &state.material_browser {
        let popup = centered(area, 92, 86);
        frame.render_widget(Clear, popup);
        crate::toolboxes::material_lookup::view::draw(frame, popup, theme, browser, true, regions);
    }
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    crate::widgets::popup::centered_rect(pct_x, pct_y, area)
}

fn compute_label_width(rows: &[FieldRow]) -> u16 {
    rows.iter().filter(|r| !matches!(r, FieldRow::Header(_))).map(|r| model::row_label(*r).len()).max().unwrap_or(0) as u16 + 2
}

fn display_value(state: &LugAnalysisState, row: FieldRow) -> String {
    let m = &state.model;
    match row {
        FieldRow::Header(_) => String::new(),
        FieldRow::Number(t) => t.format_value(m.number_value(t)),
        FieldRow::ToggleHeadShape => if m.head_round { "Round (e = W/2)" } else { "Custom" }.to_string(),
        FieldRow::OpenMaterialPicker => m.material().name.to_string(),
        FieldRow::ToggleBushing => if m.bushing { "On (pressed into the hole)" } else { "Off (pin bears on the hole)" }.to_string(),
        FieldRow::OpenBushingMaterialPicker => m.bushing_material().name.to_string(),
        FieldRow::ToggleMeshDensity => if m.elements_around.is_some() { format!("Custom ({} around)", m.effective_elements_around()) } else { m.density.label().to_string() },
        FieldRow::MeshSection => format!("{} around the bore", m.effective_elements_around()),
        FieldRow::ToggleAutoRefine => if m.auto_refine { "On (loaded sector of a loose pin)" } else { "Off (uniform spacing)" }.to_string(),
        FieldRow::MeshAdvice => state.mesh_advice_text(),
        FieldRow::ToggleSolver => m.solver.label().to_string(),
        FieldRow::TogglePlastic => if m.plastic { "On (plane-strain collapse)" } else { "Off (elastic only)" }.to_string(),
        FieldRow::ToggleFlowRule => m.flow_rule.label().to_string(),
        FieldRow::TogglePinBody => m.pin_body.label().to_string(),
        FieldRow::ToggleFiniteStrain => if m.finite_strain { "Finite strain (true stress-strain)" } else { "Perfectly plastic (flow stress rule)" }.to_string(),
        FieldRow::ToggleSecondOrder => if m.second_order { "On (P-delta)" } else { "Off (small displacement)" }.to_string(),
        FieldRow::TogglePinBending => if m.pin_bending { "On (double shear, slices)" } else { "Off (uniform through the thickness)" }.to_string(),
    }
}

fn row_hint(m: &model::LugUiModel, row: FieldRow) -> Option<String> {
    match row {
        FieldRow::Number(t) => m.validation_hint(t),
        _ => None,
    }
}

fn fields_required_width(state: &LugAnalysisState) -> u16 {
    let rows = model::field_rows(&state.model);
    let label_width = compute_label_width(&rows);
    let max_value = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let value_len = if state.editing && i == state.selected { state.edit_buffer.chars().count() + 1 } else { display_value(state, *row).chars().count() }.min(crate::widgets::scroll_list::VALUE_CAP);
            let hint_len = row_hint(&state.model, *row).map(|h| h.chars().count() + 4).unwrap_or(0);
            value_len + hint_len
        })
        .max()
        .unwrap_or(0) as u16;
    2 + label_width + max_value + 2
}

fn draw_fields(frame: &mut Frame, area: Rect, theme: &Theme, state: &LugAnalysisState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(focused)).title(" Inputs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = model::field_rows(&state.model);
    let hint = rows.get(state.selected).map(|r| model::field_hint(*r)).unwrap_or("");
    let max_hint_lines = inner.height.saturating_sub(4).max(1);
    let hint_height = crate::widgets::hint_panel::hint_panel_height(hint, inner.width, max_hint_lines);
    let (list_area, hint_area) = if inner.height > hint_height + 1 {
        let split = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(hint_height)]).split(inner);
        (split[0], Some(split[1]))
    } else {
        (inner, None)
    };

    let label_width = compute_label_width(&rows) as usize;
    let mut heights: Vec<u16> = Vec::with_capacity(rows.len());
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            if let FieldRow::Header(text) = row {
                heights.push(1);
                return ListItem::new(Line::from(Span::styled(format!("-- {text} --"), theme.title_style(false).add_modifier(Modifier::BOLD))));
            }
            if *row == FieldRow::MeshSection {
                // A header that opens and closes on a click, Enter or Space.
                heights.push(1);
                let selected = focused && i == state.selected;
                let chevron = if state.model.mesh_open { "\u{25be}" } else { "\u{25b8}" };
                let summary = if state.model.mesh_open { String::new() } else { format!("  ({} around the bore)", state.model.effective_elements_around()) };
                let style = if selected { theme.selected_row_style() } else { theme.title_style(false).add_modifier(Modifier::BOLD) };
                return ListItem::new(Line::from(Span::styled(format!("{}-- {chevron} Mesh{summary} --", if selected { "> " } else { "" }), style)));
            }
            let selected = focused && i == state.selected;
            let value = if selected && state.editing { state.edit_buffer.with_cursor() } else { display_value(state, *row) };
            let marker = if selected { "> " } else { "  " };
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            let extra = row_hint(&state.model, *row).map(|h| Span::styled(format!("  \u{26a0} {h}"), theme.status_style(StatusTone::Danger)));
            let (item, h) = crate::widgets::scroll_list::field_item(marker, model::row_label(*row), label_width, &value, list_area.width, style, extra);
            heights.push(h);
            item
        })
        .collect();

    let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(state.selected));
    regions.lug_analysis_rows.extend(crate::mouse::list_row_regions_var(list_area, offset, &heights));

    if let Some(hint_area) = hint_area {
        frame.render_widget(Paragraph::new(Line::from(Span::styled(hint, theme.disabled_style()))).wrap(Wrap { trim: true }), hint_area);
    }
}

fn draw_readout(frame: &mut Frame, area: Rect, theme: &Theme, state: &LugAnalysisState) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(" Results ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    crate::widgets::scroll_paragraph::render(frame, inner, theme, readout_lines(theme, state), state.results_scroll);
}

fn ksi(v: f64) -> String {
    format!("{:.1} ksi", v / 1000.0)
}

fn status_glyph(s: Status) -> (&'static str, StatusTone) {
    match s {
        Status::Pass => ("\u{2713}", StatusTone::Success),
        Status::Warn => ("\u{26a0}", StatusTone::Warning),
        Status::Fail => ("\u{2717}", StatusTone::Danger),
        Status::Info => ("\u{b7}", StatusTone::Neutral),
    }
}

fn check_line<'a>(theme: &Theme, c: &Check) -> Line<'a> {
    let (glyph, tone) = status_glyph(c.status);
    let ms = c.margin.map_or("      -".to_string(), |m| format!("MS {m:+.2}"));
    Line::from(vec![
        Span::styled(format!("{glyph} {:<21}", c.name), theme.status_style(tone)),
        Span::styled(format!("{ms:<9}"), theme.status_style(tone)),
        Span::raw(c.detail.clone()),
    ])
}

pub fn readout_lines<'a>(theme: &Theme, state: &LugAnalysisState) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();

    if let Some(job) = &state.job {
        lines.push(Line::from(Span::styled(format!("\u{2026} analysing ({:.1} s)", job.started.elapsed().as_secs_f64()), theme.status_style(StatusTone::Info))));
    }
    if let Some(err) = &state.error {
        lines.push(Line::from(Span::styled(format!("\u{2717} {err}"), theme.status_style(StatusTone::Danger))));
    }
    let Some(run) = &state.run else {
        if state.job.is_none() && state.error.is_none() {
            lines.push(Line::from(Span::styled("Waiting for the first analysis...", theme.disabled_style())));
        }
        return lines;
    };
    if state.error.is_some() || state.job.is_some() || state.model.input().ok().as_ref() != Some(&run.input) {
        lines.push(Line::from(Span::styled("Showing the last completed analysis; the inputs have changed.", theme.disabled_style())));
    }
    lines.push(Line::from(""));
    lines.extend(summary_lines(theme, run));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Checks", theme.title_style(false).add_modifier(Modifier::BOLD))));
    lines.extend(run.checks.iter().map(|c| check_line(theme, c)));
    if !run.notes.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Notes", theme.title_style(false).add_modifier(Modifier::BOLD))));
        lines.extend(run.notes.iter().map(|n| Line::from(Span::styled(format!("\u{2022} {n}"), theme.status_style(StatusTone::Warning)))));
    }
    if state.show_numbers {
        lines.push(Line::from(""));
        lines.extend(profile_lines(theme, run));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(model_line(run), theme.disabled_style())));
    lines
}

fn summary_lines<'a>(theme: &Theme, run: &LugRun) -> Vec<Line<'a>> {
    let s = &run.solution;
    let i = &run.input;
    let mut out = vec![Line::from(Span::styled("Contact", theme.title_style(false).add_modifier(Modifier::BOLD)))];
    out.push(Line::from(format!("  Load {:.1} lbf at {:.1} deg    pin travel {:.5} in", i.case.load_lbf, i.case.angle_deg, s.bearing_deflection)));
    if s.peak_pressure > 0.0 {
        let arc = if s.contact_arc_deg > 359.0 { "all round".to_string() } else { format!("{:.1} deg about {:.0} deg", s.contact_arc_deg, s.contact_centre_deg.rem_euclid(360.0)) };
        out.push(Line::from(format!("  Patch {arc}    peak pressure {}    P/(D t) {}", ksi(s.peak_pressure), ksi(s.bearing_stress))));
        let slipping = s.contact.iter().filter(|p| p.slipping).count();
        if i.pin.friction > 0.0 {
            out.push(Line::from(Span::styled(format!("  Friction {:.2}: {} of {} contact points slipping", i.pin.friction, slipping, s.contact.iter().filter(|p| p.pressure > 0.0).count()), theme.disabled_style())));
        }
    } else {
        out.push(Line::from(Span::styled("  The pin is not touching the bore (clearance).", theme.disabled_style())));
    }
    if let Some(l) = &run.limit {
        out.push(Line::from(""));
        out.push(Line::from(Span::styled("Ultimate (plastic collapse)", theme.title_style(false).add_modifier(Modifier::BOLD))));
        out.push(Line::from(format!("  Collapse load {:.0} lbf at flow stress {} ({}){}", l.limit_load_lbf, ksi(l.flow_stress), i.flow_rule.label(), if l.plateau { "" } else { "   (lower bound)" })));
        if i.case.load_lbf > 0.0 {
            out.push(Line::from(format!("  Applied {:.0} lbf = {:.0} % of collapse    plastic zone {:.0} % of the lug at collapse", i.case.load_lbf, 100.0 * i.case.load_lbf / l.limit_load_lbf, 100.0 * l.curve.iter().max_by(|a, b| a.load_lbf.total_cmp(&b.load_lbf)).map_or(0.0, |c| c.plastic_fraction))));
        }
    }
    if let (Some(f), Some(e)) = (&run.finite, i.ductility) {
        out.push(Line::from(""));
        out.push(Line::from(Span::styled("Ultimate (finite-strain collapse)", theme.title_style(false).add_modifier(Modifier::BOLD))));
        out.push(Line::from(format!("  Collapse load {:.0} lbf, true stress-strain, failure strain {:.2}{}", f.collapse_lbf, e, if f.strain_limited || f.peak_reached { "" } else { "   (lower bound: the travel cap ended the run)" })));
        if i.case.load_lbf > 0.0 {
            out.push(Line::from(format!("  Applied {:.0} lbf = {:.0} % of collapse    pin travel at collapse {:.4} in    {} steps in {:.1} s", i.case.load_lbf, 100.0 * i.case.load_lbf / f.collapse_lbf, f.curve.last().map_or(0.0, |c| c.travel), f.curve.len(), f.elapsed_ms / 1e3)));
        }
    }
    if let (Some(b), Some(bi)) = (&s.bushing, &i.bushing) {
        out.push(Line::from(""));
        out.push(Line::from(Span::styled("Bushing and fit", theme.title_style(false).add_modifier(Modifier::BOLD))));
        out.push(Line::from(format!("  Fit pressure {} unloaded (interference {:+.4} in, friction {:.2})", ksi(b.fit_pressure_unloaded), bi.interference_dia, bi.friction)));
        out.push(Line::from(format!("  At the load: mean {}  peak {}  {:.0} % of the interface has lost contact", ksi(b.interface_mean_pressure), ksi(b.interface_peak_pressure), 100.0 * b.separated_fraction)));
        out.push(Line::from(format!("  Bushing: peak hoop {} at {:.0} deg  peak von Mises {}  bearing P/(ID t) {}", ksi(b.peak_hoop), b.peak_hoop_angle_deg, ksi(b.peak_von_mises), ksi(b.bearing_stress))));
    }
    if let Some((t, hoop)) = &run.thickness {
        out.push(Line::from(""));
        out.push(Line::from(Span::styled("Pin bending (through the thickness)", theme.title_style(false).add_modifier(Modifier::BOLD))));
        let n = t.slice_load.len();
        let mean = i.case.load_lbf / i.geometry.thickness;
        let bars: String = t.slice_load.iter().map(|q| ["\u{2581}", "\u{2582}", "\u{2583}", "\u{2584}", "\u{2585}", "\u{2586}", "\u{2587}", "\u{2588}"][(((q / mean) / t.peaking.max(1e-9)) * 7.0).round().clamp(0.0, 7.0) as usize]).collect();
        out.push(Line::from(format!("  Bearing by slice (clevis ear at both ends): {bars}   peaking {:.2} x the mean over {n} slices", t.peaking)));
        out.push(Line::from(format!("  Pin: moment {:.0} lbf-in, bending {}, shear {}    hoop at the most loaded slice {}", t.max_moment, ksi(t.pin_bending_stress), ksi(t.pin_shear_stress), ksi(*hoop))));
    }
    out.push(Line::from(""));
    out.push(Line::from(Span::styled("Stress (elastic FE)", theme.title_style(false).add_modifier(Modifier::BOLD))));
    out.push(Line::from(format!("  Lug: peak hoop {} at {:.0} deg    peak von Mises {}", ksi(s.peak_hoop), s.peak_hoop_angle_deg, ksi(s.peak_von_mises))));
    out.push(Line::from(format!("  Net section {}    Kt (net) {:.2}", ksi(s.net_section_stress), s.kt_net)));
    if let Some(c) = &run.comparison {
        out.extend(comparison_lines(theme, run, c));
    }
    out
}

/// The legacy solver's numbers beside the kernel's, with the difference of each.
fn comparison_lines<'a>(theme: &Theme, run: &LugRun, c: &model::Comparison) -> Vec<Line<'a>> {
    let k = &run.solution;
    let l = &c.solution;
    let diff = |a: f64, b: f64| if b.abs() > 1e-12 { format!("{:+.1} %", 100.0 * (a / b - 1.0)) } else { "-".to_string() };
    let row = |name: &str, a: String, b: String, d: String| Line::from(format!("{name:<22}{a:>13}{b:>13}{d:>10}"));
    let mut out = vec![
        Line::from(""),
        Line::from(Span::styled("Solver comparison (kernel against legacy condensed)", theme.title_style(false).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(format!("{:<22}{:>13}{:>13}{:>10}", "", "kernel", "legacy", "diff"), theme.disabled_style())),
        row("Peak hoop", ksi(k.peak_hoop), ksi(l.peak_hoop), diff(k.peak_hoop, l.peak_hoop)),
        row("Peak von Mises", ksi(k.peak_von_mises), ksi(l.peak_von_mises), diff(k.peak_von_mises, l.peak_von_mises)),
        row("Peak pressure", ksi(k.peak_pressure), ksi(l.peak_pressure), diff(k.peak_pressure, l.peak_pressure)),
        row("Pin travel", format!("{:.5} in", k.bearing_deflection), format!("{:.5} in", l.bearing_deflection), diff(k.bearing_deflection, l.bearing_deflection)),
        row("Contact patch", format!("{:.1} deg", k.contact_arc_deg), format!("{:.1} deg", l.contact_arc_deg), format!("{:+.1} deg", k.contact_arc_deg - l.contact_arc_deg)),
    ];
    if let (Some(a), Some(b)) = (&run.limit, &c.limit) {
        out.push(row("Collapse load", format!("{:.0} lbf", a.limit_load_lbf), format!("{:.0} lbf", b.limit_load_lbf), diff(a.limit_load_lbf, b.limit_load_lbf)));
    }
    if let (Some(a), Some(b)) = (&run.finite, &c.finite) {
        out.push(row("Collapse (finite)", format!("{:.0} lbf", a.collapse_lbf), format!("{:.0} lbf", b.collapse_lbf), diff(a.collapse_lbf, b.collapse_lbf)));
    }
    out.push(Line::from(Span::styled(format!("legacy solver: {:.0} ms; kernel: {:.0} ms", c.ms, run.total_ms), theme.disabled_style())));
    out
}

fn model_line(run: &LugRun) -> String {
    let s = &run.solution;
    format!(
        "FE ({}): {} nodes, {} dofs, {} model, {} Newton its; {:.0} ms total ({}{}); mesh aspect max {:.1}, bore {:.1}",
        if run.solver == model::SolverChoice::Kernel { "kernel" } else { "legacy" },
        s.mesh.nodes,
        s.dofs,
        if run.input.case.is_axial() { "half-lug" } else { "full-lug" },
        s.contact_iterations,
        run.total_ms,
        if run.reused_model { "model reused" } else { "model built" },
        match (&run.limit, run.limit_reused) {
            (Some(_), true) => ", collapse reused",
            (Some(_), false) => ", collapse computed",
            _ => "",
        },
        s.mesh.aspect_max,
        s.mesh.bore_aspect_max
    )
}

/// Bore angles every 15 degrees: contact pressure, friction and hoop stress.
fn profile_lines<'a>(theme: &Theme, run: &LugRun) -> Vec<Line<'a>> {
    let s = &run.solution;
    let symmetric = run.input.case.is_axial();
    let fold = |a: f64| -> f64 {
        let a = a.rem_euclid(360.0);
        if symmetric && a > 180.0 { 360.0 - a } else { a }
    };
    let diff = |a: f64, b: f64| {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    };
    let max_hoop = s.bore.iter().map(|b| b.hoop.abs()).fold(1.0, f64::max);
    let mut out = vec![
        Line::from(Span::styled("Bore profile (angle about the hole centre; the load acts toward the pin direction)", theme.title_style(false).add_modifier(Modifier::BOLD))),
        // No leading spaces anywhere: the paragraph trims them per line, which would shear the columns.
        Line::from(Span::styled(format!("{:<6}{:>9}{:>11}{:>11}{}   hoop 0 .. peak", "angle", "pressure", "friction", "hoop", if s.bushing.is_some() { format!("{:>10}", "fit p") } else { String::new() }), theme.disabled_style())),
    ];
    for k in 0..24 {
        let ang = k as f64 * 15.0;
        let want = fold(ang);
        let bore = s.bore.iter().min_by(|a, b| diff(a.angle_deg, want).total_cmp(&diff(b.angle_deg, want)));
        let cp = s.contact.iter().min_by(|a, b| diff(a.angle_deg, want).total_cmp(&diff(b.angle_deg, want)));
        let (Some(bore), Some(cp)) = (bore, cp) else { continue };
        let mirrored = symmetric && ang.rem_euclid(360.0) > 180.0;
        let shear = if mirrored { -cp.shear } else { cp.shear };
        let bar_len = ((bore.hoop.max(0.0) / max_hoop) * 24.0).round() as usize;
        let fit = s.bushing.as_ref().map(|b| {
            let nearest = b.interface.iter().min_by(|x, y| diff(x.angle_deg, want).total_cmp(&diff(y.angle_deg, want)));
            format!("{:>10.1}", nearest.map_or(0.0, |p| p.pressure) / 1000.0)
        });
        out.push(Line::from(format!("{ang:<6.0}{:>9.1}{:>11.1}{:>11.1}{}   {}", cp.pressure / 1000.0, shear / 1000.0, bore.hoop / 1000.0, fit.unwrap_or_default(), "\u{2588}".repeat(bar_len))));
    }
    out.push(Line::from(Span::styled(if s.bushing.is_some() { "pressure, friction and hoop in ksi on the pin-bearing surface (the bushing's bore); fit p is the bushing-to-lug interface pressure" } else { "pressure, friction and hoop in ksi; hoop is the tangential stress on the bore surface" }, theme.disabled_style())));
    if let Some(l) = &run.limit {
        out.push(Line::from(""));
        out.push(Line::from(Span::styled("Plastic load-travel curve (pin travel from its force-free position)", theme.title_style(false).add_modifier(Modifier::BOLD))));
        out.push(Line::from(Span::styled(format!("{:<10}{:>11}{:>10}", "travel in", "load lbf", "plastic"), theme.disabled_style())));
        for c in &l.curve {
            out.push(Line::from(format!("{:<10.4}{:>11.0}{:>9.0}%", c.travel, c.load_lbf, 100.0 * c.plastic_fraction)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn rendered(state: &LugAnalysisState, w: u16, h: u16) -> (String, crate::mouse::MouseRegions) {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, Rect::new(0, 0, w, h), &Theme::default_palette(), state, true, &mut regions)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text = (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n").collect();
        (text, regions)
    }

    fn solved() -> LugAnalysisState {
        let mut s = LugAnalysisState::default();
        let input = s.model.input().unwrap();
        let (res, cache) = model::run(&input, None);
        s.run = Some(res.unwrap());
        s.cache_for_test(cache);
        s.seen = Some((input, std::time::Instant::now()));
        s
    }

    #[test]
    fn before_any_result_the_readout_says_it_is_waiting() {
        let (text, regions) = rendered(&LugAnalysisState::default(), 140, 36);
        assert!(text.contains("Waiting for the first analysis") && text.contains("Lug Geometry") && text.contains("Hole Diameter"), "{text}");
        assert!(!regions.lug_analysis_rows.is_empty());
    }

    #[test]
    fn a_solved_lug_shows_contact_stress_and_every_check() {
        let (text, _) = rendered(&solved(), 150, 44);
        for want in ["Contact", "Stress (elastic FE)", "Peak hoop", "Kt (net)", "Checks", "Bearing", "Net section", "First yield", "FE ("] {
            assert!(text.contains(want), "missing {want}:\n{text}");
        }
    }

    #[test]
    fn the_d_panel_adds_the_bore_profile() {
        let mut s = solved();
        s.show_numbers = true;
        s.results_scroll = 200; // scroll past the summary to the profile
        let (text, _) = rendered(&s, 150, 44);
        assert!(text.contains("Bore profile") || text.contains("pressure, friction and hoop"), "{text}");
    }

    #[test]
    fn invalid_inputs_show_the_reason() {
        let mut s = LugAnalysisState::default();
        s.model.commit_number(model::NumberTarget::HoleDia, 1.9);
        s.tick();
        let (text, _) = rendered(&s, 140, 36);
        assert!(text.contains("width must exceed"), "{text}");
        assert!(text.contains('\u{26a0}'), "the offending field is flagged inline");
    }

    #[test]
    fn the_material_browser_overlays_the_workspace() {
        let s = LugAnalysisState { material_browser: Some(crate::toolboxes::material_lookup::MaterialLookupState { picking: true, ..Default::default() }), ..Default::default() };
        let (text, _) = rendered(&s, 150, 44);
        assert!(text.contains("Choose a material") && text.contains("Enter: use this material"), "{text}");
    }

    #[test]
    fn the_mesh_header_shows_its_state_and_summary_and_is_a_clickable_row() {
        let mut s = LugAnalysisState::default();
        let (text, regions) = rendered(&s, 150, 50);
        assert!(text.contains("\u{25b8} Mesh") && text.contains("72 around the bore"), "collapsed header with its summary:\n{text}");
        assert!(!text.contains("Max Growth Ratio"));
        s.model.mesh_open = true;
        let (text, _) = rendered(&s, 150, 60);
        assert!(text.contains("\u{25be} Mesh") && text.contains("Elements Around Bore") && text.contains("Max Growth Ratio") && text.contains("First Layer Aspect") && text.contains("Mesh Size Test") && text.contains("not run"), "{text}");
        assert!(!regions.lug_analysis_rows.is_empty());
    }

    #[test]
    fn a_comparison_run_shows_the_legacy_numbers_beside_the_kernel() {
        let mut s = LugAnalysisState::default();
        s.model.solver = model::SolverChoice::Compare;
        s.model.plastic = false;
        let input = s.model.input().unwrap();
        let (res, cache) = model::run(&input, None);
        s.run = Some(res.unwrap());
        s.cache_for_test(cache);
        let lines: String = readout_lines(&Theme::default_palette(), &s).iter().map(|l| l.spans.iter().map(|x| x.content.to_string()).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert!(lines.contains("Solver comparison") && lines.contains("legacy") && lines.contains("Peak hoop") && lines.contains("Pin travel") && lines.contains("FE (kernel)"), "{lines}");
    }

    #[test]
    fn narrow_and_degenerate_sizes_do_not_panic() {
        let mut s = solved();
        s.show_numbers = true;
        s.material_browser = Some(crate::toolboxes::material_lookup::MaterialLookupState::default());
        for (w, h) in [(1, 1), (12, 5), (40, 10), (80, 24), (200, 6), (60, 70)] {
            rendered(&s, w, h);
        }
    }

    #[test]
    fn an_old_result_is_marked_when_the_inputs_have_changed() {
        let mut s = solved();
        s.model.commit_number(model::NumberTarget::Load, 1234.0);
        let lines: String = readout_lines(&Theme::default_palette(), &s).iter().map(|l| l.spans.iter().map(|x| x.content.to_string()).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert!(lines.contains("last completed analysis"), "{lines}");
    }
}

#[cfg(test)]
mod alignment_tests {
    use super::*;

    /// The paragraph trims leading spaces from every line, so a table whose rows
    /// start with padding would shear; every profile row must start with text.
    #[test]
    fn profile_rows_start_with_text_and_share_one_column_layout() {
        let mut s = LugAnalysisState::default();
        let input = s.model.input().unwrap();
        let (res, _) = model::run(&input, None);
        s.run = Some(res.unwrap());
        let lines = profile_lines(&Theme::default_palette(), s.run.as_ref().unwrap());
        let text: Vec<String> = lines.iter().map(|l| l.spans.iter().map(|x| x.content.to_string()).collect()).collect();
        assert!(text.iter().all(|l| l.is_empty() || !l.starts_with(' ')), "{text:?}");
        let header_cols = text[1].find("pressure").unwrap() + "pressure".len();
        for row in &text[2..2 + 24] {
            // right-aligned columns end at the same character as their header.
            let end = row.char_indices().nth(6 + 9).map(|(i, _)| i).unwrap();
            assert!(row[..end].trim_end().len() <= header_cols, "{row}");
        }
        assert!(text[2 + 24].contains("pressure, friction and hoop"));
        assert!(text.iter().any(|l| l.contains("Plastic load-travel curve")), "the collapse curve follows the profile");
    }
}
