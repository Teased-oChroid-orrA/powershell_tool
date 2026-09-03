//! Settings persistence across relaunches - egui port of
//! `app/src/persistence.rs`. That module is unusable here as-is (it
//! imports `dioxus::prelude::*` for its `AppState`-coupled `apply`/`save`
//! signatures, so pulling it in would drag a dioxus dependency into
//! app-egui); the underlying `PersistedState` shape and the config-
//! directory resolution logic are duplicated here in plain serde form.
//!
//! Scoped to exactly what this crate's tools currently expose (see
//! docs/issue-11-phase-14.md's scope-cut list) - persisting a field this
//! UI doesn't have a control for yet would be dead weight, not
//! forward-compatibility. Extend `PersistedState` as each tool's real
//! settings surface grows, the same way `app/`'s own `PersistedState`
//! grew field-by-field over this session (see its `#[serde(default)]`
//! comments for why old config files must never hard-fail to load).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct PersistedState {
    pub dark: Option<bool>,
    pub rail_pinned: Option<bool>,

    pub search_path: String,
    pub filters_text: String,
    pub parallel: Option<bool>,

    pub pv_outer_diameter: Option<f64>,
    pub pv_wall_thickness: Option<f64>,
    pub pv_internal_pressure: Option<f64>,
    pub pv_external_pressure: Option<f64>,
    pub pv_closed_ends: Option<bool>,
    pub pv_material_id: String,
    pub pv_required_ms: Option<f64>,
    pub pv_unsupported_length: Option<f64>,

    pub bu_bore_dia: Option<f64>,
    pub bu_id_bushing: Option<f64>,
    pub bu_housing_len: Option<f64>,
    pub bu_housing_width: Option<f64>,
    pub bu_edge_dist: Option<f64>,
    pub bu_interference: Option<f64>,
    pub bu_mat_housing: String,
    pub bu_mat_bushing: String,
    pub bu_d_t: Option<f64>,
    pub bu_min_wall_straight: Option<f64>,
}

fn config_path() -> Option<PathBuf> {
    // Win-x64 is the only real deployment target (CLAUDE.md's "Target
    // environment") - the other branches exist only so local development
    // also persists settings, matching app/src/persistence.rs's own
    // documented reasoning for the same hand-rolled (no `dirs` crate)
    // resolution.
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"));

    base.map(|b| b.join("GSEngineeringToolbench").join("settings-egui.json"))
}

pub fn load() -> Option<PersistedState> {
    let path = config_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(state: &PersistedState) {
    let Some(path) = config_path() else { return };
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    // Best-effort, same as app/'s own persistence and cache writes - a
    // failed settings save is never a reason to interrupt the user.
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, json);
    }
}
