//! Ports `TextInFilesSearch.Core/Services/CacheService.cs`: a small JSON
//! cache mapping each file's path to its last search result, fingerprinted
//! by the settings that affect matching (filters, mode, etc). A file whose
//! size and modified time haven't changed since the fingerprint last
//! matched is reused untouched instead of being re-read - the single
//! biggest speed win for repeated searches over the same large folder.

use std::collections::HashMap;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::models::{ExcludeScope, FileSearchResult, FileSearchStatus, LineHit, MatchMode, SearchSettings};

/// One cached file's prior result, keyed by full path in [`CacheFile`].
///
/// `last_write_time_ticks` is nanoseconds since the Unix epoch (not .NET's
/// 100ns-since-0001-01-01 ticks) - this value is only ever compared to
/// itself across runs as a freshness fingerprint, never parsed by anything
/// else, so the exact epoch/unit doesn't need to match the C# original, only
/// that it's a stable, monotonic per-file mtime encoding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFileEntry {
    pub length: i64,
    pub last_write_time_ticks: i64,
    pub status: FileSearchStatus,
    pub hits: Vec<LineHit>,
    pub created: DateTime<Local>,
    pub modified: DateTime<Local>,
    pub lines_cache: Vec<String>,
    pub total_line_count: i32,
    pub proximity_min_range: Option<i32>,
    pub low_confidence_pdf: bool,
    pub error_message: Option<String>,
}

