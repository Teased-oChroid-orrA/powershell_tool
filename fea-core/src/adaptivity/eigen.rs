//! Adaptive refinement for eigenvalue problems (natural frequencies; buckling with a stated limitation).
//!
//! For a mass-normalised eigenpair `(w, phi_h)` the Rayleigh quotient satisfies `w_h - w = a(e, e) - w m(e, e)` with
//! `e = phi - phi_h`; the mass term is of higher order in `h` for vibration, so to leading order the eigenvalue error is the
//! squared energy-norm error of the mode shape. That is estimated by the stress-recovery estimate of `phi_h` (SPR+, material
//! aware): `eta_e` per element, `w - w_h ~ -sum eta_e^2`, and `sum eta_e^2` is also the marking indicator. The problem is
//! self-adjoint, so no dual solve is needed.
//!
//! **Mode tracking.** After a remesh the mode being followed is found again by the modal assurance criterion between the old
//! shape carried onto the new mesh and the new modes, in the new mass inner product; a best match below the threshold stops
//! the run (`ModeLost`) instead of silently following a different mode. **Repeated eigenvalues** make single modes undefined:
//! modes whose eigenvalues agree within a tolerance form a cluster, the estimate is that of the whole cluster (the sum of the
//! squared errors), the cluster is tracked as a set, and the result is `Degraded`.
//!
//! **Buckling.** `a(e, e) - lambda b(e, e)` has a geometric-stiffness term of the same order as the energy term, which this
//! estimate does not include: a buckling indicator is useful for marking, its value is not an error bound (`Degraded`).

use super::controller::{element_sizes, state_check, AdaptSize, MeshedProblem};
use super::estimator::Reliability;
use super::marking::{dorfler, trim_to_budget, Budget};
use super::snapshot::SolverSnapshot;
use super::transfer::{NodalInterpolation, StateTransferOperator};
use crate::adapt::{size_field_at_nodes, size_field_at_nodes_3d, AdaptOptions, NodalSize};
use crate::analysis::Model;
use crate::dynamics::{Mode, ModalOptions};
use crate::meshing::quality::QualityThresholds;
use crate::meshing::MeshError;

/// Estimate for one mode (or one cluster of repeated modes).
#[derive(Debug, Clone)]
pub struct ModeEstimate {
    /// Indices of the modes the estimate covers (more than one for a cluster).
    pub modes: Vec<usize>,
    /// Sum of the eigenvalues `omega^2` of those modes.
    pub value: f64,
    /// `sum eta^2`: estimated `value_h - value` (positive: frequencies of a conforming mesh converge from above).
    pub error: f64,
    /// `error / value`.
    pub relative_error: f64,
    /// `value - error`.
    pub corrected: f64,
    /// Per element `sum eta_e^2` over the covered modes.
    pub indicator: Vec<f64>,
    pub reliability: Reliability,
}

/// Groups of consecutive modes whose eigenvalues agree within `rel_tol` of each other (singletons included), as index ranges.
pub fn clusters(omega2: &[f64], rel_tol: f64) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..=omega2.len() {
        let split = i == omega2.len() || (omega2[i] - omega2[i - 1]).abs() > rel_tol * omega2[i - 1].abs().max(f64::MIN_POSITIVE);
        if split {
            out.push((start, i - 1));
            start = i;
        }
    }
    out
}

/// Estimate the error of modes `first..=last` (a cluster, or a single mode when `first == last`).
pub fn estimate_modes(model: &Model, modes: &[Mode], first: usize, last: usize) -> Result<ModeEstimate, MeshError> {
    let mut indicator = vec![0.0; model.mesh.n_elems()];
    let (mut value, mut error) = (0.0, 0.0);
    let mut reliability = Reliability::Reliable;
    for (k, m) in modes.iter().enumerate().take(last + 1).skip(first) {
        let zz = model.zz_error(&m.shape, 0.0).map_err(MeshError::Failed)?;
        if zz.eta.iter().any(|v| !v.is_finite()) {
            reliability = Reliability::Failed(format!("the stress recovery of mode {k} is not finite"));
        }
        for (acc, e) in indicator.iter_mut().zip(&zz.eta) {
            *acc += e * e;
        }
        value += m.omega2;
        error += zz.total * zz.total;
    }
    if last > first && reliability == Reliability::Reliable {
        reliability = Reliability::Degraded(format!("modes {first}..={last} form a cluster of repeated eigenvalues: the estimate is for the cluster, not for a single mode"));
    }
    Ok(ModeEstimate { modes: (first..=last).collect(), value, error, relative_error: error / value.abs().max(f64::MIN_POSITIVE), corrected: value - error, indicator, reliability })
}

