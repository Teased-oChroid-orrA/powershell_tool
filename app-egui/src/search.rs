//! Search Files tool - egui port of the core workflow in `app/src/state.rs`
//! (`AppState::run_search`) and `app/src/components.rs` (`SettingsPanel`/
//! `ResultsPanel`).
//!
//! `search-core`'s `orchestrator::run`/`run_candidates` was already
//! framework-agnostic (progress flows over a plain `tokio::sync::mpsc::
//! UnboundedSender<SearchProgressReport>`, never touching a Dioxus
//! `Signal` directly - confirmed by reading it, not assumed) - reused
//! completely unchanged here. Only the *driver* around it is new: Dioxus's
//! `Signal`-per-field state + `self.progress_percent.set(...)` becomes a
//! plain `SearchUiState` behind an `Arc<Mutex<_>>`, written to from a
//! background task on a persistent `tokio::runtime::Runtime` and read once
//! per frame in `update()` - the standard egui pattern for bridging async
//! work into an immediate-mode UI.
//!
//! This pass closes the gap `docs/app-egui-parity-checklist.md` tracked as
//! OPEN: every `SearchSettings` field now has a real control (Matching/
//! Scope-and-output/Performance-and-robustness/Fast-re-search-index
//! sections, mirroring `app/src/components.rs::SettingsPanel`'s own
//! grouping so the two UIs stay comparable during the migration), plus
//! multi-root search, CSV/JSON export, presets, recent searches, and the
//! native-search index-first candidate-query routing `run_search` already
//! does. Every field/helper here is a straight port of an already-tested
//! `app/src/state.rs` counterpart (named in each function's own comment),
//! so this is new UI wiring to logic `search-core` already exercises in
//! its own 149-test suite, not new engine logic.

use std::sync::{Arc, Mutex};

use eframe::egui;
use search_core::matching::CompiledMatchState;
use search_core::models::{
    ExcludeScope, FileSearchResult, FileSearchStatus, GroupByMode, MatchMode, SearchProgressReport, SearchRunResult, SearchSettings,
};
use search_core::orchestrator::{self, OrchestratorError};
use search_core::report;
use tokio_util::sync::CancellationToken;

use crate::design::components::ToastKind;
use crate::persistence::{IndexLocation, RecentSearch, SavedPreset, SearchFieldsSnap};
use crate::theme::Tokens;
use crate::widgets::card;

/// Serializes every index-writing operation (the automatic post-search
/// reindex, the manual Build/Rebuild actions) against the same index
/// directory - mirrors `app/src/state.rs::INDEX_WRITE_LOCK` exactly. Two
/// `IndexWriter`s opened concurrently against the same Tantivy directory
/// is a real, reported crash on Windows ("An index writer was killed...")
/// that Tantivy's own writer lock doesn't reliably prevent there; a
/// single process-wide lock held for the whole open-write-commit sequence
/// is the simplest correct fix regardless of which two operations race.
static INDEX_WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, PartialEq)]
pub struct ExtensionOption {
    pub extension: String,
    pub category: String,
    pub is_selected: bool,
}

fn initial_extension_catalog() -> Vec<ExtensionOption> {
    search_core::models::extension_catalog::CATEGORIES
        .iter()
        .flat_map(|cat| cat.extensions.iter().map(move |ext| ExtensionOption { extension: ext.to_string(), category: cat.category.to_string(), is_selected: false }))
        .collect()
}

/// Mirrors `app/src/state.rs::filtered_extensions` exactly - duplicated
/// (not imported from `app`) for the same reason `persistence.rs`
/// duplicates `PersistedState`: pulling in the `app` crate would drag its
/// dioxus dependency into `app-egui`.
fn filtered_extensions(catalog: &[ExtensionOption], filter_text: &str) -> Vec<ExtensionOption> {
    let needle = filter_text.trim().to_lowercase();
    if needle.is_empty() {
        catalog.to_vec()
    } else {
        catalog.iter().filter(|e| e.extension.to_lowercase().contains(&needle) || e.category.to_lowercase().contains(&needle)).cloned().collect()
    }
}

fn selected_extensions_summary(catalog: &[ExtensionOption]) -> String {
    let selected: Vec<&str> = catalog.iter().filter(|e| e.is_selected).map(|e| e.extension.as_str()).collect();
    if selected.is_empty() {
        "Using built-in default extension list.".to_string()
    } else {
        format!("Searching: {}", selected.join(", "))
    }
}

