//! Cross-relaunch persistence for the Bushing ID user library
//! (`bushing_id_picker.rs`) - unlike the reamer catalog, there is no
//! industry catalog for a finished bushing ID, so this library is purely
//! user-defined/imported entries, same `crate::library` machinery
//! (import/export/duplicate-pruning/conflict-resolution/labeling) as
//! `reamer_persistence.rs`, keyed by `label` instead of `size_label`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::library::{self, LibraryItem};
use crate::paths::app_data_dir;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedBushingId {
    pub label: String,
    pub id_in: f64,
}

fn default_file_path() -> PathBuf {
    app_data_dir().unwrap_or_default().join("bushing-id-library.json")
}

pub fn load() -> Vec<LibraryItem<PersistedBushingId>> {
    load_from(&default_file_path()).unwrap_or_default()
}

pub fn load_from(path: &Path) -> Option<Vec<LibraryItem<PersistedBushingId>>> {
    let json = std::fs::read_to_string(path).ok()?;
    library::import_json(&json).ok()
}

pub fn save(items: &[LibraryItem<PersistedBushingId>]) {
    save_to(&default_file_path(), items);
}

pub fn save_to(path: &Path, items: &[LibraryItem<PersistedBushingId>]) {
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

    #[test]
    fn save_and_load_round_trip_via_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bushing-id-library.json");
        let items = vec![LibraryItem { item: PersistedBushingId { label: "0.3750".to_string(), id_in: 0.375 }, labels: vec!["Preferred".to_string()] }];
        save_to(&path, &items);
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, items);
    }

    #[test]
    fn load_from_missing_file_returns_none() {
        assert!(load_from(Path::new("/this/does/not/exist/bushing-id-library.json")).is_none());
    }
}
