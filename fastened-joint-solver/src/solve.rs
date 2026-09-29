//! Top-level solver pipeline (spec section 52):
//!
//! ```text
//! Geometry validation -> thread geometry -> bearing geometry ->
//! fastener compliance -> member compliance -> torque/preload root solve ->
//! compatibility solve -> nut rotation -> stress state -> contact state ->
//! external load state -> verification
//! ```
//!
//! `JointInputs` is the whole engineering configuration; `compute` (or its
//! mode-specific wrappers) is the one entry point, returning a single
//! immutable `JointSolution` (spec section 68) a UI only ever reads from -
//! it never independently recomputes any engineering value.

use crate::bearing::{average_bearing_pressure, bearing_torque, BearingPressureModel};
use crate::compliance::{build_fastener_segments, fastener_axial_compliance, fastener_torsional_compliance, member_stack_compliance, FastenerSegment, MemberStack};
use crate::quadrature::composite_simpson;
use crate::root::{brent, RootError};
use crate::service::{apply_external_load, joint_load_fraction, slip_capacity, slip_margin, ServiceLoadState};
use crate::stress::{axial_stress, principal_stresses, torsional_shear_stress, von_mises_stress, PrincipalStresses, StressState};
use crate::thread::{thread_torque, ThreadGeometry};
use crate::thread_load_distribution::{self, ThreadLoadDistribution, ThreadLoadDistributionInputs};
use crate::thread_shear::{shear_margin, thread_shear_area, thread_shear_stress};
use crate::uncertainty::{worst_case_corners, Bound};
use crate::validation::{self, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnalysisMode {
    /// User supplies installation torque; solver determines preload.
    TorqueControlled { applied_torque: f64 },
    /// User supplies target preload directly; solver determines the
    /// installation torque that would produce it.
    PreloadControlled { target_preload: f64 },
    /// User supplies turns (revolutions) after snug; solver determines
    /// preload and torque from the elastic closure this rotation implies.
    /// Requires an explicit snug/reference state (spec section 26) -
    /// `snug_preload` is that reference, almost always `0.0`.
    RotationControlled { turns_after_snug: f64, snug_preload: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TighteningMember {
    Nut,
    BoltHead,
}

/// Bounded/tolerance inputs for the two uncertainty engines (spec section
/// 13/51): thread friction, bearing friction, pitch diameter, bearing
/// (contact outer radius) diameter, and applied-torque tolerance - the
/// same five independent variables the spec's own example list names.
/// Only meaningful (and only ever evaluated) for [`AnalysisMode::TorqueControlled`] -
/// preload is the user's own direct input in the other two modes, so there
/// is nothing to propagate uncertainty *into*.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UncertaintyInputs {
    /// Fractional (not percent) +/- band on `mu_thread`, e.g. `0.2` = +/-20%.
    pub mu_thread_tol_fraction: f64,
    pub mu_bearing_tol_fraction: f64,
    /// Absolute +/- band on pitch diameter, same length unit as the rest
    /// of this crate's inputs.
    pub pitch_diameter_tol: f64,
    /// Absolute +/- band on the bearing contact outer radius.
    pub bearing_outer_radius_tol: f64,
    pub applied_torque_tol_fraction: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UncertaintyResult {
    pub preload_min: f64,
    pub preload_nominal: f64,
    pub preload_max: f64,
}

/// Monte Carlo settings - spec section 51's second uncertainty engine.
/// Samples are drawn uniformly within the *same* five bounds
/// [`UncertaintyInputs`] already defines for the worst-case engine (a
/// deliberate simplification: one set of tolerance inputs powers both
/// engines, rather than requiring a second, separate distribution-shape UI
/// per variable - absent more specific distribution data, uniform-within-
/// tolerance is the standard default assumption). `seed` makes every run
/// exactly reproducible (spec: "use a deterministic seed option for
/// reproducibility").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonteCarloSettings {
    pub samples: u32,
    pub seed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonteCarloResult {
    /// Number of samples that actually produced a finite, converged
    /// preload - may be less than the requested count if some corners of
    /// the sampled space fail to bracket a root.
    pub samples: u32,
    pub seed: u64,
    pub preload_mean: f64,
    pub preload_std_dev: f64,
    pub preload_min: f64,
    pub preload_max: f64,
}

/// Minimal deterministic PRNG (SplitMix64 - Vigna's public-domain
/// generator, the standard seed-expansion step for the xoshiro/xoroshiro
/// family) - no external `rand` dependency needed for a single uniform
/// generator used only for Monte Carlo sampling here. Fully reproducible
/// given the same seed, which is the only requirement this crate's Monte
/// Carlo engine actually needs (spec section 51).
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform `f64` in `[0, 1)`.
    fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        if hi <= lo {
            return lo;
        }
        lo + (hi - lo) * self.next_unit()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointGeometry {
    pub thread: ThreadGeometry,
    /// Nut-side bearing (nut/washer) annular contact - inner radius
    /// (clearance hole radius) and outer radius (bearing face/washer OD / 2).
    /// Always used for clamped-member contact pressure/stress regardless of
    /// `JointInputs.tightening_from` (spec section 48 only distinguishes
    /// which face's friction drives the torque-to-preload solve, not which
    /// face bears on the clamped stack).
    pub bearing_inner_radius: f64,
    pub bearing_outer_radius: f64,
    /// Head-side bearing annular contact, independently sized from the
    /// nut-side pair above (spec section 48: "allow separate head bearing
    /// geometry / nut bearing geometry"). `None` assumes the head-side
    /// geometry is the same as the nut-side pair - a documented default,
    /// not a silent one, for the common case (same washer/bearing-face size
    /// both ends) where the distinction doesn't matter.
    pub head_bearing_inner_radius: Option<f64>,
    pub head_bearing_outer_radius: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrictionInputs {
    pub mu_thread: f64,
    pub mu_bearing: f64,
    pub bearing_model: BearingPressureModel,
    /// Constant prevailing torque (self-locking nut/insert), independent
    /// of preload (spec section 49) - `0.0` for a plain nut.
    pub prevailing_torque: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JointInputs {
    pub geometry: JointGeometry,
    pub friction: FrictionInputs,
    pub tightening_from: TighteningMember,
    /// Physical shank/free-thread segments only - head and thread-
    /// engagement additional elastic length are appended internally via
    /// [`build_fastener_segments`].
    pub fastener_segments: Vec<FastenerSegment>,
    pub fastener_e: f64,
    pub fastener_nu: f64,
    pub member_stack: MemberStack,
    /// Pressure-cone half-angle - a common assumption is 30 deg (spec
    /// section 18 leaves it "configurable or selected from an applicable
    /// engineering model").
    pub cone_half_angle_deg: f64,
    pub mode: AnalysisMode,
    /// Optional external service axial load (spec section 39).
    pub external_axial_load: Option<f64>,
    /// One friction coefficient per clamped interface, for slip-capacity
    /// evaluation (spec section 42) - empty if not evaluated.
    pub friction_interfaces: Vec<f64>,
    pub applied_shear_load: Option<f64>,
    pub proof_load: Option<f64>,
    pub yield_strength: Option<f64>,
    pub ultimate_strength: Option<f64>,
    /// `None` skips both uncertainty engines entirely (the common case) -
    /// distinct from a zero-width `UncertaintyInputs`, which would run them
    /// and report a degenerate (zero-width) band.
    pub uncertainty: Option<UncertaintyInputs>,
    /// `None` skips the Monte Carlo engine even when `uncertainty` is
    /// `Some` (worst-case can run alone).
    pub monte_carlo: Option<MonteCarloSettings>,
    /// `None` skips the advanced per-thread spring-coupled load
    /// distribution entirely (spec section 36's own words: "this should be
    /// an advanced analysis option rather than mandatory for initial UI
    /// interaction").
    pub thread_load_distribution: Option<ThreadLoadDistributionInputs>,
    /// Optional settlement/embedment displacement (spec section 24:
    /// surface flattening, coating compression, washer seating, thread
    /// seating, interface roughness) - `None` means `0` (spec's own words:
    /// "if embedment is not enabled, delta_embed = 0" - never silently
    /// assumed otherwise). Only affects `AnalysisMode::RotationControlled`,
    /// where it reduces the preload a given tool rotation actually
    /// achieves (part of the imposed displacement is consumed by
    /// settlement rather than elastic stretch); reported in
    /// `DeformationState::embedment_settlement`/`total_closure` regardless
    /// of mode for consistency with spec section 24's own
    /// `delta_required = delta_b + delta_m + delta_embed` identity.
    pub embedment_settlement: Option<f64>,
    /// Optional thread stripping/shear check (spec section 35) - both must
    /// be `Some` for `JointSolution.thread_shear_margin` to be computed
    /// (the length to derive an area, the strength to derive a margin).
    pub thread_engagement_length: Option<f64>,
    pub thread_shear_strength: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TorqueBreakdown {
    pub applied: f64,
    pub thread: f64,
    pub bearing: f64,
    pub prevailing: f64,
    /// `applied - (thread + bearing + prevailing)` - the equilibrium
    /// residual (spec section 55/77). Must be within the solver's
    /// tolerance for `TorqueControlled` mode; for the other two modes this
    /// is `applied - required` where `applied` is defined as the required
    /// torque itself, so the residual is exactly zero by construction.
    pub residual: f64,
}

impl TorqueBreakdown {
    pub fn required(&self) -> f64 {
        self.thread + self.bearing + self.prevailing
    }

    pub fn thread_fraction(&self) -> f64 {
        fraction(self.thread, self.required())
    }

    pub fn bearing_fraction(&self) -> f64 {
        fraction(self.bearing, self.required())
    }

    pub fn prevailing_fraction(&self) -> f64 {
        fraction(self.prevailing, self.required())
    }
}

fn fraction(part: f64, whole: f64) -> f64 {
    if whole.abs() < 1e-15 {
        0.0
    } else {
        part / whole
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComplianceState {
    pub c_b: f64,
    pub c_m: f64,
    pub k_b: f64,
    pub k_m: f64,
    pub load_fraction: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeformationState {
    pub fastener_elongation: f64,
    pub member_compression: f64,
    /// The settlement/embedment displacement folded into `total_closure`
    /// below (spec section 24) - `0.0` when `JointInputs.embedment_settlement`
    /// is `None`.
    pub embedment_settlement: f64,
    pub total_closure: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotationState {
    pub revolutions: f64,
    pub degrees: f64,
    /// Fastener torsional twist over its own body during tightening,
    /// estimated from the thread-torque component transmitted along the
    /// shank (spec section 27 - bearing friction torque is reacted
    /// directly at the head/nut face and does not further twist the
    /// shank, a standard simplifying assumption stated explicitly here
    /// rather than folded silently into the total).
    pub torsional_twist_degrees: f64,
    /// `degrees + torsional_twist_degrees` - the tool must rotate further
    /// than the nut-to-fastener relative rotation alone by the amount the
    /// fastener itself twists up (spec section 27).
    pub estimated_tool_rotation_degrees: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnergyState {
    pub fastener_strain_energy: f64,
    pub member_strain_energy: f64,
    pub tightening_work: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StressSection {
    pub axial: f64,
    pub torsional_shear: f64,
    pub von_mises: f64,
    pub principal: PrincipalStresses,
    pub yield_margin: f64,
    pub ultimate_margin: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StressSummary {
    pub shank: StressSection,
    pub tensile_stress_area: StressSection,
    pub thread_root: StressSection,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SolverStatus {
    Converged,
    /// Torque-controlled mode only: the prevailing torque alone already
    /// meets or exceeds the applied torque, so no positive preload solves
    /// the equilibrium - reported as zero preload rather than a spurious
    /// negative root.
    PrevailingTorqueExceedsApplied,
    NotBracketed,
    MaxIterationsExceeded,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JointSolution {
    pub status: SolverStatus,
    pub preload: f64,
    pub torque: TorqueBreakdown,
    pub compliance: ComplianceState,
    pub deformation: DeformationState,
    pub rotation: RotationState,
    pub energy: EnergyState,
    pub stress: StressSummary,
    pub bearing_contact_pressure: f64,
    pub member_average_compressive_stress: f64,
    pub service_load: Option<ServiceLoadState>,
    pub slip_capacity: Option<f64>,
    pub slip_margin: Option<f64>,
    pub warnings: Vec<String>,
    pub uncertainty: Option<UncertaintyResult>,
    pub monte_carlo: Option<MonteCarloResult>,
    pub thread_load_distribution: Option<ThreadLoadDistribution>,
    /// `None` unless both `JointInputs.thread_engagement_length` and
    /// `JointInputs.thread_shear_strength` are supplied (spec section 35).
    pub thread_shear_margin: Option<f64>,
}

/// The bearing (inner, outer) radii of whichever component is actually
/// rotated during tightening (spec section 48) - `Nut` uses the nut-side
/// pair every other quantity in this module already uses; `BoltHead` uses
/// the head-side pair, falling back to the nut-side pair when no distinct
/// head geometry was supplied (see `JointGeometry`'s own doc comment).
fn active_bearing_radii(inputs: &JointInputs) -> (f64, f64) {
    match inputs.tightening_from {
        TighteningMember::Nut => (inputs.geometry.bearing_inner_radius, inputs.geometry.bearing_outer_radius),
        TighteningMember::BoltHead => (
            inputs.geometry.head_bearing_inner_radius.unwrap_or(inputs.geometry.bearing_inner_radius),
            inputs.geometry.head_bearing_outer_radius.unwrap_or(inputs.geometry.bearing_outer_radius),
        ),
    }
}

fn torque_residual(preload: f64, inputs: &JointInputs) -> f64 {
    let (r_i, r_o) = active_bearing_radii(inputs);
    let t_thread = thread_torque(preload, &inputs.geometry.thread, inputs.friction.mu_thread);
    let t_bearing = bearing_torque(preload, inputs.friction.mu_bearing, r_i, r_o, inputs.friction.bearing_model);
    t_thread + t_bearing + inputs.friction.prevailing_torque
}

fn torque_breakdown_at(preload: f64, applied: f64, inputs: &JointInputs) -> TorqueBreakdown {
    let (r_i, r_o) = active_bearing_radii(inputs);
    let thread = thread_torque(preload, &inputs.geometry.thread, inputs.friction.mu_thread);
    let bearing = bearing_torque(preload, inputs.friction.mu_bearing, r_i, r_o, inputs.friction.bearing_model);
    let prevailing = inputs.friction.prevailing_torque;
    TorqueBreakdown { applied, thread, bearing, prevailing, residual: applied - (thread + bearing + prevailing) }
}

/// Validates every structural input before any solve is attempted (spec
/// section 69) - the caller should surface `Err` directly rather than
/// attempting to interpret a garbage `JointSolution`.
pub fn validate(inputs: &JointInputs) -> Result<(), ValidationError> {
    validation::validate_thread(&inputs.geometry.thread)?;
    validation::validate_material(inputs.fastener_e, inputs.fastener_nu)?;
    validation::validate_friction(inputs.friction.mu_thread)?;
    validation::validate_friction(inputs.friction.mu_bearing)?;
    validation::validate_contact_annulus(inputs.geometry.bearing_inner_radius, inputs.geometry.bearing_outer_radius)?;
    validation::validate_fastener_segments(&inputs.fastener_segments)?;
    validation::validate_member_stack(&inputs.member_stack)?;
    Ok(())
}

fn fastener_g(e: f64, nu: f64) -> f64 {
    e / (2.0 * (1.0 + nu))
}

/// Solves the torque-controlled preload for an arbitrary
/// (`mu_thread`, `mu_bearing`, pitch diameter, bearing outer radius,
/// applied torque) corner - the single root-solve implementation both the
/// nominal `compute()` path and the uncertainty engines
/// ([`evaluate_worst_case`]/[`monte_carlo`]) call, so a perturbed corner can
/// never silently diverge from the nominal solve's own equilibrium logic.
/// Returns `f64::NAN` (never a panic) when the corner has no positive root
/// (prevailing torque alone meets/exceeds the applied torque) or Brent
/// fails to bracket/converge - callers that aggregate many corners
/// (`worst_case_corners`, Monte Carlo) already skip non-finite results.
fn solve_preload_for_corner(inputs: &JointInputs, mu_thread: f64, mu_bearing: f64, pitch_diameter: f64, bearing_inner_radius: f64, bearing_outer_radius: f64, applied_torque: f64) -> f64 {
    if inputs.friction.prevailing_torque >= applied_torque {
        return 0.0;
    }
    let mut geometry = inputs.geometry.thread;
    geometry.d2 = pitch_diameter;
    let residual = |f: f64| {
        let t_thread = thread_torque(f, &geometry, mu_thread);
        let t_bearing = bearing_torque(f, mu_bearing, bearing_inner_radius, bearing_outer_radius, inputs.friction.bearing_model);
        t_thread + t_bearing + inputs.friction.prevailing_torque - applied_torque
    };
    let mut hi = 1.0_f64.max(applied_torque);
    let mut tries = 0;
    while residual(hi) < 0.0 && tries < 200 {
        hi *= 2.0;
        tries += 1;
    }
    match brent(0.0, hi, residual, 1e-9 * hi.max(1.0), 200) {
        Ok(sol) => sol.x,
        Err(_) => f64::NAN,
    }
}

fn uncertainty_bounds(inputs: &JointInputs, u: &UncertaintyInputs, applied_torque: f64) -> [Bound; 5] {
    let (_, active_outer) = active_bearing_radii(inputs);
    [
        Bound { min: inputs.friction.mu_thread * (1.0 - u.mu_thread_tol_fraction), max: inputs.friction.mu_thread * (1.0 + u.mu_thread_tol_fraction) },
        Bound { min: inputs.friction.mu_bearing * (1.0 - u.mu_bearing_tol_fraction), max: inputs.friction.mu_bearing * (1.0 + u.mu_bearing_tol_fraction) },
        Bound { min: inputs.geometry.thread.d2 - u.pitch_diameter_tol, max: inputs.geometry.thread.d2 + u.pitch_diameter_tol },
        Bound { min: active_outer - u.bearing_outer_radius_tol, max: active_outer + u.bearing_outer_radius_tol },
        Bound { min: applied_torque * (1.0 - u.applied_torque_tol_fraction), max: applied_torque * (1.0 + u.applied_torque_tol_fraction) },
    ]
}

/// Deterministic exhaustive worst-case evaluation (spec section 13) across
/// the five independent bounded variables in `u` - `2^5 = 32` corners,
/// inexpensive enough to always run in full rather than sampling. The
/// bearing inner radius is held at its active (nut- or head-side, per
/// `tightening_from`) value throughout - only the outer radius is
/// toleranced (spec section 13's own five-variable list).
fn evaluate_worst_case(inputs: &JointInputs, u: &UncertaintyInputs, applied_torque: f64, nominal_preload: f64) -> UncertaintyResult {
    let (active_inner, _) = active_bearing_radii(inputs);
    let bounds = uncertainty_bounds(inputs, u, applied_torque);
    let result = worst_case_corners(&bounds, |p| solve_preload_for_corner(inputs, p[0], p[1], p[2], active_inner, p[3], p[4]));
    UncertaintyResult { preload_min: result.min, preload_nominal: nominal_preload, preload_max: result.max }
}

/// Monte Carlo sampling (spec section 51) - uniform sampling within the
/// same five bounds [`evaluate_worst_case`] uses, see [`MonteCarloSettings`]'s
/// own doc comment for why one set of tolerance inputs powers both engines.
fn monte_carlo(inputs: &JointInputs, u: &UncertaintyInputs, settings: &MonteCarloSettings, applied_torque: f64) -> Option<MonteCarloResult> {
    let (active_inner, _) = active_bearing_radii(inputs);
    let bounds = uncertainty_bounds(inputs, u, applied_torque);
    let mut rng = SplitMix64(settings.seed);
    let (mut sum, mut sum_sq, mut count) = (0.0_f64, 0.0_f64, 0u32);
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for _ in 0..settings.samples.max(1) {
        let mu_thread = rng.uniform(bounds[0].min, bounds[0].max);
        let mu_bearing = rng.uniform(bounds[1].min, bounds[1].max);
        let d2 = rng.uniform(bounds[2].min, bounds[2].max);
        let r_o = rng.uniform(bounds[3].min, bounds[3].max);
        let torque = rng.uniform(bounds[4].min, bounds[4].max);
        let f = solve_preload_for_corner(inputs, mu_thread, mu_bearing, d2, active_inner, r_o, torque);
        if f.is_finite() {
            sum += f;
            sum_sq += f * f;
            count += 1;
            min = min.min(f);
            max = max.max(f);
        }
    }
    if count == 0 {
        return None;
    }
    let mean = sum / count as f64;
    let variance = (sum_sq / count as f64 - mean * mean).max(0.0);
    Some(MonteCarloResult { samples: count, seed: settings.seed, preload_mean: mean, preload_std_dev: variance.sqrt(), preload_min: min, preload_max: max })
}

fn compliance_state(inputs: &JointInputs) -> ComplianceState {
    let full_segments = build_fastener_segments(&inputs.fastener_segments, &inputs.geometry.thread);
    let c_b = fastener_axial_compliance(&full_segments, inputs.fastener_e);
    let contact_diameter = 2.0 * inputs.geometry.bearing_outer_radius;
    let c_m = member_stack_compliance(&inputs.member_stack, contact_diameter, contact_diameter, inputs.cone_half_angle_deg);
    let k_b = if c_b > 0.0 { 1.0 / c_b } else { 0.0 };
    let k_m = if c_m > 0.0 { 1.0 / c_m } else { 0.0 };
    ComplianceState { c_b, c_m, k_b, k_m, load_fraction: joint_load_fraction(k_b, k_m) }
}

/// Solves `preload` from `inputs.mode`, then builds the full `JointSolution`
/// from that converged (or otherwise resolved) preload - the single entry
/// point every UI/toolbox call site should use (spec section 68's "prefer
/// a single immutable solved result object").
pub fn compute(inputs: &JointInputs) -> Result<JointSolution, ValidationError> {
    validate(inputs)?;

    let compliance = compliance_state(inputs);
    let (status, preload, applied_torque) = match inputs.mode {
        AnalysisMode::TorqueControlled { applied_torque } => {
            if inputs.friction.prevailing_torque >= applied_torque {
                (SolverStatus::PrevailingTorqueExceedsApplied, 0.0, applied_torque)
            } else {
                // Re-run through the exact same bracket/Brent logic the
                // uncertainty engines use per corner (`solve_preload_for_corner`,
                // called here at the nominal friction/geometry/torque
                // values) - one implementation, never two that could drift.
                let (active_inner, active_outer) = active_bearing_radii(inputs);
                let nominal = solve_preload_for_corner(
                    inputs,
                    inputs.friction.mu_thread,
                    inputs.friction.mu_bearing,
                    inputs.geometry.thread.d2,
                    active_inner,
                    active_outer,
                    applied_torque,
                );
                if nominal.is_finite() {
                    (SolverStatus::Converged, nominal, applied_torque)
                } else {
                    // `solve_preload_for_corner` collapses every failure mode
                    // to NaN for the uncertainty engines' benefit; the
                    // nominal path re-derives which one it actually was so
                    // `SolverStatus` still distinguishes them for the UI.
                    let residual = |f: f64| torque_residual(f, inputs) - applied_torque;
                    let mut hi = 1.0_f64.max(applied_torque);
                    let mut tries = 0;
                    while residual(hi) < 0.0 && tries < 200 {
                        hi *= 2.0;
                        tries += 1;
                    }
                    match brent(0.0, hi, residual, 1e-9 * hi.max(1.0), 200) {
                        Ok(sol) => (SolverStatus::Converged, sol.x, applied_torque),
                        Err(RootError::NotBracketed) => (SolverStatus::NotBracketed, 0.0, applied_torque),
                        Err(RootError::MaxIterationsExceeded { best_estimate, .. }) => (SolverStatus::MaxIterationsExceeded, best_estimate, applied_torque),
                    }
                }
            }
        }
        AnalysisMode::PreloadControlled { target_preload } => {
            let required = torque_residual(target_preload, inputs);
            (SolverStatus::Converged, target_preload, required)
        }
        AnalysisMode::RotationControlled { turns_after_snug, snug_preload } => {
            let lead = inputs.geometry.thread.lead();
            let delta_required = turns_after_snug * lead;
            let total_compliance = compliance.c_b + compliance.c_m;
            // Spec section 24: part of the imposed rotation's displacement
            // is consumed by settlement rather than elastic stretch, so it
            // reduces (never increases) the preload a given rotation
            // actually achieves - clamped at 0 so settlement larger than
            // the imposed displacement itself can't drive a negative
            // "elastic" delta.
            let embed = inputs.embedment_settlement.unwrap_or(0.0);
            let delta_from_snug = if total_compliance > 0.0 { (delta_required - embed).max(0.0) } else { 0.0 };
            let preload = snug_preload + if total_compliance > 0.0 { delta_from_snug / total_compliance } else { 0.0 };
            let required_torque = torque_residual(preload, inputs);
            (SolverStatus::Converged, preload, required_torque)
        }
    };

    let torque = torque_breakdown_at(preload, applied_torque, inputs);

    let full_segments = build_fastener_segments(&inputs.fastener_segments, &inputs.geometry.thread);
    let fastener_elongation = preload * compliance.c_b;
    let member_compression = preload * compliance.c_m;
    let embedment_settlement = inputs.embedment_settlement.unwrap_or(0.0);
    let deformation =
        DeformationState { fastener_elongation, member_compression, embedment_settlement, total_closure: fastener_elongation + member_compression + embedment_settlement };

    let lead = inputs.geometry.thread.lead();
    let revolutions = if lead > 0.0 { deformation.total_closure / lead } else { 0.0 };
    let degrees = revolutions * 360.0;
    let g = fastener_g(inputs.fastener_e, inputs.fastener_nu);
    let torsional_compliance = fastener_torsional_compliance(&full_segments, g);
    let torsional_twist_rad = torque.thread * torsional_compliance;
    let torsional_twist_degrees = torsional_twist_rad.to_degrees();
    let rotation = RotationState { revolutions, degrees, torsional_twist_degrees, estimated_tool_rotation_degrees: degrees + torsional_twist_degrees };

    let fastener_strain_energy = 0.5 * preload * deformation.fastener_elongation;
    let member_strain_energy = 0.5 * preload * deformation.member_compression;
    let theta_final_rad = rotation.degrees.to_radians();
    let t_prevailing = inputs.friction.prevailing_torque;
    let t_final = torque.required();
    let tightening_work = if theta_final_rad > 0.0 {
        composite_simpson(0.0, theta_final_rad, 50, |theta| {
            let frac = theta / theta_final_rad;
            t_prevailing + (t_final - t_prevailing) * frac
        })
    } else {
        0.0
    };
    let energy = EnergyState { fastener_strain_energy, member_strain_energy, tightening_work };

    let thread_g = &inputs.geometry.thread;
    let stress_at = |area: f64, radius: f64, polar_moment: f64| -> StressSection {
        let state = StressState { axial: axial_stress(preload, area), torsional_shear: torsional_shear_stress(torque.thread, radius, polar_moment) };
        let vm = von_mises_stress(state);
        let principal = principal_stresses(state);
        let yield_margin = inputs.yield_strength.map_or(f64::INFINITY, |sy| if vm > 0.0 { sy / vm - 1.0 } else { f64::INFINITY });
        let ultimate_margin = inputs.ultimate_strength.map_or(f64::INFINITY, |su| if vm > 0.0 { su / vm - 1.0 } else { f64::INFINITY });
        StressSection { axial: state.axial, torsional_shear: state.torsional_shear, von_mises: vm, principal, yield_margin, ultimate_margin }
    };
    let shank_diameter = inputs.fastener_segments.first().map(|s| s.diameter).unwrap_or(thread_g.d);
    let shank_area = std::f64::consts::FRAC_PI_4 * shank_diameter * shank_diameter;
    let shank_polar_moment = std::f64::consts::PI * shank_diameter.powi(4) / 32.0;
    let stress = StressSummary {
        shank: stress_at(shank_area, shank_diameter / 2.0, shank_polar_moment),
        tensile_stress_area: stress_at(thread_g.tensile_stress_area(), thread_g.d2 / 2.0, thread_g.root_polar_moment()),
        thread_root: stress_at(thread_g.root_area(), thread_g.d3 / 2.0, thread_g.root_polar_moment()),
    };

    let bearing_contact_pressure = average_bearing_pressure(preload, inputs.geometry.bearing_inner_radius, inputs.geometry.bearing_outer_radius);
    let contact_diameter = 2.0 * inputs.geometry.bearing_outer_radius;
    let member_average_compressive_stress = if let Some(first) = inputs.member_stack.members.first() {
        let area = std::f64::consts::FRAC_PI_4 * (contact_diameter * contact_diameter - first.hole_diameter * first.hole_diameter);
        axial_stress(preload, area)
    } else {
        0.0
    };

    let service_load = inputs.external_axial_load.map(|p| apply_external_load(preload, compliance.load_fraction, p));
    let slip_capacity_value = if inputs.friction_interfaces.is_empty() { None } else { Some(slip_capacity(&inputs.friction_interfaces.iter().map(|mu| (*mu, preload)).collect::<Vec<_>>())) };
    let slip_margin_value = match (slip_capacity_value, inputs.applied_shear_load) {
        (Some(capacity), Some(shear)) => Some(slip_margin(capacity, shear)),
        _ => None,
    };

    let separation_margin = service_load.map(|s| s.separation_margin).unwrap_or(f64::INFINITY);
    let warnings = validation::warnings_for(preload, inputs.proof_load, stress.tensile_stress_area.von_mises, inputs.yield_strength, separation_margin, slip_margin_value.unwrap_or(f64::INFINITY));

    // Both uncertainty engines only apply to a torque-controlled solve that
    // actually converged - the other two modes take preload as a direct
    // input with nothing upstream of it to propagate tolerance through, and
    // a corner evaluation from an already-failed nominal solve would be
    // meaningless.
    let torque_controlled_applied = matches!(inputs.mode, AnalysisMode::TorqueControlled { .. }).then_some(applied_torque);
    let uncertainty = match (inputs.uncertainty, torque_controlled_applied, status) {
        (Some(u), Some(at), SolverStatus::Converged) => Some(evaluate_worst_case(inputs, &u, at, preload)),
        _ => None,
    };
    let monte_carlo_result = match (inputs.uncertainty, inputs.monte_carlo, torque_controlled_applied, status) {
        (Some(u), Some(mc), Some(at), SolverStatus::Converged) => monte_carlo(inputs, &u, &mc, at),
        _ => None,
    };

    // Unlike the uncertainty engines (torque-controlled only), thread load
    // distribution is meaningful for any mode that produced a preload
    // value - it just redistributes that preload across the engaged
    // threads, so it is gated only on the input being present.
    let thread_load_distribution_result =
        inputs.thread_load_distribution.as_ref().and_then(|tld| thread_load_distribution::compute(preload, &inputs.geometry.thread, inputs.fastener_e, inputs.fastener_nu, tld));

    let thread_shear_margin_result = match (inputs.thread_engagement_length, inputs.thread_shear_strength) {
        (Some(engagement_length), Some(strength)) => {
            let area = thread_shear_area(inputs.geometry.thread.d2, engagement_length);
            let stress = thread_shear_stress(preload, area);
            Some(shear_margin(Some(strength), stress))
        }
        _ => None,
    };

    Ok(JointSolution {
        status,
        preload,
        torque,
        compliance,
        deformation,
        rotation,
        energy,
        stress,
        bearing_contact_pressure,
        member_average_compressive_stress,
        service_load,
        slip_capacity: slip_capacity_value,
        slip_margin: slip_margin_value,
        warnings,
        uncertainty,
        monte_carlo: monte_carlo_result,
        thread_load_distribution: thread_load_distribution_result,
        thread_shear_margin: thread_shear_margin_result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compliance::Member;

    fn base_inputs(mode: AnalysisMode) -> JointInputs {
        let thread = ThreadGeometry { d: 10.0, d2: 9.026, d3: 8.160, pitch: 1.5, starts: 1, thread_angle_deg: 60.0 };
        JointInputs {
            geometry: JointGeometry { thread, bearing_inner_radius: 5.5, bearing_outer_radius: 9.5, head_bearing_inner_radius: None, head_bearing_outer_radius: None },
            friction: FrictionInputs { mu_thread: 0.15, mu_bearing: 0.15, bearing_model: BearingPressureModel::UniformPressure, prevailing_torque: 0.0 },
            tightening_from: TighteningMember::Nut,
            fastener_segments: vec![FastenerSegment { length: 25.0, diameter: 10.0 }],
            fastener_e: 200_000.0,
            fastener_nu: 0.29,
            member_stack: MemberStack {
                members: vec![Member { thickness: 20.0, e: 70_000.0, hole_diameter: 11.0, outer_diameter: None }],
            },
            cone_half_angle_deg: 30.0,
            mode,
            external_axial_load: None,
            friction_interfaces: Vec::new(),
            applied_shear_load: None,
            proof_load: None,
            yield_strength: None,
            ultimate_strength: None,
            uncertainty: None,
            monte_carlo: None,
            thread_load_distribution: None,
            embedment_settlement: None,
            thread_engagement_length: None,
            thread_shear_strength: None,
        }
    }

    #[test]
    fn tightening_from_bolt_head_uses_the_head_side_bearing_geometry_not_the_nut_side() {
        // Spec section 48: "Nut turned, bolt head held" and "Bolt head
        // turned, nut held" are not mechanically identical - a distinct
        // head-side bearing outer radius must change the solved preload
        // when tightening from the head, and must NOT change it when
        // tightening from the nut (same inputs, only `tightening_from`
        // differs).
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.geometry.head_bearing_inner_radius = Some(4.0);
        inputs.geometry.head_bearing_outer_radius = Some(20.0); // much larger bearing face than the nut side (9.5)

        inputs.tightening_from = TighteningMember::Nut;
        let nut_turned = compute(&inputs).unwrap();

        inputs.tightening_from = TighteningMember::BoltHead;
        let head_turned = compute(&inputs).unwrap();

        assert_eq!(nut_turned.status, SolverStatus::Converged);
        assert_eq!(head_turned.status, SolverStatus::Converged);
        assert!(
            (nut_turned.preload - head_turned.preload).abs() > 1.0,
            "distinct head-side bearing geometry must change the solve: nut-turned={} head-turned={}",
            nut_turned.preload,
            head_turned.preload
        );

        // Sanity: with no head-side geometry supplied (`None`), both sides
        // must solve identically (the documented "assume symmetric" default).
        inputs.geometry.head_bearing_inner_radius = None;
        inputs.geometry.head_bearing_outer_radius = None;
        inputs.tightening_from = TighteningMember::Nut;
        let symmetric_nut = compute(&inputs).unwrap();
        inputs.tightening_from = TighteningMember::BoltHead;
        let symmetric_head = compute(&inputs).unwrap();
        assert!((symmetric_nut.preload - symmetric_head.preload).abs() < 1e-6);
    }

    #[test]
    fn torque_controlled_mode_converges_and_satisfies_equilibrium() {
        let inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        let solution = compute(&inputs).unwrap();
        assert_eq!(solution.status, SolverStatus::Converged);
        assert!(solution.preload > 0.0);
        // Spec section 77: torque equilibrium must hold within tolerance.
        assert!(solution.torque.residual.abs() < 1.0, "residual {} too large", solution.torque.residual);
    }

    #[test]
    fn zero_friction_thread_and_bearing_yields_the_lead_angle_only_limit() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 10_000.0 });
        inputs.friction.mu_thread = 0.0;
        inputs.friction.mu_bearing = 0.0;
        let solution = compute(&inputs).unwrap();
        assert_eq!(solution.status, SolverStatus::Converged);
        assert!(solution.preload > 0.0);
        assert!(solution.torque.residual.abs() < 1.0);
    }

    #[test]
    fn torque_to_preload_to_torque_round_trip_recovers_the_original_torque() {
        // Spec section 74.
        let inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 35_000.0 });
        let forward = compute(&inputs).unwrap();
        assert_eq!(forward.status, SolverStatus::Converged);

        let mut backward_inputs = inputs.clone();
        backward_inputs.mode = AnalysisMode::PreloadControlled { target_preload: forward.preload };
        let backward = compute(&backward_inputs).unwrap();
        assert!((backward.torque.required() - 35_000.0).abs() / 35_000.0 < 1e-6, "recovered torque {} vs original 35000", backward.torque.required());
    }

    #[test]
    fn rotation_to_preload_to_rotation_round_trip_recovers_the_original_rotation() {
        // Spec section 75.
        let mut inputs = base_inputs(AnalysisMode::RotationControlled { turns_after_snug: 0.5, snug_preload: 0.0 });
        let forward = compute(&inputs).unwrap();

        inputs.mode = AnalysisMode::PreloadControlled { target_preload: forward.preload };
        let backward = compute(&inputs).unwrap();
        assert!((backward.rotation.revolutions - 0.5).abs() < 1e-6, "recovered revolutions {} vs original 0.5", backward.rotation.revolutions);
    }

    #[test]
    fn increasing_torque_produces_increasing_preload() {
        // Spec section 78: monotonicity.
        let mut last_preload = 0.0;
        for torque in [10_000.0, 20_000.0, 30_000.0, 40_000.0, 50_000.0] {
            let inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: torque });
            let solution = compute(&inputs).unwrap();
            assert!(solution.preload > last_preload, "preload must strictly increase with applied torque");
            last_preload = solution.preload;
        }
    }

    #[test]
    fn increasing_friction_decreases_achieved_preload_for_fixed_torque() {
        // Spec section 79.
        let mut low_mu = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        low_mu.friction.mu_thread = 0.10;
        low_mu.friction.mu_bearing = 0.10;
        let mut high_mu = low_mu.clone();
        high_mu.friction.mu_thread = 0.20;
        high_mu.friction.mu_bearing = 0.20;

        let low = compute(&low_mu).unwrap();
        let high = compute(&high_mu).unwrap();
        assert!(high.preload < low.preload, "higher friction must yield lower preload for the same applied torque");
    }

    #[test]
    fn torque_breakdown_fractions_sum_to_one() {
        let inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        let solution = compute(&inputs).unwrap();
        let sum = solution.torque.thread_fraction() + solution.torque.bearing_fraction() + solution.torque.prevailing_fraction();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn prevailing_torque_exceeding_applied_torque_yields_zero_preload_not_a_bad_root() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 10.0 });
        inputs.friction.prevailing_torque = 20.0;
        let solution = compute(&inputs).unwrap();
        assert_eq!(solution.status, SolverStatus::PrevailingTorqueExceedsApplied);
        assert_eq!(solution.preload, 0.0);
    }

    #[test]
    fn embedment_settlement_reduces_preload_achieved_by_rotation_controlled_mode() {
        let mut inputs = base_inputs(AnalysisMode::RotationControlled { turns_after_snug: 0.5, snug_preload: 0.0 });
        let without_settlement = compute(&inputs).unwrap();
        inputs.embedment_settlement = Some(0.1); // same length unit as `lead`
        let with_settlement = compute(&inputs).unwrap();
        assert!(with_settlement.preload < without_settlement.preload);
        assert!((with_settlement.deformation.embedment_settlement - 0.1).abs() < 1e-9);
        // Spec section 24 identity: delta_required = delta_b + delta_m + delta_embed.
        let lead = inputs.geometry.thread.lead();
        assert!((with_settlement.deformation.total_closure - 0.5 * lead).abs() < 1e-6);
    }

    #[test]
    fn embedment_settlement_larger_than_the_imposed_rotation_clamps_preload_at_the_snug_value() {
        let mut inputs = base_inputs(AnalysisMode::RotationControlled { turns_after_snug: 0.01, snug_preload: 500.0 });
        inputs.embedment_settlement = Some(1_000.0); // far exceeds the imposed displacement
        let solution = compute(&inputs).unwrap();
        assert!((solution.preload - 500.0).abs() < 1e-6, "preload should collapse to snug_preload, got {}", solution.preload);
    }

    #[test]
    fn embedment_settlement_is_zero_by_default() {
        let inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        let solution = compute(&inputs).unwrap();
        assert_eq!(solution.deformation.embedment_settlement, 0.0);
    }

    #[test]
    fn thread_shear_margin_is_none_unless_both_length_and_strength_are_supplied() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        assert!(compute(&inputs).unwrap().thread_shear_margin.is_none());
        inputs.thread_engagement_length = Some(10.0);
        assert!(compute(&inputs).unwrap().thread_shear_margin.is_none(), "length alone must not be enough");
        inputs.thread_engagement_length = None;
        inputs.thread_shear_strength = Some(200.0);
        assert!(compute(&inputs).unwrap().thread_shear_margin.is_none(), "strength alone must not be enough");
    }

    #[test]
    fn thread_shear_margin_reflects_the_preload_over_shear_area_ratio() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.thread_engagement_length = Some(15.0);
        inputs.thread_shear_strength = Some(200.0);
        let solution = compute(&inputs).unwrap();
        let margin = solution.thread_shear_margin.unwrap();
        let area = crate::thread_shear::thread_shear_area(inputs.geometry.thread.d2, 15.0);
        let expected_stress = solution.preload / area;
        let expected_margin = 200.0 / expected_stress - 1.0;
        assert!((margin - expected_margin).abs() < 1e-6);
    }

    #[test]
    fn thread_load_distribution_is_none_when_not_requested() {
        let inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        let solution = compute(&inputs).unwrap();
        assert!(solution.thread_load_distribution.is_none());
    }

    #[test]
    fn thread_load_distribution_wires_through_compute_and_balances_the_solved_preload() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.thread_load_distribution = Some(ThreadLoadDistributionInputs { engaged_threads: 8, nut_modulus: 200_000.0, nut_poisson_ratio: 0.3, nut_outer_diameter: 16.0 });
        let solution = compute(&inputs).unwrap();
        let tld = solution.thread_load_distribution.expect("requested thread load distribution must be computed");
        assert_eq!(tld.per_thread_loads.len(), 8);
        let sum: f64 = tld.per_thread_loads.iter().sum();
        assert!((sum - solution.preload).abs() < 1e-6, "thread loads must sum to the solved preload: sum={sum} preload={}", solution.preload);
        assert_eq!(tld.first_thread_load, tld.per_thread_loads[0]);
    }

    #[test]
    fn thread_load_distribution_also_applies_to_preload_controlled_mode() {
        // Unlike the uncertainty engines, this is meaningful for any mode
        // that produces a preload value, not just torque-controlled.
        let mut inputs = base_inputs(AnalysisMode::PreloadControlled { target_preload: 12_000.0 });
        inputs.thread_load_distribution = Some(ThreadLoadDistributionInputs { engaged_threads: 5, nut_modulus: 200_000.0, nut_poisson_ratio: 0.3, nut_outer_diameter: 16.0 });
        let solution = compute(&inputs).unwrap();
        let tld = solution.thread_load_distribution.expect("must compute for PreloadControlled mode too");
        let sum: f64 = tld.per_thread_loads.iter().sum();
        assert!((sum - 12_000.0).abs() < 1e-6);
    }

    #[test]
    fn external_load_below_separation_reduces_member_compression() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.external_axial_load = Some(1000.0);
        let solution = compute(&inputs).unwrap();
        let service = solution.service_load.unwrap();
        assert!(service.member_load < solution.preload);
        assert!(service.bolt_load > solution.preload);
    }

    #[test]
    fn slip_capacity_and_margin_are_reported_when_interfaces_and_shear_are_given() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.friction_interfaces = vec![0.3];
        inputs.applied_shear_load = Some(500.0);
        let solution = compute(&inputs).unwrap();
        assert!(solution.slip_capacity.is_some());
        assert!(solution.slip_margin.is_some());
    }

    #[test]
    fn invalid_thread_geometry_is_rejected_before_solving() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.geometry.thread.d3 = 20.0; // root > major, nonsensical
        assert!(compute(&inputs).is_err());
    }

    #[test]
    fn worst_case_uncertainty_brackets_the_nominal_preload() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.uncertainty = Some(UncertaintyInputs { mu_thread_tol_fraction: 0.2, mu_bearing_tol_fraction: 0.2, pitch_diameter_tol: 0.05, bearing_outer_radius_tol: 0.1, applied_torque_tol_fraction: 0.1 });
        let solution = compute(&inputs).unwrap();
        let u = solution.uncertainty.expect("uncertainty must be evaluated for a converged torque-controlled solve");
        assert!(u.preload_min < u.preload_nominal, "min {} vs nominal {}", u.preload_min, u.preload_nominal);
        assert!(u.preload_max > u.preload_nominal, "max {} vs nominal {}", u.preload_max, u.preload_nominal);
        assert!((u.preload_nominal - solution.preload).abs() < 1e-9);
    }

    #[test]
    fn uncertainty_is_not_evaluated_for_preload_controlled_mode() {
        let mut inputs = base_inputs(AnalysisMode::PreloadControlled { target_preload: 20_000.0 });
        inputs.uncertainty = Some(UncertaintyInputs { mu_thread_tol_fraction: 0.2, mu_bearing_tol_fraction: 0.2, pitch_diameter_tol: 0.05, bearing_outer_radius_tol: 0.1, applied_torque_tol_fraction: 0.1 });
        let solution = compute(&inputs).unwrap();
        assert!(solution.uncertainty.is_none(), "preload is a direct input in this mode - nothing upstream to propagate tolerance through");
    }

    #[test]
    fn monte_carlo_mean_lands_near_the_nominal_preload_and_within_the_worst_case_band() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.uncertainty = Some(UncertaintyInputs { mu_thread_tol_fraction: 0.15, mu_bearing_tol_fraction: 0.15, pitch_diameter_tol: 0.02, bearing_outer_radius_tol: 0.05, applied_torque_tol_fraction: 0.05 });
        inputs.monte_carlo = Some(MonteCarloSettings { samples: 2000, seed: 42 });
        let solution = compute(&inputs).unwrap();
        let mc = solution.monte_carlo.expect("monte carlo must run alongside worst-case when both are configured");
        let u = solution.uncertainty.unwrap();
        assert_eq!(mc.seed, 42);
        assert!(mc.samples > 1900, "most samples should converge for a well-behaved corner space");
        assert!(mc.preload_mean > u.preload_min && mc.preload_mean < u.preload_max);
        assert!((mc.preload_mean - solution.preload).abs() / solution.preload < 0.05, "mean {} should land close to nominal {}", mc.preload_mean, solution.preload);
        assert!(mc.preload_std_dev > 0.0);
    }

    #[test]
    fn monte_carlo_is_deterministic_given_the_same_seed() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.uncertainty = Some(UncertaintyInputs { mu_thread_tol_fraction: 0.2, mu_bearing_tol_fraction: 0.2, pitch_diameter_tol: 0.05, bearing_outer_radius_tol: 0.1, applied_torque_tol_fraction: 0.1 });
        inputs.monte_carlo = Some(MonteCarloSettings { samples: 500, seed: 7 });
        let a = compute(&inputs).unwrap().monte_carlo.unwrap();
        let b = compute(&inputs).unwrap().monte_carlo.unwrap();
        assert_eq!(a, b, "the same seed must reproduce byte-identical statistics");
    }

    #[test]
    fn monte_carlo_does_not_run_without_uncertainty_bounds_even_if_settings_are_present() {
        let mut inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        inputs.monte_carlo = Some(MonteCarloSettings { samples: 100, seed: 1 });
        let solution = compute(&inputs).unwrap();
        assert!(solution.monte_carlo.is_none());
    }

    #[test]
    fn energy_balance_is_approximately_consistent() {
        // Spec section 76 - within the numerical-integration/model
        // tolerance, not exact equality (the tightening-work model is a
        // ramp approximation, not a full frictional-dissipation solve).
        let inputs = base_inputs(AnalysisMode::TorqueControlled { applied_torque: 40_000.0 });
        let solution = compute(&inputs).unwrap();
        let elastic = solution.energy.fastener_strain_energy + solution.energy.member_strain_energy;
        assert!(solution.energy.tightening_work >= elastic * 0.5, "tightening work {} implausibly small next to elastic energy {}", solution.energy.tightening_work, elastic);
    }
}
