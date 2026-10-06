//! Sectioned aerospace/military fastener picker popup - AN / NAS (UNJF) /
//! NAS (UNF) / MS21250 / Hi-Lok, same purpose as
//! `toolboxes/bushing/reamer_picker.rs`: populate every thread-geometry
//! field (and the shank diameter) from a real standard size in one step,
//! rather than requiring an engineer to hand-derive pitch/root diameter
//! from a thread callout. Every populated field remains an ordinary
//! editable `Number` row afterward - `PreloadModel::matching_bolt` reports
//! `None` ("Custom") the moment any of them is hand-edited away from the
//! selected entry, so there's no hidden "locked to catalog" state to fight.
//!
//! Sections are non-selectable dividers - same `Header`-skip navigation
//! technique `toolboxes/bushing/model.rs`/`toolboxes/preload_analysis/model.rs`
//! already use for their own field lists, reused here rather than
//! reinvented.

use crate::widgets::popup::centered_rect;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use fastened_joint_solver::thread_catalog::{BoltCatalogEntry, AN_BOLT_CATALOG, HI_LOK_CATALOG, MS_BOLT_CATALOG, NAS_UNF_BOLT_CATALOG, NAS_UNJF_BOLT_CATALOG};

use crate::theme::Theme;

use super::model::PreloadModel;
use crate::app::Effect;

