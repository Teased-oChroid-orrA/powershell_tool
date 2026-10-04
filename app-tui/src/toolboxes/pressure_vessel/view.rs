//! Rendering for the Pressure Vessel Analyzer toolbox. Wide terminals show
//! the editable field list beside a live-updating readout of every derived
//! result (governing failure mode, classification, spec summary,
//! minimum-thickness solve, every check); narrow terminals stack the two
//! vertically - same responsive rule, and the same `fields_required_width`
//! technique, as `toolboxes/fastener_hole/view.rs` uses (that module's own
//! `AGENTS.md` pitfall entry documents the truncation bug a fixed-percentage
//! split caused there; this view avoids it from the start, and extends the
//! same "size for the actual worst-case content" discipline to the inline
//! per-field validation hints added here).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::theme::{StatusTone, Theme};

use mechanics_core::lame::lame_constants;
use pressure_vessel_solver::buckling::BucklingApplicability;
use pressure_vessel_solver::failure::{governing, tresca_stress, von_mises_stress, MarginResult};
use pressure_vessel_solver::geometry::{classify, GeometryClassification};
use pressure_vessel_solver::stress::{stress_at_inner_surface_with_thermal, stress_at_outer_surface_with_thermal, StressState};
use pressure_vessel_solver::thickness::ThicknessSolverOutcome;

use super::model::{self, FieldRow, PressureVesselModel};
use super::PressureVesselState;

const MIN_READOUT_WIDTH: u16 = 34;

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, state: &PressureVesselState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(" Pressure Vessel Analyzer - Space/Enter: toggle/pick/edit \u{b7} d: details \u{b7} e: export ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let required_fields_width = fields_required_width(state);
    let (fields_area, readout_area) = if inner.width >= required_fields_width + MIN_READOUT_WIDTH {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(required_fields_width), Constraint::Min(MIN_READOUT_WIDTH)])
            .split(inner);
        (cols[0], cols[1])
    } else {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(50), Constraint::Percentage(50)]).split(inner);
        (rows[0], rows[1])
    };

    regions.workspace_panes.push((area, super::PANE_MAIN));
    draw_fields(frame, fields_area, theme, state, focused, regions);
    draw_readout(frame, readout_area, theme, &state.model, state.show_numbers, state.results_scroll);
}

fn compute_label_width(rows: &[FieldRow]) -> u16 {
    rows.iter().filter(|r| !matches!(r, FieldRow::Header(_))).map(|r| model::row_label(*r).len()).max().unwrap_or(0) as u16 + 2
}

fn row_hint(model: &PressureVesselModel, row: FieldRow) -> Option<String> {
    match row {
        FieldRow::Number(target) => target.validation_hint(model),
        _ => None,
    }
}

fn hint_span(hint: &str, theme: &Theme) -> Span<'static> {
    Span::styled(format!("  \u{26a0} {hint}"), theme.status_style(StatusTone::Danger))
}

/// Total pane width (border included) the fields list needs so no row's
/// `marker + label + value` (plus its inline validation hint, when
/// present) is ever clipped by the list widget - the same fix, for the
/// same reason, as `fastener_hole/view.rs::fields_required_width`, now
/// also covering the hint text this view adds.
fn fields_required_width(state: &PressureVesselState) -> u16 {
    let rows = model::field_rows();
    let label_width = compute_label_width(&rows);
    let max_value_width = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let value_len = if state.editing && i == state.selected {
                state.edit_buffer.chars().count() + 1
            } else {
                display_value(&state.model, *row).chars().count()
            }
            .min(crate::widgets::scroll_list::VALUE_CAP);
            let hint_len = row_hint(&state.model, *row).map(|h| h.chars().count() + 4).unwrap_or(0); // "  ⚠ " + text
            value_len + hint_len
        })
        .max()
        .unwrap_or(0) as u16;
    // "> " marker (2) + label + value(+hint) + left/right block border (2).
    2 + label_width + max_value_width + 2
}

