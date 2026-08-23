using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;

namespace TextInFilesSearch.Services;

/// <summary>
/// Best-effort, dependency-free text extraction for every supported format.
/// This is a direct C# port of the PowerShell tool's extraction functions -
/// same algorithms, same safety limits, same documented limitations - because
/// the original logic was already built entirely on .NET APIs (ZipArchive,
/// DeflateStream, Regex) rather than anything PowerShell-specific.
/// </summary>
public static class TextExtractionService
{
    // ------------------------------------------------------------------
    // Binary sniff
    // ------------------------------------------------------------------

    /// <summary>NUL bytes in the first chunk essentially never appear in real text files.</summary>
    public static bool LooksBinary(byte[] bytes)
    {
        if (bytes is null || bytes.Length == 0) return false;
        int checkLen = Math.Min(bytes.Length, 4096);
        for (int i = 0; i < checkLen; i++)
        {
            if (bytes[i] == 0) return true;
        }
        return false;
    }

    // ------------------------------------------------------------------
    // Encoding detection
    // ------------------------------------------------------------------

    /// <summary>
    /// Converts bytes to text with basic encoding detection: BOM first, then a
    /// strict UTF-8 validity check, falling back to Windows-1252 for files
    /// with neither - avoids garbled characters on older non-UTF8 text files.
    /// </summary>
    public static string DecodeText(byte[] bytes)
    {
        if (bytes is null || bytes.Length == 0) return string.Empty;

        if (bytes.Length >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF)
            return Encoding.UTF8.GetString(bytes, 3, bytes.Length - 3);

        if (bytes.Length >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE)
            return Encoding.Unicode.GetString(bytes, 2, bytes.Length - 2);

        if (bytes.Length >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF)
            return Encoding.BigEndianUnicode.GetString(bytes, 2, bytes.Length - 2);

        try
        {
            var strictUtf8 = new UTF8Encoding(false, throwOnInvalidBytes: true);
            return strictUtf8.GetString(bytes);
        }
        catch (DecoderFallbackException)
        {
            try
            {
                // Windows-1252 requires the CodePages provider to be registered
                // once at startup (done in App.xaml.cs) since .NET doesn't
                // include legacy code pages by default.
                return Encoding.GetEncoding(1252).GetString(bytes);
            }
            catch
            {
                return Encoding.Latin1.GetString(bytes);
            }
        }
    }

    // ------------------------------------------------------------------
    // Line splitting
    // ------------------------------------------------------------------

    /// <summary>
    /// Splits on \r\n, \n, and a lone \r (classic Mac line endings) alike -
    /// splitting only on \r\n/\n left old-format files as one giant "line",
    /// making line numbers and before/after context in the report meaningless
    /// for that file even though substring/regex matching still found hits.
    /// </summary>
    public static string[] SplitLines(string text)
    {
        if (string.IsNullOrEmpty(text)) return Array.Empty<string>();
        return text.Replace("\r\n", "\n").Replace("\r", "\n").Split('\n');
    }

    // ------------------------------------------------------------------
    // DOCX
    // ------------------------------------------------------------------

    public static string[]? ExtractDocxLines(byte[] bytes)
    {
        try
        {
            using var ms = new MemoryStream(bytes);
            using var zip = new ZipArchive(ms, ZipArchiveMode.Read);
            var entry = zip.GetEntry("word/document.xml");
            if (entry is null) return null;

            string xml;
            using (var reader = new StreamReader(entry.Open()))
            {
                xml = reader.ReadToEnd();
            }

            xml = xml.Replace("</w:p>", "\n").Replace("<w:br/>", "\n").Replace("<w:br />", "\n");
            xml = Regex.Replace(xml, "<[^>]+>", string.Empty);
            xml = DecodeXmlEntities(xml);

            return SplitLines(xml);
        }
        catch
        {
            return null;
        }
    }

    // ------------------------------------------------------------------
    // PPTX (slides, speaker notes, and SmartArt diagram text)
    // ------------------------------------------------------------------

