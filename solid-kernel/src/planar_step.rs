//! Validation/classification of sampled planar trim polygons, not exact curved
//! trim intersection certification. No tessellation or production CAD switch.
use crate::{
    analytic_step::{AnalyticModel, Surface},
    step::{Error, ErrorKind},
};
use fea_core::meshing::tet_topology::orientation;
use std::cmp::Ordering;
type P = [f64; 2];
#[derive(Debug, Clone)]
pub struct PlanarLoop {
    pub bound_id: u64,
    /// No repeated closing point. Source traversal is retained, not rewound.
    pub points: Vec<P>,
    pub is_outer: bool,
}
#[derive(Debug, Clone)]
pub struct PlanarTrims {
    pub face_id: u64,
    pub loops: Vec<PlanarLoop>,
}
fn fail(kind: ErrorKind, message: &str) -> Error {
    Error {
        offset: 0,
        kind,
        detail: message.into(),
    }
}
fn sign(a: P, b: P, c: P) -> Result<Ordering, Error> {
    orientation(
        [a[0], a[1], 0.],
        [b[0], b[1], 0.],
        [c[0], c[1], 0.],
        [0., 0., 1.],
    )
    .map_err(|_| {
        fail(
            ErrorKind::Geometry,
            "planar predicate arithmetic range exceeded",
        )
    })
}
fn between(a: P, b: P, p: P) -> bool {
    (0..2).all(|i| a[i].min(b[i]) <= p[i] && p[i] <= a[i].max(b[i]))
}
fn intersects(a: P, b: P, c: P, d: P) -> Result<bool, Error> {
    let [s, t, u, v] = [
        sign(a, b, c)?,
        sign(a, b, d)?,
        sign(c, d, a)?,
        sign(c, d, b)?,
    ];
    Ok((s == Ordering::Equal && between(a, b, c))
        || (t == Ordering::Equal && between(a, b, d))
        || (u == Ordering::Equal && between(c, d, a))
        || (v == Ordering::Equal && between(c, d, b))
        || (s != Ordering::Equal
            && t != Ordering::Equal
            && u != Ordering::Equal
            && v != Ordering::Equal
            && s != t
            && u != v))
}
fn inside(p: P, polygon: &[P]) -> Result<bool, Error> {
    let mut winding = 0i64;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let s = sign(a, b, p)?;
        if a[1] <= p[1] && b[1] > p[1] && s == Ordering::Greater {
            winding += 1;
        }
        if a[1] > p[1] && b[1] <= p[1] && s == Ordering::Less {
            winding -= 1;
        }
    }
    Ok(winding != 0)
}
fn classify(loops: &mut [PlanarLoop], max_checks: usize) -> Result<(), Error> {
    let mut checks = 0usize;
    let mut charge = || {
        checks = checks
            .checked_add(1)
            .filter(|n| *n <= max_checks)
            .ok_or_else(|| fail(ErrorKind::Limit, "planar trim comparison budget exceeded"))?;
        Ok::<(), Error>(())
    };
    for (li, l) in loops.iter().enumerate() {
        let n = l.points.len();
        if n < 3 {
            return Err(fail(
                ErrorKind::Geometry,
                "trim polygon needs at least three points",
            ));
        }
        for i in 0..n {
            let a = l.points[i];
            let b = l.points[(i + 1) % n];
            let c = l.points[(i + 2) % n];
            if a == b {
                return Err(fail(
                    ErrorKind::Geometry,
                    "zero-length sampled trim segment",
                ));
            }
            if sign(a, b, c)? == Ordering::Equal && (between(a, b, c) || between(b, c, a)) {
                return Err(fail(
                    ErrorKind::Geometry,
                    "adjacent sampled trim segments overlap",
                ));
            }
            for j in i + 1..n {
                if j == i + 1 || (i == 0 && j == n - 1) {
                    continue;
                }
                charge()?;
                if intersects(a, b, l.points[j], l.points[(j + 1) % n])? {
                    return Err(fail(
                        ErrorKind::Geometry,
                        "sampled trim polygon self-intersects or touches",
                    ));
                }
            }
            for other in loops.iter().skip(li + 1) {
                for j in 0..other.points.len() {
                    charge()?;
                    if intersects(
                        a,
                        b,
                        other.points[j],
                        other.points[(j + 1) % other.points.len()],
                    )? {
                        return Err(fail(
                            ErrorKind::Geometry,
                            "sampled trim polygons intersect or touch",
                        ));
                    }
                }
            }
        }
    }
    let mut depths = Vec::new();
    for (i, l) in loops.iter().enumerate() {
        let mut depth = 0;
        for (j, other) in loops.iter().enumerate() {
            if i != j {
                // Bound containment work as well as intersection comparisons.
                for _ in &other.points {
                    charge()?;
                }
                if inside(l.points[0], &other.points)? {
                    depth += 1;
                }
            }
        }
        depths.push(depth);
    }
    if depths.iter().filter(|d| **d == 0).count() != 1 || depths.iter().any(|d| *d > 1) {
        return Err(fail(
            ErrorKind::Geometry,
            "require one outer trim and unnested holes",
        ));
    }
    for (l, depth) in loops.iter_mut().zip(depths) {
        l.is_outer = depth == 0;
    }
    Ok(())
}
impl AnalyticModel {
    /// Classify polygons sampled from shared edges on one planar face. This
    /// preserves source ordering and rejects ambiguous/touching approximations;
    /// it does not certify intersections between the original exact curves.
    /// `max_checks` bounds projected-point expansion and pair/containment work.
    pub fn sampled_planar_trims(
        &self,
        face_id: u64,
        max_checks: usize,
    ) -> Result<PlanarTrims, Error> {
        let face = self
            .body()
            .faces
            .get(&face_id)
            .ok_or_else(|| fail(ErrorKind::Geometry, "unknown face"))?;
        let surface = self.surface(face_id).unwrap();
        if !matches!(surface, Surface::Plane(_)) {
            return Err(fail(
                ErrorKind::Geometry,
                "periodic/nonplanar trim charts are not implemented",
            ));
        }
        let mut loops = Vec::new();
        let mut remaining = max_checks;
        for bound in &face.bounds {
            let mut points = Vec::new();
            for usage in &bound.edges {
                let samples = self.edge_points(usage.edge).unwrap();
                remaining = remaining.checked_sub(samples.len() - 1).ok_or_else(|| {
                    fail(ErrorKind::Limit, "planar trim expansion budget exceeded")
                })?;
                // Each edge's terminal point belongs to the next edge. A closed
                // circle likewise contributes its start once, not twice.
                for i in 0..samples.len() - 1 {
                    let p = &samples[if usage.forward {
                        i
                    } else {
                        samples.len() - 1 - i
                    }];
                    points.push(
                        surface
                            .project(*p)
                            .ok_or_else(|| {
                                fail(ErrorKind::Geometry, "unresolved planar projection")
                            })?
                            .chart,
                    );
                }
            }
            if !bound.forward {
                points.reverse();
            }
            loops.push(PlanarLoop {
                bound_id: bound.id,
                points,
                is_outer: false,
            });
        }
        classify(&mut loops, remaining)?;
        for (l, b) in loops.iter().zip(&face.bounds) {
            if b.explicitly_outer && !l.is_outer {
                return Err(fail(
                    ErrorKind::Geometry,
                    "declared outer bound is geometrically nested",
                ));
            }
        }
        Ok(PlanarTrims { face_id, loops })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn polygon(points: &[[f64; 2]]) -> PlanarLoop {
        PlanarLoop {
            bound_id: 0,
            points: points.to_vec(),
            is_outer: false,
        }
    }
    #[test]
    fn order_and_winding_do_not_determine_trim_roles() {
        let mut loops = vec![
            polygon(&[[1., 1.], [2., 1.], [2., 2.], [1., 2.]]),
            polygon(&[[0., 0.], [0., 3.], [3., 3.], [3., 0.]]),
        ];
        classify(&mut loops, 1000).unwrap();
        assert!(!loops[0].is_outer);
        assert!(loops[1].is_outer);
    }
    #[test]
    fn crossings_touching_overlaps_and_disconnected_outers_are_refused() {
        for points in [
            vec![[0., 0.], [2., 2.], [0., 2.], [2., 0.]],
            vec![[0., 0.], [2., 0.], [1., 0.], [0., 2.]],
            vec![[0., 0.], [1., 0.], [0., 0.], [0., 1.]],
        ] {
            assert!(classify(&mut [polygon(&points)], 1000).is_err());
        }
        let square = polygon(&[[0., 0.], [2., 0.], [2., 2.], [0., 2.]]);
        let touching = polygon(&[[0., 1.], [1., 1.], [1., 1.5]]);
        assert!(classify(&mut [square.clone(), touching], 1000).is_err());
        let separate = polygon(&[[3., 0.], [4., 0.], [4., 1.], [3., 1.]]);
        assert!(classify(&mut [square.clone(), separate], 1000).is_err());
        assert!(matches!(
            classify(&mut [square], 0).unwrap_err().kind,
            ErrorKind::Limit
        ));
    }
}
