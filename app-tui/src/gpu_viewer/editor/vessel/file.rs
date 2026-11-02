//! The Pressure Vessel case file: every input of `PressureVesselModel` (the numbers by name, the end condition, the
//! material by name) as JSON, so a file outlives a reordering of the rows. A custom material travels with the case
//! (`custom_material`) and is added to the catalog when a machine opening it does not have it. Results are never
//! stored: they are recomputed.
use crate::toolboxes::pressure_vessel::{
    model::{is_custom, NumberTarget, PressureVesselModel},
    persistence::PersistedMaterial,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FORMAT_VERSION: u32 = 1;

/// Every number input with its key in the file.
pub const NUMBERS: [(NumberTarget, &str); 7] = [
    (NumberTarget::OuterDiameter, "outer diameter"),
    (NumberTarget::WallThickness, "wall thickness"),
    (NumberTarget::InternalPressure, "internal pressure"),
    (NumberTarget::ExternalPressure, "external pressure"),
    (NumberTarget::RequiredMs, "required minimum MS"),
    (NumberTarget::UnsupportedLength, "unsupported length"),
    (NumberTarget::TemperatureDifferential, "temperature differential"),
];
#[allow(dead_code)]
fn every_number_target_is_listed(t: NumberTarget) {
    use NumberTarget::*;
    match t {
        OuterDiameter | WallThickness | InternalPressure | ExternalPressure | RequiredMs | UnsupportedLength | TemperatureDifferential => {}
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VesselFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub numbers: BTreeMap<String, f64>,
    #[serde(default)]
    pub closed_ends: Option<bool>,
    #[serde(default)]
    pub material: String,
    /// The material's properties when it is a user-added one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_material: Option<PersistedMaterial>,
}

impl VesselFile {
    pub fn capture(m: &PressureVesselModel, name: &str) -> Self {
        let material = m.material();
        Self {
            version: FORMAT_VERSION,
            name: name.to_string(),
            numbers: NUMBERS.iter().map(|(t, key)| (key.to_string(), m.number_value(*t))).collect(),
            closed_ends: Some(m.closed_ends),
            material: material.name.to_string(),
            custom_material: is_custom(material).then(|| material.into()),
        }
    }

    /// Put the file's inputs into `m` and recompute. Returns what could not be applied (a material this catalog does not
    /// have and the file does not describe); everything else is applied.
    pub fn apply(&self, m: &mut PressureVesselModel) -> Vec<String> {
        let mut skipped = Vec::new();
        for (t, key) in NUMBERS {
            if let Some(v) = self.numbers.get(key) {
                m.commit_number(t, *v);
            }
        }
        if let Some(closed) = self.closed_ends {
            m.closed_ends = closed;
        }
        if !self.material.is_empty() {
            match m.material_catalog().iter().position(|c| c.name == self.material) {
                Some(i) => m.material_index = i,
                None => match self.custom_material.as_ref().filter(|c| c.name == self.material) {
                    Some(c) => m.add_custom_material(c.name.clone(), c.e_ksi, c.sy_ksi, c.ftu_ksi, c.nu, c.alpha_u_f),
                    None => skipped.push(format!("material: {}", self.material)),
                },
            }
        }
        m.recompute();
        skipped
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plain data serializes")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let f: Self = serde_json::from_str(text).map_err(|e| format!("not a pressure vessel case: {e}"))?;
        if f.version > FORMAT_VERSION {
            return Err(format!("written by a newer version (format {}, this build reads {FORMAT_VERSION})", f.version));
        }
        Ok(f)
    }
}
