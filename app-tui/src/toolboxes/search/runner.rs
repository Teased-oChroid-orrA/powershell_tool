//! The async background work for the Search Files toolbox: running a
//! (possibly multi-root) search and writing its report(s). Lives in the
//! library (not `main.rs`) specifically so it's exercisable by a real
//! `#[tokio::test]` integration test against a tempdir fixture, not just
//! manually - `main.rs` stays a thin caller (see `lib.rs`'s module doc on
//! why the crate is split this way).

use std::path::{Path, PathBuf};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use search_core::models::{SearchRunResult, SearchSettings};
use search_core::orchestrator::OrchestratorError;

use crate::app::AppEvent;
use crate::debug_log::log;

use super::indexing::{self, IndexSettings};

/// Runs a (possibly multi-root) search one root at a time, so a slow or
/// huge first root never starves progress reporting for the rest. Mirrors
/// `app/`'s `run_search` orchestration: a raw `tokio::spawn` runs the
/// actual `orchestrator::run`/`run_candidates` call and only ever writes
/// into its own progress channel, never touching shared state directly;
/// THIS function is the one that drains that channel and forwards each
/// report into the app-wide `AppEvent` channel as `AppEvent::SearchProgress`.
///
/// When `index.enabled` and a fast re-search index already exists for a
/// root, the file list is narrowed via a trigram query first
/// (`orchestrator::run_candidates` instead of the full-scan
/// `orchestrator::run`) - a safe superset per `indexing::trigram_candidates`'s
/// own contract; `None` (no index yet, or no safe narrowing possible) always
/// falls back to a full scan, never treated as an error.
pub async fn run_search(
    tx: mpsc::UnboundedSender<AppEvent>,
    cancellation: CancellationToken,
    roots: Vec<String>,
    base_settings: SearchSettings,
    index: IndexSettings,
) {
    log("SEARCH", format!("run start: roots={} filters={} regex={} use_index={}", roots.len(), base_settings.filters.len(), base_settings.use_regex, index.enabled));
    let mut accumulated = SearchRunResult::default();
    let mut cancelled = false;

    for root in roots {
        if cancellation.is_cancelled() {
            cancelled = true;
            break;
        }

        let mut root_settings = base_settings.clone();
        root_settings.search_path = root.clone();

        let candidates = narrow_via_index(&tx, index, &root, &root_settings, &cancellation).await;

        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();
        let run_cancellation = cancellation.clone();
        let handle = tokio::spawn(async move {
            match candidates {
                Some(paths) => search_core::orchestrator::run_candidates(&paths, root_settings, Some(progress_tx), run_cancellation).await,
                None => search_core::orchestrator::run(root_settings, Some(progress_tx), run_cancellation).await,
            }
        });

        while let Some(report) = progress_rx.recv().await {
            if tx.send(AppEvent::SearchProgress(report)).is_err() {
                return;
            }
        }

        match handle.await {
            Ok(Ok(run_result)) => {
                super::model::merge_run_result(&mut accumulated, run_result);
            }
            Ok(Err(OrchestratorError::Cancelled)) => {
                cancelled = true;
                break;
            }
            Ok(Err(err)) => {
                let _ = tx.send(AppEvent::SearchFinished(Err(err)));
                return;
            }
            // The spawned orchestrator task panicked or was aborted - treat
            // it the same as a cancellation rather than silently hanging
            // the UI in "running" forever.
            Err(_join_error) => {
                let _ = tx.send(AppEvent::SearchFinished(Err(OrchestratorError::Cancelled)));
                return;
            }
        }
    }

    let outcome = if cancelled { Err(OrchestratorError::Cancelled) } else { Ok(accumulated) };
    let _ = tx.send(AppEvent::SearchFinished(outcome));
}

