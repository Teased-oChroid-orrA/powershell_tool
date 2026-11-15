//! Adaptivity support for small-strain plasticity (J2): the tangent-transpose adjoint of the converged state and the transfer
//! of the material history to a new mesh with a check that the transferred state is admissible and re-equilibrates.
//!
//! **Tangent adjoint.** At a converged state `R(u; lambda) = f_int(u) - lambda f_ext = 0` with the consistent tangent
//! `K_T = dR/du`, the adjoint of a goal `J(u) = g . u` is `K_T^T z = g` (`K_T` is symmetric for associative plasticity without
//! contact friction, so it is `K_T z = g`). The sensitivity of `J` to the load factor is then `dJ/dlambda = z . f_ext`, which the
//! tests compare with a finite difference of the nonlinear solve. The tangent is the one of the *last accepted step* (the return
//! map depends on the state it started from), so `NlSolution::prev_state` is required for a multi-step solution. Contact states
//! are not part of this tangent and are refused.
//!
//! **History transfer.** Plastic strain and equivalent plastic strain are smooth fields; a new Gauss point takes the
//! inverse-distance average of the nearest old Gauss points of the same block (a convex combination, so the equivalent plastic
//! strain stays non-negative and the deviatoric plastic strain stays trace-free). The transferred state is not trusted: it is
//! used as the start of a zero-increment Newton solve whose first residual is the transfer error, which is reported; a transfer
//! that does not re-equilibrate leaves the caller to solve from the virgin state (`Unsupported` is never replaced by a guess).

use super::goal::LinearFunctional;
use super::snapshot::SolverSnapshot;
use super::transfer::{NodalInterpolation, StateTransferOperator};
use crate::analysis::Model;
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::Loads;
use crate::material::GpState;
use crate::meshing::MeshError;
use crate::nonlinear::{assemble_nl_public, NlOptions, NlSolution, NlState, Start};
use std::collections::HashMap;

/// Result of the adjoint solve at a converged nonlinear state.
pub struct TangentAdjoint {
    pub z: Vec<f64>,
    /// `|K_T z - g| / |g|` over the free dofs.
    pub residual: f64,
}

/// Solve `K_T z = g` at the converged state of `sol`.
pub fn tangent_adjoint(model: &Model, bc: &Dirichlet, sol: &NlSolution, goal: &LinearFunctional) -> Result<TangentAdjoint, MeshError> {
    if !sol.state.contact.is_empty() {
        return Err(MeshError::Unsupported("the tangent adjoint of a contact state (the contact stiffness is not assembled here)".into()));
    }
    if goal.gradient.len() != model.mesh.n_dofs() {
        return Err(MeshError::Failed(format!("goal '{}' has {} gradient entries for {} dofs", goal.name, goal.gradient.len(), model.mesh.n_dofs())));
    }
    let virgin = NlState::new(&model.mesh);
    let old = sol.prev_state.as_ref().unwrap_or(&virgin);
    let mut new = NlState::new(&model.mesh);
    let (_, k) = assemble_nl_public(&model.mesh, &model.pattern, &sol.u, old, &mut new, true).map_err(MeshError::Failed)?;
    let k = k.ok_or_else(|| MeshError::Failed("no tangent was assembled".into()))?;
    let red = Reduced::with_ordering(&model.pattern, bc, Some(&model.mesh.nodes), Ordering::Auto).map_err(|e| MeshError::Failed(e.to_string()))?;
    let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
    // the same tangent solver the optimisation sensitivities use (`derivative::path`): one adjoint code path
    let solver = crate::derivative::linalg::TangentSolver::new(&red, &model.pattern, &k, model.mesh.n_dofs(), &free, &[]).map_err(|e| MeshError::Failed(e.to_string()))?;
    let rhs: Vec<f64> = free.iter().map(|&d| goal.gradient[d]).collect();
    let rhs = solver.solve_transpose(&rhs).map_err(MeshError::Failed)?;
    let mut z = vec![0.0; model.mesh.n_dofs()];
    for (&d, v) in free.iter().zip(&rhs) {
        z[d] = *v;
    }
    let mut kz = vec![0.0; z.len()];
    k.matvec_add(&model.pattern, &z, &mut kz);
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for &d in &free {
        num += (kz[d] - goal.gradient[d]).powi(2);
        den += goal.gradient[d].powi(2);
    }
    Ok(TangentAdjoint { z, residual: if den > 0.0 { (num / den).sqrt() } else { 0.0 } })
}

/// A material history and displacement field on a new mesh.
pub struct HistoryTransfer {
    pub state: NlState,
    pub u: Vec<f64>,
    /// New nodes the old mesh does not contain (their displacement is zero).
    pub unlocated_nodes: usize,
    /// Largest equivalent plastic strain before and after.
    pub max_ep: (f64, f64),
}

