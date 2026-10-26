//! The Fastener Hole case file: every input of `FastenerHoleModel` (the hole type, how tolerances are entered, the
//! countersink's solved dimension and secondary method, and each toleranced dimension as min / nominal / max), as
//! JSON keyed by name so a file outlives a reordering of the enums. Results are never stored: they are recomputed.
use crate::toolboxes::fastener_hole::domain::{CountersinkSolveFor, SecondaryCountersinkMethod, TolerancedValue};
use crate::toolboxes::fastener_hole::model::{FastenerHoleModel, HoleType, NumberTarget, ToleranceInputMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FORMAT_VERSION: u32 = 1;

/// Every input dimension with its key in the file.
pub const DIMENSIONS: [(NumberTarget, &str); 8] = [
    (NumberTarget::RegularHole1, "hole 1 diameter"),
    (NumberTarget::RegularHole2, "hole 2 diameter"),
    (NumberTarget::RegularReference, "secondary reference hole"),
    (NumberTarget::CsOuterDiameter, "countersink outer diameter"),
    (NumberTarget::CsHoleDiameter, "countersink hole diameter"),
    (NumberTarget::CsDepth, "countersink depth"),
    (NumberTarget::CsAngle, "countersink angle"),
    (NumberTarget::CsSecondaryHoleDiameter, "secondary hole diameter"),
];
#[allow(dead_code)]
fn every_number_target_is_listed(t: NumberTarget) {
    use NumberTarget::*;
    match t {
        RegularHole1 | RegularHole2 | RegularReference | CsOuterDiameter | CsHoleDiameter | CsDepth | CsAngle | CsSecondaryHoleDiameter => {}
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FastenerFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub choices: BTreeMap<String, String>,
    /// `[min, nominal, max]` per dimension.
    #[serde(default)]
    pub dimensions: BTreeMap<String, [f64; 3]>,
}

/// The value after cycling `start` until its label reads `want` (`None`: no such choice in this build).
fn from_label<T: Copy>(start: T, cycle: fn(T) -> T, label: fn(T) -> &'static str, want: &str) -> Option<T> {
    let mut v = start;
    for _ in 0..8 {
        if label(v) == want {
            return Some(v);
        }
        v = cycle(v);
    }
    None
}

impl FastenerFile {
    pub fn capture(m: &FastenerHoleModel, name: &str) -> Self {
        let choices = [
            ("hole type", m.hole_type.label()),
            ("tolerance entry", m.tolerance_mode.label()),
            ("solve for", m.countersink.solve_for.label()),
            ("secondary method", m.countersink.secondary_method.label()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let dimensions = DIMENSIONS
            .iter()
            .map(|(t, key)| {
                let v = m.get_toleranced(*t);
                (key.to_string(), [v.min, v.nominal, v.max])
            })
            .collect();
        Self { version: FORMAT_VERSION, name: name.to_string(), choices, dimensions }
    }

    /// Put the file's inputs into `m` and recompute. Returns what could not be applied (a choice this build does not
    /// offer, a dimension whose bounds are not a valid tolerance band); everything else is applied.
    pub fn apply(&self, m: &mut FastenerHoleModel) -> Vec<String> {
        let mut skipped = Vec::new();
        let mut choice = |key: &str, apply: &mut dyn FnMut(&str) -> bool| {
            if let Some(want) = self.choices.get(key) {
                if !apply(want) {
                    skipped.push(format!("{key}: {want}"));
                }
            }
        };
        choice("hole type", &mut |w| from_label(HoleType::Regular, HoleType::cycle, HoleType::label, w).map(|v| m.hole_type = v).is_some());
        choice("tolerance entry", &mut |w| from_label(ToleranceInputMode::NominalTol, ToleranceInputMode::cycle, ToleranceInputMode::label, w).map(|v| m.tolerance_mode = v).is_some());
        choice("solve for", &mut |w| from_label(CountersinkSolveFor::Angle, CountersinkSolveFor::cycle, CountersinkSolveFor::label, w).map(|v| m.countersink.solve_for = v).is_some());
        choice("secondary method", &mut |w| from_label(SecondaryCountersinkMethod::PreserveDepth, SecondaryCountersinkMethod::cycle, SecondaryCountersinkMethod::label, w).map(|v| m.countersink.secondary_method = v).is_some());
        for (t, key) in DIMENSIONS {
            let Some([min, nominal, max]) = self.dimensions.get(key).copied() else { continue };
            // A nominal outside its own band is not a tolerance the form could have produced.
            match TolerancedValue::from_bounds_with_nominal(nominal, min, max).ok().filter(|v| v.nominal == nominal) {
                Some(v) => m.set_toleranced(t, v),
                None => skipped.push(format!("{key}: {min} / {nominal} / {max}")),
            }
        }
        m.recompute();
        skipped
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plain data serializes")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let f: Self = serde_json::from_str(text).map_err(|e| format!("not a fastener hole case: {e}"))?;
        if f.version > FORMAT_VERSION {
            return Err(format!("written by a newer version (format {}, this build reads {FORMAT_VERSION})", f.version));
        }
        Ok(f)
    }
}
