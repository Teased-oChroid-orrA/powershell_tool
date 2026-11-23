//! How near a hole is to a free edge of its plate: the geometry the edge-distance estimate (`edge-check`) needs from a
//! sketch problem. Only straight-edged and circular outlines are read; an outline with arcs, a slot, a region other than
//! the first, or a hole that is not a circle is refused with the reason, never guessed.

use crate::problem::{Geometry, Load, Problem, Shape};

/// A circular hole and the nearest free boundary of its plate.
#[derive(Debug, Clone, PartialEq)]
pub struct HoleEdge {
    pub radius: f64,
    pub centre: [f64; 2],
    /// Distance from the hole centre to the nearest free boundary, in the problem's length unit.
    pub edge_distance: f64,
    /// Name of that boundary (`left`, `edge3`, `outer`).
    pub edge: String,
}

fn point_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len2 > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1]) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p[0] - a[0] - t * ab[0]).hypot(p[1] - a[1] - t * ab[1])
}

impl Problem {
    /// The nearest free boundary of hole `n` (1-based, as `Load::Bearing::hole`). A boundary is free when no support and
    /// no edge load names it.
    pub fn hole_edge(&self, n: usize) -> Result<HoleEdge, String> {
        let Geometry::Sketch { outer, .. } = &self.geometry else { return Err("the hole check reads a sketch, not an imported mesh or a solid".into()) };
        let Some((shape, region, _)) = self.hole_at(n) else { return Err(format!("there is no hole {n}")) };
        if region != 0 {
            return Err("the hole check reads the first region only".into());
        }
        let Shape::Circle { cx, cy, r } = shape else { return Err(format!("hole {n} is not a circle")) };
        let (centre, radius) = ([*cx, *cy], *r);
        let used: Vec<&str> = self.supports.iter().filter_map(|s| s.edge()).chain(self.loads.iter().filter_map(Load::edge)).collect();
        let free = |name: &str| !used.contains(&name);
        let mut best: Option<(f64, String)> = None;
        let mut consider = |d: f64, name: String| {
            if free(&name) && best.as_ref().is_none_or(|(b, _)| d < *b) {
                best = Some((d, name));
            }
        };
        match outer {
            Shape::Rect { x0, y0, x1, y1 } => {
                consider(centre[1] - y0.min(*y1), "bottom".into());
                consider(x0.max(*x1) - centre[0], "right".into());
                consider(y0.max(*y1) - centre[1], "top".into());
                consider(centre[0] - x0.min(*x1), "left".into());
            }
            Shape::Polygon { pts } => {
                for i in 0..pts.len() {
                    consider(point_segment(centre, pts[i], pts[(i + 1) % pts.len()]), format!("edge{}", i + 1));
                }
            }
            Shape::Circle { cx: ox, cy: oy, r: or } => consider(or - (centre[0] - ox).hypot(centre[1] - oy), "outer".into()),
            Shape::Slot { .. } | Shape::Outline { .. } => return Err("the outline has arcs or is a slot: only rectangles, polygons and circles are read".into()),
        }
        let (edge_distance, edge) = best.ok_or("every boundary carries a support or a load: there is no free edge")?;
        if edge_distance <= radius {
            return Err(format!("the hole reaches the boundary: the centre is {edge_distance:.4} from {edge}, the radius {radius:.4}"));
        }
        Ok(HoleEdge { radius, centre, edge_distance, edge })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::Support;

    fn plate() -> Problem {
        let mut p = crate::templates::blank();
        p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 4.0, y1: 2.0 }, holes: vec![Shape::Circle { cx: 0.8, cy: 1.0, r: 0.25 }], depth: 1.0, layers: 4, regions: Vec::new() };
        p.supports.clear();
        p.loads.clear();
        p
    }

    #[test]
    fn the_nearest_free_edge_of_a_rectangle_is_found_by_name() {
        let h = plate().hole_edge(1).unwrap();
        assert_eq!((h.edge.as_str(), h.radius), ("left", 0.25));
        assert!((h.edge_distance - 0.8).abs() < 1e-12, "{h:?}");
    }

    #[test]
    fn a_supported_edge_is_not_free() {
        let mut p = plate();
        p.supports.push(Support::fixed("left"));
        let h = p.hole_edge(1).unwrap();
        assert_eq!(h.edge, "bottom", "the next nearest (tied with the top, the first wins): the centre is 1.0 from the bottom");
        assert!((h.edge_distance - 1.0).abs() < 1e-12);
    }

    #[test]
    fn polygons_and_circles_are_read_and_arcs_and_non_circular_holes_are_refused() {
        let mut p = plate();
        if let Geometry::Sketch { outer, .. } = &mut p.geometry {
            *outer = Shape::Polygon { pts: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]] };
        }
        let h = p.hole_edge(1).unwrap();
        assert_eq!(h.edge, "edge4", "the polygon's fourth edge runs from (0, 2) to (0, 0)");
        assert!((h.edge_distance - 0.8).abs() < 1e-12);
        if let Geometry::Sketch { outer, .. } = &mut p.geometry {
            *outer = Shape::Circle { cx: 1.0, cy: 1.0, r: 3.0 };
        }
        assert!((p.hole_edge(1).unwrap().edge_distance - 2.8).abs() < 1e-12);
        if let Geometry::Sketch { outer, .. } = &mut p.geometry {
            *outer = Shape::Slot { cx: 0.0, cy: 0.0, length: 5.0, width: 2.0, angle: 0.0 };
        }
        assert!(p.hole_edge(1).unwrap_err().contains("arcs or is a slot"));
        assert!(plate().hole_edge(2).unwrap_err().contains("no hole 2"));
        let mut q = plate();
        if let Geometry::Sketch { holes, .. } = &mut q.geometry {
            holes[0] = Shape::Slot { cx: 0.8, cy: 1.0, length: 0.5, width: 0.2, angle: 0.0 };
        }
        assert!(q.hole_edge(1).unwrap_err().contains("not a circle"));
    }

    #[test]
    fn a_hole_through_the_boundary_is_refused() {
        let mut p = plate();
        if let Geometry::Sketch { holes, .. } = &mut p.geometry {
            holes[0] = Shape::Circle { cx: 0.2, cy: 1.0, r: 0.25 };
        }
        assert!(p.hole_edge(1).unwrap_err().contains("reaches the boundary"));
    }
}
