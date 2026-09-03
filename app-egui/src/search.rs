//! Search Files tool - egui port of the core workflow in `app/src/state.rs`
//! (`AppState::run_search`) and `app/src/components.rs` (`SettingsPanel`/
//! `ResultsPanel`).
//!
//! `search-core`'s `orchestrator::run` was already framework-agnostic
//! (progress flows over a plain `tokio::sync::mpsc::UnboundedSender<
//! SearchProgressReport>`, never touching a Dioxus `Signal` directly) -
//! reused completely unchanged here. Only the *driver* around it is new:
//! Dioxus's `Signal`-per-field state + `self.progress_percent.set(...)`
//! becomes a plain `SearchUiState` behind an `Arc<Mutex<_>>`, written to
//! from a background task on a persistent `tokio::runtime::Runtime` and
//! read once per frame in `update()` - the standard egui pattern for
//! bridging async work into an immediate-mode UI.
//!
//! Scope of this pass (real, working, not a stub): folder picker, filter
//! text, parallel search via the real orchestrator, live progress, a
//! results list, cancel. NOT yet ported (tracked in
//! `docs/issue-11-phase-14.md`, not silently dropped): the Matching/
//! Scope-and-output/Performance/Fast-re-search-index expander sections,
//! HTML/CSV/JSON report export, presets, recent searches, drag-drop,
//! desktop notifications, the extension type-to-filter picker (uses the
//! engine's default extension list unconditionally for now).

use std::sync::{Arc, Mutex};

use eframe::egui;
use search_core::models::{FileSearchResult, FileSearchStatus, SearchProgressReport, SearchRunResult, SearchSettings};
use search_core::orchestrator::{self, OrchestratorError};
use search_core::report;
use tokio_util::sync::CancellationToken;

use crate::theme::Tokens;

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
}

pub struct SearchTool {
    search_path: String,
    filters_text: String,
    parallel: bool,
    shared: Arc<Mutex<SearchUiState>>,
    cancel_token: Option<CancellationToken>,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl SearchTool {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>) -> Self {
        Self {
            search_path: String::new(),
            filters_text: String::new(),
            parallel: true,
            shared: Arc::new(Mutex::new(SearchUiState::default())),
            cancel_token: None,
            runtime,
        }
    }

    fn build_settings(&self) -> SearchSettings {
        let filters: Vec<String> = self.filters_text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        SearchSettings {
            search_path: self.search_path.clone(),
            output_folder: self.search_path.clone(),
            filters,
            parallel: self.parallel,
            ..SearchSettings::default()
        }
    }

    pub fn is_running(&self) -> bool {
        self.shared.lock().unwrap().is_running
    }

    pub fn search_path(&self) -> &str {
        &self.search_path
    }
    pub fn filters_text(&self) -> &str {
        &self.filters_text
    }
    pub fn parallel(&self) -> bool {
        self.parallel
    }
    pub fn restore(&mut self, search_path: String, filters_text: String, parallel: bool) {
        self.search_path = search_path;
        self.filters_text = filters_text;
        self.parallel = parallel;
    }

    pub fn trigger_run(&mut self) {
        self.start();
    }
    pub fn trigger_cancel(&mut self) {
        if let Some(t) = &self.cancel_token {
            t.cancel();
        }
    }