/// Makes the fast re-search index usable for `root`, then asks
/// `search_core::native_index::narrow_candidates` for a candidate list that
/// is guaranteed not to hide a real match (regex-aware, intersected with the
/// current scope, always including files added/changed since the last
/// build).
///
/// * Index missing, empty or unreadable (e.g. older schema): it is built
///   first, streaming the same progress events as a manual build. A cancelled
///   run cancels that build too.
/// * Index present: used as-is. If the folder changed since the build, an
///   `AppEvent::IndexStale` asks the UI to offer an update once the search
///   finishes - the search itself never waits for it and is still exact.
///
/// Reports how the index was used via `AppEvent::IndexNarrowed`. Any failure
/// degrades to a full scan, never to a wrong or empty result.
async fn narrow_via_index(
    tx: &mpsc::UnboundedSender<AppEvent>,
    index: IndexSettings,
    root: &str,
    settings: &SearchSettings,
    cancellation: &CancellationToken,
) -> Option<Vec<String>> {
    if !index.enabled {
        return None;
    }
    let note = |text: String| {
        // The on-screen note may name the index folder; the log gets counts only.
        log("SEARCH", format!("index: {}", text.split(" [").next().unwrap_or("")));
        let _ = tx.send(AppEvent::IndexNarrowed(text));
    };

    let index_dir = indexing::index_directory(index.location, root, &settings.output_folder);
    let mut just_built = false;
    let mut engine = if indexing::index_is_built(&index_dir) { open_engine(&index_dir).await.ok() } else { None };
    if engine.is_none() {
        log("SEARCH", "index missing or unreadable - building it before searching");
        let built = indexing::build_or_rebuild_index(tx.clone(), settings.clone(), index_dir.clone(), cancellation.clone()).await;
        if cancellation.is_cancelled() {
            return None;
        }
        if !built {
            note(format!("Index could not be built - full scan (details in the debug log) [{}]", index_dir.display()));
            return None;
        }
        just_built = true;
        engine = open_engine(&index_dir).await.ok();
    }
    let Some(engine) = engine else {
        note(format!("Index unreadable - full scan; run Build / update from the command palette (Ctrl+P) [{}]", index_dir.display()));
        return None;
    };

    match search_core::native_index::narrow_candidates(settings, &engine, cancellation).await {
        Ok(out) => {
            if out.is_stale() && !just_built {
                let _ = tx.send(AppEvent::IndexStale { new_or_changed: out.needs_update, removed: out.removed });
            }
            match &out.candidates {
                Some(c) => note(format!(
                    "Index narrowed {} → {} file(s) ({} from index, {} new/changed since build, {} docs indexed)",
                    out.scannable,
                    c.len(),
                    out.from_index,
                    out.stale_or_new,
                    out.index_docs
                )),
                None => note("Index cannot narrow this query (short/regex filter) - full scan".to_string()),
            }
            out.candidates
        }
        Err(e) => {
            log("ERROR", format!("search: narrowing failed: {e:?}"));
            note(format!("Index query failed - full scan ({e})"));
            None
        }
    }
}

async fn open_engine(index_dir: &Path) -> Result<native_search::engine::NativeSearchEngine, ()> {
    let dir = index_dir.to_path_buf();
    match tokio::task::spawn_blocking(move || native_search::engine::NativeSearchEngine::open_or_create(&dir)).await {
        Ok(Ok(engine)) => Ok(engine),
        Ok(Err(e)) => {
            log("ERROR", format!("search: cannot open index: {e:?}"));
            Err(())
        }
        Err(e) => {
            log("ERROR", format!("search: index open task failed: {e}"));
            Err(())
        }
    }
}

