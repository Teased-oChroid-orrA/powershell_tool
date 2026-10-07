//! Central application state and the single synchronous event reducer.
//! `handle_event` never touches the terminal, the async runtime, or the OS
//! directly - it only mutates `AppState` and returns `Effect`s for
//! `main.rs` to execute (spawn a search task, write a report, open a path,
//! ...). This keeps the whole decision layer testable without a real
//! terminal or tokio runtime, and gives the app exactly one mutation point
//! for state that background tasks also touch - a narrower surface than
//! either existing GUI head's own pattern (see the migration plan's Event
//! and State Model section).

use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use search_core::models::{SearchRunResult, SearchSettings};
use search_core::orchestrator::OrchestratorError;

use crate::command_palette::{Command, CommandPalette};
use crate::modal::{ConfirmAction, ConfirmDialog, ModalState};
use crate::mouse::{self, ClickTarget, MouseRegions};
use crate::nav::{FocusArea, FocusState, NavigationState, ToolId};
use crate::notifications::NotificationQueue;
use crate::theme::{StatusTone, Theme};
use crate::toolboxes::bushing::{self, BushingState};
use crate::toolboxes::fastener_hole::{self, FastenerHoleState};
use crate::toolboxes::fea_workbench::{self, FeaWorkbenchState};
use crate::toolboxes::eccentric_bushing::{self, EccentricState};
use crate::toolboxes::lug_analysis::{self, LugAnalysisState};
use crate::toolboxes::material_lookup::{self, MaterialLookupState};
use crate::toolboxes::preload_analysis::{self, PreloadAnalysisState};
use crate::toolboxes::pressure_vessel::{self, PressureVesselState};
use crate::toolboxes::search::{self, SearchToolState};

pub struct AppState {
    pub theme: Theme,
    pub nav: NavigationState,
    pub focus: FocusState,
    pub modal: ModalState,
    pub notifications: NotificationQueue,
    pub should_quit: bool,
    pub search: SearchToolState,
    pub fastener_hole: FastenerHoleState,
    pub pressure_vessel: PressureVesselState,
    pub bushing: BushingState,
    pub preload_analysis: PreloadAnalysisState,
    pub lug_analysis: LugAnalysisState,
    pub eccentric: EccentricState,
pub fea_workbench: FeaWorkbenchState,
    pub material_lookup: MaterialLookupState,
    /// What the last left-click landed on and when - compared against the
    /// next click to detect a double-click (see `mouse` module doc and
    /// `handle_mouse` below). Not persisted, not meaningful outside the
    /// live session.
    pub last_click: Option<(Instant, ClickTarget)>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            nav: NavigationState::default(),
            focus: FocusState::default(),
            modal: ModalState::default(),
            notifications: NotificationQueue::default(),
            should_quit: false,
            search: SearchToolState::default(),
            fastener_hole: FastenerHoleState::default(),
            pressure_vessel: PressureVesselState::default(),
            bushing: BushingState::default(),
            preload_analysis: PreloadAnalysisState::default(),
            lug_analysis: LugAnalysisState::default(),
            eccentric: EccentricState::default(),
            fea_workbench: FeaWorkbenchState::default(),
            material_lookup: MaterialLookupState::default(),
            last_click: None,
        }
    }
}

impl AppState {
    /// Number of focusable panes in the active toolbox's workspace, for
    /// Tab-cycling.
    pub fn workspace_pane_count(&self) -> u8 {
        match self.nav.active_tool {
            ToolId::Search => search::PANE_COUNT,
            ToolId::FastenerHole => fastener_hole::PANE_COUNT,
            ToolId::PressureVessel => pressure_vessel::PANE_COUNT,
            ToolId::Bushing => bushing::PANE_COUNT,
            ToolId::PreloadAnalysis => preload_analysis::PANE_COUNT,
            ToolId::LugAnalysis => lug_analysis::PANE_COUNT,
            ToolId::EccentricBushing => eccentric_bushing::PANE_COUNT,
            ToolId::FeaWorkbench => fea_workbench::PANE_COUNT,
            ToolId::MaterialLookup => material_lookup::PANE_COUNT,
            _ => 0,
        }
    }

    /// A running search should ask for confirmation before Quit - the
    /// active toolbox's own running flag is the source of truth, not a
    /// separate app-level "busy" bit that could drift out of sync with it.
    pub fn is_busy(&self) -> bool {
        self.search.run.is_running
    }
}

pub enum AppEvent {
    Terminal(Event),
    Tick,
    SearchProgress(search_core::models::SearchProgressReport),
    SearchFinished(Result<SearchRunResult, OrchestratorError>),
    /// Sent just before a successful `SearchFinished`: where the run's time went.
    SearchDiagnostics(Box<search::diagnostics::SearchDiagnostics>),
    ReportWritten(Option<String>),
    IndexBuildProgress(search_core::native_index::CorpusIndexProgress),
    IndexBuildFinished(native_search::error::NsResult<search_core::native_index::CorpusIndexOutcome>),
    /// One-line description of how the latest search used the fast index.
    IndexNarrowed(String),
    /// A search found the fast index out of date (files added/changed/removed
    /// since the build); the UI offers an update once the search finishes.
    IndexStale { new_or_changed: usize, removed: usize },
    ExtensionsScanned(Result<Vec<String>, String>),
    /// Result of reading a user-typed path for the reamer-library "Import"
    /// action (`toolboxes/bushing/reamer_picker.rs`) - file I/O, so it goes
    /// through the same `Effect`-request / `AppEvent`-response round trip
    /// as `ScanExtensions`/`ExtensionsScanned`, never a direct filesystem
    /// read from inside `handle_key`.
    ReamerLibraryFileRead(Result<String, String>),
    /// Same round trip as `ReamerLibraryFileRead`, for the Bushing material
    /// library's own "Import" prompt (`toolboxes/bushing/material_picker.rs`).
    BushingMaterialLibraryFileRead(Result<String, String>),
    /// Same round trip, for the Bushing ID library
    /// (`toolboxes/bushing/bushing_id_picker.rs`).
    BushingIdLibraryFileRead(Result<String, String>),
    /// A worker finished the Bushing edge-distance cross-check `id`
    /// (`Effect::RunEdgeCheck`); a cancelled or superseded id is ignored.
    EdgeCheckFinished { id: u64, run: Box<crate::toolboxes::bushing::edge_check::EdgeCheckRun> },
    /// A worker finished Lug Analysis job `id` (`Effect::RunLugAnalysis`); the
    /// condensed model it built (or reused) comes back for the cache, and a
    /// stale id is ignored.
    /// A worker finished Eccentric Bushing job `id` (`Effect::RunEccentric`).
    EccentricFinished { id: u64, result: Box<Result<eccentric_bushing::model::Output, String>> },
    LugAnalysisFinished { id: u64, result: Box<Result<lug_analysis::model::LugRun, String>>, cache: Option<lug_analysis::model::CachedModel> },
    /// A worker finished Lug Analysis mesh-size test `id` (`Effect::RunLugMeshTest`).
    LugMeshTestFinished { id: u64, result: Box<Result<lug_analysis::mesh_test::MeshAdvice, String>> },
    /// A worker finished the Preload Analysis finite-element member-compliance cross-check `id`.
    MemberFeFinished { id: u64, result: Box<Result<preload_analysis::fe_check::FeResult, String>> },
    /// A worker finished FEA Workbench mesh preview `id` (`Effect::RunFeaPreview`).
    FeaPreviewFinished { id: u64, result: Box<Result<fea_core::Mesh, String>> },
    /// A worker finished FEA Workbench solve `id` (`Effect::RunFeaSolve`).
    FeaSolveFinished { id: u64, result: Box<Result<fea_problem::Solved, String>> },
    /// A file read requested by `Effect::ReadTextFile` finished.
    TextFileRead { purpose: fea_workbench::FilePurpose, path: String, result: Result<String, String> },
    /// A write requested by `Effect::WriteTextFile` finished.
    TextFileWritten { path: String, result: Result<(), String> },
    Quit,
}

/// Side effects `handle_event` asks `main.rs` to perform - anything that
/// needs the async runtime or the outside world (spawning a search,
/// writing a report, opening a file, touching the clipboard) rather than a
/// plain state mutation. Kept as a real enum (not executed inline) so
/// `handle_event` stays a pure function callable from a unit test with no
/// terminal, no tokio runtime, and no filesystem/OS access.
#[derive(Debug)]
pub enum Effect {
    StartSearch { roots: Vec<String>, settings: SearchSettings, index: search::indexing::IndexSettings },
    /// `write_html` comes from `SearchToolConfig.export_html` - a TUI-only
    /// toggle, not a `SearchSettings` field (`SearchSettings` only carries
    /// `export_csv`/`export_json`/`open_report_when_done`; HTML export is
    /// gated purely at the UI layer in both existing heads too).
    WriteReport { settings: SearchSettings, run_result: SearchRunResult, write_html: bool },
    OpenPath(String),
    CopyToClipboard(String),
    /// Writes `contents` to `path`, creating the parent directory if
    /// needed, then opens it - used by the "export this file's hits"
    /// result action.
    WriteTextFileAndOpen { path: String, contents: String },
    PersistSearchSettings,
    BuildIndex { settings: SearchSettings, index_dir: std::path::PathBuf, cancel: CancellationToken },
    ScanExtensions { root: String, exclude_folders: Vec<String>, include_hidden: bool },
    /// Writes the Pressure Vessel Analyzer's plain-text report
    /// (`pressure_vessel::view::build_report_text`, already fully built by
    /// the time this effect is constructed - pure text generation, no I/O)
    /// to its fixed report path and opens it.
    ExportPressureVesselReport(String),
    /// Snapshots `state.pressure_vessel.model.custom_materials` and writes
    /// it to disk - same "derive from current state at execution time"
    /// pattern as `PersistSearchSettings`.
    PersistPressureVesselMaterials,
    /// Writes the Bushing Workbench's plain-text report
    /// (`bushing::view::build_report_text`) to its fixed report path and
    /// opens it - same pattern as `ExportPressureVesselReport`.
    ExportBushingReport(String),
    /// Runs the Bushing edge-distance cross-check (~0.3 s quick, ~3 s with
    /// the contact FE) on a blocking worker and reports back with
    /// `AppEvent::EdgeCheckFinished`; the input was already validated by
    /// `edge_check::prepare`.
    RunEdgeCheck { id: u64, input: ::edge_check::runner::EdgeInput, cfg: ::edge_check::runner::EdgeConfig, deep: bool },
    /// Runs one Lug Analysis (contact FE, 0.03-1 s) on a blocking worker and
    /// reports back with `AppEvent::LugAnalysisFinished`; `cache` is the
    /// condensed model from the last run, reused when the geometry is unchanged.
    /// Runs one Eccentric Bushing task (contact FE, seconds to tens of seconds) on a blocking worker and reports back
    /// with `AppEvent::EccentricFinished`.
    RunEccentric { id: u64, task: eccentric_bushing::model::Task, input: Box<eccentric_bushing::model::Inputs> },
    RunLugAnalysis { id: u64, input: Box<lug_analysis::model::LugInput>, cache: Option<lug_analysis::model::CachedModel> },
    /// Run the brief mesh-size test for these inputs on a worker; reports back with `AppEvent::LugMeshTestFinished`.
    RunLugMeshTest { id: u64, input: Box<lug_analysis::model::LugInput> },
    /// Run the Preload Analysis finite-element member-compliance check on a worker; reports back with `AppEvent::MemberFeFinished`.
    RunMemberFe { id: u64, input: Box<preload_analysis::fe_check::FeInput> },
    /// Build the FEA Workbench mesh for these inputs on a worker; reports back with `AppEvent::FeaPreviewFinished`.
    RunFeaPreview { id: u64, problem: Box<fea_problem::Problem>, import_text: Option<String> },
    /// Solve a FEA Workbench problem on a worker; reports back with `AppEvent::FeaSolveFinished`.
    RunFeaSolve { id: u64, problem: Box<fea_problem::Problem>, import_text: Option<String> },
    /// Writes `contents` to `path` (creating the folder) without opening it; reports back with `AppEvent::TextFileWritten`.
    WriteTextFile { path: String, contents: String },
    /// Reads the text file at `path` for `purpose`; reports back with `AppEvent::TextFileRead`.
    ReadTextFile { purpose: fea_workbench::FilePurpose, path: String },
    /// Writes the Preload Analysis toolbox's plain-text report to its
    /// fixed report path and opens it - same pattern as
    /// `ExportPressureVesselReport`/`ExportBushingReport`.
    ExportPreloadAnalysisReport(String),
    /// Writes the Fastener Holes toolbox's plain-text report to its fixed
    /// report path and opens it - same pattern as the other three
    /// toolboxes' own `Export*Report` effects.
    ExportFastenerHoleReport(String),
    /// Reads the file at `path` (a user-typed path from the reamer
    /// library's "Import" prompt) and reports the result via
    /// `AppEvent::ReamerLibraryFileRead` - reading is I/O, so it can't
    /// happen synchronously inside `handle_key`.
    ImportReamerLibraryFile(String),
    /// Writes `contents` (already-built JSON, pure and synchronous - see
    /// `library::export_json`) to `path`, creating the parent directory if
    /// needed - the reamer library's "Export" action.
    ExportReamerLibraryFile { path: String, contents: String },
    /// Snapshots `state.bushing.reamer_picker.library` and writes it to
    /// disk - same "derive from current state at execution time" pattern
    /// as `PersistPressureVesselMaterials`.
    PersistReamerLibrary,
    /// Same trio as the three `*ReamerLibraryFile`/`PersistReamerLibrary`
    /// effects above, for the Bushing material library
    /// (`toolboxes/bushing/material_picker.rs`).
    ImportBushingMaterialLibraryFile(String),
    ExportBushingMaterialLibraryFile { path: String, contents: String },
    PersistBushingMaterialLibrary,
    /// Same trio, for the Bushing ID library
    /// (`toolboxes/bushing/bushing_id_picker.rs`).
    ImportBushingIdLibraryFile(String),
    ExportBushingIdLibraryFile { path: String, contents: String },
    PersistBushingIdLibrary,
}

