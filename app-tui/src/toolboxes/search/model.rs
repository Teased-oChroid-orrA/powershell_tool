//! Pure, ratatui-free business logic for the Search Files toolbox. No
//! `ratatui`/`crossterm` import belongs in this file - everything here is
//! plain data in, plain data out, so it's testable without a terminal or a
//! tokio runtime (see the migration plan's state-ownership rule: UI state
//! must not own search/business logic).
//!
//! Ported from `app/src/state.rs::AppState` (the Dioxus head's fuller,
//! authoritative implementation per the plan's parity decision - not
//! `app-egui/`'s thinner `apply_progress`, which drops `in_flight_files`/
//! `last_completed_result` entirely). Every `Signal<T>` field there becomes
//! a plain field here; every `.read()`/`.set()`/`.write()` call becomes a
//! direct field access.

use search_core::models::{
    ExcludeScope, FileSearchResult, FileSearchStatus, GroupByMode, InFlightFileStatus, MatchMode,
    SearchProgressReport, SearchRunResult, SearchRunSummary, SearchSettings,
};

/// Mirrors `SearchSettings` field-for-field, plus the raw text-buffer
/// fields the UI edits directly (comma-separated filter/exclude/folder
/// lists, parsed into `Vec<String>` only at `build_settings` time - same
/// split `app/`'s `AppState` uses between e.g. `filters_text: Signal<String>`
/// and the parsed `SearchSettings.filters: Vec<String>`).
///
/// Fast re-search index fields (`index_for_fast_search`, `IndexLocation`,
/// etc.) are deliberately omitted - that feature is explicitly deferred to
/// a follow-up phase per the approved migration plan.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchToolConfig {
    pub search_path: String,
    pub search_paths_extra: Vec<String>,
    pub output_folder: String,
    pub output_name: String,
    pub filters_text: String,
    pub exclude_filters_text: String,

    pub match_mode: MatchMode,
    pub proximity_lines: i32,
    pub use_regex: bool,
    pub whole_word: bool,
    pub exclude_scope: ExcludeScope,

    /// Currently unused - `app/` and `app-egui` bind this to a text box for
    /// typing an extension not found by their (static or scanned) catalog
    /// and adding it as a custom entry; `extension_picker.rs`'s modal has no
    /// equivalent input yet, so nothing in this crate reads or writes this
    /// field today. Kept (rather than removed) only because it's already
    /// part of the persisted settings-tui.json schema.
    pub extension_filter_text: String,
    /// `None` means "use the built-in default catalog" - mirrors
    /// `SearchSettings.extensions`'s own convention exactly, so this is a
    /// direct pass-through at `build_settings` time rather than something
    /// derived from a separately-tracked extension-catalog struct. Written by
    /// the `extension_picker.rs` modal (`Effect::ScanExtensions` round trip
    /// via `toolboxes::search::handle_key`), not by the Settings form itself.
    pub selected_extensions: Option<Vec<String>>,
    pub exclude_folders_text: String,
    pub include_hidden: bool,
    pub max_file_size_mb: f64,
    pub group_by: GroupByMode,

    pub export_html: bool,
    pub open_report_when_done: bool,
    pub export_csv: bool,
    pub export_json: bool,

    pub parallel: bool,
    pub throttle_limit: i32,
    pub heavy_throttle_limit: i32,
    pub cache_file_path: String,
    pub dry_run: bool,
    pub pdf_timeout_seconds: i32,
    pub ocr_scanned_pdfs: bool,
    pub file_timeout_seconds: i32,
    pub max_retries: i32,

    /// Fast re-search (native-search/Tantivy) settings - a small embedded
    /// struct (see `indexing::IndexSettings`'s own doc comment) rather than
    /// flattened fields, to keep this feature's surface visually grouped.
    pub index: super::indexing::IndexSettings,
}

