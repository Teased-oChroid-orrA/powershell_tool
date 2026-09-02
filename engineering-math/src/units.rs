//! Extensible unit/quantity system - issue #10 Phase 1: "Support an
//! extensible unit/quantity system appropriate to the selected engineering
//! toolboxes. At minimum investigate/support: inch; millimeter; force;
//! pressure; torque; angle; area; volume; length; mass." Exactly those
//! families, no more - this is a foundation sized to what's actually
//! asked for, not a speculative general-purpose units library.
//!
//! Each family picks one **base unit** (chosen to match this workspace's
//! existing imperial-first convention - `bushing-solver` is imperial-only
//! by explicit design) and every other unit in that family converts
//! through it. Conversion factors are exact, cited constants, not
//! approximations:
//! - 1 in = 25.4 mm (exact, international yard-and-pound agreement, 1959).
//! - 1 lbf = 4.4482216152605 N (exact, derived from the international
//!   avoirdupois pound and standard gravity).
//! - 1 lbm = 0.45359237 kg (exact, same 1959 agreement).
//! - 1 psi = 1 lbf/in² = 4.4482216152605 N / (0.0254 m)² =
//!   6894.757293168361 Pa (derived, not independently defined).
//! - 1 rad = 180/π degrees (exact, by definition of radian).

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LengthUnit {
    Inch,
    Millimeter,
    Foot,
    Meter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForceUnit {
    PoundForce,
    Newton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PressureUnit {
    Psi,
    Ksi,
    Pascal,
    Megapascal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TorqueUnit {
    InchPoundForce,
    FootPoundForce,
    NewtonMeter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AngleUnit {
    Degree,
    Radian,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AreaUnit {
    SquareInch,
    SquareMillimeter,
    SquareFoot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeUnit {
    CubicInch,
    CubicMillimeter,
    CubicFoot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MassUnit {
    PoundMass,
    Kilogram,
}

/// One tagged unit from any supported family. `Quantity` pairs a raw
/// value with one of these; two quantities can only convert/compare
/// within the same family (enforced by [`UnitMismatch`], not by panicking).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Unit {
    Length(LengthUnit),
    Force(ForceUnit),
    Pressure(PressureUnit),
    Torque(TorqueUnit),
    Angle(AngleUnit),
    Area(AreaUnit),
    Volume(VolumeUnit),
    Mass(MassUnit),
}

impl Unit {
    /// Multiply a value in this unit by this factor to get the value in
    /// the family's base unit (Inch/PoundForce/Psi/InchPoundForce/Degree/
    /// SquareInch/CubicInch/PoundMass respectively).
    fn to_base_factor(self) -> f64 {
        match self {
            Unit::Length(LengthUnit::Inch) => 1.0,
            Unit::Length(LengthUnit::Millimeter) => 1.0 / 25.4,
            Unit::Length(LengthUnit::Foot) => 12.0,
            Unit::Length(LengthUnit::Meter) => 1000.0 / 25.4,

            Unit::Force(ForceUnit::PoundForce) => 1.0,
            Unit::Force(ForceUnit::Newton) => 1.0 / 4.448_221_615_260_5,

            Unit::Pressure(PressureUnit::Psi) => 1.0,
            Unit::Pressure(PressureUnit::Ksi) => 1000.0,
            Unit::Pressure(PressureUnit::Pascal) => 1.0 / 6894.757_293_168_361,
            Unit::Pressure(PressureUnit::Megapascal) => 1_000_000.0 / 6894.757_293_168_361,

            Unit::Torque(TorqueUnit::InchPoundForce) => 1.0,
            Unit::Torque(TorqueUnit::FootPoundForce) => 12.0,
            // 1 N*m = (1/0.0254 in) * (1/4.4482216152605 lbf) in*lbf
            Unit::Torque(TorqueUnit::NewtonMeter) => (1.0 / 0.0254) / 4.448_221_615_260_5,

            Unit::Angle(AngleUnit::Degree) => 1.0,
            Unit::Angle(AngleUnit::Radian) => 180.0 / std::f64::consts::PI,

            Unit::Area(AreaUnit::SquareInch) => 1.0,
            Unit::Area(AreaUnit::SquareMillimeter) => 1.0 / (25.4 * 25.4),
            Unit::Area(AreaUnit::SquareFoot) => 144.0,

            Unit::Volume(VolumeUnit::CubicInch) => 1.0,
            Unit::Volume(VolumeUnit::CubicMillimeter) => 1.0 / (25.4 * 25.4 * 25.4),
            Unit::Volume(VolumeUnit::CubicFoot) => 1728.0,

            Unit::Mass(MassUnit::PoundMass) => 1.0,
            Unit::Mass(MassUnit::Kilogram) => 1.0 / 0.453_592_37,
        }
    }

    fn family_name(self) -> &'static str {
        match self {
            Unit::Length(_) => "length",
            Unit::Force(_) => "force",
            Unit::Pressure(_) => "pressure",
            Unit::Torque(_) => "torque",
            Unit::Angle(_) => "angle",
            Unit::Area(_) => "area",
            Unit::Volume(_) => "volume",
            Unit::Mass(_) => "mass",
        }
    }

    fn same_family(self, other: Unit) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Unit::Length(LengthUnit::Inch) => "in",
            Unit::Length(LengthUnit::Millimeter) => "mm",
            Unit::Length(LengthUnit::Foot) => "ft",
            Unit::Length(LengthUnit::Meter) => "m",
            Unit::Force(ForceUnit::PoundForce) => "lbf",
            Unit::Force(ForceUnit::Newton) => "N",
            Unit::Pressure(PressureUnit::Psi) => "psi",
            Unit::Pressure(PressureUnit::Ksi) => "ksi",
            Unit::Pressure(PressureUnit::Pascal) => "Pa",
            Unit::Pressure(PressureUnit::Megapascal) => "MPa",
            Unit::Torque(TorqueUnit::InchPoundForce) => "in\u{00b7}lbf",
            Unit::Torque(TorqueUnit::FootPoundForce) => "ft\u{00b7}lbf",
            Unit::Torque(TorqueUnit::NewtonMeter) => "N\u{00b7}m",
            Unit::Angle(AngleUnit::Degree) => "\u{00b0}",
            Unit::Angle(AngleUnit::Radian) => "rad",
            Unit::Area(AreaUnit::SquareInch) => "in\u{00b2}",
            Unit::Area(AreaUnit::SquareMillimeter) => "mm\u{00b2}",
            Unit::Area(AreaUnit::SquareFoot) => "ft\u{00b2}",
            Unit::Volume(VolumeUnit::CubicInch) => "in\u{00b3}",
            Unit::Volume(VolumeUnit::CubicMillimeter) => "mm\u{00b3}",
            Unit::Volume(VolumeUnit::CubicFoot) => "ft\u{00b3}",
            Unit::Mass(MassUnit::PoundMass) => "lbm",
            Unit::Mass(MassUnit::Kilogram) => "kg",
        };
        f.write_str(s)
    }
}

/// Two units from different families were used together where they must
/// match (a conversion or a same-family comparison).
#[derive(Debug, Clone, PartialEq)]
pub struct UnitMismatch {
    pub from: &'static str,
    pub to: &'static str,
}

impl fmt::Display for UnitMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot convert {} to {} - different unit families", self.from, self.to)
    }
}

impl std::error::Error for UnitMismatch {}

/// A value paired with its unit. Arithmetic is deliberately NOT
/// implemented on `Quantity` directly (per issue #10's own "calculation
/// precision \u{2260} display preferences" separation, calculations work
/// in plain `f64` at full precision - see `precision.rs` - and `Quantity`
/// exists for input/output/display/conversion at the boundary, not as a
/// unit-checked arithmetic type threaded through every formula).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: Unit,
}

