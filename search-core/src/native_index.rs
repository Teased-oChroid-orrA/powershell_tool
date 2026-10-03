//! Ports the *policy* half of `NativeSearchService.cs` +
//! `NativeSearchPaths.cs` + `MainViewModel.cs`'s `IndexHitsForFastSearch`/
//! `RunNativeSearchAsync`: index-per-searched-folder placement (ADR-011),
//! auto-exclusion of that folder from the normal search, and
//! skip-reindex-if-unchanged via `get_document_metadata`.
//!
//! The FFI-boundary plumbing the C# side needed (`NativeSearchInterop`,
//! `NativeSearchHandle`, `SafeHandle` marshaling, `DangerousAddRef` dances
//! to work around a `LibraryImport` marshaller gap) is DEAD CODE in this
//! port: `native-search`'s `engine.rs` is called directly, in-process, as a
//! normal Rust library dependency. No C ABI crossing, no marshaling
//! workarounds, no `SafeHandle` - the whole class of bug that plumbing
//! existed to guard against doesn't exist here.

use std::path::{Path, PathBuf};

use native_search::engine::{DocumentInput, NativeSearchEngine, SearchHit};
use native_search::error::{NsError, NsResult};
use tokio_util::sync::CancellationToken;

use crate::extraction;
use crate::file_reader;
use crate::models::{FileSearchResult, SearchSettings};
use crate::orchestrator::filter_by_extension;

/// Name of the index subfolder created at the root of whatever folder is
/// being searched. Dot-prefixed (matches the convention of tool-owned
/// folders like `.git`) so it reads as "not a document in this folder" at
/// a glance. This exact constant must also be what
/// [`ensure_index_folder_excluded`] adds to `SearchSettings.exclude_folders`
/// - both sides using the same constant (not a hand-typed copy) is what
/// keeps the exclusion and the actual folder name from silently drifting
/// apart.
pub const INDEX_FOLDER_NAME: &str = ".native-search-index";

/// `search_path`/[`INDEX_FOLDER_NAME`] - the index lives inside the folder
/// it indexes (ADR-011), not a global per-machine location, so a "Fast
/// re-search" only ever searches documents that came from indexing *this*
/// folder tree, and deleting the folder naturally takes its index with it.
pub fn index_directory(search_path: &str) -> PathBuf {
    Path::new(search_path).join(INDEX_FOLDER_NAME)
}

/// Opens (or creates) the index at `index_directory`, automatically
/// deleting and rebuilding it if it was built with an older schema
/// version (`NsStatus::CorruptIndex` - see `engine.rs::open_or_create`'s
/// own doc comment: schema changes, like Phase 1's new `trigram` field,
/// have no in-place migration path). Without this, every existing
/// `.native-search-index` folder on disk would start hard-erroring the
/// instant this schema change ships, with no recovery but a user manually
/// deleting the folder - auto-rebuilding is the honest fix, not a
/// workaround, since a from-scratch rebuild really is the only valid
/// recovery here and there's no reason to make the user do it by hand.
pub fn open_or_create_with_rebuild(index_directory: &Path) -> NsResult<NativeSearchEngine> {
    match NativeSearchEngine::open_or_create(index_directory) {
        Ok(engine) => Ok(engine),
        Err(e) if e.status == native_search::error::NsStatus::CorruptIndex => {
            std::fs::remove_dir_all(index_directory)
                .map_err(|io_err| NsError::index_error(format!("could not remove outdated index at {}: {io_err}", index_directory.display())))?;
            std::fs::create_dir_all(index_directory)
                .map_err(|io_err| NsError::index_error(format!("could not recreate index directory at {}: {io_err}", index_directory.display())))?;
            NativeSearchEngine::open_or_create(index_directory)
        }
        Err(e) => Err(e),
    }
}

/// Creates the index directory if it doesn't already exist -
/// `NativeSearchEngine::open_or_create` requires the directory to already
/// be present (native-search does no filesystem provisioning of its own).
pub fn ensure_index_directory_exists(index_directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(index_directory)
}

/// Adds [`INDEX_FOLDER_NAME`] to `exclude_folders` if not already present
/// (case-insensitively) - the index folder must never itself be walked and
/// indexed as if it were a document.
pub fn ensure_index_folder_excluded(exclude_folders: &mut Vec<String>) {
    if !exclude_folders.iter().any(|f| f.eq_ignore_ascii_case(INDEX_FOLDER_NAME)) {
        exclude_folders.push(INDEX_FOLDER_NAME.to_string());
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IndexOutcome {
    pub indexed_count: i32,
    pub skipped_count: i32,
}

impl IndexOutcome {
    pub fn status_message(&self) -> String {
        match (self.indexed_count, self.skipped_count) {
            (0, 0) => "No hits to index for fast re-search.".to_string(),
            (0, s) => format!("All {s} file(s) already up to date in the fast index. Search above."),
            (i, 0) => format!("Indexed {i} file(s) for fast re-search. Search above."),
            (i, s) => format!("Indexed {i} file(s), {s} already up to date. Search above."),
        }
    }
}

/// Indexes this run's hit files into native_search so a later "Fast
/// re-search" can search them without re-walking/re-extracting the folder
/// (issue #2). A file whose modified time and size match what's already
/// stored for it is skipped entirely - re-indexing an unchanged file is
/// wasted work, and `get_document_metadata` answers "did this change" from
/// the index itself, with no separate cache file needed.
pub fn index_hits_for_fast_search(engine: &NativeSearchEngine, hits: &[FileSearchResult]) -> NsResult<IndexOutcome> {
    let mut outcome = IndexOutcome::default();

    for r in hits {
        let modified_unix = r.modified.timestamp();
        if let Some((existing_modified, existing_size)) = engine.get_document_metadata(&r.full_name)? {
            if existing_modified == modified_unix && existing_size == r.file_length {
                outcome.skipped_count += 1;
                continue;
            }
        }

        let path = Path::new(&r.full_name);
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let extension = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let body = r.lines_cache.join("\n");

        engine.index_document(DocumentInput {
            id: &r.full_name,
            path: &r.full_name,
            filename: &file_name,
            extension: &extension,
            title: "",
            modified_unix,
            created_unix: r.created.timestamp(),
            size: r.file_length,
            body: &body,
        })?;
        outcome.indexed_count += 1;
    }

    if outcome.indexed_count > 0 {
        engine.commit()?;
    }

    tracing::info!(indexed = outcome.indexed_count, skipped = outcome.skipped_count, "fast-search index update complete");
    Ok(outcome)
}

/// Coarse phase of a corpus index build - lets a UI (and the debug log)
/// say *what* is slow instead of showing one opaque percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndexStage {
    #[default]
    Enumerating,
    /// Comparing each file's modified time/size against the index to find
    /// the ones that actually need (re)indexing.
    Checking,
    /// Reading + extracting + indexing the files that changed.
    Indexing,
    Committing,
}

impl IndexStage {
    pub fn label(self) -> &'static str {
        match self {
            IndexStage::Enumerating => "Scanning folder",
            IndexStage::Checking => "Checking for changes",
            IndexStage::Indexing => "Indexing",
            IndexStage::Committing => "Committing",
        }
    }
}

