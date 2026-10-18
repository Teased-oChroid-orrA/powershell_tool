//! A body moved by prescribed boundary displacements, with remeshing and state transfer as one controlled loop (epic #59).
//!
//! [`meshing::motion::move_mesh_sliding`] relocates nodes without changing connectivity until the elements cannot follow
//! (`Outcome::NeedsRemesh`). [`advance`] closes the loop: on that hand-over it asks the caller's remesher for a new mesh of the
//! current shape, carries the nodal field to it with [`NodalInterpolation`] (an unlocated node is an error, never a silent
//! zero), asks the caller again for the constraints of the *new* mesh (node numbers change; the target does not) and moves on
//! from there, up to a budget of remeshes. Nothing is applied silently: every segment's motion report is kept, a remesh that
//! yields no progress ends the loop with an error, and the field is only ever the interpolant of the previous mesh's field.
//! Nodal fields travel with their nodes during a motion (the Lagrangian reading); this is not an ALE formulation.

use super::{NodalInterpolation, SolverSnapshot, StateTransferOperator};
use crate::mesh::Mesh;
use crate::meshing::motion::{move_mesh_sliding, MotionOptions, MotionReport, Outcome, Prescribed, Slide};
use crate::meshing::MeshError;
use crate::Model;

/// What drives the motion on one mesh: the remaining displacement of the prescribed nodes and the sliding constraints.
#[derive(Debug, Clone, Default)]
pub struct Constraints {
    pub prescribed: Vec<Prescribed>,
    pub slides: Vec<Slide>,
}

#[derive(Debug, Clone, Copy)]
pub struct AdvanceOptions {
    pub motion: MotionOptions,
    /// Remeshes allowed before the loop gives up with the last valid mesh.
    pub max_remeshes: usize,
    /// Transfer of the carried field to a remeshed mesh.
    pub transfer: NodalInterpolation,
    /// Smallest fraction of the remaining motion a segment after a remesh must achieve; less means the remesher does not help
    /// and the loop stops with an error instead of remeshing forever.
    pub min_progress: f64,
}

impl Default for AdvanceOptions {
    fn default() -> Self {
        Self { motion: MotionOptions::default(), max_remeshes: 4, transfer: NodalInterpolation { max_unlocated_fraction: 0.2 }, min_progress: 1e-3 }
    }
}

#[derive(Debug, Clone)]
pub struct AdvanceReport {
    /// The motion report of every segment (one per mesh used).
    pub segments: Vec<MotionReport>,
    pub remeshes: usize,
    /// Nodes of each new mesh outside the previous one (their carried value is zero), per remesh.
    pub unlocated: Vec<usize>,
    pub complete: bool,
}

#[derive(Debug, Clone)]
pub struct Advanced {
    pub mesh: Mesh,
    /// The carried nodal field on `mesh` (`dim` values per node).
    pub field: Vec<f64>,
    pub report: AdvanceReport,
}

