//! Ports `TextInFilesSearch.Core/Services/TextExtractionService.cs`.
//!
//! The C# original is deliberately dependency-free: DOCX/PPTX/XLSX are read
//! via `ZipArchive` + regex tag-stripping rather than a real OOXML parser,
//! and PDF is read via a hand-rolled stream/ASCII85/FlateDecode walker
//! rather than a PDF library. This port follows the same approach with
//! `zip` + `flate2` + `regex` filling the role `ZipArchive`/`DeflateStream`/
//! `Regex` played in .NET, rather than adopting heavier format-specific
//! crates (`calamine`, `docx-rust`, `pdf-extract`, ...) whose extraction
//! algorithms would differ from the original and risk silently drifting
//! from the byte-for-byte-tested behavior the existing fixture tests pin
//! down (see docs/rust-rewrite-status.md).
//!
//! One deliberate difference from `regex` crate usage here vs. C#: none of
//! the patterns in this file need lookaround, so the plain (non-backtracking)
//! `regex` crate is used throughout, not `fancy-regex`. That also means the
//! C# side's explicit `TimeSpan.FromSeconds(2)` per-regex timeout (a ReDoS
//! guard against .NET's backtracking engine) has no equivalent need here:
//! `regex` is immune to catastrophic backtracking by construction. The PDF
//! extractor's *overall* wall-clock timeout is a different, still-needed
//! thing (bounding total work across many streams, not any single regex)
//! and is ported via `std::time::Instant` below.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use regex::Regex;

// --------------------------------------------------------------------
// Extension dispatch
// --------------------------------------------------------------------

/// Result of [`extract_lines_by_extension`] on success - just the lines
/// plus the one extractor-specific signal (`extract_pdf_lines`'s
/// reliability heuristic) any caller needs, not the full `FileSearchResult`
/// shape (that's `orchestrator`'s job, not extraction's).
pub struct ExtractedLines {
    pub lines: Vec<String>,
    pub low_confidence_pdf: bool,
}

/// Distinguishes *why* extraction produced no lines - `orchestrator.rs`
/// maps this to a different `FileSearchStatus` per variant (`Binary` vs
/// `ReadError`), so this can't just collapse to `Option<Vec<String>>`.
#[derive(Debug, PartialEq, Eq)]
pub enum ExtractLinesError {
    /// NUL byte sniffed in the first chunk - not a text-bearing format at
    /// all, never even attempted.
    Binary,
    /// The format-specific extractor ran but returned nothing usable
    /// (`None`, or `Some(vec![])`).
    Failed,
}

/// Extension-to-extractor dispatch, factored out of `orchestrator.rs`'s
/// `process_one_file` so the same table backs both the normal search path
/// and the proactive corpus indexer (`native_index.rs`) - one place that
/// knows "which extractor for which extension," not two that could drift
/// apart as formats are added (see `CLAUDE.md`'s extraction design notes
/// for why each format's extractor itself is hand-rolled, not a generic
/// parser crate - this function only owns the dispatch, not the parsing).
///
/// `on_pdf_progress` mirrors `extract_pdf_lines`'s own live-progress
/// callback - required by this app's standing "PDF extraction must never
/// go silent" rule (see CLAUDE.md's "Live progress reporting" section);
/// pass `None` for callers (like the corpus indexer) that don't have a
/// per-file progress UI to feed.
pub fn extract_lines_by_extension(
    ext: &str,
    bytes: &[u8],
    pdf_timeout_seconds: u64,
    on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>,
) -> Result<ExtractedLines, ExtractLinesError> {
    let mut low_confidence_pdf = false;

    let lines: Option<Vec<String>> = match ext {
        ".docx" => extract_docx_lines(bytes),
        ".pptx" => extract_pptx_lines(bytes),
        ".xlsx" => extract_xlsx_lines(bytes),
        ".zip" => extract_zip_archive_lines(bytes, 2),
        ".pdf" => {
            let (pdf_lines, _truncated) = extract_pdf_lines(bytes, pdf_timeout_seconds, on_pdf_progress);
            if let Some(pl) = &pdf_lines {
                low_confidence_pdf = !pdf_extraction_looks_reliable(pl);
            }
            pdf_lines
        }
        ".rtf" => extract_rtf_lines(bytes),
        _ => {
            if looks_binary(bytes) {
                return Err(ExtractLinesError::Binary);
            }
            Some(split_lines(&decode_text(bytes)))
        }
    };

    match lines {
        Some(l) if !l.is_empty() => Ok(ExtractedLines { lines: l, low_confidence_pdf }),
        _ => Err(ExtractLinesError::Failed),
    }
}

// --------------------------------------------------------------------
// Binary sniff
// --------------------------------------------------------------------

/// NUL bytes in the first chunk essentially never appear in real text files.
pub fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let check_len = bytes.len().min(4096);
    bytes[..check_len].contains(&0)
}

// --------------------------------------------------------------------
// Encoding detection
// --------------------------------------------------------------------

