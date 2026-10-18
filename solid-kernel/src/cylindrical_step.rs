//! Bounded single-loop cylindrical charts. Holes, winding bands without explicit
//! seams and exact curved-trim intersection certification remain unsupported.
use crate::{
    analytic_step::{AnalyticModel, Surface},
    planar_step::{classify, TrimLoop},
    step::{Error, ErrorKind},
};
use std::collections::BTreeMap;
use std::f64::consts::TAU;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartPolicy {
    #[default]
    Strict,
    CanonicalizeWithinTolerance,
}
#[derive(Debug, Clone)]
pub struct CylindricalTrims {
    pub face_id: u64,
    /// Physical (circumferential arc length, axial distance), in millimetres.
    /// Opposite sides of a seam may represent identical world points.
    pub boundary: TrimLoop,
    /// Every bound of the face in source order (the outer one and the holes), `is_outer` set.
    pub loops: Vec<TrimLoop>,
    pub circumference_mm: f64,
    pub adjustments: Vec<ChartAdjustment>,
}
/// Explicit parameter-only adjustment. Authoritative world boundary samples
/// are unchanged; both parameter and surface-projection shifts are bounded.
#[derive(Debug, Clone)]
pub struct ChartAdjustment {
    pub face: u64,
    pub edge: u64,
    pub edge_use: u64,
    pub bound: u64,
    pub sample: usize,
    pub before: [f64; 2],
    pub after: [f64; 2],
    pub world_deviation_mm: f64,
}
fn fail(kind: ErrorKind, text: &str) -> Error {
    Error {
        offset: 0,
        kind,
        detail: text.into(),
    }
}
pub(crate) fn unwrap_period(raw: f64, previous: f64, period: f64) -> Result<f64, Error> {
    // Reconstruct from the raw parameter plus an integer period. Adding
    // incremental remainders would accumulate drift around a long boundary.
    let u = raw + ((previous - raw) / period).round() * period;
    if (u - previous).abs() == period / 2. {
        return Err(fail(
            ErrorKind::Geometry,
            "ambiguous half-turn cylindrical trim segment",
        ));
    }
    if !u.is_finite() {
        return Err(fail(ErrorKind::Geometry, "unresolved cylindrical chart"));
    }
    Ok(u)
}
impl AnalyticModel {
    /// Unwrap a single explicitly seamed, contractible boundary. The complete
    /// sampled boundary must close in one chart, not merely modulo a turn.
    /// This checks polygonal approximations, not exact curved intersections.
    pub fn sampled_cylindrical_trims(
        &self,
        face_id: u64,
        max_work: usize,
    ) -> Result<CylindricalTrims, Error> {
        self.cylindrical_trims_impl(face_id, max_work, false)
    }
    /// Explicitly canonicalize analytic chart invariants within the existing
    /// geometry tolerance, returning every changed sample. No topology/world
    /// coordinate is changed and strict import remains the default.
    pub fn canonicalized_cylindrical_trims(
        &self,
        face_id: u64,
        max_work: usize,
    ) -> Result<CylindricalTrims, Error> {
        self.cylindrical_trims_impl(face_id, max_work, true)
    }
    fn cylindrical_trims_impl(
        &self,
        face_id: u64,
        max_work: usize,
        canonicalize: bool,
    ) -> Result<CylindricalTrims, Error> {
        let face = self
            .body()
            .faces
            .get(&face_id)
            .ok_or_else(|| fail(ErrorKind::Geometry, "unknown face"))?;
        let surface = self.surface(face_id).unwrap();
        let period = match surface {
            Surface::Cylinder { radius, .. } => radius * TAU,
            _ => return Err(fail(ErrorKind::Geometry, "require cylindrical surface")),
        };
        if face.bounds.is_empty() {
            return Err(fail(ErrorKind::Geometry, "a face without bounds"));
        }
        let mut adjustments = Vec::new();
        let mut remaining = max_work;
        let mut unwrapped: Vec<Vec<[f64; 2]>> = Vec::with_capacity(face.bounds.len());
        for bound in &face.bounds {
            let anchor_work = if canonicalize {
                bound
                    .edges
                    .len()
                    .checked_mul(2)
                    .ok_or_else(|| fail(ErrorKind::Limit, "chart anchor work overflow"))?
            } else {
                0
            };
            remaining = remaining
                .checked_sub(anchor_work)
                .ok_or_else(|| fail(ErrorKind::Limit, "chart anchor budget exceeded"))?;
            let mut anchors = BTreeMap::<u64, [Option<f64>; 2]>::new();
            if canonicalize {
                for usage in &bound.edges {
                    let edge = &self.body().edges[&usage.edge];
                    let fixed = self
                        .cylindrical_edge_constraint(face_id, edge.id)
                        .unwrap_or([None, None]);
                    for vertex in [edge.start, edge.end] {
                        let anchor = anchors.entry(vertex).or_insert([None, None]);
                        for axis in 0..2 {
                            if let Some(mut value) = fixed[axis] {
                                if let Some(old) = anchor[axis] {
                                    if axis == 0 {
                                        value = unwrap_period(value, old, period)?;
                                    }
                                    if (value - old).abs() > self.geometry_tolerance_mm() {
                                        return Err(fail(
                                            ErrorKind::Geometry,
                                            "ambiguous cylindrical chart anchors",
                                        ));
                                    }
                                } else {
                                    anchor[axis] = Some(value);
                                }
                            }
                        }
                    }
                }
            }
            let mut points: Vec<[f64; 2]> = Vec::new();
            for usage in &bound.edges {
                let samples = self.edge_points(usage.edge).unwrap();
                remaining = remaining.checked_sub(samples.len() - 1).ok_or_else(|| {
                    fail(
                        ErrorKind::Limit,
                        "cylindrical trim expansion budget exceeded",
                    )
                })?;
                for i in 0..samples.len() - 1 {
                    let sample = if usage.forward {
                        i
                    } else {
                        samples.len() - 1 - i
                    };
                    let p = samples[sample];
                    let mut chart = surface
                        .project(p)
                        .ok_or_else(|| {
                            fail(ErrorKind::Geometry, "unresolved cylindrical projection")
                        })?
                        .chart;
                    if canonicalize {
                        let before = chart;
                        let fixed = self
                            .cylindrical_edge_constraint(face_id, usage.edge)
                            .unwrap_or([None, None]);
                        for axis in 0..2 {
                            if let Some(value) = fixed[axis] {
                                chart[axis] = value;
                            }
                        }
                        let edge = &self.body().edges[&usage.edge];
                        let vertex = if sample == 0 {
                            Some(edge.start)
                        } else if sample == samples.len() - 1 {
                            Some(edge.end)
                        } else {
                            None
                        };
                        if let Some(vertex) = vertex {
                            for axis in 0..2 {
                                if let Some(value) = anchors[&vertex][axis] {
                                    chart[axis] = value;
                                }
                            }
                        }
                        chart[0] = unwrap_period(chart[0], before[0], period)?;
                        let projected = surface.point(chart).ok_or_else(|| {
                            fail(ErrorKind::Geometry, "unresolved canonical chart projection")
                        })?;
                        let world_deviation_mm = crate::norm(crate::sub(projected, p));
                        let parameter_shift = (chart[0] - before[0]).hypot(chart[1] - before[1]);
                        if !world_deviation_mm.is_finite()
                            || world_deviation_mm > self.geometry_tolerance_mm()
                            || parameter_shift > self.geometry_tolerance_mm()
                        {
                            return Err(fail(
                                ErrorKind::Geometry,
                                "chart canonicalization exceeds the geometry tolerance",
                            ));
                        }
                        if chart != before {
                            adjustments.push(ChartAdjustment {
                                face: face_id,
                                edge: usage.edge,
                                edge_use: usage.id,
                                bound: bound.id,
                                sample,
                                before,
                                after: chart,
                                world_deviation_mm,
                            });
                        }
                    }
                    if let Some(last) = points.last() {
                        chart[0] = unwrap_period(chart[0], last[0], period)?;
                    }
                    points.push(chart);
                }
            }
            if points.len() < 3 {
                return Err(fail(
                    ErrorKind::Geometry,
                    "insufficient cylindrical trim samples",
                ));
            }
            let first = points[0][0];
            let closing = unwrap_period(first, points.last().unwrap()[0], period)?;
            // Arithmetic-only closure bound, independent of the geometry chord
            // tolerance. No multi-turn or noncontractible loop is closed by fiat.
            let roundoff = 64. * f64::EPSILON * period;
            if (closing - first).abs() > roundoff {
                return Err(fail(
                    ErrorKind::Geometry,
                    "cylindrical boundary winds without a chart-closing seam",
                ));
            }
            let min = points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
            let max = points
                .iter()
                .map(|p| p[0])
                .fold(f64::NEG_INFINITY, f64::max);
            if max - min > period + roundoff {
                return Err(fail(
                    ErrorKind::Geometry,
                    &format!("cannot certify a single-turn cylindrical chart: width={:e}, period={:e}, roundoff={:e}", max-min, period, roundoff),
                ));
            }
            if !bound.forward {
                points.reverse();
            }
            unwrapped.push(points);
        }
        // The outer loop is the one of largest area; every other loop is a hole, shifted by whole periods until its first
        // point lies inside the outer loop (a hole is contractible, so it sits in the chart exactly once).
        let area = |p: &[[f64; 2]]| -> f64 {
            (0..p.len())
                .map(|i| {
                    let (a, b) = (p[i], p[(i + 1) % p.len()]);
                    a[0] * b[1] - b[0] * a[1]
                })
                .sum::<f64>()
                .abs()
        };
        let outer = (0..unwrapped.len())
            .max_by(|&i, &j| area(&unwrapped[i]).total_cmp(&area(&unwrapped[j])))
            .unwrap();
        if unwrapped.len() > 1 {
            let outer_loop = unwrapped[outer].clone();
            for (i, hole) in unwrapped.iter_mut().enumerate() {
                if i == outer {
                    continue;
                }
                let mut placed = false;
                for shift in [0., period, -period, 2. * period, -2. * period] {
                    let moved: Vec<[f64; 2]> = hole.iter().map(|p| [p[0] + shift, p[1]]).collect();
                    let mut trial = [
                        TrimLoop {
                            bound_id: 0,
                            points: outer_loop.clone(),
                            is_outer: false,
                        },
                        TrimLoop {
                            bound_id: 1,
                            points: moved.clone(),
                            is_outer: false,
                        },
                    ];
                    if classify(&mut trial, remaining).is_ok()
                        && trial[0].is_outer
                        && !trial[1].is_outer
                    {
                        *hole = moved;
                        placed = true;
                        break;
                    }
                }
                if !placed {
                    return Err(fail(
                        ErrorKind::Geometry,
                        "a cylindrical hole loop cannot be placed inside the face's outer loop",
                    ));
                }
            }
        }
        let mut loops: Vec<TrimLoop> = face
            .bounds
            .iter()
            .zip(unwrapped)
            .map(|(bound, points)| TrimLoop {
                bound_id: bound.id,
                points,
                is_outer: false,
            })
            .collect();
        classify(&mut loops, remaining)?;
        let boundary = loops.iter().find(|l| l.is_outer).unwrap().clone();
        Ok(CylindricalTrims {
            face_id,
            boundary,
            loops,
            circumference_mm: period,
            adjustments,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn periodic_unwrap_is_continuous_and_refuses_ambiguous_half_turns() {
        assert!((unwrap_period(-3., 3., TAU).unwrap() - (TAU - 3.)).abs() < 1e-14);
        assert!((unwrap_period(3., -3., TAU).unwrap() - (3. - TAU)).abs() < 1e-14);
        assert!(unwrap_period(std::f64::consts::PI, 0., TAU).is_err());
    }
}