pub fn handle_event(state: &mut AppState, event: AppEvent) -> Vec<Effect> {
    match event {
        AppEvent::Terminal(Event::Key(key)) if key.kind == KeyEventKind::Press => {
            handle_key(state, key)
        }
        AppEvent::Terminal(_) => Vec::new(),
        AppEvent::Tick => {
            state.notifications.expire();
            state.bushing.expire_edge_notice();
            match state.nav.active_tool {
                crate::nav::ToolId::Bushing => state.bushing.auto_edge_check(),
                crate::nav::ToolId::LugAnalysis => state.lug_analysis.tick(),
                crate::nav::ToolId::FeaWorkbench => state.fea_workbench.tick(),
                crate::nav::ToolId::PreloadAnalysis => state.preload_analysis.tick(),
                _ => Vec::new(),
            }
        }
        AppEvent::SearchProgress(report) => {
            search::model::apply_progress(&mut state.search.run, report);
            Vec::new()
        }
        AppEvent::SearchFinished(result) => handle_search_finished(state, result),
        AppEvent::SearchDiagnostics(d) => {
            state.search.run.diagnostics = Some(*d);
            Vec::new()
        }
        AppEvent::ReportWritten(path) => {
            state.search.run.last_report_path = path.clone();
            match path {
                Some(p) if state.search.config.open_report_when_done => vec![Effect::OpenPath(p)],
                _ => Vec::new(),
            }
        }
        AppEvent::IndexBuildProgress(progress) => {
            state.search.index_run.apply_progress(&progress);
            Vec::new()
        }
        AppEvent::IndexStale { new_or_changed, removed } => {
            state.search.index_run.stale = Some((new_or_changed, removed));
            Vec::new()
        }
        AppEvent::IndexNarrowed(note) => {
            state.search.index_run.last_narrow = Some(note);
            Vec::new()
        }
        AppEvent::IndexBuildFinished(result) => {
            state.search.index_cancel = None;
            match result {
                Ok(outcome) => {
                    state.search.index_run.apply_outcome(&outcome);
                    if outcome.failed_count > 0 {
                        // `failed_count` alone told the user something broke but
                        // never why - surface the actual per-file reason (first
                        // failure is usually representative of a systemic cause:
                        // permissions, AV lock, unsupported format) instead of
                        // leaving them to guess. Every failure is in the debug log.
                        let detail = outcome.failed_files.first().map(|f| format!(" - e.g. {f}")).unwrap_or_default();
                        state.notifications.push(
                            format!("Index build: {} file(s) failed{detail}", outcome.failed_count),
                            StatusTone::Warning,
                        );
                    } else if outcome.cancelled {
                        state.notifications.push("Index build cancelled (partial index kept)", StatusTone::Info);
                    } else {
                        state.notifications.push(
                            format!("Index ready: {} documents", outcome.index_docs),
                            StatusTone::Success,
                        );
                    }
                }
                Err(e) => {
                    state.search.index_run.apply_error(e.to_string());
                    state.notifications.push(format!("Index build failed: {e}"), StatusTone::Danger);
                }
            }
            Vec::new()
        }
        AppEvent::ExtensionsScanned(result) => {
            state.search.extension_picker = match result {
                Ok(mut available) => {
                    // Recently-selected extensions stay pickable even when
                    // this particular folder doesn't happen to contain one
                    // right now - appended (not merged-and-resorted, which
                    // would change `scan_extensions`' existing alphabetical
                    // output order for a plain scan with nothing to add)
                    // after the scanned list, most-recent first, before
                    // `open_with` so its own selection-seeding logic
                    // (unaffected by this) still just sees one plain
                    // `available` list.
                    for ext in &state.search.recent_extensions {
                        if !available.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                            available.push(ext.clone());
                        }
                    }
                    search::extension_picker::ExtensionPicker::open_with(available, state.search.config.selected_extensions.as_deref())
                }
                Err(e) => search::extension_picker::ExtensionPicker::open_with_error(e),
            };
            Vec::new()
        }
        AppEvent::ReamerLibraryFileRead(result) => handle_reamer_library_file_read(state, result),
        AppEvent::BushingMaterialLibraryFileRead(result) => handle_bushing_material_library_file_read(state, result),
        AppEvent::BushingIdLibraryFileRead(result) => handle_bushing_id_library_file_read(state, result),
        AppEvent::EdgeCheckFinished { id, run } => {
            // A quick automatic re-run is silent; manual and deep runs get a toast.
            let was_running = state.bushing.edge_job.as_ref().is_some_and(|j| j.id == id && (!j.auto || j.deep));
            state.bushing.finish_edge_check(id, *run);
            if was_running {
                state.notifications.push("Edge check finished - see the Edge-Distance Cross-Check in Bushing Results", StatusTone::Success);
            }
            Vec::new()
        }
        AppEvent::EccentricFinished { id, result } => {
            state.eccentric.finish(id, *result);
            Vec::new()
        }
        AppEvent::LugAnalysisFinished { id, result, cache } => {
            state.lug_analysis.finish(id, *result, cache);
            Vec::new()
        }
        AppEvent::LugMeshTestFinished { id, result } => {
            state.lug_analysis.finish_mesh_test(id, *result);
            Vec::new()
        }
        AppEvent::MemberFeFinished { id, result } => {
            state.preload_analysis.finish_fe(id, *result);
            Vec::new()
        }
        AppEvent::FeaPreviewFinished { id, result } => {
            state.fea_workbench.finish_preview(id, *result);
            Vec::new()
        }
        AppEvent::FeaSolveFinished { id, result } => {
            state.fea_workbench.finish_solve(id, *result);
            Vec::new()
        }
        AppEvent::TextFileRead { purpose, path, result } => {
            if let Some((msg, tone)) = state.fea_workbench.file_read(purpose, path, result) {
                state.notifications.push(msg, tone);
            }
            Vec::new()
        }
        AppEvent::TextFileWritten { path, result } => {
            match result {
                Ok(()) => state.notifications.push(format!("Saved {path}"), StatusTone::Success),
                Err(e) => state.notifications.push(format!("Could not write {path}: {e}"), StatusTone::Danger),
            }
            Vec::new()
        }
        AppEvent::Quit => {
            request_quit(state);
            Vec::new()
        }
    }
}

/// Applies a just-read reamer-library import file: parses it, classifies
/// every item against the existing library (`library::classify_import`),
/// immediately applies additions/label-merges, and queues any real
/// conflicts for interactive resolution (`ReamerPickerState::handle_key`).
/// A parse failure or unreadable file surfaces as a toast, never a panic or
/// a silently-dropped import.
fn handle_reamer_library_file_read(state: &mut AppState, result: Result<String, String>) -> Vec<Effect> {
    let text = match result {
        Ok(text) => text,
        Err(e) => {
            state.notifications.push(e, StatusTone::Danger);
            return Vec::new();
        }
    };
    let incoming = match crate::library::import_json::<crate::toolboxes::bushing::reamer_persistence::PersistedReamer>(&text) {
        Ok(items) => items,
        Err(e) => {
            state.notifications.push(e, StatusTone::Danger);
            return Vec::new();
        }
    };
    let outcomes = crate::library::classify_import(&state.bushing.reamer_picker.library, incoming, |p| p.size_label.clone());
    let (queue, added, merged) = crate::library::ConflictQueue::new(outcomes, &mut state.bushing.reamer_picker.library);
    let conflicts_remaining = !queue.is_empty();
    state.bushing.reamer_picker.pending_conflicts = if conflicts_remaining { Some(queue) } else { None };
    state.bushing.model.sync_user_reamers(&state.bushing.reamer_picker.library);
    let suffix = if conflicts_remaining { " - conflicts need review (k: keep, o: overwrite, a/z: apply to all)" } else { "" };
    state.notifications.push(format!("Reamer library: {added} added, {merged} label update(s){suffix}"), StatusTone::Success);
    vec![Effect::PersistReamerLibrary]
}

