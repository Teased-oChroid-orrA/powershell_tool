//! Bounded single-loop cylindrical charts. Holes, winding bands without explicit
//! seams and exact curved-trim intersection certification remain unsupported.
use crate::{
    analytic_step::{AnalyticModel, Surface},
    planar_step::{classify, TrimLoop},
    step::{Error, ErrorKind},
};
use std::f64::consts::TAU;
#[derive(Debug, Clone)]
pub struct CylindricalTrims {
    pub face_id: u64,
    /// Physical (circumferential arc length, axial distance), in millimetres.
    /// Opposite sides of a seam may represent identical world points.
    pub boundary: TrimLoop,
    pub circumference_mm: f64,
}
fn fail(kind: ErrorKind, text: &str) -> Error {
    Error {
        offset: 0,
        kind,
        detail: text.into(),
    }
}
fn unwrap_u(raw: f64, previous: f64, period: f64) -> Result<f64, Error> {
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
        if face.bounds.len() != 1 {
            return Err(fail(
                ErrorKind::Geometry,
                "cylindrical holes/unseamed bands are not implemented",
            ));
        }
        let bound = &face.bounds[0];
        let mut remaining = max_work;
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
                let p = samples[if usage.forward {
                    i
                } else {
                    samples.len() - 1 - i
                }];
                let mut chart = surface
                    .project(p)
                    .ok_or_else(|| fail(ErrorKind::Geometry, "unresolved cylindrical projection"))?
                    .chart;
                if let Some(last) = points.last() {
                    chart[0] = unwrap_u(chart[0], last[0], period)?;
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
        let closing = unwrap_u(first, points.last().unwrap()[0], period)?;
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
        let mut loops = [TrimLoop {
            bound_id: bound.id,
            points,
            is_outer: false,
        }];
        classify(&mut loops, remaining)?;
        Ok(CylindricalTrims {
            face_id,
            boundary: loops.into_iter().next().unwrap(),
            circumference_mm: period,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn periodic_unwrap_is_continuous_and_refuses_ambiguous_half_turns() {
        assert!((unwrap_u(-3., 3., TAU).unwrap() - (TAU - 3.)).abs() < 1e-14);
        assert!((unwrap_u(3., -3., TAU).unwrap() - (3. - TAU)).abs() < 1e-14);
        assert!(unwrap_u(std::f64::consts::PI, 0., TAU).is_err());
    }
}
