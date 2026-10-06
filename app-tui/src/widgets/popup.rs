//! Shared geometry for centred overlays (pickers, palette, help).

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// The `percent_x` x `percent_y` rectangle centred in `area`.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage((100 - percent_y) / 2), Constraint::Percentage(percent_y), Constraint::Percentage((100 - percent_y) / 2)])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage((100 - percent_x) / 2), Constraint::Percentage(percent_x), Constraint::Percentage((100 - percent_x) / 2)])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_centred_and_sized_by_percentage() {
        let r = centered_rect(50, 50, Rect::new(0, 0, 100, 40));
        assert_eq!((r.x, r.y, r.width, r.height), (25, 10, 50, 20));
    }

    #[test]
    fn full_size_is_the_whole_area() {
        let a = Rect::new(0, 0, 80, 24);
        assert_eq!(centered_rect(100, 100, a), a);
    }
}
