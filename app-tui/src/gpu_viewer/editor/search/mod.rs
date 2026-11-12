//! The Search workbench in the native window: the terminal Search Files toolbox's own state, run, index and report
//! (`toolboxes::search`: `SearchToolState`, `run_or_notify`, `apply_event`, `exec::execute`) behind the CAD chrome.
//! It is a bench without a 3D model: its page (`Authoring::document`) is a search bar, live progress, the results
//! table and the selected file's highlighted matches, drawn where a model would be. Options are inspector rows over
//! the terminal form's own settings table (`settings_view::settings`), so labels, parsing and persistence are shared.
//!
//! Background work runs on this session's own tokio runtime and reports through the same `AppEvent`s the terminal
//! uses; `poll` folds them in with the shared reducer. Nothing here searches, matches or extracts: `search-core` and
//! `native-search` do.
#[cfg(feature = "neural-search")]
mod alerts;
mod find;
#[cfg(feature = "semantic-search")]
mod diff;
#[cfg(feature = "nl-find")]
mod nl;
#[cfg(feature = "neural-search")]
mod feedback;
#[cfg(test)]
mod tests;
mod view;

use super::{
    authoring::{Authoring, Card, StepState},
    camera::Camera,
    session::{EditorAction, Key, KeyResult, Mods},
};
use crate::app::{AppEvent, Effect};
use crate::gpu_viewer::{
    bench::Bench,
    menu::{Item, Menu},
    ui::{DockItem, Document, RibbonTab, TreeNode, ValueCard},
    Scene,
};
use crate::notifications::NotificationQueue;
use crate::toolboxes::search::{
    self as search, exec, model, persistence, settings_view, SearchToolState,
};
use search_core::models::FileSearchResult;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// What a click in the Search workbench asks for (carried by `EditorAction::Search`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchAct {
    Run,
    Cancel,
    /// Select result `.0` (an index into the run's results).
    Select(usize),
    PreviousMatch,
    NextMatch,
    /// Open result `.0` with the system's default program.
    Open(usize),
    OpenSelected,
    /// Open related file `.0` of the preview's "Related files" list with the system's default program.
    OpenRelatedFile(usize),
    Reveal,
    CopyPath,
    ExportHits,
    OpenReport,
    SortBy(SortKey),
    /// How the filters combine (the global mode; a group's `[tag]` overrides it).
    Match(search_core::models::MatchMode),
    /// Show only results of extension `.0` (an index into `types()`); `None` shows all.
    OnlyType(Option<usize>),
    /// A setting of the terminal form (`settings_view::settings`): type a value, or flip / cycle it.
    Edit(usize),
    Step(usize),
    ApplyEdit,
    CancelEdit,
    /// Choose the folder with the operating system's folder picker.
    BrowseFolder,
    /// Choose the folder reports and exports are written to, with the same picker.
    BrowseOutput,
    BuildIndex,
    /// Add every other folder with a recorded fast index to this search (the machine-level catalog).
    AlsoIndexedFolders,
    #[cfg(feature = "neural-search")]
    SaveAlert,
    #[cfg(feature = "neural-search")]
    CaptureAlert(usize),
    #[cfg(feature = "neural-search")]
    CheckAlert(usize),
    #[cfg(feature = "neural-search")]
    DeleteAlert(usize),
    #[cfg(feature = "semantic-search")]
    SemanticSearch,
    /// Step the Find reading: auto, words, sentence.
    FindAs,
    #[cfg(feature = "semantic-search")]
    OpenSemantic(usize),
    #[cfg(feature = "nl-find")]
    NlUnderstand,
    #[cfg(feature = "nl-find")]
    NlFind,
    #[cfg(feature = "nl-find")]
    NlCancel,
    #[cfg(feature = "nl-find")]
    NlRemove(usize),
    #[cfg(feature = "nl-find")]
    NlMerge(usize),
    #[cfg(feature = "nl-find")]
    NlOpen(usize, usize),
    #[cfg(feature = "semantic-search")]
    FindRelated,
    #[cfg(feature = "semantic-search")]
    OpenRelated(usize),
    #[cfg(feature = "semantic-search")]
    CompareRelated(usize),
    CancelIndex,
    UpdateStaleIndex,
    DismissStale,
    PickExtensions,
    ToggleExtension(usize),
    AllExtensions,
    CloseExtensions,
    ApplyRecent(usize),
    ApplyRecentFilters(usize),
    ApplyRecentFolder(usize),
    ApplyCorrection(usize),
    WidenProximity,
    AddCorpusTerm(usize),
    AddSynonym(usize),
    RemoveRecent(usize),
    ApplyPreset(usize),
    DeletePreset(usize),
    SavePreset,
    ClearRecents,
    #[cfg(feature = "neural-search")]
    VoteRelevance(usize, search_core::relevance::FeedbackLabel),
    #[cfg(feature = "neural-search")]
    ToggleRelevanceFeedback,
    #[cfg(feature = "neural-search")]
    SummarizeSelected,
    #[cfg(feature = "neural-search")]
    ResetRelevanceFeedback,
    #[cfg(feature = "neural-search")]
    ExportRelevanceFeedback,
    /// Bring the tools of a step (the tree).
    StepTab(&'static str),
    /// Show or hide the Exclude row under the bar.
    ToggleExclude,
    /// What an exclusion leaves out: the whole file or only the lines.
    ExcludeScope(search_core::models::ExcludeScope),
}

/// What the results table is sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// The order the files were found in.
    Found,
    Name,
    Hits,
    Size,
    Modified,
    /// Files with the most distinctive hit lines first (`search_core::ordering`, BM25-style over the hit lines).
    BestMatch,
    #[cfg(feature = "neural-search")]
    Relevance,
}

/// A typed value being edited in a value card: a setting (`Some(index)`) or a new preset's name (`None`).
struct Edit {
    setting: Option<usize>,
    text: String,
}

/// Inputs of the page (`DocInput::id`).
pub(crate) const INPUT_FOLDER: u8 = 0;
pub(crate) const INPUT_FIND: u8 = 1;
pub(crate) const INPUT_FILTER: u8 = 2;
pub(crate) const INPUT_EXCLUDE: u8 = 3;
pub(crate) const INPUT_OUTPUT: u8 = 4;
#[cfg(feature = "nl-find")]
pub(crate) const INPUT_NL: u8 = 6;
/// Settings shown on the page instead of in a popover (`settings_view::settings` indices).
const SETTING_PATH: usize = 0;
const SETTING_FILTERS: usize = 2;
const SETTING_EXCLUDE: usize = 3;
const SETTING_EXCLUDE_SCOPE: usize = 8;
const SETTING_OUTPUT: usize = 14;

pub struct SearchSession {
    st: SearchToolState,
    rows_cache: RefCell<Option<view::RowsCache>>,
    /// Position of each result in the Best match order, kept for the run state it was computed for.
    best_match: RefCell<Option<((usize, String), Vec<usize>)>>,
    notes: NotificationQueue,
    /// Owns the search, report, index and scan tasks; dropped with the window.
    rt: tokio::runtime::Runtime,
    tx: mpsc::UnboundedSender<AppEvent>,
    rx: mpsc::UnboundedReceiver<AppEvent>,
    status: String,
    completed_search_query: Option<String>,
    step: &'static str,
    /// The results filter (file name or folder text, case-insensitive).
    filter: String,
    sort: (SortKey, bool),
    /// The selected result (index into `st.run.results`).
    selected: Option<usize>,
    /// The selected hit line in the current result.
    match_index: usize,
    match_scroll_request: u64,
    preview: Option<Preview>,
    preview_generation: u64,
    preview_tx: mpsc::UnboundedSender<PreviewEvent>,
    preview_rx: mpsc::UnboundedReceiver<PreviewEvent>,
    #[cfg(feature = "neural-search")]
    entity_tx: mpsc::UnboundedSender<EntityEvent>,
    #[cfg(feature = "neural-search")]
    entity_rx: mpsc::UnboundedReceiver<EntityEvent>,
    #[cfg(feature = "neural-search")]
    neural_tx: mpsc::UnboundedSender<NeuralRankEvent>,
    #[cfg(feature = "neural-search")]
    neural_rx: mpsc::UnboundedReceiver<NeuralRankEvent>,
    #[cfg(feature = "neural-search")]
    neural_request: Option<String>,
    #[cfg(feature = "neural-search")]
    neural_running: bool,
    #[cfg(feature = "neural-search")]
    neural_order: Option<(String, Vec<usize>)>,
    #[cfg(feature = "neural-search")]
    summary_tx: mpsc::UnboundedSender<SummaryEvent>,
    #[cfg(feature = "neural-search")]
    summary_rx: mpsc::UnboundedReceiver<SummaryEvent>,
    #[cfg(feature = "neural-search")]
    summary_request: Option<SummaryKey>,
    #[cfg(feature = "neural-search")]
    summary_running: bool,
    #[cfg(feature = "neural-search")]
    summary: Option<(SummaryKey, search_core::neural_search::ExtractiveSummary)>,
    #[cfg(feature = "neural-search")]
    summary_error: Option<String>,
    #[cfg(feature = "neural-search")]
    summary_cancel: Option<CancellationToken>,
    #[cfg(feature = "neural-search")]
    relevance_tx: mpsc::UnboundedSender<RelevanceRankEvent>,
    #[cfg(feature = "neural-search")]
    relevance_rx: mpsc::UnboundedReceiver<RelevanceRankEvent>,
    #[cfg(feature = "neural-search")]
    relevance_request: Option<String>,
    #[cfg(feature = "neural-search")]
    relevance_running: bool,
    #[cfg(feature = "neural-search")]
    relevance_scores: Option<(
        String,
        std::collections::HashMap<usize, search_core::relevance::RelevanceExplanation>,
    )>,
    #[cfg(feature = "neural-search")]
    feedback_generation: u64,
    #[cfg(feature = "neural-search")]
    alerts: Vec<alerts::Record>,
    #[cfg(feature = "neural-search")]
    alert_tx: mpsc::UnboundedSender<AlertEvent>,
    #[cfg(feature = "neural-search")]
    alert_rx: mpsc::UnboundedReceiver<AlertEvent>,
    #[cfg(feature = "neural-search")]
    alert_jobs: std::collections::HashMap<String, u64>,
    #[cfg(feature = "neural-search")]
    alert_cancels: std::collections::HashMap<String, CancellationToken>,
    #[cfg(feature = "neural-search")]
    alert_status: Option<String>,
    #[cfg(feature = "neural-search")]
    last_alert_changes: Option<(String, Vec<search_core::alerts::AlertChange>)>,
    #[cfg(feature = "neural-search")]
    alert_generation: u64,
    #[cfg(feature = "semantic-search")]
    semantic_tx: mpsc::UnboundedSender<SemanticSearchEvent>,
    #[cfg(feature = "semantic-search")]
    semantic_rx: mpsc::UnboundedReceiver<SemanticSearchEvent>,
    #[cfg(feature = "semantic-search")]
    semantic_request: Option<SemanticSearchKey>,
    #[cfg(feature = "semantic-search")]
    semantic_running: bool,
    #[cfg(feature = "semantic-search")]
    semantic_hits: Option<Vec<search_core::semantic_index::SemanticHit>>,
    #[cfg(feature = "semantic-search")]
    semantic_error: Option<String>,
    /// How the Find text is read (Auto picks from what was typed).
    find_as: find::FindAs,
    #[cfg(feature = "semantic-search")]
    related_tx: mpsc::UnboundedSender<RelatedSearchEvent>,
    #[cfg(feature = "semantic-search")]
    related_rx: mpsc::UnboundedReceiver<RelatedSearchEvent>,
    #[cfg(feature = "semantic-search")]
    related_request: Option<RelatedSearchKey>,
    #[cfg(feature = "semantic-search")]
    related_running: bool,
    #[cfg(feature = "semantic-search")]
    related_hits: Option<(
        RelatedSearchKey,
        Vec<search_core::semantic_index::SemanticHit>,
    )>,
    #[cfg(feature = "semantic-search")]
    related_error: Option<String>,
    #[cfg(feature = "semantic-search")]
    diff_tx: mpsc::UnboundedSender<RelatedDiffEvent>,
    #[cfg(feature = "semantic-search")]
    diff_rx: mpsc::UnboundedReceiver<RelatedDiffEvent>,
    #[cfg(feature = "semantic-search")]
    diff_request: Option<RelatedDiffKey>,
    #[cfg(feature = "semantic-search")]
    diff_running: bool,
    #[cfg(feature = "semantic-search")]
    diff_cancel: Option<CancellationToken>,
    #[cfg(feature = "semantic-search")]
    diff_result: Option<(RelatedDiffKey, diff::LineDiff)>,
    #[cfg(feature = "semantic-search")]
    diff_error: Option<String>,
    #[cfg(feature = "semantic-search")]
    diff_generation: u64,
    #[cfg(feature = "semantic-search")]
    diff_worker: std::sync::Arc<tokio::sync::Semaphore>,
    #[cfg(feature = "nl-find")]
    nl: nl::NlFind,
    preview_cancel: Option<CancellationToken>,
    edit: Option<Edit>,
    /// Asks for a folder, starting in the given one (the system dialog; tests answer without one).
    pick_folder: fn(&Path) -> Option<PathBuf>,
    /// The fast index went out of date during the last run: offer an update.
    stale: Option<(usize, usize)>,
    index_started: Option<Instant>,
    /// Watches the searched folder while "Keep index current" is on (root it watches, watcher).
    watcher: Option<(String, search_core::watch::FolderWatcher)>,
    /// When the watcher was last tried, so a folder without an index or a failing start is retried slowly.
    watcher_tried: Option<Instant>,
    /// The Exclude row is shown (it opens shown when the saved settings already exclude something).
    show_exclude: bool,
    /// Write `settings-tui.json` (off in tests, which must never touch the user's file).
    persist: bool,
    /// Where the settings live (`Authoring::path`).
    settings_file: PathBuf,
}

