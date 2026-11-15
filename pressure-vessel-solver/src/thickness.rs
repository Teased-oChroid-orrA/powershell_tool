//! Minimum wall thickness solver - issue #11 Phase 8: "Determine the
//! minimum wall thickness for which every applicable failure mode
//! satisfies the user-defined required minimum Margin of Safety."
//!
//! **Never assumes which mode controls.** Every candidate thickness
//! re-runs [`crate::failure::evaluate_failure_modes`] in full and takes
//! the [`crate::failure::governing`] (minimum) margin across all four -
//! issue #11's own explicit warning: "The solver shall not: Assume
//! failure mode X controls -> Solve only failure mode X... The
//! controlling mode may change as thickness changes."
//!
//! **Search method: bisection on outer radius**, relying on a real,
//! separately-tested property of this problem (not assumed): increasing
//! wall thickness at fixed inner radius and fixed pressure never makes
//! any of the four v1 margins worse (see
//! `governing_margin_is_monotonically_non_decreasing_with_wall_thickness`
//! in this module's tests) - the physically expected behavior for a
//! plain thick cylinder (more material reacting the same load), verified
//! rather than taken on faith before relying on it for a bisection search
//! to be sound.

use crate::failure::{evaluate_failure_modes, governing, MarginResult};
use crate::geometry::CylinderGeometry;
use crate::pressure::PressureLoading;
use mechanics_core::materials::Material;

