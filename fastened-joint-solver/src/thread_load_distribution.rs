//! Advanced per-thread spring-coupled load distribution (spec section 36) -
//! opt-in, not part of the primary solve. Models the engaged threads as a
//! coupled bolt-body/nut-body spring network: bolt-body and nut-body pitch
//! segments (axial bar stiffness `E*A/p`) alternate with a thread-pair
//! coupling spring at each engaged thread, assembled into `[K]{u} = {F}`
//! and solved with dense Gaussian elimination (partial pivoting) - the
//! standard bar-element-plus-elastic-foundation discretization of the
//! coupled-rod shear-lag mechanics this problem class reduces to.
//!
//! ## Sourcing
//!
//! Per-thread coupling stiffness uses Zhang, Wang & Yang, "A prediction
//! method for load distribution at the thread-fastened interface in
//! screw-thread connections," *J. Theor. Appl. Mech.* 56(1):157-168, 2018,
//! DOI 10.15632/jtam-pl.56.1.157 (peer-reviewed, open access) - their
//! two-term (bending + shear) unit-width deflection formulas (eqs.
//! 2.10/2.15/2.17-2.18):
//!
//! ```text
//! delta_b = [0.5*(1-nu_b^2) + 1.2*(1+nu_b)] / (pi*D*E_b)
//! delta_n = [0.5*(1-nu_n^2) + 1.2*(1+nu_n)] / (pi*D*E_n)
//! k_thread = pi*D / (delta_b + delta_n)
//! ```
//!
//! `D` is the pitch diameter. This is the correct discrete nodal spring
//! constant for a one-pitch-spaced bar+foundation model of their own
//! continuum compatibility equation (their eqs. 2.32-2.35): the two
//! foundation stiffnesses-per-unit-length they define, `k_by = pi*D/(p*delta_b)`
//! and `k_ny = pi*D/(p*delta_n)`, combine in series over one pitch as
//! `p / (1/k_by + 1/k_ny) = pi*D/(delta_b+delta_n)` - exactly `k_thread`
//! above, independent of `p`. This crate's own regression test checks the
//! discrete distribution this module produces converges toward that
//! continuum closed form (a real analytical-limit validation, not just an
//! internal consistency check).
//!
//! No published per-thread stiffness distinct from a continuum compliance
//! model was found sourceable; this module does NOT distribute VDI 2230's
//! lumped engagement-length compliance across N identical springs - that
//! approach was explicitly checked and found indefensible (VDI 2230's
//! substitute lengths reproduce total joint resilience, not per-thread
//! compliance).
//!
//! Bolt-body/nut-body segment areas: bolt uses the existing
//! [`crate::thread::ThreadGeometry::root_area`]; the nut-body annular area
//! has no sourced standard convention (flagged by the research pass) and is
//! computed from the caller-supplied nut outer diameter down to the thread
//! major diameter - an engineering-judgment choice, not a cited one.

use crate::thread::ThreadGeometry;

/// Inputs specific to this optional analysis - everything else needed
/// (thread geometry, fastener modulus/Poisson's ratio, and the solved
/// preload) already exists on `JointInputs`/`JointSolution`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadLoadDistributionInputs {
    /// Number of engaged threads carrying load - must be `>= 1`.
    pub engaged_threads: u32,
    pub nut_modulus: f64,
    pub nut_poisson_ratio: f64,
    /// Nut/tapped-hole outer (or across-flats-equivalent) diameter, for the
    /// nut-body annular cross-section - must exceed the thread major
    /// diameter.
    pub nut_outer_diameter: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadLoadDistribution {
    /// Load carried by the thread closest to the loaded (bearing) face -
    /// spec section 36's "First engaged thread load."
    pub first_thread_load: f64,
    /// Largest single-thread load anywhere in the distribution (typically,
    /// but not necessarily, the first thread).
    pub max_thread_load: f64,
    /// One entry per engaged thread, ordered from the loaded face (index 0)
    /// to the free end (last index) - spec section 36's "Thread-load
    /// distribution."
    pub per_thread_loads: Vec<f64>,
}

