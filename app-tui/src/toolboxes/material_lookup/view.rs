//! Rendering for Material Lookup: a search line, the active filters, the
//! result list beside (or above) a property panel, and the side-by-side
//! comparison when materials are marked.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, ListItem, Paragraph};
use ratatui::Frame;

use super::model::{self, compare_rows, format_value};
use super::MaterialLookupState;
use crate::theme::{StatusTone, Theme};
use crate::widgets::empty_state;
use mechanics_core::materials::Material;

/// Width at which the property panel moves beside the list instead of below it.
const WIDE: u16 = 104;
const COL: usize = 7; // width of a numeric list column

pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, state: &MaterialLookupState, focused: bool, regions: &mut crate::mouse::MouseRegions) {
    let title = if state.picking {
        " Choose a material - type to search \u{b7} \u{2190}/\u{2192} group \u{b7} Enter: use this material \u{b7} Esc: cancel "
    } else {
        " Material Lookup - type to search \u{b7} \u{2190}/\u{2192} group \u{b7} Enter mark \u{b7} F2 compare \u{b7} F3 export "
    };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(focused)).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    regions.workspace_panes.push((area, super::PANE_MAIN));

    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)]).split(inner);
    frame.render_widget(Paragraph::new(crate::widgets::material_detail::search_line(theme, &state.query.text, "Ctrl+B basis \u{b7} Ctrl+S sort \u{b7} Ctrl+R reverse", rows[0].width)), rows[0]);
    frame.render_widget(Paragraph::new(filter_line(theme, state)), rows[1]);

    let body = rows[2];
    if body.height == 0 {
        return;
    }
    let (list_area, side_area) = if body.width >= WIDE {
        let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Min(46), Constraint::Length(58)]).split(body);
        (cols[0], cols[1])
    } else {
        let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Percentage(55), Constraint::Percentage(45)]).split(body);
        (parts[0], parts[1])
    };

    draw_list(frame, list_area, theme, state, regions);
    draw_side(frame, side_area, theme, state);
}

fn filter_line<'a>(theme: &Theme, state: &MaterialLookupState) -> Line<'a> {
    let q = &state.query;
    let group = model::groups().get(q.group).cloned().unwrap_or_default();
    let arrow = if q.sort == model::SortKey::Name { if q.descending { "Z-A" } else { "A-Z" } } else if q.descending { "high first" } else { "low first" };
    let mut spans = vec![
        Span::styled("Group: ", theme.disabled_style()),
        Span::raw(group),
        Span::styled("   Basis: ", theme.disabled_style()),
        Span::raw(q.basis.label()),
        Span::styled("   Sort: ", theme.disabled_style()),
        Span::raw(format!("{} ({arrow})", q.sort.label())),
        Span::styled(format!("   {} of {} shown", state.hits.len(), model::catalog().len()), theme.disabled_style()),
    ];
    if !state.marked.is_empty() {
        spans.push(Span::styled(format!("   {} marked", state.marked.len()), theme.status_style(StatusTone::Success)));
    }
    Line::from(spans)
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        format!("{text:<width$}")
    } else {
        let head: String = text.chars().take(width.saturating_sub(1)).collect();
        format!("{head}\u{2026}")
    }
}

fn column(v: f64, decimals: usize) -> String {
    if v > 0.0 {
        format!("{v:>COL$.decimals$}")
    } else {
        format!("{:>COL$}", "-")
    }
}

