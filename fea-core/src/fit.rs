//! Controls and contact builder shared by every bushing-in-housing analysis on the kernel (the lug's
//! pressed bushing, the eccentric bushing): the speed-versus-tightness knobs of the augmented-Lagrangian
//! Newton loop and the interference-fit interface (two passes, Coulomb friction, overlap).

use crate::contact::{ContactRule, ContactSpec};
use crate::nonlinear::{NlOptions, NlSolution, Start};

/// Augmented-Lagrangian / Newton controls of the kernel runs: the speed-versus-tightness knobs. The
/// multiplier passes remove the penalty error, so a soft penalty only speeds Newton up (measured on
/// the lug: 10 E/a is 5-13x faster than 100 E/a for the same answer, `docs/fea-core.md`).
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    /// Normal penalty in units of `E / bore radius`.
    pub penalty_factor: f64,
    /// Multiplier passes per load step.
    pub max_outer: usize,
    /// Stop the passes when the multipliers change by less than this (relative to the peak pressure).
    pub outer_tol: f64,
    /// Chord-Newton reuse of one factorisation (0 = full Newton).
    pub chord_iters: usize,
    /// Newton residual tolerance (relative).
    pub newton_tol: f64,
    /// Displacement-driven runs: the pin travel of one load step as a fraction of the bore radius.
    pub step_fraction: f64,
    /// Accept a Newton iteration that stalls (chattering friction) below this fraction of the force scale.
    pub stall_tol: f64,
    /// The first load step as a fraction of the regular one (a displacement-driven pin meets its stiffest transition at the first touch).
    pub first_step: f64,
    /// `NlOptions::stick_slip_guard`: hold the contact active set when a frictional solve cycles at a patch edge.
    pub stick_slip_guard: bool,
}

/// Tangential penalty over normal penalty of every frictional contact in the workspace (see [`Tuning::eps_t`]).
pub const EPS_T_RATIO: f64 = 0.03;

impl Tuning {
    /// Frictionless elastic contact (one load step is enough: the response is smooth).
    pub fn frictionless() -> Self {
        Self { penalty_factor: 10.0, max_outer: 8, outer_tol: 5e-3, chord_iters: 0, newton_tol: 1e-4, step_fraction: 0.015, stall_tol: 0.0, first_step: 1.0, stick_slip_guard: false }
    }

    /// Coulomb friction: a stiffer penalty and tighter passes (sticking multipliers converge slowly, and a
    /// soft penalty leaves the peak pressure ~10 % low).
    pub fn friction() -> Self {
        Self { penalty_factor: 30.0, max_outer: 12, outer_tol: 2e-3, chord_iters: 0, newton_tol: 1e-4, step_fraction: 0.015, stall_tol: 0.0, first_step: 1.0, stick_slip_guard: true }
    }

    /// Displacement-driven plastic collapse: one multiplier pass per step and a 1e-4 Newton tolerance (the
    /// load-travel curve is read at every step; the collapse load of an oblique case with friction moved
    /// 0.01 % against two passes at 1e-6, in 60 % of the time). Steps larger than 0.015 of the bore radius
    /// shifted the plateau by 2 %.
    pub fn collapse() -> Self {
        Self { penalty_factor: 10.0, max_outer: 1, outer_tol: 1e-2, chord_iters: 0, newton_tol: 1e-4, step_fraction: 0.015, stall_tol: 1e-3, first_step: 0.25, stick_slip_guard: false }
    }

    /// The tight reference setting (deformable contact, the 3D models).
    pub fn reference() -> Self {
        Self { penalty_factor: 100.0, max_outer: 12, outer_tol: 2e-3, chord_iters: 0, newton_tol: 1e-9, step_fraction: 0.015, stall_tol: 0.0, first_step: 1.0, stick_slip_guard: false }
    }

    pub fn options(&self, steps: usize) -> NlOptions {
        NlOptions { steps, outer_tol: self.outer_tol, max_outer: self.max_outer, chord_iters: self.chord_iters, tol: self.newton_tol, stall_tol: self.stall_tol, first_step: self.first_step, stick_slip_guard: self.stick_slip_guard, ..NlOptions::default() }
    }

    pub fn eps_n(&self, e_psi: f64, bore_radius: f64) -> f64 {
        self.penalty_factor * e_psi / bore_radius
    }

    /// The tangential (friction) penalty that goes with a normal penalty `eps_n`: a soft one, `EPS_T_RATIO x eps_n`. The
    /// multiplier passes make the answer independent of it, and a stiffer one makes partial slip hard for Newton: stacked
    /// 3D blocks of different moduli sheared near the friction limit converged at 0.03 and 0.01 and stagnated at 0.1.
    pub fn eps_t(&self, eps_n: f64) -> f64 {
        EPS_T_RATIO * eps_n
    }
}

/// The interference-fit interface of a bushing pressed into a housing bore: two deformable passes (each body the
/// slave of the other), reduced collocation, Coulomb friction `mu` with a soft tangential penalty, and the radial
/// `overlap` (interference) as the initial penetration. `housing` and `bushing` are the facing edges in kernel
/// order `[corner, corner, midside]`; `margin` bounds how far the interface may slide (the matrix pattern is
/// built once from the initial proximity).
pub fn interference_contacts(name: &str, housing: Vec<Vec<usize>>, bushing: Vec<Vec<usize>>, eps_n: f64, mu: f64, overlap: f64, margin: f64) -> [ContactSpec; 2] {
    ContactSpec::two_pass(name, housing, bushing, eps_n).map(|spec| spec.with_rule(ContactRule::Reduced).with_friction(mu, EPS_T_RATIO * eps_n).with_overlap(overlap).with_margin(margin))
}

/// The converged interference-fit state (interface contacts only) as the start of a run that continues from it:
/// displacements, the interface contacts and the Gauss-point material state are kept. `leading` contacts that are
/// new in the continued run and come before the interfaces (the pin) start fresh.
pub fn start_after_fit(fit: &NlSolution, leading: usize) -> Start {
    let mut contact = vec![None; leading];
    contact.extend(fit.state.contact.iter().cloned().map(Some));
    Start { u: fit.u.clone(), contact, gp: Some(fit.state.gp.clone()), lambda: 0.0 }
}

/// The state a run stopped in, as the start of a continued one at load factor `lambda` of the new run (displacements,
/// every contact state and the material state are kept, the contacts keep their order).
pub fn start_from(sol: &NlSolution, lambda: f64) -> Start {
    Start { u: sol.u.clone(), contact: sol.state.contact.iter().cloned().map(Some).collect(), gp: Some(sol.state.gp.clone()), lambda }
}