/// Converts bytes to text with basic encoding detection: BOM first, then a
/// strict UTF-8 validity check, falling back to Windows-1252 for files with
/// neither - avoids garbled characters on older non-UTF8 text files.
pub fn decode_text(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return decode_utf16le(&bytes[2..]);
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        return decode_utf16be(&bytes[2..]);
    }

    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            // The C# original (TextExtractionService.cs) falls back to
            // Windows-1252 for the *whole file* the instant strict UTF-8
            // fails, with no partial-validity check - ported 1:1 at first.
            // That whole-file fallback has a real bug though, present in
            // both: a file that's genuinely UTF-8 but has a handful of
            // stray invalid bytes (mid-file corruption, a paste from a
            // different encoding, one bad byte from a buggy writer) gets
            // its ENTIRE text re-decoded as Windows-1252, turning every
            // correct multi-byte UTF-8 character (accented letters, smart
            // quotes, em-dashes, ...) into mojibake - e.g. the UTF-8 bytes
            // for U+2019 (right single quote) get reread one byte at a time as three
            // separate cp1252 glyphs ("â€™"). That's the exact "gibberish
            // in .txt results" pattern - deliberately diverging from
            // strict parity here since this is a plain heuristic, not an
            // extraction algorithm the byte-for-byte fixture tests pin
            // down.
            //
            // Fix: measure how much of the file is actually invalid UTF-8
            // (summing the invalid byte runs `from_utf8`'s error reports).
            // If it's a small fraction, the file is genuinely UTF-8 with
            // isolated corruption - use `from_utf8_lossy`, which preserves
            // every valid multi-byte character and only replaces the
            // actually-bad bytes with U+FFFD. Only fall back to decoding
            // the whole file as Windows-1252 when a large fraction of it
            // fails UTF-8 validity - the systematic-high-bit-byte pattern
            // of a genuinely legacy-encoded file.
            let invalid_ratio = utf8_invalid_byte_ratio(bytes);
            if invalid_ratio < 0.05 {
                String::from_utf8_lossy(bytes).into_owned()
            } else {
                // Windows-1252 decode is infallible in encoding_rs (every
                // byte value maps to something) - no further fallback
                // needed, unlike the C# side's belt-and-suspenders Latin1
                // catch (registering the 1252 codepage never fails in
                // encoding_rs the way .NET's optional CodePages provider
                // can).
                let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
                decoded.into_owned()
            }
        }
    }
}

/// Fraction of `bytes` that lies within an invalid UTF-8 sequence, per
/// `str::from_utf8`'s own error reporting (`valid_up_to`/`error_len`) -
/// walks the reported invalid runs rather than re-implementing UTF-8
/// validation.
fn utf8_invalid_byte_ratio(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut invalid = 0usize;
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(_) => break,
            Err(e) => {
                let bad_len = e.error_len().unwrap_or(rest.len() - e.valid_up_to());
                invalid += bad_len;
                let advance = e.valid_up_to() + bad_len;
                if advance == 0 || advance > rest.len() {
                    invalid += rest.len().saturating_sub(e.valid_up_to());
                    break;
                }
                rest = &rest[advance..];
                if rest.is_empty() {
                    break;
                }
            }
        }
    }
    invalid as f64 / bytes.len() as f64
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

fn decode_utf16be(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Decodes raw bytes as Latin-1 (ISO-8859-1): every byte maps 1:1 to the
/// Unicode scalar value of the same number, which is always a valid `char`.
/// Used for the PDF extractor, which - like the C# original - deliberately
/// treats PDF content as an opaque byte stream via a lossless byte<->char
/// mapping rather than "real" text decoding, since PDF stream bytes are not
/// necessarily any particular text encoding.
fn decode_latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Inverse of `decode_latin1` - valid as long as the string only contains
/// characters produced by `decode_latin1` (which is the only way strings
/// reach this function in this module).
fn encode_latin1(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

// --------------------------------------------------------------------
// Line splitting
// --------------------------------------------------------------------

/// Splits on \r\n, \n, and a lone \r (classic Mac line endings) alike -
/// splitting only on \r\n/\n left old-format files as one giant "line",
/// making line numbers and before/after context in the report meaningless
/// for that file even though substring/regex matching still found hits.
pub fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(|s| s.to_string())
        .collect()
}

// --------------------------------------------------------------------
// Shared OOXML helpers
// --------------------------------------------------------------------

fn tag_strip_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("<[^>]+>").unwrap())
}

fn strip_tags(xml: &str) -> String {
    tag_strip_re().replace_all(xml, "").into_owned()
}

fn decode_xml_entities(xml: &str) -> String {
    xml.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn read_zip_entry_to_string(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut entry = zip.by_name(name).ok()?;
    let mut s = String::new();
    entry.read_to_string(&mut s).ok()?;
    Some(s)
}

// --------------------------------------------------------------------
// DOCX
// --------------------------------------------------------------------

pub fn extract_docx_lines(bytes: &[u8]) -> Option<Vec<String>> {
    let cursor = Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;
    let xml = read_zip_entry_to_string(&mut zip, "word/document.xml")?;

    let xml = xml
        .replace("</w:p>", "\n")
        .replace("<w:br/>", "\n")
        .replace("<w:br />", "\n");
    let xml = strip_tags(&xml);
    let xml = decode_xml_entities(&xml);

    Some(split_lines(&xml))
}

// --------------------------------------------------------------------
// PPTX (slides, speaker notes, and SmartArt diagram text)
// --------------------------------------------------------------------

fn slide_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^ppt/slides/slide(\d+)\.xml$").unwrap())
}

