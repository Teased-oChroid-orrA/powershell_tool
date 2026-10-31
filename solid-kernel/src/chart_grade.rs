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
            // Each split segment is bisected on the edge's own curve (by parameter: arc angle, line or spline parameter), not
            // split where the chart put its point. That point can lie arbitrarily near an end of the segment; repeated rounds
            // then left samples nanometres apart, below the edge-to-face projection residual (a spline edge of ABC `00006600`
            // refused as overlapping trim segments, and its circle edges carried about 57,000 such gaps that made the shell
            // unmeshable). Bisection keeps every new sample at least a power-of-two part of its source segment.
            let mut wanted: BTreeMap<u64, Vec<(usize, f64)>> = BTreeMap::new();
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
                    wanted.entry(s.edge).or_default().push((s.segment, 0.5));
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

    /// Grading bisects every split segment (Phases 156-157). A chart-requested fraction near an end of a segment had left
    /// samples of ABC `00006600` nanometres apart (a spline edge refused, its circle edges carried about 57,000 such gaps).
    /// Every graded interval, measured in the edge's own parameter, must be a dyadic part of one source interval at a dyadic
    /// offset.
    #[test]
    fn grading_bisects_every_split_segment() {
        let data = inventory(include_str!("../tests/data/cross-bore-filleted.step"), Limits::default()).unwrap();
        let model = AnalyticModel::read(&data, Options { geometry_tolerance_mm: 1e-4, ..Options::default() }).unwrap();
        let (graded, _) = model
            .graded(GradeOptions { policy: ChartPolicy::CanonicalizeWithinTolerance, ..GradeOptions::default() }, 50_000_000)
            .unwrap();
        let mut split_edges = 0;
        for &edge in model.body().edges.keys() {
            let (source, after) = (model.edge_sample_parameters(edge), graded.edge_sample_parameters(edge));
            if after.len() > source.len() {
                split_edges += 1;
            }
            for w in after.windows(2) {
                let k = source.partition_point(|&u| u <= w[0] + 1e-12).clamp(1, source.len() - 1);
                let (a, b) = (source[k - 1], source[k]);
                let (size, offset) = ((w[1] - w[0]) / (b - a), (w[0] - a) / (w[1] - w[0]));
                let depth = -size.log2();
                assert!(
                    w[1] <= b + 1e-9 * (b - a)
                        && (size * 2f64.powi(depth.round() as i32) - 1.).abs() < 1e-6
                        && (offset - offset.round()).abs() < 1e-6,
                    "edge #{edge}: [{}, {}] is not a bisection of [{a}, {b}]",
                    w[0],
                    w[1]
                );
            }
        }
        assert!(split_edges > 0, "grading split no edge");
    }
}
