//! Bridges the pure `bushing-solver`/`mechanics-core` engine into UI-facing
//! state, the same split `toolboxes/pressure_vessel/model.rs` uses: zero
//! `ratatui`/`crossterm` here - only `mod.rs` (key routing) and `view.rs`
//! (rendering) know about the terminal.
//!
//! **Deliberately not ported** from `app`'s/`app-egui`'s own Bushing
//! Workbench: the axial cross-section sketch (`app-egui/src/sketches.rs`'s
//! `bushing_cross_section`, `app/src/bushing_visualizer.rs`) - visual
//! presentation with no terminal equivalent, same call `pressure_vessel`
//! already made for its own cross-section sketches. Every numeric result
//! either GUI head computes is still exposed here.
//!
//! Every field maps 1:1 onto `bushing_solver::solve::BushingInputs` - see
//! that type's own doc comment for units (imperial only: in, psi/ksi, lbf,
//! degF) and the "every new field defaults through `..Default::default()`"
//! contract this module's `Default` impl mirrors by construction (each
//! field here is set to the exact value that produces the same
//! `BushingInputs` default would).

use bushing_solver::countersink::CsMode;
use bushing_solver::geometry::{BushingType, IdType};
use bushing_solver::reamers::ReamerEntry;
use bushing_solver::solve::{compute, BushingInputs, BushingOutput, EndConstraint};
use bushing_solver::tolerance::{BoreCapability, EnforcementPolicy};
use mechanics_core::materials::{Material, MATERIALS};

/// Leaks a user-added material's owned `String` fields into `&'static str`
/// so it can be stored as an ordinary `&'static Material` alongside the
/// built-in `MATERIALS` table - same technique
/// `pressure_vessel::model::leak_custom_material` uses, extended with the
/// `fbru_ksi`/`fsu_ksi` (bearing/shear ultimate) fields that toolbox
/// deliberately zeroes because `pressure-vessel-solver` never reads them;
/// `bushing_solver::solve::compute` *does* read both (edge-bearing/shear
/// margin checks), so this toolbox's own add-material form must collect
/// them for real. The leak is bounded by how many materials a user
/// manually adds in a session, same reasoning as that toolbox's own
/// comment.
fn leak_custom_material(name: String, e_ksi: f64, sy_ksi: f64, fbru_ksi: f64, fsu_ksi: f64, ftu_ksi: f64, nu: f64, alpha_u_f: f64) -> &'static Material {
    let id: &'static str = Box::leak(format!("custom:{name}").into_boxed_str());
    let name: &'static str = Box::leak(name.into_boxed_str());
    Box::leak(Box::new(Material { id, name, e_ksi, sy_ksi, fbru_ksi, fsu_ksi, ftu_ksi, nu, alpha_u_f }))
}

pub fn cycle_bushing_type(t: BushingType) -> BushingType {
    match t {
        BushingType::Straight => BushingType::Flanged,
        BushingType::Flanged => BushingType::Countersink,
        BushingType::Countersink => BushingType::Straight,
    }
}

pub fn label_bushing_type(t: BushingType) -> &'static str {
    match t {
        BushingType::Straight => "Straight",
        BushingType::Flanged => "Flanged",
        BushingType::Countersink => "Countersink (OD)",
    }
}

pub fn cycle_id_type(t: IdType) -> IdType {
    match t {
        IdType::Straight => IdType::Countersink,
        IdType::Countersink => IdType::Straight,
    }
}

pub fn label_id_type(t: IdType) -> &'static str {
    match t {
        IdType::Straight => "Straight",
        IdType::Countersink => "Countersink (ID)",
    }
}

pub fn cycle_end_constraint(e: EndConstraint) -> EndConstraint {
    match e {
        EndConstraint::Free => EndConstraint::OneEnd,
        EndConstraint::OneEnd => EndConstraint::BothEnds,
        EndConstraint::BothEnds => EndConstraint::Free,
    }
}

pub fn label_end_constraint(e: EndConstraint) -> &'static str {
    match e {
        EndConstraint::Free => "Free",
        EndConstraint::OneEnd => "One End",
        EndConstraint::BothEnds => "Both Ends",
    }
}

pub fn cycle_cs_mode(m: CsMode) -> CsMode {
    match m {
        CsMode::DepthAngle => CsMode::DiaAngle,
        CsMode::DiaAngle => CsMode::DiaDepth,
        CsMode::DiaDepth => CsMode::DepthAngle,
    }
}

pub fn label_cs_mode(m: CsMode) -> &'static str {
    match m {
        CsMode::DepthAngle => "Depth+Angle -> Dia",
        CsMode::DiaAngle => "Dia+Angle -> Depth",
        CsMode::DiaDepth => "Dia+Depth -> Angle",
    }
}

/// Which physical quantity a numeric row edits - every variant maps 1:1
/// onto a `BushingInputs` field (see this module's own doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberTarget {
    BoreDia,
    BoreTolPlus,
    BoreTolMinus,
    IdBushing,
    Interference,
    InterferenceTolPlus,
    InterferenceTolMinus,
    HousingLen,
    HousingWidth,
    EdgeDist,
    Friction,
    DeltaT,
    MinWallStraight,
    EdgeLoadAngleDeg,
    Load,
    FlangeOd,
    FlangeThk,
    MinWallNeck,
    CsDia,
    CsDepth,
    CsAngle,
    CsDiaTolPlus,
    CsDiaTolMinus,
    CsDepthTolPlus,
    CsDepthTolMinus,
    CsAngleTolPlus,
    CsAngleTolMinus,
    ExtCsDia,
    ExtCsDepth,
    ExtCsAngle,
    ExtCsDiaTolPlus,
    ExtCsDiaTolMinus,
    ExtCsDepthTolPlus,
    ExtCsDepthTolMinus,
    ExtCsAngleTolPlus,
    ExtCsAngleTolMinus,
    MaxBoreNominalShift,
    BoreCapabilityMinWidth,
    AssemblyHousingTemp,
    AssemblyBushingTemp,
}

