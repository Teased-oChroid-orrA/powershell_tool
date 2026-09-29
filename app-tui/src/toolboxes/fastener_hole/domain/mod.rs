//! Fastener-hole engineering domain: tolerance normalization, regular-hole
//! fit analysis, and countersink geometry/solving. Zero `ratatui`/
//! `crossterm` dependency anywhere in this module tree - see this
//! toolbox's own `mod.rs` doc comment for why that boundary matters.

pub mod countersink;
pub mod errors;
pub mod regular_hole;
pub mod surface_area;
pub mod tolerance;

pub use countersink::{AreaPreservationCheck, CountersinkGeometry, CountersinkInputs, CountersinkSolveFor, SecondaryCountersinkMethod};
pub use errors::GeometryError;
pub use regular_hole::{FitClassification, FitEnvelope, FitPreservationCheck};
pub use tolerance::{DimensionInterval, TolerancedValue};
