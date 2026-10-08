//! Nonlinear static analysis: J2 plasticity in small or finite strain (`material.rs`), Newton
//! iterations with the consistent tangent, load or displacement control with automatic step cutting,
//! optional post-hoc line search, and Crisfield cylindrical arc-length control for limit points.
//!
//! The internal force and tangent of an element come from one displacement-gradient formulation
//! for every analysis type: `H_mJ = sum_a u_{a,m} dN_a/dX_J` (plus the hoop entry `u_r / r` for
//! axisymmetry), a material update `S(H)`, `f_{a,i} = int S_iJ dN_a/dX_J` and
//! `K_{(a,i),(b,k)} = int (dH/du_ai) : dS/dH : (dH/du_bk)`; `S` is the Cauchy stress (small strain)
//! or the first Piola-Kirchhoff stress (finite strain, total Lagrangian on the reference mesh).
//! Elastic blocks of a nonlinear model reuse the optimised linear element kernels.
//!
//! Loads are conservative ("dead"): point forces, tractions, pressure and body forces keep their
//! reference-configuration values; `Control::Load` scales loads *and* prescribed displacements
//! together with the load factor `lambda`.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::analysis::Model;
use crate::assembly::{BlockMatrix, Pattern, NO_BLOCK};
use crate::contact::{ContactSet, ContactSpec, ContactStats, CpState, ExtraOut, Master};
use crate::element::MAX_NODES;
use crate::kernel::{self, Work};
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::material::*;
use crate::mesh::{Block, Mesh, Physics};
use rayon::prelude::*;
use std::cell::Cell;
use std::sync::Arc;
use std::time::Instant;

/// Material state of every Gauss point (empty for elastic blocks).
#[derive(Clone, Debug)]
pub struct NlState {
    /// Per mesh block, `element * ngp + g`.
    pub gp: Vec<Vec<GpState>>,
    /// Per contact interface, per slave Gauss point (empty without contact).
    pub contact: Vec<Vec<CpState>>,
    /// Free rigid-body translations (the extra unknowns of force-controlled rigid masters).
    pub rigid_q: Vec<f64>,
}

impl NlState {
    /// Virgin state: no plastic strain.
    pub fn new(mesh: &Mesh) -> Self {
        Self { gp: mesh.blocks.iter().map(|b| b.plasticity.map_or(Vec::new(), |j| vec![j.initial_state(); b.n_elems() * b.kind.table().ngp])).collect(), contact: Vec::new(), rigid_q: Vec::new() }
    }

    /// Largest equivalent plastic strain anywhere.
    pub fn max_ep(&self) -> f64 {
        self.gp.iter().flatten().fold(0.0f64, |m, s| m.max(s[6]))
    }

    /// Fraction of the plastic blocks' Gauss points with any plastic strain.
    pub fn plastic_fraction(&self) -> f64 {
        let (mut n, mut yielded) = (0usize, 0usize);
        for g in self.gp.iter().flatten() {
            n += 1;
            yielded += usize::from(g[6] > 0.0);
        }
        if n == 0 { 0.0 } else { yielded as f64 / n as f64 }
    }

    /// Equivalent plastic strain at every Gauss point of every plastic block with its reference position.
    pub fn gauss_ep(&self, mesh: &Mesh) -> Vec<([f64; 3], f64)> {
        let mut out = Vec::new();
        for (b, blk) in mesh.blocks.iter().enumerate() {
            if self.gp[b].is_empty() {
                continue;
            }
            let t = blk.kind.table();
            for e in 0..blk.n_elems() {
                for g in 0..t.ngp {
                    let n = t.n_at(g);
                    let mut x = [0.0; 3];
                    for (a, &nd) in blk.elem(e).iter().enumerate() {
                        for i in 0..3 {
                            x[i] += n[a] * mesh.nodes[nd][i];
                        }
                    }
                    out.push((x, self.gp[b][e * t.ngp + g][6]));
                }
            }
        }
        out
    }
}

struct Shared<T>(*mut T);
unsafe impl<T> Send for Shared<T> {}
unsafe impl<T> Sync for Shared<T> {}

/// `K += w * sum_gp (dH/du_a)^T A (dH/du_b)` for any dimension (and the axisymmetric hoop entry): `A` is the
/// 9 x 9 `dS/dH` tangent, `dH/du` a 9-vector per dof.
#[allow(clippy::too_many_arguments)]
fn accumulate_tangent_generic(k: &mut [f64], nn: usize, d: usize, nd: usize, gr: &[f64], nsh: &[f64], radius: f64, axisym: bool, tangent: &[[f64; 9]; 9], w: f64) {
    let mut ba = [[0.0f64; 9]; 3 * MAX_NODES];
    for a in 0..nn {
        for i in 0..d {
            for jj in 0..d {
                ba[a * d + i][3 * i + jj] = gr[a * d + jj];
            }
            if axisym && i == 0 {
                ba[a * d][8] = nsh[a] / radius;
            }
        }
    }
    let mut ab = [[0.0f64; 9]; 3 * MAX_NODES];
    for b in 0..nd {
        for r in 0..9 {
            ab[b][r] = (0..9).map(|c| tangent[r][c] * ba[b][c]).sum();
        }
    }
    for a in 0..nd {
        for b in 0..nd {
            k[a * nd + b] += w * (0..9).map(|r| ba[a][r] * ab[b][r]).sum::<f64>();
        }
    }
}

/// The same for a 2D non-axisymmetric element, using that only `H_ij` with `i, j < 2` matter and that dof
/// `(a, i)` moves only row `i` of `H` (about 5 times fewer operations than the generic form):
/// `K[(a,i),(b,k)] += w * sum_{j,l} g_a[j] A[(i,j),(k,l)] g_b[l]`.
fn accumulate_tangent_2d(k: &mut [f64], nn: usize, gr: &[f64], tangent: &[[f64; 9]; 9], w: f64) {
    let nd = 2 * nn;
    // t[a][i][kk][l] = sum_j g_a[j] A[(i,j),(kk,l)]
    let mut t = [[[[0.0f64; 2]; 2]; 2]; MAX_NODES];
    for a in 0..nn {
        let (g0, g1) = (gr[a * 2], gr[a * 2 + 1]);
        for i in 0..2 {
            for kk in 0..2 {
                for l in 0..2 {
                    t[a][i][kk][l] = g0 * tangent[3 * i][3 * kk + l] + g1 * tangent[3 * i + 1][3 * kk + l];
                }
            }
        }
    }
    for a in 0..nn {
        for b in 0..nn {
            let (g0, g1) = (gr[b * 2], gr[b * 2 + 1]);
            for i in 0..2 {
                for kk in 0..2 {
                    k[(a * 2 + i) * nd + b * 2 + kk] += w * (t[a][i][kk][0] * g0 + t[a][i][kk][1] * g1);
                }
            }
        }
    }
}

/// Element internal force `fe` and tangent `ke` (`(a*d+i, b*d+k)`, row major) of one plastic element.
#[allow(clippy::too_many_arguments)] // a hot kernel: every input is a distinct piece of element data
fn plastic_element(blk: &Block, physics: Physics, xyz: &[[f64; 3]], ue: &[f64], old: &[GpState], new: &mut [GpState], work: &mut Work, fe: &mut [f64], ke: Option<&mut [f64]>) -> Result<(), String> {
    let j2 = blk.plasticity.expect("plastic block");
    let kind = blk.kind;
    let t = kind.table();
    let (nn, d, ngp) = (t.nn, t.dim, t.ngp);
    let nd = nn * d;
    kernel::geometry(kind, physics, xyz, work).map_err(|e| e.to_string())?;
    let mo = Moduli::new(blk.material.e, blk.material.nu);
    let axisym = matches!(physics, Physics::Axisymmetric);
    let plane_stress = matches!(physics, Physics::PlaneStress { .. });
    fe[..nd].fill(0.0);
    let mut ke = ke;
    if let Some(k) = ke.as_deref_mut() {
        k[..nd * nd].fill(0.0);
    }
    for g in 0..ngp {
        let gr = &work.grad[g * nn * d..(g + 1) * nn * d];
        let nsh = t.n_at(g);
        let mut h = [[0.0f64; 3]; 3];
        for a in 0..nn {
            for i in 0..d {
                for k in 0..d {
                    h[i][k] += ue[a * d + i] * gr[a * d + k];
                }
            }
        }
        let radius = work.radius[g];
        if axisym {
            let ur: f64 = (0..nn).map(|a| nsh[a] * ue[a * 2]).sum();
            h[2][2] = ur / radius;
        }
        let up = if j2.large_strain {
            let mut f = h;
            for i in 0..3 {
                f[i][i] += 1.0;
            }
            if plane_stress {
                finite_strain_plane_stress(mo, &j2.law, &f, &old[g], 1.0 - blk.material.nu / (1.0 - blk.material.nu) * (h[0][0] + h[1][1])).map(|r| r.0)
            } else {
                finite_strain_update(mo, &j2.law, &f, &old[g])
            }
            .ok_or_else(|| "material state failed (inverted element or non-positive stretch)".to_string())?
        } else {
            let eps: M3 = std::array::from_fn(|i| std::array::from_fn(|j| 0.5 * (h[i][j] + h[j][i])));
            match (&blk.material.aniso, plane_stress) {
                (Some(an), true) => small_strain_plane_stress_aniso(&an.d, &j2.law, &eps, &old[g]).0,
                (Some(an), false) => small_strain_update_aniso(&an.d, &j2.law, &eps, &old[g]),
                (None, true) => small_strain_plane_stress(mo, &j2.law, &eps, &old[g]).0,
                (None, false) => small_strain_update(mo, &j2.law, &eps, &old[g]),
            }
        };
        new[g] = up.state;
        let (s, w) = (&up.stress, work.wdet[g]);
        for a in 0..nn {
            for i in 0..d {
                let mut fi = 0.0;
                for k in 0..d {
                    fi += s[i][k] * gr[a * d + k];
                }
                if axisym && i == 0 {
                    fi += s[2][2] * nsh[a] / radius;
                }
                fe[a * d + i] += w * fi;
            }
        }
        if let Some(k) = ke.as_deref_mut() {
            if d == 2 && !axisym {
                accumulate_tangent_2d(k, nn, gr, &up.tangent, w);
            } else {
                accumulate_tangent_generic(k, nn, d, nd, gr, nsh, radius, axisym, &up.tangent, w);
            }
        }
    }
    Ok(())
}