impl NumberTarget {
    pub fn label(self) -> &'static str {
        match self {
            NumberTarget::BoreDia => "Bore Diameter",
            NumberTarget::BoreTolPlus => "Bore Tol +",
            NumberTarget::BoreTolMinus => "Bore Tol -",
            NumberTarget::IdBushing => "Bushing ID",
            NumberTarget::Interference => "Target Interference",
            NumberTarget::InterferenceTolPlus => "Interference Tol +",
            NumberTarget::InterferenceTolMinus => "Interference Tol -",
            NumberTarget::HousingLen => "Housing Length",
            NumberTarget::HousingWidth => "Housing Width",
            NumberTarget::EdgeDist => "Edge Distance",
            NumberTarget::Friction => "Friction Coefficient",
            NumberTarget::DeltaT => "Service Temp Change",
            NumberTarget::MinWallStraight => "Min Straight Wall",
            NumberTarget::EdgeLoadAngleDeg => "Edge Load Angle",
            NumberTarget::Load => "Applied Edge Load",
            NumberTarget::FlangeOd => "Flange OD",
            NumberTarget::FlangeThk => "Flange Thickness",
            NumberTarget::MinWallNeck => "Min Neck Wall",
            NumberTarget::CsDia => "Internal CS Diameter",
            NumberTarget::CsDepth => "Internal CS Depth",
            NumberTarget::CsAngle => "Internal CS Angle",
            NumberTarget::CsDiaTolPlus => "Internal CS Dia Tol +",
            NumberTarget::CsDiaTolMinus => "Internal CS Dia Tol -",
            NumberTarget::CsDepthTolPlus => "Internal CS Depth Tol +",
            NumberTarget::CsDepthTolMinus => "Internal CS Depth Tol -",
            NumberTarget::CsAngleTolPlus => "Internal CS Angle Tol +",
            NumberTarget::CsAngleTolMinus => "Internal CS Angle Tol -",
            NumberTarget::ExtCsDia => "External CS Diameter",
            NumberTarget::ExtCsDepth => "External CS Depth",
            NumberTarget::ExtCsAngle => "External CS Angle",
            NumberTarget::ExtCsDiaTolPlus => "External CS Dia Tol +",
            NumberTarget::ExtCsDiaTolMinus => "External CS Dia Tol -",
            NumberTarget::ExtCsDepthTolPlus => "External CS Depth Tol +",
            NumberTarget::ExtCsDepthTolMinus => "External CS Depth Tol -",
            NumberTarget::ExtCsAngleTolPlus => "External CS Angle Tol +",
            NumberTarget::ExtCsAngleTolMinus => "External CS Angle Tol -",
            NumberTarget::MaxBoreNominalShift => "Max Bore Nominal Shift",
            NumberTarget::BoreCapabilityMinWidth => "Bore Capability Min Width",
            NumberTarget::AssemblyHousingTemp => "Assembly Housing Temp",
            NumberTarget::AssemblyBushingTemp => "Assembly Bushing Temp",
        }
    }

    pub(super) fn is_angle(self) -> bool {
        matches!(
            self,
            NumberTarget::CsAngle | NumberTarget::CsAngleTolPlus | NumberTarget::CsAngleTolMinus | NumberTarget::ExtCsAngle | NumberTarget::ExtCsAngleTolPlus | NumberTarget::ExtCsAngleTolMinus | NumberTarget::EdgeLoadAngleDeg
        )
    }

    fn is_temperature(self) -> bool {
        matches!(self, NumberTarget::DeltaT | NumberTarget::AssemblyHousingTemp | NumberTarget::AssemblyBushingTemp)
    }

    pub fn format_value(self, value: f64) -> String {
        match self {
            NumberTarget::Friction => format!("{value:.3}"),
            NumberTarget::Load => format!("{value:.1} lbf"),
            _ if self.is_angle() => format!("{value:.3} deg"),
            _ if self.is_temperature() => format!("{value:+.1} \u{b0}F"),
            _ => format!("{value:.4} in"),
        }
    }
}

/// A UI-level classification/preset, not a new solver input -
/// `bushing_solver::solve` already fully models the underlying mechanics
/// (positive `interference` = press/shrink; near-zero/negative = clearance/
/// slip) via `interference`/`assembly_thermal_enabled`. See
/// `cycle_fit_type`'s doc comment for what each variant actually changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FitType {
    #[default]
    Press,
    Shrink,
    Clearance,
    Slip,
}

pub fn cycle_fit_type(t: FitType) -> FitType {
    match t {
        FitType::Press => FitType::Shrink,
        FitType::Shrink => FitType::Clearance,
        FitType::Clearance => FitType::Slip,
        FitType::Slip => FitType::Press,
    }
}

