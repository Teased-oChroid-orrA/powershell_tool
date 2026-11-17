//! The Pressure Vessel case file: every input of `PressureVesselModel` (the numbers by name, the end condition, the
//! material by name) as JSON, so a file outlives a reordering of the rows. A custom material travels with the case
//! (`custom_material`) and is added to the catalog when a machine opening it does not have it. Results are never
//! stored: they are recomputed.
use crate::toolboxes::pressure_vessel::{
    model::{is_custom, HeadComponent, HeadInputs, NumberTarget, PressureVesselModel},
    persistence::PersistedMaterial,
    requirements::{self, MaterialKind, Requirement},
    line::LineRunInputs,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Older readers must not silently ignore wall-loss inputs and show optimistic checks.
// Format 3 adds the code check's inputs; an older build would drop them and show a different check.
// Format 4 adds the requirement rows, the material class, the allowable overrides and the line run.
// Format 5 adds the line run's segment (end conditions, the bound); an older build would solve the whole run instead.
const FORMAT_VERSION: u32 = 5;

/// Every number input with its key in the file.
pub const NUMBERS: [(NumberTarget, &str); 15] = [
    (NumberTarget::OuterDiameter, "outer diameter"),
    (NumberTarget::WallThickness, "wall thickness"),
    (NumberTarget::CorrosionAllowance, "corrosion allowance"),
    (NumberTarget::MillUndertolerance, "mill undertolerance percent"),
    (NumberTarget::InternalPressure, "internal pressure"),
    (NumberTarget::ExternalPressure, "external pressure"),
    (NumberTarget::RequiredMs, "required minimum MS"),
    (NumberTarget::UnsupportedLength, "unsupported length"),
    (NumberTarget::TemperatureDifferential, "temperature differential"),
    (NumberTarget::HeadRadius, "head inside radius"),
    (NumberTarget::HeadCrownRadius, "head crown radius"),
    (NumberTarget::HeadKnuckleRadius, "head knuckle radius"),
    (NumberTarget::HeadAllowableStress, "head allowable stress"),
    (NumberTarget::HeadEfficiency, "head joint efficiency"),
    (NumberTarget::HeadPressure, "head design pressure"),
];
#[allow(dead_code)]
fn every_number_target_is_listed(t: NumberTarget) {
    use NumberTarget::*;
    match t {
        OuterDiameter | WallThickness | CorrosionAllowance | MillUndertolerance | InternalPressure | ExternalPressure | RequiredMs | UnsupportedLength | TemperatureDifferential | HeadRadius | HeadCrownRadius | HeadKnuckleRadius | HeadAllowableStress | HeadEfficiency | HeadPressure => {}
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
    /// The code check's component, by `HeadComponent::label`.
    #[serde(default)]
    pub head_component: Option<String>,
    /// The material's properties when it is a user-added one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_material: Option<PersistedMaterial>,
    /// "Minimum wall by requirement" rows; absent in cases older than format 4 (seeded from the material then).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirements: Option<Vec<Requirement>>,
    /// The person's material class when it differs from the catalog's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_kind: Option<MaterialKind>,
    /// Overrides of yield, ultimate and ultimate compression strength (ksi).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowables: Option<[Option<f64>; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_run: Option<LineRunInputs>,
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
            head_component: Some(m.head.component.label().to_string()),
            custom_material: is_custom(material).then(|| material.into()),
            requirements: Some(m.requirements.clone()),
            material_kind: m.kind_override,
            allowables: Some(m.allowable_override),
            line_run: Some(m.line.clone()),
        }
    }

    /// Put the file's inputs into `m` and recompute. Returns what could not be applied (a material this catalog does not
    /// have and the file does not describe); everything else is applied.
    pub fn apply(&self, m: &mut PressureVesselModel) -> Vec<String> {
        let mut skipped = Vec::new();
        // Cases written before wall allowances mean zero loss, including when applied over an edited model.
        m.corrosion_allowance = 0.0;
        m.mill_undertolerance = 0.0;
        // Cases written before the code check carry none: the defaults, also over an edited model.
        if self.version < 3 {
            m.head = HeadInputs::default();
        }
        for (t, key) in NUMBERS {
            if let Some(v) = self.numbers.get(key) {
                m.commit_number(t, *v);
            }
        }
        match self.head_component.as_deref().map(|l| (l, HeadComponent::from_label(l))) {
            Some((_, Some(c))) => m.head.component = c,
            Some((l, None)) => skipped.push(format!("code check component: {l}")),
            None => {}
        }
        if let Some(closed) = self.closed_ends {
            m.closed_ends = closed;
        }
        if !self.material.is_empty() {
            match m.material_catalog().iter().position(|c| c.name == self.material) {
                Some(i) => m.select_material(i),
                None => match self.custom_material.as_ref().filter(|c| c.name == self.material) {
                    Some(c) => m.add_custom_material(c.name.clone(), c.e_ksi, c.sy_ksi, c.ftu_ksi, c.nu, c.alpha_u_f),
                    None => skipped.push(format!("material: {}", self.material)),
                },
            }
        }
        // Format 4 inputs come after the material (a material change reseeds and resets overrides); older cases carry none:
        // the seeds and the defaults, also over an edited model.
        m.kind_override = self.material_kind;
        m.allowable_override = self.allowables.unwrap_or([None; 3]);
        m.requirements = match &self.requirements {
            Some(rows) if self.version >= 4 => rows.clone(),
            _ => requirements::seeds(m.kind()),
        };
        m.line = match &self.line_run {
            Some(l) if self.version >= 4 => l.clone(),
            _ => LineRunInputs::new(m.internal_pressure, m.external_pressure, m.closed_ends, m.temperature_differential),
        };
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