/// Live status while [`build_or_update_corpus_index`] is running - handed
/// to the caller's progress callback, same shape/spirit as
/// `orchestrator::SearchProgressReport` but scoped to what indexing
/// actually has to report (no match-mode/hit-count concepts here).
#[derive(Debug, Clone, Default)]
pub struct CorpusIndexProgress {
    pub stage: IndexStage,
    pub files_processed: i32,
    pub total_files: i32,
    pub current_file: String,
    pub indexed_count: i32,
    pub failed_count: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CorpusIndexOutcome {
    pub indexed_count: i32,
    pub skipped_count: i32,
    pub failed_count: i32,
    /// One entry per failure ("path: error message" for a document, or
    /// "commit at N files: error message" for a batch commit) - a real
    /// end-of-run summary instead of one opaque error covering the whole
    /// folder. Capped implicitly by `failed_count` staying small in
    /// practice (a genuinely broken environment fails every remaining
    /// file, at which point the caller should stop and investigate, not
    /// receive an unbounded list) - not truncated here, since a caller
    /// choosing to display only the first N is a presentation decision.
    pub failed_files: Vec<String>,
    /// Failure counts keyed by `ext=<extension>: <error with paths removed>` -
    /// contains no file or folder names, so it is safe to write to a
    /// diagnostic log (unlike `failed_files`, which names every file).
    pub failure_summary: std::collections::BTreeMap<String, u32>,
    /// Files found by the folder walk (before extension filtering).
    pub enumerated_count: i32,
    /// Files left after extension filtering - the set the index covers.
    pub candidate_count: i32,
    /// Files skipped because they exceed `max_file_size_mb`.
    pub too_large_count: i32,
    /// Files that are binary or yielded no extractable text (e.g. scanned
    /// PDFs). Stored as empty documents - not failures - so the index knows
    /// their modified time/size and never reports them as stale.
    pub no_text_count: i32,
    /// Directories the walk could not enter (permissions, broken links,
    /// path-length limits) - their contents are NOT in the index.
    pub enumeration_errors: i32,
    pub elapsed_ms: u64,
    /// Documents in the index after the build (all files, not just this run's).
    pub index_docs: u64,
    /// The build was cancelled part-way; everything indexed before the
    /// cancel was committed and is usable.
    pub cancelled: bool,
}

/// Replaces anything path-like in `text` with `<path>`: a whitespace/quote-
/// delimited token containing `/` or `\\`, or starting with a drive letter
/// (`C:`), and any quoted segment containing a separator. Backstop for text
/// that may embed a path (OS/library error strings) before it is logged.
pub fn redact_paths(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let mut quote: Option<char> = None;
    let mut quoted = String::new();
    let is_pathy = |t: &str| {
        t.contains('/') || t.contains('\\') || {
            let b = t.as_bytes();
            b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
        }
    };
    let flush = |token: &mut String, out: &mut String| {
        if !token.is_empty() {
            out.push_str(if is_pathy(token) { "<path>" } else { token });
            token.clear();
        }
    };
    for c in text.chars() {
        if let Some(q) = quote {
            if c == q {
                out.push(q);
                out.push_str(if is_pathy(&quoted) { "<path>" } else { &quoted });
                out.push(q);
                quote = None;
                quoted.clear();
            } else {
                quoted.push(c);
            }
        } else if c == '"' || c == '\'' {
            flush(&mut token, &mut out);
            quote = Some(c);
        } else if c.is_whitespace() || matches!(c, '(' | ')' | ',' | '[' | ']' | '{' | '}' | '=') {
            flush(&mut token, &mut out);
            out.push(c);
        } else {
            token.push(c);
        }
    }
    if let Some(q) = quote {
        // Unterminated quote: treat the remainder as one token.
        out.push(q);
        out.push_str(if is_pathy(&quoted) { "<path>" } else { &quoted });
    }
    flush(&mut token, &mut out);
    out
}

fn record_failure(outcome: &mut CorpusIndexOutcome, full_name: &str, ext: &str, error: &str) {
    outcome.failed_count += 1;
    outcome.failed_files.push(format!("{full_name}: {error}"));
    *outcome.failure_summary.entry(format!("ext={ext}: {}", redact_paths(error))).or_insert(0) += 1;
}

/// How many newly-indexed documents accumulate before an intermediate
/// commit - batches writes (epic #6 §21: "avoid excessive random writes...
/// committing periodically") rather than committing after every single
/// document, which would be real, avoidable overhead given the `trigram`
/// field's much higher per-document token count than the default
/// tokenizer. A final commit always happens at the end regardless of
/// whether this threshold was reached.
const COMMIT_BATCH_SIZE: i32 = 200;

/// Proactively indexes every extension-matching file under
/// `settings.search_path` (issue #6 Phase 1 - see the plan doc) -
/// independent of any filter text, unlike [`index_hits_for_fast_search`]
/// (which only ever indexed a completed run's *hits*). Reuses the exact
/// same walk/extension-filter/size-limit scoping a normal search uses
/// (`file_reader::enumerate_files_safely` +
/// `orchestrator::filter_by_extension`) and the same extension-dispatch
/// extraction table (`extraction::extract_lines_by_extension`) -
/// `process_one_file` in `orchestrator.rs` and this function are the two
/// callers of that shared table, kept from drifting apart.
///
/// A file that fails to read/extract is counted in `failed_count` and
/// skipped, never aborts the whole indexing run - matches this app's
/// established per-file error isolation (`process_one_file`'s own
/// behavior, epic §17's "a bad file must never stop indexing").
///
/// Read + extraction of changed files runs concurrently (bounded by
/// `settings.throttle_limit` when `settings.parallel`), writes to the index
/// stay serialized on this task. Cancelling commits what was indexed so far
/// and returns `Ok` with `cancelled = true`.
pub async fn build_or_update_corpus_index(
    settings: &SearchSettings,
    engine: &NativeSearchEngine,
    cancellation: &CancellationToken,
    on_progress: Option<&mut dyn FnMut(CorpusIndexProgress)>,
) -> NsResult<CorpusIndexOutcome> {
    build_or_update_corpus_index_impl(settings, engine, cancellation, on_progress).await
}

/// Same as [`build_or_update_corpus_index`], for callers that need the
/// returned future to be `Send` - a real `tokio::spawn`'d task on a
/// multi-thread `Runtime` (egui/eframe's async bridge) requires it,
/// unlike Dioxus's own single-threaded task spawner (every existing
/// caller of the plain function above). An unannotated `dyn FnMut`
/// trait object never carries `Send` even when the concrete closure
/// underneath does, so the plain function's future is unconditionally
/// `!Send` regardless of what's passed to it - this generic sibling
/// avoids the trait-object erasure entirely (monomorphized per caller),
/// rather than adding `+ Send` to the shared function, which would
/// force it onto callers (like Dioxus's) whose closures capture
/// non-`Send` reactive state and don't need it.
pub async fn build_or_update_corpus_index_send<F: FnMut(CorpusIndexProgress) + Send>(
    settings: &SearchSettings,
    engine: &NativeSearchEngine,
    cancellation: &CancellationToken,
    on_progress: Option<&mut F>,
) -> NsResult<CorpusIndexOutcome> {
    build_or_update_corpus_index_impl(settings, engine, cancellation, on_progress).await
}

/// Owned, `Copy` slice of `SearchSettings` the per-file read+extract task
/// needs - a spawned task must be `'static`, so it cannot borrow `settings`.
#[derive(Clone, Copy)]
struct ReadParams {
    file_timeout_seconds: u64,
    max_retries: i32,
    retry_delay_ms: u64,
    pdf_timeout_seconds: u64,
    ocr_scanned_pdfs: bool,
}

/// Why [`read_and_extract`] produced no lines.
enum ReadFail {
    /// Binary file or a format that yielded no text - indexed as an empty doc.
    NoText(&'static str),
    Error(String),
}

/// Reads one file and extracts its text lines. Errors are pre-formatted
/// strings (the failure list is display-only).
async fn read_and_extract(full_name: String, ext: String, p: ReadParams, cancellation: CancellationToken) -> Result<Vec<String>, ReadFail> {
    let bytes = file_reader::read_file_bytes_robust(&full_name, p.file_timeout_seconds, p.max_retries, p.retry_delay_ms, None, &cancellation)
        .await
        .map_err(|e| ReadFail::Error(e.to_string()))?;
    // Extraction is synchronous and CPU-bound (PDF/OOXML/zip) - keep it off
    // the async worker threads so concurrent reads are never starved by it.
    tokio::task::spawn_blocking(move || {
        extraction::extract_lines_by_extension(&ext, &bytes, p.pdf_timeout_seconds, None, p.ocr_scanned_pdfs)
            .map(|e| e.lines)
            .map_err(|e| match e {
                extraction::ExtractLinesError::Binary => ReadFail::NoText("binary file"),
                extraction::ExtractLinesError::Failed => ReadFail::NoText("no extractable text"),
            })
    })
    .await
    .map_err(|e| ReadFail::Error(format!("extraction task failed: {e}")))?
}

/// One staged-but-uncommitted document, kept so a failed commit (the writer
/// and its uncommitted documents are discarded) can be replayed.
struct PendingDoc {
    id: String,
    filename: String,
    ext: String,
    modified: i64,
    created: i64,
    size: i64,
    body: String,
}

impl PendingDoc {
    fn stage(&self, engine: &NativeSearchEngine) -> NsResult<()> {
        engine.index_document(DocumentInput {
            id: &self.id,
            path: &self.id,
            filename: &self.filename,
            extension: &self.ext,
            title: "",
            modified_unix: self.modified,
            created_unix: self.created,
            size: self.size,
            body: &self.body,
        })
    }
}

/// Commit attempts per batch. Windows antivirus/sync clients can deny the
/// creation of a segment file for a moment (`os error 5`); a commit that
/// fails discards the batch, so each retry re-stages it first.
const COMMIT_ATTEMPTS: u32 = 4;
/// Intermediate commit also triggers on staged body bytes so replay memory
/// stays bounded when documents are large.
const COMMIT_BATCH_BYTES: usize = 32 * 1024 * 1024;

async fn commit_with_replay(engine: &NativeSearchEngine, pending: &[PendingDoc]) -> Result<(), String> {
    let mut last = match engine.commit() {
        Ok(()) => return Ok(()),
        Err(e) => e.to_string(),
    };
    for attempt in 2..=COMMIT_ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_millis(250 * u64::from(attempt))).await;
        let result = pending.iter().try_for_each(|d| d.stage(engine)).and_then(|()| engine.commit());
        match result {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// Commits `pending` (replaying on failure) and records the outcome: on a
/// permanent failure the batch's documents move from indexed to failed.
async fn commit_pending(engine: &NativeSearchEngine, pending: &mut Vec<PendingDoc>, outcome: &mut CorpusIndexOutcome, label: &str) {
    if let Err(e) = commit_with_replay(engine, pending).await {
        let n = pending.len() as i32;
        outcome.indexed_count -= n;
        outcome.failed_count += n;
        outcome.failed_files.push(format!("{label} ({n} document(s) lost): {e}"));
        *outcome.failure_summary.entry(format!("{label}: {}", redact_paths(&e))).or_insert(0) += n as u32;
    }
    pending.clear();
}

async fn build_or_update_corpus_index_impl<F: FnMut(CorpusIndexProgress) + ?Sized>(
    settings: &SearchSettings,
    engine: &NativeSearchEngine,
    cancellation: &CancellationToken,
    mut on_progress: Option<&mut F>,
) -> NsResult<CorpusIndexOutcome> {
    let started = std::time::Instant::now();
    let mut outcome = CorpusIndexOutcome::default();
    let mut report = |outcome: &CorpusIndexOutcome, stage: IndexStage, done: i32, total: i32, current: &str| {
        if let Some(cb) = on_progress.as_deref_mut() {
            cb(CorpusIndexProgress {
                stage,
                files_processed: done,
                total_files: total,
                current_file: current.to_string(),
                indexed_count: outcome.indexed_count,
                failed_count: outcome.failed_count,
            });
        }
    };

    report(&outcome, IndexStage::Enumerating, 0, 0, &settings.search_path);
    // The directory walk is blocking filesystem work; running it inline would
    // park an async worker thread for the whole walk of a large tree.
    let walk_settings = (settings.search_path.clone(), settings.include_hidden, settings.exclude_folders.clone());
    let walk_cancel = cancellation.clone();
    let walked = tokio::task::spawn_blocking(move || {
        file_reader::enumerate_files_safely(&walk_settings.0, walk_settings.1, &walk_settings.2, &walk_cancel, None)
    })
    .await
    .map_err(|e| NsError::index_error(format!("directory walk task failed: {e}")))?;
    let Ok((all_files, enum_errors)) = walked else {
        // Cancelled before anything was written: nothing to commit.
        outcome.cancelled = true;
        outcome.elapsed_ms = started.elapsed().as_millis() as u64;
        return Ok(outcome);
    };
    outcome.enumerated_count = all_files.len() as i32;
    outcome.enumeration_errors = enum_errors;

    let candidates = filter_by_extension(all_files, settings);
    let max_bytes = (settings.max_file_size_mb * 1024.0 * 1024.0) as i64;
    let total_files = candidates.len() as i32;
    outcome.candidate_count = total_files;

    // Pass 1: decide which files need work. One bulk read of the index's
    // stored (modified, size) per id - per-file term lookups cost ~0.6us per
    // segment each, ~18s for 100k files.
    let known = engine.all_document_metadata().unwrap_or_default();
    let mut work: Vec<(file_reader::EnumeratedFile, String, String)> = Vec::new();
    for (i, file) in candidates.into_iter().enumerate() {
        if cancellation.is_cancelled() {
            outcome.cancelled = true;
            break;
        }
        let full_name = file.path.to_string_lossy().into_owned();
        if i % 256 == 0 {
            report(&outcome, IndexStage::Checking, i as i32, total_files, &full_name);
        }
        if file.length > max_bytes {
            outcome.too_large_count += 1;
            continue;
        }
        let modified_unix = file.modified.timestamp();
        if let Some(&(existing_modified, existing_size)) = known.get(&full_name) {
            if existing_modified == modified_unix && existing_size == file.length {
                outcome.skipped_count += 1;
                continue;
            }
        }
        let ext = file.path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
        work.push((file, full_name, ext));
    }

    // Pass 2: read+extract concurrently in order-preserving windows, index serially.
    let params = ReadParams {
        file_timeout_seconds: settings.file_timeout_seconds as u64,
        max_retries: settings.max_retries,
        retry_delay_ms: settings.retry_delay_ms as u64,
        pdf_timeout_seconds: settings.pdf_timeout_seconds as u64,
        ocr_scanned_pdfs: settings.ocr_scanned_pdfs,
    };
    let concurrency = if settings.parallel { settings.throttle_limit.clamp(1, 16) as usize } else { 1 };
    let work_total = work.len() as i32;
    let mut pending: Vec<PendingDoc> = Vec::new();
    let mut pending_bytes = 0usize;
    let mut done = 0i32;
    let mut work_iter = work.into_iter();

    'outer: while !outcome.cancelled {
        let window: Vec<_> = work_iter.by_ref().take(concurrency).collect();
        if window.is_empty() {
            break;
        }
        let handles: Vec<_> = window
            .iter()
            .map(|(_, full_name, ext)| tokio::spawn(read_and_extract(full_name.clone(), ext.clone(), params, cancellation.clone())))
            .collect();

        for ((file, full_name, ext), handle) in window.into_iter().zip(handles) {
            if cancellation.is_cancelled() {
                outcome.cancelled = true;
                handle.abort();
                continue;
            }
            report(&outcome, IndexStage::Indexing, done, work_total, &full_name);
            done += 1;
            let lines = match handle.await {
                Ok(Ok(l)) => l,
                Ok(Err(ReadFail::NoText(why))) => {
                    outcome.no_text_count += 1;
                    *outcome.failure_summary.entry(format!("ext={ext}: {why} (indexed empty)")).or_insert(0) += 1;
                    Vec::new()
                }
                Ok(Err(ReadFail::Error(e))) => {
                    record_failure(&mut outcome, &full_name, &ext, &e);
                    continue;
                }
                Err(e) => {
                    record_failure(&mut outcome, &full_name, &ext, &format!("read task failed: {e}"));
                    continue;
                }
            };

            let doc = PendingDoc {
                filename: file.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                id: full_name.clone(),
                ext: ext.clone(),
                modified: file.modified.timestamp(),
                created: file.created.timestamp(),
                size: file.length,
                body: lines.join("\n"),
            };
            // A single document failure must not abort the loop and discard
            // every remaining file: count it, keep going.
            if let Err(e) = doc.stage(engine) {
                record_failure(&mut outcome, &full_name, &ext, &e.to_string());
                continue;
            }
            outcome.indexed_count += 1;
            pending_bytes += doc.body.len();
            pending.push(doc);

            if pending.len() as i32 >= COMMIT_BATCH_SIZE || pending_bytes >= COMMIT_BATCH_BYTES {
                report(&outcome, IndexStage::Committing, done, work_total, &full_name);
                commit_pending(engine, &mut pending, &mut outcome, "commit").await;
                pending_bytes = 0;
            }
        }
        if outcome.cancelled {
            break 'outer;
        }
    }

    if !pending.is_empty() {
        report(&outcome, IndexStage::Committing, done, work_total, "");
        commit_pending(engine, &mut pending, &mut outcome, "final commit").await;
    }

    outcome.elapsed_ms = started.elapsed().as_millis() as u64;
    outcome.index_docs = engine.num_docs();
    tracing::info!(
        indexed = outcome.indexed_count,
        skipped = outcome.skipped_count,
        failed = outcome.failed_count,
        cancelled = outcome.cancelled,
        "corpus index build complete"
    );
    Ok(outcome)
}

/// Result of [`narrow_candidates`] - the candidate list plus the numbers a
/// UI/debug log needs to explain *why* a search did or did not use the index.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NarrowOutcome {
    /// `None` = no safe narrowing possible (no usable trigrams, regex with no
    /// required literals, ...) - the caller must do a normal full scan.
    pub candidates: Option<Vec<String>>,
    pub index_docs: u64,
    /// Files a full scan would have processed under the current settings.
    pub scannable: usize,
    pub from_index: usize,
    /// Files in scope that the index did not cover or has out of date
    /// (added/changed since the last build) - always included.
    pub stale_or_new: usize,
    /// Subset of `stale_or_new` an index update would actually (re)index
    /// (excludes files over `max_file_size_mb`, which are never indexed).
    pub needs_update: usize,
    /// Indexed documents whose file no longer exists on disk.
    pub removed: usize,
}

impl NarrowOutcome {
    /// The folder changed since the build in a way an update would fix.
    pub fn is_stale(&self) -> bool {
        self.needs_update > 0 || self.removed > 0
    }
}

/// Case-insensitive, separator-insensitive comparison key for a file path:
/// the index stores the exact string the walk produced, and on Windows the
/// same file may be reported with different case or `/` vs `\`.
fn path_key(p: &str) -> String {
    if cfg!(windows) {
        p.replace('\\', "/").to_lowercase()
    } else {
        p.to_string()
    }
}

/// Narrows a re-search to the files that can possibly match, using the
/// trigram index - **without ever dropping a real match**:
///
/// * the trigram query is a safe superset of the files containing the filter
///   (regex mode uses only the literal chunks every match must contain, and
///   falls back to a full scan when none can be proven);
/// * the result is intersected with what a full scan would walk *right now*
///   (current extension/exclude/hidden settings), so a settings change since
///   the last build cannot resurrect files the user excluded;
/// * every in-scope file that is missing from the index, or whose modified
///   time/size no longer matches it (added or edited after the build) is
///   included regardless, so a stale index can never hide a file.
pub async fn narrow_candidates(
    settings: &SearchSettings,
    engine: &NativeSearchEngine,
    cancellation: &CancellationToken,
) -> NsResult<NarrowOutcome> {
    let mut out = NarrowOutcome { index_docs: engine.num_docs(), ..Default::default() };

    let walk = (settings.search_path.clone(), settings.include_hidden, settings.exclude_folders.clone());
    let walk_cancel = cancellation.clone();
    let walked = tokio::task::spawn_blocking(move || file_reader::enumerate_files_safely(&walk.0, walk.1, &walk.2, &walk_cancel, None))
        .await
        .map_err(|e| NsError::index_error(format!("directory walk task failed: {e}")))?;
    let Ok((all_files, _)) = walked else {
        return Err(NsError::cancelled("candidate narrowing cancelled"));
    };
    let in_scope = filter_by_extension(all_files, settings);
    out.scannable = in_scope.len();
    let max_bytes = (settings.max_file_size_mb * 1024.0 * 1024.0) as i64;

    // Freshness: which in-scope files the index lacks or has out of date.
    let known = engine.all_document_metadata().unwrap_or_default();
    let mut stale: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut scope_keys: std::collections::HashSet<String> = std::collections::HashSet::with_capacity(in_scope.len());
    for file in &in_scope {
        let full_name = file.path.to_string_lossy().into_owned();
        scope_keys.insert(path_key(&full_name));
        let up_to_date = matches!(
            known.get(&full_name),
            Some(&(m, sz)) if m == file.modified.timestamp() && sz == file.length
        );
        if !up_to_date {
            out.stale_or_new += 1;
            if file.length <= max_bytes {
                out.needs_update += 1;
            }
            stale.insert(path_key(&full_name));
        }
    }
    if let Ok(ids) = engine.all_document_ids() {
        out.removed = ids.iter().filter(|id| !scope_keys.contains(&path_key(id)) && !Path::new(id.as_str()).exists()).count();
    }

    let indexed_paths = if settings.use_regex {
        let mut chunk_sets = Vec::with_capacity(settings.filters.len());
        for f in &settings.filters {
            match crate::regex_literals::required_literal_chunks(f) {
                Some(chunks) if !chunks.is_empty() => chunk_sets.push(chunks),
                _ => return Ok(out),
            }
        }
        engine.trigram_candidate_paths_for_chunk_sets(&chunk_sets)?
    } else {
        engine.trigram_candidate_paths(&settings.filters)?
    };
    let Some(indexed_paths) = indexed_paths else {
        return Ok(out);
    };
    let from_index: std::collections::HashSet<String> = indexed_paths.iter().map(|p| path_key(p)).collect();

    let mut chosen = Vec::new();
    for file in in_scope {
        let full_name = file.path.to_string_lossy().into_owned();
        let key = path_key(&full_name);
        if from_index.contains(&key) {
            out.from_index += 1;
            chosen.push(full_name);
        } else if stale.contains(&key) {
            chosen.push(full_name);
        }
    }
    out.candidates = Some(chosen);
    Ok(out)
}

/// Searches whatever's currently in the native_search index (built up via
/// [`index_hits_for_fast_search`] on prior runs) - a separate capability
/// from the normal per-run line scan (`orchestrator::run`), not a
/// replacement for it.
pub fn search(engine: &NativeSearchEngine, query: &str, limit: usize) -> NsResult<Vec<SearchHit>> {
    let start = std::time::Instant::now();
    let result = engine.search(query, limit, None);
    tracing::debug!(
        query = %query,
        result_count = result.as_ref().map(|r| r.len()).unwrap_or(0),
        elapsed_us = start.elapsed().as_micros() as u64,
        "query complete"
    );
    result
}

/// Issue #6 §50 "Index Health/Maintenance" - "remove orphaned documents":
/// deletes every indexed document whose path no longer exists on disk
/// (moved, renamed, or deleted since the file was indexed - possible any
/// time the corpus index isn't perfectly current with the filesystem,
/// e.g. before the next scheduled reconciliation scan or watcher event).
/// Commits once at the end if anything was actually removed. Returns the
/// number of documents removed.
pub fn remove_orphaned_documents(engine: &NativeSearchEngine) -> NsResult<usize> {
    let mut removed = 0usize;
    for id in engine.all_document_ids()? {
        if !Path::new(&id).exists() {
            engine.delete_document(&id)?;
            removed += 1;
        }
    }
    if removed > 0 {
        engine.commit()?;
    }
    Ok(removed)
}

/// Issue #6 §50 - "verify index": opens the index *without* the
/// auto-rebuild-on-schema-mismatch behavior `open_or_create_with_rebuild`
/// has (that would silently "fix" a corrupt/stale-schema index rather
/// than reporting it) and returns its document count on success. An `Err`
/// here - most commonly `NsStatus::CorruptIndex`, per `open_or_create`'s
/// own doc comment - is the caller's signal to offer/perform a rebuild,
/// not something this function does on its own.
pub fn verify_index(index_directory: &Path) -> NsResult<u64> {
    let engine = NativeSearchEngine::open_or_create(index_directory)?;
    Ok(engine.num_docs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FileSearchStatus;
    use chrono::{Local, TimeZone};

    fn sample_hit(full_name: &str, modified_unix: i64, size: i64, body_lines: &[&str]) -> FileSearchResult {
        FileSearchResult {
            full_name: full_name.to_string(),
            status: FileSearchStatus::Hit,
            hits: vec![],
            created: Local.timestamp_opt(modified_unix, 0).unwrap(),
            modified: Local.timestamp_opt(modified_unix, 0).unwrap(),
            file_length: size,
            lines_cache: body_lines.iter().map(|s| s.to_string()).collect(),
            total_line_count: body_lines.len() as i32,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }
    }

    #[test]
    fn open_or_create_with_rebuild_behaves_like_plain_open_in_the_normal_case() {
        // The corrupt-index-recovery branch itself is exercised at the
        // native-search level (engine::tests::
        // opening_index_with_mismatched_schema_is_corrupt_index_not_panic
        // constructs the mismatched-schema fixture that error path needs -
        // search-core deliberately has no direct tantivy dependency per
        // ADR-001, so it can't build that same fixture here). This
        // confirms the wrapper isn't a regression for the common,
        // non-corrupt case any caller actually hits most of the time.
        let dir = tempfile::tempdir().unwrap();
        let engine = open_or_create_with_rebuild(dir.path()).unwrap();
        engine.index_document(DocumentInput {
            id: "1",
            path: "/x/a.txt",
            filename: "a.txt",
            extension: ".txt",
            title: "",
            modified_unix: 0,
            created_unix: 0,
            size: 1,
            body: "hello",
        }).unwrap();
        engine.commit().unwrap();
        drop(engine);

        let reopened = open_or_create_with_rebuild(dir.path()).unwrap();
        assert_eq!(reopened.num_docs(), 1, "reopening an already-current-schema index must not rebuild it");
    }

    #[test]
    fn index_directory_is_nested_inside_search_path() {
        let dir = index_directory("/x/y/project");
        assert_eq!(dir, PathBuf::from("/x/y/project/.native-search-index"));
    }

    #[test]
    fn ensure_index_folder_excluded_adds_once_case_insensitively() {
        let mut folders = vec!["bin".to_string()];
        ensure_index_folder_excluded(&mut folders);
        assert_eq!(folders, vec!["bin".to_string(), INDEX_FOLDER_NAME.to_string()]);

        ensure_index_folder_excluded(&mut folders);
        assert_eq!(folders.len(), 2, "must not add a duplicate");

        let mut already_upper = vec![".NATIVE-SEARCH-INDEX".to_string()];
        ensure_index_folder_excluded(&mut already_upper);
        assert_eq!(already_upper.len(), 1, "case-insensitive match must not add a duplicate");
    }

    #[test]
    fn status_message_covers_all_four_combinations() {
        assert_eq!(IndexOutcome { indexed_count: 0, skipped_count: 0 }.status_message(), "No hits to index for fast re-search.");
        assert_eq!(
            IndexOutcome { indexed_count: 0, skipped_count: 3 }.status_message(),
            "All 3 file(s) already up to date in the fast index. Search above."
        );
        assert_eq!(
            IndexOutcome { indexed_count: 2, skipped_count: 0 }.status_message(),
            "Indexed 2 file(s) for fast re-search. Search above."
        );
        assert_eq!(
            IndexOutcome { indexed_count: 2, skipped_count: 3 }.status_message(),
            "Indexed 2 file(s), 3 already up to date. Search above."
        );
    }

    /// Reproduces the actual app flow end to end - real files on disk,
    /// through `orchestrator::run` (not a hand-built `FileSearchResult`
    /// like the other tests here), through `index_hits_for_fast_search`,
    /// then a real `NativeSearchEngine::open_or_create` + `search` in a
    /// fresh engine instance (matching `app/src/state.rs`'s
    /// `run_native_search`, which always opens a brand new engine handle
    /// rather than reusing the one indexing used) - written to chase down
    /// a "the indexer doesn't work" report the synthetic-hit tests above
    /// wouldn't have caught.
    /// The core correctness claim of issue #6 Phase 1: the index-first
    /// path (trigram candidate query -> `orchestrator::run_candidates`)
    /// must find exactly the same hits, with identical line/context data,
    /// as the unchanged full-scan path (`orchestrator::run`) - the index
    /// is a fast pre-filter, never a second, potentially-divergent way of
    /// getting search results. Uses a filter ("eng") that the *default*
    /// Tantivy tokenizer would NOT match as a token, to prove this isn't
    /// silently relying on token-level matching happening to agree with
    /// substring matching here.
    #[tokio::test]
    async fn index_first_routing_agrees_with_full_scan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "torque spec deviation on engine mount\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "unrelated corrosion inspection notes\n").unwrap();
        std::fs::write(dir.path().join("c.txt"), "another engineering report, different content\n").unwrap();

        let mut exclude_folders = Vec::new();
        ensure_index_folder_excluded(&mut exclude_folders);
        let settings = crate::models::SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            output_folder: dir.path().to_string_lossy().into_owned(),
            filters: vec!["eng".to_string()],
            exclude_folders,
            ..Default::default()
        };

        let full_scan = crate::orchestrator::run(settings.clone(), None, tokio_util::sync::CancellationToken::new())
            .await
            .unwrap();

        let index_dir = index_directory(&dir.path().to_string_lossy());
        ensure_index_directory_exists(&index_dir).unwrap();
        let engine = NativeSearchEngine::open_or_create(&index_dir).unwrap();
        let build_outcome =
            build_or_update_corpus_index(&settings, &engine, &tokio_util::sync::CancellationToken::new(), None)
                .await
                .unwrap();
        assert_eq!(build_outcome.indexed_count, 3);

        let candidates = engine.trigram_candidate_paths(&settings.filters).unwrap().expect("3-char filter must narrow");
        assert_eq!(candidates.len(), 2, "a.txt and c.txt contain 'eng' as a substring, b.txt does not");

        let index_first =
            crate::orchestrator::run_candidates(&candidates, settings, None, tokio_util::sync::CancellationToken::new())
                .await
                .unwrap();

        let mut full_hit_files: Vec<&str> = full_scan
            .file_results
            .iter()
            .filter(|r| r.status == FileSearchStatus::Hit)
            .map(|r| r.full_name.as_str())
            .collect();
        let mut narrowed_hit_files: Vec<&str> = index_first
            .file_results
            .iter()
            .filter(|r| r.status == FileSearchStatus::Hit)
            .map(|r| r.full_name.as_str())
            .collect();
        full_hit_files.sort();
        narrowed_hit_files.sort();
        assert_eq!(full_hit_files, narrowed_hit_files, "index-first must find exactly the same hit files as a full scan");

        for full_name in &full_hit_files {
            let from_full = full_scan.file_results.iter().find(|r| r.full_name == *full_name).unwrap();
            let from_narrowed = index_first.file_results.iter().find(|r| r.full_name == *full_name).unwrap();
            assert_eq!(from_full.hits.len(), from_narrowed.hits.len());
            assert_eq!(from_full.hits[0].match_line, from_narrowed.hits[0].match_line, "line content must be identical");
        }
    }

