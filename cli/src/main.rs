//! Headless CLI for search-core (issue #6 §60). Not a port of every
//! `SettingsPanel` field in `app/` - a reasonable, useful subset of
//! `SearchSettings` exposed as flags, defaulting everything else to
//! `SearchSettings::default()`. Proves search-core is genuinely usable
//! without Dioxus (the whole point of §60), and doubles as a fast local
//! way to run a search/generate a report without opening the GUI.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use search_core::models::{ExcludeScope, GroupByMode, MatchMode, SearchSettings};
use search_core::{native_index, orchestrator, report};
use tokio_util::sync::CancellationToken;

mod interactive;

#[derive(Copy, Clone, ValueEnum)]
pub(crate) enum CliMatchMode {
    AnyLine,
    AllInFile,
    Proximity,
}

impl From<CliMatchMode> for MatchMode {
    fn from(m: CliMatchMode) -> Self {
        match m {
            CliMatchMode::AnyLine => MatchMode::AnyLine,
            CliMatchMode::AllInFile => MatchMode::AllInFile,
            CliMatchMode::Proximity => MatchMode::Proximity,
        }
    }
}

#[derive(Copy, Clone, ValueEnum)]
pub(crate) enum CliExcludeScope {
    Line,
    File,
}

impl From<CliExcludeScope> for ExcludeScope {
    fn from(s: CliExcludeScope) -> Self {
        match s {
            CliExcludeScope::Line => ExcludeScope::Line,
            CliExcludeScope::File => ExcludeScope::File,
        }
    }
}

#[derive(Copy, Clone, ValueEnum)]
pub(crate) enum CliGroupBy {
    Created,
    Modified,
    None,
}

impl From<CliGroupBy> for GroupByMode {
    fn from(g: CliGroupBy) -> Self {
        match g {
            CliGroupBy::Created => GroupByMode::Created,
            CliGroupBy::Modified => GroupByMode::Modified,
            CliGroupBy::None => GroupByMode::None,
        }
    }
}

/// Recursively search a folder for keyword filters and write an HTML (and
/// optionally CSV/JSON) report - the same search-core engine `app/`'s
/// Dioxus GUI uses, headless.
#[derive(Parser)]
#[command(name = "search-cli", version, about)]
pub(crate) struct Cli {
    /// Folder to search, recursively. Omit together with --filter to be
    /// prompted interactively instead (or pass --interactive explicitly
    /// alongside a folder to still get prompted for everything else).
    #[arg(required_unless_present = "interactive")]
    pub(crate) search_path: Option<PathBuf>,

    /// Filter text - at least one required (unless --interactive).
    /// Repeat for multiple filters (any-line mode: a line matching ANY of
    /// them is a hit).
    #[arg(short = 'f', long = "filter", required_unless_present = "interactive")]
    pub(crate) filters: Vec<String>,

    /// Walk through an interactive menu (folder, filters, mode, and -
    /// optionally - advanced options) instead of requiring flags upfront.
    #[arg(short = 'i', long)]
    pub(crate) interactive: bool,

    /// Folder to write the report(s) into. Defaults to the search folder.
    #[arg(short = 'o', long)]
    pub(crate) output_folder: Option<PathBuf>,

    /// Report base file name (without extension). Defaults to a
    /// timestamped name, same as the GUI.
    #[arg(long)]
    pub(crate) output_name: Option<String>,

    /// Exclude filter text - lines/files matching any of these are
    /// dropped. Repeat for multiple.
    #[arg(short = 'x', long = "exclude")]
    pub(crate) exclude_filters: Vec<String>,

    #[arg(long, value_enum, default_value = "line")]
    pub(crate) exclude_scope: CliExcludeScope,

    #[arg(long, value_enum, default_value = "any-line")]
    pub(crate) mode: CliMatchMode,

    /// Only meaningful with --mode proximity.
    #[arg(long, default_value_t = 5)]
    pub(crate) proximity_lines: i32,

    /// Treat filters as regular expressions instead of literal substrings.
    #[arg(long)]
    pub(crate) regex: bool,

    /// Match whole words/tokens only (ignored in --regex mode).
    #[arg(long)]
    pub(crate) whole_word: bool,