/// Same shape as `handle_reamer_library_file_read`, for the Bushing
/// material library - additionally re-syncs `BushingModel::custom_materials`
/// from the merged library, since a leaked `&'static Material` can't be
/// mutated in place to reflect an overwrite (see
/// `BushingModel::sync_custom_materials_from_library`'s own doc comment).
fn handle_bushing_material_library_file_read(state: &mut AppState, result: Result<String, String>) -> Vec<Effect> {
    let text = match result {
        Ok(text) => text,
        Err(e) => {
            state.notifications.push(e, StatusTone::Danger);
            return Vec::new();
        }
    };
    let incoming = match crate::library::import_json::<crate::toolboxes::bushing::material_persistence::PersistedMaterial>(&text) {
        Ok(items) => items,
        Err(e) => {
            state.notifications.push(e, StatusTone::Danger);
            return Vec::new();
        }
    };
    let outcomes = crate::library::classify_import(&state.bushing.material_picker.library, incoming, |p| p.name.clone());
    let (queue, added, merged) = crate::library::ConflictQueue::new(outcomes, &mut state.bushing.material_picker.library);
    let conflicts_remaining = !queue.is_empty();
    state.bushing.material_picker.pending_conflicts = if conflicts_remaining { Some(queue) } else { None };
    state.bushing.model.sync_custom_materials_from_library(&state.bushing.material_picker.library);
    let suffix = if conflicts_remaining { " - conflicts need review (k: keep, o: overwrite, a/z: apply to all)" } else { "" };
    state.notifications.push(format!("Material library: {added} added, {merged} label update(s){suffix}"), StatusTone::Success);
    vec![Effect::PersistBushingMaterialLibrary]
}

/// Same shape as `handle_reamer_library_file_read`, for the Bushing ID
/// library - no combined-catalog re-sync needed (unlike materials), since
/// this library has no built-in catalog to merge against.
fn handle_bushing_id_library_file_read(state: &mut AppState, result: Result<String, String>) -> Vec<Effect> {
    let text = match result {
        Ok(text) => text,
        Err(e) => {
            state.notifications.push(e, StatusTone::Danger);
            return Vec::new();
        }
    };
    let incoming = match crate::library::import_json::<crate::toolboxes::bushing::bushing_id_persistence::PersistedBushingId>(&text) {
        Ok(items) => items,
        Err(e) => {
            state.notifications.push(e, StatusTone::Danger);
            return Vec::new();
        }
    };
    let outcomes = crate::library::classify_import(&state.bushing.bushing_id_picker.library, incoming, |p| p.label.clone());
    let (queue, added, merged) = crate::library::ConflictQueue::new(outcomes, &mut state.bushing.bushing_id_picker.library);
    let conflicts_remaining = !queue.is_empty();
    state.bushing.bushing_id_picker.pending_conflicts = if conflicts_remaining { Some(queue) } else { None };
    let suffix = if conflicts_remaining { " - conflicts need review (k: keep, o: overwrite, a/z: apply to all)" } else { "" };
    state.notifications.push(format!("Bushing ID library: {added} added, {merged} label update(s){suffix}"), StatusTone::Success);
    vec![Effect::PersistBushingIdLibrary]
}

fn handle_search_finished(state: &mut AppState, result: Result<SearchRunResult, OrchestratorError>) -> Vec<Effect> {
    state.search.run.is_running = false;
    state.search.run.elapsed = state.search.run.started.map(|s| s.elapsed());
    state.search.run.in_flight_files.clear();
    state.search.cancel_token = None;
    offer_index_update(state);

    match result {
        Ok(run_result) => {
            state.search.run.results_summary_text = search::model::summarize(&run_result.summary);
            state.notifications.push("Search finished", StatusTone::Success);
            // Opening the report needs the HTML file even if HTML export is off.
            let write_html = state.search.config.export_html || state.search.config.open_report_when_done;
            let settings = search::model::build_settings(&state.search.config);
            vec![Effect::WriteReport { settings, run_result, write_html }]
        }
        Err(OrchestratorError::Cancelled) => {
            state.search.run.status_text = "Cancelled.".to_string();
            state.notifications.push("Search cancelled", StatusTone::Info);
            Vec::new()
        }
        Err(e) => {
            state.search.run.status_text = format!("Error: {e}");
            state.notifications.push("Search failed", StatusTone::Danger);
            Vec::new()
        }
    }
}

/// If the search that just ended found the fast index out of date, asks
/// whether to update it. Deferred to the end of the run so a prompt never
/// interrupts a search in progress; never replaces an already-open modal.
fn offer_index_update(state: &mut AppState) {
    let Some((new_or_changed, removed)) = state.search.index_run.stale.take() else { return };
    if state.modal.is_open() || state.search.index_run.is_building {
        return;
    }
    let mut parts = Vec::new();
    if new_or_changed > 0 {
        parts.push(format!("{new_or_changed} new/changed"));
    }
    if removed > 0 {
        parts.push(format!("{removed} removed"));
    }
    state.modal = ModalState::Confirm(ConfirmDialog {
        title: "Fast index is out of date".into(),
        message: format!("{} file(s) since the index was built. Update the index now?", parts.join(", ")),
        on_confirm: ConfirmAction::UpdateIndex,
    });
}

fn request_quit(state: &mut AppState) {
    if state.is_busy() {
        state.modal = ModalState::Confirm(ConfirmDialog {
            title: "Quit?".into(),
            message: "A search is still running. Quit anyway?".into(),
            on_confirm: ConfirmAction::Quit,
        });
    } else {
        state.should_quit = true;
    }
}

fn handle_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    // Modal input takes priority over every global binding while open.
    if state.modal.is_open() {
        return handle_modal_key(state, key);
    }

    // Toolbox-local key routing (text-field editing, list navigation)
    // takes priority over global bindings while a workspace pane is
    // focused, so e.g. typing "quit" into the search path field never
    // triggers the global `q` quit binding. See `toolboxes::search::handle_key`.
    if let FocusArea::Workspace(pane) = state.focus.area {
        match state.nav.active_tool {
            ToolId::Search => {
                let (consumed, effects) =
                    search::handle_key(&mut state.search, &mut state.notifications, pane, key);
                if consumed {
                    return effects;
                }
            }
            ToolId::FastenerHole => {
                let (consumed, effects) = fastener_hole::handle_key(&mut state.fastener_hole, key);
                if consumed {
                    return effects;
                }
            }
            ToolId::PressureVessel => {
                let (consumed, effects) = pressure_vessel::handle_key(&mut state.pressure_vessel, key);
                if consumed {
                    return effects;
                }
            }
            ToolId::Bushing => {
                let (consumed, effects) = bushing::handle_key(&mut state.bushing, key);
                if consumed {
                    return effects;
                }
            }
            ToolId::PreloadAnalysis => {
                let (consumed, effects) = preload_analysis::handle_key(&mut state.preload_analysis, key);
                if consumed {
                    return effects;
                }
            }
            ToolId::EccentricBushing => {
                let (consumed, effects) = eccentric_bushing::handle_key(&mut state.eccentric, &state.bushing.model, key);
                if consumed {
                    return effects;
                }
            }
            ToolId::LugAnalysis => {
                let (consumed, effects) = lug_analysis::handle_key(&mut state.lug_analysis, key);
                if consumed {
                    return effects;
                }
            }
            ToolId::FeaWorkbench => {
                let (consumed, effects) = fea_workbench::handle_key(&mut state.fea_workbench, key);
                if consumed {
                    return effects;
                }
            }
            ToolId::MaterialLookup => {
                let (consumed, effects) = material_lookup::handle_key(&mut state.material_lookup, key);
                if consumed {
                    return effects;
                }
            }
            _ => {}
        }
    }

    match key.code {
        // `'p' | 'P'`, not a bare `'p'`: crossterm's Windows backend derives
        // the char's case from Shift XOR Caps Lock even for control-code
        // keys (see `get_char_for_key` in crossterm's
        // `event/sys/windows/parse.rs`), so with Caps Lock on, Ctrl+P
        // arrives as `Char('P')` with only `CONTROL` set - a bare `'p'`
        // silently drops the binding on Windows only (never reproduces on
        // macOS/Linux, where Ctrl+letter is always reported lowercase).
        // Same root cause, same fix shape as the Shift/Caps-Lock nav-key
        // bug already fixed in `search::is_plain_char` - that fix never
        // touched this global match block, which is why this one slipped
        // through.
        KeyCode::Char('p' | 'P') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.modal = ModalState::Palette(CommandPalette::default());
        }
        KeyCode::Char('?') => state.modal = ModalState::Help,
        KeyCode::Char('q' | 'Q') => request_quit(state),
        KeyCode::Tab => {
            let panes = state.workspace_pane_count();
            state.focus.cycle_forward(panes);
        }
        KeyCode::BackTab => {
            let panes = state.workspace_pane_count();
            state.focus.cycle_backward(panes);
        }
        KeyCode::Left | KeyCode::Right if state.focus.area == FocusArea::Rail => {
            let idx = ToolId::ALL.iter().position(|t| *t == state.nav.active_tool).unwrap_or(0);
            let len = ToolId::ALL.len();
            let step: i32 = if key.code == KeyCode::Left { -1 } else { 1 };
            // Skip past disabled entries to the nearest enabled one in the
            // pressed direction (wrapping), rather than stopping at the
            // first (possibly disabled) neighbor - `idx` is recomputed from
            // `state.nav.active_tool` every keypress, and `activate` is a
            // no-op on a disabled target, so landing on a disabled neighbor
            // once would otherwise permanently block reaching any enabled
            // tool beyond it (a real bug this exposed, back when `Bushing`
            // was still disabled: `PressureVessel` became unreachable via
            // Left/Right once enabled, since disabled `Bushing` sat
            // directly between it and `FastenerHole` in `ToolId::ALL` -
            // now all five migrated toolboxes are enabled, but the same
            // skip logic still matters for the remaining disabled
            // `Dupes`/`Rename`/`Logs` run at the end of the list).
            for offset in 1..=len {
                let next = ((idx as i32 + step * offset as i32).rem_euclid(len as i32)) as usize;
                if ToolId::ALL[next].enabled() {
                    state.nav.activate(ToolId::ALL[next]);
                    break;
                }
            }
        }
        _ => {}
    }
    Vec::new()
}

fn handle_modal_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    // Read before the `&mut state.modal` borrow below - `ToolId` is `Copy`,
    // so this is a cheap snapshot, not a lingering borrow conflict.
    let active_tool = state.nav.active_tool;
    match &mut state.modal {
        ModalState::Palette(palette) => match key.code {
            KeyCode::Esc => state.modal.close(),
            KeyCode::Enter => {
                let picked = palette.picked(active_tool);
                state.modal.close();
                if let Some(cmd) = picked {
                    return execute_command(state, cmd);
                }
            }
            KeyCode::Up => palette.move_selection(active_tool, -1),
            KeyCode::Down => palette.move_selection(active_tool, 1),
            KeyCode::Backspace => palette.backspace(),
            KeyCode::Char(c) => palette.push_char(c),
            _ => {}
        },
        ModalState::Help => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('?')) {
                state.modal.close();
            }
        }
        ModalState::Confirm(dialog) => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let action = dialog.on_confirm;
                state.modal.close();
                match action {
                    ConfirmAction::Quit => state.should_quit = true,
                    ConfirmAction::UpdateIndex => return search::start_index_build(&mut state.search),
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => state.modal.close(),
            _ => {}
        },
        ModalState::None => {}
    }
    Vec::new()
}

const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

