//! Colour-contour drawing: a `fea_problem` raster painted with half blocks (each text cell holds two
//! square-ish pixels, the upper in the foreground, the lower in the background).

use fea_problem::raster::Raster;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::Frame;

/// Blue - cyan - green - yellow - red for `t` in `0..=1`.
pub fn colour(t: f64) -> (u8, u8, u8) {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    const STOPS: [(f64, (f64, f64, f64)); 5] = [(0.0, (30.0, 60.0, 200.0)), (0.25, (30.0, 190.0, 230.0)), (0.5, (60.0, 200.0, 90.0)), (0.75, (240.0, 220.0, 50.0)), (1.0, (220.0, 40.0, 30.0))];
    for w in STOPS.windows(2) {
        let ((t0, c0), (t1, c1)) = (w[0], w[1]);
        if t <= t1 {
            let f = (t - t0) / (t1 - t0);
            let mix = |a: f64, b: f64| (a + f * (b - a)).round() as u8;
            return (mix(c0.0, c1.0), mix(c0.1, c1.1), mix(c0.2, c1.2));
        }
    }
    (220, 40, 30)
}

/// Fill used when no field is drawn (a mesh preview).
const FLAT: (u8, u8, u8) = (70, 110, 150);

fn shade(c: (u8, u8, u8), k: f64) -> Color {
    Color::Rgb((c.0 as f64 * k) as u8, (c.1 as f64 * k) as u8, (c.2 as f64 * k) as u8)
}

/// Normalised position of `v` in the raster's range.
fn norm(r: &Raster, v: f64) -> f64 {
    if r.max > r.min {
        (v - r.min) / (r.max - r.min)
    } else {
        0.5
    }
}

/// Raster size (pixels) that fills `area` (two pixel rows per text row).
pub fn pixel_size(area: Rect) -> (usize, usize) {
    (area.width as usize, area.height as usize * 2)
}

/// Paint the raster into `area`. `field` false draws the flat mesh preview colour; `mesh` strokes
/// the element outlines.
pub fn draw(frame: &mut Frame, area: Rect, r: &Raster, field: bool, mesh: bool) {
    if area.width == 0 || area.height == 0 || r.w == 0 || r.h == 0 {
        return;
    }
    let pixel = |x: usize, y: usize| -> Option<Color> {
        if x >= r.w || y >= r.h {
            return None;
        }
        let v = r.at(x, y);
        if v.is_nan() {
            return None;
        }
        let base = if field { colour(norm(r, v)) } else { FLAT };
        let edge = mesh && r.edge[y * r.w + x];
        Some(shade(base, if edge { 0.45 } else { 1.0 }))
    };
    let buf = frame.buffer_mut();
    for row in 0..area.height {
        for col in 0..area.width {
            let (top, bottom) = (pixel(col as usize, row as usize * 2), pixel(col as usize, row as usize * 2 + 1));
            if top.is_none() && bottom.is_none() {
                continue;
            }
            if let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) {
                cell.set_symbol("\u{2580}");
                cell.set_fg(top.unwrap_or(Color::Reset));
                cell.set_bg(bottom.unwrap_or(Color::Reset));
                if top.is_none() {
                    // Only the lower half is body: draw the lower half block instead.
                    cell.set_symbol("\u{2584}");
                    cell.set_fg(bottom.unwrap_or(Color::Reset));
                    cell.set_bg(Color::Reset);
                }
            }
        }
    }
}

/// The colour-bar glyph colours for a legend of `n` cells.
pub fn legend(n: usize) -> Vec<Color> {
    (0..n).map(|i| {
        let c = colour(if n > 1 { i as f64 / (n - 1) as f64 } else { 0.5 });
        Color::Rgb(c.0, c.1, c.2)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_ramp_runs_blue_to_red_and_clamps() {
        assert_eq!(colour(0.0), (30, 60, 200));
        assert_eq!(colour(1.0), (220, 40, 30));
        assert_eq!(colour(-3.0), colour(0.0));
        assert_eq!(colour(9.0), colour(1.0));
        assert_eq!(colour(f64::NAN), colour(0.0));
        let mid = colour(0.5);
        assert!(mid.1 > mid.0 && mid.1 > mid.2, "green in the middle: {mid:?}");
    }
}