fn parse_list(text: &str) -> Vec<String> {
    text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Mirrors `Path.GetInvalidFileNameChars()` on Windows (the shipped
/// target - see `CLAUDE.md`): every ASCII control character plus
/// `< > : " / \ | ? *`.
fn sanitize_file_name(name: &str) -> String {
    name.chars().map(|c| if is_invalid_windows_filename_char(c) { '_' } else { c }).collect()
}

fn is_invalid_windows_filename_char(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20
}

/// Folds one root's `SearchRunResult` into the running multi-root total -
/// ported unchanged from `app/src/state.rs::merge_run_result`.
fn merge_run_result(acc: &mut SearchRunResult, mut next: SearchRunResult) {
    acc.file_results.append(&mut next.file_results);
    acc.summary.files_searched += next.summary.files_searched;
    acc.summary.skipped_too_large += next.summary.skipped_too_large;
    acc.summary.skipped_binary += next.summary.skipped_binary;
    acc.summary.skipped_read_error += next.summary.skipped_read_error;
    acc.summary.skipped_by_exclude += next.summary.skipped_by_exclude;
    acc.summary.skipped_by_mode += next.summary.skipped_by_mode;
    acc.summary.skipped_unexpected_error += next.summary.skipped_unexpected_error;
    acc.summary.cache_reused += next.summary.cache_reused;
    acc.summary.enumeration_errors += next.summary.enumeration_errors;
    acc.summary.warnings.append(&mut next.summary.warnings);
    if let Some(mut cands) = next.dry_run_candidates.take() {
        acc.dry_run_candidates.get_or_insert_with(Vec::new).append(&mut cands);
    }
}

/// Opens (auto-rebuilding if needed) `root`'s index and brings it fully up
/// to date via `build_or_update_corpus_index` - the single-root indexing
/// step shared by the automatic post-search reindex (`SearchTool::start`)
/// and the explicit Build/Rebuild buttons. Mirrors `app/src/state.rs::
/// index_one_root`, with an added progress callback the manual-button
/// path uses to show live per-file status (the automatic path passes a
/// no-op).
async fn index_one_root_with_progress(
    settings: &SearchSettings,
    index_dir: &std::path::Path,
    mut on_progress: impl FnMut(search_core::native_index::CorpusIndexProgress) + Send,
) -> native_search::error::NsResult<search_core::native_index::CorpusIndexOutcome> {
    search_core::native_index::ensure_index_directory_exists(index_dir).map_err(|e| native_search::error::NsError::index_error(e.to_string()))?;
    let engine = search_core::native_index::open_or_create_with_rebuild(index_dir)?;
    search_core::native_index::build_or_update_corpus_index_send(settings, &engine, &CancellationToken::new(), Some(&mut on_progress)).await
}

/// `OutputFolder` placement keys each searched directory to its OWN
/// subdirectory under the output folder - NOT one shared index for every
/// root ever pointed at that output folder. A real, confirmed bug this
/// fixes: two completely unrelated searches (different `search_path`,
/// same output folder) used to collide into the same corpus, silently
/// mixing their results together - the same isolation `SearchFolder`
/// placement already gets for free (each root's own directory) has to be
/// reproduced explicitly here since there's only one output folder to
/// place things in. Keyed by a short stable hash of the root path (not
/// the raw path itself - Windows path length/invalid-character limits
/// make a raw path unsafe as a directory name) plus the root's last
/// component for a human-readable prefix, so two DIFFERENT folders that
/// happen to share a name (e.g. two drives' own "Projects" folder) still
/// can't collide.
/// Real recursive on-disk size of an index directory (Tantivy segment
/// files) - used to surface an actual index-size stat rather than
/// leaving the size/search-capability tradeoff (the trigram field's own
/// cost) silent. Best-effort: an unreadable entry just doesn't count
/// toward the total rather than failing the whole build.
fn dir_size_bytes(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .filter_map(|e| e.ok())
        .map(|entry| match entry.file_type() {
            Ok(ft) if ft.is_dir() => dir_size_bytes(&entry.path()),
            Ok(_) => entry.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

fn resolve_index_dir(location: IndexLocation, root: &str, output_folder: &str) -> std::path::PathBuf {
    match location {
        IndexLocation::SearchFolder => search_core::native_index::index_directory(root),
        IndexLocation::OutputFolder => {
            let name = sanitized_root_key(root);
            std::path::Path::new(output_folder.trim()).join(".native-search-index").join(name)
        }
    }
}

fn sanitized_root_key(root: &str) -> String {
    use std::hash::{Hash, Hasher};
    let normalized = root.trim().trim_end_matches(['/', '\\']).to_lowercase();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    let hash = hasher.finish();

    let readable = std::path::Path::new(root.trim())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".to_string());
    let readable: String = readable.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("{readable}-{hash:x}")
}

fn format_relative_age(age: std::time::Duration) -> String {
    let secs = age.as_secs();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{} minute(s) ago", secs / 60)
    } else if secs < 86400 {
        format!("{} hour(s) ago", secs / 3600)
    } else {
        format!("{} day(s) ago", secs / 86400)
    }
}

/// Best-effort desktop toast on completion - opt-in
/// (`desktop_notification_when_done`, defaults OFF) and wrapped in
/// `spawn_blocking` + `catch_unwind`, matching `app/src/state.rs::
/// notify_search_complete` exactly. That function's own doc comment
/// records a real, confirmed-on-Windows crash (~5s after every search
/// completes) from an EARLIER version of this exact app that fired this
/// notification unconditionally on the async task calling it - this
/// crate's own `search.rs` had silently reintroduced that already-fixed
/// bug (unconditional `notify_rust::Notification::...show()` on the
/// runtime task, no opt-in gate, no `spawn_blocking`, no `catch_unwind`)
/// before this pass. Fixed by porting the real fix, not re-deriving one.
fn notify_search_complete(summary: String) {
    tokio::task::spawn_blocking(move || {
        let _ = std::panic::catch_unwind(|| {
            let _ = notify_rust::Notification::new().summary("Search complete - GS Engineering Text Search").body(&summary).show();
        });
    });
}

#[derive(Default)]
pub struct SearchUiState {
    pub is_running: bool,
    pub files_completed: i32,
    pub total_files: i32,
    pub hits_so_far: i32,
    pub status_text: String,
    pub results: Vec<FileSearchResult>,
    pub summary_text: String,
    pub report_path: Option<String>,
    /// Status line for the manual "Build/update index"/"Rebuild from
    /// scratch" actions AND the automatic post-search reindex - mirrors
    /// `app/src/state.rs::index_build_status_text`, folded into this same
    /// shared progress area rather than a second hidden caption for the
    /// same reason that file documents: a real report there once was
    /// "indexing doesn't work" when it was actually succeeding silently
    /// where nobody had the section expanded to see it.
    pub index_build_status_text: String,
    pub is_building_index: bool,
}

pub struct SearchTool {
    // ---- Required ----
    search_path: String,
    search_paths_extra: Vec<String>,
    output_folder: String,
    output_name: String,
    filters_text: String,

    // ---- Matching ----
    match_mode: MatchMode,
    proximity_lines: i32,
    use_regex: bool,
    whole_word: bool,
    exclude_filters_text: String,
    exclude_scope: ExcludeScope,

    // ---- Scope and output ----
    extension_catalog: Vec<ExtensionOption>,
    extension_filter_text: String,
    exclude_folders_text: String,
    include_hidden: bool,
    max_file_size_mb: f64,
    group_by: GroupByMode,
    export_html: bool,
    desktop_notification_when_done: bool,
    open_report_when_done: bool,
    export_csv: bool,
    export_json: bool,

    // ---- Performance and robustness ----
    parallel: bool,
    throttle_limit: i32,
    heavy_throttle_limit: i32,
    cache_file_path: String,
    dry_run: bool,
    pdf_timeout_seconds: i32,
    ocr_scanned_pdfs: bool,
    file_timeout_seconds: i32,
    max_retries: i32,

    // ---- Fast re-search (native_search) ----
    index_for_fast_search: bool,
    index_location: IndexLocation,
    /// A Build/Rebuild click found an index already at the target
    /// location - the action is held here pending an explicit second
    /// confirm, rather than running immediately, per the user's explicit
    /// "warn, then let user confirm" decision. `bool` is `force_rebuild`.
    pending_index_confirm: Option<bool>,

    recent_searches: Vec<RecentSearch>,
    saved_presets: Vec<SavedPreset>,
    preset_name_input: String,

    /// Which results view is showing - a plain display preference (not
    /// persisted; nothing about it changes what was searched for).
    results_view: ResultsView,
    /// Edge-detects the index build's running->done transition (polled
    /// each frame, since the background build reports through
    /// `shared`/`SearchUiState`, not an egui event) so the completion
    /// toast fires exactly once per build, not every frame it stays done.
    index_build_was_running: bool,
    graph: crate::graph::GraphState,
    last_clicked_graph_node: Option<String>,

    shared: Arc<Mutex<SearchUiState>>,
    cancel_token: Option<CancellationToken>,
    runtime: Arc<tokio::runtime::Runtime>,
}

#[derive(Clone, Copy, PartialEq, Default)]
enum ResultsView {
    #[default]
    List,
    BrainMap,
}

impl SearchTool {
    /// Built from `SearchFieldsSnap::default()` (single source of truth
    /// for every field's default - see that impl's doc comment for why a
    /// second, hand-copied default list here would risk drifting from it
    /// exactly the way `#[derive(Default)]` silently did before this
    /// pass), plus the fields a snapshot doesn't carry (extension
    /// catalog, recent/presets, background-task plumbing).
    pub fn new(runtime: Arc<tokio::runtime::Runtime>) -> Self {
        let mut tool = Self {
            search_path: String::new(),
            search_paths_extra: Vec::new(),
            output_folder: String::new(),
            output_name: String::new(),
            filters_text: String::new(),

            match_mode: MatchMode::AnyLine,
            proximity_lines: 5,
            use_regex: false,
            whole_word: false,
            exclude_filters_text: String::new(),
            exclude_scope: ExcludeScope::Line,

            extension_catalog: initial_extension_catalog(),
            extension_filter_text: String::new(),
            exclude_folders_text: String::new(),
            include_hidden: false,
            max_file_size_mb: 50.0,
            group_by: GroupByMode::Created,
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
            pdf_timeout_seconds: 15,
            ocr_scanned_pdfs: false,
            file_timeout_seconds: 30,
            max_retries: 3,

            index_for_fast_search: false,
            index_location: IndexLocation::SearchFolder,
            pending_index_confirm: None,

            recent_searches: Vec::new(),
            saved_presets: Vec::new(),
            preset_name_input: String::new(),

            results_view: ResultsView::default(),
            index_build_was_running: false,
            graph: crate::graph::GraphState::default(),
            last_clicked_graph_node: None,

            shared: Arc::new(Mutex::new(SearchUiState::default())),
            cancel_token: None,
            runtime,
        };
        tool.apply_snapshot(SearchFieldsSnap::default());
        tool
    }

    fn build_selected_extensions(&self) -> Option<Vec<String>> {
        let selected: Vec<String> = self.extension_catalog.iter().filter(|e| e.is_selected).map(|e| e.extension.clone()).collect();
        if selected.is_empty() {
            None
        } else {
            Some(selected)
        }
    }

    fn build_exclude_folders(&self) -> Vec<String> {
        let mut folders = parse_list(&self.exclude_folders_text);
        search_core::native_index::ensure_index_folder_excluded(&mut folders);
        folders
    }

    /// Mirrors `app/src/state.rs::AppState::build_settings` field-for-field.
    fn build_settings(&self) -> SearchSettings {
        let output_name_raw = self.output_name.trim().to_string();
        let cache_path_raw = self.cache_file_path.trim().to_string();
        SearchSettings {
            search_path: self.search_path.trim().to_string(),
            output_folder: self.output_folder.trim().to_string(),
            output_name: if output_name_raw.is_empty() { None } else { Some(sanitize_file_name(&output_name_raw)) },
            filters: parse_list(&self.filters_text),
            exclude_filters: parse_list(&self.exclude_filters_text),
            match_mode: self.match_mode,
            proximity_lines: self.proximity_lines,
            exclude_scope: self.exclude_scope,
            whole_word: self.whole_word,
            use_regex: self.use_regex,
            group_by: self.group_by,
            extensions: self.build_selected_extensions(),
            exclude_folders: self.build_exclude_folders(),
            include_hidden: self.include_hidden,
            max_file_size_mb: self.max_file_size_mb,
            max_embed_lines: 4000,
            pdf_timeout_seconds: self.pdf_timeout_seconds,
            ocr_scanned_pdfs: self.ocr_scanned_pdfs,
            export_csv: self.export_csv,
            export_json: self.export_json,
            open_report_when_done: self.open_report_when_done,
            parallel: self.parallel,
            throttle_limit: self.throttle_limit,
            heavy_throttle_limit: self.heavy_throttle_limit,
            cache_file_path: if cache_path_raw.is_empty() { None } else { Some(cache_path_raw) },
            failure_log_path: None,
            dry_run: self.dry_run,
            max_retries: self.max_retries,
            retry_delay_ms: 250,
            file_timeout_seconds: self.file_timeout_seconds,
        }
    }

    pub fn to_snapshot(&self) -> SearchFieldsSnap {
        SearchFieldsSnap {
            search_path: self.search_path.clone(),
            search_paths_extra: self.search_paths_extra.clone(),
            output_folder: self.output_folder.clone(),
            output_name: self.output_name.clone(),
            filters_text: self.filters_text.clone(),
            exclude_filters_text: self.exclude_filters_text.clone(),
            match_mode: self.match_mode,
            proximity_lines: self.proximity_lines,
            use_regex: self.use_regex,
            whole_word: self.whole_word,
            exclude_scope: self.exclude_scope,
            extension_selected: self.extension_catalog.iter().filter(|e| e.is_selected).map(|e| e.extension.clone()).collect(),
            extension_filter_text: self.extension_filter_text.clone(),
            exclude_folders_text: self.exclude_folders_text.clone(),
            include_hidden: self.include_hidden,
            max_file_size_mb: self.max_file_size_mb,
            group_by: self.group_by,
            export_html: self.export_html,
            desktop_notification_when_done: self.desktop_notification_when_done,
            open_report_when_done: self.open_report_when_done,
            export_csv: self.export_csv,
            export_json: self.export_json,
            parallel: self.parallel,
            throttle_limit: self.throttle_limit,
            heavy_throttle_limit: self.heavy_throttle_limit,
            cache_file_path: self.cache_file_path.clone(),
            dry_run: self.dry_run,
            pdf_timeout_seconds: self.pdf_timeout_seconds,
            ocr_scanned_pdfs: self.ocr_scanned_pdfs,
            file_timeout_seconds: self.file_timeout_seconds,
            max_retries: self.max_retries,
            index_for_fast_search: self.index_for_fast_search,
            index_location: self.index_location,
        }
    }

    /// Applies a full settings snapshot - shared by persistence-load and
    /// preset-apply (`app/src/state.rs::apply_preset`/`persistence::
    /// apply_preset` play the same dual role there).
    pub fn apply_snapshot(&mut self, s: SearchFieldsSnap) {
        self.search_path = s.search_path;
        self.search_paths_extra = s.search_paths_extra;
        self.output_folder = s.output_folder;
        self.output_name = s.output_name;
        self.filters_text = s.filters_text;
        self.exclude_filters_text = s.exclude_filters_text;
        self.match_mode = s.match_mode;
        self.proximity_lines = s.proximity_lines;
        self.use_regex = s.use_regex;
        self.whole_word = s.whole_word;
        self.exclude_scope = s.exclude_scope;
        self.set_selected_extensions(&s.extension_selected);
        self.extension_filter_text = s.extension_filter_text;
        self.exclude_folders_text = s.exclude_folders_text;
        self.include_hidden = s.include_hidden;
        self.max_file_size_mb = s.max_file_size_mb;
        self.group_by = s.group_by;
        self.export_html = s.export_html;
        self.desktop_notification_when_done = s.desktop_notification_when_done;
        self.open_report_when_done = s.open_report_when_done;
        self.export_csv = s.export_csv;
        self.export_json = s.export_json;
        self.parallel = s.parallel;
        self.throttle_limit = s.throttle_limit;
        self.heavy_throttle_limit = s.heavy_throttle_limit;
        self.cache_file_path = s.cache_file_path;
        self.dry_run = s.dry_run;
        self.pdf_timeout_seconds = s.pdf_timeout_seconds;
        self.ocr_scanned_pdfs = s.ocr_scanned_pdfs;
        self.file_timeout_seconds = s.file_timeout_seconds;
        self.max_retries = s.max_retries;
        self.index_for_fast_search = s.index_for_fast_search;
        self.index_location = s.index_location;
        self.repair_never_valid_zeros();
    }

    /// One-time self-heal for settings files written while the
    /// `SearchFieldsSnap::default()` bug (see `persistence.rs`'s doc
    /// comment on that impl) was live: those files have these fields
    /// PRESENT with an explicit `0`, not missing, so the `#[serde(default
    /// = "fn")]` fix does nothing for them - `0` is valid JSON the
    /// deserializer loads faithfully. Every numeric input for these
    /// specific fields already clamps to a positive floor on every
    /// keystroke (`.max(1)`/`.max(0.01)` - see `CLAUDE.md`'s "Design
    /// decisions" section), so `0` is a value the UI itself can never
    /// produce - it's unambiguous evidence of a stale, pre-fix file, safe
    /// to re-default here. `proximity_lines`/`max_retries` are NOT
    /// included: both allow a real, intentional `0` via `.max(0)`.
    fn repair_never_valid_zeros(&mut self) {
        if self.max_file_size_mb <= 0.0 {
            self.max_file_size_mb = 50.0;
        }
        if self.throttle_limit < 1 {
            self.throttle_limit = search_core::models::default_throttle_limit();
        }
        if self.heavy_throttle_limit < 1 {
            self.heavy_throttle_limit = search_core::models::default_heavy_throttle_limit();
        }
        if self.pdf_timeout_seconds < 1 {
            self.pdf_timeout_seconds = 15;
        }
        if self.file_timeout_seconds < 1 {
            self.file_timeout_seconds = 30;
        }
    }

    pub fn set_recent_and_presets(&mut self, recent: Vec<RecentSearch>, presets: Vec<SavedPreset>) {
        self.recent_searches = recent;
        self.saved_presets = presets;
    }
    pub fn recent_searches(&self) -> &[RecentSearch] {
        &self.recent_searches
    }
    pub fn saved_presets(&self) -> &[SavedPreset] {
        &self.saved_presets
    }

    fn set_selected_extensions(&mut self, selected: &[String]) {
        for e in self.extension_catalog.iter_mut() {
            e.is_selected = false;
        }
        for ext in selected {
            if let Some(entry) = self.extension_catalog.iter_mut().find(|e| e.extension.eq_ignore_ascii_case(ext)) {
                entry.is_selected = true;
            } else {
                self.extension_catalog.push(ExtensionOption { extension: ext.clone(), category: "Custom".to_string(), is_selected: true });
            }
        }
    }

    fn add_custom_extension(&mut self) {
        let raw = self.extension_filter_text.trim().to_string();
        if raw.is_empty() {
            return;
        }
        let normalized = if raw.starts_with('.') { raw } else { format!(".{raw}") }.to_lowercase();
        if let Some(existing) = self.extension_catalog.iter_mut().find(|e| e.extension.eq_ignore_ascii_case(&normalized)) {
            existing.is_selected = true;
        } else {
            self.extension_catalog.push(ExtensionOption { extension: normalized, category: "Custom".to_string(), is_selected: true });
        }
        self.extension_filter_text.clear();
    }

    fn clear_selected_extensions(&mut self) {
        for e in self.extension_catalog.iter_mut() {
            e.is_selected = false;
        }
    }

    /// Live regex-filter validation, reusing the exact compile path a real
    /// run takes (`CompiledMatchState::build`) rather than re-implementing
    /// regex validation - mirrors `app/src/state.rs::regex_validation_error`.
    fn regex_validation_error(&self) -> Option<String> {
        if !self.use_regex {
            return None;
        }
        let settings =
            SearchSettings { filters: parse_list(&self.filters_text), exclude_filters: parse_list(&self.exclude_filters_text), use_regex: true, ..Default::default() };
        CompiledMatchState::build(&settings).err().map(|e| e.to_string())
    }

    /// Most-recent-first, deduplicated by (search_path, filters_text),
    /// capped at 8 - mirrors `app/src/state.rs::remember_recent_search`.
    fn remember_recent_search(&mut self) {
        let entry = RecentSearch { search_path: self.search_path.trim().to_string(), filters_text: self.filters_text.trim().to_string() };
        if entry.search_path.is_empty() || entry.filters_text.is_empty() {
            return;
        }
        self.recent_searches.retain(|r| r != &entry);
        self.recent_searches.insert(0, entry);
        self.recent_searches.truncate(8);
    }

    fn apply_recent_search(&mut self, recent: &RecentSearch) {
        self.search_path = recent.search_path.clone();
        self.filters_text = recent.filters_text.clone();
    }

    fn save_current_as_preset(&mut self, name: String) {
        let fields = self.to_snapshot();
        if let Some(existing) = self.saved_presets.iter_mut().find(|p| p.name == name) {
            existing.fields = fields;
        } else {
            self.saved_presets.push(SavedPreset { name, fields });
        }
    }

    fn apply_preset(&mut self, preset: &SavedPreset) {
        self.apply_snapshot(preset.fields.clone());
    }

    fn delete_preset(&mut self, name: &str) {
        self.saved_presets.retain(|p| p.name != name);
    }

    pub fn is_running(&self) -> bool {
        self.shared.lock().unwrap().is_running
    }

    pub fn trigger_run(&mut self) {
        self.start();
    }
    pub fn trigger_cancel(&mut self) {
        if let Some(t) = &self.cancel_token {
            t.cancel();
        }
    }

    fn roots(&self) -> Vec<String> {
        std::iter::once(self.search_path.trim().to_string()).chain(self.search_paths_extra.iter().cloned()).collect()
    }

    /// Where `root`'s fast re-search index lives, per `index_location`.
    /// `SearchFolder` defers entirely to `search_core::native_index`'s own
    /// per-root placement policy (ADR-011). `OutputFolder` is new scope
    /// this pass (per explicit user request): each root gets its OWN
    /// subdirectory under `output_folder/.native-search-index/` (see
    /// `resolve_index_dir`'s free-function doc comment for why NOT one
    /// shared directory - a real bug an earlier version of this had), so
    /// the output folder can hold many distinct indices side by side,
    /// same isolation as `SearchFolder` placement, just relocated.
    /// Deliberately NOT auto-excluded from the search the way the
    /// search-folder placement is (that auto-exclusion exists so the
    /// index doesn't index itself; the output folder is a different
    /// directory from what's being searched, so nothing to exclude).
    fn resolve_index_dir(&self, root: &str) -> std::path::PathBuf {
        resolve_index_dir(self.index_location, root, &self.output_folder)
    }

    /// Cheap, lock-free existence/freshness check for the UI - a plain
    /// filesystem stat on the index directory's own mtime, NOT an engine
    /// open (which always acquires Tantivy's writer lock - see
    /// `INDEX_WRITE_LOCK`'s doc comment - so probing it on every frame
    /// just to show a status line would itself risk contending with a
    /// real build in progress). Reports when the index was last touched,
    /// not a verified "every file is current" claim - that would require
    /// the same skip-if-unchanged walk `Build/update` already does, which
    /// is real work, not a glance.
    fn index_status_at(dir: &std::path::Path) -> Option<String> {
        if !dir.exists() {
            return None;
        }
        let modified = std::fs::metadata(dir).and_then(|m| m.modified()).ok()?;
        let age = std::time::SystemTime::now().duration_since(modified).ok()?;
        Some(format!("found - last built {}", format_relative_age(age)))
    }

    pub fn trigger_build_index(&mut self) {
        self.build_or_rebuild_corpus_index(false);
    }
    pub fn trigger_rebuild_index(&mut self) {
        self.build_or_rebuild_corpus_index(true);
    }

    /// Entry point for the Build/Rebuild buttons - warns and asks for
    /// confirmation first if an index already exists at the TARGET
    /// location (per the user's explicit "warn, then let user confirm"
    /// decision), rather than modifying it immediately. Only the primary
    /// `search_path`'s target is checked here (a multi-root search's
    /// other roots may each have their own existing index under
    /// `SearchFolder` placement - the per-root loop itself doesn't
    /// destroy anything it didn't just create in the same run, per
    /// `rebuilt_dirs`, so this first-root confirmation is enough to
    /// catch the common case without demanding one confirmation per root).
    fn request_build_or_rebuild(&mut self, force_rebuild: bool) {
        let root = self.search_path.trim().to_string();
        let target = self.resolve_index_dir(&root);
        if target.exists() {
            self.pending_index_confirm = Some(force_rebuild);
        } else if force_rebuild {
            self.trigger_rebuild_index();
        } else {
            self.trigger_build_index();
        }
    }

    /// Explicit "Build/update index" (`force_rebuild: false`) / "Rebuild
    /// from scratch" (`force_rebuild: true`) actions - mirrors
    /// `app/src/state.rs::build_or_rebuild_corpus_index` exactly,
    /// including deleting each root's `.native-search-index` folder
    /// entirely before rebuilding (so `open_or_create_with_rebuild`'s
    /// skip-if-unchanged check can't skip anything) and looping over
    /// every root (`search_path` + `search_paths_extra`), not just the
    /// primary one. Decoupled from Run Search entirely, unlike the
    /// automatic post-search reindex `start()` does - see that
    /// function's own comment.
    fn build_or_rebuild_corpus_index(&mut self, force_rebuild: bool) {
        if self.search_path.trim().is_empty() {
            return;
        }
        let base_settings = self.build_settings();
        let roots = self.roots();
        {
            let mut s = self.shared.lock().unwrap();
            s.is_building_index = true;
            s.index_build_status_text = "Starting\u{2026}".to_string();
        }
        let index_location = self.index_location;
        let shared = self.shared.clone();
        self.runtime.spawn(async move {
            let mut total = search_core::native_index::CorpusIndexOutcome::default();
            let mut root_errors: Vec<String> = Vec::new();
            // Defensive, not currently load-bearing: each root now
            // resolves to its own distinct directory under either
            // placement (see `resolve_index_dir`'s doc comment), so this
            // should never actually dedupe anything today - kept so a
            // future placement policy that DOES let two roots share a
            // directory can't silently delete one root's freshly-rebuilt
            // data while processing another root pointed at the same path.
            let mut rebuilt_dirs: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();

            for (i, root) in roots.iter().enumerate() {
                let mut settings = base_settings.clone();
                settings.search_path = root.clone();
                let index_dir = resolve_index_dir(index_location, root, &base_settings.output_folder);

                let _guard = INDEX_WRITE_LOCK.lock().await;
                if force_rebuild && !rebuilt_dirs.contains(&index_dir) && index_dir.exists() {
                    if let Err(e) = std::fs::remove_dir_all(&index_dir) {
                        root_errors.push(format!("{root}: could not remove existing index: {e}"));
                        continue;
                    }
                }
                rebuilt_dirs.insert(index_dir.clone());

                let shared_progress = shared.clone();
                let roots_len = roots.len();
                let result = index_one_root_with_progress(&settings, &index_dir, move |p| {
                    let root_prefix = if roots_len > 1 { format!("[{} of {}] ", i + 1, roots_len) } else { String::new() };
                    shared_progress.lock().unwrap().index_build_status_text =
                        format!("{root_prefix}Indexing {} of {}: {}", p.files_processed + 1, p.total_files.max(1), p.current_file);
                })
                .await;

                match result {
                    Ok(outcome) => {
                        total.indexed_count += outcome.indexed_count;
                        total.skipped_count += outcome.skipped_count;
                        total.failed_count += outcome.failed_count;
                        total.failed_files.extend(outcome.failed_files);
                    }
                    Err(e) => root_errors.push(format!("{root}: {e}")),
                }
            }

            // Real on-disk size, not a guess - the trigram (substring-
            // candidate) field indexes every file a second time at
            // 3-character granularity, which is the actual dominant size
            // driver for a large corpus, not stored text (already never
            // stored). Surfacing this makes that tradeoff visible instead
            // of a silently-growing folder nobody asked to see.
            let index_size_mb: f64 = rebuilt_dirs.iter().map(|d| dir_size_bytes(d)).sum::<u64>() as f64 / (1024.0 * 1024.0);

            let msg = if !root_errors.is_empty() {
                format!("Indexing failed for {} folder(s): {}", root_errors.len(), root_errors.join("; "))
            } else {
                // "Failed" now covers read/extraction failures AND any
                // document/commit that hit an unrecovered writer error
                // (see `native_index.rs`'s own comment) - a real
                // end-of-run summary instead of one opaque error covering
                // every remaining file in the folder, which is what a
                // single unhandled failure used to do here.
                let failed_detail = if total.failed_files.is_empty() {
                    String::new()
                } else {
                    let shown: Vec<&String> = total.failed_files.iter().take(5).collect();
                    let more = total.failed_files.len().saturating_sub(shown.len());
                    format!(
                        " ({}{})",
                        shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("; "),
                        if more > 0 { format!("; +{more} more") } else { String::new() }
                    )
                };
                format!(
                    "Indexed {} file(s){}, {} already up to date{}{}. Index size: {:.1} MB.",
                    total.indexed_count,
                    if roots.len() > 1 { format!(" across {} folder(s)", roots.len()) } else { String::new() },
                    total.skipped_count,
                    if total.failed_count > 0 { format!(", {} failed", total.failed_count) } else { String::new() },
                    failed_detail,
                    index_size_mb
                )
            };
            let mut s = shared.lock().unwrap();
            s.index_build_status_text = msg;
            s.is_building_index = false;
        });
    }

    /// Ported from `app/src/state.rs::run_search`: loops over every root
    /// (the primary `search_path` plus `search_paths_extra`), merging
    /// results across roots, and - when `index_for_fast_search` is on -
    /// routes each root through the native-search trigram candidate query
    /// first (a safe superset filter for every match mode, including
    /// regex mode when a literal requirement can be extracted), falling
    /// back to a full `orchestrator::run` scan whenever the index can't
    /// narrow the file list. Never invents a result with confidence from
    /// an empty/missing index - `None` always means "full scan".
    fn start(&mut self) {
        if self.search_path.trim().is_empty() {
            return;
        }
        self.remember_recent_search();
        let base_settings = self.build_settings();
        let roots: Vec<String> = std::iter::once(base_settings.search_path.clone()).chain(self.search_paths_extra.iter().cloned()).collect();
        let index_for_fast_search = self.index_for_fast_search;
        let index_location = self.index_location;
        let notify_on_done = self.desktop_notification_when_done;
        let export_html = self.export_html;

        let cancellation = CancellationToken::new();
        self.cancel_token = Some(cancellation.clone());
        {
            let mut s = self.shared.lock().unwrap();
            *s = SearchUiState { is_running: true, status_text: "Starting...".to_string(), ..Default::default() };
        }
        let shared = self.shared.clone();
        self.runtime.spawn(async move {
            let mut combined: Option<SearchRunResult> = None;
            let mut cancelled = false;
            let mut hard_error: Option<String> = None;

            for (root_idx, root) in roots.iter().enumerate() {
                if cancellation.is_cancelled() {
                    cancelled = true;
                    break;
                }
                if roots.len() > 1 {
                    shared.lock().unwrap().status_text = format!("Searching folder {} of {}: {root}", root_idx + 1, roots.len());
                }

                let mut run_settings = base_settings.clone();
                run_settings.search_path = root.clone();

                let query_candidates: Option<Vec<String>> = if index_for_fast_search {
                    let index_dir = resolve_index_dir(index_location, root, &base_settings.output_folder);
                    let use_regex = run_settings.use_regex;
                    let filters = run_settings.filters.clone();
                    tokio::task::spawn_blocking(move || -> Option<Vec<String>> {
                        search_core::native_index::ensure_index_directory_exists(&index_dir).ok()?;
                        let engine = search_core::native_index::open_or_create_with_rebuild(&index_dir).ok()?;
                        if engine.num_docs() == 0 {
                            return None;
                        }
                        if use_regex {
                            let chunk_sets: Option<Vec<Vec<String>>> = filters.iter().map(|f| search_core::regex_literals::required_literal_chunks(f)).collect();
                            let chunk_sets = chunk_sets?;
                            engine.trigram_candidate_paths_for_chunk_sets(&chunk_sets).ok().flatten()
                        } else {
                            engine.trigram_candidate_paths(&filters).ok().flatten()
                        }
                    })
                    .await
                    .unwrap_or(None)
                } else {
                    None
                };

                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SearchProgressReport>();
                let run_cancellation = cancellation.clone();
                let spawn_settings = run_settings.clone();
                let join_handle = match query_candidates {
                    Some(candidates) => tokio::spawn(async move { orchestrator::run_candidates(&candidates, spawn_settings, Some(tx), run_cancellation).await }),
                    None => tokio::spawn(async move { orchestrator::run(spawn_settings, Some(tx), run_cancellation).await }),
                };

                while let Some(report) = rx.recv().await {
                    apply_progress(&mut shared.lock().unwrap(), report);
                }

                match join_handle.await {
                    Ok(Ok(result)) => match &mut combined {
                        None => combined = Some(result),
                        Some(acc) => merge_run_result(acc, result),
                    },
                    Ok(Err(OrchestratorError::Cancelled)) => {
                        cancelled = true;
                        break;
                    }
                    Ok(Err(e)) => {
                        hard_error = Some(e.to_string());
                        break;
                    }
                    Err(join_err) => {
                        hard_error = Some(join_err.to_string());
                        break;
                    }
                }
            }

            if let Some(err) = hard_error {
                shared.lock().unwrap().status_text = format!("Error: {err}");
            } else if cancelled {
                shared.lock().unwrap().status_text = "Cancelled.".to_string();
            } else if let Some(result) = combined {
                {
                    let mut s = shared.lock().unwrap();
                    finish(&mut s, &base_settings, export_html, result);
                }

                // Keeps the fast-re-search index current after every
                // completed search, same as `app/src/state.rs::
                // finish_successful_run` - indexes the WHOLE corpus (every
                // extension-matching file under every root), not just this
                // run's hit files, since the trigram candidate-filter's
                // safe-superset guarantee only holds if the index actually
                // covers every file that could match, not just files a
                // past search happened to prove were hits.
                // `build_or_update_corpus_index` is skip-if-unchanged per
                // file, so this stays cheap once every root is current.
                if index_for_fast_search {
                    let mut total = search_core::native_index::CorpusIndexOutcome::default();
                    let mut root_errors: Vec<String> = Vec::new();
                    for root in &roots {
                        let mut root_settings = base_settings.clone();
                        root_settings.search_path = root.clone();
                        let index_dir = resolve_index_dir(index_location, root, &base_settings.output_folder);
                        let _guard = INDEX_WRITE_LOCK.lock().await;
                        match index_one_root_with_progress(&root_settings, &index_dir, |_| {}).await {
                            Ok(outcome) => {
                                total.indexed_count += outcome.indexed_count;
                                total.skipped_count += outcome.skipped_count;
                                total.failed_count += outcome.failed_count;
                            }
                            Err(e) => root_errors.push(format!("{root}: {e}")),
                        }
                    }
                    let msg = if !root_errors.is_empty() {
                        format!("Fast re-search indexing failed for {} folder(s): {}", root_errors.len(), root_errors.join("; "))
                    } else {
                        format!(
                            "Indexed {} file(s){}, {} already up to date{}.",
                            total.indexed_count,
                            if roots.len() > 1 { format!(" across {} folder(s)", roots.len()) } else { String::new() },
                            total.skipped_count,
                            if total.failed_count > 0 { format!(", {} failed to extract", total.failed_count) } else { String::new() }
                        )
                    };
                    shared.lock().unwrap().index_build_status_text = msg;
                }

                let summary_for_notify = {
                    let s = shared.lock().unwrap();
                    notify_on_done.then(|| s.summary_text.clone())
                };
                if let Some(summary) = summary_for_notify {
                    notify_search_complete(summary);
                }
            }
            shared.lock().unwrap().is_running = false;
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, tokens: &Tokens, toasts: &mut crate::design::components::ToastQueue) {
        let is_running = self.shared.lock().unwrap().is_running;

        // Native dropped-file handling - `eframe` (via `winit`) already
        // surfaces this on `RawInput` (`ctx.input(|i| &i.raw.dropped_files)`)
        // with no hand-rolled interception needed.
        let dropped: Vec<_> = ui.ctx().input(|i| i.raw.dropped_files.clone());
        if let Some(f) = dropped.first() {
            if let Some(path) = &f.path {
                if path.is_dir() {
                    self.search_path = path.display().to_string();
                } else if let Some(parent) = path.parent() {
                    self.search_path = parent.display().to_string();
                }
            }
        }

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(300.0);
                egui::ScrollArea::vertical().id_salt("search_settings_scroll").show(ui, |ui| {
                    self.settings_column(ui, tokens, is_running, toasts);
                });
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width());
                self.results_column(ui, tokens);
            });
        });
    }

    fn settings_column(&mut self, ui: &mut egui::Ui, tokens: &Tokens, is_running: bool, toasts: &mut crate::design::components::ToastQueue) {
        card(ui, tokens, |ui| {
            crate::widgets::card_title(ui, "Required");
            ui.add_space(4.0);
            ui.colored_label(tokens.fg_muted, egui::RichText::new("Search folder").size(11.5));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search_path).desired_width(190.0));
                if ui.add_enabled(!is_running, egui::Button::new("Browse\u{2026}")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.search_path = path.display().to_string();
                    }
                }
            });

            if !self.search_paths_extra.is_empty() {
                ui.add_space(4.0);
                ui.colored_label(tokens.fg_muted, "Additional folders");
                let mut to_remove = None;
                for path in &self.search_paths_extra {
                    if ui.add_enabled(!is_running, egui::Button::new(format!("{path} \u{d7}"))).clicked() {
                        to_remove = Some(path.clone());
                    }
                }
                if let Some(p) = to_remove {
                    self.search_paths_extra.retain(|x| x != &p);
                }
            }
            if ui.add_enabled(!is_running, egui::Button::new("+ Add another folder")).clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    let path = path.display().to_string();
                    if path != self.search_path.trim() && !self.search_paths_extra.iter().any(|p| p == &path) {
                        self.search_paths_extra.push(path);
                    }
                }
            }

            ui.add_space(6.0);
            ui.colored_label(tokens.fg_muted, egui::RichText::new("Output folder").size(11.5));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.output_folder).desired_width(190.0));
                if ui.add_enabled(!is_running, egui::Button::new("Browse\u{2026}")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.output_folder = path.display().to_string();
                    }
                }
            });

            ui.add_space(6.0);
            ui.colored_label(tokens.fg_muted, egui::RichText::new("Filters (comma-separated)").size(11.5));
            ui.add(egui::TextEdit::singleline(&mut self.filters_text).desired_width(f32::INFINITY).hint_text("e.g. invoice, overdue"));

            if !self.recent_searches.is_empty() {
                ui.add_space(6.0);
                ui.colored_label(tokens.fg_muted, "Recent");
                let mut apply: Option<RecentSearch> = None;
                for recent in self.recent_searches.clone() {
                    if ui.button(recent.label()).on_hover_text(&recent.search_path).clicked() {
                        apply = Some(recent);
                    }
                }
                if let Some(r) = apply {
                    self.apply_recent_search(&r);
                }
            }

            ui.add_space(6.0);
            ui.colored_label(tokens.fg_muted, "Presets");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.preset_name_input).hint_text("Preset name\u{2026}"));
                if ui.add_enabled(!self.preset_name_input.trim().is_empty(), egui::Button::new("Save current")).clicked() {
                    let name = self.preset_name_input.trim().to_string();
                    self.save_current_as_preset(name);
                    self.preset_name_input.clear();
                }
            });
            if !self.saved_presets.is_empty() {
                let names: Vec<String> = self.saved_presets.iter().map(|p| p.name.clone()).collect();
                let mut apply_idx = None;
                let mut delete_name: Option<String> = None;
                for name in &names {
                    ui.horizontal(|ui| {
                        if ui.button(name).clicked() {
                            apply_idx = Some(name.clone());
                        }
                        if ui.small_button("\u{d7}").on_hover_text("Delete this preset").clicked() {
                            delete_name = Some(name.clone());
                        }
                    });
                }
                if let Some(name) = apply_idx {
                    if let Some(preset) = self.saved_presets.iter().find(|p| p.name == name).cloned() {
                        self.apply_preset(&preset);
                    }
                }
                if let Some(name) = delete_name {
                    self.delete_preset(&name);
                }
            }
        });
        ui.add_space(10.0);

        egui::CollapsingHeader::new("Matching").show(ui, |ui| {
            ui.colored_label(tokens.fg_muted, egui::RichText::new("Match mode").font(crate::design::typography::label()));
            ui.add_space(3.0);
            crate::design::components::segmented(
                ui,
                tokens,
                &mut self.match_mode,
                &[(MatchMode::AnyLine, "Any line"), (MatchMode::AllInFile, "All in file"), (MatchMode::Proximity, "Proximity")],
            );
            ui.add_space(7.0);
            if self.match_mode == MatchMode::Proximity {
                ui.horizontal(|ui| {
                    ui.label("Proximity lines");
                    let mut text = self.proximity_lines.to_string();
                    if ui.text_edit_singleline(&mut text).changed() {
                        if let Ok(v) = text.parse::<i32>() {
                            self.proximity_lines = v.max(0);
                        }
                    }
                });
            }
            ui.checkbox(&mut self.use_regex, "Use regex");
            if let Some(err) = self.regex_validation_error() {
                ui.colored_label(tokens.danger, err);
            }
            // Whole-word mode requires regex mode OFF - `matching.rs`'s
            // `is_hit` checks `use_regex` first and never even looks at
            // `whole_word` when it's on, so a checked-but-regex-active box
            // would silently do nothing. Hidden, not just disabled.
            if !self.use_regex {
                ui.checkbox(&mut self.whole_word, "Whole word matching");
            }
            ui.label("Exclude filters (comma-separated)");
            ui.text_edit_singleline(&mut self.exclude_filters_text);
            if !self.exclude_filters_text.trim().is_empty() {
                ui.colored_label(tokens.fg_muted, egui::RichText::new("Exclude scope").font(crate::design::typography::label()));
                ui.add_space(3.0);
                crate::design::components::segmented(ui, tokens, &mut self.exclude_scope, &[(ExcludeScope::Line, "Line"), (ExcludeScope::File, "File")]);
            }
        });

        egui::CollapsingHeader::new("Scope and output").show(ui, |ui| {
            ui.label("File extensions - type to filter, tick to select");
            ui.text_edit_singleline(&mut self.extension_filter_text).on_hover_text("e.g. doc, py, log...");
            let filtered = filtered_extensions(&self.extension_catalog, &self.extension_filter_text);
            egui::ScrollArea::vertical().max_height(160.0).id_salt("ext_catalog_scroll").show(ui, |ui| {
                for opt in &filtered {
                    let mut checked = opt.is_selected;
                    if ui.checkbox(&mut checked, format!("{} ({})", opt.extension, opt.category)).changed() {
                        if let Some(entry) = self.extension_catalog.iter_mut().find(|e| e.extension == opt.extension) {
                            entry.is_selected = checked;
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.extension_filter_text.trim().is_empty(), egui::Button::new("Add as custom extension")).clicked() {
                    self.add_custom_extension();
                }
                if ui.button("Clear selection").clicked() {
                    self.clear_selected_extensions();
                }
            });
            ui.colored_label(tokens.fg_subtle, selected_extensions_summary(&self.extension_catalog));

            ui.add_space(6.0);
            ui.label("Exclude folders (comma-separated)");
            ui.text_edit_singleline(&mut self.exclude_folders_text);
            ui.checkbox(&mut self.include_hidden, "Include hidden files");
            ui.horizontal(|ui| {
                ui.label("Max file size (MB)");
                let mut text = format!("{:.2}", self.max_file_size_mb);
                if ui.text_edit_singleline(&mut text).changed() {
                    if let Ok(v) = text.parse::<f64>() {
                        self.max_file_size_mb = v.max(0.01);
                    }
                }
            });
            ui.colored_label(tokens.fg_muted, egui::RichText::new("Group by").font(crate::design::typography::label()));
            ui.add_space(3.0);
            crate::design::components::segmented(
                ui,
                tokens,
                &mut self.group_by,
                &[(GroupByMode::Created, "Created"), (GroupByMode::Modified, "Modified"), (GroupByMode::None, "None")],
            );
            ui.add_space(7.0);
            ui.checkbox(&mut self.export_html, "Generate HTML report");
            ui.checkbox(&mut self.open_report_when_done, "Open report when done");
            ui.checkbox(&mut self.desktop_notification_when_done, "Desktop notification when done (may be unstable on some Windows setups)");
            ui.checkbox(&mut self.export_csv, "Export CSV");
            ui.checkbox(&mut self.export_json, "Export JSON");
        });

        egui::CollapsingHeader::new("Performance and robustness").show(ui, |ui| {
            ui.checkbox(&mut self.parallel, "Parallel processing");
            if self.parallel {
                ui.horizontal(|ui| {
                    ui.label("Throttle limit (light files)");
                    let mut text = self.throttle_limit.to_string();
                    if ui.text_edit_singleline(&mut text).changed() {
                        if let Ok(v) = text.parse::<i32>() {
                            self.throttle_limit = v.max(1);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Throttle limit (heavy files: PDF/DOCX/PPTX/XLSX/ZIP)");
                    let mut text = self.heavy_throttle_limit.to_string();
                    if ui.text_edit_singleline(&mut text).changed() {
                        if let Ok(v) = text.parse::<i32>() {
                            self.heavy_throttle_limit = v.max(1);
                        }
                    }
                });
            }
            ui.label("Cache file (blank = disabled)");
            ui.text_edit_singleline(&mut self.cache_file_path);
            ui.checkbox(&mut self.dry_run, "Dry run (list files only)");
            ui.horizontal(|ui| {
                ui.label("PDF extraction timeout (s)");
                let mut text = self.pdf_timeout_seconds.to_string();
                if ui.text_edit_singleline(&mut text).changed() {
                    if let Ok(v) = text.parse::<i32>() {
                        self.pdf_timeout_seconds = v.max(1);
                    }
                }
            });
            ui.checkbox(&mut self.ocr_scanned_pdfs, "OCR image-only/scanned PDFs (slower)");
            ui.horizontal(|ui| {
                ui.label("Per-file read timeout (s)");
                let mut text = self.file_timeout_seconds.to_string();
                if ui.text_edit_singleline(&mut text).changed() {
                    if let Ok(v) = text.parse::<i32>() {
                        self.file_timeout_seconds = v.max(1);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Max retries (locked files)");
                let mut text = self.max_retries.to_string();
                if ui.text_edit_singleline(&mut text).changed() {
                    if let Ok(v) = text.parse::<i32>() {
                        self.max_retries = v.max(0);
                    }
                }
            });
        });

        egui::CollapsingHeader::new("Fast re-search index").show(ui, |ui| {
            ui.checkbox(&mut self.index_for_fast_search, "Index this folder for fast re-search");
            if self.index_for_fast_search {
                let is_building = self.shared.lock().unwrap().is_building_index;
                let has_path = !self.search_path.trim().is_empty();
                let root = self.search_path.trim().to_string();

                // Design System Epic Phase 2 "Toast" component's one real
                // wired usage - fires exactly once on the build's
                // running->done edge (`index_build_was_running`), not
                // every frame the index happens to sit idle-done.
                if self.index_build_was_running && !is_building {
                    let status = self.shared.lock().unwrap().index_build_status_text.clone();
                    toasts.push(ToastKind::Success, if status.is_empty() { "Index build complete.".to_string() } else { status });
                }
                self.index_build_was_running = is_building;

                crate::design::components::select_field(
                    ui,
                    tokens,
                    "Index location",
                    match self.index_location {
                        IndexLocation::SearchFolder => "Search folder (default)",
                        IndexLocation::OutputFolder => "Output folder",
                    },
                    |ui| {
                        ui.selectable_value(&mut self.index_location, IndexLocation::SearchFolder, "Search folder (default)");
                        ui.selectable_value(&mut self.index_location, IndexLocation::OutputFolder, "Output folder");
                    },
                );
                if self.index_location == IndexLocation::OutputFolder {
                    ui.colored_label(tokens.fg_subtle, "Each searched folder gets its own index under the output folder - safe to point multiple, unrelated searches at the same output location.");
                }

                // Detected at BOTH locations regardless of which one is
                // selected, per explicit instruction - so switching
                // locations later doesn't surprise the user with an
                // index they didn't know was already sitting there.
                if has_path {
                    let search_folder_dir = resolve_index_dir(IndexLocation::SearchFolder, &root, &self.output_folder);
                    let output_folder_dir = resolve_index_dir(IndexLocation::OutputFolder, &root, &self.output_folder);
                    for (label, dir) in [("Search folder", &search_folder_dir), ("Output folder", &output_folder_dir)] {
                        match Self::index_status_at(dir) {
                            Some(status) => ui.colored_label(tokens.fg_muted, format!("{label}: {status}")),
                            None => ui.colored_label(tokens.fg_subtle, format!("{label}: no index found")),
                        };
                    }
                }

                if let Some(force_rebuild) = self.pending_index_confirm {
                    let action = if force_rebuild { "Rebuild from scratch (deletes the existing index first)" } else { "Build/update" };
                    ui.colored_label(tokens.warning, format!("An index already exists at the target location. {action} anyway?"));
                    ui.horizontal(|ui| {
                        if ui.button("Yes, proceed").clicked() {
                            self.pending_index_confirm = None;
                            if force_rebuild {
                                self.trigger_rebuild_index();
                            } else {
                                self.trigger_build_index();
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_index_confirm = None;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(has_path && !is_building, egui::Button::new(if is_building { "Indexing\u{2026}" } else { "Build/update index" })).clicked() {
                            self.request_build_or_rebuild(false);
                        }
                        let rebuild_resp = crate::design::components::button(ui, tokens, crate::design::components::ButtonVariant::Danger, "Rebuild from scratch", has_path && !is_building);
                        if crate::design::components::tooltip(rebuild_resp, tokens, "Delete and fully rebuild the index from scratch - use if results from the fast index look wrong or stale").clicked() {
                            self.request_build_or_rebuild(true);
                        }
                    });
                }
                // Live/final index-build progress lives ONLY in the
                // Timeline now (results_column, via
                // `index_build_timeline_stage`) - showing it here too was
                // the exact duplication a user reported (progress
                // appearing under this dropdown instead of the Timeline).
                ui.colored_label(tokens.fg_subtle, "Run Search (left) uses this index automatically for non-regex filters, narrowing to candidate files before the real line-by-line scan, and keeps it current after every completed search. Progress shows in the Timeline above the results.");
            } else {
                ui.colored_label(tokens.fg_subtle, "Enable indexing above, then click \u{201c}Build/update index\u{201d} or run a search - either keeps this folder's index current.");
            }
        });

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let run_width = ui.available_width() * 0.6;
            if crate::design::components::button_sized(ui, tokens, crate::design::components::ButtonVariant::Primary, "\u{25B6} Run search", !is_running, egui::vec2(run_width, 0.0)).clicked() {
                self.start();
            }
            if is_running && ui.button("Cancel").clicked() {
                if let Some(t) = &self.cancel_token {
                    t.cancel();
                }
            }
        });
    }

    fn results_column(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let s = self.shared.lock().unwrap();
        let index_stage = index_build_timeline_stage(&s);
        let has_progress_content = s.is_running || s.total_files > 0 || !s.summary_text.is_empty() || s.report_path.is_some() || index_stage.is_some();
        if has_progress_content {
            card(ui, tokens, |ui| {
                // Index-build progress used to live ONLY inside the "Fast
                // re-search index" section (a real, user-reported UX bug -
                // the artifact's own "Search progress = Timeline" banner
                // names this component by name, and indexing progress
                // never touched it). Prepended here instead of a second,
                // separate progress widget, so there's exactly one place
                // any kind of progress ever shows up.
                let mut stages = Vec::new();
                if let Some(stage) = index_stage {
                    stages.push(stage);
                }
                stages.extend(search_timeline_stages(&s));
                timeline(ui, tokens, &stages);
                if let Some(path) = s.report_path.clone() {
                    ui.add_space(6.0);
                    if ui.button("\u{1F4C4} Open HTML report").clicked() {
                        let _ = open::that(&path);
                    }
                }
            });
            ui.add_space(10.0);
        }
        card(ui, tokens, |ui| {
            ui.horizontal(|ui| {
                crate::widgets::card_title(ui, "Results");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    crate::design::components::segmented(ui, tokens, &mut self.results_view, &[(ResultsView::BrainMap, "Brain Map"), (ResultsView::List, "List")]);
                });
            });
            if s.results.is_empty() {
                crate::design::components::empty_state(ui, tokens, "\u{1F50D}", "No results yet", "Set a search folder and run a search to see matches here.");
            } else {
                match self.results_view {
                    ResultsView::List => {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for r in s.results.iter() {
                                result_row(ui, tokens, r);
                            }
                        });
                    }
                    ResultsView::BrainMap => {
                        // Real bipartite graph (file <-> matched filter),
                        // not decoration - see `graph.rs`'s module doc.
                        // Click syncs back into the list by scrolling
                        // there isn't a stable per-row id to target
                        // without restructuring `result_row`, so for now
                        // this surfaces the clicked path as a status
                        // caption instead of a silent no-op - a real,
                        // disclosed interim behavior, not a fake control.
                        if let Some(path) = crate::graph::brain_map(ui, tokens, &s.results, &mut self.graph, 420.0) {
                            self.last_clicked_graph_node = Some(path);
                        }
                        if let Some(path) = &self.last_clicked_graph_node {
                            ui.add_space(4.0);
                            ui.colored_label(tokens.fg_muted, format!("Selected: {path}"));
                        }
                    }
                }
            }
        });
    }
}

