//! Ports `TextInFilesSearch.Core/Services/ReportExportService.cs`: exports a
//! completed search run as a self-contained HTML report (with the same
//! dark-mode CSS, table of contents, and per-filter bar chart as the
//! PowerShell tool's report) and/or flat CSV/JSON files, one row per hit.

use std::collections::HashMap;
use std::sync::OnceLock;

use chrono::{DateTime, Datelike, Local};
use fancy_regex::RegexBuilder as FancyRegexBuilder;
use serde::Serialize;

use crate::matching::whole_word_pattern;
use crate::models::{FileSearchResult, FileSearchStatus, GroupByMode, LineHit, MatchMode, SearchRunResult, SearchSettings};

/// Base64-encoded GS Engineering banner, embedded as a data URI in every
/// report so it stays a single self-contained file. Unlike the C# original
/// (which loads this from an embedded assembly resource and falls back to
/// no banner if missing at runtime), `include_bytes!` embeds it at compile
/// time - if the asset were ever missing, the build itself would fail
/// rather than silently producing a bannerless report.
const BANNER_JPG: &[u8] = include_bytes!("../assets/Banner.jpg");

fn banner_data_uri() -> &'static str {
    static URI: OnceLock<String> = OnceLock::new();
    URI.get_or_init(|| {
        use base64::Engine;
        format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(BANNER_JPG)
        )
    })
}

/// Where a report's HTML text actually goes - `Buffer` accumulates
/// everything into one `String` (what [`build_html_report`] returns, used
/// by tests and any caller that genuinely needs the whole report as a
/// string), `Writer` streams it straight to a file (what
/// [`write_html_report`] uses - the actual production report-writing
/// path). Both drive the exact same generation logic
/// ([`write_report_to_sink`]) - the sink is the only thing that differs.
enum ReportSink<'a> {
    Buffer(&'a mut String),
    Writer(&'a mut dyn std::io::Write),
}

impl ReportSink<'_> {
    /// Moves `chunk`'s content to the sink and empties `chunk` either way
    /// - appended for `Buffer` (which needs everything resident anyway),
    /// written straight to disk and dropped for `Writer`. This second
    /// case is the actual streaming behavior epic #6 §35 asks for
    /// ("write result / write result / write result... do not construct
    /// massive HTML strings in memory"): `chunk` is called at each
    /// natural boundary - after the header, after the table of contents,
    /// after every single file block - so at most one such piece of
    /// formatted HTML is ever resident at once when writing to a file,
    /// not the whole report.
    fn commit(&mut self, chunk: &mut String) -> std::io::Result<()> {
        match self {
            ReportSink::Buffer(buf) => buf.push_str(chunk),
            ReportSink::Writer(w) => w.write_all(chunk.as_bytes())?,
        }
        chunk.clear();
        Ok(())
    }
}

/// Builds the full HTML report as one in-memory `String` - for tests and
/// any caller that genuinely needs the whole report as a string, not a
/// file. The actual production report-writing path
/// (`app/src/state.rs`'s `finish_successful_run`) uses
/// [`write_html_report`] instead, which streams to disk without ever
/// holding the whole formatted report in memory - see that function's
/// doc comment.
pub fn build_html_report(settings: &SearchSettings, run: &SearchRunResult) -> String {
    let mut result = String::new();
    write_report_to_sink(&mut ReportSink::Buffer(&mut result), settings, run)
        .expect("writing into a String sink is infallible");
    result
}

/// Streams the HTML report directly to `path` - see [`ReportSink::commit`]
/// for what "streams" means here concretely. Returns the written file's
/// final byte size (from the filesystem, not a pre-computed in-memory
/// length) so callers can warn on a very large report without needing to
/// have held the whole thing in memory first just to call `.len()` on it.
pub fn write_html_report(path: &str, settings: &SearchSettings, run: &SearchRunResult) -> std::io::Result<u64> {
    let start = std::time::Instant::now();
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    write_report_to_sink(&mut ReportSink::Writer(&mut writer), settings, run)?;
    std::io::Write::flush(&mut writer)?;
    drop(writer);
    tracing::info!(path = %path, elapsed_ms = start.elapsed().as_millis() as u64, "export: HTML report written");
    Ok(std::fs::metadata(path)?.len())
}