/// Assemble the internal force vector (and the tangent if `want_k`) at displacements `u` from the
/// committed state `old`; the updated trial state goes to `new`.
pub(crate) fn assemble_nl(mesh: &Mesh, pat: &Pattern, u: &[f64], old: &NlState, new: &mut NlState, want_k: bool) -> Result<(Vec<f64>, Option<BlockMatrix>), String> {
    let d = pat.d;
    let dd = d * d;
    let mut f = vec![0.0; mesh.n_dofs()];
    let mut k = want_k.then(|| BlockMatrix::zeros(pat));
    let kptr = Shared(k.as_mut().map_or(std::ptr::null_mut(), |m| m.vals.as_mut_ptr()));
    let fptr = Shared(f.as_mut_ptr());
    let sptr: Vec<Shared<GpState>> = new.gp.iter_mut().map(|v| Shared(v.as_mut_ptr())).collect();
    let (kptr, fptr, sptr) = (&kptr, &fptr, &sptr);
    for class in &pat.colors {
        class.par_iter().try_for_each_init(
            || (Work::new(), vec![[0.0f64; 3]; MAX_NODES], vec![0.0f64; 3 * MAX_NODES], vec![0.0f64; 3 * MAX_NODES], vec![0.0f64; 9 * MAX_NODES * MAX_NODES], vec![0.0f64; MAX_NODES * MAX_NODES * 10]),
            |(work, xyz, ue, fe, ke, kk), &(bi, e)| -> Result<(), String> {
                let blk = &mesh.blocks[bi as usize];
                let nn = blk.kind.n_nodes();
                let nd = nn * d;
                let conn = blk.elem(e as usize);
                for (a, &nd_) in conn.iter().enumerate() {
                    xyz[a] = mesh.nodes[nd_];
                    for i in 0..d {
                        ue[a * d + i] = u[nd_ * d + i];
                    }
                }
                if blk.plasticity.is_some() {
                    let ngp = blk.kind.table().ngp;
                    let base = e as usize * ngp;
                    // SAFETY: elements of a colour touch disjoint state ranges, nodes and blocks.
                    let new_s = unsafe { std::slice::from_raw_parts_mut(sptr[bi as usize].0.add(base), ngp) };
                    plastic_element(blk, mesh.physics, &xyz[..nn], &ue[..nd], &old.gp[bi as usize][base..base + ngp], new_s, work, fe, if want_k { Some(&mut ke[..nd * nd]) } else { None })?;
                } else {
                    let layout = kernel::stiffness_fast(blk.kind, mesh.physics, &blk.material, &xyz[..nn], work, kk).map_err(|e| e.to_string())?;
                    for a in 0..nn {
                        for i in 0..d {
                            let mut s = 0.0;
                            for b in 0..nn {
                                for j in 0..d {
                                    let v = kernel::entry(layout, kk, nn, d, a, b, i, j);
                                    s += v * ue[b * d + j];
                                    if want_k {
                                        ke[(a * d + i) * nd + b * d + j] = v;
                                    }
                                }
                            }
                            fe[a * d + i] = s;
                        }
                    }
                }
                // SAFETY: colour classes share no node, so the force entries and matrix blocks written are disjoint.
                unsafe {
                    for (a, &nd_) in conn.iter().enumerate() {
                        for i in 0..d {
                            *fptr.0.add(nd_ * d + i) += fe[a * d + i];
                        }
                    }
                    if want_k {
                        let map = &pat.scatter[bi as usize][e as usize * nn * nn..(e as usize + 1) * nn * nn];
                        for a in 0..nn {
                            for b in 0..nn {
                                let pos = map[a * nn + b];
                                if pos == NO_BLOCK {
                                    continue;
                                }
                                let dst = kptr.0.add(pos as usize * dd);
                                for i in 0..d {
                                    for j in 0..d {
                                        *dst.add(i * d + j) += ke[(a * d + i) * nd + b * d + j];
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            },
        )?;
    }
    Ok((f, k))
}

/// Benchmark entry point to the assembly (internal force and tangent at displacements `u`).
#[doc(hidden)]
pub fn assemble_nl_public(mesh: &Mesh, pat: &Pattern, u: &[f64], old: &NlState, new: &mut NlState, want_k: bool) -> Result<(Vec<f64>, Option<BlockMatrix>), String> {
    assemble_nl(mesh, pat, u, old, new, want_k)
}

// ------------------------------------------------------------------ driver

/// How many times a contact solve is repeated with doubled margins after a node pair fell outside the matrix pattern.
const MARGIN_RETRIES: usize = 4;

/// Cycles of defect-correction refinement of a frictional Newton step.
const REFINE_ITERS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    /// Load factor (and prescribed displacements) from 0 to 1 in automatically cut steps.
    Load,
    /// Cylindrical arc-length (Crisfield) with step `ds` in displacement norm, until the load factor
    /// reaches `lambda_max` or `max_steps` steps have converged. Loads only (prescribed displacements
    /// must be zero).
    Arc { ds: f64, lambda_max: f64, max_steps: usize },
}

/// A cooperative stop for a long solve: a wall-clock deadline and/or a flag another thread raises. The solver checks it
/// at every Newton iteration and load step and returns the state it reached with `Stop::Interrupted`.
/// What a converged load step reports to a [`StepObserver`].
#[derive(Debug, Clone, Copy)]
pub struct StepEvent {
    /// Converged steps of this run so far (including this one).
    pub step: usize,
    pub lambda: f64,
    /// Norm of the force on the first contact master (the pin's pressing force), when there is contact.
    pub master_force: Option<f64>,
    pub factorisations: usize,
}

/// A callback invoked after every converged load step, on the solving thread (live progress for a long solve).
#[derive(Clone)]
pub struct StepObserver(pub std::sync::Arc<dyn Fn(&StepEvent) + Send + Sync>);

impl std::fmt::Debug for StepObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StepObserver")
    }
}

/// Newton iterations without a 3 % gain of the best residual after which a contact solve is called stagnated.
const STAGNATION_ITERATIONS: usize = 8;
/// Step cuts a stagnating load step gets under `stick_slip_guard`: halving the step released none of the stick-slip cycles
/// seen (a failing first attempt cost 59 s with all 12 cuts, 13 s with 3), and the caller has other attempts to try.
const STAGNATION_CUTS: usize = 3;
/// Iterations a solve gets on top of `max_iter` once its active set is held.
const HOLD_EXTRA_ITERATIONS: usize = 15;
/// Prefix of the Newton error of a stagnated solve (`Stop::NoConvergence` carries it).
pub const STAGNATED: &str = "stagnated";
#[derive(Debug, Clone, Default)]
pub struct Interrupt {
    pub deadline: Option<Instant>,
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Interrupt {
    /// Why it fired, if it has: a cancel request or the deadline.
    pub fn reason(&self) -> Option<&'static str> {
        if self.cancel.as_ref().is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
            Some("cancelled")
        } else if self.deadline.is_some_and(|d| Instant::now() >= d) {
            Some("time budget used up")
        } else {
            None
        }
    }

    pub fn triggered(&self) -> bool {
        self.reason().is_some()
    }
}

#[derive(Debug, Clone)]
pub struct NlOptions {
    pub control: Control,
    /// Initial number of load steps to `lambda = 1` (load control).
    pub steps: usize,
    pub max_iter: usize,
    /// Convergence: free-dof residual norm relative to the larger of the external and internal force norms.
    pub tol: f64,
    /// Backtrack when a full Newton step increases the residual.
    pub line_search: bool,
    /// Step halvings allowed before giving up.
    pub max_cuts: usize,
    /// Stop the analysis when the equivalent plastic strain anywhere reaches this.
    pub failure_ep: Option<f64>,
    /// Chord (modified) Newton: reuse one factorization of the tangent for up to this many
    /// iterations while the residual keeps falling by at least a factor of four per iteration
    /// (`0` = full Newton). The factorization dominates a 2D/3D iteration, the tangent assembly
    /// does not (docs/fea-core.md).
    pub chord_iters: usize,
    /// Contact: augmented-Lagrangian multiplier passes per load step (each a Newton solve).
    pub max_outer: usize,
    /// Contact: stop the passes when the multipliers change by less than this (relative to the peak pressure).
    pub outer_tol: f64,
    /// Displacement-driven contact: stop once the first master's force has flattened (flat for two steps
    /// in a row, or under 1.2 % gained over the last two, or falling): the limit load is on the curve.
    pub stop_on_plateau: bool,
    /// Displacement-driven contact: stop once the first master's force has fallen this fraction below its
    /// peak (`0` = off); how a hardening law's limit is read, since its curve never flattens.
    pub stop_on_fall: f64,
    /// Contact analyses: stop once the first master carries at least this force (norm), whatever the load factor.
    /// The stage that presses a body in by prescribed displacement until it carries a target load (`0` = off).
    pub stop_at_force: f64,
    /// Accept an iteration that used up `max_iter` with a residual below this fraction of the force scale
    /// (`0` = never): frictional contact can chatter between stick and slip without reaching the tight
    /// tolerance, and a load-travel curve does not need it.
    pub stall_tol: f64,
    /// Load control: the first step as a fraction of `1 / steps` (`1` = the regular step). A displacement-driven pin
    /// meets its stiffest transition at the first touch, and a step that fails there costs a full set of iterations
    /// before it is halved.
    pub first_step: f64,
    /// Give up early on a deadline or a cancel request (`Stop::Interrupted`).
    pub interrupt: Interrupt,
    /// Frictional contact that cycles between stick and slip (the residual alternates or plateaus far above `tol`):
    /// (1) a Newton step that had to be backtracked sets the fraction of the next ones, which recovers by doubling;
    /// (2) 8 iterations without a 3 % gain of the best residual end the solve as stagnated: the lowest iterate is
    /// accepted when `stall_tol` allows it (counted in `NlSolution::stalled_solves`), else the solve fails at once
    /// (`STAGNATED`; the load step gets 3 cuts, not `max_cuts`). Off by default: callers that converge slowly
    /// but surely (the lug solver's elastic pin) must not be cut short.
    pub stick_slip_guard: bool,
    /// Called after every converged load step.
    pub observer: Option<StepObserver>,
}

impl NlOptions {
    /// Stop at the failure strain of library material `id` (`mechanics_core::fracture`): the
    /// equivalent plastic strain at which that material is taken to fail in bearing/tension.
    pub fn failure_for_material(mut self, id: &str) -> Result<Self, String> {
        use mechanics_core::fracture::{failure_strain, Basis};
        let f = failure_strain(id).ok_or_else(|| format!("no failure strain for material '{id}'"))?;
        if f.basis == Basis::NotApplicable {
            return Err(format!("the finite-strain failure rule does not apply to '{id}'"));
        }
        self.failure_ep = Some(f.value);
        Ok(self)
    }
}

impl Default for NlOptions {
    fn default() -> Self {
        Self { control: Control::Load, steps: 10, max_iter: 25, tol: 1e-9, line_search: true, max_cuts: 12, failure_ep: None, chord_iters: 0, max_outer: 12, outer_tol: 1e-4, stop_on_plateau: false, stop_on_fall: 0.0, stop_at_force: 0.0, stall_tol: 0.0, first_step: 1.0, interrupt: Interrupt::default(), stick_slip_guard: false, observer: None }
    }
}

#[derive(Debug, Clone)]
pub struct StepInfo {
    pub lambda: f64,
    pub iterations: usize,
    /// Residual norm at every iteration of the step.
    pub residuals: Vec<f64>,
    pub max_ep: f64,
    /// Fraction of plastic Gauss points that have yielded at the end of the step.
    pub plastic_fraction: f64,
    /// Step halvings needed before this step converged.
    pub cuts: usize,
    /// The tangent was not positive definite during this (arc-length) step: the equilibrium path is
    /// unstable here (past a limit point or on a collapse mechanism).
    pub indefinite: bool,
    /// Contact analyses: the total force on the master of each interface at the converged step
    /// (the load-travel curve of a displacement-driven rigid pin). Empty otherwise.
    pub master_force: Vec<[f64; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stop {
    /// `lambda = 1` reached (load control) or `lambda_max` (arc-length).
    Completed,
    /// The failure strain was reached.
    FailureStrain,
    /// `max_steps` arc-length steps.
    StepLimit,
    /// Newton could not converge even with the smallest step.
    NoConvergence(String),
    /// The pin load flattened or fell (`NlOptions::stop_on_plateau` / `stop_on_fall`): the limit is on the curve.
    Plateau,
    /// The first master reached `NlOptions::stop_at_force`.
    ForceReached,
    /// `NlOptions::interrupt` fired (deadline or cancel request): the solution is the last converged step, not a result.
    Interrupted,
}

pub struct NlSolution {
    pub u: Vec<f64>,
    pub state: NlState,
    pub lambda: f64,
    pub steps: Vec<StepInfo>,
    /// Internal minus applied force at the fixed dofs.
    pub reactions: Vec<f64>,
    pub stop: Stop,
    pub factorisations: usize,
    /// Newton solves accepted because the residual stalled below `NlOptions::stall_tol` (frictional stick-slip chatter
    /// that does not reach the tolerance), not because it met it. `0` when every solve converged.
    pub stalled_solves: usize,
    /// Newton solves that stagnated and were finished with a held active set (`ContactSet::hold_active_set`).
    pub held_solves: usize,
    pub elapsed_ms: f64,
    /// Contact summary of the final state (`None` without contact).
    pub contact: Option<ContactStats>,
    /// Reference positions of the slave collocation points per interface (order of `state.contact`).
    pub contact_points: Vec<Vec<[f64; 3]>>,
    /// Integration weights of those points: pressure times weight is the force a point carries.
    pub contact_weights: Vec<Vec<f64>>,
    /// Final translation of every rigid master (`shift + lambda travel + free part`), per interface.
    pub rigid_translation: Vec<[f64; 3]>,
    /// Net force the weak grounding springs (`Loads::ground`) carry at the end: the leak of the way a body held only by
    /// contact is kept from floating (compare it with the applied load). `0` without springs.
    pub ground_leak: f64,
}

impl NlSolution {
    /// The tractions of contact interface `spec` (in the order the interfaces were given): per slave point the
    /// position, normal, pressure, friction traction and weight, a first-class result instead of reading raw point
    /// states. `mu` is the interface's friction coefficient (it only decides which points are `slipping`). Integrals of
    /// these (the transmitted force `sum p w n`, a torque `sum x cross t w`) are the reliable outputs: pointwise values
    /// scatter on non-matching deformable meshes.
    pub fn contact_tractions(&self, spec: usize, mu: f64) -> Vec<crate::contact::ContactTraction> {
        let (Some(states), Some(points), Some(weights)) = (self.state.contact.get(spec), self.contact_points.get(spec), self.contact_weights.get(spec)) else { return Vec::new() };
        states
            .iter()
            .zip(points)
            .zip(weights)
            .map(|((s, x), w)| {
                let friction = if s.active { s.fric } else { [0.0; 3] };
                let fmag = norm(&friction);
                crate::contact::ContactTraction { x: *x, normal: s.n, pressure: if s.active { s.p } else { 0.0 }, friction, weight: *w, active: s.active, slipping: s.active && mu > 0.0 && fmag >= mu * s.p * (1.0 - 1e-3) }
            })
            .collect()
    }

    pub fn complete(&self) -> bool {
        matches!(self.stop, Stop::Completed | Stop::FailureStrain | Stop::StepLimit | Stop::Plateau | Stop::ForceReached)
    }

    /// Peak load factor over the converged steps.
    pub fn peak_lambda(&self) -> f64 {
        self.steps.iter().fold(0.0f64, |m, s| m.max(s.lambda))
    }
}

/// Solve the small dense system `a x = b` (row-major, `n x n`) by Gaussian elimination with partial pivoting.
fn solve_dense(a: &mut [f64], b: &mut [f64], n: usize) -> Result<Vec<f64>, String> {
    let scale = a.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-300);
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i * n + c].abs().total_cmp(&a[j * n + c].abs())).unwrap();
        if a[p * n + c].abs() < 1e-13 * scale {
            return Err("the rigid-body degrees of freedom are singular (a free pin with no contact)".into());
        }
        if p != c {
            for k in 0..n {
                a.swap(c * n + k, p * n + k);
            }
            b.swap(c, p);
        }
        for i in c + 1..n {
            let f = a[i * n + c] / a[c * n + c];
            for k in c..n {
                a[i * n + k] -= f * a[c * n + k];
            }
            b[i] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        x[i] = (b[i] - (i + 1..n).map(|k| a[i * n + k] * x[k]).sum::<f64>()) / a[i * n + i];
    }
    Ok(x)
}

fn norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

struct Ctx<'a> {
    model: &'a Model,
    pat: Arc<Pattern>,
    contacts: Option<ContactSet>,
    red: Reduced,
    free: Vec<usize>,
    fixed: Vec<usize>,
    f_ext: Vec<f64>,
    bc: &'a Dirichlet,
    opt: NlOptions,
    factorisations: Cell<usize>,
    /// Newton solves accepted on a stalled residual (`NlOptions::stall_tol`) rather than the tolerance.
    stalled: Cell<usize>,
    /// Newton solves that stagnated and went on with a held active set.
    held: Cell<usize>,
    start: Option<&'a Start>,
    ground: Vec<(usize, f64)>,
    /// Follower pressures (face nodes, p): see `Loads::followers`.
    followers: Vec<(Vec<usize>, f64)>,
    /// Seconds spent (evaluation, factorisation, solves), reported under `NL_PROFILE`.
    timers: [Cell<f64>; 5],
}