/// Icon-badge result row, matching the mockup's `.result-row`
/// (`.result-icon` + name/meta + a right-aligned match count).
fn result_row(ui: &mut egui::Ui, tokens: &Tokens, r: &FileSearchResult) {
    ui.horizontal(|ui| {
        let ext = std::path::Path::new(&r.full_name).extension().and_then(|e| e.to_str()).unwrap_or("").to_uppercase();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 6.0, tokens.bg_sunken);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, ext, egui::FontId::proportional(8.5), tokens.fg_subtle);
        ui.vertical(|ui| {
            ui.strong(file_name(&r.full_name));
            ui.colored_label(tokens.fg_subtle, &r.full_name);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.colored_label(tokens.fg_muted, egui::RichText::new(r.hits.len().to_string()).monospace());
        });
    });
    ui.separator();
}

fn file_name(full_path: &str) -> &str {
    full_path.rsplit(['/', '\\']).next().unwrap_or(full_path)
}

enum StageState {
    Done,
    Active,
    Pending,
}

struct TimelineStage {
    state: StageState,
    title: String,
    sub: String,
}

/// Progress timeline - matches the approved artifact's `.p3-row`/`.p3-dot`/
/// `.p3-line` design exactly (fetched CSS: a connecting vertical line
/// behind numbered/checked dots, each with a bold title + muted
/// subtitle), the design this project's own banner names by name
/// ("Search progress = Timeline") - the flat "X / Y files" + progress-bar
/// row this replaced didn't match it at all. Two REAL stages (Scan, then
/// Search), not the mockup's illustrative three ("Folder scanned" /
/// "Index built" / "Searching") - this app's fast re-search index, when
/// on, narrows candidates as part of the search stage itself, it isn't a
/// separate phase every run goes through, so a fake third stage would
/// claim something not actually true of this pipeline. Manually painted
/// (one `allocate_exact_size` + `painter_at`, same as `ladder`/`tile`),
/// not built from sequential `ui.horizontal`/`with_layout` widget calls -
/// that pattern already produced two separate real, screenshot-confirmed
/// overlap bugs elsewhere in this crate (see `components.rs::tile`'s and
/// `ladder`'s doc comments).
fn timeline(ui: &mut egui::Ui, tokens: &Tokens, stages: &[TimelineStage]) {
    let row_h = 40.0;
    let dot_r = 9.0;
    let width = ui.available_width();
    let height = row_h * stages.len() as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let dot_x = rect.left() + dot_r;

    if stages.len() > 1 {
        let y0 = rect.top() + row_h / 2.0 + dot_r;
        let y1 = rect.top() + row_h * (stages.len() - 1) as f32 + row_h / 2.0 - dot_r;
        painter.line_segment([egui::pos2(dot_x, y0), egui::pos2(dot_x, y1)], egui::Stroke::new(2.0, tokens.border));
    }

    for (i, stage) in stages.iter().enumerate() {
        let cy = rect.top() + row_h * i as f32 + row_h / 2.0;
        let (bg, fg, glyph) = match stage.state {
            StageState::Done => (tokens.good_bg, tokens.good, "\u{2714}"),
            StageState::Active => (tokens.accent.gamma_multiply(0.18), tokens.accent_strong, "\u{25CF}"),
            StageState::Pending => (tokens.bg_sunken, tokens.fg_subtle, ""),
        };
        painter.circle_filled(egui::pos2(dot_x, cy), dot_r, bg);
        if matches!(stage.state, StageState::Active) {
            painter.circle_stroke(egui::pos2(dot_x, cy), dot_r, egui::Stroke::new(2.0, tokens.accent));
        }
        if !glyph.is_empty() {
            painter.text(egui::pos2(dot_x, cy), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(9.0), fg);
        }
        let text_x = dot_x + dot_r + 10.0;
        let title_color = if matches!(stage.state, StageState::Pending) { tokens.fg_subtle } else { tokens.fg };
        painter.text(egui::pos2(text_x, cy - 8.0), egui::Align2::LEFT_CENTER, &stage.title, egui::FontId::proportional(12.5), title_color);
        if !stage.sub.is_empty() {
            painter.text(egui::pos2(text_x, cy + 8.0), egui::Align2::LEFT_CENTER, &stage.sub, egui::FontId::proportional(11.0), tokens.fg_subtle);
        }
    }
}

