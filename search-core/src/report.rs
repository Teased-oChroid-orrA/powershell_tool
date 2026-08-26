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
use crate::models::{FileSearchStatus, GroupByMode, LineHit, MatchMode, SearchRunResult, SearchSettings};

/// Base64-encoded GS Engineering banner, embedded as a data URI in every
/// report so it stays a single self-contained file. Unlike the C# original
/// (which loads this from an embedded assembly resource and falls back to
/// no banner if missing at runtime), `include_bytes!` embeds it at compile
/// time - if the asset were ever missing, the build itself would fail
/// rather than silently producing a bannerless report.
const BANNER_JPG: &[u8] = include_bytes!("../../src/TextInFilesSearch.Core/Assets/Banner.jpg");

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
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    write_report_to_sink(&mut ReportSink::Writer(&mut writer), settings, run)?;
    std::io::Write::flush(&mut writer)?;
    drop(writer);
    Ok(std::fs::metadata(path)?.len())
}

fn write_report_to_sink(sink: &mut ReportSink, settings: &SearchSettings, run: &SearchRunResult) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n");
    out.push_str("<html lang=\"en\"><head><meta charset=\"UTF-8\">\n");
    out.push_str("<title>Text Search Report</title>\n");
    out.push_str(CSS_BLOCK);
    out.push('\n');
    out.push_str("</head><body>\n");
    out.push_str(&format!(
        "<img class=\"report-banner\" src=\"{}\" alt=\"GS Engineering\" />\n",
        banner_data_uri()
    ));
    out.push_str("<h1>Text Search Report</h1>\n");

    let hit_results: Vec<&crate::models::FileSearchResult> =
        run.file_results.iter().filter(|r| r.status == FileSearchStatus::Hit).collect();

    out.push_str("<div class=\"summary\">\n");
    out.push_str(&format!(
        "<div><strong>Search folder:</strong> {}</div>\n",
        html_escape(&settings.search_path)
    ));
    let regex_note = if settings.use_regex { "(regex mode)" } else { "" };
    let whole_word_note = if settings.whole_word && !settings.use_regex {
        "(whole word)"
    } else {
        ""
    };
    out.push_str(&format!(
        "<div><strong>Filters:</strong> {} {} {}</div>\n",
        html_escape(&settings.filters.join(", ")),
        regex_note,
        whole_word_note
    ));
    if !settings.exclude_filters.is_empty() {
        out.push_str(&format!(
            "<div><strong>Excluding:</strong> {} (scope: {:?})</div>\n",
            html_escape(&settings.exclude_filters.join(", ")),
            settings.exclude_scope
        ));
    }
    let proximity_note = if settings.match_mode == MatchMode::Proximity {
        format!(" (within {} line(s))", settings.proximity_lines)
    } else {
        String::new()
    };
    let parallel_note = if settings.parallel {
        " &nbsp; <strong>Parallel:</strong> yes"
    } else {
        ""
    };
    out.push_str(&format!(
        "<div><strong>Match mode:</strong> {:?}{} &nbsp; <strong>Grouped by:</strong> {:?}{}</div>\n",
        settings.match_mode, proximity_note, settings.group_by, parallel_note
    ));
    out.push_str(&format!(
        "<div><strong>Run time:</strong> {}</div>\n",
        Local::now().format("%Y-%m-%d %H:%M:%S")
    ));
    let total_hits: usize = hit_results.iter().map(|r| r.hits.len()).sum();
    out.push_str(&format!(
        "<div><strong>Files searched:</strong> {} &nbsp; <strong>Files with hits:</strong> {} &nbsp; <strong>Total hits:</strong> {}</div>\n",
        run.summary.files_searched,
        hit_results.len(),
        total_hits
    ));
    out.push_str(&format!(
        "<div><strong>Skipped:</strong> {} too large, {} binary, {} unreadable/locked/unsupported, {} with unexpected errors, {} excluded, {} missing required filters</div>\n",
        run.summary.skipped_too_large,
        run.summary.skipped_binary,
        run.summary.skipped_read_error,
        run.summary.skipped_unexpected_error,
        run.summary.skipped_by_exclude,
        run.summary.skipped_by_mode
    ));

    // Case-insensitive aggregation by filter *text* (not slot) - two
    // filters differing only by case share one bar-chart bucket, matching
    // the C# side's `Dictionary<string,int>(StringComparer.OrdinalIgnoreCase)`.
    let mut aggregate_counts: HashMap<String, i32> = HashMap::new();
    for r in &hit_results {
        for h in &r.hits {
            for mf in &h.matched_filters {
                *aggregate_counts.entry(mf.to_lowercase()).or_insert(0) += 1;
            }
        }
    }

    if !aggregate_counts.is_empty() {
        let max_count = *aggregate_counts.values().max().unwrap();
        out.push_str("<div style=\"margin-top:0.6em;\"><strong>Hits by filter:</strong></div>\n");
        for f in &settings.filters {
            let c = *aggregate_counts.get(&f.to_lowercase()).unwrap_or(&0);
            let pct = if max_count > 0 { (100.0 * c as f64 / max_count as f64) as i32 } else { 0 };
            let f_html = html_escape(f);
            out.push_str(&format!(
                "<div class=\"bar-row\"><span class=\"bar-label\" title=\"{f_html}\">{f_html}</span><span class=\"bar-track\"><span class=\"bar-fill\" style=\"width:{pct}%\"></span></span><span class=\"bar-count\">{c}</span></div>\n"
            ));
        }
    }

    out.push_str(
        "<div style=\"margin-top:0.6em;\">Click a file below to expand its content in this page. A separate small link inside lets you open the real file if you want it.</div>\n",
    );
    out.push_str("</div>\n");
    sink.commit(&mut out)?;

    if hit_results.is_empty() {
        out.push_str("<p class=\"no-hits\">No matches found.</p>\n");
    } else if settings.group_by == GroupByMode::None {
        let mut ordered: Vec<&crate::models::FileSearchResult> = hit_results.clone();
        ordered.sort_by(|a, b| a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()));

        let toc_entries: Vec<(String, String)> = ordered
            .iter()
            .enumerate()
            .map(|(i, r)| (file_name_of(&r.full_name), format!("file-{}", i + 1)))
            .collect();
        append_toc(&mut out, &toc_entries);
        sink.commit(&mut out)?;

        for (idx, r) in ordered.iter().enumerate() {
            append_file_block(&mut out, r, settings, &format!("file-{}", idx + 1));
            sink.commit(&mut out)?;
        }
    } else {
        let dated: Vec<(&crate::models::FileSearchResult, DateTime<Local>)> = hit_results
            .iter()
            .map(|r| {
                let d = if settings.group_by == GroupByMode::Created { r.created } else { r.modified };
                (*r, d)
            })
            .collect();

        let mut years: Vec<i32> = dated.iter().map(|(_, d)| d.year()).collect();
        years.sort_unstable();
        years.dedup();
        years.reverse();

        let toc_entries: Vec<(String, String)> =
            years.iter().map(|y| (y.to_string(), format!("year-{y}"))).collect();
        append_toc(&mut out, &toc_entries);
        sink.commit(&mut out)?;

        let mut file_anchor = 0;
        for year in years {
            let year_items: Vec<&(&crate::models::FileSearchResult, DateTime<Local>)> =
                dated.iter().filter(|(_, d)| d.year() == year).collect();
            let year_hits: usize = year_items.iter().map(|(r, _)| r.hits.len()).sum();
            out.push_str(&format!(
                "<details class=\"year-block\" id=\"year-{year}\"><summary class=\"year-summary\">{year} &mdash; {} file(s), {year_hits} hit(s)</summary>\n",
                year_items.len()
            ));
            out.push_str("<div class=\"year-body\">\n");

            let mut months: Vec<u32> = year_items.iter().map(|(_, d)| d.month()).collect();
            months.sort_unstable();
            months.dedup();
            months.reverse();
            sink.commit(&mut out)?;

            for month in months {
                let mut month_items: Vec<&(&crate::models::FileSearchResult, DateTime<Local>)> =
                    year_items.iter().filter(|(_, d)| d.month() == month).cloned().collect();
                let month_name = month_name_english(month);
                let month_hits: usize = month_items.iter().map(|(r, _)| r.hits.len()).sum();
                out.push_str(&format!(
                    "<details class=\"month-block\"><summary class=\"month-summary\">{month_name} &mdash; {} file(s), {month_hits} hit(s)</summary>\n",
                    month_items.len()
                ));
                out.push_str("<div class=\"month-body\">\n");
                sink.commit(&mut out)?;

                month_items.sort_by(|a, b| b.1.cmp(&a.1));

                for (r, _) in month_items {
                    file_anchor += 1;
                    append_file_block(&mut out, r, settings, &format!("file-{file_anchor}"));
                    sink.commit(&mut out)?;
                }

                out.push_str("</div></details>\n");
            }

            out.push_str("</div></details>\n");
        }
    }

    out.push_str("</body></html>\n");
    sink.commit(&mut out)?;
    Ok(())
}