fn synthetic_key(code: KeyCode) -> KeyEvent {
    KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

/// True if this click landed on the same logical target (not just the same
/// pixel) as the immediately preceding one, within `DOUBLE_CLICK_WINDOW` -
/// and always records this click as the new "last click" regardless.
/// Comparing by `ClickTarget` rather than raw coordinates means a human's
/// slightly-different click position on the same row still counts.
fn is_double_click(last_click: &mut Option<(Instant, ClickTarget)>, target: ClickTarget) -> bool {
    let now = Instant::now();
    let is_double =
        matches!(last_click, Some((t, last_target)) if *last_target == target && now.duration_since(*t) < DOUBLE_CLICK_WINDOW);
    *last_click = Some((now, target));
    is_double
}

/// Mouse event routing - sibling to `handle_key`, same `Effect`-returning
/// pure-function contract, called directly from `main.rs` for
/// `Event::Mouse` (see that module's doc comment for why this bypasses
/// `handle_event`). `regions` is the hit-test geometry the most recent
/// render published (`widgets::shell::draw` via `mouse::MouseRegions`).
pub fn handle_mouse(state: &mut AppState, regions: &MouseRegions, event: MouseEvent) -> Vec<Effect> {
    let (col, row) = (event.column, event.row);
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // A click closes the finished-run tooltip (a running one stays).
            state.bushing.edge_notice = None;
            handle_click(state, regions, col, row)
        }
        MouseEventKind::ScrollUp => {
            handle_scroll(state, regions, col, row, -1);
            Vec::new()
        }
        MouseEventKind::ScrollDown => {
            handle_scroll(state, regions, col, row, 1);
            Vec::new()
        }
        // Plain movement only drives the Bushing cross-check tooltips.
        MouseEventKind::Moved => {
            if state.nav.active_tool == ToolId::Bushing {
                state.bushing.edge_hover = match mouse::hit(&regions.bushing_actions, col, row) {
                    Some(bushing::BushingAction::EdgeInfo(topic)) => Some((topic, col, row)),
                    _ => None,
                };
            }
            Vec::new()
        }
        // Right/middle click and drag have no bound action in this phase -
        // explicitly ignored rather than falling through to an unrelated
        // handler.
        _ => Vec::new(),
    }
}

fn handle_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    // Modal input takes priority over every other target while open, same
    // as `handle_key`'s own priority order.
    if state.modal.is_open() {
        return handle_modal_click(state, regions, col, row);
    }

    // Toolbox-local overlay (not a global `ModalState`) - takes priority
    // over the rail/workspace behind it while open, same as a modal would.
    if state.search.extension_picker.open {
        if let Some(i) = mouse::hit(&regions.extension_rows, col, row) {
            state.search.extension_picker.cursor = i;
            search::extension_picker::handle_key(&mut state.search.extension_picker, synthetic_key(KeyCode::Char(' ')));
        }
        return Vec::new();
    }

    // Bushing Fixes window: modal for the mouse - only its own tabs, rows and
    // buttons react; a click anywhere else is swallowed.
    if state.nav.active_tool == ToolId::Bushing && state.bushing.advice.open {
        if let Some(action) = mouse::hit(&regions.bushing_actions, col, row) {
            if let bushing::BushingAction::AdviceRow(i) = action {
                let double = is_double_click(&mut state.last_click, ClickTarget::BushingAdviceRow(i));
                state.bushing.perform(action);
                if double {
                    return state.bushing.perform(bushing::BushingAction::AdviceApply);
                }
                return Vec::new();
            }
            return state.bushing.perform(action);
        }
        return Vec::new();
    }

    // Preload joint-template window: modal for the mouse.
    if state.nav.active_tool == ToolId::PreloadAnalysis && state.preload_analysis.template_picker.open {
        if let Some(action) = mouse::hit(&regions.template_actions, col, row) {
            let double = matches!(action, preload_analysis::template_picker::TemplateAction::Row(i) if is_double_click(&mut state.last_click, ClickTarget::TemplateRow(i)));
            let pa = &mut state.preload_analysis;
            let mut applied = pa.template_picker.perform(action, &mut pa.model);
            if double && applied.is_none() {
                applied = pa.template_picker.perform(preload_analysis::template_picker::TemplateAction::Apply, &mut pa.model);
            }
            pa.after_template(applied);
        }
        return Vec::new();
    }

    // The Lug Analysis material browser is modal for the mouse.
    if state.nav.active_tool == ToolId::LugAnalysis {
        if let Some(browser) = state.lug_analysis.material_browser.as_mut() {
            if let Some(i) = mouse::hit(&regions.material_lookup_rows, col, row) {
                browser.cursor = i;
                if is_double_click(&mut state.last_click, ClickTarget::MaterialLookupRow(i)) {
                    return lug_analysis::handle_key(&mut state.lug_analysis, synthetic_key(KeyCode::Enter)).1;
                }
            }
            return Vec::new();
        }
    }

    // The FEA Workbench material browser is modal for the mouse.
    if state.nav.active_tool == ToolId::FeaWorkbench {
        if let Some(browser) = state.fea_workbench.material_browser.as_mut() {
            if let Some(i) = mouse::hit(&regions.material_lookup_rows, col, row) {
                browser.cursor = i;
                if is_double_click(&mut state.last_click, ClickTarget::MaterialLookupRow(i)) {
                    return fea_workbench::handle_key(&mut state.fea_workbench, synthetic_key(KeyCode::Enter)).1;
                }
            }
            return Vec::new();
        }
    }

    if state.pressure_vessel.material_picker.open {
        if let Some(i) = mouse::hit(&regions.material_rows, col, row) {
            state.pressure_vessel.material_picker.cursor = i;
            if is_double_click(&mut state.last_click, ClickTarget::MaterialRow(i)) {
                let (_, effects) = pressure_vessel::material_picker::handle_key(&mut state.pressure_vessel.material_picker, &mut state.pressure_vessel.model, synthetic_key(KeyCode::Enter));
                return effects;
            }
        }
        return Vec::new();
    }

    if state.bushing.material_picker.open {
        if let Some(i) = mouse::hit(&regions.material_rows, col, row) {
            state.bushing.material_picker.cursor = i;
            if is_double_click(&mut state.last_click, ClickTarget::MaterialRow(i)) {
                let (_, effects) = bushing::material_picker::handle_key(&mut state.bushing.material_picker, &mut state.bushing.model, synthetic_key(KeyCode::Enter));
                return effects;
            }
        }
        return Vec::new();
    }

    if state.bushing.reamer_picker.open {
        if let Some(i) = mouse::hit(&regions.reamer_rows, col, row) {
            state.bushing.reamer_picker.cursor = i;
            if is_double_click(&mut state.last_click, ClickTarget::ReamerRow(i)) {
                let (_, effects) = bushing::reamer_picker::handle_key(&mut state.bushing.reamer_picker, &mut state.bushing.model, synthetic_key(KeyCode::Enter));
                return effects;
            }
        }
        return Vec::new();
    }

    if let Some(tool) = mouse::hit(&regions.rail, col, row) {
        state.nav.activate(tool);
        state.focus.area = FocusArea::Rail;
        return Vec::new();
    }

    match state.nav.active_tool {
        ToolId::Search => handle_search_click(state, regions, col, row),
        ToolId::FastenerHole => handle_fastener_click(state, regions, col, row),
        ToolId::PressureVessel => handle_pressure_vessel_click(state, regions, col, row),
        ToolId::Bushing => handle_bushing_click(state, regions, col, row),
        ToolId::PreloadAnalysis => handle_preload_analysis_click(state, regions, col, row),
        ToolId::EccentricBushing => handle_eccentric_click(state, regions, col, row),
        ToolId::LugAnalysis => handle_lug_analysis_click(state, regions, col, row),
        ToolId::FeaWorkbench => handle_fea_workbench_click(state, regions, col, row),
        ToolId::MaterialLookup => handle_material_lookup_click(state, regions, col, row),
        _ => Vec::new(),
    }
}

fn handle_modal_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    match &state.modal {
        ModalState::Palette(_) => {
            let Some(i) = mouse::hit(&regions.palette_rows, col, row) else {
                return Vec::new();
            };
            if let ModalState::Palette(palette) = &mut state.modal {
                palette.selected = i;
            }
            // A command palette is menu-like - a single click executes it,
            // the same as pressing Enter on the already-selected row.
            handle_modal_key(state, synthetic_key(KeyCode::Enter))
        }
        ModalState::Confirm(_) => {
            if regions.confirm_yes.is_some_and(|r| mouse::contains(r, col, row)) {
                return handle_modal_key(state, synthetic_key(KeyCode::Char('y')));
            }
            if regions.confirm_no.is_some_and(|r| mouse::contains(r, col, row)) {
                return handle_modal_key(state, synthetic_key(KeyCode::Char('n')));
            }
            Vec::new()
        }
        ModalState::Help => {
            // No interactive elements inside - any click anywhere closes it,
            // same as Esc/`?`.
            if regions.help_overlay.is_some_and(|r| mouse::contains(r, col, row)) {
                return handle_modal_key(state, synthetic_key(KeyCode::Esc));
            }
            Vec::new()
        }
        ModalState::None => Vec::new(),
    }
}

fn handle_search_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    match state.search.screen {
        search::ToolboxScreen::Run => {
            if let Some(i) = mouse::hit(&regions.results_rows, col, row) {
                state.focus.area = FocusArea::Workspace(search::PANE_RESULTS);
                state.search.selected_result = i;
                if is_double_click(&mut state.last_click, ClickTarget::ResultRow(i)) {
                    let (_, effects) = search::handle_key(
                        &mut state.search,
                        &mut state.notifications,
                        search::PANE_RESULTS,
                        synthetic_key(KeyCode::Enter),
                    );
                    return effects;
                }
                return Vec::new();
            }
            if let Some(pane) = mouse::hit(&regions.workspace_panes, col, row) {
                state.focus.area = FocusArea::Workspace(pane);
            }
            Vec::new()
        }
        search::ToolboxScreen::Settings => handle_settings_click(state, regions, col, row),
    }
}

fn handle_settings_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    use crate::toolboxes::search::settings_view::Section;

    if let Some(i) = mouse::hit(&regions.settings_field_rows, col, row) {
        state.search.settings.section = Section::Fields;
        state.search.settings.selected = i;
        state.search.settings.clamp_selection();
        if is_double_click(&mut state.last_click, ClickTarget::SettingsFieldRow(i)) {
            let (_, effects) = search::settings_view::handle_key(
                &mut state.search.settings,
                &mut state.search.config,
                &state.search.recent_searches,
                &mut state.search.saved_presets,
                synthetic_key(KeyCode::Enter),
            );
            return effects;
        }
        return Vec::new();
    }
    if let Some(i) = mouse::hit(&regions.recents_rows, col, row) {
        state.search.settings.section = Section::Recents;
        state.search.settings.recent_selected = i;
        if is_double_click(&mut state.last_click, ClickTarget::RecentRow(i)) {
            let (_, effects) = search::settings_view::handle_key(
                &mut state.search.settings,
                &mut state.search.config,
                &state.search.recent_searches,
                &mut state.search.saved_presets,
                synthetic_key(KeyCode::Enter),
            );
            return effects;
        }
        return Vec::new();
    }
    if let Some(i) = mouse::hit(&regions.presets_rows, col, row) {
        state.search.settings.section = Section::Presets;
        state.search.settings.preset_selected = i;
        if is_double_click(&mut state.last_click, ClickTarget::PresetRow(i)) {
            let (_, effects) = search::settings_view::handle_key(
                &mut state.search.settings,
                &mut state.search.config,
                &state.search.recent_searches,
                &mut state.search.saved_presets,
                synthetic_key(KeyCode::Enter),
            );
            return effects;
        }
        return Vec::new();
    }
    if let Some(section) = mouse::hit(&regions.settings_section_panes, col, row) {
        state.search.settings.section = section;
    }
    Vec::new()
}