fn notes_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^ppt/notesSlides/notesSlide(\d+)\.xml$").unwrap())
}

fn diagram_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^ppt/diagrams/data\d*\.xml$").unwrap())
}

pub fn extract_pptx_lines(bytes: &[u8]) -> Option<Vec<String>> {
    let cursor = Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;

    let names: Vec<String> = zip.file_names().map(|s| s.to_string()).collect();

    let mut slide_entries: Vec<(i32, String)> = names
        .iter()
        .filter_map(|n| slide_re().captures(n).map(|c| (c[1].parse::<i32>().unwrap(), n.clone())))
        .collect();
    slide_entries.sort_by_key(|(num, _)| *num);

    let mut note_entries: HashMap<i32, String> = HashMap::new();
    for n in &names {
        if let Some(c) = notes_re().captures(n) {
            note_entries.insert(c[1].parse().unwrap(), n.clone());
        }
    }

    let mut diagram_entries: Vec<String> = names
        .iter()
        .filter(|n| diagram_re().is_match(n))
        .cloned()
        .collect();
    diagram_entries.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));

    if slide_entries.is_empty() && diagram_entries.is_empty() {
        return None;
    }

    let mut all_lines = Vec::new();

    for (num, name) in &slide_entries {
        all_lines.push(format!("--- Slide {} ---", num));
        let entry_text = read_zip_entry_to_string(&mut zip, name)?;
        all_lines.extend(extract_pptx_part_text(&entry_text));

        if let Some(note_name) = note_entries.get(num) {
            let note_text = read_zip_entry_to_string(&mut zip, note_name)?;
            let note_lines = extract_pptx_part_text(&note_text);
            if !note_lines.is_empty() {
                all_lines.push(format!("--- Slide {} notes ---", num));
                all_lines.extend(note_lines);
            }
        }
    }

    let mut diagram_num = 0;
    for name in &diagram_entries {
        diagram_num += 1;
        let entry_text = read_zip_entry_to_string(&mut zip, name)?;
        let diagram_lines = extract_pptx_part_text(&entry_text);
        if !diagram_lines.is_empty() {
            all_lines.push(format!("--- SmartArt diagram {} ---", diagram_num));
            all_lines.extend(diagram_lines);
        }
    }

    if all_lines.is_empty() {
        None
    } else {
        Some(all_lines)
    }
}

/// Shared drawingml (a:p/a:t) text extraction used for slides, speaker
/// notes, and SmartArt diagram data - all three use the same run/paragraph
/// markup.
fn extract_pptx_part_text(xml: &str) -> Vec<String> {
    let replaced = xml
        .replace("</a:p>", "\n")
        .replace("<a:br/>", "\n")
        .replace("<a:br />", "\n");
    let stripped = strip_tags(&replaced);
    let decoded = decode_xml_entities(&stripped);
    split_lines(&decoded)
        .into_iter()
        .filter(|l| !l.trim().is_empty())
        .collect()
}

// --------------------------------------------------------------------
// XLSX
// --------------------------------------------------------------------

fn shared_si_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<si\b[^>]*>(.*?)</si>").unwrap())
}

fn sheet_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^xl/worksheets/sheet(\d+)\.xml$").unwrap())
}

fn row_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<row\b[^>]*>(.*?)</row>").unwrap())
}

fn cell_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?s)<c\b([^>]*)>(.*?)</c>"#).unwrap())
}

fn inline_str_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"t="(inlineStr|str)""#).unwrap())
}

fn v_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<v>(.*?)</v>").unwrap())
}