impl Default for SearchToolConfig {
    fn default() -> Self {
        Self {
            search_path: String::new(),
            search_paths_extra: Vec::new(),
            output_folder: String::new(),
            output_name: String::new(),
            filters_text: String::new(),
            exclude_filters_text: String::new(),

            match_mode: MatchMode::AnyLine,
            proximity_lines: 5,
            use_regex: false,
            whole_word: false,
            exclude_scope: ExcludeScope::Line,

            extension_filter_text: String::new(),
            selected_extensions: None,
            exclude_folders_text: String::new(),
            include_hidden: false,
            max_file_size_mb: 50.0,
            group_by: GroupByMode::Created,

            export_html: true,
            open_report_when_done: false,
            export_csv: false,
            export_json: false,

            parallel: false,
            throttle_limit: search_core::models::default_throttle_limit(),
            heavy_throttle_limit: search_core::models::default_heavy_throttle_limit(),
            cache_file_path: String::new(),
            dry_run: false,
            pdf_timeout_seconds: 15,
            ocr_scanned_pdfs: false,
            file_timeout_seconds: 30,
            max_retries: 3,
            index: super::indexing::IndexSettings::default(),
        }
    }
}

/// Ported verbatim from `app/src/state.rs::parse_list` (state.rs:1168-1170).
pub fn parse_list(text: &str) -> Vec<String> {
    text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// How one filter group is applied (its trailing `[tag]` in the Filters field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMode {
    /// `[any]` / `[all]` / `[near N]` (N lines; plain `[near]` = the global range).
    Match(MatchMode, Option<i32>),
    /// `[not]` drops files containing any of the filters; `[not line]` only
    /// ignores the matching lines.
    Exclude(ExcludeScope),
}

/// One `;`-separated part of the Filters field: comma-separated filters plus
/// an optional trailing mode tag. `None` = the global mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGroup {
    pub filters: Vec<String>,
    pub mode: Option<GroupMode>,
}

/// Splits the Filters field into groups: `house ; floor, two [near 3] ;
/// draft [not]` is three groups - "house" (global mode), "floor"+"two" within
/// 3 lines, and files containing "draft" excluded. Without a `;` it is one
/// group, exactly as before, so the feature costs no screen space until it is
/// used. Empty groups are dropped. Only the exact tags above count (with or
/// without a space before them: `Test 4[not]` works), so a regex ending in a
/// character class such as `x[abc]` is not mistaken for one.
pub fn parse_filter_groups(text: &str) -> Vec<ParsedGroup> {
    text.split(';')
        .filter_map(|part| {
            let part = part.trim();
            let (body, mode) = split_mode_tag(part);
            let filters = parse_list(body);
            (!filters.is_empty()).then_some(ParsedGroup { filters, mode })
        })
        .collect()
}

fn split_mode_tag(part: &str) -> (&str, Option<GroupMode>) {
    let Some(open) = part.rfind('[') else { return (part, None) };
    if !part.ends_with(']') {
        return (part, None);
    }
    let tag = part[open + 1..part.len() - 1].trim().to_lowercase();
    let mode = match tag.as_str() {
        "any" => Some(GroupMode::Match(MatchMode::AnyLine, None)),
        "all" => Some(GroupMode::Match(MatchMode::AllInFile, None)),
        "near" => Some(GroupMode::Match(MatchMode::Proximity, None)),
        "not" => Some(GroupMode::Exclude(ExcludeScope::File)),
        "not line" => Some(GroupMode::Exclude(ExcludeScope::Line)),
        t => t
            .strip_prefix("near")
            .and_then(|n| n.trim().parse::<i32>().ok())
            .map(|n| GroupMode::Match(MatchMode::Proximity, Some(n.max(0)))),
    };
    match mode {
        Some(m) => (part[..open].trim_end(), Some(m)),
        None => (part, None),
    }
}

