//! Fastened Joint Preload Analysis model: a threaded fastener tightened to a
//! specified installation torque, with the coupled mechanical state of the
//! fastener and clamped joint solved over `fastened-joint-solver` (thread +
//! bearing friction torque equilibrium, preload, fastener/member elastic
//! compliance, nut rotation, installation stress state, service-load/separation
//! behavior - see that crate's own `lib.rs` doc comment for the full scope).
//! The Preload window (`gpu_viewer/editor/preload/`, opened from the dashboard)
//! is its only head.
//!
//! `model.rs` bridges the solver into UI-facing state (field rows, bolt
//! catalog, members), `fe_check.rs` is the member-compliance FE check on the
//! general kernel, `joint_templates.rs` the joint presets and random joints,
//! and `report.rs` the plain-text report and row text.

pub mod fe_check;
pub mod joint_templates;
pub mod model;
pub mod report;
