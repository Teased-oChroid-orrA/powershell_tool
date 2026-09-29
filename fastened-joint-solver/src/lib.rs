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
//! **Advanced per-thread load distribution** (spec section 36, spring-coupled
//! individual engaged threads) is implemented in [`thread_load_distribution`] -
//! a dense `[K]{u}={F}` linear system (bolt-body/nut-body pitch segments plus
//! a per-thread coupling spring, per-thread stiffness sourced from Zhang et
//! al. 2018's two-term deflection model), wired into `JointSolution.thread_load_distribution`
//! and opt-in via `JointInputs.thread_load_distribution` (spec's own words:
//! "advanced analysis option rather than mandatory for initial UI
//! interaction" - hence `Option`-gated, matching the uncertainty engines'
//! own pattern, not always-on). Validated against the continuum sinh
//! closed-form solution as a real differential test (`thread_load_distribution.rs`'s
//! own `first_thread_fraction_converges_toward_the_continuum_sinh_prediction_as_engagement_grows`).
//!
//! Embedment/settlement (spec section 24) is implemented via `JointInputs.embedment_settlement` -
//! reduces the preload `AnalysisMode::RotationControlled` derives from an
//! imposed rotation, and is reported in `DeformationState::embedment_settlement`/
//! `total_closure` regardless of mode. Thread stripping/shear (spec section
//! 35) is implemented in [`thread_shear`] via the FED-STD-H28 same-material
//! simplified area formula (this crate has no thread-class tolerance-band
//! dimensions to support the full external/internal-distinct form - see
//! that module's own doc comment), producing `JointSolution.thread_shear_margin`
//! when both `JointInputs.thread_engagement_length` and `thread_shear_strength`
//! are supplied.
//!
//! Head-side vs. nut-side bearing geometry (spec section 48) is real:
//! `JointGeometry.head_bearing_inner_radius`/`head_bearing_outer_radius`
//! (defaulting to the nut-side pair when unset) are selected by
//! `JointInputs.tightening_from` for every torque/friction calculation -
//! this was previously a dead UI toggle (`TighteningMember` was stored but
//! never read by `solve::compute`); fixed this session.
//!
//! One deliberate scope cut remains, explicitly labeled "future
//! enhancement" in the spec itself rather than a core requirement:
//! - **Locking-feature prevailing torque as a function of rotation/engagement**
//!   (`T_prevailing(theta, F)`) - the spec's own words: "do not require
//!   this for baseline implementation." [`solve::FrictionInputs::prevailing_torque`]
//!   is a constant, which the spec explicitly permits as the baseline.
//!
//! ## Fastener catalog scope (`thread_catalog`)
//!
//! AN3-AN20, NAS (UNJF tension/shear bolts and UNF machine bolts), MS21250,
//! and Hi-Lok (HL18 pin) geometry are all sourced (see `thread_catalog.rs`'s
//! own doc comment for citations). Two things were deliberately left out
//! rather than partially fabricated:
//! - **MS20004-MS20024** (internal wrenching bolts) - only MS20004 itself
//!   (1/4-28) could be sourced; the series is omitted entirely rather than
//!   shipping 19 invented entries alongside the one real one.
//! - **Lockbolts** (Huck/pull-type and stump-type swaged collar fasteners) -
//!   no catalog or analysis-mode path at all. Lockbolts are installed by
//!   swaging (tension/tension or hammer-swage), not torque, so this
//!   crate's torque/preload/rotation-controlled framework does not apply to
//!   them, and no sourced swage-load-to-clamp-force formula was found to
//!   build a correct alternative model on. Hi-Lok pins ARE cataloged (their
//!   thread geometry is real and useful for stress/margin calculations),
//!   but selecting one does not suggest a torque or preload value either -
//!   no sourced clamp-up-force-vs-diameter table exists publicly for those
//!   either (only ultimate shear/tension strength is published) - see
//!   `preload_analysis/bolt_picker.rs`'s own doc comment.

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