/// Modal assurance criterion in the mass inner product `m`: `(a.M.b)^2 / ((a.M.a)(b.M.b))`, 1 for the same shape.
pub fn mac(mass_times: &dyn Fn(&[f64]) -> Vec<f64>, a: &[f64], b: &[f64]) -> f64 {
    let mb = mass_times(b);
    let ma = mass_times(a);
    let dot = |x: &[f64], y: &[f64]| x.iter().zip(y).map(|(p, q)| p * q).sum::<f64>();
    let (ab, aa, bb) = (dot(a, &mb), dot(a, &ma), dot(b, &mb));
    if aa <= 0.0 || bb <= 0.0 { 0.0 } else { ab * ab / (aa * bb) }
}

/// The mode of `new` that continues `old_shape` of `old`: the largest MAC and its value.
pub fn track(old: &Model, old_shape: &[f64], new: &Model, new_modes: &[Mode]) -> Result<(usize, f64), MeshError> {
    let snap = SolverSnapshot::new(&old.mesh, old_shape.to_vec(), 0.0, "mode").map_err(MeshError::Failed)?;
    let carried = NodalInterpolation { max_unlocated_fraction: 0.2 }.transfer(old, &snap, &new.mesh)?.snapshot.u;
    let m = new.assemble_mass(crate::dynamics::MassKind::Consistent).map_err(MeshError::Failed)?;
    let mass_times = |v: &[f64]| -> Vec<f64> {
        let mut out = vec![0.0; v.len()];
        m.matvec_add(&new.pattern, v, &mut out);
        out
    };
    let mut best = (0usize, -1.0f64);
    for (j, mode) in new_modes.iter().enumerate() {
        let v = mac(&mass_times, &carried, &mode.shape);
        if v > best.1 {
            best = (j, v);
        }
    }
    Ok(best)
}

#[derive(Debug, Clone)]
pub struct EigenConfig {
    /// Modes to follow (0-based, in the order of the initial mesh).
    pub modes: Vec<usize>,
    /// Allowed relative error of each followed eigenvalue (or cluster).
    pub rel_tol: f64,
    pub theta: f64,
    pub max_cycles: usize,
    pub budget: Budget,
    pub refine_ratio: f64,
    pub max_growth: f64,
    pub adapt: AdaptOptions,
    pub cluster_tol: f64,
    /// Smallest accepted MAC between a mode and its continuation.
    pub mac_min: f64,
    /// Extra modes computed beyond the highest followed one (they reveal cluster members and mode crossings).
    pub extra_modes: usize,
    pub quality: QualityThresholds,
    pub state_tolerance: f64,
}

