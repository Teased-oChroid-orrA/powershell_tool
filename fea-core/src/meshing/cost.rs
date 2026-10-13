//! Solver-aware cost of a mesh: what it takes to assemble and factor it, predicted from the mesh alone (an exact symbolic
//! analysis, no numeric factorisation) and turned into seconds by rates measured on the machine at hand. The prediction
//! compares candidate meshes of the *same nominal accuracy*; it does not decide accuracy (an estimator does).
//!
//! Total cost of a workload, as in the epic: `generation + n_assemble * assembly + n_factor * factor + n_solve * back-solve
//! + estimation + adaptation`; the weights are the workload's counts, not tuned constants.

use super::MeshError;
use crate::analysis::Model;
use crate::linear::{Ordering, Reduced};
use crate::mesh::Mesh;

/// Size of the work a mesh implies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshCost {
    pub dofs: usize,
    pub elements: usize,
    /// Entries of the symmetric block-lower stiffness (stored scalars).
    pub matrix_nnz: usize,
    /// Entries of the Cholesky factor after the best ordering (exact symbolic analysis).
    pub factor_nnz: usize,
    /// `sum over elements of (nodes * dofs per node)^2`: element-stiffness work.
    pub assembly_units: f64,
    /// `factor_nnz^1.5`: the factorisation work proxy (exact for 2D nested dissection up to a constant, a fair proxy in 3D).
    pub factor_units: f64,
}

impl MeshCost {
    /// Cost of `mesh` (one node fixed so the system is not empty).
    pub fn of(mesh: &Mesh) -> Result<Self, MeshError> {
        let model = Model::new(mesh.clone()).map_err(MeshError::Failed)?;
        let mut bc = model.dirichlet();
        if !mesh.nodes.is_empty() {
            bc.fix_node(0);
        }
        let red = Reduced::with_ordering(&model.pattern, &bc, Some(&mesh.nodes), Ordering::Auto).map_err(|e| MeshError::Failed(e.to_string()))?;
        let d = mesh.dim();
        let assembly_units: f64 = mesh.blocks.iter().map(|b| b.n_elems() as f64 * ((b.kind.n_nodes() * d) as f64).powi(2)).sum();
        let factor_nnz = red.factor_nnz();
        Ok(Self { dofs: mesh.n_dofs(), elements: mesh.n_elems(), matrix_nnz: model.pattern.col_ptr.last().map_or(0, |&n| n as usize * d * d), factor_nnz, assembly_units, factor_units: (factor_nnz as f64).powf(1.5) })
    }
}

/// What the analysis will do with the mesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Workload {
    pub assemblies: f64,
    pub factorisations: f64,
    /// Back-substitutions (load cases, adjoint solves).
    pub solves: f64,
    /// Extra seconds per mesh for error estimation and adaptation (measured by the caller, `0` when not applicable).
    pub estimation_seconds: f64,
    pub adaptation_seconds: f64,
}

impl Workload {
    /// One linear static solve.
    pub fn linear_static() -> Self {
        Self { assemblies: 1.0, factorisations: 1.0, solves: 1.0, estimation_seconds: 0.0, adaptation_seconds: 0.0 }
    }

    /// A nonlinear analysis re-assembling and re-factoring `iterations` times.
    pub fn nonlinear(iterations: f64) -> Self {
        Self { assemblies: iterations, factorisations: iterations, solves: iterations, estimation_seconds: 0.0, adaptation_seconds: 0.0 }
    }
}

/// Seconds per unit of work, measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostModel {
    pub assembly_s_per_unit: f64,
    pub factor_s_per_unit: f64,
    pub solve_s_per_factor_nnz: f64,
}

impl CostModel {
    /// Measure the rates by assembling and solving `sample` (best of `repeats` runs, to shed scheduling noise).
    pub fn calibrate(sample: &Mesh, repeats: usize) -> Result<Self, MeshError> {
        let cost = MeshCost::of(sample)?;
        let model = Model::new(sample.clone()).map_err(MeshError::Failed)?;
        let mut bc = model.dirichlet();
        for (n, x) in sample.nodes.iter().enumerate() {
            if x[0] <= sample.nodes.iter().map(|y| y[0]).fold(f64::INFINITY, f64::min) + 1e-12 {
                for c in 0..sample.dim() {
                    bc.fix(n, c, 0.0);
                }
            }
        }
        let xmax = sample.nodes.iter().map(|y| y[0]).fold(f64::NEG_INFINITY, f64::max);
        let loads = crate::Loads { nodal: sample.nodes.iter().enumerate().filter(|(_, x)| x[0] >= xmax - 1e-12).map(|(n, _)| (n, [1.0, 0.0, 0.0])).collect(), ..Default::default() };
        let (mut asm, mut fac, mut slv) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
        for _ in 0..repeats.max(1) {
            let sol = model.solve_static(&loads, &bc).map_err(MeshError::Failed)?;
            asm = asm.min(sol.timings.assemble_ms);
            fac = fac.min(sol.timings.factor_ms);
            slv = slv.min(sol.timings.solve_ms);
        }
        Ok(Self {
            assembly_s_per_unit: asm * 1e-3 / cost.assembly_units.max(1.0),
            factor_s_per_unit: fac * 1e-3 / cost.factor_units.max(1.0),
            solve_s_per_factor_nnz: slv * 1e-3 / (cost.factor_nnz as f64).max(1.0),
        })
    }

