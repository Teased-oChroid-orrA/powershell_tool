//! A reusable calculation-trace system - issue #10 Phase 1: "reusable
//! calculation-trace system exposing inputs, formulas, intermediates, and
//! unrounded results." Generic on purpose: any toolbox's solver pushes
//! [`CalcStep`]s as it computes, and a UI can render the resulting
//! [`CalcTrace`] the same way regardless of which toolbox produced it -
//! unlike `bushing_workbench.rs`'s existing `DerivationBlock`, which is
//! real and works but has its formula list and value formatting hand-
//! written specifically for bushing fields.
//!
//! Every recorded value is the **unrounded** result (issue #10's own
//! absolute precision rule) - rounding only happens later, at display
//! time, via [`crate::precision::PrecisionPolicy`]. This module doesn't
//! call into `PrecisionPolicy` itself; a UI layer combines the two when
//! it actually renders a step.

use serde::{Deserialize, Serialize};

/// One named input value feeding a [`CalcStep`] - kept as `(name, value)`
/// pairs rather than a free-form string so a UI can format each value
/// with its own unit/precision rules instead of parsing prose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalcInput {
    pub name: String,
    pub value: f64,
}

/// One step of a calculation: what formula ran, what it consumed, and
/// what it produced - unrounded. `formula` is a short, human-readable
/// expression (e.g. `"pressure = delta / (term_b + term_h)"`), not a
/// machine-parsed one; this trace is for a human to read, not to replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalcStep {
    pub label: String,
    pub formula: String,
    pub inputs: Vec<CalcInput>,
    pub result: f64,
}

/// An ordered record of every step a calculation took, from raw inputs to
/// final unrounded result. Cheap to build (`Vec` push), cheap to ignore
/// (a caller that doesn't want a trace just never constructs one) -
/// deliberately not threaded through every function signature as a
/// mandatory parameter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CalcTrace {
    pub steps: Vec<CalcStep>,
}

impl CalcTrace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one step. `inputs` takes `(&str, f64)` pairs for call-site
    /// brevity; stored as owned [`CalcInput`]s.
    pub fn step(&mut self, label: impl Into<String>, formula: impl Into<String>, inputs: &[(&str, f64)], result: f64) {
        self.steps.push(CalcStep {
            label: label.into(),
            formula: formula.into(),
            inputs: inputs.iter().map(|(name, value)| CalcInput { name: (*name).to_string(), value: *value }).collect(),
            result,
        });
    }

    /// The last recorded step's result, if any - the calculation's final
    /// unrounded output, when the trace was built in calculation order.
    pub fn final_result(&self) -> Option<f64> {
        self.steps.last().map(|s| s.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_record_in_order_with_unrounded_results() {
        let mut trace = CalcTrace::new();
        trace.step("Contact pressure", "p = delta / (term_b + term_h)", &[("delta", 0.0015), ("term_b", 1e-7), ("term_h", 2e-7)], 5000.0 / 3.0);
        trace.step("Housing hoop stress", "sigma_h = p * (D^2+d^2)/(D^2-d^2)", &[("p", 5000.0 / 3.0)], 12345.6789);
        assert_eq!(trace.steps.len(), 2);
        assert_eq!(trace.steps[0].inputs[0].name, "delta");
        assert_eq!(trace.steps[0].result, 5000.0 / 3.0, "result must be the exact unrounded f64, not pre-rounded");
    }

    #[test]
    fn final_result_is_the_last_steps_result() {
        let mut trace = CalcTrace::new();
        trace.step("a", "a = 1", &[], 1.0);
        trace.step("b", "b = a + 1", &[("a", 1.0)], 2.0);
        assert_eq!(trace.final_result(), Some(2.0));
    }

    #[test]
    fn empty_trace_has_no_final_result() {
        let trace = CalcTrace::new();
        assert_eq!(trace.final_result(), None);
    }

    #[test]
    fn trace_round_trips_through_json() {
        let mut trace = CalcTrace::new();
        trace.step("step", "x = y", &[("y", 2.0)], 2.0);
        let json = serde_json::to_string(&trace).unwrap();
        let back: CalcTrace = serde_json::from_str(&json).unwrap();
        assert_eq!(trace, back);
    }
}