fn display_value(model: &PressureVesselModel, row: FieldRow) -> String {
    match row {
        FieldRow::Header(_) => String::new(),
        FieldRow::ToggleEndCondition => if model.closed_ends { "Closed ends" } else { "Open ends" }.to_string(),
        FieldRow::OpenMaterialPicker => model.material().name.to_string(),
        FieldRow::Number(target) => target.format_value(model.number_value(target)),
    }
}

fn draw_fields(frame: &mut Frame, area: Rect, theme: &Theme, state: &PressureVesselState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(focused)).title(" Inputs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = model::field_rows();
    let hint = rows.get(state.selected).map(|r| model::field_hint(*r)).unwrap_or("");
    // Reserve at least 4 rows for the field list itself - the hint panel
    // may grow to fit a long wrapped hint, but never past the point of
    // crushing the list to nothing (same technique
    // `toolboxes/bushing/view.rs::draw_fields` uses).
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
            let selected = focused && i == state.selected;
            let value = if selected && state.editing {
                state.edit_buffer.with_cursor()
            } else {
                display_value(&state.model, *row)
            };
            let marker = if selected { "> " } else { "  " };
            let label = model::row_label(*row);
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            let extra = row_hint(&state.model, *row).map(|hint| hint_span(&hint, theme));
            let (item, h) = crate::widgets::scroll_list::field_item(marker, label, label_width, &value, list_area.width, style, extra);
            heights.push(h);
            item
        })
        .collect();

    let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(state.selected));
    regions.pressure_vessel_rows.extend(crate::mouse::list_row_regions_var(list_area, offset, &heights));

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

/// `em dash` for an unbounded (infinite) margin - no governing demand for
/// that check - matching both existing GUI heads' own `fmt_margin`
/// (`app/src/components.rs`, `app-egui/src/components.rs`).
fn fmt_margin(margin: f64) -> String {
    if margin.is_infinite() { "\u{2014}".to_string() } else { format!("{margin:+.2}") }
}

fn draw_readout(frame: &mut Frame, area: Rect, theme: &Theme, model: &PressureVesselModel, show_numbers: bool, scroll: u16) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(" Results ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let lines = readout_lines(theme, model, show_numbers);
    crate::widgets::scroll_paragraph::render(frame, inner, theme, lines, scroll);
}

