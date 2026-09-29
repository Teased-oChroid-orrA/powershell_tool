//! Cross-relaunch persistence for the Pressure Vessel Analyzer toolbox -
//! currently just user-added custom materials (`material_picker.rs`'s
//! "add new material" flow). Mirrors `toolboxes/search/persistence.rs`'s
//! per-OS path resolution exactly (same reasoning: Windows is the only
//! real shipping target, the other branches only help local development),
//! under a fourth, distinct filename so this toolbox's settings never
//! collide with Search Files' `settings-tui.json` or either other head's
//! own settings file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

fn app_data_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("GSEngineeringToolbench"))
}

fn config_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("settings-tui-pressure-vessel.json"))
}

/// Fixed (non-timestamped) export destination for the `e` "export report"
/// key - overwritten on every export, so it always reflects the latest
/// vessel design rather than accumulating a file per press. Path
/// resolution lives here (not in the pure `view.rs::build_report_text`,
/// which only builds the text) so `Effect::ExportPressureVesselReport`'s
/// execution in `main.rs` is the only place that touches the filesystem,
/// matching this crate's own "side effects only in `execute_effect`" rule.
pub fn report_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("reports").join("pressure-vessel-report.txt"))
}

/// One user-added material - plain owned fields (unlike
/// `mechanics_core::materials::Material`'s `&'static str` id/name, which
/// exist for a `'static` built-in table, not a deserialized one).
/// `fbru_ksi`/`fsu_ksi` are deliberately not carried here - this toolbox
/// never reads them (see `model.rs::leak_custom_material`'s own comment).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedMaterial {
    pub name: String,
    pub e_ksi: f64,
    pub sy_ksi: f64,
    pub ftu_ksi: f64,
    pub nu: f64,
    pub alpha_u_f: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedFile {
    #[serde(default)]
    materials: Vec<PersistedMaterial>,
}

/// Empty (not `Option`/`Result`) on a missing/corrupt/unwritable file -
/// "no custom materials yet" is indistinguishable from "couldn't read the
/// file" for this toolbox's purposes, and both should behave identically
/// to a fresh install, matching `search/persistence.rs::load`'s own
/// "never a reason to interrupt startup" philosophy.
pub fn load() -> Vec<PersistedMaterial> {
    config_path().and_then(|p| load_from(&p)).map(|f| f.materials).unwrap_or_default()
}

fn load_from(path: &Path) -> Option<PersistedFile> {
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

/// Best-effort, write-to-temp-then-rename - same crash-safety reasoning as
/// `search/persistence.rs::save_to`. A failed save is never a reason to
/// interrupt the user.
pub fn save(materials: &[PersistedMaterial]) {
    if let Some(path) = config_path() {
        save_to(&path, &PersistedFile { materials: materials.to_vec() });
    }
}

fn save_to(path: &Path, file: &PersistedFile) {
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(json) = serde_json::to_string_pretty(file) else { return };
    let tmp_path = path.with_extension("json.tmp");
    if std::fs::write(&tmp_path, json).is_ok() {
        let _ = std::fs::rename(&tmp_path, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip_via_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings-tui-pressure-vessel.json");
        let materials = vec![PersistedMaterial { name: "Unobtainium".to_string(), e_ksi: 99999.0, sy_ksi: 5000.0, ftu_ksi: 6000.0, nu: 0.25, alpha_u_f: 3.0 }];
        save_to(&path, &PersistedFile { materials: materials.clone() });
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.materials, materials);
    }

    #[test]
    fn load_from_missing_file_returns_none_not_a_panic() {
        assert!(load_from(Path::new("/this/does/not/exist/settings.json")).is_none());
    }

    #[test]
    fn load_from_a_file_missing_the_materials_key_defaults_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        let loaded = load_from(&path).unwrap();
        assert!(loaded.materials.is_empty());
    }
}
