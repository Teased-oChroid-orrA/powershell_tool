//! Solver-aware cost of a mesh: what it takes to assemble and factor it, predicted from the mesh alone (an exact symbolic
//! analysis, no numeric factorisation) and turned into seconds by rates measured on the machine at hand. The prediction
//! compares candidate meshes of the *same nominal accuracy*; it does not decide accuracy (an estimator does).
//!
//! Total cost of a workload, as in the epic: `CAD + generation + estimation + adaptation + transfer + analyses * (n_assemble *
//! assembly + n_factor * factor + n_solve * back-solve)`; the weights are the workload's counts, not tuned constants. The
//! one-time terms (CAD, generation, estimation, adaptation, transfer) are paid once per mesh, the solver terms once per
//! analysis, so a mesh that is expensive to make pays off only after `break_even_analyses` analyses. A candidate can also be
//! bounded by a memory estimate and a time limit (`choose_within`).

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
    /// Rough peak memory of a direct solve in bytes: the factor and the stiffness (8-byte scalars) plus a few vectors per
    /// dof. An estimate for comparing candidates against a budget, not an allocation guarantee.
    pub fn memory_bytes(&self) -> u64 {
        8 * (self.factor_nnz as u64 + self.matrix_nnz as u64 + 8 * self.dofs as u64)
    }

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
        Ok(Self { dofs: mesh.n_dofs(), elements: mesh.n_elems(), matrix_nnz: model.pattern.col_ptr.last().map_or(0, |&n| n * d * d), factor_nnz, assembly_units, factor_units: (factor_nnz as f64).powf(1.5) })
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
    /// Seconds per mesh for CAD import, healing and association (measured by the caller, `0` when the geometry is given).
    pub cad_seconds: f64,
    /// Seconds per mesh to transfer fields and history from the previous mesh (adaptive loops, remeshing during a run).
    pub transfer_seconds: f64,
    /// Analyses run on the same mesh (design sweeps, load cases solved separately): the solver terms are paid this many
    /// times, the one-time terms once.
    pub analyses: f64,
}

impl Workload {
    /// One linear static solve.
    pub fn linear_static() -> Self {
        Self { assemblies: 1.0, factorisations: 1.0, solves: 1.0, estimation_seconds: 0.0, adaptation_seconds: 0.0, cad_seconds: 0.0, transfer_seconds: 0.0, analyses: 1.0 }
    }

    /// A nonlinear analysis re-assembling and re-factoring `iterations` times.
    pub fn nonlinear(iterations: f64) -> Self {
        Self { assemblies: iterations, factorisations: iterations, solves: iterations, ..Self::linear_static() }
    }

    /// The same work repeated `analyses` times on one mesh.
    pub fn repeated(self, analyses: f64) -> Self {
        Self { analyses, ..self }
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
        self.one_time_seconds(workload, generation_seconds) + self.per_analysis_seconds(cost, workload) * workload.analyses
    }

    /// Terms paid once per mesh.
    pub fn one_time_seconds(&self, workload: &Workload, generation_seconds: f64) -> f64 {
        workload.cad_seconds + generation_seconds + workload.estimation_seconds + workload.adaptation_seconds + workload.transfer_seconds
    }

    /// Solver terms of one analysis.
    pub fn per_analysis_seconds(&self, cost: &MeshCost, workload: &Workload) -> f64 {
        workload.assemblies * self.assembly_s_per_unit * cost.assembly_units
            + workload.factorisations * self.factor_s_per_unit * cost.factor_units
            + workload.solves * self.solve_s_per_factor_nnz * cost.factor_nnz as f64
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
    /// Candidates excluded, with the reason (unvalidated, over the memory or time limit).
    pub rejected: Vec<(usize, String)>,
}

/// Limits a candidate must respect to be chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Limits {
    pub max_memory_bytes: Option<u64>,
    pub max_seconds: Option<f64>,
}

