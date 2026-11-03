//! The Preload Analysis case file: every input of `PreloadModel` as JSON. Numbers are saved by name (so a file outlives
//! a reordering of the rows), the member stack as a list, the analysis mode, tightening member and bearing model by
//! their labels, and the on/off switches. Results are never stored: they are recomputed.
use crate::toolboxes::preload_analysis::joint_templates::LayerRole;
use crate::toolboxes::preload_analysis::model::{cycle_bearing_model, cycle_tightening_from, label_bearing_model, label_tightening_from, MemberUi, Mode, NumberTarget, PreloadModel, MAX_MEMBERS};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FORMAT_VERSION: u32 = 1;

/// Every number input that is not a member's, with its key in the file.
pub const NUMBERS: [(NumberTarget, &str); 42] = [
    (NumberTarget::AppliedTorque, "applied torque"),
    (NumberTarget::TargetPreload, "target preload"),
    (NumberTarget::TurnsAfterSnug, "turns after snug"),
    (NumberTarget::SnugPreload, "snug preload"),
    (NumberTarget::MuThread, "thread friction"),
    (NumberTarget::MuBearing, "bearing friction"),
    (NumberTarget::PrevailingTorque, "prevailing torque"),
    (NumberTarget::BearingInnerRadius, "bearing inner radius"),
    (NumberTarget::BearingOuterRadius, "bearing outer radius"),
    (NumberTarget::HeadBearingInnerRadius, "head bearing inner radius"),
    (NumberTarget::HeadBearingOuterRadius, "head bearing outer radius"),
    (NumberTarget::ConeHalfAngleDeg, "cone half angle"),
    (NumberTarget::ThreadMajorDia, "thread major diameter"),
    (NumberTarget::ThreadPitchDia, "thread pitch diameter"),
    (NumberTarget::ThreadRootDia, "thread root diameter"),
    (NumberTarget::ThreadPitch, "thread pitch"),
    (NumberTarget::ThreadStarts, "thread starts"),
    (NumberTarget::ThreadAngleDeg, "thread angle"),
    (NumberTarget::FastenerE, "fastener modulus"),
    (NumberTarget::FastenerNu, "fastener poisson ratio"),
    (NumberTarget::ShankLength, "shank length"),
    (NumberTarget::ShankDiameter, "shank diameter"),
    (NumberTarget::ExternalAxialLoad, "external axial load"),
    (NumberTarget::AppliedShearLoad, "applied shear load"),
    (NumberTarget::FrictionInterfaceMu, "interface friction"),
    (NumberTarget::ProofLoad, "proof load"),
    (NumberTarget::YieldStrength, "yield strength"),
    (NumberTarget::UltimateStrength, "ultimate strength"),
    (NumberTarget::MuThreadTolPct, "thread friction tolerance percent"),
    (NumberTarget::MuBearingTolPct, "bearing friction tolerance percent"),
    (NumberTarget::PitchDiameterTol, "pitch diameter tolerance"),
    (NumberTarget::BearingRadiusTol, "bearing radius tolerance"),
    (NumberTarget::TorqueTolPct, "torque tolerance percent"),
    (NumberTarget::MonteCarloSamples, "monte carlo samples"),
    (NumberTarget::MonteCarloSeed, "monte carlo seed"),
    (NumberTarget::EngagedThreads, "engaged threads"),
    (NumberTarget::NutModulus, "nut modulus"),
    (NumberTarget::NutPoissonRatio, "nut poisson ratio"),
    (NumberTarget::NutOuterDiameter, "nut outer diameter"),
    (NumberTarget::EmbedmentSettlement, "embedment settlement"),
    (NumberTarget::ThreadEngagementLength, "thread engagement length"),
    (NumberTarget::ThreadShearStrength, "thread shear strength"),
];
/// A new number input must be added to `NUMBERS` (or, a member's, to `FileMember`) before this compiles again.
#[allow(dead_code)]
fn every_number_target_is_listed(t: NumberTarget) {
    use NumberTarget::*;
    match t {
        AppliedTorque | TargetPreload | TurnsAfterSnug | SnugPreload | MuThread | MuBearing | PrevailingTorque | BearingInnerRadius | BearingOuterRadius | HeadBearingInnerRadius | HeadBearingOuterRadius | ConeHalfAngleDeg | ThreadMajorDia | ThreadPitchDia
        | ThreadRootDia | ThreadPitch | ThreadStarts | ThreadAngleDeg | FastenerE | FastenerNu | ShankLength | ShankDiameter | ExternalAxialLoad | AppliedShearLoad | FrictionInterfaceMu | ProofLoad | YieldStrength | UltimateStrength | MuThreadTolPct
        | MuBearingTolPct | PitchDiameterTol | BearingRadiusTol | TorqueTolPct | MonteCarloSamples | MonteCarloSeed | EngagedThreads | NutModulus | NutPoissonRatio | NutOuterDiameter | EmbedmentSettlement | ThreadEngagementLength
        | ThreadShearStrength => {}
        MemberThickness(_) | MemberModulus(_) | MemberPoissonRatio(_) | MemberHoleDiameter(_) | MemberOuterDiameter(_) => {}
    }
}

