//! Fast re-search indexing (native-search/Tantivy) for the Search Files
//! toolbox: the build/rebuild task, its live status model, and the
//! diagnostics that make a Windows-only failure debuggable from
//! `toolbench-debug.log` (see `crate::debug_log`).
//!
//! The build is deliberately defensive about the ways it can fail
//! *silently*: a panic inside the spawned task is converted into an
//! `IndexBuildFinished(Err)` event (otherwise the UI spins forever), the
//! finished index is reopened and queried before success is reported (a
//! build that "reached 100%" but cannot be reopened is a failure), and
//! deleting a stale index retries on Windows' transient sharing violations.
//!
//! Ratatui-free by design (see `model.rs`'s own module doc for the same
//! rule); the render layer lives in `index_view.rs`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use native_search::error::{NsError, NsResult, NsStatus};
use search_core::embed_policy::EmbedPolicy;
use search_core::models::SearchSettings;
use search_core::native_index::{
    self, build_or_update_corpus_index_lazy_send, ensure_index_directory_exists,
    open_or_create_with_rebuild, redact_paths, CorpusIndexOutcome, CorpusIndexProgress, IndexStage,
};

use crate::app::AppEvent;
use crate::debug_log::{self, log};

/// Where the fast re-search index lives - mirrors
/// `app-egui/src/persistence.rs::IndexLocation` exactly (same two
/// variants, same default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IndexLocation {
    #[default]
    SearchFolder,
    OutputFolder,
    /// The app's private data folder: extracted text and vectors stay out of folders that are synced, backed up or committed.
    AppData,
}

impl IndexLocation {
    pub fn cycle(self) -> Self {
        match self {
            IndexLocation::SearchFolder => IndexLocation::OutputFolder,
            IndexLocation::OutputFolder => IndexLocation::AppData,
            IndexLocation::AppData => IndexLocation::SearchFolder,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            IndexLocation::SearchFolder => "Search folder",
            IndexLocation::OutputFolder => "Output folder",
            IndexLocation::AppData => "App data (private)",
        }
    }
}

/// The user-facing "index for fast re-search" settings - a small struct
/// embedded as `SearchToolConfig::index`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IndexSettings {
    pub enabled: bool,
    pub location: IndexLocation,
    #[serde(default)]
    pub semantic: bool,
    /// Keep the index current by watching the folder (opt-in; user-mode, nothing installed or changed outside the app).
    #[serde(default)]
    pub watch: bool,
    /// Embed the semantic index in the background, a few files at a time, instead of in one long build (opt-in; the
    /// exact index is complete after the first pass). Only meaningful with `semantic`.
    #[serde(default)]
    pub lazy_embed: bool,
}

/// Files embedded per pass when lazy embedding is on. A pass also walks the folder once, so it must be large next to
/// that walk, yet short enough that the Search page is blocked only briefly between passes.
pub const LAZY_EMBED_PASS_FILES: usize = 200;

impl IndexSettings {
    /// Embedding budget of one build: lazy only when both the semantic index and lazy embedding are on.
    pub fn embed_policy(&self) -> EmbedPolicy {
        EmbedPolicy { limit: (self.semantic && self.lazy_embed).then_some(LAZY_EMBED_PASS_FILES), ..Default::default() }
    }
}

/// Live index-build status the render layer reads. Deliberately separate
/// from `SearchRunState` - a plain search run and an index build are
/// different, independently-triggerable operations.
#[derive(Debug, Clone, Default)]
pub struct IndexRunState {
    pub is_building: bool,
    pub status_text: String,
    pub last_error: Option<String>,
    /// How the most recent search used (or did not use) the index.
    pub last_narrow: Option<String>,
    /// Documents in the index at the end of the last successful build.
    pub docs: Option<u64>,
    /// 0..=100 while building; drives the Run view's progress gauge.
    pub percent: f64,
    /// File being processed right now (full path; shown in the in-flight box).
    pub current_file: String,
    /// Set by a search that found the index out of date: (new/changed files,
    /// removed files). Consumed when that search finishes to prompt an update.
    pub stale: Option<(usize, usize)>,
    /// Files that failed in the most recent finished build (shown in the
    /// post-search index health line).
    pub last_build_failed: Option<i32>,
    /// Lazy embedding: (files embedded so far, files still waiting). `None` when the last build was not lazy.
    pub embedding: Option<(i32, i32)>,
    pub(super) indexing_started: Option<Instant>,
}

impl IndexRunState {
    pub fn begin(&mut self) {
        *self = IndexRunState {
            is_building: true,
            status_text: "Starting…".to_string(),
            docs: self.docs,
            last_narrow: self.last_narrow.take(),
            stale: None,
            last_build_failed: self.last_build_failed,
            embedding: self.embedding,
            ..Default::default()
        };
    }

    /// Folds one progress report into a one-line status: stage, counts,
    /// throughput and ETA while indexing, and the current file's name.
    pub fn apply_progress(&mut self, p: &CorpusIndexProgress) {
        self.is_building = true;
        if !p.current_file.is_empty() {
            self.current_file = p.current_file.clone();
        }
        let ratio = |done: i32, total: i32| {
            (f64::from(done.max(0)) / f64::from(total.max(1)) * 100.0).clamp(0.0, 100.0)
        };
        self.status_text = match p.stage {
            IndexStage::Enumerating => {
                self.percent = 0.0;
                format!("{}…", p.stage.label())
            }
            IndexStage::Checking => {
                self.percent = ratio(p.files_processed, p.total_files);
                format!(
                    "{} {}/{}",
                    p.stage.label(),
                    p.files_processed,
                    p.total_files
                )
            }
            IndexStage::Committing => format!("{} ({} indexed)…", p.stage.label(), p.indexed_count),
            IndexStage::Indexing => {
                let started = *self.indexing_started.get_or_insert_with(Instant::now);
                let secs = started.elapsed().as_secs_f64();
                let done = p.files_processed.max(0) as f64;
                let total = p.total_files.max(1) as f64;
                self.percent = ratio(p.files_processed, p.total_files);
                let mut text = format!(
                    "Indexing {}/{} ({:.0}%)",
                    p.files_processed,
                    p.total_files,
                    self.percent.floor()
                );
                if secs >= 2.0 && done >= 1.0 {
                    let rate = done / secs;
                    let eta = ((total - done) / rate).max(0.0);
                    text.push_str(&format!(" · {rate:.1} files/s · ETA {}", format_eta(eta)));
                }
                if p.failed_count > 0 {
                    text.push_str(&format!(" · {} failed", p.failed_count));
                }
                text
            }
        };
    }

