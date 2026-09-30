//! Cross-relaunch persistence for the Bushing Workbench's user-added/
//! imported material library (`material_picker.rs`'s "add new material"
//! form plus its import/export overlay). Mirrors
//! `toolboxes/pressure_vessel/persistence.rs`'s `PersistedMaterial` shape,
//! extended with `fbru_ksi`/`fsu_ksi` (bearing/shear ultimate) - that
//! toolbox zeroes them because `pressure-vessel-solver` never reads them,
//! but `bushing_solver::solve::compute` does (edge-bearing/shear margin
//! checks), so this toolbox's form must collect them for real. A
//! material's `name` is its catalog key (the same role
//! `ReamerEntry::size_label` plays for reamers), so it's a normal field on
//! the persisted type rather than reconstructed separately.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::library::{self, LibraryItem};
use crate::paths::app_data_dir;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedMaterial {
    pub name: String,
    pub e_ksi: f64,
    pub sy_ksi: f64,
    pub fbru_ksi: f64,
    pub fsu_ksi: f64,
    pub ftu_ksi: f64,
    pub nu: f64,
    pub alpha_u_f: f64,
}

fn default_file_path() -> PathBuf {
    app_data_dir().unwrap_or_default().join("bushing-material-library.json")
}

pub fn load() -> Vec<LibraryItem<PersistedMaterial>> {
    load_from(&default_file_path()).unwrap_or_default()
}

pub fn load_from(path: &Path) -> Option<Vec<LibraryItem<PersistedMaterial>>> {
    let json = std::fs::read_to_string(path).ok()?;
    library::import_json(&json).ok()
}

pub fn save(items: &[LibraryItem<PersistedMaterial>]) {
    save_to(&default_file_path(), items);
}

pub fn save_to(path: &Path, items: &[LibraryItem<PersistedMaterial>]) {
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let json = library::export_json(items);
    let tmp_path = path.with_extension("json.tmp");
    if std::fs::write(&tmp_path, json).is_ok() {
        let _ = std::fs::rename(&tmp_path, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LibraryItem<PersistedMaterial> {
        LibraryItem {
            item: PersistedMaterial { name: "Unobtainium".to_string(), e_ksi: 99999.0, sy_ksi: 5000.0, fbru_ksi: 6000.0, fsu_ksi: 4000.0, ftu_ksi: 6000.0, nu: 0.25, alpha_u_f: 3.0 },
            labels: vec!["Preferred".to_string()],
        }
    }

    #[test]
    fn save_and_load_round_trip_via_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bushing-material-library.json");
        let items = vec![sample()];
        save_to(&path, &items);
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, items);
    }

    #[test]
    fn load_from_missing_file_returns_none() {
        assert!(load_from(Path::new("/this/does/not/exist/bushing-material-library.json")).is_none());
    }
}
