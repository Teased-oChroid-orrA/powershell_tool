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
    let mut accumulated = SearchRunResult::default();
    let mut cancelled = false;

    for root in roots {
        if cancellation.is_cancelled() {
            cancelled = true;
            break;
        }

        let mut root_settings = base_settings.clone();
        root_settings.search_path = root.clone();

        let candidates = narrow_via_index(index, &root, &base_settings.output_folder, &root_settings.filters).await;

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

/// Opens the fast re-search index for `root` (if one already exists - a
/// missing index is "not available yet", not an error) and asks it for a
/// safe-superset candidate list. Runs on a blocking-pool thread since
/// opening a Tantivy index is a blocking filesystem operation, mirroring
/// `app/`'s own "wrap Tantivy opens in `spawn_blocking`" rule.
async fn narrow_via_index(
    index: IndexSettings,
    root: &str,
    output_folder: &str,
    filters: &[String],
) -> Option<Vec<String>> {
    if !index.enabled {
        return None;
    }

    let index_dir = indexing::index_directory(index.location, root, output_folder);
    if !index_dir.exists() {
        return None;
    }

    let filters = filters.to_vec();
    tokio::task::spawn_blocking(move || {
        let engine = search_core::native_index::open_or_create_with_rebuild(&index_dir).ok()?;
        indexing::trigram_candidates(&engine, &filters).ok().flatten()
    })
    .await
    .unwrap_or(None)
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
