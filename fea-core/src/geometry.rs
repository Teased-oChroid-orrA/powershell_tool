//! 2D region geometry for meshing: closed loops of straight and circular-arc segments (an outer
//! boundary and any number of holes), each segment carrying a name that becomes a node set and a
//! surface of the generated mesh.
//!
//! Curves are parametrised by `t` in `[0, 1]`; the mesher evaluates them for every boundary node, so
//! arcs are represented exactly at nodes (including the midside nodes of quadratic elements).

use crate::mesh::Elastic;
use std::f64::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    Line { a: [f64; 2], b: [f64; 2] },
    /// Circular arc about `c` from angle `a0` to `a1` (radians); `a1 < a0` runs clockwise. The sweep
    /// magnitude is at most a full turn.
    Arc { c: [f64; 2], r: f64, a0: f64, a1: f64 },
}

impl Curve {
    pub fn line(a: [f64; 2], b: [f64; 2]) -> Self {
        Curve::Line { a, b }
    }

    pub fn arc(c: [f64; 2], r: f64, a0: f64, a1: f64) -> Self {
        Curve::Arc { c, r, a0, a1 }
    }

    pub fn point(&self, t: f64) -> [f64; 2] {
        match *self {
            Curve::Line { a, b } => [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
            Curve::Arc { c, r, a0, a1 } => {
                let th = a0 + t * (a1 - a0);
                [c[0] + r * th.cos(), c[1] + r * th.sin()]
            }
        }
    }

    pub fn start(&self) -> [f64; 2] {
        self.point(0.0)
    }

    pub fn end(&self) -> [f64; 2] {
        self.point(1.0)
    }

    pub fn length(&self) -> f64 {
        match *self {
            Curve::Line { a, b } => (b[0] - a[0]).hypot(b[1] - a[1]),
            Curve::Arc { r, a0, a1, .. } => r * (a1 - a0).abs(),
        }
    }

    /// Angle swept (radians, zero for a line).
    pub fn sweep(&self) -> f64 {
        match *self {
            Curve::Line { .. } => 0.0,
            Curve::Arc { a0, a1, .. } => (a1 - a0).abs(),
        }
    }

    fn reversed(&self) -> Curve {
        match *self {
            Curve::Line { a, b } => Curve::Line { a: b, b: a },
            Curve::Arc { c, r, a0, a1 } => Curve::Arc { c, r, a0: a1, a1: a0 },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub curve: Curve,
    pub name: String,
}

/// A closed chain of segments, each starting where the previous one ends.
#[derive(Debug, Clone, PartialEq)]
pub struct Loop {
    pub segments: Vec<Segment>,
}

impl Loop {
    pub fn new(segments: Vec<Segment>) -> Result<Self, String> {
        let l = Self { segments };
        l.validate()?;
        Ok(l)
    }

    /// Closed polygon through `pts`; every edge gets the same `name`.
    pub fn polygon(pts: &[[f64; 2]], name: &str) -> Result<Self, String> {
        Self::polygon_named(pts, &vec![name; pts.len()])
    }

    /// Closed polygon, edge `i` (from `pts[i]` to `pts[i + 1]`) named `names[i]`.
    pub fn polygon_named(pts: &[[f64; 2]], names: &[&str]) -> Result<Self, String> {
        if pts.len() < 3 || names.len() != pts.len() {
            return Err("a polygon needs at least three points and one name per edge".into());
        }
        Self::new((0..pts.len()).map(|i| Segment { curve: Curve::line(pts[i], pts[(i + 1) % pts.len()]), name: names[i].to_string() }).collect())
    }

    /// Axis-aligned rectangle; edges named `bottom`, `right`, `top`, `left`.
    pub fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> Result<Self, String> {
        Self::polygon_named(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], &["bottom", "right", "top", "left"])
    }

    /// Full circle as two half arcs (counter-clockwise), both named `name`.
    pub fn circle(c: [f64; 2], r: f64, name: &str) -> Result<Self, String> {
        Self::new(vec![
            Segment { curve: Curve::arc(c, r, 0.0, std::f64::consts::PI), name: name.to_string() },
            Segment { curve: Curve::arc(c, r, std::f64::consts::PI, TAU), name: name.to_string() },
        ])
    }

    fn validate(&self) -> Result<(), String> {
        if self.segments.is_empty() {
            return Err("an empty loop".into());
        }
        let scale = self.segments.iter().map(|s| s.curve.length()).fold(0.0f64, f64::max).max(1e-300);
        for (i, s) in self.segments.iter().enumerate() {
            if s.curve.length().is_nan() || s.curve.length() <= 1e-12 * scale {
                return Err(format!("segment {i} ('{}') has zero length", s.name));
            }
            if let Curve::Arc { r, a0, a1, .. } = s.curve {
                if r.is_nan() || r <= 0.0 || (a1 - a0).abs() > TAU + 1e-12 {
                    return Err(format!("segment {i} ('{}') is not a valid arc", s.name));
                }
            }
            let (e, nxt) = (s.curve.end(), self.segments[(i + 1) % self.segments.len()].curve.start());
            if (e[0] - nxt[0]).hypot(e[1] - nxt[1]) > 1e-9 * scale {
                return Err(format!("segment {i} ('{}') does not meet the next segment: {e:?} vs {nxt:?}", s.name));
            }
        }
        Ok(())
    }

    /// Signed area (positive counter-clockwise) of the loop with arcs integrated exactly.
    pub fn signed_area(&self) -> f64 {
        self.segments
            .iter()
            .map(|s| match s.curve {
                Curve::Line { a, b } => 0.5 * (a[0] * b[1] - b[0] * a[1]),
                // Green's theorem: 1/2 * int (x dy - y dx) over the arc.
                Curve::Arc { c, r, a0, a1 } => 0.5 * (c[0] * r * (a1.sin() - a0.sin()) - c[1] * r * (a1.cos() - a0.cos()) + r * r * (a1 - a0)),
            })
            .sum()
    }

    fn reversed(&self) -> Loop {
        Loop { segments: self.segments.iter().rev().map(|s| Segment { curve: s.curve.reversed(), name: s.name.clone() }).collect() }
    }

    /// The loop traversed counter-clockwise (`ccw`) or clockwise.
    pub fn oriented(&self, ccw: bool) -> Loop {
        if (self.signed_area() > 0.0) == ccw {
            self.clone()
        } else {
            self.reversed()
        }
    }

    /// The loop as a closed polyline (arcs sampled every few degrees).
    fn polyline(&self) -> Vec<[f64; 2]> {
        let mut pts = Vec::new();
        for s in &self.segments {
            let n = if matches!(s.curve, Curve::Line { .. }) { 1 } else { ((s.curve.sweep() / 0.05).ceil() as usize).max(8) };
            pts.extend((0..n).map(|k| s.curve.point(k as f64 / n as f64)));
        }
        pts
    }

    /// Whether the boundaries of the two loops cross or touch.
    fn crosses(&self, other: &Loop) -> bool {
        let (a, b) = (self.polyline(), other.polyline());
        let orient = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
        // `r` (known collinear with `p`-`q`) lies within the segment's bounding box.
        let within = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| r[0] >= p[0].min(q[0]) && r[0] <= p[0].max(q[0]) && r[1] >= p[1].min(q[1]) && r[1] <= p[1].max(q[1]);
        for i in 0..a.len() {
            let (p1, p2) = (a[i], a[(i + 1) % a.len()]);
            for j in 0..b.len() {
                let (q1, q2) = (b[j], b[(j + 1) % b.len()]);
                let (d1, d2, d3, d4) = (orient(p1, p2, q1), orient(p1, p2, q2), orient(q1, q2, p1), orient(q1, q2, p2));
                if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
                    return true;
                }
                // Touching or collinear overlap only counts when the point really lies on the other segment.
                if (d1 == 0.0 && within(p1, p2, q1)) || (d2 == 0.0 && within(p1, p2, q2)) || (d3 == 0.0 && within(q1, q2, p1)) || (d4 == 0.0 && within(q1, q2, p2)) {
                    return true;
                }
            }
        }
        false
    }

    /// Whether `p` is inside (even-odd crossing count of the closed polyline; arcs sampled finely).
    fn contains(&self, p: [f64; 2]) -> bool {
        let poly = self.polyline();
        let mut crossings = 0;
        for k in 0..poly.len() {
            let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > p[0] {
                    crossings += 1;
                }
            }
        }
        crossings % 2 == 1
    }
}

