//! `PrecisionPolicy` - issue #10 Phase 1: "supporting decimal places,
//! significant figures, operation-aware rounding, and configuration",
//! plus the epic's own **absolute precision rule**: "All intermediate
//! calculations must retain full available internal precision. No
//! intermediate rounding."
//!
//! That rule shapes this module's whole API: nothing here mutates a
//! running calculation's precision automatically (Rust has no operator-
//! overload hook that could do that safely without becoming exactly the
//! kind of surprising, implicit magic this project avoids elsewhere).
//! Calculations are written in plain `f64` at full IEEE-754 precision,
//! the same as every other crate in this workspace already does
//! (`bushing-solver` never rounds mid-formula). `PrecisionPolicy` is
//! consulted at two well-defined boundaries instead:
//! - when an engineer explicitly wants to snap an intermediate result to
//!   an operation-appropriate rounding rule (`round_addition_subtraction`/
//!   `round_multiplication_division`, issue #10's own "addition/
//!   subtraction governed by decimal places; multiplication/division by
//!   significant figures" split), and
//! - when a value is about to be **displayed** (`format_for_display`),
//!   which never feeds back into any calculation.
//!
//! This is also the module issue #10 asks every toolbox to route through
//! instead of hand-rolling: "No toolbox may independently implement:
//! rounding; significant figures; engineering decimal formatting."

use serde::{Deserialize, Serialize};

/// One rounding rule: fixed decimal places, or a fixed count of
/// significant figures. Two different, real engineering conventions
/// (issue #10's own split is addition/subtraction -> decimal places,
/// multiplication/division -> significant figures) - never conflated.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RoundingRule {
    DecimalPlaces(u32),
    SignificantFigures(u32),
}

impl RoundingRule {
    /// Rounds `value` to this rule, returning a plain `f64` - for the
    /// rare case a calculation genuinely wants a rounded intermediate
    /// (e.g. matching a manufacturing process's real achievable
    /// resolution), not for routine formatting - use
    /// [`PrecisionPolicy::format_for_display`] for that instead, since a
    /// rounded `f64` alone loses trailing-zero display information
    /// (`1.50` prints as `1.5`).
    pub fn apply(self, value: f64) -> f64 {
        if !value.is_finite() {
            return value;
        }
        match self {
            RoundingRule::DecimalPlaces(places) => {
                let factor = 10f64.powi(places as i32);
                (value * factor).round() / factor
            }
            RoundingRule::SignificantFigures(sig_figs) => round_to_significant_figures(value, sig_figs),
        }
    }

    /// Formats `value` as a display string honoring this rule exactly -
    /// trailing zeros preserved (`SignificantFigures(3)` on `1.5` prints
    /// `"1.50"`, not `"1.5"`), the detail a bare rounded `f64` can't carry.
    pub fn format(self, value: f64) -> String {
        if !value.is_finite() {
            return value.to_string();
        }
        match self {
            RoundingRule::DecimalPlaces(places) => format!("{:.*}", places as usize, value),
            RoundingRule::SignificantFigures(sig_figs) => format_significant_figures(value, sig_figs),
        }
    }
}

fn round_to_significant_figures(value: f64, sig_figs: u32) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    let magnitude = value.abs().log10().floor() as i32;
    let power = sig_figs as i32 - magnitude - 1;
    let factor = 10f64.powi(power);
    (value * factor).round() / factor
}

/// Decimal places needed to show exactly `sig_figs` significant figures
/// of `value`, clamped to zero for magnitudes large enough that the
/// requested significant-figure count would otherwise call for negative
/// decimal places (e.g. 2 sig figs of 12345 - a real simplification for
/// very large values, not a bug: this policy is aimed at typical
/// engineering dimension/stress magnitudes, not arbitrary-scale numbers).
fn format_significant_figures(value: f64, sig_figs: u32) -> String {
    if value == 0.0 {
        return format!("{:.*}", sig_figs.saturating_sub(1) as usize, 0.0);
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (sig_figs as i32 - magnitude - 1).max(0) as usize;
    format!("{:.*}", decimals, value)
}

/// The engineering-rounding rules for one calculation context, plus the
/// separate display rule - issue #10's own "Calculation precision !=
/// Engineering precision rules != Display preferences" split, kept as
/// three distinct fields rather than one rule reused three ways.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PrecisionPolicy {
    pub addition_subtraction: RoundingRule,
    pub multiplication_division: RoundingRule,
    pub display: RoundingRule,
}

