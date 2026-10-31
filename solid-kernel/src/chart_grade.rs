//! Graded boundary sampling of a model: every face chart's Delaunay refinement may want to split its boundary segments, and
//! those segments are shared with the neighbouring faces. This module runs the refinement face by face, collects the
//! wanted splits per shared edge, inserts them on the edge's own curve, and repeats until no face asks for more (or a bound
//! is reached). Afterwards each face triangulates with the refined boundary, so the surface mesh is graded across faces
//! instead of forcing needles where a long edge meets a small feature.
use crate::{
    analytic_step::AnalyticModel,
    chart_mesh::BoundarySplit,
    cylindrical_step::ChartPolicy,
    step::{Error, ErrorKind},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradeOptions {
    /// Target smallest chart angle of the refinement.
    pub min_angle_deg: f64,
    /// Boundary segments shorter than twice this (mm) are never split: it bounds the grading around tiny features.
    pub floor_mm: f64,
    pub max_rounds: usize,
    /// Refinement points per face and round.
    pub max_points_per_face: usize,
    pub policy: ChartPolicy,
    /// Total edge samples added over all rounds.
    pub max_added_samples: usize,
}
impl Default for GradeOptions {
    fn default() -> Self {
        Self {
            min_angle_deg: 25.,
            floor_mm: 0.1,
            max_rounds: 40,
            max_points_per_face: 4000,
            policy: ChartPolicy::Strict,
            max_added_samples: 200_000,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct GradeReport {
    pub rounds: usize,
    pub samples_added: usize,
    /// Whether the last round asked for no further split.
    pub converged: bool,
}
impl AnalyticModel {
    /// A copy of the model whose shared edges carry the extra samples the face charts' refinement asks for.
    pub fn graded(
        &self,
        options: GradeOptions,
        max_work: usize,
    ) -> Result<(AnalyticModel, GradeReport), Error> {
        let mut model = self.clone();
        let mut report = GradeReport {
            rounds: 0,
            samples_added: 0,
            converged: false,
        };
        let faces: Vec<u64> = model.body().faces.keys().copied().collect();
        for _ in 0..options.max_rounds {
            // Splits keep the chart's fractions: its concentric-shell splits grade toward small features, below the floor
            // when a sliver face needs it. But a new sample nearer than the model's geometry tolerance to another sample of
            // its segment is dropped: the edge and its faces only agree to that tolerance, so such samples carry no geometry
            // and their face-chart images can swap order. Faces sharing a segment asked for nearly equal fractions, and
            // repeated splits near one end did the same; on ABC `00006600` that left a spline edge refused as overlapping
            // trim segments (Phase 156) and about 57,000 sub-10 nm gaps on circle edges (Phase 157).
            let mut wanted: BTreeMap<u64, BTreeMap<usize, Vec<(f64, f64)>>> = BTreeMap::new();
            for &face in &faces {
                let splits: Vec<BoundarySplit> = model.boundary_splits(
                    face,
                    max_work,
                    options.policy,
                    options.min_angle_deg,
                    options.floor_mm,
                    options.max_points_per_face,
                )?;
                for s in splits {
                    // A spline's parameter is not proportional to the chart: its interval is bisected (Phase 156).
                    let t = if model.edge_is_spline(s.edge) { 0.5 } else { s.t };
                    wanted.entry(s.edge).or_default().entry(s.segment).or_default().push((t, s.spacing));
                }
            }
            if wanted.is_empty() {
                report.converged = true;
                break;
            }
            let before: usize = model
                .body()
                .edges
                .keys()
                .map(|e| model.edge_points(*e).unwrap().len())
                .sum();
            for (edge, splits) in wanted {
                let mut kept = Vec::new();
                for (segment, mut ts) in splits {
                    ts.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let mut last = 0.;
                    for (t, spacing) in ts {
                        if t - last >= spacing && 1. - t >= spacing {
                            kept.push((segment, t));
                            last = t;
                        }
                    }
                }
                let splits = kept;
                model.insert_edge_splits(edge, &splits)?;
            }
            let after: usize = model
                .body()
                .edges
                .keys()
                .map(|e| model.edge_points(*e).unwrap().len())
                .sum();
            report.rounds += 1;
            report.samples_added += after - before;
            if after == before {
                break;
            }
            if report.samples_added > options.max_added_samples {
                return Err(Error {
                    offset: 0,
                    kind: ErrorKind::Limit,
                    detail: "graded sampling exceeded its added-sample budget".into(),
                });
            }
        }
        Ok((model, report))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytic_step::Options;
    use crate::step::{inventory, Limits};

    /// Grading never puts a new edge sample nearer than the model's geometry tolerance to another sample of its edge
    /// (Phase 157). Without that rule, faces sharing a segment and repeated splits near one end left samples of ABC
    /// `00006600` nanometres apart.
    #[test]
    fn grading_keeps_new_samples_a_geometry_tolerance_apart() {
        let mut graded_edges = 0;
        for (name, source) in [
            ("cross-bore-filleted", include_str!("../tests/data/cross-bore-filleted.step")),
            ("t-junction-filleted", include_str!("../tests/data/t-junction-filleted.step")),
            ("tilted-hole-filleted", include_str!("../tests/data/tilted-hole-filleted.step")),
            ("torus", include_str!("../tests/data/torus.step")),
        ] {
            let data = inventory(source, Limits::default()).unwrap();
            let model = AnalyticModel::read(&data, Options::foreign_cad(0.05)).unwrap();
            let tolerance = model.geometry_tolerance_mm();
            let (graded, _) = model
                .graded(GradeOptions { policy: ChartPolicy::CanonicalizeWithinTolerance, ..GradeOptions::default() }, 50_000_000)
                .unwrap();
            for &edge in model.body().edges.keys() {
                let (source, after) = (model.edge_points(edge).unwrap(), graded.edge_points(edge).unwrap());
                if after.len() > source.len() {
                    graded_edges += 1;
                }
                for w in after.windows(2) {
                    let original = source.windows(2).any(|s| s[0] == w[0] && s[1] == w[1]);
                    let gap = (0..3).map(|k| (w[1][k] - w[0][k]).powi(2)).sum::<f64>().sqrt();
                    assert!(original || gap >= tolerance, "{name} edge #{edge}: new samples {gap:e} mm apart (tolerance {tolerance})");
                }
            }
        }
        assert!(graded_edges > 0, "grading split no edge");
    }
}