    /// Extensions to search, comma-separated (e.g. "txt,log,pdf"). Omit
    /// to use the built-in default extension catalog, same as the GUI.
    #[arg(long, value_delimiter = ',')]
    pub(crate) extensions: Option<Vec<String>>,

    /// Folder names to exclude from the walk, comma-separated. Matched by
    /// whole path segment, not substring.
    #[arg(long, value_delimiter = ',')]
    pub(crate) exclude_folders: Vec<String>,

    #[arg(long)]
    pub(crate) include_hidden: bool,

    #[arg(long, default_value_t = 50.0)]
    pub(crate) max_file_size_mb: f64,

    #[arg(long, value_enum, default_value = "created")]
    pub(crate) group_by: CliGroupBy,

    /// Disable parallel processing (sequential is easier to reason about
    /// for scripted/CI use, but much slower on large folders).
    #[arg(long)]
    pub(crate) no_parallel: bool,

    #[arg(long)]
    pub(crate) throttle_limit: Option<i32>,

    #[arg(long)]
    pub(crate) heavy_throttle_limit: Option<i32>,

    /// List what would be searched without reading or writing anything.
    #[arg(long)]
    pub(crate) dry_run: bool,

    #[arg(long)]
    pub(crate) csv: bool,

    #[arg(long)]
    pub(crate) json: bool,

    /// JSON Lines export (one compact JSON object per line) - better
    /// suited to large exports and downstream pipeline processing
    /// (`jq`, `grep`, etc.) than a single pretty-printed JSON array.
    #[arg(long)]
    pub(crate) jsonl: bool,

    /// Also build/update the persistent fast-search index for this folder
    /// (issue #6 Phase 1) while searching - the CLI's equivalent of the
    /// GUI's "Index this folder for fast re-search" checkbox.
    #[arg(long)]
    pub(crate) index: bool,

    /// Path to a persistent SQLite extraction-failure log (issue #6
    /// §12/§16 - not exposed in the GUI yet, see search-core's
    /// failure_log.rs). A file that fails extraction is skipped on
    /// future runs (using this same path, on unchanged content) instead
    /// of being re-attempted every time.
    #[arg(long)]
    pub(crate) failure_log: Option<PathBuf>,
}

