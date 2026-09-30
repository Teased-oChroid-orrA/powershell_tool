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

use super::model::{self, BushingModel, FieldRow};
use super::BushingState;

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
    draw_readout(frame, readout_area, theme, &state.model, state.show_numbers, state.results_scroll);

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
}

fn compute_label_width(rows: &[FieldRow]) -> u16 {
    rows.iter().filter(|r| !matches!(r, FieldRow::Header(_))).map(|r| model::row_label(*r).len()).max().unwrap_or(0) as u16 + 2
}

fn display_value(model: &BushingModel, row: FieldRow) -> String {
    match row {
        FieldRow::Header(_) => String::new(),
        FieldRow::ToggleFitType => model::label_fit_type(model.fit_type).to_string(),
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

    let label_width = compute_label_width(&rows) as usize;

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            if let FieldRow::Header(text) = row {
                return ListItem::new(Line::from(Span::styled(format!("-- {text} --"), theme.title_style(false).add_modifier(Modifier::BOLD))));
            }
            let selected = focused && i == state.selected;
            let value = if selected && state.editing { format!("{}_", state.edit_buffer) } else { display_value(&state.model, *row) };
            let marker = if selected { "> " } else { "  " };
            let label = model::row_label(*row);
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            ListItem::new(Line::from(Span::styled(format!("{marker}{label:<label_width$}{value}"), style)))
        })
        .collect();

    let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(state.selected));
    regions.bushing_rows.extend(crate::mouse::list_row_regions(list_area, offset, rows.len()));

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

fn draw_readout(frame: &mut Frame, area: Rect, theme: &Theme, model: &BushingModel, show_numbers: bool, scroll: u16) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(" Results ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let lines = readout_lines(theme, model, show_numbers);
    crate::widgets::scroll_paragraph::render(frame, inner, theme, lines, scroll);
}

fn readout_lines<'a>(theme: &'a Theme, model: &'a BushingModel, show_numbers: bool) -> Vec<Line<'a>> {
    let out = &model.output;
    let mut lines = Vec::new();

    let (headline, tone) = if out.enforcement_satisfied && !out.fail_straight && !out.fail_neck && out.governing.margin >= 0.0 {
        ("PASS", StatusTone::Success)
    } else {
        ("REVIEW", StatusTone::Danger)
    };
    lines.push(Line::from(vec![
        Span::styled(headline, theme.status_style(tone).add_modifier(Modifier::BOLD)),
        Span::raw(format!("  governing: {} ({})", out.governing.name, fmt_margin(out.governing.margin))),
    ]));
    lines.push(Line::from(format!(
        "Tolerance: {} ({} note(s))",
        match out.tolerance_status {
            ToleranceStatus::Ok => "OK",
            ToleranceStatus::Clamped => "Clamped",
            ToleranceStatus::Infeasible => "INFEASIBLE",
        },
        out.tolerance_notes.len()
    )));
    for note in &out.tolerance_notes {
        lines.push(Line::from(Span::styled(format!("  \u{26a0} {note}"), theme.status_style(StatusTone::Warning))));
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
    lines.push(Line::from(format!(
        "  Straight wall       {:.4} in  ({}, range {:.4}..{:.4})",
        out.wall_straight,
        if out.fail_straight { "FAIL vs min" } else { "OK" },
        out.wall_straight_range.min,
        out.wall_straight_range.max
    )));
    lines.push(Line::from(format!("  Neck wall           {:.4} in  ({})", out.wall_neck, if out.fail_neck { "FAIL vs min" } else { "OK" })));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Contact / Stress", theme.title_style(false))));
    lines.push(Line::from(format!("  Contact pressure    {:.0} psi  (range {:.0}..{:.0})", out.pressure, out.pressure_range.min, out.pressure_range.max)));
    lines.push(Line::from(vec![
        Span::raw(format!("  Housing hoop stress {:>10.0} psi  MS ", out.stress_hoop_housing)),
        Span::styled(fmt_margin(out.housing_ms), Style::default().fg(tone_color(tone_for_margin(out.housing_ms)))),
    ]));
    lines.push(Line::from(vec![
        Span::raw(format!("  Bushing hoop stress {:>10.0} psi  MS ", out.stress_hoop_bushing)),
        Span::styled(fmt_margin(out.bushing_ms), Style::default().fg(tone_color(tone_for_margin(out.bushing_ms)))),
    ]));
    if out.axial_constraint_factor > 0.0 {
        lines.push(Line::from(format!("  Housing axial stress {:>9.0} psi", out.stress_axial_housing)));
        lines.push(Line::from(format!("  Bushing axial stress {:>9.0} psi", out.stress_axial_bushing)));
    }
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Install", theme.title_style(false))));
    lines.push(Line::from(format!("  Install force       {:.1} lbf  (range {:.1}..{:.1})", out.install_force, out.install_force_range.min, out.install_force_range.max)));
    lines.push(Line::from(format!("  Retained (in-service) force {:.1} lbf", out.retained_install_force)));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Edge Distance", theme.title_style(false))));
    lines.push(Line::from(format!("  Actual e/D          {:.3}", out.ed_actual)));
    lines.push(Line::from(vec![
        Span::raw(format!("  Sequencing margin   min {:.3}  actual/min ", out.ed_min_sequence)),
        Span::styled(fmt_margin(out.sequence_margin), Style::default().fg(tone_color(tone_for_margin(out.sequence_margin)))),
    ]));
    lines.push(Line::from(vec![
        Span::raw(format!("  Strength margin     min {:.3}  actual/min ", out.ed_min_strength)),
        Span::styled(fmt_margin(out.strength_margin), Style::default().fg(tone_color(tone_for_margin(out.strength_margin)))),
    ]));
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("Governing Candidates", theme.title_style(false))));
    for c in &out.candidates {
        lines.push(Line::from(vec![
            Span::raw(format!("  {:<28}", c.name)),
            Span::styled(fmt_margin(c.margin), Style::default().fg(tone_color(tone_for_margin(c.margin)))),
        ]));
    }

    if show_numbers {
        lines.push(Line::from(""));
        lines.extend(numbers_panel_lines(theme, model));
    }

    lines
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
    s.push_str(&format!("Target interference: {:.4} in\n\n", model.interference));

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
}