/// Short label for a group's mode, as shown in the Run view's group summary.
pub fn mode_label(mode: GroupMode, global_proximity: i32) -> String {
    let (mode, n) = match mode {
        GroupMode::Exclude(ExcludeScope::File) => return "exclude file".to_string(),
        GroupMode::Exclude(ExcludeScope::Line) => return "exclude lines".to_string(),
        GroupMode::Match(m, n) => (m, n.unwrap_or(global_proximity)),
    };
    match mode {
        MatchMode::AnyLine => "any line".to_string(),
        MatchMode::AllInFile => "all in file".to_string(),
        MatchMode::Proximity => format!("within {n} lines"),
    }
}

/// Group's mode with the global default filled in (for the summary line).
pub fn effective_mode(group: &ParsedGroup, config: &SearchToolConfig) -> GroupMode {
    group.mode.unwrap_or(GroupMode::Match(config.match_mode, None))
}

/// Primary filters/mode and extra groups for `config`, resolved against the
/// global mode (Settings) - the single place the Filters field is interpreted.
/// The primary is the first group that is not an exclusion.
fn resolve_filters(config: &SearchToolConfig) -> (Vec<String>, MatchMode, i32, Vec<search_core::models::FilterGroup>) {
    let groups = parse_filter_groups(&config.filters_text);
    let primary_idx = groups.iter().position(|g| !matches!(g.mode, Some(GroupMode::Exclude(_))));
    let mut primary = (Vec::new(), config.match_mode, config.proximity_lines);
    let mut extra = Vec::new();
    for (i, g) in groups.into_iter().enumerate() {
        let (match_mode, proximity_lines, exclude) = match g.mode {
            Some(GroupMode::Match(m, n)) => (m, n.unwrap_or(config.proximity_lines), None),
            Some(GroupMode::Exclude(scope)) => (config.match_mode, config.proximity_lines, Some(scope)),
            None => (config.match_mode, config.proximity_lines, None),
        };
        if Some(i) == primary_idx {
            primary = (g.filters, match_mode, proximity_lines);
        } else {
            extra.push(search_core::models::FilterGroup { filters: g.filters, match_mode, proximity_lines, exclude });
        }
    }
    (primary.0, primary.1, primary.2, extra)
}

/// Ported from `app/src/state.rs::AppState::build_settings` (state.rs:437-485),
/// field-for-field. `max_embed_lines` (4000) and `retry_delay_ms` (250) are
/// not exposed in any Search Files UI in either existing head either - same
/// scope boundary, not an oversight.
pub fn build_settings(config: &SearchToolConfig) -> SearchSettings {
    let output_name_raw = config.output_name.trim().to_string();
    let cache_path_raw = config.cache_file_path.trim().to_string();

    let mut exclude_folders = parse_list(&config.exclude_folders_text);
    // Matches `app/`'s `build_exclude_folders` exactly: always exclude the
    // fast re-search index folder, regardless of whether this phase's UI
    // exposes indexing - a stray `.native-search-index/` left behind by
    // another head (or a future indexing pass here) must never be treated
    // as searchable content.
    search_core::native_index::ensure_index_folder_excluded(&mut exclude_folders);

    let (filters, match_mode, proximity_lines, filter_groups) = resolve_filters(config);

    SearchSettings {
        search_path: config.search_path.trim().to_string(),
        output_folder: config.output_folder.trim().to_string(),
        output_name: if output_name_raw.is_empty() {
            None
        } else {
            Some(sanitize_file_name(&output_name_raw))
        },
        filters,
        filter_groups,
        exclude_filters: parse_list(&config.exclude_filters_text),
        match_mode,
        proximity_lines,
        exclude_scope: config.exclude_scope,
        whole_word: config.whole_word,
        use_regex: config.use_regex,
        group_by: config.group_by,
        extensions: config.selected_extensions.clone(),
        exclude_folders,
        include_hidden: config.include_hidden,
        max_file_size_mb: config.max_file_size_mb,
        max_embed_lines: 4000,
        pdf_timeout_seconds: config.pdf_timeout_seconds,
        ocr_scanned_pdfs: config.ocr_scanned_pdfs,
        export_csv: config.export_csv,
        export_json: config.export_json,
        open_report_when_done: config.open_report_when_done,
        parallel: config.parallel,
        throttle_limit: config.throttle_limit,
        heavy_throttle_limit: config.heavy_throttle_limit,
        cache_file_path: if cache_path_raw.is_empty() { None } else { Some(cache_path_raw) },
        failure_log_path: None,
        dry_run: config.dry_run,
        max_retries: config.max_retries,
        retry_delay_ms: 250,
        file_timeout_seconds: config.file_timeout_seconds,
    }
}