#[derive(Debug, Clone, Copy, PartialEq)]
enum PickerRow {
    Section(&'static str),
    Entry(&'static BoltCatalogEntry),
}

/// The picker's own name for the catalog an entry came from - only Hi-Lok
/// needs a distinct footer note (no sourced preload/torque-off value), so
/// this is the one place that association is needed.
fn is_hi_lok(entry: &BoltCatalogEntry) -> bool {
    HI_LOK_CATALOG.iter().any(|e| std::ptr::eq(e, entry))
}

fn rows() -> Vec<PickerRow> {
    let mut rows = Vec::new();
    for (label, catalog) in [
        ("AN3-AN20 (Aerospace Standard)", AN_BOLT_CATALOG),
        ("NAS Tension/Shear Bolts (UNJF)", NAS_UNJF_BOLT_CATALOG),
        ("NAS1003-1020 Machine Bolts (UNF)", NAS_UNF_BOLT_CATALOG),
        ("MS21250 (UNJF)", MS_BOLT_CATALOG),
        ("Hi-Lok HL18 Pins (UNJ)", HI_LOK_CATALOG),
    ] {
        rows.push(PickerRow::Section(label));
        rows.extend(catalog.iter().map(PickerRow::Entry));
    }
    rows
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BoltPickerState {
    pub open: bool,
    pub cursor: usize,
}

impl BoltPickerState {
    /// Opens with the cursor on the currently-matching catalog entry (if
    /// any), so re-opening the picker on an already-standard bolt doesn't
    /// reset the selection back to the top.
    pub fn open_for(model: &PreloadModel) -> Self {
        let rows = rows();
        let cursor = model
            .matching_bolt()
            .and_then(|m| rows.iter().position(|r| matches!(r, PickerRow::Entry(e) if e.designation == m.designation)))
            .unwrap_or_else(|| rows.iter().position(|r| matches!(r, PickerRow::Entry(_))).unwrap_or(0));
        Self { open: true, cursor }
    }

    fn move_cursor(&mut self, delta: i32) {
        let rows = rows();
        if rows.is_empty() {
            self.cursor = 0;
            return;
        }
        let len = rows.len() as i32;
        let mut next = self.cursor as i32;
        for _ in 0..rows.len() {
            next = (next + delta).rem_euclid(len);
            if matches!(rows[next as usize], PickerRow::Entry(_)) {
                break;
            }
        }
        self.cursor = next as usize;
    }
}

pub fn handle_key(picker: &mut BoltPickerState, model: &mut PreloadModel, key: KeyEvent) -> (bool, Vec<Effect>) {
    match key.code {
        KeyCode::Up => {
            picker.move_cursor(-1);
            (true, Vec::new())
        }
        KeyCode::Down => {
            picker.move_cursor(1);
            (true, Vec::new())
        }
        KeyCode::Enter => {
            if let Some(PickerRow::Entry(entry)) = rows().get(picker.cursor) {
                model.select_bolt(entry);
            }
            picker.open = false;
            (true, Vec::new())
        }
        KeyCode::Esc => {
            picker.open = false;
            (true, Vec::new())
        }
        _ => (false, Vec::new()),
    }
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, picker: &BoltPickerState, regions: &mut crate::mouse::MouseRegions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let popup = centered_rect(60, 70, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style(true))
        .title(" Fastener Catalog - Esc: keep custom ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = rows();
    let selected_is_hi_lok = matches!(rows.get(picker.cursor), Some(PickerRow::Entry(e)) if is_hi_lok(e));
    let (list_area, note_area) = if selected_is_hi_lok && inner.height > 4 {
        let split = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(1), Constraint::Length(3)]).split(inner);
        (split[0], Some(split[1]))
    } else {
        (inner, None)
    };

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| match row {
            PickerRow::Section(label) => ListItem::new(Line::from(Span::styled(format!("-- {label} --"), theme.title_style(false).add_modifier(Modifier::BOLD)))),
            PickerRow::Entry(entry) => {
                let selected = i == picker.cursor;
                let marker = if selected { "> " } else { "  " };
                let style = if selected { theme.selected_row_style() } else { Style::default() };
                ListItem::new(Line::from(Span::styled(format!("{marker}{}", entry.designation), style)))
            }
        })
        .collect();
    let offset = crate::widgets::scroll_list::render(frame, list_area, items, Some(picker.cursor));
    regions.bolt_rows.extend(crate::mouse::list_row_regions(list_area, offset, rows.len()));

    if let Some(note_area) = note_area {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Thread geometry only - Hi-Lok preload is set by the collar's designed torque-off value, not typed in here. Set Target Preload from the applicable SRM/spec.",
                theme.disabled_style(),
            )))
            .wrap(Wrap { trim: true }),
            note_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn open_for_positions_the_cursor_on_the_current_matching_bolt() {
        let model = PreloadModel::default(); // default is AN6
        let picker = BoltPickerState::open_for(&model);
        let rows = rows();
        match rows[picker.cursor] {
            PickerRow::Entry(e) => assert_eq!(e.designation, model.matching_bolt().unwrap().designation),
            PickerRow::Section(_) => panic!("cursor must land on an Entry row"),
        }
    }

    #[test]
    fn open_for_lands_off_the_first_section_header_when_nothing_matches() {
        let mut model = PreloadModel::default();
        model.commit_number(super::super::model::NumberTarget::ThreadMajorDia, 3.0); // matches nothing
        let picker = BoltPickerState::open_for(&model);
        assert!(matches!(rows()[picker.cursor], PickerRow::Entry(_)));
    }

    #[test]
    fn enter_selects_the_highlighted_bolt_and_closes() {
        let mut model = PreloadModel::default();
        let rows = rows();
        let first_entry_index = rows.iter().position(|r| matches!(r, PickerRow::Entry(_))).unwrap();
        let mut picker = BoltPickerState { open: true, cursor: first_entry_index };
        handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        assert!(!picker.open);
        let PickerRow::Entry(expected) = rows[first_entry_index] else { unreachable!() };
        assert_eq!(model.matching_bolt().unwrap().designation, expected.designation);
    }

    #[test]
    fn esc_closes_without_changing_the_model() {
        let mut model = PreloadModel::default();
        let before = model.thread_major_dia;
        let mut picker = BoltPickerState { open: true, cursor: 5 };
        handle_key(&mut picker, &mut model, key(KeyCode::Esc));
        assert!(!picker.open);
        assert_eq!(model.thread_major_dia, before);
    }

    #[test]
    fn up_down_navigation_skips_section_headers() {
        let rows = rows();
        let first_entry_index = rows.iter().position(|r| matches!(r, PickerRow::Entry(_))).unwrap();
        let mut model = PreloadModel::default();
        let mut picker = BoltPickerState { open: true, cursor: first_entry_index };
        handle_key(&mut picker, &mut model, key(KeyCode::Up));
        assert!(matches!(rows[picker.cursor], PickerRow::Entry(_)), "must not land on a section header");
        // Wrapping backward from the first entry must land on the last
        // entry (the very last row is always an entry, never a section).
        assert!(matches!(rows.last().unwrap(), PickerRow::Entry(_)));
        assert_eq!(picker.cursor, rows.len() - 1);
        handle_key(&mut picker, &mut model, key(KeyCode::Down));
        assert_eq!(picker.cursor, first_entry_index);
    }

    #[test]
    fn selecting_a_hi_lok_entry_fills_geometry_but_never_writes_an_applied_torque_or_preload() {
        let mut model = PreloadModel::default();
        let before_torque = model.applied_torque;
        let before_preload = model.target_preload;
        let rows = rows();
        let hi_lok_index = rows.iter().position(|r| matches!(r, PickerRow::Entry(e) if is_hi_lok(e))).unwrap();
        let mut picker = BoltPickerState { open: true, cursor: hi_lok_index };
        handle_key(&mut picker, &mut model, key(KeyCode::Enter));
        let PickerRow::Entry(expected) = rows[hi_lok_index] else { unreachable!() };
        assert_eq!(model.matching_bolt().unwrap().designation, expected.designation);
        assert_eq!(model.applied_torque, before_torque);
        assert_eq!(model.target_preload, before_preload);
    }

    #[test]
    fn render_does_not_panic_at_normal_or_degenerate_sizes() {
        let rows = rows();
        let picker = BoltPickerState { open: true, cursor: rows.iter().position(|r| matches!(r, PickerRow::Entry(_))).unwrap() };
        for (w, h) in [(100, 40), (0, 0), (1, 1)] {
            let backend = TestBackend::new(w.max(1), h.max(1));
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| render(f, Rect { x: 0, y: 0, width: w, height: h }, &Theme::default_palette(), &picker, &mut regions)).unwrap();
        }
    }

    #[test]
    fn render_does_not_panic_with_a_hi_lok_entry_selected_showing_the_note() {
        let rows = rows();
        let hi_lok_index = rows.iter().position(|r| matches!(r, PickerRow::Entry(e) if is_hi_lok(e))).unwrap();
        let picker = BoltPickerState { open: true, cursor: hi_lok_index };
        for width in [40u16, 60, 100, 140] {
            let backend = TestBackend::new(width, 40);
            let mut terminal = Terminal::new(backend).unwrap();
            let mut regions = crate::mouse::MouseRegions::default();
            terminal.draw(|f| render(f, Rect { x: 0, y: 0, width, height: 40 }, &Theme::default_palette(), &picker, &mut regions)).unwrap();
        }
    }
}
