//! Ports `TextInFilesSearch.Core/Models/SearchModels.cs`,
//! `Models/ExtensionCatalog.cs`, and `Models/InFlightFileStatus.cs`. Pure
//! data - no logic - so this is a direct, low-risk 1:1 port; see
//! docs/rust-rewrite-status.md for what's ported vs. still pending.

use std::time::Duration;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MatchMode {
    #[default]
    AnyLine,
    AllInFile,
    Proximity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExcludeScope {
    #[default]
    Line,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GroupByMode {
    #[default]
    Created,
    Modified,
    None,
}

/// Every user-configurable setting for a search run - also the shape used
/// to fingerprint the incremental cache (see `cache.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSettings {
    pub search_path: String,
    pub output_folder: String,
    pub output_name: Option<String>,

    pub filters: Vec<String>,
    pub exclude_filters: Vec<String>,

    pub match_mode: MatchMode,
    pub proximity_lines: i32,
    pub exclude_scope: ExcludeScope,

    pub whole_word: bool,
    pub use_regex: bool,

    pub group_by: GroupByMode,

    /// `None` means "use the built-in default list" (`extension_catalog::all_extensions()`).
    pub extensions: Option<Vec<String>>,

    pub exclude_folders: Vec<String>,

    pub include_hidden: bool,
    pub max_file_size_mb: f64,
    pub max_embed_lines: i32,
    pub pdf_timeout_seconds: i32,

    pub export_csv: bool,
    pub export_json: bool,
    pub open_report_when_done: bool,

    pub parallel: bool,
    pub throttle_limit: i32,
    /// Separate concurrency limit for CPU/memory-heavier extraction
    /// (`.pdf`/`.docx`/`.pptx`/`.xlsx`/`.zip` - see
    /// `orchestrator::is_heavy_extension`) from `throttle_limit`, which
    /// now governs everything else (plain text, `.rtf`, ...) - epic #6
    /// §19: "TXT/LOG processing should not necessarily compete with large
    /// PDF extraction under the same limits." A folder with a handful of
    /// large PDFs mixed into thousands of small log files no longer lets
    /// the PDFs' extraction cost starve the log files' throughput (or vice
    /// versa - a huge burst of light files no longer queues behind a
    /// separate, smaller heavy-format limit either), since each class
    /// gets its own semaphore.
    pub heavy_throttle_limit: i32,

    pub cache_file_path: Option<String>,
    pub dry_run: bool,

    pub max_retries: i32,
    pub retry_delay_ms: i32,
    pub file_timeout_seconds: i32,
}

/// Default parallel throttle limit - scaled to the machine's core count
/// rather than the fixed `5` the PowerShell tool and C# port both used.
/// `5` was tuned for a much older baseline and, per
/// `docs/epic-ui-performance-and-design.md`'s large-folder performance
/// investigation, concurrency is the next real lever now that literal-mode
/// matching no longer pays `fancy_regex` overhead. 2x the core count is a
/// reasonable starting point for I/O-bound work (most time per file is
/// spent waiting on disk/extraction, not pegging a CPU core), clamped to
/// [4, 32] so a single-core CI runner still gets some concurrency and a
/// very-many-core machine doesn't spawn an unreasonable number of
/// concurrent file handles. `available_parallelism()` can fail (rare,
/// sandboxed/unusual environments) - falls back to the old fixed default
/// in that case rather than panicking.
pub fn default_throttle_limit() -> i32 {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(5);
    ((cores * 2) as i32).clamp(4, 32)
}

/// Default concurrency limit for heavy-format extraction
/// (`heavy_throttle_limit`) - closer to the raw core count (not 2x, like
/// the light-format default above) since ZIP/OOXML/PDF parsing is more
/// CPU/memory-bound per file than plain-text reading is, so running many
/// more of them at once than there are cores buys little real throughput
/// while multiplying peak memory. Clamped to [2, 16] - a smaller ceiling
/// than the light default's [4, 32], deliberately, for the same reason.
pub fn default_heavy_throttle_limit() -> i32 {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cores as i32).clamp(2, 16)
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            search_path: String::new(),
            output_folder: String::new(),
            output_name: None,
            filters: Vec::new(),
            exclude_filters: Vec::new(),
            match_mode: MatchMode::default(),
            proximity_lines: 5,
            exclude_scope: ExcludeScope::default(),
            whole_word: false,
            use_regex: false,
            group_by: GroupByMode::default(),
            extensions: None,
            exclude_folders: Vec::new(),
            include_hidden: false,
            max_file_size_mb: 50.0,
            max_embed_lines: 4000,
            pdf_timeout_seconds: 15,
            export_csv: false,
            export_json: false,
            open_report_when_done: false,
            parallel: false,
            throttle_limit: default_throttle_limit(),
            heavy_throttle_limit: default_heavy_throttle_limit(),
            cache_file_path: None,
            dry_run: false,
            max_retries: 3,
            retry_delay_ms: 250,
            file_timeout_seconds: 30,
        }
    }
}

