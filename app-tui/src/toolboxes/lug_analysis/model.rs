//! UI-facing state and the (pure) analysis pipeline of the Lug Analysis
//! toolbox. All mechanics are in `lug-solver`; this module only turns the
//! editable fields into a solver input, runs it (reusing the condensed model
//! when only the pin or load changed) and compares the result with the
//! material allowables.

use lug_solver::fea::FeaLug;
use lug_solver::{auto_refinement_for, BushingSpec, FiniteLug, FsMaterial, FsOptions, FsResult, Hardening, LimitLoad, LimitOptions, LoadCase, LugGeometry, LugModel, LugSolution, Material as FeMaterial, MeshSpec, PinBending, PinBody, PinSpec, PlaneMode, Shear, Thermal, ThicknessResult};
use mechanics_core::fracture::Basis;
use mechanics_core::materials::{fbru_at_edge_ratio, Material};
use std::sync::Arc;

use crate::toolboxes::material_lookup::model::catalog;

/// Mesh density preset: elements around the full bore at the coarse (far) spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshDensity {
    Coarse,
    Normal,
    Fine,
}

impl MeshDensity {
    pub fn elements_around(self) -> usize {
        match self {
            MeshDensity::Coarse => 48,
            MeshDensity::Normal => 72,
            MeshDensity::Fine => 120,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MeshDensity::Coarse => "Coarse (48 around the bore)",
            MeshDensity::Normal => "Normal (72 around the bore)",
            MeshDensity::Fine => "Fine (120 around the bore)",
        }
    }

    pub fn next(self) -> MeshDensity {
        match self {
            MeshDensity::Coarse => MeshDensity::Normal,
            MeshDensity::Normal => MeshDensity::Fine,
            MeshDensity::Fine => MeshDensity::Coarse,
        }
    }
}

/// Which finite-element solver runs the analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolverChoice {
    /// The general kernel (`fea-core`): the main solver. Every analysis the toolbox offers runs on it.
    Kernel,
    /// The legacy condensed / finite-strain solvers of `lug-solver`, kept as a comparison.
    Legacy,
    /// Both: the kernel's answer is shown with the legacy solver's beside it.
    Compare,
}

impl SolverChoice {
    pub fn next(self) -> Self {
        match self {
            SolverChoice::Kernel => SolverChoice::Legacy,
            SolverChoice::Legacy => SolverChoice::Compare,
            SolverChoice::Compare => SolverChoice::Kernel,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SolverChoice::Kernel => "General kernel (fea-core)",
            SolverChoice::Legacy => "Legacy condensed solver",
            SolverChoice::Compare => "Compare kernel with legacy",
        }
    }
}

/// The material numbers the analysis uses (psi), copied out of the catalog so a
/// worker thread owns them.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialLimits {
    pub name: String,
    pub e_psi: f64,
    pub nu: f64,
    pub ftu_psi: f64,
    pub fty_psi: f64,
    pub fsu_psi: f64,
    pub fbru_psi: f64,
    pub fbru_e15_psi: f64,
    /// Linear thermal expansion coefficient (per deg F).
    pub alpha: f64,
    /// Handbook elongation as a fraction (0 = not tabulated).
    pub elong: f64,
    /// Library failure strain for the finite-strain collapse and how it was obtained (`None` for a
    /// user-added material).
    pub failure: Option<(f64, &'static str)>,
}

impl MaterialLimits {
    /// Flow stress of the perfectly plastic ultimate analysis: the mean of yield and
    /// ultimate, capped by `sqrt(3) Fsu`, never below yield - the rule `edge-check`'s contact
    /// FE uses and validates against NACA TN 1503 (one authoritative copy).
    pub fn flow_stress(&self) -> f64 {
        ::edge_check::models::contact_model::flow_stress(&::edge_check::types::Strengths { e: self.e_psi, nu: self.nu, sy: self.fty_psi, fsu: if self.fsu_psi > 0.0 { self.fsu_psi } else { 0.6 * self.ftu_psi }, ftu: self.ftu_psi, fbru: self.fbru_psi, fbru_e15: self.fbru_e15_psi })
    }

    /// Flow stress of the perfectly plastic collapse under `rule` (`elongation` is the fraction used
    /// by [`FlowRule::TrueUltimate`]). Never below yield.
    pub fn flow_for(&self, rule: FlowRule, elongation: f64) -> f64 {
        let v = match rule {
            FlowRule::Mean => return self.flow_stress(),
            FlowRule::Ultimate => self.ftu_psi,
            FlowRule::TrueUltimate => self.ftu_psi * (1.0 + elongation.max(0.0)),
        };
        v.max(self.fty_psi)
    }

    pub fn from_material(m: &Material) -> Self {
        Self { name: m.name.to_string(), e_psi: m.e_ksi * 1000.0, nu: m.nu, ftu_psi: m.ftu_ksi * 1000.0, fty_psi: m.sy_ksi * 1000.0, fsu_psi: m.fsu_ksi * 1000.0, fbru_psi: m.fbru_ksi * 1000.0, fbru_e15_psi: m.fbru_e15_ksi * 1000.0, alpha: m.alpha_u_f * 1e-6, elong: m.extra.map_or(0.0, |x| x.elong.l.max(x.elong.lt)) / 100.0, failure: ::mechanics_core::fracture::failure_strain(m.id).filter(|f| f.basis != Basis::NotApplicable).map(|f| (f.value, basis_label(f.basis))) }
    }
}

/// Short label of how a library failure strain was obtained.
pub fn basis_label(b: Basis) -> &'static str {
    match b {
        Basis::Validated => "validated against NACA pin-bearing tests",
        Basis::Family => "alloy-family multiplier from tensile reduction of area, not bearing-validated",
        Basis::Bound => "lower bound ln(1 + elongation), no family evidence",
        Basis::Default => "default: the material has no tabulated elongation",
        Basis::Capped => "capped at 0.50 (strains above 0.4 are unvalidated)",
        Basis::Typical => "typical elongation of the condition it stands for",
        Basis::NotApplicable => "not applicable",
    }
}

/// How the flow-stress rules did on the twelve NACA TN 1503 points, per solver (`lug-solver/tests/
/// validation_naca_tn1503.rs` for the legacy solver, `tests/kernel_naca.rs` for the kernel, whose fully converged
/// collapse sits about 3 % above the legacy one).
pub fn naca_statistics(solver: SolverChoice) -> &'static str {
    if solver == SolverChoice::Legacy {
        "(Ftu + Fty)/2 was 9 to 28 % low (mean 18 %), Ftu 1 to 17 % low (mean 10 %, never high), Ftu (1 + elongation) -5 to +10 % (mean 4.3 %)"
    } else {
        "(Ftu + Fty)/2 was 6 to 25 % low (mean 15 %), Ftu from 13 % low to 2 % high (mean 7 % low), Ftu (1 + elongation) -2 to +14 % (mean +5 %)"
    }
}

/// A bushing pressed into the lug hole.
#[derive(Debug, Clone, PartialEq)]
pub struct BushingInput {
    pub inner_dia: f64,
    /// Diametral interference: bushing outer diameter minus hole diameter.
    pub interference_dia: f64,
    /// Coulomb friction between bushing and hole.
    pub friction: f64,
    pub material: MaterialLimits,
}

impl BushingInput {
    pub fn spec(&self) -> BushingSpec {
        BushingSpec { inner_dia: self.inner_dia, material: FeMaterial { e_psi: self.material.e_psi, nu: self.material.nu }, interference_dia: self.interference_dia, friction: self.friction }
    }
}

/// The flow stress of the perfectly plastic collapse. The pin-bearing tests of NACA TN 1503
/// (`lug-solver/tests/validation_naca_tn1503.rs`) give, over twelve points: `Mean` (Ftu + Fty)/2
/// 9-28 % low (mean 18 %); `Ultimate` Ftu 1-17 % low (mean 10 %, never high); `TrueUltimate` (legacy solver; the kernel's, `naca_statistics`, are about 3 % higher)
/// Ftu (1 + elongation) -5 to +10 % (mean 4.3 %), high for the brittle 75S bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowRule {
    Mean,
    Ultimate,
    TrueUltimate,
}

impl FlowRule {
    pub fn next(self) -> Self {
        match self {
            FlowRule::Mean => FlowRule::Ultimate,
            FlowRule::Ultimate => FlowRule::TrueUltimate,
            FlowRule::TrueUltimate => FlowRule::Mean,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FlowRule::Mean => "(Ftu + Fty)/2",
            FlowRule::Ultimate => "Ftu",
            FlowRule::TrueUltimate => "Ftu (1 + elongation)",
        }
    }
}

/// What the pin is made of: rigid (never meshed) or an elastic disc of a common pin material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinBodyChoice {
    Rigid,
    Steel,
    Titanium,
}

impl PinBodyChoice {
    pub fn next(self) -> Self {
        match self {
            PinBodyChoice::Rigid => PinBodyChoice::Steel,
            PinBodyChoice::Steel => PinBodyChoice::Titanium,
            PinBodyChoice::Titanium => PinBodyChoice::Rigid,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PinBodyChoice::Rigid => "Rigid (analytic circle)",
            PinBodyChoice::Steel => "Elastic steel (E 29000 ksi)",
            PinBodyChoice::Titanium => "Elastic titanium (E 16000 ksi)",
        }
    }

    /// `(E psi, nu, expansion per deg F)`; `None` for the rigid pin.
    pub fn material(self) -> Option<(f64, f64, f64)> {
        match self {
            PinBodyChoice::Rigid => None,
            PinBodyChoice::Steel => Some((29.0e6, 0.30, 6.5e-6)),
            PinBodyChoice::Titanium => Some((16.0e6, 0.34, 4.9e-6)),
        }
    }
}

/// Everything one analysis depends on; two equal inputs give the same result.
#[derive(Debug, Clone, PartialEq)]
pub struct LugInput {
    pub geometry: LugGeometry,
    pub material: MaterialLimits,
    pub pin: PinSpec,
    pub case: LoadCase,
    pub mesh: MeshSpec,
    pub bushing: Option<BushingInput>,
    /// Also run the plane-strain elastic-perfectly-plastic collapse (ultimate capacity).
    pub plastic: bool,
    pub flow_rule: FlowRule,
    /// Elongation (fraction) of the true-ultimate rule: the handbook value, else 10 %.
    pub elongation: f64,
    /// Finite-strain collapse (true stress-strain, large deformation): the equivalent plastic strain
    /// at which the lug is taken to fail. `None` = the perfectly plastic collapse under `flow_rule`.
    pub ductility: Option<f64>,
    /// The failure strain was typed by the user rather than taken from the library.
    pub failure_overridden: bool,
    /// Second-order (P-delta) elastic analysis.
    pub second_order: bool,
    /// Through-thickness pin bending in double shear: the clevis load centroid's distance (in)
    /// from the lug face.
    pub pin_bending: Option<f64>,
    pub solver: SolverChoice,
    /// Refine the bore mesh around the loaded sector of a loose pin (on top of `mesh`).
    pub auto_refine: bool,
}

/// What the expensive condensed model depends on (not the pin, friction or load size).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelKey {
    geometry: LugGeometry,
    material: FeMaterial,
    mesh: MeshSpec,
    symmetric: bool,
    bushing: Option<BushingSpec>,
}

/// A built model kept between runs so an unchanged geometry skips meshing, factorising and
/// condensing. The kernel's lugs and the legacy condensed one are built on first use.
#[derive(Clone)]
pub struct CachedModel {
    pub key: ModelKey,
    /// The legacy condensed lug (also needed by the pin-bending analysis).
    pub model: Option<Arc<LugModel>>,
    /// The legacy plane-strain twin used for the plastic collapse, built on first use.
    pub plastic_model: Option<Arc<LugModel>>,
    /// The kernel's plane-stress lug (with the geometric-nonlinearity switch it was built for).
    pub kernel: Option<(bool, Arc<FeaLug>)>,
    /// The kernel's plane-strain twin for the collapse.
    pub kernel_plastic: Option<Arc<FeaLug>>,
    /// Collapse results with what they depend on (they do not depend on the applied load).
    pub limits: Vec<(LimitKey, Arc<LimitLoad>)>,
    /// Finite-strain collapse results (also independent of the applied load).
    pub finites: Vec<(FiniteKey, Arc<FsResult>)>,
}

impl CachedModel {
    fn new(key: ModelKey) -> Self {
        Self { key, model: None, plastic_model: None, kernel: None, kernel_plastic: None, limits: Vec::new(), finites: Vec::new() }
    }
}

/// Everything a collapse load depends on besides the geometry/mesh in [`ModelKey`].
#[derive(Debug, Clone, PartialEq)]
pub struct LimitKey {
    pin: PinSpec,
    angle_deg: f64,
    flow_stress: f64,
    kernel: bool,
}