impl Quantity {
    pub fn new(value: f64, unit: Unit) -> Self {
        Self { value, unit }
    }

    /// This quantity's value expressed in its family's base unit.
    pub fn to_base(&self) -> f64 {
        self.value * self.unit.to_base_factor()
    }

    /// Converts to a different unit within the same family. `Err` if
    /// `target` is from a different family - never silently produces a
    /// nonsense number by converting, say, inches to psi.
    pub fn convert(&self, target: Unit) -> Result<Quantity, UnitMismatch> {
        if !self.unit.same_family(target) {
            return Err(UnitMismatch { from: self.unit.family_name(), to: target.family_name() });
        }
        let base = self.to_base();
        Ok(Quantity { value: base / target.to_base_factor(), unit: target })
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.value, self.unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64, label: &str) {
        let diff = (actual - expected).abs();
        let tol = expected.abs() * 1e-9 + 1e-12;
        assert!(diff <= tol, "{label}: expected {expected}, got {actual} (diff {diff})");
    }

    #[test]
    fn inch_to_millimeter_matches_the_exact_1959_definition() {
        let q = Quantity::new(1.0, Unit::Length(LengthUnit::Inch));
        let mm = q.convert(Unit::Length(LengthUnit::Millimeter)).unwrap();
        close(mm.value, 25.4, "1 in in mm");
    }