fn append_toc(out: &mut String, entries: &[(String, String)]) {
    if entries.len() <= 3 {
        return;
    }
    out.push_str("<div class=\"toc\"><strong>Jump to:</strong><br/>\n");
    for (label, anchor) in entries {
        out.push_str(&format!("<a href=\"#{anchor}\">{}</a>\n", html_escape(label)));
    }
    out.push_str("</div>\n");
}

fn append_file_block(out: &mut String, r: &crate::models::FileSearchResult, settings: &SearchSettings, anchor_id: &str) {
    let safe_path = html_escape(&r.full_name);
    let uri = file_uri(&r.full_name);
    let hit_count = r.hits.len();
    let hit_word = if hit_count == 1 { "hit" } else { "hits" };

    let hit_line_map: HashMap<i32, &Vec<String>> = r.hits.iter().map(|h| (h.line_number, &h.matched_filters)).collect();

    out.push_str(&format!("<details class=\"file-block\" id=\"{anchor_id}\">\n"));
    out.push_str(&format!(
        "<summary><span class=\"file-header-text\">{safe_path}</span> <span class=\"lineno\">({hit_count} {hit_word})</span></summary>\n"
    ));
    out.push_str("<div class=\"expanded-body\">\n");
    out.push_str(&format!(
        "<p><a class=\"filelink\" href=\"{uri}\">Open original file &#8599;</a> <span class=\"file-path-text\">{safe_path}</span></p>\n"
    ));
    out.push_str(&format!(
        "<p class=\"meta-line\">Created: {} &nbsp;|&nbsp; Modified: {}</p>\n",
        r.created.format("%Y-%m-%d %H:%M"),
        r.modified.format("%Y-%m-%d %H:%M")
    ));

    if r.low_confidence_pdf {
        out.push_str(
            "<p class=\"confidence-note\">This PDF's extracted text looks unreliable (often a sign of embedded/subsetted fonts) - if you expected more hits here, open the file directly to check manually.</p>\n",
        );
    }

    if settings.match_mode == MatchMode::AllInFile {
        out.push_str(&format!(
            "<p class=\"truncate-note\">All required filters were found somewhere in this file: {}</p>\n",
            html_escape(&settings.filters.join(", "))
        ));
    } else if settings.match_mode == MatchMode::Proximity {
        let mr_text = match r.proximity_min_range {
            Some(mr) => format!("{mr} line(s)"),
            None => "unknown".to_string(),
        };
        out.push_str(&format!(
            "<p class=\"truncate-note\">All required filters found within {mr_text} of each other (limit: {}): {}</p>\n",
            settings.proximity_lines,
            html_escape(&settings.filters.join(", "))
        ));
    }

    let mut per_filter_counts: HashMap<String, i32> = HashMap::new();
    for h in &r.hits {
        for mf in &h.matched_filters {
            *per_filter_counts.entry(mf.to_lowercase()).or_insert(0) += 1;
        }
    }

    if !per_filter_counts.is_empty() {
        let parts: Vec<String> = settings
            .filters
            .iter()
            .filter(|f| per_filter_counts.contains_key(&f.to_lowercase()))
            .map(|f| format!("{}: {}", html_escape(f), per_filter_counts[&f.to_lowercase()]))
            .collect();
        out.push_str(&format!(
            "<p class=\"meta-line\"><strong>Hits by filter:</strong> {}</p>\n",
            parts.join(" &nbsp;|&nbsp; ")
        ));
    }

    let truncated = r.total_line_count as usize > r.lines_cache.len();
    if truncated {
        out.push_str(&format!(
            "<p class=\"truncate-note\">Showing lines 1-{} of {} total extracted lines below. Open the original file to see the rest.</p>\n",
            r.lines_cache.len(),
            r.total_line_count
        ));
    }

    if !r.lines_cache.is_empty() {
        out.push_str("<pre class=\"full-file\">\n");
        for (idx, raw_line) in r.lines_cache.iter().enumerate() {
            let ln = (idx + 1) as i32;
            let num_prefix = html_escape(&format!("{ln:>6}: "));
            if let Some(matched_filters) = hit_line_map.get(&ln) {
                let formatted = highlight_matches(raw_line, matched_filters, settings);
                out.push_str(&format!("<span class=\"hitline\">{num_prefix}{formatted}</span>\n"));
            } else {
                out.push_str(&format!("{num_prefix}{}\n", html_escape(raw_line)));
            }
        }
        out.push_str("</pre>\n");
    }

    let beyond_hits: Vec<&LineHit> = r.hits.iter().filter(|h| h.line_number as usize > r.lines_cache.len()).collect();
    if !beyond_hits.is_empty() {
        out.push_str("<p class=\"truncate-note\">Additional hit(s) beyond the shown preview:</p>\n");
        for h in beyond_hits {
            out.push_str("<div class=\"hit\">\n");
            out.push_str(&format!("<div class=\"lineno\">Line {}</div>\n", h.line_number));
            if let Some(before) = &h.before {
                out.push_str(&format!("<pre class=\"context before\">{}</pre>\n", html_escape(before)));
            }
            out.push_str(&format!(
                "<pre class=\"context matchline\">{}</pre>\n",
                highlight_matches(&h.match_line, &h.matched_filters, settings)
            ));
            if let Some(after) = &h.after {
                out.push_str(&format!("<pre class=\"context after\">{}</pre>\n", html_escape(after)));
            }
            out.push_str("</div>\n");
        }
    }

    out.push_str("</div></details>\n");
}