/// Everything a finite-strain collapse depends on besides the geometry/mesh in [`ModelKey`].
#[derive(Debug, Clone, PartialEq)]
pub struct FiniteKey {
    pin_dia: f64,
    friction: f64,
    angle_deg: f64,
    failure_strain: f64,
    elongation: f64,
    fty: f64,
    ftu: f64,
    mesh_around: usize,
    kernel: bool,
}

impl std::fmt::Debug for CachedModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CachedModel({:?})", self.key.geometry)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Info,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub detail: String,
    /// Margin of safety (`allowable / applied - 1`) where the check has one.
    pub margin: Option<f64>,
    pub status: Status,
}

#[derive(Debug, Clone)]
pub struct LugRun {
    pub input: LugInput,
    pub solution: LugSolution,
    pub checks: Vec<Check>,
    pub notes: Vec<String>,
    /// Elastic-perfectly-plastic collapse (plane strain), when requested.
    pub limit: Option<Arc<LimitLoad>>,
    /// Finite-strain collapse, when requested (and supported for this case).
    pub finite: Option<Arc<FsResult>>,
    /// The collapse came from the cache (only the applied load changed).
    pub limit_reused: bool,
    /// The condensed model came from the cache.
    pub reused_model: bool,
    /// Through-thickness pin bending and the hoop stress at its most loaded slice, when asked for.
    pub thickness: Option<(ThicknessResult, f64)>,
    pub total_ms: f64,
    /// The solver that produced `solution` (a kernel failure falls back to the legacy solver).
    pub solver: SolverChoice,
    /// The legacy solver's answer for the same inputs, when the Compare solver is chosen.
    pub comparison: Option<Box<Comparison>>,
}

/// The legacy solver's result beside the kernel's (Solver: Compare).
#[derive(Debug, Clone)]
pub struct Comparison {
    pub solution: LugSolution,
    pub limit: Option<Arc<LimitLoad>>,
    pub finite: Option<Arc<FsResult>>,
    /// Wall time of the legacy analysis.
    pub ms: f64,
}

// ---------------------------------------------------------------------------
// Editable fields
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberTarget {
    HoleDia,
    Width,
    Edge,
    HeadCorner,
    Thickness,
    ModelLength,
    PinDia,
    Friction,
    Load,
    LoadAngle,
    BushingId,
    BushingInterference,
    BushingFriction,
    TemperatureChange,
    Ductility,
    ClevisOffset,
    Elongation,
    ElementsAround,
    MaxGrowth,
    FirstLayerAspect,
}