    #[test]
    fn round_trip_conversion_returns_to_the_original_value() {
        let original = Quantity::new(3.75, Unit::Length(LengthUnit::Inch));
        let mm = original.convert(Unit::Length(LengthUnit::Millimeter)).unwrap();
        let back = mm.convert(Unit::Length(LengthUnit::Inch)).unwrap();
        close(back.value, original.value, "round trip in->mm->in");
    }

    #[test]
    fn psi_to_pascal_matches_the_derived_constant() {
        let q = Quantity::new(1.0, Unit::Pressure(PressureUnit::Psi));
        let pa = q.convert(Unit::Pressure(PressureUnit::Pascal)).unwrap();
        close(pa.value, 6894.757293168361, "1 psi in Pa");
    }

    #[test]
    fn ksi_to_psi_is_exactly_a_factor_of_1000() {
        let q = Quantity::new(70.0, Unit::Pressure(PressureUnit::Ksi));
        let psi = q.convert(Unit::Pressure(PressureUnit::Psi)).unwrap();
        close(psi.value, 70_000.0, "70 ksi in psi");
    }

    #[test]
    fn degree_to_radian_matches_pi_over_180() {
        let q = Quantity::new(180.0, Unit::Angle(AngleUnit::Degree));
        let rad = q.convert(Unit::Angle(AngleUnit::Radian)).unwrap();
        close(rad.value, std::f64::consts::PI, "180 deg in rad");
    }

    #[test]
    fn cross_family_conversion_is_a_typed_error_not_a_panic_or_garbage_value() {
        let q = Quantity::new(1.0, Unit::Length(LengthUnit::Inch));
        let err = q.convert(Unit::Pressure(PressureUnit::Psi)).unwrap_err();
        assert_eq!(err.from, "length");
        assert_eq!(err.to, "pressure");
    }

    #[test]
    fn pound_mass_to_kilogram_matches_the_exact_1959_definition() {
        let q = Quantity::new(1.0, Unit::Mass(MassUnit::PoundMass));
        let kg = q.convert(Unit::Mass(MassUnit::Kilogram)).unwrap();
        close(kg.value, 0.45359237, "1 lbm in kg");
    }

    #[test]
    fn square_inch_to_square_millimeter_is_the_squared_length_factor() {
        let q = Quantity::new(1.0, Unit::Area(AreaUnit::SquareInch));
        let mm2 = q.convert(Unit::Area(AreaUnit::SquareMillimeter)).unwrap();
        close(mm2.value, 25.4 * 25.4, "1 in^2 in mm^2");
    }

    #[test]
    fn display_formats_value_and_unit_suffix() {
        let q = Quantity::new(0.5, Unit::Length(LengthUnit::Inch));
        assert_eq!(format!("{q}"), "0.5 in");
    }
}
