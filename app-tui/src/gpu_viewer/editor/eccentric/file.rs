//! The eccentric-bushing case: its own inputs plus where the Bushing case it reads lives. Nothing the Bushing workbench
//! owns (bore, ID, interference, friction, length, load, materials, edge distance) is stored here: it is read from that
//! file, so it is entered once.
use crate::toolboxes::eccentric_bushing::model::EccentricUi;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EccentricFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    /// The Bushing case this reads (a path, relative to this file's folder when not absolute).
    #[serde(default = "default_case")]
    pub bushing_case: String,
    #[serde(default = "d_offset")]
    pub offset: f64,
    #[serde(default = "d_angle")]
    pub load_angle: f64,
    #[serde(default = "d_boss")]
    pub boss_factor: f64,
    #[serde(default)]
    pub plane_strain: bool,
    #[serde(default = "d_clearance")]
    pub pin_clearance: f64,
    #[serde(default = "d_friction")]
    pub pin_friction: f64,
    #[serde(default = "d_modulus")]
    pub pin_modulus: f64,
    #[serde(default = "d_nu")]
    pub pin_nu: f64,
    #[serde(default = "yes")]
    pub credit_pin_load: bool,
    #[serde(default)]
    pub direct_onset: bool,
    #[serde(default)]
    pub edge_limited: bool,
    #[serde(default)]
    pub advanced_open: bool,
}

fn default_case() -> String {
    crate::gpu_viewer::EditorKind::Bushing.default_file().into()
}
fn d_offset() -> f64 {
    EccentricUi::default().offset
}
fn d_angle() -> f64 {
    EccentricUi::default().load_angle
}
fn d_boss() -> f64 {
    EccentricUi::default().boss_factor
}
fn d_clearance() -> f64 {
    EccentricUi::default().pin_clearance
}
fn d_friction() -> f64 {
    EccentricUi::default().pin_friction
}
fn d_modulus() -> f64 {
    EccentricUi::default().pin_modulus
}
fn d_nu() -> f64 {
    EccentricUi::default().pin_nu
}
fn yes() -> bool {
    true
}

impl EccentricFile {
    pub fn new(name: &str, bushing_case: &str, ui: &EccentricUi) -> Self {
        Self {
            version: FORMAT_VERSION,
            name: name.into(),
            bushing_case: bushing_case.into(),
            offset: ui.offset,
            load_angle: ui.load_angle,
            boss_factor: ui.boss_factor,
            plane_strain: ui.plane_strain,
            pin_clearance: ui.pin_clearance,
            pin_friction: ui.pin_friction,
            pin_modulus: ui.pin_modulus,
            pin_nu: ui.pin_nu,
            credit_pin_load: ui.credit_pin_load,
            direct_onset: ui.direct_onset,
            edge_limited: ui.edge_limited,
            advanced_open: ui.advanced_open,
        }
    }
    pub fn ui(&self) -> EccentricUi {
        let mut ui = EccentricUi { advanced_open: self.advanced_open, plane_strain: self.plane_strain, credit_pin_load: self.credit_pin_load, direct_onset: self.direct_onset, edge_limited: self.edge_limited, ..EccentricUi::default() };
        // The same clamps a typed value gets.
        for (t, v) in [
            (crate::toolboxes::eccentric_bushing::model::NumberTarget::Offset, self.offset),
            (crate::toolboxes::eccentric_bushing::model::NumberTarget::LoadAngle, self.load_angle),
            (crate::toolboxes::eccentric_bushing::model::NumberTarget::BossFactor, self.boss_factor),
            (crate::toolboxes::eccentric_bushing::model::NumberTarget::PinClearance, self.pin_clearance),
            (crate::toolboxes::eccentric_bushing::model::NumberTarget::PinFriction, self.pin_friction),
            (crate::toolboxes::eccentric_bushing::model::NumberTarget::PinModulus, self.pin_modulus),
            (crate::toolboxes::eccentric_bushing::model::NumberTarget::PinNu, self.pin_nu),
        ] {
            ui.commit_number(t, v);
        }
        ui
    }
    /// The Bushing case's path as this file's folder resolves it.
    pub fn case_path(&self, file: &Path) -> PathBuf {
        let p = Path::new(&self.bushing_case);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            file.parent().unwrap_or(Path::new("")).join(p)
        }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plain data serialises")
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        let f: Self = serde_json::from_str(text).map_err(|e| format!("not an eccentric bushing file: {e}"))?;
        if f.version > FORMAT_VERSION {
            return Err(format!("this file is version {}, newer than this program reads ({FORMAT_VERSION})", f.version));
        }
        Ok(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inputs_round_trip_and_a_sparse_file_keeps_the_defaults() {
        let mut ui = EccentricUi::default();
        ui.offset = 0.035;
        ui.edge_limited = true;
        ui.plane_strain = true;
        ui.pin_modulus = 16.0;
        let f = EccentricFile::new("case", "b.json", &ui);
        let back = EccentricFile::from_json(&f.to_json()).unwrap();
        assert_eq!(back.ui(), ui);
        let sparse = EccentricFile::from_json(r#"{"offset": 0.05}"#).unwrap();
        assert_eq!(sparse.ui().offset, 0.05);
        assert_eq!(sparse.ui().load_angle, EccentricUi::default().load_angle);
        assert_eq!(sparse.bushing_case, "bushing-case.json");
    }

    #[test]
    fn a_value_out_of_range_is_clamped_like_a_typed_one_and_a_bad_file_is_refused() {
        let f = EccentricFile::from_json(r#"{"offset": -1, "pin_nu": 0.9, "boss_factor": 0.2}"#).unwrap();
        let ui = f.ui();
        assert_eq!((ui.offset, ui.pin_nu, ui.boss_factor), (0.0, 0.49, 1.5));
        assert!(EccentricFile::from_json("{ nope").unwrap_err().contains("not an eccentric"));
        assert!(EccentricFile::from_json(r#"{"version": 9}"#).unwrap_err().contains("newer"));
    }

    #[test]
    fn the_bushing_case_is_found_beside_the_file_unless_it_is_absolute() {
        let f = EccentricFile::new("c", "wing.json", &EccentricUi::default());
        assert_eq!(f.case_path(Path::new("/work/ecc.json")), Path::new("/work/wing.json"));
        let g = EccentricFile::new("c", "/abs/wing.json", &EccentricUi::default());
        assert_eq!(g.case_path(Path::new("/work/ecc.json")), Path::new("/abs/wing.json"));
    }
}