/// A planar region: an outer loop minus holes, with a material for the generated mesh.
#[derive(Debug, Clone)]
pub struct Region {
    /// Counter-clockwise.
    pub outer: Loop,
    /// Clockwise.
    pub holes: Vec<Loop>,
    pub material: Elastic,
}

impl Region {
    pub fn new(outer: Loop, holes: Vec<Loop>, material: Elastic) -> Result<Self, String> {
        material.validate()?;
        let region = Self { outer: outer.oriented(true), holes: holes.iter().map(|h| h.oriented(false)).collect(), material };
        for (i, h) in region.holes.iter().enumerate() {
            if h.crosses(&region.outer) || !region.outer.contains(h.segments[0].curve.start()) {
                return Err(format!("hole {i} is not inside the outer boundary"));
            }
            for (j, o) in region.holes.iter().enumerate().skip(i + 1) {
                if h.crosses(o) || o.contains(h.segments[0].curve.start()) || h.contains(o.segments[0].curve.start()) {
                    return Err(format!("holes {i} and {j} overlap"));
                }
            }
        }
        Ok(region)
    }

    pub fn area(&self) -> f64 {
        self.outer.signed_area() + self.holes.iter().map(|h| h.signed_area()).sum::<f64>()
    }

    /// Whether `p` is in the region (inside the outer loop, outside every hole).
    pub fn contains(&self, p: [f64; 2]) -> bool {
        self.outer.contains(p) && !self.holes.iter().any(|h| h.contains(p))
    }

