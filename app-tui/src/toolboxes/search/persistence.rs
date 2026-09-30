//! Cross-relaunch settings persistence for the Search Files toolbox. Uses
//! `crate::paths::app_data_dir` (shared with every other toolbox's
//! persistence module) - Windows is the only real shipping target (root
//! CLAUDE.md's "Target environment" invariant), so the other branches
//! exist only to make local development also persist settings.
//!
//! File name is its own, distinct from every other toolbox's own settings
//! file under the same shared `"GSEngineeringToolbench"` folder - all
//! coexist without clobbering each other.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::SearchToolConfig;
use crate::paths::app_data_dir;
use search_core::models::{ExcludeScope, GroupByMode, MatchMode};

fn config_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("settings-tui.json"))
}

/// Every `SearchToolConfig` field, `#[serde(default)]`/`Option<T>` for
/// anything that could plausibly gain a new field later - same reasoning
/// `app/`'s `PersistedState` and `app-egui/`'s `SearchFieldsSnap` both
/// document explicitly: an old config file missing a field added since
/// must fall back to the CURRENT CODE default for that field, not to
/// `bool`/`String`'s own blanket zero value (a derived `#[derive(Default)]`
/// would silently do the latter, which for a field like `export_html`
/// would be a real, silent behavior change for existing users - see
/// `app/src/persistence.rs`'s `export_html` field comment for the exact
/// incident this guards against).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSearchSettings {
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

    pub match_mode: Option<MatchMode>,
    pub proximity_lines: Option<i32>,
    #[serde(default)]
    pub use_regex: bool,
    #[serde(default)]
    pub whole_word: bool,
    pub exclude_scope: Option<ExcludeScope>,

    #[serde(default)]
    pub extension_filter_text: String,
    #[serde(default)]
    pub selected_extensions: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_folders_text: String,
    #[serde(default)]
    pub include_hidden: bool,
    pub max_file_size_mb: Option<f64>,
    pub group_by: Option<GroupByMode>,

    // Defaults true on missing/old config files, same as `app/`'s
    // `export_html` - this app always generated the HTML report
    // unconditionally before the toggle existed.
    #[serde(default = "default_true")]
    pub export_html: bool,
    #[serde(default)]
    pub open_report_when_done: bool,
    #[serde(default)]
    pub export_csv: bool,
    #[serde(default)]
    pub export_json: bool,

    #[serde(default)]
    pub parallel: bool,
    pub throttle_limit: Option<i32>,
    #[serde(default)]
    pub heavy_throttle_limit: Option<i32>,
    #[serde(default)]
    pub cache_file_path: String,
    #[serde(default)]
    pub dry_run: bool,
    pub pdf_timeout_seconds: Option<i32>,
    #[serde(default)]
    pub ocr_scanned_pdfs: bool,
    pub file_timeout_seconds: Option<i32>,
    pub max_retries: Option<i32>,

    #[serde(default)]
    pub index_enabled: bool,
    pub index_location: Option<super::indexing::IndexLocation>,
}

fn default_true() -> bool {
    true
}

