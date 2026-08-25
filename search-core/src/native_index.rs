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
use native_search::error::NsResult;

use crate::models::FileSearchResult;

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

    Ok(outcome)
}

/// Searches whatever's currently in the native_search index (built up via
/// [`index_hits_for_fast_search`] on prior runs) - a separate capability
/// from the normal per-run line scan (`orchestrator::run`), not a
/// replacement for it.
pub fn search(engine: &NativeSearchEngine, query: &str, limit: usize) -> NsResult<Vec<SearchHit>> {
    engine.search(query, limit, None)
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
}