pub fn label_fit_type(t: FitType) -> &'static str {
    match t {
        FitType::Press => "Press Fit",
        FitType::Shrink => "Shrink Fit",
        FitType::Clearance => "Clearance Fit",
        FitType::Slip => "Slip Fit",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRow {
    /// A non-selectable section divider - navigation skips over it (see
    /// `BushingState::move_selection`). Purely organizational: splits the
    /// ~45-field list into the same logical groups an engineer would fill
    /// out on a paper bushing-fit worksheet, so the list reads as a form
    /// with sections rather than one long undifferentiated column.
    Header(&'static str),
    ToggleFitType,
    ToggleBushingType,
    ToggleIdType,
    ToggleEndConstraint,
    ToggleCsMode,
    ToggleExtCsMode,
    ToggleEnforcementEnabled,
    ToggleLockBore,
    TogglePreserveBoreNominal,
    ToggleAllowBoreNominalShift,
    ToggleAssemblyThermalEnabled,
    OpenHousingMaterialPicker,
    OpenBushingMaterialPicker,
    Number(NumberTarget),
}

pub fn row_label(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(text) => text,
        FieldRow::ToggleFitType => "Fit Type",
        FieldRow::ToggleBushingType => "OD Geometry",
        FieldRow::ToggleIdType => "ID Geometry",
        FieldRow::ToggleEndConstraint => "End Constraint",
        FieldRow::ToggleCsMode => "Internal CS Mode",
        FieldRow::ToggleExtCsMode => "External CS Mode",
        FieldRow::ToggleEnforcementEnabled => "Strict Interference Enforcement",
        FieldRow::ToggleLockBore => "Bore Locked (Reamer-Fixed)",
        FieldRow::TogglePreserveBoreNominal => "Preserve Bore Nominal",
        FieldRow::ToggleAllowBoreNominalShift => "Allow Bore Nominal Shift",
        FieldRow::ToggleAssemblyThermalEnabled => "Install Thermal Assist",
        FieldRow::OpenHousingMaterialPicker => "Housing Material",
        FieldRow::OpenBushingMaterialPicker => "Bushing Material",
        FieldRow::Number(target) => target.label(),
    }
}

/// One-line description shown in the bottom "Hint" panel while this row is
/// selected - orientation for a field whose name alone (e.g. "Lambda"
/// wouldn't be, but "Edge Load Angle" still benefits from stating units/
/// defaults/consequences) doesn't fully explain what it does or how it's
/// used downstream. `Header` rows are never selectable, so they have no
/// hint.
pub fn field_hint(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(_) => "",
        FieldRow::ToggleFitType => "Selecting a fit type loads its typical Target Interference and tolerance band for the current bore (Press ~0.003 x D, Shrink ~0.005 x D plus Install Thermal Assist, Clearance/Slip negative). Everything stays editable afterward; a mismatch between fit type and interference sign is flagged in Results.",
        FieldRow::Number(NumberTarget::BoreDia) => "Housing bore nominal diameter. Enter opens the aircraft reamer catalog to pick a real reamed size; press 'm' inside that picker to type an exact value instead.",
        FieldRow::Number(NumberTarget::BoreTolPlus) | FieldRow::Number(NumberTarget::BoreTolMinus) => "Bore tolerance band. A band wider than the interference tolerance band makes the fit Infeasible (see Tolerance status in Results).",
        FieldRow::Number(NumberTarget::IdBushing) => "Bushing inner (through) diameter - the finished bore the installed part/shaft actually uses. Enter opens a user-saved library of past values ('n' saves the current one); 'm' inside it types an exact value instead.",
        FieldRow::Number(NumberTarget::Interference) => "Target nominal diametral interference (Bore - Bushing OD, negative). Drives contact pressure and every downstream stress/margin.",
        FieldRow::Number(NumberTarget::InterferenceTolPlus) | FieldRow::Number(NumberTarget::InterferenceTolMinus) => "Interference tolerance band - must be at least as wide as the bore tolerance band for a feasible fit.",
        FieldRow::Number(NumberTarget::HousingLen) => "Housing length along the bushing axis - drives install force, axial stress scaling, and the edge-distance sequencing thickness.",
        FieldRow::Number(NumberTarget::HousingWidth) => "Available surrounding housing material width - bounds the finite-plate stress correction (psi/lambda in the Numbers panel).",
        FieldRow::Number(NumberTarget::EdgeDist) => "Distance from bore center to the nearest free edge - compared against the sequencing/strength minimums in the Edge Distance results.",
        FieldRow::OpenHousingMaterialPicker => "Housing material - drives modulus, yield strength, and thermal expansion for the outer (housing) region.",
        FieldRow::OpenBushingMaterialPicker => "Bushing material - drives modulus, yield strength, and thermal expansion for the inner (bushing) region.",
        FieldRow::Number(NumberTarget::Friction) => "Installation friction coefficient between bushing OD and housing bore - drives install/retained force. Enter opens a list of typical values with usage notes; 'm' inside it types an exact value instead. Still editable afterward like any other number.",
        FieldRow::Number(NumberTarget::DeltaT) => "In-service uniform temperature change from the install condition - adds a thermal interference delta from the two materials' differing expansion.",
        FieldRow::ToggleEndConstraint => "How the bushing is axially restrained - governs how much of the hoop stress converts into an estimated axial stress (Free = none).",
        FieldRow::Number(NumberTarget::MinWallStraight) => "Minimum acceptable straight-section wall thickness - Straight Wall in Results fails below this.",
        FieldRow::Number(NumberTarget::EdgeLoadAngleDeg) => "Angle of the applied edge load relative to the bore axis - shallower angles demand more edge distance.",
        FieldRow::Number(NumberTarget::Load) => "Applied edge load used by the edge-distance strength check.",
        FieldRow::ToggleBushingType => "Outer-diameter geometry: Straight (uniform OD), Flanged (adds a flange beyond the housing), or Countersink (OD chamfer cut into the housing end).",
        FieldRow::Number(NumberTarget::FlangeOd) | FieldRow::Number(NumberTarget::FlangeThk) => "Flange geometry - extends axially beyond the housing only, does not thin the in-housing wall.",
        FieldRow::Number(NumberTarget::MinWallNeck) => "Minimum acceptable neck wall thickness once countersink/flange geometry is accounted for - equals the straight wall minimum when neither ID nor OD is countersunk.",
        FieldRow::ToggleIdType => "Inner-diameter geometry: Straight bore, or Countersink (a chamfer cut into the bushing ID, thinning the neck wall at that end).",
        FieldRow::ToggleCsMode | FieldRow::ToggleExtCsMode => "Which two countersink dimensions are the direct inputs - the third is solved from the other two plus the base diameter it's cut into.",
        FieldRow::Number(NumberTarget::CsDia) | FieldRow::Number(NumberTarget::ExtCsDia) => "Countersink diameter at its widest point.",
        FieldRow::Number(NumberTarget::CsDepth) | FieldRow::Number(NumberTarget::ExtCsDepth) => "Countersink depth along the bore axis.",
        FieldRow::Number(NumberTarget::CsAngle) | FieldRow::Number(NumberTarget::ExtCsAngle) => "Full included countersink angle.",
        FieldRow::Number(NumberTarget::CsDiaTolPlus)
        | FieldRow::Number(NumberTarget::CsDiaTolMinus)
        | FieldRow::Number(NumberTarget::CsDepthTolPlus)
        | FieldRow::Number(NumberTarget::CsDepthTolMinus)
        | FieldRow::Number(NumberTarget::CsAngleTolPlus)
        | FieldRow::Number(NumberTarget::CsAngleTolMinus)
        | FieldRow::Number(NumberTarget::ExtCsDiaTolPlus)
        | FieldRow::Number(NumberTarget::ExtCsDiaTolMinus)
        | FieldRow::Number(NumberTarget::ExtCsDepthTolPlus)
        | FieldRow::Number(NumberTarget::ExtCsDepthTolMinus)
        | FieldRow::Number(NumberTarget::ExtCsAngleTolPlus)
        | FieldRow::Number(NumberTarget::ExtCsAngleTolMinus) => "Tolerance on this countersink dimension - only meaningful when it's a direct input for the current mode; feeds the worst-case neck-wall corner search either way.",
        FieldRow::ToggleEnforcementEnabled => "When enabled, an infeasible bore/interference fit is auto-tightened toward feasibility instead of just being reported Infeasible.",
        FieldRow::ToggleLockBore => "A reamer-fixed bore can't be auto-tightened at all - enforcement is blocked with a note instead.",
        FieldRow::TogglePreserveBoreNominal => "Keep the entered bore nominal fixed while tightening the band, rather than letting the nominal shift.",
        FieldRow::ToggleAllowBoreNominalShift => "Allow the bore nominal itself to shift (up to Max Bore Nominal Shift) when tightening isn't enough alone.",
        FieldRow::Number(NumberTarget::MaxBoreNominalShift) => "Maximum amount the bore nominal is allowed to shift during enforcement.",
        FieldRow::Number(NumberTarget::BoreCapabilityMinWidth) => "Process-capability floor on the bore's achievable tolerance width - 0 means no floor is enforced.",
        FieldRow::ToggleAssemblyThermalEnabled => "Model a shrink-fit install assist (e.g. chilling the bushing or heating the housing) as a separate install-time temperature, distinct from the in-service Service Temp Change above.",
        FieldRow::Number(NumberTarget::AssemblyHousingTemp) | FieldRow::Number(NumberTarget::AssemblyBushingTemp) => "Absolute part temperature at the moment of installation (reference is 70 deg F).",
    }
}

