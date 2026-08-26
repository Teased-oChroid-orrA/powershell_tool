//! Settings/recent-search persistence across relaunches (epic §22).
//! Hand-rolled config-directory resolution rather than adding a `dirs`/
//! `directories` crate dependency for one lookup - this app's shipped
//! target is win-x64 only (`%APPDATA%`), and the macOS/Linux branches
//! below exist only so local development on this machine also persists
//! settings, not because either is a real deployment target (see
//! CLAUDE.md's "Target environment").

use std::path::PathBuf;

use dioxus::prelude::*;
use search_core::models::{ExcludeScope, GroupByMode, MatchMode};
use serde::{Deserialize, Serialize};

use crate::state::{AppState, ExtensionOption, RecentSearch};

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct PersistedState {
    pub search_path: String,
    // `serde(default)` specifically here (unlike the other fields above,
    // which predate this one) so a config file saved before multi-root
    // search existed still deserializes successfully instead of `load()`
    // silently discarding the whole file (`.ok()` on a hard parse failure)
    // over one new, harmless-to-default field.
    #[serde(default)]
    pub search_paths_extra: Vec<String>,
    pub output_folder: String,
    pub output_name: String,
    pub filters_text: String,
    pub exclude_filters_text: String,
    pub match_mode: Option<MatchMode>,
    pub proximity_lines: Option<i32>,
    pub exclude_scope: Option<ExcludeScope>,
    pub whole_word: bool,
    pub use_regex: bool,
    pub exclude_folders_text: String,
    pub include_hidden: bool,
    pub max_file_size_mb: Option<f64>,
    pub group_by: Option<GroupByMode>,
    pub open_report_when_done: bool,
    pub export_csv: bool,
    pub export_json: bool,
    pub parallel: bool,
    pub throttle_limit: Option<i32>,
    #[serde(default)]
    pub heavy_throttle_limit: Option<i32>,
    pub cache_file_path: String,
    pub dry_run: bool,
    pub pdf_timeout_seconds: Option<i32>,
    pub file_timeout_seconds: Option<i32>,
    pub max_retries: Option<i32>,
    pub index_for_fast_search: bool,
    pub selected_extensions: Vec<String>,
    pub recent_searches: Vec<RecentSearch>,
    // `serde(default)` for the same reason as `search_paths_extra` above -
    // a config file saved before named presets existed shouldn't fail to
    // load entirely over one new, harmless-to-default field.
    //
    // NOTE: `SavedPreset::settings` is ALSO a `PersistedState`, so this
    // field is structurally recursive (a preset's stored snapshot has its
    // own `saved_presets` field). `build_snapshot` always zeroes it on the
    // snapshot handed to `AppState::save_current_as_preset` specifically
    // to prevent that recursion from actually storing data at every
    // level - each preset's OWN nested snapshot always has an empty
    // `saved_presets`, so this can't balloon in size across repeated
    // saves. Only the outer, top-level snapshot `save()` writes to disk
    // carries the real list.
    #[serde(default)]
    pub saved_presets: Vec<crate::state::SavedPreset>,
    pub dark_theme: bool,
}

fn config_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("GSEngineeringTextSearch").join("settings.json"))
}

