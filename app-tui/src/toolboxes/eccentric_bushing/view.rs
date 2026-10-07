//! Rendering for the Eccentric Bushing toolbox: the field list beside (or above) the readout of the spin check.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::model::{self, FieldRow, Task};
use super::EccentricState;
use crate::theme::{StatusTone, Theme};
use crate::toolboxes::bushing::model::BushingModel;

const MIN_READOUT_WIDTH: u16 = 44;
const FIELDS_WIDTH: u16 = 46;

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, state: &EccentricState, bushing: &BushingModel, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(focused))
        .title(" Eccentric Bushing - r: analyse \u{b7} m: max offset \u{b7} l: max load \u{b7} d: profile \u{b7} e: export ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let (fields_area, readout_area) = if inner.width >= FIELDS_WIDTH + MIN_READOUT_WIDTH {
        let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Length(FIELDS_WIDTH), Constraint::Min(MIN_READOUT_WIDTH)]).split(inner);
        (cols[0], cols[1])
    } else {
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(45), Constraint::Percentage(55)]).split(inner);
        (rows[0], rows[1])
    };
    regions.workspace_panes.push((area, super::PANE_MAIN));
    draw_fields(frame, fields_area, theme, state, focused, regions);
    draw_readout(frame, readout_area, theme, state, bushing);
}

fn display_value(state: &EccentricState, row: FieldRow) -> String {
    match row {
        FieldRow::Header(_) => String::new(),
        FieldRow::Number(t) => model::format_value(t, state.ui.number_value(t)),
        FieldRow::TogglePlane => if state.ui.plane_strain { "Constrained (plane strain)" } else { "Free ends (plane stress)" }.to_string(),
        FieldRow::ToggleHousing => if state.ui.edge_limited { "Edge-limited plate" } else { "Round boss" }.to_string(),
        FieldRow::ToggleDirectOnset => if state.ui.direct_onset { "On (slower)" } else { "Off (integral only)" }.to_string(),
        FieldRow::TogglePinCredit => if state.ui.credit_pin_load { "With pin load (realistic)" } else { "Fit alone (conservative)" }.to_string(),
        FieldRow::Run(task) => match &state.job {
            Some(j) if j.task == task => format!("\u{2026} running ({:.0} s)", j.started.elapsed().as_secs_f64()),
            _ => "Enter".to_string(),
        },
    }
}

fn draw_fields(frame: &mut Frame, area: Rect, theme: &Theme, state: &EccentricState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(focused)).title(" Inputs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows = model::field_rows();
    let hint = rows.get(state.selected).map(|r| model::field_hint(*r)).unwrap_or("");
    let hint_height = crate::widgets::hint_panel::hint_panel_height(hint, inner.width, inner.height.saturating_sub(4).max(1));
    let (list_area, hint_area) = if inner.height > hint_height + 1 {
        let split = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(hint_height)]).split(inner);
        (split[0], Some(split[1]))
    } else {
        (inner, None)
    };
    let label_width = rows.iter().filter(|r| !matches!(r, FieldRow::Header(_))).map(|r| model::row_label(*r).len()).max().unwrap_or(0) + 2;
    let mut heights = Vec::with_capacity(rows.len());
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            if let FieldRow::Header(text) = row {
                heights.push(1);
                return ListItem::new(Line::from(Span::styled(format!("-- {text} --"), theme.title_style(false).add_modifier(Modifier::BOLD))));
            }
            let selected = focused && i == state.selected;
            let value = if selected && state.editing { state.edit_buffer.with_cursor() } else { display_value(state, *row) };
            let style = if selected { theme.selected_row_style() } else { Style::default() };
            let (item, h) = crate::widgets::scroll_list::field_item(if selected { "> " } else { "  " }, model::row_label(*row), label_width, &value, list_area.width, style, None);
            heights.push(h);
            item
        })
        .collect();
    let offset = crate::widgets::scroll_list::render(frame, list_area, items, focused.then_some(state.selected));
    regions.eccentric_rows.extend(crate::mouse::list_row_regions_var(list_area, offset, &heights));
    if let Some(hint_area) = hint_area {
        frame.render_widget(Paragraph::new(Line::from(Span::styled(hint, theme.disabled_style()))).wrap(Wrap { trim: true }), hint_area);
    }
}