fn readout_lines<'a>(theme: &'a Theme, model: &'a PressureVesselModel, show_numbers: bool) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    let (geometry, pressure) = match (&model.geometry, &model.pressure) {
        (Ok(g), Ok(p)) => (*g, *p),
        (g, p) => {
            if let Err(e) = g {
                lines.push(Line::from(Span::styled(format!("Invalid geometry: {e:?}"), theme.status_style(StatusTone::Danger))));
            }
            if let Err(e) = p {
                lines.push(Line::from(Span::styled(format!("Invalid pressure: {e:?}"), theme.status_style(StatusTone::Danger))));
            }
            return lines;
        }
    };

    if model.rows.is_empty() {
        return lines;
    }

    let governing_result = governing(&model.rows).clone();
    let classification = classify(&geometry);
    let passed = model.rows.iter().filter(|r| r.margin.is_finite() && r.margin >= 0.0).count();
    let total = model.rows.len();
    let all_pass = passed == total;

    let (headline_text, headline_tone) = if all_pass { ("PASS", StatusTone::Success) } else { ("REVIEW", StatusTone::Danger) };
    lines.push(Line::from(vec![
        Span::styled(headline_text, theme.status_style(headline_tone).add_modifier(Modifier::BOLD)),
        Span::raw(format!("  {passed} / {total} checks passed")),
    ]));
    lines.push(Line::from(format!("Governing: {} ({})", governing_result.name, fmt_margin(governing_result.margin))));
    lines.push(Line::from(format!(
        "Classification: {}",
        if classification == GeometryClassification::ThinWall { "Thin-wall" } else { "Thick-wall" }
    )));
    lines.push(Line::from(format!("Wall thickness: {:.4} in", geometry.wall_thickness())));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Vessel Specification", theme.title_style(false))));
    lines.push(Line::from(format!("  Outer diameter   {:.4} in", 2.0 * geometry.outer_radius)));
    lines.push(Line::from(format!("  Inner diameter   {:.4} in", 2.0 * geometry.inner_radius)));
    lines.push(Line::from(format!("  Design pressure  {:.0} psi internal, {:.0} psi external", pressure.internal_pressure, pressure.external_pressure)));
    lines.push(Line::from(format!("  Material         {}", model.material().name)));
    match &model.thickness_outcome {
        Some(ThicknessSolverOutcome::Converged(sol)) => {
            lines.push(Line::from(format!(
                "  Min. wall thickness for required MS {:.2} (pressure only, not thermal-aware): {:.4} in",
                model.required_ms, sol.wall_thickness
            )));
        }
        Some(ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found }) => {
            lines.push(Line::from(Span::styled(
                format!("  No wall thickness up to {largest_radius_tried:.1} in satisfies the required MS (best margin found: {})", fmt_margin(*best_margin_found)),
                theme.status_style(StatusTone::Warning),
            )));
        }
        None => {}
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Checks", theme.title_style(false))));
    for r in &model.rows {
        lines.push(check_line(r));
    }
    match &model.buckling {
        BucklingApplicability::NotApplicable => {
            lines.push(Line::from(Span::styled("  Buckling: not applicable - no external pressure entered.", theme.disabled_style())));
        }
        BucklingApplicability::InsufficientData => {
            lines.push(Line::from(Span::styled("  Buckling: insufficient data - enter an unsupported length.", theme.disabled_style())));
        }
        BucklingApplicability::OutsideValidityRange => {
            lines.push(Line::from(Span::styled("  Buckling: outside the formulas' validity range (OD/t < 40).", theme.disabled_style())));
        }
        BucklingApplicability::Evaluated(_) => {}
    }
    if model.temperature_differential == 0.0 {
        lines.push(Line::from(Span::styled("  Thermal: not applicable - no temperature differential entered.", theme.disabled_style())));
    } else {
        lines.push(Line::from(format!(
            "  Thermal: {:+.1} \u{b0}F (inner - outer) superposed into the four checks above, not shown as a separate row.",
            model.temperature_differential
        )));
    }

    if show_numbers {
        lines.push(Line::from(""));
        lines.extend(numbers_panel_lines(theme, model, &geometry, &pressure));
    }

    lines
}

fn check_line(r: &MarginResult) -> Line<'static> {
    let color = tone_color(tone_for_margin(r.margin));
    Line::from(vec![
        Span::raw(format!("  {:<24}", r.name)),
        Span::raw(format!("{:>9.0} / {:<9.0}", r.applied, r.allowable)),
        Span::styled(format!(" {}", fmt_margin(r.margin)), Style::default().fg(color)),
    ])
}

/// Maps a [`StatusTone`] to a concrete `ratatui` color without borrowing a
/// `Theme` - lets [`check_line`] and [`numbers_panel_lines`] build
/// `'static` `Line`s. Kept in exact sync with `Theme::status_style`'s own
/// color choice by construction (both switch on the same enum).
fn tone_color(tone: StatusTone) -> Color {
    match tone {
        StatusTone::Neutral => Color::Reset,
        StatusTone::Success => Color::Green,
        StatusTone::Warning => Color::Yellow,
        StatusTone::Danger => Color::Red,
        StatusTone::Info => Color::Blue,
    }
}