/// The complete navigable row list for the current geometry/enforcement
/// selection - only fields relevant to the current mode are ever shown,
/// same "only the mode's direct inputs are editable" discipline
/// `toolboxes/fastener_hole/model.rs::field_rows` established. Grouped
/// under `Header` rows so the ~40+ possible fields read as a sectioned
/// form (Bore & Fit / Housing Geometry / Materials & Friction /
/// Installation / OD Geometry / ID Geometry / external-countersink-only /
/// Tolerance Enforcement / Install Thermal Assist) rather than one long
/// undifferentiated list.
pub fn field_rows(model: &BushingModel) -> Vec<FieldRow> {
    let mut rows = vec![
        FieldRow::Header("Bore & Fit"),
        FieldRow::ToggleFitType,
        FieldRow::Number(NumberTarget::BoreDia),
        FieldRow::Number(NumberTarget::BoreTolPlus),
        FieldRow::Number(NumberTarget::BoreTolMinus),
        FieldRow::Number(NumberTarget::IdBushing),
        FieldRow::Number(NumberTarget::Interference),
        FieldRow::Number(NumberTarget::InterferenceTolPlus),
        FieldRow::Number(NumberTarget::InterferenceTolMinus),
        FieldRow::Header("Housing Geometry"),
        FieldRow::Number(NumberTarget::HousingLen),
        FieldRow::Number(NumberTarget::HousingWidth),
        FieldRow::Number(NumberTarget::EdgeDist),
        FieldRow::Header("Materials & Friction"),
        FieldRow::OpenHousingMaterialPicker,
        FieldRow::OpenBushingMaterialPicker,
        FieldRow::Number(NumberTarget::Friction),
        FieldRow::Number(NumberTarget::DeltaT),
        FieldRow::Header("Installation"),
        FieldRow::ToggleEndConstraint,
        FieldRow::Number(NumberTarget::MinWallStraight),
        FieldRow::Number(NumberTarget::EdgeLoadAngleDeg),
        FieldRow::Number(NumberTarget::Load),
        FieldRow::Header("OD Geometry"),
        FieldRow::ToggleBushingType,
    ];
    if model.bushing_type == BushingType::Flanged {
        rows.push(FieldRow::Number(NumberTarget::FlangeOd));
        rows.push(FieldRow::Number(NumberTarget::FlangeThk));
    }
    rows.push(FieldRow::Number(NumberTarget::MinWallNeck));
    rows.push(FieldRow::Header("ID Geometry"));
    rows.push(FieldRow::ToggleIdType);
    if model.id_type == IdType::Countersink {
        rows.push(FieldRow::ToggleCsMode);
        if model.cs_mode != CsMode::DepthAngle {
            rows.push(FieldRow::Number(NumberTarget::CsDia));
            rows.push(FieldRow::Number(NumberTarget::CsDiaTolPlus));
            rows.push(FieldRow::Number(NumberTarget::CsDiaTolMinus));
        }
        if model.cs_mode != CsMode::DiaAngle {
            rows.push(FieldRow::Number(NumberTarget::CsDepth));
            rows.push(FieldRow::Number(NumberTarget::CsDepthTolPlus));
            rows.push(FieldRow::Number(NumberTarget::CsDepthTolMinus));
        }
        if model.cs_mode != CsMode::DiaDepth {
            rows.push(FieldRow::Number(NumberTarget::CsAngle));
            rows.push(FieldRow::Number(NumberTarget::CsAngleTolPlus));
            rows.push(FieldRow::Number(NumberTarget::CsAngleTolMinus));
        }
    }
    if model.bushing_type == BushingType::Countersink {
        rows.push(FieldRow::Header("External Countersink"));
        rows.push(FieldRow::ToggleExtCsMode);
        if model.ext_cs_mode != CsMode::DepthAngle {
            rows.push(FieldRow::Number(NumberTarget::ExtCsDia));
            rows.push(FieldRow::Number(NumberTarget::ExtCsDiaTolPlus));
            rows.push(FieldRow::Number(NumberTarget::ExtCsDiaTolMinus));
        }
        if model.ext_cs_mode != CsMode::DiaAngle {
            rows.push(FieldRow::Number(NumberTarget::ExtCsDepth));
            rows.push(FieldRow::Number(NumberTarget::ExtCsDepthTolPlus));
            rows.push(FieldRow::Number(NumberTarget::ExtCsDepthTolMinus));
        }
        if model.ext_cs_mode != CsMode::DiaDepth {
            rows.push(FieldRow::Number(NumberTarget::ExtCsAngle));
            rows.push(FieldRow::Number(NumberTarget::ExtCsAngleTolPlus));
            rows.push(FieldRow::Number(NumberTarget::ExtCsAngleTolMinus));
        }
    }
    rows.push(FieldRow::Header("Tolerance Enforcement"));
    rows.push(FieldRow::ToggleEnforcementEnabled);
    if model.enforcement_enabled {
        rows.push(FieldRow::ToggleLockBore);
        rows.push(FieldRow::TogglePreserveBoreNominal);
        rows.push(FieldRow::ToggleAllowBoreNominalShift);
        if model.allow_bore_nominal_shift {
            rows.push(FieldRow::Number(NumberTarget::MaxBoreNominalShift));
        }
        rows.push(FieldRow::Number(NumberTarget::BoreCapabilityMinWidth));
    }
    rows.push(FieldRow::Header("Install Thermal Assist"));
    rows.push(FieldRow::ToggleAssemblyThermalEnabled);
    if model.assembly_thermal_enabled {
        rows.push(FieldRow::Number(NumberTarget::AssemblyHousingTemp));
        rows.push(FieldRow::Number(NumberTarget::AssemblyBushingTemp));
    }
    rows
}

/// The whole toolbox's engineering state, plus every derived result -
/// recomputed fresh on every field mutation via [`BushingModel::recompute`],
/// mirroring `PressureVesselModel`'s own discipline. Defaults reproduce
/// `bushing_solver::solve`'s own differential-tested fixture
/// (`tests/differential.rs`'s base input), not an arbitrary guess.
#[derive(Clone)]
pub struct BushingModel {
    pub fit_type: FitType,
    pub bore_dia: f64,
    pub bore_tol_plus: f64,
    pub bore_tol_minus: f64,
    pub id_bushing: f64,
    pub interference: f64,
    pub interference_tol_plus: f64,
    pub interference_tol_minus: f64,
    pub housing_len: f64,
    pub housing_width: f64,
    pub edge_dist: f64,
    pub housing_material_index: usize,
    pub bushing_material_index: usize,
    pub friction: f64,
    pub delta_t: f64,
    pub end_constraint: EndConstraint,
    pub min_wall_straight: f64,
    pub edge_load_angle_deg: f64,
    pub load: f64,

    pub bushing_type: BushingType,
    pub id_type: IdType,
    pub flange_od: f64,
    pub flange_thk: f64,
    pub min_wall_neck: f64,

    pub cs_mode: CsMode,
    pub cs_dia: f64,
    pub cs_depth: f64,
    pub cs_angle: f64,
    pub cs_dia_tol_plus: f64,
    pub cs_dia_tol_minus: f64,
    pub cs_depth_tol_plus: f64,
    pub cs_depth_tol_minus: f64,
    pub cs_angle_tol_plus: f64,
    pub cs_angle_tol_minus: f64,

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

    pub enforcement_enabled: bool,
    pub lock_bore: bool,
    pub preserve_bore_nominal: bool,
    pub allow_bore_nominal_shift: bool,
    pub max_bore_nominal_shift: f64,
    pub bore_capability_min_width: f64,

    pub assembly_thermal_enabled: bool,
    pub assembly_housing_temp: f64,
    pub assembly_bushing_temp: f64,

    /// User-added/imported materials, appended to the built-in `MATERIALS`
    /// table by [`BushingModel::material_catalog`] - `housing_material_index`/
    /// `bushing_material_index` index into that combined sequence, built-ins
    /// first. Loaded from `material_persistence.rs` at construction, same
    /// pattern `PressureVesselModel::custom_materials` already established.
    pub custom_materials: Vec<&'static Material>,

    pub output: BushingOutput,

    /// Pass/warn/fail per check and verified fixes for whatever fails -
    /// refreshed by every `recompute` (see `advice.rs`).
    pub checks: Vec<super::advice::Check>,
    pub recommendations: Vec<super::advice::Recommendation>,
    /// `toggle_fit_type` switched Install Thermal Assist on for Shrink Fit;
    /// leaving Shrink Fit switches it back off only in that case.
    thermal_enabled_by_fit: bool,
}

