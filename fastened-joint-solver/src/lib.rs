//! Fastened Joint Preload Analysis - models a threaded fastener tightened
//! to a specified installation torque and solves the coupled mechanical
//! state of the fastener and clamped joint (thread + bearing friction
//! torque equilibrium, preload, fastener/member elastic compliance, nut
//! rotation, installation stress state, service-load/separation
//! behavior). Zero UI dependency - a TUI/GUI head only ever calls
//! [`solve::compute`] and renders the returned [`solve::JointSolution`].
//!
//! ## Scope (see the toolbox's own `AGENTS.md` entry for the full record)
//!
//! The primary solver never depends on the reduced `T = K*F*d` torque
//! coefficient - [`thread::thread_torque`] retains pitch diameter, lead
//! angle, and thread flank angle explicitly. Both of the spec's two
//! uncertainty engines are real: [`uncertainty::worst_case_corners`] (spec
//! section 13's deterministic exhaustive corner search) and a seeded
//! uniform-sampling Monte Carlo engine (spec section 51's second engine) -
//! both wired directly into [`solve::compute`]'s `JointSolution.uncertainty`/
//! `JointSolution.monte_carlo` output fields (enabled via `JointInputs.uncertainty`/
//! `JointInputs.monte_carlo`), not left as standalone utilities a caller
//! would have to assemble itself.
//!
//! Two deliberate scope cuts remain, both explicitly labeled "optional"/
//! "advanced" in the spec itself rather than core requirements:
//! - **Advanced per-thread load distribution** (spring-coupled individual
//!   engaged threads, a small `[K]{u}={F}` linear system) - the spec's own
//!   words: "this should be an advanced analysis option rather than
//!   mandatory for initial UI interaction." Thread-root stress here uses
//!   the standard uniform-root-area assumption instead.
//! - **Locking-feature prevailing torque as a function of rotation/engagement**
//!   (`T_prevailing(theta, F)`) - the spec's own words: "do not require
//!   this for baseline implementation." [`solve::FrictionInputs::prevailing_torque`]
//!   is a constant, which the spec explicitly permits as the baseline.

pub mod bearing;
pub mod compliance;
pub mod quadrature;
pub mod root;
pub mod service;
pub mod solve;
pub mod stress;
pub mod thread;
pub mod thread_catalog;
pub mod thread_load_distribution;
pub mod thread_shear;
pub mod uncertainty;
pub mod validation;