/// Text-only substitute for the KaTeX derivation view neither GUI head's
/// own port carries over here (see this toolbox's `mod.rs` doc comment) -
/// the actual cross-section sketch stays cut (genuinely impossible in a
/// terminal), but the derivation's real content - formula substitution
/// with the vessel's live numbers - is real to port: the formula labels
/// and substitution shape below mirror `app/src/pressure_vessel_workbench.rs`'s
/// `pv_derivation_value` (its `lame_constants_solved`/`pv_hoop_at_inner_surface`/
/// `pv_closed_end_axial_stress`/`pv_von_mises_stress`/`pv_tresca_stress`/
/// `pv_windenburg_trilling` cases), reused as plain text instead of that
/// function's PNG-formula-image + substituted-string pairing - no new
/// solver math, every value already flows from `pressure-vessel-solver`
/// into this model. Thermal stress (when a temperature differential is
/// entered) is folded into the shown radial/hoop values via the same
/// `*_with_thermal` functions `PressureVesselModel::recompute` itself
/// uses, so this panel never drifts from what the Checks above actually
/// evaluated.
fn numbers_panel_lines<'a>(
    theme: &'a Theme,
    model: &'a PressureVesselModel,
    geometry: &pressure_vessel_solver::geometry::CylinderGeometry,
    pressure: &pressure_vessel_solver::pressure::PressureLoading,
) -> Vec<Line<'a>> {
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled("Numbers (d to hide)", theme.title_style(false))));

    let a = geometry.inner_radius;
    let b = geometry.outer_radius;
    let (c1, c2) = lame_constants(a, b, pressure.internal_pressure, pressure.external_pressure);
    lines.push(Line::from(Span::styled("  sigma_theta(r) = C1 + C2/r^2,  sigma_r(r) = C1 - C2/r^2", theme.disabled_style())));
    lines.push(Line::from(format!(
        "  a = {a:.4} in, b = {b:.4} in, p_i = {:.0} psi, p_o = {:.0} psi -> C1 = {c1:.1} psi, C2 = {c2:.1} psi\u{b7}in\u{b2}",
        pressure.internal_pressure, pressure.external_pressure
    )));
    lines.push(Line::from(""));

    let thermal = model.thermal_loading(model.material());
    let inner = stress_at_inner_surface_with_thermal(geometry, pressure, thermal.as_ref());
    let outer = stress_at_outer_surface_with_thermal(geometry, pressure, thermal.as_ref());

    lines.push(Line::from(format!(
        "  Inner surface: hoop = C1 + C2/a^2 = {inner_hoop:.0} psi; radial = C1 - C2/a^2 = {inner_radial:.0} psi (boundary condition: -p_i)",
        inner_hoop = inner.hoop,
        inner_radial = inner.radial
    )));
    if model.closed_ends {
        lines.push(Line::from(format!(
            "  Closed-end axial stress (force equilibrium on the end cap, = C1) = {:.0} psi",
            inner.axial
        )));
    }
    lines.push(Line::from(format!(
        "  von Mises: sigma1={:.0} (radial), sigma2={:.0} (hoop), sigma3={:.0} (axial) -> sigma_vm = {:.0} psi",
        inner.radial,
        inner.hoop,
        inner.axial,
        von_mises_stress(&inner)
    )));
    let governing_result = governing(&model.rows).clone();
    lines.push(Line::from(format!(
        "  Tresca: max - min of the three principal stresses = {:.0} psi{}",
        tresca_stress(&inner),
        if governing_result.name == "Tresca (max shear)" { " (governs - lowest margin for this vessel)" } else { "" }
    )));
    lines.push(Line::from(""));

    for (label, s) in [("Inner surface", inner), ("Outer surface", outer)] {
        lines.push(Line::from(Span::styled(label, theme.title_style(false))));
        lines.extend(surface_breakdown_lines(s));
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "  Buckling: p_cr = D(n^2-1)/r^3 (n=2 ring limit), via max() with the Windenburg-Trilling short-span formula",
        theme.disabled_style(),
    )));
    match &model.buckling {
        BucklingApplicability::Evaluated(result) => {
            lines.push(Line::from(format!(
                "  Governing critical pressure = {:.0} psi (applied {:.0} psi, MS = {})",
                result.allowable,
                result.applied,
                fmt_margin(result.margin)
            )));
        }
        BucklingApplicability::NotApplicable => lines.push(Line::from("  Not evaluated: no external pressure entered.")),
        BucklingApplicability::InsufficientData => lines.push(Line::from("  Not evaluated: no unsupported length entered.")),
        BucklingApplicability::OutsideValidityRange => lines.push(Line::from("  Not evaluated: outside the formulas' thin-shell validity range (OD/t < 40).")),
    }

    lines
}