/// Mirrors `Path.GetInvalidFileNameChars()` on Windows (the shipped
/// target) - ported verbatim from `app/src/state.rs::sanitize_file_name`.
pub fn sanitize_file_name(name: &str) -> String {
    name.chars().map(|c| if is_invalid_windows_filename_char(c) { '_' } else { c }).collect()
}

fn is_invalid_windows_filename_char(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20
}

/// Live regex validation, computed via the SAME compile path a real run
/// takes (`search_core::matching::CompiledMatchState::build`) rather than
/// a separate hand-rolled check, so this can never disagree with what a
/// run would actually do. Ported from `app/src/state.rs::regex_validation_error`.
pub fn regex_validation_error(config: &SearchToolConfig) -> Option<String> {
    if !config.use_regex {
        return None;
    }
    let (filters, match_mode, proximity_lines, filter_groups) = resolve_filters(config);
    let settings = SearchSettings {
        filters,
        filter_groups,
        match_mode,
        proximity_lines,
        exclude_filters: parse_list(&config.exclude_filters_text),
        use_regex: true,
        ..Default::default()
    };
    search_core::matching::CompiledMatchState::build(&settings).err().map(|e| e.to_string())
}

/// Plain-text rendering of one file's hits (line number, matched filters,
/// the match line itself) - for the per-row "Export hits" action, pulling
/// one file's matches out on their own rather than the whole run's HTML
/// report. Ported verbatim from `app/src/state.rs::FileResultView::hits_as_text`.
pub fn hits_as_text(result: &FileSearchResult) -> String {
    let mut out = format!("{}\n{}\n\n", result.full_name, "=".repeat(result.full_name.len()));
    for hit in &result.hits {
        out.push_str(&format!(
            "Line {} (matched: {}):\n{}\n\n",
            hit.line_number,
            hit.matched_filters.join(", "),
            hit.match_line
        ));
    }
    out
}

/// The live-run data a background search task and the render loop both
/// touch - the plain-field equivalent of `app/`'s live-run `Signal` fields
/// (state.rs:229-268). Owned by the toolbox's integration point
/// (`toolboxes/search/mod.rs`), mutated only by `apply_progress`/
/// `finish_run` below and by the main event loop's `handle_event` reducer -
/// never read directly from a spawned background task (see the plan's
/// Event and State Model section for why that boundary matters).
#[derive(Debug, Clone, Default)]
pub struct SearchRunState {
    pub is_running: bool,
    pub progress_percent: f64,
    pub status_text: String,
    pub in_flight_files: Vec<InFlightFileStatus>,
    pub results: Vec<FileSearchResult>,
    pub results_summary_text: String,
    pub has_results: bool,
    pub last_report_path: Option<String>,
    /// When the run began (drives elapsed/rate/ETA in the progress panel).
    pub started: Option<std::time::Instant>,
    /// Final wall-clock time, set when the run ends.
    pub elapsed: Option<std::time::Duration>,
    pub files_completed: i32,
    pub total_files: i32,
}

impl SearchRunState {
    /// Seconds since the run began, or its final duration once finished.
    pub fn elapsed_secs(&self) -> Option<f64> {
        self.elapsed.or_else(|| self.started.map(|s| s.elapsed())).map(|d| d.as_secs_f64())
    }
}