/// The cheapest candidate by predicted total time. Candidates that `NeedsValidation` take part only when
/// `allow_unvalidated`; ties and non-finite predictions go to the first candidate (put the verified baseline first).
pub fn choose(candidates: &[Candidate], model: &CostModel, workload: &Workload, allow_unvalidated: bool) -> Result<Choice, MeshError> {
    choose_within(candidates, model, workload, allow_unvalidated, Limits::default())
}

/// [`choose`] under limits: a candidate whose memory estimate or predicted time exceeds a limit is excluded (and listed in
/// `Choice::rejected`), the baseline included. When no candidate is feasible the answer is an error naming the limits, never
/// a silently infeasible pick.
pub fn choose_within(candidates: &[Candidate], model: &CostModel, workload: &Workload, allow_unvalidated: bool, limits: Limits) -> Result<Choice, MeshError> {
    if candidates.is_empty() {
        return Err(MeshError::Failed("no candidate meshes".into()));
    }
    let mut predictions = Vec::new();
    let mut rejected = Vec::new();
    let mut best: Option<(usize, f64)> = None;
    for (i, c) in candidates.iter().enumerate() {
        let cost = MeshCost::of(&c.mesh)?;
        let t = model.predict(&cost, workload, c.generation_seconds);
        predictions.push((c.name.clone(), cost, t));
        if !(i == 0 || allow_unvalidated || c.equivalence == Equivalence::Nominal) {
            rejected.push((i, "needs validation".to_string()));
            continue;
        }
        if limits.max_memory_bytes.is_some_and(|m| cost.memory_bytes() > m) {
            rejected.push((i, format!("memory estimate {} bytes exceeds the limit", cost.memory_bytes())));
            continue;
        }
        if limits.max_seconds.is_some_and(|m| t > m) {
            rejected.push((i, format!("predicted {t:.3} s exceeds the limit")));
            continue;
        }
        if t.is_finite() && best.is_none_or(|(_, bt)| t < bt) {
            best = Some((i, t));
        }
    }
    match best {
        Some((index, _)) => Ok(Choice { index, predictions, rejected }),
        None if limits == Limits::default() => Ok(Choice { index: 0, predictions, rejected }),
        None => Err(MeshError::Failed(format!("no candidate satisfies the limits {limits:?}: {rejected:?}"))),
    }
}

