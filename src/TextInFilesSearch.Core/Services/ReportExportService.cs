using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using TextInFilesSearch.Models;

namespace TextInFilesSearch.Services;

/// <summary>
/// Exports a completed search run as a self-contained HTML report (with the
/// same dark-mode CSS, table of contents, and per-filter bar chart as the
/// PowerShell tool's report) and/or flat CSV/JSON files, one row per hit.
/// </summary>
public static class ReportExportService
{
    /// <summary>
    /// Base64-encoded GS Engineering banner, embedded as a data URI in every
    /// report so it stays a single self-contained file (matches the rest of
    /// this report - no external image to go missing if it's moved).
    /// Computed once and cached; falls back to no banner (rather than
    /// failing the whole report) if the embedded resource is ever missing.
    /// </summary>
    private static readonly Lazy<string?> BannerDataUri = new(() =>
    {
        using var stream = typeof(ReportExportService).Assembly.GetManifestResourceStream("Banner.jpg");
        if (stream is null) return null;
        using var ms = new MemoryStream();
        stream.CopyTo(ms);
        return "data:image/jpeg;base64," + Convert.ToBase64String(ms.ToArray());
    });

    public static string BuildHtmlReport(SearchSettings settings, SearchRunResult run)
    {
        var sb = new StringBuilder();
        sb.AppendLine("<!DOCTYPE html>");
        sb.AppendLine("<html lang=\"en\"><head><meta charset=\"UTF-8\">");
        sb.AppendLine("<title>Text Search Report</title>");
        sb.AppendLine(CssBlock);
        sb.AppendLine("</head><body>");
        if (BannerDataUri.Value is { } bannerUri)
        {
            sb.AppendLine($"<img class=\"report-banner\" src=\"{bannerUri}\" alt=\"GS Engineering\" />");
        }
        sb.AppendLine("<h1>Text Search Report</h1>");

        var hitResults = run.FileResults.Where(r => r.Status == FileSearchStatus.Hit).ToList();

        sb.AppendLine("<div class=\"summary\">");
        sb.AppendLine($"<div><strong>Search folder:</strong> {Html(settings.SearchPath)}</div>");
        sb.AppendLine($"<div><strong>Filters:</strong> {Html(string.Join(", ", settings.Filters))} {(settings.UseRegex ? "(regex mode)" : "")} {(settings.WholeWord && !settings.UseRegex ? "(whole word)" : "")}</div>");
        if (settings.ExcludeFilters.Count > 0)
            sb.AppendLine($"<div><strong>Excluding:</strong> {Html(string.Join(", ", settings.ExcludeFilters))} (scope: {settings.ExcludeScope})</div>");
        sb.AppendLine($"<div><strong>Match mode:</strong> {settings.MatchMode}{(settings.MatchMode == MatchMode.Proximity ? $" (within {settings.ProximityLines} line(s))" : "")} &nbsp; <strong>Grouped by:</strong> {settings.GroupBy}{(settings.Parallel ? " &nbsp; <strong>Parallel:</strong> yes" : "")}</div>");
        sb.AppendLine($"<div><strong>Run time:</strong> {DateTime.Now:yyyy-MM-dd HH:mm:ss}</div>");
        sb.AppendLine($"<div><strong>Files searched:</strong> {run.Summary.FilesSearched} &nbsp; <strong>Files with hits:</strong> {hitResults.Count} &nbsp; <strong>Total hits:</strong> {hitResults.Sum(r => r.Hits.Count)}</div>");
        sb.AppendLine($"<div><strong>Skipped:</strong> {run.Summary.SkippedTooLarge} too large, {run.Summary.SkippedBinary} binary, {run.Summary.SkippedReadError} unreadable/locked/unsupported, {run.Summary.SkippedUnexpectedError} with unexpected errors, {run.Summary.SkippedByExclude} excluded, {run.Summary.SkippedByMode} missing required filters</div>");

        var aggregateCounts = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
        foreach (var r in hitResults)
            foreach (var h in r.Hits)
                foreach (var mf in h.MatchedFilters)
                    aggregateCounts[mf] = aggregateCounts.GetValueOrDefault(mf) + 1;

        if (aggregateCounts.Count > 0)
        {
            int maxCount = aggregateCounts.Values.Max();
            sb.AppendLine("<div style=\"margin-top:0.6em;\"><strong>Hits by filter:</strong></div>");
            foreach (var f in settings.Filters)
            {
                int c = aggregateCounts.GetValueOrDefault(f);
                int pct = maxCount > 0 ? (int)(100.0 * c / maxCount) : 0;
                sb.AppendLine($"<div class=\"bar-row\"><span class=\"bar-label\" title=\"{Html(f)}\">{Html(f)}</span><span class=\"bar-track\"><span class=\"bar-fill\" style=\"width:{pct}%\"></span></span><span class=\"bar-count\">{c}</span></div>");
            }
        }

        sb.AppendLine("<div style=\"margin-top:0.6em;\">Click a file below to expand its content in this page. A separate small link inside lets you open the real file if you want it.</div>");
        sb.AppendLine("</div>");

        if (hitResults.Count == 0)
        {
            sb.AppendLine("<p class=\"no-hits\">No matches found.</p>");
        }
        else if (settings.GroupBy == GroupByMode.None)
        {
            var ordered = hitResults.OrderBy(r => r.FullName, StringComparer.OrdinalIgnoreCase).ToList();
            AppendToc(sb, ordered.Select((r, i) => (Path.GetFileName(r.FullName), $"file-{i + 1}")));
            int idx = 0;
            foreach (var r in ordered)
            {
                idx++;
                AppendFileBlock(sb, r, settings, $"file-{idx}");
            }
        }
        else
        {
            var grouped = hitResults
                .Select(r => new
                {
                    Result = r,
                    Date = settings.GroupBy == GroupByMode.Created ? r.Created : r.Modified
                })
                .ToList();

            var byYear = grouped.GroupBy(x => x.Date.Year).OrderByDescending(g => g.Key).ToList();

            AppendToc(sb, byYear.Select(g => (g.Key.ToString(CultureInfo.InvariantCulture), $"year-{g.Key}")));

            int fileAnchor = 0;
            foreach (var yearGroup in byYear)
            {
                int yearHits = yearGroup.Sum(x => x.Result.Hits.Count);
                sb.AppendLine($"<details class=\"year-block\" id=\"year-{yearGroup.Key}\"><summary class=\"year-summary\">{yearGroup.Key} &mdash; {yearGroup.Count()} file(s), {yearHits} hit(s)</summary>");
                sb.AppendLine("<div class=\"year-body\">");

                var byMonth = yearGroup.GroupBy(x => x.Date.Month).OrderByDescending(g => g.Key);
                foreach (var monthGroup in byMonth)
                {
                    string monthName = new DateTime(yearGroup.Key, monthGroup.Key, 1).ToString("MMMM", CultureInfo.InvariantCulture);
                    int monthHits = monthGroup.Sum(x => x.Result.Hits.Count);
                    sb.AppendLine($"<details class=\"month-block\"><summary class=\"month-summary\">{monthName} &mdash; {monthGroup.Count()} file(s), {monthHits} hit(s)</summary>");
                    sb.AppendLine("<div class=\"month-body\">");

                    foreach (var item in monthGroup.OrderByDescending(x => x.Date))
                    {
                        fileAnchor++;
                        AppendFileBlock(sb, item.Result, settings, $"file-{fileAnchor}");
                    }

                    sb.AppendLine("</div></details>");
                }

                sb.AppendLine("</div></details>");
            }
        }

        sb.AppendLine("</body></html>");
        return sb.ToString();
    }