    /// Predicted seconds for `workload` on a mesh of cost `cost`, plus the measured mesh generation time.
    pub fn predict(&self, cost: &MeshCost, workload: &Workload, generation_seconds: f64) -> f64 {
        generation_seconds
            + workload.assemblies * self.assembly_s_per_unit * cost.assembly_units
            + workload.factorisations * self.factor_s_per_unit * cost.factor_units
            + workload.solves * self.solve_s_per_factor_nnz * cost.factor_nnz as f64
            + workload.estimation_seconds
            + workload.adaptation_seconds
    }
}

/// How a candidate relates to the accuracy specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Equivalence {
    /// Built from the same size function with isotropic elements: nominally the same accuracy as the baseline.
    Nominal,
    /// Anisotropic or otherwise coarser in some direction: cheaper, but only valid where the solution is smooth in that
    /// direction; needs validation (an error estimate) before it is trusted.
    NeedsValidation,
}

/// A mesh candidate for one analysis.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub mesh: Mesh,
    pub generation_seconds: f64,
    pub equivalence: Equivalence,
}

/// Outcome of a comparison.
#[derive(Debug, Clone)]
pub struct Choice {
    pub index: usize,
    /// Predicted seconds and cost of every candidate, in order.
    pub predictions: Vec<(String, MeshCost, f64)>,
}

/// The cheapest candidate by predicted total time. Candidates that `NeedsValidation` take part only when
/// `allow_unvalidated`; ties and non-finite predictions go to the first candidate (put the verified baseline first).
pub fn choose(candidates: &[Candidate], model: &CostModel, workload: &Workload, allow_unvalidated: bool) -> Result<Choice, MeshError> {
    if candidates.is_empty() {
        return Err(MeshError::Failed("no candidate meshes".into()));
    }
    let mut predictions = Vec::new();
    let mut best: Option<(usize, f64)> = None;
    for (i, c) in candidates.iter().enumerate() {
        let cost = MeshCost::of(&c.mesh)?;
        let t = model.predict(&cost, workload, c.generation_seconds);
        predictions.push((c.name.clone(), cost, t));
        let eligible = i == 0 || allow_unvalidated || c.equivalence == Equivalence::Nominal;
        if eligible && t.is_finite() && best.is_none_or(|(_, bt)| t < bt) {
            best = Some((i, t));
        }
    }
    Ok(Choice { index: best.map_or(0, |b| b.0), predictions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};

    fn mesh(kind: ElementKind, n: usize) -> Mesh {
        grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1e7, 0.3), [n, n, 1], &|p| p).unwrap()
    }

    #[test]
    fn cost_grows_with_the_mesh_and_matches_known_counts() {
        let small = MeshCost::of(&mesh(ElementKind::Quad4, 4)).unwrap();
        let big = MeshCost::of(&mesh(ElementKind::Quad4, 16)).unwrap();
        assert_eq!(small.dofs, 2 * 25);
        assert_eq!(small.elements, 16);
        assert!((small.assembly_units - 16.0 * 64.0).abs() < 1e-9);
        assert!(big.factor_nnz > 8 * small.factor_nnz && big.factor_units > 20.0 * small.factor_units);
        // a Tri3 mesh of the same grid has more elements and a different factor
        let tri = MeshCost::of(&mesh(ElementKind::Tri3, 4)).unwrap();
        assert_eq!(tri.elements, 32);
    }

    #[test]
    fn the_cheapest_nominal_candidate_wins_and_unvalidated_ones_need_permission() {
        let model = CostModel { assembly_s_per_unit: 1e-9, factor_s_per_unit: 1e-9, solve_s_per_factor_nnz: 1e-9 };
        let work = Workload::linear_static();
        let cands = vec![
            Candidate { name: "baseline".into(), mesh: mesh(ElementKind::Quad4, 16), generation_seconds: 0.0, equivalence: Equivalence::Nominal },
            Candidate { name: "coarse-unvalidated".into(), mesh: mesh(ElementKind::Quad4, 6), generation_seconds: 0.0, equivalence: Equivalence::NeedsValidation },
            Candidate { name: "fine".into(), mesh: mesh(ElementKind::Quad4, 24), generation_seconds: 0.0, equivalence: Equivalence::Nominal },
        ];
        assert_eq!(choose(&cands, &model, &work, false).unwrap().index, 0);
        assert_eq!(choose(&cands, &model, &work, true).unwrap().index, 1);
        // a slow generator moves the choice: the baseline pays 100 s of generation
        let mut slow = cands.clone();
        slow[0].generation_seconds = 100.0;
        slow[2].generation_seconds = 0.0;
        // `fine` is bigger but cheaper to generate; with 100 s against microseconds of solving it wins
        assert_eq!(choose(&slow, &model, &work, false).unwrap().index, 2);
        assert!(choose(&[], &model, &work, false).is_err());
    }

    #[test]
    #[ignore = "timing: run with --ignored --nocapture"]
    fn predictions_track_measured_solve_times() {
        let model = CostModel::calibrate(&mesh(ElementKind::Quad8, 100), 3).unwrap();
        for n in [50usize, 70, 140] {
            let m = mesh(ElementKind::Quad8, n);
            let cost = MeshCost::of(&m).unwrap();
            let mut predicted = model.predict(&cost, &Workload::linear_static(), 0.0);
            predicted += 0.0;
            let measured = CostModel::calibrate(&m, 3).unwrap();
            let meas_total = measured.predict(&cost, &Workload::linear_static(), 0.0);
            eprintln!("n={n}: predicted {predicted:.4}s measured {meas_total:.4}s ratio {:.2}", predicted / meas_total);
            assert!(predicted / meas_total > 0.4 && predicted / meas_total < 2.5);
        }
    }
}
