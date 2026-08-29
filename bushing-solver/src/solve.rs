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
use crate::countersink::{enumerate_countersink_corners, solve_countersink, CsCorner, CsMode};
use crate::geometry::{compute_minimum_bushing_wall, resolve_bushing_section_params, BushingSectionInput};
pub use crate::geometry::{BushingType, IdType};
use crate::materials::{get_material, Material};
use crate::tolerance::{
    build_od_tolerance, clamp, containment_violations, enforce_bore_band_for_target, resolve_tolerance, BoreCapability, EnforcementPolicy,
    ResolveToleranceInput, ToleranceMode, ToleranceRange, ToleranceStatus,
};

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

    /// Bore-tolerance auto-adjustment policy - disabled (`enabled: false`)
    /// reproduces the original v1 behavior of reporting `Infeasible`
    /// honestly rather than tightening the bore band.
    pub enforcement: EnforcementPolicy,
    /// Bore process-capability floor consulted by the enforcement policy.
    pub bore_capability: Option<BoreCapability>,
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

    pub install_force: f64,
    pub retained_install_force: f64,

    pub ed_actual: f64,
    pub ed_min_sequence: f64,
    pub ed_min_strength: f64,
    pub sequence_margin: f64,
    pub strength_margin: f64,

    pub candidates: Vec<MarginCandidate>,
    pub governing: MarginCandidate,

    pub bore_tol: ToleranceRange,
    pub interference_tol: ToleranceRange,
    pub od_tol: ToleranceRange,
    pub achieved_interference_tol: ToleranceRange,
    pub tolerance_status: ToleranceStatus,
    pub tolerance_notes: Vec<String>,
    pub enforcement_satisfied: bool,
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
    let nominal_ext = cs_solved_od.map(|c| (c.dia, c.depth)).unwrap_or((od_installed, 0.0));
    let nominal_int = cs_solved_id.map(|c| (c.dia, c.depth)).unwrap_or((input.id_bushing, 0.0));

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
        let internal_corners =
            enumerate_countersink_corners(input.cs_mode, cs_solved_id.map(|c| c.angle_deg).unwrap_or(input.cs_angle), input.id_bushing, cs_internal_dia_tol, cs_internal_depth_tol);
        let external_corners = if input.bushing_type == BushingType::Countersink {
            enumerate_countersink_corners(input.ext_cs_mode, cs_solved_od.map(|c| c.angle_deg).unwrap_or(input.ext_cs_angle), od_installed, cs_external_dia_tol, cs_external_depth_tol)
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
    let term_b = (bore_tol.nominal / eb) * (((bore_tol.nominal.powi(2) + input.id_bushing.powi(2)) / (bore_tol.nominal.powi(2) - input.id_bushing.powi(2))) - mat_bushing.nu);
    let term_h = psi * (bore_tol.nominal / eh) * (((effective_od_housing.powi(2) + bore_tol.nominal.powi(2)) / (effective_od_housing.powi(2) - bore_tol.nominal.powi(2))) + mat_housing.nu);
    let pressure = if delta > 0.0 { delta / (term_b + term_h) } else { 0.0 };

    // solveEngine.ts:435-448 - hoop stresses, axial estimate, install force.
    let stress_hoop_housing = pressure * ((effective_od_housing.powi(2) + bore_tol.nominal.powi(2)) / (effective_od_housing.powi(2) - bore_tol.nominal.powi(2)));
    let stress_hoop_bushing = -pressure * ((bore_tol.nominal.powi(2) + input.id_bushing.powi(2)) / (bore_tol.nominal.powi(2) - input.id_bushing.powi(2)));
    let axial_constraint_factor = end_constraint_factor(input.end_constraint);
    let axial_length_factor = clamp(input.housing_len / (4.0 * wall_straight).max(1e-6), 0.0, 1.0);
    let friction = input.friction.filter(|f| f.is_finite()).unwrap_or(0.15);
    let retained_install_force = friction * pressure * std::f64::consts::PI * bore_tol.nominal * input.housing_len;
    // v1 has no separate install-state thermal-assist temperature inputs
    // (solveEngine.ts:255-263), so install pressure == retained pressure
    // and install_force == retained_install_force - both computed from
    // the same `pressure`/`delta`, matching what the TS source itself
    // falls back to when no assembly-temperature override is given.
    let install_force = retained_install_force;

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
        let external_corners =
            enumerate_countersink_corners(input.ext_cs_mode, cs_solved_od.map(|c| c.angle_deg).unwrap_or(input.ext_cs_angle), od_installed, cs_external_dia_tol, cs_external_depth_tol);
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
        install_force,
        retained_install_force,
        ed_actual,
        ed_min_sequence,
        ed_min_strength,
        sequence_margin,
        strength_margin,
        candidates,
        governing,
        bore_tol,
        interference_tol,
        od_tol: od_fit.od,
        achieved_interference_tol: od_fit.achieved_interference,
        tolerance_status: od_fit.status,
        tolerance_notes: od_fit.notes,
        enforcement_satisfied,
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
}