    public static string[]? ExtractPptxLines(byte[] bytes)
    {
        try
        {
            using var ms = new MemoryStream(bytes);
            using var zip = new ZipArchive(ms, ZipArchiveMode.Read);

            var slideEntries = zip.Entries
                .Where(e => Regex.IsMatch(e.FullName, @"^ppt/slides/slide(\d+)\.xml$"))
                .Select(e => new { Entry = e, Num = int.Parse(Regex.Match(e.FullName, @"\d+").Value) })
                .OrderBy(x => x.Num)
                .ToList();

            var noteEntries = zip.Entries
                .Where(e => Regex.IsMatch(e.FullName, @"^ppt/notesSlides/notesSlide(\d+)\.xml$"))
                .ToDictionary(e => int.Parse(Regex.Match(e.FullName, @"\d+").Value), e => e);

            var diagramEntries = zip.Entries
                .Where(e => Regex.IsMatch(e.FullName, @"^ppt/diagrams/data\d*\.xml$"))
                .OrderBy(e => e.FullName, StringComparer.OrdinalIgnoreCase)
                .ToList();

            if (slideEntries.Count == 0 && diagramEntries.Count == 0) return null;

            var allLines = new List<string>();

            foreach (var s in slideEntries)
            {
                allLines.Add($"--- Slide {s.Num} ---");
                allLines.AddRange(ExtractPptxPartText(s.Entry));

                if (noteEntries.TryGetValue(s.Num, out var noteEntry))
                {
                    var noteLines = ExtractPptxPartText(noteEntry);
                    if (noteLines.Count > 0)
                    {
                        allLines.Add($"--- Slide {s.Num} notes ---");
                        allLines.AddRange(noteLines);
                    }
                }
            }

            int diagramNum = 0;
            foreach (var d in diagramEntries)
            {
                diagramNum++;
                var diagramLines = ExtractPptxPartText(d);
                if (diagramLines.Count > 0)
                {
                    allLines.Add($"--- SmartArt diagram {diagramNum} ---");
                    allLines.AddRange(diagramLines);
                }
            }

            return allLines.Count > 0 ? allLines.ToArray() : null;
        }
        catch
        {
            return null;
        }
    }

    /// <summary>Shared drawingml (a:p/a:t) text extraction used for slides, speaker notes, and SmartArt diagram data - all three use the same run/paragraph markup.</summary>
    private static List<string> ExtractPptxPartText(ZipArchiveEntry entry)
    {
        string xml;
        using (var reader = new StreamReader(entry.Open()))
        {
            xml = reader.ReadToEnd();
        }

        xml = xml.Replace("</a:p>", "\n").Replace("<a:br/>", "\n").Replace("<a:br />", "\n");
        xml = Regex.Replace(xml, "<[^>]+>", string.Empty);
        xml = DecodeXmlEntities(xml);

        var result = new List<string>();
        foreach (var line in SplitLines(xml))
        {
            if (line.Trim().Length > 0) result.Add(line);
        }
        return result;
    }

    // ------------------------------------------------------------------
    // XLSX
    // ------------------------------------------------------------------