impl From<&SearchToolConfig> for PersistedSearchSettings {
    fn from(config: &SearchToolConfig) -> Self {
        Self {
            search_path: config.search_path.clone(),
            search_paths_extra: config.search_paths_extra.clone(),
            output_folder: config.output_folder.clone(),
            output_name: config.output_name.clone(),
            filters_text: config.filters_text.clone(),
            exclude_filters_text: config.exclude_filters_text.clone(),
            match_mode: Some(config.match_mode),
            proximity_lines: Some(config.proximity_lines),
            use_regex: config.use_regex,
            whole_word: config.whole_word,
            exclude_scope: Some(config.exclude_scope),
            extension_filter_text: config.extension_filter_text.clone(),
            selected_extensions: config.selected_extensions.clone(),
            exclude_folders_text: config.exclude_folders_text.clone(),
            include_hidden: config.include_hidden,
            max_file_size_mb: Some(config.max_file_size_mb),
            group_by: Some(config.group_by),
            export_html: config.export_html,
            open_report_when_done: config.open_report_when_done,
            export_csv: config.export_csv,
            export_json: config.export_json,
            parallel: config.parallel,
            throttle_limit: Some(config.throttle_limit),
            heavy_throttle_limit: Some(config.heavy_throttle_limit),
            cache_file_path: config.cache_file_path.clone(),
            dry_run: config.dry_run,
            pdf_timeout_seconds: Some(config.pdf_timeout_seconds),
            ocr_scanned_pdfs: config.ocr_scanned_pdfs,
            file_timeout_seconds: Some(config.file_timeout_seconds),
            max_retries: Some(config.max_retries),
            index_enabled: config.index.enabled,
            index_location: Some(config.index.location),
        }
    }
}