fn write_report_to_sink(sink: &mut ReportSink, settings: &SearchSettings, run: &SearchRunResult) -> std::io::Result<()> {
    let mut out = String::new();
    let colors = FilterColors::new(settings);

    // Files with hits in display order: by path, or newest first when grouped by a date.
    let mut hit_results: Vec<&FileSearchResult> =
        run.file_results.iter().filter(|r| r.status == FileSearchStatus::Hit).collect();
    if settings.group_by == GroupByMode::None {
        hit_results.sort_by_cached_key(|r| r.full_name.to_lowercase());
    } else {
        hit_results.sort_by(|a, b| group_date(b, settings.group_by).cmp(&group_date(a, settings.group_by)));
    }
    let total_hits: usize = hit_results.iter().map(|r| r.hits.len()).sum();

    out.push_str("<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"UTF-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str("<title>Text Search Report</title>\n");
    out.push_str(CSS_BLOCK);
    out.push('\n');
    out.push_str(HEAD_SCRIPT);
    out.push_str("\n</head><body>\n");
    append_masthead(&mut out, settings, run);
    let has_hits = !hit_results.is_empty();
    out.push_str(if has_hits { "<div class=\"shell\">\n" } else { "<div class=\"shell solo\">\n" });
    if has_hits {
        append_rail(&mut out, &hit_results);
    }
    out.push_str("<main>\n");
    sink.commit(&mut out)?;

    append_kpis(&mut out, run, hit_results.len(), total_hits);
    append_definition_and_filter_bars(&mut out, settings, &colors, &hit_results, total_hits);
    if has_hits {
        append_insights(&mut out, settings, run, &hit_results, total_hits);
        append_toolbar(&mut out, settings, &hit_results);
        sink.commit(&mut out)?;
        out.push_str("<div class=\"count-line\" id=\"count\"></div>\n<section id=\"files\" aria-label=\"Results\">\n");
        for (idx, r) in hit_results.iter().enumerate() {
            append_file_block(&mut out, r, settings, &colors, idx + 1);
            sink.commit(&mut out)?;
        }
        out.push_str("</section>\n");
    } else {
        out.push_str("<p class=\"no-hits\">No matches found.</p>\n");
    }

    out.push_str("<footer>Generated by Toolbench Search &middot; one self-contained file, no network access</footer>\n");
    out.push_str("</main></div>\n<div class=\"toast\" id=\"toast\" role=\"status\"></div>\n");
    out.push_str(REPORT_SCRIPT);
    out.push_str("\n</body></html>\n");
    sink.commit(&mut out)?;
    Ok(())
}

/// How many distinct chart/mark colours the stylesheet defines (`--c1`..`--c6`, `mark.m2`..`mark.m6`).
const COLOR_SLOTS: usize = 6;

/// The colour slot of each filter, so a filter has the same colour in its chip, its bar and its highlights.
pub(crate) struct FilterColors(Vec<String>);

impl FilterColors {
    pub(crate) fn new(settings: &SearchSettings) -> Self {
        Self(settings.all_filters().map(|f| f.to_lowercase()).collect())
    }

    pub(crate) fn slot(&self, filter: &str) -> usize {
        let lower = filter.to_lowercase();
        self.0.iter().position(|f| *f == lower).unwrap_or(0) % COLOR_SLOTS
    }
}

fn color_var(slot: usize) -> String {
    format!("var(--c{})", slot % COLOR_SLOTS + 1)
}

fn group_date(r: &FileSearchResult, mode: GroupByMode) -> DateTime<Local> {
    if mode == GroupByMode::Created {
        r.created
    } else {
        r.modified
    }
}

fn append_masthead(out: &mut String, settings: &SearchSettings, run: &SearchRunResult) {
    out.push_str("<header class=\"masthead\"><div class=\"masthead-in\"><div class=\"brand\">");
    out.push_str(&format!(
        "<img class=\"report-banner\" src=\"{}\" alt=\"GS Engineering\" />",
        banner_data_uri()
    ));
    out.push_str("<div><div class=\"eyebrow\">GS Engineering &middot; Toolbench Search</div><h1>Text Search Report</h1></div></div>\n");
    out.push_str(&format!(
        "<div class=\"run-meta\"><span class=\"pill\">Run <b class=\"run-time\">{}</b></span>",
        Local::now().format("%Y-%m-%d %H:%M:%S")
    ));
    if run.summary.total_elapsed_seconds > 0.0 {
        out.push_str(&format!("<span class=\"pill\">Elapsed <b>{:.2}s</b></span>", run.summary.total_elapsed_seconds));
    }
    if settings.parallel {
        out.push_str("<span class=\"pill\">Parallel <b>on</b></span>");
    }
    out.push_str("</div></div></header>\n");
}

fn append_rail(out: &mut String, hit_results: &[&FileSearchResult]) {
    out.push_str(&format!(
        "<aside class=\"rail\" aria-label=\"Files with hits\"><h2>Files with hits <span id=\"railCount\">{}</span></h2>\n<nav class=\"rail-list\" id=\"rail\">\n",
        hit_results.len()
    ));
    let density = |r: &FileSearchResult| r.hits.len() as f64 / (r.total_line_count.max(0) as usize).max(r.lines_cache.len()).max(1) as f64;
    let densest = hit_results.iter().map(|r| density(r)).fold(0.0_f64, f64::max);
    for (idx, r) in hit_results.iter().enumerate() {
        let ext = ext_of(&r.full_name);
        // The bar under a file's name is its hit density against the densest file: where the hits are packed tight.
        let heat = if densest > 0.0 { (100.0 * density(r) / densest).clamp(6.0, 100.0) } else { 6.0 };
        out.push_str(&format!(
            "<a href=\"#file-{n}\" data-id=\"file-{n}\" style=\"--heat:{heat:.0}%\"><span class=\"ftype sm\" style=\"background:{}\">{}</span><span class=\"nm\">{}</span><span class=\"n\">{}</span></a>\n",
            type_color(&ext),
            type_label(&ext),
            html_escape(&file_name_of(&r.full_name)),
            r.hits.len(),
            n = idx + 1
        ));
    }
    out.push_str(
        "</nav><div class=\"rail-foot js-only\"><button class=\"btn\" id=\"themeBtn\" title=\"Toggle light / dark\">&#9680; Theme</button><button class=\"btn\" id=\"printBtn\" title=\"Print or save as PDF\">&#9113; Print</button></div></aside>\n",
    );
}

fn append_kpis(out: &mut String, run: &SearchRunResult, hit_files: usize, total_hits: usize) {
    let s = &run.summary;
    // `files_searched` leaves out files that could not be read at all, so coverage compares it with those.
    let unread = (s.skipped_too_large + s.skipped_read_error + s.skipped_unexpected_error) as f64;
    let considered = s.files_searched as f64 + unread;
    let coverage = if considered > 0.0 { 100.0 * s.files_searched as f64 / considered } else { 100.0 };
    let searched = s.files_searched.max(0) as f64;
    let hit_share = if searched > 0.0 { format!("{:.2} % of files searched", 100.0 * hit_files as f64 / searched) } else { "no files searched".to_string() };
    let per_file = if hit_files > 0 { format!("{:.1} hits per matching file", total_hits as f64 / hit_files as f64) } else { "no matching files".to_string() };
    let cached = if s.cache_reused > 0 { format!("{} reused from cache", s.cache_reused) } else { "read this run".to_string() };
    let unread_note = if unread > 0.0 {
        format!("{unread:.0} not read (too large, locked or unreadable)")
    } else {
        "every file was read".to_string()
    };
    let kpi = |label: &str, value: String, sub: String| {
        format!("<div class=\"card kpi\"><div class=\"lbl\">{label}</div><div class=\"val\">{value}</div><div class=\"sub\">{sub}</div></div>\n")
    };
    out.push_str("<section class=\"grid kpis\" aria-label=\"Summary\">\n");
    out.push_str(&kpi("Files searched", group_thousands(s.files_searched.max(0) as u64), cached));
    out.push_str(&kpi("Files with hits", group_thousands(hit_files as u64), hit_share));
    out.push_str(&kpi("Total hits", group_thousands(total_hits as u64), per_file));
    out.push_str(&kpi("Coverage", format!("{coverage:.1}<span class=\"unit\"> %</span>"), unread_note));
    out.push_str("</section>\n");
}

fn append_definition_and_filter_bars(
    out: &mut String,
    settings: &SearchSettings,
    colors: &FilterColors,
    hit_results: &[&FileSearchResult],
    total_hits: usize,
) {
    let mut notes = Vec::new();
    if settings.use_regex {
        notes.push("regex");
    } else if settings.whole_word {
        notes.push("whole word");
    }
    let note = if notes.is_empty() { String::new() } else { format!(" <span class=\"muted\">({})</span>", notes.join(", ")) };
    let chips: String = settings
        .all_filters()
        .map(|f| format!("<span class=\"chip\"><i class=\"dot\" style=\"background:{}\"></i>{}</span>", color_var(colors.slot(f)), html_escape(f)))
        .collect();
    let proximity = if settings.match_mode == MatchMode::Proximity {
        format!(" (within {} line(s))", settings.proximity_lines)
    } else {
        String::new()
    };

    out.push_str("<section class=\"grid two\">\n<div class=\"card\"><div class=\"card-h\"><h3>Search definition</h3></div><div class=\"card-b\"><dl class=\"defs\">\n");
    let row = |out: &mut String, label: &str, value: &str| {
        out.push_str(&format!("<div class=\"qrow\"><dt>{label}</dt><dd>{value}</dd></div>\n"));
    };
    row(out, "Folder", &format!("<span class=\"mono\">{}</span>", html_escape(&settings.search_path)));
    row(out, "Filters", &format!("{chips}{note}"));
    row(out, "Match mode", &format!("{:?}{proximity}", settings.match_mode));
    if !settings.exclude_filters.is_empty() {
        let excluded: String = settings.exclude_filters.iter().map(|f| format!("<span class=\"chip\">{}</span>", html_escape(f))).collect();
        row(out, "Excluding", &format!("{excluded} <span class=\"muted\">(scope: {:?})</span>", settings.exclude_scope));
    }
    row(out, "Grouped by", &format!("{:?}", settings.group_by));
    out.push_str("</dl></div></div>\n");

    // Case-insensitive aggregation by filter *text* (not slot) - two
    // filters differing only by case share one bar-chart bucket, matching
    // the C# side's `Dictionary<string,int>(StringComparer.OrdinalIgnoreCase)`.
    let mut aggregate_counts: HashMap<String, i32> = HashMap::new();
    for r in hit_results {
        for h in &r.hits {
            for mf in &h.matched_filters {
                *aggregate_counts.entry(mf.to_lowercase()).or_insert(0) += 1;
            }
        }
    }
    out.push_str(&format!("<div class=\"card\"><div class=\"card-h\"><h3>Hits by filter</h3><span>{total_hits} hits</span></div><div class=\"card-b\">\n"));
    if aggregate_counts.is_empty() {
        out.push_str("<p class=\"muted\">No filter matched.</p>\n");
    } else {
        let max_count = *aggregate_counts.values().max().unwrap();
        for f in settings.all_filters() {
            let c = *aggregate_counts.get(&f.to_lowercase()).unwrap_or(&0);
            let pct = if max_count > 0 { (100.0 * c as f64 / max_count as f64) as i32 } else { 0 };
            let f_html = html_escape(f);
            out.push_str(&format!(
                "<div class=\"bar-row\"><span class=\"bar-label\" title=\"{f_html}\"><i class=\"dot\" style=\"background:{col}\"></i> {f_html}</span><span class=\"bar-track\"><span class=\"bar-fill\" style=\"width:{pct}%;background:{col}\"></span></span><span class=\"bar-count\">{c}</span></div>\n",
                col = color_var(colors.slot(f))
            ));
        }
    }
    out.push_str("</div></div>\n</section>\n");
}

fn append_insights(
    out: &mut String,
    settings: &SearchSettings,
    run: &SearchRunResult,
    hit_results: &[&FileSearchResult],
    total_hits: usize,
) {
    out.push_str("<section class=\"grid three\">\n");

    // File types by hit count: five largest plus the rest.
    let mut by_ext: HashMap<String, usize> = HashMap::new();
    for r in hit_results {
        *by_ext.entry(ext_of(&r.full_name)).or_insert(0) += r.hits.len();
    }
    let mut types: Vec<(String, usize)> = by_ext.into_iter().collect();
    types.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if types.len() > COLOR_SLOTS {
        let rest: usize = types.split_off(COLOR_SLOTS - 1).iter().map(|t| t.1).sum();
        types.push((String::new(), rest));
    }
    out.push_str("<div class=\"card\"><div class=\"card-h\"><h3>File types</h3><span>by hit count</span></div><div class=\"card-b\"><div class=\"donut-wrap\">");
    out.push_str("<svg width=\"130\" height=\"130\" viewBox=\"0 0 42 42\" role=\"img\" aria-label=\"Hits by file type\"><circle cx=\"21\" cy=\"21\" r=\"15.9\" fill=\"none\" stroke=\"var(--surface-2)\" stroke-width=\"6\"/>");
    let mut offset = 25.0_f64;
    let mut legend = String::new();
    for (i, (ext, hits)) in types.iter().enumerate() {
        let pct = 100.0 * *hits as f64 / total_hits.max(1) as f64;
        out.push_str(&format!(
            "<circle cx=\"21\" cy=\"21\" r=\"15.9\" fill=\"none\" stroke=\"{}\" stroke-width=\"6\" stroke-dasharray=\"{:.2} {:.2}\" stroke-dashoffset=\"{offset:.2}\"/>",
            color_var(i),
            (pct - 0.6).max(0.1),
            100.0 - pct + 0.6
        ));
        offset -= pct;
        let label = if ext.is_empty() { "other".to_string() } else { format!(".{}", html_escape(ext)) };
        legend.push_str(&format!("<span><i class=\"dot\" style=\"background:{}\"></i> {label} <b>{hits}</b></span>", color_var(i)));
    }
    out.push_str(&format!(
        "<text x=\"21\" y=\"22.5\" text-anchor=\"middle\" font-size=\"7\" font-weight=\"700\" fill=\"var(--ink)\">{total_hits}</text></svg><div class=\"legend col\">{legend}</div></div></div></div>\n"
    ));

    // Files by year of the date the report is grouped by (modified when ungrouped), newest twelve years.
    let date_label = if settings.group_by == GroupByMode::Created { "created" } else { "modified" };
    let mut by_year: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
    for r in hit_results {
        *by_year.entry(group_date(r, settings.group_by).year()).or_insert(0) += 1;
    }
    let years: Vec<(i32, usize)> = by_year.into_iter().rev().take(12).rev().collect();
    let max_year = years.iter().map(|y| y.1).max().unwrap_or(1) as f64;
    let width = (14 + years.len() * 50).max(260);
    out.push_str(&format!(
        "<div class=\"card\"><div class=\"card-h\"><h3>Activity by year</h3><span>files {date_label}</span></div><div class=\"card-b\"><svg viewBox=\"0 0 {width} 120\" width=\"100%\" preserveAspectRatio=\"xMinYMid meet\" role=\"img\" aria-label=\"Files by year\">"
    ));
    for (i, (year, count)) in years.iter().enumerate() {
        let x = 14 + i * 50;
        let h = *count as f64 / max_year * 78.0;
        out.push_str(&format!(
            "<rect x=\"{x}\" y=\"{:.1}\" width=\"36\" height=\"{h:.1}\" rx=\"6\" fill=\"var(--c2)\" opacity=\".85\"/><text x=\"{}\" y=\"112\" text-anchor=\"middle\" font-size=\"10\" fill=\"var(--ink-3)\">{year}</text><text x=\"{}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"10\" font-weight=\"700\" fill=\"var(--ink)\">{count}</text>",
            96.0 - h,
            x + 18,
            x + 18,
            90.0 - h
        ));
    }
    out.push_str("</svg></div></div>\n");

    // Coverage: what was skipped and which files raised warnings.
    let s = &run.summary;
    let skips: [(&str, i32, &str); 6] = [
        ("too large", s.skipped_too_large, "var(--c3)"),
        ("binary", s.skipped_binary, "var(--c4)"),
        ("unreadable, locked or unsupported", s.skipped_read_error, "var(--c6)"),
        ("with unexpected errors", s.skipped_unexpected_error, "var(--bad)"),
        ("excluded", s.skipped_by_exclude, "var(--c1)"),
        ("missing required filters", s.skipped_by_mode, "var(--c2)"),
    ];
    let skipped: i32 = skips.iter().map(|k| k.1).sum();
    out.push_str(&format!("<div class=\"card\" id=\"skipped\"><div class=\"card-h\"><h3>Coverage</h3><span>{skipped} skipped</span></div><div class=\"card-b\">"));
    if skipped > 0 {
        out.push_str("<div class=\"segbar\">");
        for (label, n, col) in skips.iter().filter(|k| k.1 > 0) {
            out.push_str(&format!("<i style=\"flex:{n};background:{col}\" title=\"{n} {label}\"></i>"));
        }
        out.push_str("</div><div class=\"legend\">");
        for (label, n, col) in skips.iter().filter(|k| k.1 > 0) {
            out.push_str(&format!("<span><i class=\"dot\" style=\"background:{col}\"></i> {n} {label}</span>"));
        }
        out.push_str("</div>");
    } else {
        out.push_str("<p class=\"muted\">No files were skipped.</p>");
    }
    const SHOWN_WARNINGS: usize = 6;
    if !s.warnings.is_empty() {
        out.push_str("<div class=\"warnings\">");
        for w in s.warnings.iter().take(SHOWN_WARNINGS) {
            out.push_str(&format!(
                "<div class=\"w\"><span class=\"badge warn\">Error</span><span><code>{}</code> &mdash; {}</span></div>",
                html_escape(&w.full_name),
                html_escape(&w.message)
            ));
        }
        if s.warnings.len() > SHOWN_WARNINGS {
            out.push_str(&format!("<div class=\"w muted\">and {} more</div>", s.warnings.len() - SHOWN_WARNINGS));
        }
        out.push_str("</div>");
    }
    out.push_str("</div></div>\n</section>\n");
}

fn append_toolbar(out: &mut String, settings: &SearchSettings, hit_results: &[&FileSearchResult]) {
    let mut by_ext: HashMap<String, usize> = HashMap::new();
    for r in hit_results {
        *by_ext.entry(ext_of(&r.full_name)).or_insert(0) += 1;
    }
    let mut exts: Vec<(String, usize)> = by_ext.into_iter().collect();
    exts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let date_sort = if settings.group_by == GroupByMode::Created { "Sort: newest created" } else { "Sort: newest modified" };
    out.push_str("<div class=\"toolbar js-only\" role=\"search\">\n");
    out.push_str("<label class=\"search\"><span aria-hidden=\"true\">&#8981;</span><input id=\"q\" type=\"search\" placeholder=\"Filter files and matches in this report\" aria-label=\"Filter report\"><kbd>/</kbd></label>\n");
    out.push_str("<div class=\"seg\" id=\"types\" aria-label=\"File type filter\">");
    for (ext, _) in &exts {
        let label = if ext.is_empty() { "(none)".to_string() } else { format!(".{}", html_escape(ext)) };
        out.push_str(&format!("<button aria-pressed=\"true\" data-ext=\"{}\">{label}</button>", html_escape(ext)));
    }
    out.push_str("</div>\n");
    out.push_str(&format!(
        "<select class=\"btn\" id=\"sort\" aria-label=\"Sort files\"><option value=\"default\">Sort: report order</option><option value=\"path\">Sort: path</option><option value=\"hits\">Sort: most hits</option><option value=\"date\">{date_sort}</option><option value=\"density\">Sort: hit density</option></select>\n"
    ));
    out.push_str("<button class=\"btn\" id=\"expand\">Expand all</button><button class=\"btn\" id=\"collapse\">Collapse all</button>\n</div>\n");
}

fn append_file_block(out: &mut String, r: &FileSearchResult, settings: &SearchSettings, colors: &FilterColors, number: usize) {
    let safe_path = html_escape(&r.full_name);
    let uri = file_uri(&r.full_name);
    let hit_count = r.hits.len();
    let hit_word = if hit_count == 1 { "hit" } else { "hits" };
    let ext = ext_of(&r.full_name);
    let name = file_name_of(&r.full_name);
    let dir = r.full_name.strip_suffix(name.as_str()).unwrap_or("");
    let line_total = (r.total_line_count.max(0) as usize).max(r.lines_cache.len()).max(1);

    let hit_line_map: HashMap<i32, &Vec<String>> = r.hits.iter().map(|h| (h.line_number, &h.matched_filters)).collect();

    out.push_str(&format!(
        "<details class=\"file\" id=\"file-{number}\" data-i=\"{number}\" data-name=\"{}\" data-ext=\"{}\" data-hits=\"{hit_count}\" data-date=\"{}\" data-density=\"{:.4}\">\n",
        html_escape(&r.full_name.to_lowercase()),
        html_escape(&ext),
        group_date(r, settings.group_by).format("%Y-%m-%d"),
        hit_count as f64 / line_total as f64
    ));
    out.push_str(&format!(
        "<summary><div class=\"ftype\" style=\"background:{}\">{}</div><div class=\"fmain\"><div class=\"fname\">{}</div><div class=\"fpath\"><bdi>{}</bdi></div></div><div class=\"fright\">",
        type_color(&ext),
        type_label(&ext),
        html_escape(&name),
        html_escape(dir)
    ));
    if let Some(label) = r.pdf_warning() {
        out.push_str(&format!("<span class=\"badge warn\" title=\"OCR text confidence\">{}</span>", html_escape(&label)));
    }
    out.push_str(&hit_strip(r, line_total));
    out.push_str(&format!("<span class=\"badge\">{hit_count} {hit_word}</span><span class=\"chev\">&rsaquo;</span></div></summary>\n"));

    out.push_str("<div class=\"fbody\">\n<div class=\"fmeta\">");
    out.push_str(&format!(
        "<span>Created <b>{}</b></span><span>Modified <b>{}</b></span><span>Size <b>{}</b></span>",
        r.created.format("%Y-%m-%d %H:%M"),
        r.modified.format("%Y-%m-%d %H:%M"),
        human_size(r.file_length)
    ));
    if let Some(slides) = pptx_slides(r) {
        out.push_str(&format!("<span>Location <b>{slides}</b></span>"));
    }

    let mut per_filter_counts: HashMap<String, i32> = HashMap::new();
    for h in &r.hits {
        for mf in &h.matched_filters {
            *per_filter_counts.entry(mf.to_lowercase()).or_insert(0) += 1;
        }
    }
    for f in settings.all_filters().filter(|f| per_filter_counts.contains_key(&f.to_lowercase())) {
        out.push_str(&format!(
            "<span class=\"chip\" data-slot=\"{}\"><i class=\"dot\" style=\"background:{}\"></i>{} {}</span>",
            colors.slot(f),
            color_var(colors.slot(f)),
            html_escape(f),
            per_filter_counts[&f.to_lowercase()]
        ));
    }
    out.push_str(&format!(
        "<span class=\"sp\"></span><button class=\"btn js-only\" data-act=\"prev\">&uarr; Prev hit</button><button class=\"btn js-only\" data-act=\"next\">&darr; Next hit</button><button class=\"btn js-only\" data-act=\"copy\" data-path=\"{safe_path}\">Copy path</button><a class=\"btn filelink\" href=\"{uri}\">Open original &#8599;</a></div>\n"
    ));

    if r.low_confidence_pdf {
        let scored = match (r.ocr_percent(), r.ocr_is_poor()) {
            (Some(pct), true) => format!(" It was read by OCR with a low model confidence of {pct}%."),
            (Some(pct), false) => format!(" It was read by OCR with a model confidence of {pct}%."),
            (None, _) => String::new(),
        };
        out.push_str(&format!(
            "<p class=\"note warn confidence-note\">This PDF's extracted text may be incomplete or misrecognized - check the original if the matches are important.{scored}</p>\n",
        ));
    }

    if settings.match_mode == MatchMode::AllInFile {
        out.push_str(&format!(
            "<p class=\"note info truncate-note\">All required filters were found somewhere in this file: {}</p>\n",
            html_escape(&settings.filters.join(", "))
        ));
    } else if settings.match_mode == MatchMode::Proximity {
        let mr_text = match r.proximity_min_range {
            Some(mr) => format!("{mr} line(s)"),
            None => "unknown".to_string(),
        };
        out.push_str(&format!(
            "<p class=\"note info truncate-note\">All required filters found within {mr_text} of each other (limit: {}): {}</p>\n",
            settings.proximity_lines,
            html_escape(&settings.filters.join(", "))
        ));
    }

    let truncated = r.total_line_count as usize > r.lines_cache.len();
    if truncated {
        out.push_str(&format!(
            "<p class=\"note warn truncate-note\">Showing lines 1-{} of {} total extracted lines below. Open the original file to see the rest.</p>\n",
            r.lines_cache.len(),
            r.total_line_count
        ));
    }

    if !r.lines_cache.is_empty() {
        out.push_str(&format!(
            "<div class=\"code\"><div class=\"code-h\"><span>{hit_count} matching {} &middot; {} extracted lines</span></div><div class=\"cwrap\"><div class=\"lines\">\n",
            if hit_count == 1 { "line" } else { "lines" },
            r.lines_cache.len()
        ));
        for (idx, raw_line) in r.lines_cache.iter().enumerate() {
            let ln = (idx + 1) as i32;
            if let Some(matched_filters) = hit_line_map.get(&ln) {
                let formatted = highlight_matches(raw_line, matched_filters, settings, colors);
                out.push_str(&format!("<div class=\"ln hit\"><i>{ln}</i><span>{formatted}</span></div>\n"));
            } else {
                out.push_str(&format!("<div class=\"ln\"><i>{ln}</i><span>{}</span></div>\n", html_escape(raw_line)));
            }
        }
        out.push_str("</div></div></div>\n");
    }

    let beyond_hits: Vec<&LineHit> = r.hits.iter().filter(|h| h.line_number as usize > r.lines_cache.len()).collect();
    if !beyond_hits.is_empty() {
        out.push_str("<div class=\"code\"><div class=\"code-h\"><span>Additional hit(s) beyond the shown preview</span></div><div class=\"cwrap\"><div class=\"lines\">\n");
        for h in beyond_hits {
            if let Some(before) = &h.before {
                out.push_str(&format!("<div class=\"ln ctx\"><i></i><span>{}</span></div>\n", html_escape(before)));
            }
            out.push_str(&format!(
                "<div class=\"ln hit\"><i>{}</i><span>{}</span></div>\n",
                h.line_number,
                highlight_matches(&h.match_line, &h.matched_filters, settings, colors)
            ));
            if let Some(after) = &h.after {
                out.push_str(&format!("<div class=\"ln ctx\"><i></i><span>{}</span></div>\n", html_escape(after)));
            }
        }
        out.push_str("</div></div></div>\n");
    }

    out.push_str("</div></details>\n");
}

/// A small strip showing where in the file the hits are, one tick per distinct horizontal position.
fn hit_strip(r: &FileSearchResult, line_total: usize) -> String {
    const WIDTH: usize = 84;
    let mut xs: Vec<usize> = r
        .hits
        .iter()
        .map(|h| {
            let frac = (h.line_number.max(1) as f64 / line_total.max(h.line_number.max(1) as usize) as f64).min(1.0);
            4 + (frac * (WIDTH - 8) as f64) as usize
        })
        .collect();
    xs.sort_unstable();
    xs.dedup();
    let mut svg = format!("<svg class=\"heat\" viewBox=\"0 0 {WIDTH} 22\" aria-hidden=\"true\"><rect width=\"{WIDTH}\" height=\"22\" rx=\"5\" fill=\"var(--surface-2)\" stroke=\"var(--line)\"/>");
    for x in xs {
        svg.push_str(&format!("<rect x=\"{}\" y=\"4\" width=\"3\" height=\"14\" rx=\"1.5\" fill=\"var(--hit-bar)\"/>", x - 1));
    }
    svg.push_str("</svg>");
    svg
}

/// "Slides 2, 5" for the PPTX slides that hold hits, from the same markers the CSV export reads.
fn pptx_slides(r: &FileSearchResult) -> Option<String> {
    if !r.full_name.to_lowercase().ends_with(".pptx") {
        return None;
    }
    let mut slides: Vec<String> = Vec::new();
    for h in &r.hits {
        if let Some(s) = pptx_slide_location(&r.lines_cache, h.line_number) {
            let s = s.trim_start_matches("Slide ").to_string();
            if !slides.contains(&s) {
                slides.push(s);
            }
        }
    }
    if slides.is_empty() {
        return None;
    }
    Some(format!("Slide{} {}", if slides.len() == 1 { "" } else { "s" }, html_escape(&slides.join(", "))))
}

/// Lower-case extension without the dot; empty when the file has none.
fn ext_of(full_name: &str) -> String {
    std::path::Path::new(full_name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// The 2-4 character badge for a file type.
fn type_label(ext: &str) -> String {
    if ext.is_empty() {
        "FILE".to_string()
    } else {
        html_escape(&ext.chars().take(4).collect::<String>().to_uppercase())
    }
}

fn type_color(ext: &str) -> &'static str {
    match ext {
        "pdf" => "#c0392b",
        "xlsx" | "xls" | "xlsm" | "csv" => "#1d7a46",
        "docx" | "doc" | "rtf" => "#2b5fb4",
        "pptx" | "ppt" => "#c25a1a",
        "zip" => "#7a5af8",
        "md" | "txt" | "log" => "#546e7a",
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "cs" | "java" | "go" => "#a5522a",
        _ => "#4a5d78",
    }
}

pub(crate) fn human_size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if b < 1024.0 {
        format!("{} B", b as i64)
    } else if b < 1024.0 * 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", b / (1024.0 * 1024.0 * 1024.0))
    }
}

fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// HTML-encodes a line and wraps every actual matched span - literal,
/// whole-word, or regex filters alike - in `<mark>`. Finds real character
/// ranges in the raw line first (so regex mode is highlighted too), merges
/// overlapping ranges from different filters, then encodes each
/// plain/highlighted piece in turn. A filter other than the first gets a
/// `m2`..`m6` class so its colour matches its chip and bar.
fn highlight_matches(line: &str, matched_filters: &[String], settings: &SearchSettings, colors: &FilterColors) -> String {
    if line.is_empty() {
        return html_escape(line);
    }
    let merged = highlight_ranges(line, matched_filters, settings, colors);
    if merged.is_empty() {
        return html_escape(line);
    }

    let mut out = String::new();
    let mut pos = 0usize;
    for (start, end, slot) in merged {
        if start > pos {
            out.push_str(&html_escape(&line[pos..start]));
        }
        if slot == 0 {
            out.push_str("<mark>");
        } else {
            out.push_str(&format!("<mark class=\"m{}\">", slot + 1));
        }
        out.push_str(&html_escape(&line[start..end]));
        out.push_str("</mark>");
        pos = end;
    }
    if pos < line.len() {
        out.push_str(&html_escape(&line[pos..]));
    }
    out
}

/// The byte ranges `(start, end, colour slot)` of every match of `matched_filters` in `line`, sorted and with
/// overlapping ranges from different filters merged (the earlier range keeps its slot). The report's highlighting and
/// the viewer tab's both draw exactly these ranges.
pub(crate) fn highlight_ranges(line: &str, matched_filters: &[String], settings: &SearchSettings, colors: &FilterColors) -> Vec<(usize, usize, usize)> {
    let mut ranges: Vec<(usize, usize, usize)> = Vec::new();
    for filter in matched_filters {
        let slot = colors.slot(filter);
        // A NEAR query highlights both of its words.
        for f in &crate::near::expand_terms(std::slice::from_ref(filter), settings.use_regex) {
            if f.is_empty() {
                continue;
            }
            let pattern = if settings.use_regex {
                f.clone()
            } else if settings.whole_word {
                whole_word_pattern(f)
            } else {
                fancy_regex::escape(f).into_owned()
            };

            let rx = match FancyRegexBuilder::new(&pattern).case_insensitive(true).build() {
                Ok(r) => r,
                // An invalid/expensive user regex just isn't highlighted here.
                Err(_) => continue,
            };

            for m in rx.find_iter(line).flatten() {
                if !m.as_str().is_empty() {
                    ranges.push((m.start(), m.end(), slot));
                }
            }
        }
    }

    ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize, usize)> = Vec::new();
    for r in ranges {
        if let Some(last) = merged.last_mut() {
            if r.0 <= last.1 {
                if r.1 > last.1 {
                    last.1 = r.1;
                }
                continue;
            }
        }
        merged.push(r);
    }
    merged
}

fn html_escape(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn file_name_of(full_name: &str) -> String {
    std::path::Path::new(full_name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| full_name.to_string())
}

/// Builds a `file://` URI from a filesystem path - approximates .NET's
/// `new Uri(path).AbsoluteUri` (backslash-to-slash normalization plus
/// percent-encoding of each path segment). Not byte-identical to .NET's
/// output in every edge case, but every consumer only needs a valid,
/// openable `file://` link.
pub(crate) fn file_uri(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let segments: Vec<String> = normalized.split('/').map(|s| urlencoding::encode(s).into_owned()).collect();
    let joined = segments.join("/");
    if let Some(stripped) = joined.strip_prefix('/') {
        format!("file:///{stripped}")
    } else {
        format!("file:///{joined}")
    }
}

// ------------------------------------------------------------------
// CSV / JSON export
// ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExportRow {
    pub file_path: String,
    pub line_number: i32,
    /// Best-effort finer-grained location than `line_number` alone -
    /// currently only populated for PPTX (`"Slide N"`/`"Slide N notes"`,
    /// derived from the `--- Slide N ---` marker lines `extract_pptx_lines`
    /// already emits into `lines_cache` - no extraction change, no new
    /// storage per hit, matches epic #6 §29's "prefer the approach that
    /// minimizes storage" - just scan a slice of the file's already-stored
    /// `lines_cache` at export time). `None` for every other format,
    /// including PDF: this extractor is a regex/content-stream scanner,
    /// not a structural PDF parser (see extraction.rs's PDF section
    /// comment), so it has no reliable page-boundary information to
    /// report - a documented known limitation (docs/issue-6-phase-6.md),
    /// not a silently-wrong guess.
    pub location: Option<String>,
    pub matched_filters: String,
    pub before: Option<String>,
    pub match_line: String,
    pub after: Option<String>,
    pub created: DateTime<Local>,
    pub modified: DateTime<Local>,
    /// Epic #6 §69 "Export Metadata" - file size in bytes. `filename`/
    /// `extension` from that section's list are deliberately not
    /// separate columns - both are trivially derivable from `file_path`
    /// by any downstream consumer (a spreadsheet formula, `jq`, etc.) and
    /// duplicating them here would just be redundant data. `score` is
    /// also omitted: this is a literal/regex line scanner, not a ranked
    /// search - there is no real relevance score to report, and a
    /// fabricated constant would be actively misleading.
    pub file_size: i64,
}

/// Scans `lines_cache` backward from `line_number` for the nearest
/// `--- Slide N ---`/`--- Slide N notes ---` marker `extract_pptx_lines`
/// already inserts (see extraction.rs) - purely a read of already-stored
/// data, no new extraction work. Only called for `.pptx` files.
fn pptx_slide_location(lines_cache: &[String], line_number: i32) -> Option<String> {
    let idx = usize::try_from(line_number).ok()?;
    lines_cache
        .get(..idx.min(lines_cache.len()))?
        .iter()
        .rev()
        .find_map(|l| {
            let inner = l.strip_prefix("--- Slide ")?.strip_suffix(" ---")?;
            Some(format!("Slide {inner}"))
        })
}

