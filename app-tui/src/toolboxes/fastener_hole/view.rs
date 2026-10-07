//! Rendering for the Fastener Hole toolbox. Wide terminals show the
//! editable field list beside a live-updating readout of every calculated/
//! transferred/preserved/derived value (spec section 29); narrow terminals
//! stack the two vertically (spec section 30) rather than ever hiding a
//! result.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, ListItem, Paragraph, Wrap};

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
        .title(crate::widgets::title::toolbox_title("Fastener Holes", &["Enter Edit"], area.width));
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
    draw_readout(frame, readout_area, theme, &state.model, state.results_scroll);
}

/// Widest label (in display characters, e.g. `"Hole Type"` or
/// `"Secondary Hole Diameter -Tol"`) across every currently-visible row,
/// plus a 2-column gutter before the value - shared by `draw_fields`
/// (to build each row string) and `fields_required_width` (to size the
/// pane that holds them), so the two can never drift apart.
fn compute_label_width(rows: &[FieldRow]) -> u16 {
    rows.iter()
        .filter(|r| !matches!(r, FieldRow::Header(_)))
        .map(|r| row_label(*r).len())
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

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            if let FieldRow::Header(text) = row {
                return ListItem::new(Line::from(Span::styled(format!("-- {text} --"), theme.title_style(false).add_modifier(Modifier::BOLD))));
            }
            let selected = focused && i == state.selected;
            let value = if selected && state.editing {
                state.edit_buffer.with_cursor()
            } else {
                display_value(&state.model, *row)
            };
            let marker = if selected { "> " } else { "  " };
            let label = row_label(*row);
            let style = if selected { theme.selected_row_style() } else { ratatui::style::Style::default() };
            ListItem::new(Line::from(Span::styled(format!("{marker}{label:<label_width$}{value}"), style)))
        })
        .collect();

    let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(state.selected));
    regions.fastener_rows.extend(crate::mouse::list_row_regions(list_area, offset, rows.len()));

    if let Some(hint_area) = hint_area {
        frame.render_widget(Paragraph::new(Line::from(Span::styled(hint, theme.disabled_style()))).wrap(Wrap { trim: true }), hint_area);
    }
}

fn row_label(row: FieldRow) -> String {
    match row {
        FieldRow::Header(text) => text.to_string(),
        FieldRow::ToggleHoleType => "Hole Type".to_string(),
        FieldRow::ToggleToleranceMode => "Tolerance Input".to_string(),
        FieldRow::ToggleSolveFor => "Solve For".to_string(),
        FieldRow::ToggleSecondaryMethod => "Secondary Method".to_string(),
        FieldRow::Number(_, part, label) => format!("{label} {}", part.suffix()),
    }
}

fn display_value(model: &FastenerHoleModel, row: FieldRow) -> String {
    match row {
        FieldRow::Header(_) => String::new(),
        FieldRow::ToggleHoleType => model.hole_type.label().to_string(),
        FieldRow::ToggleToleranceMode => model.tolerance_mode.label().to_string(),
        FieldRow::ToggleSolveFor => model.countersink.solve_for.label().to_string(),
        FieldRow::ToggleSecondaryMethod => model.countersink.secondary_method.label().to_string(),
        FieldRow::Number(target, part, _) => format_part(target, part, &model.get_toleranced(target)),
    }
}

fn draw_readout(frame: &mut Frame, area: Rect, theme: &Theme, model: &FastenerHoleModel, scroll: u16) {
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
    crate::widgets::scroll_paragraph::render(frame, inner, theme, lines, scroll);
}

fn value_line<'a>(theme: &'a Theme, label: impl Into<String>, value: String, tag: &'a str, tone: StatusTone) -> Line<'a> {
    Line::from(vec![Span::raw(format!("{}: ", label.into())), Span::raw(value), origin_span(theme, tag, tone)])
}

fn error_line<'a>(theme: &'a Theme, label: &'a str, err: GeometryError) -> Line<'a> {
    Line::from(vec![Span::raw(format!("{label}: ")), Span::raw("-- "), origin_span(theme, "INVALID", StatusTone::Danger), Span::raw(format!(" {err}"))])
}

