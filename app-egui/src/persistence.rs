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

use search_core::models::{ExcludeScope, GroupByMode, MatchMode};
use serde::{Deserialize, Serialize};

/// Every user-configurable Search field this UI exposes - one shared shape
/// used for cross-relaunch persistence, named presets, AND `PersistedState`
/// itself (`PersistedState::search`), matching `app/src/state.rs`'s own
/// `SavedPreset::settings` reusing its cross-relaunch `PersistedState`
/// shape rather than inventing a second one. `#[serde(default)]` on every
/// field: an old file (or hand-edited one) missing a field this UI didn't
/// have yet must fall back to that field's default, not fail the whole
/// load - the exact bug class `search-path`/`filters_text`/`parallel`
/// (this struct's only fields before this pass) were already silently
/// exposed to before any config file on disk had ever exercised it.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct SearchFieldsSnap {
    #[serde(default)]
    pub search_path: String,
    #[serde(default)]
    pub search_paths_extra: Vec<String>,
    #[serde(default)]
    pub output_folder: String,
    #[serde(default)]
    pub output_name: String,
    #[serde(default)]
    pub filters_text: String,
    #[serde(default)]
    pub exclude_filters_text: String,
    #[serde(default)]
    pub match_mode: MatchMode,
    #[serde(default = "default_proximity_lines")]
    pub proximity_lines: i32,
    #[serde(default)]
    pub use_regex: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default)]
    pub exclude_scope: ExcludeScope,
    #[serde(default)]
    pub extension_selected: Vec<String>,
    #[serde(default)]
    pub extension_filter_text: String,
    #[serde(default)]
    pub exclude_folders_text: String,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default = "default_max_file_size_mb")]
    pub max_file_size_mb: f64,
    #[serde(default)]
    pub group_by: GroupByMode,
    #[serde(default = "default_true")]
    pub export_html: bool,
    #[serde(default)]
    pub desktop_notification_when_done: bool,
    #[serde(default)]
    pub open_report_when_done: bool,
    #[serde(default)]
    pub export_csv: bool,
    #[serde(default)]
    pub export_json: bool,
    #[serde(default)]
    pub parallel: bool,
    #[serde(default = "search_core::models::default_throttle_limit")]
    pub throttle_limit: i32,
    #[serde(default = "search_core::models::default_heavy_throttle_limit")]
    pub heavy_throttle_limit: i32,
    #[serde(default)]
    pub cache_file_path: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_pdf_timeout_seconds")]
    pub pdf_timeout_seconds: i32,
    #[serde(default)]
    pub ocr_scanned_pdfs: bool,
    #[serde(default = "default_file_timeout_seconds")]
    pub file_timeout_seconds: i32,
    #[serde(default = "default_max_retries")]
    pub max_retries: i32,
    #[serde(default)]
    pub index_for_fast_search: bool,
    #[serde(default)]
    pub index_location: IndexLocation,
}

fn default_proximity_lines() -> i32 {
    5
}
fn default_max_file_size_mb() -> f64 {
    50.0
}
fn default_pdf_timeout_seconds() -> i32 {
    15
}
fn default_file_timeout_seconds() -> i32 {
    30
}
fn default_max_retries() -> i32 {
    3
}
fn default_true() -> bool {
    true
}

/// Hand-written, NOT `#[derive(Default)]` - a derived impl would give
/// every numeric field `0`/`0.0`, ignoring the `#[serde(default = "fn")]`
/// functions above entirely. Those only apply when a field is missing
/// from an ALREADY-PRESENT `search` JSON object; when the whole `search`
/// key is absent (e.g. an old settings file saved before this struct
/// existed), `PersistedState.search`'s own `#[serde(default)]` falls back
/// to `SearchFieldsSnap::default()` directly - a derived impl would have
/// silently zeroed every Performance/robustness field (throttle limits,
/// timeouts, max retries, max file size) on first load after an upgrade.
/// Found from a real screenshot showing exactly that: every numeric field
/// in the Performance section reading `0`. Every value here must match
/// its sibling `#[serde(default = "fn")]` function so the two paths never
/// diverge again - `SearchTool::new()` builds its initial state from this
/// same impl (`SearchTool::default_snapshot`) for the same reason.
impl Default for SearchFieldsSnap {
    fn default() -> Self {
        Self {
            search_path: String::new(),
            search_paths_extra: Vec::new(),
            output_folder: String::new(),
            output_name: String::new(),
            filters_text: String::new(),
            exclude_filters_text: String::new(),
            match_mode: MatchMode::default(),
            proximity_lines: default_proximity_lines(),
            use_regex: false,
            whole_word: false,
            exclude_scope: ExcludeScope::default(),
            extension_selected: Vec::new(),
            extension_filter_text: String::new(),
            exclude_folders_text: String::new(),
            include_hidden: false,
            max_file_size_mb: default_max_file_size_mb(),
            group_by: GroupByMode::default(),
            export_html: true,
            desktop_notification_when_done: false,
            open_report_when_done: false,
            export_csv: false,
            export_json: false,
            parallel: false,
            throttle_limit: search_core::models::default_throttle_limit(),
            heavy_throttle_limit: search_core::models::default_heavy_throttle_limit(),
            cache_file_path: String::new(),
            dry_run: false,
            pdf_timeout_seconds: default_pdf_timeout_seconds(),
            ocr_scanned_pdfs: false,
            file_timeout_seconds: default_file_timeout_seconds(),
            max_retries: default_max_retries(),
            index_for_fast_search: false,
            index_location: IndexLocation::SearchFolder,
        }
    }
}

