//! The largest offset (and load) that still holds the bushing.
//!
//! The capacity of the fit alone does not depend on the pin load, so the search starts on that cheap stage (one contact
//! solve per candidate). With `Inputs::credit_pin_load` the margin is judged on the loaded run, whose capacity is
//! usually higher (the pin's pressure squeezes the bushing): the answer is then looked for above the fit-only one, with
//! full analyses, and below it when contact is lost under the load.

use crate::model::{analyze_from_fit, analyze_with, fit_capacity_with, solve_fit, Inputs, INTERRUPTED, MIN_WALL_FRACTION};
use crate::control::Control;

/// Result of [`max_offset`] / [`max_load`].
#[derive(Debug, Clone)]
pub struct OffsetLimit {
    /// The limiting offset (in) or load (lbf); infinite for a load when the pin load causes no torque. When the search
    /// stopped early (`halted`) it is the largest value verified to hold: the true limit lies in `[value, upper]`.
    pub value: f64,
    /// The smallest value known not to hold, or not resolved (the search stopped before it), above `value`; `None`
    /// when the search ran to the top of its range or the answer is exact to the tolerance.
    pub upper: Option<f64>,
    /// Why the search stopped before reaching its tolerance (cancelled, time budget, a solve that failed), with the
    /// bracket the answer is known to lie in. `None` for a finished search.
    pub halted: Option<String>,
    /// A candidate that could not be solved bounds the answer from above and was counted as not holding: the value is
    /// conservative, the true limit may be higher. `None` when every candidate near the answer solved.
    pub caveat: Option<String>,
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

/// What a [`search_up`] found.
struct Found {
    /// Largest `x` verified to hold.
    lo: f64,
    /// Smallest `x` known not to hold or not resolved (`None`: the top of the range holds).
    hi: Option<f64>,
    reached_top: bool,
    evals: usize,
    halted: Option<String>,
    /// A candidate that could not be solved bounds the bracket from above: it was treated as not holding.
    caveat: Option<String>,
}

/// Largest `x` in `[lo, hi]` with `margin(x) >= 0`, given `margin(lo) >= 0` (`margin_lo`, when it is known) and a margin
/// that falls with `x`. The first round solves `k` equally spaced candidates in parallel and keeps the sub-interval where
/// the margin changes sign. When both ends of that bracket have a margin, the next round puts three candidates around the
/// root the straight line between them predicts (the margin is smooth), so a 2 % answer costs about `k + 3` solves in two
/// rounds instead of `2k`; a root outside those three just moves the bracket and the next round interpolates again. Without
/// a known margin the round subdivides evenly. A solve that fails is not a margin: that candidate is treated as not
/// holding (the answer is then conservative) and reported as a `caveat` when it is what bounds the answer from above. When
/// `ctl`'s interrupt fires (every solve in flight stops with it) the search stops and reports the bracket it had (`halted`).
fn search_up(lo: f64, hi: f64, tol: f64, ctl: &Control, margin_lo: Option<f64>, margin: impl Fn(f64) -> Result<f64, String> + Sync) -> Found {
    let k = section_count();
    let (mut lo_now, mut hi_now, mut evals) = (lo, hi, 0);
    let (mut m_lo, mut m_hi) = (margin_lo.filter(|m| m.is_finite()), None::<f64>);
    let range = hi - lo;
    let mut bracketed = false;
    let mut failed_at: Option<(f64, String)> = None;
    let caveat_for = |failed: &Option<(f64, String)>, top: Option<f64>| failed.as_ref().filter(|(x, _)| top.is_some_and(|t| (t - x).abs() <= 1e-12 * range.abs().max(1e-300))).map(|(x, e)| format!("the solve at {x:.5} failed ({e}); it is counted as not holding, so the limit may be higher"));
    loop {
        // After the first round `hi_now` is a point that failed: it is not solved again.
        let interpolated = match (bracketed, m_lo, m_hi) {
            (true, Some(a), Some(b)) => {
                let (ga, gb) = (a + MARGIN_TOL, b + MARGIN_TOL);
                let root = lo_now + (hi_now - lo_now) * (ga / (ga - gb)).clamp(0.0, 1.0);
                let d = 0.4 * tol * range;
                let pts: Vec<f64> = [root - d, root, root + d].into_iter().filter(|&x| x > lo_now + 1e-12 * range && x < hi_now - 1e-12 * range).collect();
                (pts.len() >= 2 && hi_now - lo_now > 3.0 * d).then_some(pts)
            }
            _ => None,
        };
        let points = interpolated.unwrap_or_else(|| {
            let parts = if bracketed { k + 1 } else { k };
            (1..=k).map(|j| lo_now + (hi_now - lo_now) * j as f64 / parts as f64).collect()
        });
        let margins = parallel_map(&points, |x| {
            let m = margin(x);
            ctl.solve_done();
            m
        });
        evals += points.len();
        // The first candidate that fails to hold or to solve ends the round.
        let bad = margins.iter().position(|m| m.as_ref().map_or(true, |&m| m < -MARGIN_TOL));
        let Some(j) = bad else {
            if !bracketed {
                return Found { lo: hi_now, hi: None, reached_top: hi_now >= hi - 1e-12, evals, halted: None, caveat: None };
            }
            // Every candidate holds: the bracket moves up to the last one.
            (lo_now, m_lo) = (points[points.len() - 1], margins[points.len() - 1].as_ref().ok().copied());
            bracketed = true;
            if hi_now - lo_now <= tol * range {
                return Found { lo: lo_now, hi: Some(hi_now), reached_top: false, evals, halted: None, caveat: caveat_for(&failed_at, Some(hi_now)) };
            }
            continue;
        };
        // An interrupt ends the search at the last point verified before it.
        if let Some(i) = margins.iter().position(|m| m.as_ref().is_err_and(|e| e == INTERRUPTED)) {
            let ok_before = margins[..i].iter().rposition(|m| m.as_ref().is_ok_and(|&m| m >= -MARGIN_TOL));
            let below = ok_before.map_or(lo_now, |k| points[k]);
            return Found { lo: below, hi: Some(points[i]), reached_top: false, evals, halted: Some(ctl.interrupt.reason().unwrap_or("stopped").to_string()), caveat: None };
        }
        let below = if j == 0 { lo_now } else { points[j - 1] };
        if let Err(e) = &margins[j] {
            failed_at = Some((points[j], e.clone()));
        }
        if j > 0 {
            m_lo = margins[j - 1].as_ref().ok().copied().filter(|m| m.is_finite());
        }
        m_hi = margins[j].as_ref().ok().copied().filter(|m| m.is_finite());
        (lo_now, hi_now) = (below, points[j]);
        bracketed = true;
        ctl.bracket(lo_now, hi_now);
        if hi_now - lo_now <= tol * range {
            return Found { lo: lo_now, hi: Some(hi_now), reached_top: false, evals, halted: None, caveat: caveat_for(&failed_at, Some(hi_now)) };
        }
    }
}

fn limit(f: &Found, wall_limit: f64, bounded_by_wall: bool, set_by_loaded_run: bool, evaluations: usize) -> OffsetLimit {
    OffsetLimit { value: f.lo, upper: f.hi, halted: f.halted.clone(), caveat: f.caveat.clone(), wall_limit, bounded_by_wall, set_by_loaded_run, evaluations }
}

/// Largest offset at which the design capacity carries `F e |sin(phi)|`, to `tol` of the search range.
pub fn max_offset(inp: &Inputs, tol: f64) -> Result<OffsetLimit, String> {
    max_offset_with(inp, tol, &Control::default())
}

/// [`max_offset`] that stops when `ctl`'s interrupt fires (a deadline or a cancel request), returning the bracket it had.
pub fn max_offset_with(inp: &Inputs, tol: f64, ctl: &Control) -> Result<OffsetLimit, String> {
    inp.validate()?;
    let wall = inp.bore_radius() - inp.bushing_id / 2.0;
    let bound = wall - MIN_WALL_FRACTION * inp.bore_dia;
    let wall_limit = (wall - inp.min_wall).max(0.0);
    let fit_margin = |e: f64| -> Result<f64, String> {
        let cap = fit_capacity_with(&Inputs { offset: e, ..*inp }, ctl)?;
        let req = inp.load_lbf * torque_per_load(inp, e);
        Ok(if req > 0.0 { cap / req - 1.0 } else { f64::INFINITY })
    };
    let loaded_margin = |e: f64| analyze_with(&Inputs { offset: e, ..*inp }, ctl).map(|a| a.margin);
    // The fit-alone limit first: it is the answer on the conservative basis and the starting point on the other.
    let fit = search_up(0.0, bound, tol, ctl, None, fit_margin);
    let mut evaluations = fit.evals;
    if fit.halted.is_some() || !inp.credit_pin_load || inp.load_lbf <= 0.0 {
        return Ok(limit(&fit, wall_limit, fit.reached_top, false, evaluations));
    }
    evaluations += 1;
    let e_fit = fit.lo;
    let start = match loaded_margin(e_fit) {
        Ok(m) => m,
        Err(e) => {
            // The loaded run could not be judged at the fit-alone limit: that limit stands, flagged as not refined.
            let why = if e == INTERRUPTED { ctl.interrupt.reason().unwrap_or("stopped").to_string() } else { format!("the loaded solve at {e_fit:.5} failed ({e})") };
            return Ok(OffsetLimit { halted: Some(why), ..limit(&fit, wall_limit, false, false, evaluations) });
        }
    };
    let found = if start >= -MARGIN_TOL {
        // The usual case: the pin's squeeze raises the capacity, so the hold lasts beyond the fit-alone limit.
        search_up(e_fit, bound, tol, ctl, Some(start), loaded_margin)
    } else {
        // Contact lost under the load: the loaded capacity is below the fit's, look below.
        search_up(0.0, e_fit, tol, ctl, None, loaded_margin)
    };
    Ok(limit(&found, wall_limit, found.reached_top && start >= -MARGIN_TOL, true, evaluations + found.evals))
}

/// Largest pin load the interface holds at the entered offset and load angle (`F_spin`).
pub fn max_load(inp: &Inputs, tol: f64) -> Result<OffsetLimit, String> {
    max_load_with(inp, tol, &Control::default())
}

/// [`max_load`] that stops when `ctl`'s interrupt fires (a deadline or a cancel request), returning the bracket it had.
pub fn max_load_with(inp: &Inputs, tol: f64, ctl: &Control) -> Result<OffsetLimit, String> {
    inp.validate()?;
    let arm = torque_per_load(inp, inp.offset);
    if arm <= 0.0 {
        return Ok(OffsetLimit { value: f64::INFINITY, upper: None, halted: None, caveat: None, wall_limit: 0.0, bounded_by_wall: false, set_by_loaded_run: false, evaluations: 0 });
    }
    // On the fit-alone basis the capacity does not depend on the load: a closed form from one solve.
    // The fit (stage 1) does not depend on the pin load: it is solved once and every probe starts from it.
    let fit = solve_fit(inp, ctl).map_err(|e| if e == INTERRUPTED { format!("{}: no result yet", ctl.interrupt.reason().unwrap_or("stopped")) } else { e })?;
    let f_fit = fit.capacity(inp.friction) / arm;
    let fit_only = OffsetLimit { value: f_fit, upper: None, halted: None, caveat: None, wall_limit: 0.0, bounded_by_wall: false, set_by_loaded_run: false, evaluations: 1 };
    if !inp.credit_pin_load {
        return Ok(fit_only);
    }
    let loaded_margin = |f: f64| analyze_from_fit(&Inputs { load_lbf: f, ..*inp }, &fit, ctl).map(|a| a.margin);
    let start = match loaded_margin(f_fit) {
        Ok(m) => m,
        Err(e) => {
            // The conservative fit-alone limit stands, flagged as not refined.
            let why = if e == INTERRUPTED { ctl.interrupt.reason().unwrap_or("stopped").to_string() } else { format!("the loaded solve at {f_fit:.0} lbf failed ({e})") };
            return Ok(OffsetLimit { halted: Some(why), ..fit_only });
        }
    };
    let found = if start >= -MARGIN_TOL {
        // The squeeze grows with the load, so the loaded capacity carries more than the fit's: look up to 3x.
        search_up(f_fit, 3.0 * f_fit, tol, ctl, Some(start), loaded_margin)
    } else {
        search_up(0.0, f_fit, tol, ctl, None, loaded_margin)
    };
    Ok(limit(&found, 0.0, false, true, 2 + found.evals))
}

/// One offset of [`sweep_offset`].
#[derive(Debug, Clone)]
pub struct SweepPoint {
    pub offset: f64,
    /// Design capacity over required torque, minus one (infinite with no spin torque); `None` when the solve failed.
    pub margin: Option<f64>,
    /// Torque capacity and demand, lbf in.
    pub capacity: f64,
    pub required: f64,
    /// Why the solve at this offset failed or stopped.
    pub error: Option<String>,
}

/// The margin at `points` offsets from 0 to the numerical wall floor (the range `max_offset` searches), solved in
/// parallel: the curve a margin-versus-offset plot needs. A point whose solve fails or is stopped carries its reason
/// and the rest are still returned.
pub fn sweep_offset(inp: &Inputs, points: usize, ctl: &Control) -> Result<Vec<SweepPoint>, String> {
    inp.validate()?;
    let wall = inp.bore_radius() - inp.bushing_id / 2.0;
    let bound = wall - MIN_WALL_FRACTION * inp.bore_dia;
    if points < 2 || bound <= 0.0 {
        return Err("the sweep needs at least two offsets and a wall thicker than the minimum".into());
    }
    let offsets: Vec<f64> = (0..points).map(|i| bound * i as f64 / (points - 1) as f64).collect();
    Ok(parallel_map(&offsets, |e| {
        let r = analyze_with(&Inputs { offset: e, ..*inp }, ctl);
        ctl.solve_done();
        match r {
            Ok(a) => SweepPoint { offset: e, margin: Some(a.margin), capacity: a.design_capacity, required: a.torque_required, error: None },
            Err(m) => SweepPoint { offset: e, margin: None, capacity: 0.0, required: 0.0, error: Some(if m == INTERRUPTED { ctl.interrupt.reason().unwrap_or("stopped").to_string() } else { m }) },
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Control {
        Control::default()
    }

    #[test]
    fn interpolation_finds_a_smooth_root_in_about_k_plus_three_solves() {
        let k = section_count();
        for root in [0.0123, 0.37, 0.83, 0.95] {
            // A smooth falling margin (hyperbolic, like capacity / demand - 1).
            let m = move |x: f64| -> Result<f64, String> { Ok(root / x.max(1e-9) - 1.0) };
            let f = search_up(0.0, 1.0, 0.02, &none(), None, m);
            // The search accepts a margin down to -MARGIN_TOL, so its edge is root / (1 - MARGIN_TOL).
            let edge = root / (1.0 - MARGIN_TOL);
            assert!(f.halted.is_none() && f.lo <= edge + 1e-9 && edge < f.hi.unwrap_or(1.0) + 1e-9, "{root}: {:?}", (f.lo, f.hi));
            assert!(f.hi.unwrap_or(1.0) - f.lo <= 0.02 + 1e-12, "{root}: bracket {:?}", (f.lo, f.hi));
            assert!(f.evals <= 2 * k + 3, "{root}: {} solves for k = {k}", f.evals);
        }
        // A margin that never fails reaches the top of the range in one round.
        let f = search_up(0.0, 1.0, 0.02, &none(), None, |_| Ok(5.0));
        assert!(f.reached_top && f.lo == 1.0 && f.evals == k);
    }

    #[test]
    fn a_known_start_margin_saves_a_round_and_a_step_margin_still_converges() {
        let k = section_count();
        let smooth = |x: f64| -> Result<f64, String> { Ok(0.6 / x.max(1e-9) - 1.0) };
        let f = search_up(0.2, 1.0, 0.02, &none(), Some(2.0), smooth);
        assert!(f.lo <= 0.6 * 1.003 && 0.6 <= f.hi.unwrap() && f.hi.unwrap() - f.lo <= 0.02 * 0.8 + 1e-12, "{:?}", (f.lo, f.hi));
        assert!(f.evals <= 2 * k + 3);
        // A discontinuous (stepped) margin: the interpolation is wrong, the search still brackets the step.
        let step = |x: f64| -> Result<f64, String> { Ok(if x <= 0.4 { 3.0 } else { -0.5 }) };
        let f = search_up(0.0, 1.0, 0.02, &none(), None, step);
        assert!(f.lo <= 0.4 && 0.4 <= f.hi.unwrap() && f.hi.unwrap() - f.lo <= 0.02 + 1e-12, "{:?}", (f.lo, f.hi));
    }

    #[test]
    fn a_probe_that_fails_to_solve_counts_as_not_holding_and_is_reported() {
        // Solves above 0.5 fail; the true limit (margin 0 at 0.7) lies above them, so the answer is conservative.
        let m = |x: f64| -> Result<f64, String> { if x > 0.5 { Err("no convergence".into()) } else { Ok(0.7 / x - 1.0) } };
        let f = search_up(0.0, 1.0, 0.02, &none(), None, m);
        assert!(f.halted.is_none() && f.lo <= 0.5 && f.lo > 0.48, "{:?}", (f.lo, f.hi));
        assert!(f.caveat.as_deref().is_some_and(|c| c.contains("failed") && c.contains("higher")), "{:?}", f.caveat);
        // Failures far above the answer do not matter: the limit is below them.
        let m = |x: f64| -> Result<f64, String> { if x > 0.9 { Err("no convergence".into()) } else { Ok(0.3 / x - 1.0) } };
        let f = search_up(0.0, 1.0, 0.02, &none(), None, m);
        assert!(f.caveat.is_none() && (f.lo - 0.3).abs() < 0.02, "{:?}", (f.lo, f.hi, f.caveat));
    }

    #[test]
    fn an_interrupt_ends_the_search_at_the_last_verified_point() {
        let m = |x: f64| -> Result<f64, String> { if x > 0.5 { Err(INTERRUPTED.into()) } else { Ok(2.0) } };
        let f = search_up(0.0, 1.0, 0.02, &none(), None, m);
        assert!(f.halted.is_some() && f.lo <= 0.5 && f.hi.is_some_and(|h| h > 0.5));
    }
}