/// Writes the HTML/CSV/JSON export(s) for a finished run, then reports the
/// HTML report's path back so the UI can offer "Open report". Meant to run
/// on a blocking-pool thread (`tokio::task::spawn_blocking`) since these
/// are synchronous filesystem calls - see the plan's Performance section
/// on never blocking the event-loop task.
pub fn write_report(
    tx: &mpsc::UnboundedSender<AppEvent>,
    settings: SearchSettings,
    run_result: SearchRunResult,
    write_html: bool,
) {
    let mut report_path = None;

    if write_html {
        let path = default_export_path(&settings, "html");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if search_core::report::write_html_report(&path.to_string_lossy(), &settings, &run_result).is_ok() {
            report_path = Some(path.to_string_lossy().into_owned());
        }
    }

    if settings.export_csv || settings.export_json {
        let rows = search_core::report::build_export_rows(&run_result);
        if settings.export_csv {
            let _ = search_core::report::write_csv(&default_export_path(&settings, "csv").to_string_lossy(), &rows);
        }
        if settings.export_json {
            let _ = search_core::report::write_json(&default_export_path(&settings, "json").to_string_lossy(), &rows);
        }
    }

    let _ = tx.send(AppEvent::ReportWritten(report_path));
}

fn default_export_path(settings: &SearchSettings, extension: &str) -> PathBuf {
    let stem = settings.output_name.clone().unwrap_or_else(|| "search-report".to_string());
    Path::new(&settings.output_folder).join(format!("{stem}.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    use crate::toolboxes::search::model::{build_settings, SearchToolConfig};

    fn drain_progress(rx: &mut mpsc::UnboundedReceiver<AppEvent>) -> Vec<AppEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn a_real_search_over_a_tempdir_finds_hits_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello world\nneedle here\n").unwrap();
        fs::write(dir.path().join("b.txt"), "nothing interesting\n").unwrap();

        let mut config = SearchToolConfig { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        config.filters_text = "needle".to_string();
        let settings = build_settings(&config);

        let (tx, mut rx) = mpsc::unbounded_channel();
        run_search(tx, CancellationToken::new(), vec![config.search_path.clone()], settings, IndexSettings::default()).await;

        let events = drain_progress(&mut rx);
        assert!(events.iter().any(|e| matches!(e, AppEvent::SearchProgress(_))), "expected at least one progress event");
        let finished = events.iter().find_map(|e| match e {
            AppEvent::SearchFinished(result) => Some(result),
            _ => None,
        });
        let Some(Ok(run_result)) = finished else { panic!("expected a successful SearchFinished event") };
        assert_eq!(run_result.file_results.iter().filter(|f| f.status == search_core::models::FileSearchStatus::Hit).count(), 1);
    }

    /// End to end through the real app path: build the index, then add a new
    /// file and change none of the others - an index-enabled search must
    /// still find the new file (a stale index must never hide it) and must
    /// report that it narrowed.
    #[tokio::test]
    async fn an_index_enabled_search_still_finds_files_added_after_the_build() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello world\nneedle here\n").unwrap();
        fs::write(dir.path().join("b.txt"), "nothing interesting\n").unwrap();

        let mut config = SearchToolConfig { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        config.filters_text = "needle".to_string();
        config.index.enabled = true;
        let settings = build_settings(&config);

        let index_dir = indexing::index_directory(config.index.location, &config.search_path, "");
        let (itx, mut irx) = mpsc::unbounded_channel();
        indexing::build_or_rebuild_index(itx, settings.clone(), index_dir, CancellationToken::new()).await;
        let built = drain_progress(&mut irx).into_iter().find_map(|e| match e {
            AppEvent::IndexBuildFinished(r) => Some(r),
            _ => None,
        });
        built.expect("finished event").expect("build ok");

        fs::write(dir.path().join("c.txt"), "a brand new needle file\n").unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        run_search(tx, CancellationToken::new(), vec![config.search_path.clone()], settings, config.index).await;
        let events = drain_progress(&mut rx);
        let note = events.iter().find_map(|e| match e {
            AppEvent::IndexNarrowed(n) => Some(n.clone()),
            _ => None,
        });
        assert!(note.expect("narrowing note").contains("narrowed"), "index must have been used");
        let Some(AppEvent::SearchFinished(Ok(run_result))) = events.iter().find(|e| matches!(e, AppEvent::SearchFinished(_))) else { panic!("expected success") };
        let hits: Vec<_> = run_result.file_results.iter().filter(|f| f.status == search_core::models::FileSearchStatus::Hit).map(|f| f.full_name.clone()).collect();
        assert_eq!(hits.len(), 2, "a.txt (indexed) and c.txt (added after the build): {hits:?}");
        assert!(hits.iter().any(|h| h.ends_with("c.txt")));
        assert!(
            events.iter().any(|e| matches!(e, AppEvent::IndexStale { new_or_changed: 1, removed: 0 })),
            "the added file must make the index report itself stale"
        );
    }

    /// Filter groups end to end through the real runner: primary + a proximity
    /// group + a file-scope exclusion, with the index enabled.
    #[tokio::test]
    async fn filter_groups_with_an_exclusion_work_through_the_real_search_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "a needle line\nzxq marker\n").unwrap();
        fs::write(dir.path().join("excluded.txt"), "a needle line\nzxq marker\nadded later\n").unwrap();
        fs::write(dir.path().join("nogroup.txt"), "a needle line\n").unwrap();
        for index_enabled in [false, true] {
            let mut config = SearchToolConfig { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
            config.filters_text = "needle ; zxq [near 2] ; added [not]".to_string();
            config.index.enabled = index_enabled;
            let settings = build_settings(&config);
            let (tx, mut rx) = mpsc::unbounded_channel();
            run_search(tx, CancellationToken::new(), vec![config.search_path.clone()], settings, config.index).await;
            let events = drain_progress(&mut rx);
            let Some(AppEvent::SearchFinished(Ok(run_result))) = events.iter().find(|e| matches!(e, AppEvent::SearchFinished(_))) else { panic!("expected success") };
            let hits: Vec<_> = run_result.file_results.iter().filter(|f| f.status == search_core::models::FileSearchStatus::Hit).map(|f| f.full_name.clone()).collect();
            assert_eq!(hits.len(), 1, "index_enabled={index_enabled}: {hits:?}");
            assert!(hits[0].ends_with("keep.txt"));
        }
    }

    #[tokio::test]
    async fn an_up_to_date_index_is_used_without_a_stale_prompt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "needle here\n").unwrap();
        let mut config = SearchToolConfig { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        config.filters_text = "needle".to_string();
        config.index.enabled = true;
        let settings = build_settings(&config);
        let index_dir = indexing::index_directory(config.index.location, &config.search_path, "");
        let (itx, _irx) = mpsc::unbounded_channel();
        assert!(indexing::build_or_rebuild_index(itx, settings.clone(), index_dir, CancellationToken::new()).await);

        let (tx, mut rx) = mpsc::unbounded_channel();
        run_search(tx, CancellationToken::new(), vec![config.search_path.clone()], settings, config.index).await;
        let events = drain_progress(&mut rx);
        assert!(events.iter().any(|e| matches!(e, AppEvent::IndexNarrowed(n) if n.contains("narrowed"))));
        assert!(!events.iter().any(|e| matches!(e, AppEvent::IndexStale { .. })), "nothing changed since the build");
        assert!(!events.iter().any(|e| matches!(e, AppEvent::IndexBuildProgress(_))), "an existing index must not be rebuilt");
    }

    #[tokio::test]
    async fn an_index_enabled_search_without_a_built_index_builds_it_then_uses_it() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
        fs::write(dir.path().join("b.txt"), "other\n").unwrap();
        let mut config = SearchToolConfig { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        config.filters_text = "needle".to_string();
        config.index.enabled = true;
        let settings = build_settings(&config);
        let index_dir = indexing::index_directory(config.index.location, &config.search_path, "");
        assert!(!index_dir.exists());

        let (tx, mut rx) = mpsc::unbounded_channel();
        run_search(tx, CancellationToken::new(), vec![config.search_path.clone()], settings, config.index).await;
        let events = drain_progress(&mut rx);
        assert!(indexing::index_is_built(&index_dir), "the index must have been generated");
        assert!(events.iter().any(|e| matches!(e, AppEvent::IndexBuildProgress(_))));
        assert!(events.iter().any(|e| matches!(e, AppEvent::IndexBuildFinished(Ok(_)))));
        assert!(events.iter().any(|e| matches!(e, AppEvent::IndexNarrowed(n) if n.contains("narrowed 2 → 1"))), "{:?}", events.iter().filter_map(|e| match e { AppEvent::IndexNarrowed(n) => Some(n.clone()), _ => None }).collect::<Vec<_>>());
        assert!(!events.iter().any(|e| matches!(e, AppEvent::IndexStale { .. })), "a just-built index is not stale");
        assert!(matches!(events.last(), Some(AppEvent::SearchFinished(Ok(_)))));
    }

    #[tokio::test]
    async fn an_empty_index_folder_counts_as_not_built_and_is_generated() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
        let mut config = SearchToolConfig { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        config.filters_text = "needle".to_string();
        config.index.enabled = true;
        let index_dir = indexing::index_directory(config.index.location, &config.search_path, "");
        fs::create_dir_all(&index_dir).unwrap();
        let settings = build_settings(&config);
        let (tx, mut rx) = mpsc::unbounded_channel();
        run_search(tx, CancellationToken::new(), vec![config.search_path.clone()], settings, config.index).await;
        drain_progress(&mut rx);
        assert!(indexing::index_is_built(&index_dir));
    }

    #[tokio::test]
    async fn cancelling_before_the_run_starts_reports_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello\n").unwrap();

        let config = SearchToolConfig { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        let settings = build_settings(&config);

        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let (tx, mut rx) = mpsc::unbounded_channel();
        run_search(tx, cancellation, vec![config.search_path.clone()], settings, IndexSettings::default()).await;

        let events = drain_progress(&mut rx);
        assert!(matches!(events.last(), Some(AppEvent::SearchFinished(Err(OrchestratorError::Cancelled)))));
    }

    #[tokio::test]
    async fn cancelling_mid_run_stops_promptly_and_reports_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            fs::write(dir.path().join(format!("file{i}.txt")), "hello world\n".repeat(50)).unwrap();
        }

        let config = SearchToolConfig { search_path: dir.path().to_string_lossy().into_owned(), parallel: true, ..Default::default() };
        let settings = build_settings(&config);

        let cancellation = CancellationToken::new();
        let cancel_clone = cancellation.clone();
        let (tx, mut rx) = mpsc::unbounded_channel();

        let started = std::time::Instant::now();
        let handle = tokio::spawn(run_search(tx, cancellation, vec![config.search_path.clone()], settings, IndexSettings::default()));
        // Cancel once the run has genuinely started, not after a guessed
        // wall-clock delay - a fixed `sleep` here raced against real work
        // and intermittently lost: with a tiny fixture (20 small files),
        // the whole run could finish before (or right around) a 1ms sleep
        // elapsed, especially under a loaded/parallel `cargo test`, so the
        // run already reported success by the time cancellation was applied.
        // `orchestrator::run` unconditionally sends an `is_enumerating`
        // progress report before any file work starts (see
        // `search-core/src/orchestrator.rs::run`), so waiting for the
        // first `SearchProgress` event is a real, always-available signal
        // that the run is in flight - deterministic, no risk of hanging
        // (that event is sent before enumeration even begins) and no
        // arbitrary duration to tune.
        let saw_progress = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = rx.recv().await {
                if matches!(event, AppEvent::SearchProgress(_)) {
                    return true;
                }
                if matches!(event, AppEvent::SearchFinished(_)) {
                    return false;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(saw_progress, "expected at least one progress event before the run finished");
        cancel_clone.cancel();
        handle.await.unwrap();

        assert!(started.elapsed() < Duration::from_secs(5), "cancellation should stop the run promptly, not hang");
        let events = drain_progress(&mut rx);
        assert!(matches!(events.last(), Some(AppEvent::SearchFinished(Err(OrchestratorError::Cancelled)))));
    }
}