/// `None` when no index build has happened yet this session - the
/// Timeline shouldn't grow a permanent extra row for a feature that was
/// never used. `Active` while `is_building_index`, `Done` afterward
/// (carrying the same end-of-run summary text
/// `build_or_rebuild_corpus_index` already writes to
/// `index_build_status_text` - not a separate message).
fn index_build_timeline_stage(s: &SearchUiState) -> Option<TimelineStage> {
    if s.is_building_index {
        Some(TimelineStage { state: StageState::Active, title: "Building index\u{2026}".to_string(), sub: s.index_build_status_text.clone() })
    } else if !s.index_build_status_text.is_empty() {
        Some(TimelineStage { state: StageState::Done, title: "Index updated".to_string(), sub: s.index_build_status_text.clone() })
    } else {
        None
    }
}

fn search_timeline_stages(s: &SearchUiState) -> Vec<TimelineStage> {
    let scan_done = s.total_files > 0 || (!s.is_running && !s.summary_text.is_empty());
    let scan = if scan_done {
        TimelineStage { state: StageState::Done, title: "Folder scanned".to_string(), sub: format!("{} file(s) found", s.total_files) }
    } else if s.is_running {
        TimelineStage { state: StageState::Active, title: "Scanning folder\u{2026}".to_string(), sub: s.status_text.clone() }
    } else {
        TimelineStage { state: StageState::Pending, title: "Scan folder".to_string(), sub: String::new() }
    };

    let search_done = !s.is_running && !s.summary_text.is_empty();
    let search = if search_done {
        TimelineStage { state: StageState::Done, title: "Search complete".to_string(), sub: s.summary_text.clone() }
    } else if s.is_running && scan_done {
        TimelineStage {
            state: StageState::Active,
            title: "Searching\u{2026}".to_string(),
            sub: format!("{} / {} files \u{b7} {} hits so far", s.files_completed, s.total_files, s.hits_so_far),
        }
    } else if s.is_running {
        TimelineStage { state: StageState::Pending, title: "Searching\u{2026}".to_string(), sub: String::new() }
    } else {
        TimelineStage { state: StageState::Pending, title: s.status_text.clone(), sub: String::new() }
    };

    vec![scan, search]
}