fn draw_list(frame: &mut Frame, area: Rect, theme: &Theme, state: &MaterialLookupState, regions: &mut crate::mouse::MouseRegions) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(" Materials ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 20 || inner.height == 0 {
        return;
    }
    if state.hits.is_empty() {
        empty_state::render(frame, inner, theme, "No materials match", Some("Esc clears the search; \u{2190}/\u{2192} change the group; Ctrl+B the basis"));
        return;
    }
    let fixed = 2 + 4 + 3 * (COL + 1); // marker + mark tag + three columns
    let name_w = (inner.width as usize).saturating_sub(fixed).max(8);
    let header = Line::from(Span::styled(format!("{:<w$}{:>c$} {:>c$} {:>c$}", "  name", "Ftu", "Fty", "E ksi", w = name_w + 6, c = COL), theme.disabled_style()));
    let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    frame.render_widget(Paragraph::new(header), parts[0]);

    let cat = model::catalog();
    let items: Vec<ListItem> = state
        .hits
        .iter()
        .enumerate()
        .map(|(row, &i)| {
            let m = cat[i];
            let selected = row == state.cursor;
            let marker = if selected { "> " } else { "  " };
            let tag = match state.marked.iter().position(|&x| x == i) {
                Some(p) => format!("[{}] ", p + 1),
                None => "    ".to_string(),
            };
            let style = if selected { theme.selected_row_style() } else if state.marked.contains(&i) { theme.status_style(StatusTone::Success) } else { Style::default() };
            ListItem::new(Line::from(Span::styled(format!("{marker}{tag}{} {} {} {}", clip(m.name, name_w), column(m.ftu_ksi, 1), column(m.sy_ksi, 1), column(m.e_ksi, 0)), style)))
        })
        .collect();
    let offset = crate::widgets::scroll_list::render(frame, parts[1], items, Some(state.cursor));
    regions.material_lookup_rows.extend(crate::mouse::list_row_regions(parts[1], offset, state.hits.len()));
}

fn draw_side(frame: &mut Frame, area: Rect, theme: &Theme, state: &MaterialLookupState) {
    let comparing = state.compare && !state.marked.is_empty();
    let title = if comparing { " Comparison - best of each row marked * " } else { " Properties " };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(false)).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let cat = model::catalog();
    let lines = if comparing {
        let ms: Vec<&Material> = state.marked.iter().map(|&i| cat[i]).collect();
        compare_lines(theme, &ms, inner.width)
    } else if let Some(i) = state.current() {
        property_lines(theme, cat[i])
    } else {
        Vec::new()
    };
    crate::widgets::scroll_paragraph::render(frame, inner, theme, lines, state.detail_scroll);
}

/// The handbook record (or the curated solver fields) plus the derived
/// strength-to-weight figures the handbook table does not print.
pub fn property_lines<'a>(theme: &Theme, m: &Material) -> Vec<Line<'a>> {
    let mut out = vec![Line::from(Span::styled(m.name.to_string(), theme.title_style(false).add_modifier(Modifier::BOLD))), Line::from("")];
    out.extend(crate::widgets::material_detail::lines(theme, m));
    out.push(Line::from(""));
    let sy = model::specific_yield_in(m).map(|v| format!("{v:.0} in"));
    let se = model::specific_modulus_in(m).map(|v| format!("{v:.0} in"));
    match (sy, se) {
        (Some(sy), Some(se)) => out.push(Line::from(format!("Fty / density {sy}   E / density {se}"))),
        _ => out.push(Line::from(Span::styled("No density in the table: curated typical values carry none, so the weight-specific figures are not shown.", theme.disabled_style()))),
    }
    out.push(Line::from(Span::styled(format!("Thermal expansion {:.1} microstrain/F; Poisson ratio {:.2}", m.alpha_u_f, m.nu), theme.disabled_style())));
    out
}