    pub fn apply_outcome(&mut self, outcome: &CorpusIndexOutcome) {
        self.is_building = false;
        self.last_error = None;
        self.percent = 100.0;
        self.current_file.clear();
        self.docs = Some(outcome.index_docs);
        self.last_build_failed = Some(outcome.failed_count);
        let secs = outcome.elapsed_ms as f64 / 1000.0;
        let lead = if outcome.cancelled {
            "Index build cancelled"
        } else {
            "Index ready"
        };
        self.status_text = format!(
            "{lead}: {} docs · {} new/updated · {} unchanged · {} failed · {secs:.1}s",
            outcome.index_docs, outcome.indexed_count, outcome.skipped_count, outcome.failed_count
        );
        if let Some((done, pending)) = self.embedding_after(outcome) {
            self.embedding = Some((done, pending));
            if pending > 0 {
                self.status_text.push_str(&format!(" · {done} of {} embedded", done + pending));
            }
        } else {
            self.embedding = None;
        }
        if outcome.no_text_count > 0 {
            self.status_text
                .push_str(&format!(" · {} without text", outcome.no_text_count));
        }
        if outcome.enumeration_errors > 0 {
            self.status_text.push_str(&format!(
                " · {} folder(s) unreadable",
                outcome.enumeration_errors
            ));
        }
    }

    /// Embedding progress after `outcome`: counts accumulate over the chained passes of one lazy run and restart
    /// when the previous run had nothing pending. `None` when this build was not lazy (nothing pending and nothing
    /// embedded in a run that did not continue a lazy one).
    fn embedding_after(&self, outcome: &CorpusIndexOutcome) -> Option<(i32, i32)> {
        let continuing = self.embedding.filter(|&(_, pending)| pending > 0).map_or(0, |(done, _)| done);
        let done = continuing + outcome.semantic_embedded_count;
        (outcome.semantic_pending_count > 0 || continuing > 0).then_some((done, outcome.semantic_pending_count))
    }

    /// Whether another lazy pass should follow `outcome`: files are pending, the run was not cancelled, and this pass
    /// embedded something (otherwise every remaining file fails and chaining would loop forever).
    pub fn wants_next_embed_pass(outcome: &CorpusIndexOutcome) -> bool {
        outcome.semantic_pending_count > 0 && outcome.semantic_embedded_count > 0 && !outcome.cancelled
    }

    pub fn apply_error(&mut self, error: String) {
        self.is_building = false;
        self.current_file.clear();
        self.last_error = Some(match debug_log::path() {
            Some(p) => format!("{error} (details: {})", p.display()),
            None => error,
        });
    }
}

