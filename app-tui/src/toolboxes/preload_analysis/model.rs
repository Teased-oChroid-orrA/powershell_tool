//! Bridges the pure `fastened-joint-solver` engine into UI-facing state,
//! the same split every other toolbox in this crate uses: zero
//! `ratatui`/`crossterm` here - only `mod.rs` (key routing) and `view.rs`
//! (rendering) know about the terminal.
//!
//! **UI scope note**: the solver crate accepts an arbitrary
//! `Vec<FastenerSegment>` (shank/free-thread/head/thread-engagement
//! regions individually), but this toolbox's field list edits exactly one
//! physical shank segment - the head and thread-engagement additional
//! elastic-length segments are appended automatically by
//! `fastened_joint_solver::compliance::build_fastener_segments` inside the
//! solver itself. A future revision could expose a fully editable
//! multi-segment fastener profile (reduced shank, separate free-thread
//! length, ...) the same way the member stack below is editable, but the
//! underlying solver already supports it without any change - this is a
//! UI scope decision, not a solver limitation.

use fastened_joint_solver::bearing::BearingPressureModel;
use fastened_joint_solver::compliance::{FastenerSegment, Member, MemberStack};
use fastened_joint_solver::solve::{compute, AnalysisMode, FrictionInputs, JointGeometry, JointInputs, JointSolution, MonteCarloSettings, TighteningMember, UncertaintyInputs};
use fastened_joint_solver::thread_load_distribution::ThreadLoadDistributionInputs;
use fastened_joint_solver::thread::ThreadGeometry;
use fastened_joint_solver::thread_catalog::{find_matching, BoltCatalogEntry, AN_BOLT_CATALOG};
use fastened_joint_solver::validation::ValidationError;

pub const MAX_MEMBERS: usize = 6;
pub const MIN_MEMBERS: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    TorqueControlled,
    PreloadControlled,
    RotationControlled,
}

impl Mode {
    pub fn cycle(self) -> Self {
        match self {
            Mode::TorqueControlled => Mode::PreloadControlled,
            Mode::PreloadControlled => Mode::RotationControlled,
            Mode::RotationControlled => Mode::TorqueControlled,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::TorqueControlled => "Torque Controlled",
            Mode::PreloadControlled => "Preload Controlled",
            Mode::RotationControlled => "Nut Rotation Controlled",
        }
    }
}

pub fn cycle_tightening_from(t: TighteningMember) -> TighteningMember {
    match t {
        TighteningMember::Nut => TighteningMember::BoltHead,
        TighteningMember::BoltHead => TighteningMember::Nut,
    }
}

pub fn label_tightening_from(t: TighteningMember) -> &'static str {
    match t {
        TighteningMember::Nut => "Nut Turned, Head Held",
        TighteningMember::BoltHead => "Head Turned, Nut Held",
    }
}

pub fn cycle_bearing_model(m: BearingPressureModel) -> BearingPressureModel {
    match m {
        BearingPressureModel::UniformPressure => BearingPressureModel::UniformWear,
        BearingPressureModel::UniformWear => BearingPressureModel::UniformPressure,
    }
}