    private static void AppendToc(StringBuilder sb, IEnumerable<(string Label, string AnchorId)> entries)
    {
        var list = entries.ToList();
        if (list.Count <= 3) return;

        sb.AppendLine("<div class=\"toc\"><strong>Jump to:</strong><br/>");
        foreach (var (label, anchor) in list)
            sb.AppendLine($"<a href=\"#{anchor}\">{Html(label)}</a>");
        sb.AppendLine("</div>");
    }

    private static void AppendFileBlock(StringBuilder sb, FileSearchResult r, SearchSettings settings, string anchorId)
    {
        string safePath = Html(r.FullName);
        var uri = new Uri(r.FullName).AbsoluteUri;
        int hitCount = r.Hits.Count;
        string hitWord = hitCount == 1 ? "hit" : "hits";

        var hitLineMap = r.Hits.ToDictionary(h => h.LineNumber, h => h.MatchedFilters);

        sb.AppendLine($"<details class=\"file-block\" id=\"{anchorId}\">");
        sb.AppendLine($"<summary><span class=\"file-header-text\">{safePath}</span> <span class=\"lineno\">({hitCount} {hitWord})</span></summary>");
        sb.AppendLine("<div class=\"expanded-body\">");
        sb.AppendLine($"<p><a class=\"filelink\" href=\"{uri}\">Open original file &#8599;</a> <span class=\"file-path-text\">{safePath}</span></p>");
        sb.AppendLine($"<p class=\"meta-line\">Created: {r.Created:yyyy-MM-dd HH:mm} &nbsp;|&nbsp; Modified: {r.Modified:yyyy-MM-dd HH:mm}</p>");

        if (r.LowConfidencePdf)
        {
            sb.AppendLine("<p class=\"confidence-note\">This PDF's extracted text looks unreliable (often a sign of embedded/subsetted fonts) - if you expected more hits here, open the file directly to check manually.</p>");
        }

        if (settings.MatchMode == MatchMode.AllInFile)
        {
            sb.AppendLine($"<p class=\"truncate-note\">All required filters were found somewhere in this file: {Html(string.Join(", ", settings.Filters))}</p>");
        }
        else if (settings.MatchMode == MatchMode.Proximity)
        {
            string mrText = r.ProximityMinRange is { } mr ? $"{mr} line(s)" : "unknown";
            sb.AppendLine($"<p class=\"truncate-note\">All required filters found within {mrText} of each other (limit: {settings.ProximityLines}): {Html(string.Join(", ", settings.Filters))}</p>");
        }

        var perFilterCounts = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
        foreach (var h in r.Hits)
            foreach (var mf in h.MatchedFilters)
                perFilterCounts[mf] = perFilterCounts.GetValueOrDefault(mf) + 1;

        if (perFilterCounts.Count > 0)
        {
            var parts = settings.Filters.Where(f => perFilterCounts.ContainsKey(f)).Select(f => $"{Html(f)}: {perFilterCounts[f]}");
            sb.AppendLine($"<p class=\"meta-line\"><strong>Hits by filter:</strong> {string.Join(" &nbsp;|&nbsp; ", parts)}</p>");
        }

        bool truncated = r.TotalLineCount > r.LinesCache.Count;
        if (truncated)
        {
            sb.AppendLine($"<p class=\"truncate-note\">Showing lines 1-{r.LinesCache.Count} of {r.TotalLineCount} total extracted lines below. Open the original file to see the rest.</p>");
        }

        if (r.LinesCache.Count > 0)
        {
            sb.AppendLine("<pre class=\"full-file\">");
            for (int ln = 1; ln <= r.LinesCache.Count; ln++)
            {
                string rawLine = r.LinesCache[ln - 1];
                string numPrefix = Html($"{ln,6}: ");
                if (hitLineMap.TryGetValue(ln, out var matchedFilters))
                {
                    string formatted = HighlightMatches(rawLine, matchedFilters, settings);
                    sb.AppendLine($"<span class=\"hitline\">{numPrefix}{formatted}</span>");
                }
                else
                {
                    sb.AppendLine($"{numPrefix}{Html(rawLine)}");
                }
            }
            sb.AppendLine("</pre>");
        }

        var beyondHits = r.Hits.Where(h => h.LineNumber > r.LinesCache.Count).ToList();
        if (beyondHits.Count > 0)
        {
            sb.AppendLine("<p class=\"truncate-note\">Additional hit(s) beyond the shown preview:</p>");
            foreach (var h in beyondHits)
            {
                sb.AppendLine("<div class=\"hit\">");
                sb.AppendLine($"<div class=\"lineno\">Line {h.LineNumber}</div>");
                if (h.Before is not null) sb.AppendLine($"<pre class=\"context before\">{Html(h.Before)}</pre>");
                sb.AppendLine($"<pre class=\"context matchline\">{HighlightMatches(h.MatchLine, h.MatchedFilters, settings)}</pre>");
                if (h.After is not null) sb.AppendLine($"<pre class=\"context after\">{Html(h.After)}</pre>");
                sb.AppendLine("</div>");
            }
        }

        sb.AppendLine("</div></details>");
    }

