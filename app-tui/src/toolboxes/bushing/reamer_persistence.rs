//! Cross-relaunch persistence for user-added/imported reamer library
//! entries (`reamer_picker.rs`'s own library overlay). Mirrors
//! `toolboxes/pressure_vessel/persistence.rs`'s "plain owned-field struct,
//! not the solver crate's own type" pattern (`PersistedMaterial` there,
//! `PersistedReamer` here) - `bushing_solver::reamers::ReamerEntry`
//! deliberately has no serde dependency (that crate is a pure, zero-GUI
//! calculation library), so this crate owns the JSON-shaped type and maps
//! to/from the solver's own type at the boundary.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::library::{self, LibraryItem};
use crate::paths::app_data_dir;
use bushing_solver::reamers::{AvailabilityTier, ReamerEntry};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedReamer {
    pub size_label: String,
    pub nominal_in: f64,
    pub tool_tolerance_plus_in: f64,
    pub tool_tolerance_minus_in: f64,
    #[serde(default)]
    pub notes: String,
}

impl PersistedReamer {
    pub fn from_entry(entry: &ReamerEntry) -> Self {
        Self { size_label: entry.size_label.clone(), nominal_in: entry.nominal_in, tool_tolerance_plus_in: entry.tool_tolerance_plus_in, tool_tolerance_minus_in: entry.tool_tolerance_minus_in, notes: entry.notes.clone() }
    }

    /// User-added entries have no built-in availability tier/preferred-rank
    /// concept (those are properties of the sourced aircraft reamer
    /// catalog specifically) - `Common`/`None` render with no tier tag,
    /// same as most built-in entries.
    pub fn to_entry(&self) -> ReamerEntry {
        ReamerEntry {
            size_label: self.size_label.clone(),
            nominal_in: self.nominal_in,
            tool_tolerance_plus_in: self.tool_tolerance_plus_in,
            tool_tolerance_minus_in: self.tool_tolerance_minus_in,
            availability_tier: AvailabilityTier::Common,
            preferred_rank: None,
            notes: self.notes.clone(),
        }
    }
}

pub fn default_file_path() -> PathBuf {
    app_data_dir().unwrap_or_default().join("reamer-library.json")
}

pub fn load() -> Vec<LibraryItem<PersistedReamer>> {
    load_from(&default_file_path()).unwrap_or_default()
}

pub fn load_from(path: &Path) -> Option<Vec<LibraryItem<PersistedReamer>>> {
    let json = std::fs::read_to_string(path).ok()?;
    library::import_json(&json).ok()
}

pub fn save(items: &[LibraryItem<PersistedReamer>]) {
    save_to(&default_file_path(), items);
}

pub fn save_to(path: &Path, items: &[LibraryItem<PersistedReamer>]) {
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

/// The built-in catalog re-expressed as unlabeled library items, for the
/// "export built-in + current definitions" action (per-spec: export must
/// cover the built-ins too, not only user-added entries).
pub fn builtin_as_library_items() -> Vec<LibraryItem<PersistedReamer>> {
    bushing_solver::reamers::all_reamers().into_iter().map(|e| LibraryItem::new(PersistedReamer::from_entry(e))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip_via_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reamer-library.json");
        let items = vec![LibraryItem { item: PersistedReamer { size_label: "Q".to_string(), nominal_in: 0.332, tool_tolerance_plus_in: 0.0003, tool_tolerance_minus_in: 0.0, notes: String::new() }, labels: vec!["Preferred".to_string()] }];
        save_to(&path, &items);
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, items);
    }

    #[test]
    fn load_from_missing_file_returns_none() {
        assert!(load_from(Path::new("/this/does/not/exist/reamer-library.json")).is_none());
    }

    #[test]
    fn to_entry_round_trips_the_numeric_fields() {
        let persisted = PersistedReamer { size_label: "Q".to_string(), nominal_in: 0.332, tool_tolerance_plus_in: 0.0003, tool_tolerance_minus_in: 0.0001, notes: "custom".to_string() };
        let entry = persisted.to_entry();
        assert_eq!(entry.size_label, "Q");
        assert_eq!(entry.nominal_in, 0.332);
        assert_eq!(entry.notes, "custom");
    }

    #[test]
    fn builtin_as_library_items_covers_the_whole_catalog() {
        let items = builtin_as_library_items();
        assert_eq!(items.len(), bushing_solver::reamers::all_reamers().len());
        assert!(items.iter().all(|i| i.labels.is_empty()));
    }
}