    /// <summary>
    /// Best-effort, dependency-free XLSX extraction: resolves xl/sharedStrings.xml,
    /// then walks each xl/worksheets/sheetN.xml emitting one "line" per row
    /// (cell values tab-joined) - same regex tag-strip approach as DOCX/PPTX
    /// rather than a full XML parser, consistent with the rest of this file.
    /// Sheets are numbered by file order rather than resolved to their real
    /// names (would require parsing workbook.xml + its rels), matching the
    /// existing "Slide N" convention used for PPTX.
    /// </summary>
    public static string[]? ExtractXlsxLines(byte[] bytes)
    {
        try
        {
            using var ms = new MemoryStream(bytes);
            using var zip = new ZipArchive(ms, ZipArchiveMode.Read);

            var sharedStrings = new List<string>();
            var sharedEntry = zip.GetEntry("xl/sharedStrings.xml");
            if (sharedEntry is not null)
            {
                string sharedXml;
                using (var reader = new StreamReader(sharedEntry.Open()))
                {
                    sharedXml = reader.ReadToEnd();
                }

                foreach (Match m in Regex.Matches(sharedXml, @"(?s)<si\b[^>]*>(.*?)</si>"))
                {
                    string inner = Regex.Replace(m.Groups[1].Value, "<[^>]+>", string.Empty);
                    sharedStrings.Add(DecodeXmlEntities(inner));
                }
            }

            var sheetEntries = zip.Entries
                .Where(e => Regex.IsMatch(e.FullName, @"^xl/worksheets/sheet(\d+)\.xml$"))
                .Select(e => new { Entry = e, Num = int.Parse(Regex.Match(e.FullName, @"\d+").Value) })
                .OrderBy(x => x.Num)
                .ToList();

            if (sheetEntries.Count == 0) return null;

            var allLines = new List<string>();

            foreach (var s in sheetEntries)
            {
                string xml;
                using (var reader = new StreamReader(s.Entry.Open()))
                {
                    xml = reader.ReadToEnd();
                }

                allLines.Add($"--- Sheet {s.Num} ---");

                foreach (Match rowMatch in Regex.Matches(xml, @"(?s)<row\b[^>]*>(.*?)</row>"))
                {
                    var cellValues = new List<string>();
                    foreach (Match cellMatch in Regex.Matches(rowMatch.Value, @"(?s)<c\b([^>]*)>(.*?)</c>"))
                    {
                        string cellAttrs = cellMatch.Groups[1].Value;
                        string cellInner = cellMatch.Groups[2].Value;
                        bool isSharedString = Regex.IsMatch(cellAttrs, "t=\"s\"");
                        bool isInlineString = Regex.IsMatch(cellAttrs, "t=\"(inlineStr|str)\"");

                        string cellText;
                        if (isSharedString)
                        {
                            var vMatch = Regex.Match(cellInner, @"(?s)<v>(.*?)</v>");
                            cellText = vMatch.Success
                                && int.TryParse(vMatch.Groups[1].Value, out int idx)
                                && idx >= 0 && idx < sharedStrings.Count
                                ? sharedStrings[idx]
                                : string.Empty;
                        }
                        else if (isInlineString)
                        {
                            string stripped = Regex.Replace(cellInner, "<[^>]+>", string.Empty);
                            cellText = DecodeXmlEntities(stripped);
                        }
                        else
                        {
                            var vMatch = Regex.Match(cellInner, @"(?s)<v>(.*?)</v>");
                            cellText = vMatch.Success ? DecodeXmlEntities(vMatch.Groups[1].Value) : string.Empty;
                        }

                        if (cellText.Length > 0) cellValues.Add(cellText);
                    }

                    if (cellValues.Count > 0) allLines.Add(string.Join("\t", cellValues));
                }
            }

            return allLines.Count > 0 ? allLines.ToArray() : null;
        }
        catch
        {
            return null;
        }
    }

    // ------------------------------------------------------------------
    // ZIP archives (recurse into entries, dispatch by inner extension)
    // ------------------------------------------------------------------

    private const int ZipMaxEntriesScanned = 500;
    private const long ZipMaxUncompressedBytesTotal = 200_000_000;
    private const long ZipMaxEntryUncompressedBytes = 20_000_000;

    private sealed class ZipScanState
    {
        public int EntriesScanned;
        public long TotalBytesRead;
    }

    /// <summary>
    /// Best-effort search inside a .zip archive: recurses into each entry and
    /// dispatches to the matching extractor by extension (including nested
    /// zips, up to maxDepth). Bounded against zip-bomb-style abuse by a
    /// shared entry-count and total-uncompressed-bytes budget across the
    /// whole scan (including nested zips, not reset per nesting level), plus
    /// a per-entry size cap - oversized or over-budget entries are silently
    /// skipped rather than causing a hang or an out-of-memory failure.
    /// </summary>
    public static string[]? ExtractZipArchiveLines(byte[] bytes, int maxDepth = 2)
    {
        try
        {
            var lines = new List<string>();
            var state = new ZipScanState();
            ExtractZipEntries(bytes, lines, depth: 0, maxDepth, state);
            return lines.Count > 0 ? lines.ToArray() : null;
        }
        catch
        {
            return null;
        }
    }