    /// <summary>
    /// HTML-encodes a line and wraps every actual matched span - literal,
    /// whole-word, or regex filters alike - in &lt;mark&gt;. Finds real
    /// character ranges in the raw line first (so regex mode is highlighted
    /// too), merges overlapping ranges from different filters, then encodes
    /// each plain/highlighted piece in turn.
    /// </summary>
    private static string HighlightMatches(string line, IReadOnlyList<string> matchedFilters, SearchSettings settings)
    {
        if (string.IsNullOrEmpty(line)) return Html(line);

        var ranges = new List<(int Start, int End)>();
        foreach (var f in matchedFilters)
        {
            if (string.IsNullOrEmpty(f)) continue;
            string pattern = settings.UseRegex ? f
                : settings.WholeWord ? WholeWordHelper.BuildPattern(f)
                : System.Text.RegularExpressions.Regex.Escape(f);
            try
            {
                foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(line, pattern, System.Text.RegularExpressions.RegexOptions.IgnoreCase))
                {
                    if (m.Length > 0) ranges.Add((m.Index, m.Index + m.Length));
                }
            }
            catch { /* an invalid/expensive user regex just isn't highlighted here */ }
        }

        if (ranges.Count == 0) return Html(line);

        var sorted = ranges.OrderBy(r => r.Start).ToList();
        var merged = new List<(int Start, int End)>();
        foreach (var r in sorted)
        {
            if (merged.Count > 0 && r.Start <= merged[^1].End)
            {
                if (r.End > merged[^1].End) merged[^1] = (merged[^1].Start, r.End);
            }
            else
            {
                merged.Add(r);
            }
        }

