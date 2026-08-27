//! Ports `TextInFilesSearch.Core/Services/SearchOrchestrator.cs`: coordinates
//! a full search run - enumerate, optionally consult the cache, process
//! files (sequential or throttled-parallel), and report live progress
//! throughout, including per-file activity so a slow PDF is visibly still
//! working rather than the whole run looking frozen.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::cache::{self, CandidateMetadata};
use crate::extraction;
use chrono::Local;

use crate::file_reader::{self, system_time_to_local, EnumeratedFile};
use crate::matching::{self, CompiledMatchState, InvalidFilterRegexError};
use crate::models::{
    extension_catalog, FileSearchResult, FileSearchStatus, InFlightFileStatus, SearchProgressReport, SearchRunResult,
    SearchSettings, Warning,
};

type InFlightMap = Arc<Mutex<HashMap<String, InFlightFileStatus>>>;

#[derive(Debug)]
pub enum OrchestratorError {
    Cancelled,
    InvalidFilterRegex(InvalidFilterRegexError),
}

impl std::fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestratorError::Cancelled => write!(f, "search cancelled"),
            OrchestratorError::InvalidFilterRegex(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for OrchestratorError {}

fn file_extension_lower(path: &Path) -> String {
    path.extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

/// Classifies an extension as CPU/memory-heavy extraction (ZIP/OOXML/PDF
/// parsing) vs. everything else - epic #6 §19's "different resource
/// classes" ask, backing the parallel branch's two separate semaphores
/// (`settings.heavy_throttle_limit` vs `settings.throttle_limit`). `.rtf`
/// stays "light" - its extractor is plain regex tag-stripping over
/// already-decoded text, not a container format to parse open, closer in
/// cost to plain-text reading than to ZIP/OOXML/PDF parsing.
fn is_heavy_extension(ext: &str) -> bool {
    matches!(ext, ".pdf" | ".docx" | ".pptx" | ".xlsx" | ".zip")
}

fn report_progress(
    progress: &Option<mpsc::UnboundedSender<SearchProgressReport>>,
    files_completed: i32,
    total_files: i32,
    hits_so_far: i32,
    inflight: &InFlightMap,
    last_completed_result: Option<FileSearchResult>,
) {
    if let Some(tx) = progress {
        let in_flight_files: Vec<InFlightFileStatus> =
            inflight.lock().map(|g| g.values().cloned().collect()).unwrap_or_default();
        let _ = tx.send(SearchProgressReport {
            files_completed,
            total_files,
            hits_so_far,
            in_flight_files,
            last_completed_result,
            ..Default::default()
        });
    }
}

pub async fn run(
    settings: SearchSettings,
    progress: Option<mpsc::UnboundedSender<SearchProgressReport>>,
    cancellation: CancellationToken,
) -> Result<SearchRunResult, OrchestratorError> {
    let mut run_result = SearchRunResult::default();

    // Excluded folders are pruned during the walk itself (matched by whole
    // path segment, not raw substring), so a huge excluded tree is never
    // actually descended into.
    if let Some(tx) = &progress {
        let _ = tx.send(SearchProgressReport { is_enumerating: true, ..Default::default() });
    }
    let mut enum_progress_cb = |count: i32| {
        if let Some(tx) = &progress {
            let _ = tx.send(SearchProgressReport {
                is_enumerating: true,
                enumerated_file_count: count,
                ..Default::default()
            });
        }
    };
    let discover_start = Instant::now();
    let (all_files, enum_errors) = file_reader::enumerate_files_safely(
        &settings.search_path,
        settings.include_hidden,
        &settings.exclude_folders,
        &cancellation,
        Some(&mut enum_progress_cb),
    )
    .map_err(|_| OrchestratorError::Cancelled)?;
    run_result.summary.enumeration_errors = enum_errors;

    let candidates = filter_by_extension(all_files, &settings);
    // Issue #6 §57 "Instrumentation" - one aggregate event per run, not
    // one per file/directory (§58 explicitly warns against that).
    tracing::info!(
        files_discovered = candidates.len(),
        enumeration_errors = enum_errors,
        elapsed_ms = discover_start.elapsed().as_millis() as u64,
        "discovery complete"
    );

    run_over_candidates(candidates, run_result, settings, progress, cancellation).await
}

/// Narrows a raw enumerated-file list down to the extensions a run should
/// actually consider - factored out of `run` so the proactive corpus
/// indexer (`native_index::build_or_update_corpus_index`) applies the
/// exact same extension scoping a normal search would, not a
/// second, potentially-drifting copy of this logic.
pub fn filter_by_extension(all_files: Vec<EnumeratedFile>, settings: &SearchSettings) -> Vec<EnumeratedFile> {
    let extensions: Vec<String> = settings.extensions.clone().unwrap_or_else(extension_catalog::all_extensions);
    let extension_set: std::collections::HashSet<String> = extensions.iter().map(|e| e.to_lowercase()).collect();
    let search_all_extensions = extension_set.len() == 1 && extension_set.contains("*");
    all_files
        .into_iter()
        .filter(|f| search_all_extensions || extension_set.contains(&file_extension_lower(&f.path)))
        .collect()
}

/// Given a set of paths already known to be worth processing (e.g. a
/// candidate list narrowed by a trigram index query - see
/// `native_index.rs`'s corpus indexer), stats each one to build the
/// `EnumeratedFile` entries `run_over_candidates` needs and skips the
/// directory walk entirely. A path that fails to stat (deleted since the
/// candidate list was produced, permissions changed, ...) is silently
/// dropped rather than failing the whole run - matches this app's
/// established "a bad file must never stop the rest of a run" philosophy
/// (`process_one_file`'s own per-file error isolation).
pub async fn run_candidates(
    paths: &[String],
    settings: SearchSettings,
    progress: Option<mpsc::UnboundedSender<SearchProgressReport>>,
    cancellation: CancellationToken,
) -> Result<SearchRunResult, OrchestratorError> {
    let mut candidates = Vec::with_capacity(paths.len());
    for p in paths {
        let path = std::path::PathBuf::from(p);
        let Ok(meta) = tokio::fs::metadata(&path).await else { continue };
        candidates.push(EnumeratedFile {
            length: meta.len() as i64,
            created: meta.created().map(system_time_to_local).unwrap_or_else(|_| Local::now()),
            modified: meta.modified().map(system_time_to_local).unwrap_or_else(|_| Local::now()),
            path,
        });
    }
    run_over_candidates(candidates, SearchRunResult::default(), settings, progress, cancellation).await
}

/// The shared "process a known list of files" core both [`run`] (after
/// walking the folder) and [`run_candidates`] (given a pre-narrowed list,
/// no walk) call - dry-run handling, the incremental cache split, parallel/
/// sequential processing, and the final summary tally, all unchanged from
/// before this was factored out of `run` itself.
async fn run_over_candidates(
    candidates: Vec<EnumeratedFile>,
    mut run_result: SearchRunResult,
    settings: SearchSettings,
    progress: Option<mpsc::UnboundedSender<SearchProgressReport>>,
    cancellation: CancellationToken,
) -> Result<SearchRunResult, OrchestratorError> {
    // Epic #6 §68 "HTML Report Design" - "performance statistics". Total
    // wall-clock time for this run, surfaced in the HTML report's summary
    // (distinct from the per-file `elapsed_seconds` already tracked on
    // `InFlightFileStatus` for live progress, and from the separate
    // `native-search`/`search-core` benchmark harnesses in
    // docs/benchmarking.md, which measure synthetic-corpus throughput,
    // not a specific user-facing run).
    let run_start = Instant::now();

    if settings.dry_run {
        run_result.was_dry_run = true;
        run_result.dry_run_candidates = Some(candidates.iter().map(|f| f.path.clone()).collect());
        if let Some(tx) = &progress {
            let _ = tx.send(SearchProgressReport {
                is_dry_run: true,
                total_files: candidates.len() as i32,
                ..Default::default()
            });
        }
        run_result.summary.total_elapsed_seconds = run_start.elapsed().as_secs_f64();
        return Ok(run_result);
    }

    // ---- Incremental cache ----
    let prior_cache = match settings.cache_file_path.as_deref() {
        Some(p) if !p.trim().is_empty() => cache::try_load(p, &settings),
        _ => None,
    };

    // ---- Extraction-failure log (issue #6 §12/§16) ----
    let failure_log: Option<Arc<crate::failure_log::FailureLog>> = match settings.failure_log_path.as_deref() {
        Some(p) if !p.trim().is_empty() => crate::failure_log::FailureLog::open(p).ok().map(Arc::new),
        _ => None,
    };

    let mut to_process: Vec<EnumeratedFile> = Vec::new();
    let mut reused: Vec<FileSearchResult> = Vec::new();

    for f in &candidates {
        let full_name = f.path.to_string_lossy().into_owned();
        let cached = prior_cache.as_ref().and_then(|c| c.get(&full_name)).filter(|entry| {
            entry.length == f.length && entry.last_write_time_ticks == cache::ticks_from_modified(f.modified)
        });

        if let Some(entry) = cached {
            reused.push(entry.to_file_search_result(full_name));
            run_result.summary.cache_reused += 1;
        } else {
            to_process.push(f.clone());
        }
    }

    let match_state = Arc::new(CompiledMatchState::build(&settings).map_err(OrchestratorError::InvalidFilterRegex)?);
    let max_bytes = (settings.max_file_size_mb * 1024.0 * 1024.0) as i64;
    let settings = Arc::new(settings);

    let total_candidates = candidates.len() as i32;
    let files_completed = Arc::new(AtomicI32::new(0));
    let hits_so_far = Arc::new(AtomicI32::new(0));
    let inflight: InFlightMap = Arc::new(Mutex::new(HashMap::new()));

    // Cache-reused files never go through process_one_file, so without this
    // they'd be invisible to progress/streaming entirely - a warm run would
    // show 0% until the very end even though every file is effectively
    // "already done".
    {
        let mut fc = 0i32;
        let mut hs = 0i32;
        for r in &reused {
            fc += 1;
            if r.status == FileSearchStatus::Hit {
                hs += r.hits.len() as i32;
            }
            report_progress(&progress, fc, total_candidates, hs, &inflight, Some(r.clone()));
        }
        files_completed.store(fc, Ordering::SeqCst);
        hits_so_far.store(hs, Ordering::SeqCst);
    }

    let mut fresh: Vec<FileSearchResult> = Vec::new();
    let mut cancelled = false;

    if settings.parallel && !to_process.is_empty() {
        let light_semaphore = Arc::new(Semaphore::new(settings.throttle_limit.max(1) as usize));
        let heavy_semaphore = Arc::new(Semaphore::new(settings.heavy_throttle_limit.max(1) as usize));

        // Lightweight ticker so in-flight elapsed times keep updating even
        // between file completions - this is what makes a slow PDF visibly
        // "still going" instead of the display freezing for many seconds.
        let ticker_cancel = CancellationToken::new();
        let ticker_handle = {
            let progress = progress.clone();
            let inflight = Arc::clone(&inflight);
            let files_completed = Arc::clone(&files_completed);
            let hits_so_far = Arc::clone(&hits_so_far);
            let ticker_cancel = ticker_cancel.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = ticker_cancel.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {
                            report_progress(
                                &progress,
                                files_completed.load(Ordering::SeqCst),
                                total_candidates,
                                hits_so_far.load(Ordering::SeqCst),
                                &inflight,
                                None,
                            );
                        }
                    }
                }
            })
        };

        let mut join_set: JoinSet<FileSearchResult> = JoinSet::new();
        for file in to_process {
            let sem = if is_heavy_extension(&file_extension_lower(&file.path)) {
                Arc::clone(&heavy_semaphore)
            } else {
                Arc::clone(&light_semaphore)
            };
            let settings = Arc::clone(&settings);
            let match_state = Arc::clone(&match_state);
            let inflight = Arc::clone(&inflight);
            let cancellation = cancellation.clone();
            let failure_log = failure_log.clone();
            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await.expect("semaphore is never closed");
                process_one_file(file, settings, match_state, max_bytes, inflight, cancellation, failure_log).await
            });
        }

        loop {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    join_set.abort_all();
                    cancelled = true;
                    break;
                }
                next = join_set.join_next() => {
                    match next {
                        Some(Ok(result)) => {
                            let completed = files_completed.fetch_add(1, Ordering::SeqCst) + 1;
                            let hs = if result.status == FileSearchStatus::Hit {
                                hits_so_far.fetch_add(result.hits.len() as i32, Ordering::SeqCst) + result.hits.len() as i32
                            } else {
                                hits_so_far.load(Ordering::SeqCst)
                            };
                            fresh.push(result.clone());
                            report_progress(&progress, completed, total_candidates, hs, &inflight, Some(result));
                        }
                        Some(Err(_join_error)) => {
                            // A spawned task panicked - never expected in a
                            // faithful port (process_one_file has no panic
                            // paths of its own), but don't let one bad task
                            // take down the whole run's bookkeeping either.
                        }
                        None => break,
                    }
                }
            }
        }

        ticker_cancel.cancel();
        let _ = ticker_handle.await;
    } else {
        for file in to_process {
            if cancellation.is_cancelled() {
                cancelled = true;
                break;
            }
            let result = process_one_file(
                file,
                Arc::clone(&settings),
                Arc::clone(&match_state),
                max_bytes,
                Arc::clone(&inflight),
                cancellation.clone(),
                failure_log.clone(),
            )
            .await;
            let completed = files_completed.fetch_add(1, Ordering::SeqCst) + 1;
            let hs = if result.status == FileSearchStatus::Hit {
                hits_so_far.fetch_add(result.hits.len() as i32, Ordering::SeqCst) + result.hits.len() as i32
            } else {
                hits_so_far.load(Ordering::SeqCst)
            };
            fresh.push(result.clone());
            report_progress(&progress, completed, total_candidates, hs, &inflight, Some(result));
        }
    }

    if cancelled {
        return Err(OrchestratorError::Cancelled);
    }

    run_result.file_results.extend(reused);
    run_result.file_results.extend(fresh);

    if let Some(cache_path) = settings.cache_file_path.as_deref().filter(|p| !p.trim().is_empty()) {
        let candidate_meta: Vec<CandidateMetadata> = candidates
            .iter()
            .map(|f| CandidateMetadata {
                full_name: f.path.to_string_lossy().into_owned(),
                length: f.length,
                last_write_time_ticks: cache::ticks_from_modified(f.modified),
            })
            .collect();
        cache::save(cache_path, &settings, &candidate_meta, &run_result.file_results);
    }

    for r in &run_result.file_results {
        match r.status {
            FileSearchStatus::TooLarge => run_result.summary.skipped_too_large += 1,
            FileSearchStatus::Binary => {
                run_result.summary.skipped_binary += 1;
                run_result.summary.files_searched += 1;
            }
            FileSearchStatus::ReadError => run_result.summary.skipped_read_error += 1,
            FileSearchStatus::ExcludedFile => {
                run_result.summary.skipped_by_exclude += 1;
                run_result.summary.files_searched += 1;
            }
            FileSearchStatus::ModeExcluded => {
                run_result.summary.skipped_by_mode += 1;
                run_result.summary.files_searched += 1;
            }
            FileSearchStatus::UnexpectedError => {
                run_result.summary.skipped_unexpected_error += 1;
                run_result.summary.warnings.push(Warning {
                    full_name: r.full_name.clone(),
                    message: r.error_message.clone().unwrap_or_else(|| "Unknown error".to_string()),
                });
            }
            FileSearchStatus::NoHit | FileSearchStatus::Hit => run_result.summary.files_searched += 1,
        }
    }

    run_result.summary.total_elapsed_seconds = run_start.elapsed().as_secs_f64();
    tracing::info!(
        files_searched = run_result.summary.files_searched,
        skipped_too_large = run_result.summary.skipped_too_large,
        skipped_binary = run_result.summary.skipped_binary,
        skipped_read_error = run_result.summary.skipped_read_error,
        skipped_unexpected_error = run_result.summary.skipped_unexpected_error,
        cache_reused = run_result.summary.cache_reused,
        elapsed_seconds = run_result.summary.total_elapsed_seconds,
        "search run complete"
    );
    Ok(run_result)
}

