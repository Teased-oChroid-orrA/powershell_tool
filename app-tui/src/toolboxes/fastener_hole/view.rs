//! Rendering for the Fastener Hole toolbox. Wide terminals show the
//! editable field list beside a live-updating readout of every calculated/
//! transferred/preserved/derived value (spec section 29); narrow terminals
//! stack the two vertically (spec section 30) rather than ever hiding a
//! result.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, ListItem, Paragraph};

use crate::theme::{StatusTone, Theme};

use super::domain::{CountersinkSolveFor, FitClassification, GeometryError, SecondaryCountersinkMethod, TolerancedValue};
use super::model::{self, FastenerHoleModel, FieldRow, HoleType, NumberPart, NumberTarget};
use super::FastenerHoleState;

/// Minimum usable width for the "Calculated + Secondary / Derived" column
/// once the fields pane has taken exactly the width it needs - below this,
/// a horizontal split isn't worth it even if the fields pane would fit.
const MIN_READOUT_WIDTH: u16 = 30;

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, state: &FastenerHoleState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(" Fastener Holes - Space/Enter: toggle or edit ");
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

    regions.workspace_panes.push((area, crate::toolboxes::fastener_hole::PANE_MAIN));
    draw_fields(frame, fields_area, theme, state, focused, regions);
    draw_readout(frame, readout_area, theme, &state.model);
}

/// Widest label (in display characters, e.g. `"Hole Type"` or
/// `"Secondary Hole Diameter -Tol"`) across every currently-visible row,
/// plus a 2-column gutter before the value - shared by `draw_fields`
/// (to build each row string) and `fields_required_width` (to size the
/// pane that holds them), so the two can never drift apart.
fn compute_label_width(rows: &[FieldRow]) -> u16 {
    rows.iter()
        .map(|r| match r {
            FieldRow::Number(_, part, label) => label.len() + 1 + part.suffix().len(),
            FieldRow::ToggleHoleType => "Hole Type".len(),
            FieldRow::ToggleToleranceMode => "Tolerance Input".len(),
            FieldRow::ToggleSolveFor => "Solve For".len(),
            FieldRow::ToggleSecondaryMethod => "Secondary Method".len(),
        })
        .max()
        .unwrap_or(0) as u16
        + 2
}

/// Total pane width (border included) the fields list needs so that no
/// row's `marker + label + value` is ever clipped by the list widget -
/// this is the fix for the truncation bug: the old code picked a fixed
/// 45%-of-available-width column regardless of how wide the longest row
/// actually was, so anything but a very wide terminal cut values off
/// right at the pane's border. `draw()` now only uses a horizontal split
/// when this much room is actually available.
fn fields_required_width(state: &FastenerHoleState) -> u16 {
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
    // "> " marker (2) + label + value + left/right block border (2).
    2 + label_width + max_value_width + 2
}

fn origin_span<'a>(theme: &Theme, tag: &'a str, tone: StatusTone) -> Span<'a> {
    Span::styled(format!(" [{tag}]"), theme.status_style(tone))
}

fn unit_suffix(target: NumberTarget) -> &'static str {
    if target.is_angle() { " deg" } else { " in" }
}

fn format_part(target: NumberTarget, part: NumberPart, value: &TolerancedValue) -> String {
    let raw = model::part_value(value, part);
    let text = if target.is_angle() { model::format_angle(raw) } else { model::format_linear(raw) };
    format!("{text}{}", unit_suffix(target))
}

fn draw_fields(frame: &mut Frame, area: Rect, theme: &Theme, state: &FastenerHoleState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(focused)).title(" Primary / Design ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = model::field_rows(&state.model);
    let label_width = compute_label_width(&rows) as usize;

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let selected = focused && i == state.selected;
            let value = if selected && state.editing {
                format!("{}_", state.edit_buffer)
            } else {
                display_value(&state.model, *row)
            };
            let marker = if selected { "> " } else { "  " };
            let label = row_label(*row);
            let style = if selected { theme.selected_row_style() } else { ratatui::style::Style::default() };
            ListItem::new(Line::from(Span::styled(format!("{marker}{label:<label_width$}{value}"), style)))
        })
        .collect();

    let offset = crate::widgets::scroll_list::render(frame, inner, items, focused.then_some(state.selected));
    regions.fastener_rows.extend(crate::mouse::list_row_regions(inner, offset, rows.len()));
}