fn surface_breakdown_lines(s: StressState) -> Vec<Line<'static>> {
    vec![
        Line::from(format!("  Radial     {:>10.0} psi", s.radial)),
        Line::from(format!("  Hoop       {:>10.0} psi", s.hoop)),
        Line::from(format!("  Axial      {:>10.0} psi", s.axial)),
        Line::from(format!("  Von Mises  {:>10.0} psi", von_mises_stress(&s))),
        Line::from(format!("  Tresca     {:>10.0} psi", tresca_stress(&s))),
    ]
}

/// Plain-text export - vessel spec, governing/classification, every check,
/// buckling/thermal notes, and the minimum-thickness solve outcome. Pure
/// and synchronous (no filesystem access) so `mod.rs`'s `e` key can build
/// it directly in the reducer and hand the finished string to `main.rs`
/// via `Effect::ExportPressureVesselReport` - the reducer itself still
/// never touches the filesystem (see this crate's own `Contracts` in
/// `AGENTS.md`).
pub fn build_report_text(model: &PressureVesselModel) -> String {
    let mut out = String::new();
    out.push_str("Pressure Vessel Analyzer Report\n");
    out.push_str("================================\n\n");

    let (geometry, pressure) = match (&model.geometry, &model.pressure) {
        (Ok(g), Ok(p)) => (*g, *p),
        (g, p) => {
            if let Err(e) = g {
                out.push_str(&format!("Invalid geometry: {e:?}\n"));
            }
            if let Err(e) = p {
                out.push_str(&format!("Invalid pressure: {e:?}\n"));
            }
            return out;
        }
    };

    out.push_str(&format!("Outer diameter:   {:.4} in\n", 2.0 * geometry.outer_radius));
    out.push_str(&format!("Inner diameter:   {:.4} in\n", 2.0 * geometry.inner_radius));
    out.push_str(&format!("Wall thickness:   {:.4} in\n", geometry.wall_thickness()));
    out.push_str(&format!("Internal pressure: {:.0} psi\n", pressure.internal_pressure));
    out.push_str(&format!("External pressure: {:.0} psi\n", pressure.external_pressure));
    out.push_str(&format!("End condition:    {}\n", if model.closed_ends { "Closed ends" } else { "Open ends" }));
    out.push_str(&format!("Material:         {}\n", model.material().name));
    if model.temperature_differential != 0.0 {
        out.push_str(&format!("Temperature differential (inner - outer): {:+.1} degF\n", model.temperature_differential));
    }
    out.push_str(&format!("Classification:   {}\n", if classify(&geometry) == GeometryClassification::ThinWall { "Thin-wall" } else { "Thick-wall" }));
    out.push('\n');

    if !model.rows.is_empty() {
        let governing_result = governing(&model.rows);
        out.push_str(&format!("Governing: {} ({})\n\n", governing_result.name, fmt_margin(governing_result.margin)));
        out.push_str("Checks:\n");
        for r in &model.rows {
            out.push_str(&format!("  {:<28} applied {:>10.0} psi  allowable {:>10.0} psi  MS {}\n", r.name, r.applied, r.allowable, fmt_margin(r.margin)));
        }
        out.push('\n');
    }

    match &model.buckling {
        BucklingApplicability::NotApplicable => out.push_str("Buckling: not applicable - no external pressure entered.\n"),
        BucklingApplicability::InsufficientData => out.push_str("Buckling: insufficient data - enter an unsupported length.\n"),
        BucklingApplicability::OutsideValidityRange => out.push_str("Buckling: outside the formulas' validity range (OD/t < 40).\n"),
        BucklingApplicability::Evaluated(_) => {}
    }

    match &model.thickness_outcome {
        Some(ThicknessSolverOutcome::Converged(sol)) => {
            out.push_str(&format!("Minimum wall thickness for required MS {:.2} (pressure only, not thermal-aware): {:.4} in\n", model.required_ms, sol.wall_thickness));
        }
        Some(ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found }) => {
            out.push_str(&format!("No wall thickness up to {largest_radius_tried:.1} in satisfies the required MS (best margin found: {}).\n", fmt_margin(*best_margin_found)));
        }
        None => {}
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::model::NumberTarget;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn draw_at(width: u16, height: u16, state: &PressureVesselState) {
        let backend = TestBackend::new(width.max(1), height.max(1));
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, area, &Theme::default_palette(), state, true, &mut regions)).unwrap();
    }

    #[test]
    fn draw_does_not_panic_at_normal_width() {
        draw_at(140, 30, &PressureVesselState::default());
    }

    /// Regression test for the hint-truncation bug (see `bushing/view.rs`'s
    /// twin test): selects the field with the single longest `field_hint()`
    /// string and asserts every word of it appears somewhere in the
    /// rendered buffer, across a range of widths including narrow ones.
    #[test]
    fn the_longest_hint_is_never_truncated_across_a_range_of_widths() {
        let rows = model::field_rows();
        let (longest_index, longest_hint) =
            rows.iter().enumerate().map(|(i, r)| (i, model::field_hint(*r))).max_by_key(|(_, hint)| hint.len()).expect("at least one field row");
        assert!(!longest_hint.is_empty());

        for width in [40u16, 50, 60, 84, 98, 140] {
            let mut state = PressureVesselState::default();
            state.selected = longest_index;
            let backend = TestBackend::new(width, 40);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &state, true, &mut regions)).unwrap();

            let buffer = terminal.backend().buffer().clone();
            let rendered: String = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n").collect();

            for word in longest_hint.split_whitespace() {
                assert!(rendered.contains(word), "word `{word}` from the longest hint missing at width {width}:\n{rendered}");
            }
        }
    }

    #[test]
    fn draw_does_not_panic_at_narrow_width() {
        draw_at(50, 30, &PressureVesselState::default());
    }

    #[test]
    fn draw_does_not_panic_at_degenerate_sizes() {
        for (w, h) in [(0, 0), (1, 1), (40, 0), (0, 10)] {
            draw_at(w, h, &PressureVesselState::default());
        }
    }

    #[test]
    fn draw_does_not_panic_when_geometry_is_invalid() {
        let mut state = PressureVesselState::default();
        state.model.commit_number(NumberTarget::WallThickness, 10.0);
        draw_at(140, 30, &state);
    }

    #[test]
    fn draw_does_not_panic_with_the_numbers_panel_shown() {
        let mut state = PressureVesselState::default();
        state.show_numbers = true;
        draw_at(140, 40, &state);
        draw_at(50, 40, &state);
    }

    fn rendered_lines(width: u16, height: u16, state: &PressureVesselState) -> Vec<String> {
        let backend = TestBackend::new(width.max(1), height.max(1));
        let mut terminal = Terminal::new(backend).unwrap();
        let area = Rect::new(0, 0, width, height);
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, area, &Theme::default_palette(), state, true, &mut regions)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()).collect()
    }

    /// Regression test for the derivation-formula text added to the
    /// Numbers panel (formula substitution reused from
    /// `app/src/pressure_vessel_workbench.rs::pv_derivation_value` as plain
    /// text) - asserts the formula labels and the boundary-condition/
    /// von-Mises/Tresca substitution lines actually reach the rendered
    /// panel, not just that rendering doesn't panic.
    #[test]
    fn numbers_panel_shows_lame_and_stress_formula_substitution() {
        let mut state = PressureVesselState::default();
        state.show_numbers = true;
        let lines = rendered_lines(160, 60, &state);
        let joined = lines.join("\n");
        assert!(joined.contains("sigma_theta(r) = C1 + C2/r^2"), "missing Lame formula label:\n{joined}");
        assert!(joined.contains("boundary condition"), "missing boundary-condition substitution:\n{joined}");
        assert!(joined.contains("sigma_vm"), "missing von Mises formula substitution:\n{joined}");
        assert!(joined.contains("Tresca: max - min"), "missing Tresca formula substitution:\n{joined}");
        assert!(joined.contains("Buckling: p_cr = D(n^2-1)/r^3"), "missing buckling formula label:\n{joined}");
    }

    #[test]
    fn numbers_panel_buckling_line_shows_governing_pressure_when_evaluated() {
        let mut state = PressureVesselState::default();
        state.show_numbers = true;
        // Thin-wall geometry (OD/t >= 40) is required for buckling to be
        // evaluated at all - the default 6in OD / 1in wall (OD/t = 6) is
        // deliberately outside that validity range.
        state.model.commit_number(NumberTarget::WallThickness, 0.1);
        state.model.commit_number(NumberTarget::ExternalPressure, 5.0);
        state.model.commit_number(NumberTarget::UnsupportedLength, 20.0);
        let lines = rendered_lines(160, 60, &state);
        let joined = lines.join("\n");
        assert!(joined.contains("Governing critical pressure"), "expected an evaluated buckling result:\n{joined}");
    }

    #[test]
    fn draw_does_not_panic_with_a_validation_hint_showing() {
        let mut state = PressureVesselState::default();
        state.model.commit_number(NumberTarget::InternalPressure, -100.0);
        draw_at(140, 30, &state);
        draw_at(50, 30, &state);
    }

    #[test]
    fn field_rows_are_never_truncated_across_a_range_of_widths() {
        let rows = model::field_rows();
        for width in [50u16, 60, 70, 84, 90, 95, 98, 110, 140] {
            let mut state = PressureVesselState::default();
            for (i, row) in rows.iter().enumerate() {
                if matches!(row, FieldRow::Header(_)) {
                    continue; // rendered as "-- text --", not "label + value"
                }
                state.selected = i;
                let backend = TestBackend::new(width, 40);
                let mut terminal = Terminal::new(backend).unwrap();
                let mut regions = crate::mouse::MouseRegions::default();
                terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &state, true, &mut regions)).unwrap();

                let buffer = terminal.backend().buffer().clone();
                let rendered: Vec<String> =
                    (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()).collect();

                let label_width = compute_label_width(&rows) as usize;
                let label = model::row_label(*row);
                let value = display_value(&state.model, *row);
                let expected = format!("{label:<label_width$}{value}");
                let found = rendered.iter().any(|line| line.contains(&expected));
                assert!(found, "row `{label}` truncated at width {width}: expected `{expected}` in:\n{}", rendered.join("\n"));
            }
        }
    }

    /// Same truncation regression, this time forcing every row's
    /// validation hint on at once (an invalid combination of inputs) -
    /// the widest real-world case `fields_required_width` must size for.
    #[test]
    fn field_rows_with_validation_hints_are_never_truncated() {
        let mut state = PressureVesselState::default();
        state.model.commit_number(NumberTarget::OuterDiameter, 0.0);
        state.model.commit_number(NumberTarget::InternalPressure, -1.0);
        state.model.commit_number(NumberTarget::ExternalPressure, -1.0);
        state.model.commit_number(NumberTarget::UnsupportedLength, -1.0);

        let rows = model::field_rows();
        for width in [60u16, 84, 98, 140] {
            for (i, row) in rows.iter().enumerate() {
                state.selected = i;
                let backend = TestBackend::new(width, 40);
                let mut terminal = Terminal::new(backend).unwrap();
                let mut regions = crate::mouse::MouseRegions::default();
                terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &state, true, &mut regions)).unwrap();

                if let Some(hint) = row_hint(&state.model, *row) {
                    let buffer = terminal.backend().buffer().clone();
                    let rendered: Vec<String> =
                        (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()).collect();
                    let found = rendered.iter().any(|line| line.contains(hint.as_str()));
                    assert!(found, "hint `{hint}` for row {i} truncated at width {width}:\n{}", rendered.join("\n"));
                }
            }
        }
    }

    #[test]
    fn build_report_text_contains_the_governing_mode_and_every_check() {
        let model = PressureVesselModel::default();
        let text = build_report_text(&model);
        assert!(text.contains("Governing:"));
        for r in &model.rows {
            assert!(text.contains(r.name), "report must mention check `{}`", r.name);
        }
    }

    #[test]
    fn build_report_text_on_invalid_geometry_still_produces_readable_output() {
        let mut model = PressureVesselModel::default();
        model.commit_number(NumberTarget::WallThickness, 10.0);
        let text = build_report_text(&model);
        assert!(text.contains("Invalid geometry"));
    }
}