pub fn label_bearing_model(m: BearingPressureModel) -> &'static str {
    match m {
        BearingPressureModel::UniformPressure => "Uniform Pressure",
        BearingPressureModel::UniformWear => "Uniform Wear",
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberUi {
    /// Display role (`Plate`, `Washer`, `Shim`, ...) - a label for the stack
    /// diagram only; the solver sees thickness/modulus/hole/outer diameter.
    pub name: &'static str,
    pub thickness: f64,
    pub e: f64,
    pub hole_diameter: f64,
    /// `0.0` means "unbounded" (no outer-geometry truncation) - same
    /// zero-sentinel convention `BushingModel::bore_capability_min_width`
    /// already uses in this crate.
    pub outer_diameter: f64,
}

impl Default for MemberUi {
    fn default() -> Self {
        Self { name: "Plate", thickness: 10.0, e: 70_000.0, hole_diameter: 11.0, outer_diameter: 0.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberTarget {
    AppliedTorque,
    TargetPreload,
    TurnsAfterSnug,
    SnugPreload,
    MuThread,
    MuBearing,
    PrevailingTorque,
    BearingInnerRadius,
    BearingOuterRadius,
    HeadBearingInnerRadius,
    HeadBearingOuterRadius,
    ConeHalfAngleDeg,
    ThreadMajorDia,
    ThreadPitchDia,
    ThreadRootDia,
    ThreadPitch,
    ThreadStarts,
    ThreadAngleDeg,
    FastenerE,
    FastenerNu,
    ShankLength,
    ShankDiameter,
    ExternalAxialLoad,
    AppliedShearLoad,
    FrictionInterfaceMu,
    ProofLoad,
    YieldStrength,
    UltimateStrength,
    MemberThickness(usize),
    MemberModulus(usize),
    MemberHoleDiameter(usize),
    MemberOuterDiameter(usize),
    MuThreadTolPct,
    MuBearingTolPct,
    PitchDiameterTol,
    BearingRadiusTol,
    TorqueTolPct,
    MonteCarloSamples,
    MonteCarloSeed,
    EngagedThreads,
    NutModulus,
    NutPoissonRatio,
    NutOuterDiameter,
    EmbedmentSettlement,
    ThreadEngagementLength,
    ThreadShearStrength,
}

impl NumberTarget {
    pub fn label(self) -> String {
        match self {
            NumberTarget::AppliedTorque => "Applied Torque".to_string(),
            NumberTarget::TargetPreload => "Target Preload".to_string(),
            NumberTarget::TurnsAfterSnug => "Turns After Snug".to_string(),
            NumberTarget::SnugPreload => "Snug Reference Preload".to_string(),
            NumberTarget::MuThread => "Thread Friction (mu_thread)".to_string(),
            NumberTarget::MuBearing => "Bearing Friction (mu_bearing)".to_string(),
            NumberTarget::PrevailingTorque => "Prevailing Torque".to_string(),
            NumberTarget::BearingInnerRadius => "Bearing Contact Inner Radius".to_string(),
            NumberTarget::BearingOuterRadius => "Bearing Contact Outer Radius".to_string(),
            NumberTarget::HeadBearingInnerRadius => "Head Bearing Inner Radius (0 = same as nut)".to_string(),
            NumberTarget::HeadBearingOuterRadius => "Head Bearing Outer Radius (0 = same as nut)".to_string(),
            NumberTarget::ConeHalfAngleDeg => "Pressure-Cone Half-Angle".to_string(),
            NumberTarget::ThreadMajorDia => "Major Diameter (d)".to_string(),
            NumberTarget::ThreadPitchDia => "Pitch Diameter (d2)".to_string(),
            NumberTarget::ThreadRootDia => "Root Diameter (d3)".to_string(),
            NumberTarget::ThreadPitch => "Thread Pitch (p)".to_string(),
            NumberTarget::ThreadStarts => "Thread Starts".to_string(),
            NumberTarget::ThreadAngleDeg => "Thread Included Angle".to_string(),
            NumberTarget::FastenerE => "Fastener Modulus (E)".to_string(),
            NumberTarget::FastenerNu => "Fastener Poisson's Ratio".to_string(),
            NumberTarget::ShankLength => "Shank Length".to_string(),
            NumberTarget::ShankDiameter => "Shank Diameter".to_string(),
            NumberTarget::ExternalAxialLoad => "External Axial Load".to_string(),
            NumberTarget::AppliedShearLoad => "Applied Shear Load".to_string(),
            NumberTarget::FrictionInterfaceMu => "Interface Friction (slip)".to_string(),
            NumberTarget::ProofLoad => "Proof Load".to_string(),
            NumberTarget::YieldStrength => "Yield Strength".to_string(),
            NumberTarget::UltimateStrength => "Ultimate Strength".to_string(),
            NumberTarget::MemberThickness(i) => format!("Member {} Thickness", i + 1),
            NumberTarget::MemberModulus(i) => format!("Member {} Modulus (E)", i + 1),
            NumberTarget::MemberHoleDiameter(i) => format!("Member {} Hole Diameter", i + 1),
            NumberTarget::MemberOuterDiameter(i) => format!("Member {} Outer Diameter (0=unbounded)", i + 1),
            NumberTarget::MuThreadTolPct => "Thread Friction Tolerance".to_string(),
            NumberTarget::MuBearingTolPct => "Bearing Friction Tolerance".to_string(),
            NumberTarget::PitchDiameterTol => "Pitch Diameter Tolerance".to_string(),
            NumberTarget::BearingRadiusTol => "Bearing Radius Tolerance".to_string(),
            NumberTarget::TorqueTolPct => "Applied Torque Tolerance".to_string(),
            NumberTarget::MonteCarloSamples => "Monte Carlo Samples".to_string(),
            NumberTarget::MonteCarloSeed => "Monte Carlo Seed".to_string(),
            NumberTarget::EngagedThreads => "Engaged Thread Count".to_string(),
            NumberTarget::NutModulus => "Nut/Tapped-Hole Modulus (E)".to_string(),
            NumberTarget::NutPoissonRatio => "Nut/Tapped-Hole Poisson's Ratio".to_string(),
            NumberTarget::NutOuterDiameter => "Nut Outer / Across-Flats-Equivalent Diameter".to_string(),
            NumberTarget::EmbedmentSettlement => "Embedment / Settlement (delta_embed)".to_string(),
            NumberTarget::ThreadEngagementLength => "Thread Engagement Length".to_string(),
            NumberTarget::ThreadShearStrength => "Thread Shear Strength".to_string(),
        }
    }

    fn is_angle(self) -> bool {
        matches!(self, NumberTarget::ConeHalfAngleDeg | NumberTarget::ThreadAngleDeg)
    }

    /// Every input is imperial (in, lbf, in-lbf, psi) - the same
    /// convention `bushing-solver`/`pressure-vessel-solver` already fix
    /// throughout this repo (see `bushing-solver/AGENTS.md`'s Contracts:
    /// "a metric value silently produces a wrong-but-plausible number, not
    /// an error" - showing the unit on every field is this toolbox's own
    /// guard against that same failure mode, since nothing here is
    /// type-enforced either).
    pub fn format_value(self, value: f64) -> String {
        match self {
            NumberTarget::MuThread | NumberTarget::MuBearing | NumberTarget::FastenerNu | NumberTarget::FrictionInterfaceMu | NumberTarget::NutPoissonRatio => format!("{value:.3}"),
            NumberTarget::ThreadStarts | NumberTarget::EngagedThreads => format!("{value:.0}"),
            _ if self.is_angle() => format!("{value:.2} deg"),
            NumberTarget::AppliedTorque | NumberTarget::PrevailingTorque => format!("{value:.2} in-lbf"),
            NumberTarget::TargetPreload
            | NumberTarget::SnugPreload
            | NumberTarget::ExternalAxialLoad
            | NumberTarget::AppliedShearLoad
            | NumberTarget::ProofLoad => format!("{value:.1} lbf"),
            NumberTarget::FastenerE | NumberTarget::YieldStrength | NumberTarget::UltimateStrength | NumberTarget::MemberModulus(_) | NumberTarget::NutModulus | NumberTarget::ThreadShearStrength => {
                format!("{value:.0} psi")
            }
            NumberTarget::TurnsAfterSnug => format!("{value:.4} rev"),
            NumberTarget::MuThreadTolPct | NumberTarget::MuBearingTolPct | NumberTarget::TorqueTolPct => format!("\u{b1}{value:.1}%"),
            NumberTarget::PitchDiameterTol | NumberTarget::BearingRadiusTol => format!("\u{b1}{value:.4} in"),
            NumberTarget::MonteCarloSamples => format!("{value:.0}"),
            NumberTarget::MonteCarloSeed => format!("{value:.0}"),
            _ => format!("{value:.4} in"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRow {
    /// A non-selectable section divider - see `bushing/model.rs::FieldRow::Header`'s
    /// doc comment for the same reasoning, applied here to this toolbox's
    /// own ~40-field list.
    Header(&'static str),
    ToggleMode,
    OpenBoltPicker,
    OpenTemplatePicker,
    ToggleTighteningFrom,
    /// Opens and closes the Advanced section (rows after it in `field_rows`).
    AdvancedSection,
    ToggleBearingModel,
    ToggleMemberStiffness,
    ToggleExternalLoadEnabled,
    ToggleSlipEnabled,
    ToggleStrengthLimitsEnabled,
    ToggleAddMember,
    ToggleRemoveMember,
    ToggleUncertaintyEnabled,
    ToggleMonteCarloEnabled,
    ToggleThreadLoadDistributionEnabled,
    Number(NumberTarget),
}

pub fn row_label(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(text) => text,
        FieldRow::ToggleMode => "Analysis Mode",
        FieldRow::OpenBoltPicker => "Bolt (AN Standard)",
        FieldRow::OpenTemplatePicker => "Joint Template",
        FieldRow::ToggleTighteningFrom => "Tightening From",
        FieldRow::AdvancedSection => "Advanced settings",
        FieldRow::ToggleBearingModel => "Bearing Pressure Model",
        FieldRow::ToggleMemberStiffness => "Member Stiffness",
        FieldRow::ToggleExternalLoadEnabled => "External Service Load",
        FieldRow::ToggleSlipEnabled => "Transverse Load / Slip Check",
        FieldRow::ToggleStrengthLimitsEnabled => "Material Strength Limits",
        FieldRow::ToggleAddMember => "Add Joint Member",
        FieldRow::ToggleRemoveMember => "Remove Last Joint Member",
        FieldRow::ToggleUncertaintyEnabled => "Uncertainty Analysis",
        FieldRow::ToggleMonteCarloEnabled => "Monte Carlo Sampling",
        FieldRow::ToggleThreadLoadDistributionEnabled => "Thread Load Distribution",
        FieldRow::Number(_) => "",
    }
}

/// One-line description shown in the bottom "Hint" panel while this row is
/// selected - same purpose as `bushing/model.rs::field_hint`.
pub fn field_hint(row: FieldRow) -> &'static str {
    match row {
        FieldRow::Header(_) => "",
        FieldRow::ToggleMode => "Torque Controlled solves preload from applied torque (the physical installation process). Preload Controlled and Nut Rotation Controlled invert the solve for design studies.",
        FieldRow::OpenBoltPicker => "Pick a standard AN3-AN20 aerospace bolt to auto-fill every thread dimension and the shank diameter below - Esc closes without changing anything, and every filled-in value can still be hand-edited afterward.",
        FieldRow::OpenTemplatePicker => "Pick a common bolted-joint stack-up (washers, plates, shim, fitting, nut...) or press g in the window to generate random joints until one looks right. Applying it fills the member stack, bearing radii, shank length, nut data and a typical torque, then analyzes it - everything stays editable.",
        FieldRow::ToggleTighteningFrom => "Which component actually rotates during installation - the friction torque path (and which bearing surface's diameter matters) differs between the two.",
        FieldRow::Number(NumberTarget::AppliedTorque) => "Installation torque applied at the wrench. The solver finds the preload whose full thread+bearing+prevailing torque equilibrium matches this value.",
        FieldRow::Number(NumberTarget::TargetPreload) => "Desired clamping force - the solver reports the installation torque that would produce it.",
        FieldRow::Number(NumberTarget::TurnsAfterSnug) => "Relative nut-to-bolt revolutions past the snug reference state - the solver derives preload from the elastic closure this rotation implies.",
        FieldRow::Number(NumberTarget::SnugPreload) => "Reference preload at the snug point this rotation is measured from - almost always 0.",
        FieldRow::Number(NumberTarget::MuThread) => "Thread-thread friction coefficient - the dominant term in the full V-thread torque equation.",
        FieldRow::Number(NumberTarget::MuBearing) => "Friction coefficient at the rotating bearing face (under the head, nut, or washer).",
        FieldRow::ToggleBearingModel => "Contact-pressure assumption under the bearing face - Uniform Pressure (new/rigid parts) or Uniform Wear (broken-in/softer parts) - changes the effective friction radius, not the friction coefficient itself.",
        FieldRow::AdvancedSection => "Friction model, member stiffness model, thread detail (filled in by the Bolt picker), member outer geometry, uncertainty and thread load distribution. Enter, Space or a click opens and closes the section; the solve uses these values whether it is open or not.",
        FieldRow::ToggleMemberStiffness => "How the clamped members' compliance is found. Pressure Cone uses the half-angle below. Finite Element meshes the stack on the general FE kernel (fea-core) and uses the cone half-angle that reproduces its compliance, so the whole solve (preload, load fraction, separation, slip) follows the FE value. Falls back to the cone angle while the FE result is pending or unavailable.",
        FieldRow::Number(NumberTarget::PrevailingTorque) => "Constant torque from a self-locking nut/insert, independent of preload - kept separate from the preload-producing friction torque.",
        FieldRow::Number(NumberTarget::BearingInnerRadius) => "Inner radius of the nut-side annular bearing contact (the clearance hole radius). Always used for clamped-member contact pressure, and for the torque solve when Tightening From is Nut.",
        FieldRow::Number(NumberTarget::BearingOuterRadius) => "Outer radius of the nut-side annular bearing contact (nut/washer bearing face radius).",
        FieldRow::Number(NumberTarget::HeadBearingInnerRadius) => "Inner radius of the bolt-head-side bearing contact, used for the torque solve only when Tightening From is Head Turned. 0 assumes it matches the nut-side inner radius above.",
        FieldRow::Number(NumberTarget::HeadBearingOuterRadius) => "Outer radius of the bolt-head-side bearing contact, used for the torque solve only when Tightening From is Head Turned. 0 assumes it matches the nut-side outer radius above.",
        FieldRow::Number(NumberTarget::ConeHalfAngleDeg) => "Rotscher pressure-cone half-angle for the clamped-member compliance model - 30 deg is a common assumption absent more specific data.",
        FieldRow::Number(NumberTarget::ThreadMajorDia) => "Nominal/major thread diameter (d) - use the Bolt picker above to fill this from a real standard size.",
        FieldRow::Number(NumberTarget::ThreadPitchDia) => "Pitch diameter (d2) - the primary lever arm in the thread-torque equation.",
        FieldRow::Number(NumberTarget::ThreadRootDia) => "Minor/root diameter (d3) - governs the thread-root stress section and torsional stiffness.",
        FieldRow::Number(NumberTarget::ThreadPitch) => "Axial distance between adjacent threads (1/TPI for a standard inch series).",
        FieldRow::Number(NumberTarget::ThreadStarts) => "Number of thread starts - 1 for a standard single-start thread; lead = starts x pitch.",
        FieldRow::Number(NumberTarget::ThreadAngleDeg) => "Full included thread angle - 60 deg for UN/UNF/UNC and metric ISO threads.",
        FieldRow::Number(NumberTarget::FastenerE) => "Fastener elastic modulus - drives axial/torsional compliance and every stress-to-strain conversion.",
        FieldRow::Number(NumberTarget::FastenerNu) => "Fastener Poisson's ratio - used to derive shear modulus G = E / (2(1+nu)) for torsional twist.",
        FieldRow::Number(NumberTarget::ShankLength) => "Unthreaded shank length between the head and the start of engaged threads.",
        FieldRow::Number(NumberTarget::ShankDiameter) => "Shank diameter - also used as the Shank stress section's diameter in the Results stress table.",
        FieldRow::ToggleExternalLoadEnabled => "Model an additional external axial service load applied to the already-preloaded joint (separation/load-sharing behavior).",
        FieldRow::Number(NumberTarget::ExternalAxialLoad) => "External axial load applied after installation - shared between the fastener and the clamped members by their relative stiffness.",
        FieldRow::ToggleSlipEnabled => "Evaluate friction-grip slip resistance against an applied transverse (shear) load.",
        FieldRow::Number(NumberTarget::FrictionInterfaceMu) => "Friction coefficient at the clamped interface resisting transverse slip.",
        FieldRow::Number(NumberTarget::AppliedShearLoad) => "Applied transverse load compared against the slip capacity.",
        FieldRow::ToggleStrengthLimitsEnabled => "Enter material strength limits to get yield/ultimate margins of safety on every stress section.",
        FieldRow::Number(NumberTarget::ProofLoad) => "Proof load - triggers a warning if the solved preload exceeds it.",
        FieldRow::Number(NumberTarget::YieldStrength) => "Material yield strength - drives the yield margin of safety on every stress section.",
        FieldRow::Number(NumberTarget::UltimateStrength) => "Material ultimate strength - drives the ultimate margin of safety on every stress section.",
        FieldRow::Number(NumberTarget::MemberThickness(_)) => "This member's thickness along the clamped stack.",
        FieldRow::Number(NumberTarget::MemberModulus(_)) => "This member's elastic modulus.",
        FieldRow::Number(NumberTarget::MemberHoleDiameter(_)) => "This member's clearance-hole diameter.",
        FieldRow::Number(NumberTarget::MemberOuterDiameter(_)) => "This member's own available outer geometry - truncates the pressure cone if narrower than its ideal growth; 0 means unbounded.",
        FieldRow::ToggleAddMember => "Append another member to the clamped stack (up to 6 - washers count as members).",
        FieldRow::ToggleRemoveMember => "Remove the last member in the clamped stack (at least 1 must remain).",
        FieldRow::ToggleUncertaintyEnabled => "Evaluate a worst-case preload range across friction/geometry/torque tolerance - deterministic, exhaustive over every +/- corner (only meaningful in Torque Controlled mode).",
        FieldRow::Number(NumberTarget::MuThreadTolPct) => "Thread friction +/- tolerance as a percent of its nominal value.",
        FieldRow::Number(NumberTarget::MuBearingTolPct) => "Bearing friction +/- tolerance as a percent of its nominal value.",
        FieldRow::Number(NumberTarget::PitchDiameterTol) => "Pitch diameter +/- tolerance band.",
        FieldRow::Number(NumberTarget::BearingRadiusTol) => "Bearing contact outer radius +/- tolerance band.",
        FieldRow::Number(NumberTarget::TorqueTolPct) => "Applied torque +/- tolerance as a percent (wrench/gauge accuracy).",
        FieldRow::ToggleMonteCarloEnabled => "Also run a seeded random sample within the same tolerance bounds above, reporting a mean/std-dev in addition to the worst-case min/max.",
        FieldRow::Number(NumberTarget::MonteCarloSamples) => "Number of random samples to draw - more samples narrow the statistical noise but take proportionally longer.",
        FieldRow::Number(NumberTarget::MonteCarloSeed) => "Random seed - the same seed always reproduces the exact same sample set and statistics.",
        FieldRow::ToggleThreadLoadDistributionEnabled => "Advanced: model each engaged thread as a coupled spring (bolt-body, nut-body, and thread-engagement stiffness) and solve for the load each individual thread carries, rather than assuming it is spread evenly. Optional per the originating spec - off by default.",
        FieldRow::Number(NumberTarget::EngagedThreads) => "Number of threads actually engaged in the joint (roughly engagement length / thread pitch).",
        FieldRow::Number(NumberTarget::NutModulus) => "Elastic modulus of the nut or tapped-hole material - only needs to differ from the fastener's own modulus for a dissimilar-material joint.",
        FieldRow::Number(NumberTarget::NutPoissonRatio) => "Poisson's ratio of the nut or tapped-hole material.",
        FieldRow::Number(NumberTarget::NutOuterDiameter) => "Nut outer diameter (or an across-flats-equivalent diameter) - sets the nut-body cross-section carrying load between engaged threads.",
        FieldRow::Number(NumberTarget::EmbedmentSettlement) => "Settlement displacement consumed by surface flattening/coating compression/seating rather than elastic stretch - reduces the preload this rotation actually achieves. 0 means not modeled.",
        FieldRow::Number(NumberTarget::ThreadEngagementLength) => "Actual physical thread engagement length for the thread-shear check below - distinct from the VDI 2230 elastic-equivalent length used in the compliance model.",
        FieldRow::Number(NumberTarget::ThreadShearStrength) => "Material shear strength - both this and Thread Engagement Length must be set to compute a thread-shear margin of safety.",
    }
}

#[derive(Clone)]
pub struct PreloadModel {
    pub mode: Mode,
    pub applied_torque: f64,
    pub target_preload: f64,
    pub turns_after_snug: f64,
    pub snug_preload: f64,

    pub mu_thread: f64,
    pub mu_bearing: f64,
    pub bearing_model: BearingPressureModel,
    pub prevailing_torque: f64,
    pub tightening_from: TighteningMember,

    pub bearing_inner_radius: f64,
    pub bearing_outer_radius: f64,
    /// `0.0` means "same as the nut-side pair above" - same zero-sentinel
    /// convention `MemberUi::outer_diameter` already uses in this file.
    pub head_bearing_inner_radius: f64,
    pub head_bearing_outer_radius: f64,
    pub cone_half_angle_deg: f64,
    /// Member compliance from the finite-element model (`fe_check`) instead of the pressure cone.
    pub member_stiffness_fe: bool,
    /// The Advanced section of the field list is open.
    pub advanced_open: bool,
    /// Cone half angle matching the FE compliance of exactly the current joint; set by `PreloadAnalysisState::sync_fe_angle`.
    pub fe_angle_deg: Option<f64>,

    pub thread_major_dia: f64,
    pub thread_pitch_dia: f64,
    pub thread_root_dia: f64,
    pub thread_pitch: f64,
    pub thread_starts: f64,
    pub thread_angle_deg: f64,

    pub fastener_e: f64,
    pub fastener_nu: f64,
    pub shank_length: f64,
    pub shank_diameter: f64,

    /// `0.0` means "not modeled" (spec section 24: "if embedment is not
    /// enabled, delta_embed = 0") - only affects the solve in
    /// `Mode::RotationControlled`, shown only for that mode.
    pub embedment_settlement: f64,

    pub members: Vec<MemberUi>,

    pub external_load_enabled: bool,
    pub external_axial_load: f64,

    pub slip_enabled: bool,
    pub friction_interface_mu: f64,
    pub applied_shear_load: f64,

    pub strength_limits_enabled: bool,
    pub proof_load: f64,
    pub yield_strength: f64,
    pub ultimate_strength: f64,
    /// `0.0` on either disables the thread-shear check (spec section 35) -
    /// both are needed to derive an area and a margin.
    pub thread_engagement_length: f64,
    pub thread_shear_strength: f64,

    pub uncertainty_enabled: bool,
    /// Stored as a percent (20.0 = +/-20%), not a fraction - matches what
    /// the edit buffer and `NumberTarget::format_value` both show directly;
    /// only `build_inputs` converts to the fraction the solver expects.
    pub mu_thread_tol_pct: f64,
    pub mu_bearing_tol_pct: f64,
    pub pitch_diameter_tol: f64,
    pub bearing_radius_tol: f64,
    pub torque_tol_pct: f64,
    pub monte_carlo_enabled: bool,
    pub monte_carlo_samples: f64,
    pub monte_carlo_seed: f64,

    pub thread_load_distribution_enabled: bool,
    pub engaged_threads: f64,
    pub nut_modulus: f64,
    pub nut_poisson_ratio: f64,
    pub nut_outer_diameter: f64,

    pub output: Result<JointSolution, ValidationError>,
}

impl Default for PreloadModel {
    fn default() -> Self {
        // Defaults model a real AN6 (.3750-24 UNF) aerospace structural
        // bolt through a 0.75in aluminum joint - self-consistent imperial
        // units throughout (in, lbf, in-lbf, psi), matching
        // `bushing-solver`/`pressure-vessel-solver`'s own fixed unit
        // convention elsewhere in this repo. Thread dimensions are AN6's
        // own catalog entry (see `fastened_joint_solver::thread_catalog`),
        // so the Bolt row shows "AN6" rather than "Custom" out of the box;
        // 360 in-lbf is a physically reasonable dry installation torque
        // for this size (order-of-magnitude, not a specific published spec
        // value for this exact designation).
        let an6 = &AN_BOLT_CATALOG[3]; // AN3,AN4,AN5,AN6 - index 3
        let g = an6.thread_geometry();
        let mut model = Self {
            mode: Mode::default(),
            applied_torque: 360.0,
            target_preload: 2400.0,
            turns_after_snug: 0.5,
            snug_preload: 0.0,
            mu_thread: 0.15,
            mu_bearing: 0.15,
            bearing_model: BearingPressureModel::UniformPressure,
            prevailing_torque: 0.0,
            tightening_from: TighteningMember::Nut,
            bearing_inner_radius: 0.203,
            bearing_outer_radius: 0.40,
            head_bearing_inner_radius: 0.0,
            head_bearing_outer_radius: 0.0,
            cone_half_angle_deg: 30.0,
            member_stiffness_fe: false,
            advanced_open: false,
            fe_angle_deg: None,
            thread_major_dia: g.d,
            thread_pitch_dia: g.d2,
            thread_root_dia: g.d3,
            thread_pitch: g.pitch,
            thread_starts: g.starts as f64,
            thread_angle_deg: g.thread_angle_deg,
            fastener_e: 29_000_000.0,
            fastener_nu: 0.29,
            shank_length: 1.0,
            shank_diameter: g.d,
            embedment_settlement: 0.0,
            members: vec![MemberUi { name: "Plate", thickness: 0.75, e: 10_300_000.0, hole_diameter: 0.406, outer_diameter: 0.0 }],
            external_load_enabled: false,
            external_axial_load: 500.0,
            slip_enabled: false,
            friction_interface_mu: 0.3,
            applied_shear_load: 500.0,
            strength_limits_enabled: false,
            proof_load: 0.0,
            yield_strength: 0.0,
            ultimate_strength: 0.0,
            thread_engagement_length: 0.0,
            thread_shear_strength: 0.0,
            uncertainty_enabled: false,
            mu_thread_tol_pct: 20.0,
            mu_bearing_tol_pct: 20.0,
            pitch_diameter_tol: 0.001,
            bearing_radius_tol: 0.01,
            torque_tol_pct: 10.0,
            monte_carlo_enabled: false,
            monte_carlo_samples: 2000.0,
            monte_carlo_seed: 42.0,
            thread_load_distribution_enabled: false,
            // ~8 engaged threads is a common rule-of-thumb nut thickness
            // (~0.8x major diameter) divided by AN6's pitch - a starting
            // point, not a computed value; same steel properties as the
            // fastener by default (same 29e6 psi / 0.29), and a nut
            // circumscribed diameter roughly matching an AN6 hex nut's
            // across-corners dimension.
            engaged_threads: 8.0,
            nut_modulus: 29_000_000.0,
            nut_poisson_ratio: 0.29,
            nut_outer_diameter: 0.65,
            output: Err(ValidationError::EmptyMemberStack),
        };
        model.recompute();
        model
    }
}

/// The complete navigable row list for the current mode/toggle selection -
/// only fields relevant right now are ever shown (spec section 66's
/// Basic/Advanced split is approximated here by hiding whole sections
/// behind their own enable toggle rather than a separate mode, consistent
/// with how this crate's other toolboxes already gate advanced fields).
pub fn field_rows(model: &PreloadModel) -> Vec<FieldRow> {
    let mut rows = vec![FieldRow::Header("Tightening"), FieldRow::ToggleMode];
    match model.mode {
        Mode::TorqueControlled => rows.push(FieldRow::Number(NumberTarget::AppliedTorque)),
        Mode::PreloadControlled => rows.push(FieldRow::Number(NumberTarget::TargetPreload)),
        Mode::RotationControlled => {
            rows.push(FieldRow::Number(NumberTarget::TurnsAfterSnug));
            rows.push(FieldRow::Number(NumberTarget::SnugPreload));
            rows.push(FieldRow::Number(NumberTarget::EmbedmentSettlement));
        }
    }
    rows.push(FieldRow::ToggleTighteningFrom);
    if model.tightening_from == TighteningMember::BoltHead {
        rows.push(FieldRow::Number(NumberTarget::HeadBearingInnerRadius));
        rows.push(FieldRow::Number(NumberTarget::HeadBearingOuterRadius));
    }

    rows.push(FieldRow::Header("Friction"));
    rows.push(FieldRow::Number(NumberTarget::MuThread));
    rows.push(FieldRow::Number(NumberTarget::MuBearing));

    rows.push(FieldRow::Header("Bearing Geometry"));
    rows.push(FieldRow::Number(NumberTarget::BearingInnerRadius));
    rows.push(FieldRow::Number(NumberTarget::BearingOuterRadius));

    rows.push(FieldRow::Header("Fastener"));
    rows.push(FieldRow::OpenBoltPicker);
    rows.push(FieldRow::Number(NumberTarget::ThreadMajorDia));
    rows.push(FieldRow::Number(NumberTarget::ShankLength));
    rows.push(FieldRow::Number(NumberTarget::ShankDiameter));

    rows.push(FieldRow::Header("Joint Stack"));
    rows.push(FieldRow::OpenTemplatePicker);
    for i in 0..model.members.len() {
        rows.push(FieldRow::Number(NumberTarget::MemberThickness(i)));
        rows.push(FieldRow::Number(NumberTarget::MemberModulus(i)));
        rows.push(FieldRow::Number(NumberTarget::MemberHoleDiameter(i)));
    }
    if model.members.len() < MAX_MEMBERS {
        rows.push(FieldRow::ToggleAddMember);
    }
    if model.members.len() > MIN_MEMBERS {
        rows.push(FieldRow::ToggleRemoveMember);
    }

    rows.push(FieldRow::Header("Service Load & Slip"));
    rows.push(FieldRow::ToggleExternalLoadEnabled);
    if model.external_load_enabled {
        rows.push(FieldRow::Number(NumberTarget::ExternalAxialLoad));
    }
    rows.push(FieldRow::ToggleSlipEnabled);
    if model.slip_enabled {
        rows.push(FieldRow::Number(NumberTarget::FrictionInterfaceMu));
        rows.push(FieldRow::Number(NumberTarget::AppliedShearLoad));
    }

    rows.push(FieldRow::Header("Material Strength Limits"));
    rows.push(FieldRow::ToggleStrengthLimitsEnabled);
    if model.strength_limits_enabled {
        rows.push(FieldRow::Number(NumberTarget::ProofLoad));
        rows.push(FieldRow::Number(NumberTarget::YieldStrength));
        rows.push(FieldRow::Number(NumberTarget::UltimateStrength));
        rows.push(FieldRow::Number(NumberTarget::ThreadEngagementLength));
        rows.push(FieldRow::Number(NumberTarget::ThreadShearStrength));
    }


    // Advanced: friction model detail, member stiffness model, catalog-filled thread data, member
    // outer geometry and the optional analyses. Collapsed by default; the solve uses them regardless.
    rows.push(FieldRow::AdvancedSection);
    if !model.advanced_open {
        return rows;
    }
    rows.push(FieldRow::Header("Friction & Stiffness"));
    rows.push(FieldRow::ToggleBearingModel);
    rows.push(FieldRow::Number(NumberTarget::PrevailingTorque));
    rows.push(FieldRow::ToggleMemberStiffness);
    rows.push(FieldRow::Number(NumberTarget::ConeHalfAngleDeg));
    rows.push(FieldRow::Header("Thread & Fastener Detail"));
    rows.push(FieldRow::Number(NumberTarget::ThreadPitchDia));
    rows.push(FieldRow::Number(NumberTarget::ThreadRootDia));
    rows.push(FieldRow::Number(NumberTarget::ThreadPitch));
    rows.push(FieldRow::Number(NumberTarget::ThreadStarts));
    rows.push(FieldRow::Number(NumberTarget::ThreadAngleDeg));
    rows.push(FieldRow::Number(NumberTarget::FastenerE));
    rows.push(FieldRow::Number(NumberTarget::FastenerNu));
    rows.push(FieldRow::Header("Member Outer Geometry"));
    for i in 0..model.members.len() {
        rows.push(FieldRow::Number(NumberTarget::MemberOuterDiameter(i)));
    }

    rows.push(FieldRow::Header("Uncertainty Analysis"));
    rows.push(FieldRow::ToggleUncertaintyEnabled);
    if model.uncertainty_enabled {
        rows.push(FieldRow::Number(NumberTarget::MuThreadTolPct));
        rows.push(FieldRow::Number(NumberTarget::MuBearingTolPct));
        rows.push(FieldRow::Number(NumberTarget::PitchDiameterTol));
        rows.push(FieldRow::Number(NumberTarget::BearingRadiusTol));
        rows.push(FieldRow::Number(NumberTarget::TorqueTolPct));
        rows.push(FieldRow::ToggleMonteCarloEnabled);
        if model.monte_carlo_enabled {
            rows.push(FieldRow::Number(NumberTarget::MonteCarloSamples));
            rows.push(FieldRow::Number(NumberTarget::MonteCarloSeed));
        }
    }

    rows.push(FieldRow::Header("Thread Load Distribution"));
    rows.push(FieldRow::ToggleThreadLoadDistributionEnabled);
    if model.thread_load_distribution_enabled {
        rows.push(FieldRow::Number(NumberTarget::EngagedThreads));
        rows.push(FieldRow::Number(NumberTarget::NutModulus));
        rows.push(FieldRow::Number(NumberTarget::NutPoissonRatio));
        rows.push(FieldRow::Number(NumberTarget::NutOuterDiameter));
    }
    rows
}

impl PreloadModel {
    fn thread_geometry(&self) -> ThreadGeometry {
        ThreadGeometry {
            d: self.thread_major_dia,
            d2: self.thread_pitch_dia,
            d3: self.thread_root_dia,
            pitch: self.thread_pitch,
            starts: self.thread_starts.round().max(1.0) as u32,
            thread_angle_deg: self.thread_angle_deg,
        }
    }

    fn member_stack(&self) -> MemberStack {
        MemberStack {
            members: self
                .members
                .iter()
                .map(|m| Member { thickness: m.thickness, e: m.e, hole_diameter: m.hole_diameter, outer_diameter: (m.outer_diameter > 0.0).then_some(m.outer_diameter) })
                .collect(),
        }
    }

    /// What the finite-element member-compliance cross-check needs, once the joint solves.
    pub fn fe_input(&self) -> Option<super::fe_check::FeInput> {
        let solution = self.output.as_ref().ok()?;
        let inputs = self.build_inputs();
        Some(super::fe_check::FeInput {
            members: self.member_stack().members,
            contact_diameter: 2.0 * inputs.geometry.bearing_outer_radius,
            cone_half_angle_deg: self.cone_half_angle_deg,
            fastener_compliance: solution.compliance.c_b,
        })
    }

    /// The half angle the solver integrates with: the FE-equivalent one in Finite Element mode once
    /// available for this joint, otherwise the user's.
    pub fn solver_cone_angle_deg(&self) -> f64 {
        match (self.member_stiffness_fe, self.fe_angle_deg) {
            (true, Some(a)) => a,
            _ => self.cone_half_angle_deg,
        }
    }

    /// Whether the displayed solution really uses the FE compliance.
    pub fn uses_fe_stiffness(&self) -> bool {
        self.member_stiffness_fe && self.fe_angle_deg.is_some()
    }

    pub fn toggle_member_stiffness(&mut self) {
        self.member_stiffness_fe = !self.member_stiffness_fe;
        self.recompute();
    }

    /// Sets the FE-equivalent angle; returns whether the solve changed.
    pub fn set_fe_angle(&mut self, angle: Option<f64>) -> bool {
        if self.fe_angle_deg == angle {
            return false;
        }
        self.fe_angle_deg = angle;
        if self.member_stiffness_fe {
            self.recompute();
        }
        true
    }

    fn build_inputs(&self) -> JointInputs {
        let mode = match self.mode {
            Mode::TorqueControlled => AnalysisMode::TorqueControlled { applied_torque: self.applied_torque },
            Mode::PreloadControlled => AnalysisMode::PreloadControlled { target_preload: self.target_preload },
            Mode::RotationControlled => AnalysisMode::RotationControlled { turns_after_snug: self.turns_after_snug, snug_preload: self.snug_preload },
        };
        JointInputs {
            geometry: JointGeometry {
                thread: self.thread_geometry(),
                bearing_inner_radius: self.bearing_inner_radius,
                bearing_outer_radius: self.bearing_outer_radius,
                head_bearing_inner_radius: (self.head_bearing_inner_radius > 0.0).then_some(self.head_bearing_inner_radius),
                head_bearing_outer_radius: (self.head_bearing_outer_radius > 0.0).then_some(self.head_bearing_outer_radius),
            },
            friction: FrictionInputs { mu_thread: self.mu_thread, mu_bearing: self.mu_bearing, bearing_model: self.bearing_model, prevailing_torque: self.prevailing_torque },
            tightening_from: self.tightening_from,
            fastener_segments: vec![FastenerSegment { length: self.shank_length, diameter: self.shank_diameter }],
            fastener_e: self.fastener_e,
            fastener_nu: self.fastener_nu,
            member_stack: self.member_stack(),
            cone_half_angle_deg: self.solver_cone_angle_deg(),
            mode,
            external_axial_load: self.external_load_enabled.then_some(self.external_axial_load),
            friction_interfaces: if self.slip_enabled { vec![self.friction_interface_mu] } else { Vec::new() },
            applied_shear_load: self.slip_enabled.then_some(self.applied_shear_load),
            proof_load: (self.strength_limits_enabled && self.proof_load > 0.0).then_some(self.proof_load),
            yield_strength: (self.strength_limits_enabled && self.yield_strength > 0.0).then_some(self.yield_strength),
            ultimate_strength: (self.strength_limits_enabled && self.ultimate_strength > 0.0).then_some(self.ultimate_strength),
            uncertainty: self.uncertainty_enabled.then_some(UncertaintyInputs {
                mu_thread_tol_fraction: self.mu_thread_tol_pct / 100.0,
                mu_bearing_tol_fraction: self.mu_bearing_tol_pct / 100.0,
                pitch_diameter_tol: self.pitch_diameter_tol,
                bearing_outer_radius_tol: self.bearing_radius_tol,
                applied_torque_tol_fraction: self.torque_tol_pct / 100.0,
            }),
            monte_carlo: (self.uncertainty_enabled && self.monte_carlo_enabled)
                .then_some(MonteCarloSettings { samples: self.monte_carlo_samples.max(1.0) as u32, seed: self.monte_carlo_seed.max(0.0) as u64 }),
            thread_load_distribution: self.thread_load_distribution_enabled.then_some(ThreadLoadDistributionInputs {
                engaged_threads: self.engaged_threads.max(1.0) as u32,
                nut_modulus: self.nut_modulus,
                nut_poisson_ratio: self.nut_poisson_ratio,
                nut_outer_diameter: self.nut_outer_diameter,
            }),
            embedment_settlement: (self.embedment_settlement > 0.0).then_some(self.embedment_settlement),
            thread_engagement_length: (self.thread_engagement_length > 0.0).then_some(self.thread_engagement_length),
            thread_shear_strength: (self.thread_shear_strength > 0.0).then_some(self.thread_shear_strength),
        }
    }

    pub fn recompute(&mut self) {
        self.output = compute(&self.build_inputs());
    }

    pub fn number_value(&self, target: NumberTarget) -> f64 {
        match target {
            NumberTarget::AppliedTorque => self.applied_torque,
            NumberTarget::TargetPreload => self.target_preload,
            NumberTarget::TurnsAfterSnug => self.turns_after_snug,
            NumberTarget::SnugPreload => self.snug_preload,
            NumberTarget::MuThread => self.mu_thread,
            NumberTarget::MuBearing => self.mu_bearing,
            NumberTarget::PrevailingTorque => self.prevailing_torque,
            NumberTarget::BearingInnerRadius => self.bearing_inner_radius,
            NumberTarget::BearingOuterRadius => self.bearing_outer_radius,
            NumberTarget::HeadBearingInnerRadius => self.head_bearing_inner_radius,
            NumberTarget::HeadBearingOuterRadius => self.head_bearing_outer_radius,
            NumberTarget::ConeHalfAngleDeg => self.cone_half_angle_deg,
            NumberTarget::ThreadMajorDia => self.thread_major_dia,
            NumberTarget::ThreadPitchDia => self.thread_pitch_dia,
            NumberTarget::ThreadRootDia => self.thread_root_dia,
            NumberTarget::ThreadPitch => self.thread_pitch,
            NumberTarget::ThreadStarts => self.thread_starts,
            NumberTarget::ThreadAngleDeg => self.thread_angle_deg,
            NumberTarget::FastenerE => self.fastener_e,
            NumberTarget::FastenerNu => self.fastener_nu,
            NumberTarget::ShankLength => self.shank_length,
            NumberTarget::ShankDiameter => self.shank_diameter,
            NumberTarget::ExternalAxialLoad => self.external_axial_load,
            NumberTarget::AppliedShearLoad => self.applied_shear_load,
            NumberTarget::FrictionInterfaceMu => self.friction_interface_mu,
            NumberTarget::ProofLoad => self.proof_load,
            NumberTarget::YieldStrength => self.yield_strength,
            NumberTarget::UltimateStrength => self.ultimate_strength,
            NumberTarget::MemberThickness(i) => self.members.get(i).map(|m| m.thickness).unwrap_or(0.0),
            NumberTarget::MemberModulus(i) => self.members.get(i).map(|m| m.e).unwrap_or(0.0),
            NumberTarget::MemberHoleDiameter(i) => self.members.get(i).map(|m| m.hole_diameter).unwrap_or(0.0),
            NumberTarget::MemberOuterDiameter(i) => self.members.get(i).map(|m| m.outer_diameter).unwrap_or(0.0),
            NumberTarget::MuThreadTolPct => self.mu_thread_tol_pct,
            NumberTarget::MuBearingTolPct => self.mu_bearing_tol_pct,
            NumberTarget::PitchDiameterTol => self.pitch_diameter_tol,
            NumberTarget::BearingRadiusTol => self.bearing_radius_tol,
            NumberTarget::TorqueTolPct => self.torque_tol_pct,
            NumberTarget::MonteCarloSamples => self.monte_carlo_samples,
            NumberTarget::MonteCarloSeed => self.monte_carlo_seed,
            NumberTarget::EngagedThreads => self.engaged_threads,
            NumberTarget::NutModulus => self.nut_modulus,
            NumberTarget::NutPoissonRatio => self.nut_poisson_ratio,
            NumberTarget::NutOuterDiameter => self.nut_outer_diameter,
            NumberTarget::EmbedmentSettlement => self.embedment_settlement,
            NumberTarget::ThreadEngagementLength => self.thread_engagement_length,
            NumberTarget::ThreadShearStrength => self.thread_shear_strength,
        }
    }

    /// Commits one edited numeric field. A non-finite result is silently
    /// ignored - same convention every other toolbox in this crate uses.
    pub fn commit_number(&mut self, target: NumberTarget, raw: f64) {
        if !raw.is_finite() {
            return;
        }
        match target {
            NumberTarget::AppliedTorque => self.applied_torque = raw,
            NumberTarget::TargetPreload => self.target_preload = raw,
            NumberTarget::TurnsAfterSnug => self.turns_after_snug = raw,
            NumberTarget::SnugPreload => self.snug_preload = raw,
            NumberTarget::MuThread => self.mu_thread = raw,
            NumberTarget::MuBearing => self.mu_bearing = raw,
            NumberTarget::PrevailingTorque => self.prevailing_torque = raw,
            NumberTarget::BearingInnerRadius => self.bearing_inner_radius = raw,
            NumberTarget::BearingOuterRadius => self.bearing_outer_radius = raw,
            NumberTarget::HeadBearingInnerRadius => self.head_bearing_inner_radius = raw.max(0.0),
            NumberTarget::HeadBearingOuterRadius => self.head_bearing_outer_radius = raw.max(0.0),
            NumberTarget::ConeHalfAngleDeg => self.cone_half_angle_deg = raw,
            NumberTarget::ThreadMajorDia => self.thread_major_dia = raw,
            NumberTarget::ThreadPitchDia => self.thread_pitch_dia = raw,
            NumberTarget::ThreadRootDia => self.thread_root_dia = raw,
            NumberTarget::ThreadPitch => self.thread_pitch = raw,
            NumberTarget::ThreadStarts => self.thread_starts = raw.max(1.0),
            NumberTarget::ThreadAngleDeg => self.thread_angle_deg = raw,
            NumberTarget::FastenerE => self.fastener_e = raw,
            NumberTarget::FastenerNu => self.fastener_nu = raw,
            NumberTarget::ShankLength => self.shank_length = raw,
            NumberTarget::ShankDiameter => self.shank_diameter = raw,
            NumberTarget::ExternalAxialLoad => self.external_axial_load = raw,
            NumberTarget::AppliedShearLoad => self.applied_shear_load = raw,
            NumberTarget::FrictionInterfaceMu => self.friction_interface_mu = raw,
            NumberTarget::ProofLoad => self.proof_load = raw,
            NumberTarget::YieldStrength => self.yield_strength = raw,
            NumberTarget::UltimateStrength => self.ultimate_strength = raw,
            NumberTarget::MemberThickness(i) => {
                if let Some(m) = self.members.get_mut(i) {
                    m.thickness = raw;
                }
            }
            NumberTarget::MemberModulus(i) => {
                if let Some(m) = self.members.get_mut(i) {
                    m.e = raw;
                }
            }
            NumberTarget::MemberHoleDiameter(i) => {
                if let Some(m) = self.members.get_mut(i) {
                    m.hole_diameter = raw;
                }
            }
            NumberTarget::MemberOuterDiameter(i) => {
                if let Some(m) = self.members.get_mut(i) {
                    m.outer_diameter = raw;
                }
            }
            NumberTarget::MuThreadTolPct => self.mu_thread_tol_pct = raw,
            NumberTarget::MuBearingTolPct => self.mu_bearing_tol_pct = raw,
            NumberTarget::PitchDiameterTol => self.pitch_diameter_tol = raw,
            NumberTarget::BearingRadiusTol => self.bearing_radius_tol = raw,
            NumberTarget::TorqueTolPct => self.torque_tol_pct = raw,
            NumberTarget::MonteCarloSamples => self.monte_carlo_samples = raw.max(1.0),
            NumberTarget::MonteCarloSeed => self.monte_carlo_seed = raw.max(0.0),
            NumberTarget::EngagedThreads => self.engaged_threads = raw.max(1.0),
            NumberTarget::NutModulus => self.nut_modulus = raw,
            NumberTarget::NutPoissonRatio => self.nut_poisson_ratio = raw,
            NumberTarget::NutOuterDiameter => self.nut_outer_diameter = raw,
            NumberTarget::EmbedmentSettlement => self.embedment_settlement = raw.max(0.0),
            NumberTarget::ThreadEngagementLength => self.thread_engagement_length = raw.max(0.0),
            NumberTarget::ThreadShearStrength => self.thread_shear_strength = raw.max(0.0),
        }
        self.recompute();
    }

    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.cycle();
        self.recompute();
    }

    pub fn toggle_tightening_from(&mut self) {
        self.tightening_from = cycle_tightening_from(self.tightening_from);
        self.recompute();
    }

    pub fn toggle_bearing_model(&mut self) {
        self.bearing_model = cycle_bearing_model(self.bearing_model);
        self.recompute();
    }

    pub fn toggle_external_load_enabled(&mut self) {
        self.external_load_enabled = !self.external_load_enabled;
        self.recompute();
    }

    pub fn toggle_slip_enabled(&mut self) {
        self.slip_enabled = !self.slip_enabled;
        self.recompute();
    }

    pub fn toggle_strength_limits_enabled(&mut self) {
        self.strength_limits_enabled = !self.strength_limits_enabled;
        self.recompute();
    }

    pub fn toggle_uncertainty_enabled(&mut self) {
        self.uncertainty_enabled = !self.uncertainty_enabled;
        self.recompute();
    }

    pub fn toggle_monte_carlo_enabled(&mut self) {
        self.monte_carlo_enabled = !self.monte_carlo_enabled;
        self.recompute();
    }

    pub fn toggle_thread_load_distribution_enabled(&mut self) {
        self.thread_load_distribution_enabled = !self.thread_load_distribution_enabled;
        self.recompute();
    }

    /// The standard AN bolt this model's current thread geometry/shank
    /// diameter matches exactly, if any - `None` means "Custom" (either
    /// never picked from the catalog, or hand-edited since).
    pub fn matching_bolt(&self) -> Option<&'static BoltCatalogEntry> {
        find_matching(&self.thread_geometry()).filter(|e| (e.major_diameter - self.shank_diameter).abs() < 1e-6)
    }

    /// Populates every thread-geometry field (and the shank diameter) from
    /// a standard AN catalog entry - the fields remain ordinary editable
    /// `Number` rows afterward, so any of them can still be hand-adjusted
    /// (which simply makes `matching_bolt` report `None`/"Custom" again).
    pub fn select_bolt(&mut self, entry: &BoltCatalogEntry) {
        let g = entry.thread_geometry();
        self.thread_major_dia = g.d;
        self.thread_pitch_dia = g.d2;
        self.thread_root_dia = g.d3;
        self.thread_pitch = g.pitch;
        self.thread_starts = g.starts as f64;
        self.thread_angle_deg = g.thread_angle_deg;
        self.shank_diameter = g.d;
        self.recompute();
    }

    pub fn add_member(&mut self) {
        if self.members.len() < MAX_MEMBERS {
            self.members.push(MemberUi::default());
            self.recompute();
        }
    }

    pub fn remove_member(&mut self) {
        if self.members.len() > MIN_MEMBERS {
            self.members.pop();
            self.recompute();
        }
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
    fn default_model_solves_successfully() {
        let model = PreloadModel::default();
        assert!(model.output.is_ok(), "{:?}", model.output);
    }

    #[test]
    fn default_model_produces_positive_preload() {
        let model = PreloadModel::default();
        let solution = model.output.as_ref().unwrap();
        assert!(solution.preload > 0.0);
    }

    #[test]
    fn commit_number_updates_field_and_recomputes() {
        let mut model = PreloadModel::default();
        model.commit_number(NumberTarget::AppliedTorque, 400.0);
        assert_eq!(model.applied_torque, 400.0);
        let before = PreloadModel::default().output.unwrap().preload;
        let after = model.output.unwrap().preload;
        assert!(after > before, "higher applied torque must yield higher preload");
    }

    #[test]
    fn commit_number_silently_ignores_a_non_finite_result() {
        let mut model = PreloadModel::default();
        let before = model.applied_torque;
        model.commit_number(NumberTarget::AppliedTorque, f64::NAN);
        assert_eq!(model.applied_torque, before);
    }

    #[test]
    fn toggle_mode_cycles_through_all_three_variants() {
        let mut model = PreloadModel::default();
        assert_eq!(model.mode, Mode::TorqueControlled);
        model.toggle_mode();
        assert_eq!(model.mode, Mode::PreloadControlled);
        model.toggle_mode();
        assert_eq!(model.mode, Mode::RotationControlled);
        model.toggle_mode();
        assert_eq!(model.mode, Mode::TorqueControlled);
    }

    #[test]
    fn field_rows_shows_the_right_primary_input_for_each_mode() {
        let mut model = PreloadModel::default();
        assert!(field_rows(&model).contains(&FieldRow::Number(NumberTarget::AppliedTorque)));
        model.mode = Mode::PreloadControlled;
        assert!(field_rows(&model).contains(&FieldRow::Number(NumberTarget::TargetPreload)));
        assert!(!field_rows(&model).contains(&FieldRow::Number(NumberTarget::AppliedTorque)));
        model.mode = Mode::RotationControlled;
        assert!(field_rows(&model).contains(&FieldRow::Number(NumberTarget::TurnsAfterSnug)));
    }

    #[test]
    fn add_and_remove_member_change_the_member_count_and_field_rows() {
        let mut model = PreloadModel::default();
        assert_eq!(model.members.len(), 1);
        model.add_member();
        assert_eq!(model.members.len(), 2);
        assert!(field_rows(&model).contains(&FieldRow::Number(NumberTarget::MemberThickness(1))));
        model.remove_member();
        assert_eq!(model.members.len(), 1);
        assert!(!field_rows(&model).contains(&FieldRow::Number(NumberTarget::MemberThickness(1))));
    }

    #[test]
    fn add_member_is_capped_at_max_members() {
        let mut model = PreloadModel::default();
        for _ in 0..10 {
            model.add_member();
        }
        assert_eq!(model.members.len(), MAX_MEMBERS);
    }

    #[test]
    fn remove_member_never_goes_below_min_members() {
        let mut model = PreloadModel::default();
        for _ in 0..10 {
            model.remove_member();
        }
        assert_eq!(model.members.len(), MIN_MEMBERS);
    }

    #[test]
    fn external_load_fields_are_hidden_until_enabled() {
        let mut model = PreloadModel::default();
        assert!(!field_rows(&model).contains(&FieldRow::Number(NumberTarget::ExternalAxialLoad)));
        model.toggle_external_load_enabled();
        assert!(field_rows(&model).contains(&FieldRow::Number(NumberTarget::ExternalAxialLoad)));
    }

    #[test]
    fn enabling_external_load_populates_the_service_load_result() {
        let mut model = PreloadModel::default();
        model.toggle_external_load_enabled();
        assert!(model.output.as_ref().unwrap().service_load.is_some());
    }

    #[test]
    fn enabling_slip_check_populates_slip_capacity_and_margin() {
        let mut model = PreloadModel::default();
        model.toggle_slip_enabled();
        let solution = model.output.as_ref().unwrap();
        assert!(solution.slip_capacity.is_some());
        assert!(solution.slip_margin.is_some());
    }

    #[test]
    fn format_for_edit_round_trips_through_parse() {
        let s = format_for_edit(0.25);
        assert_eq!(s.parse::<f64>().unwrap(), 0.25);
        assert_eq!(format_for_edit(0.0), "0");
    }
}