struct Cloud {
    cell: f64,
    origin: [f64; 3],
    bins: HashMap<[i64; 3], Vec<usize>>,
    pts: Vec<[f64; 3]>,
    vals: Vec<GpState>,
}

impl Cloud {
    fn new(pts: Vec<[f64; 3]>, vals: Vec<GpState>, cell: f64) -> Self {
        let mut origin = [f64::INFINITY; 3];
        for p in &pts {
            for c in 0..3 {
                origin[c] = origin[c].min(p[c]);
            }
        }
        let mut cl = Self { cell: cell.max(1e-300), origin, bins: HashMap::new(), pts, vals };
        for i in 0..cl.pts.len() {
            let k = cl.key(cl.pts[i]);
            cl.bins.entry(k).or_default().push(i);
        }
        cl
    }

    fn key(&self, x: [f64; 3]) -> [i64; 3] {
        std::array::from_fn(|c| ((x[c] - self.origin[c]) / self.cell).floor() as i64)
    }

    /// The `k` nearest points, searching rings of cells until `k` are found and no unvisited cell can be closer.
    fn nearest(&self, x: [f64; 3], k: usize) -> Vec<(f64, usize)> {
        let c = self.key(x);
        let mut found: Vec<(f64, usize)> = Vec::new();
        for ring in 0..64i64 {
            for i in -ring..=ring {
                for j in -ring..=ring {
                    for l in -ring..=ring {
                        if i.abs().max(j.abs()).max(l.abs()) != ring {
                            continue;
                        }
                        if let Some(v) = self.bins.get(&[c[0] + i, c[1] + j, c[2] + l]) {
                            for &p in v {
                                let d2: f64 = (0..3).map(|a| (self.pts[p][a] - x[a]).powi(2)).sum();
                                found.push((d2, p));
                            }
                        }
                    }
                }
            }
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            found.truncate(k);
            // every point of an unvisited ring is farther than `ring * cell`
            if found.len() == k && found[k - 1].0.sqrt() <= ring as f64 * self.cell {
                break;
            }
            if ring > 8 && found.len() == k {
                break;
            }
        }
        found
    }
}

/// Transfer the displacements and the plastic history of `old` (state `old_state`, displacements `old_u`) to the mesh of `new`.
/// Blocks are matched by name, then by position. Contact states are not transferred.
pub fn transfer_history(old: &Model, old_u: &[f64], old_state: &NlState, new: &Model, neighbours: usize) -> Result<HistoryTransfer, MeshError> {
    if !old_state.contact.is_empty() {
        return Err(MeshError::Unsupported("contact states are not transferred between meshes".into()));
    }
    if old.mesh.dim() != new.mesh.dim() {
        return Err(MeshError::Failed("the meshes have different dimensions".into()));
    }
    let k = neighbours.max(1);
    let mut state = NlState::new(&new.mesh);
    let snap = SolverSnapshot::new(&old.mesh, old_u.to_vec(), 0.0, "old").map_err(MeshError::Failed)?;
    let t = NodalInterpolation { max_unlocated_fraction: 0.2 }.transfer(old, &snap, &new.mesh)?;
    for (bi, blk) in new.mesh.blocks.iter().enumerate() {
        if blk.plasticity.is_none() {
            continue;
        }
        let ob = old.mesh.blocks.iter().position(|b| b.name == blk.name && b.plasticity.is_some()).or_else(|| old.mesh.blocks.get(bi).filter(|b| b.plasticity.is_some()).map(|_| bi)).ok_or_else(|| MeshError::Failed(format!("block '{}' has no plastic counterpart in the old mesh", blk.name)))?;
        let oblk = &old.mesh.blocks[ob];
        let ot = oblk.kind.table();
        let mut pts = Vec::with_capacity(oblk.n_elems() * ot.ngp);
        let mut h = 0.0f64;
        for e in 0..oblk.n_elems() {
            let xyz: Vec<[f64; 3]> = oblk.elem(e).iter().map(|&n| old.mesh.nodes[n]).collect();
            for g in 0..ot.ngp {
                let n = ot.n_at(g);
                pts.push(std::array::from_fn(|c| (0..xyz.len()).map(|a| n[a] * xyz[a][c]).sum()));
            }
            let size = (0..3).map(|c| xyz.iter().map(|p| p[c]).fold(f64::NEG_INFINITY, f64::max) - xyz.iter().map(|p| p[c]).fold(f64::INFINITY, f64::min)).fold(0.0f64, f64::max);
            h += size;
        }
        let cloud = Cloud::new(pts, old_state.gp[ob].clone(), h / oblk.n_elems().max(1) as f64);
        let nt = blk.kind.table();
        let dst = &mut state.gp[bi];
        for e in 0..blk.n_elems() {
            let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| new.mesh.nodes[n]).collect();
            for g in 0..nt.ngp {
                let n = nt.n_at(g);
                let x: [f64; 3] = std::array::from_fn(|c| (0..xyz.len()).map(|a| n[a] * xyz[a][c]).sum());
                let near = cloud.nearest(x, k);
                let mut acc = [0.0f64; 7];
                let mut wsum = 0.0;
                for &(d2, p) in &near {
                    let w = 1.0 / (d2 + 1e-24);
                    for (a, v) in acc.iter_mut().zip(&cloud.vals[p]) {
                        *a += w * v;
                    }
                    wsum += w;
                }
                for a in acc.iter_mut() {
                    *a /= wsum;
                }
                acc[6] = acc[6].max(0.0);
                dst[e * nt.ngp + g] = acc;
            }
        }
    }
    let unlocated = t.unlocated.len();
    Ok(HistoryTransfer { max_ep: (old_state.max_ep(), state.max_ep()), state, u: t.snapshot.u, unlocated_nodes: unlocated })
}