impl Default for EigenConfig {
    fn default() -> Self {
        Self {
            modes: vec![0],
            rel_tol: 1e-3,
            theta: 0.5,
            max_cycles: 8,
            budget: Budget::default(),
            refine_ratio: 0.5,
            max_growth: 3.0,
            adapt: AdaptOptions { target_rel_error: 0.0, order: 2.0, min_factor: 0.0, max_factor: 1.0, grading: 0.4, h_min: 0.0, h_max: f64::INFINITY },
            cluster_tol: 1e-3,
            mac_min: 0.8,
            extra_modes: 3,
            quality: QualityThresholds::default(),
            state_tolerance: 0.02,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EigenCycle {
    pub cycle: usize,
    pub dofs: usize,
    pub elements: usize,
    /// Followed modes (indices on this mesh), their eigenvalues, relative error estimates and the MAC with the previous mesh.
    pub tracked: Vec<usize>,
    pub omega2: Vec<f64>,
    pub relative_error: Vec<f64>,
    pub mac: Vec<f64>,
    pub clusters: Vec<(usize, usize)>,
    pub marked: usize,
    pub reliability: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EigenStop {
    Converged,
    MaxCycles,
    BudgetExhausted,
    NothingToRefine,
    /// The continuation of a followed mode on the new mesh is not recognisable (MAC below the threshold).
    ModeLost(String),
    RolledBack(String),
}

pub struct EigenResult {
    pub problem: MeshedProblem,
    pub modes: Vec<Mode>,
    pub tracked: Vec<usize>,
    pub stop: EigenStop,
    pub history: Vec<EigenCycle>,
}

fn modal(p: &MeshedProblem, n: usize) -> Result<Vec<Mode>, MeshError> {
    let opt = ModalOptions { n_modes: n, ..ModalOptions::default() };
    p.model.modal(&p.bc, &opt).map(|m| m.modes).map_err(MeshError::Failed)
}

/// Adaptive refinement of the natural frequencies in `cfg.modes`.
pub fn run_eigen_amr(initial: MeshedProblem, mesher: &mut dyn FnMut(&AdaptSize) -> Result<MeshedProblem, String>, cfg: &EigenConfig) -> Result<EigenResult, MeshError> {
    if cfg.modes.is_empty() {
        return Err(MeshError::Failed("at least one mode must be followed".into()));
    }
    let dim = initial.model.mesh.dim();
    let children = (1.0 / cfg.refine_ratio).powi(dim as i32);
    let n_modes = cfg.modes.iter().max().copied().unwrap_or(0) + 1 + cfg.extra_modes;
    let mut current = initial;
    let mut modes = modal(&current, n_modes)?;
    let mut tracked: Vec<usize> = cfg.modes.clone();
    let mut mac_prev = vec![1.0; tracked.len()];
    let mut history = Vec::new();
    for cycle in 0..=cfg.max_cycles {
        // clusters among the computed modes; every followed mode joins its cluster
        let omega2: Vec<f64> = modes.iter().map(|m| m.omega2).collect();
        let cl = clusters(&omega2, cfg.cluster_tol);
        let mut groups: Vec<(usize, usize)> = Vec::new();
        for &t in &tracked {
            if let Some(&g) = cl.iter().find(|g| g.0 <= t && t <= g.1) {
                if !groups.contains(&g) {
                    groups.push(g);
                }
            }
        }
        let mut estimates = Vec::new();
        for &(a, b) in &groups {
            estimates.push(estimate_modes(&current.model, &modes, a, b)?);
        }
        let mut indicator = vec![0.0; current.model.mesh.n_elems()];
        for e in &estimates {
            for (acc, v) in indicator.iter_mut().zip(&e.indicator) {
                *acc += v / e.value.abs().max(f64::MIN_POSITIVE); // each group counts relative to its own eigenvalue
            }
        }
        let mut rec = EigenCycle {
            cycle,
            dofs: current.model.mesh.n_dofs(),
            elements: current.model.mesh.n_elems(),
            tracked: tracked.clone(),
            omega2: tracked.iter().map(|&t| modes[t].omega2).collect(),
            relative_error: tracked.iter().map(|&t| estimates.iter().find(|e| e.modes.contains(&t)).map_or(f64::NAN, |e| e.relative_error)).collect(),
            mac: mac_prev.clone(),
            clusters: groups.clone(),
            marked: 0,
            reliability: estimates.iter().filter_map(|e| if let Reliability::Degraded(w) | Reliability::Failed(w) = &e.reliability { Some(w.clone()) } else { None }).collect(),
        };
        let result = |history: &mut Vec<EigenCycle>, rec: EigenCycle, stop: EigenStop, current: MeshedProblem, modes: Vec<Mode>, tracked: Vec<usize>| {
            history.push(rec);
            Ok(EigenResult { problem: current, modes, tracked, stop, history: std::mem::take(history) })
        };
        if estimates.iter().any(|e| matches!(e.reliability, Reliability::Failed(_))) {
            return result(&mut history, rec, EigenStop::RolledBack("an eigen-estimate failed".into()), current, modes, tracked);
        }
        if estimates.iter().all(|e| e.relative_error <= cfg.rel_tol) {
            return result(&mut history, rec, EigenStop::Converged, current, modes, tracked);
        }
        if cycle == cfg.max_cycles {
            return result(&mut history, rec, EigenStop::MaxCycles, current, modes, tracked);
        }
        // mark
        let n = current.model.mesh.n_elems();
        let mandatory = vec![false; n];
        let mut marked = dorfler(&indicator, cfg.theta, &mandatory, false)?;
        let cost = crate::meshing::cost::MeshCost::of(&current.model.mesh)?;
        let cap = cfg.budget.element_cap(cost.dofs, n).min((cfg.max_growth.max(1.0) * n as f64) as usize);
        if !trim_to_budget(&mut marked.marked, &indicator, &mandatory, n, cap, children) {
            return result(&mut history, rec, EigenStop::BudgetExhausted, current, modes, tracked);
        }
        marked.n_marked = marked.marked.iter().filter(|&&m| m).count();
        rec.marked = marked.n_marked;
        if marked.n_marked == 0 {
            return result(&mut history, rec, EigenStop::NothingToRefine, current, modes, tracked);
        }
        let h_old = element_sizes(&current)?;
        let h_new: Vec<f64> = (0..n).map(|e| (if marked.marked[e] { h_old[e] * cfg.refine_ratio } else { h_old[e] }).clamp(cfg.adapt.h_min.max(f64::MIN_POSITIVE), cfg.adapt.h_max)).collect();
        let size = if dim == 2 {
            size_field_at_nodes(&current.model.mesh, &h_new, &cfg.adapt, NodalSize::Min).map(AdaptSize::D2)
        } else {
            size_field_at_nodes_3d(&current.model.mesh, &h_new, &cfg.adapt, NodalSize::Min).map(AdaptSize::D3)
        };
        let size = match size {
            Ok(s) => s,
            Err(e) => return result(&mut history, rec, EigenStop::RolledBack(format!("size field: {e}")), current, modes, tracked),
        };
        let next = match mesher(&size) {
            Ok(p) => p,
            Err(e) => return result(&mut history, rec, EigenStop::RolledBack(format!("the mesher failed: {e}")), current, modes, tracked),
        };
        let checks = (|| -> Result<(), String> {
            next.model.mesh.quality().require(&cfg.quality).map_err(|e| e.to_string())?;
            next.model.mesh.conformity().require_conforming().map_err(|e| e.to_string())?;
            state_check(&current, &next, cfg.state_tolerance)
        })();
        if let Err(why) = checks {
            return result(&mut history, rec, EigenStop::RolledBack(why), current, modes, tracked);
        }
        let next_modes = match modal(&next, n_modes) {
            Ok(m) => m,
            Err(e) => return result(&mut history, rec, EigenStop::RolledBack(format!("the new mesh has no modal solution: {e}")), current, modes, tracked),
        };
        // follow every tracked mode; an unrecognisable continuation stops the run
        let mut new_tracked = Vec::new();
        let mut new_mac = Vec::new();
        for &t in &tracked {
            let (j, m) = track(&current.model, &modes[t].shape, &next.model, &next_modes)?;
            if m < cfg.mac_min {
                let why = format!("mode {t} has no continuation on the new mesh (best MAC {m:.3} < {})", cfg.mac_min);
                return result(&mut history, rec, EigenStop::ModeLost(why), current, modes, tracked);
            }
            new_tracked.push(j);
            new_mac.push(m);
        }
        history.push(rec);
        current = next;
        modes = next_modes;
        tracked = new_tracked;
        mac_prev = new_mac;
    }
    unreachable!("the loop returns at max_cycles")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clusters_group_repeated_eigenvalues_within_the_tolerance() {
        let w = [1.0, 2.0, 2.0005, 2.001, 5.0, 9.0, 9.00001];
        let c = clusters(&w, 1e-3);
        assert_eq!(c, vec![(0, 0), (1, 3), (4, 4), (5, 6)]);
        assert_eq!(clusters(&[], 1e-3), vec![]);
        assert_eq!(clusters(&[3.0], 1e-3), vec![(0, 0)]);
        // a chain of neighbours each within tolerance of the previous one is one cluster
        assert_eq!(clusters(&[1.0, 1.0009, 1.0018], 1e-3), vec![(0, 2)]);
    }

    #[test]
    fn mac_is_one_for_a_scaled_copy_and_zero_for_orthogonal_shapes() {
        let identity = |v: &[f64]| v.to_vec();
        assert!((mac(&identity, &[1.0, 2.0, 3.0], &[-2.0, -4.0, -6.0]) - 1.0).abs() < 1e-14);
        assert!(mac(&identity, &[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-14);
        assert_eq!(mac(&identity, &[0.0, 0.0], &[1.0, 0.0]), 0.0);
        let weighted = |v: &[f64]| vec![2.0 * v[0], 0.5 * v[1]];
        assert!((mac(&weighted, &[1.0, 1.0], &[1.0, 1.0]) - 1.0).abs() < 1e-14);
    }
}