struct Preview {
    generation: u64,
    path: String,
    text: Option<crate::gpu_viewer::ui::DocText>,
    status: String,
    /// Why this file relates to others, from the catalog's stored edges (read once when the preview opens).
    related: Vec<search_core::catalog_edges::Reason>,
    #[cfg(feature = "neural-search")]
    entities: Option<Result<search_core::entities::EntityExtraction, String>>,
}

enum PreviewEvent {
    Progress(u64, String),
    Done(u64, Result<Vec<String>, String>),
}

#[cfg(feature = "neural-search")]
struct EntityEvent {
    generation: u64,
    path: String,
    result: Result<search_core::entities::EntityExtraction, String>,
}

#[cfg(feature = "neural-search")]
struct NeuralRankEvent {
    key: String,
    order: Result<Vec<usize>, String>,
}

#[cfg(feature = "neural-search")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct SummaryKey {
    path: String,
    preview_generation: u64,
}

#[cfg(feature = "neural-search")]
struct SummaryEvent {
    key: SummaryKey,
    result: Result<search_core::neural_search::ExtractiveSummary, String>,
}

#[cfg(feature = "neural-search")]
struct RelevanceRankEvent {
    key: String,
    scores: Result<Vec<search_core::relevance::RelevanceExplanation>, String>,
    feedback_error: Option<String>,
}

#[cfg(feature = "neural-search")]
enum AlertEvent {
    Progress {
        id: String,
        generation: u64,
        text: String,
    },
    Finished {
        id: String,
        generation: u64,
        capture: bool,
        result: Result<(alerts::Record, Vec<search_core::alerts::AlertChange>), String>,
    },
}

#[cfg(feature = "semantic-search")]
struct SemanticSearchEvent {
    key: SemanticSearchKey,
    result: Result<Vec<search_core::semantic_index::SemanticHit>, String>,
}

#[cfg(feature = "semantic-search")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RelatedSearchKey {
    source_path: String,
    preview_generation: u64,
    scope: SemanticSearchKey,
}

#[cfg(feature = "semantic-search")]
struct RelatedSearchEvent {
    key: RelatedSearchKey,
    result: Result<Vec<search_core::semantic_index::SemanticHit>, String>,
}

#[cfg(feature = "semantic-search")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RelatedDiffKey {
    related: RelatedSearchKey,
    candidate_path: String,
}

#[cfg(feature = "semantic-search")]
struct RelatedDiffEvent {
    generation: u64,
    key: RelatedDiffKey,
    result: Result<diff::LineDiff, String>,
}

#[cfg(feature = "semantic-search")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticSearchKey {
    query: String,
    index_location: crate::toolboxes::search::indexing::IndexLocation,
    output_folder: String,
    roots: Vec<SemanticSearchScope>,
}

#[cfg(feature = "semantic-search")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticSearchScope {
    index: PathBuf,
    root: String,
    extensions: Option<Vec<String>>,
    exclude_folders: Vec<String>,
    include_hidden: bool,
    max_file_size_mb: u64,
}

#[cfg(feature = "semantic-search")]
fn semantic_search_roots(
    config: &crate::toolboxes::search::model::SearchToolConfig,
) -> Vec<(PathBuf, search_core::models::SearchSettings)> {
    std::iter::once(config.search_path.clone())
        .chain(config.search_paths_extra.iter().cloned())
        .filter(|path| !path.trim().is_empty())
        .map(|root| {
            let mut settings = crate::toolboxes::search::model::build_settings(config);
            settings.search_path = root.clone();
            let index = crate::toolboxes::search::indexing::index_directory(
                config.index.location,
                &root,
                &config.output_folder,
            );
            (index, settings)
        })
        .collect()
}

#[cfg(feature = "semantic-search")]
fn semantic_search_key(
    config: &crate::toolboxes::search::model::SearchToolConfig,
    query: String,
    roots: &[(PathBuf, search_core::models::SearchSettings)],
) -> SemanticSearchKey {
    SemanticSearchKey {
        query,
        index_location: config.index.location,
        output_folder: config.output_folder.clone(),
        roots: roots
            .iter()
            .map(|(index, settings)| SemanticSearchScope {
                index: index.clone(),
                root: settings.search_path.clone(),
                extensions: settings.extensions.clone(),
                exclude_folders: settings.exclude_folders.clone(),
                include_hidden: settings.include_hidden,
                max_file_size_mb: settings.max_file_size_mb.to_bits(),
            })
            .collect(),
    }
}

#[cfg(feature = "semantic-search")]
fn current_semantic_search_key(
    config: &crate::toolboxes::search::model::SearchToolConfig,
) -> Option<SemanticSearchKey> {
    let query = config.filters_text.trim().to_string();
    if query.is_empty() {
        return None;
    }
    let roots = semantic_search_roots(config);
    (!roots.is_empty()).then(|| semantic_search_key(config, query, &roots))
}

#[cfg(feature = "semantic-search")]
fn related_search_key(
    config: &crate::toolboxes::search::model::SearchToolConfig,
    source_path: String,
    preview_generation: u64,
) -> Option<RelatedSearchKey> {
    let roots = semantic_search_roots(config);
    (!roots.is_empty()).then(|| RelatedSearchKey {
        scope: semantic_search_key(config, source_path.clone(), &roots),
        source_path,
        preview_generation,
    })
}

