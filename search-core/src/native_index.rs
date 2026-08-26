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

/// Live status while [`build_or_update_corpus_index`] is running - handed
/// to the caller's progress callback, same shape/spirit as
/// `orchestrator::SearchProgressReport` but scoped to what indexing
/// actually has to report (no match-mode/hit-count concepts here).
pub struct CorpusIndexProgress {
    pub files_processed: i32,
    pub total_files: i32,
    pub current_file: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CorpusIndexOutcome {
    pub indexed_count: i32,
    pub skipped_count: i32,
    pub failed_count: i32,
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
pub async fn build_or_update_corpus_index(
    settings: &SearchSettings,
    engine: &NativeSearchEngine,
    cancellation: &CancellationToken,
    mut on_progress: Option<&mut dyn FnMut(CorpusIndexProgress)>,
) -> NsResult<CorpusIndexOutcome> {
    let (all_files, _enum_errors) = file_reader::enumerate_files_safely(
        &settings.search_path,
        settings.include_hidden,
        &settings.exclude_folders,
        cancellation,
        None,
    )
    .map_err(|_| NsError::cancelled("corpus indexing cancelled during directory enumeration"))?;

    let candidates = filter_by_extension(all_files, settings);
    let max_bytes = (settings.max_file_size_mb * 1024.0 * 1024.0) as i64;
    let total_files = candidates.len() as i32;

    let mut outcome = CorpusIndexOutcome::default();
    let mut pending_commits = 0i32;

    for (i, file) in candidates.into_iter().enumerate() {
        if cancellation.is_cancelled() {
            return Err(NsError::cancelled("corpus indexing cancelled"));
        }
        let full_name = file.path.to_string_lossy().into_owned();
        if let Some(cb) = on_progress.as_deref_mut() {
            cb(CorpusIndexProgress { files_processed: i as i32, total_files, current_file: full_name.clone() });
        }

        if file.length > max_bytes {
            continue;
        }

        let modified_unix = file.modified.timestamp();
        if let Ok(Some((existing_modified, existing_size))) = engine.get_document_metadata(&full_name) {
            if existing_modified == modified_unix && existing_size == file.length {
                outcome.skipped_count += 1;
                continue;
            }
        }

        let bytes = match file_reader::read_file_bytes_robust(
            &full_name,
            settings.file_timeout_seconds as u64,
            settings.max_retries,
            settings.retry_delay_ms as u64,
            None,
            cancellation,
        )
        .await
        {
            Ok(b) => b,
            Err(_) => {
                outcome.failed_count += 1;
                continue;
            }
        };

        let ext = file.path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
        let extracted = extraction::extract_lines_by_extension(&ext, &bytes, settings.pdf_timeout_seconds as u64, None);
        let lines = match extracted {
            Ok(e) => e.lines,
            Err(_) => {
                outcome.failed_count += 1;
                continue;
            }
        };

        let file_name = file.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let body = lines.join("\n");
        engine.index_document(DocumentInput {
            id: &full_name,
            path: &full_name,
            filename: &file_name,
            extension: &ext,
            title: "",
            modified_unix,
            created_unix: file.created.timestamp(),
            size: file.length,
            body: &body,
        })?;
        outcome.indexed_count += 1;
        pending_commits += 1;

        if pending_commits >= COMMIT_BATCH_SIZE {
            engine.commit()?;
            pending_commits = 0;
        }
    }

    if pending_commits > 0 {
        engine.commit()?;
    }

    tracing::info!(
        indexed = outcome.indexed_count,
        skipped = outcome.skipped_count,
        failed = outcome.failed_count,
        "corpus index build complete"
    );
    Ok(outcome)
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
}