impl Default for BushingModel {
    fn default() -> Self {
        let mut model = Self {
            fit_type: FitType::default(),
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
            housing_material_index: MATERIALS.iter().position(|m| m.id == "al7075").unwrap_or(0),
            bushing_material_index: MATERIALS.iter().position(|m| m.id == "bronze").unwrap_or(0),
            friction: 0.15,
            delta_t: 0.0,
            end_constraint: EndConstraint::Free,
            min_wall_straight: 0.05,
            edge_load_angle_deg: 40.0,
            load: 1000.0,
            bushing_type: BushingType::Straight,
            id_type: IdType::Straight,
            flange_od: 0.75,
            flange_thk: 0.06,
            min_wall_neck: 0.05,
            cs_mode: CsMode::default(),
            cs_dia: 0.5,
            cs_depth: 0.125,
            cs_angle: 100.0,
            cs_dia_tol_plus: 0.0,
            cs_dia_tol_minus: 0.0,
            cs_depth_tol_plus: 0.0,
            cs_depth_tol_minus: 0.0,
            cs_angle_tol_plus: 0.0,
            cs_angle_tol_minus: 0.0,
            ext_cs_mode: CsMode::default(),
            ext_cs_dia: 0.6,
            ext_cs_depth: 0.06,
            ext_cs_angle: 100.0,
            ext_cs_dia_tol_plus: 0.0,
            ext_cs_dia_tol_minus: 0.0,
            ext_cs_depth_tol_plus: 0.0,
            ext_cs_depth_tol_minus: 0.0,
            ext_cs_angle_tol_plus: 0.0,
            ext_cs_angle_tol_minus: 0.0,
            enforcement_enabled: false,
            lock_bore: true,
            preserve_bore_nominal: true,
            allow_bore_nominal_shift: false,
            max_bore_nominal_shift: 0.0,
            bore_capability_min_width: 0.0,
            assembly_thermal_enabled: false,
            assembly_housing_temp: 70.0,
            assembly_bushing_temp: 70.0,
            custom_materials: super::material_persistence::load()
                .into_iter()
                .map(|li| leak_custom_material(li.item.name, li.item.e_ksi, li.item.sy_ksi, li.item.fbru_ksi, li.item.fsu_ksi, li.item.ftu_ksi, li.item.nu, li.item.alpha_u_f))
                .collect(),
            output: compute(&BushingInputs::default()),
            checks: Vec::new(),
            recommendations: Vec::new(),
            thermal_enabled_by_fit: false,
        };
        model.recompute();
        model
    }
}