pub fn build_export_rows(run: &SearchRunResult) -> Vec<ExportRow> {
    run.file_results
        .iter()
        .filter(|r| r.status == FileSearchStatus::Hit)
        .flat_map(|r| {
            let is_pptx = r.full_name.to_lowercase().ends_with(".pptx");
            r.hits.iter().map(move |h| ExportRow {
                file_path: r.full_name.clone(),
                line_number: h.line_number,
                location: if is_pptx { pptx_slide_location(&r.lines_cache, h.line_number) } else { None },
                matched_filters: h.matched_filters.join("; "),
                before: h.before.clone(),
                match_line: h.match_line.clone(),
                after: h.after.clone(),
                created: r.created,
                modified: r.modified,
                file_size: r.file_length,
            })
        })
        .collect()
}

/// Leading characters that a spreadsheet application (Excel, Google Sheets,
/// LibreOffice) treats as the start of a formula.
const CSV_FORMULA_TRIGGER_CHARS: &[char] = &['=', '+', '-', '@', '\t', '\r'];

/// CSV-encodes one field, guarding against CSV/formula injection: this
/// export reflects arbitrary matched file content verbatim, so a line
/// starting with =, +, -, or @ would execute as a formula if the CSV is
/// opened in a spreadsheet app. Prefixing with a single quote neutralizes
/// that while keeping the visible text unchanged, per standard OWASP
/// CSV-injection guidance.
fn csv_field(value: Option<&str>) -> String {
    let mut value = value.unwrap_or("").to_string();
    if let Some(first) = value.chars().next() {
        if CSV_FORMULA_TRIGGER_CHARS.contains(&first) {
            value = format!("'{value}");
        }
    }
    let needs_quoting = value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r');
    let escaped = value.replace('"', "\"\"");
    if needs_quoting {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

/// Streams directly to a buffered file writer - epic #6 §36 ("CSV export
/// must stream results... do not construct the entire export in memory
/// before writing it"). Previously built one `String` holding the whole
/// formatted (escaped/quoted) CSV text before a single `std::fs::write` -
/// roughly 2-3x the raw row data's size once CSV quoting/escaping is
/// applied, held in memory alongside `rows` itself for no reason once
/// each row is only ever needed once, in order.
pub fn write_csv(path: &str, rows: &[ExportRow]) -> std::io::Result<()> {
    use std::io::Write;
    let mut writer = std::io::BufWriter::new(std::fs::File::create(path)?);
    writer.write_all(b"FilePath,LineNumber,Location,MatchedFilters,Before,MatchLine,After,Created,Modified,FileSize\n")?;
    for row in rows {
        let fields = [
            csv_field(Some(&row.file_path)),
            csv_field(Some(&row.line_number.to_string())),
            csv_field(row.location.as_deref()),
            csv_field(Some(&row.matched_filters)),
            csv_field(row.before.as_deref()),
            csv_field(Some(&row.match_line)),
            csv_field(row.after.as_deref()),
            csv_field(Some(&row.created.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))),
            csv_field(Some(&row.modified.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))),
            csv_field(Some(&row.file_size.to_string())),
        ];
        writer.write_all(fields.join(",").as_bytes())?;
        writer.write_all(b"\n")?;
    }
    writer.flush()
}

/// `serde_json::to_writer_pretty` serializes directly into the buffered
/// file writer as it walks `rows`, never building the full JSON `String`
/// `to_string_pretty` used to (same epic #6 §36/§37 "stream, don't
/// buffer the whole export" concern as `write_csv`, applied via
/// serde_json's own streaming `Serializer` rather than a hand-rolled
/// writer loop, since JSON's nesting/escaping rules aren't as trivial to
/// hand-stream correctly as CSV's).
pub fn write_json(path: &str, rows: &[ExportRow]) -> std::io::Result<()> {
    let writer = std::io::BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer_pretty(writer, rows).map_err(std::io::Error::from)
}

