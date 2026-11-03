//! Pressure Vessel Analyzer model: cylindrical-vessel stress, failure-mode,
//! buckling, thermal-stress and minimum-thickness analysis over
//! `pressure-vessel-solver`. The Pressure Vessel window
//! (`gpu_viewer/editor/vessel/`, opened from the dashboard) is its only head.
//!
//! `model.rs` bridges the pure solver crate into UI-facing state (the field
//! rows and their values), `report.rs` builds the plain-text report and row
//! text, `material_picker.rs` holds the catalog filter and the "add new
//! material" form, and `persistence.rs` keeps materials added that way
//! between runs.

pub mod material_picker;
pub mod model;
pub mod persistence;
pub mod report;
