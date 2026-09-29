//! Worst-case tolerance/friction-corner uncertainty evaluation (spec
//! section 13): deterministic exhaustive evaluation of every `2^n` corner
//! combination of `n` independent bounded variables (friction, pitch
//! diameter, bearing diameters, applied torque tolerance, ...) - preferred
//! over a statistical method for small `n` because these evaluations are
//! inexpensive (spec: "because these calculations are inexpensive, favor
//! deterministic exhaustive evaluation for small n").

/// An independent bounded input variable's `[min, max]` range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bound {
    pub min: f64,
    pub max: f64,
}

impl Bound {
    pub fn point(value: f64) -> Self {
        Self { min: value, max: value }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorstCaseResult {
    pub min: f64,
    pub max: f64,
}

/// Evaluates `eval` at every one of the `2^n` combinations of each
/// `bounds[i]`'s min/max endpoint, returning the overall min/max of the
/// result. `eval` receives one full corner point (`bounds.len()` values,
/// in the same order as `bounds`). Non-finite evaluations (e.g. a corner
/// that happens to be physically degenerate) are skipped rather than
/// poisoning the min/max with `NaN`/`inf`.
///
/// Capped at `n <= 20` (a million corners) as a sanity guard - every real
/// caller in this crate uses `n` in the 4-6 range (spec section 13's own
/// example list), so this is a defensive bound, not a real limitation.
pub fn worst_case_corners(bounds: &[Bound], eval: impl Fn(&[f64]) -> f64) -> WorstCaseResult {
    let n = bounds.len().min(20);
    let corner_count = 1usize << n;
    let mut point = vec![0.0; bounds.len()];
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for mask in 0..corner_count {
        for (i, b) in bounds.iter().enumerate() {
            point[i] = if (mask >> i) & 1 == 1 { b.max } else { b.min };
        }
        let value = eval(&point);
        if value.is_finite() {
            min = min.min(value);
            max = max.max(value);
        }
    }
    if !min.is_finite() && !max.is_finite() {
        // No finite evaluation succeeded at all - degenerate input; report
        // as a zero-width band at 0 rather than propagate `inf`/`-inf` into
        // a UI margin/percentage computation.
        return WorstCaseResult { min: 0.0, max: 0.0 };
    }
    WorstCaseResult { min, max }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_variable_reduces_to_the_two_endpoint_evaluations() {
        let bounds = [Bound { min: 1.0, max: 3.0 }];
        let result = worst_case_corners(&bounds, |p| p[0] * 2.0);
        assert_eq!(result, WorstCaseResult { min: 2.0, max: 6.0 });
    }

    #[test]
    fn two_variables_evaluate_all_four_corners() {
        let bounds = [Bound { min: 1.0, max: 2.0 }, Bound { min: 10.0, max: 20.0 }];
        let result = worst_case_corners(&bounds, |p| p[0] + p[1]);
        // corners: (1,10)=11, (2,10)=12, (1,20)=21, (2,20)=22
        assert_eq!(result, WorstCaseResult { min: 11.0, max: 22.0 });
    }

    #[test]
    fn a_point_bound_contributes_no_variation() {
        let bounds = [Bound::point(5.0), Bound { min: 1.0, max: 2.0 }];
        let result = worst_case_corners(&bounds, |p| p[0] * p[1]);
        assert_eq!(result, WorstCaseResult { min: 5.0, max: 10.0 });
    }

    #[test]
    fn non_finite_corners_are_skipped_not_propagated() {
        let bounds = [Bound { min: -1.0, max: 1.0 }];
        // 1/x is +/-infinity nowhere in this range except exactly at 0,
        // which isn't a corner - use a function that's actually
        // non-finite at one corner to prove the skip logic.
        let result = worst_case_corners(&bounds, |p| if p[0] < 0.0 { f64::NAN } else { p[0] });
        assert_eq!(result, WorstCaseResult { min: 1.0, max: 1.0 });
    }
}
