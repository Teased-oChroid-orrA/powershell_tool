//! The Lug Analysis case file: every input of `LugUiModel` as JSON. Numbers are saved by name (so a file outlives a
//! reordering of the rows), the lug and bushing materials by their catalog name (an index would shift with the
//! catalog), the mesh density, solver, flow rule and pin body by their labels, the on/off switches by name, and the
//! three overrides (elements around the bore, elongation, failure strain) only when set. Results are never stored.
use crate::toolboxes::lug_analysis::model::{FlowRule, LugUiModel, MeshDensity, NumberTarget, PinBodyChoice, SolverChoice};
use crate::toolboxes::material_lookup::model::catalog;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FORMAT_VERSION: u32 = 2;

/// Every number input that is not an override, with its key in the file.
pub const NUMBERS: [(NumberTarget, &str); 17] = [
    (NumberTarget::HoleDia, "hole diameter"),
    (NumberTarget::Width, "width"),
    (NumberTarget::Edge, "edge distance"),
    (NumberTarget::HeadCorner, "head corner radius"),
    (NumberTarget::Thickness, "thickness"),
    (NumberTarget::ModelLength, "model length"),
    (NumberTarget::PinDia, "pin diameter"),
    (NumberTarget::Friction, "pin friction"),
    (NumberTarget::Load, "pin load"),
    (NumberTarget::LoadAngle, "load angle"),
    (NumberTarget::BushingId, "bushing inner diameter"),
    (NumberTarget::BushingInterference, "bushing interference"),
    (NumberTarget::BushingFriction, "bushing friction"),
    (NumberTarget::TemperatureChange, "temperature change"),
    (NumberTarget::ClevisOffset, "clevis load offset"),
    (NumberTarget::MaxGrowth, "max growth ratio"),
    (NumberTarget::FirstLayerAspect, "first layer aspect"),
];
/// A new number input must be added to `NUMBERS` (or, an override, to `LugFile`) before this compiles again.
#[allow(dead_code)]
fn every_number_target_is_listed(t: NumberTarget) {
    use NumberTarget::*;
    match t {
        HoleDia | Width | Edge | HeadCorner | Thickness | ModelLength | PinDia | Friction | Load | LoadAngle | BushingId | BushingInterference | BushingFriction | TemperatureChange | ClevisOffset | MaxGrowth | FirstLayerAspect => {}
        Ductility | Elongation | ElementsAround | PinFty | PinFsu => {}
    }
}