pub(crate) fn format_eta(secs: f64) -> String {
    let s = secs.round() as u64;
    if s >= 3600 {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// The machine-level catalog of indexed folders (`search_core::catalog`), in the app data folder.
pub fn catalog_path() -> Option<PathBuf> {
    crate::paths::app_data_dir().map(|d| d.join("catalog.sqlite"))
}

/// Registers a freshly built index in the catalog and refreshes its file list. Best effort: the catalog is
/// derived data, so a failure is logged and never fails the build. Tests never touch the user's real catalog.
fn record_in_catalog(root: &str, index_dir: &Path) {
    let Some(path) = catalog_path().filter(|_| !cfg!(test)) else { return };
    match record_in(&path, root, index_dir) {
        Ok(n) => log("INDEX", format!("catalog: {n} file(s) recorded")),
        Err(e) => log("WARN", format!("catalog update failed: {}", redact_paths(&e))),
    }
}

fn record_in(catalog: &Path, root: &str, index_dir: &Path) -> Result<usize, String> {
    let mut c = search_core::catalog::Catalog::open(catalog).map_err(|e| e.to_string())?;
    c.register_root(root, index_dir).map_err(|e| e.to_string())?;
    let files = c.sync_root(root)?;
    // Identifying strings (part numbers, standards) for "files sharing X"; only new or changed files are read.
    let extracted = c.index_entities(root, None)?;
    log("INDEX", format!("catalog: entities extracted for {extracted} file(s)"));
    // Relationships (folder, duplicate, version, shared-entity edges) are derived from the rows just written; about
    // 5 s for 100,000 files, on top of the entity pass (docs/knowledge-engine-audit.md, section 8).
    let edges = c.rebuild_edges()?;
    log("INDEX", format!("catalog: {} relationship edge(s), {} added, {} updated, {} removed", edges.total, edges.added, edges.updated, edges.removed));
    Ok(files)
}

/// Folders with a recorded fast index that still exist, for "Also search all indexed folders".
pub fn catalogued_folders() -> Vec<String> {
    let Some(path) = catalog_path().filter(|p| p.exists()) else { return Vec::new() };
    search_core::catalog::Catalog::open(&path)
        .and_then(|c| c.roots())
        .map(|roots| roots.into_iter().map(|r| r.path).filter(|p| Path::new(p).is_dir()).collect())
        .unwrap_or_default()
}

/// Why `path` relates to other indexed files (identical copies, versions, shared entities), from the catalog's
/// stored edges. Empty when there is no catalog or no edge. Tests never read the user's real catalog.
pub fn related_reasons(path: &str) -> Vec<search_core::catalog_edges::Reason> {
    let Some(catalog) = catalog_path().filter(|p| !cfg!(test) && p.exists()) else { return Vec::new() };
    search_core::catalog::Catalog::open(&catalog).and_then(|c| c.reasons_for(path)).unwrap_or_default()
}

/// Resolves the on-disk index directory for one root, mirroring
/// `app-egui/src/search.rs::resolve_index_dir` exactly: `SearchFolder`
/// delegates to `search_core::native_index::index_directory` (the index
/// lives inside the folder it indexes, ADR-011); `OutputFolder` keys each
/// root to its own subdirectory under the output folder via a
/// hashed+readable key, so multiple roots never collide or share one index.
pub fn index_directory(location: IndexLocation, search_path: &str, output_folder: &str) -> PathBuf {
    match location {
        IndexLocation::SearchFolder => native_index::index_directory(search_path),
        IndexLocation::OutputFolder => {
            let name = sanitized_root_key(search_path);
            Path::new(output_folder.trim())
                .join(".native-search-index")
                .join(name)
        }
        // No app data folder (no HOME / APPDATA): fall back to the folder itself rather than fail.
        IndexLocation::AppData => match crate::paths::app_data_dir() {
            Some(base) => base.join("indexes").join(sanitized_root_key(search_path)),
            None => native_index::index_directory(search_path),
        },
    }
}

/// Ported verbatim from `app-egui/src/search.rs::sanitized_root_key` - a
/// readable folder-name prefix plus a hash of the full normalized path, so
/// two different roots that happen to share a leaf folder name (e.g.
/// `/a/project` and `/b/project`) never collide.
fn sanitized_root_key(root: &str) -> String {
    use std::hash::{Hash, Hasher};
    let normalized = root.trim().trim_end_matches(['/', '\\']).to_lowercase();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    let hash = hasher.finish();

    let readable = Path::new(root.trim())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".to_string());
    let readable: String = readable
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{readable}-{hash:x}")
}

/// Logs everything about the environment that could make an index build
/// fail on one machine and not another: path shape/length, sync-folder and
/// UNC hints, whether the index folder is actually writable (a real write
/// + read-back probe, with the raw OS error code), what is already in the
/// index folder (including a stale Tantivy writer lock) and the effective
/// settings. Pure diagnostics - never changes behavior.
fn log_preflight(settings: &SearchSettings, index_dir: &Path, wipe_first: bool) {
    let root = Path::new(&settings.search_path);
    // Privacy: shapes and booleans only - never the path itself.
    let canonical = std::fs::canonicalize(root);
    log(
        "INDEX",
        format!(
            "preflight: search_path exists={} is_dir={} chars={} canonicalize={} verbatim_prefix={}",
            root.exists(),
            root.is_dir(),
            settings.search_path.chars().count(),
            match &canonical {
                Ok(_) => "ok".to_string(),
                Err(e) => format!("failed (os error {:?})", e.raw_os_error()),
            },
            canonical.as_ref().map(|c| c.to_string_lossy().starts_with("\\\\?\\")).unwrap_or(false)
        ),
    );
    log(
        "INDEX",
        format!(
            "preflight: index_dir chars={} wipe_first={wipe_first}",
            index_dir.as_os_str().len()
        ),
    );
    let lower = index_dir.to_string_lossy().to_lowercase();
    if index_dir.as_os_str().len() > 200 {
        log("WARN", "index path is longer than 200 chars - Windows MAX_PATH (260) can break Tantivy segment file names");
    }
    if lower.starts_with("\\\\") {
        log("WARN", "index path is a UNC/network path - memory-mapped index files are unreliable on network shares");
    }
    for hint in [
        "onedrive",
        "dropbox",
        "google drive",
        "icloud",
        "sharepoint",
    ] {
        if lower.contains(hint) {
            log("WARN", format!("index path is inside a cloud-synced folder ({hint}) - sync clients can lock/placeholder index files"));
        }
    }

    match std::fs::create_dir_all(index_dir) {
        Ok(()) => {
            let probe = index_dir.join(".write-probe");
            let result = std::fs::write(&probe, b"probe")
                .and_then(|_| std::fs::read(&probe))
                .map(|b| b.len());
            let _ = std::fs::remove_file(&probe);
            match result {
                Ok(n) => log(
                    "INDEX",
                    format!("preflight: index dir writable (probe round-trip {n} bytes)"),
                ),
                Err(e) => log(
                    "ERROR",
                    format!(
                        "preflight: index dir NOT writable: {e} (os error {:?})",
                        e.raw_os_error()
                    ),
                ),
            }
        }
        Err(e) => log(
            "ERROR",
            format!(
                "preflight: cannot create index dir: {e} (os error {:?})",
                e.raw_os_error()
            ),
        ),
    }

    match std::fs::read_dir(index_dir) {
        Ok(entries) => {
            let mut total = 0u64;
            let mut count = 0usize;
            for entry in entries.flatten() {
                let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                total += len;
                count += 1;
            }
            log("INDEX", format!("preflight: index dir holds {count} entries, {total} bytes (lock file present: {})", index_dir.join(".tantivy-writer.lock").exists()));
        }
        Err(e) => log("INDEX", format!("preflight: index dir not listable: {e}")),
    }

    log(
        "INDEX",
        format!(
            "preflight: settings extensions={} exclude_folders={} include_hidden={} max_file_size_mb={} parallel={} throttle={} ocr={} timeout_s={} retries={}",
            settings.extensions.as_ref().map(|e| e.len().to_string()).unwrap_or_else(|| "default-catalog".to_string()),
            settings.exclude_folders.len(),
            settings.include_hidden,
            settings.max_file_size_mb,
            settings.parallel,
            settings.throttle_limit,
            settings.ocr_scanned_pdfs,
            settings.file_timeout_seconds,
            settings.max_retries
        ),
    );
}

/// Deletes a stale index folder. On Windows a freshly closed memory map or an
/// antivirus scan can hold a segment file for a moment, failing the delete
/// with a sharing violation - retry briefly before giving up.
fn remove_index_dir_with_retry(index_dir: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 1..=6u32 {
        match std::fs::remove_dir_all(index_dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                log(
                    "INDEX",
                    format!(
                        "remove_dir_all attempt {attempt}/6 failed: {e} (os error {:?})",
                        e.raw_os_error()
                    ),
                );
                last = Some(e);
                std::thread::sleep(Duration::from_millis(250 * u64::from(attempt)));
            }
        }
    }
    Err(last.expect("loop ran at least once"))
}

/// True when `index_dir` holds a finished Tantivy index (its `meta.json`
/// exists). An empty or half-created folder is "not built".
pub fn index_is_built(index_dir: &Path) -> bool {
    index_dir.join("meta.json").is_file()
        && search_core::native_index::semantic_version_current(index_dir)
}

/// Builds or updates the fast re-search index for one root: an existing
/// index is updated incrementally (unchanged files are skipped); only an
/// index that cannot be opened or reopened (corrupt, wrong schema) is deleted
/// and rebuilt from scratch, once. Streams progress as
/// `AppEvent::IndexBuildProgress` and exactly one final
/// `AppEvent::IndexBuildFinished` - **always**, even if the build task
/// panics. Meant to be `tokio::spawn`'d by `main.rs`'s effect executor in
/// response to `Effect::BuildIndex`, or awaited by a search that needs the
/// index first (`runner::narrow_via_index`). Returns whether the index is
/// complete and usable (`Ok` and not cancelled).
pub async fn build_or_rebuild_index(
    tx: UnboundedSender<AppEvent>,
    settings: SearchSettings,
    index_dir: PathBuf,
    cancel: CancellationToken,
) -> bool {
    build_or_rebuild_index_with(tx, settings, index_dir, cancel, EmbedPolicy::default()).await
}

/// [`build_or_rebuild_index`] with an embedding budget (`IndexSettings::embed_policy`): the exact index is complete
/// afterwards, at most `embed.limit` files are embedded, and the outcome counts what is still pending.
pub async fn build_or_rebuild_index_with(
    tx: UnboundedSender<AppEvent>,
    settings: SearchSettings,
    index_dir: PathBuf,
    cancel: CancellationToken,
    embed: EmbedPolicy,
) -> bool {
    run_index_job(tx, settings, index_dir, cancel, None, embed).await
}

/// Brings the index up to date for just the paths a folder watcher reported (see
/// `search_core::native_index::update_corpus_index_paths_send`); same progress and finish events as a build.
pub async fn update_index_paths(
    tx: UnboundedSender<AppEvent>,
    settings: SearchSettings,
    index_dir: PathBuf,
    cancel: CancellationToken,
    paths: Vec<String>,
) -> bool {
    run_index_job(tx, settings, index_dir, cancel, Some(paths), EmbedPolicy::default()).await
}

async fn run_index_job(
    tx: UnboundedSender<AppEvent>,
    settings: SearchSettings,
    index_dir: PathBuf,
    cancel: CancellationToken,
    paths: Option<Vec<String>>,
    embed: EmbedPolicy,
) -> bool {
    log("INDEX", if paths.is_some() { "=== update from watcher" } else { "=== build start" });
    let _ = tx.send(AppEvent::IndexBuildProgress(CorpusIndexProgress {
        current_file: "Starting…".to_string(),
        ..Default::default()
    }));

    let started = Instant::now();
    let settings_root = settings.search_path.clone();
    let index_dir_for_catalog = index_dir.clone();
    let task_tx = tx.clone();
    // Run in its own task so a panic is a `JoinError` here rather than a
    // silently dead task that leaves the UI spinning forever.
    let joined = tokio::spawn(async move {
        match build_or_rebuild_index_inner(&task_tx, &settings, &index_dir, false, &cancel, paths.as_deref(), &embed).await {
            Err(e) if e.status == NsStatus::CorruptIndex => {
                log(
                    "WARN",
                    format!(
                        "index unusable ({}), deleting it and rebuilding from scratch",
                        redact_paths(&e.to_string())
                    ),
                );
                // A damaged index cannot take a path delta: rebuild it completely.
                build_or_rebuild_index_inner(&task_tx, &settings, &index_dir, true, &cancel, None, &embed).await
            }
            other => other,
        }
    })
    .await;
    let result = match joined {
        Ok(r) => r,
        Err(join_error) => Err(NsError::index_error(format!(
            "index build task crashed: {join_error}"
        ))),
    };

    match &result {
        Ok(o) => log(
            "INDEX",
            format!(
                "=== build finished in {:?}: docs={} indexed={} skipped={} failed={} too_large={} no_text={} enumerated={} candidates={} enum_errors={} cancelled={}",
                started.elapsed(),
                o.index_docs,
                o.indexed_count,
                o.skipped_count,
                o.failed_count,
                o.too_large_count,
                o.no_text_count,
                o.enumerated_count,
                o.candidate_count,
                o.enumeration_errors,
                o.cancelled
            ),
        ),
        Err(e) => log("ERROR", format!("=== build FAILED after {:?}: {e:?}", started.elapsed())),
    }
    let usable = matches!(&result, Ok(o) if !o.cancelled);
    let _ = tx.send(AppEvent::IndexBuildFinished(result));
    if usable {
        // After the finish event: reading every changed file again for entities must not delay "index ready".
        let (root, dir) = (settings_root, index_dir_for_catalog);
        let _ = tokio::task::spawn_blocking(move || record_in_catalog(&root, &dir)).await;
    }
    usable
}

async fn build_or_rebuild_index_inner(
    tx: &UnboundedSender<AppEvent>,
    settings: &SearchSettings,
    index_dir: &Path,
    wipe_first: bool,
    cancel: &CancellationToken,
    paths: Option<&[String]>,
    embed: &EmbedPolicy,
) -> NsResult<CorpusIndexOutcome> {
    {
        let (s, d) = (settings.clone(), index_dir.to_path_buf());
        let _ = tokio::task::spawn_blocking(move || log_preflight(&s, &d, wipe_first)).await;
    }

    if !Path::new(&settings.search_path).is_dir() {
        return Err(NsError::index_error(format!(
            "search path {:?} is not an existing folder",
            settings.search_path
        )));
    }

    if wipe_first && index_dir.exists() {
        let d = index_dir.to_path_buf();
        tokio::task::spawn_blocking(move || remove_index_dir_with_retry(&d))
            .await
            .map_err(|e| NsError::index_error(format!("delete task failed: {e}")))?
            .map_err(|e| {
                NsError::index_error(format!(
                    "could not remove existing index at {}: {e}",
                    index_dir.display()
                ))
            })?;
        log("INDEX", "unusable index removed for rebuild");
    }
    ensure_index_directory_exists(index_dir).map_err(|e| {
        NsError::index_error(format!(
            "cannot create index folder {}: {e}",
            index_dir.display()
        ))
    })?;

    let opened = Instant::now();
    // Any failure to open an existing index means it is unusable: the caller
    // then wipes it and rebuilds from scratch (one retry).
    let engine = open_or_create_with_rebuild(index_dir)
        .map_err(|e| NsError::new(NsStatus::CorruptIndex, format!("cannot open index: {e}")))?;
    log(
        "INDEX",
        format!(
            "engine opened in {:?}: docs_before={} segments={}",
            opened.elapsed(),
            engine.num_docs(),
            engine.segment_count()
        ),
    );

    let progress_tx = tx.clone();
    let mut last_sent: Option<Instant> = None;
    let mut last_stage: Option<IndexStage> = None;
    let mut last_logged_bucket = -1i64;
    let mut on_progress = move |p: CorpusIndexProgress| {
        let stage_changed = last_stage != Some(p.stage);
        if stage_changed {
            log(
                "INDEX",
                format!(
                    "stage -> {} (processed {}/{} indexed={} failed={})",
                    p.stage.label(),
                    p.files_processed,
                    p.total_files,
                    p.indexed_count,
                    p.failed_count
                ),
            );
            last_stage = Some(p.stage);
        }
        let bucket = i64::from(p.files_processed) / 500;
        if p.stage == IndexStage::Indexing && bucket != last_logged_bucket {
            last_logged_bucket = bucket;
            log(
                "INDEX",
                format!(
                    "progress {}/{} indexed={} failed={}",
                    p.files_processed, p.total_files, p.indexed_count, p.failed_count
                ),
            );
        }
        // The UI only redraws a few times a second; flooding the event
        // channel with one message per file just queues stale redraws.
        // A stage change is always sent: a slow commit would otherwise leave the
        // previous stage's text on screen for its whole duration.
        let due = stage_changed
            || last_sent
                .map(|t| t.elapsed() >= Duration::from_millis(100))
                .unwrap_or(true);
        if due {
            last_sent = Some(Instant::now());
            let _ = progress_tx.send(AppEvent::IndexBuildProgress(p));
        }
    };

    let mut outcome = match paths {
        // Watcher delta: only the reported paths, no walk and no freshness pass over every file.
        Some(changed) => {
            native_index::update_corpus_index_paths_send(settings, &engine, cancel, changed, Some(&mut on_progress))
                .await?
        }
        None => build_or_update_corpus_index_lazy_send(settings, &engine, cancel, Some(&mut on_progress), embed).await?,
    };

    // Aggregated by extension + error text; never per-file (no file names in the log).
    for (kind, count) in &outcome.failure_summary {
        log("INDEX-FAIL", format!("{count} x {kind}"));
    }

    if !outcome.cancelled && paths.is_none() {
        // (A watcher delta already removed what vanished; this is a stat of every document.)
        // Documents whose files were deleted since the last build would
        // otherwise linger and keep being offered as search candidates.
        match native_index::remove_orphaned_documents(&engine) {
            Ok(n) => log("INDEX", format!("orphan cleanup removed {n} document(s)")),
            Err(e) => log("WARN", format!("orphan cleanup failed: {e:?}")),
        }
    }
    outcome.index_docs = engine.num_docs();
    drop(engine); // release the writer lock/mmaps before verifying through a fresh open

    verify_built_index(index_dir, &outcome)
        .map_err(|e| NsError::new(NsStatus::CorruptIndex, e.to_string()))?;
    Ok(outcome)
}

/// Success is not "the progress bar hit 100%": reopen the finished index the
/// way a search will, check the document count, and look one document up.
fn verify_built_index(index_dir: &Path, outcome: &CorpusIndexOutcome) -> NsResult<()> {
    let engine =
        native_search::engine::NativeSearchEngine::open_or_create(index_dir).map_err(|e| {
            NsError::index_error(format!("index was built but cannot be reopened: {e}"))
        })?;
    let docs = engine.num_docs();
    log(
        "INDEX",
        format!(
            "verify: reopened ok, docs={docs} segments={} (build reported {})",
            engine.segment_count(),
            outcome.index_docs
        ),
    );
    let expected_min = i64::from(outcome.indexed_count) + i64::from(outcome.skipped_count);
    if expected_min > 0 && docs == 0 {
        return Err(NsError::index_error(format!(
            "index verification failed: build processed {expected_min} file(s) but the reopened index holds 0 documents"
        )));
    }
    if (docs as i64) < expected_min {
        log("WARN", format!("verify: reopened index has {docs} docs, fewer than the {expected_min} indexed/unchanged files"));
    }
    if let Some(id) = engine.all_document_ids()?.into_iter().next() {
        let found = engine.get_document_metadata(&id)?.is_some();
        log(
            "INDEX",
            format!("verify: sample document lookup -> {found}"),
        );
        if !found {
            return Err(NsError::index_error(
                "index verification failed: a stored document cannot be looked up after reopen",
            ));
        }
    }
    Ok(())
}

/// Documents present in the index whose source file no longer exists -
/// thin wrapper over `search_core::native_index::remove_orphaned_documents`.
pub fn remove_orphaned_documents(
    engine: &native_search::engine::NativeSearchEngine,
) -> NsResult<usize> {
    native_index::remove_orphaned_documents(engine)
}

/// Opens the index read-only and returns its document count - thin wrapper
/// over `search_core::native_index::verify_index`.
pub fn verify_index(index_directory: &Path) -> NsResult<u64> {
    native_index::verify_index(index_directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use search_core::native_index::build_or_update_corpus_index_send;

    #[test]
    fn search_folder_location_delegates_to_native_index_directory() {
        let dir = index_directory(IndexLocation::SearchFolder, "/tmp/project", "/tmp/output");
        assert_eq!(dir, native_index::index_directory("/tmp/project"));
    }

    #[test]
    fn app_data_location_is_private_per_root_and_cycles_back() {
        let a = index_directory(IndexLocation::AppData, "/tmp/a/project", "/tmp/output");
        let b = index_directory(IndexLocation::AppData, "/tmp/b/project", "/tmp/output");
        assert_ne!(a, b, "two roots sharing a leaf name never share an index");
        for dir in [&a, &b] {
            assert!(!dir.starts_with("/tmp/a") && !dir.starts_with("/tmp/b") && !dir.starts_with("/tmp/output"));
        }
        if let Some(base) = crate::paths::app_data_dir() {
            assert!(a.starts_with(base.join("indexes")));
        }
        let mut seen = vec![IndexLocation::SearchFolder];
        let mut at = IndexLocation::SearchFolder.cycle();
        while at != IndexLocation::SearchFolder {
            seen.push(at);
            at = at.cycle();
        }
        assert_eq!(seen.len(), 3, "cycle visits every location once");
        // Stored settings keep working: old values parse, the new one round-trips.
        let old: IndexLocation = serde_json::from_str("\"OutputFolder\"").unwrap();
        assert_eq!(old, IndexLocation::OutputFolder);
        let json = serde_json::to_string(&IndexLocation::AppData).unwrap();
        assert_eq!(serde_json::from_str::<IndexLocation>(&json).unwrap(), IndexLocation::AppData);
    }

    #[test]
    fn output_folder_location_keys_by_sanitized_root() {
        let dir = index_directory(IndexLocation::OutputFolder, "/tmp/project", "/tmp/output");
        assert!(dir.starts_with(Path::new("/tmp/output/.native-search-index")));
        assert!(dir.to_string_lossy().contains("project-"));
    }

    #[test]
    fn output_folder_location_never_collides_across_different_roots_with_the_same_leaf_name() {
        let a = index_directory(IndexLocation::OutputFolder, "/a/project", "/out");
        let b = index_directory(IndexLocation::OutputFolder, "/b/project", "/out");
        assert_ne!(a, b);
    }

    #[test]
    fn location_cycles_through_all_variants_and_wraps() {
        assert_eq!(
            IndexLocation::SearchFolder.cycle(),
            IndexLocation::OutputFolder
        );
        assert_eq!(IndexLocation::OutputFolder.cycle(), IndexLocation::AppData);
        assert_eq!(IndexLocation::AppData.cycle(), IndexLocation::SearchFolder);
    }

    #[tokio::test]
    async fn building_an_index_over_a_real_tempdir_creates_a_queryable_index() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world\nneedle here\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "nothing interesting\n").unwrap();

        let settings = SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let index_dir = index_directory(IndexLocation::SearchFolder, &settings.search_path, "");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        build_or_rebuild_index(tx, settings, index_dir.clone(), CancellationToken::new()).await;

        let mut finished = None;
        while let Ok(event) = rx.try_recv() {
            if let AppEvent::IndexBuildFinished(result) = event {
                finished = Some(result);
            }
        }
        let outcome = finished
            .expect("expected an IndexBuildFinished event")
            .expect("index build should succeed");
        assert_eq!(outcome.indexed_count, 2);

        let doc_count = verify_index(&index_dir).unwrap();
        assert_eq!(doc_count, 2);
    }

    #[tokio::test]
    async fn trigram_candidates_narrow_to_the_matching_file_after_a_build() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world\nneedle here\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "nothing interesting\n").unwrap();

        let settings = SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let index_dir = index_directory(IndexLocation::SearchFolder, &settings.search_path, "");
        ensure_index_directory_exists(&index_dir).unwrap();
        let engine = open_or_create_with_rebuild(&index_dir).unwrap();
        let mut noop = |_p: CorpusIndexProgress| {};
        build_or_update_corpus_index_send(
            &settings,
            &engine,
            &CancellationToken::new(),
            Some(&mut noop),
        )
        .await
        .unwrap();

        let candidates = engine
            .trigram_candidate_paths(&["needle".to_string()])
            .unwrap();
        assert!(candidates.is_some());
        let candidates = candidates.unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].ends_with("a.txt"));
    }

    #[tokio::test]
    async fn a_built_index_is_recorded_in_the_catalog_with_its_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let settings = SearchSettings { search_path: root.clone(), ..Default::default() };
        let index_dir = index_directory(IndexLocation::SearchFolder, &root, "");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(build_or_rebuild_index(tx, settings, index_dir.clone(), CancellationToken::new()).await);

        let catalog = dir.path().join("catalog-test.sqlite");
        assert_eq!(record_in(&catalog, &root, &index_dir).unwrap(), 1);
        let c = search_core::catalog::Catalog::open(&catalog).unwrap();
        assert_eq!(c.roots().unwrap()[0].path, root);
        assert!(c.files().unwrap()[0].path.ends_with("a.txt"));
    }

    #[tokio::test]
    async fn recording_a_built_index_derives_the_folder_edges() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "same words\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "same words\n").unwrap();
        std::fs::write(dir.path().join("spec_v1.txt"), "first\n").unwrap();
        std::fs::write(dir.path().join("spec_v2.txt"), "second\n").unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let settings = SearchSettings { search_path: root.clone(), ..Default::default() };
        let index_dir = index_directory(IndexLocation::SearchFolder, &root, "");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(build_or_rebuild_index(tx, settings, index_dir.clone(), CancellationToken::new()).await);

        let catalog = dir.path().join("catalog-edges-test.sqlite");
        record_in(&catalog, &root, &index_dir).unwrap();
        let c = search_core::catalog::Catalog::open(&catalog).unwrap();
        let a = c.files().unwrap().into_iter().find(|f| f.path.ends_with("a.txt")).unwrap().path;
        let facts = c.facts_for(&a).unwrap();
        assert!(facts.iter().any(|e| e.kind == "FILE_IN_FOLDER"), "{facts:?}");
        // The reasons are keyed by the same path the file list (and so a result row) uses.
        let v1 = c.files().unwrap().into_iter().find(|f| f.path.ends_with("spec_v1.txt")).unwrap().path;
        let reasons = c.reasons_for(&v1).unwrap();
        assert_eq!(reasons.len(), 1, "{reasons:?}");
        assert!(reasons[0].text.contains("spec_v2.txt") && !reasons[0].inferred, "{reasons:?}");
    }

    #[tokio::test]
    async fn building_again_updates_the_existing_index_instead_of_recreating_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let settings = SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let index_dir = index_directory(IndexLocation::SearchFolder, &settings.search_path, "");

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(
            build_or_rebuild_index(
                tx.clone(),
                settings.clone(),
                index_dir.clone(),
                CancellationToken::new()
            )
            .await
        );
        // A marker file inside the index folder survives only if the folder is not wiped.
        std::fs::write(index_dir.join("marker.keep"), "x").unwrap();
        std::fs::write(dir.path().join("b.txt"), "second file\n").unwrap();

        let (tx2, mut rx2) = tokio::sync::mpsc::unbounded_channel();
        assert!(
            build_or_rebuild_index(tx2, settings, index_dir.clone(), CancellationToken::new())
                .await
        );
        assert!(
            index_dir.join("marker.keep").exists(),
            "an intact index must be updated in place, not deleted"
        );
        assert_eq!(verify_index(&index_dir).unwrap(), 2);
        let outcome = std::iter::from_fn(|| rx2.try_recv().ok()).find_map(|e| match e {
            AppEvent::IndexBuildFinished(Ok(o)) => Some(o),
            _ => None,
        });
        let o = outcome.expect("finished");
        assert_eq!(
            (o.indexed_count, o.skipped_count),
            (1, 1),
            "only the new file is indexed"
        );
    }

    #[tokio::test]
    async fn a_corrupt_index_is_wiped_and_rebuilt_from_scratch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let settings = SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let index_dir = index_directory(IndexLocation::SearchFolder, &settings.search_path, "");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(
            build_or_rebuild_index(
                tx.clone(),
                settings.clone(),
                index_dir.clone(),
                CancellationToken::new()
            )
            .await
        );

        std::fs::write(index_dir.join("meta.json"), b"{ this is not json").unwrap();
        std::fs::write(index_dir.join("marker.gone"), "x").unwrap();
        assert!(
            build_or_rebuild_index(tx, settings, index_dir.clone(), CancellationToken::new()).await,
            "a corrupt index must self-heal"
        );
        assert!(
            !index_dir.join("marker.gone").exists(),
            "the corrupt index folder was wiped"
        );
        assert_eq!(verify_index(&index_dir).unwrap(), 1);
    }

    #[test]
    fn progress_status_reports_stage_counts_percent_and_current_file() {
        let mut st = IndexRunState::default();
        st.begin();
        st.apply_progress(&CorpusIndexProgress {
            stage: IndexStage::Enumerating,
            ..Default::default()
        });
        assert!(st.status_text.starts_with("Scanning folder"));
        st.apply_progress(&CorpusIndexProgress {
            stage: IndexStage::Indexing,
            files_processed: 5,
            total_files: 20,
            current_file: "/x/y/report.pdf".to_string(),
            indexed_count: 5,
            failed_count: 2,
        });
        assert!(
            st.status_text.contains("Indexing 5/20 (25%)"),
            "{}",
            st.status_text
        );
        assert!(st.status_text.contains("2 failed"));
        // The file name lives in the in-flight box, not the (truncatable) status line.
        assert_eq!(st.current_file, "/x/y/report.pdf");
        assert_eq!(st.percent, 25.0);
        assert!(st.is_building);
        st.apply_progress(&CorpusIndexProgress {
            stage: IndexStage::Committing,
            files_processed: 5,
            total_files: 20,
            ..Default::default()
        });
        assert_eq!(
            st.current_file, "/x/y/report.pdf",
            "a report without a file must not blank the last one"
        );
        assert_eq!(st.percent, 25.0, "committing keeps the last percentage");
    }

    #[test]
    fn outcome_and_error_end_the_building_state() {
        let mut st = IndexRunState::default();
        st.begin();
        st.apply_outcome(&CorpusIndexOutcome {
            index_docs: 7,
            indexed_count: 3,
            skipped_count: 4,
            ..Default::default()
        });
        assert!(!st.is_building);
        assert_eq!(st.docs, Some(7));
        assert!(st.status_text.contains("7 docs"));
        st.begin();
        st.apply_error("boom".to_string());
        assert!(!st.is_building);
        assert!(st.last_error.as_deref().unwrap().starts_with("boom"));
    }

    fn lazy_outcome(embedded: i32, pending: i32) -> CorpusIndexOutcome {
        CorpusIndexOutcome { semantic_embedded_count: embedded, semantic_pending_count: pending, ..Default::default() }
    }

    #[test]
    fn lazy_embedding_is_opt_in_needs_the_semantic_index_and_reports_n_of_m_across_passes() {
        assert_eq!(IndexSettings::default().embed_policy().limit, None);
        assert_eq!(IndexSettings { lazy_embed: true, ..Default::default() }.embed_policy().limit, None, "no semantic index: nothing to embed lazily");
        assert_eq!(
            IndexSettings { semantic: true, lazy_embed: true, ..Default::default() }.embed_policy().limit,
            Some(LAZY_EMBED_PASS_FILES)
        );
        let mut st = IndexRunState::default();
        st.begin();
        st.apply_outcome(&lazy_outcome(200, 300));
        assert!(st.status_text.contains("200 of 500 embedded"), "{}", st.status_text);
        st.begin();
        st.apply_outcome(&lazy_outcome(200, 100));
        assert!(st.status_text.contains("400 of 500 embedded"), "{}", st.status_text);
        st.begin();
        st.apply_outcome(&lazy_outcome(100, 0));
        assert!(!st.status_text.contains("embedded"), "finished run shows no pending line: {}", st.status_text);
        st.begin();
        st.apply_outcome(&lazy_outcome(7, 0));
        assert_eq!(st.embedding, None, "a later non-lazy build starts from zero");
    }

    #[test]
    fn another_lazy_pass_follows_only_while_files_are_pending_and_progress_is_made() {
        assert!(IndexRunState::wants_next_embed_pass(&lazy_outcome(200, 5)));
        assert!(!IndexRunState::wants_next_embed_pass(&lazy_outcome(200, 0)));
        assert!(!IndexRunState::wants_next_embed_pass(&lazy_outcome(0, 5)), "every remaining file fails: stop");
        let mut cancelled = lazy_outcome(200, 5);
        cancelled.cancelled = true;
        assert!(!IndexRunState::wants_next_embed_pass(&cancelled));
    }

    #[test]
    fn eta_formats_seconds_minutes_hours() {
        assert_eq!(format_eta(9.0), "9s");
        assert_eq!(format_eta(95.0), "1m35s");
        assert_eq!(format_eta(7_500.0), "2h05m");
    }

    #[tokio::test]
    async fn building_over_a_missing_folder_reports_an_error_event_instead_of_hanging() {
        let settings = SearchSettings {
            search_path: "/definitely/not/a/real/folder".to_string(),
            ..Default::default()
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let idx = std::env::temp_dir().join("toolbench-missing-root-idx");
        build_or_rebuild_index(tx, settings, idx, CancellationToken::new()).await;
        let mut finished = None;
        while let Ok(e) = rx.try_recv() {
            if let AppEvent::IndexBuildFinished(r) = e {
                finished = Some(r);
            }
        }
        assert!(finished
            .expect("a Finished event must always be sent")
            .is_err());
    }

    #[tokio::test]
    async fn a_cancelled_build_still_finishes_with_a_cancelled_outcome() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let settings = SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let index_dir = index_directory(IndexLocation::SearchFolder, &settings.search_path, "");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        build_or_rebuild_index(tx, settings, index_dir, cancel).await;
        let mut finished = None;
        while let Ok(e) = rx.try_recv() {
            if let AppEvent::IndexBuildFinished(r) = e {
                finished = Some(r);
            }
        }
        assert!(finished.unwrap().unwrap().cancelled);
    }

    /// Privacy contract: whatever a build and a search do, the debug log must
    /// not contain file names, folder names or search terms.
    #[tokio::test]
    async fn the_debug_log_contains_no_file_names_folder_names_or_search_terms() {
        let logs = tempfile::tempdir().unwrap();
        crate::debug_log::init_in(logs.path());

        let root = tempfile::tempdir().unwrap();
        let secret_dir = root.path().join("ConfidentialClientFolderQ7");
        std::fs::create_dir_all(&secret_dir).unwrap();
        std::fs::write(
            secret_dir.join("SecretProjectNameXyz.txt"),
            "needle ZebraTermQ9 here\n",
        )
        .unwrap();
        // Corrupt office file: exercises the per-file failure path.
        std::fs::write(secret_dir.join("BrokenBudgetXyz.docx"), b"not a real docx").unwrap();

        let settings = SearchSettings {
            search_path: root.path().to_string_lossy().into_owned(),
            filters: vec!["ZebraTermQ9".to_string()],
            ..Default::default()
        };
        let index_dir = index_directory(IndexLocation::SearchFolder, &settings.search_path, "");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        build_or_rebuild_index(
            tx,
            settings.clone(),
            index_dir.clone(),
            CancellationToken::new(),
        )
        .await;
        // Also a failing build (missing folder) so the error path is covered.
        let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();
        let bad = SearchSettings {
            search_path: root
                .path()
                .join("MissingFolderZz")
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        };
        build_or_rebuild_index(tx2, bad, index_dir, CancellationToken::new()).await;

        let log =
            std::fs::read_to_string(logs.path().join(crate::debug_log::LOG_FILE_NAME)).unwrap();
        assert!(
            log.contains("build start") && log.contains("build finished"),
            "log must still be useful:\n{log}"
        );
        let root_name = root
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for forbidden in [
            "SecretProjectNameXyz",
            "BrokenBudgetXyz",
            "ConfidentialClientFolderQ7",
            "ZebraTermQ9",
            "MissingFolderZz",
            root_name.as_str(),
        ] {
            assert!(
                !log.contains(forbidden),
                "debug log leaked `{forbidden}`:\n{log}"
            );
        }
    }
}
