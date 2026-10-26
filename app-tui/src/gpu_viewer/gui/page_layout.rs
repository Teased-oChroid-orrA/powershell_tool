//! Where a page's two cards (the results list and the selected file's detail) go in the room under the bar: side by
//! side or stacked, either one first, split where the user left the splitter. Pure geometry, so the drawing code in
//! `document.rs` and its tests agree on it.
use crate::gpu_viewer::display::PageLayout;
use egui::{Pos2, Rect};

/// Space between the cards; the splitter lives in it.
pub(super) const GAP: f32 = 12.0;
/// Least width of a card side by side: a narrower room stacks them whatever the layout says.
pub(super) const MIN_SIDE: f32 = 380.0;
/// Least height of a card when stacked: a shorter room shows the list alone.
pub(super) const MIN_STACKED: f32 = 190.0;

/// The rectangles of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Split {
    pub list: Rect,
    pub detail: Option<Rect>,
    /// The gap between the cards, where the splitter is grabbed.
    pub splitter: Option<Rect>,
    /// The cards are one above the other this frame (asked for, or the room is too narrow).
    pub stacked: bool,
}

/// Lay out `rest` for `layout`; `has_detail` false gives the list the whole room.
pub(super) fn split(rest: Rect, layout: PageLayout, has_detail: bool) -> Split {
    let stacked = layout.stacked || rest.width() < 2.0 * MIN_SIDE + GAP;
    let alone = Split { list: rest, detail: None, splitter: None, stacked };
    if !has_detail {
        return alone;
    }
    let (size, least) = if stacked { (rest.height(), MIN_STACKED) } else { (rest.width(), MIN_SIDE) };
    if size < 2.0 * least + GAP {
        return alone;
    }
    let room = size - GAP;
    let first = (room * layout.split).clamp(least, room - least);
    let (a, gap, b) = if stacked {
        let y = rest.top() + first;
        (Rect::from_min_max(rest.min, Pos2::new(rest.right(), y)), Rect::from_min_max(Pos2::new(rest.left(), y), Pos2::new(rest.right(), y + GAP)), Rect::from_min_max(Pos2::new(rest.left(), y + GAP), rest.max))
    } else {
        let x = rest.left() + first;
        (Rect::from_min_max(rest.min, Pos2::new(x, rest.bottom())), Rect::from_min_max(Pos2::new(x, rest.top()), Pos2::new(x + GAP, rest.bottom())), Rect::from_min_max(Pos2::new(x + GAP, rest.top()), rest.max))
    };
    let (list, detail) = if layout.detail_first { (b, a) } else { (a, b) };
    Split { list, detail: Some(detail), splitter: Some(gap), stacked }
}

/// The layout a card dropped at `at` asks for: the side of `rest` nearest the pointer (measured against the room's
/// own proportions) takes the dragged card, the other card takes the rest. The dragged card keeps its share.
pub(super) fn dropped(rest: Rect, at: Pos2, dragged_detail: bool, now: PageLayout) -> PageLayout {
    let d = at - rest.center();
    let (dx, dy) = (d.x / rest.width().max(1.0), d.y / rest.height().max(1.0));
    let stacked = dy.abs() > dx.abs();
    let dragged_first = if stacked { dy < 0.0 } else { dx < 0.0 };
    let detail_first = dragged_first == dragged_detail;
    // The share of whichever card was first before; flip it when the other card is first now.
    let dragged_share = if now.detail_first == dragged_detail { now.split } else { 1.0 - now.split };
    let split = if dragged_first { dragged_share } else { 1.0 - dragged_share };
    PageLayout { detail_first, stacked, split }
}

/// The share of the first card for a splitter dragged to `at`.
pub(super) fn split_at(rest: Rect, at: Pos2, stacked: bool) -> f32 {
    let (offset, size) = if stacked { (at.y - rest.top(), rest.height()) } else { (at.x - rest.left(), rest.width()) };
    ((offset - GAP / 2.0) / (size - GAP).max(1.0)).clamp(0.0, 1.0)
}