/// Best-effort, dependency-free XLSX extraction: resolves
/// `xl/sharedStrings.xml`, then walks each `xl/worksheets/sheetN.xml`
/// emitting one "line" per row (cell values tab-joined) - same regex
/// tag-strip approach as DOCX/PPTX rather than a full XML parser. Sheets
/// are numbered by file order rather than resolved to their real names,
/// matching the "Slide N" convention used for PPTX.
pub fn extract_xlsx_lines(bytes: &[u8]) -> Option<Vec<String>> {
    let cursor = Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).ok()?;

    let mut shared_strings: Vec<String> = Vec::new();
    if let Some(shared_xml) = read_zip_entry_to_string(&mut zip, "xl/sharedStrings.xml") {
        for cap in shared_si_re().captures_iter(&shared_xml) {
            let inner = strip_tags(&cap[1]);
            shared_strings.push(decode_xml_entities(&inner));
        }
    }

    let names: Vec<String> = zip.file_names().map(|s| s.to_string()).collect();
    let mut sheet_entries: Vec<(i32, String)> = names
        .iter()
        .filter_map(|n| sheet_re().captures(n).map(|c| (c[1].parse::<i32>().unwrap(), n.clone())))
        .collect();
    sheet_entries.sort_by_key(|(num, _)| *num);

    if sheet_entries.is_empty() {
        return None;
    }

    let mut all_lines = Vec::new();

    for (num, name) in &sheet_entries {
        let xml = read_zip_entry_to_string(&mut zip, name)?;
        all_lines.push(format!("--- Sheet {} ---", num));

        for row_cap in row_re().captures_iter(&xml) {
            let row_content = &row_cap[0];
            let mut cell_values = Vec::new();

            for cell_cap in cell_re().captures_iter(row_content) {
                let cell_attrs = &cell_cap[1];
                let cell_inner = &cell_cap[2];
                // Plain substring search, not regex - `t="s"` has no
                // special regex characters, matching the C# side's
                // `Regex.IsMatch(cellAttrs, "t=\"s\"")` behavior exactly
                // while skipping the unnecessary regex compile.
                let is_shared_string = cell_attrs.contains("t=\"s\"");
                let is_inline_string = inline_str_re().is_match(cell_attrs);

                let cell_text = if is_shared_string {
                    v_re()
                        .captures(cell_inner)
                        .and_then(|v| v[1].parse::<usize>().ok())
                        .and_then(|idx| shared_strings.get(idx).cloned())
                        .unwrap_or_default()
                } else if is_inline_string {
                    let stripped = strip_tags(cell_inner);
                    decode_xml_entities(&stripped)
                } else {
                    v_re()
                        .captures(cell_inner)
                        .map(|v| decode_xml_entities(&v[1]))
                        .unwrap_or_default()
                };

                if !cell_text.is_empty() {
                    cell_values.push(cell_text);
                }
            }

            if !cell_values.is_empty() {
                all_lines.push(cell_values.join("\t"));
            }
        }
    }

    if all_lines.is_empty() {
        None
    } else {
        Some(all_lines)
    }
}

// --------------------------------------------------------------------
// ZIP archives (recurse into entries, dispatch by inner extension)
// --------------------------------------------------------------------

const ZIP_MAX_ENTRIES_SCANNED: i32 = 500;
const ZIP_MAX_UNCOMPRESSED_BYTES_TOTAL: i64 = 200_000_000;
const ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES: i64 = 20_000_000;

#[derive(Default)]
struct ZipScanState {
    entries_scanned: i32,
    total_bytes_read: i64,
}

