//! Straight-bushing interference-fit solve, ported line-for-line from
//! engineering.toolbox's `src/lib/core/bushing/solveEngine.ts`'s
//! `computeState`/`toOutput` - the straight-bushing-only subset (no
//! countersink/flange geometry, no duty/process/approval review layers -
//! see this crate's `Cargo.toml` comment and
//! `docs/bushing-workbench-status.md` in the main repo for the exact
//! scope decision). Every formula below is annotated with its TS source
//! line for anyone diffing against the original; `tests/differential.rs`
//! proves this port matches the real TS engine's output on a real input,
//! not just that it "looks right."

use crate::bearing::{calculate_universal_bearing, BearingSegment};
use crate::countersink::{cs_angle_tolerance_from_base, cs_depth_tolerance_from_base, cs_dia_tolerance_from_base, enumerate_countersink_corners, solve_countersink, CsCorner, CsMode};
use crate::geometry::{compute_minimum_bushing_wall, resolve_bushing_section_params, BushingSectionInput};
pub use crate::geometry::{BushingType, IdType};
use crate::materials::{get_material, Material};
use crate::tolerance::{
    build_od_tolerance, clamp, containment_violations, enforce_bore_band_for_target, make_range, resolve_tolerance, BoreCapability, EnforcementPolicy,
    ResolveToleranceInput, ToleranceMode, ToleranceRange, ToleranceStatus,
};

/// Absolute temperature (imperial, deg F) install-time thermal-assist
/// deltas are measured from - `referenceTemperature('imperial')` in
/// `solveEngine.ts` (the metric branch, `20`, isn't relevant to this
/// imperial-only port).
const ASSEMBLY_REFERENCE_TEMP_F: f64 = 70.0;

/// A result value shown the same way its driving inputs are entered -
/// nominal plus the range that the input tolerance bands actually
/// propagate into, not just a single point value. `min`/`max` come from
/// re-evaluating the pressure/stress/margin chain at the bore/interference
/// tolerance band's real achieved extremes
/// (`BushingOutput::achieved_interference_tol`'s `.lower`/`.upper`), not
/// from perturbing this result in isolation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangedValue {
    pub nominal: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum EndConstraint {
    #[default]
    Free,
    OneEnd,
    BothEnds,
}

/// Bushing inputs - imperial units only for v1 (in, psi/ksi, lbf, °F). A
/// metric front-end would convert to these units before calling
/// `compute`, the same boundary the TS source draws with its own `units`
/// field feeding unit-aware sub-calculations.
///
/// Every new (post-straight-bushing-v1) field defaults through
/// `..Default::default()` to the exact values that reproduce the
/// original straight-bushing-only behavior (`BushingType::Straight`,
/// `IdType::Straight`, `EnforcementPolicy { enabled: false, .. }`) - so
/// existing straight-bushing callers/tests don't need to change.
#[derive(Debug, Clone, Default)]
pub struct BushingInputs {
    /// Housing bore nominal diameter, in.
    pub bore_dia: f64,
    pub bore_tol_plus: f64,
    pub bore_tol_minus: f64,
    /// Bushing inner diameter, in.
    pub id_bushing: f64,
    /// Target nominal diametral interference, in.
    pub interference: f64,
    pub interference_tol_plus: f64,
    pub interference_tol_minus: f64,
    /// Housing length along the bushing axis, in.
    pub housing_len: f64,
    /// Available surrounding housing width, in.
    pub housing_width: f64,
    /// Edge distance from bore center to nearest free edge, in.
    pub edge_dist: f64,
    pub mat_housing: String,
    pub mat_bushing: String,
    /// Installation friction coefficient. `None` falls back to 0.15,
    /// matching the TS source's own fallback.
    pub friction: Option<f64>,
    /// Uniform temperature change from the reference (install)
    /// temperature to the current state, °F.
    pub d_t: f64,
    pub end_constraint: EndConstraint,
    /// Minimum acceptable straight-wall thickness, in.
    pub min_wall_straight: f64,
    /// Edge load angle, degrees. `None` falls back to 40.
    pub edge_load_angle_deg: Option<f64>,
    /// Applied edge load, lbf. `None` falls back to 1000.
    pub load: Option<f64>,

    /// OD geometry: straight cylinder, flanged, or externally countersunk.
    pub bushing_type: BushingType,
    /// ID geometry: straight bore or internally countersunk.
    pub id_type: IdType,
    /// Flange outer diameter, in. Only meaningful when `bushing_type == Flanged`.
    pub flange_od: f64,
    /// Flange thickness, in. Only meaningful when `bushing_type == Flanged`.
    pub flange_thk: f64,
    /// Minimum acceptable neck-wall thickness (thinnest point once
    /// countersink/flange geometry is accounted for), in.
    pub min_wall_neck: f64,

    /// Internal (ID-side) countersink mode/geometry. Only meaningful when
    /// `id_type == Countersink`.
    pub cs_mode: CsMode,
    pub cs_dia: f64,
    pub cs_depth: f64,
    pub cs_angle: f64,
    pub cs_dia_tol_plus: f64,
    pub cs_dia_tol_minus: f64,
    pub cs_depth_tol_plus: f64,
    pub cs_depth_tol_minus: f64,
    /// Beyond the TS reference (which has no angle-tolerance input at
    /// all in any mode, confirmed by reading its schema) - only
    /// meaningful when `cs_mode` makes angle a direct input
    /// (`DepthAngle`/`DiaAngle`); ignored in `DiaDepth` mode, where
    /// angle is derived and gets a real propagated tolerance instead
    /// (`cs_angle_tolerance_from_base`).
    pub cs_angle_tol_plus: f64,
    pub cs_angle_tol_minus: f64,

    /// External (OD-side) countersink mode/geometry. Only meaningful when
    /// `bushing_type == Countersink`.
    pub ext_cs_mode: CsMode,
    pub ext_cs_dia: f64,
    pub ext_cs_depth: f64,
    pub ext_cs_angle: f64,
    pub ext_cs_dia_tol_plus: f64,
    pub ext_cs_dia_tol_minus: f64,
    pub ext_cs_depth_tol_plus: f64,
    pub ext_cs_depth_tol_minus: f64,
    pub ext_cs_angle_tol_plus: f64,
    pub ext_cs_angle_tol_minus: f64,

    /// Bore-tolerance auto-adjustment policy - disabled (`enabled: false`)
    /// reproduces the original v1 behavior of reporting `Infeasible`
    /// honestly rather than tightening the bore band.
    pub enforcement: EnforcementPolicy,
    /// Bore process-capability floor consulted by the enforcement policy.
    pub bore_capability: Option<BoreCapability>,

    /// Absolute part temperature (deg F) at the moment of installation -
    /// e.g. a bushing chilled with dry ice, or a housing heated, to
    /// temporarily shrink the interference for easier assembly (a real
    /// shrink-fit install technique, distinct from the in-service `d_t`
    /// temperature change above). `None` means "not thermally assisted at
    /// install" - `install_force`/`install_pressure` then fall back to
    /// the same in-service delta as `retained_install_force`/`pressure`,
    /// matching the TS source's own `hasAssemblyThermalAssist` fallback
    /// exactly (`solveEngine.ts:261-263`).
    pub assembly_housing_temperature: Option<f64>,
    pub assembly_bushing_temperature: Option<f64>,
}