/// HTML-encodes a line and wraps every actual matched span - literal,
/// whole-word, or regex filters alike - in `<mark>`. Finds real character
/// ranges in the raw line first (so regex mode is highlighted too), merges
/// overlapping ranges from different filters, then encodes each
/// plain/highlighted piece in turn.
fn highlight_matches(line: &str, matched_filters: &[String], settings: &SearchSettings) -> String {
    if line.is_empty() {
        return html_escape(line);
    }

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for f in matched_filters {
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
                ranges.push((m.start(), m.end()));
            }
        }
    }

    if ranges.is_empty() {
        return html_escape(line);
    }

    ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
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

    let mut out = String::new();
    let mut pos = 0usize;
    for (start, end) in merged {
        if start > pos {
            out.push_str(&html_escape(&line[pos..start]));
        }
        out.push_str("<mark>");
        out.push_str(&html_escape(&line[start..end]));
        out.push_str("</mark>");
        pos = end;
    }
    if pos < line.len() {
        out.push_str(&html_escape(&line[pos..]));
    }
    out
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
fn file_uri(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let segments: Vec<String> = normalized.split('/').map(|s| urlencoding::encode(s).into_owned()).collect();
    let joined = segments.join("/");
    if let Some(stripped) = joined.strip_prefix('/') {
        format!("file:///{stripped}")
    } else {
        format!("file:///{joined}")
    }
}

