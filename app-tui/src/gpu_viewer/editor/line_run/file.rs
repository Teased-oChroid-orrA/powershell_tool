//! The Line Run case file: every input (`model::Inputs`, units included) as JSON. Constants and allowables that were
//! never entered stay absent and stay unset when the file is opened; results are recomputed, never stored.
use crate::toolboxes::line_run::model::{Inputs, LineRunModel};
use serde::{Deserialize, Serialize};

const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineRunFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub inputs: Inputs,
}

impl LineRunFile {
    pub fn capture(m: &LineRunModel, name: &str) -> Self {
        Self { version: FORMAT_VERSION, name: name.to_string(), inputs: m.inputs.clone() }
    }
    pub fn apply(&self, m: &mut LineRunModel) {
        m.set_inputs(self.inputs.clone());
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plain data serializes")
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        let f: Self = serde_json::from_str(text).map_err(|e| format!("not a line run case: {e}"))?;
        if f.version > FORMAT_VERSION {
            return Err(format!("written by a newer version (format {}, this build reads {FORMAT_VERSION})", f.version));
        }
        Ok(f)
    }
}