impl NumberTarget {
    pub fn label(self) -> &'static str {
        match self {
            NumberTarget::HoleDia => "Hole Diameter D",
            NumberTarget::Width => "Lug Width W",
            NumberTarget::Edge => "Edge Distance e",
            NumberTarget::HeadCorner => "Head Corner Radius",
            NumberTarget::Thickness => "Thickness t",
            NumberTarget::ModelLength => "Model Length L",
            NumberTarget::PinDia => "Pin Diameter",
            NumberTarget::Friction => "Friction Coefficient",
            NumberTarget::Load => "Pin Load P",
            NumberTarget::LoadAngle => "Load Angle",
            NumberTarget::BushingId => "Bushing Inner Dia",
            NumberTarget::BushingInterference => "Bushing Interference",
            NumberTarget::BushingFriction => "Bushing Fit Friction",
            NumberTarget::TemperatureChange => "Temperature Change",
            NumberTarget::Ductility => "Failure Strain",
            NumberTarget::ClevisOffset => "Clevis Load Offset",
            NumberTarget::Elongation => "Elongation",
            NumberTarget::ElementsAround => "Elements Around Bore",
            NumberTarget::MaxGrowth => "Max Growth Ratio",
            NumberTarget::FirstLayerAspect => "First Layer Aspect",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            NumberTarget::HoleDia => "Bore diameter of the lug.",
            NumberTarget::Width => "Shank width. Net section is (W - D) t on each side of the hole.",
            NumberTarget::Edge => "Distance from the hole centre to the end of the head (e/D is the usual edge ratio).",
            NumberTarget::HeadCorner => "Radius of the two corners at the end of the head; half the width with e = W/2 is a full-round head.",
            NumberTarget::Thickness => "Lug thickness (plane stress through the thickness).",
            NumberTarget::ModelLength => "Finite-element model only: distance from the hole centre to the clamped far end. Long enough (about 2.5 W or more) that the clamp does not affect the hole.",
            NumberTarget::PinDia => "Rigid pin (never meshed). Smaller than the hole is clearance, larger is interference.",
            NumberTarget::Friction => "Coulomb friction between pin and bore; typically 0.15 to 0.3 for metal joints.",
            NumberTarget::Load => "Total pin load (lbf).",
            NumberTarget::BushingId => "Inner diameter of the bushing: the surface the pin bears on. The hole diameter above is its outer seat.",
            NumberTarget::BushingInterference => "Diametral interference: bushing outer diameter minus hole diameter (positive is a press fit, negative a clearance). Typical 0.0005 to 0.002 in.",
            NumberTarget::BushingFriction => "Coulomb friction between the bushing and the hole; typically 0.15 to 0.3.",
            NumberTarget::TemperatureChange => "Uniform temperature change from the fit temperature (deg F). Free thermal expansion changes the fit only: a steel bushing in aluminium loses interference when heated. Structural restraint of the lug is not modelled.",
            NumberTarget::Ductility => "Equivalent plastic strain at which the finite-strain collapse ends. Starts from the material library (m ln(1 + elongation); aluminium 2014/2024/7075 validated against the NACA TN 1503 and TN 1502 bearing tests within -9 to +3 %, other families from tensile reduction of area, the rest a lower bound; the evidence line is in the Notes). Type a value to override.",
            NumberTarget::Elongation => "Elongation at fracture used by the true-ultimate flow rule, Ftu (1 + elongation): the true stress at large strain. Starts from the handbook value when the material has one, else 10 %.",
            NumberTarget::ClevisOffset => "Double shear: distance from the lug face to the load centroid of each clevis ear (about half the ear thickness). It sets the pin's bending moment.",
            NumberTarget::LoadAngle => "Load direction measured from the lug axis: 0 pulls the pin away from the shank (tension), 90 is transverse, 180 pushes toward the shank.",
            NumberTarget::ElementsAround => "Elements around the full bore circle at the far spacing (12 to 240). More is slower and converges the stress peaks; use the mesh test below for a recommendation.",
            NumberTarget::MaxGrowth => "Largest layer-to-layer size growth from the bore outward (1.05 to 2). Lower is smoother and adds layers.",
            NumberTarget::FirstLayerAspect => "Radial over tangential size of the first element layer at the bore (0.3 to 3). Smaller is thinner next to the bore.",
        }
    }

    pub fn format_value(self, v: f64) -> String {
        match self {
            NumberTarget::Friction | NumberTarget::BushingFriction => format!("{v:.3}"),
            NumberTarget::BushingInterference => format!("{v:+.4} in"),
            NumberTarget::Load => format!("{v:.1} lbf"),
            NumberTarget::LoadAngle => format!("{v:.1} deg"),
            NumberTarget::TemperatureChange => format!("{v:+.0} F"),
            NumberTarget::Ductility | NumberTarget::Elongation => format!("{:.1} %", v * 100.0),
            NumberTarget::ElementsAround => format!("{v:.0}"),
            NumberTarget::MaxGrowth | NumberTarget::FirstLayerAspect => format!("{v:.2}"),
            _ => format!("{v:.4} in"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRow {
    Header(&'static str),
    Number(NumberTarget),
    ToggleHeadShape,
    OpenMaterialPicker,
    ToggleMeshDensity,
    TogglePlastic,
    ToggleBushing,
    OpenBushingMaterialPicker,
    TogglePinBody,
    ToggleFiniteStrain,
    ToggleSecondOrder,
    TogglePinBending,
    ToggleFlowRule,
    /// The collapsible mesh section's header (Enter, Space or a click opens / closes it).
    MeshSection,
    ToggleAutoRefine,
    /// Runs the brief mesh-size test; with a result, applies the recommendation.
    MeshAdvice,
    ToggleSolver,
}

pub fn row_label(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(t) => t,
        FieldRow::Number(t) => t.label(),
        FieldRow::ToggleHeadShape => "Head Shape",
        FieldRow::OpenMaterialPicker => "Lug Material",
        FieldRow::ToggleMeshDensity => "Mesh Density",
        FieldRow::TogglePlastic => "Plastic Limit Load",
        FieldRow::ToggleBushing => "Bushing",
        FieldRow::OpenBushingMaterialPicker => "Bushing Material",
        FieldRow::TogglePinBody => "Pin Body",
        FieldRow::ToggleFiniteStrain => "Ultimate Model",
        FieldRow::ToggleSecondOrder => "Geometric Nonlinearity",
        FieldRow::TogglePinBending => "Pin Bending",
        FieldRow::ToggleFlowRule => "Flow Stress Rule",
        FieldRow::MeshSection => "Advanced settings",
        FieldRow::ToggleAutoRefine => "Contact Refinement",
        FieldRow::MeshAdvice => "Mesh Size Test",
        FieldRow::ToggleSolver => "Solver",
    }
}

pub fn field_hint(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(_) => "",
        FieldRow::Number(t) => t.hint(),
        FieldRow::ToggleHeadShape => "Round: the head is a half circle concentric with the hole (e = W/2). Custom: set the edge distance and corner radius yourself.",
        FieldRow::OpenMaterialPicker => "Enter opens the material browser (MIL-HDBK-5J conditions and typical values). E, Fty, Ftu and the bearing allowables come from it.",
        FieldRow::ToggleMeshDensity => "Preset for the elements around the bore at the far spacing (typing a number below overrides it). The loaded sector of a loose pin is refined on top of this.",
        FieldRow::MeshSection => "Solver, plasticity, nonlinearity, pin bending and mesh settings; the defaults suit most lugs. Enter, Space or a click opens and closes the section.",
        FieldRow::ToggleAutoRefine => "On: finer angular spacing around the loaded sector when a loose pin makes a narrow contact patch (Hertz width). Off: the uniform spacing only.",
        FieldRow::MeshAdvice => "Enter runs a brief test: the elastic case at several mesh sizes (frictionless, in parallel), timed and compared, and recommends the coarsest size whose hoop stress, pressure and pin travel have converged. Enter again applies the recommendation.",
        FieldRow::ToggleSolver => "Kernel: the general finite-element kernel (fea-core) runs every analysis. Legacy: the older condensed / finite-strain solvers of lug-solver. Compare: both, the legacy result shown beside the kernel's. A case the kernel cannot run falls back to the legacy solver with a note.",
        FieldRow::ToggleBushing => "A bushing pressed into the hole: the pin then bears on the bushing, whose outer surface meets the hole in a contact with the interference fit and friction (the fit pressure is solved and can be lost). Turning it on sets the pin just under the bushing's inner diameter.",
        FieldRow::OpenBushingMaterialPicker => "Enter opens the material browser for the bushing (elastic in every analysis).",
        FieldRow::ToggleFlowRule => "Flow stress of the perfectly plastic collapse. (Ftu + Fty)/2 is the old rule (15 % low on average on NACA TN 1503, kernel solver). Ftu is 7 % low on average, from 13 % low to 2 % high. Ftu (1 + elongation) is the true stress at large strain: 5 % mean error but up to 14 % high for brittle alloys (75S), so it is not conservative. The note under Results gives the figures of the solver in use.",
        FieldRow::TogglePinBody => "Rigid: the pin is an analytic circle. Elastic: the pin is meshed as a disc, ovalises under the bearing load and spreads it (lowers the edge pressure peak of a loose pin). The collapse load does not depend on it.",
        FieldRow::ToggleFiniteStrain => "Flow stress: a perfectly plastic collapse at the chosen flow stress. Finite strain: a true stress-strain curve (from Fty, Ftu and the elongation), large-deformation kinematics and a failure strain taken from the material library (validated for 2014, 2024 and 7075 aluminium against 20 NACA bearing points, within -9 to +3 %). Axial loads without a bushing only; a few seconds.",
        FieldRow::ToggleSecondOrder => "Add the geometric stiffness of the stress state (P-delta): tension in the lug stiffens it, compression softens it. Matters for slender lugs and oblique loads; costs a rebuild of the model.",
        FieldRow::TogglePinBending => "Double shear: the pin bends between the clevis ears and the lug, so the lug faces carry more bearing load than the middle. Gives the slice distribution, the peaking factor and the pin bending and shear stresses.",
        FieldRow::TogglePlastic => "Also find the collapse load: plane-strain elastic-perfectly-plastic at (Ftu + Fty)/2 (capped by sqrt(3) Fsu), the pin driven until the load peaks. Adds the Ultimate margin; independent of the applied load.",
    }
}

pub fn format_for_edit(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Material used when the toolbox opens: a handbook 7075-T651 plate (it has an
/// e/D = 1.5 bearing value), else the first curated material.
pub fn default_material_index() -> usize {
    catalog().iter().position(|m| m.name.contains("7075") && m.name.contains("T651 Plate") && m.fbru_e15_ksi > 0.0).unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct LugUiModel {
    pub hole_dia: f64,
    pub width: f64,
    pub head_round: bool,
    pub edge: f64,
    pub head_corner: f64,
    pub thickness: f64,
    pub model_length: f64,
    pub material_index: usize,
    pub pin_dia: f64,
    pub friction: f64,
    pub load: f64,
    pub load_angle: f64,
    pub density: MeshDensity,
    /// Typed elements-around override of the density preset.
    pub elements_around: Option<usize>,
    pub max_growth: f64,
    pub first_layer_aspect: f64,
    pub auto_refine: bool,
    /// The collapsible mesh section is open.
    pub mesh_open: bool,
    pub solver: SolverChoice,
    pub plastic: bool,
    pub flow_rule: FlowRule,
    /// Elongation override (fraction); `None` follows the material.
    pub elongation: Option<f64>,
    pub bushing: bool,
    pub bushing_id: f64,
    pub bushing_interference: f64,
    pub bushing_friction: f64,
    pub bushing_material_index: usize,
    pub pin_body: PinBodyChoice,
    /// Temperature change (deg F).
    pub delta_t: f64,
    /// Finite-strain ultimate model (needs Plastic Limit Load).
    pub finite_strain: bool,
    /// Failure strain override (equivalent plastic strain, fraction); `None` follows the library.
    pub failure_strain: Option<f64>,
    pub second_order: bool,
    pub pin_bending: bool,
    /// Clevis ear load centroid from the lug face (in).
    pub clevis_offset: f64,
}

/// Aluminium bronze, the usual lug bushing material.
pub fn default_bushing_material_index() -> usize {
    catalog().iter().position(|m| m.name.contains("Al-Bronze")).unwrap_or(0)
}

impl Default for LugUiModel {
    fn default() -> Self {
        Self {
            hole_dia: 0.5,
            width: 1.5,
            head_round: true,
            edge: 0.75,
            head_corner: 0.75,
            thickness: 0.25,
            model_length: 3.75,
            material_index: default_material_index(),
            pin_dia: 0.499,
            friction: 0.15,
            load: 4000.0,
            load_angle: 0.0,
            density: MeshDensity::Normal,
            elements_around: None,
            max_growth: MeshSpec::default().max_growth,
            first_layer_aspect: MeshSpec::default().first_layer_aspect,
            auto_refine: true,
            mesh_open: false,
            solver: SolverChoice::Kernel,
            plastic: true,
            flow_rule: FlowRule::Ultimate,
            elongation: None,
            bushing: false,
            bushing_id: 0.375,
            bushing_interference: 0.001,
            bushing_friction: 0.2,
            bushing_material_index: default_bushing_material_index(),
            pin_body: PinBodyChoice::Rigid,
            delta_t: 0.0,
            finite_strain: false,
            failure_strain: None,
            second_order: false,
            pin_bending: false,
            clevis_offset: 0.1,
        }
    }
}

/// The rows in display order; the head rows depend on the head shape.
pub fn field_rows(model: &LugUiModel) -> Vec<FieldRow> {
    let mut rows = vec![
        FieldRow::Header("Lug Geometry"),
        FieldRow::Number(NumberTarget::HoleDia),
        FieldRow::Number(NumberTarget::Width),
        FieldRow::ToggleHeadShape,
    ];
    if !model.head_round {
        rows.push(FieldRow::Number(NumberTarget::Edge));
        rows.push(FieldRow::Number(NumberTarget::HeadCorner));
    }
    rows.extend([
        FieldRow::Number(NumberTarget::Thickness),
        FieldRow::Number(NumberTarget::ModelLength),
        FieldRow::Header("Material"),
        FieldRow::OpenMaterialPicker,
        FieldRow::Header("Bushing"),
        FieldRow::ToggleBushing,
    ]);
    if model.bushing {
        rows.extend([
            FieldRow::Number(NumberTarget::BushingId),
            FieldRow::Number(NumberTarget::BushingInterference),
            FieldRow::Number(NumberTarget::BushingFriction),
            FieldRow::OpenBushingMaterialPicker,
        ]);
    }
    rows.extend([
        FieldRow::Header("Pin"),
        FieldRow::TogglePinBody,
        FieldRow::Number(NumberTarget::PinDia),
        FieldRow::Number(NumberTarget::Friction),
        FieldRow::Header("Load"),
        FieldRow::Number(NumberTarget::Load),
        FieldRow::Number(NumberTarget::LoadAngle),
        FieldRow::Number(NumberTarget::TemperatureChange),
        FieldRow::MeshSection,
    ]);
    if model.mesh_open {
        rows.extend([
            FieldRow::ToggleMeshDensity,
            FieldRow::Number(NumberTarget::ElementsAround),
            FieldRow::Number(NumberTarget::MaxGrowth),
            FieldRow::Number(NumberTarget::FirstLayerAspect),
            FieldRow::ToggleAutoRefine,
            FieldRow::MeshAdvice,
        ]);

        rows.extend([FieldRow::Header("Analysis"), FieldRow::ToggleSolver, FieldRow::TogglePlastic]);
        if model.plastic {
            if !model.finite_strain {
                rows.push(FieldRow::ToggleFlowRule);
                if model.flow_rule == FlowRule::TrueUltimate {
                    rows.push(FieldRow::Number(NumberTarget::Elongation));
                }
            }
            rows.push(FieldRow::ToggleFiniteStrain);
            if model.finite_strain {
                rows.push(FieldRow::Number(NumberTarget::Ductility));
                rows.push(FieldRow::Number(NumberTarget::Elongation));
            }
        }
        rows.push(FieldRow::ToggleSecondOrder);
        rows.push(FieldRow::TogglePinBending);
        if model.pin_bending {
            rows.push(FieldRow::Number(NumberTarget::ClevisOffset));
        }
    }
    rows
}

impl LugUiModel {
    pub fn material(&self) -> &'static Material {
        let cat = catalog();
        cat[self.material_index.min(cat.len() - 1)]
    }

    /// Elongation fraction for the true-ultimate rule: the override, else the handbook value, else 10 %.
    pub fn effective_elongation(&self) -> f64 {
        self.elongation.unwrap_or_else(|| {
            let e = MaterialLimits::from_material(self.material()).elong;
            if e > 0.0 { e } else { 0.10 }
        })
    }

    /// Failure strain of the finite-strain collapse: the override, else the library value for the
    /// selected material, else the default for materials the library does not know.
    pub fn effective_failure_strain(&self) -> f64 {
        self.failure_strain.unwrap_or_else(|| MaterialLimits::from_material(self.material()).failure.map_or(::mechanics_core::fracture::DEFAULT_FAILURE_STRAIN, |f| f.0))
    }

    pub fn bushing_material(&self) -> &'static Material {
        let cat = catalog();
        cat[self.bushing_material_index.min(cat.len() - 1)]
    }

    /// Diameter of the surface the pin bears on.
    pub fn contact_dia(&self) -> f64 {
        if self.bushing {
            self.bushing_id
        } else {
            self.hole_dia
        }
    }

    pub fn toggle_bushing(&mut self) {
        self.bushing = !self.bushing;
        // Keep the pin sensible for the surface it now bears on.
        self.pin_dia = ((self.contact_dia() - 0.001) * 10_000.0).round() / 10_000.0;
    }

    pub fn number_value(&self, t: NumberTarget) -> f64 {
        match t {
            NumberTarget::HoleDia => self.hole_dia,
            NumberTarget::Width => self.width,
            NumberTarget::Edge => self.effective_edge(),
            NumberTarget::HeadCorner => self.effective_corner(),
            NumberTarget::Thickness => self.thickness,
            NumberTarget::ModelLength => self.model_length,
            NumberTarget::PinDia => self.pin_dia,
            NumberTarget::Friction => self.friction,
            NumberTarget::Load => self.load,
            NumberTarget::LoadAngle => self.load_angle,
            NumberTarget::BushingId => self.bushing_id,
            NumberTarget::BushingInterference => self.bushing_interference,
            NumberTarget::BushingFriction => self.bushing_friction,
            NumberTarget::TemperatureChange => self.delta_t,
            NumberTarget::Ductility => self.effective_failure_strain(),
            NumberTarget::ClevisOffset => self.clevis_offset,
            NumberTarget::Elongation => self.effective_elongation(),
            NumberTarget::ElementsAround => self.effective_elements_around() as f64,
            NumberTarget::MaxGrowth => self.max_growth,
            NumberTarget::FirstLayerAspect => self.first_layer_aspect,
        }
    }

    /// Elements around the bore: the typed value, else the density preset.
    pub fn effective_elements_around(&self) -> usize {
        self.elements_around.unwrap_or_else(|| self.density.elements_around())
    }

    pub fn effective_edge(&self) -> f64 {
        if self.head_round {
            self.width / 2.0
        } else {
            self.edge
        }
    }

    pub fn effective_corner(&self) -> f64 {
        if self.head_round {
            self.width / 2.0
        } else {
            self.head_corner
        }
    }

    /// Stores a typed value; a non-finite or out-of-range value leaves the old one.
    pub fn commit_number(&mut self, t: NumberTarget, raw: f64) {
        if !raw.is_finite() {
            return;
        }
        match t {
            NumberTarget::HoleDia if raw > 0.0 => self.hole_dia = raw,
            NumberTarget::Width if raw > 0.0 => self.width = raw,
            NumberTarget::Edge if raw > 0.0 => self.edge = raw,
            NumberTarget::HeadCorner if raw >= 0.0 => self.head_corner = raw,
            NumberTarget::Thickness if raw > 0.0 => self.thickness = raw,
            NumberTarget::ModelLength if raw > 0.0 => self.model_length = raw,
            NumberTarget::PinDia if raw > 0.0 => self.pin_dia = raw,
            NumberTarget::Friction if (0.0..=2.0).contains(&raw) => self.friction = raw,
            NumberTarget::Load if raw >= 0.0 => self.load = raw,
            NumberTarget::LoadAngle => self.load_angle = raw.clamp(0.0, 180.0),
            NumberTarget::BushingId if raw > 0.0 => self.bushing_id = raw,
            NumberTarget::BushingInterference if raw.abs() <= 0.02 => self.bushing_interference = raw,
            NumberTarget::BushingFriction if (0.0..=2.0).contains(&raw) => self.bushing_friction = raw,
            NumberTarget::TemperatureChange if raw.abs() <= 1000.0 => self.delta_t = raw,
            NumberTarget::Ductility if (0.005..=0.8).contains(&raw) => self.failure_strain = Some(raw),
            NumberTarget::ClevisOffset if (0.0..=5.0).contains(&raw) => self.clevis_offset = raw,
            NumberTarget::Elongation if (0.0..=1.0).contains(&raw) => self.elongation = Some(raw),
            NumberTarget::ElementsAround if (12.0..=240.0).contains(&raw) => self.elements_around = Some(raw.round() as usize),
            NumberTarget::MaxGrowth if (1.05..=2.0).contains(&raw) => self.max_growth = raw,
            NumberTarget::FirstLayerAspect if (0.3..=3.0).contains(&raw) => self.first_layer_aspect = raw,
            _ => {}
        }
    }

    pub fn toggle_head_shape(&mut self) {
        if self.head_round {
            // Custom starts from the round head the user was looking at.
            self.edge = self.width / 2.0;
            self.head_corner = self.width / 2.0;
        }
        self.head_round = !self.head_round;
    }

    pub fn geometry(&self) -> LugGeometry {
        LugGeometry {
            hole_dia: self.hole_dia,
            width: self.width,
            edge: self.effective_edge(),
            length: self.model_length,
            thickness: self.thickness,
            head_corner_radius: self.effective_corner(),
            far_corner_radius: 0.0,
        }
    }

    /// Inline warning for a field whose value cannot be analysed.
    pub fn validation_hint(&self, t: NumberTarget) -> Option<String> {
        let g = self.geometry();
        let a = g.bore_radius();
        match t {
            NumberTarget::Width if g.width <= g.hole_dia => Some("must exceed the hole diameter".into()),
            NumberTarget::HoleDia if g.width <= g.hole_dia => Some("must be smaller than the width".into()),
            NumberTarget::Edge if g.edge <= a => Some("must exceed the hole radius".into()),
            NumberTarget::HeadCorner if g.head_corner_radius > g.width / 2.0 + 1e-12 => Some("cannot exceed half the width".into()),
            NumberTarget::HeadCorner if g.head_corner_radius > g.edge + 1e-12 => Some("cannot exceed the edge distance".into()),
            NumberTarget::ModelLength if g.length <= a => Some("must exceed the hole radius".into()),
            NumberTarget::PinDia if self.pin_dia > self.contact_dia() * 1.2 => Some(if self.bushing { "far larger than the bushing bore" } else { "far larger than the hole" }.into()),
            NumberTarget::BushingId if self.bushing && self.bushing_id >= g.hole_dia * 0.999 => Some("must be smaller than the hole".into()),
            NumberTarget::HoleDia if self.bushing && self.bushing_id >= g.hole_dia * 0.999 => Some("must exceed the bushing inner diameter".into()),
            _ => None,
        }
    }

    pub fn input(&self) -> Result<LugInput, String> {
        let geometry = self.geometry();
        geometry.validate()?;
        if !(self.pin_dia > 0.0 && self.pin_dia <= self.contact_dia() * 1.2) {
            return Err("the pin diameter must be positive and not far larger than the surface it bears on".into());
        }
        let bushing = if self.bushing {
            if !(self.bushing_id > 0.0 && self.bushing_id < geometry.hole_dia * 0.999) {
                return Err("the bushing's inner diameter must be smaller than the hole".into());
            }
            let material = MaterialLimits::from_material(self.bushing_material());
            if !(material.e_psi > 0.0 && material.nu > 0.0 && material.nu < 0.5) {
                return Err("the bushing material needs a positive modulus and a Poisson ratio between 0 and 0.5".into());
            }
            Some(BushingInput { inner_dia: self.bushing_id, interference_dia: self.bushing_interference, friction: self.bushing_friction, material })
        } else {
            None
        };
        let material = MaterialLimits::from_material(self.material());
        if !(material.e_psi > 0.0 && material.nu > 0.0 && material.nu < 0.5) {
            return Err("the material needs a positive modulus and a Poisson ratio between 0 and 0.5".into());
        }
        let pin_material = self.pin_body.material();
        let thermal = Thermal { delta_t: self.delta_t, alpha_lug: material.alpha, alpha_bushing: bushing.as_ref().map_or(0.0, |b| b.material.alpha), alpha_pin: pin_material.map_or(material.alpha, |m| m.2) };
        let pin = PinSpec { thermal, body: pin_material.map_or(PinBody::Rigid, |m| PinBody::Elastic(FeMaterial { e_psi: m.0, nu: m.1 })), ..PinSpec::new(self.pin_dia, self.friction) };
        Ok(LugInput {
            geometry,
            material,
            pin,
            case: LoadCase { load_lbf: self.load, angle_deg: self.load_angle },
            mesh: MeshSpec { elements_around: self.effective_elements_around(), max_growth: self.max_growth, first_layer_aspect: self.first_layer_aspect, refine: None },
            bushing,
            plastic: self.plastic,
            flow_rule: self.flow_rule,
            elongation: self.effective_elongation(),
            ductility: (self.plastic && self.finite_strain).then(|| self.effective_failure_strain()),
            failure_overridden: self.failure_strain.is_some(),
            second_order: self.second_order,
            pin_bending: self.pin_bending.then_some(self.clevis_offset),
            solver: self.solver,
            auto_refine: self.auto_refine,
        })
    }
}

// ---------------------------------------------------------------------------
// The analysis
// ---------------------------------------------------------------------------

/// The condensed lug of the legacy solver, built on first use.
fn legacy_model(cached: &mut CachedModel) -> Result<Arc<LugModel>, String> {
    if let Some(m) = &cached.model {
        return Ok(m.clone());
    }
    let k = &cached.key;
    let m = Arc::new(LugModel::build_bushed(&k.geometry, k.material, k.mesh, k.symmetric, PlaneMode::Stress, k.bushing)?);
    cached.model = Some(m.clone());
    Ok(m)
}

/// The kernel's lug in the given plane idealisation (plane stress for the elastic analysis, plane strain
/// for the collapse); `geometric` switches on the second-order (finite-strain elastic) behaviour.
fn kernel_model(cached: &mut CachedModel, mode: PlaneMode, geometric: bool) -> Result<Arc<FeaLug>, String> {
    match mode {
        PlaneMode::Stress => {
            if let Some((g, m)) = &cached.kernel {
                if *g == geometric {
                    return Ok(m.clone());
                }
            }
        }
        PlaneMode::Strain => {
            if let Some(m) = &cached.kernel_plastic {
                return Ok(m.clone());
            }
        }
    }
    let k = &cached.key;
    let mut lug = FeaLug::build_bushed(&k.geometry, k.material, k.mesh, k.symmetric, mode, k.bushing)?;
    lug.set_geometric_nonlinearity(geometric && mode == PlaneMode::Stress);
    let lug = Arc::new(lug);
    match mode {
        PlaneMode::Stress => cached.kernel = Some((geometric, lug.clone())),
        PlaneMode::Strain => cached.kernel_plastic = Some(lug.clone()),
    }
    Ok(lug)
}

/// Why the kernel cannot run this case at all (it then falls back to the legacy solver).
fn kernel_unsupported(input: &LugInput) -> Option<&'static str> {
    if matches!(input.pin.body, PinBody::Elastic(_)) && !input.case.is_axial() && input.pin.friction > 0.0 {
        return Some("a meshed elastic pin with friction needs the symmetric (axial) model: friction puts a torque on the pin that only the clevis reacts");
    }
    None
}

/// The elastic analysis on `solver` (`kernel` true: the general kernel).
fn elastic(cached: &mut CachedModel, input: &LugInput, kernel: bool) -> Result<LugSolution, String> {
    if kernel {
        if let Some(why) = kernel_unsupported(input) {
            return Err(why.to_string());
        }
        kernel_model(cached, PlaneMode::Stress, input.second_order)?.analyze(input.pin, input.case)
    } else {
        let model = legacy_model(cached)?;
        if input.second_order { model.solve_second_order(input.pin, input.case) } else { model.solve(input.pin, input.case) }
    }
}

/// What a collapse analysis produced.
struct CollapseOut {
    limit: Option<Arc<LimitLoad>>,
    finite: Option<Arc<FsResult>>,
    /// The result came from the cache (only the applied load changed).
    reused: bool,
    notes: Vec<String>,
}

/// The plastic collapse (perfectly plastic under the flow rule, or finite strain) on one solver, reusing
/// `cached` results.
fn collapse(cached: &mut CachedModel, input: &LugInput, kernel: bool) -> CollapseOut {
    let (mut limit, mut finite, mut reused, mut notes) = (None, None, false, Vec::new());
    // The legacy finite-strain solver covers axial loads without a bushing; the kernel covers every case (the
    // failure-strain calibration itself was made on axial, unbushed lugs).
    let finite_ok = input.ductility.is_some() && (kernel || (input.case.is_axial() && input.bushing.is_none()));
    if input.ductility.is_some() && !finite_ok {
        notes.push("Finite-strain collapse by the legacy solver is available for axial loads without a bushing; the perfectly plastic collapse is shown instead (the kernel solver covers every case).".to_string());
    }
    if let (true, Some(eps_f)) = (finite_ok, input.ductility) {
        // 32 elements around at the Normal density: the mesh the library failure strains are calibrated on.
        let around = (input.mesh.elements_around * 4 / 9).max(16);
        let pin_dia = input.pin.thermal.pin_diameter(input.pin.diameter, input.bushing.is_some());
        let fkey = FiniteKey { pin_dia, friction: input.pin.friction, angle_deg: input.case.angle_deg, failure_strain: eps_f, elongation: input.elongation, fty: input.material.fty_psi, ftu: input.material.ftu_psi, mesh_around: around, kernel };
        if let Some((_, r)) = cached.finites.iter().find(|(k, _)| *k == fkey) {
            finite = Some(r.clone());
            reused = true;
        } else {
            let result = Hardening::true_curve(input.material.fty_psi, input.material.ftu_psi, input.elongation, 1.5).and_then(|law| {
                let spec = MeshSpec { elements_around: around, ..MeshSpec::default() };
                if kernel {
                    let fe = FeaLug::build_bushed(&input.geometry, FeMaterial { e_psi: input.material.e_psi, nu: input.material.nu }, spec, input.case.is_axial(), PlaneMode::Strain, input.bushing.as_ref().map(|b| b.spec()))?;
                    fe.finite_collapse(PinSpec { friction: input.pin.friction, ..PinSpec::new(pin_dia, input.pin.friction) }, input.case, law, Some(eps_f), 1.5)
                } else {
                    let mat = FsMaterial { e_psi: input.material.e_psi, nu: input.material.nu, law };
                    let fl = FiniteLug::build(&input.geometry, spec, mat, true)?;
                    fl.collapse(input.case, FsOptions { strain_limit: Some(eps_f), travel_cap_over_a: 1.5, ..FsOptions::new(pin_dia, input.pin.friction) })
                }
            });
            match result {
                Ok(r) => {
                    let r = Arc::new(r);
                    cached.finites.truncate(3);
                    cached.finites.insert(0, (fkey, r.clone()));
                    finite = Some(r);
                }
                Err(e) => notes.push(format!("The finite-strain collapse could not be found: {e}")),
            }
        }
    }
    if input.plastic && !finite_ok {
        let flow = input.material.flow_for(input.flow_rule, input.elongation);
        let lkey = LimitKey { pin: input.pin, angle_deg: input.case.angle_deg, flow_stress: flow, kernel };
        if let Some((_, l)) = cached.limits.iter().find(|(k, _)| *k == lkey) {
            limit = Some(l.clone());
            reused = true;
        } else {
            let found = if kernel {
                kernel_model(cached, PlaneMode::Strain, false).and_then(|fe| fe.limit_load(input.pin, input.case, flow, LimitOptions::default()))
            } else {
                let pm = match &cached.plastic_model {
                    Some(m) => Ok(m.clone()),
                    None => {
                        let k = &cached.key;
                        LugModel::build_bushed(&k.geometry, k.material, k.mesh, k.symmetric, PlaneMode::Strain, k.bushing).map(Arc::new)
                    }
                };
                pm.and_then(|pm| {
                    let l = pm.limit_load(input.pin, input.case, flow)?;
                    cached.plastic_model = Some(pm);
                    Ok(l)
                })
            };
            match found {
                Ok(l) => {
                    let l = Arc::new(l);
                    cached.limits.truncate(3);
                    cached.limits.insert(0, (lkey, l.clone()));
                    limit = Some(l);
                }
                Err(e) => notes.push(format!("The plastic limit load could not be found: {e}")),
            }
        }
    }
    CollapseOut { limit, finite, reused, notes }
}

/// Run one analysis, reusing `cache` when the model it holds still matches. Returns the result and the
/// model to keep for next time.
pub fn run(input: &LugInput, cache: Option<CachedModel>) -> (Result<LugRun, String>, Option<CachedModel>) {
    let t0 = std::time::Instant::now();
    let fe_material = FeMaterial { e_psi: input.material.e_psi, nu: input.material.nu };
    let mut mesh = input.mesh;
    if mesh.refine.is_none() && input.auto_refine {
        let contact_radius = input.bushing.as_ref().map_or(input.geometry.bore_radius(), |b| b.inner_dia / 2.0);
        mesh.refine = auto_refinement_for(&input.geometry, fe_material, input.pin, input.case, mesh.elements_around, contact_radius);
    }
    let key = ModelKey { geometry: input.geometry, material: fe_material, mesh, symmetric: input.case.is_axial(), bushing: input.bushing.as_ref().map(|b| b.spec()) };
    let (mut cached, reused) = match cache {
        Some(c) if c.key == key => (c, true),
        _ => (CachedModel::new(key), false),
    };
    let mut extra_notes = Vec::new();

    // The elastic analysis and the plastic collapse are independent: they run side by side, each on its own copy
    // of the cached models (the copies share the built lugs; their results are merged afterwards). A kernel
    // failure falls back to the legacy solver.
    let mut solver = if input.solver == SolverChoice::Legacy { SolverChoice::Legacy } else { SolverChoice::Kernel };
    let t_main = std::time::Instant::now();
    let (mut cache_e, mut cache_c) = (cached.clone(), cached.clone());
    let use_kernel = solver == SolverChoice::Kernel;
    let (mut solved, collapsed) = std::thread::scope(|sc| {
        let job = input.plastic.then(|| sc.spawn(|| collapse(&mut cache_c, input, use_kernel)));
        let solved = elastic(&mut cache_e, input, use_kernel);
        (solved, job.map(|j| j.join().unwrap_or_else(|_| CollapseOut { limit: None, finite: None, reused: false, notes: vec!["The collapse analysis panicked.".to_string()] })))
    });
    cached.model = cache_e.model.clone().or(cached.model.take());
    cached.kernel = cache_e.kernel.clone().or(cached.kernel.take());
    if collapsed.is_some() {
        cached.plastic_model = cache_c.plastic_model.clone().or(cached.plastic_model.take());
        cached.kernel_plastic = cache_c.kernel_plastic.clone().or(cached.kernel_plastic.take());
        cached.limits = cache_c.limits;
        cached.finites = cache_c.finites;
    }
    if solver == SolverChoice::Kernel {
        if let Err(why) = &solved {
            extra_notes.push(format!("The kernel solver could not run this case ({why}); the legacy condensed solver was used."));
            solver = SolverChoice::Legacy;
            solved = elastic(&mut cached, input, false);
        }
    }
    let solution = match solved {
        Ok(s) => s,
        Err(e) => return (Err(e), Some(cached)),
    };
    let main_ms = t_main.elapsed().as_secs_f64() * 1e3;

    // Plastic collapse. It does not depend on the applied load, so it is reused while only the load changes;
    // a failure is a note, never a lost elastic result.
    let (mut limit, mut finite, mut limit_reused) = (None, None, false);
    if let Some(c) = collapsed {
        extra_notes.extend(c.notes.clone());
        // A solver that fell back (or found no collapse) is retried on the legacy solver.
        let wasted = solver == SolverChoice::Legacy && use_kernel;
        if (use_kernel && c.limit.is_none() && c.finite.is_none()) || wasted {
            let c2 = collapse(&mut cached, input, false);
            if c2.limit.is_some() || c2.finite.is_some() {
                if !wasted {
                    extra_notes.push("The kernel solver found no collapse; the legacy solver's is shown.".to_string());
                }
                extra_notes.extend(c2.notes);
                (limit, finite, limit_reused) = (c2.limit, c2.finite, c2.reused);
            }
        } else {
            (limit, finite, limit_reused) = (c.limit, c.finite, c.reused);
        }
    }

    // The legacy solver beside the kernel's (Solver: Compare).
    let comparison = if input.solver == SolverChoice::Compare && solver == SolverChoice::Kernel {
        let t = std::time::Instant::now();
        match elastic(&mut cached, input, false) {
            Ok(s) => {
                let c = if input.plastic { collapse(&mut cached, input, false) } else { CollapseOut { limit: None, finite: None, reused: false, notes: Vec::new() } };
                extra_notes.extend(c.notes.into_iter().map(|x| format!("Legacy: {x}")));
                Some(Box::new(Comparison { solution: s, limit: c.limit, finite: c.finite, ms: t.elapsed().as_secs_f64() * 1e3 }))
            }
            Err(e) => {
                extra_notes.push(format!("The legacy solver could not run this case for the comparison: {e}"));
                None
            }
        }
    } else {
        None
    };

    // Pin bending through the thickness (double shear) and the hoop stress at the most loaded slice.
    let mut thickness = None;
    if let Some(offset) = input.pin_bending {
        let (e_psi, nu) = match input.pin.body {
            PinBody::Elastic(m) => (m.e_psi, m.nu),
            PinBody::Rigid => (29.0e6, 0.30),
        };
        let bending = PinBending { e_psi, nu, shear: Shear::Double { offset }, slices: 9 };
        match legacy_model(&mut cached).and_then(|lm| {
            let t = lm.through_thickness(input.pin, input.case, bending)?;
            let peak_slice = t.slice_load.iter().cloned().fold(0.0, f64::max);
            let hoop = lm.solve(input.pin, LoadCase { load_lbf: peak_slice * input.geometry.thickness, angle_deg: input.case.angle_deg }).map(|s| s.peak_hoop).unwrap_or(0.0);
            Ok((t, hoop))
        }) {
            Ok(t) => thickness = Some(t),
            Err(e) => extra_notes.push(format!("The pin bending analysis failed: {e}")),
        }
    }

    let (checks, mut notes) = evaluate_with(input, &solution, limit.as_deref(), finite.as_deref(), thickness.as_ref(), solver);
    notes.extend(extra_notes);
    if solver == SolverChoice::Kernel {
        notes.insert(0, format!("Solved on the general finite-element kernel (fea-core) in {main_ms:.0} ms."));
    }
    let run = LugRun { input: input.clone(), solution, checks, notes, limit, finite, limit_reused, reused_model: reused, thickness, total_ms: t0.elapsed().as_secs_f64() * 1e3, solver, comparison };
    (Ok(run), Some(cached))
}

/// Margins against the material allowables and the cautions a reader needs.
pub fn evaluate(input: &LugInput, s: &LugSolution, limit: Option<&LimitLoad>) -> (Vec<Check>, Vec<String>) {
    evaluate_with(input, s, limit, None, None, input.solver)
}

/// [`evaluate`] with the through-thickness pin bending result (and the hoop stress at its most
/// loaded slice) when the analysis computed one.
pub fn evaluate_with(input: &LugInput, s: &LugSolution, limit: Option<&LimitLoad>, finite: Option<&FsResult>, thickness: Option<&(ThicknessResult, f64)>, used: SolverChoice) -> (Vec<Check>, Vec<String>) {
    let g = &input.geometry;
    let m = &input.material;
    let p = input.case.load_lbf;
    let d = g.hole_dia;
    let t = g.thickness;
    let ksi = |v: f64| format!("{:.1} ksi", v / 1000.0);
    let mut checks = Vec::new();
    let mut notes = Vec::new();

    // Bearing: P / (D t) against the tabulated Fbru at this e/D.
    let e_over_d = g.edge / d;
    let sb = p / (d * t);
    checks.push(match fbru_at_edge_ratio(m.fbru_psi, m.fbru_e15_psi, e_over_d) {
        Some(f) if sb > 0.0 => {
            let ms = f / sb - 1.0;
            Check { name: "Bearing", detail: format!("P/(D t) {} vs Fbru {} at e/D {e_over_d:.2}", ksi(sb), ksi(f)), margin: Some(ms), status: if ms >= 0.0 { Status::Pass } else { Status::Fail } }
        }
        Some(f) => Check { name: "Bearing", detail: format!("no load; Fbru {} at e/D {e_over_d:.2}", ksi(f)), margin: None, status: Status::Info },
        None => Check { name: "Bearing", detail: format!("no tabulated Fbru at e/D {e_over_d:.2} (needs e/D >= 2.0, or >= 1.5 with an e/D 1.5 value); P/(D t) = {}", ksi(sb)), margin: None, status: Status::Info },
    });

    // Net section (axial component only; a transverse load bends the lug instead).
    let axial = p * input.case.angle_deg.to_radians().cos().abs();
    let transverse_dominant = input.case.angle_deg.to_radians().sin().abs() > 0.5;
    checks.push(if transverse_dominant {
        Check { name: "Net section", detail: "not evaluated: a mostly transverse load bends the lug (see the peak stresses)".into(), margin: None, status: Status::Info }
    } else if g.width > d && m.ftu_psi > 0.0 && axial > 0.0 {
        let sn = axial / ((g.width - d) * t);
        let ms = m.ftu_psi / sn - 1.0;
        Check { name: "Net section", detail: format!("P/((W - D) t) {} vs Ftu {} (nominal, before any efficiency factor)", ksi(sn), ksi(m.ftu_psi)), margin: Some(ms), status: if ms >= 0.0 { Status::Pass } else { Status::Fail } }
    } else {
        Check { name: "Net section", detail: "no axial load".into(), margin: None, status: Status::Info }
    });

    // Elastic stress indicators from the FE.
    checks.push(if m.fty_psi > 0.0 && s.peak_von_mises > 0.0 {
        let ms = m.fty_psi / s.peak_von_mises - 1.0;
        Check {
            name: "First yield",
            detail: format!("peak von Mises {} at ({:.2}, {:.2}) vs Fty {}", ksi(s.peak_von_mises), s.peak_von_mises_at[0], s.peak_von_mises_at[1], ksi(m.fty_psi)),
            margin: Some(ms),
            status: if ms >= 0.0 { Status::Pass } else { Status::Warn },
        }
    } else {
        Check { name: "First yield", detail: "no stress".into(), margin: None, status: Status::Info }
    });
    checks.push(if m.ftu_psi > 0.0 && s.peak_hoop > 0.0 {
        let ms = m.ftu_psi / s.peak_hoop - 1.0;
        Check { name: "Peak hoop vs Ftu", detail: format!("elastic hoop peak {} at {:.0} deg vs Ftu {}", ksi(s.peak_hoop), s.peak_hoop_angle_deg, ksi(m.ftu_psi)), margin: Some(ms), status: if ms >= 0.0 { Status::Pass } else { Status::Warn } }
    } else {
        Check { name: "Peak hoop vs Ftu", detail: "no tensile hoop stress".into(), margin: None, status: Status::Info }
    });

    if let (Some(b), Some(bi)) = (&s.bushing, &input.bushing) {
        checks.push(if bi.material.fty_psi > 0.0 && b.peak_von_mises > 0.0 {
            let ms = bi.material.fty_psi / b.peak_von_mises - 1.0;
            Check { name: "Bushing stress", detail: format!("peak von Mises {} vs bushing Fty {}", ksi(b.peak_von_mises), ksi(bi.material.fty_psi)), margin: Some(ms), status: if ms >= 0.0 { Status::Pass } else { Status::Fail } }
        } else {
            Check { name: "Bushing stress", detail: "no stress or no yield strength for the bushing material".into(), margin: None, status: Status::Info }
        });
        checks.push(if b.fit_pressure_unloaded <= 1.0 {
            Check { name: "Bushing fit", detail: "no interference fit pressure (a clearance or zero interference): the bushing is held only by the load".into(), margin: None, status: Status::Warn }
        } else if b.separated_fraction > 0.5 {
            Check { name: "Bushing fit", detail: format!("fit pressure {} unloaded, but {:.0} % of the interface has lost contact at the load", ksi(b.fit_pressure_unloaded), 100.0 * b.separated_fraction), margin: None, status: Status::Warn }
        } else {
            Check { name: "Bushing fit", detail: format!("fit pressure {} unloaded; {:.0} % of the interface separated at the load", ksi(b.fit_pressure_unloaded), 100.0 * b.separated_fraction), margin: None, status: Status::Pass }
        });
    }
    if let Some(l) = limit {
        let flow = l.flow_stress;
        let model_label = format!("plane-strain EPP at {} = {}", ksi(flow), input.flow_rule.label());
        checks.push(if p > 0.0 {
            let ms = l.limit_load_lbf / p - 1.0;
            Check {
                name: "Ultimate (plastic)",
                detail: format!("collapse {:.0} lbf ({model_label}) vs applied {:.0} lbf{}", l.limit_load_lbf, p, if l.plateau { "" } else { "; lower bound (the run ended before the load peaked)" }),
                margin: Some(ms),
                status: if ms < 0.0 { Status::Fail } else if l.plateau { Status::Pass } else { Status::Warn },
            }
        } else {
            Check { name: "Ultimate (plastic)", detail: format!("collapse {:.0} lbf ({model_label}); no applied load", l.limit_load_lbf), margin: None, status: Status::Info }
        });
        if input.flow_rule == FlowRule::TrueUltimate {
            notes.push(format!("Flow rule Ftu (1 + elongation) is not conservative for every alloy: on NACA TN 1503 it over-predicted the brittle 75S bars by up to {}. Use Ftu for a strength check.", if used == SolverChoice::Legacy { "10 %" } else { "14 %" }));
        }
        notes.push(format!("Ultimate capacity is a perfectly plastic plane-strain collapse at the flow stress {} (the pin's stiffness does not change a limit load). On the 12 NACA TN 1503 pin-bearing tests ({}): {}. A conservative estimate, not a test-correlated allowable.", ksi(flow), if used == SolverChoice::Legacy { "legacy solver" } else { "kernel solver" }, naca_statistics(used)));
    }
    if let (Some(f), Some(eps_f)) = (finite, input.ductility) {
        let label = format!("finite strain, true stress-strain from Fty {} / Ftu {} / elongation {:.1} %, failure strain {:.2}", ksi(m.fty_psi), ksi(m.ftu_psi), 100.0 * input.elongation, eps_f);
        let defined = f.strain_limited || f.peak_reached;
        checks.push(if p > 0.0 {
            let ms = f.collapse_lbf / p - 1.0;
            Check {
                name: "Ultimate (finite)",
                detail: format!("collapse {:.0} lbf ({label}) vs applied {:.0} lbf{}", f.collapse_lbf, p, if defined { "" } else { "; lower bound (the pin travel cap ended the run)" }),
                margin: Some(ms),
                status: if ms < 0.0 { Status::Fail } else if defined { Status::Pass } else { Status::Warn },
            }
        } else {
            Check { name: "Ultimate (finite)", detail: format!("collapse {:.0} lbf ({label}); no applied load", f.collapse_lbf), margin: None, status: Status::Info }
        });
        let source = match (input.failure_overridden, m.failure) {
            (true, _) => "typed by you".to_string(),
            (false, Some((_, basis))) => format!("library, {basis}"),
            (false, None) => "default (a material outside the library)".to_string(),
        };
        notes.push(format!("Finite-strain collapse: the load where the equivalent plastic strain anywhere reaches the failure strain {eps_f:.2} ({source}), or its peak, whichever comes first. The rule m ln(1 + elongation) reproduced 20 NACA TN 1503 / TN 1502 pin-bearing points within -9 to +3 % for 7075, 2014 and 2024 aluminium; other families are not bearing-validated. The result is proportional to this material constant."));
        if let Some(n) = &f.note {
            notes.push(format!("Finite strain: {n}"));
        }
    }

    // Solver self-checks and the discretisation error estimate.
    let v = &s.verification;
    checks.push(Check {
        name: "Solution check",
        detail: format!("equilibrium {:.0e}, energy {:.0e}, penetration {:.0e}, friction cone {:.0e}; mesh error estimate {:.1} % (Zienkiewicz-Zhu)", v.force_balance, v.energy_balance, v.max_penetration, v.friction_excess, 100.0 * v.discretisation_error),
        margin: None,
        status: if !v.ok() { Status::Fail } else if v.discretisation_error > 0.10 { Status::Warn } else { Status::Pass },
    });
    if v.discretisation_error > 0.10 {
        notes.push(format!("The mesh error estimate is {:.0} % (energy norm around the pin): switch Mesh Density to Fine and compare, especially the von Mises peak (the bore hoop stress converges faster).", 100.0 * v.discretisation_error));
    }
    if let Some((t, hoop)) = thickness {
        let peak_bearing = t.bearing_stress.iter().cloned().fold(0.0, f64::max);
        checks.push(match fbru_at_edge_ratio(m.fbru_psi, m.fbru_e15_psi, e_over_d) {
            Some(f) if peak_bearing > 0.0 => {
                let ms = f / peak_bearing - 1.0;
                Check { name: "Bearing (peak slice)", detail: format!("most loaded slice {} (peaking {:.2} x the mean) vs Fbru {}", ksi(peak_bearing), t.peaking, ksi(f)), margin: Some(ms), status: if ms >= 0.0 { Status::Pass } else { Status::Fail } }
            }
            _ => Check { name: "Bearing (peak slice)", detail: format!("most loaded slice {} (peaking {:.2} x the mean)", ksi(peak_bearing), t.peaking), margin: None, status: Status::Info },
        });
        checks.push(Check { name: "Pin bending", detail: format!("moment {:.0} lbf-in, bending stress {}, shear {} (mean V/A)", t.max_moment, ksi(t.pin_bending_stress), ksi(t.pin_shear_stress)), margin: None, status: Status::Info });
        if *hoop > 0.0 && m.ftu_psi > 0.0 {
            let ms = m.ftu_psi / hoop - 1.0;
            checks.push(Check { name: "Hoop (peak slice)", detail: format!("elastic hoop {} at the most loaded slice vs Ftu {}", ksi(*hoop), ksi(m.ftu_psi)), margin: Some(ms), status: if ms >= 0.0 { Status::Pass } else { Status::Warn } });
        }
        notes.push("Pin bending: double shear, the clevis ears as point reactions, the lug as slices on its own load-travel curve (steel pin unless an elastic pin is chosen). No pin allowable is checked.".to_string());
        notes.push(format!("The slice model (a pin as a beam on the lug's springs) gives the mean-field distribution: peaking {:.2}. A 3D kernel model of the same stubby pin (`lug-solver/tests/fea_bridge.rs`) shows the lug face bearing 12-17 % above the mean, and that face value grows with mesh refinement (a load-transfer singularity at the lug face: no converged elastic value exists), so treat the peak slice as a lower bound for a stubby pin.", t.peaking));
    }
    if input.second_order {
        notes.push(if used == SolverChoice::Legacy {
            "Second-order analysis: the geometric stiffness of the stress state is included (small strains, the mesh is not moved).".to_string()
        } else {
            "Second-order analysis: the lug is a finite-strain elastic body on the kernel (large rotations and displacements, so tension stiffens it and compression softens it; checked against the beam-column amplification).".to_string()
        });
    }
    if used == SolverChoice::Legacy && (input.pin.friction > 0.0 || input.bushing.as_ref().is_some_and(|b| b.friction > 0.0)) {
        notes.push("Friction on the legacy solver depends on its load path and normal penalty: its peak hoop stress sits about 8 % below the path-resolved kernel's at 45 deg and friction 0.15 (and moves ~5 % with its penalty). The kernel is the reference; compare with Solver: Compare.".to_string());
    }
    if input.pin.thermal.delta_t != 0.0 {
        notes.push(format!("Thermal: {:+.0} F of free expansion changes the fit (hole, bushing and pin sizes); structural thermal stress is not modelled.", input.pin.thermal.delta_t));
    }

    let a = g.bore_radius();
    if s.bearing_deflection > 0.1 * a {
        notes.push(format!("Pin travel {:.4} in is large against the bore radius: the small-displacement model loses accuracy.", s.bearing_deflection));
    }
    if !input.case.is_axial() {
        notes.push("Oblique/transverse load: the far end is clamped, so the response (bending) depends on the model length.".to_string());
    }
    if s.contact_arc_deg > 359.0 && s.peak_pressure > 0.0 {
        notes.push("Contact all round the bore (an interference or very tight fit): the fit pressure is included.".to_string());
    } else if s.contact_arc_deg > 0.0 && s.contact_arc_deg < 20.0 {
        notes.push(format!("Narrow {:.1} deg contact patch (a loose pin): the contact pressure peak is local; refinement of the loaded sector is applied.", s.contact_arc_deg));
    }
    if s.peak_von_mises > 0.0 && m.fty_psi > 0.0 && s.peak_von_mises > m.fty_psi {
        notes.push(if limit.is_some() {
            "Elastic peaks above yield mean local plasticity at the hole edge; they are indicators. The ultimate margin comes from the plastic collapse.".to_string()
        } else {
            "Elastic analysis: stresses above yield mean local plasticity at the hole edge. The elastic peaks are indicators, not the ultimate capacity (turn on Plastic Limit Load).".to_string()
        });
    }
    if e_over_d < 1.0 {
        notes.push(format!("Edge ratio e/D {e_over_d:.2} is very small: shear-out through the head is likely to govern."));
    }
    (checks, notes)
}

/// Plain-text report (written by the `e` key).
pub fn report_text(run: &LugRun) -> String {
    let i = &run.input;
    let g = &i.geometry;
    let s = &run.solution;
    let mut r = String::from("Lug Analysis (rigid-pin contact FE, plane stress)\n\n");
    r.push_str(&format!("Material      {}  (E {:.0} ksi, nu {:.2}, Fty {:.1} ksi, Ftu {:.1} ksi)\n", i.material.name, i.material.e_psi / 1000.0, i.material.nu, i.material.fty_psi / 1000.0, i.material.ftu_psi / 1000.0));
    r.push_str(&format!("Geometry      D {:.4}  W {:.4}  e {:.4}  t {:.4}  head corner {:.4}  model length {:.4} in\n", g.hole_dia, g.width, g.edge, g.thickness, g.head_corner_radius, g.length));
    if let (Some(b), Some(bi)) = (&s.bushing, &i.bushing) {
        r.push_str(&format!("Bushing       {} inner dia {:.4} in, interference {:+.4} in, fit friction {:.2}\n", bi.material.name, bi.inner_dia, bi.interference_dia, bi.friction));
        r.push_str(&format!("Fit           {:.1} ksi unloaded; at the load mean {:.1} / peak {:.1} ksi, {:.0} % separated; bushing peak hoop {:.1} ksi, von Mises {:.1} ksi\n", b.fit_pressure_unloaded / 1000.0, b.interface_mean_pressure / 1000.0, b.interface_peak_pressure / 1000.0, 100.0 * b.separated_fraction, b.peak_hoop / 1000.0, b.peak_von_mises / 1000.0));
    }
    r.push_str(&format!("Pin           dia {:.4} in  ({:+.4} in on the radius)  friction {:.3}  body {}\n", i.pin.diameter, (i.pin.diameter - g.hole_dia) / 2.0, i.pin.friction, match i.pin.body { PinBody::Rigid => "rigid", PinBody::Elastic(_) => "elastic" }));
    if i.pin.thermal.delta_t != 0.0 {
        r.push_str(&format!("Temperature   {:+.0} F (free expansion changes the fit)\n", i.pin.thermal.delta_t));
    }
    r.push_str(&format!("Load          {:.1} lbf at {:.1} deg from the lug axis\n\n", i.case.load_lbf, i.case.angle_deg));
    r.push_str(&format!("Contact       patch {:.1} deg about {:.0} deg, peak pressure {:.1} ksi, bearing stress P/(D t) {:.1} ksi, pin travel {:.5} in\n", s.contact_arc_deg, s.contact_centre_deg.rem_euclid(360.0), s.peak_pressure / 1000.0, s.bearing_stress / 1000.0, s.bearing_deflection));
    r.push_str(&format!("Stress        peak hoop {:.1} ksi at {:.0} deg, peak von Mises {:.1} ksi, net section {:.1} ksi, Kt (net) {:.2}\n\n", s.peak_hoop / 1000.0, s.peak_hoop_angle_deg, s.peak_von_mises / 1000.0, s.net_section_stress / 1000.0, s.kt_net));
    if let Some(l) = &run.limit {
        let what = format!("at flow stress {:.1} ksi (plane-strain elastic-perfectly-plastic)", l.flow_stress / 1000.0);
        r.push_str(&format!("Collapse      {:.0} lbf {what}, {} steps, {} iterations, {:.0} ms{}\n\n", l.limit_load_lbf, l.curve.len(), l.plastic_iterations, l.elapsed_ms, if run.limit_reused { " (reused)" } else { "" }));
    }
    if let (Some(f), Some(e)) = (&run.finite, i.ductility) {
        r.push_str(&format!("Collapse      {:.0} lbf, finite strain (true stress-strain, failure strain {e:.2}), {} steps, {} factorisations, {:.0} ms{}\n\n", f.collapse_lbf, f.curve.len(), f.factorisations, f.elapsed_ms, if run.limit_reused { " (reused)" } else { "" }));
    }
    if let Some((t, hoop)) = &run.thickness {
        r.push_str(&format!("Pin bending   peaking {:.2} x mean over {} slices, moment {:.0} lbf-in, bending {:.1} ksi, shear {:.1} ksi, hoop at the most loaded slice {:.1} ksi\n\n", t.peaking, t.slice_load.len(), t.max_moment, t.pin_bending_stress / 1000.0, t.pin_shear_stress / 1000.0, hoop / 1000.0));
    }
    r.push_str("Checks\n");
    for c in &run.checks {
        let ms = c.margin.map_or("   -  ".to_string(), |m| format!("{m:+6.2}"));
        r.push_str(&format!("  {:<18} MS {ms}  {:<5} {}\n", c.name, format!("{:?}", c.status).to_uppercase(), c.detail));
    }
    if !run.notes.is_empty() {
        r.push_str("\nNotes\n");
        for n in &run.notes {
            r.push_str(&format!("  - {n}\n"));
        }
    }
    r.push_str(&format!("\nModel         {} nodes, {} dofs, bandwidth {}, {} Newton iterations; {:.0} ms ({} model)\n", s.mesh.nodes, s.dofs, s.bandwidth, s.contact_iterations, run.total_ms, if run.reused_model { "reused" } else { "built" }));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> LugInput {
        LugUiModel::default().input().unwrap()
    }

    #[test]
    fn oblique_loads_run_at_every_angle_with_the_default_lug() {
        for angle in [15.0, 45.0, 90.0, 135.0] {
            let mut m = LugUiModel::default();
            m.commit_number(NumberTarget::LoadAngle, angle);
            let (r, _) = run(&m.input().unwrap(), None);
            let r = r.unwrap_or_else(|e| panic!("angle {angle}: {e}"));
            assert!(!r.checks.is_empty() && r.solution.peak_hoop > 0.0, "angle {angle}");
        }
    }

    #[test]
    fn the_default_lug_is_valid_and_uses_a_handbook_plate_with_an_e15_value() {
        let m = LugUiModel::default();
        assert!(m.input().is_ok());
        assert!(m.material().fbru_e15_ksi > 0.0, "{}", m.material().name);
        assert!(m.head_round && (m.effective_edge() - 0.75).abs() < 1e-12);
    }

    #[test]
    fn field_rows_hide_the_head_rows_in_round_mode_and_show_them_when_custom() {
        let mut m = LugUiModel::default();
        let has = |m: &LugUiModel, t| field_rows(m).contains(&FieldRow::Number(t));
        assert!(!has(&m, NumberTarget::Edge) && !has(&m, NumberTarget::HeadCorner));
        m.toggle_head_shape();
        assert!(has(&m, NumberTarget::Edge) && has(&m, NumberTarget::HeadCorner));
        assert!((m.edge - 0.75).abs() < 1e-12, "custom starts from the round head");
    }

    #[test]
    fn a_round_head_follows_the_width() {
        let mut m = LugUiModel::default();
        m.commit_number(NumberTarget::Width, 2.0);
        assert!((m.effective_edge() - 1.0).abs() < 1e-12 && (m.effective_corner() - 1.0).abs() < 1e-12);
        assert!((m.geometry().edge - 1.0).abs() < 1e-12);
    }

    #[test]
    fn invalid_values_are_ignored_and_invalid_geometry_is_an_error() {
        let mut m = LugUiModel::default();
        m.commit_number(NumberTarget::HoleDia, -1.0);
        m.commit_number(NumberTarget::Load, f64::NAN);
        m.commit_number(NumberTarget::Friction, 5.0);
        assert_eq!((m.hole_dia, m.load, m.friction), (0.5, 4000.0, 0.15));
        m.commit_number(NumberTarget::HoleDia, 1.6); // wider than the lug
        assert!(m.input().is_err());
        assert!(m.validation_hint(NumberTarget::Width).is_some());
        assert!(m.validation_hint(NumberTarget::HoleDia).is_some());
        m.commit_number(NumberTarget::LoadAngle, 400.0);
        assert_eq!(m.load_angle, 180.0, "the angle is clamped to 0..180");
    }

    #[test]
    fn the_pin_must_be_a_sensible_size() {
        let mut m = LugUiModel::default();
        m.commit_number(NumberTarget::PinDia, 0.9);
        assert!(m.input().is_err());
        assert!(m.validation_hint(NumberTarget::PinDia).is_some());
    }

    #[test]
    fn the_default_analysis_runs_and_reports_every_check() {
        let (res, cache) = run(&input(), None);
        let r = res.unwrap();
        assert!(cache.is_some() && !r.reused_model);
        let names: Vec<_> = r.checks.iter().map(|c| c.name).collect();
        assert_eq!(names, ["Bearing", "Net section", "First yield", "Peak hoop vs Ftu", "Ultimate (plastic)", "Solution check"]);
        assert!(r.solution.peak_hoop > 0.0 && r.solution.kt_net > 1.0, "Kt {}", r.solution.kt_net);
        let text = report_text(&r);
        assert!(text.contains("Lug Analysis") && text.contains("Checks") && text.contains("Bearing"), "{text}");
    }

    #[test]
    fn turning_the_plastic_option_off_skips_the_collapse_and_its_check() {
        let mut i = input();
        i.plastic = false;
        let r = run(&i, None).0.unwrap();
        assert!(r.limit.is_none() && r.checks.iter().all(|c| c.name != "Ultimate (plastic)"));
        assert!(r.notes.iter().any(|n| n.contains("turn on Plastic Limit Load")) || r.solution.peak_von_mises < i.material.fty_psi);
    }

    #[test]
    fn the_ultimate_margin_is_collapse_over_applied_minus_one_and_the_collapse_is_load_independent() {
        let mut i = input();
        i.case.load_lbf = 3000.0;
        let (res, cache) = run(&i, None);
        let a = res.unwrap();
        let l = a.limit.clone().expect("collapse computed");
        assert!(!a.limit_reused && l.plateau && l.limit_load_lbf > 5000.0, "{} lbf", l.limit_load_lbf);
        let c = a.checks.iter().find(|c| c.name == "Ultimate (plastic)").unwrap();
        assert!((c.margin.unwrap() - (l.limit_load_lbf / 3000.0 - 1.0)).abs() < 1e-9);
        // A different applied load reuses the collapse (it does not depend on the load).
        i.case.load_lbf = 5000.0;
        let b = run(&i, cache).0.unwrap();
        assert!(b.limit_reused, "only the applied load changed");
        assert_eq!(b.limit.as_ref().unwrap().limit_load_lbf, l.limit_load_lbf);
        assert!(b.checks.iter().find(|c| c.name == "Ultimate (plastic)").unwrap().margin.unwrap() < c.margin.unwrap());
    }

    #[test]
    fn changing_the_pin_or_the_material_recomputes_the_collapse() {
        let mut i = input();
        let (_, cache) = run(&i, None);
        i.pin = PinSpec::new(0.497, 0.15);
        let (res, cache) = run(&i, cache);
        assert!(!res.unwrap().limit_reused, "a different pin is a different collapse");
        i.material.ftu_psi *= 1.1;
        assert!(!run(&i, cache).0.unwrap().limit_reused, "a different flow stress too");
    }

    #[test]
    fn an_overloaded_lug_fails_the_ultimate_check() {
        let mut i = input();
        i.case.load_lbf = 40_000.0;
        let r = run(&i, None).0.unwrap();
        let c = r.checks.iter().find(|c| c.name == "Ultimate (plastic)").unwrap();
        assert_eq!(c.status, Status::Fail);
        assert!(c.margin.unwrap() < 0.0);
    }

    fn check<'a>(r: &'a LugRun, name: &str) -> &'a Check {
        r.checks.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no check {name}: {:?}", r.checks.iter().map(|c| c.name).collect::<Vec<_>>()))
    }

    #[test]
    fn the_solution_check_passes_for_the_default_lug() {
        let (r, _) = run(&input(), None);
        let r = r.unwrap();
        let c = check(&r, "Solution check");
        assert_eq!(c.status, Status::Pass, "{}", c.detail);
        assert!(c.detail.contains("Zienkiewicz-Zhu"));
    }

    #[test]
    fn the_new_rows_follow_their_switches() {
        let mut m = LugUiModel::default();
        m.mesh_open = true; // the analysis switches sit in the Advanced section
        let has = |m: &LugUiModel, r: FieldRow| field_rows(m).contains(&r);
        assert!(has(&m, FieldRow::TogglePinBody) && has(&m, FieldRow::ToggleSecondOrder) && has(&m, FieldRow::TogglePinBending));
        assert!(has(&m, FieldRow::Number(NumberTarget::TemperatureChange)));
        assert!(has(&m, FieldRow::ToggleFiniteStrain) && !has(&m, FieldRow::Number(NumberTarget::Ductility)));
        m.finite_strain = true;
        assert!(has(&m, FieldRow::Number(NumberTarget::Ductility)));
        m.plastic = false;
        assert!(!has(&m, FieldRow::ToggleFiniteStrain) && !has(&m, FieldRow::Number(NumberTarget::Ductility)));
        assert!(m.input().unwrap().ductility.is_none(), "finite strain only applies with the plastic limit load");
        assert!(!has(&m, FieldRow::Number(NumberTarget::ClevisOffset)));
        m.pin_bending = true;
        assert!(has(&m, FieldRow::Number(NumberTarget::ClevisOffset)));
        // Out-of-range values are ignored.
        m.commit_number(NumberTarget::TemperatureChange, 5000.0);
        m.commit_number(NumberTarget::Ductility, 0.0);
        assert_eq!((m.delta_t, m.failure_strain), (0.0, None));
        m.commit_number(NumberTarget::TemperatureChange, -150.0);
        assert_eq!(m.input().unwrap().pin.thermal.delta_t, -150.0);
    }

    #[test]
    fn an_elastic_pin_lowers_the_pressure_peak_and_the_rigid_default_is_unchanged() {
        let rigid = input();
        assert!(rigid.pin.body == PinBody::Rigid && rigid.pin.thermal.delta_t == 0.0 && rigid.pin.diameter == 0.499, "the default pin is the plain rigid pin at the fit temperature");
        let mut m = LugUiModel { plastic: false, pin_body: PinBodyChoice::Steel, ..LugUiModel::default() };
        let (e, _) = run(&m.input().unwrap(), None);
        m.pin_body = PinBodyChoice::Rigid;
        let (r, _) = run(&m.input().unwrap(), None);
        let (e, r) = (e.unwrap(), r.unwrap());
        assert!(e.solution.peak_pressure < r.solution.peak_pressure, "{} vs {}", e.solution.peak_pressure, r.solution.peak_pressure);
    }

    #[test]
    fn finite_strain_collapse_runs_is_cached_and_follows_the_failure_strain() {
        let mut m = LugUiModel { finite_strain: true, ..LugUiModel::default() };
        let (first, cache) = run(&m.input().unwrap(), None);
        let first = first.unwrap();
        let f = first.finite.as_ref().expect("a finite-strain result");
        assert!(first.limit.is_none() && f.collapse_lbf > 5000.0 && (f.strain_limited || f.peak_reached), "{f:?}");
        assert!(check(&first, "Ultimate (finite)").detail.contains("failure strain 0.0"), "{}", check(&first, "Ultimate (finite)").detail);
        assert!(first.notes.iter().any(|n| n.contains("library, validated")), "{:?}", first.notes);
        // Only the applied load changes: the collapse is reused.
        m.commit_number(NumberTarget::Load, 3000.0);
        let (again, cache) = run(&m.input().unwrap(), cache);
        assert!(again.as_ref().unwrap().limit_reused);
        // A tougher material (higher failure strain) carries more.
        m.commit_number(NumberTarget::Ductility, 0.5);
        let (tough, _) = run(&m.input().unwrap(), cache);
        let tough = tough.unwrap();
        assert!(!tough.limit_reused && tough.finite.as_ref().unwrap().collapse_lbf > f.collapse_lbf, "{} vs {}", tough.finite.as_ref().unwrap().collapse_lbf, f.collapse_lbf);
    }

    #[test]
    fn finite_strain_covers_oblique_loads_and_bushings_on_the_kernel_and_falls_back_on_the_legacy_solver() {
        // The legacy finite-strain solver is axial and unbushed only: the flow-rule collapse is shown with a note.
        let mut m = LugUiModel { finite_strain: true, solver: SolverChoice::Legacy, ..LugUiModel::default() };
        m.commit_number(NumberTarget::LoadAngle, 30.0);
        let (r, _) = run(&m.input().unwrap(), None);
        let r = r.unwrap();
        assert!(r.finite.is_none() && r.limit.is_some());
        assert!(r.notes.iter().any(|n| n.contains("Finite-strain collapse by the legacy solver is available for axial")));
        // The kernel runs it for the oblique load and for a bushed lug.
        m.solver = SolverChoice::Kernel;
        let (r, _) = run(&m.input().unwrap(), None);
        let r = r.unwrap();
        assert!(r.finite.is_some() && r.limit.is_none() && r.solver == SolverChoice::Kernel, "{:?}", r.notes);
        m.commit_number(NumberTarget::LoadAngle, 0.0);
        m.toggle_bushing();
        let (r, _) = run(&m.input().unwrap(), None);
        assert!(r.unwrap().finite.is_some());
    }

    #[test]
    fn pin_bending_adds_the_slice_checks_and_second_order_runs() {
        let m = LugUiModel { plastic: false, thickness: 0.8, load: 9000.0, pin_bending: true, ..LugUiModel::default() };
        let (r, _) = run(&m.input().unwrap(), None);
        let r = r.unwrap();
        let (t, hoop) = r.thickness.as_ref().expect("pin bending result");
        assert!(t.peaking > 1.0 && *hoop > 0.0);
        assert!(r.checks.iter().any(|c| c.name == "Pin bending") && r.checks.iter().any(|c| c.name == "Hoop (peak slice)"));
        assert!(report_text(&r).contains("Pin bending"));
        // P-delta on a stocky axial lug changes little but must run and pass its self-checks.
        let m = LugUiModel { plastic: false, second_order: true, ..LugUiModel::default() };
        let (r, _) = run(&m.input().unwrap(), None);
        let r = r.unwrap();
        assert_eq!(check(&r, "Solution check").status, Status::Pass);
        assert!(r.notes.iter().any(|n| n.contains("Second-order")));
    }

    #[test]
    fn a_temperature_change_alters_a_bushed_fit() {
        let mut m = LugUiModel { plastic: false, ..LugUiModel::default() };
        m.toggle_bushing();
        let (cold, _) = run(&m.input().unwrap(), None);
        m.commit_number(NumberTarget::TemperatureChange, 150.0);
        let (hot, _) = run(&m.input().unwrap(), None);
        let fit = |r: &LugRun| r.solution.bushing.as_ref().unwrap().fit_pressure_unloaded;
        let (c, h) = (fit(&cold.unwrap()), fit(&hot.unwrap()));
        // Aluminium-bronze bushing in aluminium: both expand, the fit moves; it must differ.
        assert!((c - h).abs() > 0.01 * c.max(1.0), "{c} vs {h}");
    }

    #[test]
    fn the_flow_rules_order_the_collapse_load_and_follow_their_rows() {
        let m = MaterialLimits { name: "t".into(), e_psi: 1e7, nu: 0.33, ftu_psi: 77_000.0, fty_psi: 70_000.0, fsu_psi: 48_000.0, fbru_psi: 0.0, fbru_e15_psi: 0.0, alpha: 12.8e-6, elong: 0.0, failure: None };
        assert_eq!(m.flow_for(FlowRule::Mean, 0.1), m.flow_stress());
        assert_eq!(m.flow_for(FlowRule::Ultimate, 0.1), 77_000.0);
        assert!((m.flow_for(FlowRule::TrueUltimate, 0.10) - 84_700.0).abs() < 1e-6);
        let mut ui = LugUiModel::default();
        ui.mesh_open = true; // the analysis switches sit in the Advanced section
        assert_eq!(ui.flow_rule, FlowRule::Ultimate, "the validated, never-high rule is the default");
        let has = |m: &LugUiModel, r: FieldRow| field_rows(m).contains(&r);
        assert!(has(&ui, FieldRow::ToggleFlowRule) && !has(&ui, FieldRow::Number(NumberTarget::Elongation)));
        ui.flow_rule = FlowRule::TrueUltimate;
        assert!(has(&ui, FieldRow::Number(NumberTarget::Elongation)));
        ui.commit_number(NumberTarget::Elongation, 0.19);
        assert_eq!(ui.input().unwrap().elongation, 0.19);
        ui.finite_strain = true;
        assert!(!has(&ui, FieldRow::ToggleFlowRule), "finite strain replaces the flow rule");
        let collapse = |rule| {
            let ui = LugUiModel { flow_rule: rule, ..LugUiModel::default() };
            run(&ui.input().unwrap(), None).0.unwrap().limit.unwrap().limit_load_lbf
        };
        let (mean, ult, tru) = (collapse(FlowRule::Mean), collapse(FlowRule::Ultimate), collapse(FlowRule::TrueUltimate));
        assert!(mean < ult && ult < tru, "{mean} {ult} {tru}");
    }

    fn bushed_input() -> LugInput {
        let mut m = LugUiModel::default();
        m.toggle_bushing();
        m.plastic = false;
        m.input().unwrap()
    }

    #[test]
    fn a_bushed_lug_reports_the_bushing_checks_and_keeps_its_cache_key_apart() {
        let i = bushed_input();
        assert!(i.bushing.is_some() && i.pin.diameter < 0.375);
        let (res, cache) = run(&i, None);
        let r = res.unwrap();
        let names: Vec<_> = r.checks.iter().map(|c| c.name).collect();
        assert!(names.contains(&"Bushing stress") && names.contains(&"Bushing fit"), "{names:?}");
        assert!(r.solution.bushing.as_ref().unwrap().fit_pressure_unloaded > 1000.0);
        // The same lug without the bushing is a different condensed model.
        let mut plain = i.clone();
        plain.bushing = None;
        plain.pin = PinSpec::new(0.499, 0.15);
        assert!(!run(&plain, cache).0.unwrap().reused_model);
        let text = report_text(&r);
        assert!(text.contains("Bushing") && text.contains("Fit "), "{text}");
    }

    #[test]
    fn the_pin_must_fit_the_bushing_bore_and_the_bore_must_be_smaller_than_the_hole() {
        let mut m = LugUiModel::default();
        m.toggle_bushing();
        m.commit_number(NumberTarget::PinDia, 0.499); // fits the hole, not the 0.375 bushing bore
        assert!(m.input().is_err());
        assert!(m.validation_hint(NumberTarget::PinDia).is_some());
        let mut m = LugUiModel::default();
        m.toggle_bushing();
        m.commit_number(NumberTarget::BushingId, 0.6);
        assert!(m.input().is_err());
        assert!(m.validation_hint(NumberTarget::BushingId).is_some() && m.validation_hint(NumberTarget::HoleDia).is_some());
    }

    #[test]
    fn the_bushed_plastic_collapse_is_cached_with_the_bushing() {
        let mut i = bushed_input();
        i.plastic = true;
        i.mesh.elements_around = 48;
        let (res, cache) = run(&i, None);
        let a = res.unwrap();
        assert!(a.limit.is_some());
        i.case.load_lbf = 2000.0;
        assert!(run(&i, cache).0.unwrap().limit_reused);
    }

    #[test]
    fn the_flow_stress_is_the_validated_edge_check_rule() {
        let m = MaterialLimits { name: "t".into(), e_psi: 1e7, nu: 0.33, ftu_psi: 77_000.0, fty_psi: 70_000.0, fsu_psi: 48_000.0, fbru_psi: 0.0, fbru_e15_psi: 0.0, alpha: 12.8e-6, elong: 0.0, failure: None };
        assert!((m.flow_stress() - 73_500.0).abs() < 1e-6, "(Ftu + Fty)/2");
        let capped = MaterialLimits { fsu_psi: 30_000.0, ..m.clone() };
        assert!((capped.flow_stress() - 3f64.sqrt() * 30_000.0).abs() < 1e-6 || capped.flow_stress() >= 70_000.0, "capped by sqrt(3) Fsu but never below yield");
        let weak = MaterialLimits { fsu_psi: 5_000.0, ..m };
        assert_eq!(weak.flow_stress(), 70_000.0, "never below yield");
    }

    #[test]
    fn an_oblique_load_also_gets_a_collapse() {
        let mut i = input();
        i.case.angle_deg = 45.0;
        i.case.load_lbf = 1500.0;
        let r = run(&i, None).0.unwrap();
        let l = r.limit.expect("full-lug collapse");
        assert!(l.limit_load_lbf > 1500.0, "{}", l.limit_load_lbf);
    }

    #[test]
    fn a_second_run_with_a_different_load_reuses_the_condensed_model() {
        let mut i = input();
        let (_, cache) = run(&i, None);
        i.case.load_lbf = 2500.0;
        let (res, cache2) = run(&i, cache);
        assert!(res.unwrap().reused_model, "the load only changes the contact solve");
        assert!(cache2.is_some());
    }

    #[test]
    fn a_geometry_change_rebuilds_the_model() {
        let mut i = input();
        let (_, cache) = run(&i, None);
        i.geometry.thickness = 0.3;
        let (res, _) = run(&i, cache);
        assert!(!res.unwrap().reused_model);
    }

    #[test]
    fn bearing_margin_is_the_allowable_over_the_applied_stress_minus_one() {
        let i = input();
        let (res, _) = run(&i, None);
        let r = res.unwrap();
        let c = r.checks.iter().find(|c| c.name == "Bearing").unwrap();
        let ed = i.geometry.edge / i.geometry.hole_dia;
        let f = fbru_at_edge_ratio(i.material.fbru_psi, i.material.fbru_e15_psi, ed).unwrap();
        let sb = i.case.load_lbf / (i.geometry.hole_dia * i.geometry.thickness);
        assert!((c.margin.unwrap() - (f / sb - 1.0)).abs() < 1e-9);
    }

    #[test]
    fn a_heavier_load_lowers_every_margin_and_can_fail_bearing() {
        let mut lo = input();
        lo.case.load_lbf = 1000.0;
        let mut hi = input();
        hi.case.load_lbf = 20000.0;
        let a = run(&lo, None).0.unwrap();
        let b = run(&hi, None).0.unwrap();
        for name in ["Bearing", "Net section", "First yield"] {
            let (x, y) = (a.checks.iter().find(|c| c.name == name).unwrap(), b.checks.iter().find(|c| c.name == name).unwrap());
            assert!(y.margin.unwrap() < x.margin.unwrap(), "{name}");
        }
        assert!(b.checks.iter().any(|c| c.status == Status::Fail), "{:?}", b.checks);
    }

    #[test]
    fn a_missing_bearing_allowable_is_explained_not_guessed() {
        let mut i = input();
        i.material.fbru_e15_psi = 0.0; // e/D = 1.5 and no e/D 1.5 value
        let r = run(&i, None).0.unwrap();
        let c = r.checks.iter().find(|c| c.name == "Bearing").unwrap();
        assert_eq!(c.status, Status::Info);
        assert!(c.margin.is_none() && c.detail.contains("no tabulated Fbru"), "{}", c.detail);
    }

    #[test]
    fn a_transverse_load_skips_the_net_section_check_and_notes_the_clamp() {
        let mut i = input();
        i.case.angle_deg = 90.0;
        i.case.load_lbf = 400.0;
        let r = run(&i, None).0.unwrap();
        let c = r.checks.iter().find(|c| c.name == "Net section").unwrap();
        assert_eq!(c.status, Status::Info);
        assert!(r.notes.iter().any(|n| n.contains("clamped")), "{:?}", r.notes);
    }

    #[test]
    fn solver_errors_become_messages() {
        let mut i = input();
        i.pin.diameter = f64::NAN;
        let (res, _) = run(&i, None);
        assert!(res.is_err());
    }

    #[test]
    fn the_kernel_is_the_default_solver_and_the_legacy_solver_stays_available_for_comparison() {
        // Frictionless: the two solvers solve the same smooth problem. (With friction the kernel is the reference -
        // path-resolved and penalty independent - and the legacy solver lands about 8 % lower on the hoop stress.)
        let m = LugUiModel { plastic: false, friction: 0.0, ..LugUiModel::default() };
        let (k, _) = run(&m.input().unwrap(), None);
        let k = k.unwrap();
        assert_eq!(k.solver, SolverChoice::Kernel);
        assert!(k.notes.iter().any(|n| n.contains("general finite-element kernel")), "{:?}", k.notes);
        let legacy = LugUiModel { solver: SolverChoice::Legacy, plastic: false, friction: 0.0, ..LugUiModel::default() };
        let (l, _) = run(&legacy.input().unwrap(), None);
        let l = l.unwrap();
        assert_eq!(l.solver, SolverChoice::Legacy);
        assert!(l.comparison.is_none() && !l.notes.iter().any(|n| n.contains("general finite-element kernel")));
        // Same lug, two independent solvers: the peak hoop stress agrees to a few percent.
        let rel = (k.solution.peak_hoop / l.solution.peak_hoop - 1.0).abs();
        assert!(rel < 0.06, "kernel {} vs legacy {}", k.solution.peak_hoop, l.solution.peak_hoop);
    }

    #[test]
    fn compare_runs_both_and_keeps_the_kernel_result_primary() {
        let m = LugUiModel { solver: SolverChoice::Compare, plastic: true, friction: 0.0, ..LugUiModel::default() };
        let (r, _) = run(&m.input().unwrap(), None);
        let r = r.unwrap();
        assert_eq!(r.solver, SolverChoice::Kernel);
        let c = r.comparison.as_ref().expect("the legacy result beside it");
        assert!(c.limit.is_some() && r.limit.is_some() && c.ms > 0.0);
        assert!((r.solution.peak_hoop / c.solution.peak_hoop - 1.0).abs() < 0.06);
        let (kc, lc) = (r.limit.as_ref().unwrap().limit_load_lbf, c.limit.as_ref().unwrap().limit_load_lbf);
        assert!((kc / lc - 1.0).abs() < 0.08, "collapse kernel {kc} vs legacy {lc}");
    }

    #[test]
    fn a_case_the_kernel_cannot_run_falls_back_to_the_legacy_solver_with_a_note() {
        // A meshed elastic pin with friction on the full (oblique) lug needs the clevis torque reaction.
        let mut m = LugUiModel { plastic: false, pin_body: PinBodyChoice::Steel, ..LugUiModel::default() };
        m.commit_number(NumberTarget::LoadAngle, 30.0);
        let (r, _) = run(&m.input().unwrap(), None);
        let r = r.unwrap();
        assert_eq!(r.solver, SolverChoice::Legacy);
        assert!(r.notes.iter().any(|n| n.contains("The kernel solver could not run this case") && n.contains("legacy")), "{:?}", r.notes);
        assert!(r.solution.peak_hoop > 10_000.0);
    }

    #[test]
    fn the_kernel_runs_every_feature_of_the_toolbox() {
        // Bushing with a thermal change, an oblique load, second order and the collapse: nothing falls back.
        let mut m = LugUiModel { plastic: true, second_order: true, delta_t: 60.0, ..LugUiModel::default() };
        m.toggle_bushing();
        m.commit_number(NumberTarget::LoadAngle, 20.0);
        m.commit_number(NumberTarget::Load, 1500.0);
        let (r, _) = run(&m.input().unwrap(), None);
        let r = r.unwrap();
        eprintln!("features: {:.1} s, notes {:?}", r.total_ms / 1e3, r.notes);
        assert_eq!(r.solver, SolverChoice::Kernel, "{:?}", r.notes);
        assert!(r.solution.bushing.is_some() && r.limit.is_some());
        assert!(!r.notes.iter().any(|n| n.contains("legacy solver")), "{:?}", r.notes);
    }

    /// Timing probe (ignored): the default case on each solver.
    #[test]
    #[ignore]
    fn default_case_timing() {
        for (name, solver, finite, angle) in [("kernel", SolverChoice::Kernel, false, 0.0), ("legacy", SolverChoice::Legacy, false, 0.0), ("kernel finite", SolverChoice::Kernel, true, 0.0), ("kernel oblique", SolverChoice::Kernel, false, 45.0), ("legacy oblique", SolverChoice::Legacy, false, 45.0)] {
            let mut m = LugUiModel { solver, finite_strain: finite, ..LugUiModel::default() };
            m.commit_number(NumberTarget::LoadAngle, angle);
            let t = std::time::Instant::now();
            let (r, _) = run(&m.input().unwrap(), None);
            let r = r.unwrap();
            eprintln!("{name}: {:.0} ms total, {} nodes {} dofs, notes {:?} (hoop {:.0}, collapse {:.0})", t.elapsed().as_secs_f64() * 1e3, r.solution.mesh.nodes, r.solution.dofs, r.notes, r.solution.peak_hoop, r.limit.as_ref().map_or(r.finite.as_ref().map_or(0.0, |f| f.collapse_lbf), |l| l.limit_load_lbf));
        }
    }
}