    #[tokio::test]
    async fn regex_mode_index_first_routing_agrees_with_full_scan() {
        // Same shape as `index_first_routing_agrees_with_full_scan`, but
        // exercises the §24 "regex candidate filtering" path: the filter
        // is a regex pattern, narrowed via
        // `regex_literals::required_literal_chunks("eng.*mount")` =>
        // ["eng", "mount"] rather than the plain-literal trigram path.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "torque spec deviation on engine mount\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "unrelated corrosion inspection notes\n").unwrap();
        std::fs::write(dir.path().join("c.txt"), "engine reported separately from any mount reference\n").unwrap();

        let mut exclude_folders = Vec::new();
        ensure_index_folder_excluded(&mut exclude_folders);
        let settings = crate::models::SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            output_folder: dir.path().to_string_lossy().into_owned(),
            filters: vec!["eng.*mount".to_string()],
            use_regex: true,
            exclude_folders,
            ..Default::default()
        };

        let full_scan = crate::orchestrator::run(settings.clone(), None, tokio_util::sync::CancellationToken::new())
            .await
            .unwrap();

        let index_dir = index_directory(&dir.path().to_string_lossy());
        ensure_index_directory_exists(&index_dir).unwrap();
        let engine = NativeSearchEngine::open_or_create(&index_dir).unwrap();
        let build_outcome =
            build_or_update_corpus_index(&settings, &engine, &tokio_util::sync::CancellationToken::new(), None)
                .await
                .unwrap();
        assert_eq!(build_outcome.indexed_count, 3);