/// One named margin-of-safety candidate - `governing` is whichever of
/// these has the lowest margin (TS source's own `governing` reduction).
#[derive(Debug, Clone, PartialEq)]
pub struct MarginCandidate {
    pub name: &'static str,
    pub margin: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BushingOutput {
    pub od_installed: f64,
    pub wall_straight: f64,
    pub fail_straight: bool,
    /// Minimum wall thickness once countersink/flange geometry is
    /// accounted for - equal to `wall_straight` for `BushingType::Straight`.
    pub wall_neck: f64,
    /// `wall_neck` at nominal countersink geometry, before the corner
    /// worst-case search - equal to `wall_neck` when `id_type != Countersink`.
    pub wall_neck_nominal: f64,
    pub fail_neck: bool,
    pub cs_solved_id: Option<CsCorner>,
    pub cs_solved_od: Option<CsCorner>,

    pub delta_thermal: f64,
    pub delta_user: f64,
    pub delta_total: f64,

    pub pressure: f64,
    pub effective_od_housing: f64,
    pub d_equivalent: f64,
    pub psi: f64,
    pub lambda: f64,
    pub term_b: f64,
    pub term_h: f64,

    pub stress_hoop_housing: f64,
    pub stress_hoop_bushing: f64,
    pub housing_ms: f64,
    pub bushing_ms: f64,

    pub axial_constraint_factor: f64,
    pub axial_length_factor: f64,
    /// `axial_constraint_factor * axial_length_factor * material_nu *
    /// stress_hoop_*` - the actual, visible effect of the `end_constraint`
    /// input (`EndConstraint::Free` always yields 0.0 here, matching
    /// "no axial constraint -> no axial stress estimate").
    pub stress_axial_housing: f64,
    pub stress_axial_bushing: f64,
    /// Full hoop/radial/axial stress *distribution* across each region's
    /// wall (`bore_tol.nominal/2` down to `id_bushing/2` for the bushing,
    /// `bore_tol.nominal/2` up to `effective_od_housing/2` for the
    /// housing) - not just the two boundary values above.
    pub bushing_stress_field: Vec<crate::lame::LameSample>,
    pub housing_stress_field: Vec<crate::lame::LameSample>,

    pub install_force: f64,
    pub retained_install_force: f64,

    pub ed_actual: f64,
    pub ed_min_sequence: f64,
    pub ed_min_strength: f64,
    pub sequence_margin: f64,
    pub strength_margin: f64,

    pub candidates: Vec<MarginCandidate>,
    pub governing: MarginCandidate,

    /// Result ranges mirroring the input tolerance format (nominal +
    /// min/max), evaluated at the achieved-interference band's real
    /// extremes (`achieved_interference_tol.lower`/`.upper`) rather than
    /// reported as bare point values - see `RangedValue`'s own doc
    /// comment for exactly what varies and what's held constant.
    pub wall_straight_range: RangedValue,
    pub pressure_range: RangedValue,
    pub stress_hoop_housing_range: RangedValue,
    pub stress_hoop_bushing_range: RangedValue,
    pub housing_ms_range: RangedValue,
    pub bushing_ms_range: RangedValue,
    pub install_force_range: RangedValue,
    pub retained_install_force_range: RangedValue,
    pub sequence_margin_range: RangedValue,

    pub bore_tol: ToleranceRange,
    pub interference_tol: ToleranceRange,
    pub od_tol: ToleranceRange,
    pub achieved_interference_tol: ToleranceRange,
    pub tolerance_status: ToleranceStatus,
    pub tolerance_notes: Vec<String>,
    pub enforcement_satisfied: bool,

    /// The countersink dimension that's DERIVED (not a direct user
    /// input) for the active `cs_mode`/`ext_cs_mode` gets a real,
    /// propagated tolerance range here - e.g. in `DepthAngle` mode,
    /// depth is the direct input and diameter is derived, so
    /// `cs_internal_dia_tol` is populated (`cs_dia_tolerance_from_base`)
    /// while `cs_internal_depth_tol` is just the user's own resolved
    /// depth-tolerance input passed through. `None` when the
    /// corresponding side isn't countersunk at all. Matches
    /// `toOutput`'s conditional `csInternalDia`/`csInternalDepth`/
    /// `csExternalDia`/`csExternalDepth` fields (`solveEngine.ts:799-802`).
    pub cs_internal_dia_tol: Option<ToleranceRange>,
    pub cs_internal_depth_tol: Option<ToleranceRange>,
    pub cs_external_dia_tol: Option<ToleranceRange>,
    pub cs_external_depth_tol: Option<ToleranceRange>,
    /// New scope (no TS reference equivalent): countersink angle tolerance,
    /// resolved from user input or derived when `CsMode::DiaDepth` makes
    /// angle the solved dimension. See `countersink::cs_angle_tolerance_from_base`.
    pub cs_internal_angle_tol: Option<ToleranceRange>,
    pub cs_external_angle_tol: Option<ToleranceRange>,

    /// Install-time delta/pressure/force, distinct from the in-service
    /// `delta_total`/`pressure`/`retained_install_force` above - reflects
    /// a shrink-fit assembly-temperature assist
    /// (`assembly_housing_temperature`/`assembly_bushing_temperature`)
    /// when either is set, otherwise falls back to the same in-service
    /// numbers (see `BushingInputs`'s own doc comment).
    pub assembly_thermal_delta: f64,
    pub install_delta: f64,
    pub install_pressure: f64,
}

/// Everything the pressure/stress/margin chain needs that does NOT vary
/// across the achieved-interference tolerance band - captured once at
/// nominal geometry so `delta_dependent_chain` only has to vary `delta`
/// itself. Held constant deliberately: the bore/OD's own tolerance is
/// typically two to three orders of magnitude smaller than the
/// dimensions themselves, so its effect on the *compliance* terms
/// (`term_b`/`term_h`/`effective_od_housing`) is negligible next to
/// varying the interference (`delta`) across its real achieved range,
/// which is the dominant driver of pressure/stress variation.
struct DeltaInvariants {
    term_b: f64,
    term_h: f64,
    bore_nominal: f64,
    id_bushing: f64,
    effective_od_housing: f64,
    friction: f64,
    housing_len: f64,
    fbru_base: f64,
    tau: f64,
    sin_theta: f64,
    ed_actual: f64,
    sy_housing_psi: f64,
    sy_bushing_psi: f64,
}

struct DeltaDependent {
    pressure: f64,
    stress_hoop_housing: f64,
    stress_hoop_bushing: f64,
    housing_ms: f64,
    bushing_ms: f64,
    install_force: f64,
    sequence_margin: f64,
}

/// Re-evaluates the delta-dependent half of `compute`'s physics chain
/// (`solveEngine.ts:414-448,481-486,619-624` - pressure through hoop
/// margins, install force, and edge-distance sequencing margin) at an
/// arbitrary interference value. Used at nominal `delta` (matching the
/// existing single-point behavior exactly - not a behavior change) and
/// again at the achieved-interference band's `.lower`/`.upper` to build
/// each `RangedValue` result. `ed_min_strength`/`strength_margin` are
/// deliberately not included: neither actually depends on `delta`
/// (`e_required_strength` depends only on `t_eff_seq`/`tau`/`sin_theta`/
/// `load`), so they're constant across the whole interference range and
/// the caller reuses the nominal value directly rather than recomputing
/// an identical number twice.
fn delta_dependent_chain(delta: f64, inv: &DeltaInvariants) -> DeltaDependent {
    let pressure = if delta > 0.0 { delta / (inv.term_b + inv.term_h) } else { 0.0 };
    // Full Lamé thick-wall hoop stress at the shared bore interface -
    // `crate::lame::lame_stress_at_radius` evaluated at r = interface,
    // same general function `compute`'s nominal-delta path and the
    // per-radius stress-field plot both use, never a separately
    // hand-rolled copy of the same closed-form expression.
    let (_, stress_hoop_housing) = crate::lame::lame_stress_at_radius(inv.bore_nominal / 2.0, inv.bore_nominal / 2.0, inv.effective_od_housing / 2.0, pressure, 0.0);
    let (_, stress_hoop_bushing) = crate::lame::lame_stress_at_radius(inv.bore_nominal / 2.0, inv.id_bushing / 2.0, inv.bore_nominal / 2.0, 0.0, pressure);
    let install_force = inv.friction * pressure * std::f64::consts::PI * inv.bore_nominal * inv.housing_len;
    let fbru_eff = inv.fbru_base * 1000.0 + 0.8 * pressure;
    let e_required_seq = if inv.tau > 0.0 { (inv.bore_nominal * fbru_eff) / (2.0 * inv.tau * inv.sin_theta) } else { f64::INFINITY };
    let ed_min_sequence = if e_required_seq > 0.0 { e_required_seq / inv.bore_nominal } else { f64::INFINITY };
    let sequence_margin = if ed_min_sequence.is_finite() && ed_min_sequence > 0.0 { inv.ed_actual / ed_min_sequence - 1.0 } else { f64::INFINITY };
    let housing_ms = if stress_hoop_housing != 0.0 { inv.sy_housing_psi / stress_hoop_housing - 1.0 } else { f64::INFINITY };
    let bushing_ms = if stress_hoop_bushing != 0.0 { inv.sy_bushing_psi / stress_hoop_bushing.abs() - 1.0 } else { f64::INFINITY };
    DeltaDependent { pressure, stress_hoop_housing, stress_hoop_bushing, housing_ms, bushing_ms, install_force, sequence_margin }
}

/// Builds a `RangedValue` from the min/max evaluations, ordering `min`/
/// `max` by actual value (not by which achieved-interference extreme
/// produced it) - housing hoop stress increases with interference so its
/// max stress comes from max interference, but housing MS/margins are
/// inversely related to stress, so their "max" (best-case) comes from
/// the *same* max-interference evaluation's stress being *lower* margin,
/// not higher. Sorting by value rather than by source avoids getting
/// that backwards per-field.
fn ranged(nominal: f64, a: f64, b: f64) -> RangedValue {
    RangedValue { nominal, min: a.min(b), max: a.max(b) }
}

fn end_constraint_factor(ec: EndConstraint) -> f64 {
    match ec {
        EndConstraint::Free => 0.0,
        EndConstraint::OneEnd => 0.5,
        EndConstraint::BothEnds => 1.0,
    }
}

/// See `solveEngine.ts` lines 183-590 (`computeState`) and 632-733+
/// (`toOutput`'s `hoop`/`edgeDistance`/`physics`/`governing` sections) for
/// the original this mirrors.
pub fn compute(input: &BushingInputs) -> BushingOutput {
    let mat_housing: &Material = get_material(&input.mat_housing);
    let mat_bushing: &Material = get_material(&input.mat_bushing);

    // solveEngine.ts:187-204 - bore/interference tolerance resolution
    // (nominal_tol mode only - v1 doesn't expose the `limits`-mode entry
    // path since there's no UI for it yet).
    let mut bore_tol = resolve_tolerance(ResolveToleranceInput {
        mode: ToleranceMode::NominalTol,
        nominal: input.bore_dia,
        plus: input.bore_tol_plus,
        minus: input.bore_tol_minus,
        lower: None,
        upper: None,
        min_floor: Some(1e-6),
    });
    let interference_tol = resolve_tolerance(ResolveToleranceInput {
        mode: ToleranceMode::NominalTol,
        nominal: input.interference,
        plus: input.interference_tol_plus,
        minus: input.interference_tol_minus,
        lower: None,
        upper: None,
        min_floor: None,
    });

    // solveEngine.ts:206,242-248 - OD tolerance solve + containment check.
    let mut od_fit = build_od_tolerance(bore_tol, interference_tol);
    // solveEngine.ts:207-241 - bore-capability/interference-policy
    // auto-adjustment. Disabled (`input.enforcement.enabled == false`,
    // the default) reproduces the original v1 behavior of reporting
    // `Infeasible` honestly rather than tightening the bore band.
    if input.enforcement.enabled && od_fit.status == ToleranceStatus::Infeasible {
        if input.enforcement.lock_bore {
            od_fit.notes.push("Strict interference enforcement is blocked because bore is locked (reamer-fixed).".to_string());
        } else {
            let enforced = enforce_bore_band_for_target(bore_tol, interference_tol, input.bore_capability.as_ref(), &input.enforcement);
            if enforced.changed {
                bore_tol = enforced.adjusted;
                od_fit = build_od_tolerance(bore_tol, interference_tol);
            }
            if let Some(note) = enforced.note {
                od_fit.notes.insert(0, note);
            }
        }
    }
    let vio = containment_violations(od_fit.achieved_interference, interference_tol);
    // solveEngine.ts:243 - `!enforceInterference || (containment satisfied)`.
    let enforcement_satisfied = !input.enforcement.enabled || (vio.lower_violation <= crate::tolerance::EPS && vio.upper_violation <= crate::tolerance::EPS);
    let od_installed = od_fit.od.nominal;

    // solveEngine.ts:250-254 - thermal + user interference -> net delta.
    // Imperial-only, so no metric dT*1.8 conversion (solveEngine.ts:251)
    // is needed here.
    let delta_thermal = (mat_bushing.alpha_u_f - mat_housing.alpha_u_f) * 1e-6 * bore_tol.nominal * input.d_t;
    let delta_user = od_fit.achieved_interference.nominal;
    let delta = delta_user + delta_thermal;

    // solveEngine.ts:255-263 - shrink-fit install-time thermal assist
    // (e.g. bushing chilled with dry ice, housing heated) - a real,
    // separate feature from the in-service `d_t`/`delta_thermal` above,
    // v1 never ported: `install_force`/`install_pressure` previously just
    // reused the in-service `pressure`/`retained_install_force`
    // unconditionally, silently discarding this input class entirely.
    let housing_assembly_delta_f = input.assembly_housing_temperature.map_or(0.0, |t| t - ASSEMBLY_REFERENCE_TEMP_F);
    let bushing_assembly_delta_f = input.assembly_bushing_temperature.map_or(0.0, |t| t - ASSEMBLY_REFERENCE_TEMP_F);
    let assembly_thermal_delta = (mat_bushing.alpha_u_f * bushing_assembly_delta_f - mat_housing.alpha_u_f * housing_assembly_delta_f) * 1e-6 * bore_tol.nominal;
    let has_assembly_thermal_assist = input.assembly_housing_temperature.is_some() || input.assembly_bushing_temperature.is_some();
    let install_delta = delta_user + if has_assembly_thermal_assist { assembly_thermal_delta } else { delta_thermal };

    // solveEngine.ts:356 - straight wall thickness (no countersink term).
    let wall_straight = ((od_installed - input.id_bushing) / 2.0).max(0.0);
    let fail_straight = wall_straight < input.min_wall_straight;

    // solveEngine.ts:265-270 - solve internal/external countersink
    // geometry (the derived dimension is re-solved from the other two +
    // the base diameter it's cut into). A non-countersink side has no
    // solved corner at all - `None`, not a meaningless placeholder.
    let cs_solved_id =
        (input.id_type == IdType::Countersink).then(|| solve_countersink(input.cs_mode, input.cs_dia, input.cs_depth, input.cs_angle, input.id_bushing));
    let cs_solved_od = (input.bushing_type == BushingType::Countersink)
        .then(|| solve_countersink(input.ext_cs_mode, input.ext_cs_dia, input.ext_cs_depth, input.ext_cs_angle, od_installed));

    // solveEngine.ts:274-309 - resolved tolerance bands for whichever CS
    // dia/depth field is the direct user input, needed for the corner
    // worst-case searches below.
    let cs_internal_dia_tol =
        resolve_tolerance(ResolveToleranceInput { mode: ToleranceMode::NominalTol, nominal: input.cs_dia, plus: input.cs_dia_tol_plus, minus: input.cs_dia_tol_minus, lower: None, upper: None, min_floor: Some(0.0) });
    let cs_internal_depth_tol = resolve_tolerance(ResolveToleranceInput {
        mode: ToleranceMode::NominalTol,
        nominal: input.cs_depth,
        plus: input.cs_depth_tol_plus,
        minus: input.cs_depth_tol_minus,
        lower: None,
        upper: None,
        min_floor: Some(0.0),
    });
    let cs_external_dia_tol = resolve_tolerance(ResolveToleranceInput {
        mode: ToleranceMode::NominalTol,
        nominal: input.ext_cs_dia,
        plus: input.ext_cs_dia_tol_plus,
        minus: input.ext_cs_dia_tol_minus,
        lower: None,
        upper: None,
        min_floor: Some(0.0),
    });
    let cs_external_depth_tol = resolve_tolerance(ResolveToleranceInput {
        mode: ToleranceMode::NominalTol,
        nominal: input.ext_cs_depth,
        plus: input.ext_cs_depth_tol_plus,
        minus: input.ext_cs_depth_tol_minus,
        lower: None,
        upper: None,
        min_floor: Some(0.0),
    });
    // Beyond the TS reference (no angle-tolerance input exists there at
    // all) - only meaningful when angle is a direct input for the mode
    // (`enumerate_countersink_corners`/`cs_*_tolerance_from_base` both
    // gate on mode themselves, same as dia/depth already do).
    let cs_internal_angle_tol = resolve_tolerance(ResolveToleranceInput {
        mode: ToleranceMode::NominalTol,
        nominal: input.cs_angle,
        plus: input.cs_angle_tol_plus,
        minus: input.cs_angle_tol_minus,
        lower: None,
        upper: None,
        min_floor: Some(0.0),
    });
    let cs_external_angle_tol = resolve_tolerance(ResolveToleranceInput {
        mode: ToleranceMode::NominalTol,
        nominal: input.ext_cs_angle,
        plus: input.ext_cs_angle_tol_plus,
        minus: input.ext_cs_angle_tol_minus,
        lower: None,
        upper: None,
        min_floor: Some(0.0),
    });
    let nominal_ext = cs_solved_od.map(|c| (c.dia, c.depth)).unwrap_or((od_installed, 0.0));
    let nominal_int = cs_solved_id.map(|c| (c.dia, c.depth)).unwrap_or((input.id_bushing, 0.0));

    // solveEngine.ts:311-335 - the countersink dimension that's DERIVED
    // (not the mode's direct input) gets a real tolerance range
    // propagated from the base diameter it's cut into plus the sibling
    // direct-input tolerances - e.g. in `DepthAngle` mode, depth and
    // angle are direct and diameter is derived, so the diameter's
    // "tolerance" is computed here (`cs_dia_tolerance_from_base`), not
    // just reported as an exact point. In `DiaDepth` mode, angle itself
    // is the derived dimension (`cs_angle_tolerance_from_base`) - a real
    // extension beyond the TS reference (see `countersink.rs`'s doc
    // comments), which has no angle-tolerance concept in any mode.
    let cs_internal_base = make_range(ToleranceMode::NominalTol, input.id_bushing, input.id_bushing, Some(input.id_bushing));
    let cs_external_base = od_fit.od;
    let cs_internal_dia_tol_out = if input.cs_mode == CsMode::DepthAngle {
        cs_solved_id.map_or(cs_internal_dia_tol, |c| cs_dia_tolerance_from_base(input.cs_mode, c.dia, c.depth, c.angle_deg, cs_internal_base, Some(cs_internal_depth_tol), Some(cs_internal_angle_tol)))
    } else {
        cs_internal_dia_tol
    };
    let cs_internal_depth_tol_out = if input.cs_mode == CsMode::DiaAngle {
        cs_solved_id.map_or(cs_internal_depth_tol, |c| cs_depth_tolerance_from_base(input.cs_mode, c.depth, c.dia, c.angle_deg, cs_internal_base, None, Some(cs_internal_dia_tol), Some(cs_internal_angle_tol)))
    } else {
        cs_internal_depth_tol
    };
    let cs_internal_angle_tol_out = if input.cs_mode == CsMode::DiaDepth {
        cs_solved_id.map_or(cs_internal_angle_tol, |c| cs_angle_tolerance_from_base(input.cs_mode, c.angle_deg, c.dia, c.depth, cs_internal_base, Some(cs_internal_dia_tol), Some(cs_internal_depth_tol)))
    } else {
        cs_internal_angle_tol
    };
    let cs_external_dia_tol_out = if input.ext_cs_mode == CsMode::DepthAngle {
        cs_solved_od.map_or(cs_external_dia_tol, |c| cs_dia_tolerance_from_base(input.ext_cs_mode, c.dia, c.depth, c.angle_deg, cs_external_base, Some(cs_external_depth_tol), Some(cs_external_angle_tol)))
    } else {
        cs_external_dia_tol
    };
    let cs_external_depth_tol_out = if input.ext_cs_mode == CsMode::DiaAngle {
        cs_solved_od.map_or(cs_external_depth_tol, |c| cs_depth_tolerance_from_base(input.ext_cs_mode, c.depth, c.dia, c.angle_deg, cs_external_base, None, Some(cs_external_dia_tol), Some(cs_external_angle_tol)))
    } else {
        cs_external_depth_tol
    };
    let cs_external_angle_tol_out = if input.ext_cs_mode == CsMode::DiaDepth {
        cs_solved_od.map_or(cs_external_angle_tol, |c| cs_angle_tolerance_from_base(input.ext_cs_mode, c.angle_deg, c.dia, c.depth, cs_external_base, Some(cs_external_dia_tol), Some(cs_external_depth_tol)))
    } else {
        cs_external_angle_tol
    };

    // solveEngine.ts:356-410 - neck-wall minimum thickness. Only an
    // internally-countersunk ID actually thins the wall (an externally
    // countersunk/flanged OD flares outward or only extends axially
    // beyond the housing - proven equal to `wall_straight` in
    // `geometry.rs`'s own tests), matching the TS source's own gate
    // exactly (`idType !== 'countersink' ? wallStraight : ...`).
    let section_input = |cs_ext: (f64, f64), cs_int: (f64, f64)| BushingSectionInput {
        bore_dia: bore_tol.nominal,
        housing_len: input.housing_len,
        housing_width: input.housing_width,
        id_bushing: input.id_bushing,
        bushing_type: input.bushing_type,
        id_type: input.id_type,
        flange_od: input.flange_od,
        flange_thk: input.flange_thk,
        od_bushing: od_installed,
        cs_external: Some(cs_ext),
        cs_internal: Some(cs_int),
    };
    let wall_neck_nominal = if input.id_type != IdType::Countersink {
        wall_straight
    } else {
        compute_minimum_bushing_wall(&resolve_bushing_section_params(&section_input(nominal_ext, nominal_int)))
    };
    // solveEngine.ts:376-410 - worst-case corner search: minimize wall
    // thickness across every physically self-consistent combination of
    // internal/external countersink corners (each corner is re-solved via
    // `enumerate_countersink_corners`, never an independently-perturbed
    // dia/depth pair).
    let wall_neck = if input.id_type != IdType::Countersink {
        wall_neck_nominal
    } else {
        let internal_corners = enumerate_countersink_corners(input.cs_mode, input.id_bushing, cs_internal_dia_tol, cs_internal_depth_tol, cs_internal_angle_tol);
        let external_corners = if input.bushing_type == BushingType::Countersink {
            enumerate_countersink_corners(input.ext_cs_mode, od_installed, cs_external_dia_tol, cs_external_depth_tol, cs_external_angle_tol)
        } else {
            vec![CsCorner { dia: nominal_ext.0, depth: nominal_ext.1, angle_deg: 0.0 }]
        };
        let mut worst = f64::INFINITY;
        for int_c in &internal_corners {
            for ext_c in &external_corners {
                let wall = compute_minimum_bushing_wall(&resolve_bushing_section_params(&section_input((ext_c.dia, ext_c.depth), (int_c.dia, int_c.depth))));
                worst = worst.min(wall);
            }
        }
        wall_neck_nominal.min(worst)
    };
    let fail_neck = wall_neck < input.min_wall_neck;

    // solveEngine.ts:414-433 - Lame compliance terms + contact pressure,
    // including the finite-plate (psi/lambda) housing correction.
    let eh = mat_housing.e_ksi * 1000.0;
    let eb = mat_bushing.e_ksi * 1000.0;
    let r_sat = bore_tol.nominal * 2.0;
    let w_eff = input.housing_width.min(r_sat * 2.0);
    let e_eff = input.edge_dist.min(r_sat);
    let area_housing = (w_eff * (e_eff * 2.0) - (std::f64::consts::PI * bore_tol.nominal.powi(2)) / 4.0).max(1e-6);
    let d_equivalent = ((4.0 * area_housing) / std::f64::consts::PI + bore_tol.nominal.powi(2)).sqrt();
    let lambda = (e_eff / (d_equivalent / 2.0)).min(1.0);
    let psi = 1.0 + 0.2 * (1.0 - lambda);
    let effective_od_housing = d_equivalent;
    // Shrink-fit compliance terms, derived (not re-derived) from the
    // general thick-wall Lamé engine in `lame.rs`: apply a unit contact
    // pressure at the shared bore interface to each region and read off
    // its own diametral compliance there. `psi` (the finite-plate
    // housing correction above) is a bushing-specific geometry factor,
    // not part of the general pressure-vessel physics, so it's
    // multiplied in here rather than folded into the general function.
    let term_b = crate::lame::diametral_interference_compliance(input.id_bushing / 2.0, bore_tol.nominal / 2.0, bore_tol.nominal / 2.0, 0.0, 1.0, eb, mat_bushing.nu);
    let term_h = psi * crate::lame::diametral_interference_compliance(bore_tol.nominal / 2.0, effective_od_housing / 2.0, bore_tol.nominal / 2.0, 1.0, 0.0, eh, mat_housing.nu);
    let pressure = if delta > 0.0 { delta / (term_b + term_h) } else { 0.0 };

    // solveEngine.ts:435-448 - hoop stresses, axial estimate, install force.
    // Full Lamé thick-wall hoop stress at the shared bore interface -
    // same general function (and the same boundary-radius/pressure
    // convention) `bushing_stress_field`/`housing_stress_field` below use
    // for the full per-radius plot, so the boundary value shown here and
    // the field's own boundary sample can never silently disagree.
    let (_, stress_hoop_housing) = crate::lame::lame_stress_at_radius(bore_tol.nominal / 2.0, bore_tol.nominal / 2.0, effective_od_housing / 2.0, pressure, 0.0);
    let (_, stress_hoop_bushing) = crate::lame::lame_stress_at_radius(bore_tol.nominal / 2.0, input.id_bushing / 2.0, bore_tol.nominal / 2.0, 0.0, pressure);
    let axial_constraint_factor = end_constraint_factor(input.end_constraint);
    let axial_length_factor = clamp(input.housing_len / (4.0 * wall_straight).max(1e-6), 0.0, 1.0);
    // solveEngine.ts:443-444 - this is where `end_constraint` actually
    // does something: `axial_constraint_factor`/`axial_length_factor`
    // were computed above but never previously multiplied into a real
    // axial stress anywhere in this port, so choosing Free/OneEnd/BothEnds
    // had no effect on any displayed result - a real gap, not a
    // by-design omission. `axial_scale` gates both boundary-level axial
    // stress here and the per-radius stress field's own axial term below
    // (`lame::sample_lame_field`) - the same scale factor feeds
    // both, per the TS source using it in both places.
    let axial_scale = axial_constraint_factor * axial_length_factor;
    let stress_axial_housing = axial_scale * mat_housing.nu * stress_hoop_housing;
    let stress_axial_bushing = axial_scale * mat_bushing.nu * stress_hoop_bushing;
    let friction = input.friction.filter(|f| f.is_finite()).unwrap_or(0.15);
    let retained_install_force = friction * pressure * std::f64::consts::PI * bore_tol.nominal * input.housing_len;
    // solveEngine.ts:434,447-448 - install-time force/pressure, from
    // `install_delta` (the shrink-fit-assisted delta when assembly
    // temperatures are given, otherwise the same in-service delta as
    // `retained_install_force` above - `install_delta`'s own definition
    // already encodes that fallback).
    let install_pressure = if install_delta > 0.0 { install_delta / (term_b + term_h) } else { 0.0 };
    let install_force = friction * install_pressure * std::f64::consts::PI * bore_tol.nominal * input.housing_len;

    // solveEngine.ts:450-480 - edge-distance strength's effective
    // sequencing thickness. Reduces to exactly `housing_len` for
    // `Straight`/`Flanged` (a single cylindrical, eta=1.0 parent segment
    // the length of the housing - `bearing.rs`'s own tests prove this),
    // so no behavior change there; `Countersink` gets the real bearing
    // profile plus a worst-external-corner search.
    let build_bearing_profile = |od_dia: f64, od_depth: f64| -> Vec<BearingSegment> {
        if input.bushing_type == BushingType::Countersink {
            let mut segments = vec![BearingSegment { d_top: od_dia, d_bottom: od_installed, height: od_depth.min(input.housing_len), eta: None, is_parent: true }];
            if od_depth < input.housing_len {
                segments.push(BearingSegment { d_top: od_installed, d_bottom: od_installed, height: (input.housing_len - od_depth).max(0.0), eta: None, is_parent: true });
            }
            segments
        } else {
            vec![BearingSegment { d_top: od_installed, d_bottom: od_installed, height: input.housing_len, eta: None, is_parent: true }]
        }
    };
    // `... || input.housingLen` in the TS source - JS `||` replaces an
    // exact-zero (not just absent) `t_eff_sequence` with `housingLen`.
    let t_eff_seq_or_housing_len = |dia: f64, depth: f64| {
        let t = calculate_universal_bearing(&build_bearing_profile(dia, depth)).t_eff_sequence;
        if t != 0.0 { t } else { input.housing_len }
    };
    let t_eff_seq_nominal = t_eff_seq_or_housing_len(nominal_ext.0, nominal_ext.1);
    let t_eff_seq = if input.bushing_type != BushingType::Countersink {
        t_eff_seq_nominal
    } else {
        let external_corners = enumerate_countersink_corners(input.ext_cs_mode, od_installed, cs_external_dia_tol, cs_external_depth_tol, cs_external_angle_tol);
        let worst = external_corners.iter().map(|c| t_eff_seq_or_housing_len(c.dia, c.depth)).fold(f64::INFINITY, f64::min);
        t_eff_seq_nominal.min(worst)
    };
    let edge_load_angle_deg = input.edge_load_angle_deg.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(40.0);
    let sin_theta = (edge_load_angle_deg.abs() * std::f64::consts::PI / 180.0).sin().max(1e-6);
    // toPsiFromKsi(matHousing.Fbru_ksi || matHousing.Sy_ksi || 0) - JS `||`
    // falls through on a zero/absent value, not just `undefined`.
    let fbru_base = if mat_housing.fbru_ksi != 0.0 { mat_housing.fbru_ksi } else { mat_housing.sy_ksi };
    let fbru_eff = fbru_base * 1000.0 + 0.8 * pressure;
    let tau = (if mat_housing.fsu_ksi != 0.0 { mat_housing.fsu_ksi } else { mat_housing.sy_ksi }) * 1000.0;
    let e_required_seq = if tau > 0.0 { (bore_tol.nominal * fbru_eff) / (2.0 * tau * sin_theta) } else { f64::INFINITY };
    let load = input.load.filter(|v| v.is_finite()).unwrap_or(1000.0);
    let e_required_strength = if 2.0 * t_eff_seq * tau * sin_theta > 1e-9 { load / (2.0 * t_eff_seq * tau * sin_theta) } else { f64::INFINITY };
    let ed_min_sequence = if e_required_seq > 0.0 { e_required_seq / bore_tol.nominal } else { f64::INFINITY };
    let ed_min_strength = if e_required_strength > 0.0 { e_required_strength / bore_tol.nominal } else { f64::INFINITY };
    let ed_actual = if bore_tol.nominal > 0.0 { input.edge_dist / bore_tol.nominal } else { f64::INFINITY };
    let sequence_margin = if ed_min_sequence.is_finite() && ed_min_sequence > 0.0 { ed_actual / ed_min_sequence - 1.0 } else { f64::INFINITY };
    let strength_margin = if ed_min_strength.is_finite() && ed_min_strength > 0.0 { ed_actual / ed_min_strength - 1.0 } else { f64::INFINITY };

    // solveEngine.ts:504-510 - governing (lowest-margin) check.
    let candidates = vec![
        MarginCandidate { name: "Edge distance (sequencing)", margin: sequence_margin },
        MarginCandidate { name: "Edge distance (strength)", margin: strength_margin },
        MarginCandidate { name: "Straight wall thickness", margin: wall_straight / input.min_wall_straight - 1.0 },
        MarginCandidate { name: "Neck wall thickness", margin: wall_neck / input.min_wall_neck - 1.0 },
    ];
    let governing = candidates.iter().cloned().reduce(|best, cur| if cur.margin < best.margin { cur } else { best }).expect("candidates is never empty");

    // toOutput (solveEngine.ts:703-716) - hoop margins of safety.
    let housing_ms = if stress_hoop_housing != 0.0 { (mat_housing.sy_ksi * 1000.0) / stress_hoop_housing - 1.0 } else { f64::INFINITY };
    let bushing_ms = if stress_hoop_bushing != 0.0 { (mat_bushing.sy_ksi * 1000.0) / stress_hoop_bushing.abs() - 1.0 } else { f64::INFINITY };

    // solveEngine.ts:633-648 (`toOutput`'s `lame.field`) - the actual
    // hoop/radial/axial stress *distribution* across each region's wall,
    // not just its two boundary values. `41` matches the TS source's own
    // default sample count.
    let bushing_stress_field = crate::lame::sample_lame_field(input.id_bushing / 2.0, bore_tol.nominal / 2.0, 0.0, pressure, axial_scale, mat_bushing.nu, 41);
    let housing_stress_field = crate::lame::sample_lame_field(bore_tol.nominal / 2.0, effective_od_housing / 2.0, pressure, 0.0, axial_scale, mat_housing.nu, 41);

    // Result ranges mirroring the input tolerance format (nominal +
    // min/max) - evaluated at the achieved-interference band's real
    // extremes, not perturbed independently. `wall_straight` depends
    // only on `od_tol`/`id_bushing` (no delta/pressure involved), so its
    // range comes directly from `od_fit.od.lower`/`.upper`; everything
    // else re-runs `delta_dependent_chain` at the two extreme deltas.
    let wall_straight_range = ranged(
        wall_straight,
        ((od_fit.od.lower - input.id_bushing) / 2.0).max(0.0),
        ((od_fit.od.upper - input.id_bushing) / 2.0).max(0.0),
    );
    let invariants = DeltaInvariants {
        term_b,
        term_h,
        bore_nominal: bore_tol.nominal,
        id_bushing: input.id_bushing,
        effective_od_housing,
        friction,
        housing_len: input.housing_len,
        fbru_base,
        tau,
        sin_theta,
        ed_actual,
        sy_housing_psi: mat_housing.sy_ksi * 1000.0,
        sy_bushing_psi: mat_bushing.sy_ksi * 1000.0,
    };
    let at_lower = delta_dependent_chain(od_fit.achieved_interference.lower + delta_thermal, &invariants);
    let at_upper = delta_dependent_chain(od_fit.achieved_interference.upper + delta_thermal, &invariants);
    let pressure_range = ranged(pressure, at_lower.pressure, at_upper.pressure);
    let stress_hoop_housing_range = ranged(stress_hoop_housing, at_lower.stress_hoop_housing, at_upper.stress_hoop_housing);
    let stress_hoop_bushing_range = ranged(stress_hoop_bushing, at_lower.stress_hoop_bushing, at_upper.stress_hoop_bushing);
    let housing_ms_range = ranged(housing_ms, at_lower.housing_ms, at_upper.housing_ms);
    let bushing_ms_range = ranged(bushing_ms, at_lower.bushing_ms, at_upper.bushing_ms);
    let retained_install_force_range = ranged(retained_install_force, at_lower.install_force, at_upper.install_force);
    // `install_force` is install-delta-driven (possibly assembly-thermal-
    // assisted), a genuinely different bracket from the in-service
    // `achieved_interference` range above - the assembly/thermal-assist
    // offset itself isn't toleranced (a single entered install
    // temperature, not a tolerance band), so only `delta_user`'s own
    // range carries through.
    let install_assist_offset = if has_assembly_thermal_assist { assembly_thermal_delta } else { delta_thermal };
    let at_install_lower = delta_dependent_chain(od_fit.achieved_interference.lower + install_assist_offset, &invariants);
    let at_install_upper = delta_dependent_chain(od_fit.achieved_interference.upper + install_assist_offset, &invariants);
    let install_force_range = ranged(install_force, at_install_lower.install_force, at_install_upper.install_force);
    let sequence_margin_range = ranged(sequence_margin, at_lower.sequence_margin, at_upper.sequence_margin);

    BushingOutput {
        od_installed,
        wall_straight,
        fail_straight,
        wall_neck,
        wall_neck_nominal,
        fail_neck,
        cs_solved_id,
        cs_solved_od,
        delta_thermal,
        delta_user,
        delta_total: delta,
        pressure,
        effective_od_housing,
        d_equivalent,
        psi,
        lambda,
        term_b,
        term_h,
        stress_hoop_housing,
        stress_hoop_bushing,
        housing_ms,
        bushing_ms,
        axial_constraint_factor,
        axial_length_factor,
        stress_axial_housing,
        stress_axial_bushing,
        bushing_stress_field,
        housing_stress_field,
        install_force,
        retained_install_force,
        ed_actual,
        ed_min_sequence,
        ed_min_strength,
        sequence_margin,
        strength_margin,
        candidates,
        governing,
        wall_straight_range,
        pressure_range,
        stress_hoop_housing_range,
        stress_hoop_bushing_range,
        housing_ms_range,
        bushing_ms_range,
        install_force_range,
        retained_install_force_range,
        sequence_margin_range,
        bore_tol,
        interference_tol,
        od_tol: od_fit.od,
        achieved_interference_tol: od_fit.achieved_interference,
        tolerance_status: od_fit.status,
        tolerance_notes: od_fit.notes,
        enforcement_satisfied,
        cs_internal_dia_tol: (input.id_type == IdType::Countersink).then_some(cs_internal_dia_tol_out),
        cs_internal_depth_tol: (input.id_type == IdType::Countersink).then_some(cs_internal_depth_tol_out),
        cs_external_dia_tol: (input.bushing_type == BushingType::Countersink).then_some(cs_external_dia_tol_out),
        cs_external_depth_tol: (input.bushing_type == BushingType::Countersink).then_some(cs_external_depth_tol_out),
        cs_internal_angle_tol: (input.id_type == IdType::Countersink).then_some(cs_internal_angle_tol_out),
        cs_external_angle_tol: (input.bushing_type == BushingType::Countersink).then_some(cs_external_angle_tol_out),
        assembly_thermal_delta,
        install_delta,
        install_pressure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_input() -> BushingInputs {
        BushingInputs {
            bore_dia: 0.5,
            bore_tol_plus: 0.0,
            bore_tol_minus: 0.0,
            id_bushing: 0.375,
            interference: 0.0015,
            interference_tol_plus: 0.0,
            interference_tol_minus: 0.0,
            housing_len: 0.5,
            housing_width: 1.5,
            edge_dist: 0.75,
            mat_housing: "al7075".to_string(),
            mat_bushing: "bronze".to_string(),
            friction: Some(0.15),
            d_t: 0.0,
            end_constraint: EndConstraint::Free,
            min_wall_straight: 0.05,
            edge_load_angle_deg: None,
            load: None,
            ..Default::default()
        }
    }

    #[test]
    fn zero_or_negative_net_interference_yields_zero_pressure() {
        let mut input = base_input();
        input.interference = 0.0;
        let out = compute(&input);
        assert_eq!(out.pressure, 0.0);
        assert_eq!(out.stress_hoop_housing, 0.0);
    }

    #[test]
    fn positive_interference_yields_positive_pressure_and_opposite_signed_hoop_stresses() {
        let out = compute(&base_input());
        assert!(out.pressure > 0.0);
        // Housing is in tension (positive), bushing is in compression
        // (negative) - opposite signs, same physical loading, exactly as
        // the TS source's own sign convention documents.
        assert!(out.stress_hoop_housing > 0.0);
        assert!(out.stress_hoop_bushing < 0.0);
    }

    #[test]
    fn thin_wall_below_minimum_is_flagged() {
        let mut input = base_input();
        input.min_wall_straight = 1.0; // impossibly large minimum
        let out = compute(&input);
        assert!(out.fail_straight);
    }

    #[test]
    fn governing_candidate_is_the_lowest_margin_among_candidates() {
        let out = compute(&base_input());
        let min_margin = out.candidates.iter().map(|c| c.margin).fold(f64::INFINITY, f64::min);
        assert_eq!(out.governing.margin, min_margin);
    }

    #[test]
    fn end_constraint_free_produces_zero_axial_stress() {
        // The pre-fix bug: axial_constraint_factor/axial_length_factor
        // were computed but never multiplied into an actual axial
        // stress, so switching EndConstraint had no visible effect on
        // any output at all. Free must still legitimately zero it (its
        // own factor is 0), but Free producing 0.0 was true even under
        // the bug - the real proof is the BothEnds/OneEnd test below.
        let out = compute(&base_input());
        assert_eq!(out.axial_constraint_factor, 0.0);
        assert_eq!(out.stress_axial_housing, 0.0);
        assert_eq!(out.stress_axial_bushing, 0.0);
    }

    #[test]
    fn end_constraint_both_ends_matches_real_ts_axial_stress() {
        // Golden values captured from the real TS engine (2026-08-29),
        // same base fixture as `differential.rs`, with
        // `endConstraint: 'both_ends'`.
        let mut input = base_input();
        input.end_constraint = EndConstraint::BothEnds;
        let out = compute(&input);
        assert_eq!(out.axial_constraint_factor, 1.0);
        assert_eq!(out.axial_length_factor, 1.0);
        assert!((out.stress_axial_housing - 3457.0024080600056).abs() < 1e-6);
        assert!((out.stress_axial_bushing - (-10678.607997445815)).abs() < 1e-6);

        // One end should be exactly half of both-ends (axial_constraint_factor
        // 0.5 vs 1.0, everything else identical) - proving the constraint
        // factor, not just a hardcoded pair of golden numbers, actually
        // drives the result.
        input.end_constraint = EndConstraint::OneEnd;
        let one_end = compute(&input);
        assert!((one_end.stress_axial_housing - out.stress_axial_housing / 2.0).abs() < 1e-6);
        assert!((one_end.stress_axial_bushing - out.stress_axial_bushing / 2.0).abs() < 1e-6);
    }

    #[test]
    fn stress_fields_are_populated_and_bracket_the_boundary_stress_values() {
        let mut input = base_input();
        input.end_constraint = EndConstraint::BothEnds;
        let out = compute(&input);
        assert_eq!(out.bushing_stress_field.len(), 41);
        assert_eq!(out.housing_stress_field.len(), 41);
        // Boundary samples must agree with the already-verified single-point
        // stress_hoop_bushing/stress_hoop_housing (the outer edge of the
        // bushing region and the inner edge of the housing region are both
        // the bore/OD interface).
        let bushing_outer = out.bushing_stress_field.last().unwrap();
        assert!((bushing_outer.sigma_theta - out.stress_hoop_bushing).abs() < 1e-6);
        let housing_inner = out.housing_stress_field.first().unwrap();
        assert!((housing_inner.sigma_theta - out.stress_hoop_housing).abs() < 1e-6);
    }

    #[test]
    fn no_assembly_temperature_makes_install_force_equal_retained_install_force() {
        // The pre-fix behavior for the common case (no shrink-fit assist
        // entered) - install_force must still exactly equal
        // retained_install_force, not silently change now that they're
        // computed via genuinely separate code paths.
        let out = compute(&base_input());
        assert_eq!(out.install_force, out.retained_install_force);
        assert_eq!(out.install_delta, out.delta_total);
    }

    #[test]
    fn assembly_temperature_assist_matches_real_ts_install_state_physics() {
        // Golden values captured from the real TS engine (2026-08-29):
        // housing left at 70F (no change from the 70F reference), bushing
        // chilled to -20F to ease installation - a real shrink-fit assist.
        let mut input = base_input();
        input.assembly_housing_temperature = Some(70.0);
        input.assembly_bushing_temperature = Some(-20.0);
        let out = compute(&input);
        assert!((out.assembly_thermal_delta - (-0.000405)).abs() < 1e-9);
        assert!((out.install_delta - 0.0010949999999999458).abs() < 1e-9);
        assert!((out.install_pressure - 6419.727866699693).abs() < 1e-6);
        assert!((out.install_force - 756.3063714026035).abs() < 1e-6);
        // Retained (in-service) numbers must be completely unaffected by
        // an install-time-only thermal assist.
        assert!((out.retained_install_force - 1036.0361252090597).abs() < 1e-6);
        assert!((out.delta_total - 0.0014999999999999458).abs() < 1e-9);
    }

    #[test]
    fn zero_tolerance_bands_collapse_every_range_to_its_own_nominal() {
        // base_input has bore_tol_plus/minus and interference_tol_plus/minus
        // all 0.0 - a zero-width band, so every RangedValue's min/max must
        // exactly equal its nominal (no phantom range from a degenerate
        // tolerance).
        let out = compute(&base_input());
        for (name, r) in [
            ("wall_straight", out.wall_straight_range),
            ("pressure", out.pressure_range),
            ("stress_hoop_housing", out.stress_hoop_housing_range),
            ("stress_hoop_bushing", out.stress_hoop_bushing_range),
            ("housing_ms", out.housing_ms_range),
            ("bushing_ms", out.bushing_ms_range),
            ("install_force", out.install_force_range),
            ("retained_install_force", out.retained_install_force_range),
            ("sequence_margin", out.sequence_margin_range),
        ] {
            assert!((r.min - r.nominal).abs() < 1e-6, "{name}: min {} != nominal {}", r.min, r.nominal);
            assert!((r.max - r.nominal).abs() < 1e-6, "{name}: max {} != nominal {}", r.max, r.nominal);
        }
    }

    #[test]
    fn nonzero_tolerance_bands_produce_a_real_pressure_and_stress_range_bracketing_nominal() {
        let mut input = base_input();
        // Bore band must stay narrower than the interference band or
        // `build_od_tolerance` reports `Infeasible` and collapses `od`
        // to a single point (a real, separately-tested case in
        // `tolerance.rs`) - not what this test is checking.
        input.bore_tol_plus = 0.0002;
        input.bore_tol_minus = 0.0002;
        input.interference_tol_plus = 0.0008;
        input.interference_tol_minus = 0.0008;
        let out = compute(&input);
        assert_eq!(out.tolerance_status, crate::tolerance::ToleranceStatus::Ok, "test fixture must stay tolerance-feasible for this assertion to mean anything");

        // min <= nominal <= max for every physically monotonic-in-interference
        // quantity, and the band must be genuinely non-degenerate (not
        // silently collapsed back to a point).
        assert!(out.pressure_range.min < out.pressure_range.nominal);
        assert!(out.pressure_range.nominal < out.pressure_range.max);
        assert!(out.stress_hoop_housing_range.min < out.stress_hoop_housing_range.nominal);
        assert!(out.stress_hoop_housing_range.nominal < out.stress_hoop_housing_range.max);
        // Housing MS is inversely related to stress - its "max" (best
        // case, safest) must correspond to the *lowest* stress, not the
        // highest interference.
        assert!(out.housing_ms_range.max > out.housing_ms_range.nominal);
        assert!(out.housing_ms_range.nominal > out.housing_ms_range.min);
        // wall_straight range must come from the OD tolerance band
        // directly, independent of the pressure/stress chain.
        assert!(out.wall_straight_range.min < out.wall_straight_range.nominal);
        assert!(out.wall_straight_range.nominal < out.wall_straight_range.max);
    }

    #[test]
    fn angle_tolerance_survives_to_the_output_when_angle_is_a_direct_input() {
        // Regression: `cs_*_angle_tol_out` used to unconditionally call
        // `cs_angle_tolerance_from_base`, which collapses to a point
        // range in every mode except `DiaDepth` (angle is derived only
        // there) - silently discarding the user's own resolved
        // `cs_angle_tol_plus/minus` in `DepthAngle`/`DiaAngle` mode,
        // where angle IS the direct input. Must mirror the dia/depth
        // pattern: only call the derive-function when this mode makes
        // angle the derived dimension, else pass the resolved direct
        // tolerance straight through.
        let mut input = base_input();
        input.id_type = IdType::Countersink;
        input.bushing_type = BushingType::Countersink;
        input.cs_mode = CsMode::DepthAngle;
        input.cs_dia = 0.5;
        input.cs_depth = 0.08;
        input.cs_angle = 100.0;
        input.cs_angle_tol_plus = 2.0;
        input.cs_angle_tol_minus = 1.0;
        input.ext_cs_mode = CsMode::DiaAngle;
        input.ext_cs_dia = 0.6;
        input.ext_cs_depth = 0.06;
        input.ext_cs_angle = 100.0;
        input.ext_cs_angle_tol_plus = 3.0;
        input.ext_cs_angle_tol_minus = 1.5;

        let out = compute(&input);

        let internal_angle = out.cs_internal_angle_tol.expect("id_type is Countersink");
        assert!((internal_angle.upper - internal_angle.lower).abs() > 1e-6, "DepthAngle mode: angle is a direct input, its resolved tolerance must not collapse to a point");
        assert!((internal_angle.upper - 102.0).abs() < 1e-9, "internal angle upper must equal nominal + tol_plus");
        assert!((internal_angle.lower - 99.0).abs() < 1e-9, "internal angle lower must equal nominal - tol_minus");

        let external_angle = out.cs_external_angle_tol.expect("bushing_type is Countersink");
        assert!((external_angle.upper - external_angle.lower).abs() > 1e-6, "DiaAngle mode: angle is a direct input, its resolved tolerance must not collapse to a point");
        assert!((external_angle.upper - 103.0).abs() < 1e-9, "external angle upper must equal nominal + tol_plus");
        assert!((external_angle.lower - 98.5).abs() < 1e-9, "external angle lower must equal nominal - tol_minus");
    }
}