fn main() -> ExitCode {
    let mut cli = Cli::parse();
    if cli.interactive {
        match interactive::gather(cli) {
            Ok(filled) => cli = filled,
            Err(e) => {
                eprintln!("Error: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start the async runtime");
    rt.block_on(run(cli))
}

async fn run(cli: Cli) -> ExitCode {
    // Guaranteed Some here: clap enforces search_path/filters unless
    // --interactive, and the --interactive branch in main() fills both
    // in via the wizard before run() is ever called.
    let search_path = cli
        .search_path
        .as_ref()
        .expect("search_path must be populated by clap or the interactive wizard before run()")
        .to_string_lossy()
        .into_owned();
    let output_folder = cli
        .output_folder
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| search_path.clone());

    let mut settings = SearchSettings {
        search_path: search_path.clone(),
        output_folder,
        output_name: cli.output_name,
        filters: cli.filters,
        exclude_filters: cli.exclude_filters,
        match_mode: cli.mode.into(),
        proximity_lines: cli.proximity_lines,
        exclude_scope: cli.exclude_scope.into(),
        whole_word: cli.whole_word,
        use_regex: cli.regex,
        group_by: cli.group_by.into(),
        extensions: cli.extensions,
        exclude_folders: cli.exclude_folders,
        include_hidden: cli.include_hidden,
        max_file_size_mb: cli.max_file_size_mb,
        parallel: !cli.no_parallel,
        dry_run: cli.dry_run,
        export_csv: cli.csv,
        export_json: cli.json,
        ..SearchSettings::default()
    };
    if let Some(v) = cli.throttle_limit {
        settings.throttle_limit = v;
    }
    if let Some(v) = cli.heavy_throttle_limit {
        settings.heavy_throttle_limit = v;
    }
    if let Some(path) = &cli.failure_log {
        settings.failure_log_path = Some(path.to_string_lossy().into_owned());
    }
    if cli.index {
        native_index::ensure_index_folder_excluded(&mut settings.exclude_folders);
    }

    let cancellation = CancellationToken::new();
    // No progress channel - a headless run prints one summary at the end,
    // not a live-updating display a terminal can't easily redraw anyway.
    let result = match orchestrator::run(settings.clone(), None, cancellation).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if result.was_dry_run {
        let count = result.dry_run_candidates.as_ref().map(|c| c.len()).unwrap_or(0);
        println!("Dry run: {count} file(s) would be searched. Nothing was read or written.");
        return ExitCode::SUCCESS;
    }

    let hit_count = result.file_results.iter().filter(|r| r.status == search_core::models::FileSearchStatus::Hit).count();
    let total_hits: i32 = result.file_results.iter().map(|r| r.hits.len() as i32).sum();
    println!(
        "Searched {} file(s). {hit_count} file(s) with hits, {total_hits} total hits. Skipped: {} too large, {} binary, {} unreadable, {} unexpected errors.",
        result.summary.files_searched,
        result.summary.skipped_too_large,
        result.summary.skipped_binary,
        result.summary.skipped_read_error,
        result.summary.skipped_unexpected_error,
    );

    if let Err(e) = std::fs::create_dir_all(&settings.output_folder) {
        eprintln!("Error creating output folder: {e}");
        return ExitCode::FAILURE;
    }
    let output_name = match &settings.output_name {
        Some(n) if n.to_lowercase().ends_with(".html") => n.clone(),
        Some(n) => format!("{n}.html"),
        None => format!("SearchResults_{}.html", chrono::Local::now().format("%Y%m%d_%H%M%S")),
    };
    let report_path = PathBuf::from(&settings.output_folder).join(&output_name);
    match report::write_html_report(&report_path.to_string_lossy(), &settings, &result) {
        Ok(bytes) => println!("Wrote {} ({bytes} bytes)", report_path.display()),
        Err(e) => {
            eprintln!("Error writing report: {e}");
            return ExitCode::FAILURE;
        }
    }

    if settings.export_csv || settings.export_json || cli.jsonl {
        let rows = report::build_export_rows(&result);
        let stem = report_path.with_extension("");
        if settings.export_csv {
            let csv_path = stem.with_extension("csv");
            match report::write_csv(&csv_path.to_string_lossy(), &rows) {
                Ok(()) => println!("Wrote {}", csv_path.display()),
                Err(e) => eprintln!("Error writing CSV: {e}"),
            }
        }
        if settings.export_json {
            let json_path = stem.with_extension("json");
            match report::write_json(&json_path.to_string_lossy(), &rows) {
                Ok(()) => println!("Wrote {}", json_path.display()),
                Err(e) => eprintln!("Error writing JSON: {e}"),
            }
        }
        if cli.jsonl {
            let jsonl_path = stem.with_extension("jsonl");
            match report::write_jsonl(&jsonl_path.to_string_lossy(), &rows) {
                Ok(()) => println!("Wrote {}", jsonl_path.display()),
                Err(e) => eprintln!("Error writing JSONL: {e}"),
            }
        }
    }

    if cli.index {
        // Indexes the whole matching-extension corpus itself (walks the
        // folder independently) - doesn't need this run's hit results.
        let index_dir = native_index::index_directory(&settings.search_path);
        match native_index::ensure_index_directory_exists(&index_dir)
            .map_err(|e| e.to_string())
            .and_then(|()| native_index::open_or_create_with_rebuild(&index_dir).map_err(|e| e.to_string()))
        {
            Ok(engine) => {
                match native_index::build_or_update_corpus_index(&settings, &engine, &CancellationToken::new(), None).await {
                    Ok(outcome) => println!(
                        "Indexed {} file(s), {} already up to date, {} failed.",
                        outcome.indexed_count, outcome.skipped_count, outcome.failed_count
                    ),
                    Err(e) => eprintln!("Indexing failed: {e}"),
                }
            }
            Err(e) => eprintln!("Indexing failed: {e}"),
        }
    }

    ExitCode::SUCCESS
}