    private static void ExtractZipEntries(byte[] bytes, List<string> lines, int depth, int maxDepth, ZipScanState state)
    {
        if (depth > maxDepth) return;

        ZipArchive zip;
        try
        {
            var ms = new MemoryStream(bytes);
            zip = new ZipArchive(ms, ZipArchiveMode.Read);
        }
        catch
        {
            return;
        }

        using (zip)
        {
            foreach (var entry in zip.Entries)
            {
                if (state.EntriesScanned >= ZipMaxEntriesScanned)
                {
                    lines.Add("[... zip entry scan limit reached, remaining entries skipped ...]");
                    return;
                }
                if (string.IsNullOrEmpty(entry.Name)) continue; // directory entry
                if (entry.Length > ZipMaxEntryUncompressedBytes) continue;
                if (state.TotalBytesRead + entry.Length > ZipMaxUncompressedBytesTotal)
                {
                    lines.Add("[... zip total-size scan limit reached, remaining entries skipped ...]");
                    return;
                }

                state.EntriesScanned++;

                byte[] entryBytes;
                try
                {
                    using var es = entry.Open();
                    using var msEntry = new MemoryStream();
                    es.CopyTo(msEntry);
                    entryBytes = msEntry.ToArray();
                }
                catch
                {
                    continue;
                }

                state.TotalBytesRead += entryBytes.LongLength;

                string ext = Path.GetExtension(entry.Name).ToLowerInvariant();
                switch (ext)
                {
                    case ".docx":
                        AppendEntryLines(lines, entry.FullName, ExtractDocxLines(entryBytes));
                        break;
                    case ".pptx":
                        AppendEntryLines(lines, entry.FullName, ExtractPptxLines(entryBytes));
                        break;
                    case ".xlsx":
                        AppendEntryLines(lines, entry.FullName, ExtractXlsxLines(entryBytes));
                        break;
                    case ".rtf":
                        AppendEntryLines(lines, entry.FullName, ExtractRtfLines(entryBytes));
                        break;
                    case ".pdf":
                        AppendEntryLines(lines, entry.FullName, ExtractPdfLines(entryBytes, 5, out _));
                        break;
                    case ".zip":
                        if (depth < maxDepth)
                        {
                            lines.Add($"--- {entry.FullName} ---");
                            ExtractZipEntries(entryBytes, lines, depth + 1, maxDepth, state);
                        }
                        break;
                    default:
                        if (!LooksBinary(entryBytes))
                            AppendEntryLines(lines, entry.FullName, SplitLines(DecodeText(entryBytes)));
                        break;
                }
            }
        }
    }

    private static void AppendEntryLines(List<string> lines, string entryName, string[]? entryLines)
    {
        if (entryLines is null || entryLines.Length == 0) return;
        lines.Add($"--- {entryName} ---");
        lines.AddRange(entryLines);
    }

    private static string DecodeXmlEntities(string xml) =>
        xml.Replace("&amp;", "&").Replace("&lt;", "<").Replace("&gt;", ">")
           .Replace("&quot;", "\"").Replace("&apos;", "'");

    // ------------------------------------------------------------------
    // RTF
    // ------------------------------------------------------------------

    private static readonly HashSet<string> RtfIgnoreGroups = new(StringComparer.Ordinal)
    {
        "fonttbl", "colortbl", "stylesheet", "info", "generator", "pict",
        "object", "footer", "footerf", "footerl", "footerr",
        "header", "headerf", "headerl", "headerr",
        "footnote", "xe", "tc", "field", "shppict", "nonshppict",
        "themedata", "colorschememapping", "datastore", "listtable", "listoverridetable"
    };