/// Ported EXACTLY from `app/src/state.rs::AppState::apply_progress`
/// (state.rs:621-651) - this is the plan's single most important parity
/// requirement (root CLAUDE.md: per-file in-flight status is "a hard
/// requirement, not a nice-to-have"). Do not simplify this into a coarser
/// "start/done" update - see the module doc comment above.
pub fn apply_progress(run: &mut SearchRunState, report: SearchProgressReport) {
    if report.is_enumerating {
        run.status_text = if report.enumerated_file_count > 0 {
            format!("Scanning folders... {} file(s) found so far", report.enumerated_file_count)
        } else {
            "Scanning folders...".to_string()
        };
        return;
    }

    if report.total_files > 0 {
        run.files_completed = report.files_completed;
        run.total_files = report.total_files;
        run.progress_percent = 100.0 * report.files_completed as f64 / report.total_files as f64;
        run.status_text = format!(
            "{} of {} file(s) - {} hit(s) so far",
            report.files_completed, report.total_files, report.hits_so_far
        );
    }

    // Overwritten wholesale every report, exactly like the original - this
    // is the per-file "still working" ticker; a stale entry left behind
    // would misreport a file as in-flight after it actually finished.
    run.in_flight_files = report.in_flight_files;

    if let Some(r) = &report.last_completed_result {
        if r.status == FileSearchStatus::Hit {
            let already_present =
                run.results.iter().any(|existing| existing.full_name.eq_ignore_ascii_case(&r.full_name));
            if !already_present {
                run.results.push(r.clone());
                run.has_results = true;
            }
        }
    }
}

/// Folds one root's `SearchRunResult` into the running multi-root total.
/// Ported verbatim from `app/src/state.rs::merge_run_result` (state.rs:1151-1166).
pub fn merge_run_result(acc: &mut SearchRunResult, mut next: SearchRunResult) {
    acc.file_results.append(&mut next.file_results);
    acc.summary.files_searched += next.summary.files_searched;
    acc.summary.skipped_too_large += next.summary.skipped_too_large;
    acc.summary.skipped_binary += next.summary.skipped_binary;
    acc.summary.skipped_read_error += next.summary.skipped_read_error;
    acc.summary.skipped_by_exclude += next.summary.skipped_by_exclude;
    acc.summary.skipped_by_mode += next.summary.skipped_by_mode;
    acc.summary.skipped_unexpected_error += next.summary.skipped_unexpected_error;
    acc.summary.cache_reused += next.summary.cache_reused;
    acc.summary.enumeration_errors += next.summary.enumeration_errors;
    acc.summary.warnings.append(&mut next.summary.warnings);
    if let Some(mut cands) = next.dry_run_candidates.take() {
        acc.dry_run_candidates.get_or_insert_with(Vec::new).append(&mut cands);
    }
}