/// One clamped member, head side first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileMember {
    /// The role shown in the stack (`Plate`, `Washer`, ...).
    pub role: String,
    pub thickness: f64,
    pub e: f64,
    #[serde(default = "default_member_nu")]
    pub nu: f64,
    pub hole_diameter: f64,
    /// `0`: unbounded.
    #[serde(default)]
    pub outer_diameter: f64,
}

fn default_member_nu() -> f64 { 0.3 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreloadFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub numbers: BTreeMap<String, f64>,
    #[serde(default)]
    pub members: Vec<FileMember>,
    /// `Mode::label`, `label_tightening_from`, `label_bearing_model`.
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub tightening_from: String,
    #[serde(default)]
    pub bearing_model: String,
    /// The on/off switches by name.
    #[serde(default)]
    pub switches: BTreeMap<String, bool>,
}

/// Every switch with its key in the file.
fn switches(m: &mut PreloadModel) -> [(&'static str, &mut bool); 8] {
    [
        ("finite element member stiffness", &mut m.member_stiffness_fe),
        ("advanced open", &mut m.advanced_open),
        ("external load", &mut m.external_load_enabled),
        ("slip check", &mut m.slip_enabled),
        ("strength limits", &mut m.strength_limits_enabled),
        ("uncertainty", &mut m.uncertainty_enabled),
        ("monte carlo", &mut m.monte_carlo_enabled),
        ("thread load distribution", &mut m.thread_load_distribution_enabled),
    ]
}

/// The value among `start`, `cycle(start)`, ... whose label is `label`.
fn by_label<T: Copy + PartialEq>(start: T, cycle: impl Fn(T) -> T, label: impl Fn(T) -> &'static str, wanted: &str) -> Option<T> {
    let mut v = start;
    loop {
        if label(v) == wanted {
            return Some(v);
        }
        v = cycle(v);
        if v == start {
            return None;
        }
    }
}

impl PreloadFile {
    pub fn capture(m: &PreloadModel, name: &str) -> Self {
        let mut copy = m.clone();
        Self {
            version: FORMAT_VERSION,
            name: name.to_string(),
            numbers: NUMBERS.iter().map(|(t, key)| (key.to_string(), m.number_value(*t))).collect(),
            members: m.members.iter().map(|u| FileMember { role: u.name.to_string(), thickness: u.thickness, e: u.e, nu: u.nu, hole_diameter: u.hole_diameter, outer_diameter: u.outer_diameter }).collect(),
            mode: m.mode.label().to_string(),
            tightening_from: label_tightening_from(m.tightening_from).to_string(),
            bearing_model: label_bearing_model(m.bearing_model).to_string(),
            switches: switches(&mut copy).into_iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    /// Put the file's inputs into `m` and solve once. Returns what could not be applied (an unknown label, a member
    /// stack that is empty or longer than `MAX_MEMBERS`); everything else is applied.
    pub fn apply(&self, m: &mut PreloadModel) -> Vec<String> {
        let mut skipped = Vec::new();
        for (t, key) in NUMBERS {
            if let Some(v) = self.numbers.get(key) {
                if !m.set_number(t, *v) {
                    skipped.push(format!("{key}: not a number"));
                }
            }
        }
        match self.members.len() {
            0 => {}
            n if n > MAX_MEMBERS => skipped.push(format!("members: {n} (at most {MAX_MEMBERS})")),
            _ => {
                m.members = self
                    .members
                    .iter()
                    .enumerate()
                    .map(|(i, f)| MemberUi {
                        // An unknown role is a plate: the role only names the layer in the stack picture.
                        name: LayerRole::from_name(&f.role).unwrap_or(LayerRole::Plate).name(),
                        thickness: f.thickness,
                        e: f.e,
                        nu: if f.nu.is_finite() && f.nu > -1.0 && f.nu < 0.5 { f.nu } else {
                            skipped.push(format!("member {} Poisson ratio: must be greater than -1 and less than 0.5", i + 1));
                            default_member_nu()
                        },
                        hole_diameter: f.hole_diameter,
                        outer_diameter: f.outer_diameter,
                    })
                    .collect()
            }
        }
        if !self.mode.is_empty() {
            match by_label(Mode::default(), Mode::cycle, Mode::label, &self.mode) {
                Some(v) => m.mode = v,
                None => skipped.push(format!("mode: {}", self.mode)),
            }
        }
        if !self.tightening_from.is_empty() {
            match by_label(m.tightening_from, cycle_tightening_from, label_tightening_from, &self.tightening_from) {
                Some(v) => m.tightening_from = v,
                None => skipped.push(format!("tightening from: {}", self.tightening_from)),
            }
        }
        if !self.bearing_model.is_empty() {
            match by_label(m.bearing_model, cycle_bearing_model, label_bearing_model, &self.bearing_model) {
                Some(v) => m.bearing_model = v,
                None => skipped.push(format!("bearing model: {}", self.bearing_model)),
            }
        }
        for (key, flag) in switches(m) {
            if let Some(v) = self.switches.get(key) {
                *flag = *v;
            }
        }
        // The FE-equivalent angle belongs to the joint it was computed for, never to a file.
        m.fe_angle_deg = None;
        m.recompute();
        skipped
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plain data serializes")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let f: Self = serde_json::from_str(text).map_err(|e| format!("not a preload case: {e}"))?;
        if f.version > FORMAT_VERSION {
            return Err(format!("written by a newer version (format {}, this build reads {FORMAT_VERSION})", f.version));
        }
        Ok(f)
    }
}