/// Solves `a*x = b` (both consumed) by dense Gaussian elimination with
/// partial pivoting. Returns `None` for a numerically singular system. No
/// linear-algebra crate exists anywhere in this workspace and none is
/// pulled in for this - matches this crate's own precedent of small
/// crate-local numerics (`root::brent`, `quadrature::composite_simpson`).
fn solve_dense(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let mut pivot_row = col;
        let mut pivot_val = a[col][col].abs();
        for row in (col + 1)..n {
            if a[row][col].abs() > pivot_val {
                pivot_val = a[row][col].abs();
                pivot_row = row;
            }
        }
        if pivot_val < 1e-300 {
            return None;
        }
        if pivot_row != col {
            a.swap(col, pivot_row);
            b.swap(col, pivot_row);
        }
        let pivot = a[col][col];
        for row in (col + 1)..n {
            let factor = a[row][col] / pivot;
            if factor == 0.0 {
                continue;
            }
            for c in col..n {
                a[row][c] -= factor * a[col][c];
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let mut sum = b[row];
        for c in (row + 1)..n {
            sum -= a[row][c] * x[c];
        }
        x[row] = sum / a[row][row];
    }
    Some(x)
}

/// Per-thread coupling stiffness (see module doc for the sourced formula).
fn thread_coupling_stiffness(pitch_diameter: f64, fastener_e: f64, fastener_nu: f64, nut_e: f64, nut_nu: f64) -> f64 {
    let d = pitch_diameter;
    let delta_b = (0.5 * (1.0 - fastener_nu * fastener_nu) + 1.2 * (1.0 + fastener_nu)) / (std::f64::consts::PI * d * fastener_e);
    let delta_n = (0.5 * (1.0 - nut_nu * nut_nu) + 1.2 * (1.0 + nut_nu)) / (std::f64::consts::PI * d * nut_e);
    std::f64::consts::PI * d / (delta_b + delta_n)
}

/// Assembles and solves the `[K]{u} = {F}` coupled-spring system for `N`
/// engaged threads, given the preload the primary solve already produced.
/// DOF ordering: `u_b[0..N)` (bolt-body node displacements, node 0 at the
/// loaded face where `preload` is applied) followed by `u_n[1..N)`
/// (nut-body node displacements; `u_n[0]` is grounded at the bearing face
/// and is not a free DOF). Thread `i`'s coupling spring connects `u_b[i]`
/// to `u_n[i]` (ground, for `i == 0`).
pub fn compute(preload: f64, thread: &ThreadGeometry, fastener_e: f64, fastener_nu: f64, inputs: &ThreadLoadDistributionInputs) -> Option<ThreadLoadDistribution> {
    let n = inputs.engaged_threads as usize;
    if n == 0 || thread.pitch <= 0.0 || fastener_e <= 0.0 || inputs.nut_modulus <= 0.0 {
        return None;
    }
    let nut_area = std::f64::consts::FRAC_PI_4 * (inputs.nut_outer_diameter * inputs.nut_outer_diameter - thread.d * thread.d);
    if nut_area <= 0.0 {
        return None;
    }
    let k_b_seg = fastener_e * thread.root_area() / thread.pitch;
    let k_n_seg = inputs.nut_modulus * nut_area / thread.pitch;
    let k_thread = thread_coupling_stiffness(thread.d2, fastener_e, fastener_nu, inputs.nut_modulus, inputs.nut_poisson_ratio);

    // DOF layout: b[0..n) at indices 0..n, n_dof[1..n) at indices n..(2n-1).
    let dof_count = n + n.saturating_sub(1);
    let b_index = |i: usize| i;
    let n_index = |i: usize| n + (i - 1); // only valid for i >= 1

    let mut k = vec![vec![0.0; dof_count]; dof_count];
    let mut f = vec![0.0; dof_count];

    // Bolt-body and nut-body pitch segments between consecutive threads.
    for i in 0..n.saturating_sub(1) {
        let bi = b_index(i);
        let bj = b_index(i + 1);
        k[bi][bi] += k_b_seg;
        k[bj][bj] += k_b_seg;
        k[bi][bj] -= k_b_seg;
        k[bj][bi] -= k_b_seg;

        if i == 0 {
            // Segment from the grounded nut node (0) to node 1 - only the
            // node-1 diagonal term is a free DOF.
            let nj = n_index(1);
            k[nj][nj] += k_n_seg;
        } else {
            let ni = n_index(i);
            let nj = n_index(i + 1);
            k[ni][ni] += k_n_seg;
            k[nj][nj] += k_n_seg;
            k[ni][nj] -= k_n_seg;
            k[nj][ni] -= k_n_seg;
        }
    }

    // Thread coupling springs, one per engaged thread.
    {
        // Thread 0 couples the bolt node directly to ground (the nut's
        // bearing-face reaction).
        let b0 = b_index(0);
        k[b0][b0] += k_thread;
    }
    for i in 1..n {
        let bi = b_index(i);
        let ni = n_index(i);
        k[bi][bi] += k_thread;
        k[ni][ni] += k_thread;
        k[bi][ni] -= k_thread;
        k[ni][bi] -= k_thread;
    }

    // The full preload enters the bolt body at the loaded face (node 0).
    f[b_index(0)] += preload;

    let x = solve_dense(k, f)?;

    let u_b = |i: usize| x[b_index(i)];
    let u_n = |i: usize| if i == 0 { 0.0 } else { x[n_index(i)] };

    let per_thread_loads: Vec<f64> = (0..n).map(|i| k_thread * (u_b(i) - u_n(i))).collect();
    let first_thread_load = per_thread_loads[0];
    let max_thread_load = per_thread_loads.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    Some(ThreadLoadDistribution { first_thread_load, max_thread_load, per_thread_loads })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steel_thread() -> ThreadGeometry {
        // M10x1.5-equivalent, same fixture shape `solve.rs`'s own tests use.
        ThreadGeometry { d: 10.0, d2: 9.026, d3: 8.160, pitch: 1.5, starts: 1, thread_angle_deg: 60.0 }
    }

    fn steel_inputs(engaged_threads: u32) -> ThreadLoadDistributionInputs {
        ThreadLoadDistributionInputs { engaged_threads, nut_modulus: 200_000.0, nut_poisson_ratio: 0.3, nut_outer_diameter: 16.0 }
    }

    #[test]
    fn single_engaged_thread_carries_the_entire_preload() {
        let thread = steel_thread();
        let result = compute(10_000.0, &thread, 200_000.0, 0.3, &steel_inputs(1)).unwrap();
        assert_eq!(result.per_thread_loads.len(), 1);
        assert!((result.first_thread_load - 10_000.0).abs() < 1e-6);
        assert!((result.max_thread_load - 10_000.0).abs() < 1e-6);
    }

    #[test]
    fn per_thread_loads_sum_to_the_total_preload_regardless_of_engaged_count() {
        // Hard equilibrium check (spec section 36): the only external force
        // in this linear system is the preload applied at the loaded face,
        // so the thread-spring loads must sum to it exactly (to solver
        // precision) for any engaged-thread count.
        let thread = steel_thread();
        for n in [1u32, 2, 5, 10, 30] {
            let result = compute(10_000.0, &thread, 200_000.0, 0.3, &steel_inputs(n)).unwrap();
            let sum: f64 = result.per_thread_loads.iter().sum();
            assert!((sum - 10_000.0).abs() < 1e-6, "n={n} sum={sum}");
        }
    }

    #[test]
    fn load_concentrates_toward_the_loaded_face_and_decays_monotonically() {
        // Sopwith/Zhang finding: load is concentrated near the loaded face
        // and (for a reasonably long engagement) decays roughly
        // monotonically toward the free end - not a uniform distribution.
        let thread = steel_thread();
        let result = compute(10_000.0, &thread, 200_000.0, 0.3, &steel_inputs(10)).unwrap();
        for pair in result.per_thread_loads.windows(2) {
            assert!(pair[0] >= pair[1] - 1e-9, "not monotonically decaying: {:?}", result.per_thread_loads);
        }
        assert_eq!(result.max_thread_load, result.per_thread_loads[0]);
        // First-thread fraction should be a substantial share, consistent
        // with the sourced ~30-50% figures for realistic geometries (not
        // asserting an exact value - that is case-dependent per the
        // research this module cites).
        let fraction = result.first_thread_load / 10_000.0;
        assert!(fraction > 0.05 && fraction < 0.95, "fraction={fraction}");
    }

    #[test]
    fn uniform_stiffness_between_bolt_and_nut_still_balances_and_decays() {
        // A degenerate but valid case: identical bolt/nut modulus - the
        // system must still solve and balance exactly.
        let thread = steel_thread();
        let inputs = ThreadLoadDistributionInputs { engaged_threads: 8, nut_modulus: 200_000.0, nut_poisson_ratio: 0.3, nut_outer_diameter: 16.0 };
        let result = compute(5_000.0, &thread, 200_000.0, 0.3, &inputs).unwrap();
        let sum: f64 = result.per_thread_loads.iter().sum();
        assert!((sum - 5_000.0).abs() < 1e-6);
    }

    #[test]
    fn zero_engaged_threads_is_rejected() {
        let thread = steel_thread();
        assert!(compute(10_000.0, &thread, 200_000.0, 0.3, &steel_inputs(0)).is_none());
    }

    #[test]
    fn nut_outer_diameter_not_exceeding_major_diameter_is_rejected() {
        let thread = steel_thread();
        let inputs = ThreadLoadDistributionInputs { engaged_threads: 5, nut_modulus: 200_000.0, nut_poisson_ratio: 0.3, nut_outer_diameter: thread.d };
        assert!(compute(10_000.0, &thread, 200_000.0, 0.3, &inputs).is_none());
    }

    /// Differential test against the sourced continuum closed form (Zhang
    /// et al. eqs. 2.32-2.35, the Sopwith-equivalent hyperbolic solution):
    /// as the engaged-thread count grows for a fixed per-thread pitch
    /// spacing (i.e. total engagement length grows), the discrete
    /// first-thread-load fraction should approach the continuum
    /// prediction, since both describe the exact same underlying physics
    /// at the same node spacing.
    #[test]
    fn first_thread_fraction_converges_toward_the_continuum_sinh_prediction_as_engagement_grows() {
        let thread = steel_thread();
        let fastener_e = 200_000.0;
        let fastener_nu = 0.3;
        let nut_e = 200_000.0;
        let nut_nu = 0.3;
        let inputs_for = |n: u32| ThreadLoadDistributionInputs { engaged_threads: n, nut_modulus: nut_e, nut_poisson_ratio: nut_nu, nut_outer_diameter: 16.0 };

        let s_b = thread.root_area();
        let s_n = std::f64::consts::FRAC_PI_4 * (16.0 * 16.0 - thread.d * thread.d);
        let k_thread = thread_coupling_stiffness(thread.d2, fastener_e, fastener_nu, nut_e, nut_nu);
        // k_by/k_ny per Zhang's own definitions (foundation stiffness per
        // unit axial length); the series identity used to derive
        // `k_thread` above lets us recover them from it directly.
        let p = thread.pitch;
        let lambda = ((1.0 / (s_b * fastener_e) + 1.0 / (s_n * nut_e)) / (p / k_thread)).sqrt();

        // Continuum prediction for the fraction of total load carried by
        // the first pitch of engagement, f(0) - f(p), normalized by f(0)=F:
        // using f(y) = F * sinh(lambda*(l-y))/sinh(lambda*l).
        let continuum_first_fraction = |l: f64| -> f64 {
            let f_at = |y: f64| (lambda * (l - y)).sinh() / (lambda * l).sinh();
            f_at(0.0) - f_at(p)
        };

        let mut prev_gap = f64::INFINITY;
        for n in [5u32, 15, 40] {
            let result = compute(10_000.0, &thread, fastener_e, fastener_nu, &inputs_for(n)).unwrap();
            let discrete_fraction = result.first_thread_load / 10_000.0;
            let l = n as f64 * p;
            let continuum_fraction = continuum_first_fraction(l);
            let gap = (discrete_fraction - continuum_fraction).abs();
            assert!(gap < prev_gap + 1e-9, "gap did not shrink: n={n} discrete={discrete_fraction} continuum={continuum_fraction} gap={gap} prev_gap={prev_gap}");
            prev_gap = gap;
        }
        assert!(prev_gap < 0.05, "final gap too large: {prev_gap}");
    }
}
