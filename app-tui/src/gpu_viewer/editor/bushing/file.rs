//! The bushing file: the workbench inputs as JSON, so a case can be saved, reopened and handed over. Numbers are keyed by
//! their row label, choices by their label, materials by their catalog id; a value a file does not carry keeps the
//! workbench default, and a key this build does not know is ignored, so older and newer files both open.
use crate::toolboxes::bushing::model::{self, BushingModel, FitType, NumberTarget, ToleranceMode};
use bushing_solver::countersink::CsMode;
use bushing_solver::geometry::{BushingType, IdType};
use bushing_solver::solve::EndConstraint;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FORMAT_VERSION: u32 = 1;

/// Every number row. The match forces a compile error when a target is added without a line here.
pub const NUMBERS: [NumberTarget; 40] = {
    use NumberTarget::*;
    [
        BoreDia, BoreTolPlus, BoreTolMinus, IdBushing, Interference, InterferenceTolPlus, InterferenceTolMinus, HousingLen, HousingWidth, EdgeDist, Friction, DeltaT, MinWallStraight, EdgeLoadAngleDeg, Load, FlangeOd, FlangeThk, MinWallNeck, CsDia, CsDepth, CsAngle,
        CsDiaTolPlus, CsDiaTolMinus, CsDepthTolPlus, CsDepthTolMinus, CsAngleTolPlus, CsAngleTolMinus, ExtCsDia, ExtCsDepth, ExtCsAngle, ExtCsDiaTolPlus, ExtCsDiaTolMinus, ExtCsDepthTolPlus, ExtCsDepthTolMinus, ExtCsAngleTolPlus, ExtCsAngleTolMinus, MaxBoreNominalShift,
        BoreCapabilityMinWidth, AssemblyHousingTemp, AssemblyBushingTemp,
    ]
};
#[allow(dead_code)]
fn every_number_target_is_listed(t: NumberTarget) {
    use NumberTarget::*;
    match t {
        BoreDia | BoreTolPlus | BoreTolMinus | IdBushing | Interference | InterferenceTolPlus | InterferenceTolMinus | HousingLen | HousingWidth | EdgeDist | Friction | DeltaT | MinWallStraight | EdgeLoadAngleDeg | Load | FlangeOd | FlangeThk | MinWallNeck | CsDia | CsDepth | CsAngle | CsDiaTolPlus
        | CsDiaTolMinus | CsDepthTolPlus | CsDepthTolMinus | CsAngleTolPlus | CsAngleTolMinus | ExtCsDia | ExtCsDepth | ExtCsAngle | ExtCsDiaTolPlus | ExtCsDiaTolMinus | ExtCsDepthTolPlus | ExtCsDepthTolMinus | ExtCsAngleTolPlus | ExtCsAngleTolMinus | MaxBoreNominalShift
        | BoreCapabilityMinWidth | AssemblyHousingTemp | AssemblyBushingTemp => {}
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BushingFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub numbers: BTreeMap<String, f64>,
    #[serde(default)]
    pub choices: BTreeMap<String, String>,
    #[serde(default)]
    pub flags: BTreeMap<String, bool>,
    #[serde(default)]
    pub housing_material: Option<String>,
    #[serde(default)]
    pub bushing_material: Option<String>,
}

/// The state after cycling `start` through `cycle` until `label` reads `want`.
fn from_label<T: Copy>(start: T, cycle: fn(T) -> T, label: fn(T) -> &'static str, want: &str) -> Option<T> {
    let mut v = start;
    for _ in 0..8 {
        if label(v) == want {
            return Some(v);
        }
        v = cycle(v);
    }
    None
}

impl BushingFile {
    pub fn capture(m: &BushingModel, name: &str) -> Self {
        let numbers = NUMBERS.iter().map(|t| (t.label().to_string(), m.number_value(*t))).collect();
        let choices = [
            ("fit type", model::label_fit_type(m.fit_type)),
            ("tolerance entry", m.tolerance_mode.label()),
            ("od geometry", model::label_bushing_type(m.bushing_type)),
            ("id geometry", model::label_id_type(m.id_type)),
            ("end constraint", model::label_end_constraint(m.end_constraint)),
            ("internal cs mode", model::label_cs_mode(m.cs_mode)),
            ("external cs mode", model::label_cs_mode(m.ext_cs_mode)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let flags = [
            ("enforcement", m.enforcement_enabled),
            ("lock bore", m.lock_bore),
            ("preserve bore nominal", m.preserve_bore_nominal),
            ("allow bore nominal shift", m.allow_bore_nominal_shift),
            ("install thermal assist", m.assembly_thermal_enabled),
            ("advanced open", m.advanced_open),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        Self { version: FORMAT_VERSION, name: name.to_string(), numbers, choices, flags, housing_material: Some(m.housing_material().id.to_string()), bushing_material: Some(m.bushing_material().id.to_string()) }
    }

    /// Put the file's inputs into `m` and recompute. Returns what could not be applied (an unknown material, a choice this
    /// build does not offer); everything else is applied.
    pub fn apply(&self, m: &mut BushingModel) -> Vec<String> {
        let mut skipped = Vec::new();
        for t in NUMBERS {
            if let Some(v) = self.numbers.get(t.label()).copied().filter(|v| v.is_finite()) {
                m.set_number(t, v);
            }
        }
        let mut choice = |key: &str, apply: &mut dyn FnMut(&str) -> bool| {
            if let Some(want) = self.choices.get(key) {
                if !apply(want) {
                    skipped.push(format!("{key}: {want}"));
                }
            }
        };
        choice("fit type", &mut |w| from_label(FitType::default(), model::cycle_fit_type, model::label_fit_type, w).map(|v| m.fit_type = v).is_some());
        choice("tolerance entry", &mut |w| [ToleranceMode::default(), ToleranceMode::default().toggled()].into_iter().find(|t| t.label() == w).map(|v| m.tolerance_mode = v).is_some());
        choice("od geometry", &mut |w| from_label(BushingType::Straight, model::cycle_bushing_type, model::label_bushing_type, w).map(|v| m.bushing_type = v).is_some());
        choice("id geometry", &mut |w| from_label(IdType::Straight, model::cycle_id_type, model::label_id_type, w).map(|v| m.id_type = v).is_some());
        choice("end constraint", &mut |w| from_label(EndConstraint::Free, model::cycle_end_constraint, model::label_end_constraint, w).map(|v| m.end_constraint = v).is_some());
        choice("internal cs mode", &mut |w| from_label(CsMode::default(), model::cycle_cs_mode, model::label_cs_mode, w).map(|v| m.cs_mode = v).is_some());
        choice("external cs mode", &mut |w| from_label(CsMode::default(), model::cycle_cs_mode, model::label_cs_mode, w).map(|v| m.ext_cs_mode = v).is_some());
        let flag = |key: &str| self.flags.get(key).copied();
        if let Some(v) = flag("enforcement") {
            m.enforcement_enabled = v;
        }
        if let Some(v) = flag("lock bore") {
            m.lock_bore = v;
        }
        if let Some(v) = flag("preserve bore nominal") {
            m.preserve_bore_nominal = v;
        }
        if let Some(v) = flag("allow bore nominal shift") {
            m.allow_bore_nominal_shift = v;
        }
        if let Some(v) = flag("install thermal assist") {
            m.assembly_thermal_enabled = v;
        }
        if let Some(v) = flag("advanced open") {
            m.advanced_open = v;
        }
        let catalog = m.material_catalog();
        let find = |id: &Option<String>| id.as_deref().map(|id| catalog.iter().position(|c| c.id == id).ok_or_else(|| format!("material {id}")));
        let (housing, bushing) = (find(&self.housing_material), find(&self.bushing_material));
        if let Some(r) = housing {
            match r {
                Ok(i) => m.housing_material_index = i,
                Err(e) => skipped.push(e),
            }
        }
        if let Some(r) = bushing {
            match r {
                Ok(i) => m.bushing_material_index = i,
                Err(e) => skipped.push(e),
            }
        }
        m.recompute();
        skipped
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("plain data serialises")
    }
    pub fn from_json(text: &str) -> Result<Self, String> {
        let f: Self = serde_json::from_str(text).map_err(|e| format!("not a bushing file: {e}"))?;
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
    fn every_number_row_is_listed_once_with_its_own_label() {
        let mut labels: Vec<_> = NUMBERS.iter().map(|t| t.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), NUMBERS.len());
    }

    #[test]
    fn a_changed_model_round_trips_through_json_and_the_solver_agrees() {
        let mut m = BushingModel::default();
        m.commit_number(NumberTarget::BoreDia, 0.625);
        m.commit_number(NumberTarget::Load, 2500.0);
        m.commit_number(NumberTarget::BoreTolPlus, 0.0005);
        m.toggle_fit_type();
        m.toggle_bushing_type();
        m.toggle_end_constraint();
        m.advanced_open = true;
        m.select_housing_material(3);
        m.select_bushing_material(5);
        let file = BushingFile::capture(&m, "case");
        let back = BushingFile::from_json(&file.to_json()).unwrap();
        assert_eq!(back, file);
        let mut m2 = BushingModel::default();
        assert!(back.apply(&mut m2).is_empty());
        assert_eq!(BushingFile::capture(&m2, "case"), file);
        assert_eq!(m2.output.pressure, m.output.pressure);
        assert_eq!(m2.output.ed_actual, m.output.ed_actual);
        assert_eq!((m2.fit_type, m2.bushing_type, m2.end_constraint), (m.fit_type, m.bushing_type, m.end_constraint));
        assert_eq!((m2.housing_material().id, m2.bushing_material().id), (m.housing_material().id, m.bushing_material().id));
    }

    #[test]
    fn a_file_that_lacks_keys_keeps_the_defaults_and_unknown_things_are_reported_not_fatal() {
        let mut m = BushingModel::default();
        let d = BushingFile::capture(&BushingModel::default(), "");
        let f = BushingFile::from_json(r#"{"numbers":{"Applied Edge Load":1234.0,"Not A Row":9.0},"choices":{"fit type":"Warp"},"housing_material":"unobtainium"}"#).unwrap();
        let skipped = f.apply(&mut m);
        assert_eq!(m.load, 1234.0);
        assert_eq!(m.bore_dia, BushingModel::default().bore_dia);
        assert_eq!(m.fit_type, FitType::default());
        assert_eq!(skipped, ["fit type: Warp".to_string(), "material unobtainium".to_string()]);
        assert_eq!(BushingFile::capture(&m, "").housing_material, d.housing_material, "an unknown material leaves the current one");
    }

    #[test]
    fn a_broken_or_newer_file_is_refused_with_a_reason() {
        assert!(BushingFile::from_json("{ nope").unwrap_err().contains("not a bushing file"));
        assert!(BushingFile::from_json(r#"{"version": 99}"#).unwrap_err().contains("newer"));
    }
}
