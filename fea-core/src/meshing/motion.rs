//! Mesh motion: relocate the nodes of a mesh without changing its connectivity, so that prescribed boundary displacements are
//! followed by the interior.
//!
//! The interior displacement solves a pseudo-elastic problem on the mesh itself: every element becomes its own block with a
//! Young's modulus proportional to `(mean size / element size)^exponent`, so small elements are stiff and take little of the
//! motion (the standard remedy for the collapse of the elements next to a moving wall); the prescribed nodes carry the given
//! displacement, every other node is free. The motion is applied in sub-steps. After each one the mesh is checked
//! (inversion, scaled Jacobian floor, edge-ratio ceiling): a step that fails is rolled back and retried with half the
//! increment; when the increment falls below `min_increment` the motion stops at the last valid mesh and reports
//! [`Outcome::NeedsRemesh`], the hand-over to local remeshing. Nothing here is an ALE formulation (no mesh velocity, no
//! convective terms): it is the geometric motion subsystem such a formulation would use.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use super::{
    quality::{QualityReport, QualityThresholds},
    MeshError,
};
use crate::{
    linear::Dirichlet,
    loads::Loads,
    mesh::{Block, Elastic, Mesh},
    Model,
};

#[derive(Debug, Clone, Copy)]
pub struct MotionOptions {
    /// Number of sub-steps of the first attempt.
    pub steps: usize,
    /// Element stiffness `~ size^-exponent` relative to the mean size (`0` = uniform, plain pseudo-solid).
    pub stiffening_exponent: f64,
    /// Floors and ceilings every intermediate and the final mesh must satisfy.
    pub thresholds: QualityThresholds,
    /// Smallest fraction of the total motion a retried step may be shrunk to before the motion gives up.
    pub min_increment: f64,
    pub max_retries: usize,
}