    /// <summary>
    /// Small dependency-free RTF-to-text converter. Walks the RTF character by
    /// character tracking group nesting, skips destination groups with no
    /// visible document text, and converts \par/\line/\tab and \uNNNN / \'hh
    /// escapes into real characters. Not a full RTF spec implementation.
    /// Returns null if it doesn't look like RTF.
    /// </summary>
    public static string[]? ExtractRtfLines(byte[] bytes)
    {
        string raw = DecodeText(bytes);
        if (!raw.StartsWith("{\\rtf", StringComparison.Ordinal)) return null;

        var sb = new StringBuilder();
        int len = raw.Length;
        int i = 0;
        int depth = 0;
        int skipDepth = -1;

        while (i < len)
        {
            char ch = raw[i];

            if (ch == '{') { depth++; i++; continue; }
            if (ch == '}')
            {
                if (skipDepth >= 0 && depth <= skipDepth) skipDepth = -1;
                depth--;
                i++;
                continue;
            }

            if (ch == '\\')
            {
                i++;
                if (i >= len) break;
                char c2 = raw[i];

                if (c2 == '*')
                {
                    i++;
                    if (skipDepth < 0) skipDepth = depth;
                    continue;
                }
                else if (IsAsciiLetter(c2))
                {
                    int wordStart = i;
                    while (i < len && IsAsciiLetter(raw[i])) i++;
                    string word = raw.Substring(wordStart, i - wordStart);

                    int numStart = i;
                    if (i < len && raw[i] == '-') i++;
                    while (i < len && char.IsDigit(raw[i])) i++;
                    string numStr = raw.Substring(numStart, i - numStart);

                    if (i < len && raw[i] == ' ') i++;

                    if (RtfIgnoreGroups.Contains(word))
                    {
                        if (skipDepth < 0) skipDepth = depth;
                    }
                    else if (word is "par" or "line" or "row" or "cell")
                    {
                        if (skipDepth < 0) sb.Append('\n');
                    }
                    else if (word == "tab")
                    {
                        if (skipDepth < 0) sb.Append('\t');
                    }
                    else if (word == "u")
                    {
                        if (skipDepth < 0 && numStr.Length > 0 && int.TryParse(numStr, out int codepoint))
                        {
                            if (codepoint < 0) codepoint += 65536;
                            try { sb.Append((char)codepoint); } catch { /* ignore invalid codepoint */ }
                        }
                        if (i < len && raw[i] != '\\' && raw[i] != '{' && raw[i] != '}') i++;
                    }
                    continue;
                }
                else
                {
                    if (c2 == '\'')
                    {
                        i++;
                        if (i + 1 < len)
                        {
                            string hex = raw.Substring(i, 2);
                            i += 2;
                            if (skipDepth < 0 && int.TryParse(hex, System.Globalization.NumberStyles.HexNumber, null, out int byteVal))
                            {
                                sb.Append((char)byteVal);
                            }
                        }
                        continue;
                    }
                    else if (c2 == '\\' || c2 == '{' || c2 == '}')
                    {
                        if (skipDepth < 0) sb.Append(c2);
                        i++;
                        continue;
                    }
                    else if (c2 == '~')
                    {
                        if (skipDepth < 0) sb.Append(' ');
                        i++;
                        continue;
                    }
                    else
                    {
                        i++;
                        continue;
                    }
                }
            }

            if (skipDepth < 0) sb.Append(ch);
            i++;
        }

        return SplitLines(sb.ToString());
    }