pub fn compare_lines<'a>(theme: &Theme, ms: &[&Material], width: u16) -> Vec<Line<'a>> {
    let label_w = 19usize;
    let unit_w = 9usize;
    let n = ms.len().max(1);
    let col_w = ((width as usize).saturating_sub(label_w + unit_w) / n).clamp(8, 18);
    let mut out = Vec::new();
    for (i, m) in ms.iter().enumerate() {
        out.push(Line::from(vec![Span::styled(format!("[{}] ", i + 1), theme.status_style(StatusTone::Success)), Span::raw(m.name.to_string())]));
    }
    out.push(Line::from(""));
    // Starts with text, not blanks: the paragraph trims leading spaces per line.
    let mut head = format!("{:<label_w$}{:<unit_w$}", "Property", "unit");
    for i in 0..ms.len() {
        head.push_str(&format!("{:>col_w$}", format!("[{}]", i + 1)));
    }
    out.push(Line::from(Span::styled(head, theme.disabled_style())));
    for r in compare_rows(ms) {
        let mut spans = vec![Span::raw(format!("{:<label_w$}", r.label)), Span::styled(format!("{:<unit_w$}", r.unit), theme.disabled_style())];
        for (i, v) in r.values.iter().enumerate() {
            let best = r.best.contains(&i);
            let text = format!("{}{}", format_value(*v, r.decimals), if best { "*" } else { " " });
            let style = if best { theme.status_style(StatusTone::Success) } else { Style::default() };
            spans.push(Span::styled(format!("{text:>col_w$}"), style));
        }
        out.push(Line::from(spans));
    }
    out.push(Line::from(""));
    out.push(Line::from(Span::styled("Strengths use the lowest grain direction (L / LT / ST); * marks the largest value of a row where larger is better.", theme.disabled_style())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render(state: &MaterialLookupState, w: u16, h: u16) -> (String, crate::mouse::MouseRegions) {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut regions = crate::mouse::MouseRegions::default();
        terminal.draw(|f| draw(f, Rect::new(0, 0, w, h), &Theme::default_palette(), state, true, &mut regions)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text = (0..buf.area.height).map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n").collect();
        (text, regions)
    }

    #[test]
    fn wide_layout_shows_the_list_and_the_property_panel() {
        let state = MaterialLookupState::default();
        let (text, regions) = render(&state, 150, 40);
        assert!(text.contains("Materials") && text.contains("Properties") && text.contains("Ftu"), "{text}");
        let first: String = model::catalog()[state.hits[0]].name.chars().take(10).collect();
        assert!(text.contains(&first), "the first listed material ({first}) is shown:\n{text}");
        assert!(!regions.material_lookup_rows.is_empty());
    }

    #[test]
    fn picking_mode_says_enter_chooses() {
        let state = MaterialLookupState { picking: true, ..Default::default() };
        let (text, _) = render(&state, 150, 30);
        assert!(text.contains("Enter: use this material") && !text.contains("Enter mark"), "{text}");
    }

    #[test]
    fn narrow_layout_stacks_the_panels_and_still_renders() {
        let state = MaterialLookupState::default();
        let (text, _) = render(&state, 70, 36);
        assert!(text.contains("Materials") && text.contains("Properties"), "{text}");
    }

    #[test]
    fn an_empty_result_explains_itself() {
        let mut state = MaterialLookupState::default();
        state.query.text = "zzzz".into();
        state.hits = model::filter_sort(&state.query);
        let (text, regions) = render(&state, 120, 30);
        assert!(text.contains("No materials match"), "{text}");
        assert!(regions.material_lookup_rows.is_empty());
    }

    #[test]
    fn marking_shows_a_tag_and_the_comparison_lists_every_row() {
        let mut state = MaterialLookupState::default();
        state.toggle_mark();
        state.cursor = 3;
        state.toggle_mark();
        state.compare = true;
        let (text, _) = render(&state, 150, 40);
        assert!(text.contains("Comparison") && text.contains("[1]") && text.contains("[2]") && text.contains("Fty / density"), "{text}");
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        let mut state = MaterialLookupState::default();
        state.toggle_mark();
        state.compare = true;
        for (w, h) in [(1, 1), (10, 4), (30, 6), (200, 3), (60, 60)] {
            render(&state, w, h);
        }
    }

    #[test]
    fn every_handbook_material_renders_property_lines() {
        let theme = Theme::default_palette();
        for m in model::catalog().iter().step_by(37) {
            assert!(property_lines(&theme, m).len() >= 4, "{}", m.name);
        }
    }
}