/// Plain-text export - every input field plus the full Secondary/Derived +
/// Fit Analysis (or Calculated + Secondary/Derived) content. Pure and
/// synchronous, no filesystem access, same "the toolbox builds the string,
/// `main.rs` writes it" pattern as `bushing::view::build_report_text` -
/// deliberately a separate, independent builder rather than converting the
/// `Line`-based `regular_readout_lines`/`countersink_readout_lines`
/// content, matching how `bushing`/`pressure_vessel` keep their own
/// plain-text export logic independent of their ratatui rendering.
pub fn build_report_text(model: &FastenerHoleModel) -> String {
    let mut s = String::new();
    s.push_str("Fastener Holes Report\n");
    s.push_str("======================\n\n");
    s.push_str(&format!("Hole type: {}\n", match model.hole_type { HoleType::Regular => "Regular", HoleType::Countersunk => "Countersunk" }));
    s.push_str(&format!(
        "Tolerance input mode: {}\n\n",
        match model.tolerance_mode { super::model::ToleranceInputMode::NominalTol => "Nominal +/- Tolerance", super::model::ToleranceInputMode::MinMax => "Min/Max" }
    ));

    s.push_str("Inputs:\n");
    for row in model::field_rows(model) {
        match row {
            FieldRow::Header(text) => s.push_str(&format!("-- {text} --\n")),
            other => s.push_str(&format!("  {}: {}\n", row_label(other), display_value(model, other))),
        }
    }
    s.push('\n');

    match model.hole_type {
        HoleType::Regular => {
            s.push_str("Secondary Companion Hole:\n");
            match &model.regular_secondary {
                Ok((companion, check)) => {
                    s.push_str(&format!("  Diameter  {} in\n", model::format_linear_band(companion)));
                    s.push_str(&format!("  Envelope preservation: {}\n\n", if check.preserved { "preserved" } else { "NOT preserved" }));
                }
                Err(e) => s.push_str(&format!("  INVALID: {e}\n\n")),
            }
            s.push_str("Fit Analysis (Fit = Hole 2 - Hole 1):\n");
            let fit = &model.regular_fit;
            s.push_str(&format!("  Fit  {} in\n", model::format_band(fit.nominal, fit.min, fit.max, model::format_linear)));
            s.push_str(&format!(
                "  Classification: {}\n",
                match fit.classification {
                    FitClassification::Clearance => "CLEARANCE",
                    FitClassification::Transition => "TRANSITION",
                    FitClassification::Interference => "INTERFERENCE",
                }
            ));
        }
        HoleType::Countersunk => {
            s.push_str("Primary Countersink:\n");
            match &model.countersink_primary {
                Ok(geometry) => {
                    let solved_label = solve_for_label(geometry.solve_for);
                    let solved_value = match geometry.solve_for {
                        CountersinkSolveFor::OuterDiameter => geometry.outer_diameter,
                        CountersinkSolveFor::HoleDiameter => geometry.hole_diameter,
                        CountersinkSolveFor::Depth => geometry.depth,
                        CountersinkSolveFor::Angle => geometry.angle_deg,
                    };
                    let band = if matches!(geometry.solve_for, CountersinkSolveFor::Angle) { model::format_angle_band(&solved_value) } else { model::format_linear_band(&solved_value) };
                    s.push_str(&format!("  {solved_label}: {band}\n"));
                }
                Err(e) => s.push_str(&format!("  INVALID: {e}\n")),
            }
            match &model.countersink_primary_area {
                Ok(area) => s.push_str(&format!("  Lateral Surface Area: {} in^2\n\n", model::format_area_band(area))),
                Err(_) => s.push_str("  Lateral Surface Area: INCOMPLETE\n\n"),
            }

            s.push_str(&format!("Secondary Countersink ({}):\n", model.countersink.secondary_method.label()));
            match &model.countersink_secondary {
                Ok(geometry) => {
                    s.push_str(&format!("  Outer Diameter  {} in\n", model::format_linear_band(&geometry.outer_diameter)));
                    s.push_str(&format!("  Depth           {} in\n", model::format_linear_band(&geometry.depth)));
                    s.push_str(&format!("  Angle           {} deg\n", model::format_angle_band(&geometry.angle_deg)));
                    if let Ok(area) = &model.countersink_secondary_area {
                        s.push_str(&format!("  Lateral Area    {} in^2\n", model::format_area_band(area)));
                    }
                }
                Err(e) => s.push_str(&format!("  INVALID: {e}\n")),
            }
            if let Some(check) = &model.countersink_area_check {
                s.push_str(&format!(
                    "\nArea Preservation Verification: Primary {} in^2, Secondary {} in^2, Delta {} in^2 ({})\n",
                    model::format_area(check.primary_area),
                    model::format_area(check.secondary_area),
                    model::format_area(check.delta),
                    if check.preserved { "preserved" } else { "NOT preserved" }
                ));
            }
        }
    }
    s
}