/// Best-effort search inside a .zip archive: recurses into each entry and
/// dispatches to the matching extractor by extension (including nested
/// zips, up to `max_depth`). Bounded against zip-bomb-style abuse by a
/// shared entry-count and total-uncompressed-bytes budget across the whole
/// scan (including nested zips, not reset per nesting level), plus a
/// per-entry size cap - oversized or over-budget entries are silently
/// skipped rather than causing a hang or an out-of-memory failure.
pub fn extract_zip_archive_lines(bytes: &[u8], max_depth: i32) -> Option<Vec<String>> {
    let mut lines = Vec::new();
    let mut state = ZipScanState::default();
    extract_zip_entries(bytes, &mut lines, 0, max_depth, &mut state);
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

fn extract_zip_entries(
    bytes: &[u8],
    lines: &mut Vec<String>,
    depth: i32,
    max_depth: i32,
    state: &mut ZipScanState,
) {
    if depth > max_depth {
        return;
    }

    let cursor = Cursor::new(bytes);
    let mut zip = match zip::ZipArchive::new(cursor) {
        Ok(z) => z,
        Err(_) => return,
    };

    for i in 0..zip.len() {
        if state.entries_scanned >= ZIP_MAX_ENTRIES_SCANNED {
            lines.push("[... zip entry scan limit reached, remaining entries skipped ...]".to_string());
            return;
        }

        let (full_name, is_dir, entry_len) = {
            let entry = match zip.by_index(i) {
                Ok(e) => e,
                Err(_) => continue,
            };
            (entry.name().to_string(), entry.is_dir(), entry.size() as i64)
        };

        if is_dir {
            continue;
        }
        if entry_len > ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES {
            continue;
        }
        if state.total_bytes_read + entry_len > ZIP_MAX_UNCOMPRESSED_BYTES_TOTAL {
            lines.push("[... zip total-size scan limit reached, remaining entries skipped ...]".to_string());
            return;
        }

        state.entries_scanned += 1;

        let mut entry_bytes = Vec::new();
        {
            let mut entry = match zip.by_index(i) {
                Ok(e) => e,
                Err(_) => continue,
            };
            if entry.read_to_end(&mut entry_bytes).is_err() {
                continue;
            }
        }

        state.total_bytes_read += entry_bytes.len() as i64;

        let ext = path_extension_lower(&full_name);
        match ext.as_str() {
            ".docx" => append_entry_lines(lines, &full_name, extract_docx_lines(&entry_bytes)),
            ".pptx" => append_entry_lines(lines, &full_name, extract_pptx_lines(&entry_bytes)),
            ".xlsx" => append_entry_lines(lines, &full_name, extract_xlsx_lines(&entry_bytes)),
            ".rtf" => append_entry_lines(lines, &full_name, extract_rtf_lines(&entry_bytes)),
            ".pdf" => {
                let (pdf_lines, _truncated) = extract_pdf_lines(&entry_bytes, 5, None);
                append_entry_lines(lines, &full_name, pdf_lines);
            }
            ".zip" => {
                if depth < max_depth {
                    lines.push(format!("--- {} ---", full_name));
                    extract_zip_entries(&entry_bytes, lines, depth + 1, max_depth, state);
                }
            }
            _ => {
                if !looks_binary(&entry_bytes) {
                    append_entry_lines(lines, &full_name, Some(split_lines(&decode_text(&entry_bytes))));
                }
            }
        }
    }
}

fn append_entry_lines(lines: &mut Vec<String>, entry_name: &str, entry_lines: Option<Vec<String>>) {
    if let Some(el) = entry_lines {
        if !el.is_empty() {
            lines.push(format!("--- {} ---", entry_name));
            lines.extend(el);
        }
    }
}

fn path_extension_lower(full_name: &str) -> String {
    let base = full_name.rsplit('/').next().unwrap_or(full_name);
    match base.rfind('.') {
        Some(idx) => base[idx..].to_lowercase(),
        None => String::new(),
    }
}

// --------------------------------------------------------------------
// RTF
// --------------------------------------------------------------------

const RTF_IGNORE_GROUPS: &[&str] = &[
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "generator",
    "pict",
    "object",
    "footer",
    "footerf",
    "footerl",
    "footerr",
    "header",
    "headerf",
    "headerl",
    "headerr",
    "footnote",
    "xe",
    "tc",
    "field",
    "shppict",
    "nonshppict",
    "themedata",
    "colorschememapping",
    "datastore",
    "listtable",
    "listoverridetable",
];

/// Small dependency-free RTF-to-text converter. Walks the RTF character by
/// character tracking group nesting, skips destination groups with no
/// visible document text, and converts \par/\line/\tab and \uNNNN / \'hh
/// escapes into real characters. Not a full RTF spec implementation.
/// Returns `None` if it doesn't look like RTF.
///
/// One platform difference from the C# original: a `\uNNNN` escape whose
/// codepoint falls in the UTF-16 surrogate range (0xD800-0xDFFF) is silently
/// dropped here rather than appended, because Rust's `String` must be valid
/// UTF-8 and cannot hold an unpaired surrogate the way a .NET `string` can.
/// This only affects malformed/unusual RTF with a lone surrogate escape,
/// not normal documents.
pub fn extract_rtf_lines(bytes: &[u8]) -> Option<Vec<String>> {
    let raw = decode_text(bytes);
    if !raw.starts_with("{\\rtf") {
        return None;
    }

    let chars: Vec<char> = raw.chars().collect();
    let len = chars.len();
    let mut out = String::new();
    let mut i = 0usize;
    let mut depth: i32 = 0;
    let mut skip_depth: i32 = -1;

    while i < len {
        let ch = chars[i];

        if ch == '{' {
            depth += 1;
            i += 1;
            continue;
        }
        if ch == '}' {
            if skip_depth >= 0 && depth <= skip_depth {
                skip_depth = -1;
            }
            depth -= 1;
            i += 1;
            continue;
        }

        if ch == '\\' {
            i += 1;
            if i >= len {
                break;
            }
            let c2 = chars[i];

            if c2 == '*' {
                i += 1;
                if skip_depth < 0 {
                    skip_depth = depth;
                }
                continue;
            } else if c2.is_ascii_alphabetic() {
                let word_start = i;
                while i < len && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word: String = chars[word_start..i].iter().collect();

                let num_start = i;
                if i < len && chars[i] == '-' {
                    i += 1;
                }
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let num_str: String = chars[num_start..i].iter().collect();

                if i < len && chars[i] == ' ' {
                    i += 1;
                }

                if RTF_IGNORE_GROUPS.contains(&word.as_str()) {
                    if skip_depth < 0 {
                        skip_depth = depth;
                    }
                } else if word == "par" || word == "line" || word == "row" || word == "cell" {
                    if skip_depth < 0 {
                        out.push('\n');
                    }
                } else if word == "tab" {
                    if skip_depth < 0 {
                        out.push('\t');
                    }
                } else if word == "u" {
                    if skip_depth < 0 {
                        if let Ok(mut codepoint) = num_str.parse::<i32>() {
                            if codepoint < 0 {
                                codepoint += 65536;
                            }
                            if let Some(c) = char::from_u32(codepoint as u32) {
                                out.push(c);
                            }
                        }
                    }
                    if i < len && chars[i] != '\\' && chars[i] != '{' && chars[i] != '}' {
                        i += 1;
                    }
                }
                continue;
            } else if c2 == '\'' {
                i += 1;
                if i + 1 < len {
                    let hex: String = chars[i..i + 2].iter().collect();
                    i += 2;
                    if skip_depth < 0 {
                        if let Ok(byte_val) = i32::from_str_radix(&hex, 16) {
                            if let Some(c) = char::from_u32(byte_val as u32) {
                                out.push(c);
                            }
                        }
                    }
                }
                continue;
            } else if c2 == '\\' || c2 == '{' || c2 == '}' {
                if skip_depth < 0 {
                    out.push(c2);
                }
                i += 1;
                continue;
            } else if c2 == '~' {
                if skip_depth < 0 {
                    out.push(' ');
                }
                i += 1;
                continue;
            } else {
                i += 1;
                continue;
            }
        }

        if skip_depth < 0 {
            out.push(ch);
        }
        i += 1;
    }

    Some(split_lines(&out))
}

// --------------------------------------------------------------------
// PDF (best-effort, no OCR, no ToUnicode CMap resolution - see docs)
// --------------------------------------------------------------------

fn stream_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)(.{0,400}?)stream\r?\n(.*?)endstream").unwrap())
}