fn row_label(row: FieldRow) -> String {
    match row {
        FieldRow::ToggleHoleType => "Hole Type".to_string(),
        FieldRow::ToggleToleranceMode => "Tolerance Input".to_string(),
        FieldRow::ToggleSolveFor => "Solve For".to_string(),
        FieldRow::ToggleSecondaryMethod => "Secondary Method".to_string(),
        FieldRow::Number(_, part, label) => format!("{label} {}", part.suffix()),
    }
}

fn display_value(model: &FastenerHoleModel, row: FieldRow) -> String {
    match row {
        FieldRow::ToggleHoleType => model.hole_type.label().to_string(),
        FieldRow::ToggleToleranceMode => model.tolerance_mode.label().to_string(),
        FieldRow::ToggleSolveFor => model.countersink.solve_for.label().to_string(),
        FieldRow::ToggleSecondaryMethod => model.countersink.secondary_method.label().to_string(),
        FieldRow::Number(target, part, _) => format_part(target, part, &model.get_toleranced(target)),
    }
}

fn draw_readout(frame: &mut Frame, area: Rect, theme: &Theme, model: &FastenerHoleModel) {
    let title = match model.hole_type {
        HoleType::Regular => " Secondary / Derived + Fit Analysis ",
        HoleType::Countersunk => " Calculated + Secondary / Derived ",
    };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let lines = match model.hole_type {
        HoleType::Regular => regular_readout_lines(theme, model),
        HoleType::Countersunk => countersink_readout_lines(theme, model),
    };
    frame.render_widget(Paragraph::new(lines), inner);
}

fn value_line<'a>(theme: &'a Theme, label: impl Into<String>, value: String, tag: &'a str, tone: StatusTone) -> Line<'a> {
    Line::from(vec![Span::raw(format!("{}: ", label.into())), Span::raw(value), origin_span(theme, tag, tone)])
}

fn error_line<'a>(theme: &'a Theme, label: &'a str, err: GeometryError) -> Line<'a> {
    Line::from(vec![Span::raw(format!("{label}: ")), Span::raw("-- "), origin_span(theme, "INVALID", StatusTone::Danger), Span::raw(format!(" {err}"))])
}

fn regular_readout_lines<'a>(theme: &'a Theme, model: &'a FastenerHoleModel) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled("Secondary Companion Hole", theme.title_style(false))));
    match &model.regular_secondary {
        Ok((companion, check)) => {
            lines.push(value_line(theme, "  Nominal", model::format_linear(companion.nominal) + " in", "DERIVED", StatusTone::Info));
            lines.push(value_line(theme, "  Min", model::format_linear(companion.min) + " in", "DERIVED", StatusTone::Info));
            lines.push(value_line(theme, "  Max", model::format_linear(companion.max) + " in", "DERIVED", StatusTone::Info));
            lines.push(value_line(theme, "  -Tol", model::format_linear(companion.tol_minus()), "DERIVED", StatusTone::Info));
            lines.push(value_line(theme, "  +Tol", model::format_linear(companion.tol_plus()), "DERIVED", StatusTone::Info));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Envelope Preservation", theme.title_style(false))));
            let (mark, tone) = if check.preserved { ("preserved", StatusTone::Success) } else { ("NOT preserved", StatusTone::Danger) };
            lines.push(Line::from(Span::styled(format!("  Secondary fit envelope {mark}"), theme.status_style(tone))));
        }
        Err(e) => lines.push(error_line(theme, "  Companion hole", *e)),
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Fit Analysis (Fit = Hole 2 - Hole 1)", theme.title_style(false))));
    let fit = &model.regular_fit;
    lines.push(Line::from(format!("  Min Fit      {} in", model::format_linear(fit.min))));
    lines.push(Line::from(format!("  Nominal Fit  {} in", model::format_linear(fit.nominal))));
    lines.push(Line::from(format!("  Max Fit      {} in", model::format_linear(fit.max))));
    let (label, tone) = match fit.classification {
        FitClassification::Clearance => ("CLEARANCE", StatusTone::Success),
        FitClassification::Transition => ("TRANSITION", StatusTone::Warning),
        FitClassification::Interference => ("INTERFERENCE", StatusTone::Danger),
    };
    lines.push(Line::from(vec![Span::raw("  Classification: "), Span::styled(label, theme.status_style(tone))]));

    lines
}