fn handle_fastener_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    if let Some(i) = mouse::hit(&regions.fastener_rows, col, row) {
        state.focus.area = FocusArea::Workspace(fastener_hole::PANE_MAIN);
        state.fastener_hole.selected = i;
        state.fastener_hole.clamp_selection();
        if is_double_click(&mut state.last_click, ClickTarget::FastenerRow(i)) {
            let (_, effects) = fastener_hole::handle_key(&mut state.fastener_hole, synthetic_key(KeyCode::Enter));
            return effects;
        }
        return Vec::new();
    }
    if let Some(pane) = mouse::hit(&regions.workspace_panes, col, row) {
        state.focus.area = FocusArea::Workspace(pane);
    }
    Vec::new()
}

fn handle_pressure_vessel_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    if let Some(i) = mouse::hit(&regions.pressure_vessel_rows, col, row) {
        state.focus.area = FocusArea::Workspace(pressure_vessel::PANE_MAIN);
        state.pressure_vessel.selected = i;
        state.pressure_vessel.clamp_selection();
        if is_double_click(&mut state.last_click, ClickTarget::PressureVesselRow(i)) {
            let (_, effects) = pressure_vessel::handle_key(&mut state.pressure_vessel, synthetic_key(KeyCode::Enter));
            return effects;
        }
        return Vec::new();
    }
    if let Some(pane) = mouse::hit(&regions.workspace_panes, col, row) {
        state.focus.area = FocusArea::Workspace(pane);
    }
    Vec::new()
}

fn handle_bushing_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    // Results pane: action-bar buttons and flagged check lines.
    if let Some(action) = mouse::hit(&regions.bushing_actions, col, row) {
        state.focus.area = FocusArea::Workspace(bushing::PANE_MAIN);
        return state.bushing.perform(action);
    }
    if let Some(i) = mouse::hit(&regions.bushing_rows, col, row) {
        state.focus.area = FocusArea::Workspace(bushing::PANE_MAIN);
        state.bushing.selected = i;
        state.bushing.clamp_selection();
        // A section header opens and closes on a single click.
        if bushing::model::field_rows(&state.bushing.model).get(state.bushing.selected) == Some(&bushing::model::FieldRow::AdvancedSection) {
            return bushing::handle_key(&mut state.bushing, synthetic_key(KeyCode::Enter)).1;
        }
        if is_double_click(&mut state.last_click, ClickTarget::BushingRow(i)) {
            let (_, effects) = bushing::handle_key(&mut state.bushing, synthetic_key(KeyCode::Enter));
            return effects;
        }
        return Vec::new();
    }
    if let Some(pane) = mouse::hit(&regions.workspace_panes, col, row) {
        state.focus.area = FocusArea::Workspace(pane);
    }
    Vec::new()
}

fn handle_preload_analysis_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    if let Some(i) = mouse::hit(&regions.preload_analysis_rows, col, row) {
        state.focus.area = FocusArea::Workspace(preload_analysis::PANE_MAIN);
        state.preload_analysis.selected = i;
        state.preload_analysis.clamp_selection();
        // A section header opens and closes on a single click.
        if preload_analysis::model::field_rows(&state.preload_analysis.model).get(state.preload_analysis.selected) == Some(&preload_analysis::model::FieldRow::AdvancedSection) {
            return preload_analysis::handle_key(&mut state.preload_analysis, synthetic_key(KeyCode::Enter)).1;
        }
        if is_double_click(&mut state.last_click, ClickTarget::PreloadAnalysisRow(i)) {
            let (_, effects) = preload_analysis::handle_key(&mut state.preload_analysis, synthetic_key(KeyCode::Enter));
            return effects;
        }
        return Vec::new();
    }
    if let Some(pane) = mouse::hit(&regions.workspace_panes, col, row) {
        state.focus.area = FocusArea::Workspace(pane);
    }
    Vec::new()
}

fn handle_eccentric_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    if let Some(i) = mouse::hit(&regions.eccentric_rows, col, row) {
        state.focus.area = FocusArea::Workspace(eccentric_bushing::PANE_MAIN);
        state.eccentric.selected = i;
        state.eccentric.clamp_selection();
        // A section header opens and closes on a single click.
        if state.eccentric.selected_row() == Some(eccentric_bushing::model::FieldRow::AdvancedSection) {
            return eccentric_bushing::handle_key(&mut state.eccentric, &state.bushing.model, synthetic_key(KeyCode::Enter)).1;
        }
        if is_double_click(&mut state.last_click, ClickTarget::EccentricRow(i)) {
            return eccentric_bushing::handle_key(&mut state.eccentric, &state.bushing.model, synthetic_key(KeyCode::Enter)).1;
        }
        return Vec::new();
    }
    if let Some(pane) = mouse::hit(&regions.workspace_panes, col, row) {
        state.focus.area = FocusArea::Workspace(pane);
    }
    Vec::new()
}

fn handle_lug_analysis_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    if let Some(i) = mouse::hit(&regions.lug_analysis_rows, col, row) {
        state.focus.area = FocusArea::Workspace(lug_analysis::PANE_MAIN);
        state.lug_analysis.selected = i;
        state.lug_analysis.clamp_selection();
        // A section header opens and closes on a single click.
        if matches!(state.lug_analysis.selected_row_for_click(), Some(lug_analysis::model::FieldRow::MeshSection)) {
            return lug_analysis::handle_key(&mut state.lug_analysis, synthetic_key(KeyCode::Enter)).1;
        }
        if is_double_click(&mut state.last_click, ClickTarget::LugAnalysisRow(i)) {
            return lug_analysis::handle_key(&mut state.lug_analysis, synthetic_key(KeyCode::Enter)).1;
        }
        return Vec::new();
    }
    if let Some(pane) = mouse::hit(&regions.workspace_panes, col, row) {
        state.focus.area = FocusArea::Workspace(pane);
    }
    Vec::new()
}

fn handle_fea_workbench_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    if let Some(i) = mouse::hit(&regions.fea_workbench_rows, col, row) {
        state.focus.area = FocusArea::Workspace(fea_workbench::PANE_MAIN);
        state.fea_workbench.selected = i;
        state.fea_workbench.clamp_selection();
        if is_double_click(&mut state.last_click, ClickTarget::FeaWorkbenchRow(i)) {
            return fea_workbench::handle_key(&mut state.fea_workbench, synthetic_key(KeyCode::Enter)).1;
        }
        return Vec::new();
    }
    if let Some(pane) = mouse::hit(&regions.workspace_panes, col, row) {
        state.focus.area = FocusArea::Workspace(pane);
    }
    Vec::new()
}

fn handle_material_lookup_click(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16) -> Vec<Effect> {
    if let Some(i) = mouse::hit(&regions.material_lookup_rows, col, row) {
        state.focus.area = FocusArea::Workspace(material_lookup::PANE_MAIN);
        state.material_lookup.cursor = i.min(state.material_lookup.hits.len().saturating_sub(1));
        if is_double_click(&mut state.last_click, ClickTarget::MaterialLookupRow(i)) {
            state.material_lookup.toggle_mark();
        }
        return Vec::new();
    }
    if let Some(pane) = mouse::hit(&regions.workspace_panes, col, row) {
        state.focus.area = FocusArea::Workspace(pane);
    }
    Vec::new()
}

/// Scroll wheel routing: moves whatever list the cursor is currently over
/// by one row, reusing each toolbox's existing Up/Down keyboard handling
/// verbatim (via a synthetic key) rather than re-deriving the same
/// wrap/clamp logic a third time. Never produces an `Effect` - none of the
/// reused Up/Down arms do either.
fn handle_scroll(state: &mut AppState, regions: &MouseRegions, col: u16, row: u16, delta: i32) {
    let key = synthetic_key(if delta < 0 { KeyCode::Up } else { KeyCode::Down });

    if state.modal.is_open() {
        let active_tool = state.nav.active_tool;
        if let ModalState::Palette(palette) = &mut state.modal {
            if mouse::hit(&regions.palette_rows, col, row).is_some() {
                palette.move_selection(active_tool, delta);
            }
        }
        return;
    }

    if state.search.extension_picker.open {
        if mouse::hit(&regions.extension_rows, col, row).is_some() {
            search::extension_picker::handle_key(&mut state.search.extension_picker, key);
        }
        return;
    }

    if state.nav.active_tool == ToolId::PreloadAnalysis && state.preload_analysis.template_picker.open {
        if regions.template_window.map(|r| mouse::contains(r, col, row)).unwrap_or(false) {
            preload_analysis::handle_key(&mut state.preload_analysis, key);
        }
        return;
    }

    if state.nav.active_tool == ToolId::Bushing && state.bushing.advice.open {
        if regions.advice_window.map(|r| mouse::contains(r, col, row)).unwrap_or(false) {
            bushing::handle_key(&mut state.bushing, key);
        }
        return;
    }

    if state.nav.active_tool == ToolId::LugAnalysis {
        if let Some(browser) = state.lug_analysis.material_browser.as_mut() {
            if mouse::hit(&regions.material_lookup_rows, col, row).is_some() {
                material_lookup::handle_key(browser, key);
            }
            return;
        }
    }
    if state.nav.active_tool == ToolId::FeaWorkbench {
        if let Some(browser) = state.fea_workbench.material_browser.as_mut() {
            if mouse::hit(&regions.material_lookup_rows, col, row).is_some() {
                material_lookup::handle_key(browser, key);
            }
            return;
        }
    }

    if state.pressure_vessel.material_picker.open {
        if mouse::hit(&regions.material_rows, col, row).is_some() {
            pressure_vessel::material_picker::handle_key(&mut state.pressure_vessel.material_picker, &mut state.pressure_vessel.model, key);
        }
        return;
    }

    if state.bushing.material_picker.open {
        if mouse::hit(&regions.material_rows, col, row).is_some() {
            bushing::material_picker::handle_key(&mut state.bushing.material_picker, &mut state.bushing.model, key);
        }
        return;
    }

    if state.bushing.reamer_picker.open {
        if mouse::hit(&regions.reamer_rows, col, row).is_some() {
            bushing::reamer_picker::handle_key(&mut state.bushing.reamer_picker, &mut state.bushing.model, key);
        }
        return;
    }

    match state.nav.active_tool {
        ToolId::Search => {
            if mouse::hit(&regions.results_rows, col, row).is_some() {
                search::handle_key(&mut state.search, &mut state.notifications, search::PANE_RESULTS, key);
            } else if mouse::hit(&regions.settings_field_rows, col, row).is_some()
                || mouse::hit(&regions.recents_rows, col, row).is_some()
                || mouse::hit(&regions.presets_rows, col, row).is_some()
            {
                search::settings_view::handle_key(
                    &mut state.search.settings,
                    &mut state.search.config,
                    &state.search.recent_searches,
                    &mut state.search.saved_presets,
                    key,
                );
            }
        }
        ToolId::FastenerHole => {
            if mouse::hit(&regions.fastener_rows, col, row).is_some() {
                fastener_hole::handle_key(&mut state.fastener_hole, key);
            }
        }
        ToolId::PressureVessel => {
            if mouse::hit(&regions.pressure_vessel_rows, col, row).is_some() {
                pressure_vessel::handle_key(&mut state.pressure_vessel, key);
            }
        }
        ToolId::Bushing => {
            if mouse::hit(&regions.bushing_rows, col, row).is_some() {
                bushing::handle_key(&mut state.bushing, key);
            } else if regions.bushing_results.map(|r| mouse::contains(r, col, row)).unwrap_or(false) {
                // Wheel over the Results readout scrolls it (3 rows a notch).
                let step = 3u16;
                state.bushing.results_scroll = if delta < 0 { state.bushing.results_scroll.saturating_sub(step) } else { state.bushing.results_scroll.saturating_add(step) };
            }
        }
        ToolId::PreloadAnalysis => {
            if mouse::hit(&regions.preload_analysis_rows, col, row).is_some() {
                preload_analysis::handle_key(&mut state.preload_analysis, key);
            }
        }
        ToolId::EccentricBushing => {
            if mouse::hit(&regions.eccentric_rows, col, row).is_some() {
                eccentric_bushing::handle_key(&mut state.eccentric, &state.bushing.model, key);
            }
        }
        ToolId::LugAnalysis => {
            if mouse::hit(&regions.lug_analysis_rows, col, row).is_some() {
                lug_analysis::handle_key(&mut state.lug_analysis, key);
            }
        }
        ToolId::FeaWorkbench => {
            if mouse::hit(&regions.fea_workbench_rows, col, row).is_some() {
                fea_workbench::handle_key(&mut state.fea_workbench, key);
            }
        }
        ToolId::MaterialLookup => {
            if mouse::hit(&regions.material_lookup_rows, col, row).is_some() {
                material_lookup::handle_key(&mut state.material_lookup, key);
            }
        }
        _ => {}
    }
}

