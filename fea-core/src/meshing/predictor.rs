//! Initial mesh-size prediction for a 2D region, before any solution exists: where the geometry and the problem data say the
//! mesh must be fine (curved boundary, re-entrant corners, loads, restraints, contact patches), as a size function for the
//! meshers. Every source asks for a size `s_i` at a place; the predicted size is the smallest `s_i + g d_i` (`d_i` the
//! distance to the source, `g` the grading rate), capped by the base size. That is a Lipschitz size field with constant `g`,
//! so neighbouring elements differ by at most about `g` times their distance: the size jump the quality gate checks is
//! bounded by construction. This is a predictor, not an estimator: adaptivity (`adaptivity::run_amr`) corrects it.

use crate::geometry::{Curve, Region};
use std::f64::consts::{PI, TAU};

#[derive(Debug, Clone, Copy)]
enum Source {
    Point([f64; 2]),
    Line([f64; 2], [f64; 2]),
}

impl Source {
    fn distance(&self, p: [f64; 2]) -> f64 {
        match *self {
            Self::Point(q) => (p[0] - q[0]).hypot(p[1] - q[1]),
            Self::Line(a, b) => {
                let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
                let l2 = ab[0] * ab[0] + ab[1] * ab[1];
                let t = if l2 > 0.0 { ((ap[0] * ab[0] + ap[1] * ab[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
                (p[0] - a[0] - t * ab[0]).hypot(p[1] - a[1] - t * ab[1])
            }
        }
    }
}

/// How the geometry contributes to the predicted size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeometryRules {
    /// Largest angle one element may span on a circular arc (radians): size at an arc is at most `r * arc_angle`.
    pub arc_angle: f64,
    /// Size at a re-entrant corner (interior angle above `reentrant_angle`) as a fraction of the base size.
    pub corner_fraction: f64,
    /// Interior angle (radians) from which a corner counts as re-entrant.
    pub reentrant_angle: f64,
}

impl Default for GeometryRules {
    fn default() -> Self {
        Self { arc_angle: 0.35, corner_fraction: 0.25, reentrant_angle: PI + 0.1 }
    }
}

/// A size function assembled from sources.
#[derive(Debug, Clone)]
pub struct SizePredictor2d {
    base: f64,
    min: f64,
    grading: f64,
    sources: Vec<(Source, f64)>,
}

impl SizePredictor2d {
    /// `base` is the size far from every source, `min` the smallest size any source may ask for, `grading` the growth of the
    /// size per unit distance from a source (0.2 to 0.5 is typical; it must be positive).
    pub fn new(base: f64, min: f64, grading: f64) -> Result<Self, String> {
        if !(base > 0.0 && min > 0.0 && min <= base && grading > 0.0 && grading.is_finite()) {
            return Err("size predictor needs 0 < min <= base and a positive finite grading rate".into());
        }
        Ok(Self { base, min, grading, sources: Vec::new() })
    }

    /// A load, restraint or any point of interest that wants size `size` there.
    pub fn add_point(&mut self, p: [f64; 2], size: f64) -> &mut Self {
        self.sources.push((Source::Point(p), size.clamp(self.min, self.base)));
        self
    }

    /// A line of interest (a contact patch, a loaded edge) that wants size `size` along it.
    pub fn add_line(&mut self, a: [f64; 2], b: [f64; 2], size: f64) -> &mut Self {
        self.sources.push((Source::Line(a, b), size.clamp(self.min, self.base)));
        self
    }

    /// Curved boundaries and re-entrant corners of `region`.
    pub fn add_geometry(&mut self, region: &Region, rules: GeometryRules) -> &mut Self {
        for l in region.loops() {
            for s in &l.segments {
                if let Curve::Arc { r, .. } = s.curve {
                    let size = r * rules.arc_angle;
                    let n = ((s.curve.length() / (0.5 * size)).ceil() as usize).clamp(1, 512);
                    for i in 0..=n {
                        let p = s.curve.point(i as f64 / n as f64);
                        self.add_point(p, size);
                    }
                }
                let v = s.curve.start();
                if interior_angle(region, v) > rules.reentrant_angle {
                    self.add_point(v, rules.corner_fraction * self.base);
                }
            }
        }
        self
    }

    /// The predicted size at `p`.
    pub fn size(&self, p: [f64; 2]) -> f64 {
        self.sources.iter().fold(self.base, |s, (src, size)| s.min(size + self.grading * src.distance(p)))
    }

    /// The size function the meshers take.
    pub fn size_fn(&self) -> impl Fn([f64; 2]) -> f64 + '_ {
        move |p| self.size(p)
    }
}

/// Interior angle of the region at a boundary point `v`, from the fraction of a small circle around it that lies inside.
fn interior_angle(region: &Region, v: [f64; 2]) -> f64 {
    let (lo, hi) = region.bounds();
    let eps = 1e-4 * (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    const N: usize = 720;
    let inside = (0..N).filter(|&k| {
        let th = (k as f64 + 0.5) * TAU / N as f64;
        region.contains([v[0] + eps * th.cos(), v[1] + eps * th.sin()])
    });
    inside.count() as f64 * TAU / N as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Loop;
    use crate::mesh::Elastic;

    fn region(outer: Loop, holes: Vec<Loop>) -> Region {
        Region::new(outer, holes, Elastic::new(1.0, 0.3)).unwrap()
    }

    #[test]
    fn a_hole_and_a_reentrant_corner_ask_for_small_elements_a_convex_corner_does_not() {
        let l = Loop::polygon(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [2.0, 2.0], [2.0, 4.0], [0.0, 4.0]], "l").unwrap();
        let mut p = SizePredictor2d::new(1.0, 0.05, 0.3).unwrap();
        p.add_geometry(&region(l, vec![]), GeometryRules::default());
        assert!((p.size([2.0, 2.0]) - 0.25).abs() < 1e-12, "re-entrant corner {}", p.size([2.0, 2.0]));
        assert_eq!(p.size([0.0, 0.0]), 1.0, "a convex corner needs no refinement");
        assert_eq!(p.size([4.0, 0.0]), 1.0);

        let hole = Loop::circle([5.0, 2.0], 0.5, "hole").unwrap();
        let mut q = SizePredictor2d::new(1.0, 0.05, 0.3).unwrap();
        q.add_geometry(&region(Loop::rectangle(0.0, 0.0, 10.0, 4.0).unwrap(), vec![hole]), GeometryRules::default());
        let at_hole = q.size([5.5, 2.0]);
        assert!(at_hole <= 0.5 * 0.35 + 1e-12, "arc size {at_hole}");
        assert!(q.size([1.0, 2.0]) > 0.9 && q.size([0.0, 0.0]) == 1.0);
    }

    #[test]
    fn the_size_field_is_lipschitz_with_the_grading_rate_and_never_exceeds_the_base() {
        let mut p = SizePredictor2d::new(1.0, 0.02, 0.3).unwrap();
        p.add_point([1.0, 1.0], 0.05).add_line([3.0, 0.0], [3.0, 2.0], 0.1);
        let pts: Vec<[f64; 2]> = (0..40).flat_map(|i| (0..40).map(move |j| [i as f64 * 0.13, j as f64 * 0.07])).collect();
        for a in &pts {
            assert!(p.size(*a) <= 1.0 && p.size(*a) >= 0.02);
            for b in pts.iter().step_by(37) {
                let d = (a[0] - b[0]).hypot(a[1] - b[1]);
                assert!((p.size(*a) - p.size(*b)).abs() <= 0.3 * d + 1e-12);
            }
        }
        assert!((p.size([1.0, 1.0]) - 0.05).abs() < 1e-12 && (p.size([3.0, 1.0]) - 0.1).abs() < 1e-12);
    }

    #[test]
    fn invalid_parameters_are_refused() {
        assert!(SizePredictor2d::new(1.0, 2.0, 0.3).is_err() && SizePredictor2d::new(1.0, 0.1, 0.0).is_err());
    }
}