impl Default for MotionOptions {
    fn default() -> Self {
        Self {
            steps: 4,
            stiffening_exponent: 2.0,
            thresholds: QualityThresholds {
                min_scaled_jacobian: 0.05,
                max_edge_ratio: 100.0,
                max_size_jump: f64::INFINITY,
            },
            min_increment: 1e-4,
            max_retries: 40,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The full prescribed motion was applied.
    Complete,
    /// The motion stopped at the last valid mesh: the elements cannot follow the rest of it by node relocation alone.
    NeedsRemesh {
        applied_fraction: f64,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct MotionReport {
    pub outcome: Outcome,
    /// Fraction of the prescribed displacement applied (`1` when complete).
    pub applied_fraction: f64,
    pub steps_taken: usize,
    pub rollbacks: usize,
    pub min_scaled_jacobian_before: f64,
    pub min_scaled_jacobian_after: f64,
    pub max_edge_ratio_after: f64,
    /// Largest displacement of any node.
    pub max_node_displacement: f64,
    /// Equilibrium of the pseudo-elastic solves that were kept: the largest reaction at a free dof relative to the largest
    /// reaction at a prescribed dof (round-off level for a sound solve).
    pub worst_free_reaction_ratio: f64,
}

/// A node with a prescribed total displacement; the coordinates of `displacement` beyond the mesh dimension are ignored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prescribed {
    pub node: usize,
    pub displacement: [f64; 3],
}

fn pseudo_mesh(mesh: &Mesh, exponent: f64) -> Mesh {
    let quality = mesh.quality();
    let mean =
        quality.elements.iter().map(|q| q.size).sum::<f64>() / quality.elements.len().max(1) as f64;
    let mut out = Mesh::new(mesh.physics);
    out.nodes = mesh.nodes.clone();
    let mut k = 0;
    for b in &mesh.blocks {
        let nn = b.kind.n_nodes();
        for e in 0..b.n_elems() {
            let size = quality.elements[k].size;
            k += 1;
            let ratio = if size > 0. && mean > 0. {
                (mean / size).powf(exponent)
            } else {
                1.
            };
            out.blocks.push(Block {
                kind: b.kind,
                conn: b.conn[e * nn..(e + 1) * nn].to_vec(),
                material: Elastic::new(ratio.clamp(1e-6, 1e6), 0.3),
                name: String::new(),
                plasticity: None,
                density: 0.,
                thermal: None,
            });
        }
    }
    out
}

fn judge(q: &QualityReport, t: &QualityThresholds) -> Result<(), String> {
    if q.inverted > 0 {
        return Err(format!("{} inverted element(s)", q.inverted));
    }
    if q.min_scaled_jacobian < t.min_scaled_jacobian {
        return Err(format!(
            "scaled Jacobian {:.4} below the floor {:.4}",
            q.min_scaled_jacobian, t.min_scaled_jacobian
        ));
    }
    if q.max_edge_ratio > t.max_edge_ratio {
        return Err(format!(
            "edge ratio {:.1} above the ceiling {:.1}",
            q.max_edge_ratio, t.max_edge_ratio
        ));
    }
    Ok(())
}

/// Move `mesh` so that each prescribed node reaches `position + displacement`. Returns the moved mesh (the last valid one when
/// the motion could not be completed) and the report. The input mesh must itself pass the thresholds.
pub fn move_mesh(
    mesh: &Mesh,
    prescribed: &[Prescribed],
    options: MotionOptions,
) -> Result<(Mesh, MotionReport), MeshError> {
    if options.steps == 0 || options.min_increment <= 0. {
        return Err(MeshError::Failed(
            "motion needs at least one step and a positive minimum increment".into(),
        ));
    }
    if prescribed
        .iter()
        .any(|p| p.node >= mesh.nodes.len() || p.displacement.iter().any(|v| !v.is_finite()))
    {
        return Err(MeshError::Failed(
            "prescribed node out of range or displacement not finite".into(),
        ));
    }
    if prescribed.is_empty() {
        return Err(MeshError::Failed(
            "no prescribed node: the interior would move rigidly".into(),
        ));
    }
    let start = mesh.quality();
    judge(&start, &options.thresholds)
        .map_err(|e| MeshError::Failed(format!("the mesh to move is already unacceptable: {e}")))?;
    let d = mesh.dim();
    let mut current = mesh.clone();
    let mut report = MotionReport {
        outcome: Outcome::Complete,
        applied_fraction: 0.,
        steps_taken: 0,
        rollbacks: 0,
        min_scaled_jacobian_before: start.min_scaled_jacobian,
        min_scaled_jacobian_after: start.min_scaled_jacobian,
        max_edge_ratio_after: start.max_edge_ratio,
        max_node_displacement: 0.,
        worst_free_reaction_ratio: 0.,
    };
    let mut increment = 1.0 / options.steps as f64;
    let mut retries = 0;
    while report.applied_fraction < 1. - 1e-12 {
        let step = increment.min(1. - report.applied_fraction);
        // Remaining displacement of the prescribed nodes, scaled to this step: the prescribed targets are fixed, so a step is
        // the share `step / (1 - applied)` of what is left.
        let share = step / (1. - report.applied_fraction);
        let pseudo = pseudo_mesh(&current, options.stiffening_exponent);
        let model = Model::new(pseudo).map_err(MeshError::Failed)?;
        let mut bc: Dirichlet = model.dirichlet();
        for p in prescribed {
            // `p.displacement` is the total; what is left of it is `(1 - applied)` of that.
            for c in 0..d {
                bc.fix(
                    p.node,
                    c,
                    p.displacement[c] * (1. - report.applied_fraction) * share,
                );
            }
        }
        let solution = model
            .solve_static(&Loads::default(), &bc)
            .map_err(MeshError::Failed)?;
        let mut trial = current.clone();
        for (n, x) in trial.nodes.iter_mut().enumerate() {
            for c in 0..d {
                x[c] += solution.u[n * d + c];
            }
        }
        let q = trial.quality();
        match judge(&q, &options.thresholds) {
            Ok(()) => {
                report.max_node_displacement = trial
                    .nodes
                    .iter()
                    .zip(&mesh.nodes)
                    .map(|(a, b)| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt())
                    .fold(0., f64::max);
                let (mut fixed_max, mut free_max) = (0f64, 0f64);
                for (i, r) in solution.reactions.iter().enumerate() {
                    if bc.fixed[i] {
                        fixed_max = fixed_max.max(r.abs());
                    } else {
                        free_max = free_max.max(r.abs());
                    }
                }
                report.worst_free_reaction_ratio = report
                    .worst_free_reaction_ratio
                    .max(free_max / fixed_max.max(1e-300));
                report.min_scaled_jacobian_after = q.min_scaled_jacobian;
                report.max_edge_ratio_after = q.max_edge_ratio;
                report.applied_fraction += step;
                report.steps_taken += 1;
                current = trial;
                retries = 0;
                // Recover the increment after a success that followed a shrink.
                increment = (increment * 2.).min(1.0 / options.steps as f64);
            }
            Err(reason) => {
                report.rollbacks += 1;
                retries += 1;
                increment = step / 2.;
                if increment < options.min_increment || retries > options.max_retries {
                    report.outcome = Outcome::NeedsRemesh {
                        applied_fraction: report.applied_fraction,
                        reason,
                    };
                    break;
                }
            }
        }
    }
    Ok((current, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{element::ElementKind, generate::grid, mesh::Physics};

    fn column(n: usize) -> Mesh {
        grid(
            Physics::Solid,
            ElementKind::Hex8,
            Elastic::new(1., 0.3),
            [n, n, n],
            &|p| p,
        )
        .expect("box mesh")
    }

    fn face_nodes(mesh: &Mesh, axis: usize, value: f64) -> Vec<usize> {
        (0..mesh.nodes.len())
            .filter(|&i| (mesh.nodes[i][axis] - value).abs() < 1e-12)
            .collect()
    }

    #[test]
    fn a_wall_moved_by_a_third_of_the_box_is_followed_by_the_interior() {
        let mesh = column(4);
        let mut prescribed: Vec<Prescribed> = face_nodes(&mesh, 2, 0.)
            .into_iter()
            .map(|node| Prescribed {
                node,
                displacement: [0.; 3],
            })
            .collect();
        prescribed.extend(face_nodes(&mesh, 2, 1.).into_iter().map(|node| Prescribed {
            node,
            displacement: [0., 0., -0.3],
        }));
        let (moved, r) = move_mesh(&mesh, &prescribed, MotionOptions::default()).unwrap();
        assert_eq!(r.outcome, Outcome::Complete, "{r:?}");
        assert!((r.applied_fraction - 1.).abs() < 1e-9);
        // The top face sits at 0.7 and the interior layers are spread between: z increases monotonically along each column.
        for &n in &face_nodes(&mesh, 2, 1.) {
            assert!((moved.nodes[n][2] - 0.7).abs() < 1e-9);
        }
        let q = moved.quality();
        assert_eq!(q.inverted, 0);
        assert!(q.min_scaled_jacobian > 0.9, "{:?}", q.min_scaled_jacobian);
        // Connectivity is untouched.
        assert_eq!(moved.blocks[0].conn, mesh.blocks[0].conn);
        assert!(
            r.worst_free_reaction_ratio < 1e-8,
            "{}",
            r.worst_free_reaction_ratio
        );
    }

    #[test]
    fn an_impossible_motion_stops_at_the_last_valid_mesh_and_hands_over_to_remeshing() {
        let mesh = column(3);
        let mut prescribed: Vec<Prescribed> = face_nodes(&mesh, 2, 0.)
            .into_iter()
            .map(|node| Prescribed {
                node,
                displacement: [0.; 3],
            })
            .collect();
        // Push the top through the bottom.
        prescribed.extend(face_nodes(&mesh, 2, 1.).into_iter().map(|node| Prescribed {
            node,
            displacement: [0., 0., -1.5],
        }));
        let (moved, r) = move_mesh(&mesh, &prescribed, MotionOptions::default()).unwrap();
        match &r.outcome {
            Outcome::NeedsRemesh {
                applied_fraction,
                reason,
            } => {
                assert!(
                    *applied_fraction > 0. && *applied_fraction < 1.,
                    "{applied_fraction}"
                );
                assert!(!reason.is_empty());
            }
            o => panic!("expected a hand-over to remeshing, got {o:?}"),
        }
        assert!(r.rollbacks >= 1);
        // The returned mesh is the last valid one.
        let q = moved.quality();
        assert_eq!(q.inverted, 0);
        assert!(q.min_scaled_jacobian >= MotionOptions::default().thresholds.min_scaled_jacobian);
    }

    #[test]
    fn bad_input_is_refused_not_guessed() {
        let mesh = column(2);
        let p = [Prescribed {
            node: 0,
            displacement: [0.; 3],
        }];
        assert!(move_mesh(&mesh, &[], MotionOptions::default()).is_err());
        assert!(move_mesh(
            &mesh,
            &[Prescribed {
                node: 10_000,
                displacement: [0.; 3]
            }],
            MotionOptions::default()
        )
        .is_err());
        assert!(move_mesh(
            &mesh,
            &[Prescribed {
                node: 0,
                displacement: [f64::NAN, 0., 0.]
            }],
            MotionOptions::default()
        )
        .is_err());
        assert!(move_mesh(
            &mesh,
            &p,
            MotionOptions {
                steps: 0,
                ..MotionOptions::default()
            }
        )
        .is_err());
        // A mesh that already violates the limits cannot be moved.
        let mut bent = column(2);
        bent.nodes[13] = [5., 5., 5.];
        assert!(move_mesh(&bent, &p, MotionOptions::default()).is_err());
    }

    #[test]
    fn size_based_stiffening_protects_the_small_elements_next_to_the_moving_wall() {
        // Layers cluster towards z = 1 (the wall that moves), the usual boundary-layer situation.
        let graded = grid(
            Physics::Solid,
            ElementKind::Hex8,
            Elastic::new(1., 0.3),
            [3, 3, 8],
            &|p| [p[0], p[1], 1. - (1. - p[2]).powf(2.5)],
        )
        .unwrap();
        let mut prescribed: Vec<Prescribed> = face_nodes(&graded, 2, 0.)
            .into_iter()
            .map(|node| Prescribed {
                node,
                displacement: [0.; 3],
            })
            .collect();
        prescribed.extend(
            face_nodes(&graded, 2, 1.)
                .into_iter()
                .map(|node| Prescribed {
                    node,
                    displacement: [0., 0., -0.5],
                }),
        );
        let run = |exponent: f64| {
            let (_, r) = move_mesh(
                &graded,
                &prescribed,
                MotionOptions {
                    stiffening_exponent: exponent,
                    ..MotionOptions::default()
                },
            )
            .unwrap();
            r
        };
        let (uniform, stiff) = (run(0.), run(2.));
        eprintln!(
            "uniform: applied {:.3}, min scaled Jacobian {:.3}, edge ratio {:.1}; stiffened: applied {:.3}, {:.3}, {:.1}",
            uniform.applied_fraction, uniform.min_scaled_jacobian_after, uniform.max_edge_ratio_after, stiff.applied_fraction, stiff.min_scaled_jacobian_after, stiff.max_edge_ratio_after
        );
        assert!(stiff.applied_fraction >= uniform.applied_fraction - 1e-9);
        // Both complete the motion; the stiffened one keeps a clearly better worst element (0.48 against 0.23 measured).
        assert!(uniform.applied_fraction > 1. - 1e-9 && stiff.applied_fraction > 1. - 1e-9);
        assert!(stiff.min_scaled_jacobian_after > 1.5 * uniform.min_scaled_jacobian_after);
    }
}