#[cfg(feature = "semantic-search")]
async fn load_comparison_lines(
    path: &str,
    config: &crate::toolboxes::search::model::SearchToolConfig,
    cancel: &CancellationToken,
) -> Result<Vec<String>, String> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| error.to_string())?;
    let max_bytes = ((config.max_file_size_mb * 1_048_576.0) as u64).min(diff::MAX_BYTES as u64);
    if metadata.len() > max_bytes {
        return Err("A file exceeds the 1 MiB comparison input limit.".into());
    }
    let bytes = search_core::file_reader::read_file_bytes_robust(
        path,
        config.file_timeout_seconds.max(1) as u64,
        0,
        0,
        None,
        cancel,
    )
    .await
    .map_err(|error| format!("{error:?}"))?;
    if bytes.len() > diff::MAX_BYTES {
        return Err("A file exceeds the 1 MiB comparison input limit.".into());
    }
    let extension = Path::new(path)
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    let timeout = config.pdf_timeout_seconds.max(1) as u64;
    let ocr = config.ocr_scanned_pdfs;
    tokio::task::spawn_blocking(move || {
        search_core::extraction::extract_lines_by_extension(&extension, &bytes, timeout, None, ocr)
            .map(|extracted| extracted.lines)
            .map_err(|error| format!("{error:?}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(feature = "semantic-search")]
fn semantic_hits_match_config<'a>(
    config: &crate::toolboxes::search::model::SearchToolConfig,
    request: Option<&SemanticSearchKey>,
    hits: Option<&'a [search_core::semantic_index::SemanticHit]>,
) -> Option<&'a [search_core::semantic_index::SemanticHit]> {
    let request = request?;
    let hits = hits?;
    (current_semantic_search_key(config).as_ref() == Some(request)).then_some(hits)
}

/// Write `sources` into `dir` and index each as one semantic chunk, as a built semantic index would hold them.
#[cfg(all(test, feature = "semantic-search"))]
pub(crate) fn semantic_fixture(dir: &Path, sources: &[(&str, &str)]) {
    use crate::toolboxes::search::indexing::{index_directory, IndexLocation};
    use search_core::semantic_index::SemanticIndex;

    let index_dir = index_directory(IndexLocation::SearchFolder, &dir.to_string_lossy(), "");
    std::fs::create_dir_all(&index_dir).unwrap();
    let semantic = SemanticIndex::open(&index_dir.join("semantic.sqlite")).unwrap();
    for (name, text) in sources {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        semantic
            .index_document(
                &path.to_string_lossy(),
                search_core::cache::ticks_from_modified(chrono::DateTime::<chrono::Local>::from(
                    metadata.modified().unwrap(),
                )),
                metadata.len() as i64,
                &[(*text).into()],
            )
            .unwrap();
    }
}

impl Drop for SearchSession {
    fn drop(&mut self) {
        #[cfg(feature = "semantic-search")]
        if let Some(cancel) = self.diff_cancel.take() {
            cancel.cancel();
        }
        if let Some(cancel) = &self.preview_cancel {
            cancel.cancel();
        }
        #[cfg(feature = "neural-search")]
        for cancellation in self.alert_cancels.values() {
            cancellation.cancel();
        }
        search::request_cancel(&self.st);
        if let Some(c) = &self.st.index_cancel {
            c.cancel();
        }
        if self.persist {
            persistence::save(&self.st.to_persisted_file());
        }
    }
}

impl SearchSession {
    #[cfg(feature = "neural-search")]
    pub(super) fn current_summary(&self) -> Option<&search_core::neural_search::ExtractiveSummary> {
        let (key, summary) = self.summary.as_ref()?;
        let preview = self.preview.as_ref()?;
        (preview.path == key.path
            && preview.generation == key.preview_generation
            && self
                .current_result()
                .is_some_and(|result| result.full_name == key.path)
            && preview.text.is_some())
        .then_some(summary)
    }

    #[cfg(feature = "semantic-search")]
    pub(super) fn current_semantic_hits(
        &self,
    ) -> Option<&[search_core::semantic_index::SemanticHit]> {
        semantic_hits_match_config(
            &self.st.config,
            self.semantic_request.as_ref(),
            self.semantic_hits.as_deref(),
        )
    }

    #[cfg(feature = "semantic-search")]
    pub(super) fn current_related_hits(
        &self,
    ) -> Option<&[search_core::semantic_index::SemanticHit]> {
        let (key, hits) = self.related_hits.as_ref()?;
        let preview = self.preview.as_ref()?;
        let selected = self.current_result()?;
        if preview.path != selected.full_name || preview.text.is_none() {
            return None;
        }
        let current = related_search_key(&self.st.config, preview.path.clone(), preview.generation);
        (current.as_ref() == Some(key)).then_some(hits)
    }

    #[cfg(feature = "semantic-search")]
    fn current_related_diff(&self) -> Option<&diff::LineDiff> {
        let (key, result) = self.diff_result.as_ref()?;
        let preview = self.preview.as_ref()?;
        let current =
            related_search_key(&self.st.config, preview.path.clone(), preview.generation)?;
        if current != key.related || preview.path != key.related.source_path {
            return None;
        }
        self.current_related_hits()?
            .iter()
            .any(|hit| hit.path == key.candidate_path)
            .then_some(result)
    }

    /// A session on the saved Search settings (shared with the terminal toolbox); `folder` replaces the saved
    /// search folder when given.
    pub fn open(folder: Option<&Path>) -> Result<Self, String> {
        let mut s = Self::with_state(SearchToolState::default(), !cfg!(test))?;
        if let Some(f) = folder.filter(|f| !f.as_os_str().is_empty()) {
            s.st.config.search_path = f.to_string_lossy().into_owned();
        }
        Ok(s)
    }

    /// A session on default settings that never reads or writes the user's settings file.
    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        let mut s = Self::with_state(SearchToolState::default(), false).expect("runtime");
        // Never a real dialog in a test: Browse answers "cancelled" unless a test sets its own picker.
        s.pick_folder = |_| None;
        // An empty output folder is the working folder: a test that runs a search must not leave a report in the
        // crate. Tests that check the report turn it back on with their own output folder.
        s.st.config.export_html = false;
        s
    }
    /// Search `folder` for `words` exactly and semantically with the Index on, and wait for both, as the window does.
    #[cfg(all(test, feature = "semantic-search"))]
    pub(crate) fn words_for_snapshot(&mut self, folder: &Path, words: &str) {
        self.st.config.search_path = folder.to_string_lossy().into_owned();
        self.st.config.index.semantic = true;
        self.find_as = find::FindAs::Words;
        self.document_field(INPUT_FIND, words.into());
        self.act_on_for_snapshot(SearchAct::Run);
        self.act_on_for_snapshot(SearchAct::SemanticSearch);
        let started = Instant::now();
        while self.semantic_running || self.st.run.is_running {
            assert!(started.elapsed() < std::time::Duration::from_secs(60));
            self.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.poll();
    }
    /// Natural-language Find: understand `sentence` (stopping at the chips when `find` is false), as the window does.
    #[cfg(all(test, feature = "nl-find"))]
    pub(crate) fn nl_for_snapshot(&mut self, folder: &Path, sentence: &str, find: bool) {
        self.st.config.search_path = folder.to_string_lossy().into_owned();
        self.st.config.index.semantic = true;
        self.nl.sentence = sentence.into();
        self.nl.reset();
        let mut steps = vec![SearchAct::NlUnderstand];
        if find {
            steps.push(SearchAct::NlFind);
        }
        for step in steps {
            self.act(crate::gpu_viewer::editor::session::EditorAction::Search(step));
            let started = Instant::now();
            while self.nl.stage != nl::Stage::Idle {
                assert!(started.elapsed() < std::time::Duration::from_secs(60));
                self.poll();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            self.poll();
        }
    }
    #[cfg(all(test, feature = "semantic-search"))]
    fn act_on_for_snapshot(&mut self, act: SearchAct) {
        self.act(crate::gpu_viewer::editor::session::EditorAction::Search(act));
    }
    /// Show the index as 60 percent built, as a running build would.
    #[cfg(test)]
    pub(crate) fn building_for_snapshot(&mut self, percent: f64) {
        self.st.index_run.begin();
        self.st.index_run.percent = percent;
    }
    #[cfg(test)]
    pub(crate) fn set_folder_picker(&mut self, pick: fn(&Path) -> Option<PathBuf>) {
        self.pick_folder = pick;
    }

    fn with_state(st: SearchToolState, persist: bool) -> Result<Self, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("search")
            .build()
            .map_err(|e| format!("cannot start the search workers: {e}"))?;
        let (tx, rx) = mpsc::unbounded_channel();
        let (preview_tx, preview_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "neural-search")]
        let (entity_tx, entity_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "neural-search")]
        let (neural_tx, neural_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "neural-search")]
        let (summary_tx, summary_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "neural-search")]
        let (relevance_tx, relevance_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "neural-search")]
        let (alert_tx, alert_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "semantic-search")]
        let (semantic_tx, semantic_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "semantic-search")]
        let (related_tx, related_rx) = mpsc::unbounded_channel();
        #[cfg(feature = "semantic-search")]
        let (diff_tx, diff_rx) = mpsc::unbounded_channel();
        let settings_file = crate::paths::app_data_dir().map_or_else(
            || PathBuf::from("settings-tui.json"),
            |d| d.join("settings-tui.json"),
        );
        #[cfg(feature = "neural-search")]
        let (alerts, alert_load_errors) = if persist {
            alerts::load_all(&alerts::directory(&settings_file))
        } else {
            (Vec::new(), Vec::new())
        };
        Ok(Self {
            rows_cache: RefCell::new(None),
            best_match: RefCell::new(None),
            notes: NotificationQueue::default(),
            rt,
            tx,
            rx,
            status: {
                #[cfg(feature = "neural-search")]
                if let Some(error) = alert_load_errors.first() {
                    format!("Saved search alerts: {error}")
                } else {
                    "Choose a folder, type what to find and press Enter.".into()
                }
                #[cfg(not(feature = "neural-search"))]
                {
                    "Choose a folder, type what to find and press Enter.".into()
                }
            },
            completed_search_query: None,
            step: "Search",
            filter: String::new(),
            sort: (SortKey::Found, false),
            selected: None,
            match_index: 0,
            match_scroll_request: 0,
            preview: None,
            preview_generation: 0,
            preview_tx,
            preview_rx,
            #[cfg(feature = "neural-search")]
            entity_tx,
            #[cfg(feature = "neural-search")]
            entity_rx,
            #[cfg(feature = "neural-search")]
            neural_tx,
            #[cfg(feature = "neural-search")]
            neural_rx,
            #[cfg(feature = "neural-search")]
            neural_request: None,
            #[cfg(feature = "neural-search")]
            neural_running: false,
            #[cfg(feature = "neural-search")]
            neural_order: None,
            #[cfg(feature = "neural-search")]
            summary_tx,
            #[cfg(feature = "neural-search")]
            summary_rx,
            #[cfg(feature = "neural-search")]
            summary_request: None,
            #[cfg(feature = "neural-search")]
            summary_running: false,
            #[cfg(feature = "neural-search")]
            summary: None,
            #[cfg(feature = "neural-search")]
            summary_error: None,
            #[cfg(feature = "neural-search")]
            summary_cancel: None,
            #[cfg(feature = "neural-search")]
            relevance_tx,
            #[cfg(feature = "neural-search")]
            relevance_rx,
            #[cfg(feature = "neural-search")]
            relevance_request: None,
            #[cfg(feature = "neural-search")]
            relevance_running: false,
            #[cfg(feature = "neural-search")]
            relevance_scores: None,
            #[cfg(feature = "neural-search")]
            feedback_generation: 0,
            #[cfg(feature = "neural-search")]
            alerts,
            #[cfg(feature = "neural-search")]
            alert_tx,
            #[cfg(feature = "neural-search")]
            alert_rx,
            #[cfg(feature = "neural-search")]
            alert_jobs: std::collections::HashMap::new(),
            #[cfg(feature = "neural-search")]
            alert_cancels: std::collections::HashMap::new(),
            #[cfg(feature = "neural-search")]
            alert_status: None,
            #[cfg(feature = "neural-search")]
            last_alert_changes: None,
            #[cfg(feature = "neural-search")]
            alert_generation: 0,
            #[cfg(feature = "semantic-search")]
            semantic_tx,
            #[cfg(feature = "semantic-search")]
            semantic_rx,
            #[cfg(feature = "semantic-search")]
            semantic_request: None,
            #[cfg(feature = "semantic-search")]
            semantic_running: false,
            #[cfg(feature = "semantic-search")]
            semantic_hits: None,
            #[cfg(feature = "semantic-search")]
            semantic_error: None,
            find_as: find::FindAs::Auto,
            #[cfg(feature = "nl-find")]
            nl: nl::NlFind::new(),
            #[cfg(feature = "semantic-search")]
            related_tx,
            #[cfg(feature = "semantic-search")]
            related_rx,
            #[cfg(feature = "semantic-search")]
            related_request: None,
            #[cfg(feature = "semantic-search")]
            related_running: false,
            #[cfg(feature = "semantic-search")]
            related_hits: None,
            #[cfg(feature = "semantic-search")]
            related_error: None,
            #[cfg(feature = "semantic-search")]
            diff_tx,
            #[cfg(feature = "semantic-search")]
            diff_rx,
            #[cfg(feature = "semantic-search")]
            diff_request: None,
            #[cfg(feature = "semantic-search")]
            diff_running: false,
            #[cfg(feature = "semantic-search")]
            diff_cancel: None,
            #[cfg(feature = "semantic-search")]
            diff_result: None,
            #[cfg(feature = "semantic-search")]
            diff_error: None,
            #[cfg(feature = "semantic-search")]
            diff_generation: 0,
            #[cfg(feature = "semantic-search")]
            diff_worker: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
            preview_cancel: None,
            edit: None,
            pick_folder: system_folder_picker,
            stale: None,
            index_started: None,
            watcher: None,
            watcher_tried: None,
            show_exclude: !st.config.exclude_filters_text.trim().is_empty(),
            persist,
            st,
            settings_file,
        })
    }

    pub fn state(&self) -> &SearchToolState {
        &self.st
    }

    /// Put the session in the middle of a run (for the snapshot of the progress card): a quarter done, three files
    /// being read, one of them slow.
    #[cfg(test)]
    pub(crate) fn state_mut_for_snapshot(&mut self) {
        use search_core::models::InFlightFileStatus;
        let root = self.st.config.search_path.clone();
        let run = &mut self.st.run;
        run.is_running = true;
        run.started = Some(Instant::now() - std::time::Duration::from_secs(14));
        run.elapsed = None;
        run.files_completed = 1_204;
        run.total_files = 4_810;
        run.status_text = "1204 of 4810 file(s) - 37 hit(s) so far".into();
        run.in_flight_files = vec![
            InFlightFileStatus {
                file_name: format!("{root}/specs/Structural Repair Manual 51-10.pdf"),
                status_text: "Extracting PDF text".into(),
                elapsed_seconds: 6.4,
            },
            InFlightFileStatus {
                file_name: format!("{root}/notes/meeting-2026-09.docx"),
                status_text: "Reading".into(),
                elapsed_seconds: 0.3,
            },
            InFlightFileStatus {
                file_name: format!("{root}/archive/drawings.zip"),
                status_text: "Opening archive".into(),
                elapsed_seconds: 1.1,
            },
        ];
        self.stale = None;
    }

    // ------------------------------------------------------------------ effects and events

    fn execute(&mut self, effects: Vec<Effect>) {
        let _rt = self.rt.enter();
        for e in effects {
            if matches!(e, Effect::PersistSearchSettings) && !self.persist {
                continue;
            }
            // Every effect the Search toolbox returns is a Search effect; anything else would be a bug upstream.
            let _ = exec::execute(&self.tx, &mut self.st, e);
        }
    }
    fn save_settings(&mut self) {
        self.execute(vec![Effect::PersistSearchSettings]);
    }
    /// The newest notification becomes the status line.
    fn take_note(&mut self) {
        if let Some(t) = self.notes.visible().last() {
            self.status = t.message.clone();
        }
        self.notes = NotificationQueue::default();
    }

    pub fn run(&mut self) {
        if self.st.run.is_running {
            self.status = "A search is already running (Esc stops it).".into();
            return;
        }
        self.clear_insight();
        match find::classify(&self.st.config.filters_text, self.find_as) {
            #[cfg(feature = "nl-find")]
            find::Intent::Sentence if self.nl_ready() => {
                // A sentence is not a keyword filter: read it as conditions, then find with them. The conditions stay
                // on the page as chips; removing one and pressing Find again repeats the step.
                self.nl.sentence = self.st.config.filters_text.trim().to_string();
                self.nl_understand();
                if self.nl.conditions.as_ref().is_some_and(|list| !list.is_empty()) {
                    self.nl_find();
                }
                return;
            }
            #[cfg(feature = "nl-find")]
            find::Intent::Sentence if self.find_as == find::FindAs::Sentence => {
                self.status = "A sentence needs the Semantic index: turn it on in Options, then build the index.".into();
                return;
            }
            _ => {}
        }
        if let Some(e) = model::regex_validation_error(&self.st.config) {
            self.status = format!("Fix the pattern first: {e}");
            return;
        }
        let effects = search::run_or_notify(&mut self.st, &mut self.notes);
        self.take_note();
        if !effects.is_empty() {
            self.completed_search_query = Some(self.st.config.filters_text.clone());
            #[cfg(feature = "neural-search")]
            {
                self.relevance_request = None;
                self.relevance_scores = None;
            }
            self.selected = None;
            self.match_index = 0;
            self.match_scroll_request = self.match_scroll_request.wrapping_add(1);
            if let Some(cancel) = self.preview_cancel.take() {
                cancel.cancel();
            }
            self.preview = None;
            self.stale = None;
            self.status = format!("Searching {}...", self.st.config.search_path.trim());
            self.execute(effects);
            self.save_settings();
            // With a ready Semantic index the same Find also looks for cited passages by meaning.
            #[cfg(feature = "semantic-search")]
            if self.st.config.index.semantic && self.st.index_run.docs.is_some() {
                self.semantic_search();
                self.status = format!("Searching {}...", self.st.config.search_path.trim());
            }
        }
    }

    /// A new Find replaces what the last one showed beside the files: conditions, semantic passages.
    fn clear_insight(&mut self) {
        #[cfg(feature = "semantic-search")]
        {
            self.semantic_hits = None;
            self.semantic_error = None;
            self.semantic_request = None;
        }
        #[cfg(feature = "nl-find")]
        self.nl.reset();
    }

    fn cancel(&mut self) {
        if self.st.run.is_running {
            search::request_cancel(&self.st);
            self.status = "Stopping the search...".into();
        } else if let Some(c) = &self.st.index_cancel {
            c.cancel();
            self.status = "Stopping the index build (what is indexed is kept)...".into();
        }
    }

    #[cfg(feature = "neural-search")]
    fn save_alert(&mut self) {
        if !self.persist {
            self.status = "Saved alerts are disabled in this test session.".into();
            return;
        }
        if self.st.run.is_running || self.st.index_run.is_building {
            self.status = "Wait for the current search or index build to finish.".into();
            return;
        }
        if self.alerts.len() >= search_core::alerts::MAX_SAVED_ALERTS {
            self.status = format!(
                "Saved alert limit is {}.",
                search_core::alerts::MAX_SAVED_ALERTS
            );
            return;
        }
        let name = self.st.config.filters_text.trim().to_string();
        if name.is_empty() || self.st.config.search_path.trim().is_empty() {
            self.status = "Choose a folder and query before saving an alert.".into();
            return;
        }
        let alert = match search_core::alerts::SavedSearchAlert::create(
            name,
            model::build_settings(&self.st.config),
        ) {
            Ok(alert) => alert,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let record = alerts::Record::new(
            alert,
            persistence::PersistedSearchSettings::from(&self.st.config),
        );
        let directory = alerts::directory(&self.settings_file);
        match alerts::save(&directory, &record) {
            Ok(()) => {
                self.alerts.push(record);
                self.status = "Saved alert. Capture a baseline before checking for changes.".into();
            }
            Err(error) => self.status = format!("Could not save alert: {error}"),
        }
    }

    #[cfg(feature = "neural-search")]
    fn run_saved_alert(&mut self, index: usize, capture: bool) {
        if self.st.run.is_running || self.st.index_run.is_building {
            self.status = "Wait for the current search or index build to finish.".into();
            return;
        }
        let Some(record) = self.alerts.get(index).cloned() else {
            return;
        };
        let id = record.alert.id.clone();
        if self.alert_jobs.contains_key(&id) {
            self.status = "This alert is already running.".into();
            return;
        }
        if !capture && record.alert.baseline.is_none() {
            self.status = "Capture a baseline before checking this alert.".into();
            return;
        }
        let roots = std::iter::once(record.alert.settings.search_path.clone())
            .chain(record.ui_settings.search_paths_extra.iter().cloned())
            .map(|root| root.trim().to_string())
            .filter(|root| !root.is_empty())
            .collect::<Vec<_>>();
        if roots.is_empty() {
            self.status = "This alert has no search folder.".into();
            return;
        }
        let index_settings = crate::toolboxes::search::indexing::IndexSettings {
            enabled: record.ui_settings.index_enabled,
            location: record.ui_settings.index_location.unwrap_or_default(),
            semantic: false,
            watch: false,
            lazy_embed: false,
        };
        let generation = self.alert_generation;
        self.alert_generation = self.alert_generation.wrapping_add(1);
        self.alert_jobs.insert(id.clone(), generation);
        let cancellation = CancellationToken::new();
        self.alert_cancels.insert(id.clone(), cancellation.clone());
        self.last_alert_changes = None;
        self.alert_status = Some(format!("{}: starting exact search…", record.alert.name));
        self.status = self.alert_status.clone().unwrap_or_default();

        let output = self.alert_tx.clone();
        let (run_tx, mut run_rx) = mpsc::unbounded_channel();
        self.rt.spawn(async move {
            crate::toolboxes::search::runner::run_search(
                run_tx,
                cancellation,
                roots,
                record.alert.settings.clone(),
                index_settings,
            )
            .await;
            while let Some(event) = run_rx.recv().await {
                match event {
                    AppEvent::SearchProgress(progress) => {
                        let file = progress
                            .current_file_name
                            .as_deref()
                            .unwrap_or("scanning files");
                        let _ = output.send(AlertEvent::Progress {
                            id: id.clone(),
                            generation,
                            text: format!(
                                "{}: {} / {} files · {}",
                                record.alert.name,
                                progress.files_completed,
                                progress.total_files,
                                file
                            ),
                        });
                    }
                    AppEvent::SearchFinished(result) => {
                        let mut record = record;
                        let computed = match result {
                            Ok(run) => tokio::task::spawn_blocking(move || {
                                let run = search_core::alerts::AlertRun::from_search_result(&run)?;
                                let changes = if capture {
                                    record.alert.capture_baseline(&run)?;
                                    Vec::new()
                                } else {
                                    let scope = record.alert.settings.clone();
                                    record.alert.check_and_update(&scope, &run)?
                                };
                                Ok((record, changes))
                            })
                            .await
                            .unwrap_or_else(|error| Err(format!("Alert worker stopped: {error}"))),
                            Err(error) => Err(format!("Exact search failed: {error}")),
                        };
                        let _ = output.send(AlertEvent::Finished {
                            id,
                            generation,
                            capture,
                            result: computed,
                        });
                        return;
                    }
                    _ => {}
                }
            }
            let _ = output.send(AlertEvent::Finished {
                id,
                generation,
                capture,
                result: Err("Exact search ended without a result.".into()),
            });
        });
    }

    #[cfg(feature = "neural-search")]
    fn delete_alert(&mut self, index: usize) {
        let Some(record) = self.alerts.get(index) else {
            return;
        };
        let id = record.alert.id.clone();
        if self.persist {
            let directory = alerts::directory(&self.settings_file);
            if let Err(error) = alerts::remove(&directory, &id) {
                self.status = format!("Could not delete alert: {error}");
                return;
            }
        }
        if let Some(cancellation) = self.alert_cancels.remove(&id) {
            cancellation.cancel();
        }
        self.alert_jobs.remove(&id);
        self.alerts.remove(index);
        self.last_alert_changes = None;
        self.alert_status = None;
        self.status = "Deleted saved alert.".into();
    }

    pub(super) fn can_widen_proximity(&self) -> bool {
        let run = &self.st.run;
        let groups = model::parse_filter_groups(&self.st.config.filters_text);
        !run.is_running
            && run.started.is_some()
            && run.results.is_empty()
            && self.completed_search_query.as_deref() == Some(self.st.config.filters_text.as_str())
            && self.st.config.match_mode == search_core::models::MatchMode::Proximity
            && groups.len() == 1
            && groups[0].mode.is_none()
            && run
                .diagnostics
                .as_ref()
                .is_some_and(|diagnostics| diagnostics.skipped_by_mode > 0)
    }

    /// "Keep index current": starts or stops the folder watcher to match the setting, and when changes have settled
    /// and nothing else is running, updates the index for just those files (a full update if events may be lost).
    fn poll_watcher(&mut self) {
        const SETTLE: std::time::Duration = std::time::Duration::from_secs(2);
        const RETRY: std::time::Duration = std::time::Duration::from_secs(5);
        let config = &self.st.config;
        let root = config.search_path.trim().to_string();
        // Tests never start a real watcher (`persist` is off there).
        if !(self.persist && config.index.enabled && config.index.watch && !root.is_empty()) {
            self.watcher = None;
            return;
        }
        if self.watcher.as_ref().is_none_or(|(watched, _)| *watched != root) {
            self.watcher = None;
            if self.watcher_tried.is_some_and(|t| t.elapsed() < RETRY) {
                return;
            }
            self.watcher_tried = Some(Instant::now());
            let dir = crate::toolboxes::search::indexing::index_directory(config.index.location, &root, &config.output_folder);
            // Nothing to keep current until a first build exists.
            if !crate::toolboxes::search::indexing::index_is_built(&dir) {
                return;
            }
            let scope = model::build_settings(config);
            match search_core::watch::FolderWatcher::start(&root, scope.include_hidden, scope.exclude_folders) {
                Ok(watcher) => self.watcher = Some((root, watcher)),
                Err(error) => self.status = format!("Cannot watch the folder for changes: {error}"),
            }
            return;
        }
        if self.st.run.is_running || self.st.index_run.is_building {
            return;
        }
        let Some((_, watcher)) = &self.watcher else { return };
        let Some(changes) = watcher.take_quiet(SETTLE) else { return };
        if changes.overflow {
            self.status = "The folder changed more than could be tracked: updating the whole index...".into();
            self.build_index();
            return;
        }
        let count = changes.paths.len();
        let effects = search::start_index_update(&mut self.st, changes.paths.clone());
        if effects.is_empty() {
            if let Some((_, watcher)) = &self.watcher {
                watcher.restore(changes);
            }
            return;
        }
        self.index_started = Some(Instant::now());
        self.status = format!("Updating the index for {count} changed path(s)...");
        self.execute(effects);
    }

    fn build_index(&mut self) {
        if self.st.run.is_running {
            self.status = "Wait for the search to finish before building the index.".into();
            return;
        }
        let effects = search::start_index_build(&mut self.st);
        if effects.is_empty() {
            self.status = if self.st.index_run.is_building {
                "The index is already building.".into()
            } else {
                "Choose a folder first.".into()
            };
            return;
        }
        self.index_started = Some(Instant::now());
        self.status = "Building the fast index...".into();
        self.execute(effects);
    }

    #[cfg(feature = "semantic-search")]
    fn semantic_search(&mut self) {
        let query = self.st.config.filters_text.trim().to_string();
        if !self.st.config.index.semantic {
            self.status = "Enable Semantic index in Options, then build the index.".into();
            return;
        }
        if query.is_empty() {
            self.status = "Enter a query before searching semantically.".into();
            return;
        }
        if self.semantic_running {
            self.status = "Semantic search is already running.".into();
            return;
        }
        let roots = semantic_search_roots(&self.st.config);
        if roots.is_empty() {
            self.status = "Choose a folder before semantic search.".into();
            return;
        }
        let key = semantic_search_key(&self.st.config, query.clone(), &roots);
        self.semantic_request = Some(key.clone());
        self.semantic_hits = None;
        self.semantic_error = None;
        self.semantic_running = true;
        self.status = "Searching the local semantic index…".into();
        let tx = self.semantic_tx.clone();
        let worker_query = query.clone();
        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let mut hits = Vec::new();
                let mut indexed = 0usize;
                let mut errors = Vec::new();
                for (root, settings) in roots {
                    match search_core::semantic_index::search_directory_with_settings(
                        &root,
                        &worker_query,
                        10,
                        &settings,
                    ) {
                        Ok(mut found) => {
                            indexed += 1;
                            hits.append(&mut found);
                        }
                        Err(error) => errors.push(error),
                    }
                }
                if indexed == 0 {
                    return Err(errors.join("; "));
                }
                hits.sort_by(|left, right| {
                    right
                        .similarity
                        .total_cmp(&left.similarity)
                        .then_with(|| left.path.cmp(&right.path))
                });
                hits.truncate(10);
                Ok(hits)
            })
            .await
            .unwrap_or_else(|error| Err(format!("semantic search task failed: {error}")));
            let _ = tx.send(SemanticSearchEvent { key, result });
        });
    }

    #[cfg(feature = "semantic-search")]
    fn find_related_documents(&mut self) {
        if !self.st.config.index.semantic {
            self.status = "Enable Semantic index in Options, then build the index.".into();
            return;
        }
        let Some(result) = self.current_result() else {
            self.status = "Select a document first.".into();
            return;
        };
        let source_path = result.full_name.clone();
        let Some(preview) = self
            .preview
            .as_ref()
            .filter(|preview| preview.path == source_path && preview.text.is_some())
        else {
            self.status = "Wait for the selected document preview to finish loading.".into();
            return;
        };
        let query: String = preview
            .text
            .as_ref()
            .expect("checked above")
            .content
            .chars()
            .take(2_000)
            .collect();
        if query.trim().is_empty() {
            self.status = "The selected document has no text to compare.".into();
            return;
        }
        let roots = semantic_search_roots(&self.st.config);
        if roots.is_empty() {
            self.status = "Choose a folder before comparing documents.".into();
            return;
        }
        let key = related_search_key(&self.st.config, source_path.clone(), preview.generation)
            .expect("nonempty roots checked above");
        if self.related_running && self.related_request.as_ref() == Some(&key) {
            self.status = "Related-document search is already running.".into();
            return;
        }
        self.related_request = Some(key.clone());
        self.related_running = true;
        self.related_hits = None;
        self.related_error = None;
        self.invalidate_related_diff();
        self.status = "Looking for possible related documents…".into();
        let tx = self.related_tx.clone();
        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let mut hits = Vec::new();
                let mut indexed = 0usize;
                let mut errors = Vec::new();
                for (root, settings) in roots {
                    match search_core::semantic_index::search_directory_excluding_path_with_settings(
                        &root,
                        &query,
                        100,
                        &settings,
                        &source_path,
                    ) {
                        Ok(mut found) => {
                            indexed += 1;
                            hits.append(&mut found);
                        }
                        Err(error) => errors.push(error),
                    }
                }
                if indexed == 0 {
                    return Err(errors.join("; "));
                }
                Ok(search_core::semantic_index::group_similar_documents(
                    &source_path,
                    &hits,
                    3,
                ))
            })
            .await
            .unwrap_or_else(|error| Err(format!("related-document search task failed: {error}")));
            let _ = tx.send(RelatedSearchEvent { key, result });
        });
    }

    #[cfg(feature = "semantic-search")]
    fn compare_related_document(&mut self, index: usize) {
        let Some(hit) = self.current_related_hits().and_then(|hits| hits.get(index)) else {
            self.status = "Choose a current related document first.".into();
            return;
        };
        let Some(preview) = self.preview.as_ref() else {
            self.status = "Select a source document first.".into();
            return;
        };
        if self
            .current_result()
            .is_none_or(|result| result.full_name != preview.path)
        {
            self.status = "Wait for the selected document preview to update.".into();
            return;
        }
        let key = RelatedDiffKey {
            related: match related_search_key(
                &self.st.config,
                preview.path.clone(),
                preview.generation,
            ) {
                Some(key) => key,
                None => return,
            },
            candidate_path: hit.path.clone(),
        };
        if self.diff_running && self.diff_request.as_ref() == Some(&key) {
            return;
        }
        if let Some(cancel) = self.diff_cancel.take() {
            cancel.cancel();
        }
        self.diff_generation = self.diff_generation.wrapping_add(1);
        let generation = self.diff_generation;
        let cancel = CancellationToken::new();
        self.diff_cancel = Some(cancel.clone());
        self.diff_request = Some(key.clone());
        self.diff_running = true;
        self.diff_result = None;
        self.diff_error = None;
        self.status = "Comparing exact extracted lines…".into();
        let source = key.related.source_path.clone();
        let candidate = key.candidate_path.clone();
        let config = self.st.config.clone();
        let tx = self.diff_tx.clone();
        let worker = self.diff_worker.clone();
        self.rt.spawn(async move {
            let result = tokio::select! {
                _ = cancel.cancelled() => Err("Comparison cancelled.".to_string()),
                permit = worker.acquire_owned() => match permit {
                    Ok(_permit) => async {
                        let source_lines = load_comparison_lines(&source, &config, &cancel).await?;
                        let candidate_lines = load_comparison_lines(&candidate, &config, &cancel).await?;
                        tokio::task::spawn_blocking(move || diff::compare(&source_lines, &candidate_lines, &cancel))
                            .await.map_err(|error| error.to_string())?
                    }.await,
                    Err(error) => Err(error.to_string()),
                }
            };
            let _ = tx.send(RelatedDiffEvent { generation, key, result });
        });
    }

    // ------------------------------------------------------------------ results

    /// The results in table order: filtered, then sorted.
    pub(crate) fn view(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        let results = &self.st.run.results;
        let mut v: Vec<usize> = (0..results.len())
            .filter(|&i| needle.is_empty() || results[i].full_name.to_lowercase().contains(&needle))
            .collect();
        let (key, asc) = self.sort;
        let name = |r: &FileSearchResult| file_name(&r.full_name).to_lowercase();
        match key {
            SortKey::Found => {}
            SortKey::Name => v.sort_by_key(|&i| name(&results[i])),
            SortKey::Hits => v.sort_by_key(|&i| results[i].hits.len()),
            SortKey::Size => v.sort_by_key(|&i| results[i].file_length),
            SortKey::Modified => v.sort_by_key(|&i| results[i].modified),
            SortKey::BestMatch => {
                let key = (results.len(), self.st.config.filters_text.clone());
                let mut cache = self.best_match.borrow_mut();
                if cache.as_ref().map(|(k, _)| k) != Some(&key) {
                    let order = search_core::ordering::relevance_order(
                        results,
                        &crate::toolboxes::search::model::build_settings(&self.st.config),
                    );
                    let mut rank = vec![0usize; results.len()];
                    for (position, &index) in order.iter().enumerate() {
                        rank[index] = position;
                    }
                    *cache = Some((key, rank));
                }
                if let Some((_, rank)) = cache.as_ref() {
                    // Worst first, so the default (descending) view shows the best match on top.
                    v.sort_by_key(|&i| std::cmp::Reverse(rank.get(i).copied().unwrap_or(usize::MAX)));
                }
            }
            #[cfg(feature = "neural-search")]
            SortKey::Relevance => {
                if let Some((ranked_key, scores)) = &self.relevance_scores {
                    if ranked_key == &self.relevance_key() {
                        v.sort_by(|left, right| {
                            let left_score = scores
                                .get(left)
                                .map_or(f32::NEG_INFINITY, |score| score.score);
                            let right_score = scores
                                .get(right)
                                .map_or(f32::NEG_INFINITY, |score| score.score);
                            let score_order = if asc {
                                left_score.total_cmp(&right_score)
                            } else {
                                right_score.total_cmp(&left_score)
                            };
                            score_order.then_with(|| left.cmp(right))
                        });
                        return v;
                    }
                }
                return v;
            }
        }
        if key != SortKey::Found && !asc {
            v.reverse();
        }
        v
    }
    /// Position finder built from the settings the run used, for highlighting.
    fn line_explainer(&self) -> Option<search_core::explain::LineExplainer> {
        search_core::explain::LineExplainer::new(&crate::toolboxes::search::model::build_settings(
            &self.st.config,
        ))
    }
    /// The selected result, or the first shown when nothing is selected.
    pub(crate) fn current(&self) -> Option<usize> {
        if self.sort.0 == SortKey::Found {
            let needle = self.filter.trim().to_lowercase();
            let visible = |i: usize| {
                self.st.run.results.get(i).is_some_and(|r| {
                    needle.is_empty() || r.full_name.to_lowercase().contains(&needle)
                })
            };
            return self.selected.filter(|&i| visible(i)).or_else(|| {
                self.st
                    .run
                    .results
                    .iter()
                    .position(|r| needle.is_empty() || r.full_name.to_lowercase().contains(&needle))
            });
        }
        let view = self.view();
        self.selected
            .filter(|s| view.contains(s))
            .or_else(|| view.first().copied())
    }
    fn current_result(&self) -> Option<&FileSearchResult> {
        self.current().and_then(|i| self.st.run.results.get(i))
    }

    #[cfg(feature = "neural-search")]
    fn relevance_key(&self) -> String {
        format!(
            "{:?}\0{}\0{}\0{}",
            self.st.run.started,
            self.st.config.filters_text,
            self.st.run.results.len(),
            self.feedback_generation
        )
    }

    #[cfg(feature = "neural-search")]
    fn summarize_selected(&mut self) {
        if self.summary_running {
            self.status = "A document summary is already running.".into();
            return;
        }
        let Some(preview) = self.preview.as_ref() else {
            self.status = "Select a search result before summarizing.".into();
            return;
        };
        let Some(text) = preview.text.as_ref() else {
            self.status = "Wait for the selected document to finish loading.".into();
            return;
        };
        let key = SummaryKey {
            path: preview.path.clone(),
            preview_generation: preview.generation,
        };
        let content = text.content.clone();
        let cancel = CancellationToken::new();
        self.summary_request = Some(key.clone());
        self.summary = None;
        self.summary_error = None;
        self.summary_running = true;
        self.summary_cancel = Some(cancel.clone());
        self.status = "Summarizing selected document…".into();
        let tx = self.summary_tx.clone();
        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                search_core::neural_search::extractive_summary(&content, &cancel)
            })
            .await
            .unwrap_or_else(|error| Err(format!("summary worker failed: {error}")));
            let _ = tx.send(SummaryEvent { key, result });
        });
    }

    #[cfg(feature = "neural-search")]
    fn feedback_path(&self) -> PathBuf {
        self.settings_file.with_file_name("search-feedback.json")
    }

    #[cfg(feature = "neural-search")]
    pub(super) fn feedback_enabled(&self) -> bool {
        feedback::enabled(&self.feedback_path()).unwrap_or(false)
    }

    #[cfg(feature = "neural-search")]
    pub(super) fn relevance_score(
        &self,
        index: usize,
    ) -> Option<search_core::relevance::RelevanceExplanation> {
        let (key, scores) = self.relevance_scores.as_ref()?;
        if key != &self.relevance_key() {
            return None;
        }
        scores.get(&index).copied()
    }

    #[cfg(feature = "neural-search")]
    fn rank_relevance(&mut self) {
        if self.st.run.is_running {
            self.status = "Wait for exact search to finish before ranking its results.".into();
            return;
        }
        if self.st.run.results.is_empty() {
            self.status = "Run an exact search before ranking results.".into();
            return;
        }
        let query = self.st.config.filters_text.trim().to_string();
        if query.is_empty() {
            self.status = "Enter a query before ranking results.".into();
            return;
        }
        if self.relevance_running {
            self.status = "Local relevance ranking is already running.".into();
            return;
        }
        let candidates = self
            .st
            .run
            .results
            .iter()
            .map(|result| search_core::relevance::RelevanceCandidate {
                filename: file_name(&result.full_name).to_string(),
                matched_text: result
                    .hits
                    .iter()
                    .take(3)
                    .map(|hit| hit.match_line.chars().take(512).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n"),
                hit_count: result.hits.len(),
                proximity_min_range: result.proximity_min_range,
                modified_unix: result.modified.timestamp(),
            })
            .collect::<Vec<_>>();
        let (feedback_enabled, feedback_examples, feedback_error) =
            match feedback::snapshot(&self.feedback_path()) {
                Ok((enabled, examples)) => (enabled, examples, None),
                Err(error) => (false, Vec::new(), Some(error)),
            };
        let key = self.relevance_key();
        let tx = self.relevance_tx.clone();
        self.relevance_request = Some(key.clone());
        self.relevance_running = true;
        self.relevance_scores = None;
        self.sort = (SortKey::Relevance, false);
        self.status = format!("Ranking {} exact results locally…", candidates.len());
        self.rt.spawn(async move {
            let scores = tokio::task::spawn_blocking(move || {
                search_core::relevance::rank_results(&query, &candidates)
                    .map(|mut scores| {
                        if feedback_enabled {
                            search_core::relevance::apply_feedback(&mut scores, &feedback_examples);
                        }
                        scores
                    })
                    .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(RelevanceRankEvent {
                key,
                scores,
                feedback_error,
            });
        });
    }

    #[cfg(feature = "neural-search")]
    fn vote_relevance(&mut self, index: usize, label: search_core::relevance::FeedbackLabel) {
        if self.relevance_running || self.st.run.is_running {
            self.status = "Wait for relevance ranking to finish before recording feedback.".into();
            return;
        }
        if !self.feedback_enabled() {
            self.status =
                "Enable local relevance feedback in Tools before labeling results.".into();
            return;
        }
        let Some(score) = self.relevance_score(index) else {
            self.status = "Rank the current exact results before labeling a result.".into();
            return;
        };
        let example = search_core::relevance::RelevanceFeedback {
            label,
            match_text_similarity: score.match_text_similarity,
            filename_similarity: score.filename_similarity,
            exact_evidence: score.exact_evidence,
            proximity: score.proximity,
            recency: score.recency,
        };
        match feedback::record(&self.feedback_path(), example) {
            Ok(()) => {
                self.feedback_generation = self.feedback_generation.wrapping_add(1);
                self.status = "Local preference saved; updating relevance order…".into();
                self.rank_relevance();
            }
            Err(error) => self.status = format!("Cannot save relevance feedback: {error}"),
        }
    }

    fn refresh_preview(&mut self) -> bool {
        let Some(result) = self.current_result() else {
            if let Some(cancel) = self.preview_cancel.take() {
                cancel.cancel();
            }
            #[cfg(feature = "semantic-search")]
            {
                self.invalidate_related_diff();
                self.related_request = None;
                self.related_running = false;
                self.related_hits = None;
            }
            #[cfg(feature = "neural-search")]
            {
                if let Some(cancel) = &self.summary_cancel {
                    cancel.cancel();
                }
                if !self.summary_running {
                    self.summary_cancel = None;
                    self.summary_request = None;
                }
                self.summary = None;
                self.summary_error = None;
            }
            return self.preview.take().is_some();
        };
        let path = result.full_name.clone();
        if self.preview.as_ref().is_some_and(|p| p.path == path) {
            return false;
        }
        let cached = (result.total_line_count as usize <= result.lines_cache.len()
            && !result.lines_cache.is_empty())
        .then(|| view::full_text(result, &result.lines_cache, self.line_explainer().as_ref()));
        #[cfg(feature = "semantic-search")]
        {
            self.invalidate_related_diff();
            self.related_request = None;
            self.related_running = false;
            self.related_hits = None;
            self.related_error = None;
        }
        self.preview_generation = self.preview_generation.wrapping_add(1);
        let generation = self.preview_generation;
        #[cfg(feature = "neural-search")]
        {
            if let Some(cancel) = &self.summary_cancel {
                cancel.cancel();
            }
            if !self.summary_running {
                self.summary_cancel = None;
                self.summary_request = None;
            }
            self.summary = None;
            self.summary_error = None;
        }
        if let Some(cancel) = self.preview_cancel.take() {
            cancel.cancel();
        }
        if let Some(text) = cached {
            #[cfg(feature = "neural-search")]
            let entity_text = text.content.to_string();
            self.preview = Some(Preview {
                generation,
                related: crate::toolboxes::search::indexing::related_reasons(&path),
                path,
                text: Some(text),
                status: String::new(),
                #[cfg(feature = "neural-search")]
                entities: None,
            });
            #[cfg(feature = "neural-search")]
            self.start_entity_analysis(generation, entity_text);
            return true;
        }
        self.preview = Some(Preview {
            generation,
            related: crate::toolboxes::search::indexing::related_reasons(&path),
            path: path.clone(),
            text: None,
            status: "Loading the full document...".into(),
            #[cfg(feature = "neural-search")]
            entities: None,
        });
        let tx = self.preview_tx.clone();
        let cancel = CancellationToken::new();
        self.preview_cancel = Some(cancel.clone());
        let config = self.st.config.clone();
        self.rt.spawn(async move {
            let loaded = async {
                let meta = tokio::fs::metadata(&path)
                    .await
                    .map_err(|e| e.to_string())?;
                if meta.len() as f64 > config.max_file_size_mb * 1_048_576.0 {
                    return Err("The file now exceeds the search size limit.".to_string());
                }
                let bytes = search_core::file_reader::read_file_bytes_robust(
                    &path,
                    config.file_timeout_seconds.max(1) as u64,
                    0,
                    0,
                    None,
                    &cancel,
                )
                .await
                .map_err(|e| format!("{e:?}"))?;
                if cancel.is_cancelled() {
                    return Err("Cancelled".into());
                }
                let ext = Path::new(&path)
                    .extension()
                    .map(|s| format!(".{}", s.to_string_lossy().to_lowercase()))
                    .unwrap_or_default();
                let progress_tx = tx.clone();
                tokio::task::spawn_blocking(move || {
                    let mut progress = |page: i32, _elapsed: std::time::Duration| {
                        let _ = progress_tx.send(PreviewEvent::Progress(
                            generation,
                            format!("Extracting document text: page {page}..."),
                        ));
                    };
                    search_core::extraction::extract_lines_by_extension(
                        &ext,
                        &bytes,
                        config.pdf_timeout_seconds.max(1) as u64,
                        Some(&mut progress),
                        config.ocr_scanned_pdfs,
                    )
                    .map(|v| v.lines)
                    .map_err(|e| format!("{e:?}"))
                })
                .await
                .map_err(|e| e.to_string())?
            }
            .await;
            if !cancel.is_cancelled() {
                let _ = tx.send(PreviewEvent::Done(generation, loaded));
            }
        });
        true
    }

    #[cfg(feature = "semantic-search")]
    fn invalidate_related_diff(&mut self) {
        if let Some(cancel) = self.diff_cancel.take() {
            cancel.cancel();
        }
        self.diff_generation = self.diff_generation.wrapping_add(1);
        self.diff_request = None;
        self.diff_running = false;
        self.diff_result = None;
        self.diff_error = None;
    }

    #[cfg(feature = "neural-search")]
    fn start_entity_analysis(&mut self, generation: u64, text: String) {
        let Some(preview) = self.preview.as_mut() else {
            return;
        };
        if preview.generation != generation || preview.entities.is_some() {
            return;
        }
        let path = preview.path.clone();
        let cancel = self
            .preview_cancel
            .get_or_insert_with(CancellationToken::new)
            .clone();
        let worker_cancel = cancel.clone();
        let tx = self.entity_tx.clone();
        preview.entities = Some(Err("Analyzing engineering entities…".into()));
        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                search_core::entities::extract_entities(&text, &worker_cancel)
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            if !cancel.is_cancelled() {
                let _ = tx.send(EntityEvent {
                    generation,
                    path,
                    result,
                });
            }
        });
    }

    fn move_selection(&mut self, delta: i32) {
        let view = self.view();
        if view.is_empty() {
            return;
        }
        let at = self
            .current()
            .and_then(|c| view.iter().position(|&v| v == c))
            .unwrap_or(0) as i32;
        self.selected = Some(view[(at + delta).clamp(0, view.len() as i32 - 1) as usize]);
        self.match_index = 0;
        self.match_scroll_request = self.match_scroll_request.wrapping_add(1);
    }
    fn move_match(&mut self, delta: i32) {
        let count = self.current_result().map_or(0, |r| r.hits.len());
        if count > 0 {
            self.match_index = (self.match_index as i32 + delta).rem_euclid(count as i32) as usize;
            self.match_scroll_request = self.match_scroll_request.wrapping_add(1);
        }
    }
    /// Extensions among the results with their counts, most common first.
    pub(crate) fn types(&self) -> Vec<(String, usize)> {
        crate::format::extension_breakdown(&self.st.run.results)
    }
    fn corpus_term_suggestions(&self) -> Vec<String> {
        let groups = model::parse_filter_groups(&self.st.config.filters_text);
        if groups.len() != 1 || groups[0].filters.len() != 1 || groups[0].mode.is_some() {
            return Vec::new();
        }
        #[cfg(feature = "neural-search")]
        {
            let lines: Vec<String> = self
                .st
                .run
                .results
                .iter()
                .flat_map(|result| &result.hits)
                .take(40)
                .map(|hit| hit.match_line.clone())
                .collect();
            return search_core::neural_search::corpus_term_candidates(
                &groups[0].filters[0],
                &lines,
                24,
            );
        }
        #[cfg(not(feature = "neural-search"))]
        {
            let query_words: std::collections::HashSet<String> = groups[0].filters[0]
                .split_whitespace()
                .map(|word| word.to_lowercase())
                .collect();
            let stop_words = [
                "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it",
                "of", "on", "or", "the", "to", "with",
            ];
            let mut counts = std::collections::HashMap::<String, usize>::new();
            let mut sampled = 0;
            for result in &self.st.run.results {
                for hit in &result.hits {
                    if sampled == 40 {
                        break;
                    }
                    sampled += 1;
                    let mut seen_on_line = std::collections::HashSet::new();
                    for raw in hit
                        .match_line
                        .chars()
                        .take(2048)
                        .collect::<String>()
                        .split_whitespace()
                    {
                        let word = raw
                            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '#')
                            .to_lowercase();
                        if word.chars().count() < 2
                            || stop_words.contains(&word.as_str())
                            || query_words.contains(&word)
                        {
                            continue;
                        }
                        if seen_on_line.insert(word.clone()) {
                            *counts.entry(word).or_default() += 1;
                        }
                    }
                }
                if sampled == 40 {
                    break;
                }
            }
            let mut words: Vec<_> = counts.into_iter().collect();
            words.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            words.into_iter().take(3).map(|(word, _)| word).collect()
        }
    }

    fn request_neural_suggestions(&mut self) {
        #[cfg(feature = "neural-search")]
        {
            let (context, candidates) = view::neural_suggestion_inputs(self);
            let key = format!("{context}\0{}", candidates.join("\0"));
            if self.neural_request.as_deref() == Some(&key) {
                return;
            }
            self.neural_request = Some(key.clone());
            self.neural_order = None;
            if candidates.is_empty() || self.neural_running {
                return;
            }
            self.neural_running = true;
            let tx = self.neural_tx.clone();
            self.rt.spawn(async move {
                let order = tokio::task::spawn_blocking(move || {
                    search_core::neural_search::rank_suggestions(&context, &candidates)
                        .map(|scores| scores.into_iter().map(|score| score.index).collect())
                        .map_err(|error| error.to_string())
                })
                .await
                .unwrap_or_else(|error| Err(error.to_string()));
                let _ = tx.send(NeuralRankEvent { key, order });
            });
        }
    }

    fn with_current(
        &mut self,
        f: impl FnOnce(&FileSearchResult, &model::SearchToolConfig) -> Effect,
        what: &str,
    ) {
        match self.current_result() {
            Some(r) => {
                let effect = f(r, &self.st.config);
                self.status = format!("{what}: {}", file_name(&r.full_name));
                self.execute(vec![effect]);
            }
            None => self.status = "Select a result first.".into(),
        }
    }

    // ------------------------------------------------------------------ settings

    fn edit_setting(&mut self, idx: usize) {
        self.edit = Some(Edit {
            setting: Some(idx),
            text: settings_view::setting_text(&self.st.config, idx),
        });
    }
    fn apply_edit(&mut self) {
        let Some(edit) = self.edit.take() else { return };
        match edit.setting {
            Some(idx) => {
                let before = self.st.config.clone();
                settings_view::set_setting_text(&mut self.st.config, idx, &edit.text);
                let label = setting_label(idx);
                self.status = if self.st.config == before
                    && edit.text.trim() != settings_view::setting_text(&before, idx).trim()
                {
                    format!(
                        "{label}: not a valid value, kept {}.",
                        settings_view::setting_display(&before, idx)
                    )
                } else {
                    format!(
                        "{label}: {}",
                        settings_view::setting_display(&self.st.config, idx)
                    )
                };
                self.save_settings();
            }
            None => {
                let name = edit.text.trim();
                if name.is_empty() {
                    self.status = "A preset needs a name.".into();
                    return;
                }
                self.st.saved_presets.push(persistence::SavedPreset {
                    name: name.to_string(),
                    settings: persistence::PersistedSearchSettings::from(&self.st.config),
                });
                self.status = format!("Saved preset '{name}'.");
                self.save_settings();
            }
        }
    }
    fn step_setting(&mut self, idx: usize) {
        settings_view::step_setting(&mut self.st.config, idx);
        self.status = format!(
            "{}: {}",
            setting_label(idx),
            settings_view::setting_display(&self.st.config, idx)
        );
        self.save_settings();
    }

    /// The system's folder picker, opened in the first of `starts` that is a folder (or the working folder).
    fn pick(&mut self, starts: &[&str]) -> Option<PathBuf> {
        let start = starts
            .iter()
            .map(|s| PathBuf::from(s.trim()))
            .find(|p| !p.as_os_str().is_empty() && p.is_dir())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let picked = (self.pick_folder)(&start);
        if picked.is_none() {
            self.status = "No folder chosen.".into();
        }
        picked
    }
    /// Browse: the folder to search, starting in the current one.
    fn browse(&mut self) {
        let current = self.st.config.search_path.clone();
        if let Some(dir) = self.pick(&[&current]) {
            self.st.config.search_path = dir.to_string_lossy().into_owned();
            self.status = format!(
                "Folder: {}. Type what to find and press Enter.",
                dir.display()
            );
            self.save_settings();
        }
    }
    /// Fills "more folders" with the other indexed folders from the catalog; Find then searches them all.
    fn also_indexed_folders(&mut self) {
        let main = self.st.config.search_path.trim().to_lowercase();
        let others: Vec<String> = crate::toolboxes::search::indexing::catalogued_folders()
            .into_iter()
            .filter(|p| p.trim().to_lowercase() != main)
            .collect();
        self.status = if others.is_empty() {
            "No other indexed folders yet: build an index in another folder first.".into()
        } else {
            let n = others.len();
            self.st.config.search_paths_extra = others;
            self.save_settings();
            format!("Also searching {n} other indexed folder(s). Press Find.")
        };
    }
    /// Browse for the output folder, starting in it (or in the folder being searched).
    fn browse_output(&mut self) {
        let (current, searched) = (
            self.st.config.output_folder.clone(),
            self.st.config.search_path.clone(),
        );
        if let Some(dir) = self.pick(&[&current, &searched]) {
            self.st.config.output_folder = dir.to_string_lossy().into_owned();
            self.status = format!("Reports and exports go to {}.", dir.display());
            self.save_settings();
        }
    }

    fn act_search(&mut self, a: SearchAct) {
        match a {
            SearchAct::Run => self.run(),
            SearchAct::Cancel => self.cancel(),
            SearchAct::Select(i) => {
                if i < self.st.run.results.len() {
                    self.selected = Some(i);
                    self.match_index = 0;
                    self.match_scroll_request = self.match_scroll_request.wrapping_add(1);
                    self.step = "Results";
                }
            }
            SearchAct::Open(i) => {
                self.selected = Some(i);
                self.match_index = 0;
                self.match_scroll_request = self.match_scroll_request.wrapping_add(1);
                self.act_search(SearchAct::OpenSelected);
            }
            SearchAct::PreviousMatch => self.move_match(-1),
            SearchAct::NextMatch => self.move_match(1),
            SearchAct::OpenSelected => {
                self.with_current(|r, _| Effect::OpenPath(r.full_name.clone()), "Opened")
            }
            SearchAct::OpenRelatedFile(i) => {
                let other = self.preview.as_ref().and_then(|p| p.related.get(i)).map(|r| r.other.clone()).filter(|o| !o.is_empty());
                match other {
                    Some(path) => {
                        self.status = format!("Opened: {}", file_name(&path));
                        self.execute(vec![Effect::OpenPath(path)]);
                    }
                    None => self.status = "That related file is no longer listed.".into(),
                }
            }
            SearchAct::Reveal => self.with_current(
                |r, _| {
                    Effect::OpenPath(
                        Path::new(&r.full_name)
                            .parent()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    )
                },
                "Showing the folder of",
            ),
            SearchAct::CopyPath => self.with_current(
                |r, _| Effect::CopyToClipboard(r.full_name.clone()),
                "Copied the path of",
            ),
            SearchAct::ExportHits => self.with_current(export_hits, "Exported the matches of"),
            SearchAct::OpenReport => match self.st.run.last_report_path.clone() {
                Some(p) => {
                    self.status = "Opened the report.".into();
                    self.execute(vec![Effect::OpenPath(p)]);
                }
                None => {
                    self.status =
                        "No report yet: it is written when a search finishes (HTML export on)."
                            .into()
                }
            },
            SearchAct::SortBy(k) => {
                #[cfg(feature = "neural-search")]
                if k == SortKey::Relevance {
                    self.rank_relevance();
                    return;
                }
                self.sort = if self.sort.0 == k {
                    (k, !self.sort.1)
                } else {
                    (k, matches!(k, SortKey::Name))
                };
            }
            SearchAct::OnlyType(t) => {
                self.filter = t
                    .and_then(|i| self.types().get(i).map(|(e, _)| e.clone()))
                    .unwrap_or_default();
                self.step = "Results";
            }
            SearchAct::Edit(idx) => match settings_view::setting(idx).map(|s| s.4) {
                Some(settings_view::SettingKind::Typed)
                    if idx == settings_view::EXTENSIONS_FIELD =>
                {
                    self.act_search(SearchAct::PickExtensions)
                }
                Some(settings_view::SettingKind::Typed) if idx == SETTING_OUTPUT => {
                    self.browse_output()
                }
                Some(settings_view::SettingKind::Typed) => self.edit_setting(idx),
                Some(_) => self.step_setting(idx),
                None => {}
            },
            SearchAct::Step(idx) => self.step_setting(idx),
            SearchAct::Match(m) => {
                self.st.config.match_mode = m;
                self.status = format!(
                    "Match: {}",
                    model::mode_label(
                        model::GroupMode::Match(m, None),
                        self.st.config.proximity_lines
                    )
                );
                self.save_settings();
            }
            SearchAct::ApplyEdit => self.apply_edit(),
            SearchAct::CancelEdit => self.edit = None,
            SearchAct::BrowseFolder => self.browse(),
            SearchAct::AlsoIndexedFolders => self.also_indexed_folders(),
            SearchAct::BrowseOutput => self.browse_output(),
            SearchAct::BuildIndex => self.build_index(),
            #[cfg(feature = "neural-search")]
            SearchAct::SaveAlert => self.save_alert(),
            #[cfg(feature = "neural-search")]
            SearchAct::CaptureAlert(i) => self.run_saved_alert(i, true),
            #[cfg(feature = "neural-search")]
            SearchAct::CheckAlert(i) => self.run_saved_alert(i, false),
            #[cfg(feature = "neural-search")]
            SearchAct::DeleteAlert(i) => self.delete_alert(i),
            #[cfg(feature = "semantic-search")]
            SearchAct::SemanticSearch => self.semantic_search(),
            SearchAct::FindAs => self.find_as = self.find_as.next(),
            #[cfg(feature = "nl-find")]
            SearchAct::NlUnderstand => self.nl_understand(),
            #[cfg(feature = "nl-find")]
            SearchAct::NlFind => self.nl_find(),
            #[cfg(feature = "nl-find")]
            SearchAct::NlCancel => self.nl_cancel(),
            #[cfg(feature = "nl-find")]
            SearchAct::NlRemove(i) => self.nl_remove(i),
            #[cfg(feature = "nl-find")]
            SearchAct::NlMerge(i) => self.nl_merge(i),
            #[cfg(feature = "nl-find")]
            SearchAct::NlOpen(file, condition) => self.nl_open(file, condition),
            #[cfg(feature = "neural-search")]
            SearchAct::SummarizeSelected => self.summarize_selected(),
            #[cfg(feature = "semantic-search")]
            SearchAct::OpenSemantic(i) => {
                if let Some((path, first_line, last_line)) = self
                    .current_semantic_hits()
                    .and_then(|hits| hits.get(i))
                    .map(|hit| (hit.path.clone(), hit.first_line, hit.last_line))
                {
                    self.status = format!(
                        "Semantic match: {} lines {}–{}.",
                        file_name(&path),
                        first_line,
                        last_line
                    );
                    self.execute(vec![Effect::OpenPath(path)]);
                }
            }
            #[cfg(feature = "semantic-search")]
            SearchAct::FindRelated => self.find_related_documents(),
            #[cfg(feature = "semantic-search")]
            SearchAct::OpenRelated(i) => {
                if let Some((path, first_line, last_line)) = self
                    .current_related_hits()
                    .and_then(|hits| hits.get(i))
                    .map(|hit| (hit.path.clone(), hit.first_line, hit.last_line))
                {
                    self.status = format!(
                        "Possible related passage: {} lines {}–{}.",
                        file_name(&path),
                        first_line,
                        last_line
                    );
                    self.execute(vec![Effect::OpenPath(path)]);
                }
            }
            #[cfg(feature = "semantic-search")]
            SearchAct::CompareRelated(i) => self.compare_related_document(i),
            SearchAct::CancelIndex => {
                if let Some(c) = &self.st.index_cancel {
                    c.cancel();
                    self.status = "Stopping the index build (what is indexed is kept)...".into();
                }
            }
            SearchAct::UpdateStaleIndex => {
                self.stale = None;
                self.build_index();
            }
            SearchAct::DismissStale => self.stale = None,
            SearchAct::PickExtensions => {
                let root = self.st.config.search_path.trim().to_string();
                if root.is_empty() {
                    self.status = "Choose a folder first: the file types are read from it.".into();
                    return;
                }
                self.status = "Reading the file types in the folder...".into();
                let effect = Effect::ScanExtensions {
                    root,
                    exclude_folders: model::parse_list(&self.st.config.exclude_folders_text),
                    include_hidden: self.st.config.include_hidden,
                };
                self.execute(vec![effect]);
            }
            SearchAct::ToggleExtension(i) => {
                let picker = &mut self.st.extension_picker;
                if let Some(ext) = picker.available.get(i).cloned() {
                    if !picker.selected.remove(&ext) {
                        picker.selected.insert(ext);
                    }
                }
            }
            SearchAct::AllExtensions => self.st.extension_picker.selected.clear(),
            SearchAct::CloseExtensions => {
                self.st.extension_picker.open = false;
                if search::commit_extension_picker(&mut self.st) {
                    self.save_settings();
                }
                self.status = match &self.st.config.selected_extensions {
                    Some(list) => format!("Searching only {}.", list.join(", ")),
                    None => "Searching every supported file type.".into(),
                };
            }
            SearchAct::ApplyRecent(i) => {
                if let Some(r) = self.st.recent_searches.get(i).cloned() {
                    self.st.config.search_path = r.search_path;
                    self.st.config.filters_text = r.filters_text;
                    self.status = "Recent search loaded: press Enter to run it.".into();
                }
            }
            SearchAct::ApplyRecentFilters(i) => {
                if let Some(recent) = self.st.recent_searches.get(i) {
                    self.st.config.filters_text = recent.filters_text.clone();
                    self.status = "Search suggestion applied; press Enter to run it.".into();
                }
            }
            SearchAct::ApplyRecentFolder(i) => {
                if let Some(recent) = self.st.recent_searches.get(i) {
                    self.st.config.search_path = recent.search_path.clone();
                    self.status = "Folder suggestion applied; press Enter to search it.".into();
                }
            }
            SearchAct::ApplyCorrection(i) => {
                let query = &self.st.config.filters_text;
                if !self.st.run.is_running
                    && self.st.run.started.is_some()
                    && self.st.run.results.is_empty()
                    && self.completed_search_query.as_deref() == Some(query)
                {
                    if let Some(correction) = search_core::synonyms::typo_corrections(query).get(i)
                    {
                        self.st.config.filters_text = correction.query.clone();
                        self.status =
                            "Correction applied; press Enter to run the exact search.".into();
                    }
                }
            }
            SearchAct::WidenProximity => {
                if self.can_widen_proximity() {
                    let widened = self.st.config.proximity_lines.saturating_add(3);
                    if widened > self.st.config.proximity_lines {
                        self.st.config.proximity_lines = widened;
                        self.save_settings();
                        self.status = format!(
                            "Proximity window widened to {widened} lines; rerunning exact search."
                        );
                        self.run();
                    }
                }
            }
            SearchAct::AddCorpusTerm(i) => {
                if let Some(term) = self.corpus_term_suggestions().get(i).cloned() {
                    self.st.config.filters_text =
                        format!("{}, {term}", self.st.config.filters_text.trim_end());
                    self.status =
                        "Term added from matching lines; press Enter to run the expanded search."
                            .into();
                }
            }
            SearchAct::AddSynonym(i) => {
                if !view::apply_synonym(self, i) {
                    self.status = "That synonym suggestion is no longer available.".into();
                }
            }
            SearchAct::RemoveRecent(i) => {
                if i < self.st.recent_searches.len() {
                    self.st.recent_searches.remove(i);
                    self.status = "Recent search removed.".into();
                    self.save_settings();
                }
            }
            SearchAct::ApplyPreset(i) => {
                if let Some(p) = self.st.saved_presets.get(i).cloned() {
                    p.settings.apply_to(&mut self.st.config);
                    self.show_exclude |= !self.st.config.exclude_filters_text.trim().is_empty();
                    self.status = format!("Preset '{}' applied.", p.name);
                    self.save_settings();
                }
            }
            SearchAct::DeletePreset(i) => {
                if i < self.st.saved_presets.len() {
                    let p = self.st.saved_presets.remove(i);
                    self.status = format!("Preset '{}' deleted.", p.name);
                    self.save_settings();
                }
            }
            SearchAct::SavePreset => {
                self.edit = Some(Edit {
                    setting: None,
                    text: String::new(),
                });
            }
            SearchAct::ClearRecents => {
                self.st.recent_searches.clear();
                self.status = "Recent searches cleared.".into();
                self.save_settings();
            }
            #[cfg(feature = "neural-search")]
            SearchAct::VoteRelevance(index, label) => self.vote_relevance(index, label),
            #[cfg(feature = "neural-search")]
            SearchAct::ToggleRelevanceFeedback => {
                let path = self.feedback_path();
                match feedback::enabled(&path)
                    .and_then(|enabled| feedback::set_enabled(&path, !enabled).map(|()| !enabled))
                {
                    Ok(enabled) => {
                        self.feedback_generation = self.feedback_generation.wrapping_add(1);
                        self.status = if enabled {
                            "Local relevance feedback enabled. Results remain exact.".into()
                        } else {
                            "Local relevance feedback disabled; stored judgments remain available."
                                .into()
                        };
                        if !self.st.run.is_running
                            && !self.st.run.results.is_empty()
                            && !self.relevance_running
                        {
                            self.rank_relevance();
                        }
                    }
                    Err(error) => {
                        self.status = format!("Cannot update relevance feedback: {error}")
                    }
                }
            }
            #[cfg(feature = "neural-search")]
            SearchAct::ResetRelevanceFeedback => match feedback::reset(&self.feedback_path()) {
                Ok(()) => {
                    self.feedback_generation = self.feedback_generation.wrapping_add(1);
                    self.status = "Local relevance feedback cleared and disabled.".into();
                    if !self.st.run.is_running
                        && !self.st.run.results.is_empty()
                        && !self.relevance_running
                    {
                        self.rank_relevance();
                    }
                }
                Err(error) => self.status = format!("Cannot clear relevance feedback: {error}"),
            },
            #[cfg(feature = "neural-search")]
            SearchAct::ExportRelevanceFeedback => {
                if let Some(destination) = rfd::FileDialog::new()
                    .set_title("Export local relevance feedback")
                    .set_file_name("search-feedback-export.json")
                    .save_file()
                {
                    match feedback::export(&self.feedback_path(), &destination) {
                        Ok(()) => {
                            self.status =
                                format!("Local feedback exported to {}.", destination.display())
                        }
                        Err(error) => {
                            self.status = format!("Cannot export local feedback: {error}")
                        }
                    }
                }
            }
            SearchAct::StepTab(s) => self.step = s,
            SearchAct::ToggleExclude => {
                self.show_exclude = !self.show_exclude;
                let terms = model::parse_list(&self.st.config.exclude_filters_text);
                self.status = match (self.show_exclude, terms.is_empty()) {
                    (true, _) => "Type what to leave out; the switch beside it picks whole files or only the lines.".into(),
                    (false, true) => "Exclude hidden.".into(),
                    (false, false) => format!("Exclude hidden; still leaving out {}.", terms.join(", ")),
                };
            }
            SearchAct::ExcludeScope(scope) => {
                self.st.config.exclude_scope = scope;
                self.status = format!(
                    "{}: {}",
                    setting_label(SETTING_EXCLUDE_SCOPE),
                    settings_view::setting_display(&self.st.config, SETTING_EXCLUDE_SCOPE)
                );
                self.save_settings();
            }
        }
    }
}