impl BushingModel {
    /// Built-in materials first, then user-added/imported custom ones, in
    /// the order they were added - `housing_material_index`/
    /// `bushing_material_index` are indices into exactly this sequence.
    pub fn material_catalog(&self) -> Vec<&'static Material> {
        MATERIALS.iter().chain(self.custom_materials.iter().copied()).collect()
    }

    pub fn housing_material(&self) -> &'static Material {
        self.material_catalog().get(self.housing_material_index).copied().unwrap_or(&MATERIALS[0])
    }

    pub fn bushing_material(&self) -> &'static Material {
        self.material_catalog().get(self.bushing_material_index).copied().unwrap_or(&MATERIALS[0])
    }

    pub fn select_housing_material(&mut self, index: usize) {
        if index < self.material_catalog().len() {
            self.housing_material_index = index;
            self.recompute();
        }
    }

    pub fn select_bushing_material(&mut self, index: usize) {
        if index < self.material_catalog().len() {
            self.bushing_material_index = index;
            self.recompute();
        }
    }

    /// Rebuilds `custom_materials` entirely from `library` (fresh leak per
    /// entry) - called after an import merges new/overwritten entries into
    /// `MaterialPickerState::library`, since a leaked `&'static Material`
    /// can't be mutated in place to reflect an overwrite. Leaking is
    /// bounded by how often a user actually imports a library file in a
    /// session, not by anything that scales with runtime.
    pub fn sync_custom_materials_from_library(&mut self, library: &[crate::library::LibraryItem<super::material_persistence::PersistedMaterial>]) {
        self.custom_materials = library.iter().map(|li| leak_custom_material(li.item.name.clone(), li.item.e_ksi, li.item.sy_ksi, li.item.fbru_ksi, li.item.fsu_ksi, li.item.ftu_ksi, li.item.nu, li.item.alpha_u_f)).collect();
        self.housing_material_index = self.housing_material_index.min(self.material_catalog().len().saturating_sub(1));
        self.bushing_material_index = self.bushing_material_index.min(self.material_catalog().len().saturating_sub(1));
        self.recompute();
    }

    /// Adds a user-entered material to the catalog and selects it into
    /// `target` immediately - validation (non-empty name, finite/positive
    /// numeric fields) is the caller's job
    /// (`material_picker.rs::AddMaterialForm::validate`), not this
    /// method's; it trusts its inputs, same discipline
    /// `PressureVesselModel::add_custom_material` documents.
    pub fn add_custom_material(&mut self, target: super::material_picker::MaterialTarget, name: String, e_ksi: f64, sy_ksi: f64, fbru_ksi: f64, fsu_ksi: f64, ftu_ksi: f64, nu: f64, alpha_u_f: f64) {
        let material = leak_custom_material(name, e_ksi, sy_ksi, fbru_ksi, fsu_ksi, ftu_ksi, nu, alpha_u_f);
        self.custom_materials.push(material);
        let index = self.material_catalog().len() - 1;
        match target {
            super::material_picker::MaterialTarget::Housing => self.housing_material_index = index,
            super::material_picker::MaterialTarget::Bushing => self.bushing_material_index = index,
        }
        self.recompute();
    }

    fn build_inputs(&self) -> BushingInputs {
        BushingInputs {
            bore_dia: self.bore_dia,
            bore_tol_plus: self.bore_tol_plus,
            bore_tol_minus: self.bore_tol_minus,
            id_bushing: self.id_bushing,
            interference: self.interference,
            interference_tol_plus: self.interference_tol_plus,
            interference_tol_minus: self.interference_tol_minus,
            housing_len: self.housing_len,
            housing_width: self.housing_width,
            edge_dist: self.edge_dist,
            mat_housing: *self.housing_material(),
            mat_bushing: *self.bushing_material(),
            friction: Some(self.friction),
            d_t: self.delta_t,
            end_constraint: self.end_constraint,
            min_wall_straight: self.min_wall_straight,
            edge_load_angle_deg: Some(self.edge_load_angle_deg),
            load: Some(self.load),
            bushing_type: self.bushing_type,
            id_type: self.id_type,
            flange_od: self.flange_od,
            flange_thk: self.flange_thk,
            min_wall_neck: self.min_wall_neck,
            cs_mode: self.cs_mode,
            cs_dia: self.cs_dia,
            cs_depth: self.cs_depth,
            cs_angle: self.cs_angle,
            cs_dia_tol_plus: self.cs_dia_tol_plus,
            cs_dia_tol_minus: self.cs_dia_tol_minus,
            cs_depth_tol_plus: self.cs_depth_tol_plus,
            cs_depth_tol_minus: self.cs_depth_tol_minus,
            cs_angle_tol_plus: self.cs_angle_tol_plus,
            cs_angle_tol_minus: self.cs_angle_tol_minus,
            ext_cs_mode: self.ext_cs_mode,
            ext_cs_dia: self.ext_cs_dia,
            ext_cs_depth: self.ext_cs_depth,
            ext_cs_angle: self.ext_cs_angle,
            ext_cs_dia_tol_plus: self.ext_cs_dia_tol_plus,
            ext_cs_dia_tol_minus: self.ext_cs_dia_tol_minus,
            ext_cs_depth_tol_plus: self.ext_cs_depth_tol_plus,
            ext_cs_depth_tol_minus: self.ext_cs_depth_tol_minus,
            ext_cs_angle_tol_plus: self.ext_cs_angle_tol_plus,
            ext_cs_angle_tol_minus: self.ext_cs_angle_tol_minus,
            enforcement: EnforcementPolicy {
                enabled: self.enforcement_enabled,
                lock_bore: self.lock_bore,
                preserve_bore_nominal: self.preserve_bore_nominal,
                allow_bore_nominal_shift: self.allow_bore_nominal_shift,
                max_bore_nominal_shift: self.max_bore_nominal_shift,
            },
            bore_capability: (self.bore_capability_min_width > 0.0).then_some(BoreCapability { min_achievable_tol_width: Some(self.bore_capability_min_width) }),
            assembly_housing_temperature: self.assembly_thermal_enabled.then_some(self.assembly_housing_temp),
            assembly_bushing_temperature: self.assembly_thermal_enabled.then_some(self.assembly_bushing_temp),
        }
    }

    /// Solver only - no advice. What trial evaluations use (the advice
    /// search itself calls this, so it must not recurse into `recompute`).
    fn recompute_output(&mut self) {
        self.output = compute(&self.build_inputs());
    }

    pub fn recompute(&mut self) {
        self.recompute_output();
        self.checks = super::advice::evaluate(self);
        self.recommendations = super::advice::recommend(self);
    }

    /// A scratch copy with `edits` applied and the solver re-run - never
    /// touches `self`, never recurses into the advice search.
    pub(super) fn trial(&self, edits: &[super::advice::Edit]) -> BushingModel {
        let mut copy = self.clone();
        copy.checks = Vec::new();
        copy.recommendations = Vec::new();
        for e in edits {
            copy.set_number_raw(e.target, e.value);
        }
        copy.recompute_output();
        copy
    }

    /// Applies a recommendation's edits through the normal commit path and
    /// returns a one-line description of what changed, or `None` for an
    /// advice-only recommendation.
    pub fn apply_recommendation(&mut self, index: usize) -> Option<String> {
        let rec = self.recommendations.get(index)?.clone();
        if !rec.is_applicable() {
            return None;
        }
        for e in &rec.edits {
            self.set_number_raw(e.target, e.value);
        }
        self.recompute();
        Some(rec.summary)
    }

    /// Typical `(target interference, tol +, tol -)` for the selected fit
    /// type at the current bore diameter (inches). Positive interference is
    /// press/shrink, negative is clearance. The interference band is never
    /// narrower than the bore band, so a preset is never itself Infeasible.
    pub fn fit_type_preset(&self) -> (f64, f64, f64) {
        let d = self.bore_dia.abs();
        let round4 = |v: f64| (v * 10_000.0).round() / 10_000.0;
        let (interference, min_tol): (f64, f64) = match self.fit_type {
            FitType::Press => (round4(0.003 * d), 0.0005),
            FitType::Shrink => (round4(0.005 * d), 0.0005),
            FitType::Clearance => (-round4((0.004 * d).max(0.001)), 0.0005),
            FitType::Slip => (-round4((0.001 * d).max(0.0005)), 0.0003),
        };
        (interference, min_tol.max(self.bore_tol_plus), min_tol.max(self.bore_tol_minus))
    }

    pub fn number_value(&self, target: NumberTarget) -> f64 {
        match target {
            NumberTarget::BoreDia => self.bore_dia,
            NumberTarget::BoreTolPlus => self.bore_tol_plus,
            NumberTarget::BoreTolMinus => self.bore_tol_minus,
            NumberTarget::IdBushing => self.id_bushing,
            NumberTarget::Interference => self.interference,
            NumberTarget::InterferenceTolPlus => self.interference_tol_plus,
            NumberTarget::InterferenceTolMinus => self.interference_tol_minus,
            NumberTarget::HousingLen => self.housing_len,
            NumberTarget::HousingWidth => self.housing_width,
            NumberTarget::EdgeDist => self.edge_dist,
            NumberTarget::Friction => self.friction,
            NumberTarget::DeltaT => self.delta_t,
            NumberTarget::MinWallStraight => self.min_wall_straight,
            NumberTarget::EdgeLoadAngleDeg => self.edge_load_angle_deg,
            NumberTarget::Load => self.load,
            NumberTarget::FlangeOd => self.flange_od,
            NumberTarget::FlangeThk => self.flange_thk,
            NumberTarget::MinWallNeck => self.min_wall_neck,
            NumberTarget::CsDia => self.cs_dia,
            NumberTarget::CsDepth => self.cs_depth,
            NumberTarget::CsAngle => self.cs_angle,
            NumberTarget::CsDiaTolPlus => self.cs_dia_tol_plus,
            NumberTarget::CsDiaTolMinus => self.cs_dia_tol_minus,
            NumberTarget::CsDepthTolPlus => self.cs_depth_tol_plus,
            NumberTarget::CsDepthTolMinus => self.cs_depth_tol_minus,
            NumberTarget::CsAngleTolPlus => self.cs_angle_tol_plus,
            NumberTarget::CsAngleTolMinus => self.cs_angle_tol_minus,
            NumberTarget::ExtCsDia => self.ext_cs_dia,
            NumberTarget::ExtCsDepth => self.ext_cs_depth,
            NumberTarget::ExtCsAngle => self.ext_cs_angle,
            NumberTarget::ExtCsDiaTolPlus => self.ext_cs_dia_tol_plus,
            NumberTarget::ExtCsDiaTolMinus => self.ext_cs_dia_tol_minus,
            NumberTarget::ExtCsDepthTolPlus => self.ext_cs_depth_tol_plus,
            NumberTarget::ExtCsDepthTolMinus => self.ext_cs_depth_tol_minus,
            NumberTarget::ExtCsAngleTolPlus => self.ext_cs_angle_tol_plus,
            NumberTarget::ExtCsAngleTolMinus => self.ext_cs_angle_tol_minus,
            NumberTarget::MaxBoreNominalShift => self.max_bore_nominal_shift,
            NumberTarget::BoreCapabilityMinWidth => self.bore_capability_min_width,
            NumberTarget::AssemblyHousingTemp => self.assembly_housing_temp,
            NumberTarget::AssemblyBushingTemp => self.assembly_bushing_temp,
        }
    }

    /// Commits one edited numeric field. A non-finite result is silently
    /// ignored, leaving the previous value in place - same convention every
    /// other toolbox in this crate uses.
    pub fn commit_number(&mut self, target: NumberTarget, raw: f64) {
        if !raw.is_finite() {
            return;
        }
        self.set_number_raw(target, raw);
        self.recompute();
    }

    fn set_number_raw(&mut self, target: NumberTarget, raw: f64) {
        match target {
            NumberTarget::BoreDia => self.bore_dia = raw,
            NumberTarget::BoreTolPlus => self.bore_tol_plus = raw,
            NumberTarget::BoreTolMinus => self.bore_tol_minus = raw,
            NumberTarget::IdBushing => self.id_bushing = raw,
            NumberTarget::Interference => self.interference = raw,
            NumberTarget::InterferenceTolPlus => self.interference_tol_plus = raw,
            NumberTarget::InterferenceTolMinus => self.interference_tol_minus = raw,
            NumberTarget::HousingLen => self.housing_len = raw,
            NumberTarget::HousingWidth => self.housing_width = raw,
            NumberTarget::EdgeDist => self.edge_dist = raw,
            NumberTarget::Friction => self.friction = raw,
            NumberTarget::DeltaT => self.delta_t = raw,
            NumberTarget::MinWallStraight => self.min_wall_straight = raw,
            NumberTarget::EdgeLoadAngleDeg => self.edge_load_angle_deg = raw,
            NumberTarget::Load => self.load = raw,
            NumberTarget::FlangeOd => self.flange_od = raw,
            NumberTarget::FlangeThk => self.flange_thk = raw,
            NumberTarget::MinWallNeck => self.min_wall_neck = raw,
            NumberTarget::CsDia => self.cs_dia = raw,
            NumberTarget::CsDepth => self.cs_depth = raw,
            NumberTarget::CsAngle => self.cs_angle = raw,
            NumberTarget::CsDiaTolPlus => self.cs_dia_tol_plus = raw,
            NumberTarget::CsDiaTolMinus => self.cs_dia_tol_minus = raw,
            NumberTarget::CsDepthTolPlus => self.cs_depth_tol_plus = raw,
            NumberTarget::CsDepthTolMinus => self.cs_depth_tol_minus = raw,
            NumberTarget::CsAngleTolPlus => self.cs_angle_tol_plus = raw,
            NumberTarget::CsAngleTolMinus => self.cs_angle_tol_minus = raw,
            NumberTarget::ExtCsDia => self.ext_cs_dia = raw,
            NumberTarget::ExtCsDepth => self.ext_cs_depth = raw,
            NumberTarget::ExtCsAngle => self.ext_cs_angle = raw,
            NumberTarget::ExtCsDiaTolPlus => self.ext_cs_dia_tol_plus = raw,
            NumberTarget::ExtCsDiaTolMinus => self.ext_cs_dia_tol_minus = raw,
            NumberTarget::ExtCsDepthTolPlus => self.ext_cs_depth_tol_plus = raw,
            NumberTarget::ExtCsDepthTolMinus => self.ext_cs_depth_tol_minus = raw,
            NumberTarget::ExtCsAngleTolPlus => self.ext_cs_angle_tol_plus = raw,
            NumberTarget::ExtCsAngleTolMinus => self.ext_cs_angle_tol_minus = raw,
            NumberTarget::MaxBoreNominalShift => self.max_bore_nominal_shift = raw,
            NumberTarget::BoreCapabilityMinWidth => self.bore_capability_min_width = raw,
            NumberTarget::AssemblyHousingTemp => self.assembly_housing_temp = raw,
            NumberTarget::AssemblyBushingTemp => self.assembly_bushing_temp = raw,
        }
    }

    /// Shrink Fit turns on `assembly_thermal_enabled` (surfacing Install
    /// Thermal Assist's own fields in `field_rows`) - the solver's existing
    /// thermally-assisted-install modeling *is* what a shrink fit means.
    /// Press/Clearance/Slip are pure relabeling - see `FitType`'s own doc
    /// comment for why this is a UI-level preset, not a new solver input.
    pub fn toggle_fit_type(&mut self) {
        let previous = self.fit_type;
        self.fit_type = cycle_fit_type(self.fit_type);
        if self.fit_type == FitType::Shrink {
            if !self.assembly_thermal_enabled {
                self.assembly_thermal_enabled = true;
                self.thermal_enabled_by_fit = true;
            }
        } else if previous == FitType::Shrink && self.thermal_enabled_by_fit {
            self.assembly_thermal_enabled = false;
            self.thermal_enabled_by_fit = false;
        }
        // A fit type is defined by its interference: selecting one loads the
        // typical target interference and tolerance band for the current
        // bore (all still editable afterwards).
        let (interference, tol_plus, tol_minus) = self.fit_type_preset();
        self.interference = interference;
        self.interference_tol_plus = tol_plus;
        self.interference_tol_minus = tol_minus;
        self.recompute();
    }

    pub fn toggle_bushing_type(&mut self) {
        self.bushing_type = cycle_bushing_type(self.bushing_type);
        self.recompute();
    }

    pub fn toggle_id_type(&mut self) {
        self.id_type = cycle_id_type(self.id_type);
        self.recompute();
    }

    pub fn toggle_end_constraint(&mut self) {
        self.end_constraint = cycle_end_constraint(self.end_constraint);
        self.recompute();
    }

    pub fn toggle_cs_mode(&mut self) {
        self.cs_mode = cycle_cs_mode(self.cs_mode);
        self.recompute();
    }

    pub fn toggle_ext_cs_mode(&mut self) {
        self.ext_cs_mode = cycle_cs_mode(self.ext_cs_mode);
        self.recompute();
    }

    pub fn toggle_enforcement_enabled(&mut self) {
        self.enforcement_enabled = !self.enforcement_enabled;
        self.recompute();
    }

    pub fn toggle_lock_bore(&mut self) {
        self.lock_bore = !self.lock_bore;
        self.recompute();
    }

    pub fn toggle_preserve_bore_nominal(&mut self) {
        self.preserve_bore_nominal = !self.preserve_bore_nominal;
        self.recompute();
    }

    pub fn toggle_allow_bore_nominal_shift(&mut self) {
        self.allow_bore_nominal_shift = !self.allow_bore_nominal_shift;
        self.recompute();
    }

    pub fn toggle_assembly_thermal_enabled(&mut self) {
        self.assembly_thermal_enabled = !self.assembly_thermal_enabled;
        self.recompute();
    }

    /// Selecting a reamer auto-populates Bore Tolerance +/- from the
    /// reamer's own tool tolerance, not just the nominal bore diameter -
    /// a real installed bore's achievable tolerance band comes from the
    /// reamer that cut it, not a separately-guessed value.
    pub fn select_reamer(&mut self, entry: &ReamerEntry) {
        self.bore_dia = entry.nominal_in;
        self.bore_tol_plus = entry.tool_tolerance_plus_in;
        self.bore_tol_minus = entry.tool_tolerance_minus_in;
        self.recompute();
    }
}

