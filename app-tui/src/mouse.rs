//! Mouse hit-testing: turns rendered geometry into clickable regions.
//!
//! `MouseRegions` is rebuilt from scratch every frame in `widgets::shell::draw`
//! (via `clear()`), then handed (read-only) to `app::handle_mouse` for the
//! next `Event::Mouse` - it is deliberately NOT part of `AppState` (it's
//! render-derived scratch, not meaningful app state, and would need to be
//! `Default`-reset on every draw regardless). See the "Mouse navigation"
//! plan for the full architecture.

use ratatui::layout::Rect;

use crate::nav::ToolId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickTarget {
    Rail(ToolId),
    WorkspacePane(u8),
    ResultRow(usize),
    RecentRow(usize),
    PresetRow(usize),
    SettingsFieldRow(usize),
    FastenerRow(usize),
    PaletteRow(usize),
    ExtensionRow(usize),
    ConfirmYes,
    ConfirmNo,
    HelpOverlay,
}

#[derive(Debug, Clone, Default)]
pub struct MouseRegions {
    pub rail: Vec<(Rect, ToolId)>,
    /// Fallback focus-only target for a pane's empty space (a click that
    /// didn't land on any more specific row region below) - `u8` is the
    /// same `FocusArea::Workspace(u8)` pane index every toolbox already uses.
    pub workspace_panes: Vec<(Rect, u8)>,
    pub results_rows: Vec<(Rect, usize)>,
    pub recents_rows: Vec<(Rect, usize)>,
    pub presets_rows: Vec<(Rect, usize)>,
    pub settings_field_rows: Vec<(Rect, usize)>,
    /// Clicking anywhere in a Settings section's pane (not on a specific
    /// row) switches `SettingsView::section` to it - distinct from
    /// `workspace_panes` since Settings has three sub-panes sharing one
    /// workspace pane index.
    pub settings_section_panes: Vec<(Rect, crate::toolboxes::search::settings_view::Section)>,
    pub fastener_rows: Vec<(Rect, usize)>,
    pub palette_rows: Vec<(Rect, usize)>,
    pub extension_rows: Vec<(Rect, usize)>,
    pub confirm_yes: Option<Rect>,
    pub confirm_no: Option<Rect>,
    /// Whole overlay area - any click anywhere while it's open just closes
    /// it (no interactive elements inside), matching Esc/`?`.
    pub help_overlay: Option<Rect>,
}

impl MouseRegions {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// `Rect` contains-point check shared by every hit-test below.
pub fn contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

/// First region containing `(col, row)`, or `None` if the click landed
/// outside every known region - always a safe no-op, never a panic.
pub fn hit<T: Copy>(regions: &[(Rect, T)], col: u16, row: u16) -> Option<T> {
    regions.iter().find(|(rect, _)| contains(*rect, col, row)).map(|(_, t)| *t)
}

/// Turns a scrolled list's real, already-computed scroll offset (from
/// `widgets::scroll_list::render`'s return value) into one `Rect` per
/// currently-visible row, each one row tall - the single place this math
/// happens, reused by every list in the crate instead of each call site
/// recomputing (and potentially drifting from) the same offset logic.
pub fn list_row_regions(area: Rect, offset: usize, len: usize) -> Vec<(Rect, usize)> {
    if area.height == 0 || area.width == 0 || len == 0 {
        return Vec::new();
    }
    let visible = area.height as usize;
    (offset..len.min(offset + visible))
        .map(|i| {
            let y = area.y + (i - offset) as u16;
            (Rect { x: area.x, y, width: area.width, height: 1 }, i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_finds_the_containing_region() {
        let regions = vec![(Rect::new(0, 0, 10, 1), "a"), (Rect::new(0, 1, 10, 1), "b")];
        assert_eq!(hit(&regions, 5, 0), Some("a"));
        assert_eq!(hit(&regions, 5, 1), Some("b"));
    }

    #[test]
    fn hit_outside_every_region_is_none_not_a_panic() {
        let regions = vec![(Rect::new(0, 0, 10, 1), "a")];
        assert_eq!(hit(&regions, 50, 50), None);
        assert_eq!(hit::<&str>(&[], 0, 0), None);
    }

    #[test]
    fn hit_is_edge_exclusive_on_the_far_side() {
        // A 10-wide/1-tall rect at (0,0) covers columns 0..=9 - column 10 is
        // one past the end and must not match.
        let regions = vec![(Rect::new(0, 0, 10, 1), "a")];
        assert_eq!(hit(&regions, 9, 0), Some("a"));
        assert_eq!(hit(&regions, 10, 0), None);
    }

    #[test]
    fn list_row_regions_covers_only_the_visible_window() {
        let area = Rect::new(2, 3, 20, 4); // 4 visible rows
        let regions = list_row_regions(area, 5, 50);
        assert_eq!(regions.len(), 4);
        assert_eq!(regions[0], (Rect::new(2, 3, 20, 1), 5));
        assert_eq!(regions[3], (Rect::new(2, 6, 20, 1), 8));
    }

    #[test]
    fn list_row_regions_stops_at_the_real_item_count() {
        // Fewer items than the viewport is tall - no phantom rows past len.
        let area = Rect::new(0, 0, 20, 10);
        let regions = list_row_regions(area, 0, 3);
        assert_eq!(regions.len(), 3);
    }

    #[test]
    fn list_row_regions_on_degenerate_area_is_empty_not_a_panic() {
        assert!(list_row_regions(Rect::new(0, 0, 0, 0), 0, 10).is_empty());
        assert!(list_row_regions(Rect::new(0, 0, 10, 0), 0, 10).is_empty());
        assert!(list_row_regions(Rect::new(0, 0, 0, 10), 0, 10).is_empty());
        assert!(list_row_regions(Rect::new(0, 0, 10, 10), 0, 0).is_empty());
    }
}