/// The operating system's folder picker (modal; on the window's thread, as macOS requires).
fn system_folder_picker(start: &Path) -> Option<PathBuf> {
    #[cfg(feature = "gpu-viewer")]
    {
        rfd::FileDialog::new()
            .set_title("Folder to search")
            .set_directory(start)
            .pick_folder()
    }
    #[cfg(not(feature = "gpu-viewer"))]
    {
        let _ = start;
        None
    }
}

fn export_hits(r: &FileSearchResult, config: &model::SearchToolConfig) -> Effect {
    let stem = Path::new(&r.full_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "result".into());
    let dir = if config.output_folder.trim().is_empty() {
        Path::new(&r.full_name)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    } else {
        PathBuf::from(config.output_folder.trim())
    };
    let path = dir
        .join(format!("{}_hits.txt", model::sanitize_file_name(&stem)))
        .to_string_lossy()
        .into_owned();
    Effect::WriteTextFileAndOpen {
        path,
        contents: model::hits_as_text(r),
    }
}

pub(crate) fn file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

fn setting_label(idx: usize) -> &'static str {
    settings_view::setting(idx).map_or("", |s| s.2)
}

pub(crate) fn act(a: SearchAct) -> crate::gpu_viewer::controller::UiAction {
    crate::gpu_viewer::controller::UiAction::Editor(EditorAction::Search(a))
}