/// Warm start for **monotone proportional loading**, where the plastic state at the final load is (nearly) a function of the final
/// strain alone: carry only the displacements to the new mesh and take the whole load in one step from the virgin material
/// state (the return map of a single step reproduces the path to a small error). No history is interpolated, so there is no
/// transfer error to re-equilibrate; for a history that is not monotone and proportional it is wrong and
/// [`transfer_history`] (approximate, re-equilibrated, never exact) is the only option.
pub fn warm_start_displacement(old: &Model, old_u: &[f64], new: &Model, loads: &Loads, bc: &Dirichlet, opt: &NlOptions) -> Result<NlSolution, MeshError> {
    let snap = SolverSnapshot::new(&old.mesh, old_u.to_vec(), 0.0, "old").map_err(MeshError::Failed)?;
    let t = NodalInterpolation { max_unlocated_fraction: 0.2 }.transfer(old, &snap, &new.mesh)?;
    let start = Start { u: t.snapshot.u, contact: Vec::new(), gp: None, lambda: 0.0 };
    let one_step = NlOptions { steps: 1, ..opt.clone() };
    new.solve_nonlinear_contact_from(loads, bc, Vec::new(), &one_step, Some(&start)).map_err(MeshError::Failed)
}

/// How the transferred state behaved when it was re-equilibrated on the new mesh at the same load.
pub struct ReEquilibration {
    /// Relative residual of the transferred state (the transfer error) before any iteration.
    pub initial_residual: f64,
    pub iterations: usize,
    pub solution: NlSolution,
}

/// Re-equilibrate a transferred state at the load factor the solution was in equilibrium at (a final tiny load step from the
/// transferred state, the driver's own continuation). `lambda` is the load factor of the transferred state (1 for a complete
/// run). Errors if the continuation fails.
pub fn reequilibrate(new: &Model, loads: &Loads, bc: &Dirichlet, transfer: &HistoryTransfer, lambda: f64, opt: &NlOptions) -> Result<ReEquilibration, MeshError> {
    let start = Start { u: transfer.u.clone(), contact: Vec::new(), gp: Some(transfer.state.gp.clone()), lambda: lambda - 1e-6 };
    let sol = new.solve_nonlinear_contact_from(loads, bc, Vec::new(), opt, Some(&start)).map_err(MeshError::Failed)?;
    let first = sol.steps.first().ok_or_else(|| MeshError::Failed("the continuation took no step".into()))?;
    Ok(ReEquilibration { initial_residual: first.residuals.first().copied().unwrap_or(f64::NAN), iterations: first.iterations, solution: sol })
}

// ---------------------------------------------------------------------------------------------------- the nonlinear loop

type GoalEval = dyn Fn(&NlProblem, &NlSolution) -> f64 + Send + Sync;

use super::controller::{element_sizes, state_check_parts, AdaptSize, MeshedProblem};
use super::estimator::{ErrorField, Indicator, Reliability};
use super::fusion::{fuse, FusionConfig, FusionInput};
use super::marking::{dorfler, trim_to_budget, Budget};
use super::zones::{DefaultZoneResolver, PhysicsEstimator, PhysicsZoneResolver, Zone, ZoneSpec};
use crate::adapt::{size_field_at_nodes, AdaptOptions, NodalSize};
use crate::contact::ContactSpec;
use crate::element::ElementKind;
use crate::meshing::quality::QualityThresholds;
use crate::meshing::ElementLabels;
use crate::nonlinear::Stop;
use crate::recover::{complementary_energy, error_points};
use std::sync::Arc;
use std::time::Instant;

