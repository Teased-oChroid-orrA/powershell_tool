//! Fast re-search indexing (native-search/Tantivy) for the Search Files
//! toolbox. Ported behavior from both existing GUI heads'
//! `build_or_rebuild_corpus_index` (`app/src/state.rs:1025`,
//! `app-egui/src/search.rs:776`) - open-or-create-with-rebuild, then
//! `build_or_update_corpus_index_send` (the `Send`-bounded variant,
//! required here for the same reason `app-egui/` documents: this crate's
//! tokio runtime is multi-threaded, unlike Dioxus's single-threaded
//! spawner the plain function was written for) - translated from egui's
//! per-frame mutex poll / Dioxus's `Signal` writes into this crate's
//! `AppEvent` channel architecture.
//!
//! Ratatui-free by design (see `model.rs`'s own module doc for the same
//! rule); the render layer lives in `index_view.rs`.
//!
//! NOTE for integration: this file references `crate::app::AppEvent`
//! variants `IndexBuildProgress(search_core::native_index::CorpusIndexProgress)`
//! and `IndexBuildFinished(native_search::error::NsResult<search_core::native_index::CorpusIndexOutcome>)`,
//! and expects `main.rs`'s effect executor to spawn `build_or_rebuild_index`
//! in response to a new `Effect::BuildIndex { settings: SearchSettings,
//! index_dir: PathBuf, force_rebuild: bool }` variant - none of which exist
//! yet in `app.rs`. Not wired in by this file; the parent session owns that.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use native_search::error::{NsError, NsResult};
use search_core::models::SearchSettings;
use search_core::native_index::{
    self, build_or_update_corpus_index_send, ensure_index_directory_exists, open_or_create_with_rebuild, CorpusIndexOutcome,
    CorpusIndexProgress,
};

use crate::app::AppEvent;

/// Where the fast re-search index lives - mirrors
/// `app-egui/src/persistence.rs::IndexLocation` exactly (same two
/// variants, same default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IndexLocation {
    #[default]
    SearchFolder,
    OutputFolder,
}

impl IndexLocation {
    pub fn cycle(self) -> Self {
        match self {
            IndexLocation::SearchFolder => IndexLocation::OutputFolder,
            IndexLocation::OutputFolder => IndexLocation::SearchFolder,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            IndexLocation::SearchFolder => "Search folder",
            IndexLocation::OutputFolder => "Output folder",
        }
    }
}

/// The user-facing "index for fast re-search" settings - a small struct
/// meant to be embedded as one field on `SearchToolConfig`
/// (`pub index: IndexSettings`) rather than flattened directly onto it, to
/// keep this feature's surface visually grouped in one place (the parent
/// session owns adding that field; not done here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IndexSettings {
    pub enabled: bool,
    pub location: IndexLocation,
}

/// Live index-build status the render layer reads. Deliberately separate
/// from `SearchRunState` - a plain search run and an index build are
/// different, independently-triggerable operations; both existing heads
/// keep separate live-status fields for them too.
#[derive(Debug, Clone, Default)]
pub struct IndexRunState {
    pub is_building: bool,
    pub status_text: String,
    pub last_error: Option<String>,
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
            Path::new(output_folder.trim()).join(".native-search-index").join(name)
        }
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
    let readable: String =
        readable.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("{readable}-{hash:x}")
}

/// Builds (or, if `force_rebuild`, deletes-then-rebuilds) the fast
/// re-search index for one root, streaming progress as
/// `AppEvent::IndexBuildProgress` and a final `AppEvent::IndexBuildFinished`.
/// Meant to be `tokio::spawn`'d by `main.rs`'s effect executor in response
/// to `Effect::BuildIndex` - never called from `handle_event` directly,
/// the same `Effect`-only-side-effects rule the rest of this crate follows.
pub async fn build_or_rebuild_index(tx: UnboundedSender<AppEvent>, settings: SearchSettings, index_dir: PathBuf, force_rebuild: bool) {
    let _ = tx.send(AppEvent::IndexBuildProgress(CorpusIndexProgress {
        files_processed: 0,
        total_files: 0,
        current_file: "Starting…".to_string(),
    }));

    let result = build_or_rebuild_index_inner(&tx, &settings, &index_dir, force_rebuild).await;
    let _ = tx.send(AppEvent::IndexBuildFinished(result));
}