/// Where the fast re-search native index lives. `SearchFolder` (the
/// default, and the ONLY option `app/`'s original design supports - ADR-011:
/// `.native-search-index/` inside the searched folder, auto-excluded) vs.
/// `OutputFolder` (new scope this pass, per explicit user request: a
/// single shared index directory at `output_folder/.native-search-index/`
/// covering every searched root - deliberately one consolidated index in
/// the multi-root case, not one per root, since there is only one output
/// folder to place it in).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
pub enum IndexLocation {
    #[default]
    SearchFolder,
    OutputFolder,
}

/// One prior search's search-defining fields (epic §23 "recent
/// searches") - most-recent-first, deduplicated, capped at 8 by
/// `SearchTool::remember_recent_search`. Mirrors `app/src/state.rs`'s
/// `RecentSearch` exactly (path + filters only, not a full settings
/// snapshot - that's what `SavedPreset` below is for).
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct RecentSearch {
    pub search_path: String,
    pub filters_text: String,
}

impl RecentSearch {
    pub fn label(&self) -> String {
        let folder_name = std::path::Path::new(&self.search_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.search_path.clone());
        format!("{folder_name} - {}", self.filters_text)
    }
}

/// A full Search settings snapshot saved under a user-given name - unlike
/// `RecentSearch` (an automatic MRU of just path+filters), a preset
/// captures every field in `SearchFieldsSnap` and only changes when the
/// user explicitly saves over it again.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedPreset {
    pub name: String,
    pub fields: SearchFieldsSnap,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct PersistedState {
    pub dark: Option<bool>,
    pub rail_pinned: Option<bool>,

    #[serde(default)]
    pub search: SearchFieldsSnap,
    #[serde(default)]
    pub recent_searches: Vec<RecentSearch>,
    #[serde(default)]
    pub saved_presets: Vec<SavedPreset>,

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
    pub bu_min_wall_neck: Option<f64>,
    /// `"slug"`/`"countersunk"`/`"flanged"` - persisted as a string
    /// rather than deriving `Serialize`/`Deserialize` on
    /// `bushing_solver::geometry::BushingType` itself (that crate has no
    /// UI-persistence concern), same pattern as `IndexLocation` above.
    pub bu_head_type: String,
    pub bu_bushing_length: Option<f64>,
    pub bu_ext_cs_dia: Option<f64>,
    pub bu_ext_cs_depth: Option<f64>,
    pub bu_ext_cs_angle: Option<f64>,
    pub bu_lower_chamfer_min: Option<f64>,
    pub bu_lower_chamfer_max: Option<f64>,
    pub bu_lower_chamfer_angle_deg: Option<f64>,
    pub bu_head_chamfer_min: Option<f64>,
    pub bu_head_chamfer_max: Option<f64>,
    pub bu_head_chamfer_angle_deg: Option<f64>,
    pub bu_bore_tol_plus: Option<f64>,
    pub bu_bore_tol_minus: Option<f64>,
    pub bu_interference_tol_plus: Option<f64>,
    pub bu_interference_tol_minus: Option<f64>,
    pub bu_enforcement_enabled: Option<bool>,
    /// `"straight"`/`"countersunk"` - `bushing_solver::geometry::IdType`
    /// persisted as a string, same reasoning as `bu_head_type`.
    pub bu_id_type: String,
    /// `"depth_angle"`/`"dia_angle"`/`"dia_depth"` -
    /// `bushing_solver::countersink::CsMode` persisted as a string, same
    /// reasoning as `bu_head_type`.
    pub bu_cs_mode: String,
    pub bu_cs_dia: Option<f64>,
    pub bu_cs_depth: Option<f64>,
    pub bu_cs_angle: Option<f64>,
    pub bu_cs_dia_tol_plus: Option<f64>,
    pub bu_cs_dia_tol_minus: Option<f64>,
    pub bu_cs_depth_tol_plus: Option<f64>,
    pub bu_cs_depth_tol_minus: Option<f64>,
    pub bu_cs_angle_tol_plus: Option<f64>,
    pub bu_cs_angle_tol_minus: Option<f64>,
    pub bu_ext_cs_mode: String,
    pub bu_ext_cs_dia_tol_plus: Option<f64>,
    pub bu_ext_cs_dia_tol_minus: Option<f64>,
    pub bu_ext_cs_depth_tol_plus: Option<f64>,
    pub bu_ext_cs_depth_tol_minus: Option<f64>,
    pub bu_ext_cs_angle_tol_plus: Option<f64>,
    pub bu_ext_cs_angle_tol_minus: Option<f64>,
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