/// Number of analyses on one mesh after which candidate `b` (dearer to make, cheaper to solve) overtakes `a`; `None` when
/// `b` never pays off (it is not cheaper per analysis) or is already cheaper in total for one analysis.
pub fn break_even_analyses(a: &Candidate, b: &Candidate, model: &CostModel, workload: &Workload) -> Result<Option<f64>, MeshError> {
    let (ca, cb) = (MeshCost::of(&a.mesh)?, MeshCost::of(&b.mesh)?);
    let (per_a, per_b) = (model.per_analysis_seconds(&ca, workload), model.per_analysis_seconds(&cb, workload));
    let (once_a, once_b) = (model.one_time_seconds(workload, a.generation_seconds), model.one_time_seconds(workload, b.generation_seconds));
    // once_a + n per_a = once_b + n per_b  =>  n = (once_b - once_a) / (per_a - per_b)
    if per_a <= per_b || !(per_a - per_b).is_finite() {
        return Ok(None);
    }
    let n = (once_b - once_a) / (per_a - per_b);
    Ok((n > 1.0).then_some(n))
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
    fn one_time_terms_are_paid_once_and_solver_terms_once_per_analysis() {
        let model = CostModel { assembly_s_per_unit: 1e-6, factor_s_per_unit: 1e-6, solve_s_per_factor_nnz: 1e-6 };
        let cost = MeshCost::of(&mesh(ElementKind::Quad4, 8)).unwrap();
        let base = Workload::linear_static();
        let work = Workload { cad_seconds: 10.0, transfer_seconds: 3.0, estimation_seconds: 2.0, adaptation_seconds: 5.0, ..base };
        let one = model.predict(&cost, &work, 7.0);
        assert!((model.one_time_seconds(&work, 7.0) - 27.0).abs() < 1e-12);
        assert!((one - 27.0 - model.per_analysis_seconds(&cost, &work)).abs() < 1e-9);
        let ten = model.predict(&cost, &work.repeated(10.0), 7.0);
        assert!((ten - 27.0 - 10.0 * model.per_analysis_seconds(&cost, &work)).abs() < 1e-9);
        // the old counts are unchanged: a linear static solve predicts exactly the three solver terms plus generation
        let plain = model.predict(&cost, &base, 0.0);
        assert!((plain - model.per_analysis_seconds(&cost, &base)).abs() < 1e-12);
    }

    #[test]
    fn a_dearer_mesh_pays_off_exactly_at_the_break_even_number_of_analyses() {
        let model = CostModel { assembly_s_per_unit: 1e-5, factor_s_per_unit: 1e-5, solve_s_per_factor_nnz: 1e-5 };
        let work = Workload::linear_static();
        let big = Candidate { name: "big".into(), mesh: mesh(ElementKind::Quad4, 20), generation_seconds: 0.0, equivalence: Equivalence::Nominal };
        let small = Candidate { name: "small".into(), mesh: mesh(ElementKind::Quad4, 8), generation_seconds: 0.0, equivalence: Equivalence::Nominal };
        // `small` is cheaper to solve and costs 50 s more to make (an expensive optimiser).
        let mut small_dear = small.clone();
        small_dear.generation_seconds = 50.0;
        let n = break_even_analyses(&big, &small_dear, &model, &work).unwrap().expect("small pays off eventually");
        assert!(n > 1.0);
        let pick = |analyses: f64| choose(&[big.clone(), small_dear.clone()], &model, &work.repeated(analyses), false).unwrap().index;
        assert_eq!(pick(n * 0.9), 0, "before the break-even the cheap-to-make mesh wins");
        assert_eq!(pick(n * 1.1), 1, "after it the cheap-to-solve mesh wins");
        // a mesh that is no cheaper per analysis never pays off
        assert_eq!(break_even_analyses(&small, &big, &model, &work).unwrap(), None);
    }

    #[test]
    fn limits_exclude_candidates_and_an_infeasible_set_is_an_error() {
        let model = CostModel { assembly_s_per_unit: 1e-9, factor_s_per_unit: 1e-9, solve_s_per_factor_nnz: 1e-9 };
        let work = Workload::linear_static();
        let cands = vec![
            Candidate { name: "fine".into(), mesh: mesh(ElementKind::Quad4, 24), generation_seconds: 0.0, equivalence: Equivalence::Nominal },
            Candidate { name: "coarse".into(), mesh: mesh(ElementKind::Quad4, 8), generation_seconds: 0.0, equivalence: Equivalence::Nominal },
        ];
        let (fine_mem, coarse_mem) = (MeshCost::of(&cands[0].mesh).unwrap().memory_bytes(), MeshCost::of(&cands[1].mesh).unwrap().memory_bytes());
        assert!(fine_mem > coarse_mem);
        // Unconstrained, the coarse mesh is cheaper; a limit between the two memories excludes the fine one anyway and says why.
        let mid = (fine_mem + coarse_mem) / 2;
        let c = choose_within(&cands, &model, &work, false, Limits { max_memory_bytes: Some(mid), max_seconds: None }).unwrap();
        assert_eq!(c.index, 1);
        assert!(c.rejected.iter().any(|(i, why)| *i == 0 && why.contains("memory")), "{:?}", c.rejected);
        // Nothing fits under a limit below both: an error, not an infeasible pick, including for the baseline.
        let err = choose_within(&cands, &model, &work, false, Limits { max_memory_bytes: Some(coarse_mem - 1), max_seconds: None }).unwrap_err();
        assert!(format!("{err:?}").contains("no candidate satisfies"));
        // A time limit works the same way.
        assert!(choose_within(&cands, &model, &work, false, Limits { max_memory_bytes: None, max_seconds: Some(0.0) }).is_err());
        // No limits behaves exactly like `choose`.
        assert_eq!(choose_within(&cands, &model, &work, false, Limits::default()).unwrap().index, choose(&cands, &model, &work, false).unwrap().index);
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