fn text_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\((?:\\.|[^()])*\)").unwrap())
}

fn skip_marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"/Image|/FontFile|/ICCBased|/Metadata").unwrap())
}

fn tj_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bTj\b|\bTJ\b").unwrap())
}

fn pdf_escape_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\\([\\()nrtbf]|[0-7]{1,3})").unwrap())
}

/// Lightweight, dependency-free, BEST-EFFORT PDF text extractor. Finds
/// stream...endstream blocks, decodes `/ASCII85Decode` and/or
/// `/FlateDecode` filtered streams, and pulls text out of Tj/TJ show-text
/// operators.
///
/// PERFORMANCE / HANG SAFEGUARDS: skips streams whose own dictionary marks
/// them as images, embedded font programs, ICC profiles, or metadata
/// without decompressing them; caps how much of any one stream gets
/// scanned; stops after `overall_timeout_seconds` total and keeps whatever
/// text was already found, flagging the result as truncated (returned as
/// the second tuple element). Calls `on_progress` periodically so the
/// caller can surface "still working, N streams scanned, Ys elapsed"
/// rather than the UI appearing frozen on a large/complex PDF.
///
/// LIMITATIONS: no OCR; does not resolve ToUnicode CMaps, so PDFs with
/// embedded/subsetted fonts (common from LaTeX/pdflatex) may extract as
/// garbled or missing text; filters other than ASCII85Decode/FlateDecode
/// are not handled.
pub fn extract_pdf_lines(
    bytes: &[u8],
    overall_timeout_seconds: u64,
    mut on_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>,
) -> (Option<Vec<String>>, bool) {
    let raw = decode_latin1(bytes);

    let mut lines: Vec<String> = Vec::new();
    let start = Instant::now();
    let mut last_progress_report = Duration::ZERO;
    let mut streams_scanned: i32 = 0;
    const MAX_CONTENT_CHARS: usize = 2_000_000;
    let mut truncated_by_time = false;

    for caps in stream_re().captures_iter(&raw) {
        let elapsed = start.elapsed();
        if elapsed.as_secs_f64() >= overall_timeout_seconds as f64 {
            truncated_by_time = true;
            break;
        }

        streams_scanned += 1;
        if let Some(cb) = on_progress.as_deref_mut() {
            if (elapsed - last_progress_report).as_millis() >= 150 {
                cb(streams_scanned, elapsed);
                last_progress_report = elapsed;
            }
        }

        let header = &caps[1];
        if skip_marker_re().is_match(header) {
            continue;
        }

        let stream_text = &caps[2];
        if stream_text.is_empty() {
            continue;
        }

        let has_ascii85 = header.contains("/ASCII85Decode");
        let has_flate = header.contains("/FlateDecode");

        let working_bytes: Vec<u8> = if has_ascii85 {
            decode_ascii85(stream_text)
        } else {
            encode_latin1(stream_text)
        };

        let content_bytes: Option<Vec<u8>> = if !working_bytes.is_empty() {
            if has_flate {
                if working_bytes.len() > 2 {
                    inflate_raw_deflate(&working_bytes[2..])
                } else {
                    None
                }
            } else {
                Some(working_bytes)
            }
        } else {
            None
        };

        if let Some(cb_bytes) = content_bytes {
            if !cb_bytes.is_empty() {
                let content_len = cb_bytes.len().min(MAX_CONTENT_CHARS);
                let content = decode_latin1(&cb_bytes[..content_len]);

                if tj_re().is_match(&content) {
                    for tm in text_re().find_iter(&content) {
                        let s = tm.as_str();
                        let raw_inner = &s[1..s.len() - 1];
                        let inner = unescape_pdf_string(raw_inner);
                        if !inner.trim().is_empty() {
                            lines.push(inner);
                        }
                    }
                }
            }
        }
    }

    if let Some(cb) = on_progress.as_deref_mut() {
        cb(streams_scanned, start.elapsed());
    }

    if truncated_by_time && !lines.is_empty() {
        lines.push(format!(
            "[... PDF text extraction stopped early after {} seconds on this large/complex file - some text may be missing ...]",
            overall_timeout_seconds
        ));
    }

    let result = if lines.is_empty() { None } else { Some(lines) };
    (result, truncated_by_time)
}

fn inflate_raw_deflate(data: &[u8]) -> Option<Vec<u8>> {
    use flate2::read::DeflateDecoder;
    let mut decoder = DeflateDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).ok()?;
    Some(out)
}

