//! Spin capacity of an eccentric bushing in a round housing boss.
//!
//! The bushing's bore is offset by `e` from its outer-diameter centre `O`. A pin load `F` at angle `phi` to the
//! offset line acts through the bore centre and turns the bushing about `O` with the moment
//! `F e sin(phi)`. Only friction on the interference-fit interface can return that torque, so the capacity is
//! `T_cap = sum mu p |x| w` over the interface (every point at its Coulomb limit) with the pressure `p` the FE
//! gives after the fit **and** the pin load; the concentric Lame pressure is only the `e = 0` limit.
//! `docs/eccentric-bushing.md` has the derivation and the verification plan.

mod control;
mod model;
mod offset;

pub use control::{Control, Progress, ProgressState};
pub use fea_core::Interrupt;
pub use model::{analyze, analyze_fields, analyze_with, fit_capacity, fit_capacity_with, spin_onset_torque, Analysis, Elasticity, Inputs, Orthotropy, ProfileBin};
pub use offset::{max_load, max_load_with, max_offset, max_offset_with, sweep_offset, OffsetLimit, SweepPoint};
