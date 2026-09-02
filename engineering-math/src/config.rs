//! User display configuration - issue #10 Phase 1: "Engineering display
//! preferences must be persisted independently of calculation logic...
//! display changes never alter mathematical results." Deliberately a
//! separate type from [`crate::precision::PrecisionPolicy`]: this struct
//! is what a user chose to see (preferred units, how many digits, whether
//! to show the calculation trace); `PrecisionPolicy` is the engineering
//! rounding rule a calculation follows. Nothing here feeds back into a
//! calculation's own arithmetic.

use crate::units::{AngleUnit, LengthUnit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DisplayConfig {
    pub preferred_length_unit: LengthUnit,
    pub preferred_angle_unit: AngleUnit,
    pub length_decimal_places: u32,
    pub angle_decimal_places: u32,
    pub stress_significant_figures: u32,
    pub show_calculation_trace: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            preferred_length_unit: LengthUnit::Inch,
            preferred_angle_unit: AngleUnit::Degree,
            length_decimal_places: 4,
            angle_decimal_places: 1,
            stress_significant_figures: 4,
            show_calculation_trace: false,
        }
    }
}

impl DisplayConfig {
    /// Serializes to JSON for persistence across application restarts -
    /// `serde_json`, the same crate every other workspace member already
    /// depends on for this, not a new dependency.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_this_workspaces_existing_imperial_convention() {
        let cfg = DisplayConfig::default();
        assert_eq!(cfg.preferred_length_unit, LengthUnit::Inch);
        assert_eq!(cfg.preferred_angle_unit, AngleUnit::Degree);
        assert!(!cfg.show_calculation_trace);
    }

    #[test]
    fn round_trips_through_json_exactly() {
        let mut cfg = DisplayConfig::default();
        cfg.preferred_length_unit = LengthUnit::Millimeter;
        cfg.show_calculation_trace = true;
        cfg.length_decimal_places = 2;
        let json = cfg.to_json().unwrap();
        let back = DisplayConfig::from_json(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn from_json_rejects_malformed_input_instead_of_silently_defaulting() {
        let result = DisplayConfig::from_json("{ not json");
        assert!(result.is_err());
    }
}