/// The rectangle of a card's own drop preview: where it would sit after `dropped`.
pub(super) fn preview(rest: Rect, layout: PageLayout, detail: bool) -> Rect {
    let s = split(rest, layout, true);
    if detail { s.detail.unwrap_or(rest) } else { s.list }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Vec2;

    fn room(w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(w, h))
    }

    #[test]
    fn the_default_puts_the_list_left_with_the_larger_share_and_the_gap_between() {
        let r = room(1200.0, 700.0);
        let s = split(r, PageLayout::default(), true);
        let d = s.detail.unwrap();
        assert!(!s.stacked);
        assert!((s.list.width() - (1200.0 - GAP) * 0.55).abs() < 0.01, "{s:?}");
        assert_eq!(s.list.left(), r.left());
        assert!((d.left() - s.list.right() - GAP).abs() < 0.01 && d.right() == r.right());
        assert_eq!(s.splitter.unwrap().width(), GAP);
        assert_eq!(split(r, PageLayout::default(), false).list, r, "no detail: the list has the room");
    }

    #[test]
    fn detail_first_and_stacked_swap_and_rotate_the_cards() {
        let r = room(1200.0, 700.0);
        let s = split(r, PageLayout { detail_first: true, stacked: false, split: 0.4 }, true);
        let d = s.detail.unwrap();
        assert_eq!(d.left(), r.left());
        assert!(s.list.left() > d.right());
        let s = split(r, PageLayout { detail_first: false, stacked: true, split: 0.4 }, true);
        let d = s.detail.unwrap();
        assert!(s.stacked && s.list.top() == r.top() && d.bottom() == r.bottom());
        assert!((s.list.height() - (700.0 - GAP) * 0.4).abs() < 0.01);
        assert_eq!(s.list.width(), r.width());
    }

    #[test]
    fn a_narrow_room_stacks_and_a_short_one_keeps_only_the_list_and_no_card_goes_below_its_least_size() {
        let s = split(room(700.0, 700.0), PageLayout::default(), true);
        assert!(s.stacked && s.detail.is_some(), "too narrow side by side: stacked");
        let s = split(room(700.0, 300.0), PageLayout::default(), true);
        assert!(s.detail.is_none(), "too short to stack: the list alone");
        let s = split(room(1200.0, 700.0), PageLayout { split: 0.02, ..Default::default() }, true);
        assert!(s.list.width() >= MIN_SIDE - 0.01 && s.detail.unwrap().width() >= MIN_SIDE - 0.01);
    }

    #[test]
    fn dropping_a_card_near_a_side_puts_it_there_and_it_keeps_its_share() {
        let r = room(1200.0, 700.0);
        let now = PageLayout { detail_first: false, stacked: false, split: 0.6 };
        // The detail (right, 40 %) dropped on the left edge: detail first, still 40 %.
        let l = dropped(r, Pos2::new(r.left() + 20.0, r.center().y), true, now);
        assert_eq!((l.detail_first, l.stacked), (true, false));
        assert!((l.split - 0.4).abs() < 1e-6);
        // Dropped at the top: stacked above the list.
        let l = dropped(r, Pos2::new(r.center().x, r.top() + 10.0), true, now);
        assert_eq!((l.detail_first, l.stacked), (true, true));
        // The list dropped at the bottom: stacked, list second, keeping its 60 %.
        let l = dropped(r, Pos2::new(r.center().x + 30.0, r.bottom() - 5.0), false, now);
        assert_eq!((l.detail_first, l.stacked), (true, true));
        assert!((l.split - 0.4).abs() < 1e-6, "the detail first with 40 %, the list keeps 60 %");
        // The list dropped on the right: detail first.
        let l = dropped(r, Pos2::new(r.right() - 5.0, r.center().y), false, now);
        assert_eq!((l.detail_first, l.stacked), (true, false));
        // The preview is where the card lands.
        assert_eq!(preview(r, l, false), split(r, l, true).list);
    }

    #[test]
    fn the_splitter_share_follows_the_pointer() {
        let r = room(1200.0, 700.0);
        let at = Pos2::new(r.left() + GAP / 2.0 + 0.4 * (1200.0 - GAP), r.center().y);
        assert!((split_at(r, at, false) - 0.4).abs() < 1e-4);
        let s = split(r, PageLayout { split: split_at(r, at, false), ..Default::default() }, true);
        assert!((s.splitter.unwrap().center().x - at.x).abs() < 0.01, "the gap sits under the pointer");
        assert_eq!(split_at(r, Pos2::new(0.0, 0.0), false), 0.0);
    }
}