/// The stored value of a number: the edge and corner as typed (the round head ignores them, `number_value` reports
/// the effective ones).
fn raw(m: &LugUiModel, t: NumberTarget) -> f64 {
    match t {
        NumberTarget::Edge => m.edge,
        NumberTarget::HeadCorner => m.head_corner,
        t => m.number_value(t),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LugFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub numbers: BTreeMap<String, f64>,
    /// Catalog names (`Material::name`).
    #[serde(default)]
    pub material: String,
    #[serde(default)]
    pub bushing_material: String,
    /// `MeshDensity::label`, `SolverChoice::label`, `FlowRule::label`, `PinBodyChoice::label`.
    #[serde(default)]
    pub density: String,
    #[serde(default)]
    pub solver: String,
    #[serde(default)]
    pub flow_rule: String,
    #[serde(default)]
    pub pin_body: String,
    /// The on/off switches by name.
    #[serde(default)]
    pub switches: BTreeMap<String, bool>,
    /// Overrides of the density preset and the material library; absent follows them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elements_around: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elongation: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_strain: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_strength_material: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_fty_override: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_fsu_override: Option<f64>,
}

/// Every switch with its key in the file.
fn switches(m: &mut LugUiModel) -> [(&'static str, &mut bool); 8] {
    [
        ("round head", &mut m.head_round),
        ("bushing", &mut m.bushing),
        ("plastic limit load", &mut m.plastic),
        ("finite strain", &mut m.finite_strain),
        ("geometric nonlinearity", &mut m.second_order),
        ("pin bending", &mut m.pin_bending),
        ("contact refinement", &mut m.auto_refine),
        ("advanced open", &mut m.mesh_open),
    ]
}

/// The value among `start`, `next(start)`, ... whose label is `wanted`.
fn by_label<T: Copy + PartialEq>(start: T, next: impl Fn(T) -> T, label: impl Fn(T) -> &'static str, wanted: &str) -> Option<T> {
    let mut v = start;
    loop {
        if label(v) == wanted {
            return Some(v);
        }
        v = next(v);
        if v == start {
            return None;
        }
    }
}

fn material_index(name: &str) -> Option<usize> {
    catalog().iter().position(|m| m.name == name)
}

impl LugFile {
    pub fn capture(m: &LugUiModel, name: &str) -> Self {
        let mut copy = m.clone();
        Self {
            version: FORMAT_VERSION,
            name: name.to_string(),
            numbers: NUMBERS.iter().map(|(t, key)| (key.to_string(), raw(m, *t))).collect(),
            material: m.material().name.to_string(),
            bushing_material: m.bushing_material().name.to_string(),
            density: m.density.label().to_string(),
            solver: m.solver.label().to_string(),
            flow_rule: m.flow_rule.label().to_string(),
            pin_body: m.pin_body.label().to_string(),
            switches: switches(&mut copy).into_iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            elements_around: m.elements_around,
            elongation: m.elongation,
            failure_strain: m.failure_strain,
            pin_strength_material: m.pin_strength_material.clone(),
            pin_fty_override: m.pin_fty_override,
            pin_fsu_override: m.pin_fsu_override,
        }
    }

    /// Put the file's inputs into `m`. Returns what could not be applied (an unknown name or label, a number the
    /// model refuses); everything else is applied.
    pub fn apply(&self, m: &mut LugUiModel) -> Vec<String> {
        let mut skipped = Vec::new();
        if self.version < 2 {
            m.pin_strength_material = None;
            m.pin_fty_override = None;
            m.pin_fsu_override = None;
        }
        for (t, key) in NUMBERS {
            if let Some(v) = self.numbers.get(key) {
                m.commit_number(t, *v);
                if raw(m, t) != *v {
                    skipped.push(format!("{key}: {v} is out of range"));
                }
            }
        }
        for (name, slot, what) in [(&self.material, &mut m.material_index, "material"), (&self.bushing_material, &mut m.bushing_material_index, "bushing material")] {
            if name.is_empty() {
                continue;
            }
            match material_index(name) {
                Some(i) => *slot = i,
                None => skipped.push(format!("{what}: {name} is not in the catalog")),
            }
        }
        if self.version >= 2 {
            m.pin_strength_material = self.pin_strength_material.as_deref().filter(|name| material_index(name).is_some()).map(str::to_owned);
            m.pin_fty_override = self.pin_fty_override.filter(|v| v.is_finite() && *v > 0.0);
            m.pin_fsu_override = self.pin_fsu_override.filter(|v| v.is_finite() && *v > 0.0);
            if let Some(name) = self.pin_strength_material.as_deref().filter(|name| material_index(name).is_none()) {
                m.pin_fty_override = None;
                m.pin_fsu_override = None;
                skipped.push(format!("pin strength material: {name} is not in the catalog"));
            }
        }
        let mut label = |text: &str, what: &str, found: Option<()>| {
            if !text.is_empty() && found.is_none() {
                skipped.push(format!("{what}: {text}"));
            }
        };
        let density = by_label(m.density, MeshDensity::next, MeshDensity::label, &self.density);
        label(&self.density, "mesh density", density.map(|v| m.density = v));
        let solver = by_label(m.solver, SolverChoice::next, SolverChoice::label, &self.solver);
        label(&self.solver, "solver", solver.map(|v| m.solver = v));
        let rule = by_label(m.flow_rule, FlowRule::next, FlowRule::label, &self.flow_rule);
        label(&self.flow_rule, "flow stress rule", rule.map(|v| m.flow_rule = v));
        let body = by_label(m.pin_body, PinBodyChoice::next, PinBodyChoice::label, &self.pin_body);
        label(&self.pin_body, "pin body", body.map(|v| m.pin_body = v));
        for (key, flag) in switches(m) {
            if let Some(v) = self.switches.get(key) {
                *flag = *v;
            }
        }
        // The overrides: absent follows the preset or the library; a value the model refuses is reported.
        m.elements_around = None;
        m.elongation = None;
        m.failure_strain = None;
        for (t, v, key) in [(NumberTarget::ElementsAround, self.elements_around.map(|n| n as f64), "elements around"), (NumberTarget::Elongation, self.elongation, "elongation"), (NumberTarget::Ductility, self.failure_strain, "failure strain")] {
            if let Some(v) = v {
                m.commit_number(t, v);
                if m.number_value(t) != v {
                    skipped.push(format!("{key}: {v} is out of range"));
                }
            }
        }
        skipped
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plain data serializes")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let f: Self = serde_json::from_str(text).map_err(|e| format!("not a lug case: {e}"))?;
        if f.version > FORMAT_VERSION {
            return Err(format!("written by a newer version (format {}, this build reads {FORMAT_VERSION})", f.version));
        }
        Ok(f)
    }
}