impl PrecisionPolicy {
    /// A reasonable default for imperial engineering dimension/stress
    /// work (4 decimal places for add/sub - matches `bushing-solver`'s
    /// own existing `{:.4}` convention for lengths; 4 significant figures
    /// for mult/div and display - matches its own `{:.0}` psi/`{:+.2}`
    /// margin conventions closely enough to be a sane starting point).
    /// Callers with different real requirements should build their own
    /// `PrecisionPolicy` rather than mutate this one's fields after the
    /// fact - it is `Copy`, not a shared mutable singleton.
    pub fn engineering_default() -> Self {
        Self {
            addition_subtraction: RoundingRule::DecimalPlaces(4),
            multiplication_division: RoundingRule::SignificantFigures(4),
            display: RoundingRule::SignificantFigures(4),
        }
    }

    pub fn round_addition_subtraction(&self, value: f64) -> f64 {
        self.addition_subtraction.apply(value)
    }

    pub fn round_multiplication_division(&self, value: f64) -> f64 {
        self.multiplication_division.apply(value)
    }

    /// The only place a value's precision should actually be reduced for
    /// a human to read - never call this on an intermediate and keep
    /// calculating from the result.
    pub fn format_for_display(&self, value: f64) -> String {
        self.display.format(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_places_rounds_to_the_requested_precision() {
        let r = RoundingRule::DecimalPlaces(2);
        assert_eq!(r.apply(1.23456), 1.23);
        assert_eq!(r.apply(1.235), 1.24);
    }

    #[test]
    fn significant_figures_rounds_correctly_across_magnitudes() {
        let r = RoundingRule::SignificantFigures(3);
        assert_eq!(r.apply(12345.0), 12300.0);
        assert_eq!(r.apply(0.0012345), 0.00123);
        assert_eq!(r.apply(9.999), 10.0);
    }

    #[test]
    fn significant_figures_handles_negative_values_by_magnitude_not_sign() {
        let r = RoundingRule::SignificantFigures(3);
        assert_eq!(r.apply(-12345.0), -12300.0);
    }

    #[test]
    fn significant_figures_of_zero_is_zero_not_nan_or_panic() {
        let r = RoundingRule::SignificantFigures(4);
        assert_eq!(r.apply(0.0), 0.0);
    }

    #[test]
    fn format_preserves_trailing_zeros_that_a_bare_rounded_f64_would_lose() {
        let r = RoundingRule::SignificantFigures(3);
        assert_eq!(r.format(1.5), "1.50");
        assert_eq!(r.apply(1.5), 1.5); // the numeric value has no trailing-zero concept
    }

    #[test]
    fn format_decimal_places_pads_to_the_requested_width() {
        let r = RoundingRule::DecimalPlaces(4);
        assert_eq!(r.format(0.5), "0.5000");
    }

    #[test]
    fn engineering_default_separates_addition_subtraction_from_multiplication_division() {
        let p = PrecisionPolicy::engineering_default();
        assert_ne!(p.addition_subtraction, p.multiplication_division);
        // A value whose correctly-rounded decimal-place and significant-
        // figure results differ proves the two rules are actually wired
        // to different fields, not both silently defaulting to the same
        // rule.
        let v = 123.456789;
        assert_ne!(p.round_addition_subtraction(v), p.round_multiplication_division(v));
    }

    #[test]
    fn display_never_mutates_the_value_used_for_further_calculation() {
        // The whole point of the split: format_for_display returns a
        // String, not a rounded f64 a caller could accidentally keep
        // computing from.
        let p = PrecisionPolicy::engineering_default();
        let v = 1.0 / 3.0;
        let displayed = p.format_for_display(v);
        assert_eq!(v, 1.0 / 3.0, "original value must be untouched");
        assert!(displayed.len() < 20, "display string should be a short rounded representation");
    }

    #[test]
    fn policy_round_trips_through_json() {
        let p = PrecisionPolicy::engineering_default();
        let json = serde_json::to_string(&p).unwrap();
        let back: PrecisionPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