        let chunk_sets: Vec<Vec<String>> = settings
            .filters
            .iter()
            .map(|f| crate::regex_literals::required_literal_chunks(f).expect("this pattern must extract chunks"))
            .collect();
        assert_eq!(chunk_sets, vec![vec!["eng".to_string(), "mount".to_string()]]);

        let candidates = engine.trigram_candidate_paths_for_chunk_sets(&chunk_sets).unwrap().expect("must narrow");
        assert_eq!(candidates.len(), 2, "a.txt and c.txt contain both chunks, b.txt contains neither");

        let index_first =
            crate::orchestrator::run_candidates(&candidates, settings, None, tokio_util::sync::CancellationToken::new())
                .await
                .unwrap();

        let mut full_hit_files: Vec<&str> = full_scan
            .file_results
            .iter()
            .filter(|r| r.status == FileSearchStatus::Hit)
            .map(|r| r.full_name.as_str())
            .collect();
        let mut narrowed_hit_files: Vec<&str> = index_first
            .file_results
            .iter()
            .filter(|r| r.status == FileSearchStatus::Hit)
            .map(|r| r.full_name.as_str())
            .collect();
        full_hit_files.sort();
        narrowed_hit_files.sort();
        assert_eq!(full_hit_files, narrowed_hit_files, "regex index-first must find exactly the same hit files as a full scan");
    }

    #[tokio::test]
    async fn full_pipeline_orchestrator_run_then_index_then_native_search_finds_hit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("report.txt"), "quarterly torque figures\nnothing else here\n").unwrap();
        std::fs::write(dir.path().join("other.txt"), "unrelated content\n").unwrap();

        let mut exclude_folders = Vec::new();
        ensure_index_folder_excluded(&mut exclude_folders);
        let settings = crate::models::SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            output_folder: dir.path().to_string_lossy().into_owned(),
            filters: vec!["torque".to_string()],
            exclude_folders,
            ..Default::default()
        };

        let run_result = crate::orchestrator::run(settings, None, tokio_util::sync::CancellationToken::new())
            .await
            .unwrap();
        let hit_results: Vec<FileSearchResult> = run_result
            .file_results
            .into_iter()
            .filter(|r| r.status == FileSearchStatus::Hit)
            .collect();
        assert_eq!(hit_results.len(), 1, "expected exactly one real hit file from the orchestrator run");

        let index_dir = index_directory(&dir.path().to_string_lossy());
        ensure_index_directory_exists(&index_dir).unwrap();
        {
            let engine = NativeSearchEngine::open_or_create(&index_dir).unwrap();
            let outcome = index_hits_for_fast_search(&engine, &hit_results).unwrap();
            assert_eq!(outcome.indexed_count, 1, "the real hit file must actually get indexed");
        }

        // A fresh engine handle, exactly like run_native_search opens
        // separately from whatever indexed the documents.
        let search_engine = NativeSearchEngine::open_or_create(&index_dir).unwrap();
        let results = search(&search_engine, "torque", 10).unwrap();
        assert_eq!(results.len(), 1, "fast re-search must find the just-indexed document");
        assert!(results[0].id.ends_with("report.txt"), "unexpected id: {}", results[0].id);
    }

    #[tokio::test]
    async fn build_or_update_corpus_index_indexes_every_matching_file_not_just_hits() {
        // The whole point of the proactive corpus indexer vs.
        // index_hits_for_fast_search: every extension-matching file gets
        // indexed, regardless of whether it would have matched any
        // particular search filter.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple pie recipe").unwrap();
        std::fs::write(dir.path().join("b.txt"), "totally unrelated content").unwrap();
        std::fs::write(dir.path().join("c.md"), "markdown, not a matched extension").unwrap();

        let index_dir = index_directory(&dir.path().to_string_lossy());
        ensure_index_directory_exists(&index_dir).unwrap();
        let engine = NativeSearchEngine::open_or_create(&index_dir).unwrap();

        let mut exclude_folders = Vec::new();
        ensure_index_folder_excluded(&mut exclude_folders);
        let settings = crate::models::SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            output_folder: dir.path().to_string_lossy().into_owned(),
            extensions: Some(vec![".txt".to_string()]),
            exclude_folders,
            ..Default::default()
        };

        let outcome =
            build_or_update_corpus_index(&settings, &engine, &tokio_util::sync::CancellationToken::new(), None)
                .await
                .unwrap();
        assert_eq!(outcome.indexed_count, 2, "both .txt files, neither filtered by any search term");
        assert_eq!(engine.num_docs(), 2);

        let results = search(&engine, "unrelated", 10).unwrap();
        assert_eq!(results.len(), 1, "a file that would never be a search 'hit' for most terms is still indexed");
    }

    #[tokio::test]
    async fn build_or_update_corpus_index_skips_unchanged_files_on_rerun() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple pie recipe").unwrap();

        let index_dir = index_directory(&dir.path().to_string_lossy());
        ensure_index_directory_exists(&index_dir).unwrap();
        let engine = NativeSearchEngine::open_or_create(&index_dir).unwrap();

        let mut exclude_folders = Vec::new();
        ensure_index_folder_excluded(&mut exclude_folders);
        let settings = crate::models::SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            output_folder: dir.path().to_string_lossy().into_owned(),
            extensions: Some(vec![".txt".to_string()]),
            exclude_folders,
            ..Default::default()
        };

        let cancel = tokio_util::sync::CancellationToken::new();
        let first = build_or_update_corpus_index(&settings, &engine, &cancel, None).await.unwrap();
        assert_eq!(first.indexed_count, 1);

        let second = build_or_update_corpus_index(&settings, &engine, &cancel, None).await.unwrap();
        assert_eq!(second.indexed_count, 0, "unchanged file must be skipped on rerun");
        assert_eq!(second.skipped_count, 1);
    }

    #[test]
    fn index_then_search_finds_indexed_document() {
        let dir = tempfile::tempdir().unwrap();
        ensure_index_directory_exists(dir.path()).unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();

        let hits = vec![sample_hit("/x/apple.txt", 1_700_000_000, 10, &["apple pie recipe"])];
        let outcome = index_hits_for_fast_search(&engine, &hits).unwrap();
        assert_eq!(outcome.indexed_count, 1);
        assert_eq!(outcome.skipped_count, 0);

        let results = search(&engine, "apple", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "/x/apple.txt");
    }

    /// Issue #6 §52 "Concurrency Correctness" - "simultaneous indexing and
    /// searching". Runs a real corpus-index build concurrently with a
    /// burst of searches against the same live `NativeSearchEngine` -
    /// both take `&self` (interior mutability inside the engine, not a
    /// `&mut` borrow), so this must not panic, deadlock, or corrupt the
    /// index, and the indexed content must be reliably findable once the
    /// indexing future completes.
    #[tokio::test]
    async fn concurrent_indexing_and_searching_against_the_same_engine_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "torque spec deviation on engine mount\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "unrelated corrosion notes\n").unwrap();

        let mut exclude_folders = Vec::new();
        ensure_index_folder_excluded(&mut exclude_folders);
        let settings = crate::models::SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            output_folder: dir.path().to_string_lossy().into_owned(),
            exclude_folders,
            ..Default::default()
        };

        let index_dir = index_directory(&dir.path().to_string_lossy());
        ensure_index_directory_exists(&index_dir).unwrap();
        let engine = NativeSearchEngine::open_or_create(&index_dir).unwrap();

        let cancellation = tokio_util::sync::CancellationToken::new();
        let index_fut = build_or_update_corpus_index(&settings, &engine, &cancellation, None);
        let search_fut = async {
            for _ in 0..20 {
                let _ = engine.search("torque", 10, None);
                tokio::task::yield_now().await;
            }
        };
        let (index_result, ()) = tokio::join!(index_fut, search_fut);
        assert!(index_result.is_ok(), "concurrent search must not make indexing fail: {index_result:?}");
        assert_eq!(index_result.unwrap().indexed_count, 2);

        let hits = engine.search("torque", 10, None).unwrap();
        assert_eq!(hits.len(), 1, "the document must be reliably findable once indexing has completed");
    }

    #[test]
    fn reindexing_unchanged_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        ensure_index_directory_exists(dir.path()).unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();

        let hits = vec![sample_hit("/x/apple.txt", 1_700_000_000, 10, &["apple pie recipe"])];
        let first = index_hits_for_fast_search(&engine, &hits).unwrap();
        assert_eq!(first.indexed_count, 1);

        let second = index_hits_for_fast_search(&engine, &hits).unwrap();
        assert_eq!(second.indexed_count, 0, "unchanged mtime+size must be skipped");
        assert_eq!(second.skipped_count, 1);
    }

    #[test]
    fn changed_file_gets_reindexed_not_skipped() {
        let dir = tempfile::tempdir().unwrap();
        ensure_index_directory_exists(dir.path()).unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();

        let v1 = vec![sample_hit("/x/apple.txt", 1_700_000_000, 10, &["apple pie recipe"])];
        index_hits_for_fast_search(&engine, &v1).unwrap();

        let v2 = vec![sample_hit("/x/apple.txt", 1_700_000_500, 20, &["apple pie recipe, revised"])];
        let outcome = index_hits_for_fast_search(&engine, &v2).unwrap();
        assert_eq!(outcome.indexed_count, 1, "changed mtime/size must trigger a re-index");
        assert_eq!(outcome.skipped_count, 0);
    }

    #[test]
    fn remove_orphaned_documents_deletes_only_paths_missing_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let real_file = dir.path().join("still-here.txt");
        std::fs::write(&real_file, "content").unwrap();
        let real_path = real_file.to_string_lossy().into_owned();
        let gone_path = dir.path().join("deleted-since-indexing.txt").to_string_lossy().into_owned();

        ensure_index_directory_exists(dir.path()).unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        index_hits_for_fast_search(&engine, &[sample_hit(&real_path, 1_700_000_000, 7, &["kept"])]).unwrap();
        index_hits_for_fast_search(&engine, &[sample_hit(&gone_path, 1_700_000_000, 7, &["orphaned"])]).unwrap();
        assert_eq!(engine.num_docs(), 2);

        let removed = remove_orphaned_documents(&engine).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(engine.num_docs(), 1);
        assert_eq!(engine.all_document_ids().unwrap(), vec![real_path]);
    }

    #[test]
    fn remove_orphaned_documents_is_a_no_op_when_nothing_is_orphaned() {
        let dir = tempfile::tempdir().unwrap();
        let real_file = dir.path().join("still-here.txt");
        std::fs::write(&real_file, "content").unwrap();
        let real_path = real_file.to_string_lossy().into_owned();

        ensure_index_directory_exists(dir.path()).unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        index_hits_for_fast_search(&engine, &[sample_hit(&real_path, 1_700_000_000, 7, &["kept"])]).unwrap();

        assert_eq!(remove_orphaned_documents(&engine).unwrap(), 0);
        assert_eq!(engine.num_docs(), 1);
    }

    #[test]
    fn verify_index_reports_the_document_count() {
        let dir = tempfile::tempdir().unwrap();
        ensure_index_directory_exists(dir.path()).unwrap();
        {
            let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
            index_hits_for_fast_search(&engine, &[sample_hit("/x/a.txt", 1_700_000_000, 7, &["hi"])]).unwrap();
        }
        assert_eq!(verify_index(dir.path()).unwrap(), 1);
    }

    #[test]
    fn verify_index_reports_corrupt_index_instead_of_silently_rebuilding() {
        let dir = tempfile::tempdir().unwrap();
        ensure_index_directory_exists(dir.path()).unwrap();
        {
            // Build an index, then reopen with a schema that will look
            // "mismatched" to a fresh open_or_create call - simplest way
            // to reproduce is to just corrupt meta.json directly, same
            // approach `open_or_create`'s own existing corruption test
            // family uses elsewhere in this codebase.
            let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
            index_hits_for_fast_search(&engine, &[sample_hit("/x/a.txt", 1_700_000_000, 7, &["hi"])]).unwrap();
        }
        std::fs::write(dir.path().join("meta.json"), "not valid json at all").unwrap();

        assert!(verify_index(dir.path()).is_err(), "a corrupt index must surface as an error, not silently rebuild");
    }

    async fn built_engine(dir: &Path, settings: &SearchSettings) -> NativeSearchEngine {
        let index_dir = dir.join(".idx-test");
        ensure_index_directory_exists(&index_dir).unwrap();
        let engine = open_or_create_with_rebuild(&index_dir).unwrap();
        let mut noop = |_p: CorpusIndexProgress| {};
        let out = build_or_update_corpus_index_send(settings, &engine, &CancellationToken::new(), Some(&mut noop)).await.unwrap();
        assert_eq!(out.failed_count, 0, "{:?}", out.failed_files);
        engine
    }

    fn narrow_settings(dir: &Path, filter: &str) -> SearchSettings {
        let mut s = SearchSettings { search_path: dir.to_string_lossy().into_owned(), filters: vec![filter.to_string()], ..Default::default() };
        ensure_index_folder_excluded(&mut s.exclude_folders);
        s.exclude_folders.push(".idx-test".to_string());
        s
    }

    #[tokio::test]
    async fn narrowing_includes_files_added_after_the_index_was_built() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("old.txt"), "needle in old\n").unwrap();
        std::fs::write(dir.path().join("other.txt"), "nothing\n").unwrap();
        let settings = narrow_settings(dir.path(), "needle");
        let engine = built_engine(dir.path(), &settings).await;

        std::fs::write(dir.path().join("new.txt"), "needle in new\n").unwrap();
        let out = narrow_candidates(&settings, &engine, &CancellationToken::new()).await.unwrap();
        let names: Vec<String> = out.candidates.unwrap().iter().map(|p| Path::new(p).file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"old.txt".to_string()));
        assert!(names.contains(&"new.txt".to_string()), "a file added after the build must never be hidden by the index");
        assert!(!names.contains(&"other.txt".to_string()), "unchanged non-matching file must be narrowed away");
        assert_eq!(out.stale_or_new, 1);
    }

    #[tokio::test]
    async fn narrowing_includes_files_modified_after_the_index_was_built() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "nothing here\n").unwrap();
        let settings = narrow_settings(dir.path(), "needle");
        let engine = built_engine(dir.path(), &settings).await;

        // Different length => different size => detected as stale regardless of mtime granularity.
        std::fs::write(dir.path().join("a.txt"), "now it has the needle word\n").unwrap();
        let out = narrow_candidates(&settings, &engine, &CancellationToken::new()).await.unwrap();
        assert_eq!(out.candidates.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn freshly_built_index_is_not_stale_and_edits_adds_deletes_make_it_stale() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "needle one\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "needle two\n").unwrap();
        let settings = narrow_settings(dir.path(), "needle");
        let engine = built_engine(dir.path(), &settings).await;
        let token = CancellationToken::new();

        assert!(!narrow_candidates(&settings, &engine, &token).await.unwrap().is_stale());

        std::fs::write(dir.path().join("c.txt"), "added later\n").unwrap();
        let out = narrow_candidates(&settings, &engine, &token).await.unwrap();
        assert_eq!((out.needs_update, out.removed), (1, 0));

        std::fs::remove_file(dir.path().join("a.txt")).unwrap();
        let out = narrow_candidates(&settings, &engine, &token).await.unwrap();
        assert_eq!((out.needs_update, out.removed), (1, 1));
        assert!(out.is_stale());
    }

    #[tokio::test]
    async fn files_without_extractable_text_are_indexed_empty_and_never_stale() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
        std::fs::write(dir.path().join("blob.txt"), b"bin\0ary\0data").unwrap();
        std::fs::write(dir.path().join("scan.pdf"), b"not a pdf").unwrap();
        let mut settings = narrow_settings(dir.path(), "needle");
        settings.extensions = Some(vec![".txt".to_string(), ".pdf".to_string()]);
        let engine = built_engine(dir.path(), &settings).await;
        assert_eq!(engine.num_docs(), 3, "no-text files are stored so their mtime/size are known");

        let out = narrow_candidates(&settings, &engine, &CancellationToken::new()).await.unwrap();
        assert!(!out.is_stale(), "no-text files must not be reported as needing an update");
        assert_eq!(out.candidates.unwrap().len(), 1, "only the file that contains the needle");
    }

    #[tokio::test]
    async fn narrowing_respects_the_current_extension_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
        std::fs::write(dir.path().join("b.log"), "needle\n").unwrap();
        let mut settings = narrow_settings(dir.path(), "needle");
        settings.extensions = Some(vec![".txt".to_string(), ".log".to_string()]);
        let engine = built_engine(dir.path(), &settings).await;

        settings.extensions = Some(vec![".txt".to_string()]);
        let out = narrow_candidates(&settings, &engine, &CancellationToken::new()).await.unwrap();
        let cands = out.candidates.unwrap();
        assert_eq!(cands.len(), 1);
        assert!(cands[0].ends_with("a.txt"));
    }

    #[tokio::test]
    async fn narrowing_in_regex_mode_never_uses_the_raw_pattern_as_a_literal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "engine mount\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "nothing\n").unwrap();
        let mut settings = narrow_settings(dir.path(), "eng.*mount");
        settings.use_regex = true;
        let engine = built_engine(dir.path(), &settings).await;

        let out = narrow_candidates(&settings, &engine, &CancellationToken::new()).await.unwrap();
        let cands = out.candidates.expect("literal chunks 'eng' and 'mount' allow narrowing");
        assert!(cands.iter().any(|p| p.ends_with("a.txt")), "the raw pattern as a literal would have dropped this real match");
        assert!(!cands.iter().any(|p| p.ends_with("b.txt")));

        settings.filters = vec![".*".to_string()];
        let out = narrow_candidates(&settings, &engine, &CancellationToken::new()).await.unwrap();
        assert!(out.candidates.is_none(), "no provable literal => full scan");
    }

    #[tokio::test]
    async fn cancelled_build_commits_partial_work_and_reports_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
        }
        let settings = narrow_settings(dir.path(), "x");
        let index_dir = dir.path().join(".idx-test");
        ensure_index_directory_exists(&index_dir).unwrap();
        let engine = open_or_create_with_rebuild(&index_dir).unwrap();
        let token = CancellationToken::new();
        token.cancel();
        let out = build_or_update_corpus_index_send(&settings, &engine, &token, None::<&mut fn(CorpusIndexProgress)>).await.unwrap();
        assert!(out.cancelled);
        assert_eq!(out.indexed_count, 0);
    }

    #[tokio::test]
    async fn parallel_build_indexes_the_same_documents_as_sequential() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..30 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), format!("content {i}\n")).unwrap();
        }
        let mut settings = narrow_settings(dir.path(), "content");
        settings.parallel = true;
        settings.throttle_limit = 4;
        let engine = built_engine(dir.path(), &settings).await;
        assert_eq!(engine.num_docs(), 30);
    }

    #[test]
    fn redact_paths_removes_unix_windows_and_quoted_paths_but_keeps_plain_text() {
        let r = redact_paths(r#"cannot open "C:\Users\Jo Smith\secret plan.docx" at /home/jo/x.txt (os error 5) D:\a\b"#);
        assert!(!r.contains("Smith") && !r.contains("secret") && !r.contains("jo/") && !r.contains("D:\\a"), "{r}");
        assert!(r.contains("os error 5") && r.contains("cannot open"));
        assert_eq!(redact_paths("access denied"), "access denied");
    }

    #[test]
    fn failure_summary_never_contains_the_file_name() {
        let mut o = CorpusIndexOutcome::default();
        record_failure(&mut o, "/home/jo/Secret Plan.docx", ".docx", "failed reading /home/jo/Secret Plan.docx: permission denied");
        let joined = o.failure_summary.keys().cloned().collect::<Vec<_>>().join("|");
        assert!(!joined.contains("Secret") && !joined.contains("/home"), "{joined}");
        assert!(joined.contains("ext=.docx") && joined.contains("permission denied"));
    }
}
