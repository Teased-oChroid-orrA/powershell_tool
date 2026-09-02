//! Straight-bushing interference-fit calculation engine - see
//! `Cargo.toml`'s module doc comment for scope and provenance.

pub mod bearing;
pub mod countersink;
pub mod geometry;
pub mod reamers;
pub mod solve;
pub mod tolerance;

// `lame`/`materials` moved to the standalone `mechanics_core` crate
// (issue #11 Phase 1 - see docs/issue-11-phase-1.md) - both this crate's
// own internal call sites and `app`'s now reference `mechanics_core::...`
// directly rather than being re-exported through here.