/// JSON Lines - one compact JSON object per row (epic #6 §37: "for very
/// large exports, also consider JSON Lines... especially useful for large
/// datasets and downstream processing"). Unlike `write_json`'s single
/// pretty-printed array, each line is independently parseable - the format
/// scripting pipelines (`jq`, `grep`, line-oriented tools) actually want,
/// and never requires holding the whole document in memory to parse one
/// record. CLI-only for now (`--jsonl`) - no GUI use case was requested,
/// see docs/issue-6-phase-6.md.
pub fn write_jsonl(path: &str, rows: &[ExportRow]) -> std::io::Result<()> {
    use std::io::Write;
    let mut writer = std::io::BufWriter::new(std::fs::File::create(path)?);
    for row in rows {
        serde_json::to_writer(&mut writer, row).map_err(std::io::Error::from)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()
}

/// Runs before first paint: marks the page as scripted (the toolbar and buttons are hidden without it) and picks the
/// theme. Without JavaScript the stylesheet's own `prefers-color-scheme` rule applies.
const HEAD_SCRIPT: &str = "<script>(function(){var d=document.documentElement;d.className='js';d.dataset.theme=matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light'})()</script>";

/// The dark palette, written once and used both for the theme toggle and for the no-script media query.
macro_rules! dark_vars {
    () => {
        "--bg:#0c121b; --surface:#121a26; --surface-2:#0f1621; --line:#223044; --line-strong:#2e4058;
    --ink:#e8eef7; --ink-2:#a9b8cf; --ink-3:#7487a3;
    --brand:#07162a; --brand-2:#0f3a60; --accent:#38c6d2; --accent-soft:#12303a;
    --hit-bg:#33290f; --hit-bar:#e0a800; --mark-bg:#e6a817; --mark-ink:#1a1200;
    --good:#4cc38a; --warn:#e0b04a; --warn-bg:#33290f; --bad:#f08a7e; --bad-bg:#3a1f1f;
    --code-bg:#0e1520; --shadow:0 1px 2px rgba(0,0,0,.4),0 10px 28px -12px rgba(0,0,0,.6);
    --c1:#5aa7e0; --c2:#38c6d2; --c3:#f0a64a; --c4:#a08cff; --c5:#5fd19a; --c6:#ec7fb0;
    --m2:#1f7f89; --m2-ink:#eaffff; --m3:#6a45c2; --m3-ink:#f4eaff; --m4:#2c7d52; --m4-ink:#eafff3; --m5:#a23a68; --m5-ink:#fff0f6; --m6:#a8601a; --m6-ink:#fff2e4;"
    };
}

const CSS_BLOCK: &str = concat!(
    "<style>
  :root {
    --bg:#f4f6fa; --surface:#ffffff; --surface-2:#f8fafc; --line:#e3e8f0; --line-strong:#cfd7e4;
    --ink:#0f1b2d; --ink-2:#43536b; --ink-3:#7a8aa3;
    --brand:#0d2b4e; --brand-2:#14507f; --accent:#0e9aa7; --accent-soft:#e0f5f6;
    --hit-bg:#fff6d6; --hit-bar:#f0b400; --mark-bg:#ffd34d; --mark-ink:#2b1d00;
    --good:#1a8f5a; --warn:#b7791f; --warn-bg:#fff4dc; --bad:#c0392b; --bad-bg:#fdeaea;
    --code-bg:#fbfcfe; --shadow:0 1px 2px rgba(15,27,45,.06),0 8px 24px -12px rgba(15,27,45,.18);
    --radius:14px; --radius-sm:8px;
    --sans:'Segoe UI Variable Text','Segoe UI',system-ui,-apple-system,'Helvetica Neue',Arial,sans-serif;
    --mono:'Cascadia Code','SF Mono',ui-monospace,Consolas,'Liberation Mono',monospace;
    --c1:#14507f; --c2:#0e9aa7; --c3:#e08a1e; --c4:#7a5af8; --c5:#2e9e6b; --c6:#c2417a;
    --m2:#9fe0e5; --m2-ink:#062b30; --m3:#e3c8ff; --m3-ink:#24104a; --m4:#b8e8c4; --m4-ink:#0c2e18; --m5:#ffc2d9; --m5-ink:#3a0a1f; --m6:#ffd1a6; --m6-ink:#3a1c00;
  }
  [data-theme=\"dark\"] {",
    dark_vars!(),
    "}
  @media (prefers-color-scheme: dark) { :root:not(.js) {",
    dark_vars!(),
    "} }
  * { box-sizing:border-box; }
  html { scroll-behavior:smooth; scroll-padding-top:84px; }
  body { margin:0; background:var(--bg); color:var(--ink); font:15px/1.5 var(--sans); -webkit-font-smoothing:antialiased; }
  a { color:var(--brand-2); text-decoration:none; } a:hover { text-decoration:underline; }
  [data-theme=\"dark\"] a { color:var(--accent); }
  button { font:inherit; color:inherit; cursor:pointer; }
  :focus-visible { outline:2px solid var(--accent); outline-offset:2px; border-radius:4px; }
  html:not(.js) .js-only { display:none !important; }
  .muted { color:var(--ink-3); }

  .masthead { background:linear-gradient(135deg,var(--brand) 0%,var(--brand-2) 100%); color:#fff; position:relative; overflow:hidden; }
  .masthead::after { content:''; position:absolute; inset:0; background:
      radial-gradient(600px 220px at 85% -20%,rgba(56,198,210,.35),transparent 70%),
      repeating-linear-gradient(135deg,rgba(255,255,255,.03) 0 2px,transparent 2px 14px); pointer-events:none; }
  .masthead-in { max-width:1440px; margin:0 auto; padding:24px 32px; position:relative; z-index:1; display:flex; gap:24px; align-items:center; justify-content:space-between; flex-wrap:wrap; }
  .brand { display:flex; align-items:center; gap:16px; }
  img.report-banner { display:block; height:52px; width:auto; border-radius:10px; border:1px solid rgba(255,255,255,.25); }
  .eyebrow { font-size:12px; letter-spacing:.14em; text-transform:uppercase; opacity:.75; }
  h1 { margin:2px 0 0; font-size:28px; font-weight:700; letter-spacing:-.01em; }
  .run-meta { display:flex; gap:8px; flex-wrap:wrap; }
  .pill { display:inline-flex; align-items:center; gap:6px; padding:5px 11px; border-radius:999px; font-size:12.5px; background:rgba(255,255,255,.12); border:1px solid rgba(255,255,255,.2); }
  .pill b { font-weight:600; }

  .shell { max-width:1440px; margin:0 auto; padding:24px 32px 64px; display:grid; grid-template-columns:292px minmax(0,1fr); gap:28px; }
  .shell.solo { grid-template-columns:minmax(0,1fr); }
  aside.rail { position:sticky; top:16px; align-self:start; max-height:calc(100vh - 32px); display:flex; flex-direction:column; background:var(--surface); border:1px solid var(--line); border-radius:var(--radius); box-shadow:var(--shadow); overflow:hidden; }
  .rail h2 { margin:0; padding:14px 16px 8px; font-size:12px; letter-spacing:.1em; text-transform:uppercase; color:var(--ink-3); display:flex; justify-content:space-between; }
  .rail h2 span { font-weight:400; }
  .rail-list { position:relative; overflow:auto; padding:0 8px 10px; }
  .rail a { display:flex; align-items:center; gap:10px; padding:7px 8px; border-radius:8px; color:var(--ink-2); font-size:13.5px; }
  .rail a:hover { background:var(--surface-2); text-decoration:none; }
  .rail a.active { background:var(--accent-soft); color:var(--ink); }
  .rail a .n { margin-left:auto; font-variant-numeric:tabular-nums; font-size:12px; color:var(--ink-3); }
  .rail a .nm { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .rail-foot { border-top:1px solid var(--line); padding:10px 12px; display:flex; gap:8px; }

  .card { background:var(--surface); border:1px solid var(--line); border-radius:var(--radius); box-shadow:var(--shadow); min-width:0; }
  .card-h { display:flex; align-items:baseline; justify-content:space-between; padding:16px 20px 0; }
  .card-h h3 { margin:0; font-size:14px; font-weight:650; }
  .card-h span { font-size:12px; color:var(--ink-3); }
  .card-b { padding:14px 20px 18px; }
  .grid { display:grid; gap:18px; margin-bottom:18px; }
  .kpis { grid-template-columns:repeat(auto-fit,minmax(170px,1fr)); }
  .kpi { padding:16px 18px; }
  .kpi .lbl { font-size:12px; color:var(--ink-3); text-transform:uppercase; letter-spacing:.08em; }
  .kpi .val { font-size:30px; font-weight:700; letter-spacing:-.02em; margin-top:4px; font-variant-numeric:tabular-nums; }
  .kpi .unit { font-size:16px; }
  .kpi .sub { font-size:12.5px; color:var(--ink-2); }
  .two { grid-template-columns:minmax(0,1.1fr) minmax(0,1fr); }
  .three { grid-template-columns:repeat(3,minmax(0,1fr)); }

  .defs { margin:0; }
  .qrow { display:grid; grid-template-columns:110px 1fr; gap:8px 12px; font-size:13.5px; padding:6px 0; border-bottom:1px dashed var(--line); }
  .qrow:last-child { border-bottom:0; }
  .qrow dt { color:var(--ink-3); } .qrow dd { margin:0; word-break:break-word; }
  .chip { display:inline-flex; align-items:center; gap:6px; padding:2px 10px; border-radius:999px; font-size:12.5px; border:1px solid var(--line-strong); background:var(--surface-2); margin:2px 4px 2px 0; }
  .dot { width:9px; height:9px; border-radius:50%; display:inline-block; flex-shrink:0; }
  code,.mono { font-family:var(--mono); font-size:12.5px; word-break:break-all; }

  .bar-row { display:grid; grid-template-columns:140px 1fr 52px; align-items:center; gap:10px; margin:9px 0; font-size:13px; }
  .bar-label { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .bar-track { display:block; height:10px; background:var(--surface-2); border:1px solid var(--line); border-radius:99px; overflow:hidden; }
  .bar-fill { display:block; height:100%; border-radius:99px; }
  .bar-count { text-align:right; font-variant-numeric:tabular-nums; color:var(--ink-2); }
  .legend { display:flex; flex-wrap:wrap; gap:6px 14px; font-size:12.5px; color:var(--ink-2); margin-top:10px; }
  .legend.col { flex-direction:column; margin:0; }
  .donut-wrap { display:flex; align-items:center; gap:22px; }
  .segbar { display:flex; height:12px; border-radius:99px; overflow:hidden; border:1px solid var(--line); }

  .toolbar { position:sticky; top:0; z-index:20; display:flex; gap:10px; flex-wrap:wrap; align-items:center; padding:12px 14px; margin-bottom:16px; background:color-mix(in srgb,var(--surface) 88%,transparent); backdrop-filter:blur(10px); border:1px solid var(--line); border-radius:var(--radius); box-shadow:var(--shadow); }
  .search { flex:1 1 240px; display:flex; align-items:center; gap:8px; padding:8px 12px; background:var(--surface-2); border:1px solid var(--line-strong); border-radius:10px; }
  .search input { flex:1; border:0; outline:0; background:transparent; color:var(--ink); font:inherit; }
  kbd { font:11px var(--mono); padding:1px 6px; border:1px solid var(--line-strong); border-bottom-width:2px; border-radius:5px; color:var(--ink-3); }
  .btn { padding:7px 12px; border-radius:9px; border:1px solid var(--line-strong); background:var(--surface); font-size:13px; color:var(--ink); }
  .btn:hover { background:var(--surface-2); text-decoration:none; }
  .seg { display:flex; gap:6px; flex-wrap:wrap; }
  .seg button { padding:5px 11px; border-radius:999px; border:1px solid var(--line-strong); background:var(--surface); font-size:12.5px; }
  .seg button[aria-pressed=\"true\"] { background:var(--brand-2); color:#fff; border-color:var(--brand-2); }
  .count-line { font-size:13px; color:var(--ink-3); margin:0 2px 10px; }

  details.file { background:var(--surface); border:1px solid var(--line); border-radius:var(--radius); margin-bottom:12px; box-shadow:var(--shadow); overflow:hidden; }
  details.file[hidden] { display:none; }
  details.file > summary { list-style:none; cursor:pointer; display:grid; grid-template-columns:auto minmax(0,1fr) auto; gap:14px; align-items:center; padding:14px 18px; }
  details.file > summary::-webkit-details-marker { display:none; }
  details.file > summary:hover { background:var(--surface-2); }
  .ftype { width:40px; height:40px; border-radius:10px; display:grid; place-items:center; font:700 11px var(--mono); color:#fff; letter-spacing:.04em; }
  .ftype.sm { width:22px; height:22px; border-radius:6px; font-size:8px; flex-shrink:0; }
  .fmain { min-width:0; }
  .fname { font-weight:650; font-size:15px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .fpath { color:var(--ink-3); font-size:12.5px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; direction:rtl; text-align:left; }
  .fpath bdi { direction:ltr; }
  .fright { display:flex; align-items:center; gap:12px; }
  .badge { padding:3px 10px; border-radius:999px; font-size:12px; font-weight:600; background:var(--accent-soft); color:var(--ink); border:1px solid color-mix(in srgb,var(--accent) 40%,transparent); white-space:nowrap; }
  .badge.warn { background:var(--warn-bg); color:var(--warn); border-color:color-mix(in srgb,var(--warn) 40%,transparent); }
  .chev { transition:transform .15s; color:var(--ink-3); } details[open] .chev { transform:rotate(90deg); }
  .heat { width:84px; height:22px; flex-shrink:0; }
  .fbody { border-top:1px solid var(--line); }
  .fmeta { display:flex; flex-wrap:wrap; gap:6px 18px; padding:12px 18px; font-size:12.5px; color:var(--ink-2); background:var(--surface-2); border-bottom:1px solid var(--line); align-items:center; }
  .fmeta .sp { flex:1; }
  .note { margin:12px 18px 0; padding:9px 12px; border-radius:var(--radius-sm); font-size:13px; }
  .note.warn { background:var(--warn-bg); color:var(--warn); } .note.info { background:var(--accent-soft); }
  .code { margin:12px 18px 16px; border:1px solid var(--line); border-radius:var(--radius-sm); background:var(--code-bg); overflow:hidden; }
  .code-h { padding:7px 12px; border-bottom:1px solid var(--line); font-size:12px; color:var(--ink-3); }
  .lines { position:relative; max-height:420px; overflow:auto; font:12.5px/1.65 var(--mono); }
  .ln { display:grid; grid-template-columns:62px 1fr; white-space:pre-wrap; word-break:break-word; }
  .ln > i { font-style:normal; text-align:right; padding-right:12px; color:var(--ink-3); user-select:none; border-right:1px solid var(--line); background:var(--surface-2); }
  .ln > span { padding:0 12px; }
  .ln.hit { background:var(--hit-bg); box-shadow:inset 3px 0 0 var(--hit-bar); }
  .ln.hit > i { color:var(--ink); font-weight:700; }
  .ln.cur { outline:2px solid var(--accent); }
  .ln.ctx > span { color:var(--ink-3); }
  mark { background:var(--mark-bg); color:var(--mark-ink); padding:0 2px; border-radius:3px; font-weight:700; }
  mark.m2 { background:var(--m2); color:var(--m2-ink); } mark.m3 { background:var(--m3); color:var(--m3-ink); }
  mark.m4 { background:var(--m4); color:var(--m4-ink); } mark.m5 { background:var(--m5); color:var(--m5-ink); }
  mark.m6 { background:var(--m6); color:var(--m6-ink); }

  .rail a { position:relative; }
  .rail a::after { content:''; position:absolute; left:8px; right:8px; bottom:1px; height:3px; border-radius:2px; background:linear-gradient(to right,var(--hit-bar) var(--heat,0%),var(--line) var(--heat,0%)); opacity:.85; }
  .cwrap { display:flex; background:var(--code-bg); }
  .cwrap .lines { flex:1; min-width:0; font-size:var(--code-fs,12.5px); }
  .nowrap .ln > span { white-space:pre; word-break:normal; }
  .ctools { display:flex; align-items:center; gap:6px; flex-wrap:wrap; padding:6px 12px; border-bottom:1px solid var(--line); font-size:12px; color:var(--ink-3); }
  .ctools .btn { padding:3px 9px; font-size:12px; }
  .ctools .hint { margin-left:auto; }
  .ctools kbd { font:11px var(--mono); border:1px solid var(--line-strong); border-radius:4px; padding:0 5px; background:var(--surface-2); color:var(--ink-2); }
  .sb { position:sticky; top:0; z-index:2; display:flex; justify-content:space-between; gap:12px; padding:3px 12px; background:var(--surface-2); border-bottom:1px solid var(--line); font:11.5px var(--sans); color:var(--ink-2); }
  .gut { position:relative; width:18px; flex:none; background:var(--surface-2); border-left:1px solid var(--line); cursor:pointer; }
  .gut .tk { position:absolute; left:3px; right:3px; height:4px; border-radius:2px; box-shadow:0 0 0 1px color-mix(in srgb,var(--ink) 22%,transparent); pointer-events:none; }
  .gut .tk.on { left:0; right:0; height:6px; box-shadow:0 0 0 1.5px var(--accent); }
  .gut .th { position:absolute; left:0; right:0; border-radius:3px; background:var(--ink); opacity:.14; pointer-events:none; }
  .gtip { position:fixed; z-index:50; max-width:340px; padding:4px 9px; border-radius:7px; background:var(--ink); color:var(--bg); font:12px var(--sans); white-space:nowrap; overflow:hidden; text-overflow:ellipsis; pointer-events:none; display:none; }
  .fmeta .chip[data-slot] { cursor:pointer; user-select:none; }
  .fmeta .chip[data-slot].off { opacity:.45; text-decoration:line-through; }
  .ln.hit.dim { background:none; box-shadow:none; }
  .ln.hit.dim > i { color:var(--ink-3); font-weight:400; }
  mark.off { background:transparent; color:inherit; font-weight:400; }
  .ln.hid { display:none; }
  .foldm { padding:1px 12px 1px 66px; font:11px var(--sans); color:var(--ink-3); background:var(--surface-2); border-top:1px dashed var(--line); border-bottom:1px dashed var(--line); cursor:pointer; }
  .foldm:hover { color:var(--ink); }
  .warnings .w { display:flex; gap:10px; padding:8px 0; border-bottom:1px dashed var(--line); font-size:13px; }
  .warnings .w:last-child { border:0; }
  .no-hits { color:var(--ink-3); font-style:italic; }
  footer { text-align:center; color:var(--ink-3); font-size:12.5px; padding:26px 0 0; }
  .toast { position:fixed; bottom:20px; left:50%; transform:translateX(-50%) translateY(20px); background:var(--ink); color:var(--bg); padding:8px 14px; border-radius:10px; font-size:13px; opacity:0; transition:.2s; pointer-events:none; }
  .toast.show { opacity:1; transform:translateX(-50%); }

  @media (max-width:1050px) { .shell { grid-template-columns:1fr; } aside.rail { position:static; max-height:260px; } .two,.three { grid-template-columns:1fr; } }
  @media print {
    body { background:#fff; color:#000; } aside.rail,.toolbar,.toast,.js-only { display:none !important; }
    .shell { display:block; padding:0; }
    .masthead { background:#0d2b4e !important; -webkit-print-color-adjust:exact; print-color-adjust:exact; }
    details.file { box-shadow:none; break-inside:avoid; }
    .lines { max-height:none; overflow:visible; }
    .gut,.ctools,.sb,.foldm,.gtip { display:none !important; } .ln.hid { display:grid; }
    .rail a::after { display:none; }
  }
  @media (prefers-reduced-motion:reduce) { * { transition:none !important; scroll-behavior:auto !important; } }
</style>"
);

/// The report's only script. Everything works without it (cards are plain `<details>`); it adds the filter box, type
/// chips, sorting, expand/collapse, hit navigation, theme toggle and printing with every card open.
const REPORT_SCRIPT: &str = r#"<script>
(function () {
  var $ = function (s, r) { return (r || document).querySelector(s); };
  var $$ = function (s, r) { return Array.prototype.slice.call((r || document).querySelectorAll(s)); };
  var box = $('#files');
  if (!box) return;
  var files = $$('details.file'), railLinks = {}, texts = new Map();
  $$('#rail a').forEach(function (a) { railLinks[a.dataset.id] = a; });
  var q = $('#q'), sortSel = $('#sort'), count = $('#count'), railCount = $('#railCount');

  function text(el) {
    var t = texts.get(el);
    if (t === undefined) { t = el.textContent.toLowerCase(); texts.set(el, t); }
    return t;
  }
  function order() {
    var key = sortSel.value, rail = $('#rail');
    var cmp = {
      default: function (a, b) { return a.dataset.i - b.dataset.i; },
      path: function (a, b) { return a.dataset.name < b.dataset.name ? -1 : a.dataset.name > b.dataset.name ? 1 : 0; },
      hits: function (a, b) { return b.dataset.hits - a.dataset.hits; },
      date: function (a, b) { return a.dataset.date < b.dataset.date ? 1 : a.dataset.date > b.dataset.date ? -1 : 0; },
      density: function (a, b) { return b.dataset.density - a.dataset.density; }
    }[key];
    files.slice().sort(cmp).forEach(function (el) {
      box.appendChild(el);
      if (railLinks[el.id]) rail.appendChild(railLinks[el.id]);
    });
  }
  function apply() {
    var needle = q.value.trim().toLowerCase();
    var on = {};
    $$('#types button[aria-pressed="true"]').forEach(function (b) { on[b.dataset.ext] = true; });
    var shown = 0;
    files.forEach(function (el) {
      var ok = on[el.dataset.ext] === true && (!needle || el.dataset.name.indexOf(needle) >= 0 || text(el).indexOf(needle) >= 0);
      el.hidden = !ok;
      if (ok) shown++;
      if (railLinks[el.id]) railLinks[el.id].style.display = ok ? '' : 'none';
    });
    count.textContent = 'Showing ' + shown + ' of ' + files.length + ' files';
    railCount.textContent = shown + ' of ' + files.length;
  }
  function toast(m) {
    var t = $('#toast'); t.textContent = m; t.classList.add('show');
    setTimeout(function () { t.classList.remove('show'); }, 1400);
  }

  q.addEventListener('input', apply);
  sortSel.addEventListener('change', order);
  $('#types').addEventListener('click', function (e) {
    var b = e.target.closest('button'); if (!b) return;
    b.setAttribute('aria-pressed', b.getAttribute('aria-pressed') !== 'true');
    apply();
  });
  $('#expand').onclick = function () { files.forEach(function (d) { if (!d.hidden) d.open = true; }); };
  $('#collapse').onclick = function () { files.forEach(function (d) { d.open = false; }); };
  $('#themeBtn').onclick = function () {
    var r = document.documentElement; r.dataset.theme = r.dataset.theme === 'dark' ? 'light' : 'dark';
  };
  $('#printBtn').onclick = function () { window.print(); };
  var reopen = [];
  window.addEventListener('beforeprint', function () {
    reopen = files.filter(function (d) { return !d.open && !d.hidden; });
    reopen.forEach(function (d) { d.open = true; });
  });
  window.addEventListener('afterprint', function () { reopen.forEach(function (d) { d.open = false; }); reopen = []; });

  // Code view: hit gutter beside the scrollbar, the current-hit outline, filter chips, folding, keys and line links.
  // Built lazily when a file opens; without script the cards stay plain <details> (nothing here is needed to read them).
  var curFile = null, tip = null, scheduled = {};
  var view = { wrap: true, fs: 12.5 };
  try {
    var saved = JSON.parse(localStorage.getItem('rpt-view') || '{}');
    if (saved.wrap === false) view.wrap = false;
    if (saved.fs >= 10 && saved.fs <= 20) view.fs = saved.fs;
  } catch (x) {}
  var MARK = ['var(--mark-bg)', 'var(--m2)', 'var(--m3)', 'var(--m4)', 'var(--m5)', 'var(--m6)'];
  function slotOf(el) { var m = /\bm(\d)\b/.exec(el.className || ''); return m ? +m[1] - 1 : 0; }
  function cwraps(d) { return $$('.cwrap', d); }
  function liveRows(w) { return $$('.ln.hit:not(.dim)', w); }
  function allHits(d) { var out = []; cwraps(d).forEach(function (w) { out = out.concat(liveRows(w)); }); return out; }
  function openReady() { return files.filter(function (d) { return d.open && d._ready; }); }

  function applyView() {
    var r = document.documentElement;
    r.classList.toggle('nowrap', !view.wrap);
    r.style.setProperty('--code-fs', view.fs + 'px');
    $$('.js-wrap').forEach(function (b) { b.setAttribute('aria-pressed', view.wrap ? 'true' : 'false'); });
    try { localStorage.setItem('rpt-view', JSON.stringify(view)); } catch (x) {}
    requestAnimationFrame(function () { openReady().forEach(draw); });
  }
  function later(d) {
    if (scheduled[d.id]) return;
    scheduled[d.id] = true;
    requestAnimationFrame(function () { scheduled[d.id] = false; draw(d); });
  }
  function thumb(s) {
    var G = s.gut.clientHeight, tot = s.lines.scrollHeight || 1;
    s.th.style.top = Math.round(s.lines.scrollTop / tot * G) + 'px';
    s.th.style.height = Math.max(10, Math.round(s.lines.clientHeight / tot * G)) + 'px';
  }
  // The hit a code view is showing: the selected one when it lives in this view, else the one at the top third of it.
  function viewRow(w, here) {
    var s = w._s;
    if (here && w.contains(here)) return here;
    var best = 0, top = s.lines.scrollTop + s.lines.clientHeight / 3;
    s.rows.forEach(function (r, i) { if (r.offsetTop <= top) best = i; });
    return s.rows[best];
  }
  function draw(d) {
    var hits = allHits(d);
    if (d._i >= hits.length) d._i = Math.max(0, hits.length - 1);
    var here = hits[d._i], name = ($('.fname', d) || {}).textContent || '';
    cwraps(d).forEach(function (w) {
      var s = w._s; if (!s) return;
      var G = s.gut.clientHeight, tot = s.lines.scrollHeight || 1, last = -9;
      $$('.tk', s.gut).forEach(function (t) { t.remove(); });
      s.rows = liveRows(w); s.ys = [];
      var shown = s.rows.length ? viewRow(w, here) : null;
      s.rows.forEach(function (r) {
        var y = Math.round(r.offsetTop / tot * G), on = r === shown;
        s.ys.push(y);
        if (y === last && !on) return;
        last = y;
        var mk = r.querySelector('mark:not(.off)'), t = document.createElement('div');
        t.className = 'tk' + (on ? ' on' : '');
        t.style.background = MARK[mk ? Math.min(slotOf(mk), 5) : 0];
        t.style.top = y + 'px';
        s.gut.appendChild(t);
      });
      thumb(s);
      s.sb.innerHTML = '<span></span><span></span>';
      s.sb.firstChild.textContent = name;
      s.sb.lastChild.textContent = shown ? 'Hit ' + (hits.indexOf(shown) + 1) + ' of ' + hits.length + ' · line ' + (shown.querySelector('i').textContent || '') : 'No hits shown';
    });
  }
  function select(d, i, smooth) {
    var hits = allHits(d); if (!hits.length) return;
    i = (i + hits.length) % hits.length;
    d._i = d._sel = i;
    $$('.ln.cur', d).forEach(function (r) { r.classList.remove('cur'); });
    var row = hits[i], w = row.closest('.cwrap'), lines = w._s.lines;
    row.classList.add('cur');
    lines.scrollTo({ top: Math.max(0, row.offsetTop - 48), behavior: smooth ? 'smooth' : 'auto' });
    if (w !== cwraps(d)[0]) w.scrollIntoView({ block: 'nearest', behavior: smooth ? 'smooth' : 'auto' });
    draw(d);
  }
  // Next / previous steps from the hit last selected (the outlined one), not from the one the scroll happens to show.
  function step(d, dir) {
    var hits = allHits(d); if (!hits.length) return;
    var at = d._sel === undefined || !hits[d._sel] || !hits[d._sel].classList.contains('cur') ? -1 : d._sel;
    select(d, at < 0 ? (dir > 0 ? 0 : hits.length - 1) : at + dir, true);
  }
  function onScroll(d, w) {
    var s = w._s; thumb(s);
    var hits = allHits(d), rows = s.rows || [], best = 0, top = s.lines.scrollTop + s.lines.clientHeight / 3;
    rows.forEach(function (r, i) { if (r.offsetTop <= top) best = i; });
    var gi = rows.length ? hits.indexOf(rows[best]) : -1;
    if (gi >= 0 && gi !== d._i) { d._i = gi; later(d); }
  }
  function nearest(s, y) {
    var bi = -1, bd = 1e9;
    (s.ys || []).forEach(function (v, i) { var dd = Math.abs(v - y); if (dd < bd) { bd = dd; bi = i; } });
    return { i: bi, d: bd };
  }
  function showTip(e, d, w) {
    var s = w._s, r = s.gut.getBoundingClientRect(), n = nearest(s, e.clientY - r.top);
    if (!tip) { tip = document.createElement('div'); tip.className = 'gtip'; document.body.appendChild(tip); }
    if (n.i < 0 || n.d >= 9) { tip.style.display = 'none'; return; }
    var row = s.rows[n.i];
    tip.textContent = 'Line ' + row.querySelector('i').textContent + ': ' + row.querySelector('span').textContent.trim().slice(0, 140);
    tip.style.display = 'block';
    tip.style.left = (r.left - 8) + 'px'; tip.style.top = Math.max(4, e.clientY - 14) + 'px';
    tip.style.transform = 'translateX(-100%)';
  }
  function setup(d) {
    if (d._ready) return;
    d._ready = true; d._off = {}; d._fold = false; d._i = 0;
    cwraps(d).forEach(function (w, k) {
      var lines = $('.lines', w), sb = document.createElement('div'), gut = document.createElement('div'), th = document.createElement('div');
      sb.className = 'sb'; lines.insertBefore(sb, lines.firstChild);
      gut.className = 'gut'; gut.setAttribute('role', 'button');
      gut.setAttribute('aria-label', 'Hit map: click a mark to jump to that hit');
      th.className = 'th'; gut.appendChild(th); w.appendChild(gut);
      w._s = { lines: lines, gut: gut, th: th, sb: sb, rows: [], ys: [] };
      if (k === 0) {
        var bar = document.createElement('div'); bar.className = 'ctools js-only';
        bar.innerHTML = '<button class="btn js-fold" type="button">Fold lines without hits</button><button class="btn js-wrap" type="button" aria-pressed="' + view.wrap + '">Wrap</button><button class="btn js-sm" type="button" title="Smaller text">A&minus;</button><button class="btn js-lg" type="button" title="Larger text">A+</button><span class="hint"><kbd>n</kbd> <kbd>p</kbd> hit &middot; <kbd>[</kbd> <kbd>]</kbd> file</span>';
        w.parentNode.insertBefore(bar, w);
      }
      lines.addEventListener('scroll', function () { onScroll(d, w); });
      gut.addEventListener('click', function (e) {
        var s = w._s, r = gut.getBoundingClientRect(), y = e.clientY - r.top, n = nearest(s, y);
        if (n.i >= 0 && n.d < 9) select(d, allHits(d).indexOf(s.rows[n.i]), true);
        else s.lines.scrollTo({ top: y / r.height * s.lines.scrollHeight - s.lines.clientHeight / 2, behavior: 'smooth' });
      });
      gut.addEventListener('mousemove', function (e) { showTip(e, d, w); });
      gut.addEventListener('mouseleave', function () { if (tip) tip.style.display = 'none'; });
    });
    draw(d);
  }
  function applyFold(d) {
    cwraps(d).forEach(function (w) {
      var s = w._s; if (!s) return;
      $$('.foldm', s.lines).forEach(function (m) { m.remove(); });
      $$('.ln.hid', s.lines).forEach(function (r) { r.classList.remove('hid'); });
      if (!d._fold) return;
      var rows = $$('.ln', s.lines), keep = rows.map(function () { return false; }), group = [];
      rows.forEach(function (r, i) {
        if (r.classList.contains('hit') && !r.classList.contains('dim')) {
          for (var k = i - 1; k <= i + 1; k++) if (k >= 0 && k < rows.length) keep[k] = true;
        }
      });
      function flush() {
        if (!group.length) return;
        var m = document.createElement('div'); m.className = 'foldm'; m._rows = group;
        m.textContent = '··· ' + group.length + (group.length === 1 ? ' line' : ' lines') + ' without hits';
        group[0].parentNode.insertBefore(m, group[0]);
        group.forEach(function (r) { r.classList.add('hid'); });
        group = [];
      }
      rows.forEach(function (r, i) { if (keep[i]) flush(); else group.push(r); });
      flush();
    });
    $$('.js-fold', d).forEach(function (b) { b.textContent = d._fold ? 'Show all lines' : 'Fold lines without hits'; });
  }
  function applyChips(d) {
    $$('mark', d).forEach(function (m) { m.classList.toggle('off', !!d._off[slotOf(m)]); });
    $$('.ln.hit', d).forEach(function (r) {
      var ms = $$('mark', r);
      r.classList.toggle('dim', ms.length > 0 && ms.every(function (m) { return d._off[slotOf(m)]; }));
    });
    applyFold(d); draw(d);
  }
  function navFile(dir) {
    var vis = files.filter(function (d) { return !d.hidden; }); if (!vis.length) return;
    var i = vis.indexOf(curFile), d = vis[(i < 0 ? (dir > 0 ? 0 : vis.length - 1) : (i + dir + vis.length) % vis.length)];
    d.open = true; setup(d); curFile = d;
    d.scrollIntoView({ behavior: 'smooth', block: 'start' });
    select(d, 0, false);
  }
  function openHash() {
    var m = /^#(file-\d+)-L(\d+)$/.exec(location.hash), d = m && document.getElementById(m[1]);
    if (!d) return;
    d.hidden = false; d.open = true; setup(d); curFile = d;
    var cell = $$('.ln > i', d).filter(function (x) { return x.textContent === m[2]; })[0];
    if (!cell) return;
    var row = cell.parentNode;
    $$('.ln.cur', d).forEach(function (r) { r.classList.remove('cur'); });
    row.classList.add('cur'); d._sel = allHits(d).indexOf(row);
    d.scrollIntoView();
    var lines = row.closest('.lines'); lines.scrollTop = Math.max(0, row.offsetTop - 48);
    draw(d);
  }

  box.addEventListener('toggle', function (e) {
    var d = e.target;
    if (d.matches && d.matches('details.file') && d.open) { setup(d); curFile = d; draw(d); }
  }, true);
  window.addEventListener('resize', function () { openReady().forEach(later); });
  window.addEventListener('hashchange', openHash);
  document.addEventListener('click', function (e) {
    var t = e.target, d = t.closest ? t.closest('details.file') : null;
    if (d) { curFile = d; setup(d); }
    var b = t.closest('[data-act]');
    if (b) {
      if (b.dataset.act === 'copy') {
        if (navigator.clipboard) navigator.clipboard.writeText(b.dataset.path).then(function () { toast('Path copied'); }, function () { toast('Copy failed'); });
      } else if (d) { step(d, b.dataset.act === 'next' ? 1 : -1); }
      return;
    }
    if (!d) return;
    var chip = t.closest('.fmeta .chip[data-slot]');
    if (chip) {
      var slot = +chip.dataset.slot; d._off[slot] = !d._off[slot];
      chip.classList.toggle('off', !!d._off[slot]); chip.setAttribute('aria-pressed', d._off[slot] ? 'false' : 'true');
      applyChips(d); return;
    }
    var fm = t.closest('.foldm');
    if (fm) { fm._rows.forEach(function (r) { r.classList.remove('hid'); }); fm.remove(); draw(d); return; }
    var num = t.closest('.cwrap .ln > i');
    if (num && /^\d+$/.test(num.textContent)) {
      var link = location.href.split('#')[0] + '#' + d.id + '-L' + num.textContent;
      if (navigator.clipboard) navigator.clipboard.writeText(link).then(function () { toast('Link to line ' + num.textContent + ' copied'); }, function () { toast('Copy failed'); });
      return;
    }
    var btn = t.closest('.ctools button');
    if (!btn) return;
    if (btn.classList.contains('js-fold')) { d._fold = !d._fold; applyFold(d); draw(d); }
    else if (btn.classList.contains('js-wrap')) { view.wrap = !view.wrap; applyView(); }
    else if (btn.classList.contains('js-sm')) { view.fs = Math.max(10, view.fs - 1); applyView(); }
    else if (btn.classList.contains('js-lg')) { view.fs = Math.min(20, view.fs + 1); applyView(); }
  });
  document.addEventListener('keydown', function (e) {
    if (e.key === '/' && document.activeElement !== q) { e.preventDefault(); q.focus(); return; }
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    var tg = e.target, tag = tg && tg.tagName;
    if (tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA' || (tg && tg.isContentEditable)) return;
    if (e.key === 'n' || e.key === 'p') {
      var d = curFile && !curFile.hidden && curFile.open ? curFile : files.filter(function (f) { return f.open && !f.hidden; })[0];
      if (!d) return;
      setup(d); curFile = d; step(d, e.key === 'n' ? 1 : -1); e.preventDefault();
    } else if (e.key === '[' || e.key === ']') { navFile(e.key === ']' ? 1 : -1); e.preventDefault(); }
  });
  applyView();
  openHash();
  if ('IntersectionObserver' in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (!en.isIntersecting) return;
        Object.keys(railLinks).forEach(function (id) { railLinks[id].classList.toggle('active', id === en.target.id); });
        var rl = $('#rail'), a = railLinks[en.target.id];
        if (a.offsetTop < rl.scrollTop || a.offsetTop + a.offsetHeight > rl.scrollTop + rl.clientHeight) rl.scrollTop = a.offsetTop - 40;
      });
    }, { rootMargin: '-15% 0px -75% 0px' });
    files.forEach(function (el) { io.observe(el); });
  }
  apply();
})();
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FileSearchResult, SearchRunSummary};

    fn sample_result(full_name: &str, hits: Vec<LineHit>) -> FileSearchResult {
        FileSearchResult {
            full_name: full_name.to_string(),
            status: FileSearchStatus::Hit,
            hits,
            created: Local::now(),
            modified: Local::now(),
            file_length: 10,
            lines_cache: vec!["line one has apple".to_string(), "line two has banana".to_string()],
            total_line_count: 2,
            proximity_min_range: None,
            low_confidence_pdf: false,
            ocr_confidence: None,
            error_message: None,
        }
    }

    fn hit(line_number: i32, match_line: &str, filters: &[&str]) -> LineHit {
        LineHit {
            line_number,
            before: None,
            match_line: match_line.to_string(),
            after: None,
            matched_filters: filters.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// A file with `n` lines of filler whose hit lines (1-based, with their filters) read like a real document.
    fn document_with_hits(name: &str, n: usize, hits: &[(i32, &[&str])]) -> FileSearchResult {
        let filler = [
            "General notes for this section of the document.",
            "Identify all existing tools using the current implementation.",
            "Migrate consumers to the shared module.",
            "Validate results after the change.",
            "All dimensions are in millimetres unless stated.",
        ];
        let mut r = sample_result(name, Vec::new());
        r.lines_cache = (1..=n).map(|i| filler[i % filler.len()].to_string()).collect();
        r.total_line_count = n as i32;
        for (line, filters) in hits {
            let text = format!("Establish {} before release.", filters.join(" and "));
            if let Some(slot) = r.lines_cache.get_mut(*line as usize - 1) {
                *slot = text.clone();
            }
            r.hits.push(hit(*line, &text, filters));
        }
        r
    }

    #[test]
    fn the_hit_highlight_is_unchanged_and_the_code_view_has_a_wrapper_for_the_gutter() {
        let settings = SearchSettings { filters: vec!["apple".to_string(), "banana".to_string()], ..Default::default() };
        let mut run = SearchRunResult::default();
        run.file_results.push(document_with_hits("/x/a.txt", 30, &[(12, &["apple"])]));
        let html = build_html_report(&settings, &run);
        assert!(html.contains("<div class=\"ln hit\"><i>12</i><span>Establish <mark>apple</mark> before release.</span></div>"), "the hit row markup is as before");
        assert!(html.contains("<div class=\"cwrap\"><div class=\"lines\">"), "the gutter hangs off a wrapper the script fills");
        for rule in [
            ".ln.hit { background:var(--hit-bg); box-shadow:inset 3px 0 0 var(--hit-bar); }",
            ".ln.hit > i { color:var(--ink); font-weight:700; }",
            "mark { background:var(--mark-bg); color:var(--mark-ink); padding:0 2px; border-radius:3px; font-weight:700; }",
            ".ln.cur { outline:2px solid var(--accent); }",
        ] {
            assert!(html.contains(rule), "the highlight rule is kept: {rule}");
        }
        assert!(html.contains("--hit-bg:#33290f; --hit-bar:#e0a800; --mark-bg:#e6a817; --mark-ink:#1a1200;"), "dark palette values");
        assert_eq!(html.matches("<script").count(), 2, "no extra script tag: the gutter is built by the existing report script");
    }

    #[test]
    fn filter_chips_carry_their_colour_slot_so_the_script_can_dim_a_filter() {
        let settings = SearchSettings { filters: vec!["apple".to_string(), "banana".to_string()], ..Default::default() };
        let mut run = SearchRunResult::default();
        run.file_results.push(document_with_hits("/x/a.txt", 30, &[(3, &["apple"]), (9, &["banana"])]));
        let html = build_html_report(&settings, &run);
        assert!(html.contains("<span class=\"chip\" data-slot=\"0\"><i class=\"dot\""), "{html}");
        assert!(html.contains("<span class=\"chip\" data-slot=\"1\"><i class=\"dot\""));
    }

    #[test]
    fn the_rail_bar_is_hit_density_against_the_densest_file() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let mut run = SearchRunResult::default();
        run.file_results.push(document_with_hits("/x/dense.txt", 10, &[(2, &["apple"]), (4, &["apple"]), (6, &["apple"])]));
        run.file_results.push(document_with_hits("/x/sparse.txt", 100, &[(50, &["apple"])]));
        let html = build_html_report(&settings, &run);
        let bar = |name: &str| -> f64 {
            let at = html.find(&format!("<span class=\"nm\">{name}</span>")).expect("a rail link for the file");
            let tag = &html[..at];
            let start = tag.rfind("--heat:").expect("a heat value on its link") + "--heat:".len();
            tag[start..].split('%').next().unwrap().parse().unwrap()
        };
        assert_eq!(bar("dense.txt"), 100.0, "the densest file fills the bar");
        let sparse = bar("sparse.txt");
        assert!((6.0..10.0).contains(&sparse), "1 hit in 100 lines against 3 in 10 is about 3 %, floored to a visible 6 %: {sparse}");
    }

    /// Writes a report with three filters, a long file with a hit beyond its shown lines and a few short ones, for
    /// looking at the gutter in a browser: `FEA_REPORT_SAMPLE_OUT=/tmp/report.html cargo test -p search-core --lib
    /// writes_a_sample_report -- --ignored`.
    #[test]
    #[ignore = "writes a sample report to FEA_REPORT_SAMPLE_OUT"]
    fn writes_a_sample_report_for_the_browser() {
        let Ok(out) = std::env::var("FEA_REPORT_SAMPLE_OUT") else { return };
        let all: [&str; 3] = ["regression tests", "baseline", "migration"];
        let settings = SearchSettings { filters: all.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let mut run = SearchRunResult::default();
        let mut long = document_with_hits(
            "/Engineering/Specs/torque-specification.pdf",
            400,
            &[(7, &["regression tests"]), (19, &["baseline"]), (20, &["baseline"]), (34, &["regression tests"]), (48, &["migration"]), (49, &["regression tests"]),
              (120, &["baseline"]), (121, &["regression tests", "baseline"]), (250, &["migration"]), (251, &["migration"]), (380, &["regression tests"]), (382, &["regression tests", "baseline"])],
        );
        long.total_line_count = 1294;
        long.hits.push(hit(1100, "Late in the file: baseline behavior is restated.", &["baseline"]));
        run.file_results.push(long);
        run.file_results.push(document_with_hits("/Engineering/Memos/design-memo.docx", 60, &[(12, &["regression tests"]), (33, &["baseline"]), (41, &["regression tests", "migration"])]));
        run.file_results.push(document_with_hits("/Projects/Alpha/migration-plan.md", 120, &[(5, &["migration"]), (18, &["migration"]), (19, &["regression tests"]), (40, &["baseline"]), (52, &["migration"]), (70, &["regression tests"]), (88, &["migration"])]));
        run.file_results.push(document_with_hits("/Projects/Alpha/notes.txt", 24, &[(9, &["baseline"])]));
        std::fs::write(out, build_html_report(&settings, &run)).unwrap();
    }

    #[test]
    fn html_report_note_states_the_ocr_confidence_when_known() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let render = |low: bool, conf: Option<f32>| {
            let mut r = sample_result("/x/a.pdf", vec![hit(1, "line one has apple", &["apple"])]);
            r.low_confidence_pdf = low;
            r.ocr_confidence = conf;
            let mut run = SearchRunResult::default();
            run.file_results.push(r);
            build_html_report(&settings, &run)
        };
        let poor = render(true, Some(0.62));
        assert!(poor.contains("confidence-note"));
        assert!(poor.contains("low model confidence of 62%"), "{poor}");
        let fine = render(true, Some(0.985));
        assert!(fine.contains("model confidence of 98%"));
        assert!(!fine.contains("low model confidence"));
        let unscored = render(true, None);
        assert!(unscored.contains("may be incomplete or misrecognized"));
        assert!(!unscored.contains("model confidence"));
        assert!(!render(false, Some(0.5)).contains("<p class=\"note warn confidence-note\">"));
    }

    #[test]
    fn html_report_highlights_literal_matches() {
        let settings = SearchSettings {
            filters: vec!["apple".to_string(), "banana".to_string()],
            ..Default::default()
        };
        let hits = vec![
            hit(1, "line one has apple", &["apple"]),
            hit(2, "line two has banana", &["banana"]),
        ];
        let mut run = SearchRunResult::default();
        run.file_results.push(sample_result("/x/a.txt", hits));
        run.summary = SearchRunSummary::default();

        let html = build_html_report(&settings, &run);
        assert!(html.contains("<mark>apple</mark>"));
        assert!(html.contains("<mark class=\"m2\">banana</mark>"), "the second filter has its own colour");
        assert!(html.contains("prefers-color-scheme"));
        assert!(html.contains("bar-row"));
    }

    #[test]
    fn html_report_highlights_regex_mode_match_span() {
        let settings = SearchSettings {
            filters: vec!["ap+le".to_string()],
            use_regex: true,
            ..Default::default()
        };
        let hits = vec![hit(1, "appple pie is great", &["ap+le"])];
        let mut run = SearchRunResult::default();
        run.file_results.push(FileSearchResult {
            lines_cache: vec!["appple pie is great".to_string()],
            total_line_count: 1,
            ..sample_result("/x/a.txt", hits)
        });

        let html = build_html_report(&settings, &run);
        assert!(html.contains("<mark>appple</mark>"));
    }

    #[test]
    fn write_html_report_streams_identical_content_to_the_string_builder() {
        // Proves the streaming path (write_html_report, used in
        // production) and the buffering path (build_html_report, used
        // here and by every other test) produce byte-for-byte identical
        // output despite committing to their sink at completely different
        // granularities (once at the end vs. after every file block) -
        // both drive the same write_report_to_sink, only the sink differs.
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let hits = vec![hit(1, "line one has apple", &["apple"])];
        let mut run = SearchRunResult::default();
        run.file_results.push(sample_result("/x/a.txt", hits));
        run.summary = SearchRunSummary::default();

        let expected = build_html_report(&settings, &run);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.html");
        let byte_count = write_html_report(path.to_str().unwrap(), &settings, &run).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        // Both builds stamp the run time to the second; they can straddle a second boundary, so compare without it.
        let without_run_time = |html: &str| -> String {
            let marker = "<b class=\"run-time\">";
            let start = html.find(marker).expect("the report states its run time") + marker.len();
            let end = start + html[start..].find("</b>").expect("the run time ends");
            format!("{}{}", &html[..start], &html[end..])
        };
        assert_eq!(without_run_time(&written), without_run_time(&expected), "streamed file content must match the in-memory build exactly");
        assert_eq!(byte_count, written.len() as u64, "returned byte count must match the real file size");
    }

    #[test]
    fn html_report_shows_no_hits_message_when_empty() {
        let settings = SearchSettings::default();
        let run = SearchRunResult::default();
        let html = build_html_report(&settings, &run);
        assert!(html.contains("No matches found."));
    }

    #[test]
    fn export_rows_only_include_hit_status_files() {
        let mut run = SearchRunResult::default();
        run.file_results.push(sample_result("/x/a.txt", vec![hit(1, "apple", &["apple"])]));
        run.file_results.push(FileSearchResult {
            status: FileSearchStatus::NoHit,
            ..sample_result("/x/b.txt", vec![])
        });

        let rows = build_export_rows(&run);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].file_path, "/x/a.txt");
    }

    #[test]
    fn csv_export_neutralizes_formula_injection_trigger() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");

        let row = ExportRow {
            file_path: "/x/a.txt".to_string(),
            line_number: 1,
            location: None,
            matched_filters: "x".to_string(),
            before: None,
            match_line: "=SUM(A1:A10)".to_string(),
            after: None,
            created: Local::now(),
            modified: Local::now(),
            file_size: 0,
        };
        write_csv(path.to_str().unwrap(), &[row]).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("'=SUM(A1:A10)"));
    }

    #[test]
    fn json_export_writes_non_empty_pascal_case_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");

        let row = ExportRow {
            file_path: "/x/a.txt".to_string(),
            line_number: 1,
            location: None,
            matched_filters: "apple".to_string(),
            before: None,
            match_line: "apple".to_string(),
            after: None,
            created: Local::now(),
            modified: Local::now(),
            file_size: 0,
        };
        write_json(path.to_str().unwrap(), &[row]).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.is_empty());
        assert!(content.contains("\"FilePath\""));
        assert!(content.contains("\"LineNumber\""));
    }

    #[test]
    fn export_rows_derive_pptx_slide_location_from_marker_lines() {
        let mut run = SearchRunResult::default();
        let mut result = sample_result(
            "/x/deck.pptx",
            vec![hit(4, "engine specs", &["engine"]), hit(1, "before any slide marker", &["engine"])],
        );
        result.lines_cache = vec![
            "--- Slide 1 ---".to_string(),
            "intro text".to_string(),
            "--- Slide 2 ---".to_string(),
            "engine specs".to_string(),
        ];
        run.file_results.push(result);

        let rows = build_export_rows(&run);
        let by_line = |n: i32| rows.iter().find(|r| r.line_number == n).unwrap();
        assert_eq!(by_line(4).location.as_deref(), Some("Slide 2"));
        // A hit that lands directly on the "--- Slide 1 ---" marker line
        // itself still resolves to that slide - the marker line is line 1.
        assert_eq!(by_line(1).location.as_deref(), Some("Slide 1"));
    }

    #[test]
    fn export_rows_carry_the_file_size_from_the_file_result() {
        let mut run = SearchRunResult::default();
        let mut result = sample_result("/x/a.txt", vec![hit(1, "line one has apple", &["apple"])]);
        result.file_length = 12345;
        run.file_results.push(result);

        let rows = build_export_rows(&run);
        assert_eq!(rows[0].file_size, 12345);
    }

    #[test]
    fn export_rows_leave_location_none_for_non_pptx_formats() {
        let mut run = SearchRunResult::default();
        run.file_results.push(sample_result("/x/a.txt", vec![hit(1, "line one has apple", &["apple"])]));
        let rows = build_export_rows(&run);
        assert_eq!(rows[0].location, None);
    }

    #[test]
    fn write_jsonl_writes_one_compact_object_per_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.jsonl");
        let rows = vec![
            ExportRow {
                file_path: "/x/a.txt".to_string(),
                line_number: 1,
                location: None,
                matched_filters: "apple".to_string(),
                before: None,
                match_line: "apple".to_string(),
                after: None,
                created: Local::now(),
                modified: Local::now(),
                file_size: 0,
            },
            ExportRow {
                file_path: "/x/b.txt".to_string(),
                line_number: 2,
                location: Some("Slide 3".to_string()),
                matched_filters: "banana".to_string(),
                before: None,
                match_line: "banana".to_string(),
                after: None,
                created: Local::now(),
                modified: Local::now(),
                file_size: 4096,
            },
        ];
        write_jsonl(path.to_str().unwrap(), &rows).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed.is_object());
        }
        assert!(lines[1].contains("Slide 3"));
    }

    #[test]
    fn html_report_shows_elapsed_time_when_present() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let mut run = SearchRunResult::default();
        run.summary.total_elapsed_seconds = 1.5;
        run.file_results.push(sample_result("/x/a.txt", vec![hit(1, "apple pie", &["apple"])]));

        let html = build_html_report(&settings, &run);
        assert!(html.contains("Elapsed <b>1.50s</b>"));
    }

    #[test]
    fn html_report_omits_elapsed_line_when_not_tracked() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let run = SearchRunResult::default();
        let html = build_html_report(&settings, &run);
        assert!(!html.contains("Elapsed <b>"));
    }

    #[test]
    fn exclude_folders_note_and_bar_chart_use_settings_filters_order() {
        let settings = SearchSettings {
            filters: vec!["banana".to_string(), "apple".to_string()],
            ..Default::default()
        };
        let hits = vec![hit(1, "apple then banana", &["banana", "apple"])];
        let mut run = SearchRunResult::default();
        run.file_results.push(sample_result("/x/a.txt", hits));

        let html = build_html_report(&settings, &run);
        let banana_pos = html.find("bar-label\" title=\"banana\"").unwrap();
        let apple_pos = html.find("bar-label\" title=\"apple\"").unwrap();
        assert!(banana_pos < apple_pos, "bar chart must follow settings.Filters order");
    }

    #[test]
    fn html_report_embeds_banner_as_base64_data_uri() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let mut run = SearchRunResult::default();
        run.file_results.push(sample_result("/x/a.txt", vec![hit(1, "apple", &["apple"])]));

        let html = build_html_report(&settings, &run);
        assert!(html.contains("<img class=\"report-banner\" src=\"data:image/jpeg;base64,"));
    }

    fn report_for(settings: &SearchSettings, results: Vec<FileSearchResult>) -> String {
        let mut run = SearchRunResult::default();
        run.file_results = results;
        build_html_report(settings, &run)
    }

    #[test]
    fn a_second_filter_gets_its_own_mark_colour_and_the_same_dot_colour() {
        let settings = SearchSettings { filters: vec!["apple".to_string(), "banana".to_string()], ..Default::default() };
        let mut r = sample_result("/x/a.txt", vec![hit(1, "apple and banana", &["apple", "banana"])]);
        r.lines_cache = vec!["apple and banana".to_string()];
        r.total_line_count = 1;
        let html = report_for(&settings, vec![r]);
        assert!(html.contains("<mark>apple</mark> and <mark class=\"m2\">banana</mark>"), "{html}");
        assert!(html.contains("<i class=\"dot\" style=\"background:var(--c1)\"></i>apple"));
        assert!(html.contains("<i class=\"dot\" style=\"background:var(--c2)\"></i>banana"));
    }

    #[test]
    fn overlapping_matches_of_two_filters_merge_into_one_mark() {
        let settings = SearchSettings { filters: vec!["apple pie".to_string(), "pie".to_string()], ..Default::default() };
        let colors = FilterColors::new(&settings);
        let html = highlight_matches("an apple pie", &["apple pie".to_string(), "pie".to_string()], &settings, &colors);
        assert_eq!(html, "an <mark>apple pie</mark>");
    }

    #[test]
    fn report_without_hits_has_no_rail_toolbar_or_file_list() {
        let html = report_for(&SearchSettings::default(), vec![]);
        assert!(html.contains("class=\"shell solo\""));
        for absent in ["id=\"rail\"", "id=\"files\"", "class=\"toolbar", "id=\"sort\""] {
            assert!(!html.contains(absent), "{absent}");
        }
    }

    #[test]
    fn report_with_hits_lists_every_file_in_the_rail_and_as_a_card() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], group_by: GroupByMode::None, ..Default::default() };
        let html = report_for(
            &settings,
            vec![
                sample_result("/x/b.pdf", vec![hit(1, "line one has apple", &["apple"])]),
                sample_result("/x/A.txt", vec![hit(1, "line one has apple", &["apple"]), hit(2, "x apple", &["apple"])]),
            ],
        );
        // Ungrouped, files are listed by path, case-insensitively.
        let a = html.find("<details class=\"file\" id=\"file-1\"").unwrap();
        assert!(html[a..a + 400].contains("data-name=\"/x/a.txt\""));
        assert!(html.contains("<a href=\"#file-2\" data-id=\"file-2\" style=\"--heat:"));
        assert!(html.contains("data-hits=\"2\""));
        assert!(html.contains("<span class=\"n\">2</span>"));
        assert!(html.contains("<button aria-pressed=\"true\" data-ext=\"pdf\">.pdf</button>"));
        assert!(html.contains("<button aria-pressed=\"true\" data-ext=\"txt\">.txt</button>"));
    }

    #[test]
    fn grouped_report_lists_the_newest_file_first() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], group_by: GroupByMode::Modified, ..Default::default() };
        let mut old = sample_result("/x/old.txt", vec![hit(1, "apple", &["apple"])]);
        old.modified = Local::now() - chrono::Duration::days(800);
        let new = sample_result("/x/new.txt", vec![hit(1, "apple", &["apple"])]);
        let html = report_for(&settings, vec![old, new]);
        let first = html.find("<details class=\"file\" id=\"file-1\"").unwrap();
        assert!(html[first..first + 400].contains("/x/new.txt"));
        assert!(html.contains("files modified"));
    }

    #[test]
    fn report_escapes_hostile_paths_filters_and_lines() {
        let settings = SearchSettings { filters: vec!["<b>".to_string()], ..Default::default() };
        let mut r = sample_result("/x/<script>alert(1)</script>.txt", vec![hit(1, "a <b> c", &["<b>"])]);
        r.lines_cache = vec!["a <b> c".to_string()];
        r.total_line_count = 1;
        let mut run = SearchRunResult::default();
        run.file_results.push(r);
        run.summary.warnings.push(crate::models::Warning { full_name: "/y/<i>.txt".to_string(), message: "<img onerror=x>".to_string() });
        let html = build_html_report(&settings, &run);
        for raw in ["<script>alert", "<b> c", "<img onerror", "/y/<i>"] {
            assert!(!html.contains(raw), "unescaped {raw}");
        }
        assert!(html.contains("a <mark>&lt;b&gt;</mark> c"));
    }

    #[test]
    fn report_is_self_contained_and_works_without_script() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let html = report_for(&settings, vec![sample_result("/x/a.txt", vec![hit(1, "apple", &["apple"])])]);
        assert!(!html.contains("http://") && !html.contains("https://"), "the report must not reference the network");
        assert!(!html.contains(" src=\"/") && html.matches("<script").count() == 2, "only the two inline scripts");
        assert!(html.contains("<details class=\"file\" id=\"file-1\""));
        // Everything that needs script is marked so the stylesheet hides it when script is off.
        assert!(html.contains("html:not(.js) .js-only"));
        assert!(html.contains("class=\"toolbar js-only\"") && html.contains("data-act=\"copy\""));
        assert!(html.contains("@media print"));
    }

    #[test]
    fn kpis_and_coverage_come_from_the_run_summary() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let mut run = SearchRunResult::default();
        run.summary.files_searched = 1234;
        run.summary.skipped_too_large = 3;
        run.summary.skipped_binary = 5;
        run.summary.skipped_read_error = 1;
        run.file_results.push(sample_result("/x/a.txt", vec![hit(1, "apple", &["apple"])]));
        let html = build_html_report(&settings, &run);
        assert!(html.contains("<div class=\"val\">1,234</div>"));
        // 1234 of 1238 files that could be considered were read.
        assert!(html.contains("<div class=\"val\">99.7<span class=\"unit\"> %</span></div>"), "{html}");
        assert!(html.contains("9 skipped"));
        assert!(html.contains("3 too large") && html.contains("5 binary"));
    }

    #[test]
    fn coverage_with_nothing_skipped_says_so() {
        let html = report_for(&SearchSettings::default(), vec![sample_result("/x/a.txt", vec![hit(1, "apple", &["apple"])])]);
        assert!(html.contains("No files were skipped."));
    }

    #[test]
    fn many_file_types_are_folded_into_other() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let results: Vec<FileSearchResult> = ["a", "b", "c", "d", "e", "f", "g"]
            .iter()
            .map(|e| sample_result(&format!("/x/f.{e}"), vec![hit(1, "apple", &["apple"])]))
            .collect();
        let html = report_for(&settings, results);
        assert!(html.contains("other <b>2</b>"), "{html}");
        assert_eq!(html.matches("stroke-dasharray").count(), COLOR_SLOTS);
    }

    #[test]
    fn pptx_cards_name_the_slides_that_hold_hits() {
        let settings = SearchSettings { filters: vec!["engine".to_string()], ..Default::default() };
        let mut r = sample_result("/x/deck.pptx", vec![hit(4, "engine specs", &["engine"])]);
        r.lines_cache = vec!["--- Slide 1 ---".into(), "intro".into(), "--- Slide 2 ---".into(), "engine specs".into()];
        r.total_line_count = 4;
        let html = report_for(&settings, vec![r]);
        assert!(html.contains("Location <b>Slide 2</b>"));
    }

    #[test]
    fn hits_beyond_the_preview_are_listed_with_their_context() {
        let settings = SearchSettings { filters: vec!["apple".to_string()], ..Default::default() };
        let mut r = sample_result("/x/a.txt", vec![hit(50, "late apple", &["apple"])]);
        r.hits[0].before = Some("before".to_string());
        r.hits[0].after = Some("after".to_string());
        r.total_line_count = 60;
        let html = report_for(&settings, vec![r]);
        assert!(html.contains("beyond the shown preview"));
        assert!(html.contains("<div class=\"ln hit\"><i>50</i><span>late <mark>apple</mark></span></div>"));
        assert!(html.contains("<div class=\"ln ctx\"><i></i><span>before</span></div>"));
        assert!(html.contains("Showing lines 1-2 of 60"));
    }

    #[test]
    fn hit_strip_places_ticks_by_relative_position_once_per_pixel() {
        let mut r = sample_result("/x/a.txt", vec![hit(1, "a", &["a"]), hit(1000, "a", &["a"]), hit(1000, "a", &["a"])]);
        r.total_line_count = 1000;
        let svg = hit_strip(&r, 1000);
        assert_eq!(svg.matches("<rect x=").count(), 2 + 1 - 1 + 1 - 1, "background plus one tick per distinct position: {svg}");
        assert!(svg.contains("x=\"3\"") && svg.contains("x=\"79\""), "{svg}");
    }

    #[test]
    fn number_and_size_formatting() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1_000), "1,000");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(human_size(3 * 1024 * 1024 + 512 * 1024), "3.5 MB");
        assert_eq!(human_size(-5), "0 B");
    }
}