fn apply_progress(s: &mut SearchUiState, report: SearchProgressReport) {
    s.files_completed = report.files_completed;
    s.total_files = report.total_files;
    s.hits_so_far = report.hits_so_far;
    if report.is_enumerating {
        s.status_text = "Enumerating files\u{2026}".to_string();
    } else if let Some(name) = &report.current_file_name {
        s.status_text = format!("Processing {name}\u{2026}");
    }
}

fn finish(s: &mut SearchUiState, settings: &SearchSettings, export_html: bool, result: SearchRunResult) {
    if result.was_dry_run {
        let count = result.dry_run_candidates.as_ref().map(|c| c.len()).unwrap_or(0);
        s.status_text = format!("Dry run: {count} file(s) would be searched. Nothing was read or written.");
        return;
    }

    if export_html {
        let report_name = format!("SearchResults_{}.html", chrono::Local::now().format("%Y%m%d_%H%M%S"));
        let report_path = std::path::Path::new(&settings.output_folder).join(&report_name);
        match report::write_html_report(&report_path.display().to_string(), settings, &result) {
            Ok(_) => {
                s.report_path = Some(report_path.display().to_string());
                if settings.open_report_when_done {
                    let _ = open::that(&report_path);
                }
            }
            Err(e) => s.status_text = format!("Report write failed: {e}"),
        }
    }

    let hits: Vec<FileSearchResult> = result.file_results.into_iter().filter(|r| r.status == FileSearchStatus::Hit).collect();
    let total_hits: i32 = hits.iter().map(|r| r.hits.len() as i32).sum();
    s.summary_text = format!("Searched {} file(s). {} file(s) with hits, {} total hits.", result.summary.files_searched, hits.len(), total_hits);
    s.results = hits;
    s.status_text = "Done.".to_string();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directly answers "can the output folder hold multiple indices for
    /// different searched directories" (a real question raised about
    /// this feature) - two different roots, even ones that happen to
    /// share a basename (a real, non-hypothetical case: two different
    /// drives'/machines' own "Projects" folder), must resolve to
    /// different keys, and the same root must always resolve to the
    /// SAME key across calls (so re-searching the same folder finds its
    /// own existing index rather than starting a fresh one every time).
    #[test]
    fn sanitized_root_key_never_collides_across_different_roots() {
        let a = sanitized_root_key("/Users/alice/Projects");
        let b = sanitized_root_key("/Volumes/Backup/Projects");
        assert_ne!(a, b, "two different directories that share a basename must not collide");
        assert_eq!(a, sanitized_root_key("/Users/alice/Projects"), "the same root must resolve to a stable key");
        assert_eq!(a, sanitized_root_key("/Users/alice/Projects/"), "a trailing slash must not change the key");
    }

    /// Real, on-disk verification that `index_one_root_with_progress` (the
    /// function both the manual Build/Rebuild buttons and the automatic
    /// post-search reindex call) actually produces a queryable native
    /// index - not just that it compiles. Answers a direct question raised
    /// about this feature: does the index-generation path really write
    /// the expected files, or does it only look like it works from
    /// reading the source. Asserts three things a passing `cargo check`
    /// can't: (1) `.native-search-index/` is created on disk with real,
    /// non-empty segment files, (2) the engine reopens successfully from
    /// those files, (3) it reports the correct document count. A prior
    /// version of this code silently omitted the "actually update the
    /// index" step entirely (see the git history for this function) -
    /// this test would have caught that by asserting `num_docs() > 0`,
    /// not just "the call returned Ok".
    #[tokio::test]
    async fn index_one_root_with_progress_writes_a_real_queryable_index() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple banana").unwrap();
        std::fs::write(dir.path().join("b.txt"), "cherry date").unwrap();

        let settings =
            SearchSettings { search_path: dir.path().display().to_string(), filters: vec!["apple".to_string()], ..SearchSettings::default() };
        let index_dir = search_core::native_index::index_directory(&settings.search_path);

        let mut progress_calls = 0i32;
        let outcome = index_one_root_with_progress(&settings, &index_dir, |_| progress_calls += 1).await.expect("indexing should succeed");

        assert_eq!(outcome.indexed_count, 2, "both real files should be indexed, not silently skipped");
        assert!(progress_calls >= 2, "progress callback should fire at least once per file");
        assert!(index_dir.exists(), "the index directory must actually exist on disk");
        let has_real_files = std::fs::read_dir(&index_dir).unwrap().filter_map(|e| e.ok()).any(|e| e.metadata().map(|m| m.len() > 0).unwrap_or(false));
        assert!(has_real_files, "the index directory must contain real, non-empty files - not just an empty folder");

        // Reopen from scratch (a fresh `NativeSearchEngine`, not the one
        // that just wrote it) to prove the files on disk are actually a
        // valid, loadable index, not just bytes that happen to exist.
        {
            let reopened = search_core::native_index::open_or_create_with_rebuild(&index_dir).expect("a just-built index must reopen cleanly");
            assert_eq!(reopened.num_docs(), 2, "the reopened index must report both documents that were just indexed");
        } // dropped before reopening again below - Tantivy's writer lockfile
          // only allows one open `IndexWriter` per directory at a time
          // (confirmed directly: an earlier version of this test that kept
          // `reopened` alive across the next open failed with a real
          // `LockBusy` error, not a hypothetical concern).

        // Re-running against the unchanged folder must skip both files
        // (skip-if-unchanged), proving metadata was actually persisted,
        // not just a document count that happens to match by coincidence.
        let second = index_one_root_with_progress(&settings, &index_dir, |_| {}).await.expect("re-indexing an unchanged folder should succeed");
        assert_eq!(second.indexed_count, 0);
        assert_eq!(second.skipped_count, 2);
    }
}
