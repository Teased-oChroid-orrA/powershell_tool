//! The largest offset (and load) that still holds the bushing.
//!
//! The capacity of the fit alone does not depend on the pin load, so the search starts on that cheap stage (one contact
//! solve per candidate). With `Inputs::credit_pin_load` the margin is judged on the loaded run, whose capacity is
//! usually higher (the pin's pressure squeezes the bushing): the answer is then looked for above the fit-only one, with
//! full analyses, and below it when contact is lost under the load.

use crate::model::{analyze, fit_capacity, Inputs, MIN_WALL_FRACTION};

/// Result of [`max_offset`] / [`max_load`].
#[derive(Debug, Clone, Copy)]
pub struct OffsetLimit {
    /// The limiting offset (in) or load (lbf); infinite for a load when the pin load causes no torque.
    pub value: f64,
    /// The offset `Inputs::min_wall` allows (`wall - min_wall`, at least 0): beyond it the thin wall is below the
    /// requirement whether or not the bushing spins. Meaningful for `max_offset`.
    pub wall_limit: f64,
    /// The search ran to the numerical wall floor without losing the hold.
    pub bounded_by_wall: bool,
    /// The loaded capacity (not the fit alone) set the limit.
    pub set_by_loaded_run: bool,
    /// Number of finite-element solves.
    pub evaluations: usize,
}

/// A margin this close to zero from below is the search's own resolution, not a failure.
const MARGIN_TOL: f64 = 2e-3;

fn torque_per_load(inp: &Inputs, offset: f64) -> f64 {
    offset * inp.load_angle_deg.to_radians().sin().abs()
}

/// Evaluate `f` at every point on its own thread (each is a full contact solve of seconds).
fn parallel_map<T: Send>(points: &[f64], f: impl Fn(f64) -> T + Sync) -> Vec<T> {
    std::thread::scope(|scope| {
        let f = &f;
        let handles: Vec<_> = points.iter().map(|&x| scope.spawn(move || f(x))).collect();
        handles.into_iter().map(|h| h.join().expect("a contact solve panicked")).collect()
    })
}

/// How many candidates one round of the search solves at once (the cores a solve leaves idle: below ~40k dofs the
/// factorisation is sequential).
fn section_count() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8)
}

/// Largest `x` in `[lo, hi]` with `margin(x) >= 0`, given `margin(lo) >= 0` and a margin that falls with `x`: each round
/// solves `k` equally spaced candidates in parallel and keeps the sub-interval where the margin changes sign (8 sections
/// reach 2 % of the range in two rounds). Returns the value, whether it reached `hi`, and the number of solves.
fn search_up(lo: f64, hi: f64, tol: f64, margin: impl Fn(f64) -> Result<f64, String> + Sync) -> Result<(f64, bool, usize), String> {
    let k = section_count();
    let (mut lo_now, mut hi_now, mut evals) = (lo, hi, 0);
    let range = hi - lo;
    loop {
        let points: Vec<f64> = (1..=k).map(|j| lo_now + (hi_now - lo_now) * j as f64 / k as f64).collect();
        let margins = parallel_map(&points, &margin).into_iter().collect::<Result<Vec<f64>, String>>()?;
        evals += k;
        let Some(j) = margins.iter().position(|&m| m < -MARGIN_TOL) else {
            return Ok((hi_now, hi_now >= hi - 1e-12, evals));
        };
        (lo_now, hi_now) = (if j == 0 { lo_now } else { points[j - 1] }, points[j]);
        if hi_now - lo_now <= tol * range {
            return Ok((lo_now, false, evals));
        }
    }
}

/// Largest offset at which the design capacity carries `F e |sin(phi)|`, to `tol` of the search range.
pub fn max_offset(inp: &Inputs, tol: f64) -> Result<OffsetLimit, String> {
    inp.validate()?;
    let wall = inp.bore_radius() - inp.bushing_id / 2.0;
    let bound = wall - MIN_WALL_FRACTION * inp.bore_dia;
    let wall_limit = (wall - inp.min_wall).max(0.0);
    let fit_margin = |e: f64| -> Result<f64, String> {
        let cap = fit_capacity(&Inputs { offset: e, ..*inp })?;
        let req = inp.load_lbf * torque_per_load(inp, e);
        Ok(if req > 0.0 { cap / req - 1.0 } else { f64::INFINITY })
    };
    let loaded_margin = |e: f64| analyze(&Inputs { offset: e, ..*inp }).map(|a| a.margin);
    // The fit-alone limit first: it is the answer on the conservative basis and the starting point on the other.
    let (e_fit, at_bound, mut evaluations) = search_up(0.0, bound, tol, fit_margin)?;
    if !inp.credit_pin_load || inp.load_lbf <= 0.0 {
        return Ok(OffsetLimit { value: e_fit, wall_limit, bounded_by_wall: at_bound, set_by_loaded_run: false, evaluations });
    }
    evaluations += 1;
    let start = loaded_margin(e_fit)?;
    let (value, bounded, n) = if start >= -MARGIN_TOL {
        // The usual case: the pin's squeeze raises the capacity, so the hold lasts beyond the fit-alone limit.
        search_up(e_fit, bound, tol, loaded_margin)?
    } else {
        // Contact lost under the load: the loaded capacity is below the fit's, look below.
        search_up(0.0, e_fit, tol, loaded_margin)?
    };
    Ok(OffsetLimit { value, wall_limit, bounded_by_wall: bounded && start >= -MARGIN_TOL, set_by_loaded_run: true, evaluations: evaluations + n })
}

/// Largest pin load the interface holds at the entered offset and load angle (`F_spin`).
pub fn max_load(inp: &Inputs, tol: f64) -> Result<OffsetLimit, String> {
    inp.validate()?;
    let arm = torque_per_load(inp, inp.offset);
    if arm <= 0.0 {
        return Ok(OffsetLimit { value: f64::INFINITY, wall_limit: 0.0, bounded_by_wall: false, set_by_loaded_run: false, evaluations: 0 });
    }
    // On the fit-alone basis the capacity does not depend on the load: a closed form from one solve.
    let f_fit = fit_capacity(inp)? / arm;
    if !inp.credit_pin_load {
        return Ok(OffsetLimit { value: f_fit, wall_limit: 0.0, bounded_by_wall: false, set_by_loaded_run: false, evaluations: 1 });
    }
    let loaded_margin = |f: f64| analyze(&Inputs { load_lbf: f, ..*inp }).map(|a| a.margin);
    let start = loaded_margin(f_fit)?;
    let (value, _, n) = if start >= -MARGIN_TOL {
        // The squeeze grows with the load, so the loaded capacity carries more than the fit's: look up to 3x.
        search_up(f_fit, 3.0 * f_fit, tol, loaded_margin)?
    } else {
        search_up(0.0, f_fit, tol, loaded_margin)?
    };
    Ok(OffsetLimit { value, wall_limit: 0.0, bounded_by_wall: false, set_by_loaded_run: true, evaluations: 2 + n })
}