pub struct ThicknessSolverInputs {
    pub inner_radius: f64,
    pub pressure: PressureLoading,
    pub material: Material,
    pub required_minimum_ms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThicknessSolution {
    pub outer_radius: f64,
    pub wall_thickness: f64,
    pub governing: MarginResult,
    pub iterations: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThicknessSolverOutcome {
    Converged(ThicknessSolution),
    /// No outer radius tried up to the search's expansion limit satisfied
    /// the required minimum MS - issue #11: "The tool shall explain the
    /// outcome rather than returning an unexplained number."
    Infeasible { largest_radius_tried: f64, best_margin_found: f64 },
}

/// `outer_radius / inner_radius` above which the search gives up
/// expanding - a genuinely enormous wall (1000x the bore radius) that no
/// real pressure-vessel design would ever need; a required MS that can't
/// be met within this bound is reported `Infeasible`, not searched
/// forever.
pub(crate) const MAX_RADIUS_RATIO: f64 = 1000.0;

fn governing_at(inner_radius: f64, outer_radius: f64, pressure: &PressureLoading, material: &Material) -> Option<MarginResult> {
    let geometry = CylinderGeometry::new(inner_radius, outer_radius).ok()?;
    let results = evaluate_failure_modes(&geometry, pressure, material);
    Some(governing(&results).clone())
}

/// Outcome of [`bisect_outer_radius`].
pub(crate) enum RadiusSearch {
    /// `upper` satisfies the requirement; `upper - tolerance` (or the zero-wall bound) does not.
    Feasible { upper: f64, iterations: u32 },
    Infeasible { largest_radius_tried: f64, best_margin_found: f64 },
}

/// The bisection of [`solve_minimum_thickness`], generic over the margin being driven (`margin_at`
/// returns `None` for an invalid geometry): expands an upper bound by doubling the outer radius
/// until `required_ms` is met (or [`MAX_RADIUS_RATIO`]), then bisects. Sound only for a margin that
/// does not decrease with wall thickness; each caller's tests check that for its own margin.
pub(crate) fn bisect_outer_radius(inner_radius: f64, required_ms: f64, max_iterations: u32, radius_tolerance: f64, margin_at: impl Fn(f64) -> Option<f64>) -> RadiusSearch {
    let a = inner_radius;
    let mut lower = a * 1.000_001; // smallest valid outer radius, effectively zero wall
    let mut upper = a * 1.01; // start the expansion search at a thin, real wall
    let mut best_margin_found = margin_at(lower).unwrap_or(f64::NEG_INFINITY);

    let mut expansions = 0;
    loop {
        match margin_at(upper) {
            Some(m) => {
                best_margin_found = best_margin_found.max(m);
                if m >= required_ms {
                    break;
                }
            }
            None => break,
        }
        if upper >= a * MAX_RADIUS_RATIO {
            return RadiusSearch::Infeasible { largest_radius_tried: upper, best_margin_found };
        }
        upper *= 2.0;
        expansions += 1;
        if expansions > 200 {
            return RadiusSearch::Infeasible { largest_radius_tried: upper, best_margin_found };
        }
    }

    let mut iterations = 0;
    while (upper - lower) > radius_tolerance && iterations < max_iterations {
        let mid = (lower + upper) / 2.0;
        match margin_at(mid) {
            Some(m) if m >= required_ms => upper = mid,
            _ => lower = mid,
        }
        iterations += 1;
    }
    RadiusSearch::Feasible { upper, iterations }
}

/// Solves for the minimum outer radius (equivalently wall thickness)
/// satisfying `inputs.required_minimum_ms` across all four v1 failure
/// modes, via [`bisect_outer_radius`] on the governing margin. For the
/// minimum at each criterion separately see
/// [`crate::criterion_thickness`].
pub fn solve_minimum_thickness(inputs: &ThicknessSolverInputs, max_iterations: u32, radius_tolerance: f64) -> ThicknessSolverOutcome {
    let a = inputs.inner_radius;
    let search = bisect_outer_radius(a, inputs.required_minimum_ms, max_iterations, radius_tolerance, |ro| {
        governing_at(a, ro, &inputs.pressure, &inputs.material).map(|g| g.margin)
    });
    match search {
        RadiusSearch::Infeasible { largest_radius_tried, best_margin_found } => ThicknessSolverOutcome::Infeasible { largest_radius_tried, best_margin_found },
        RadiusSearch::Feasible { upper, iterations } => {
            let governing_result = governing_at(a, upper, &inputs.pressure, &inputs.material)
                .expect("upper bound was already proven valid/feasible in the expansion phase above");
            ThicknessSolverOutcome::Converged(ThicknessSolution { outer_radius: upper, wall_thickness: upper - a, governing: governing_result, iterations })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pressure::EndCondition;

    fn al7075() -> Material {
        *mechanics_core::materials::get_material("al7075")
    }

    /// The property the bisection search in this module depends on for
    /// correctness - verified directly, not assumed. Three increasing
    /// wall thicknesses at the same inner radius/pressure must produce
    /// non-decreasing governing margins.
    #[test]
    fn governing_margin_is_monotonically_non_decreasing_with_wall_thickness() {
        let pressure = PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap();
        let material = al7075();
        let m1 = governing_at(2.0, 2.2, &pressure, &material).unwrap().margin;
        let m2 = governing_at(2.0, 3.0, &pressure, &material).unwrap().margin;
        let m3 = governing_at(2.0, 5.0, &pressure, &material).unwrap().margin;
        assert!(m1 <= m2, "expected margin to improve or hold as wall thickens: {m1} -> {m2}");
        assert!(m2 <= m3, "expected margin to improve or hold as wall thickens: {m2} -> {m3}");
    }

    #[test]
    fn solved_thickness_actually_satisfies_the_required_minimum_ms() {
        let inputs = ThicknessSolverInputs {
            inner_radius: 2.0,
            pressure: PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap(),
            material: al7075(),
            required_minimum_ms: 0.5,
        };
        let outcome = solve_minimum_thickness(&inputs, 100, 1e-6);
        match outcome {
            ThicknessSolverOutcome::Converged(sol) => {
                assert!(sol.governing.margin >= 0.5, "solved thickness's own governing margin {} must satisfy the 0.5 requirement", sol.governing.margin);
                assert!(sol.wall_thickness > 0.0);
            }
            other => panic!("expected a converged solution, got {other:?}"),
        }
    }

    #[test]
    fn a_thinner_wall_than_the_solved_solution_fails_the_requirement() {
        // Proves the solver found the actual MINIMUM, not just *a*
        // feasible thickness - a wall meaningfully thinner than the
        // solved solution must fail the same requirement.
        let inputs = ThicknessSolverInputs {
            inner_radius: 2.0,
            pressure: PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap(),
            material: al7075(),
            required_minimum_ms: 0.5,
        };
        let outcome = solve_minimum_thickness(&inputs, 100, 1e-6);
        let ThicknessSolverOutcome::Converged(sol) = outcome else { panic!("expected convergence") };
        let thinner_outer_radius = inputs.inner_radius + sol.wall_thickness * 0.9;
        let g = governing_at(inputs.inner_radius, thinner_outer_radius, &inputs.pressure, &inputs.material).unwrap();
        assert!(g.margin < inputs.required_minimum_ms, "a 10% thinner wall should fail the requirement (got margin {})", g.margin);
    }

    #[test]
    fn required_ms_of_zero_matches_the_epics_own_worked_example_shape() {
        // Not the epic's exact numbers (different geometry/material), but
        // the same shape: MS Required = 0.00 is a real, meaningful,
        // commonly-used requirement (not just positive margins), and the
        // solver must handle it without special-casing.
        let inputs = ThicknessSolverInputs {
            inner_radius: 2.0,
            pressure: PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap(),
            material: al7075(),
            required_minimum_ms: 0.0,
        };
        let outcome = solve_minimum_thickness(&inputs, 100, 1e-6);
        let ThicknessSolverOutcome::Converged(sol) = outcome else { panic!("expected convergence") };
        assert!(sol.governing.margin >= 0.0);
        assert!(sol.governing.margin < 0.01, "expected a tight solve near the zero-margin boundary, got {}", sol.governing.margin);
    }

    #[test]
    fn an_unreasonably_high_required_ms_is_reported_infeasible_not_an_infinite_loop() {
        let inputs = ThicknessSolverInputs {
            inner_radius: 2.0,
            pressure: PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap(),
            material: al7075(),
            required_minimum_ms: 1_000_000.0,
        };
        let outcome = solve_minimum_thickness(&inputs, 100, 1e-6);
        assert!(matches!(outcome, ThicknessSolverOutcome::Infeasible { .. }), "expected Infeasible, got {outcome:?}");
    }
}