fn month_name_english(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
        "November", "December",
    ];
    NAMES[(month.saturating_sub(1) as usize).min(11)]
}

// ------------------------------------------------------------------
// CSV / JSON export
// ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExportRow {
    pub file_path: String,
    pub line_number: i32,
    pub matched_filters: String,
    pub before: Option<String>,
    pub match_line: String,
    pub after: Option<String>,
    pub created: DateTime<Local>,
    pub modified: DateTime<Local>,
}

pub fn build_export_rows(run: &SearchRunResult) -> Vec<ExportRow> {
    run.file_results
        .iter()
        .filter(|r| r.status == FileSearchStatus::Hit)
        .flat_map(|r| {
            r.hits.iter().map(move |h| ExportRow {
                file_path: r.full_name.clone(),
                line_number: h.line_number,
                matched_filters: h.matched_filters.join("; "),
                before: h.before.clone(),
                match_line: h.match_line.clone(),
                after: h.after.clone(),
                created: r.created,
                modified: r.modified,
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
    writer.write_all(b"FilePath,LineNumber,MatchedFilters,Before,MatchLine,After,Created,Modified\n")?;
    for row in rows {
        let fields = [
            csv_field(Some(&row.file_path)),
            csv_field(Some(&row.line_number.to_string())),
            csv_field(Some(&row.matched_filters)),
            csv_field(row.before.as_deref()),
            csv_field(Some(&row.match_line)),
            csv_field(row.after.as_deref()),
            csv_field(Some(&row.created.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))),
            csv_field(Some(&row.modified.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))),
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

const CSS_BLOCK: &str = "<style>
  :root { --bg:#fafafa; --fg:#222; --panel-bg:#eef2f7; --panel-border:#ccd; --card-bg:#fff; --card-border:#ddd;
           --year-bg:#eef6ff; --year-border:#cfe0f2; --month-bg:#f5faff; --month-border:#dbe9f7;
           --muted:#666; --note-bg:#fff9db; --note-fg:#8a6300; --hit-bg:#fff9db; --mark-bg:#ff9800; --mark-fg:#1a1200;
           --link:#0b5fa5; --pre-bg:#fbfbfb; --pre-border:#eee; --bar-bg:#e3ecf5; --bar-fill:#0b5fa5; --confidence-bg:#fde8e8; --confidence-fg:#7a1f1f; }
  @media (prefers-color-scheme: dark) {
    :root { --bg:#1b1d21; --fg:#e6e6e6; --panel-bg:#242830; --panel-border:#3a3f4a; --card-bg:#20232a; --card-border:#3a3f4a;
             --year-bg:#1d2733; --year-border:#2c3b4d; --month-bg:#1a222c; --month-border:#28374a;
             --muted:#9aa4b2; --note-bg:#3a3320; --note-fg:#e0c060; --hit-bg:#3a3320; --mark-bg:#c97b13; --mark-fg:#fff3dc;
             --link:#6cb2f2; --pre-bg:#181a1f; --pre-border:#33373f; --bar-bg:#2c3441; --bar-fill:#6cb2f2; --confidence-bg:#3a2222; --confidence-fg:#f2a3a3; }
  }
  body { font-family: Segoe UI, Arial, sans-serif; margin: 2em; background: var(--bg); color: var(--fg); }
  img.report-banner { display: block; max-width: 100%; height: auto; border-radius: 8px; margin-bottom: 1em; }
  h1 { font-size: 1.4em; }
  .summary { background: var(--panel-bg); border: 1px solid var(--panel-border); padding: 0.8em 1em; border-radius: 6px; margin-bottom: 1.5em; }
  .toc { background: var(--panel-bg); border: 1px solid var(--panel-border); padding: 0.8em 1em; border-radius: 6px; margin-bottom: 1.5em; max-height: 220px; overflow-y: auto; }
  .toc a { color: var(--link); text-decoration: none; display: inline-block; margin: 0.15em 0.6em 0.15em 0; }
  .toc a:hover { text-decoration: underline; }
  .bar-row { display: flex; align-items: center; margin: 0.2em 0; font-size: 0.9em; }
  .bar-label { width: 140px; flex-shrink: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding-right: 0.5em; }
  .bar-track { flex-grow: 1; background: var(--bar-bg); border-radius: 3px; height: 14px; position: relative; }
  .bar-fill { background: var(--bar-fill); height: 100%; border-radius: 3px; }
  .bar-count { width: 3em; text-align: right; padding-left: 0.5em; flex-shrink: 0; }
  details.year-block { background: var(--year-bg); border: 1px solid var(--year-border); border-radius: 6px; margin-bottom: 1em; padding: 0.2em 1em; }
  details.year-block > summary.year-summary { cursor: pointer; padding: 0.6em 0; font-weight: 700; font-size: 1.1em; }
  details.month-block { background: var(--month-bg); border: 1px solid var(--month-border); border-radius: 6px; margin: 0.6em 0; padding: 0.2em 1em; }
  details.month-block > summary.month-summary { cursor: pointer; padding: 0.5em 0; font-weight: 600; }
  details.file-block { background: var(--card-bg); border: 1px solid var(--card-border); border-radius: 6px; margin-bottom: 0.8em; padding: 0.2em 1em; }
  details.file-block > summary { cursor: pointer; padding: 0.6em 0; font-weight: 600; }
  details.file-block > summary:hover { color: var(--link); }
  .file-header-text { word-break: break-all; }
  .expanded-body { padding: 0.4em 0 0.8em 0; border-top: 1px dashed var(--card-border); }
  .file-path-text { color: var(--muted); font-size: 0.85em; word-break: break-all; }
  .meta-line { color: var(--muted); font-size: 0.85em; }
  a.filelink { text-decoration: none; color: var(--link); font-weight: 600; }
  a.filelink:hover { text-decoration: underline; }
  .lineno { color: var(--muted); font-size: 0.85em; font-weight: normal; }
  .truncate-note { color: var(--note-fg); background: var(--note-bg); padding: 0.4em 0.6em; border-radius: 4px; font-size: 0.9em; }
  .confidence-note { color: var(--confidence-fg); background: var(--confidence-bg); padding: 0.4em 0.6em; border-radius: 4px; font-size: 0.9em; }
  pre.full-file { white-space: pre-wrap; word-break: break-word; font-family: Consolas, monospace; font-size: 0.9em; background: var(--pre-bg); border: 1px solid var(--pre-border); padding: 0.6em; border-radius: 4px; max-height: 70vh; overflow-y: auto; color: var(--fg); }
  span.hitline { display: block; background: var(--hit-bg); border-left: 3px solid #e0b400; padding-left: 0.2em; }
  mark { background: var(--mark-bg); color: var(--mark-fg); font-weight: 700; padding: 0 3px; border-radius: 2px; }
  .hit { border-top: 1px dashed var(--card-border); padding: 0.5em 0; }
  pre.context { margin: 0.15em 0; padding: 0.15em 0.4em; white-space: pre-wrap; word-break: break-word; font-family: Consolas, monospace; color: var(--fg); }
  pre.before, pre.after { color: var(--muted); }
  pre.matchline { background: var(--hit-bg); border-left: 3px solid #e0b400; }
  .no-hits { color: var(--muted); font-style: italic; }
</style>";

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
        assert!(html.contains("<mark>banana</mark>"));
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
        assert_eq!(written, expected, "streamed file content must match the in-memory build exactly");
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
            matched_filters: "x".to_string(),
            before: None,
            match_line: "=SUM(A1:A10)".to_string(),
            after: None,
            created: Local::now(),
            modified: Local::now(),
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
            matched_filters: "apple".to_string(),
            before: None,
            match_line: "apple".to_string(),
            after: None,
            created: Local::now(),
            modified: Local::now(),
        };
        write_json(path.to_str().unwrap(), &[row]).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.is_empty());
        assert!(content.contains("\"FilePath\""));
        assert!(content.contains("\"LineNumber\""));
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
}