fn execute_command(state: &mut AppState, cmd: Command) -> Vec<Effect> {
    match cmd {
        Command::SwitchToSearch => {
            state.nav.activate(ToolId::Search);
            Vec::new()
        }
        Command::SwitchToFastenerHole => {
            state.nav.activate(ToolId::FastenerHole);
            Vec::new()
        }
        Command::SwitchToBushing => {
            state.nav.activate(ToolId::Bushing);
            Vec::new()
        }
        Command::SwitchToPressureVessel => {
            state.nav.activate(ToolId::PressureVessel);
            Vec::new()
        }
        Command::SwitchToPreloadAnalysis => {
            state.nav.activate(ToolId::PreloadAnalysis);
            Vec::new()
        }
        Command::SwitchToEccentricBushing => {
            state.nav.activate(ToolId::EccentricBushing);
            Vec::new()
        }
        Command::SwitchToLugAnalysis => {
            state.nav.activate(ToolId::LugAnalysis);
            Vec::new()
        }
        Command::SwitchToFeaWorkbench => {
            state.nav.activate(ToolId::FeaWorkbench);
            Vec::new()
        }
        Command::SolveFeaWorkbench => state.fea_workbench.start_solve(),
        Command::ExportFeaWorkbenchReport => fea_workbench::handle_key(&mut state.fea_workbench, synthetic_key(KeyCode::Char('e'))).1,
        Command::ExportFeaWorkbenchVtu => fea_workbench::handle_key(&mut state.fea_workbench, synthetic_key(KeyCode::Char('p'))).1,
        Command::SaveFeaWorkbenchProblem => fea_workbench::handle_key(&mut state.fea_workbench, synthetic_key(KeyCode::Char('j'))).1,
        Command::SwitchToMaterialLookup => {
            state.nav.activate(ToolId::MaterialLookup);
            Vec::new()
        }
        Command::SwitchToDupes => {
            state.nav.activate(ToolId::Dupes);
            Vec::new()
        }
        Command::SwitchToRename => {
            state.nav.activate(ToolId::Rename);
            Vec::new()
        }
        Command::SwitchToLogs => {
            state.nav.activate(ToolId::Logs);
            Vec::new()
        }
        Command::ToggleTheme => {
            state.theme = if state.theme.reduced_color {
                Theme::default_palette()
            } else {
                Theme::reduced_palette()
            };
            Vec::new()
        }
        Command::RunSearch => {
            state.search.screen = search::ToolboxScreen::Run;
            search::run_or_notify(&mut state.search, &mut state.notifications)
        }
        Command::CancelSearch => {
            state.search.screen = search::ToolboxScreen::Run;
            search::request_cancel(&state.search);
            Vec::new()
        }
        Command::OpenReport => match state.search.run.last_report_path.clone() {
            Some(path) => vec![Effect::OpenPath(path)],
            None => {
                state.notifications.push("No report yet", StatusTone::Info);
                Vec::new()
            }
        },
        Command::FocusPathField => {
            state.nav.activate(ToolId::Search);
            state.search.screen = search::ToolboxScreen::Run;
            state.focus.area = FocusArea::Workspace(search::PANE_PATH);
            Vec::new()
        }
        Command::ClearRecentSearches => {
            state.search.recent_searches.clear();
            vec![Effect::PersistSearchSettings]
        }
        Command::ToggleFastReSearchIndex => {
            state.search.config.index.enabled = !state.search.config.index.enabled;
            let tone = if state.search.config.index.enabled { StatusTone::Success } else { StatusTone::Info };
            let message = if state.search.config.index.enabled { "Fast re-search index enabled" } else { "Fast re-search index disabled" };
            state.notifications.push(message, tone);
            Vec::new()
        }
        Command::BuildIndex => search::start_index_build(&mut state.search),
        Command::CancelIndexBuild => {
            if let Some(token) = &state.search.index_cancel {
                token.cancel();
                state.notifications.push("Cancelling index build…", StatusTone::Info);
            } else {
                state.notifications.push("No index build is running", StatusTone::Info);
            }
            Vec::new()
        }
        Command::OpenDebugLog => match crate::debug_log::path() {
            Some(path) => vec![Effect::OpenPath(path.to_string_lossy().into_owned())],
            None => {
                state.notifications.push("Debug log is disabled (TOOLBENCH_DEBUG=0) or not writable", StatusTone::Warning);
                Vec::new()
            }
        },
        Command::ExportFastenerHoleReport => {
            vec![Effect::ExportFastenerHoleReport(fastener_hole::view::build_report_text(&state.fastener_hole.model))]
        }
        Command::ToggleBushingNumbersPanel => {
            state.bushing.show_numbers = !state.bushing.show_numbers;
            Vec::new()
        }
        Command::ExportBushingReport => {
            vec![Effect::ExportBushingReport(state.bushing.report_text())]
        }
        Command::OpenReamerPicker => {
            state.bushing.reamer_picker = bushing::reamer_picker::ReamerPickerState::open_near(&state.bushing.model);
            Vec::new()
        }
        Command::OpenHousingMaterialPicker => {
            state.bushing.material_picker = bushing::material_picker::MaterialPickerState::open_for(bushing::material_picker::MaterialTarget::Housing);
            Vec::new()
        }
        Command::OpenBushingMaterialPicker => {
            state.bushing.material_picker = bushing::material_picker::MaterialPickerState::open_for(bushing::material_picker::MaterialTarget::Bushing);
            Vec::new()
        }
        Command::TogglePressureVesselNumbersPanel => {
            state.pressure_vessel.show_numbers = !state.pressure_vessel.show_numbers;
            Vec::new()
        }
        Command::ExportPressureVesselReport => {
            vec![Effect::ExportPressureVesselReport(pressure_vessel::view::build_report_text(&state.pressure_vessel.model))]
        }
        Command::OpenPressureVesselMaterialPicker => {
            state.pressure_vessel.material_picker = pressure_vessel::material_picker::MaterialPickerState::open_now();
            Vec::new()
        }
        Command::ToggleLugAnalysisProfile => {
            state.lug_analysis.show_numbers = !state.lug_analysis.show_numbers;
            Vec::new()
        }
        Command::ExportLugAnalysisReport => lug_analysis::handle_key(&mut state.lug_analysis, synthetic_key(KeyCode::Char('e'))).1,
        Command::OpenLugMaterialBrowser => {
            let mut browser = MaterialLookupState { picking: true, ..MaterialLookupState::default() };
            if let Some(p) = browser.hits.iter().position(|&i| i == state.lug_analysis.model.material_index) {
                browser.cursor = p;
            }
            state.lug_analysis.browser_target = lug_analysis::PickTarget::Lug;
            state.lug_analysis.material_browser = Some(browser);
            Vec::new()
        }
        Command::ToggleMaterialLookupCompare => {
            state.material_lookup.compare = !state.material_lookup.compare;
            Vec::new()
        }
        Command::ExportMaterialLookupReport => material_lookup::handle_key(&mut state.material_lookup, synthetic_key(KeyCode::F(3))).1,
        Command::TogglePreloadAnalysisNumbersPanel => {
            state.preload_analysis.show_numbers = !state.preload_analysis.show_numbers;
            Vec::new()
        }
        Command::ExportPreloadAnalysisReport => {
            vec![Effect::ExportPreloadAnalysisReport(preload_analysis::view::build_report_text(&state.preload_analysis.model, &state.preload_analysis.fe))]
        }
        Command::OpenBoltPicker => {
            state.preload_analysis.bolt_picker = preload_analysis::bolt_picker::BoltPickerState::open_for(&state.preload_analysis.model);
            Vec::new()
        }
        Command::Quit => {
            request_quit(state);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn press(code: KeyCode) -> AppEvent {
        AppEvent::Terminal(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn press_with(code: KeyCode, modifiers: KeyModifiers) -> AppEvent {
        let mut key = KeyEvent::new(code, modifiers);
        key.kind = KeyEventKind::Press;
        AppEvent::Terminal(Event::Key(key))
    }

    #[test]
    fn ctrl_p_opens_palette_and_esc_closes_it() {
        let mut state = AppState::default();
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(matches!(state.modal, ModalState::Palette(_)));
        handle_event(&mut state, press(KeyCode::Esc));
        assert!(!state.modal.is_open());
    }

    #[test]
    fn command_palette_is_toolbox_scoped_end_to_end() {
        // Regression guard for the Ctrl+P palette used to show every
        // Search-only command regardless of the active toolbox - see
        // `Command::scope`/`CommandPalette::matches`'s own doc comments.
        let mut state = AppState::default();
        state.nav.activate(ToolId::Bushing);
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        let ModalState::Palette(palette) = &state.modal else { panic!("expected the palette to be open") };
        let matches = palette.matches(ToolId::Bushing);
        assert!(matches.contains(&Command::ToggleBushingNumbersPanel));
        assert!(!matches.contains(&Command::RunSearch), "a Search-only command must not appear while Bushing is active");
    }

    #[test]
    fn executing_a_toolbox_scoped_command_via_the_palette_applies_it() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Bushing);
        assert!(!state.bushing.show_numbers);
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        for c in "toggle numbers".chars() {
            handle_event(&mut state, press(KeyCode::Char(c)));
        }
        handle_event(&mut state, press(KeyCode::Enter));
        assert!(!state.modal.is_open());
        assert!(state.bushing.show_numbers);
    }

    #[test]
    fn q_quits_immediately_when_not_busy() {
        let mut state = AppState::default();
        handle_event(&mut state, press(KeyCode::Char('q')));
        assert!(state.should_quit);
    }

    #[test]
    fn ctrl_p_opens_palette_when_caps_lock_reports_uppercase_p() {
        // Regression test: crossterm's Windows backend derives the char's
        // case from Shift XOR Caps Lock even for Ctrl+<letter> combos, so
        // with Caps Lock on, Ctrl+P arrives as `Char('P')` with only
        // `CONTROL` set (never `SHIFT`). This previously fell through to
        // the catch-all `_ => {}` arm and silently did nothing on Windows.
        let mut state = AppState::default();
        handle_event(&mut state, press_with(KeyCode::Char('P'), KeyModifiers::CONTROL));
        assert!(matches!(state.modal, ModalState::Palette(_)));
    }

    #[test]
    fn q_quits_when_caps_lock_reports_uppercase_q() {
        let mut state = AppState::default();
        handle_event(&mut state, press(KeyCode::Char('Q')));
        assert!(state.should_quit);
    }

    #[test]
    fn rail_right_skips_past_disabled_tools_to_reach_the_next_enabled_one() {
        // Regression test: `ToolId::ALL` ends with the enabled MaterialLookup
        // followed by Dupes/Rename/Logs (disabled), then wraps back to Search.
        // Pressing Right from MaterialLookup must land on Search, skipping the three disabled
        // entries between them - not get permanently stuck re-selecting one
        // of them on every subsequent Right press (the bug this test
        // guards against: `activate` is a no-op on a disabled target, and
        // `idx` is recomputed from `active_tool` each keypress, so a naive
        // next-neighbor-only step can never progress past a disabled run).
        let mut state = AppState::default();
        state.nav.activate(ToolId::MaterialLookup);
        state.focus.area = FocusArea::Rail;
        handle_event(&mut state, press(KeyCode::Right));
        assert_eq!(state.nav.active_tool, ToolId::Search);
        // Left from Search must walk backward through the same disabled
        // run and land on MaterialLookup, proving the skip works in both
        // directions, not just forward.
        handle_event(&mut state, press(KeyCode::Left));
        assert_eq!(state.nav.active_tool, ToolId::MaterialLookup);
    }

    #[test]
    fn rail_left_from_search_wraps_around_to_the_last_enabled_tool() {
        let mut state = AppState::default();
        state.focus.area = FocusArea::Rail;
        handle_event(&mut state, press(KeyCode::Left));
        assert_eq!(state.nav.active_tool, ToolId::MaterialLookup, "Dupes/Rename/Logs are disabled, so wrapping left from Search lands on the last enabled tool");
    }

    #[test]
    fn q_asks_for_confirmation_when_a_search_is_running() {
        let mut state = AppState::default();
        state.search.run.is_running = true;
        handle_event(&mut state, press(KeyCode::Char('q')));
        assert!(!state.should_quit);
        assert!(matches!(state.modal, ModalState::Confirm(_)));
        handle_event(&mut state, press(KeyCode::Char('y')));
        assert!(state.should_quit);
    }

    #[test]
    fn confirm_dialog_n_cancels_without_quitting() {
        let mut state = AppState::default();
        state.search.run.is_running = true;
        handle_event(&mut state, press(KeyCode::Char('q')));
        handle_event(&mut state, press(KeyCode::Char('n')));
        assert!(!state.should_quit);
        assert!(!state.modal.is_open());
    }

    #[test]
    fn help_toggles_open_and_closed() {
        let mut state = AppState::default();
        handle_event(&mut state, press(KeyCode::Char('?')));
        assert!(matches!(state.modal, ModalState::Help));
        handle_event(&mut state, press(KeyCode::Char('?')));
        assert!(!state.modal.is_open());
    }

    #[test]
    fn picking_switch_to_search_from_palette_activates_it() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        for c in "switch to: search".chars() {
            handle_event(&mut state, press(KeyCode::Char(c)));
        }
        handle_event(&mut state, press(KeyCode::Enter));
        assert!(!state.modal.is_open());
        assert_eq!(state.nav.active_tool, ToolId::Search);
    }

    #[test]
    fn run_search_command_without_a_path_shows_a_warning_toast_not_a_crash() {
        let mut state = AppState::default();
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        for c in "run search".chars() {
            handle_event(&mut state, press(KeyCode::Char(c)));
        }
        let effects = handle_event(&mut state, press(KeyCode::Enter));
        assert!(effects.is_empty());
        assert_eq!(state.notifications.visible().len(), 1);
    }

    #[test]
    fn typing_a_path_never_triggers_the_global_quit_binding() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        state.focus.area = FocusArea::Workspace(search::PANE_PATH);
        for c in "quit".chars() {
            handle_event(&mut state, press(KeyCode::Char(c)));
        }
        assert!(!state.should_quit);
        assert_eq!(state.search.config.search_path, "quit");
    }

    #[test]
    fn enter_on_path_field_with_a_path_starts_a_search() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        state.focus.area = FocusArea::Workspace(search::PANE_PATH);
        state.search.config.search_path = "/tmp".to_string();
        let effects = handle_event(&mut state, press(KeyCode::Enter));
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::StartSearch { .. }));
        assert!(state.search.run.is_running);
    }

    #[test]
    fn ctrl_p_still_opens_the_palette_while_a_text_field_is_focused() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        state.focus.area = FocusArea::Workspace(search::PANE_PATH);
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(matches!(state.modal, ModalState::Palette(_)));
        // And it must not have leaked a stray "p" into the path field.
        assert!(state.search.config.search_path.is_empty());
    }

    #[test]
    fn search_progress_event_updates_run_state() {
        let mut state = AppState::default();
        handle_event(
            &mut state,
            AppEvent::SearchProgress(search_core::models::SearchProgressReport {
                files_completed: 1,
                total_files: 2,
                ..Default::default()
            }),
        );
        assert_eq!(state.search.run.progress_percent, 50.0);
    }

    #[test]
    fn search_finished_ok_clears_running_and_requests_a_report_write() {
        let mut state = AppState::default();
        state.search.run.is_running = true;
        let effects = handle_event(&mut state, AppEvent::SearchFinished(Ok(SearchRunResult::default())));
        assert!(!state.search.run.is_running);
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::WriteReport { .. }));
    }

    #[test]
    fn search_finished_cancelled_does_not_request_a_report_write() {
        let mut state = AppState::default();
        state.search.run.is_running = true;
        let effects = handle_event(&mut state, AppEvent::SearchFinished(Err(OrchestratorError::Cancelled)));
        assert!(!state.search.run.is_running);
        assert!(effects.is_empty());
        assert_eq!(state.search.run.status_text, "Cancelled.");
    }

    #[test]
    fn extensions_scanned_ok_opens_the_picker_with_the_results() {
        let mut state = AppState::default();
        let effects = handle_event(&mut state, AppEvent::ExtensionsScanned(Ok(vec![".txt".to_string(), ".rs".to_string()])));
        assert!(effects.is_empty());
        assert!(state.search.extension_picker.open);
        assert_eq!(state.search.extension_picker.available, vec![".txt".to_string(), ".rs".to_string()]);
    }

    #[test]
    fn extensions_scanned_appends_recent_extensions_not_found_by_the_scan() {
        let mut state = AppState::default();
        state.search.recent_extensions = vec![".pdf".to_string()];
        handle_event(&mut state, AppEvent::ExtensionsScanned(Ok(vec![".txt".to_string()])));
        assert_eq!(state.search.extension_picker.available, vec![".txt".to_string(), ".pdf".to_string()]);
    }

    #[test]
    fn extensions_scanned_does_not_duplicate_a_recent_extension_already_in_the_scan() {
        let mut state = AppState::default();
        state.search.recent_extensions = vec![".txt".to_string()];
        handle_event(&mut state, AppEvent::ExtensionsScanned(Ok(vec![".txt".to_string(), ".rs".to_string()])));
        assert_eq!(state.search.extension_picker.available, vec![".txt".to_string(), ".rs".to_string()]);
    }

    #[test]
    fn extensions_scanned_err_opens_the_picker_in_an_error_state() {
        let mut state = AppState::default();
        handle_event(&mut state, AppEvent::ExtensionsScanned(Err("not a directory".to_string())));
        assert!(state.search.extension_picker.open);
        assert_eq!(state.search.extension_picker.scan_error.as_deref(), Some("not a directory"));
    }

    #[test]
    fn toggle_fast_reindex_command_flips_the_setting() {
        let mut state = AppState::default();
        assert!(!state.search.config.index.enabled);
        handle_event(&mut state, press_with(KeyCode::Char('p'), KeyModifiers::CONTROL));
        for c in "toggle fast".chars() {
            handle_event(&mut state, press(KeyCode::Char(c)));
        }
        handle_event(&mut state, press(KeyCode::Enter));
        assert!(state.search.config.index.enabled);
    }

    #[test]
    fn open_report_when_done_opens_the_written_report_and_forces_html_output() {
        let mut state = AppState::default();
        state.search.config.export_html = false;
        state.search.config.open_report_when_done = true;
        let effects = handle_event(&mut state, AppEvent::SearchFinished(Ok(SearchRunResult::default())));
        assert!(matches!(effects.as_slice(), [Effect::WriteReport { write_html: true, .. }]), "the HTML report is needed to open it");

        let effects = handle_event(&mut state, AppEvent::ReportWritten(Some("/tmp/report.html".to_string())));
        assert!(matches!(effects.as_slice(), [Effect::OpenPath(p)] if p == "/tmp/report.html"));
    }

    #[test]
    fn a_written_report_is_not_opened_when_the_setting_is_off() {
        let mut state = AppState::default();
        state.search.config.open_report_when_done = false;
        let effects = handle_event(&mut state, AppEvent::ReportWritten(Some("/tmp/report.html".to_string())));
        assert!(effects.is_empty());
        assert_eq!(state.search.run.last_report_path.as_deref(), Some("/tmp/report.html"));
    }

    #[test]
    fn a_stale_index_prompts_for_an_update_when_the_search_finishes_and_yes_starts_a_build() {
        let mut state = AppState::default();
        state.search.config.search_path = "/tmp/some-folder".to_string();
        state.search.config.index.enabled = true;
        handle_event(&mut state, AppEvent::IndexStale { new_or_changed: 3, removed: 1 });
        assert!(!state.modal.is_open(), "the prompt must not interrupt a search in progress");

        handle_event(&mut state, AppEvent::SearchFinished(Ok(SearchRunResult::default())));
        let ModalState::Confirm(dialog) = &state.modal else { panic!("expected the update prompt") };
        assert_eq!(dialog.on_confirm, ConfirmAction::UpdateIndex);
        assert!(dialog.message.contains("3 new/changed") && dialog.message.contains("1 removed"), "{}", dialog.message);

        let effects = handle_event(&mut state, press(KeyCode::Char('y')));
        assert!(matches!(effects.as_slice(), [Effect::BuildIndex { .. }]), "one build/update path");
        assert!(state.search.index_run.is_building);
    }

    #[test]
    fn declining_the_index_update_prompt_starts_nothing_and_does_not_reprompt() {
        let mut state = AppState::default();
        state.search.config.search_path = "/tmp/some-folder".to_string();
        handle_event(&mut state, AppEvent::IndexStale { new_or_changed: 1, removed: 0 });
        handle_event(&mut state, AppEvent::SearchFinished(Ok(SearchRunResult::default())));
        let effects = handle_event(&mut state, press(KeyCode::Char('n')));
        assert!(effects.is_empty());
        assert!(!state.search.index_run.is_building);
        handle_event(&mut state, AppEvent::SearchFinished(Ok(SearchRunResult::default())));
        assert!(!state.modal.is_open(), "the prompt is per stale search, not sticky");
    }

    #[test]
    fn index_build_progress_and_finished_events_update_index_run_state() {
        let mut state = AppState::default();
        handle_event(
            &mut state,
            AppEvent::IndexBuildProgress(search_core::native_index::CorpusIndexProgress {
                stage: search_core::native_index::IndexStage::Indexing,
                files_processed: 3,
                total_files: 10,
                current_file: "a.txt".to_string(),
                ..Default::default()
            }),
        );
        assert!(state.search.index_run.is_building);
        assert!(state.search.index_run.status_text.contains("3/10"));

        handle_event(
            &mut state,
            AppEvent::IndexBuildFinished(Ok(search_core::native_index::CorpusIndexOutcome {
                indexed_count: 10,
                skipped_count: 0,
                failed_count: 0,
                failed_files: Vec::new(),
                ..Default::default()
            })),
        );
        assert!(!state.search.index_run.is_building);
        assert!(state.search.index_run.last_error.is_none());
    }

    // -------------------------------------------------------------
    // Mouse navigation
    // -------------------------------------------------------------

    use crate::mouse::MouseRegions;
    use ratatui::layout::Rect;

    fn click(col: u16, row: u16) -> MouseEvent {
        MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: col, row, modifiers: KeyModifiers::NONE }
    }

    fn scroll(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column: col, row, modifiers: KeyModifiers::NONE }
    }

    #[test]
    fn clicking_a_rail_item_switches_the_active_tool() {
        let mut state = AppState::default();
        let mut regions = MouseRegions::default();
        regions.rail.push((Rect::new(0, 0, 10, 1), ToolId::Search));
        regions.rail.push((Rect::new(0, 1, 10, 1), ToolId::FastenerHole));

        handle_mouse(&mut state, &regions, click(2, 1));
        assert_eq!(state.nav.active_tool, ToolId::FastenerHole);
        assert_eq!(state.focus.area, FocusArea::Rail);
    }

    #[test]
    fn clicking_a_disabled_rail_item_is_a_no_op() {
        let mut state = AppState::default();
        let mut regions = MouseRegions::default();
        regions.rail.push((Rect::new(0, 0, 10, 1), ToolId::Dupes));

        handle_mouse(&mut state, &regions, click(2, 0));
        assert_eq!(state.nav.active_tool, ToolId::Search, "activate() already ignores disabled tools");
    }

    #[test]
    fn clicking_outside_every_region_never_panics_and_changes_nothing() {
        let mut state = AppState::default();
        let regions = MouseRegions::default();
        let before = state.nav.active_tool;
        handle_mouse(&mut state, &regions, click(500, 500));
        assert_eq!(state.nav.active_tool, before);
    }

    #[test]
    fn clicking_a_result_row_selects_it_without_opening() {
        let mut state = AppState::default();
        state.search.run.results = vec![
            search_core::models::FileSearchResult {
                full_name: "a.txt".into(),
                status: search_core::models::FileSearchStatus::Hit,
                hits: Vec::new(),
                created: chrono::Local::now(),
                modified: chrono::Local::now(),
                file_length: 0,
                lines_cache: Vec::new(),
                total_line_count: 0,
                proximity_min_range: None,
                low_confidence_pdf: false,
                error_message: None,
            },
            search_core::models::FileSearchResult {
                full_name: "b.txt".into(),
                status: search_core::models::FileSearchStatus::Hit,
                hits: Vec::new(),
                created: chrono::Local::now(),
                modified: chrono::Local::now(),
                file_length: 0,
                lines_cache: Vec::new(),
                total_line_count: 0,
                proximity_min_range: None,
                low_confidence_pdf: false,
                error_message: None,
            },
        ];
        let mut regions = MouseRegions::default();
        regions.results_rows.push((Rect::new(0, 0, 20, 1), 0));
        regions.results_rows.push((Rect::new(0, 1, 20, 1), 1));

        let effects = handle_mouse(&mut state, &regions, click(5, 1));
        assert!(effects.is_empty(), "a single click selects, it doesn't open");
        assert_eq!(state.search.selected_result, 1);
        assert_eq!(state.focus.area, FocusArea::Workspace(search::PANE_RESULTS));
    }

    #[test]
    fn double_clicking_a_result_row_opens_it() {
        let mut state = AppState::default();
        state.search.run.results = vec![search_core::models::FileSearchResult {
            full_name: "a.txt".into(),
            status: search_core::models::FileSearchStatus::Hit,
            hits: Vec::new(),
            created: chrono::Local::now(),
            modified: chrono::Local::now(),
            file_length: 0,
            lines_cache: Vec::new(),
            total_line_count: 0,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }];
        let mut regions = MouseRegions::default();
        regions.results_rows.push((Rect::new(0, 0, 20, 1), 0));

        handle_mouse(&mut state, &regions, click(5, 0));
        let effects = handle_mouse(&mut state, &regions, click(5, 0));
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::OpenPath(ref p) if p == "a.txt"));
    }

    #[test]
    fn a_slow_second_click_is_not_treated_as_a_double_click() {
        let mut state = AppState::default();
        state.search.run.results = vec![search_core::models::FileSearchResult {
            full_name: "a.txt".into(),
            status: search_core::models::FileSearchStatus::Hit,
            hits: Vec::new(),
            created: chrono::Local::now(),
            modified: chrono::Local::now(),
            file_length: 0,
            lines_cache: Vec::new(),
            total_line_count: 0,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }];
        let mut regions = MouseRegions::default();
        regions.results_rows.push((Rect::new(0, 0, 20, 1), 0));

        handle_mouse(&mut state, &regions, click(5, 0));
        // Simulate real elapsed time by back-dating the recorded click.
        if let Some((t, _)) = state.last_click.as_mut() {
            *t -= DOUBLE_CLICK_WINDOW * 2;
        }
        let effects = handle_mouse(&mut state, &regions, click(5, 0));
        assert!(effects.is_empty(), "clicks far apart in time must not count as a double-click");
    }

    #[test]
    fn clicking_a_settings_pane_switches_section_without_a_row_hit() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        state.search.screen = search::ToolboxScreen::Settings;
        let mut regions = MouseRegions::default();
        regions.settings_section_panes.push((Rect::new(0, 0, 20, 5), search::settings_view::Section::Recents));

        handle_mouse(&mut state, &regions, click(5, 2));
        assert_eq!(state.search.settings.section, search::settings_view::Section::Recents);
    }

    #[test]
    fn double_clicking_a_settings_field_row_starts_editing() {
        let mut state = AppState::default();
        state.nav.activate(ToolId::Search);
        state.search.screen = search::ToolboxScreen::Settings;
        let mut regions = MouseRegions::default();
        regions.settings_field_rows.push((Rect::new(0, 0, 20, 1), 0)); // "Search path" - a Text field

        handle_mouse(&mut state, &regions, click(5, 0));
        handle_mouse(&mut state, &regions, click(5, 0));
        assert!(state.search.settings.editing);
    }

    #[test]
    fn clicking_confirm_yes_quits_and_confirm_no_cancels() {
        let mut state = AppState::default();
        state.modal = ModalState::Confirm(ConfirmDialog {
            title: "Quit?".into(),
            message: "still running".into(),
            on_confirm: ConfirmAction::Quit,
        });
        let mut regions = MouseRegions::default();
        regions.confirm_no = Some(Rect::new(0, 0, 8, 1));

        handle_mouse(&mut state, &regions, click(2, 0));
        assert!(!state.modal.is_open(), "clicking No dismisses the dialog");
        assert!(!state.should_quit, "clicking No must not quit the app");

        state.modal = ModalState::Confirm(ConfirmDialog {
            title: "Quit?".into(),
            message: "still running".into(),
            on_confirm: ConfirmAction::Quit,
        });
        regions.confirm_no = None;
        regions.confirm_yes = Some(Rect::new(0, 0, 9, 1));
        handle_mouse(&mut state, &regions, click(2, 0));
        assert!(state.should_quit);
    }

    #[test]
    fn clicking_anywhere_on_the_help_overlay_closes_it() {
        let mut state = AppState::default();
        state.modal = ModalState::Help;
        let mut regions = MouseRegions::default();
        regions.help_overlay = Some(Rect::new(0, 0, 80, 24));

        handle_mouse(&mut state, &regions, click(40, 12));
        assert!(!state.modal.is_open());
    }

    #[test]
    fn clicking_an_extension_row_toggles_its_selection() {
        let mut state = AppState::default();
        state.search.extension_picker =
            search::extension_picker::ExtensionPicker::open_with(vec![".txt".to_string(), ".rs".to_string()], None);
        let mut regions = MouseRegions::default();
        regions.extension_rows.push((Rect::new(0, 0, 10, 1), 0));

        handle_mouse(&mut state, &regions, click(2, 0));
        assert!(state.search.extension_picker.selected.contains(".txt"));
    }

    #[test]
    fn scrolling_over_the_results_list_moves_selection() {
        let mut state = AppState::default();
        state.search.run.results = vec![
            search_core::models::FileSearchResult {
                full_name: "a.txt".into(),
                status: search_core::models::FileSearchStatus::Hit,
                hits: Vec::new(),
                created: chrono::Local::now(),
                modified: chrono::Local::now(),
                file_length: 0,
                lines_cache: Vec::new(),
                total_line_count: 0,
                proximity_min_range: None,
                low_confidence_pdf: false,
                error_message: None,
            },
            search_core::models::FileSearchResult {
                full_name: "b.txt".into(),
                status: search_core::models::FileSearchStatus::Hit,
                hits: Vec::new(),
                created: chrono::Local::now(),
                modified: chrono::Local::now(),
                file_length: 0,
                lines_cache: Vec::new(),
                total_line_count: 0,
                proximity_min_range: None,
                low_confidence_pdf: false,
                error_message: None,
            },
        ];
        let mut regions = MouseRegions::default();
        regions.results_rows.push((Rect::new(0, 0, 20, 1), 0));
        regions.results_rows.push((Rect::new(0, 1, 20, 1), 1));

        handle_mouse(&mut state, &regions, scroll(MouseEventKind::ScrollDown, 5, 0));
        assert_eq!(state.search.selected_result, 1);
    }

    #[test]
    fn scrolling_outside_any_list_is_a_no_op() {
        let mut state = AppState::default();
        state.search.run.results = vec![search_core::models::FileSearchResult {
            full_name: "a.txt".into(),
            status: search_core::models::FileSearchStatus::Hit,
            hits: Vec::new(),
            created: chrono::Local::now(),
            modified: chrono::Local::now(),
            file_length: 0,
            lines_cache: Vec::new(),
            total_line_count: 0,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }];
        let regions = MouseRegions::default();
        handle_mouse(&mut state, &regions, scroll(MouseEventKind::ScrollDown, 5, 0));
        assert_eq!(state.search.selected_result, 0);
    }
}    #[test]
    fn the_edge_check_round_trip_runs_through_an_effect_and_an_event() {
        let mut state = AppState::default();
        state.nav.activate(crate::nav::ToolId::Bushing);
        let effects = state.bushing.perform(bushing::BushingAction::EdgeCheck);
        let [Effect::RunEdgeCheck { id, input, cfg, deep: false }] = effects.as_slice() else { panic!("{effects:?}") };
        assert!(state.bushing.edge_job.is_some());
        let run = bushing::edge_check::execute(input.clone(), cfg);
        handle_event(&mut state, AppEvent::EdgeCheckFinished { id: *id, run: Box::new(run.clone()) });
        assert!(state.bushing.edge_job.is_none() && state.bushing.edge_check.is_some());
        assert!(state.notifications.visible().iter().any(|t| t.message.contains("Edge check finished")));
        // A late result for a job that is no longer current changes nothing and adds no toast.
        let toasts = state.notifications.visible().len();
        handle_event(&mut state, AppEvent::EdgeCheckFinished { id: *id + 7, run: Box::new(run) });
        assert_eq!(state.notifications.visible().len(), toasts);
    }