/// A converged state to continue from (load control with contact): the displacements and the
/// contact states of the interfaces that existed before (e.g. an interference fit solved on its own
/// before a pin is added and loaded). `contact[i]` is the state of contact spec `i`, `None` for a new one.
#[derive(Debug, Clone)]
pub struct Start {
    pub u: Vec<f64>,
    pub contact: Vec<Option<Vec<CpState>>>,
    /// Material state of the plastic blocks (per block, empty for elastic ones).
    pub gp: Option<Vec<Vec<GpState>>>,
    /// The load factor the state is in equilibrium at (`0` for a state that carries none of this run's load): the
    /// run goes from here to `1`, so a stage that already applied part of the load is continued, not restarted.
    pub lambda: f64,
}

struct Eval {
    f: Vec<f64>,
    k: Option<BlockMatrix>,
    state: NlState,
    stats: Option<ContactStats>,
    ex: Option<ExtraOut>,
    /// Norm of the element (elastic / plastic) internal force alone: it does not cancel against the contact forces
    /// at equilibrium, so it is the force level a relative tolerance must be measured against.
    f_elem: f64,
    /// Unsymmetric remainder of the follower-load stiffness as `(dofs, row-major matrix)` per face (the symmetric part is
    /// in `k`): the exact tangent is `K + sum E^T D E`, applied by the same refinement as the frictional defect.
    load_defect: Vec<(Vec<usize>, Vec<f64>)>,
}