impl PersistedSearchSettings {
    /// Applies every field onto an existing `SearchToolConfig`, falling
    /// back to whatever `config` already held (its own `Default`, or a
    /// prior value) for any `Option` field an old config file left `None`
    /// - never falling back to a bare zero value. Mirrors
    /// `app/src/persistence.rs::apply_settings_fields`.
    pub fn apply_to(&self, config: &mut SearchToolConfig) {
        config.search_path = self.search_path.clone();
        config.search_paths_extra = self.search_paths_extra.clone();
        config.output_folder = self.output_folder.clone();
        config.output_name = self.output_name.clone();
        config.filters_text = self.filters_text.clone();
        config.exclude_filters_text = self.exclude_filters_text.clone();
        if let Some(v) = self.match_mode {
            config.match_mode = v;
        }
        if let Some(v) = self.proximity_lines {
            config.proximity_lines = v;
        }
        config.use_regex = self.use_regex;
        config.whole_word = self.whole_word;
        if let Some(v) = self.exclude_scope {
            config.exclude_scope = v;
        }
        config.extension_filter_text = self.extension_filter_text.clone();
        config.selected_extensions = self.selected_extensions.clone();
        config.exclude_folders_text = self.exclude_folders_text.clone();
        config.include_hidden = self.include_hidden;
        if let Some(v) = self.max_file_size_mb {
            config.max_file_size_mb = v;
        }
        if let Some(v) = self.group_by {
            config.group_by = v;
        }
        config.export_html = self.export_html;
        config.open_report_when_done = self.open_report_when_done;
        config.export_csv = self.export_csv;
        config.export_json = self.export_json;
        config.parallel = self.parallel;
        if let Some(v) = self.throttle_limit {
            config.throttle_limit = v;
        }
        if let Some(v) = self.heavy_throttle_limit {
            config.heavy_throttle_limit = v;
        }
        config.cache_file_path = self.cache_file_path.clone();
        config.dry_run = self.dry_run;
        if let Some(v) = self.pdf_timeout_seconds {
            config.pdf_timeout_seconds = v;
        }
        config.ocr_scanned_pdfs = self.ocr_scanned_pdfs;
        if let Some(v) = self.file_timeout_seconds {
            config.file_timeout_seconds = v;
        }
        if let Some(v) = self.max_retries {
            config.max_retries = v;
        }
        config.index.enabled = self.index_enabled;
        if let Some(v) = self.index_location {
            config.index.location = v;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentSearch {
    pub search_path: String,
    pub filters_text: String,
}

impl RecentSearch {
    /// Display label for the recent-searches list / command palette -
    /// folder name (not the full path) plus the filters used, e.g.
    /// "project - apple, banana". Mirrors `app/src/state.rs::RecentSearch::label`.
    pub fn label(&self) -> String {
        let folder_name = Path::new(&self.search_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.search_path.clone());
        format!("{folder_name} - {}", self.filters_text)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPreset {
    pub name: String,
    pub settings: PersistedSearchSettings,
}

/// The single top-level struct actually serialized to disk - settings plus
/// recents/presets in one file, rather than `app/`'s approach of nesting
/// `recent_searches`/`saved_presets` as fields directly on `PersistedState`
/// itself. Same information, flatter to reason about here since this
/// crate has no other top-level settings struct competing for the name.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedFile {
    #[serde(default)]
    pub settings: Option<PersistedSearchSettings>,
    #[serde(default)]
    pub recent_searches: Vec<RecentSearch>,
    #[serde(default)]
    pub saved_presets: Vec<SavedPreset>,
    /// Individually-selected extensions from the extension picker, most-
    /// recently-selected first - independent of `settings.selected_extensions`
    /// (the *current* search's active filter, which can be cleared/changed
    /// per run) and of `recent_searches` above (whole path+filters
    /// snapshots). See [`remember_recent_extensions`].
    #[serde(default)]
    pub recent_extensions: Vec<String>,
}

pub fn load() -> Option<PersistedFile> {
    load_from(&config_path()?)
}

fn load_from(path: &Path) -> Option<PersistedFile> {
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

/// Best-effort, write-to-temp-then-rename - a failed settings save is
/// never a reason to interrupt the user, matching both existing heads'
/// own persistence philosophy. Ports the crash-safety reasoning from
/// `app/src/persistence.rs::save` (temp file + rename, not a direct
/// truncating write, so a crash mid-write can never leave an unparseable
/// settings file behind).
pub fn save(file: &PersistedFile) {
    if let Some(path) = config_path() {
        save_to(&path, file);
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

/// Most-recent-first, deduplicated by (search_path, filters_text), capped
/// at 8. Ported from `app/src/state.rs::remember_recent_search`
/// (state.rs:361-373) as a plain function over `Vec<RecentSearch>` instead
/// of a `Signal`-mutating method.
pub fn remember_recent_search(recents: &mut Vec<RecentSearch>, search_path: String, filters_text: String) {
    let entry = RecentSearch { search_path: search_path.trim().to_string(), filters_text: filters_text.trim().to_string() };
    if entry.search_path.is_empty() || entry.filters_text.is_empty() {
        return;
    }
    recents.retain(|r| *r != entry);
    recents.insert(0, entry);
    recents.truncate(8);
}

/// Most-recent-first, deduplicated case-insensitively, capped at 8 - same
/// convention as [`remember_recent_search`], applied per-extension rather
/// than per-search. Each newly-selected extension (in the order the caller
/// passes them) is moved to the front; already-recent entries not
/// reselected this time keep their relative order behind the new ones.
pub fn remember_recent_extensions(recents: &mut Vec<String>, newly_selected: &[String]) {
    for ext in newly_selected {
        let ext = ext.trim();
        if ext.is_empty() {
            continue;
        }
        recents.retain(|r| !r.eq_ignore_ascii_case(ext));
        recents.insert(0, ext.to_string());
    }
    recents.truncate(8);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn remember_recent_extensions_moves_reselected_entries_to_front() {
        let mut recents = vec![".rs".to_string(), ".txt".to_string()];
        remember_recent_extensions(&mut recents, &[".txt".to_string()]);
        assert_eq!(recents, vec![".txt".to_string(), ".rs".to_string()]);
    }

    #[test]
    fn remember_recent_extensions_dedupes_case_insensitively() {
        let mut recents = vec![".TXT".to_string()];
        remember_recent_extensions(&mut recents, &[".txt".to_string()]);
        assert_eq!(recents, vec![".txt".to_string()]);
    }

    #[test]
    fn remember_recent_extensions_caps_at_eight() {
        let mut recents = Vec::new();
        for i in 0..10 {
            remember_recent_extensions(&mut recents, &[format!(".e{i}")]);
        }
        assert_eq!(recents.len(), 8);
        assert_eq!(recents[0], ".e9");
    }

    #[test]
    fn recent_extensions_field_round_trips_and_defaults_on_old_files() {
        let file = PersistedFile { recent_extensions: vec![".pdf".to_string()], ..Default::default() };
        let json = serde_json::to_string(&file).unwrap();
        let restored: PersistedFile = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.recent_extensions, vec![".pdf".to_string()]);

        // Old file with no `recent_extensions` key at all must not fail to
        // parse - defaults to empty, same discipline as every other field
        // added to this struct after its first release.
        let old_json = r#"{"settings":null,"recent_searches":[],"saved_presets":[]}"#;
        let old: PersistedFile = serde_json::from_str(old_json).unwrap();
        assert!(old.recent_extensions.is_empty());
    }

    #[test]
    fn persisted_settings_round_trip_through_config() {
        let mut config = SearchToolConfig { search_path: "/a/b".to_string(), use_regex: true, ..SearchToolConfig::default() };
        let persisted = PersistedSearchSettings::from(&config);
        let json = serde_json::to_string(&persisted).unwrap();
        let restored: PersistedSearchSettings = serde_json::from_str(&json).unwrap();

        config.search_path.clear();
        restored.apply_to(&mut config);
        assert_eq!(config.search_path, "/a/b");
        assert!(config.use_regex);
    }

    #[test]
    fn old_config_file_missing_export_html_defaults_to_true() {
        // Simulates a config file saved before the `export_html` field
        // existed - must default to `true` (the app's pre-existing
        // unconditional behavior), never to `bool::default()` (`false`).
        let json = r#"{"search_path":"/x"}"#;
        let persisted: PersistedSearchSettings = serde_json::from_str(json).unwrap();
        assert!(persisted.export_html);
        assert_eq!(persisted.search_path, "/x");
        assert_eq!(persisted.match_mode, None);
    }

    #[test]
    fn save_and_load_round_trip_via_temp_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings-tui.json");

        let file = PersistedFile {
            settings: Some(PersistedSearchSettings::from(&SearchToolConfig::default())),
            recent_searches: vec![RecentSearch { search_path: "/a".into(), filters_text: "x".into() }],
            saved_presets: Vec::new(),
            recent_extensions: Vec::new(),
        };
        save_to(&path, &file);
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists(), "temp file must be renamed away, not left behind");

        let loaded = load_from(&path).expect("save_to output must be loadable");
        assert_eq!(loaded.recent_searches.len(), 1);
        assert_eq!(loaded.recent_searches[0].search_path, "/a");
    }

    #[test]
    fn load_from_missing_file_returns_none_not_a_panic() {
        let dir = tempdir().unwrap();
        assert!(load_from(&dir.path().join("does-not-exist.json")).is_none());
    }

    #[test]
    fn remember_recent_search_dedupes_caps_at_eight_and_orders_most_recent_first() {
        let mut recents = Vec::new();
        for i in 0..10 {
            remember_recent_search(&mut recents, format!("/path/{i}"), "f".to_string());
        }
        assert_eq!(recents.len(), 8);
        assert_eq!(recents[0].search_path, "/path/9");

        // Re-adding an existing entry moves it to the front instead of
        // duplicating it.
        remember_recent_search(&mut recents, "/path/5".to_string(), "f".to_string());
        assert_eq!(recents.len(), 8);
        assert_eq!(recents[0].search_path, "/path/5");
    }

    #[test]
    fn remember_recent_search_ignores_empty_path_or_filters() {
        let mut recents = Vec::new();
        remember_recent_search(&mut recents, String::new(), "f".to_string());
        remember_recent_search(&mut recents, "/a".to_string(), "  ".to_string());
        assert!(recents.is_empty());
    }

    #[test]
    fn recent_search_label_uses_folder_name_not_full_path() {
        let recent = RecentSearch { search_path: "/x/y/project".to_string(), filters_text: "apple, banana".to_string() };
        assert_eq!(recent.label(), "project - apple, banana");
    }
}