/// One matched line within one file, with one line of context on each side.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LineHit {
    pub line_number: i32,
    pub before: Option<String>,
    pub match_line: String,
    pub after: Option<String>,
    pub matched_filters: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FileSearchStatus {
    Hit,
    #[default]
    NoHit,
    TooLarge,
    Binary,
    ReadError,
    ExcludedFile,
    ModeExcluded,
    UnexpectedError,
}

/// The uniform result of processing exactly one file, whether it ended up
/// with hits or was skipped for some reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSearchResult {
    pub full_name: String,
    pub status: FileSearchStatus,
    pub hits: Vec<LineHit>,
    pub created: DateTime<Local>,
    pub modified: DateTime<Local>,
    pub file_length: i64,
    pub lines_cache: Vec<String>,
    pub total_line_count: i32,
    pub proximity_min_range: Option<i32>,
    pub low_confidence_pdf: bool,
    pub error_message: Option<String>,
}

/// One accumulated warning (which file, what happened) surfaced in the
/// run summary. A named struct rather than the C# side's bare
/// `(string FullName, string Message)` tuple, so JSON export keeps field
/// names instead of serializing as a positional array - a small, harmless
/// improvement over a literal port, not a behavior change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    pub full_name: String,
    pub message: String,
}

/// Snapshot of counters accumulated across a whole run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchRunSummary {
    pub files_searched: i32,
    pub skipped_too_large: i32,
    pub skipped_binary: i32,
    pub skipped_read_error: i32,
    pub skipped_by_exclude: i32,
    pub skipped_by_mode: i32,
    pub skipped_unexpected_error: i32,
    pub cache_reused: i32,
    pub enumeration_errors: i32,
    pub warnings: Vec<Warning>,
}

/// Live status of one file currently being processed - in parallel mode
/// there can be several of these at once, up to the throttle limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InFlightFileStatus {
    pub file_name: String,
    pub status_text: String,
    pub elapsed_seconds: f64,
}

impl Default for InFlightFileStatus {
    fn default() -> Self {
        Self {
            file_name: String::new(),
            status_text: "Starting...".to_string(),
            elapsed_seconds: 0.0,
        }
    }
}

/// The full outcome of one `SearchOrchestrator` run.
#[derive(Debug, Clone, Default)]
pub struct SearchRunResult {
    pub file_results: Vec<FileSearchResult>,
    pub summary: SearchRunSummary,
    pub was_dry_run: bool,
    /// Only populated when `was_dry_run` is true; only the count is used by
    /// any caller today, so this is the candidate paths, not full metadata
    /// (unlike the C# side's `List<FileInfo>`).
    pub dry_run_candidates: Option<Vec<std::path::PathBuf>>,
}

/// A single live progress update pushed from the search engine to the UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchProgressReport {
    pub files_completed: i32,
    pub total_files: i32,
    pub hits_so_far: i32,
    pub current_file_name: Option<String>,
    pub current_file_status: Option<String>,
    #[serde(with = "duration_secs")]
    pub current_file_elapsed: Duration,
    pub is_dry_run: bool,
    pub is_enumerating: bool,
    pub enumerated_file_count: i32,
    pub in_flight_files: Vec<InFlightFileStatus>,
    pub last_completed_result: Option<FileSearchResult>,
}

mod duration_secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        d.as_secs_f64().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs_f64(f64::deserialize(d)?))
    }
}

/// One named group of related extensions, for the extension type-to-filter
/// picker UI.
#[derive(Debug, Clone)]
pub struct ExtensionCategoryDefinition {
    pub category: &'static str,
    pub extensions: &'static [&'static str],
}