/// A nonlinear analysis on one mesh: model, loads, constraints and contact interfaces (rebuilt by the mesher on every mesh).
pub struct NlProblem {
    pub model: Model,
    pub loads: Loads,
    pub bc: Dirichlet,
    pub contacts: Vec<ContactSpec>,
    pub labels: Option<ElementLabels>,
    pub quad_scale: f64,
}

/// A quantity read off a nonlinear solution; the loop watches its change from mesh to mesh (there is no adjoint estimate of a
/// nonlinear or contact quantity here, so the stopping evidence for it is the successive difference, and it is reported as such).
#[derive(Clone)]
pub struct NlGoal {
    pub name: String,
    pub eval: Arc<GoalEval>,
    /// Allowed relative change between consecutive meshes.
    pub rel_change: f64,
}

impl NlGoal {
    pub fn new(name: &str, rel_change: f64, eval: impl Fn(&NlProblem, &NlSolution) -> f64 + Send + Sync + 'static) -> Self {
        Self { name: name.to_string(), eval: Arc::new(eval), rel_change }
    }

    /// Normal force transmitted through contact interface `spec`: the sum of pressure times weight.
    pub fn contact_force(spec: usize, rel_change: f64) -> Self {
        Self::new("contact force", rel_change, move |p, s| s.contact_tractions(spec, p.contacts.get(spec).map_or(0.0, |c| c.mu)).iter().map(|t| t.pressure * t.weight).sum())
    }

    /// Half the extent of the active contact points of interface `spec` along its largest direction.
    pub fn contact_halfwidth(spec: usize, rel_change: f64) -> Self {
        Self::new("contact half-width", rel_change, move |p, s| active_extent(&s.contact_tractions(spec, p.contacts.get(spec).map_or(0.0, |c| c.mu))).0 * 0.5)
    }

    /// Largest contact pressure of interface `spec`.
    pub fn peak_pressure(spec: usize, rel_change: f64) -> Self {
        Self::new("peak contact pressure", rel_change, move |p, s| s.contact_tractions(spec, p.contacts.get(spec).map_or(0.0, |c| c.mu)).iter().map(|t| t.pressure).fold(0.0, f64::max))
    }

    pub fn mean_displacement(name: &str, node_set: &str, comp: usize, rel_change: f64) -> Self {
        let set = node_set.to_string();
        Self::new(name, rel_change, move |p, s| {
            let d = p.model.mesh.dim();
            p.model.mesh.node_sets.get(&set).map_or(f64::NAN, |nodes| nodes.iter().map(|&n| s.u[n * d + comp]).sum::<f64>() / nodes.len() as f64)
        })
    }

    pub fn max_plastic_strain(rel_change: f64) -> Self {
        Self::new("maximum plastic strain", rel_change, |_, s| s.state.max_ep())
    }
}