impl CachedFileEntry {
    pub fn to_file_search_result(&self, full_name: String) -> FileSearchResult {
        FileSearchResult {
            full_name,
            status: self.status,
            hits: self.hits.clone(),
            created: self.created,
            modified: self.modified,
            file_length: self.length,
            lines_cache: self.lines_cache.clone(),
            total_line_count: self.total_line_count,
            proximity_min_range: self.proximity_min_range,
            low_confidence_pdf: self.low_confidence_pdf,
            error_message: self.error_message.clone(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    fingerprint: String,
    entries: HashMap<String, CachedFileEntry>,
}

/// Precomputed metadata for one candidate file, supplied by the caller
/// (the directory walk already gathered it) rather than re-stat'd here -
/// mirrors the C# side taking `IReadOnlyList<FileInfo>`.
#[derive(Debug, Clone)]
pub struct CandidateMetadata {
    pub full_name: String,
    pub length: i64,
    pub last_write_time_ticks: i64,
}

#[derive(Serialize)]
struct FingerprintFields<'a> {
    filters: &'a [String],
    exclude_filters: &'a [String],
    match_mode: MatchMode,
    proximity_lines: i32,
    exclude_scope: ExcludeScope,
    whole_word: bool,
    use_regex: bool,
    max_file_size_mb: f64,
}

/// Derives a stable per-file freshness value from a modified timestamp -
/// used both when writing new cache entries and when comparing a
/// candidate's current mtime against a previously cached one. See
/// [`CachedFileEntry::last_write_time_ticks`] for why nanoseconds-since-epoch
/// (not .NET ticks) is fine here.
pub fn ticks_from_modified(modified: DateTime<Local>) -> i64 {
    modified.timestamp_nanos_opt().unwrap_or_else(|| modified.timestamp_millis().saturating_mul(1_000_000))
}

/// Only the settings that affect *matching* feed the fingerprint (not
/// output paths, parallelism, etc.) - a settings change that can't change
/// which lines match shouldn't invalidate the cache.
pub fn compute_fingerprint(settings: &SearchSettings) -> String {
    let fp = FingerprintFields {
        filters: &settings.filters,
        exclude_filters: &settings.exclude_filters,
        match_mode: settings.match_mode,
        proximity_lines: settings.proximity_lines,
        exclude_scope: settings.exclude_scope,
        whole_word: settings.whole_word,
        use_regex: settings.use_regex,
        max_file_size_mb: settings.max_file_size_mb,
    };
    serde_json::to_string(&fp).expect("fingerprint fields are always serializable")
}

/// Returns `None` if the cache file doesn't exist, can't be read, or was
/// built with different settings (a full rescan is then correct and
/// expected).
pub fn try_load(cache_file_path: &str, settings: &SearchSettings) -> Option<HashMap<String, CachedFileEntry>> {
    if !std::path::Path::new(cache_file_path).exists() {
        return None;
    }

    // A corrupt or unreadable cache file just means we start fresh - never
    // a fatal error, and the file gets overwritten at the end.
    let json = std::fs::read_to_string(cache_file_path).ok()?;
    let cache: CacheFile = serde_json::from_str(&json).ok()?;

    if cache.fingerprint != compute_fingerprint(settings) {
        return None;
    }

    Some(cache.entries)
}

pub fn save(
    cache_file_path: &str,
    settings: &SearchSettings,
    candidates: &[CandidateMetadata],
    all_results: &[FileSearchResult],
) {
    let candidate_by_path: HashMap<String, &CandidateMetadata> =
        candidates.iter().map(|c| (c.full_name.to_lowercase(), c)).collect();

    let mut entries: HashMap<String, CachedFileEntry> = HashMap::new();
    for r in all_results {
        if let Some(meta) = candidate_by_path.get(&r.full_name.to_lowercase()) {
            entries.insert(
                r.full_name.clone(),
                CachedFileEntry {
                    length: meta.length,
                    last_write_time_ticks: meta.last_write_time_ticks,
                    status: r.status,
                    hits: r.hits.clone(),
                    created: r.created,
                    modified: r.modified,
                    lines_cache: r.lines_cache.clone(),
                    total_line_count: r.total_line_count,
                    proximity_min_range: r.proximity_min_range,
                    low_confidence_pdf: r.low_confidence_pdf,
                    error_message: r.error_message.clone(),
                },
            );
        }
    }

    let cache_file = CacheFile {
        fingerprint: compute_fingerprint(settings),
        entries,
    };

    // Failing to write the cache should never fail the search itself - it
    // just means next run starts from scratch again.
    if let Ok(json) = serde_json::to_string(&cache_file) {
        let _ = atomic_write(cache_file_path, json.as_bytes());
    }
}

/// Write-to-temp-then-rename (issue #6 §51 "crash recovery" - "do not
/// mark a document as successfully indexed until the appropriate
/// persistence operation has completed"). A direct `std::fs::write`
/// truncates the destination file before writing its new content, so a
/// crash/power-loss mid-write leaves a truncated, unparseable cache file
/// that `try_load`'s `serde_json::from_str` would then fail to load -
/// silently losing the whole incremental cache, not just the last run's
/// updates. Writing to a sibling `.tmp` path first and renaming over the
/// real path means a crash mid-write only ever leaves behind an orphaned
/// `.tmp` file; the real cache file is either the old complete one or the
/// new complete one, never a partial one. `std::fs::rename` is an atomic
/// replace on both this project's target platform (Windows'
/// `MoveFileExW`/`MOVEFILE_REPLACE_EXISTING`, which `std::fs::rename`
/// uses) and Unix (`rename(2)`).
fn atomic_write(path: &str, contents: &[u8]) -> std::io::Result<()> {
    let tmp_path = format!("{path}.tmp");
    std::fs::write(&tmp_path, contents)?;
    std::fs::rename(&tmp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MatchMode;

    fn sample_result(full_name: &str) -> FileSearchResult {
        FileSearchResult {
            full_name: full_name.to_string(),
            status: FileSearchStatus::Hit,
            hits: vec![],
            created: Local::now(),
            modified: Local::now(),
            file_length: 123,
            lines_cache: vec!["line1".to_string()],
            total_line_count: 1,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }
    }

    #[test]
    fn fingerprint_changes_when_matching_settings_change() {
        let mut a = SearchSettings {
            filters: vec!["apple".to_string()],
            ..Default::default()
        };
        let fp_a = compute_fingerprint(&a);

        a.filters.push("banana".to_string());
        let fp_b = compute_fingerprint(&a);
        assert_ne!(fp_a, fp_b);

        let mut c = a.clone();
        c.match_mode = MatchMode::AllInFile;
        assert_ne!(compute_fingerprint(&a), compute_fingerprint(&c));
    }

    #[test]
    fn fingerprint_is_stable_for_identical_settings() {
        let s = SearchSettings {
            filters: vec!["x".to_string()],
            ..Default::default()
        };
        assert_eq!(compute_fingerprint(&s), compute_fingerprint(&s));
    }

    #[test]
    fn fingerprint_ignores_settings_that_do_not_affect_matching() {
        let mut a = SearchSettings {
            filters: vec!["x".to_string()],
            ..Default::default()
        };
        let mut b = a.clone();
        b.parallel = !a.parallel;
        b.throttle_limit = 99;
        b.dry_run = true;
        a.output_folder = "/somewhere/else".to_string();
        assert_eq!(compute_fingerprint(&a), compute_fingerprint(&b));
    }

    #[test]
    fn try_load_returns_none_when_file_missing() {
        let settings = SearchSettings::default();
        assert!(try_load("/definitely/does/not/exist/cache.json", &settings).is_none());
    }

    #[test]
    fn try_load_returns_none_for_corrupt_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        std::fs::write(&path, "not valid json").unwrap();

        let settings = SearchSettings::default();
        assert!(try_load(path.to_str().unwrap(), &settings).is_none());
    }

    #[test]
    fn try_load_returns_none_when_fingerprint_mismatches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");

        let settings_a = SearchSettings {
            filters: vec!["apple".to_string()],
            ..Default::default()
        };
        save(path.to_str().unwrap(), &settings_a, &[], &[]);

        let settings_b = SearchSettings {
            filters: vec!["banana".to_string()],
            ..Default::default()
        };
        assert!(try_load(path.to_str().unwrap(), &settings_b).is_none());
    }

    #[test]
    fn save_then_load_round_trips_matching_candidates_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");

        let settings = SearchSettings {
            filters: vec!["apple".to_string()],
            ..Default::default()
        };

        let candidates = vec![
            CandidateMetadata {
                full_name: "/x/a.txt".to_string(),
                length: 10,
                last_write_time_ticks: 111,
            },
            // b.txt is in all_results but NOT in candidates - must be dropped.
        ];
        let results = vec![sample_result("/x/a.txt"), sample_result("/x/b.txt")];

        save(path.to_str().unwrap(), &settings, &candidates, &results);

        let loaded = try_load(path.to_str().unwrap(), &settings).unwrap();
        assert_eq!(loaded.len(), 1);
        let entry = &loaded["/x/a.txt"];
        assert_eq!(entry.length, 10);
        assert_eq!(entry.last_write_time_ticks, 111);
        assert_eq!(entry.total_line_count, 1);

        let restored = entry.to_file_search_result("/x/a.txt".to_string());
        assert_eq!(restored.full_name, "/x/a.txt");
        assert_eq!(restored.file_length, 10);
        assert_eq!(restored.status, FileSearchStatus::Hit);
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_tmp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let path_str = path.to_str().unwrap();

        super::atomic_write(path_str, b"first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");

        super::atomic_write(path_str, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second", "rename must replace, not append or fail");

        assert!(!dir.path().join("cache.json.tmp").exists(), "the temp file must not linger after a successful write");
    }

    #[test]
    fn candidate_matching_is_case_insensitive_like_dotnet_ordinal_ignore_case() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let settings = SearchSettings::default();

        let candidates = vec![CandidateMetadata {
            full_name: "/X/A.TXT".to_string(),
            length: 5,
            last_write_time_ticks: 1,
        }];
        let results = vec![sample_result("/x/a.txt")];

        save(path.to_str().unwrap(), &settings, &candidates, &results);
        let loaded = try_load(path.to_str().unwrap(), &settings).unwrap();
        assert_eq!(loaded.len(), 1);
    }
}