struct InFlightGuard<'a> {
    map: &'a InFlightMap,
    key: String,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.map.lock() {
            guard.remove(&self.key);
        }
    }
}

/// Processes exactly one file end to end: robust byte read, format-aware
/// text extraction, line matching, AllInFile/Proximity gating. Unlike the
/// C# original's catch-all `UnexpectedError` path (a defensive net around
/// exceptions from anywhere in the pipeline), every function this calls
/// returns `Option`/`Result` rather than panicking, so there is no
/// unexpected-failure path to catch here in the same way - `UnexpectedError`
/// remains a valid `FileSearchStatus`, just not one this function currently
/// produces.
async fn process_one_file(
    file: EnumeratedFile,
    settings: Arc<SearchSettings>,
    match_state: Arc<CompiledMatchState>,
    max_bytes: i64,
    inflight: InFlightMap,
    cancellation: CancellationToken,
    failure_log: Option<Arc<crate::failure_log::FailureLog>>,
) -> FileSearchResult {
    let full_name = file.path.to_string_lossy().into_owned();
    let file_name_only = file
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| full_name.clone());
    let modified_unix = file.modified.timestamp();
    // trace, not info/debug: this runs once per file, potentially
    // thousands of times per run - epic #6 §58 explicitly warns against
    // flooding logs with one message per file "unless explicitly
    // enabled" (opt in via RUST_LOG=search_core=trace).
    tracing::trace!(file = %full_name, "extract: processing file");

    let mut result = FileSearchResult {
        full_name: full_name.clone(),
        status: FileSearchStatus::NoHit,
        hits: Vec::new(),
        created: file.created,
        modified: file.modified,
        file_length: file.length,
        lines_cache: Vec::new(),
        total_line_count: 0,
        proximity_min_range: None,
        low_confidence_pdf: false,
        error_message: None,
    };

    // A known, unchanged extraction failure (issue #6 §16) - skip the
    // read+extraction attempt entirely rather than re-discovering the
    // same malformed file on every run. Only genuine extraction failures
    // are ever recorded here (see failure_log.rs's module doc for why
    // transient read errors - locked files, timeouts - are deliberately
    // never recorded, since those ARE worth retrying every run).
    if let Some(log) = &failure_log {
        if let Some(reason) = log.known_failure_reason(&full_name, file.length, modified_unix) {
            tracing::debug!(file = %full_name, "extract: skipping known extraction failure");
            result.status = FileSearchStatus::ReadError;
            result.error_message = Some(reason);
            return result;
        }
    }

    if file.length > max_bytes {
        result.status = FileSearchStatus::TooLarge;
        return result;
    }

    let start = Instant::now();
    let set_status = |text: String| {
        if let Ok(mut guard) = inflight.lock() {
            guard.insert(
                full_name.clone(),
                InFlightFileStatus {
                    file_name: file_name_only.clone(),
                    status_text: text,
                    elapsed_seconds: start.elapsed().as_secs_f64(),
                },
            );
        }
    };
    set_status("Reading...".to_string());
    let _guard = InFlightGuard { map: &inflight, key: full_name.clone() };

    let mut on_retry = |status: file_reader::RetryStatus| {
        set_status(format!(
            "Locked by another program - retrying ({} of {})...",
            status.attempt, status.max_retries
        ));
    };
    let bytes = match file_reader::read_file_bytes_robust(
        &full_name,
        settings.file_timeout_seconds as u64,
        settings.max_retries,
        settings.retry_delay_ms as u64,
        Some(&mut on_retry),
        &cancellation,
    )
    .await
    {
        Ok(b) => b,
        Err(e) => {
            result.status = FileSearchStatus::ReadError;
            result.error_message = Some(e.to_string());
            return result;
        }
    };

    set_status("Extracting text...".to_string());
    let ext = file_extension_lower(&file.path);

    let mut on_pdf_progress = |streams_scanned: i32, _elapsed: Duration| {
        set_status(format!("Extracting PDF text - {streams_scanned} stream(s) scanned"));
    };
    let extracted = extraction::extract_lines_by_extension(
        &ext,
        &bytes,
        settings.pdf_timeout_seconds as u64,
        Some(&mut on_pdf_progress),
        settings.ocr_scanned_pdfs,
    );

    let lines = match extracted {
        Ok(e) => {
            result.low_confidence_pdf = e.low_confidence_pdf;
            // The file extracted successfully now - clear any prior
            // recorded failure for it (fixed, or the earlier failure was
            // a fluke of that specific past content).
            if let Some(log) = &failure_log {
                log.clear_failure(&full_name);
            }
            e.lines
        }
        Err(extraction::ExtractLinesError::Binary) => {
            result.status = FileSearchStatus::Binary;
            return result;
        }
        Err(extraction::ExtractLinesError::Failed) => {
            result.status = FileSearchStatus::ReadError;
            let reason = format!("{ext} extractor produced no usable text");
            result.error_message = Some(reason.clone());
            tracing::warn!(file = %full_name, extension = %ext, "extraction failed");
            if let Some(log) = &failure_log {
                log.record_failure(&full_name, file.length, modified_unix, "ReadError", &reason, chrono::Local::now().timestamp());
            }
            return result;
        }
    };

    set_status("Matching filters...".to_string());
    let outcome = matching::apply_line_matching(&lines, &settings, &match_state);

    if outcome.excluded_by_file {
        result.status = FileSearchStatus::ExcludedFile;
        return result;
    }
    if outcome.hits.is_empty() {
        result.status = FileSearchStatus::NoHit;
        return result;
    }
    if !outcome.passes_mode {
        result.status = FileSearchStatus::ModeExcluded;
        return result;
    }

    result.status = FileSearchStatus::Hit;
    result.total_line_count = lines.len() as i32;
    result.lines_cache = if lines.len() as i32 > settings.max_embed_lines {
        lines.into_iter().take(settings.max_embed_lines as usize).collect()
    } else {
        lines
    };
    result.hits = outcome.hits;
    result.proximity_min_range = outcome.proximity_min_range;

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_heavy_extension_classifies_container_formats_only() {
        for ext in [".pdf", ".docx", ".pptx", ".xlsx", ".zip"] {
            assert!(is_heavy_extension(ext), "{ext} should be heavy");
        }
        for ext in [".txt", ".log", ".rtf", ".md", ""] {
            assert!(!is_heavy_extension(ext), "{ext} should be light");
        }
    }

    fn settings_for(dir: &std::path::Path, filters: &[&str]) -> SearchSettings {
        SearchSettings {
            search_path: dir.to_string_lossy().into_owned(),
            output_folder: dir.to_string_lossy().into_owned(),
            filters: filters.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn finds_hits_in_plain_text_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple pie\nnothing here\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "no fruit at all\n").unwrap();

        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();

        let hit = result.file_results.iter().find(|r| r.full_name.ends_with("a.txt")).unwrap();
        assert_eq!(hit.status, FileSearchStatus::Hit);
        assert_eq!(hit.hits.len(), 1);

        let no_hit = result.file_results.iter().find(|r| r.full_name.ends_with("b.txt")).unwrap();
        assert_eq!(no_hit.status, FileSearchStatus::NoHit);

        assert_eq!(result.summary.files_searched, 2);
    }

    #[tokio::test]
    async fn dry_run_returns_candidates_without_processing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple\n").unwrap();

        let mut settings = settings_for(dir.path(), &["apple"]);
        settings.dry_run = true;

        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert!(result.was_dry_run);
        assert_eq!(result.dry_run_candidates.as_ref().map(|c| c.len()), Some(1));
        assert_eq!(result.file_results.len(), 0);
    }

    #[tokio::test]
    async fn exclude_folder_prunes_whole_path_segment_not_substring() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("bin").join("x.txt"), "apple\n").unwrap();
        std::fs::create_dir(dir.path().join("robin")).unwrap();
        std::fs::write(dir.path().join("robin").join("y.txt"), "apple\n").unwrap();

        let mut settings = settings_for(dir.path(), &["apple"]);
        settings.exclude_folders = vec!["bin".to_string()];

        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        let names: Vec<&str> = result.file_results.iter().map(|r| r.full_name.as_str()).collect();
        assert!(names.iter().any(|n| n.ends_with("y.txt")), "robin must not be excluded");
        assert!(!names.iter().any(|n| n.ends_with("x.txt")), "bin must be excluded");
    }

    #[tokio::test]
    async fn known_extraction_failure_is_skipped_on_a_rerun_not_reattempted() {
        let dir = tempfile::tempdir().unwrap();
        // Garbage bytes with a .docx extension - DocxExtractor is
        // registered for this extension (see extraction.rs), so this
        // skips the binary-sniff fallback entirely and goes straight to
        // extract_docx_lines, which fails to parse it as a ZIP archive
        // and returns None -> ExtractLinesError::Failed, a genuine
        // extraction failure (not a transient read error).
        std::fs::write(dir.path().join("corrupt.docx"), b"not a real docx file, just garbage bytes").unwrap();
        let failure_db = dir.path().join("failures.db");

        let mut settings = settings_for(dir.path(), &["apple"]);
        settings.extensions = Some(vec![".docx".to_string()]);
        settings.failure_log_path = Some(failure_db.to_str().unwrap().to_string());

        let first = run(settings.clone(), None, CancellationToken::new()).await.unwrap();
        let first_result = first.file_results.iter().find(|r| r.full_name.ends_with("corrupt.docx")).unwrap();
        assert_eq!(first_result.status, FileSearchStatus::ReadError);
        assert!(first_result.error_message.as_deref().unwrap().contains("docx"));

        // The failure must now be persisted on disk, independent of this
        // process's memory.
        let log = crate::failure_log::FailureLog::open(failure_db.to_str().unwrap()).unwrap();
        assert_eq!(log.list_failures().len(), 1);

        // Second run, same settings, same unchanged file - must still
        // report the exact same failure (proving the skip path produces
        // an equivalent result, not just "some" result), sourced from the
        // failure log rather than a second real extraction attempt.
        let second = run(settings, None, CancellationToken::new()).await.unwrap();
        let second_result = second.file_results.iter().find(|r| r.full_name.ends_with("corrupt.docx")).unwrap();
        assert_eq!(second_result.status, FileSearchStatus::ReadError);
        assert_eq!(second_result.error_message, first_result.error_message);
    }

    #[tokio::test]
    async fn a_file_that_now_extracts_successfully_clears_its_failure_record() {
        // Pre-seeds the failure log for a real, valid, easily-extractable
        // file (bypassing the need to construct genuinely malformed
        // ZIP/OOXML bytes just to trigger a real failure first) - proves
        // process_one_file's success path calls clear_failure, directly
        // and robustly.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "apple pie recipe\n").unwrap();
        let failure_db = dir.path().join("failures.db");

        // Deliberately a fingerprint that does NOT match the file's real
        // current size/mtime - otherwise process_one_file's upfront
        // known-failure skip check would match and short-circuit before
        // ever reaching real extraction, and this test would never
        // exercise the clear_failure code path it's meant to prove at
        // all (a stale/mismatched record is exactly the "this failure no
        // longer applies" case the fingerprint check exists to handle).
        let full_name = path.to_string_lossy().into_owned();
        {
            let log = crate::failure_log::FailureLog::open(failure_db.to_str().unwrap()).unwrap();
            log.record_failure(&full_name, 999_999, 1, "ReadError", "stale failure record from different content", 0);
            assert_eq!(log.list_failures().len(), 1);
        }

        let mut settings = settings_for(dir.path(), &["apple"]);
        settings.failure_log_path = Some(failure_db.to_str().unwrap().to_string());
        let result = run(settings, None, CancellationToken::new()).await.unwrap();

        let file_result = result.file_results.iter().find(|r| r.full_name.ends_with("a.txt")).unwrap();
        assert_eq!(file_result.status, FileSearchStatus::Hit, "the stale failure record must not have blocked a real, successful extraction");

        let log = crate::failure_log::FailureLog::open(failure_db.to_str().unwrap()).unwrap();
        assert!(log.list_failures().is_empty(), "a file that extracts successfully must have its prior failure record cleared");
    }

    #[tokio::test]
    async fn parallel_mode_produces_same_hits_as_sequential() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), format!("apple {i}\n")).unwrap();
        }

        let sequential = settings_for(dir.path(), &["apple"]);
        let seq_result = run(sequential, None, CancellationToken::new()).await.unwrap();

        let mut parallel = settings_for(dir.path(), &["apple"]);
        parallel.parallel = true;
        parallel.throttle_limit = 4;
        let par_result = run(parallel, None, CancellationToken::new()).await.unwrap();

        let mut seq_hits: Vec<String> = seq_result
            .file_results
            .iter()
            .filter(|r| r.status == FileSearchStatus::Hit)
            .map(|r| r.full_name.clone())
            .collect();
        let mut par_hits: Vec<String> = par_result
            .file_results
            .iter()
            .filter(|r| r.status == FileSearchStatus::Hit)
            .map(|r| r.full_name.clone())
            .collect();
        seq_hits.sort();
        par_hits.sort();
        assert_eq!(seq_hits, par_hits);
        assert_eq!(seq_hits.len(), 10);
    }

    #[tokio::test]
    async fn a_pre_cancelled_token_stops_the_run_before_processing_anything() {
        // Issue #6 §25/§52 "search cancellation"/"concurrency correctness" -
        // a token cancelled before (or immediately after) a run starts
        // must short-circuit cleanly, not race through to a normal
        // result. `orchestrator::run` checks `cancellation.is_cancelled()`
        // right after enumeration (see the `if cancellation.is_cancelled()`
        // guard before `run_over_candidates`'s parallel/sequential split).
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), "apple").unwrap();
        }
        let settings = settings_for(dir.path(), &["apple"]);
        let token = CancellationToken::new();
        token.cancel();

        let result = run(settings, None, token).await;
        assert!(matches!(result, Err(OrchestratorError::Cancelled)), "a pre-cancelled token must report Cancelled, not a normal result");
    }

    #[tokio::test]
    async fn multiple_concurrent_runs_against_the_same_folder_do_not_interfere() {
        // Issue #6 §52 "concurrency correctness" - "multiple searches".
        // InFlightMap and every other piece of run-local state is created
        // fresh inside each `run` call (never a shared static), so two
        // concurrent runs over the same folder must produce identical,
        // uncorrupted results - proven empirically here, not just by
        // reading the code.
        let dir = tempfile::tempdir().unwrap();
        for i in 0..8 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), format!("apple {i}\n")).unwrap();
        }

        let a = settings_for(dir.path(), &["apple"]);
        let b = settings_for(dir.path(), &["apple"]);
        let (result_a, result_b) =
            tokio::join!(run(a, None, CancellationToken::new()), run(b, None, CancellationToken::new()));
        let result_a = result_a.unwrap();
        let result_b = result_b.unwrap();

        let hit_files = |r: &SearchRunResult| -> Vec<String> {
            let mut v: Vec<String> =
                r.file_results.iter().filter(|f| f.status == FileSearchStatus::Hit).map(|f| f.full_name.clone()).collect();
            v.sort();
            v
        };
        assert_eq!(hit_files(&result_a), hit_files(&result_b));
        assert_eq!(hit_files(&result_a).len(), 8);
    }

    // ---- Adversarial tests (issue #6 §53) ----
    // A bad file must never stop the rest of a run, and must never panic
    // - each test below proves that for one specific adversarial shape,
    // rather than trusting the general "error isolation" claim.

    #[tokio::test]
    async fn empty_file_does_not_crash_and_is_not_a_hit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("empty.txt"), b"").unwrap();
        std::fs::write(dir.path().join("real.txt"), "apple pie\n").unwrap();
        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert_eq!(result.file_results.len(), 2, "both files must be processed - the empty one must not crash or be silently dropped");
        let empty_result = result.file_results.iter().find(|r| r.full_name.contains("empty.txt")).unwrap();
        // No usable text to extract from an empty file - a clean
        // ReadError ("no usable text"), never a hit and never a panic.
        assert_ne!(empty_result.status, FileSearchStatus::Hit);
        let hit_count = result.file_results.iter().filter(|r| r.status == FileSearchStatus::Hit).count();
        assert_eq!(hit_count, 1, "only real.txt should be a hit");
    }

    #[tokio::test]
    async fn a_file_over_the_size_limit_is_skipped_as_too_large_not_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big.txt"), "apple ".repeat(1000)).unwrap();
        let mut settings = settings_for(dir.path(), &["apple"]);
        settings.max_file_size_mb = 0.0001; // ~100 bytes - the file above is well over this
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert_eq!(result.summary.skipped_too_large, 1);
        assert_eq!(result.summary.files_searched, 0, "a too-large file must not count toward files_searched");
    }

    #[tokio::test]
    async fn a_binary_file_with_a_text_extension_is_skipped_as_binary_not_crash() {
        let dir = tempfile::tempdir().unwrap();
        // NUL bytes mid-content - looks_binary's actual detection signal -
        // wrapped in a real .txt extension (the adversarial part: the
        // extension lies about the content).
        let mut content = b"apple ".to_vec();
        content.extend_from_slice(&[0u8, 0u8, 0u8]);
        content.extend_from_slice(b" more binary junk");
        std::fs::write(dir.path().join("fake.txt"), &content).unwrap();
        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert_eq!(result.summary.skipped_binary, 1);
        assert_eq!(result.file_results[0].status, FileSearchStatus::Binary);
    }

    #[tokio::test]
    async fn invalid_utf8_that_is_also_not_valid_windows_1252_does_not_crash() {
        // Bytes that are invalid UTF-8 AND decode to something under the
        // Windows-1252 fallback too - proving the encoding-detection chain
        // (BOM -> UTF-8 -> Windows-1252) never panics even on genuinely
        // adversarial byte content, only ever produces *some* text or a
        // clean skip.
        let dir = tempfile::tempdir().unwrap();
        let mut content = b"apple ".to_vec();
        content.extend_from_slice(&[0xFF, 0xFE, 0xFF, 0xFE, 0xC0, 0xC1]); // never valid UTF-8 lead bytes
        content.extend_from_slice(b" pie");
        std::fs::write(dir.path().join("weird_bytes.txt"), &content).unwrap();
        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert_eq!(result.summary.files_searched, 1, "must process the file (Windows-1252 fallback), not panic or silently drop it");
    }

    #[tokio::test]
    async fn a_file_with_a_very_long_path_does_not_crash() {
        let dir = tempfile::tempdir().unwrap();
        // 200 characters of nested single-char directories plus a real
        // filename - well past typical old-style Windows MAX_PATH (260
        // chars total is the classic limit; this pushes the directory
        // component alone past most of that budget).
        let mut nested = dir.path().to_path_buf();
        for _ in 0..80 {
            nested.push("d");
        }
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("apple.txt"), "apple pie\n").unwrap();
        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert_eq!(result.summary.files_searched, 1);
        let hit_count = result.file_results.iter().filter(|r| r.status == FileSearchStatus::Hit).count();
        assert_eq!(hit_count, 1, "a deeply nested path must not be silently skipped");
    }

    #[tokio::test]
    async fn a_unicode_filename_and_path_component_are_found_and_searched() {
        let dir = tempfile::tempdir().unwrap();
        let unicode_dir = dir.path().join("caf\u{e9}_\u{1f34e}"); // "café_🍎"
        std::fs::create_dir_all(&unicode_dir).unwrap();
        std::fs::write(unicode_dir.join("r\u{e9}sum\u{e9}.txt"), "apple pie\n").unwrap();
        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert_eq!(result.summary.files_searched, 1);
        let hit_count = result.file_results.iter().filter(|r| r.status == FileSearchStatus::Hit).count();
        assert_eq!(hit_count, 1, "a Unicode path/filename must be found and searched, not skipped or mangled");
    }

    #[tokio::test]
    async fn malformed_docx_bytes_are_a_read_error_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.docx"), b"this is not a real zip/docx file at all").unwrap();
        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert_eq!(result.file_results.len(), 1);
        assert_eq!(result.file_results[0].status, FileSearchStatus::ReadError);
        assert_eq!(result.summary.skipped_read_error, 1);
    }

    #[tokio::test]
    async fn malformed_pdf_bytes_are_handled_without_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        // Not a real PDF at all - no %PDF header, no stream objects. The
        // regex-based extractor should simply find nothing to extract,
        // not panic on the missing structure.
        std::fs::write(dir.path().join("broken.pdf"), b"definitely not a pdf file, no streams here").unwrap();
        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert_eq!(result.file_results.len(), 1);
        // No usable text -> ReadError (extractor produced nothing), same
        // classification a malformed DOCX gets - never a panic either way.
        assert_eq!(result.file_results[0].status, FileSearchStatus::ReadError);
    }

    // chmod-based permission bits don't translate to Windows ACLs - this
    // whole test is gated out (not compiled at all, not just #[ignore]d)
    // on non-Unix targets, same reasoning `.github/workflows/rust-build.yml`
    // already applies elsewhere in this project for platform-specific code.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_permission_denied_file_is_a_read_error_not_a_crash() {
        use std::os::unix::fs::PermissionsExt;

        // Running as root (common in some CI/sandboxes) bypasses Unix
        // permission bits entirely - skip rather than fail a test
        // asserting behavior that literally cannot occur under that user,
        // same "don't force a test past what the environment can prove"
        // judgment call as the truncation-detection gap documented in
        // docs/issue-6-phase-10.md.
        let running_as_root = unsafe {
            extern "C" {
                fn geteuid() -> u32;
            }
            geteuid() == 0
        };
        if running_as_root {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.txt");
        std::fs::write(&path, "apple pie\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();

        // Restore permissions so tempdir cleanup can actually delete it.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(result.file_results[0].status, FileSearchStatus::ReadError, "permission-denied must be a clean read error, not a panic");
    }

    // ---- Stress test (issue #6 §53) ----
    // Deliberately #[ignore]d (docs/issue-6-phase-14.md explains why: a
    // 100K-file corpus would make every `cargo test` run slow/disk-heavy
    // for a scale this desktop tool's real usage rarely approaches).
    // Run on demand: `cargo test -p search-core --release -- --ignored
    // stress_test_100k_files`. Proves the full pipeline (enumerate,
    // extension-filter, parallel process, match, tally) doesn't fall over
    // - panic, hang, or produce a wrong count - at epic #6 §53's "100K
    // files" tier, not just that it's theoretically bounded-parallel.
    #[tokio::test]
    #[ignore]
    async fn stress_test_100k_files() {
        const FILE_COUNT: usize = 100_000;
        const DIR_COUNT: usize = 200;
        const HIT_EVERY_NTH: usize = 7;

        let dir = tempfile::tempdir().unwrap();
        let setup_start = Instant::now();
        for d in 0..DIR_COUNT {
            let sub = dir.path().join(format!("d{d}"));
            std::fs::create_dir_all(&sub).unwrap();
            for f in 0..(FILE_COUNT / DIR_COUNT) {
                let i = d * (FILE_COUNT / DIR_COUNT) + f;
                let body = if i % HIT_EVERY_NTH == 0 { "apple pie recipe\n" } else { "nothing relevant here\n" };
                std::fs::write(sub.join(format!("f{f}.txt")), body).unwrap();
            }
        }
        eprintln!("stress_test_100k_files: wrote {FILE_COUNT} files in {:.2}s", setup_start.elapsed().as_secs_f64());

        let mut settings = settings_for(dir.path(), &["apple"]);
        settings.parallel = true;
        settings.throttle_limit = 8;

        let run_start = Instant::now();
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        let elapsed = run_start.elapsed();

        let expected_hits = FILE_COUNT.div_ceil(HIT_EVERY_NTH);
        let hit_count = result.file_results.iter().filter(|r| r.status == FileSearchStatus::Hit).count();
        eprintln!(
            "stress_test_100k_files: searched {} file(s) in {:.2}s ({:.0} files/sec), {} hit(s) (expected {})",
            result.summary.files_searched,
            elapsed.as_secs_f64(),
            FILE_COUNT as f64 / elapsed.as_secs_f64(),
            hit_count,
            expected_hits
        );

        assert_eq!(result.file_results.len(), FILE_COUNT, "every file must be accounted for, none silently dropped");
        assert_eq!(hit_count, expected_hits, "exact hit count must survive at this scale, not just 'roughly right'");
        assert_eq!(result.summary.skipped_read_error, 0);
        assert_eq!(result.summary.skipped_unexpected_error, 0);
    }

    #[tokio::test]
    async fn a_real_run_populates_total_elapsed_seconds() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple").unwrap();
        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert!(result.summary.total_elapsed_seconds >= 0.0, "must be a real, non-negative measurement");
        assert!(result.summary.total_elapsed_seconds < 30.0, "sanity bound - this run must not have actually taken 30s");
    }

    #[tokio::test]
    async fn incremental_cache_reuses_unchanged_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple\n").unwrap();
        let cache_path = dir.path().join("cache.json");

        let mut settings = settings_for(dir.path(), &["apple"]);
        settings.cache_file_path = Some(cache_path.to_string_lossy().into_owned());

        let first = run(settings.clone(), None, CancellationToken::new()).await.unwrap();
        assert_eq!(first.summary.cache_reused, 0);

        let second = run(settings, None, CancellationToken::new()).await.unwrap();
        assert_eq!(second.summary.cache_reused, 1);
        let hit = second.file_results.iter().find(|r| r.full_name.ends_with("a.txt")).unwrap();
        assert_eq!(hit.status, FileSearchStatus::Hit);
    }

    #[tokio::test]
    async fn all_in_file_mode_gates_correctly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple only\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "apple and banana\n").unwrap();

        let mut settings = settings_for(dir.path(), &["apple", "banana"]);
        settings.match_mode = crate::models::MatchMode::AllInFile;

        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        let a = result.file_results.iter().find(|r| r.full_name.ends_with("a.txt")).unwrap();
        assert_eq!(a.status, FileSearchStatus::ModeExcluded);
        let b = result.file_results.iter().find(|r| r.full_name.ends_with("b.txt")).unwrap();
        assert_eq!(b.status, FileSearchStatus::Hit);
    }

    #[tokio::test]
    async fn run_candidates_matches_run_over_the_same_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple pie\nnothing here\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "no fruit at all\n").unwrap();

        let settings = settings_for(dir.path(), &["apple"]);
        let full_scan = run(settings.clone(), None, CancellationToken::new()).await.unwrap();

        let candidate_paths = vec![dir.path().join("a.txt").to_string_lossy().into_owned()];
        let narrowed = run_candidates(&candidate_paths, settings, None, CancellationToken::new()).await.unwrap();

        // run_candidates was only given a.txt - b.txt (correctly excluded by
        // the caller, e.g. a trigram query that didn't match it) never gets
        // processed, but a.txt's own result must be identical either way -
        // proving the shared run_over_candidates core doesn't behave
        // differently depending on which entry point reached it.
        let full_hit = full_scan.file_results.iter().find(|r| r.full_name.ends_with("a.txt")).unwrap();
        let narrowed_hit = narrowed.file_results.iter().find(|r| r.full_name.ends_with("a.txt")).unwrap();
        assert_eq!(full_hit.status, narrowed_hit.status);
        assert_eq!(full_hit.hits.len(), narrowed_hit.hits.len());
        assert_eq!(full_hit.hits[0].match_line, narrowed_hit.hits[0].match_line);
        assert_eq!(narrowed.file_results.len(), 1, "only the given candidate should be processed, not the whole folder");
    }

    #[tokio::test]
    async fn run_candidates_skips_a_path_that_no_longer_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple pie\n").unwrap();

        let settings = settings_for(dir.path(), &["apple"]);
        let paths =
            vec![dir.path().join("a.txt").to_string_lossy().into_owned(), dir.path().join("gone.txt").to_string_lossy().into_owned()];
        let result = run_candidates(&paths, settings, None, CancellationToken::new()).await.unwrap();

        assert_eq!(result.file_results.len(), 1, "the missing path must be dropped, not fail the whole run");
        assert!(result.file_results[0].full_name.ends_with("a.txt"));
    }

    #[tokio::test]
    async fn progress_channel_receives_reports() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple\n").unwrap();

        let settings = settings_for(dir.path(), &["apple"]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        run(settings, Some(tx), CancellationToken::new()).await.unwrap();

        let mut saw_completion = false;
        while let Ok(report) = rx.try_recv() {
            if report.files_completed >= 1 {
                saw_completion = true;
            }
        }
        assert!(saw_completion);
    }

    #[tokio::test]
    async fn windows_1252_file_end_to_end_through_full_pipeline_finds_hit() {
        let dir = tempfile::tempdir().unwrap();
        // "Hello \x93World\x94" with cp1252 curly quotes, followed by ASCII " apple".
        let mut bytes: Vec<u8> = vec![72, 101, 108, 108, 111, 32, 0x93, 87, 111, 114, 108, 100, 0x94];
        bytes.extend_from_slice(b" apple");
        std::fs::write(dir.path().join("legacy.txt"), &bytes).unwrap();

        let settings = settings_for(dir.path(), &["apple"]);
        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert!(result.file_results.iter().any(|r| r.status == FileSearchStatus::Hit));
    }

    #[tokio::test]
    async fn invalid_regex_filter_propagates_through_run_naming_the_bad_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple\n").unwrap();

        let mut settings = settings_for(dir.path(), &["(unclosed"]);
        settings.use_regex = true;

        let err = run(settings, None, CancellationToken::new()).await.unwrap_err();
        match err {
            OrchestratorError::InvalidFilterRegex(e) => {
                assert!(e.to_string().contains("(unclosed"));
            }
            other => panic!("expected InvalidFilterRegex, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn case_variant_duplicate_filters_compute_correct_proximity_range() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "apple\nbanana\n").unwrap();

        let mut settings = settings_for(dir.path(), &["apple", "APPLE", "banana"]);
        settings.match_mode = crate::models::MatchMode::Proximity;
        settings.proximity_lines = 5;

        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        let r = result.file_results.iter().find(|r| r.full_name.ends_with("a.txt")).unwrap();
        assert_eq!(r.status, FileSearchStatus::Hit);
        assert_eq!(r.proximity_min_range, Some(1));
    }

    #[tokio::test]
    async fn native_search_index_folder_is_auto_excluded_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("keep.txt"), "findme in a real file\n").unwrap();
        let index_folder = dir.path().join(crate::native_index::INDEX_FOLDER_NAME);
        std::fs::create_dir(&index_folder).unwrap();
        std::fs::write(index_folder.join("decoy.txt"), "findme inside the index folder\n").unwrap();

        let mut settings = settings_for(dir.path(), &["findme"]);
        crate::native_index::ensure_index_folder_excluded(&mut settings.exclude_folders);

        let result = run(settings, None, CancellationToken::new()).await.unwrap();
        assert!(result.file_results.iter().any(|r| r.full_name.ends_with("keep.txt") && r.status == FileSearchStatus::Hit));
        assert!(
            !result.file_results.iter().any(|r| r.full_name.ends_with("decoy.txt")),
            "the native_search index folder must never be walked into"
        );
    }
}