pub fn load() -> Option<PersistedState> {
    let path = config_path()?;
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

/// Applies a loaded `PersistedState` onto a freshly-constructed
/// `AppState` - called once, right after `AppState::new()`, before the
/// first render. `Option` fields fall back to whatever `AppState::new()`
/// already defaulted them to (so an older settings file missing a field
/// added later just uses the current default for it, rather than
/// zeroing it out).
pub fn apply(state: &mut AppState, persisted: PersistedState) {
    apply_settings_fields(state, &persisted);
    state.recent_searches.set(persisted.recent_searches);
    state.saved_presets.set(persisted.saved_presets);
}

/// Same field-by-field restore as [`apply`], minus `recent_searches` (and
/// `dark_theme`, which `apply` never touches either - handled separately
/// in `main.rs`). Used for named search presets (`AppState::apply_preset`)
/// - applying a saved preset should restore its settings without clobbering
/// the user's actual, currently-accumulating recent-searches list, which a
/// preset is a completely separate, named-and-persisted concept from (a
/// preset is deliberately saved once and only changes when the user
/// re-saves it under that name; recent searches are an automatic MRU of
/// the last 8 runs).
pub fn apply_preset(state: &mut AppState, persisted: &PersistedState) {
    apply_settings_fields(state, persisted);
}

fn apply_settings_fields(state: &mut AppState, persisted: &PersistedState) {
    let persisted = persisted.clone();
    state.search_path.set(persisted.search_path);
    state.search_paths_extra.set(persisted.search_paths_extra);
    state.output_folder.set(persisted.output_folder);
    state.output_name.set(persisted.output_name);
    state.filters_text.set(persisted.filters_text);
    state.exclude_filters_text.set(persisted.exclude_filters_text);
    if let Some(v) = persisted.match_mode {
        state.match_mode.set(v);
    }
    if let Some(v) = persisted.proximity_lines {
        state.proximity_lines.set(v);
    }
    if let Some(v) = persisted.exclude_scope {
        state.exclude_scope.set(v);
    }
    state.whole_word.set(persisted.whole_word);
    state.use_regex.set(persisted.use_regex);
    state.exclude_folders_text.set(persisted.exclude_folders_text);
    state.include_hidden.set(persisted.include_hidden);
    if let Some(v) = persisted.max_file_size_mb {
        state.max_file_size_mb.set(v);
    }
    if let Some(v) = persisted.group_by {
        state.group_by.set(v);
    }
    state.open_report_when_done.set(persisted.open_report_when_done);
    state.export_csv.set(persisted.export_csv);
    state.export_json.set(persisted.export_json);
    state.parallel.set(persisted.parallel);
    if let Some(v) = persisted.throttle_limit {
        state.throttle_limit.set(v);
    }
    if let Some(v) = persisted.heavy_throttle_limit {
        state.heavy_throttle_limit.set(v);
    }
    state.cache_file_path.set(persisted.cache_file_path);
    state.dry_run.set(persisted.dry_run);
    if let Some(v) = persisted.pdf_timeout_seconds {
        state.pdf_timeout_seconds.set(v);
    }
    if let Some(v) = persisted.file_timeout_seconds {
        state.file_timeout_seconds.set(v);
    }
    if let Some(v) = persisted.max_retries {
        state.max_retries.set(v);
    }
    state.index_for_fast_search.set(persisted.index_for_fast_search);

    if !persisted.selected_extensions.is_empty() {
        let mut catalog = state.extension_catalog.write();
        for ext in &persisted.selected_extensions {
            if let Some(existing) = catalog.iter_mut().find(|e| e.extension.eq_ignore_ascii_case(ext)) {
                existing.is_selected = true;
            } else {
                catalog.push(ExtensionOption { extension: ext.clone(), category: "Custom".to_string(), is_selected: true });
            }
        }
    }
}

/// Snapshots the persistable fields of `AppState` and writes them out.
/// Called from a `use_effect` in `App` (main.rs) that reads exactly these
/// signals, so it naturally reruns whenever any of them changes - no
/// separate "mark dirty" bookkeeping needed. Failure is silent (matches
/// this app's established pattern for the incremental search cache and
/// the HTML report - a settings file that can't be written just means
/// next launch starts from defaults again, never a reason to interrupt
/// the user).
pub fn save(state: &AppState, dark_theme: bool) {
    let Some(path) = config_path() else { return };
    let persisted = build_snapshot(state, dark_theme);

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&persisted) {
        let _ = std::fs::write(path, json);
    }
}

/// Builds a full settings snapshot from the live `AppState` - shared by
/// [`save`] (writes it to the cross-relaunch config file) and named search
/// presets (`AppState::save_current_as_preset`, which keeps a named one of
/// these rather than the single always-latest one `save` maintains).
pub fn build_snapshot(state: &AppState, dark_theme: bool) -> PersistedState {
    let selected_extensions: Vec<String> =
        state.extension_catalog.read().iter().filter(|e| e.is_selected).map(|e| e.extension.clone()).collect();

    PersistedState {
        search_path: state.search_path.read().clone(),
        search_paths_extra: state.search_paths_extra.read().clone(),
        output_folder: state.output_folder.read().clone(),
        output_name: state.output_name.read().clone(),
        filters_text: state.filters_text.read().clone(),
        exclude_filters_text: state.exclude_filters_text.read().clone(),
        match_mode: Some(*state.match_mode.read()),
        proximity_lines: Some(*state.proximity_lines.read()),
        exclude_scope: Some(*state.exclude_scope.read()),
        whole_word: *state.whole_word.read(),
        use_regex: *state.use_regex.read(),
        exclude_folders_text: state.exclude_folders_text.read().clone(),
        include_hidden: *state.include_hidden.read(),
        max_file_size_mb: Some(*state.max_file_size_mb.read()),
        group_by: Some(*state.group_by.read()),
        open_report_when_done: *state.open_report_when_done.read(),
        export_csv: *state.export_csv.read(),
        export_json: *state.export_json.read(),
        parallel: *state.parallel.read(),
        throttle_limit: Some(*state.throttle_limit.read()),
        heavy_throttle_limit: Some(*state.heavy_throttle_limit.read()),
        cache_file_path: state.cache_file_path.read().clone(),
        dry_run: *state.dry_run.read(),
        pdf_timeout_seconds: Some(*state.pdf_timeout_seconds.read()),
        file_timeout_seconds: Some(*state.file_timeout_seconds.read()),
        max_retries: Some(*state.max_retries.read()),
        index_for_fast_search: *state.index_for_fast_search.read(),
        selected_extensions,
        recent_searches: state.recent_searches.read().clone(),
        saved_presets: state.saved_presets.read().clone(),
        dark_theme,
    }
}