fn unescape_pdf_string(inner: &str) -> String {
    pdf_escape_re()
        .replace_all(inner, |caps: &regex::Captures| {
            let g = &caps[1];
            let is_octal = !g.is_empty() && g.chars().all(|c| ('0'..='7').contains(&c));
            if is_octal {
                match u32::from_str_radix(g, 8).ok().and_then(char::from_u32) {
                    Some(ch) => ch.to_string(),
                    None => String::new(),
                }
            } else {
                match g {
                    "n" => "\n".to_string(),
                    "r" => "\r".to_string(),
                    "t" => "\t".to_string(),
                    "b" => "\u{8}".to_string(),
                    "f" => "\u{c}".to_string(),
                    "(" => "(".to_string(),
                    ")" => ")".to_string(),
                    "\\" => "\\".to_string(),
                    _ => g.to_string(),
                }
            }
        })
        .into_owned()
}

/// Cheap heuristic to flag PDFs whose extracted text is probably garbled -
/// typically PDFs with embedded/subsetted fonts using custom glyph
/// encodings (common from LaTeX/pdflatex) that this extractor can't decode
/// correctly. Not a guarantee either way - a hint to double-check manually.
pub fn pdf_extraction_looks_reliable(lines: &[String]) -> bool {
    if lines.is_empty() {
        return false;
    }

    let sample_text = lines.iter().take(200).cloned().collect::<Vec<_>>().join(" ");
    if sample_text.is_empty() {
        return false;
    }

    let mut letters = 0usize;
    let mut spaces = 0usize;
    let mut printable = 0usize;
    let total = sample_text.chars().count();

    for ch in sample_text.chars() {
        if ch.is_alphabetic() {
            letters += 1;
        }
        if ch == ' ' {
            spaces += 1;
        }
        if !ch.is_control() {
            printable += 1;
        }
    }

    let letter_ratio = letters as f64 / total as f64;
    let space_ratio = spaces as f64 / total as f64;
    let printable_ratio = printable as f64 / total as f64;

    letter_ratio > 0.35 && space_ratio > 0.08 && printable_ratio > 0.9
}

// --------------------------------------------------------------------
// ASCII85 (PDF/Adobe variant)
// --------------------------------------------------------------------

