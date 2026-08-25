//! Ports `TextInFilesSearch.Core/ViewModels/MainViewModel.cs`: every
//! user-configurable setting as a Dioxus `Signal`, plus the Run/Cancel/
//! Native-Search command logic. Unlike the C# original (which injects
//! folder-picker/report-opening delegates specifically so the ViewModel
//! stays unit-testable without a WinUI reference), this crate has no such
//! constraint - `search-core` is already the fully-tested, UI-free layer,
//! so this file is allowed to call `rfd`/`open` directly.
//!
//! One deliberate simplification vs. the C# original: `NativeSearchService`
//! there caches one long-lived engine handle and disposes/reopens it when
//! `SearchPath` changes (real complexity, needed because a native handle is
//! expensive and manual disposal races are possible). Opening a Tantivy
//! index is cheap, so this port just opens (or creates) it fresh on every
//! native-search/index call instead - no cached handle, no disposal race to
//! guard against.

use std::path::Path;

use dioxus::prelude::*;
use native_search::engine::{CancellationFlag, NativeSearchEngine, SearchHit};
use search_core::models::{
    extension_catalog, ExcludeScope, FileSearchResult, FileSearchStatus, GroupByMode, InFlightFileStatus, MatchMode,
    SearchSettings,
};
use search_core::native_index;
use search_core::orchestrator::{self, OrchestratorError};
use search_core::report;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExtensionOption {
    pub extension: String,
    pub category: String,
    pub is_selected: bool,
}

#[derive(Clone, PartialEq)]
pub struct FileResultView {
    pub full_name: String,
    pub file_name: String,
    pub hit_count: usize,
    /// Full match context (before/match/after lines, matched filter
    /// names) - carried through so the preview pane (`preview.rs`) can
    /// show real highlighted match context, not just a file name and a
    /// count. Search-core's own `search-core::report` module has the
    /// canonical highlighting logic for the HTML report; the preview
    /// pane's highlighting is a plain-text-friendly equivalent of the
    /// same idea (bold the matched span), not a reimplementation of that
    /// HTML-specific code.
    pub hits: Vec<search_core::models::LineHit>,
    pub low_confidence_pdf: bool,
}

impl From<&FileSearchResult> for FileResultView {
    fn from(r: &FileSearchResult) -> Self {
        let file_name = Path::new(&r.full_name)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| r.full_name.clone());
        FileResultView {
            full_name: r.full_name.clone(),
            file_name,
            hit_count: r.hits.len(),
            hits: r.hits.clone(),
            low_confidence_pdf: r.low_confidence_pdf,
        }
    }
}

/// One prior search's search-defining fields (issue: epic §23 "recent
/// searches") - session-only, in-memory, most-recent-first, capped and
/// deduplicated in `AppState::remember_recent_search`. Deliberately not
/// persisted to disk (no settings-persistence layer exists in this app
/// yet for any other session toggle either - matches the rest of
/// `AppState`, which all resets to defaults on relaunch).
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RecentSearch {
    pub search_path: String,
    pub filters_text: String,
}

impl RecentSearch {
    pub fn label(&self) -> String {
        let folder_name = Path::new(&self.search_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.search_path.clone());
        format!("{folder_name} - {}", self.filters_text)
    }
}

#[derive(Clone, PartialEq)]
pub struct NativeHitView {
    pub id: String,
    pub path: String,
    pub filename: String,
    pub score: f32,
}

