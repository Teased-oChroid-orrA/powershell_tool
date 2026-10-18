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
                    wanted.entry(s.edge).or_default().push((s.segment, s.t));
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
                report.converged = true;
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
