//! Pressure loading and axial boundary condition - issue #11's own
//! "Pressure" and "Boundary Conditions" sections: internal pressure,
//! external pressure, combined, and closed/open axial end conditions.

/// How the vessel's ends react axial load. "The selected boundary
/// condition shall affect the applicable stress state appropriately" -
/// this is the one axial-stress-relevant choice v1 supports (see
/// `docs/issue-11-status.md`'s backlog for what's deliberately deferred:
/// externally-constrained/other axial conditions beyond these two).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EndCondition {
    /// Real end caps - pressure produces a real axial force the wall
    /// reacts (`mechanics_core::lame::closed_end_axial_stress`).
    Closed,
    /// No end caps for pressure to act on (or the axial load is reacted
    /// by an external structure, not this wall) - zero pressure-induced
    /// axial stress.
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PressureError {
    NegativePressure,
}

/// Internal + external pressure, both stored as non-negative magnitudes -
/// `mechanics_core::lame`'s own `p_inner`/`p_outer` convention already
/// encodes "compressive at that boundary" through the sign of the
/// resulting stress, so a negative pressure value here would be a
/// double-negative a caller could easily get backwards. Reject it
/// outright instead (issue #11: "Pressure sign conventions shall be
/// explicit and consistent").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressureLoading {
    pub internal_pressure: f64,
    pub external_pressure: f64,
    pub end_condition: EndCondition,
}

impl PressureLoading {
    pub fn new(internal_pressure: f64, external_pressure: f64, end_condition: EndCondition) -> Result<Self, PressureError> {
        if internal_pressure < 0.0 || external_pressure < 0.0 {
            return Err(PressureError::NegativePressure);
        }
        Ok(Self { internal_pressure, external_pressure, end_condition })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_pressures_are_accepted() {
        let p = PressureLoading::new(5000.0, 0.0, EndCondition::Closed).unwrap();
        assert_eq!(p.internal_pressure, 5000.0);
        assert_eq!(p.external_pressure, 0.0);
    }

    #[test]
    fn negative_pressure_is_rejected_for_either_side() {
        assert_eq!(PressureLoading::new(-1.0, 0.0, EndCondition::Closed), Err(PressureError::NegativePressure));
        assert_eq!(PressureLoading::new(0.0, -1.0, EndCondition::Closed), Err(PressureError::NegativePressure));
    }

    #[test]
    fn zero_pressure_on_both_sides_is_valid_not_an_error() {
        assert!(PressureLoading::new(0.0, 0.0, EndCondition::Open).is_ok());
    }
}