    /// All loops, outer first.
    pub fn loops(&self) -> impl Iterator<Item = &Loop> {
        std::iter::once(&self.outer).chain(self.holes.iter())
    }

    /// Bounding box `(min, max)` of the sampled boundary.
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for l in self.loops() {
            for s in &l.segments {
                for k in 0..=32 {
                    let p = s.curve.point(k as f64 / 32.0);
                    for i in 0..2 {
                        lo[i] = lo[i].min(p[i]);
                        hi[i] = hi[i].max(p[i]);
                    }
                }
            }
        }
        (lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn steel() -> Elastic {
        Elastic::new(30e6, 0.3)
    }

    #[test]
    fn areas_orientations_and_containment() {
        let rect = Loop::rectangle(0.0, 0.0, 4.0, 2.0).unwrap();
        assert!((rect.signed_area() - 8.0).abs() < 1e-12);
        let circle = Loop::circle([1.0, 1.0], 0.5, "bore").unwrap();
        assert!((circle.signed_area() - PI * 0.25).abs() < 1e-12, "{}", circle.signed_area());
        let region = Region::new(rect.reversed(), vec![circle.clone()], steel()).unwrap();
        // The outer loop is made counter-clockwise, the hole clockwise, whatever the input order.
        assert!(region.outer.signed_area() > 0.0 && region.holes[0].signed_area() < 0.0);
        assert!((region.area() - (8.0 - PI * 0.25)).abs() < 1e-12);
        assert!(region.contains([3.0, 1.0]) && !region.contains([1.0, 1.0]) && !region.contains([5.0, 1.0]));
        let (lo, hi) = region.bounds();
        assert!(lo == [0.0, 0.0] && hi == [4.0, 2.0]);
    }

    #[test]
    fn invalid_loops_and_regions_are_rejected() {
        // A gap between segments.
        let bad = Loop::new(vec![Segment { curve: Curve::line([0.0, 0.0], [1.0, 0.0]), name: "a".into() }, Segment { curve: Curve::line([1.1, 0.0], [0.0, 0.0]), name: "b".into() }]);
        assert!(bad.unwrap_err().contains("does not meet"));
        assert!(Loop::polygon(&[[0.0, 0.0], [1.0, 0.0]], "x").is_err());
        let outer = Loop::rectangle(0.0, 0.0, 2.0, 2.0).unwrap();
        assert!(Region::new(outer.clone(), vec![Loop::circle([5.0, 5.0], 0.5, "h").unwrap()], steel()).unwrap_err().contains("not inside"));
        let a = Loop::circle([1.0, 1.0], 0.5, "a").unwrap();
        let b = Loop::circle([1.2, 1.0], 0.5, "b").unwrap();
        assert!(Region::new(outer.clone(), vec![a, b], steel()).unwrap_err().contains("overlap"));
        // Disjoint circles with horizontal chords at the same height are collinear but do not touch.
        let (c, d) = (Loop::circle([0.6, 1.0], 0.3, "c").unwrap(), Loop::circle([1.4, 1.0], 0.3, "d").unwrap());
        assert!(Region::new(outer, vec![c, d], steel()).is_ok());
    }

    #[test]
    fn arc_parametrisation_is_exact() {
        let a = Curve::arc([1.0, 2.0], 3.0, PI / 2.0, PI);
        let p = a.point(0.5);
        assert!(((p[0] - 1.0).hypot(p[1] - 2.0) - 3.0).abs() < 1e-14);
        assert!((a.length() - 1.5 * PI).abs() < 1e-14 && (a.sweep() - PI / 2.0).abs() < 1e-14);
        assert_eq!(a.reversed().start(), a.end());
    }
}
