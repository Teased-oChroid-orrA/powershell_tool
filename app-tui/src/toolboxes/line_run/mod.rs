//! Line Run model: a supported pressure tube checked over `pressure_vessel_solver::line_run` (member actions per load
//! case, stress tensor, failure margins). The Line Run window (`gpu_viewer/editor/line_run/`, opened from the
//! dashboard) is its only head.
//!
//! `model.rs` holds the inputs, the unit system and the solver bridge; `report.rs` the plain-text report and the
//! station CSV (what the window's Export writes).

pub mod model;
pub mod report;