/// Move `mesh` to the target `constraints` describes, remeshing with `remesh` whenever the elements cannot follow, carrying
/// `field` (`dim` values per node) along. Returns the last valid mesh with `report.complete == false` when the remesh budget is
/// spent, and an error when a remesh makes no progress, the remeshed mesh is unacceptable or the transfer fails.
pub fn advance(
    mesh: &Mesh,
    field: &[f64],
    constraints: &dyn Fn(&Mesh) -> Constraints,
    remesh: &mut dyn FnMut(&Mesh) -> Result<Mesh, MeshError>,
    options: AdvanceOptions,
) -> Result<Advanced, MeshError> {
    if field.len() != mesh.n_dofs() {
        return Err(MeshError::Failed(format!("the carried field has {} values for {} dofs", field.len(), mesh.n_dofs())));
    }
    let mut current = mesh.clone();
    let mut field = field.to_vec();
    let mut report = AdvanceReport { segments: Vec::new(), remeshes: 0, unlocated: Vec::new(), complete: false };
    loop {
        let c = constraints(&current);
        let (moved, motion) = move_mesh_sliding(&current, &c.prescribed, &c.slides, options.motion)?;
        let after_remesh = report.remeshes > 0 && report.segments.len() == report.remeshes;
        let (outcome, applied) = (motion.outcome.clone(), motion.applied_fraction);
        report.segments.push(motion);
        match outcome {
            Outcome::Complete => {
                report.complete = true;
                return Ok(Advanced { mesh: moved, field, report });
            }
            Outcome::NeedsRemesh { reason, .. } => {
                if after_remesh && applied < options.min_progress {
                    return Err(MeshError::Failed(format!("the remeshed mesh made no progress ({applied:.2e} of the remaining motion): {reason}")));
                }
                if report.remeshes >= options.max_remeshes {
                    return Ok(Advanced { mesh: moved, field, report });
                }
                let next = remesh(&moved)?;
                let old_model = Model::new(moved.clone()).map_err(MeshError::Failed)?;
                let snapshot = SolverSnapshot::new(&moved, field, 0.0, "motion").map_err(MeshError::Failed)?;
                let transferred = options.transfer.transfer(&old_model, &snapshot, &next)?;
                report.unlocated.push(transferred.unlocated.len());
                field = transferred.snapshot.u;
                current = next;
                report.remeshes += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    fn unit_box(n: usize) -> Mesh {
        grid(Physics::Solid, ElementKind::Hex8, Elastic::new(1., 0.3), [n, n, n], &|p| p).expect("box")
    }

    /// A fresh `n`-division grid of the bounding box of `m` (the body is a box, so the new mesh covers it exactly).
    fn regrid(m: &Mesh, n: [usize; 3]) -> Result<Mesh, MeshError> {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for x in &m.nodes {
            for k in 0..3 {
                lo[k] = lo[k].min(x[k]);
                hi[k] = hi[k].max(x[k]);
            }
        }
        grid(Physics::Solid, ElementKind::Hex8, Elastic::new(1., 0.3), n, &move |p| std::array::from_fn(|k| lo[k] + p[k] * (hi[k] - lo[k]))).map_err(MeshError::Failed)
    }

    /// Frictionless compression: symmetry planes at x = 0 and y = 0, the bottom held in z, the top carried to height `top`.
    fn compress(top: f64) -> impl Fn(&Mesh) -> Constraints {
        move |m: &Mesh| {
            let zmax = m.nodes.iter().map(|x| x[2]).fold(f64::NEG_INFINITY, f64::max);
            let mut slides = Vec::new();
            for (n, x) in m.nodes.iter().enumerate() {
                if x[0].abs() < 1e-9 {
                    slides.push(Slide { node: n, axis: 0, displacement: 0.0 });
                }
                if x[1].abs() < 1e-9 {
                    slides.push(Slide { node: n, axis: 1, displacement: 0.0 });
                }
                if x[2].abs() < 1e-9 {
                    slides.push(Slide { node: n, axis: 2, displacement: 0.0 });
                } else if (x[2] - zmax).abs() < 1e-9 {
                    slides.push(Slide { node: n, axis: 2, displacement: top - zmax });
                }
            }
            Constraints { prescribed: Vec::new(), slides }
        }
    }

    fn strict_aspect() -> AdvanceOptions {
        let mut o = AdvanceOptions::default();
        o.motion.thresholds.max_edge_ratio = 30.0;
        o
    }

    #[test]
    fn a_compression_that_stretches_the_elements_past_the_ceiling_continues_on_a_coarser_body_with_its_field() {
        let mesh = unit_box(3);
        // The carried field is the material position, so after the motion it must still say where each point came from.
        let field: Vec<f64> = mesh.nodes.iter().flat_map(|x| [x[0], x[1], x[2]]).collect();
        let target = compress(0.03);
        // 97 % compression flattens three layers past the edge-ratio ceiling of 30: relocation alone cannot follow.
        let (_, direct) = move_mesh_sliding(&mesh, &[], &target(&mesh).slides, strict_aspect().motion).unwrap();
        assert!(matches!(direct.outcome, Outcome::NeedsRemesh { .. }), "the test needs a motion that hands over: {direct:?}");
        let mut remeshes = 0;
        let out = advance(&mesh, &field, &target, &mut |m| { remeshes += 1; regrid(m, [3, 3, 1]) }, strict_aspect()).unwrap();
        assert!(out.report.complete, "{:?}", out.report);
        assert_eq!((out.report.remeshes, remeshes), (1, 1));
        assert_eq!(out.report.segments.len(), 2);
        assert_eq!(out.report.unlocated, vec![0], "the regrid covers the old body exactly");
        // The new body has one layer: fewer nodes, the top at 0.03, the elements acceptable.
        assert!(out.mesh.nodes.len() < mesh.nodes.len());
        let zmax = out.mesh.nodes.iter().map(|x| x[2]).fold(f64::NEG_INFINITY, f64::max);
        assert!((zmax - 0.03).abs() < 1e-9, "top at {zmax}");
        let q = out.mesh.quality();
        assert_eq!(q.inverted, 0);
        assert!(q.max_edge_ratio <= 30.0, "edge ratio {}", q.max_edge_ratio);
        // The uniaxial motion is affine, so the carried material position maps back to the node exactly: (x, y) are the
        // material coordinates scaled by the lateral stretch, z by the compression.
        let xmax = out.mesh.nodes.iter().map(|x| x[0]).fold(f64::NEG_INFINITY, f64::max);
        let ymax = out.mesh.nodes.iter().map(|x| x[1]).fold(f64::NEG_INFINITY, f64::max);
        let mut worst: f64 = 0.0;
        for (n, x) in out.mesh.nodes.iter().enumerate() {
            let f = &out.field[n * 3..n * 3 + 3];
            worst = worst.max((f[0] * xmax - x[0]).abs()).max((f[1] * ymax - x[1]).abs()).max((f[2] * 0.03 - x[2]).abs());
        }
        eprintln!("segments {:?}, edge ratio {:.1}, worst material-position mismatch {worst:.2e}", out.report.segments.iter().map(|s| s.applied_fraction).collect::<Vec<_>>(), q.max_edge_ratio);
        assert!(worst < 1e-9, "material position drifted by {worst}");
    }

    #[test]
    fn a_remesher_that_does_not_help_a_spent_budget_and_bad_input_end_the_loop_explicitly() {
        let mesh = unit_box(3);
        let field = vec![0.0; mesh.n_dofs()];
        let target = compress(0.03);
        // The remesher returns the mesh unchanged: no progress after the first remesh.
        let err = advance(&mesh, &field, &target, &mut |m| Ok(m.clone()), strict_aspect()).unwrap_err();
        assert!(format!("{err:?}").contains("no progress"), "{err:?}");
        // A budget of zero remeshes returns the last valid mesh, not complete.
        let none = AdvanceOptions { max_remeshes: 0, ..strict_aspect() };
        let out = advance(&mesh, &field, &target, &mut |m| Ok(m.clone()), none).unwrap();
        assert!(!out.report.complete && out.report.remeshes == 0);
        assert_eq!(out.mesh.quality().inverted, 0);
        // A field of the wrong length is refused before anything moves.
        assert!(advance(&mesh, &field[..5], &target, &mut |m| Ok(m.clone()), strict_aspect()).is_err());
        // A failing remesher's error is passed on.
        let err = advance(&mesh, &field, &target, &mut |_| Err(MeshError::Failed("no mesher".into())), strict_aspect()).unwrap_err();
        assert!(format!("{err:?}").contains("no mesher"));
        // A remesh that leaves the old domain is refused by the transfer, never filled with zeros.
        let far = |m: &Mesh| -> Result<Mesh, MeshError> {
            let mut out = m.clone();
            for x in &mut out.nodes {
                x[0] += 10.0;
            }
            Ok(out)
        };
        let err = advance(&mesh, &field, &target, &mut { far }, strict_aspect()).unwrap_err();
        assert!(format!("{err:?}").contains("outside the old mesh"), "{err:?}");
    }

    #[test]
    fn a_heavily_twisted_box_hands_over_instead_of_failing_on_a_hidden_inversion() {
        // Before the Gauss-point check a warped Hex8 whose corner Jacobians were still positive passed the step, and the next
        // pseudo-elastic solve then refused it with an error ("inverted or degenerate element ... Gauss point").
        let mesh = unit_box(4);
        let reference = mesh.nodes.clone();
        let twist = |angle: f64| -> Vec<Prescribed> {
            let mut p = Vec::new();
            for (n, r) in reference.iter().enumerate() {
                if r.iter().any(|&c| c == 0.0 || c == 1.0) {
                    let a = angle * r[2];
                    let (dx, dy) = (r[0] - 0.5, r[1] - 0.5);
                    let target = [0.5 + dx * a.cos() - dy * a.sin(), 0.5 + dx * a.sin() + dy * a.cos(), r[2]];
                    p.push(Prescribed { node: n, displacement: [target[0] - r[0], target[1] - r[1], 0.0] });
                }
            }
            p
        };
        for deg in [200.0f64, 450.0, 720.0] {
            let (moved, r) = move_mesh_sliding(&mesh, &twist(deg.to_radians()), &[], MotionOptions::default()).unwrap_or_else(|e| panic!("twist {deg}: {e:?}"));
            assert_eq!(moved.quality().inverted, 0, "twist {deg}");
            assert!(gauss_free(&moved), "twist {deg}: {r:?}");
        }
    }

    fn gauss_free(m: &Mesh) -> bool {
        crate::meshing::motion::mesh_is_gauss_valid(m)
    }
}