        var sb = new StringBuilder();
        int pos = 0;
        foreach (var (start, end) in merged)
        {
            if (start > pos) sb.Append(Html(line[pos..start]));
            sb.Append("<mark>").Append(Html(line[start..end])).Append("</mark>");
            pos = end;
        }
        if (pos < line.Length) sb.Append(Html(line[pos..]));
        return sb.ToString();
    }

    private static string Html(string? text)
    {
        if (string.IsNullOrEmpty(text)) return string.Empty;
        return text.Replace("&", "&amp;").Replace("<", "&lt;").Replace(">", "&gt;").Replace("\"", "&quot;");
    }

    // ------------------------------------------------------------------
    // CSV / JSON export
    // ------------------------------------------------------------------

    public sealed class ExportRow
    {
        public string FilePath { get; set; } = string.Empty;
        public int LineNumber { get; set; }
        public string MatchedFilters { get; set; } = string.Empty;
        public string? Before { get; set; }
        public string MatchLine { get; set; } = string.Empty;
        public string? After { get; set; }
        public DateTime Created { get; set; }
        public DateTime Modified { get; set; }
    }

    public static List<ExportRow> BuildExportRows(SearchRunResult run) =>
        run.FileResults
           .Where(r => r.Status == FileSearchStatus.Hit)
           .SelectMany(r => r.Hits.Select(h => new ExportRow
           {
               FilePath = r.FullName,
               LineNumber = h.LineNumber,
               MatchedFilters = string.Join("; ", h.MatchedFilters),
               Before = h.Before,
               MatchLine = h.MatchLine,
               After = h.After,
               Created = r.Created,
               Modified = r.Modified
           }))
           .ToList();

    public static void WriteCsv(string path, List<ExportRow> rows)
    {
        var sb = new StringBuilder();
        sb.AppendLine("FilePath,LineNumber,MatchedFilters,Before,MatchLine,After,Created,Modified");
        foreach (var row in rows)
        {
            sb.AppendLine(string.Join(",",
                CsvField(row.FilePath), CsvField(row.LineNumber.ToString(CultureInfo.InvariantCulture)),
                CsvField(row.MatchedFilters), CsvField(row.Before), CsvField(row.MatchLine), CsvField(row.After),
                CsvField(row.Created.ToString("o")), CsvField(row.Modified.ToString("o"))));
        }
        File.WriteAllText(path, sb.ToString(), Encoding.UTF8);
    }

    /// <summary>Leading characters that a spreadsheet application (Excel, Google Sheets, LibreOffice) treats as the start of a formula.</summary>
    private static readonly char[] CsvFormulaTriggerChars = { '=', '+', '-', '@', '\t', '\r' };

    /// <summary>
    /// CSV-encodes one field, guarding against CSV/formula injection: this
    /// export reflects arbitrary matched file content verbatim, so a line
    /// starting with =, +, -, or @ would execute as a formula if the CSV is
    /// opened in a spreadsheet app. Prefixing with a single quote neutralizes
    /// that while keeping the visible text unchanged, per standard OWASP
    /// CSV-injection guidance.
    /// </summary>
    private static string CsvField(string? value)
    {
        value ??= string.Empty;
        if (value.Length > 0 && CsvFormulaTriggerChars.Contains(value[0]))
        {
            value = "'" + value;
        }
        bool needsQuoting = value.Contains(',') || value.Contains('"') || value.Contains('\n') || value.Contains('\r');
        string escaped = value.Replace("\"", "\"\"");
        return needsQuoting ? $"\"{escaped}\"" : escaped;
    }

    public static void WriteJson(string path, List<ExportRow> rows)
    {
        var options = new JsonSerializerOptions { WriteIndented = true };
        File.WriteAllText(path, JsonSerializer.Serialize(rows, options), Encoding.UTF8);
    }

    private const string CssBlock = @"<style>
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
}