impl From<SearchHit> for NativeHitView {
    fn from(h: SearchHit) -> Self {
        NativeHitView { id: h.id, path: h.path, filename: h.filename, score: h.score }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct AppState {
    // ---- Required ----
    pub search_path: Signal<String>,
    pub output_folder: Signal<String>,
    pub output_name: Signal<String>,
    pub filters_text: Signal<String>,

    // ---- Matching ----
    pub match_mode: Signal<MatchMode>,
    pub proximity_lines: Signal<i32>,
    pub use_regex: Signal<bool>,
    pub whole_word: Signal<bool>,
    pub exclude_filters_text: Signal<String>,
    pub exclude_scope: Signal<ExcludeScope>,

    // ---- Scope and output ----
    pub extension_catalog: Signal<Vec<ExtensionOption>>,
    pub extension_filter_text: Signal<String>,
    pub exclude_folders_text: Signal<String>,
    pub include_hidden: Signal<bool>,
    pub max_file_size_mb: Signal<f64>,
    pub group_by: Signal<GroupByMode>,
    pub open_report_when_done: Signal<bool>,
    pub export_csv: Signal<bool>,
    pub export_json: Signal<bool>,

    // ---- Performance and robustness ----
    pub parallel: Signal<bool>,
    pub throttle_limit: Signal<i32>,
    pub cache_file_path: Signal<String>,
    pub dry_run: Signal<bool>,
    pub pdf_timeout_seconds: Signal<i32>,
    pub file_timeout_seconds: Signal<i32>,
    pub max_retries: Signal<i32>,

    // ---- Fast re-search (native_search) ----
    pub index_for_fast_search: Signal<bool>,
    pub native_search_query: Signal<String>,
    pub native_search_status_text: Signal<String>,
    pub is_native_searching: Signal<bool>,
    pub native_search_results: Signal<Vec<NativeHitView>>,
    pub native_search_cancel: Signal<Option<CancellationFlag>>,

    // ---- Live run state ----
    pub is_running: Signal<bool>,
    pub progress_percent: Signal<f64>,
    pub status_text: Signal<String>,
    pub in_flight_files: Signal<Vec<InFlightFileStatus>>,
    pub results: Signal<Vec<FileResultView>>,
    pub results_summary_text: Signal<String>,
    /// Real substitute for list virtualization (epic §5/§31) - see
    /// `components.rs`'s `MAX_RENDERED_RESULTS`/pagination comment for
    /// why scroll-based virtualization specifically is not just deferred
    /// but architecturally incoherent to attempt in this renderer.
    pub results_page: Signal<usize>,
    /// The result currently shown in the preview pane (epic §14) - set by
    /// clicking a row in `ResultsPanel`.
    pub selected_result: Signal<Option<FileResultView>>,
    pub has_results: Signal<bool>,
    pub last_report_path: Signal<Option<String>>,
    pub cancel_token: Signal<Option<CancellationToken>>,
    pub recent_searches: Signal<Vec<RecentSearch>>,
    /// Rendered by `ContextMenu` at the app root (`main.rs`), set by any
    /// row's right-click handler - a top-level signal rather than
    /// component-local state so the menu overlay isn't clipped by
    /// whatever scrollable list the row lives in. See `context_menu.rs`
    /// for why this is a custom component (`oncontextmenu` is a
    /// framework-level event type in `dioxus-html` that's never actually
    /// dispatched by this renderer - right-click only ever arrives as an
    /// ordinary mouse event with `MouseButton::Secondary` - the same
    /// "the generic event type exists but isn't wired for this renderer"
    /// gap as `onscroll`).
    pub context_menu: Signal<Option<ContextMenuState>>,
    /// Set by a spawned task draining `fs_watch::CHANGE_EVENTS` (main.rs) -
    /// epic §21. Reset to `false` at the start of every `run_search`.
    pub folder_changed_since_search: Signal<bool>,
}

/// One open custom context menu (epic §35 - no native context-menu API
/// exists in this stack) - the file it applies to, and the position
/// (client/viewport coordinates from the triggering right-click) to
/// render it at.
#[derive(Clone, PartialEq)]
pub struct ContextMenuState {
    pub full_name: String,
    pub x: f64,
    pub y: f64,
}

impl AppState {
    pub fn new() -> Self {
        let initial_catalog: Vec<ExtensionOption> = extension_catalog::CATEGORIES
            .iter()
            .flat_map(|cat| {
                cat.extensions.iter().map(move |ext| ExtensionOption {
                    extension: ext.to_string(),
                    category: cat.category.to_string(),
                    is_selected: false,
                })
            })
            .collect();

        AppState {
            search_path: use_signal(String::new),
            output_folder: use_signal(String::new),
            output_name: use_signal(String::new),
            filters_text: use_signal(String::new),

            match_mode: use_signal(|| MatchMode::AnyLine),
            proximity_lines: use_signal(|| 5),
            use_regex: use_signal(|| false),
            whole_word: use_signal(|| false),
            exclude_filters_text: use_signal(String::new),
            exclude_scope: use_signal(|| ExcludeScope::Line),

            extension_catalog: use_signal(|| initial_catalog),
            extension_filter_text: use_signal(String::new),
            exclude_folders_text: use_signal(String::new),
            include_hidden: use_signal(|| false),
            max_file_size_mb: use_signal(|| 50.0),
            group_by: use_signal(|| GroupByMode::Created),
            open_report_when_done: use_signal(|| false),
            export_csv: use_signal(|| false),
            export_json: use_signal(|| false),

            parallel: use_signal(|| false),
            throttle_limit: use_signal(|| 5),
            cache_file_path: use_signal(String::new),
            dry_run: use_signal(|| false),
            pdf_timeout_seconds: use_signal(|| 15),
            file_timeout_seconds: use_signal(|| 30),
            max_retries: use_signal(|| 3),

            index_for_fast_search: use_signal(|| false),
            native_search_query: use_signal(String::new),
            native_search_status_text: use_signal(|| {
                "Enable \"Index for fast re-search\" below, run a search, then search here.".to_string()
            }),
            is_native_searching: use_signal(|| false),
            native_search_results: use_signal(Vec::new),
            native_search_cancel: use_signal(|| None),

            is_running: use_signal(|| false),
            progress_percent: use_signal(|| 0.0),
            status_text: use_signal(|| "Ready.".to_string()),
            in_flight_files: use_signal(Vec::new),
            results: use_signal(Vec::new),
            results_summary_text: use_signal(String::new),
            results_page: use_signal(|| 0),
            selected_result: use_signal(|| None),
            has_results: use_signal(|| false),
            last_report_path: use_signal(|| None),
            cancel_token: use_signal(|| None),
            recent_searches: use_signal(Vec::new),
            context_menu: use_signal(|| None),
            folder_changed_since_search: use_signal(|| false),
        }
    }

    /// Most-recent-first, deduplicated by (search_path, filters_text),
    /// capped at 8 - called once per `run_search` (not per keystroke).
    fn remember_recent_search(&mut self) {
        let entry = RecentSearch {
            search_path: self.search_path.read().trim().to_string(),
            filters_text: self.filters_text.read().trim().to_string(),
        };
        if entry.search_path.is_empty() || entry.filters_text.is_empty() {
            return;
        }
        let mut recent = self.recent_searches.write();
        recent.retain(|r| *r != entry);
        recent.insert(0, entry);
        recent.truncate(8);
    }

    /// Ports the "Recent" click-to-reapply interaction (epic §23) -
    /// re-populates the two search-defining fields without touching any
    /// other setting.
    pub fn apply_recent_search(&mut self, recent: &RecentSearch) {
        self.search_path.set(recent.search_path.clone());
        self.filters_text.set(recent.filters_text.clone());
    }

    pub fn can_run(&self) -> bool {
        !*self.is_running.read()
            && !self.search_path.read().trim().is_empty()
            && !self.output_folder.read().trim().is_empty()
            && !self.filters_text.read().trim().is_empty()
    }

    pub fn can_native_search(&self) -> bool {
        !*self.is_native_searching.read()
            && !self.native_search_query.read().trim().is_empty()
            && !self.search_path.read().trim().is_empty()
    }

    fn build_selected_extensions(&self) -> Option<Vec<String>> {
        let selected: Vec<String> =
            self.extension_catalog.read().iter().filter(|e| e.is_selected).map(|e| e.extension.clone()).collect();
        if selected.is_empty() {
            None
        } else {
            Some(selected)
        }
    }

    fn build_exclude_folders(&self) -> Vec<String> {
        let mut folders = parse_list(&self.exclude_folders_text.read());
        native_index::ensure_index_folder_excluded(&mut folders);
        folders
    }

    pub fn build_settings(&self) -> SearchSettings {
        let output_name_raw = self.output_name.read().trim().to_string();
        let cache_path_raw = self.cache_file_path.read().trim().to_string();

        SearchSettings {
            search_path: self.search_path.read().trim().to_string(),
            output_folder: self.output_folder.read().trim().to_string(),
            output_name: if output_name_raw.is_empty() { None } else { Some(sanitize_file_name(&output_name_raw)) },
            filters: parse_list(&self.filters_text.read()),
            exclude_filters: parse_list(&self.exclude_filters_text.read()),
            match_mode: *self.match_mode.read(),
            proximity_lines: *self.proximity_lines.read(),
            exclude_scope: *self.exclude_scope.read(),
            whole_word: *self.whole_word.read(),
            use_regex: *self.use_regex.read(),
            group_by: *self.group_by.read(),
            extensions: self.build_selected_extensions(),
            exclude_folders: self.build_exclude_folders(),
            include_hidden: *self.include_hidden.read(),
            max_file_size_mb: *self.max_file_size_mb.read(),
            // Not exposed in the UI - matches the C# original, which has no
            // XAML control for MaxEmbedLines either.
            max_embed_lines: 4000,
            pdf_timeout_seconds: *self.pdf_timeout_seconds.read(),
            export_csv: *self.export_csv.read(),
            export_json: *self.export_json.read(),
            open_report_when_done: *self.open_report_when_done.read(),
            parallel: *self.parallel.read(),
            throttle_limit: *self.throttle_limit.read(),
            cache_file_path: if cache_path_raw.is_empty() { None } else { Some(cache_path_raw) },
            dry_run: *self.dry_run.read(),
            max_retries: *self.max_retries.read(),
            // Not exposed in the UI - matches the C# original, which has no
            // XAML control for RetryDelayMs either.
            retry_delay_ms: 250,
            file_timeout_seconds: *self.file_timeout_seconds.read(),
        }
    }

    pub fn add_custom_extension(&mut self) {
        let raw = self.extension_filter_text.read().trim().to_string();
        if raw.is_empty() {
            return;
        }
        let normalized = if raw.starts_with('.') { raw } else { format!(".{raw}") }.to_lowercase();

        let mut catalog = self.extension_catalog.write();
        if let Some(existing) = catalog.iter_mut().find(|e| e.extension.eq_ignore_ascii_case(&normalized)) {
            existing.is_selected = true;
        } else {
            catalog.push(ExtensionOption { extension: normalized, category: "Custom".to_string(), is_selected: true });
        }
        drop(catalog);
        self.extension_filter_text.set(String::new());
    }

    pub fn clear_selected_extensions(&mut self) {
        for e in self.extension_catalog.write().iter_mut() {
            e.is_selected = false;
        }
    }

    pub fn cancel_search(&self) {
        if let Some(token) = self.cancel_token.read().as_ref() {
            token.cancel();
        }
    }

    pub fn cancel_native_search(&self) {
        if let Some(flag) = self.native_search_cancel.read().as_ref() {
            flag.cancel();
        }
    }

    pub async fn browse_search_folder(mut self) {
        if let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await {
            self.search_path.set(handle.path().to_string_lossy().into_owned());
        }
    }

    pub async fn browse_output_folder(mut self) {
        if let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await {
            self.output_folder.set(handle.path().to_string_lossy().into_owned());
        }
    }

    pub fn open_report(&self) {
        if let Some(path) = self.last_report_path.read().as_ref() {
            let _ = open::that(path);
        }
    }

    fn apply_progress(&mut self, report: search_core::models::SearchProgressReport) {
        if report.is_enumerating {
            self.status_text.set(if report.enumerated_file_count > 0 {
                format!("Scanning folders... {} file(s) found so far", report.enumerated_file_count)
            } else {
                "Scanning folders...".to_string()
            });
            return;
        }

        if report.total_files > 0 {
            self.progress_percent.set(100.0 * report.files_completed as f64 / report.total_files as f64);
            self.status_text.set(format!(
                "{} of {} file(s) - {} hit(s) so far",
                report.files_completed, report.total_files, report.hits_so_far
            ));
        }

        self.in_flight_files.set(report.in_flight_files);

        if let Some(r) = &report.last_completed_result {
            if r.status == FileSearchStatus::Hit {
                let already_present =
                    self.results.read().iter().any(|existing| existing.full_name.eq_ignore_ascii_case(&r.full_name));
                if !already_present {
                    self.results.write().push(FileResultView::from(r));
                    self.has_results.set(true);
                }
            }
        }
    }

    pub async fn run_search(mut self) {
        let settings = self.build_settings();
        self.remember_recent_search();
        self.folder_changed_since_search.set(false);
        self.results_page.set(0);
        self.selected_result.set(None);

        self.results.write().clear();
        self.in_flight_files.write().clear();
        self.has_results.set(false);
        self.results_summary_text.set(String::new());
        self.last_report_path.set(None);
        self.progress_percent.set(0.0);
        self.status_text.set("Starting...".to_string());

        let cancellation = CancellationToken::new();
        self.cancel_token.set(Some(cancellation.clone()));
        self.is_running.set(true);

        let (tx, mut rx) = mpsc::unbounded_channel();

        let run_settings = settings.clone();
        let run_cancellation = cancellation.clone();
        // Raw tokio::spawn (not Dioxus's spawn) is safe here: this task
        // only ever writes into an mpsc channel, never touches a Signal
        // directly - only the outer, Dioxus-spawned task (this whole
        // async fn) touches signals, and it does so from a single
        // consistent task throughout.
        let join_handle = tokio::spawn(async move { orchestrator::run(run_settings, Some(tx), run_cancellation).await });

        while let Some(report) = rx.recv().await {
            self.apply_progress(report);
        }

        match join_handle.await {
            Ok(Ok(result)) => self.finish_successful_run(settings, result).await,
            Ok(Err(OrchestratorError::Cancelled)) => self.status_text.set("Cancelled.".to_string()),
            Ok(Err(e)) => self.status_text.set(format!("Error: {e}")),
            Err(join_err) => self.status_text.set(format!("Error: {join_err}")),
        }

        self.is_running.set(false);
        self.in_flight_files.write().clear();
        self.cancel_token.set(None);
    }

    async fn finish_successful_run(&mut self, settings: SearchSettings, result: search_core::models::SearchRunResult) {
        if result.was_dry_run {
            let count = result.dry_run_candidates.as_ref().map(|c| c.len()).unwrap_or(0);
            let msg = format!("Dry run: {count} file(s) would be searched. Nothing was read or written.");
            self.status_text.set(msg.clone());
            self.results_summary_text.set(msg);
            return;
        }

        let hit_results: Vec<FileSearchResult> =
            result.file_results.iter().filter(|r| r.status == FileSearchStatus::Hit).cloned().collect();
        self.results.set(hit_results.iter().map(FileResultView::from).collect());
        self.has_results.set(!hit_results.is_empty());

        let total_hits: i32 = hit_results.iter().map(|r| r.hits.len() as i32).sum();
        let extra = if result.summary.enumeration_errors > 0 {
            format!(
                " {} folder(s)/file(s) couldn't be listed (permissions or a broken link).",
                result.summary.enumeration_errors
            )
        } else {
            String::new()
        };
        self.results_summary_text.set(format!(
            "Searched {} file(s). {} file(s) with hits, {} total hits. Skipped: {} too large, {} binary, {} unreadable, {} unexpected errors.{extra}",
            result.summary.files_searched,
            hit_results.len(),
            total_hits,
            result.summary.skipped_too_large,
            result.summary.skipped_binary,
            result.summary.skipped_read_error,
            result.summary.skipped_unexpected_error,
        ));

        let html = report::build_html_report(&settings, &result);
        let output_name = match &settings.output_name {
            Some(n) if n.to_lowercase().ends_with(".html") => n.clone(),
            Some(n) => format!("{n}.html"),
            None => format!("SearchResults_{}.html", chrono::Local::now().format("%Y%m%d_%H%M%S")),
        };

        if let Err(e) = tokio::fs::create_dir_all(&settings.output_folder).await {
            self.status_text.set(format!("Error creating output folder: {e}"));
            self.status_text.set("Done.".to_string());
            return;
        }

        let report_path = Path::new(&settings.output_folder).join(&output_name);
        match tokio::fs::write(&report_path, &html).await {
            Ok(()) => {
                let report_path_str = report_path.to_string_lossy().into_owned();
                self.last_report_path.set(Some(report_path_str.clone()));

                if settings.export_csv || settings.export_json {
                    let rows = report::build_export_rows(&result);
                    if settings.export_csv {
                        let _ = report::write_csv(&change_extension(&report_path_str, "csv"), &rows);
                    }
                    if settings.export_json {
                        let _ = report::write_json(&change_extension(&report_path_str, "json"), &rows);
                    }
                }

                if settings.open_report_when_done {
                    let _ = open::that(&report_path_str);
                }
            }
            Err(e) => self.status_text.set(format!("Error writing report: {e}")),
        }

        if *self.index_for_fast_search.read() {
            let search_path = settings.search_path.clone();
            let msg = tokio::task::spawn_blocking(move || index_hits_for_fast_search(&hit_results, &search_path))
                .await
                .unwrap_or_else(|e| format!("Fast re-search indexing failed: {e}"));
            self.native_search_status_text.set(msg);
        }

        self.status_text.set("Done.".to_string());
    }

    pub async fn run_native_search(mut self) {
        self.native_search_results.write().clear();
        self.native_search_status_text.set("Searching...".to_string());
        self.is_native_searching.set(true);

        let cancel_flag = CancellationFlag::new();
        self.native_search_cancel.set(Some(cancel_flag.clone()));

        let query = self.native_search_query.read().clone();
        let search_path = self.search_path.read().clone();

        let outcome = tokio::task::spawn_blocking(move || -> Result<Vec<NativeHitView>, native_search::error::NsError> {
            let dir = native_index::index_directory(&search_path);
            native_index::ensure_index_directory_exists(&dir)
                .map_err(|e| native_search::error::NsError::index_error(e.to_string()))?;
            let engine = NativeSearchEngine::open_or_create(&dir)?;
            let hits = engine.search(&query, 50, Some(&cancel_flag))?;
            Ok(hits.into_iter().map(NativeHitView::from).collect())
        })
        .await;

        match outcome {
            Ok(Ok(hits)) => {
                let count = hits.len();
                self.native_search_results.set(hits);
                self.native_search_status_text
                    .set(if count == 0 { "No results.".to_string() } else { format!("{count} result(s).") });
            }
            Ok(Err(e)) if e.status == native_search::error::NsStatus::Cancelled => {
                self.native_search_status_text.set("Cancelled.".to_string());
            }
            Ok(Err(e)) => self.native_search_status_text.set(format!("Error: {}", e.message)),
            Err(join_err) => self.native_search_status_text.set(format!("Error: {join_err}")),
        }

        self.is_native_searching.set(false);
        self.native_search_cancel.set(None);
    }
}

fn parse_list(text: &str) -> Vec<String> {
    text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Mirrors `Path.GetInvalidFileNameChars()` on Windows (the shipped
/// target - see CLAUDE.md): every ASCII control character plus
/// `< > : " / \ | ? *`.
fn sanitize_file_name(name: &str) -> String {
    name.chars().map(|c| if is_invalid_windows_filename_char(c) { '_' } else { c }).collect()
}

fn is_invalid_windows_filename_char(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20
}

fn change_extension(path: &str, new_ext: &str) -> String {
    match path.rfind('.') {
        Some(idx) => format!("{}.{}", &path[..idx], new_ext),
        None => format!("{path}.{new_ext}"),
    }
}

fn index_hits_for_fast_search(hits: &[FileSearchResult], search_path: &str) -> String {
    let dir = native_index::index_directory(search_path);
    if let Err(e) = native_index::ensure_index_directory_exists(&dir) {
        return format!("Fast re-search indexing failed: {e}");
    }
    let engine = match NativeSearchEngine::open_or_create(&dir) {
        Ok(e) => e,
        Err(e) => return format!("Fast re-search indexing failed: {e}"),
    };
    match native_index::index_hits_for_fast_search(&engine, hits) {
        Ok(outcome) => outcome.status_message(),
        Err(e) => format!("Fast re-search indexing failed: {e}"),
    }
}

pub fn filtered_extensions(catalog: &[ExtensionOption], filter_text: &str) -> Vec<ExtensionOption> {
    let needle = filter_text.trim().to_lowercase();
    if needle.is_empty() {
        catalog.to_vec()
    } else {
        catalog
            .iter()
            .filter(|e| e.extension.to_lowercase().contains(&needle) || e.category.to_lowercase().contains(&needle))
            .cloned()
            .collect()
    }
}

pub fn selected_extensions_summary(catalog: &[ExtensionOption]) -> String {
    let selected: Vec<&str> = catalog.iter().filter(|e| e.is_selected).map(|e| e.extension.as_str()).collect();
    if selected.is_empty() {
        "Using built-in default extension list.".to_string()
    } else {
        format!("Searching: {}", selected.join(", "))
    }
}