/// Single source of truth for every extension this app knows how to
/// search, grouped by category. `all_extensions()` is the flattened form,
/// so the "built-in default" the engine searches and the catalog the
/// picker UI shows can never drift apart - ports
/// `Models/ExtensionCatalog.cs` exactly, category groupings included.
pub mod extension_catalog {
    use super::ExtensionCategoryDefinition;

    pub const CATEGORIES: &[ExtensionCategoryDefinition] = &[
        ExtensionCategoryDefinition {
            category: "Documents",
            extensions: &[".docx", ".pdf", ".rtf", ".txt", ".md"],
        },
        ExtensionCategoryDefinition {
            category: "Spreadsheets",
            extensions: &[".xlsx", ".csv", ".tsv"],
        },
        ExtensionCategoryDefinition {
            category: "Presentations",
            extensions: &[".pptx"],
        },
        ExtensionCategoryDefinition {
            category: "Archives",
            extensions: &[".zip"],
        },
        ExtensionCategoryDefinition {
            category: "Logs & structured data",
            extensions: &[
                ".log", ".json", ".xml", ".yaml", ".yml", ".ini", ".cfg", ".conf", ".toml",
                ".env",
            ],
        },
        ExtensionCategoryDefinition {
            category: "Web",
            extensions: &[".htm", ".html", ".css", ".scss", ".less"],
        },
        ExtensionCategoryDefinition {
            category: "Code",
            extensions: &[
                ".cs", ".java", ".py", ".js", ".ts", ".jsx", ".tsx", ".go", ".rs", ".rb", ".php",
                ".swift", ".kt", ".c", ".h", ".cpp", ".hpp", ".sql",
            ],
        },
        ExtensionCategoryDefinition {
            category: "Scripts",
            extensions: &[".ps1", ".psm1", ".bat", ".cmd", ".sh", ".zsh"],
        },
    ];

    /// Flattened, deduplicated (case-insensitive), sorted (case-insensitive)
    /// list of every extension across all categories - matches the C#
    /// side's `Distinct(StringComparer.OrdinalIgnoreCase).OrderBy(...)`.
    pub fn all_extensions() -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut all: Vec<String> = CATEGORIES
            .iter()
            .flat_map(|c| c.extensions.iter())
            .filter(|ext| seen.insert(ext.to_lowercase()))
            .map(|ext| ext.to_string())
            .collect();
        all.sort_by_key(|e| e.to_lowercase());
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_match_csharp_defaults() {
        let s = SearchSettings::default();
        assert_eq!(s.proximity_lines, 5);
        assert_eq!(s.max_file_size_mb, 50.0);
        assert_eq!(s.max_embed_lines, 4000);
        assert_eq!(s.pdf_timeout_seconds, 15);
        assert!((4..=32).contains(&s.throttle_limit), "throttle_limit out of expected range: {}", s.throttle_limit);
        assert_eq!(s.max_retries, 3);
        assert_eq!(s.retry_delay_ms, 250);
        assert_eq!(s.file_timeout_seconds, 30);
        assert_eq!(s.match_mode, MatchMode::AnyLine);
        assert_eq!(s.exclude_scope, ExcludeScope::Line);
        assert_eq!(s.group_by, GroupByMode::Created);
        assert!(s.extensions.is_none());
    }

    #[test]
    fn all_extensions_is_deduplicated_and_sorted_case_insensitively() {
        let all = extension_catalog::all_extensions();
        assert!(all.contains(&".docx".to_string()));
        assert!(all.contains(&".rs".to_string()));
        let mut sorted = all.clone();
        sorted.sort_by_key(|e| e.to_lowercase());
        assert_eq!(all, sorted, "all_extensions() must already be sorted");
        let unique: std::collections::HashSet<_> =
            all.iter().map(|e| e.to_lowercase()).collect();
        assert_eq!(unique.len(), all.len(), "must be deduplicated");
    }

    #[test]
    fn all_extensions_count_matches_csharp_catalog() {
        // TextInFilesSearch.Core/Models/ExtensionCatalog.cs flattens to this
        // exact count (8 categories, 5+3+1+1+10+5+18+6, no extension
        // repeated across categories in the current C# source) - a change
        // here should be a deliberate catalog edit, not a silent drift.
        assert_eq!(extension_catalog::all_extensions().len(), 49);
    }
}