/// Trims a fixed-decimal formatted number for the edit buffer - same helper
/// every other toolbox in this crate provides as its own small copy.
pub fn format_for_edit(value: f64) -> String {
    let s = format!("{value:.6}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() { "0".to_string() } else { trimmed.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_matches_the_bushing_solver_differential_fixture() {
        let model = BushingModel::default();
        assert_eq!(model.bore_dia, 0.5);
        assert_eq!(model.id_bushing, 0.375);
        assert_eq!(model.interference, 0.0015);
        assert_eq!(model.housing_len, 0.5);
        assert_eq!(model.housing_width, 1.5);
        assert_eq!(model.edge_dist, 0.75);
        assert_eq!(model.housing_material().id, "al7075");
        assert_eq!(model.bushing_material().id, "bronze");
        assert_eq!(model.friction, 0.15);
    }

    #[test]
    fn default_model_produces_positive_pressure_and_opposite_signed_hoop_stresses() {
        let model = BushingModel::default();
        assert!(model.output.pressure > 0.0);
        assert!(model.output.stress_hoop_housing > 0.0);
        assert!(model.output.stress_hoop_bushing < 0.0);
    }

    #[test]
    fn commit_number_updates_field_and_recomputes() {
        let mut model = BushingModel::default();
        model.commit_number(NumberTarget::Interference, 0.0);
        assert_eq!(model.interference, 0.0);
        assert_eq!(model.output.pressure, 0.0);
    }

    #[test]
    fn commit_number_silently_ignores_a_non_finite_result() {
        let mut model = BushingModel::default();
        let before = model.bore_dia;
        model.commit_number(NumberTarget::BoreDia, f64::NAN);
        assert_eq!(model.bore_dia, before);
    }

    #[test]
    fn toggle_fit_type_cycles_through_all_four_variants_and_wraps() {
        let mut model = BushingModel::default();
        assert_eq!(model.fit_type, FitType::Press);
        model.toggle_fit_type();
        assert_eq!(model.fit_type, FitType::Shrink);
        model.toggle_fit_type();
        assert_eq!(model.fit_type, FitType::Clearance);
        model.toggle_fit_type();
        assert_eq!(model.fit_type, FitType::Slip);
        model.toggle_fit_type();
        assert_eq!(model.fit_type, FitType::Press);
    }

    #[test]
    fn shrink_fit_turns_on_assembly_thermal_assist() {
        let mut model = BushingModel::default();
        assert!(!model.assembly_thermal_enabled);
        model.toggle_fit_type(); // Press -> Shrink
        assert_eq!(model.fit_type, FitType::Shrink);
        assert!(model.assembly_thermal_enabled);
    }

    #[test]
    fn changing_fit_type_loads_that_fits_interference_and_tolerances() {
        let mut model = BushingModel::default();
        // Press at the default 0.5 in bore reproduces the solver fixture's own 0.0015 in.
        assert!((model.interference - 0.0015).abs() < 1e-9);
        let press = model.interference;

        model.toggle_fit_type(); // Shrink
        let shrink = model.interference;
        assert!(shrink > press, "shrink fit carries more interference than press fit");
        assert!(model.interference_tol_plus > 0.0 && model.interference_tol_minus > 0.0);

        model.toggle_fit_type(); // Clearance
        assert!(model.interference < 0.0, "clearance fit is negative interference");
        let clearance = model.interference;

        model.toggle_fit_type(); // Slip
        assert!(model.interference < 0.0 && model.interference > clearance, "slip is a tighter clearance than clearance fit");

        model.toggle_fit_type(); // back to Press
        assert!((model.interference - press).abs() < 1e-9);
        assert!(!model.assembly_thermal_enabled, "leaving Shrink undoes the thermal assist Shrink itself enabled");
    }

    #[test]
    fn fit_type_presets_scale_with_bore_and_stay_feasible_for_any_bore_tolerance() {
        let mut model = BushingModel::default();
        model.commit_number(NumberTarget::BoreDia, 1.0);
        model.commit_number(NumberTarget::BoreTolPlus, 0.002);
        model.commit_number(NumberTarget::BoreTolMinus, 0.001);
        for _ in 0..4 {
            model.toggle_fit_type();
            assert_ne!(model.output.tolerance_status, bushing_solver::tolerance::ToleranceStatus::Infeasible, "{:?}", model.fit_type);
            assert!(model.output.pressure.is_finite() && model.output.install_force.is_finite(), "{:?}", model.fit_type);
        }
        model.commit_number(NumberTarget::BoreDia, 1.0);
        model.toggle_fit_type();
        assert!(model.interference.abs() > 0.0015, "preset scales with bore diameter");
    }

    #[test]
    fn a_manually_entered_thermal_assist_survives_leaving_shrink_fit() {
        let mut model = BushingModel::default();
        model.toggle_assembly_thermal_enabled();
        assert!(model.assembly_thermal_enabled);
        model.toggle_fit_type(); // Shrink: already on, not enabled by the fit
        model.toggle_fit_type(); // Clearance
        assert!(model.assembly_thermal_enabled, "the user turned it on, the fit type must not turn it off");
    }

    #[test]
    fn toggle_bushing_type_cycles_through_all_three_variants() {
        let mut model = BushingModel::default();
        assert_eq!(model.bushing_type, BushingType::Straight);
        model.toggle_bushing_type();
        assert_eq!(model.bushing_type, BushingType::Flanged);
        model.toggle_bushing_type();
        assert_eq!(model.bushing_type, BushingType::Countersink);
        model.toggle_bushing_type();
        assert_eq!(model.bushing_type, BushingType::Straight);
    }

    #[test]
    fn field_rows_shows_flange_fields_only_when_flanged() {
        let mut model = BushingModel::default();
        assert!(!field_rows(&model).contains(&FieldRow::Number(NumberTarget::FlangeOd)));
        model.bushing_type = BushingType::Flanged;
        assert!(field_rows(&model).contains(&FieldRow::Number(NumberTarget::FlangeOd)));
    }

    #[test]
    fn field_rows_hides_the_derived_countersink_dimension() {
        let mut model = BushingModel::default();
        model.id_type = IdType::Countersink;
        model.cs_mode = CsMode::DepthAngle;
        let rows = field_rows(&model);
        assert!(!rows.contains(&FieldRow::Number(NumberTarget::CsDia)), "dia is derived in DepthAngle mode");
        assert!(rows.contains(&FieldRow::Number(NumberTarget::CsDepth)));
        assert!(rows.contains(&FieldRow::Number(NumberTarget::CsAngle)));
    }

    #[test]
    fn field_rows_hides_enforcement_sub_fields_until_enabled() {
        let mut model = BushingModel::default();
        assert!(!field_rows(&model).contains(&FieldRow::ToggleLockBore));
        model.enforcement_enabled = true;
        assert!(field_rows(&model).contains(&FieldRow::ToggleLockBore));
    }

    #[test]
    fn field_rows_hides_assembly_thermal_fields_until_enabled() {
        let mut model = BushingModel::default();
        assert!(!field_rows(&model).contains(&FieldRow::Number(NumberTarget::AssemblyHousingTemp)));
        model.assembly_thermal_enabled = true;
        assert!(field_rows(&model).contains(&FieldRow::Number(NumberTarget::AssemblyHousingTemp)));
    }

    #[test]
    fn select_housing_material_switches_and_recomputes() {
        let mut model = BushingModel::default();
        let steel_index = MATERIALS.iter().position(|m| m.id == "steel").unwrap();
        model.select_housing_material(steel_index);
        assert_eq!(model.housing_material().id, "steel");
    }

    #[test]
    fn select_material_out_of_range_is_ignored() {
        let mut model = BushingModel::default();
        let before = model.housing_material_index;
        model.select_housing_material(9999);
        assert_eq!(model.housing_material_index, before);
    }

    #[test]
    fn select_reamer_sets_bore_dia_and_recomputes() {
        let mut model = BushingModel::default();
        let reamer = &bushing_solver::reamers::all_reamers()[0];
        let target = reamer.nominal_in;
        model.select_reamer(reamer);
        assert_eq!(model.bore_dia, target);
    }

    #[test]
    fn assembly_thermal_assist_matches_the_bushing_solver_install_state_physics() {
        // Same golden fixture `bushing-solver/src/solve.rs`'s own
        // `assembly_temperature_assist_matches_real_ts_install_state_physics`
        // test proves against the real TS engine - proof this UI bridge
        // wires the assembly-thermal-assist fields through correctly, not
        // just that the underlying crate is correct in isolation.
        let mut model = BushingModel::default();
        model.assembly_thermal_enabled = true;
        model.assembly_housing_temp = 70.0;
        model.assembly_bushing_temp = -20.0;
        model.recompute();
        assert!((model.output.assembly_thermal_delta - (-0.000405)).abs() < 1e-9);
        assert!((model.output.install_force - 756.3063714026035).abs() < 1e-6);
    }

    #[test]
    fn format_for_edit_round_trips_through_parse() {
        let s = format_for_edit(0.25);
        assert_eq!(s.parse::<f64>().unwrap(), 0.25);
        assert_eq!(format_for_edit(0.0), "0");
    }
}