fn draw_readout(frame: &mut Frame, area: Rect, theme: &Theme, state: &EccentricState, bushing: &BushingModel) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(" Results ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    crate::widgets::scroll_paragraph::render(frame, inner, theme, readout_lines(theme, state, bushing), state.results_scroll);
}

fn bold<'a>(theme: &Theme, text: &str) -> Line<'a> {
    Line::from(Span::styled(text.to_string(), theme.title_style(false).add_modifier(Modifier::BOLD)))
}

pub fn readout_lines<'a>(theme: &Theme, state: &EccentricState, bushing: &BushingModel) -> Vec<Line<'a>> {
    let mut lines: Vec<Line<'a>> = Vec::new();
    if let Some(job) = &state.job {
        let what = match job.task {
            Task::Analyze => "analysing",
            Task::MaxOffset => "searching the maximum offset",
            Task::MaxLoad => "searching the maximum load",
        };
        lines.push(Line::from(Span::styled(format!("\u{2026} {what} ({:.0} s)", job.started.elapsed().as_secs_f64()), theme.status_style(StatusTone::Info))));
    }
    if let Some(err) = &state.error {
        lines.push(Line::from(Span::styled(format!("\u{2717} {err}"), theme.status_style(StatusTone::Danger))));
    }
    let input = model::build_input(bushing, &state.ui);
    lines.push(bold(theme, "From the Bushing Workbench"));
    match &input {
        Ok(i) => {
            lines.push(Line::from(format!("  Bore {:.4} in  ID {:.4} in  interference {:.4} in (dia)  length {:.3} in", i.bore_dia, i.bushing_id, i.interference_dia, i.thickness)));
            lines.push(Line::from(format!("  Friction {:.2}  pin load {:.0} lbf  {}  thin wall {:.4} in", i.friction, i.load_lbf, i.edge_distance.map_or(format!("boss OD {:.3} in", i.housing_od), |e| format!("plate, edge {e:.3} in")), i.walls().0)));
        }
        Err(why) => lines.push(Line::from(Span::styled(format!("  \u{26a0} {why}"), theme.status_style(StatusTone::Warning)))),
    }
    let fresh = |sig: &eccentric_bushing::Inputs| input.as_ref().is_ok_and(|i| i == sig);

    if let Some((sig, a)) = &state.analysis {
        lines.push(Line::from(""));
        lines.push(bold(theme, &format!("Spin check at e = {:.4} in", a.offset)));
        if !fresh(sig) {
            lines.push(Line::from(Span::styled("  (inputs changed since this run: r runs it again)", theme.disabled_style())));
        }
        let (glyph, tone) = if !a.margin.is_finite() {
            ("\u{b7}", StatusTone::Neutral)
        } else if a.margin >= 0.0 {
            ("\u{2713}", StatusTone::Success)
        } else {
            ("\u{2717}", StatusTone::Danger)
        };
        let verdict = if a.margin.is_finite() { format!("margin {:+.1} %  {}", a.margin * 100.0, if a.margin >= 0.0 { "HOLDS" } else { "SPINS" }) } else { "no spin torque at this load angle".to_string() };
        lines.push(Line::from(Span::styled(format!("  {glyph} {verdict}"), theme.status_style(tone))));
        lines.push(Line::from(format!("  Torque required F e sin(a)  {:>9.2} lbf in", a.torque_required)));
        lines.push(Line::from(format!("  Capacity, fit alone         {:>9.2} lbf in", a.torque_capacity_fit)));
        lines.push(Line::from(format!("  Capacity, with pin load     {:>9.2} lbf in", a.torque_capacity)));
        lines.push(Line::from(format!("  Design capacity ({})  {:>9.2} lbf in", if input.as_ref().is_ok_and(|i| i.credit_pin_load) { "pin loaded" } else { "fit alone " }, a.design_capacity)));
        if let Some(onset) = a.onset_torque {
            lines.push(Line::from(format!("  Direct spin simulation: onset {:.2} lbf in ({:+.1} % against the fit integral)", onset, 100.0 * (onset / a.torque_capacity_fit - 1.0))));
        }
        if a.pin_peak_pressure > 0.0 {
            lines.push(Line::from(format!("  Pin on the bore: {:.0} deg of arc, peak {:.0} psi", a.pin_arc_deg, a.pin_peak_pressure)));
        }
        lines.push(Line::from(format!("  Wall {:.4} in thin / {:.4} in thick", a.wall_thin, a.wall_thick)));
        lines.push(Line::from(format!("  Fit pressure {:.0} - {:.0} psi (thick side highest)", a.fit_pressure_min, a.fit_pressure_max)));
        if !a.wall_ok {
            lines.push(Line::from(Span::styled(
                format!("  \u{2717} thin wall {:.4} in is below the Bushing Workbench's minimum wall {:.4} in", a.wall_thin, input.as_ref().map_or(0.0, |i| i.min_wall)),
                theme.status_style(StatusTone::Danger),
            )));
        }
        if a.contact_lost_deg > 0.0 {
            lines.push(Line::from(Span::styled(format!("  \u{26a0} contact lost over {:.0} deg of the interface under the pin load", a.contact_lost_deg), theme.status_style(StatusTone::Warning))));
        }
        lines.push(Line::from(Span::styled(
            format!("  FE check: interface force ({:.0}, {:.0}) lbf, friction moment {:.2} lbf in", a.net_force[0], a.net_force[1], a.friction_torque),
            theme.disabled_style(),
        )));
        if state.show_profile {
            lines.push(Line::from(""));
            lines.extend(profile_lines(theme, a));
        }
    }
    if let Some((sig, l)) = &state.max_offset {
        lines.push(Line::from(""));
        lines.push(bold(theme, "Maximum offset that holds"));
        let stale = if fresh(sig) { "" } else { "  (inputs changed: m runs it again)" };
        let note = if l.bounded_by_wall { "  (spin never limits it: the numerical wall floor does)" } else if l.set_by_loaded_run { "  (set by contact loss under load)" } else { "" };
        lines.push(Line::from(format!("  spin allows e up to {:.4} in{note}{stale}", l.value)));
        let (glyph, tone) = if l.wall_limit < l.value { ("\u{26a0}", StatusTone::Warning) } else { ("\u{b7}", StatusTone::Neutral) };
        lines.push(Line::from(Span::styled(
            format!("  {glyph} the minimum wall allows e up to {:.4} in{}", l.wall_limit, if l.wall_limit < l.value { " - the wall governs, not spin" } else { "" }),
            theme.status_style(tone),
        )));
    }
    if let Some((sig, l)) = &state.max_load {
        lines.push(Line::from(""));
        lines.push(bold(theme, "Maximum pin load that is held"));
        let stale = if fresh(sig) { "" } else { "  (inputs changed: l runs it again)" };
        lines.push(Line::from(if l.value.is_finite() { format!("  F max = {:.0} lbf{stale}", l.value) } else { format!("  no spin torque at this angle: any load is held{stale}") }));
    }
    if state.analysis.is_none() && state.max_offset.is_none() && state.max_load.is_none() && state.job.is_none() && state.error.is_none() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Press r to analyse this offset, m for the maximum offset, l for the maximum load.", theme.disabled_style())));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Plane contact FE (fea-core); friction capacity only (lock plates and teeth are not credited); a design guide, not a certification value.",
        theme.disabled_style(),
    )));
    lines
}

/// The interface pressure every 15 degrees (the bore is offset towards 0 deg), as bars.
fn profile_lines<'a>(theme: &Theme, a: &eccentric_bushing::Analysis) -> Vec<Line<'a>> {
    let mut out = vec![bold(theme, "Interface pressure (psi): fit alone | with the pin load"), Line::from(Span::styled("  angle from the offset direction; wall thinnest at 0, thickest at 180", theme.disabled_style()))];
    let peak = a.profile.iter().map(|b| b.fit.max(b.loaded)).fold(1.0, f64::max);
    let per = a.profile.len() / 24;
    for k in 0..24 {
        let bins = &a.profile[k * per..(k + 1) * per];
        let mean = |f: fn(&eccentric_bushing::ProfileBin) -> f64| bins.iter().map(f).sum::<f64>() / bins.len() as f64;
        let (fit, loaded) = (mean(|b| b.fit), mean(|b| b.loaded));
        let bar = "\u{2588}".repeat(((loaded / peak) * 20.0).round() as usize);
        out.push(Line::from(format!("  {:>3}\u{b0} {:>7.0} | {:>7.0}  {bar}", k * 15, fit, loaded)));
    }
    out
}
