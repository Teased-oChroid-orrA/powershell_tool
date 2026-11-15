//! Pressure Vessel Stress, Failure Mode & Minimum Thickness Analyzer -
//! see `Cargo.toml`'s module doc comment for scope (issue #11 v1) and
//! `docs/issue-11-status.md`/`docs/issue-11-phase-2.md` in the
//! `powershell_tool` repo for the evidence and verification behind it.

pub mod buckling;
pub mod criterion_thickness;
pub mod failure;
pub mod geometry;
pub mod heads;
pub mod junction;
mod line_beam;
pub mod line_run;
pub mod pressure;
pub mod stress;
pub mod thermal;
pub mod thickness;
