//! Domain error type for fastener-hole geometry. No `unwrap()`/`expect()`
//! on user-derived input anywhere in this domain module tree - every
//! fallible calculation returns `Result<_, GeometryError>` instead, per the
//! spec's "do not panic from user input" rule.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryError {
    InvalidDiameter,
    OuterDiameterNotGreaterThanHole,
    InvalidDepth,
    InvalidAngle,
    InvalidToleranceRange,
    ImpossibleGeometry,
    UnderdeterminedGeometry,
    NonFiniteInput,
    AreaPreservationFailure,
}

impl std::fmt::Display for GeometryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            GeometryError::InvalidDiameter => "diameter must be finite and positive",
            GeometryError::OuterDiameterNotGreaterThanHole => "outer diameter must be greater than hole diameter",
            GeometryError::InvalidDepth => "depth must be finite and positive",
            GeometryError::InvalidAngle => "angle must be finite and within (0, 180) degrees",
            GeometryError::InvalidToleranceRange => "tolerance range must be finite",
            GeometryError::ImpossibleGeometry => "no physically valid geometry satisfies these inputs",
            GeometryError::UnderdeterminedGeometry => "exactly three of the four countersink dimensions must be given",
            GeometryError::NonFiniteInput => "input is not a finite number",
            GeometryError::AreaPreservationFailure => "secondary lateral area failed to reproduce the primary area",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for GeometryError {}