async fn build_or_rebuild_index_inner(
    tx: &UnboundedSender<AppEvent>,
    settings: &SearchSettings,
    index_dir: &Path,
    force_rebuild: bool,
) -> NsResult<CorpusIndexOutcome> {
    if force_rebuild && index_dir.exists() {
        std::fs::remove_dir_all(index_dir).map_err(|e| NsError::index_error(format!("could not remove existing index: {e}")))?;
    }
    ensure_index_directory_exists(index_dir).map_err(|e| NsError::index_error(e.to_string()))?;
    let engine = open_or_create_with_rebuild(index_dir)?;

    let tx = tx.clone();
    let mut on_progress = move |p: CorpusIndexProgress| {
        let _ = tx.send(AppEvent::IndexBuildProgress(p));
    };

    build_or_update_corpus_index_send(settings, &engine, &CancellationToken::new(), Some(&mut on_progress)).await
}

/// Safe-superset candidate narrowing for a re-search: `None` means "no safe
/// narrowing possible, fall back to a full scan" (e.g. empty filters, or a
/// filter too short to produce trigrams) - the caller (`runner.rs`, which
/// owns the decision of `orchestrator::run` vs. `orchestrator::run_candidates`)
/// should treat `None` exactly the same as "index not available/not
/// enabled", never as an error. Uses the plain-filter form
/// (`trigram_candidate_paths`) rather than the chunk-set form
/// (`trigram_candidate_paths_for_chunk_sets`, used only for regex-mode
/// filters per its own doc comment) since this wrapper covers the common
/// literal-filter case both existing heads use it for.
pub fn trigram_candidates(engine: &native_search::engine::NativeSearchEngine, filters: &[String]) -> NsResult<Option<Vec<String>>> {
    engine.trigram_candidate_paths(filters)
}

/// Documents present in the index whose source file no longer exists -
/// thin wrapper over `search_core::native_index::remove_orphaned_documents`.
pub fn remove_orphaned_documents(engine: &native_search::engine::NativeSearchEngine) -> NsResult<usize> {
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

    #[test]
    fn search_folder_location_delegates_to_native_index_directory() {
        let dir = index_directory(IndexLocation::SearchFolder, "/tmp/project", "/tmp/output");
        assert_eq!(dir, native_index::index_directory("/tmp/project"));
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
    fn location_cycles_between_both_variants_and_wraps() {
        assert_eq!(IndexLocation::SearchFolder.cycle(), IndexLocation::OutputFolder);
        assert_eq!(IndexLocation::OutputFolder.cycle(), IndexLocation::SearchFolder);
    }

    #[tokio::test]
    async fn building_an_index_over_a_real_tempdir_creates_a_queryable_index() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world\nneedle here\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "nothing interesting\n").unwrap();

        let settings = SearchSettings { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        let index_dir = index_directory(IndexLocation::SearchFolder, &settings.search_path, "");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        build_or_rebuild_index(tx, settings, index_dir.clone(), false).await;

        let mut finished = None;
        while let Ok(event) = rx.try_recv() {
            if let AppEvent::IndexBuildFinished(result) = event {
                finished = Some(result);
            }
        }
        let outcome = finished.expect("expected an IndexBuildFinished event").expect("index build should succeed");
        assert_eq!(outcome.indexed_count, 2);

        let doc_count = verify_index(&index_dir).unwrap();
        assert_eq!(doc_count, 2);
    }

    #[tokio::test]
    async fn trigram_candidates_narrow_to_the_matching_file_after_a_build() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world\nneedle here\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "nothing interesting\n").unwrap();

        let settings = SearchSettings { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        let index_dir = index_directory(IndexLocation::SearchFolder, &settings.search_path, "");
        ensure_index_directory_exists(&index_dir).unwrap();
        let engine = open_or_create_with_rebuild(&index_dir).unwrap();
        let mut noop = |_p: CorpusIndexProgress| {};
        build_or_update_corpus_index_send(&settings, &engine, &CancellationToken::new(), Some(&mut noop)).await.unwrap();

        let candidates = trigram_candidates(&engine, &["needle".to_string()]).unwrap();
        assert!(candidates.is_some());
        let candidates = candidates.unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].ends_with("a.txt"));
    }

    #[tokio::test]
    async fn force_rebuild_deletes_and_recreates_the_index_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let settings = SearchSettings { search_path: dir.path().to_string_lossy().into_owned(), ..Default::default() };
        let index_dir = index_directory(IndexLocation::SearchFolder, &settings.search_path, "");

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        build_or_rebuild_index(tx.clone(), settings.clone(), index_dir.clone(), false).await;
        assert!(index_dir.exists());

        build_or_rebuild_index(tx, settings, index_dir.clone(), true).await;
        assert!(index_dir.exists());
        assert_eq!(verify_index(&index_dir).unwrap(), 1);
    }
}