fn regular_readout_lines<'a>(theme: &'a Theme, model: &'a FastenerHoleModel) -> Vec<Line<'a>> {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled("Secondary Companion Hole", theme.title_style(false))));
    match &model.regular_secondary {
        Ok((companion, check)) => {
            lines.push(value_line(theme, "  Diameter", model::format_linear_band(companion) + " in", "DERIVED", StatusTone::Info));
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
    lines.push(Line::from(format!("  Fit  {} in", model::format_band(fit.nominal, fit.min, fit.max, model::format_linear))));
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
            let unit = if matches!(geometry.solve_for, CountersinkSolveFor::Angle) { " deg" } else { " in" };
            let band = if matches!(geometry.solve_for, CountersinkSolveFor::Angle) { model::format_angle_band(&solved_value) } else { model::format_linear_band(&solved_value) };
            lines.push(value_line(theme, &format!("  {solved_label}"), format!("{band}{unit}"), "CALCULATED", StatusTone::Success));
        }
        Err(e) => lines.push(error_line(theme, "  Geometry", *e)),
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Lateral Surface Area", theme.title_style(false))));
    match &model.countersink_primary_area {
        Ok(area) => {
            lines.push(value_line(theme, "  Primary Area", format!("{} in^2", model::format_area_band(area)), "CALCULATED", StatusTone::Success));
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
            lines.push(value_line(theme, "  Outer Diameter", format!("{} in", model::format_linear_band(&geometry.outer_diameter)), "CALCULATED", StatusTone::Success));
            lines.push(value_line(theme, "  Depth", format!("{} in", model::format_linear_band(&geometry.depth)), depth_tag, depth_tone));
            lines.push(value_line(theme, "  Angle", format!("{} deg", model::format_angle_band(&geometry.angle_deg)), angle_tag, angle_tone));
            match &model.countersink_secondary_area {
                Ok(area) => {
                    let (area_tag, area_tone) = match model.countersink.secondary_method {
                        SecondaryCountersinkMethod::PreserveDepth => ("CALCULATED", StatusTone::Success),
                        SecondaryCountersinkMethod::PreserveLateralArea => ("PRESERVED", StatusTone::Info),
                    };
                    lines.push(value_line(theme, "  Lateral Surface Area", format!("{} in^2", model::format_area_band(area)), area_tag, area_tone));
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
    fn calculated_values_show_their_tolerance_next_to_the_nominal() {
        for hole_type in [HoleType::Regular, HoleType::Countersunk] {
            let mut state = FastenerHoleState::default();
            state.model.hole_type = hole_type;
            state.model.recompute();
            let text = build_report_text(&state.model);
            assert!(text.contains(" -0.") && text.contains("/+0."), "{hole_type:?} report lacks nominal -tol/+tol bands:\n{text}");
            let theme = Theme::default_palette();
            let lines = match hole_type {
                HoleType::Regular => regular_readout_lines(&theme, &state.model),
                HoleType::Countersunk => countersink_readout_lines(&theme, &state.model),
            };
            let flat: String = lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>() + "\n").collect();
            assert!(flat.contains("/+0."), "{hole_type:?} readout lacks tolerance bands:\n{flat}");
            assert!(!flat.contains("range:") && !flat.contains("-Tol"), "old multi-line min/max/tol layout must be gone:\n{flat}");
        }
    }

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

    /// Regression test for the hint-truncation bug (see `bushing/view.rs`'s
    /// twin test): selects the field with the single longest `field_hint()`
    /// string and asserts every word of it appears somewhere in the
    /// rendered buffer, across a range of widths including narrow ones.
    #[test]
    fn the_longest_hint_is_never_truncated_across_a_range_of_widths() {
        let mut everything_on = FastenerHoleState::default();
        everything_on.model.hole_type = HoleType::Countersunk;
        let rows = model::field_rows(&everything_on.model);
        let (longest_index, longest_hint) =
            rows.iter().enumerate().map(|(i, r)| (i, model::field_hint(*r))).max_by_key(|(_, hint)| hint.len()).expect("at least one field row");
        assert!(!longest_hint.is_empty());

        for width in [40u16, 50, 60, 84, 98, 140] {
            let mut state = FastenerHoleState::default();
            state.model.hole_type = HoleType::Countersunk;
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