impl Authoring for SearchSession {
    fn bench(&self) -> Bench {
        Bench::Static
    }
    fn benches(&self) -> Vec<Bench> {
        Vec::new()
    }
    fn path(&self) -> &Path {
        &self.settings_file
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::Search(a) = action {
            self.act_search(a);
        }
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        match key {
            Key::Up => self.move_selection(-1),
            Key::Down => self.move_selection(1),
            Key::Enter if mods.command => self.run(),
            Key::Enter => self.act_search(SearchAct::OpenSelected),
            Key::Escape if self.edit.is_some() => self.edit = None,
            Key::Escape if self.st.extension_picker.open => {
                self.act_search(SearchAct::CloseExtensions)
            }
            Key::Escape if self.st.run.is_running || self.st.index_run.is_building => self.cancel(),
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'c') => {
                self.act_search(SearchAct::CopyPath)
            }
            Key::Char(c) if mods.command && c.eq_ignore_ascii_case(&'e') => {
                self.act_search(SearchAct::ExportHits)
            }
            _ => return KeyResult::Unhandled,
        }
        KeyResult::Handled
    }
    fn click(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) -> bool {
        false
    }
    fn select_at(&mut self, _px: [f32; 2], _size: [f32; 2], _cam: &Camera) {}
    fn scene(&mut self) -> Result<Scene, String> {
        Ok(Scene {
            label: "search".into(),
            positions: vec![[0.0; 3]; 3],
            triangles: vec![0, 1, 2],
            samples: vec![[0.0; 4]; 3],
            times: vec![0.0],
            range: [0.0, 1.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: String::new(),
            fields: Vec::new(),
        })
    }
    fn needs_scene(&self) -> bool {
        false
    }
    fn poll(&mut self) -> bool {
        let mut changed = false;
        self.poll_watcher();
        self.request_neural_suggestions();
        #[cfg(feature = "neural-search")]
        while let Ok(event) = self.neural_rx.try_recv() {
            self.neural_running = false;
            if self.neural_request.as_deref() == Some(&event.key) {
                if let Ok(order) = event.order {
                    self.neural_order = Some((event.key, order));
                    changed = true;
                }
            }
        }
        #[cfg(feature = "neural-search")]
        while let Ok(event) = self.summary_rx.try_recv() {
            if self.summary_request.as_ref() == Some(&event.key) {
                self.summary_running = false;
                self.summary_cancel = None;
                let current = self.preview.as_ref().is_some_and(|preview| {
                    preview.path == event.key.path
                        && preview.generation == event.key.preview_generation
                        && preview.text.is_some()
                        && self
                            .current_result()
                            .is_some_and(|result| result.full_name == event.key.path)
                });
                if current {
                    match event.result {
                        Ok(summary) => {
                            self.status = format!(
                                "Summary selected {} source lines from {} sampled lines.",
                                summary.lines.len(),
                                summary.sampled_lines
                            );
                            self.summary = Some((event.key, summary));
                        }
                        Err(error) => {
                            self.status = format!("Document summary unavailable: {error}");
                            self.summary_error = Some(error);
                        }
                    }
                }
                changed = true;
            }
        }
        #[cfg(feature = "neural-search")]
        while let Ok(event) = self.relevance_rx.try_recv() {
            self.relevance_running = false;
            if self.relevance_request.as_deref() == Some(&event.key) {
                match event.scores {
                    Ok(scores) => {
                        self.relevance_scores = Some((
                            event.key,
                            scores
                                .into_iter()
                                .map(|score| (score.index, score))
                                .collect(),
                        ));
                        self.rows_cache.get_mut().take();
                        self.status =
                            "Exact results sorted by local relevance. Exact matches are unchanged."
                                .into();
                    }
                    Err(error) => {
                        self.status = format!(
                            "Relevance ranking unavailable; exact result order kept. {error}"
                        )
                    }
                }
                if let Some(error) = event.feedback_error {
                    self.status = format!("Feedback unavailable; baseline ranking kept. {error}");
                }
                changed = true;
            }
        }
        #[cfg(feature = "neural-search")]
        self.request_neural_suggestions();
        #[cfg(feature = "neural-search")]
        while let Ok(event) = self.alert_rx.try_recv() {
            match event {
                AlertEvent::Progress {
                    id,
                    generation,
                    text,
                } if self.alert_jobs.get(&id) == Some(&generation) => {
                    self.alert_status = Some(text);
                    changed = true;
                }
                AlertEvent::Finished {
                    id,
                    generation,
                    capture,
                    result,
                } if self.alert_jobs.get(&id) == Some(&generation)
                    && self.alerts.iter().any(|record| record.alert.id == id) =>
                {
                    self.alert_jobs.remove(&id);
                    self.alert_cancels.remove(&id);
                    let index = self
                        .alerts
                        .iter()
                        .position(|record| record.alert.id == id)
                        .unwrap();
                    match result {
                        Ok((record, changes)) => {
                            let saved = if self.persist {
                                alerts::save(&alerts::directory(&self.settings_file), &record)
                            } else {
                                Ok(())
                            };
                            match saved {
                                Ok(()) => {
                                    self.alerts[index] = record.clone();
                                    self.last_alert_changes = if capture {
                                        None
                                    } else {
                                        Some((record.alert.name.clone(), changes.clone()))
                                    };
                                    let summary = if capture {
                                        format!("{}: baseline captured.", record.alert.name)
                                    } else if changes.is_empty() {
                                        format!(
                                            "{}: no new or changed exact-hit files.",
                                            record.alert.name
                                        )
                                    } else {
                                        format!("{}: {} new or changed exact-hit files; similarity scores are review aids.", record.alert.name, changes.len())
                                    };
                                    self.alert_status = Some(summary.clone());
                                    self.status = summary;
                                }
                                Err(error) => {
                                    self.alert_status = Some(format!("Alert save failed: {error}"));
                                    self.status = self.alert_status.clone().unwrap_or_default();
                                }
                            }
                        }
                        Err(error) => {
                            self.alert_status = Some(format!("Alert check failed: {error}"));
                            self.status = self.alert_status.clone().unwrap_or_default();
                        }
                    }
                    changed = true;
                }
                _ => {}
            }
        }
        #[cfg(feature = "semantic-search")]
        while let Ok(event) = self.semantic_rx.try_recv() {
            let current = current_semantic_search_key(&self.st.config);
            if self.semantic_request.as_ref() == Some(&event.key) {
                self.semantic_running = false;
                if current.as_ref() == Some(&event.key) {
                    match event.result {
                        Ok(hits) => {
                            self.status =
                                format!("Semantic search found {} cited matches.", hits.len());
                            self.semantic_hits = Some(hits);
                        }
                        Err(error) => {
                            self.status = format!("Semantic search: {error}.");
                            self.semantic_error = Some(error);
                        }
                    }
                    changed = true;
                }
            }
        }
        #[cfg(feature = "nl-find")]
        {
            changed |= self.poll_nl();
        }
        #[cfg(feature = "semantic-search")]
        while let Ok(event) = self.related_rx.try_recv() {
            if self.related_request.as_ref() == Some(&event.key) {
                self.related_running = false;
                let current = self.current_result().and_then(|selected| {
                    self.preview.as_ref().filter(|preview| {
                        preview.path == selected.full_name && preview.text.is_some()
                    })
                });
                let current_key = current.and_then(|preview| {
                    related_search_key(&self.st.config, preview.path.clone(), preview.generation)
                });
                if current_key.as_ref() == Some(&event.key) {
                    match event.result {
                        Ok(hits) => {
                            self.status =
                                format!("Found {} possible related documents.", hits.len());
                            self.related_hits = Some((event.key, hits));
                        }
                        Err(error) => {
                            self.status = format!("Related-document search unavailable: {error}");
                            self.related_error = Some(error);
                        }
                    }
                    changed = true;
                }
            }
        }
        #[cfg(feature = "semantic-search")]
        while let Ok(event) = self.diff_rx.try_recv() {
            if event.generation == self.diff_generation
                && self.diff_request.as_ref() == Some(&event.key)
            {
                self.diff_running = false;
                let current = self.preview.as_ref().is_some_and(|preview| {
                    preview.path == event.key.related.source_path
                        && preview.generation == event.key.related.preview_generation
                }) && self.current_related_hits().is_some_and(|hits| {
                    hits.iter().any(|hit| hit.path == event.key.candidate_path)
                });
                if current {
                    match event.result {
                        Ok(result) => {
                            self.status = format!(
                                "Exact extracted-line comparison: {} added, {} removed, {} unchanged.",
                                result.added, result.removed, result.unchanged
                            );
                            self.diff_result = Some((event.key, result));
                        }
                        Err(error) if error != "Comparison cancelled." => {
                            self.status = format!("Exact comparison unavailable: {error}");
                            self.diff_error = Some(error);
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        while let Ok(event) = self.rx.try_recv() {
            changed = true;
            let finished = matches!(event, AppEvent::SearchFinished(_));
            let index_done = matches!(event, AppEvent::IndexBuildFinished(_));
            let scanned = matches!(event, AppEvent::ExtensionsScanned(_));
            let effects = search::apply_event(&mut self.st, &mut self.notes, event);
            self.take_note();
            self.execute(effects);
            if finished {
                if self.st.run.status_text.starts_with("Error")
                    || self.st.run.status_text.starts_with("Cancelled")
                {
                    self.completed_search_query = None;
                }
                // Offer an index update after the run, never during it (the terminal asks with a dialog).
                if let Some(s) = self
                    .st
                    .index_run
                    .stale
                    .take()
                    .filter(|_| !self.st.index_run.is_building)
                {
                    self.stale = Some(s);
                }
                if !self.st.run.results.is_empty() {
                    self.step = "Results";
                }
                let n = self.st.run.results.len();
                if self.st.run.status_text.starts_with("Error")
                    || self.st.run.status_text.starts_with("Cancelled")
                {
                    self.status = self.st.run.status_text.clone();
                } else {
                    self.status = format!(
                        "{} file{} with matches in {}.",
                        n,
                        if n == 1 { "" } else { "s" },
                        crate::format::format_elapsed(self.st.run.elapsed_secs().unwrap_or(0.0))
                    );
                }
            }
            if index_done {
                self.index_started = None;
            }
            if scanned {
                self.edit = None;
                if let Some(e) = &self.st.extension_picker.scan_error {
                    self.status = format!("Cannot read the file types: {e}");
                } else {
                    self.status =
                        "Tick the file types to search; none ticked searches every supported type."
                            .into();
                }
            }
        }
        changed |= self.refresh_preview();
        while let Ok(event) = self.preview_rx.try_recv() {
            match event {
                PreviewEvent::Progress(generation, status)
                    if self
                        .preview
                        .as_ref()
                        .is_some_and(|p| p.generation == generation) =>
                {
                    if let Some(p) = &mut self.preview {
                        p.status = status;
                        changed = true;
                    }
                }
                PreviewEvent::Done(generation, result)
                    if self
                        .preview
                        .as_ref()
                        .is_some_and(|p| p.generation == generation) =>
                {
                    let ready = result.and_then(|lines| {
                        self.current_result()
                            .map(|r| view::full_text(r, &lines, self.line_explainer().as_ref()))
                            .ok_or_else(|| "Result changed while loading.".to_string())
                    });
                    #[cfg(feature = "neural-search")]
                    let mut entity_text = None;
                    if let Some(p) = &mut self.preview {
                        match ready {
                            Ok(text) => {
                                #[cfg(feature = "neural-search")]
                                {
                                    entity_text = Some(text.content.to_string());
                                }
                                p.text = Some(text);
                                p.status.clear();
                            }
                            Err(error) => {
                                p.status = format!("Cannot display the full document: {error}")
                            }
                        }
                    }
                    #[cfg(feature = "neural-search")]
                    if let Some(text) = entity_text {
                        self.start_entity_analysis(generation, text);
                    }
                    changed = true;
                }
                _ => {}
            }
        }
        #[cfg(feature = "neural-search")]
        while let Ok(event) = self.entity_rx.try_recv() {
            if let Some(preview) = self.preview.as_mut().filter(|preview| {
                preview.generation == event.generation && preview.path == event.path
            }) {
                preview.entities = Some(event.result);
                changed = true;
            }
        }
        changed
    }
    fn prompting(&self) -> bool {
        false
    }
    fn solving(&self) -> bool {
        self.st.run.is_running || self.st.index_run.is_building
    }
    fn solving_for(&self) -> Option<std::time::Duration> {
        if self.st.run.is_running {
            self.st.run.started.map(|s| s.elapsed())
        } else if self.st.index_run.is_building {
            self.index_started.map(|s| s.elapsed())
        } else {
            None
        }
    }
    fn status(&self) -> String {
        self.status.clone()
    }
    fn set_status(&mut self, text: String) {
        self.status = text;
    }
    fn title(&self) -> String {
        let folder = self.st.config.search_path.trim();
        if folder.is_empty() {
            "Search".into()
        } else {
            format!("Search - {}", file_name(folder))
        }
    }
    fn dirty(&self) -> bool {
        false
    }
    fn geometry_view(&self) -> bool {
        false
    }
    fn tree(&self) -> Vec<TreeNode> {
        view::tree(self)
    }
    fn properties(&self) -> Vec<DockItem> {
        crate::gpu_viewer::editor::authoring::flatten(self.inspector())
    }
    fn inspector(&self) -> Vec<Card> {
        view::inspector(self)
    }
    fn tree_menu(&self, node: u32) -> Vec<Item> {
        view::tree_menu(self, node)
    }
    fn focus_step(&self) -> Option<&'static str> {
        Some(self.step)
    }
    fn ribbon(&self) -> Vec<RibbonTab> {
        view::ribbon(self)
    }
    fn step_state(&self, tab: &str) -> Option<StepState> {
        match tab {
            "Search" => Some(
                if self.st.run.started.is_some() && !self.st.run.is_running {
                    StepState::Done
                } else {
                    StepState::Todo
                },
            ),
            "Results" => Some(if self.st.run.results.is_empty() {
                StepState::Todo
            } else {
                StepState::Done
            }),
            _ => None,
        }
    }
    fn menus(&self) -> Vec<Menu> {
        view::menus(self)
    }
    fn context_menu(&self) -> Vec<Item> {
        view::context_menu(self)
    }
    fn value_card(&self) -> Option<ValueCard> {
        view::value_card(self)
    }
    fn card_field(&mut self, _index: usize, text: String) {
        if let Some(e) = &mut self.edit {
            e.text = text;
        }
    }
    fn document(&self) -> Option<Document> {
        Some(view::document(self))
    }
    fn has_document(&self) -> bool {
        true
    }
    fn document_field(&mut self, id: u8, text: String) {
        match id {
            INPUT_FOLDER => self.st.config.search_path = text,
            INPUT_FIND => self.st.config.filters_text = text,
            INPUT_FILTER => self.filter = text,
            INPUT_EXCLUDE => self.st.config.exclude_filters_text = text,
            INPUT_OUTPUT => self.st.config.output_folder = text,
            #[cfg(feature = "nl-find")]
            INPUT_NL => {
                self.nl.sentence = text;
                self.nl.reset();
            }
            _ => {}
        }
    }
}