/// Human-readable aggregate summary line, e.g. "Skipped: 3 too large, 1
/// binary, 0 unreadable, 0 unexpected errors" - shown once a run finishes.
/// Not a verbatim port (neither existing head factors this into its own
/// standalone function - it's inlined into a larger `finish_successful_run`
/// method in both) but the wording matches `app/`'s exactly so the TUI's
/// summary line reads identically for the same run.
pub fn summarize(summary: &SearchRunSummary) -> String {
    format!(
        "Skipped: {} too large, {} binary, {} unreadable, {} unexpected errors",
        summary.skipped_too_large, summary.skipped_binary, summary.skipped_read_error, summary.skipped_unexpected_error
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use search_core::models::LineHit;

    fn config() -> SearchToolConfig {
        SearchToolConfig {
            search_path: "  /tmp/project  ".to_string(),
            filters_text: "apple, banana ,  , cherry".to_string(),
            exclude_filters_text: "skip".to_string(),
            exclude_folders_text: "node_modules, .git".to_string(),
            output_name: "  my report  ".to_string(),
            cache_file_path: String::new(),
            selected_extensions: Some(vec![".txt".to_string()]),
            ..SearchToolConfig::default()
        }
    }

    #[test]
    fn parse_list_splits_trims_and_drops_empties() {
        assert_eq!(parse_list("apple, banana ,  , cherry"), vec!["apple", "banana", "cherry"]);
        assert_eq!(parse_list(""), Vec::<String>::new());
        assert_eq!(parse_list("   "), Vec::<String>::new());
        assert_eq!(parse_list("single"), vec!["single"]);
    }

    #[test]
    fn build_settings_maps_every_field() {
        let cfg = config();
        let settings = build_settings(&cfg);
        assert_eq!(settings.search_path, "/tmp/project");
        assert_eq!(settings.filters, vec!["apple", "banana", "cherry"]);
        assert_eq!(settings.exclude_filters, vec!["skip"]);
        assert_eq!(settings.output_name, Some("my report".to_string()));
        assert_eq!(settings.extensions, Some(vec![".txt".to_string()]));
        assert_eq!(settings.cache_file_path, None);
        assert!(settings.exclude_folders.contains(&"node_modules".to_string()));
        assert!(settings.exclude_folders.contains(&".git".to_string()));
        assert_eq!(settings.max_embed_lines, 4000);
        assert_eq!(settings.retry_delay_ms, 250);
    }

    #[test]
    fn build_settings_excludes_the_fast_reindex_folder_unconditionally() {
        let settings = build_settings(&config());
        assert!(settings.exclude_folders.iter().any(|f| f.contains("native-search-index")));
    }

    #[test]
    fn sanitize_file_name_replaces_every_windows_invalid_char() {
        assert_eq!(sanitize_file_name(r#"a<b>c:d"e/f\g|h?i*j"#), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_file_name("normal-name_123"), "normal-name_123");
    }

    fn hit_result(full_name: &str) -> FileSearchResult {
        FileSearchResult {
            full_name: full_name.to_string(),
            status: FileSearchStatus::Hit,
            hits: vec![LineHit {
                line_number: 1,
                before: None,
                after: None,
                match_line: "hello".to_string(),
                matched_filters: vec!["hello".to_string()],
            }],
            created: Local::now(),
            modified: Local::now(),
            file_length: 10,
            lines_cache: Vec::new(),
            total_line_count: 1,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }
    }

    fn no_hit_result(full_name: &str) -> FileSearchResult {
        FileSearchResult { status: FileSearchStatus::NoHit, ..hit_result(full_name) }
    }

    #[test]
    fn filters_without_a_semicolon_are_one_group_exactly_as_before() {
        let g = parse_filter_groups("apple, banana");
        assert_eq!(g, vec![ParsedGroup { filters: vec!["apple".into(), "banana".into()], mode: None }]);
    }

    #[test]
    fn semicolons_split_groups_and_trailing_tags_set_the_mode() {
        let g = parse_filter_groups("house ; floor, two [near 3] ;; all, of [all] ; x [NEAR]");
        assert_eq!(g.len(), 4);
        assert_eq!(g[0].mode, None);
        assert_eq!(g[1].filters, vec!["floor", "two"]);
        assert_eq!(g[1].mode, Some(GroupMode::Match(MatchMode::Proximity, Some(3))));
        assert_eq!(g[2].mode, Some(GroupMode::Match(MatchMode::AllInFile, None)));
        assert_eq!(g[3].mode, Some(GroupMode::Match(MatchMode::Proximity, None)));
    }

    #[test]
    fn not_tags_make_exclusion_groups_and_never_become_the_primary() {
        let config = SearchToolConfig { filters_text: "draft [not] ; house ; wip [not line]".to_string(), ..Default::default() };
        let s = build_settings(&config);
        assert_eq!(s.filters, vec!["house"], "the first non-exclusion group is the primary");
        assert_eq!(s.filter_groups.len(), 2);
        assert_eq!(s.filter_groups[0].exclude, Some(ExcludeScope::File));
        assert_eq!(s.filter_groups[1].exclude, Some(ExcludeScope::Line));

        let only_not = build_settings(&SearchToolConfig { filters_text: "draft [not]".to_string(), ..Default::default() });
        assert!(only_not.filters.is_empty(), "an exclusion alone is not a search");
    }

    #[test]
    fn a_tag_directly_after_the_filter_text_is_recognized() {
        let g = parse_filter_groups("Test 1; Test 2; Test 4[not]");
        assert_eq!(g.len(), 3);
        assert_eq!(g[2].filters, vec!["Test 4"]);
        assert_eq!(g[2].mode, Some(GroupMode::Exclude(ExcludeScope::File)));
        assert_eq!(parse_filter_groups("a[near 2]")[0].mode, Some(GroupMode::Match(MatchMode::Proximity, Some(2))));
    }

    #[test]
    fn a_regex_character_class_is_not_mistaken_for_a_mode_tag() {
        let g = parse_filter_groups("colou[ru]");
        assert_eq!(g[0].filters, vec!["colou[ru]"]);
        assert_eq!(g[0].mode, None);
        let g = parse_filter_groups("a [near x]");
        assert_eq!(g[0].filters, vec!["a [near x]"], "unknown tag text stays part of the filter");
    }

    #[test]
    fn build_settings_resolves_primary_mode_and_extra_groups_against_the_global_mode() {
        let config = SearchToolConfig {
            filters_text: "house ; floor, two [near 3] ; roof".to_string(),
            match_mode: MatchMode::AllInFile,
            proximity_lines: 7,
            ..Default::default()
        };
        let s = build_settings(&config);
        assert_eq!(s.filters, vec!["house"]);
        assert_eq!(s.match_mode, MatchMode::AllInFile, "untagged primary uses the global mode");
        assert_eq!(s.filter_groups.len(), 2);
        assert_eq!((s.filter_groups[0].match_mode, s.filter_groups[0].proximity_lines), (MatchMode::Proximity, 3));
        assert_eq!((s.filter_groups[1].match_mode, s.filter_groups[1].proximity_lines), (MatchMode::AllInFile, 7));

        let tagged = SearchToolConfig { filters_text: "a, b [near 2]".to_string(), ..Default::default() };
        let s = build_settings(&tagged);
        assert_eq!((s.match_mode, s.proximity_lines), (MatchMode::Proximity, 2), "a tag on the first group sets the primary mode");
    }

    #[test]
    fn apply_progress_enumeration_sets_scanning_status_without_touching_progress() {
        let mut run = SearchRunState::default();
        apply_progress(
            &mut run,
            SearchProgressReport { is_enumerating: true, enumerated_file_count: 12, ..Default::default() },
        );
        assert_eq!(run.status_text, "Scanning folders... 12 file(s) found so far");
        assert_eq!(run.progress_percent, 0.0);
    }

    #[test]
    fn apply_progress_streams_incremental_hits_and_dedupes_by_path() {
        let mut run = SearchRunState::default();

        apply_progress(
            &mut run,
            SearchProgressReport {
                files_completed: 1,
                total_files: 4,
                hits_so_far: 1,
                last_completed_result: Some(hit_result("a.txt")),
                ..Default::default()
            },
        );
        assert_eq!(run.progress_percent, 25.0);
        assert_eq!(run.status_text, "1 of 4 file(s) - 1 hit(s) so far");
        assert_eq!(run.results.len(), 1);
        assert!(run.has_results);

        // A non-hit result must not be appended to `results`.
        apply_progress(
            &mut run,
            SearchProgressReport {
                files_completed: 2,
                total_files: 4,
                hits_so_far: 1,
                last_completed_result: Some(no_hit_result("b.txt")),
                ..Default::default()
            },
        );
        assert_eq!(run.results.len(), 1);

        // A duplicate path (case-insensitive) must not be appended twice.
        apply_progress(
            &mut run,
            SearchProgressReport {
                files_completed: 3,
                total_files: 4,
                hits_so_far: 2,
                last_completed_result: Some(hit_result("A.TXT")),
                ..Default::default()
            },
        );
        assert_eq!(run.results.len(), 1);
    }

    #[test]
    fn apply_progress_overwrites_in_flight_files_wholesale() {
        let mut run = SearchRunState::default();
        run.in_flight_files = vec![InFlightFileStatus {
            file_name: "stale.pdf".to_string(),
            status_text: "Reading...".to_string(),
            elapsed_seconds: 99.0,
        }];
        apply_progress(
            &mut run,
            SearchProgressReport {
                files_completed: 1,
                total_files: 2,
                in_flight_files: vec![InFlightFileStatus {
                    file_name: "fresh.pdf".to_string(),
                    status_text: "Extracting PDF text - 2 stream(s) scanned".to_string(),
                    elapsed_seconds: 1.2,
                }],
                ..Default::default()
            },
        );
        assert_eq!(run.in_flight_files.len(), 1);
        assert_eq!(run.in_flight_files[0].file_name, "fresh.pdf");
    }

    #[test]
    fn merge_run_result_sums_counters_and_concatenates_files() {
        let mut acc = SearchRunResult {
            file_results: vec![hit_result("a.txt")],
            summary: SearchRunSummary { files_searched: 3, skipped_binary: 1, ..Default::default() },
            ..Default::default()
        };
        let next = SearchRunResult {
            file_results: vec![hit_result("b.txt")],
            summary: SearchRunSummary { files_searched: 2, skipped_binary: 4, ..Default::default() },
            ..Default::default()
        };
        merge_run_result(&mut acc, next);
        assert_eq!(acc.file_results.len(), 2);
        assert_eq!(acc.summary.files_searched, 5);
        assert_eq!(acc.summary.skipped_binary, 5);
    }

    #[test]
    fn merge_run_result_concatenates_dry_run_candidates() {
        let mut acc = SearchRunResult {
            was_dry_run: true,
            dry_run_candidates: Some(vec![std::path::PathBuf::from("a.txt")]),
            ..Default::default()
        };
        let next = SearchRunResult {
            was_dry_run: true,
            dry_run_candidates: Some(vec![std::path::PathBuf::from("b.txt")]),
            ..Default::default()
        };
        merge_run_result(&mut acc, next);
        assert_eq!(acc.dry_run_candidates.unwrap().len(), 2);
    }

    #[test]
    fn regex_validation_is_none_when_regex_is_off() {
        let config = SearchToolConfig { use_regex: false, filters_text: "(unbalanced".to_string(), ..Default::default() };
        assert_eq!(regex_validation_error(&config), None);
    }

    #[test]
    fn regex_validation_catches_an_invalid_pattern() {
        let config = SearchToolConfig { use_regex: true, filters_text: "(unbalanced".to_string(), ..Default::default() };
        assert!(regex_validation_error(&config).is_some());
    }

    #[test]
    fn regex_validation_passes_a_valid_pattern() {
        let config = SearchToolConfig { use_regex: true, filters_text: r"\d+".to_string(), ..Default::default() };
        assert_eq!(regex_validation_error(&config), None);
    }

    #[test]
    fn hits_as_text_includes_header_and_every_hit() {
        let text = hits_as_text(&hit_result("a.txt"));
        assert!(text.starts_with("a.txt\n=====\n\n"));
        assert!(text.contains("Line 1 (matched: hello):"));
        assert!(text.contains("hello"));
    }

    #[test]
    fn summarize_matches_documented_wording() {
        let summary = SearchRunSummary {
            skipped_too_large: 3,
            skipped_binary: 1,
            skipped_read_error: 0,
            skipped_unexpected_error: 0,
            ..Default::default()
        };
        assert_eq!(summarize(&summary), "Skipped: 3 too large, 1 binary, 0 unreadable, 0 unexpected errors");
    }
}