impl Ctx<'_> {
    fn eval(&self, u: &[f64], q: &[f64], old: &NlState, lambda: f64, want_k: bool) -> Result<Eval, String> {
        let t_eval = Instant::now();
        let mut state = old.clone();
        let (mut f, mut k) = assemble_nl(&self.model.mesh, &self.pat, u, old, &mut state, want_k)?;
        let mut stats = None;
        let mut ex = None;
        let f_elem = norm(&f);
        self.timers[3].set(self.timers[3].get() + t_eval.elapsed().as_secs_f64());
        let t_contact = Instant::now();
        if let Some(cs) = &self.contacts {
            let m = cs.n_extra();
            let mut out = (m > 0).then(|| ExtraOut::new(self.model.mesh.n_dofs(), m));
            let st = cs.eval(&self.model.mesh, &self.pat, u, q, lambda, &old.contact, &mut state.contact, &mut f, k.as_mut(), out.as_mut())?;
            if !st.missing_pairs.is_empty() {
                return Err(format!("contact reached node pairs outside the matrix pattern (increase the contact margin): {:?}", &st.missing_pairs[..st.missing_pairs.len().min(4)]));
            }
            stats = Some(st);
            ex = out;
        }
        self.timers[4].set(self.timers[4].get() + t_contact.elapsed().as_secs_f64());
        // Follower pressures: the force follows the deformed surface and so does the load stiffness. Its symmetric part joins
        // the tangent, the unsymmetric remainder is kept as a defect (see `refine_consistent`).
        let mut load_defect = Vec::new();
        let d = self.model.mesh.dim();
        for (nodes, p) in &self.followers {
            let (g, jac) = loads::follower_face(&self.model.mesh, nodes, *p, u)?;
            let nd = nodes.len() * d;
            for a in 0..nodes.len() {
                for i in 0..d {
                    f[nodes[a] * d + i] -= lambda * g[a * d + i];
                }
            }
            if let Some(k) = k.as_mut() {
                // r = f_int - lambda (f_ext + g(u)): d r / d u gets -lambda J.
                let mut defect = vec![0.0; nd * nd];
                let mut any = false;
                for ra in 0..nd {
                    for cb in 0..nd {
                        let m = -lambda * jac[ra * nd + cb];
                        let sym = 0.5 * (m - lambda * jac[cb * nd + ra]);
                        defect[ra * nd + cb] = m - sym;
                        any |= (m - sym).abs() > 1e-14 * m.abs().max(1.0);
                        let (a, i, b, j) = (ra / d, ra % d, cb / d, cb % d);
                        if nodes[a] >= nodes[b] {
                            if let Some(pos) = self.pat.find(nodes[a], nodes[b]) {
                                k.vals[pos * d * d + i * d + j] += sym;
                            }
                        }
                    }
                }
                if any {
                    let dofs: Vec<usize> = nodes.iter().flat_map(|&n| (0..d).map(move |c| n * d + c)).collect();
                    load_defect.push((dofs, defect));
                }
            }
        }
        // Weak grounding springs.
        for &(dof, kk) in &self.ground {
            f[dof] += kk * u[dof];
            if let Some(k) = k.as_mut() {
                let d = self.model.mesh.dim();
                let (node, comp) = (dof / d, dof % d);
                let blk = self.pat.find(node, node).expect("diagonal block");
                k.vals[blk * d * d + comp * d + comp] += kk;
            }
        }
        state.rigid_q = q.to_vec();
        if f.iter().any(|v| !v.is_finite()) {
            return Err("non-finite internal force".into());
        }
        self.timers[0].set(self.timers[0].get() + t_eval.elapsed().as_secs_f64());
        Ok(Eval { f, k, state, stats, ex, f_elem, load_defect })
    }

    /// Residual over the free dofs followed by the residuals of the extra (rigid-body) unknowns at
    /// load factor `lambda`.
    fn residual(&self, ev: &Eval, lambda: f64) -> Vec<f64> {
        let mut r: Vec<f64> = self.free.iter().map(|&i| ev.f[i] - lambda * self.f_ext[i]).collect();
        if let (Some(ex), Some(cs)) = (&ev.ex, &self.contacts) {
            for (j, v) in ex.rq.iter().enumerate() {
                r.push(v - lambda * cs.extra_load(j));
            }
        }
        r
    }

    /// Newton step for the augmented system `[K_uu K_uq; K_qu K_qq] [du; dq] = -[r_u; r_q]` by a Schur
    /// complement on the extra unknowns (a handful) over the existing factorization of `K_uu`.
    fn newton_step(&self, fac: &crate::linear::AnyFactor<'_>, ev: &Eval, r: &[f64]) -> Result<Vec<f64>, String> {
        let nf = self.free.len();
        let m = r.len() - nf;
        let mut a: Vec<f64> = r[..nf].iter().map(|v| -v).collect();
        let b0 = a.clone();
        fac.solve_reduced(&mut a);
        if m == 0 {
            self.refine_consistent(fac, ev, &b0, &mut a);
            return Ok(a);
        }
        let ex = ev.ex.as_ref().expect("extra unknowns");
        let ndm = self.model.mesh.n_dofs();
        let bcol = |j: usize, f: usize| ex.kuq[j * ndm + self.free[f]];
        let mut z: Vec<Vec<f64>> = Vec::with_capacity(m);
        for j in 0..m {
            let mut b: Vec<f64> = (0..nf).map(|f| bcol(j, f)).collect();
            fac.solve_reduced(&mut b);
            z.push(b);
        }
        let mut s = vec![0.0; m * m];
        let mut rhs = vec![0.0; m];
        for i in 0..m {
            for j in 0..m {
                s[i * m + j] = ex.kqq[i * m + j] - (0..nf).map(|f| bcol(i, f) * z[j][f]).sum::<f64>();
            }
            rhs[i] = -r[nf + i] - (0..nf).map(|f| bcol(i, f) * a[f]).sum::<f64>();
        }
        let dq = solve_dense(&mut s, &mut rhs, m)?;
        let mut dx = vec![0.0; nf + m];
        for f in 0..nf {
            dx[f] = a[f] - (0..m).map(|j| z[j][f] * dq[j]).sum::<f64>();
        }
        dx[nf..].copy_from_slice(&dq);
        Ok(dx)
    }

    /// A residual this small against the full applied load is converged whatever the current force level (a body in
    /// free flight has nothing but round-off to measure a relative tolerance against).
    fn abs_tol(&self) -> f64 {
        1e-10 * norm(&self.f_ext)
    }

    /// Make the Newton step the one the exact frictional tangent gives: the factorised matrix `K` is symmetric and
    /// freezes sliding friction; the exact tangent is `K + D` with `D` the (unsymmetric) defect of the frictional
    /// points (`ContactStats::defect`). Iterative refinement `x += K^-1 (b - (K + D) x)` solves `(K + D) x = b` over the
    /// existing factorisation, a few cheap back-substitutions that restore fast convergence where friction slides.
    /// Stops when the defect is resolved to 1 %, or if it stops improving (then the best iterate is kept).
    fn refine_consistent(&self, fac: &crate::linear::AnyFactor<'_>, ev: &Eval, b: &[f64], x: &mut Vec<f64>) {
        let contact_defect: &[(Vec<usize>, Vec<f64>)] = ev.stats.as_ref().map_or(&[], |s| &s.defect);
        if contact_defect.is_empty() && ev.load_defect.is_empty() {
            return;
        }
        let (k, nf, n) = (ev.k.as_ref().expect("tangent"), self.free.len(), self.model.mesh.n_dofs());
        let bn = norm(b).max(1e-300);
        let apply = |x: &[f64]| -> Vec<f64> {
            let mut xf = vec![0.0; n];
            for (j, &i) in self.free.iter().enumerate() {
                xf[i] = x[j];
            }
            let mut y = vec![0.0; n];
            k.matvec_add(&self.pat, &xf, &mut y);
            for (dofs, mat) in contact_defect.iter().chain(ev.load_defect.iter()) {
                let nt = dofs.len();
                for i in 0..nt {
                    let mut s = 0.0;
                    for j in 0..nt {
                        s += mat[i * nt + j] * xf[dofs[j]];
                    }
                    y[dofs[i]] += s;
                }
            }
            (0..nf).map(|j| y[self.free[j]]).collect()
        };
        let mut best: Option<(f64, Vec<f64>)> = None;
        for _ in 0..REFINE_ITERS {
            let ax = apply(x);
            let res: Vec<f64> = b.iter().zip(&ax).map(|(bb, aa)| bb - aa).collect();
            let rn = norm(&res);
            if best.as_ref().is_none_or(|(bn_, _)| rn < *bn_) {
                best = Some((rn, x.clone()));
            } else {
                break;
            }
            if rn <= 1e-2 * bn {
                break;
            }
            let mut dx = res;
            fac.solve_reduced(&mut dx);
            for (xi, di) in x.iter_mut().zip(&dx) {
                *xi += di;
            }
        }
        if let Some((_, xb)) = best {
            *x = xb;
        }
    }

    fn scale(&self, ev: &Eval, lambda: f64) -> f64 {
        norm(&ev.f).max(ev.f_elem).max(lambda * norm(&self.f_ext)).max(1e-300)
    }

    /// Solve `K_ff x = a` and `K_ff y = b` with one factorization of the tangent of `ev` (LDL^T when the
    /// tangent is indefinite, as it is past a limit point); returns whether it was indefinite.
    fn solve2(&self, ev: &Eval, a: &mut [f64], b: &mut [f64]) -> Result<bool, String> {
        let fac = self.red.factor_any(ev.k.as_ref().expect("tangent")).map_err(|e| e.to_string())?;
        let (a0, b0) = (a.to_vec(), b.to_vec());
        fac.solve_reduced(a);
        fac.solve_reduced(b);
        for (rhs, orig) in [(&mut *a, &a0), (&mut *b, &b0)] {
            let mut x = rhs.to_vec();
            self.refine_consistent(&fac, ev, orig, &mut x);
            rhs.copy_from_slice(&x);
        }
        self.factorisations.set(self.factorisations.get() + 1);
        Ok(fac.is_indefinite())
    }

    /// As [`solve`](Self::solve) with LDL^T where the tangent is indefinite (contact with friction).
    fn solve_any(&self, ev: &Eval, rhs: &mut [f64]) -> Result<(), String> {
        let fac = self.red.factor_any(ev.k.as_ref().expect("tangent")).map_err(|e| e.to_string())?;
        fac.solve_reduced(rhs);
        self.factorisations.set(self.factorisations.get() + 1);
        Ok(())
    }

    /// Solve `K_ff x = rhs` with the tangent of `ev`.
    fn solve(&self, ev: &Eval, rhs: &mut [f64]) -> Result<(), String> {
        let fac = self.red.factor(ev.k.as_ref().expect("tangent")).map_err(|e| e.to_string())?;
        fac.solve_reduced(rhs);
        self.factorisations.set(self.factorisations.get() + 1);
        Ok(())
    }
}

