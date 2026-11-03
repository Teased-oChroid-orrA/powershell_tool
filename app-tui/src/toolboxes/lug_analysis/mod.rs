//! Lug Analysis model: a pin-loaded lug solved with the contact finite-element
//! model in `lug-solver` (on the general kernel by default; the condensed
//! rigid-pin solvers are the legacy comparison), compared with the material
//! allowables. The Lug window (`gpu_viewer/editor/lug/`, opened from the
//! dashboard) is its only head.
//!
//! `model.rs` holds the inputs, field rows, the run (`model::run`, with the
//! condensed model cached between runs) and the row text; `mesh_test.rs` the
//! brief mesh-size convergence test.

pub mod mesh_test;
pub mod model;