    private static bool IsAsciiLetter(char c) => (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');

    // ------------------------------------------------------------------
    // PDF (best-effort, no OCR, no ToUnicode CMap resolution - see docs)
    // ------------------------------------------------------------------

    private static readonly Regex StreamRegexTemplate = new(
        @"(?s)(.{0,400}?)stream\r?\n(.*?)endstream",
        RegexOptions.None, TimeSpan.FromSeconds(2));

    private static readonly Regex TextRegexTemplate = new(
        @"\((?:\\.|[^()])*\)",
        RegexOptions.None, TimeSpan.FromSeconds(2));

    private static readonly Regex SkipMarkerRegex = new("/Image|/FontFile|/ICCBased|/Metadata", RegexOptions.None);

    /// <summary>
    /// Progress callback invoked periodically while a single PDF is being
    /// processed, so the UI can show real activity (streams scanned, elapsed
    /// time) instead of going silent on a slow file - the exact complaint
    /// about the PowerShell version's console output.
    /// </summary>
    public delegate void PdfProgressCallback(int streamsScanned, TimeSpan elapsed);

    /// <summary>
    /// Lightweight, dependency-free, BEST-EFFORT PDF text extractor. Finds
    /// stream...endstream blocks, decodes /ASCII85Decode and/or /FlateDecode
    /// filtered streams, and pulls text out of Tj/TJ show-text operators.
    ///
    /// PERFORMANCE / HANG SAFEGUARDS: skips streams whose own dictionary marks
    /// them as images, embedded font programs, ICC profiles, or metadata
    /// without decompressing them; applies a short timeout to every regex
    /// match attempt; caps how much of any one stream gets scanned; stops
    /// after overallTimeoutSeconds total and keeps whatever text was already
    /// found, flagging the result as truncated. Calls onProgress periodically
    /// so the caller can surface "still working, N streams scanned, Ys
    /// elapsed" rather than the UI appearing frozen on a large/complex PDF.
    ///
    /// LIMITATIONS: no OCR; does not resolve ToUnicode CMaps, so PDFs with
    /// embedded/subsetted fonts (common from LaTeX/pdflatex) may extract as
    /// garbled or missing text; filters other than ASCII85Decode/FlateDecode
    /// are not handled.
    /// </summary>
    public static string[]? ExtractPdfLines(
        byte[] bytes,
        int overallTimeoutSeconds,
        out bool truncatedByTime,
        PdfProgressCallback? onProgress = null)
    {
        truncatedByTime = false;
        var latin1 = Encoding.Latin1;
        string raw = latin1.GetString(bytes);

        var lines = new List<string>();
        var sw = System.Diagnostics.Stopwatch.StartNew();
        var lastProgressReport = TimeSpan.Zero;
        int streamsScanned = 0;
        const int maxContentChars = 2_000_000;

        Match? match;
        try
        {
            match = StreamRegexTemplate.Match(raw);
        }
        catch
        {
            return null;
        }

        while (match is { Success: true })
        {
            if (sw.Elapsed.TotalSeconds >= overallTimeoutSeconds)
            {
                truncatedByTime = true;
                break;
            }

            streamsScanned++;
            if (onProgress is not null && (sw.Elapsed - lastProgressReport).TotalMilliseconds >= 150)
            {
                onProgress(streamsScanned, sw.Elapsed);
                lastProgressReport = sw.Elapsed;
            }

            try
            {
                string header = match.Groups[1].Value;

                if (!SkipMarkerRegex.IsMatch(header))
                {
                    string streamText = match.Groups[2].Value;

                    if (streamText.Length > 0)
                    {
                        bool hasAscii85 = header.Contains("/ASCII85Decode", StringComparison.Ordinal);
                        bool hasFlate = header.Contains("/FlateDecode", StringComparison.Ordinal);

                        byte[]? workingBytes;
                        if (hasAscii85)
                        {
                            try { workingBytes = DecodeAscii85(streamText); }
                            catch { workingBytes = null; }
                        }
                        else
                        {
                            workingBytes = latin1.GetBytes(streamText);
                        }

                        byte[]? contentBytes = null;
                        if (workingBytes is { Length: > 0 })
                        {
                            if (hasFlate)
                            {
                                if (workingBytes.Length > 2)
                                {
                                    try
                                    {
                                        using var msIn = new MemoryStream(workingBytes, 2, workingBytes.Length - 2);
                                        using var ds = new DeflateStream(msIn, CompressionMode.Decompress);
                                        using var msOut = new MemoryStream();
                                        ds.CopyTo(msOut);
                                        contentBytes = msOut.ToArray();
                                    }
                                    catch
                                    {
                                        contentBytes = null;
                                    }
                                }
                            }
                            else
                            {
                                contentBytes = workingBytes;
                            }
                        }

                        if (contentBytes is { Length: > 0 })
                        {
                            int contentLen = Math.Min(contentBytes.Length, maxContentChars);
                            string content = latin1.GetString(contentBytes, 0, contentLen);

                            bool looksLikeText = false;
                            try
                            {
                                looksLikeText = Regex.IsMatch(content, @"\bTj\b|\bTJ\b", RegexOptions.None, TimeSpan.FromSeconds(2));
                            }
                            catch { /* timeout - treat as not text */ }

                            if (looksLikeText)
                            {
                                try
                                {
                                    foreach (Match tm in TextRegexTemplate.Matches(content))
                                    {
                                        string inner = tm.Value.Substring(1, tm.Value.Length - 2);
                                        inner = UnescapePdfString(inner);
                                        if (inner.Trim().Length > 0) lines.Add(inner);
                                    }
                                }
                                catch { /* timeout mid-scan - keep what we have */ }
                            }
                        }
                    }
                }
            }
            catch
            {
                // Any per-stream failure just means we move on to the next stream
                // rather than losing the whole file.
            }

            try
            {
                match = match.NextMatch();
            }
            catch
            {
                truncatedByTime = true;
                break;
            }
        }

        onProgress?.Invoke(streamsScanned, sw.Elapsed);

        if (truncatedByTime && lines.Count > 0)
        {
            lines.Add($"[... PDF text extraction stopped early after {overallTimeoutSeconds} seconds on this large/complex file - some text may be missing ...]");
        }

        return lines.Count == 0 ? null : lines.ToArray();
    }

    private static string UnescapePdfString(string inner)
    {
        return Regex.Replace(inner, @"\\([\\()nrtbf]|[0-7]{1,3})", m =>
        {
            string g = m.Groups[1].Value;
            if (Regex.IsMatch(g, "^[0-7]{1,3}$"))
            {
                try { return ((char)Convert.ToInt32(g, 8)).ToString(); } catch { return string.Empty; }
            }
            return g switch
            {
                "n" => "\n",
                "r" => "\r",
                "t" => "\t",
                "b" => ((char)8).ToString(),
                "f" => ((char)12).ToString(),
                "(" => "(",
                ")" => ")",
                "\\" => "\\",
                _ => g
            };
        });
    }

    /// <summary>
    /// Cheap heuristic to flag PDFs whose extracted text is probably garbled -
    /// typically PDFs with embedded/subsetted fonts using custom glyph
    /// encodings (common from LaTeX/pdflatex) that this extractor can't decode
    /// correctly. Not a guarantee either way - a hint to double-check manually.
    /// </summary>
    public static bool PdfExtractionLooksReliable(string[] lines)
    {
        if (lines is null || lines.Length == 0) return false;

        string sampleText = string.Join(" ", lines.Take(200));
        if (sampleText.Length == 0) return false;

        int letters = 0, spaces = 0, printable = 0;
        int total = sampleText.Length;

        foreach (char ch in sampleText)
        {
            if (char.IsLetter(ch)) letters++;
            if (ch == ' ') spaces++;
            if (!char.IsControl(ch)) printable++;
        }

        double letterRatio = (double)letters / total;
        double spaceRatio = (double)spaces / total;
        double printableRatio = (double)printable / total;

        return letterRatio > 0.35 && spaceRatio > 0.08 && printableRatio > 0.9;
    }

    // ------------------------------------------------------------------
    // ASCII85 (PDF/Adobe variant)
    // ------------------------------------------------------------------

    /// <summary>
    /// Decodes PDF-style ASCII85 (Adobe variant: lowercase 'z' shorthand for
    /// four zero bytes - deliberately a case-sensitive char comparison, since
    /// an uppercase 'Z' is a perfectly ordinary data character, not the
    /// shorthand - this exact confusion was a real bug in the PowerShell
    /// version before its default case-insensitive comparison was caught and
    /// fixed). Optional '~&gt;' end marker; whitespace is ignored.
    /// </summary>
    public static byte[] DecodeAscii85(string text)
    {
        string t = Regex.Replace(text, @"\s", string.Empty);
        if (t.EndsWith("~>", StringComparison.Ordinal)) t = t[..^2];

        var outBytes = new List<byte>();
        var group = new int[5];
        int count = 0;

        foreach (char ch in t)
        {
            if (ch == 'z' && count == 0)
            {
                outBytes.Add(0); outBytes.Add(0); outBytes.Add(0); outBytes.Add(0);
                continue;
            }

            int val = ch - 33;
            if (val < 0 || val > 84) continue;

            group[count] = val;
            count++;
            if (count == 5)
            {
                ulong num = 0;
                foreach (int g in group) num = num * 85 + (ulong)g;
                outBytes.Add((byte)((num >> 24) & 0xFF));
                outBytes.Add((byte)((num >> 16) & 0xFF));
                outBytes.Add((byte)((num >> 8) & 0xFF));
                outBytes.Add((byte)(num & 0xFF));
                count = 0;
            }
        }

        if (count > 0)
        {
            int padCount = 5 - count;
            for (int p = 0; p < padCount; p++) group[count + p] = 84;
            ulong num = 0;
            foreach (int g in group) num = num * 85 + (ulong)g;
            var tmp = new byte[]
            {
                (byte)((num >> 24) & 0xFF),
                (byte)((num >> 16) & 0xFF),
                (byte)((num >> 8) & 0xFF),
                (byte)(num & 0xFF)
            };
            for (int k = 0; k < count - 1; k++) outBytes.Add(tmp[k]);
        }

        return outBytes.ToArray();
    }
}