    fn start(&mut self) {
        if self.search_path.trim().is_empty() {
            return;
        }
        let settings = self.build_settings();
        let cancellation = CancellationToken::new();
        self.cancel_token = Some(cancellation.clone());
        {
            let mut s = self.shared.lock().unwrap();
            *s = SearchUiState { is_running: true, status_text: "Starting...".to_string(), ..Default::default() };
        }
        let shared = self.shared.clone();
        let report_settings = settings.clone();
        self.runtime.spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SearchProgressReport>();
            let run_cancellation = cancellation.clone();
            let join_handle = tokio::spawn(async move { orchestrator::run(settings, Some(tx), run_cancellation).await });

            while let Some(report) = rx.recv().await {
                apply_progress(&mut shared.lock().unwrap(), report);
            }

            let outcome = join_handle.await;
            match outcome {
                Ok(Ok(result)) => {
                    let mut s = shared.lock().unwrap();
                    finish(&mut s, &report_settings, result);
                    let _ = notify_rust::Notification::new().summary("Search complete").body(&s.summary_text).show();
                }
                Ok(Err(OrchestratorError::Cancelled)) => shared.lock().unwrap().status_text = "Cancelled.".to_string(),
                Ok(Err(e)) => shared.lock().unwrap().status_text = format!("Error: {e}"),
                Err(join_err) => shared.lock().unwrap().status_text = format!("Error: {join_err}"),
            }
            shared.lock().unwrap().is_running = false;
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let is_running = self.shared.lock().unwrap().is_running;

        // Native dropped-file handling - `eframe` (via `winit`) already
        // surfaces this on `RawInput` (`ctx.input(|i| &i.raw.dropped_files)`)
        // with no hand-rolled interception needed. Blitz's own
        // `blitz-shell` never forwarded `WindowEvent::DroppedFile` at all
        // (see `app/src/drag_drop.rs`'s doc comment - `app/` had to wrap
        // the whole application handler just to get this), so this is
        // strictly simpler here, not just different.
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

        ui.horizontal(|ui| {
            ui.label("Search folder:");
            ui.add(egui::TextEdit::singleline(&mut self.search_path).desired_width(360.0));
            if ui.button("Browse\u{2026}").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.search_path = path.display().to_string();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Filters (comma-separated):");
            ui.add(egui::TextEdit::singleline(&mut self.filters_text).desired_width(320.0).hint_text("e.g. invoice, overdue"));
        });
        ui.checkbox(&mut self.parallel, "Parallel processing");

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!is_running, egui::Button::new("\u{25B6} Run search")).clicked() {
                self.start();
            }
            if is_running && ui.button("Cancel").clicked() {
                if let Some(t) = &self.cancel_token {
                    t.cancel();
                }
            }
        });

        ui.add_space(10.0);
        let s = self.shared.lock().unwrap();
        if s.is_running || s.total_files > 0 {
            let frac = if s.total_files > 0 { s.files_completed as f32 / s.total_files as f32 } else { 0.0 };
            ui.horizontal(|ui| {
                ui.label(format!("{} / {} files", s.files_completed, s.total_files));
                ui.label(format!("{} hits so far", s.hits_so_far));
            });
            ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)));
        }
        if !s.status_text.is_empty() {
            ui.colored_label(tokens.fg_muted, &s.status_text);
        }
        if !s.summary_text.is_empty() {
            ui.colored_label(tokens.fg_muted, &s.summary_text);
        }
        if let Some(path) = s.report_path.clone() {
            if ui.button("\u{1F4C4} Open HTML report").clicked() {
                let _ = open::that(&path);
            }
        }

        ui.add_space(10.0);
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for r in s.results.iter() {
                ui.horizontal(|ui| {
                    ui.strong(file_name(&r.full_name));
                    ui.colored_label(tokens.fg_subtle, format!("{} matches", r.hits.len()));
                });
                ui.colored_label(tokens.fg_subtle, &r.full_name);
                ui.separator();
            }
        });
    }
}

fn file_name(full_path: &str) -> &str {
    full_path.rsplit(['/', '\\']).next().unwrap_or(full_path)
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

fn finish(s: &mut SearchUiState, settings: &SearchSettings, result: SearchRunResult) {
    if result.was_dry_run {
        s.status_text = "Dry run complete.".to_string();
        return;
    }

    // Reuses search-core::report unchanged - it already only takes plain
    // SearchSettings/SearchRunResult, never a Dioxus type.
    let report_name = format!("SearchResults_{}.html", chrono::Local::now().format("%Y%m%d_%H%M%S"));
    let report_path = std::path::Path::new(&settings.output_folder).join(&report_name);
    match report::write_html_report(&report_path.display().to_string(), settings, &result) {
        Ok(_) => s.report_path = Some(report_path.display().to_string()),
        Err(e) => s.status_text = format!("Report write failed: {e}"),
    }

    let hits: Vec<FileSearchResult> = result.file_results.into_iter().filter(|r| r.status == FileSearchStatus::Hit).collect();
    let total_hits: i32 = hits.iter().map(|r| r.hits.len() as i32).sum();
    s.summary_text = format!(
        "Searched {} file(s). {} file(s) with hits, {} total hits.",
        result.summary.files_searched,
        hits.len(),
        total_hits
    );
    s.results = hits;
    s.status_text = "Done.".to_string();
}