fn solve_for_label(target: CountersinkSolveFor) -> &'static str {
    match target {
        CountersinkSolveFor::OuterDiameter => "Outer Diameter",
        CountersinkSolveFor::HoleDiameter => "Hole Diameter",
        CountersinkSolveFor::Depth => "Depth",
        CountersinkSolveFor::Angle => "Angle",
    }
}

fn countersink_readout_lines<'a>(theme: &'a Theme, model: &'a FastenerHoleModel) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled("Primary Countersink", theme.title_style(false))));
    match &model.countersink_primary {
        Ok(geometry) => {
            let solved_label = solve_for_label(geometry.solve_for);
            let solved_value = match geometry.solve_for {
                CountersinkSolveFor::OuterDiameter => geometry.outer_diameter,
                CountersinkSolveFor::HoleDiameter => geometry.hole_diameter,
                CountersinkSolveFor::Depth => geometry.depth,
                CountersinkSolveFor::Angle => geometry.angle_deg,
            };
            let target = if geometry.solve_for == CountersinkSolveFor::Angle { NumberTarget::CsAngle } else { NumberTarget::CsDepth };
            let unit = if matches!(geometry.solve_for, CountersinkSolveFor::Angle) { " deg" } else { " in" };
            let fmt = |v: f64| if matches!(geometry.solve_for, CountersinkSolveFor::Angle) { model::format_angle(v) } else { model::format_linear(v) };
            let _ = target;
            lines.push(value_line(theme, &format!("  {solved_label}"), format!("{}{unit}", fmt(solved_value.nominal)), "CALCULATED", StatusTone::Success));
            lines.push(Line::from(format!("    range: {} .. {}{unit}", fmt(solved_value.min), fmt(solved_value.max))));
        }
        Err(e) => lines.push(error_line(theme, "  Geometry", *e)),
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Lateral Surface Area", theme.title_style(false))));
    match &model.countersink_primary_area {
        Ok(area) => {
            lines.push(value_line(theme, "  Primary Area", format!("{} in^2", model::format_area(area.nominal)), "CALCULATED", StatusTone::Success));
        }
        Err(_) => lines.push(Line::from(vec![Span::raw("  Primary Area: -- "), origin_span(theme, "INCOMPLETE", StatusTone::Warning)])),
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!("Secondary Countersink ({})", model.countersink.secondary_method.label()), theme.title_style(false))));
    match &model.countersink_secondary {
        Ok(geometry) => {
            let (depth_tag, depth_tone) = match model.countersink.secondary_method {
                SecondaryCountersinkMethod::PreserveDepth => ("TRANSFERRED", StatusTone::Warning),
                SecondaryCountersinkMethod::PreserveLateralArea => ("CALCULATED", StatusTone::Success),
            };
            let (angle_tag, angle_tone) = ("TRANSFERRED", StatusTone::Warning);
            lines.push(value_line(theme, "  Outer Diameter", format!("{} in", model::format_linear(geometry.outer_diameter.nominal)), "CALCULATED", StatusTone::Success));
            lines.push(value_line(theme, "  Depth", format!("{} in", model::format_linear(geometry.depth.nominal)), depth_tag, depth_tone));
            lines.push(value_line(theme, "  Angle", format!("{} deg", model::format_angle(geometry.angle_deg.nominal)), angle_tag, angle_tone));
            match &model.countersink_secondary_area {
                Ok(area) => {
                    let (area_tag, area_tone) = match model.countersink.secondary_method {
                        SecondaryCountersinkMethod::PreserveDepth => ("CALCULATED", StatusTone::Success),
                        SecondaryCountersinkMethod::PreserveLateralArea => ("PRESERVED", StatusTone::Info),
                    };
                    lines.push(value_line(theme, "  Lateral Surface Area", format!("{} in^2", model::format_area(area.nominal)), area_tag, area_tone));
                }
                Err(_) => lines.push(Line::from(vec![Span::raw("  Lateral Surface Area: -- "), origin_span(theme, "INCOMPLETE", StatusTone::Warning)])),
            }
        }
        Err(e) => lines.push(error_line(theme, "  Geometry", *e)),
    }

    if let Some(check) = &model.countersink_area_check {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Area Preservation Verification", theme.title_style(false))));
        let (mark, tone) = if check.preserved { ("preserved", StatusTone::Success) } else { ("NOT preserved", StatusTone::Danger) };
        lines.push(Line::from(format!("  Primary Area    {} in^2", model::format_area(check.primary_area))));
        lines.push(Line::from(format!("  Secondary Area  {} in^2", model::format_area(check.secondary_area))));
        lines.push(Line::from(format!("  Delta Area      {} in^2", model::format_area(check.delta))));
        lines.push(Line::from(Span::styled(format!("  Area {mark}"), theme.status_style(tone))));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn draw_does_not_panic_at_normal_size_for_both_hole_types() {
        for hole_type in [HoleType::Regular, HoleType::Countersunk] {
            let mut state = FastenerHoleState::default();
            state.model.hole_type = hole_type;
            state.model.recompute();
            let backend = TestBackend::new(120, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &state, true, &mut regions)).unwrap();
        }
    }

    #[test]
    fn draw_does_not_panic_at_narrow_width() {
        let state = FastenerHoleState::default();
        let backend = TestBackend::new(50, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &state, true, &mut regions)).unwrap();
    }

    /// Regression test for the reported bug: at widths that used to land in
    /// the old fixed-45%/`MIN_WIDE_WIDTH = 84` split's failure zone (roughly
    /// 84-98 inner columns - wide enough to trigger the horizontal split,
    /// too narrow for the 45% column to fit the longest row), every field
    /// row's full `label + value` text must render intact once scrolled
    /// into view (selecting a row is what `scroll_list` uses to decide what
    /// to scroll to - this does not assert every row is *simultaneously*
    /// visible, which depends on terminal height and was never guaranteed).
    /// Covers both tolerance-input modes since `MinMax` produces different
    /// (shorter) rows than the default `NominalTol`.
    #[test]
    fn field_rows_are_never_truncated_across_the_old_failure_zone() {
        for width in [60u16, 70, 84, 90, 95, 98, 110, 140] {
            for tolerance_mode in [model::ToleranceInputMode::NominalTol, model::ToleranceInputMode::MinMax] {
                let mut state = FastenerHoleState::default();
                state.model.hole_type = HoleType::Countersunk;
                state.model.tolerance_mode = tolerance_mode;
                state.model.recompute();

                let rows = model::field_rows(&state.model);
                let label_width = compute_label_width(&rows) as usize;

                for (i, row) in rows.iter().enumerate() {
                    state.selected = i;

                    let backend = TestBackend::new(width, 40);
                    let mut terminal = Terminal::new(backend).unwrap();
                    let mut regions = crate::mouse::MouseRegions::default();
                    terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &state, true, &mut regions)).unwrap();

                    let buffer = terminal.backend().buffer().clone();
                    let rendered: Vec<String> =
                        (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()).collect();

                    let label = row_label(*row);
                    let value = display_value(&state.model, *row);
                    let expected = format!("{label:<label_width$}{value}");
                    let found = rendered.iter().any(|line| line.contains(&expected));
                    assert!(
                        found,
                        "selected row `{label}` truncated at width {width} (tolerance_mode={tolerance_mode:?}): expected `{expected}` in:\n{}",
                        rendered.join("\n")
                    );
                }
            }
        }
    }

    #[test]
    fn draw_does_not_panic_at_degenerate_sizes() {
        let state = FastenerHoleState::default();
        for (w, h) in [(0, 0), (1, 1), (40, 0), (0, 10)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let area = Rect::new(0, 0, w, h);
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| draw(f, area, &Theme::default_palette(), &state, false, &mut regions)).unwrap();
        }
    }

    #[test]
    fn draw_does_not_panic_when_countersink_geometry_is_invalid() {
        let mut state = FastenerHoleState::default();
        state.model.hole_type = HoleType::Countersunk;
        // Force an invalid geometry: hole diameter larger than outer diameter.
        state.model.countersink.hole = TolerancedValue::exact(0.9).unwrap();
        state.model.recompute();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, f.area(), &Theme::default_palette(), &state, true, &mut regions)).unwrap();
    }

}