/// `(extent, centre)` of the active points of an interface: the length of the bounding-box diagonal of their reference positions.
fn active_extent(t: &[crate::contact::ContactTraction]) -> (f64, [f64; 3]) {
    let act: Vec<&crate::contact::ContactTraction> = t.iter().filter(|c| c.active && c.pressure > 0.0).collect();
    if act.is_empty() {
        return (0.0, [0.0; 3]);
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for c in &act {
        for k in 0..3 {
            lo[k] = lo[k].min(c.x[k]);
            hi[k] = hi[k].max(c.x[k]);
        }
    }
    let ext = (0..3).map(|k| (hi[k] - lo[k]).powi(2)).sum::<f64>().sqrt();
    (ext, std::array::from_fn(|k| 0.5 * (lo[k] + hi[k])))
}

pub struct NlAmrConfig {
    pub nl: NlOptions,
    pub fusion: FusionConfig,
    pub theta: f64,
    pub max_cycles: usize,
    /// Relative energy-norm error of the stress (physics criterion); `None` = not required.
    pub physics_tolerance: Option<f64>,
    pub budget: Budget,
    pub refine_ratio: f64,
    pub max_growth: f64,
    pub adapt: AdaptOptions,
    pub zones: ZoneSpec,
    pub quality: QualityThresholds,
    pub state_tolerance: f64,
    /// Elements near active contact must be at most `half-width / contact_cells_across` (mandatory refinement otherwise).
    pub contact_cells_across: f64,
    /// For plastic problems without contact, start each new mesh from the displacements of the previous one in a single load
    /// step (valid for monotone proportional loading only; see [`warm_start_displacement`]).
    pub warm_start: bool,
}

impl Default for NlAmrConfig {
    fn default() -> Self {
        Self {
            nl: NlOptions::default(),
            fusion: FusionConfig { alpha: 0.0, ..FusionConfig::default() },
            theta: 0.5,
            max_cycles: 6,
            physics_tolerance: None,
            budget: Budget::default(),
            refine_ratio: 0.5,
            max_growth: 3.0,
            adapt: AdaptOptions { target_rel_error: 0.0, order: 2.0, min_factor: 0.0, max_factor: 1.0, grading: 0.4, h_min: 0.0, h_max: f64::INFINITY },
            zones: ZoneSpec { detect_geometry: false, ..ZoneSpec::default() },
            quality: QualityThresholds::default(),
            state_tolerance: 0.02,
            contact_cells_across: 6.0,
            warm_start: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NlCycle {
    pub cycle: usize,
    pub elements: usize,
    pub dofs: usize,
    pub goals: Vec<(String, f64)>,
    /// Relative change of each goal from the previous mesh (`NAN` on the first).
    pub goal_change: Vec<f64>,
    pub physics_error: f64,
    pub max_ep: f64,
    pub plastic_fraction: f64,
    pub contact_active: usize,
    pub contact_halfwidth: f64,
    /// Largest `|F_master + sum (p n + t) w| / |F_master|` over the interfaces: the transmitted traction against the force on the
    /// master (0 without contact). The loop refuses a solution where it exceeds `1e-3`.
    pub contact_balance: f64,
    pub load_steps: usize,
    pub newton_iterations: usize,
    pub factorisations: usize,
    pub marked: usize,
    pub mandatory: usize,
    pub contact_protected: usize,
    pub solve_seconds: f64,
    pub warm_started: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NlStop {
    Converged,
    MaxCycles,
    BudgetExhausted,
    NothingToRefine,
    /// The first mesh could not be solved to the full load.
    SolveFailed(String),
    /// A candidate mesh failed a check or its solve; the previous state is returned.
    RolledBack(String),
}

pub struct NlResult {
    pub problem: NlProblem,
    pub solution: NlSolution,
    pub stop: NlStop,
    pub history: Vec<NlCycle>,
}

/// Error indicators of a nonlinear state: SPR+ recovery of the Gauss-point stresses against the stress of the state evaluated at
/// the error-rule points (a plastic point takes the history of its nearest Gauss point). Small-strain plasticity only.
fn nl_zz(p: &NlProblem, sol: &NlSolution) -> Result<(Vec<f64>, f64), MeshError> {
    nl_zz_of(&p.model, sol)
}

/// The recovery-based error estimate of the converged state `sol` of `model` (small-strain plasticity; see [`nl_zz`]): element
/// energy-norm indicators against the energy norm of the recovered stress, in the shape the linear loop reads.
pub fn nl_zz_estimate(model: &Model, sol: &NlSolution) -> Result<crate::recover::ZzEstimate, MeshError> {
    let (eta, norm) = nl_zz_of(model, sol)?;
    let total = eta.iter().map(|v| v * v).sum::<f64>().sqrt();
    Ok(crate::recover::ZzEstimate { eta, total, norm })
}

fn nl_zz_of(model: &Model, sol: &NlSolution) -> Result<(Vec<f64>, f64), MeshError> {
    let mesh = &model.mesh;
    let d = mesh.dim();
    let virgin = NlState::new(mesh);
    let old = sol.prev_state.as_ref().unwrap_or(&virgin);
    let gauss = model.nl_gauss_stresses(&sol.u, old).map_err(MeshError::Failed)?;
    let rec = model.recover_spr_by_material(&gauss).map_err(MeshError::Failed)?;
    let mut eta = Vec::with_capacity(mesh.n_elems());
    let mut norm2 = 0.0;
    for (bi, blk) in mesh.blocks.iter().enumerate() {
        let nn = blk.kind.n_nodes();
        let t = blk.kind.table();
        let mut xyz = vec![[0.0; 3]; nn];
        let mut ue = vec![0.0; nn * d];
        for e in 0..blk.n_elems() {
            let conn = blk.elem(e);
            for (a, &nd) in conn.iter().enumerate() {
                xyz[a] = mesh.nodes[nd];
                for i in 0..d {
                    ue[a * d + i] = sol.u[nd * d + i];
                }
            }
            let (mut e2, mut n2) = (0.0, 0.0);
            for pt in error_points(blk.kind, mesh.physics, &xyz).map_err(MeshError::Failed)? {
                let mut star = [0.0; 6];
                for (a, &nd) in conn.iter().enumerate() {
                    let r = rec.at(bi, nd);
                    for c in 0..6 {
                        star[c] += pt.n[a] * r[c];
                    }
                }
                let h = if blk.plasticity.is_some() {
                    // history of the nearest Gauss point of this element
                    let g = (0..t.ngp).min_by(|&a, &b| {
                        let da: f64 = (0..3).map(|c| (t.xi[a][c] - pt.xi[c]).powi(2)).sum();
                        let db: f64 = (0..3).map(|c| (t.xi[b][c] - pt.xi[c]).powi(2)).sum();
                        da.total_cmp(&db)
                    });
                    let g = g.expect("an element has Gauss points");
                    crate::nonlinear::plastic_stress_at(blk, mesh.physics, &xyz, &ue, pt.xi, &old.gp[bi][e * t.ngp + g]).map_err(MeshError::Failed)?.0
                } else {
                    crate::analysis::stress_at_xi(blk.kind, mesh.physics, &blk.material, &xyz, &ue, pt.xi, 0.0).map_err(MeshError::Failed)?
                };
                let diff: [f64; 6] = std::array::from_fn(|c| star[c] - h[c]);
                e2 += pt.weight * complementary_energy(&blk.material, &diff) * 2.0;
                n2 += pt.weight * complementary_energy(&blk.material, &star) * 2.0;
            }
            eta.push(e2.sqrt());
            norm2 += n2;
        }
    }
    Ok((eta, norm2.sqrt()))
}

/// Elements that must be refined because they carry the contact patch at too coarse a size, and the patch half-width used.
fn contact_protection(p: &NlProblem, sol: &NlSolution, h_elem: &[f64], cells_across: f64) -> (Vec<bool>, f64, usize) {
    let n = p.model.mesh.n_elems();
    let mut protected = vec![false; n];
    let mut halfwidth = 0.0f64;
    let mut active_total = 0usize;
    let mut centres: Vec<([f64; 3], f64)> = Vec::new();
    for (i, spec) in p.contacts.iter().enumerate() {
        let tr = sol.contact_tractions(i, spec.mu);
        let (ext, centre) = active_extent(&tr);
        active_total += tr.iter().filter(|t| t.active && t.pressure > 0.0).count();
        if ext > 0.0 {
            halfwidth = halfwidth.max(0.5 * ext);
            centres.push((centre, 0.5 * ext));
        } else if let Some(t) = tr.iter().min_by(|a, b| a.pressure.total_cmp(&b.pressure).reverse()) {
            // no active point: the contact is not resolved at all; refine around the points that came closest (the largest
            // pressure of a multiplier pass, else every point of the interface is a candidate)
            centres.push((t.x, h_elem.iter().cloned().fold(0.0f64, f64::max)));
        }
    }
    if centres.is_empty() {
        return (protected, 0.0, 0);
    }
    let required = if halfwidth > 0.0 { halfwidth / cells_across } else { 0.0 };
    let mesh = &p.model.mesh;
    let mut g = 0;
    for blk in &mesh.blocks {
        let nc = blk.kind.n_corners();
        for e in 0..blk.n_elems() {
            let c: [f64; 3] = std::array::from_fn(|k| blk.elem(e)[..nc].iter().map(|&nd| mesh.nodes[nd][k]).sum::<f64>() / nc as f64);
            for &(x0, a) in &centres {
                let dist = (0..3).map(|k| (c[k] - x0[k]).powi(2)).sum::<f64>().sqrt();
                if dist <= 2.0 * a + h_elem[g] && h_elem[g] > required.max(f64::MIN_POSITIVE) {
                    protected[g] = true;
                }
            }
            g += 1;
        }
    }
    (protected, halfwidth, active_total)
}

struct NlEval {
    sol: NlSolution,
    goals: Vec<f64>,
    physics_rel: f64,
    fused: super::fusion::Fused,
    protected: Vec<bool>,
    halfwidth: f64,
    active: usize,
    balance: f64,
    h_elem: Vec<f64>,
    seconds: f64,
    warm: bool,
}

fn solve_problem(p: &NlProblem, cfg: &NlAmrConfig, warm: Option<(&Model, &[f64])>) -> Result<(NlSolution, bool), MeshError> {
    if let Some((old, u)) = warm {
        if cfg.warm_start && p.contacts.is_empty() {
            let sol = warm_start_displacement(old, u, &p.model, &p.loads, &p.bc, &cfg.nl)?;
            if sol.stop == Stop::Completed {
                return Ok((sol, true));
            }
        }
    }
    let sol = p.model.solve_nonlinear_contact(&p.loads, &p.bc, p.contacts.clone(), &cfg.nl).map_err(MeshError::Failed)?;
    Ok((sol, false))
}

fn nl_evaluate(p: &NlProblem, goals: &[NlGoal], cfg: &NlAmrConfig, warm: Option<(&Model, &[f64])>) -> Result<NlEval, MeshError> {
    let t = Instant::now();
    let (sol, warm_started) = solve_problem(p, cfg, warm)?;
    let seconds = t.elapsed().as_secs_f64();
    if sol.stop != Stop::Completed {
        return Err(MeshError::Failed(format!("the nonlinear solve did not reach the full load: {:?}", sol.stop)));
    }
    // contact traction verification: what the interface transmits is what the master carries
    let mut balance = 0.0f64;
    if let Some(stats) = &sol.contact {
        for (i, spec) in p.contacts.iter().enumerate() {
            let tr = sol.contact_tractions(i, spec.mu);
            let carried = stats.master_force.get(i).copied().unwrap_or([0.0; 3]);
            let sum: [f64; 3] = std::array::from_fn(|k| tr.iter().map(|t| (t.pressure * t.normal[k] + t.friction[k]) * t.weight).sum());
            let scale = carried.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-300);
            let off = (0..3).map(|k| (carried[k] + sum[k]).powi(2)).sum::<f64>().sqrt() / scale;
            balance = balance.max(off);
        }
        if balance > 1e-3 {
            return Err(MeshError::Failed(format!("the contact tractions do not balance the master force (relative difference {balance:.2e})")));
        }
    }
    let adj = p.model.mesh.adjacency();
    let (eta, norm) = nl_zz(p, &sol)?;
    let total = eta.iter().map(|v| v * v).sum::<f64>().sqrt();
    let physics_rel = total / norm.hypot(total).max(1e-300);
    let field = ErrorField::new("nl-zz", Indicator::Marking, eta, total, Reliability::Degraded("recovery-based indicator of a nonlinear state (history of the nearest Gauss point at off-Gauss points)".into()), 0.0);
    let zones = DefaultZoneResolver { spec: cfg.zones.clone() }.resolve(&p.model.mesh, &adj)?;
    let h_elem = element_sizes_nl(p)?;
    let (protected, halfwidth, active) = contact_protection(p, &sol, &h_elem, cfg.contact_cells_across);
    let n = p.model.mesh.n_elems();
    let mut fcfg = cfg.fusion.clone();
    if let (Some(tol), None) = (cfg.physics_tolerance, fcfg.physics_allowance) {
        fcfg.physics_allowance = Some(tol * norm.hypot(total) / (n as f64).sqrt());
    }
    let physics: Vec<(PhysicsEstimator, &ErrorField)> = vec![(PhysicsEstimator::SprPlus, &field)];
    let capped: Vec<bool> = (0..n).map(|e| cfg.adapt.h_min > 0.0 && h_elem[e] * cfg.refine_ratio < cfg.adapt.h_min).collect();
    let fused = fuse(&FusionInput { adjoint: &[], physics: &physics, zones: Some(&zones), capped: Some(&capped), protected: Some(&protected) }, &fcfg)?;
    let values: Vec<f64> = goals.iter().map(|g| (g.eval)(p, &sol)).collect();
    let _ = Zone::Smooth;
    Ok(NlEval { sol, goals: values, physics_rel, fused, protected, halfwidth, active, balance, h_elem, seconds, warm: warm_started })
}

fn element_sizes_nl(p: &NlProblem) -> Result<Vec<f64>, MeshError> {
    // reuse the linear loop's size-function convention through a temporary problem view
    let view = MeshedProblem { model: p.model.clone(), loads: Loads::default(), bc: p.model.dirichlet(), labels: None, quad_scale: p.quad_scale };
    let _ = ElementKind::Tri3;
    element_sizes(&view)
}

/// Adaptive loop for plastic and contact problems; see [`NlAmrConfig`]. The indicator is the recovered-stress energy of the
/// nonlinear state (`Degraded`: not an error bound), contact patches are protected by a mandatory-resolution rule, and the
/// quantities in `goals` are judged by their change from mesh to mesh.
pub fn run_amr_nonlinear(initial: NlProblem, goals: &[NlGoal], mesher: &mut dyn FnMut(&AdaptSize) -> Result<NlProblem, String>, cfg: &NlAmrConfig) -> Result<NlResult, MeshError> {
    if goals.is_empty() {
        return Err(MeshError::Failed("at least one goal is required".into()));
    }
    let dim = initial.model.mesh.dim();
    let children = (1.0 / cfg.refine_ratio).powi(dim as i32);
    let mut current = initial;
    let mut ev = nl_evaluate(&current, goals, cfg, None).map_err(|e| MeshError::Failed(format!("the first mesh failed: {e}")))?;
    let mut history: Vec<NlCycle> = Vec::new();
    let mut prev_goals: Option<Vec<f64>> = None;
    for cycle in 0..=cfg.max_cycles {
        let change: Vec<f64> = match &prev_goals {
            Some(pg) => ev.goals.iter().zip(pg).map(|(a, b)| (a - b).abs() / b.abs().max(f64::MIN_POSITIVE)).collect(),
            None => vec![f64::NAN; goals.len()],
        };
        let steps = ev.sol.steps.len();
        let newton: usize = ev.sol.steps.iter().map(|s| s.iterations).sum();
        let mut rec = NlCycle {
            cycle,
            elements: current.model.mesh.n_elems(),
            dofs: current.model.mesh.n_dofs(),
            goals: goals.iter().zip(&ev.goals).map(|(g, v)| (g.name.clone(), *v)).collect(),
            goal_change: change.clone(),
            physics_error: ev.physics_rel,
            max_ep: ev.sol.state.max_ep(),
            plastic_fraction: ev.sol.state.plastic_fraction(),
            contact_active: ev.active,
            contact_halfwidth: ev.halfwidth,
            contact_balance: ev.balance,
            load_steps: steps,
            newton_iterations: newton,
            factorisations: ev.sol.factorisations,
            marked: 0,
            mandatory: ev.fused.mandatory.iter().filter(|&&m| m).count(),
            contact_protected: ev.protected.iter().filter(|&&m| m).count(),
            solve_seconds: ev.seconds,
            warm_started: ev.warm,
        };
        let finish = |mut history: Vec<NlCycle>, rec: NlCycle, stop: NlStop, current: NlProblem, ev: NlEval| -> Result<NlResult, MeshError> {
            history.push(rec);
            Ok(NlResult { problem: current, solution: ev.sol, stop, history: std::mem::take(&mut history) })
        };
        let goals_ok = prev_goals.is_some() && goals.iter().zip(&change).all(|(g, c)| *c <= g.rel_change);
        let physics_ok = cfg.physics_tolerance.is_none_or(|t| ev.physics_rel <= t);
        let contact_ok = !ev.protected.iter().any(|&b| b);
        if goals_ok && physics_ok && contact_ok {
            return finish(history, rec, NlStop::Converged, current, ev);
        }
        if cycle == cfg.max_cycles {
            return finish(history, rec, NlStop::MaxCycles, current, ev);
        }
        let n = current.model.mesh.n_elems();
        let mut marked = dorfler(&ev.fused.score, cfg.theta, &ev.fused.mandatory, false)?;
        let cap = cfg.budget.element_cap(current.model.mesh.n_dofs(), n);
        if !trim_to_budget(&mut marked.marked, &ev.fused.score, &ev.fused.mandatory, n, cap, children) {
            return finish(history, rec, NlStop::BudgetExhausted, current, ev);
        }
        trim_to_budget(&mut marked.marked, &ev.fused.score, &vec![false; n], n, (cfg.max_growth.max(1.0) * n as f64) as usize, children);
        marked.n_marked = marked.marked.iter().filter(|&&m| m).count();
        rec.marked = marked.n_marked;
        if marked.n_marked == 0 {
            return finish(history, rec, NlStop::NothingToRefine, current, ev);
        }
        let h_new: Vec<f64> = (0..n).map(|e| (if marked.marked[e] { ev.h_elem[e] * cfg.refine_ratio } else { ev.h_elem[e] }).clamp(cfg.adapt.h_min.max(f64::MIN_POSITIVE), cfg.adapt.h_max)).collect();
        let size = if dim == 2 {
            size_field_at_nodes(&current.model.mesh, &h_new, &cfg.adapt, NodalSize::Min).map(AdaptSize::D2)
        } else {
            crate::adapt::size_field_at_nodes_3d(&current.model.mesh, &h_new, &cfg.adapt, NodalSize::Min).map(AdaptSize::D3)
        };
        let size = match size {
            Ok(s) => s,
            Err(e) => return finish(history, rec, NlStop::RolledBack(format!("size field: {e}")), current, ev),
        };
        let next = match mesher(&size) {
            Ok(p) => p,
            Err(e) => return finish(history, rec, NlStop::RolledBack(format!("the mesher failed: {e}")), current, ev),
        };
        let checks = (|| -> Result<(), String> {
            next.model.mesh.quality().require(&cfg.quality).map_err(|e| e.to_string())?;
            next.model.mesh.conformity().require_conforming().map_err(|e| e.to_string())?;
            state_check_parts((&current.model, &current.loads, &current.bc), (&next.model, &next.loads, &next.bc), cfg.state_tolerance)?;
            if next.contacts.len() != current.contacts.len() {
                return Err(format!("the contact interfaces changed from {} to {}", current.contacts.len(), next.contacts.len()));
            }
            Ok(())
        })();
        if let Err(why) = checks {
            return finish(history, rec, NlStop::RolledBack(why), current, ev);
        }
        let next_ev = match nl_evaluate(&next, goals, cfg, Some((&current.model, &ev.sol.u))) {
            Ok(e) => e,
            Err(e) => return finish(history, rec, NlStop::RolledBack(format!("the new mesh could not be solved: {e}")), current, ev),
        };
        if !cfg.budget.allows(next.model.mesh.n_dofs(), next.model.mesh.n_elems(), usize::MAX / 2) {
            return finish(history, rec, NlStop::BudgetExhausted, current, ev);
        }
        prev_goals = Some(ev.goals.clone());
        history.push(rec);
        current = next;
        ev = next_ev;
    }
    unreachable!("the loop returns at max_cycles")
}