/// Decodes PDF-style ASCII85 (Adobe variant: lowercase 'z' shorthand for
/// four zero bytes - deliberately a case-sensitive char comparison, since
/// an uppercase 'Z' is a perfectly ordinary data character, not the
/// shorthand). Optional `~>` end marker; whitespace is ignored.
pub fn decode_ascii85(text: &str) -> Vec<u8> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let t = cleaned.strip_suffix("~>").unwrap_or(&cleaned);

    let mut out_bytes = Vec::new();
    let mut group = [0u32; 5];
    let mut count = 0usize;

    for ch in t.chars() {
        if ch == 'z' && count == 0 {
            out_bytes.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }

        let val = ch as i32 - 33;
        if !(0..=84).contains(&val) {
            continue;
        }

        group[count] = val as u32;
        count += 1;
        if count == 5 {
            let mut num: u64 = 0;
            for &g in &group {
                num = num * 85 + g as u64;
            }
            out_bytes.push(((num >> 24) & 0xFF) as u8);
            out_bytes.push(((num >> 16) & 0xFF) as u8);
            out_bytes.push(((num >> 8) & 0xFF) as u8);
            out_bytes.push((num & 0xFF) as u8);
            count = 0;
        }
    }

    if count > 0 {
        let pad_count = 5 - count;
        for p in 0..pad_count {
            group[count + p] = 84;
        }
        let mut num: u64 = 0;
        for &g in &group {
            num = num * 85 + g as u64;
        }
        let tmp = [
            ((num >> 24) & 0xFF) as u8,
            ((num >> 16) & 0xFF) as u8,
            ((num >> 8) & 0xFF) as u8,
            (num & 0xFF) as u8,
        ];
        for item in tmp.iter().take(count - 1) {
            out_bytes.push(*item);
        }
    }

    out_bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_binary_detects_nul_byte() {
        assert!(looks_binary(b"hello\0world"));
        assert!(!looks_binary(b"hello world"));
        assert!(!looks_binary(b""));
    }

    #[test]
    fn decode_text_handles_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("hello".as_bytes());
        assert_eq!(decode_text(&bytes), "hello");
    }

    #[test]
    fn decode_text_falls_back_to_windows_1252() {
        // 0x93/0x94 are smart quotes in Windows-1252, invalid as UTF-8 continuation bytes here.
        let bytes = vec![0x93, b'h', b'i', 0x94];
        let decoded = decode_text(&bytes);
        assert!(decoded.contains('h') && decoded.contains('i'));
    }

    #[test]
    fn decode_text_preserves_valid_utf8_around_a_single_stray_byte() {
        // A predominantly-UTF-8 file (real accented/multi-byte characters,
        // like a genuine `apple café “quote”` line) with exactly one
        // corrupted byte spliced in (0xFF is never valid in UTF-8, in any
        // position) - simulating one bad byte from mid-file corruption or a
        // paste from a different source. Whole-file Windows-1252 fallback
        // would mangle every one of the correct multi-byte characters into
        // mojibake ("cafÃ©", "â€œquoteâ€"); the fix should instead keep
        // them intact and only replace the single bad byte.
        let mut bytes = "apple caf\u{e9} \u{201c}quote\u{201d} banana".as_bytes().to_vec();
        let stray_pos = bytes.len() / 2;
        bytes.insert(stray_pos, 0xFF);

        let decoded = decode_text(&bytes);
        assert!(decoded.contains("café"), "decoded={decoded:?}");
        assert!(decoded.contains("apple") && decoded.contains("banana"), "decoded={decoded:?}");
        assert!(decoded.contains('\u{FFFD}'), "expected a replacement char for the stray byte: {decoded:?}");
    }

    #[test]
    fn utf8_invalid_byte_ratio_matches_expectations() {
        assert_eq!(utf8_invalid_byte_ratio(b"hello world"), 0.0);
        assert_eq!(utf8_invalid_byte_ratio(b""), 0.0);
        // Every byte invalid.
        assert_eq!(utf8_invalid_byte_ratio(&[0x93, 0x94, 0xFF, 0xFE]), 1.0);
    }

    #[test]
    fn split_lines_handles_all_three_line_ending_styles() {
        assert_eq!(split_lines("a\r\nb\nc\rd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_lines(""), Vec::<String>::new());
    }

    #[test]
    fn decode_ascii85_round_trips_known_vector() {
        // "Man " encodes to "9jqo^" in Adobe ASCII85 (classic test vector).
        let decoded = decode_ascii85("9jqo^");
        assert_eq!(decoded, b"Man ");
    }

    #[test]
    fn decode_ascii85_z_shorthand_is_case_sensitive() {
        assert_eq!(decode_ascii85("z"), vec![0, 0, 0, 0]);
        // Uppercase 'Z' is ordinary data (33 + ('Z'-33)), not the shorthand -
        // it must NOT decode to four zero bytes.
        let z_upper = decode_ascii85("Z");
        assert_ne!(z_upper, vec![0, 0, 0, 0]);
    }

    #[test]
    fn extract_docx_lines_returns_none_for_non_zip_bytes() {
        assert!(extract_docx_lines(b"not a zip file").is_none());
    }

    #[test]
    fn extract_pptx_lines_returns_none_for_non_zip_bytes() {
        assert!(extract_pptx_lines(b"not a zip file").is_none());
    }

    #[test]
    fn extract_xlsx_lines_returns_none_for_non_zip_bytes() {
        assert!(extract_xlsx_lines(b"not a zip file").is_none());
    }

    #[test]
    fn extract_zip_archive_lines_returns_none_for_non_zip_bytes() {
        assert!(extract_zip_archive_lines(b"not a zip file", 2).is_none());
    }

    #[test]
    fn extract_rtf_lines_returns_none_for_non_rtf_text() {
        assert!(extract_rtf_lines(b"just plain text").is_none());
    }

    #[test]
    fn extract_rtf_lines_extracts_visible_text_and_skips_font_table() {
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}Hello\par World}";
        let lines = extract_rtf_lines(rtf).unwrap();
        let joined = lines.join("\n");
        assert!(joined.contains("Hello"));
        assert!(joined.contains("World"));
        assert!(!joined.contains("Arial"));
    }

    #[test]
    fn extract_rtf_lines_decodes_hex_escape() {
        // \'e9 is Latin-1 0xE9 ('é' under the default 1252 codepage's ANSI range).
        let rtf = b"{\\rtf1 caf\\'e9}";
        let lines = extract_rtf_lines(rtf).unwrap();
        assert!(lines.join("\n").contains('\u{e9}'));
    }

    #[test]
    fn pdf_extraction_looks_reliable_flags_garbled_text() {
        let garbled = vec!["\u{1}\u{2}\u{3}\u{4}".to_string()];
        assert!(!pdf_extraction_looks_reliable(&garbled));

        let reliable = vec!["This is normal readable English text with spaces.".to_string()];
        assert!(pdf_extraction_looks_reliable(&reliable));

        let empty: Vec<String> = Vec::new();
        assert!(!pdf_extraction_looks_reliable(&empty));
    }

    #[test]
    fn unescape_pdf_string_handles_known_escapes() {
        assert_eq!(unescape_pdf_string(r"a\nb"), "a\nb");
        assert_eq!(unescape_pdf_string(r"a\(b\)c"), "a(b)c");
        assert_eq!(unescape_pdf_string(r"\101"), "A"); // octal 101 = 'A'
        assert_eq!(unescape_pdf_string(r"a\qb"), r"a\qb"); // unrecognized escape left alone
    }

    #[test]
    fn path_extension_lower_matches_expected_cases() {
        assert_eq!(path_extension_lower("dir/file.DOCX"), ".docx");
        assert_eq!(path_extension_lower("dir/noext"), "");
        assert_eq!(path_extension_lower("a/b/c.tar.gz"), ".gz");
    }
}
