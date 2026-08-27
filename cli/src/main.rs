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
    /// alongside a folder to still get prompted for everything else). Also
    /// required (as the folder to operate on) by --verify-index and
    /// --remove-orphaned, in place of a search.
    #[arg(required_unless_present_any = ["interactive", "clear_cache", "list_failures"])]
    pub(crate) search_path: Option<PathBuf>,

    /// Filter text - at least one required, unless running in
    /// --interactive mode or performing a maintenance action
    /// (--verify-index / --remove-orphaned / --clear-cache /
    /// --list-failures) instead of a search. Repeat for multiple filters
    /// (any-line mode: a line matching ANY of them is a hit).
    #[arg(
        short = 'f',
        long = "filter",
        required_unless_present_any = ["interactive", "verify_index", "remove_orphaned", "clear_cache", "list_failures"]
    )]
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

    /// OCR fallback for image-only/scanned PDFs (no text-showing
    /// operators at all - just a drawn page image), only attempted when
    /// every other PDF text-extraction path finds nothing. Off by
    /// default: real per-file latency (roughly a second or more per
    /// page, not the millisecond range the rest of extraction runs in).
    #[arg(long)]
    pub(crate) ocr_scanned_pdfs: bool,

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

    /// Path to the incremental JSON result cache (fingerprinted by the
    /// settings that affect matching - see search-core's cache.rs). Not
    /// exposed in the GUI's CLI-equivalent flag set until now; opt-in,
    /// same as the GUI's own cache field defaulting to "unset".
    #[arg(long)]
    pub(crate) cache_file: Option<PathBuf>,

    // --- Maintenance actions (issue #6 §50) - each of these performs one
    // action and exits, instead of running a search. Mutually exclusive
    // with each other and with a normal search in practice (only the
    // first one present is honored, checked in that order in `run()`).
    /// Open the fast-search index for --search-path and report its
    /// document count, or a clear error if the index is corrupt/schema-
    /// mismatched - does NOT auto-rebuild (that would hide the exact
    /// problem this flag exists to surface). Follow up with `--index` on
    /// a normal search, or delete the `.native-search-index` folder
    /// manually, to rebuild.
    #[arg(long)]
    pub(crate) verify_index: bool,

    /// Delete every indexed document under --search-path whose file no
    /// longer exists on disk (moved/renamed/deleted since it was
    /// indexed), then commit. Prints the number removed.
    #[arg(long)]
    pub(crate) remove_orphaned: bool,

    /// Delete the file at --cache-file, if present.
    #[arg(long)]
    pub(crate) clear_cache: bool,

    /// Print every recorded extraction failure from --failure-log (path,
    /// size/modified fingerprint, status, reason, when it failed) as
    /// JSON, newest first.
    #[arg(long)]
    pub(crate) list_failures: bool,
}

fn main() -> ExitCode {
    // Issue #6 §57/§58 - search-core emits tracing events (discover/
    // extract/index/query/export spans and aggregate-not-per-file
    // summaries); this installs the subscriber that actually prints them.
    // Quiet by default (WARN) so a normal `search-cli` invocation stays
    // exactly as quiet as before this was added - opt in with
    // RUST_LOG=search_core=info (or =debug/=trace for the noisier
    // per-file events).
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();

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

/// Issue #6 §50 "Index Health/Maintenance" - four one-shot actions that
/// each perform their job and exit, instead of running a search. Checked
/// in this order; only the first one present is honored (they're not
/// meant to be combined). Kept as plain sync fns - none of the underlying
/// operations (Tantivy, rusqlite, `std::fs`) need the async runtime
/// `main()` sets up for the normal search path, so `run()` calls these
/// directly before doing anything else.
fn run_maintenance_action(cli: &Cli) -> Option<ExitCode> {
    if cli.list_failures {
        return Some(cmd_list_failures(cli));
    }
    if cli.clear_cache {
        return Some(cmd_clear_cache(cli));
    }
    if cli.verify_index {
        return Some(cmd_verify_index(cli));
    }
    if cli.remove_orphaned {
        return Some(cmd_remove_orphaned(cli));
    }
    None
}

fn cmd_verify_index(cli: &Cli) -> ExitCode {
    let Some(search_path) = &cli.search_path else {
        eprintln!("Error: --verify-index requires a search folder");
        return ExitCode::FAILURE;
    };
    let index_dir = native_index::index_directory(&search_path.to_string_lossy());
    match native_index::verify_index(&index_dir) {
        Ok(count) => {
            println!("Index OK: {count} document(s).");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Index verification failed: {e} (the index may be corrupt or from an older schema - rebuild it with a normal search using --index)");
            ExitCode::FAILURE
        }
    }
}

fn cmd_remove_orphaned(cli: &Cli) -> ExitCode {
    let Some(search_path) = &cli.search_path else {
        eprintln!("Error: --remove-orphaned requires a search folder");
        return ExitCode::FAILURE;
    };
    let index_dir = native_index::index_directory(&search_path.to_string_lossy());
    let engine = match native_index::open_or_create_with_rebuild(&index_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Error opening index: {e}");
            return ExitCode::FAILURE;
        }
    };
    match native_index::remove_orphaned_documents(&engine) {
        Ok(removed) => {
            println!("Removed {removed} orphaned document(s).");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error removing orphaned documents: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_clear_cache(cli: &Cli) -> ExitCode {
    let Some(path) = &cli.cache_file else {
        eprintln!("Error: --clear-cache requires --cache-file <path>");
        return ExitCode::FAILURE;
    };
    match std::fs::remove_file(path) {
        Ok(()) => {
            println!("Removed {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("No cache file at {} (nothing to do).", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error removing cache file: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_list_failures(cli: &Cli) -> ExitCode {
    let Some(path) = &cli.failure_log else {
        eprintln!("Error: --list-failures requires --failure-log <path>");
        return ExitCode::FAILURE;
    };
    let log = match search_core::failure_log::FailureLog::open(&path.to_string_lossy()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error opening failure log: {e}");
            return ExitCode::FAILURE;
        }
    };
    let failures = log.list_failures();
    println!("{} recorded failure(s).", failures.len());
    match serde_json::to_string_pretty(&failures) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("Error formatting failures as JSON: {e}"),
    }
    ExitCode::SUCCESS
}

/// `search_core::models::extension_catalog` (the GUI's source of truth
/// too) always stores/matches extensions with a leading dot (`.pdf`, not
/// `pdf`) - `orchestrator::filter_by_extension` compares against
/// `file_extension_lower`, which always includes the dot. The `--extensions`
/// flag's own `--help` text gives a dotless example ("txt,log,pdf") for
/// readability, so normalize here rather than silently matching zero
/// files when a user follows that example literally - `"*"` (the
/// search-all-extensions wildcard) is passed through unchanged.
fn normalize_extension(ext: String) -> String {
    if ext == "*" || ext.starts_with('.') {
        ext
    } else {
        format!(".{ext}")
    }
}

async fn run(cli: Cli) -> ExitCode {
    if let Some(code) = run_maintenance_action(&cli) {
        return code;
    }

    // Guaranteed Some here: clap enforces search_path/filters unless
    // --interactive or a maintenance action, and the --interactive branch
    // in main() fills both in via the wizard before run() is ever called.
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
        ocr_scanned_pdfs: cli.ocr_scanned_pdfs,
        group_by: cli.group_by.into(),
        extensions: cli.extensions.map(|exts| exts.into_iter().map(normalize_extension).collect()),
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
    if let Some(path) = &cli.cache_file {
        settings.cache_file_path = Some(path.to_string_lossy().into_owned());
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