impl Model {
    /// Reject a body that nothing holds: no prescribed displacement, no grounding spring and no contact touching any of
    /// its nodes. It would float (a singular system can still factorise and return an arbitrary rigid motion).
    fn check_not_floating(&self, bc: &Dirichlet, ground: &[(usize, f64)], contacts: &[ContactSpec]) -> Result<(), String> {
        let (n, d) = (self.mesh.nodes.len(), self.mesh.dim());
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], mut a: usize) -> usize {
            while p[a] != a {
                p[a] = p[p[a]];
                a = p[a];
            }
            a
        }
        let mut used = vec![false; n];
        let mut block_of = vec![0usize; n];
        for (bi, blk) in self.mesh.blocks.iter().enumerate() {
            for c in blk.conn.chunks_exact(blk.kind.n_nodes()) {
                for &a in c {
                    used[a] = true;
                    block_of[a] = bi;
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, c[0]));
                    parent[ra] = rb;
                }
            }
        }
        let mut anchored = vec![false; n];
        let hold = |node: usize, anchored: &mut Vec<bool>, parent: &mut Vec<usize>| {
            let r = find(parent, node);
            anchored[r] = true;
        };
        for node in (0..n).filter(|&i| used[i]) {
            if (0..d).any(|c| bc.fixed[node * d + c]) {
                hold(node, &mut anchored, &mut parent);
            }
        }
        for &(dof, k) in ground {
            if k > 0.0 && dof / d < n {
                hold(dof / d, &mut anchored, &mut parent);
            }
        }
        for spec in contacts {
            for face in spec.slave.iter().chain(match &spec.master {
                crate::contact::Master::Faces(f) => f.iter(),
                crate::contact::Master::Rigid(_) => [].iter(),
            }) {
                for &node in face {
                    if node < n {
                        hold(node, &mut anchored, &mut parent);
                    }
                }
            }
        }
        for node in (0..n).filter(|&i| used[i]) {
            let r = find(&mut parent, node);
            if !anchored[r] {
                let name = &self.mesh.blocks[block_of[node]].name;
                return Err(format!("the body containing block '{name}' has no supports, grounding springs or contacts: it would float"));
            }
        }
        Ok(())
    }

    /// Nonlinear static solution. Blocks with `plasticity` use J2 plasticity (small or finite strain);
    /// other blocks are linear elastic. See the module documentation for the conventions.
    pub fn solve_nonlinear(&self, loads: &Loads, bc: &Dirichlet, opt: &NlOptions) -> Result<NlSolution, String> {
        self.solve_nonlinear_contact(loads, bc, Vec::new(), opt)
    }

    /// Nonlinear solution with contact interfaces (see `contact.rs`). Contact needs load control.
    pub fn solve_nonlinear_contact(&self, loads: &Loads, bc: &Dirichlet, contacts: Vec<ContactSpec>, opt: &NlOptions) -> Result<NlSolution, String> {
        self.solve_nonlinear_contact_from(loads, bc, contacts, opt, None)
    }

    /// As [`solve_nonlinear_contact`](Self::solve_nonlinear_contact), continuing from a converged `start`
    /// (load control only): the load factor runs `0 -> 1` on top of it.
    pub fn solve_nonlinear_contact_from(&self, loads: &Loads, bc: &Dirichlet, contacts: Vec<ContactSpec>, opt: &NlOptions, start: Option<&Start>) -> Result<NlSolution, String> {
        // The matrix pattern is built once from the initial proximity (`margin`); a contact that slides to a node pair
        // outside it fails the step. Rather than make the caller guess a margin, widen every margin and rerun.
        let mut specs = contacts;
        for attempt in 0..=MARGIN_RETRIES {
            match self.solve_nonlinear_contact_once(loads, bc, specs.clone(), opt, start) {
                Err(e) if attempt < MARGIN_RETRIES && e.contains("outside the matrix pattern") => {
                    specs.iter_mut().for_each(|s| s.margin = Some(2.0 * s.effective_margin(&self.mesh)));
                }
                other => return other,
            }
        }
        unreachable!("the last attempt returns")
    }

    fn solve_nonlinear_contact_once(&self, loads: &Loads, bc: &Dirichlet, contacts: Vec<ContactSpec>, opt: &NlOptions, start: Option<&Start>) -> Result<NlSolution, String> {
        let clock = Instant::now();
        let any_plastic = self.mesh.blocks.iter().any(|b| b.plasticity.is_some());
        if any_plastic && loads.delta_t != 0.0 {
            return Err("a temperature change cannot be combined with plasticity yet".into());
        }
        let n = self.mesh.n_dofs();
        self.check_not_floating(bc, &loads.ground, &contacts)?;
        // Follower pressures are applied by the driver at the deformed configuration, not as dead loads.
        let f_ext = if loads.followers.is_empty() { loads::assemble(&self.mesh, loads)? } else { loads::assemble(&self.mesh, &Loads { followers: Vec::new(), ..loads.clone() })? };
        let contact_set = if contacts.is_empty() { None } else { Some(ContactSet::new(&self.mesh, contacts)?) };
        if contact_set.as_ref().is_some_and(|cs| cs.n_extra() > 0) && !matches!(opt.control, Control::Load) {
            return Err("a rigid master with free translations needs load control (arc-length handles deformable contact only)".into());
        }
        let _ = Master::Faces(Vec::new());
        let pat = match &contact_set {
            Some(cs) => {
                let pairs = cs.pattern_pairs(&self.mesh);
                if pairs.is_empty() {
                    self.pattern.clone()
                } else {
                    Arc::new(Pattern::new_with_extra(&self.mesh, &pairs)?)
                }
            }
            None => self.pattern.clone(),
        };
        let red = Reduced::with_ordering(&pat, bc, Some(&self.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let fixed: Vec<usize> = (0..n).filter(|&i| bc.fixed[i]).collect();
        if let Control::Arc { .. } = opt.control {
            if bc.value.iter().zip(&bc.fixed).any(|(v, f)| *f && *v != 0.0) {
                return Err("arc-length control needs zero prescribed displacements".into());
            }
        }
        let mut ctx = Ctx { model: self, pat, contacts: contact_set, red, free, fixed, f_ext, bc, opt: opt.clone(), factorisations: Cell::new(0), stalled: Cell::new(0), held: Cell::new(0), start, ground: loads.ground.clone(), followers: loads.followers.clone(), timers: Default::default() };
        let mut sol = match opt.control {
            Control::Load => ctx.run_load_control()?,
            Control::Arc { ds, lambda_max, max_steps } => ctx.run_arc_length(ds, lambda_max, max_steps)?,
        };
        sol.factorisations = ctx.factorisations.get();
        sol.stalled_solves = ctx.stalled.get();
        sol.held_solves = ctx.held.get();
        sol.elapsed_ms = clock.elapsed().as_secs_f64() * 1e3;
        if std::env::var("NL_PROFILE").is_ok() {
            eprintln!("nl profile: {:.0} ms total, evaluation {:.0} ms (element assembly {:.0}, contact {:.0}), factorisation {:.0} ms ({} of them), solves {:.0} ms", sol.elapsed_ms, 1e3 * ctx.timers[0].get(), 1e3 * ctx.timers[3].get(), 1e3 * ctx.timers[4].get(), 1e3 * ctx.timers[1].get(), sol.factorisations, 1e3 * ctx.timers[2].get());
        }
        Ok(sol)
    }
}

/// Has the first master's force flattened (the limit-load rule of the lug)? Over the last 5 % of the travel it
/// gained under 1.2 % of its top, or it fell. Measured on travel, not on the number of steps: step cuts after a
/// convergence trouble leave tiny steps over which any load looks flat.
fn load_flat(steps: &[StepInfo], gain: f64) -> bool {
    let f = |s: &StepInfo| -> f64 { s.master_force.first().map_or(0.0, |v| norm(v)) };
    let Some(last) = steps.last() else { return false };
    if steps.len() < 4 {
        return false;
    }
    let Some(j) = steps.iter().rposition(|s| s.lambda <= last.lambda - 0.05) else { return false };
    let top = steps[j..].iter().map(f).fold(0.0f64, f64::max).max(1e-12);
    f(last) - f(&steps[j]) < gain * top
}

/// Has the first master's force fallen `frac` below its peak?
fn load_fell(steps: &[StepInfo], frac: f64) -> bool {
    let f = |s: &StepInfo| -> f64 { s.master_force.first().map_or(0.0, |v| norm(v)) };
    let peak = steps.iter().map(f).fold(0.0f64, f64::max);
    steps.last().is_some_and(|s| peak > 0.0 && f(s) < (1.0 - frac) * peak)
}

impl Ctx<'_> {
    fn finish(&self, u: Vec<f64>, state: NlState, lambda: f64, steps: Vec<StepInfo>, stop: Stop) -> NlSolution {
        let mut reactions = vec![0.0; u.len()];
        let mut contact = None;
        if let Ok(ev) = self.eval(&u, &state.rigid_q, &state, lambda, false) {
            for &i in &self.fixed {
                reactions[i] = ev.f[i] - lambda * self.f_ext[i];
            }
            contact = ev.stats;
        }
        let contact_points = self.contacts.as_ref().map_or(Vec::new(), |cs| (0..cs.specs.len()).map(|i| cs.slave_positions(i)).collect());
        let rigid_translation = self.contacts.as_ref().map_or(Vec::new(), |cs| cs.rigid_translations(lambda, &state.rigid_q));
        let contact_weights = self.contacts.as_ref().map_or(Vec::new(), |cs| (0..cs.specs.len()).map(|i| cs.slave_weights(i)).collect());
        let ground_leak = {
            let d = self.model.mesh.dim();
            let mut net = [0.0f64; 3];
            for &(dof, k) in &self.ground {
                net[dof % d] += k * u[dof];
            }
            norm(&net)
        };
        NlSolution { u, state, lambda, steps, reactions, stop, factorisations: 0, stalled_solves: 0, held_solves: 0, elapsed_ms: 0.0, contact, contact_points, contact_weights, rigid_translation, ground_leak }
    }

    /// Newton iterations at load factor `lambda` from the converged `(u, state)`.
    fn newton(&self, lambda_prev: f64, lambda: f64, u0: &[f64], state0: &NlState) -> Result<(Vec<f64>, NlState, Vec<f64>), String> {
        let result = self.newton_iterations(lambda_prev, lambda, u0, state0);
        // A held active set belongs to the one solve that asked for it.
        if let Some(cs) = &self.contacts {
            cs.release_active_set();
        }
        result
    }

    fn newton_iterations(&self, lambda_prev: f64, lambda: f64, u0: &[f64], state0: &NlState) -> Result<(Vec<f64>, NlState, Vec<f64>), String> {
        let nf = self.free.len();
        let mut u = u0.to_vec();
        let mut q = state0.rigid_q.clone();
        let mut jump = vec![0.0; u.len()];
        for &i in &self.fixed {
            jump[i] = lambda * self.bc.value[i] - u0[i];
            u[i] = lambda * self.bc.value[i];
        }
        // Tangent predictor for prescribed displacements: move the free dofs by their linear response
        // to the prescribed increment (K_ff du = -K_fp dup). Without it the first residual sees one
        // badly distorted layer of elements next to the moved boundary and the step often fails.
        if jump.iter().any(|v| *v != 0.0) {
            // (Without the near-point stabiliser: it would couple a body that is only approaching to the one being moved.)
            if let Some(cs) = &self.contacts {
                cs.set_stabilise(false);
            }
            let ev0 = self.eval(u0, &q, state0, lambda_prev, true);
            if let Some(cs) = &self.contacts {
                cs.set_stabilise(true);
            }
            let ev0 = ev0?;
            let mut kd = vec![0.0; u.len()];
            ev0.k.as_ref().expect("tangent").matvec_add(&self.pat, &jump, &mut kd);
            let mut dx: Vec<f64> = self.free.iter().map(|&i| -kd[i]).collect();
            self.solve(&ev0, &mut dx)?;
            for (j, &i) in self.free.iter().enumerate() {
                u[i] += dx[j];
            }
        }
        let mut residuals = Vec::new();
        let mut ev = self.eval(&u, &q, state0, lambda, true)?;
        let mut r = self.residual(&ev, lambda);
        let mut rn = norm(&r);
        residuals.push(rn);
        if std::env::var("NL_TOPRES").is_ok() {
            let mut idx: Vec<usize> = (0..self.free.len()).collect();
            idx.sort_by(|&a, &b| r[b].abs().total_cmp(&r[a].abs()));
            let d = self.model.mesh.dim();
            let top: Vec<String> = idx.iter().take(4).map(|&j| { let dof = self.free[j]; let nd = dof / d; format!("node {nd} comp {} r {:.2e} at {:?} u {:.2e}", dof % d, r[j], &self.model.mesh.nodes[nd][..2], u[dof]) }).collect();
            eprintln!("  lambda {lambda:.4e} initial residual {rn:.3e}: {}", top.join(" | "));
        }
        // (u, q before the step, the step, residual norm before it)
        type Prev = (Vec<f64>, Vec<f64>, Vec<f64>, f64);
        let mut prev: Option<Prev> = None;
        // The factorization of a past tangent (chord Newton) and how many iterations it has served.
        let mut chord: Option<(crate::linear::AnyFactor<'_>, usize)> = None;
        let mut last_rn = f64::INFINITY;
        // Stagnation: the best residual so far and the iterations since it last improved by 3 %. Frictional contact can
        // cycle between stick and slip at a residual far above the tolerance; 8 iterations without progress is that.
        let (mut best_rn, mut since_best) = (f64::INFINITY, 0usize);
        // With `stall_tol` the iterate of the lowest residual is kept, to be accepted when the iteration stalls.
        let mut lowest: Option<(f64, Vec<f64>, Vec<f64>, NlState)> = None;
        // Only a frictional contact cycles between stick and slip: `NlOptions::stick_slip_guard` is for it.
        let frictional = self.opt.stick_slip_guard && self.contacts.as_ref().is_some_and(|cs| cs.specs.iter().any(|s| s.mu > 0.0));
        let (mut iteration, mut budget) = (0usize, self.opt.max_iter);
        while iteration <= budget {
            iteration += 1;
            if self.opt.interrupt.triggered() {
                return Err("interrupted".into());
            }
            if rn <= self.opt.tol * self.scale(&ev, lambda) || rn < 1e-14 * self.scale(&ev, lambda).max(1.0) || rn <= self.abs_tol() {
                let mut st = ev.state;
                st.rigid_q = q;
                return Ok((u, st, residuals));
            }
            if !rn.is_finite() || rn > 1e12 * (1.0 + self.scale(&ev, lambda)) {
                return Err("residual diverged".into());
            }
            if self.opt.stall_tol > 0.0 && lowest.as_ref().is_none_or(|l| rn < l.0) {
                lowest = Some((rn, u.clone(), q.clone(), ev.state.clone()));
            }
            if rn < 0.97 * best_rn {
                (best_rn, since_best) = (rn, 0);
            } else {
                since_best += 1;
            }
            // First remedy for a chattering patch edge: hold the active set (points of ~zero pressure entering and leaving
            // the contact were what the residual alternated on) and iterate on with the smooth branch.
            // (Frictional contact only: holding every stagnating contact solve was tried and broke
            // `a_lightly_loaded_stiff_penalty_contact_converges_in_few_factorisations`: a free pin that has not touched yet
            // was held out of contact, and its rigid-body dofs went singular.)
            if frictional && since_best >= STAGNATION_ITERATIONS {
                if let Some(cs) = self.contacts.as_ref().filter(|cs| !cs.active_set_held()) {
                    cs.hold_active_set(&ev.state.contact);
                    self.held.set(self.held.get() + 1);
                    if std::env::var("NL_DEBUG").is_ok() {
                        eprintln!("HELD stagnated at lambda {lambda:.4} (residual {rn:.3e}, best {best_rn:.3e}): holding the active set");
                    }
                    budget += HOLD_EXTRA_ITERATIONS;
                    ev = self.eval(&u, &q, state0, lambda, true)?;
                    r = self.residual(&ev, lambda);
                    rn = norm(&r);
                    residuals.push(rn);
                    (best_rn, since_best) = (rn, 0);
                    lowest = None;
                    prev = None;
                    chord = None;
                    continue;
                }
            }
            if frictional && since_best >= STAGNATION_ITERATIONS && self.opt.stall_tol > 0.0 {
                if let Some((_, lu, lq, mut lst)) = lowest.take().filter(|l| l.0 <= self.opt.stall_tol * self.scale(&ev, lambda)) {
                    lst.rigid_q = lq;
                    self.stalled.set(self.stalled.get() + 1);
                    return Ok((lu, lst, residuals));
                }
            }
            if frictional && since_best >= STAGNATION_ITERATIONS {
                return Err(format!("{STAGNATED} (residual {rn:.3e}, best {best_rn:.3e})"));
            }
            // Backtrack when the last full step increased the residual.
            if self.opt.line_search {
                if let Some((up, qp, dp, rp)) = prev.take() {
                    if rn > rp {
                        let mut alpha = 0.5;
                        // Contact with a free rigid body can overshoot by orders of magnitude (a
                        // penalty stiffness met after free flight), so backtrack further there.
                        for _ in 0..if self.contacts.is_some() { 24 } else { 6 } {
                            let (mut ut, mut qt) = (up.clone(), qp.clone());
                            for (j, &i) in self.free.iter().enumerate() {
                                ut[i] += alpha * dp[j];
                            }
                            for k in 0..qt.len() {
                                qt[k] += alpha * dp[nf + k];
                            }
                            if let Ok(e2) = self.eval(&ut, &qt, state0, lambda, true) {
                                let r2 = self.residual(&e2, lambda);
                                let n2 = norm(&r2);
                                if n2 <= rp || (alpha < 0.05 && self.contacts.is_none()) {
                                    u = ut;
                                    q = qt;
                                    ev = e2;
                                    r = r2;
                                    rn = n2;
                                    break;
                                }
                            }
                            alpha *= 0.5;
                        }
                        residuals.push(rn);
                    }
                }
            }
            // Refactor unless a recent factor is still contracting fast enough.
            let reuse = self.opt.chord_iters > 0 && chord.as_ref().is_some_and(|(_, age)| *age < self.opt.chord_iters) && rn < 0.25 * last_rn;
            if !reuse {
                // Cholesky; with contact the (symmetrised) frictional tangent can be indefinite, so LDL^T is the fallback there.
                let k = ev.k.as_ref().expect("tangent");
                let t_fac = Instant::now();
                // (Follower loads too: the symmetric part of their load stiffness is indefinite well before the body is.)
                let fac = if self.contacts.is_some() || !self.followers.is_empty() { self.red.factor_any(k) } else { self.red.factor(k).map(crate::linear::AnyFactor::Llt) }.map_err(|e| e.to_string())?;
                self.timers[1].set(self.timers[1].get() + t_fac.elapsed().as_secs_f64());
                self.factorisations.set(self.factorisations.get() + 1);
                chord = Some((fac, 0));
            }
            if std::env::var("NL_FDCHECK").is_ok() {
                self.fd_check(&ev, &u, &q, state0, lambda, &r)?;
            }
            let (fac, age) = chord.as_mut().expect("a factorization");
            let t_sol = Instant::now();
            let dx = self.newton_step(fac, &ev, &r)?;
            self.timers[2].set(self.timers[2].get() + t_sol.elapsed().as_secs_f64());
            *age += 1;
            last_rn = rn;
            let before = (u.clone(), q.clone(), dx.clone(), rn);
            for (j, &i) in self.free.iter().enumerate() {
                u[i] += dx[j];
            }
            for k in 0..q.len() {
                q[k] += dx[nf + k];
            }
            ev = self.eval(&u, &q, state0, lambda, true)?;
            r = self.residual(&ev, lambda);
            rn = norm(&r);
            if let Ok(mode) = std::env::var("NL_TRACE") {
                let dxn = dx.iter().map(|v| v * v).sum::<f64>().sqrt();
                let summary = self.contacts.as_ref().map_or(String::new(), |cs| cs.describe(&ev.state.contact));
                // The two largest residual dofs: a residual that stays put while `dx` shrinks is a discontinuity at them.
                let mut idx: Vec<usize> = (0..self.free.len()).collect();
                idx.sort_by(|&a, &b| r[b].abs().total_cmp(&r[a].abs()));
                let d = self.model.mesh.dim();
                let top: Vec<String> = idx.iter().take(2).map(|&j| {
                    let (dof, node) = (self.free[j], self.free[j] / d);
                    format!("node {node} comp {} r {:.1e} at ({:.3}, {:.3})", dof % d, r[j], self.model.mesh.nodes[node][0], self.model.mesh.nodes[node][1])
                }).collect();
                let gap = ev.stats.as_ref().map_or(0.0, |s| s.min_gap);
                eprintln!("  it rn {rn:.6e} |dx| {dxn:.1e} min gap {gap:.2e} q {q:?} {summary} | {}", top.join(" | "));
                if mode == "full" {
                    let act: Vec<String> = ev.state.contact.first().map_or(vec![], |v| v.iter().enumerate().filter(|(_, c)| c.active).map(|(i, c)| format!("{i}:{:.1e}", c.p)).collect());
                    eprintln!("    active {}", act.join(" "));
                }
            }
            residuals.push(rn);
            prev = Some(before);
        }
        if self.opt.stall_tol > 0.0 && rn.is_finite() {
            if let Some((_, lu, lq, mut lst)) = lowest.take().filter(|l| l.0 <= self.opt.stall_tol * self.scale(&ev, lambda)) {
                lst.rigid_q = lq;
                self.stalled.set(self.stalled.get() + 1);
                return Ok((lu, lst, residuals));
            }
        }
        if std::env::var("NL_DEBUG").is_ok() {
            eprintln!("newton lambda {lambda:.3e} failed (force scale {:.3e}, elastic {:.3e}): residuals {:?}", self.scale(&ev, lambda), ev.f_elem, residuals.iter().map(|r| format!("{r:.2e}")).collect::<Vec<_>>());
        }
        Err(format!("no convergence in {} iterations (residual {rn:.3e})", self.opt.max_iter))
    }

    /// Debug aid: compare the analytic tangent with central differences of the residual along a few directions.
    fn fd_check(&self, ev: &Eval, u: &[f64], q: &[f64], state0: &NlState, lambda: f64, r0: &[f64]) -> Result<(), String> {
        let nf = self.free.len();
        let m = r0.len() - nf;
        let k = ev.k.as_ref().expect("tangent");
        for trial in 0..5 {
            // Direction: smooth pseudo-random; trials 3 and 4 move only the mesh / only the rigid body.
            let mut dx: Vec<f64> = (0..nf + m).map(|i| ((i * 7919 + trial * 104729) as f64 * 0.618).sin()).collect();
            if trial == 3 {
                dx[nf..].fill(0.0);
            } else if trial == 4 {
                dx[..nf].fill(0.0);
            }
            let mut du = vec![0.0; u.len()];
            for (j, &i) in self.free.iter().enumerate() {
                du[i] = dx[j];
            }
            let mut kd = vec![0.0; u.len()];
            k.matvec_add(&self.pat, &du, &mut kd);
            let mut jd: Vec<f64> = self.free.iter().map(|&i| kd[i]).collect();
            let dq: Vec<f64> = dx[nf..].to_vec();
            if let Some(ex) = &ev.ex {
                let ndm = self.model.mesh.n_dofs();
                for (f, &dof) in self.free.iter().enumerate() {
                    for j in 0..m {
                        jd[f] += ex.kuq[j * ndm + dof] * dq[j];
                    }
                }
                for i in 0..m {
                    let mut v: f64 = (0..m).map(|j| ex.kqq[i * m + j] * dq[j]).sum();
                    for (f, &dof) in self.free.iter().enumerate() {
                        v += ex.kuq[i * ndm + dof] * dx[f];
                    }
                    jd.push(v);
                }
            }
            let h = std::env::var("NL_FDH").ok().and_then(|v| v.parse().ok()).unwrap_or(1e-7);
            let eval_at = |sgn: f64| -> Result<Vec<f64>, String> {
                let mut ut = u.to_vec();
                for (j, &i) in self.free.iter().enumerate() {
                    ut[i] += sgn * h * dx[j];
                }
                let qt: Vec<f64> = q.iter().zip(&dq).map(|(a, b)| a + sgn * h * b).collect();
                let e = self.eval(&ut, &qt, state0, lambda, false)?;
                Ok(self.residual(&e, lambda))
            };
            let (rp, rm) = (eval_at(1.0)?, eval_at(-1.0)?);
            let fd: Vec<f64> = rp.iter().zip(&rm).map(|(a, b)| (a - b) / (2.0 * h)).collect();
            let err_u = (0..nf).map(|i| (fd[i] - jd[i]).powi(2)).sum::<f64>().sqrt() / norm(&jd[..nf]).max(1e-300);
            let err_q = (nf..nf + m).map(|i| (fd[i] - jd[i]).powi(2)).sum::<f64>().sqrt() / norm(&jd[nf..]).max(1e-300);
            eprintln!("  fdcheck trial {trial}: free-row rel err {err_u:.2e}, extra-row rel err {err_q:.2e}, |Jd_q| {:.3e}", norm(&jd[nf..]));
        }
        let _ = r0;
        Ok(())
    }

    fn initial_state(&self) -> NlState {
        let mut s = NlState::new(&self.model.mesh);
        if let Some(cs) = &self.contacts {
            s.contact = cs.initial_state(&self.model.mesh, &self.pat);
            s.rigid_q = vec![0.0; cs.n_extra()];
        }
        s
    }

    /// The state a run starts from: rest, or the converged `start` (the displacements and contact states of
    /// interfaces that existed before; bodies appended later start at rest).
    fn initial_with_start(&self) -> Result<(Vec<f64>, NlState, f64), String> {
        let n = self.model.mesh.n_dofs();
        let (mut u, mut state) = (vec![0.0; n], self.initial_state());
        if let Some(st) = self.start {
            if st.u.len() > n {
                return Err("the start state does not belong to this model".into());
            }
            u[..st.u.len()].copy_from_slice(&st.u);
            if let Some(gp) = &st.gp {
                if gp.len() == state.gp.len() {
                    state.gp = gp.clone();
                }
            }
            for (i, c) in st.contact.iter().enumerate() {
                if let (Some(c), Some(slot)) = (c, state.contact.get_mut(i)) {
                    if c.len() != slot.len() {
                        return Err("the start state's contact points do not match".into());
                    }
                    slot.copy_from_slice(c);
                }
            }
        }
        Ok((u, state, self.start.map_or(0.0, |s| s.lambda)))
    }

    /// Augmented-Lagrangian passes at a converged load factor: update the multipliers from the converged pressures
    /// and re-solve until they stop changing, then commit the slip history. `residuals` collects the Newton work.
    fn refine_contact(&self, lambda_prev: f64, target: f64, mut un: Vec<f64>, mut sn: NlState, residuals: &mut Vec<f64>) -> (Vec<f64>, NlState) {
        let Some(cs) = &self.contacts else { return (un, sn) };
        // The multipliers `un` was solved for. The update that ends the passes is not re-solved, and a state that pairs
        // `un` with the updated multipliers is off by that update: reactions and master forces read off it miss the
        // equilibrium by `outer_tol x` the fit pressure, which is large next to a small applied load.
        let mut solved = sn.contact.clone();
        let mut ended = false;
        for _ in 0..self.opt.max_outer {
            solved = sn.contact.clone();
            let chg = cs.update_multipliers(&mut sn.contact);
            if chg < self.opt.outer_tol {
                ended = true;
                break;
            }
            match self.newton(lambda_prev, target, &un, &sn) {
                Ok((u2, s2, r2)) => {
                    if std::env::var("NL_DEBUG").is_ok() {
                        eprintln!("  AL pass at lambda {target:.4}: multiplier change {chg:.2e}, {} residual evaluations", r2.len());
                    }
                    un = u2;
                    sn = s2;
                    residuals.extend(r2);
                }
                Err(_) => {
                    ended = true;
                    break;
                }
            }
        }
        for (pts, old) in sn.contact.iter_mut().zip(&solved) {
            for (p, o) in pts.iter_mut().zip(old) {
                if ended {
                    p.lam_n = o.lam_n;
                }
                // The friction multiplier is the friction of the iterate: committing the slip history zeroes the slip, so
                // the friction a re-evaluation reproduces is exactly the multiplier.
                p.lam_t = if p.active { p.fric } else { [0.0; 3] };
            }
        }
        cs.commit_history(&mut sn.contact);
        (un, sn)
    }

    fn run_load_control(&mut self) -> Result<NlSolution, String> {
        let (mut u, mut state, lambda0) = self.initial_with_start()?;
        let (mut lambda, dl_max) = (lambda0, 1.0 / self.opt.steps.max(1) as f64);
        let mut dl = self.opt.first_step * dl_max;
        let (mut steps, mut cuts_total, mut stagnated_cuts) = (Vec::new(), 0usize, 0usize);
        while lambda < 1.0 - 1e-12 {
            if self.opt.interrupt.triggered() {
                return Ok(self.finish(u, state, lambda, steps, Stop::Interrupted));
            }
            let target = (lambda + dl).min(1.0);
            let lambda_before = lambda;
            match self.newton(lambda, target, &u, &state) {
                Ok((un, sn, mut residuals)) => {
                    let (mut un, mut sn) = (un, sn);
                    // Step growth follows the Newton effort of the step itself, not of its AL passes.
                    let newton_effort = residuals.len();
                    (un, sn) = self.refine_contact(lambda, target, un, sn, &mut residuals);
                    lambda = target;
                    if std::env::var("NL_DEBUG").is_ok() {
                        eprintln!("step ok lambda {target:.4} effort {newton_effort} total {}", residuals.len());
                    }
                    let iterations = residuals.len();
                    let max_ep = sn.max_ep();
                    let master_force = if self.contacts.is_some() { self.eval(&un, &sn.rigid_q, &sn, lambda, false).ok().and_then(|e| e.stats).map_or(Vec::new(), |st| st.master_force) } else { Vec::new() };
                    // A step that carries the first master well past the target force is taken again with half the step
                    // (the force rises steeply once a body is in contact, so a uniform step can jump far over the target).
                    if self.opt.stop_at_force > 0.0 && master_force.first().is_some_and(|f| norm(f) > 1.25 * self.opt.stop_at_force) && dl > 1e-9 {
                        lambda = lambda_before;
                        dl *= 0.5;
                        continue;
                    }
                    if let Some(obs) = &self.opt.observer {
                        (obs.0)(&StepEvent { step: steps.len() + 1, lambda, master_force: master_force.first().map(|f| norm(f)), factorisations: self.factorisations.get() });
                    }
                    steps.push(StepInfo { lambda, iterations, residuals, max_ep, plastic_fraction: sn.plastic_fraction(), cuts: cuts_total, indefinite: false, master_force });
                    u = un;
                    state = sn;
                    cuts_total = 0;
                    // Frictional contact converges linearly (the sliding traction is frozen in the tangent), so
                    // its first pass legitimately takes 10-15 iterations.
                    if newton_effort <= if self.contacts.is_some() { 16 } else { 5 } {
                        dl = (dl * 1.5).min(dl_max);
                    }
                    if self.opt.failure_ep.is_some_and(|lim| max_ep >= lim) {
                        return Ok(self.finish(u, state, lambda, steps, Stop::FailureStrain));
                    }
                    if self.opt.stop_at_force > 0.0 && steps.last().and_then(|s| s.master_force.first()).is_some_and(|f| norm(f) >= self.opt.stop_at_force) {
                        return Ok(self.finish(u, state, lambda, steps, Stop::ForceReached));
                    }
                    if (self.opt.stop_on_plateau && load_flat(&steps, 0.012)) || (self.opt.stop_on_fall > 0.0 && load_fell(&steps, self.opt.stop_on_fall)) {
                        return Ok(self.finish(u, state, lambda, steps, Stop::Plateau));
                    }
                }
                Err(e) => {
                    if self.opt.interrupt.triggered() {
                        return Ok(self.finish(u, state, lambda, steps, Stop::Interrupted));
                    }
                    if std::env::var("NL_DEBUG").is_ok() {
                        eprintln!("step to lambda {target:.4} cut: {e}");
                    }
                    cuts_total += 1;
                    if self.opt.stick_slip_guard && e.starts_with(STAGNATED) {
                        stagnated_cuts += 1;
                        if stagnated_cuts > STAGNATION_CUTS {
                            return Ok(self.finish(u, state, lambda, steps, Stop::NoConvergence(e)));
                        }
                    }
                    dl *= 0.5;
                    // A displacement-driven pin that cannot be pushed further once its load has all but flattened has
                    // reached its limit state (Newton fails on the singular plateau): not a failure.
                    if self.opt.stop_on_plateau && cuts_total >= 2 && load_flat(&steps, 0.04) {
                        return Ok(self.finish(u, state, lambda, steps, Stop::Plateau));
                    }
                    if cuts_total > self.opt.max_cuts {
                        return Ok(self.finish(u, state, lambda, steps, Stop::NoConvergence(e)));
                    }
                }
            }
        }
        Ok(self.finish(u, state, lambda, steps, Stop::Completed))
    }

    fn run_arc_length(&mut self, ds0: f64, lambda_max: f64, max_steps: usize) -> Result<NlSolution, String> {
        let (mut u, mut state, lambda0) = self.initial_with_start()?;
        let (mut lambda, mut ds) = (lambda0, ds0);
        let mut steps: Vec<StepInfo> = Vec::new();
        let mut dir: Option<(Vec<f64>, f64)> = None; // previous converged increment (free dofs, load factor)
        let mut cuts = 0usize;
        let fext_free: Vec<f64> = self.free.iter().map(|&i| self.f_ext[i]).collect();
        while steps.len() < max_steps {
            if self.opt.interrupt.triggered() {
                return Ok(self.finish(u, state, lambda, steps, Stop::Interrupted));
            }
            if lambda >= lambda_max {
                return Ok(self.finish(u, state, lambda, steps, Stop::Completed));
            }
            // Start every step from an equilibrated state: a contact (or friction) state accepted a hair above the
            // tolerance leaves a residual the arc-length constraint cannot remove once the arc gets short. One
            // load-controlled Newton at the current load (the arc-length path itself passes through such states).
            if self.contacts.is_some() && !steps.is_empty() {
                if let Ok((up, sp, _)) = self.newton(lambda, lambda, &u, &state) {
                    u = up;
                    state = sp;
                }
            }
            // Predictor: the first step follows the tangent response to the load; every later step
            // extrapolates the previous converged increment (a secant predictor), which stays on the
            // path through limit points and perfect-plasticity plateaus where the tangent is singular
            // and the sign of K^-1 f is meaningless.
            let (mut du, mut dlam): (Vec<f64>, f64) = match dir.as_ref().filter(|(d, _)| norm(d) > 1e-300) {
                Some((dprev, dlprev)) => {
                    let k = ds / norm(dprev);
                    (dprev.iter().map(|v| v * k).collect(), dlprev * k)
                }
                None => {
                    let ev0 = match self.eval(&u, &state.rigid_q, &state, lambda, true) {
                        Ok(e) => e,
                        Err(e) => return Ok(self.finish(u, state, lambda, steps, Stop::NoConvergence(e))),
                    };
                    let mut duf = fext_free.clone();
                    if let Err(e) = self.solve_any(&ev0, &mut duf) {
                        return Ok(self.finish(u, state, lambda, steps, Stop::NoConvergence(e)));
                    }
                    let k = ds / norm(&duf).max(1e-300);
                    (duf.iter().map(|v| v * k).collect(), k)
                }
            };
            let mut converged = None;
            let mut residuals = Vec::new();
            let mut failed = false;
            let mut reason = String::new();
            let mut indefinite = false;
            for _ in 0..=self.opt.max_iter {
                let mut ut = u.clone();
                for (j, &i) in self.free.iter().enumerate() {
                    ut[i] += du[j];
                }
                let ev = match self.eval(&ut, &state.rigid_q, &state, lambda + dlam, true) {
                    Ok(e) => e,
                    Err(e) => {
                        failed = true;
                        reason = format!("element evaluation failed: {e}");
                        break;
                    }
                };
                let lam = lambda + dlam;
                let r = self.residual(&ev, lam);
                let rn = norm(&r);
                residuals.push(rn);
                if rn <= self.opt.tol * self.scale(&ev, lam) || rn <= self.abs_tol() {
                    converged = Some((ut, ev.state, lam));
                    break;
                }
                if !rn.is_finite() {
                    failed = true;
                    reason = "non-finite residual".into();
                    break;
                }
                let mut dur: Vec<f64> = r.iter().map(|v| -v).collect();
                let mut duf2 = fext_free.clone();
                match self.solve2(&ev, &mut dur, &mut duf2) {
                    Ok(ind) => indefinite |= ind,
                    Err(e) => {
                        failed = true;
                        reason = format!("tangent solve failed: {e}");
                        break;
                    }
                }
                // |du + dur + dl duf2|^2 = ds^2.
                let base: Vec<f64> = du.iter().zip(&dur).map(|(a, b)| a + b).collect();
                let a: f64 = duf2.iter().map(|v| v * v).sum();
                let b: f64 = duf2.iter().zip(&base).map(|(x, y)| x * y).sum();
                let c: f64 = base.iter().map(|v| v * v).sum::<f64>() - ds * ds;
                let disc = b * b - a * c;
                if disc < 0.0 || a <= 0.0 {
                    failed = true;
                    reason = format!("the arc-length constraint has no real root (discriminant {disc:.3e})");
                    break;
                }
                let (r1, r2) = ((-b + disc.sqrt()) / a, (-b - disc.sqrt()) / a);
                // The root that keeps going the same way as the increment so far.
                let dot = |dl: f64| -> f64 { du.iter().zip(base.iter().zip(&duf2)).map(|(d0, (bb, ff))| d0 * (bb + dl * ff)).sum() };
                let ddl = if dot(r1) >= dot(r2) { r1 } else { r2 };
                for j in 0..du.len() {
                    du[j] = base[j] + ddl * duf2[j];
                }
                dlam += ddl;
            }
            match (converged, failed) {
                (Some((un, sn, lam)), false) => {
                    // The step overshot the target load: land on it exactly with a load-controlled step from the last
                    // converged state (the arc-length path has found the contact; load control finishes it).
                    if lam > lambda_max {
                        return match self.newton(lambda, lambda_max, &u, &state) {
                            Ok((ul, sl, mut res)) => {
                                let (ul, sl) = self.refine_contact(lambda, lambda_max, ul, sl, &mut res);
                                let (max_ep, iterations) = (sl.max_ep(), res.len());
                                steps.push(StepInfo { lambda: lambda_max, iterations, residuals: res, max_ep, plastic_fraction: sl.plastic_fraction(), cuts, indefinite, master_force: Vec::new() });
                                Ok(self.finish(ul, sl, lambda_max, steps, Stop::Completed))
                            }
                            Err(_) => {
                                // Not landing is not fatal: shorten the arc and try again from here.
                                cuts += 1;
                                ds *= 0.5;
                                if cuts > self.opt.max_cuts {
                                    return Ok(self.finish(u, state, lambda, steps, Stop::NoConvergence(format!("could not land on lambda {lambda_max}"))));
                                }
                                continue;
                            }
                        };
                    }
                    let (un, sn) = if self.contacts.is_some() {
                        let mut extra = Vec::new();
                        self.refine_contact(lam, lam, un, sn, &mut extra)
                    } else {
                        (un, sn)
                    };
                    let max_ep = sn.max_ep();
                    let iterations = residuals.len();
                    if std::env::var("NL_DEBUG").is_ok() {
                        eprintln!("arc step ok: lambda {lam:.5e} ds {ds:.2e} iterations {iterations}");
                    }
                    steps.push(StepInfo { lambda: lam, iterations, residuals, max_ep, plastic_fraction: sn.plastic_fraction(), cuts, indefinite, master_force: Vec::new() });
                    dir = Some((du, dlam));
                    u = un;
                    state = sn;
                    lambda = lam;
                    cuts = 0;
                    if iterations <= 4 {
                        ds = (ds * 1.5).min(4.0 * ds0);
                    } else if iterations > 8 {
                        ds *= 0.6;
                    }
                    if self.opt.failure_ep.is_some_and(|lim| max_ep >= lim) {
                        return Ok(self.finish(u, state, lambda, steps, Stop::FailureStrain));
                    }
                }
                _ => {
                    if std::env::var("NL_DEBUG").is_ok() {
                        eprintln!("arc step from lambda {lambda:.5e} failed at ds {ds:.2e}: {reason} (residuals {:?})", residuals.iter().map(|r| format!("{r:.1e}")).collect::<Vec<_>>());
                    }
                    cuts += 1;
                    ds *= 0.5;
                    if reason.is_empty() {
                        reason = format!("no convergence in {} iterations (residuals {:?})", self.opt.max_iter, residuals.iter().rev().take(4).rev().map(|r| format!("{r:.2e}")).collect::<Vec<_>>());
                    }
                    if cuts > self.opt.max_cuts {
                        return Ok(self.finish(u, state, lambda, steps, Stop::NoConvergence(format!("arc-length step could not converge at lambda {lambda:.5} (ds {ds:.2e}): {reason}"))));
                    }
                }
            }
        }
        Ok(self.finish(u, state, lambda, steps, Stop::StepLimit))
    }
}

#[cfg(test)]
mod tangent_tests {
    use super::*;

    /// The 2D specialisation of the element tangent equals the generic 9 x 9 contraction.
    #[test]
    fn the_2d_tangent_accumulation_matches_the_generic_one() {
        let nn = 9;
        let nd = 2 * nn;
        let gr: Vec<f64> = (0..nn * 2).map(|i| ((i * 37 + 11) as f64 * 0.731).sin()).collect();
        let nsh = vec![0.1; nn];
        let mut tangent = [[0.0f64; 9]; 9];
        for r in 0..9 {
            for c in 0..9 {
                tangent[r][c] = ((r * 13 + c * 7 + 3) as f64 * 0.37).cos();
            }
        }
        let (mut a, mut b) = (vec![0.0; nd * nd], vec![0.0; nd * nd]);
        accumulate_tangent_generic(&mut a, nn, 2, nd, &gr, &nsh, 1.0, false, &tangent, 0.7);
        accumulate_tangent_2d(&mut b, nn, &gr, &tangent, 0.7);
        let diff = a.iter().zip(&b).fold(0.0f64, |m, (x, y)| m.max((x - y).abs()));
        assert!(diff < 1e-12, "{diff:e}");
    }
}

#[cfg(test)]
mod plateau_tests {
    use super::*;

    fn step(lambda: f64, load: f64) -> StepInfo {
        StepInfo { lambda, iterations: 5, residuals: vec![], max_ep: 0.0, plastic_fraction: 0.0, cuts: 0, indefinite: false, master_force: vec![[load, 0.0, 0.0]] }
    }

    #[test]
    fn a_plateau_is_read_on_travel_not_on_the_number_of_steps() {
        // Rising load with regular steps: not flat. Flat over 6 % of the travel: flat.
        let rising: Vec<StepInfo> = (1..=10).map(|i| step(0.03 * i as f64, 1000.0 * i as f64)).collect();
        assert!(!load_flat(&rising, 0.012));
        let mut flat = rising.clone();
        flat.extend((11..=13).map(|i| step(0.03 * i as f64, 10_000.0)));
        assert!(load_flat(&flat, 0.012), "flat over the last 9 % of the travel");
        // After a convergence trouble the steps shrink: three equal loads over a sliver of travel say nothing.
        let mut tiny = rising.clone();
        tiny.extend([step(0.3005, 10_000.0), step(0.3011, 10_000.0), step(0.3015, 10_000.0)]);
        assert!(!load_flat(&tiny, 0.012), "tiny steps must not read as a plateau");
        // A falling load is flat (the collapse is behind it).
        let mut falling = rising;
        falling.extend((11..=14).map(|i| step(0.03 * i as f64, 10_000.0 - 400.0 * (i - 10) as f64)));
        assert!(load_flat(&falling, 0.012));
        assert!(load_fell(&falling, 0.1) && !load_fell(&falling, 0.2));
    }
}
