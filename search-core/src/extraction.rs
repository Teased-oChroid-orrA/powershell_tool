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

/// One format's extraction logic (epic #6 §5's "extractor abstraction" -
/// registering a new `Extractor` impl in [`EXTRACTORS`] is now how a
/// format gets added, not editing a match arm buried in a dispatch
/// function). Deliberately thin: this trait owns *dispatch* only, not
/// parsing - each impl below still calls the same hand-rolled, byte-for-
/// byte-tested `extract_*_lines` free functions this module always has
/// (see `CLAUDE.md`'s extraction design notes for why those stay
/// hand-rolled rather than adopting a generic OOXML/PDF parser crate;
/// this trait doesn't change that reasoning, it only changes how the
/// *dispatch table* is expressed).
///
/// `pdf_timeout_seconds`/`on_pdf_progress` are part of the shared
/// signature (not a PDF-only extra parameter) so every impl has the same
/// shape - only `PdfExtractor` actually reads them, everyone else ignores
/// them, matching this app's standing "PDF extraction must never go
/// silent" progress-reporting requirement without needing a second,
/// PDF-specific trait method.
pub trait Extractor: Send + Sync {
    /// Lowercase, dot-prefixed extensions this extractor handles (e.g.
    /// `".docx"`). Checked via `extensions().contains(&ext)` by
    /// [`extract_lines_by_extension`] - an extractor matching more than
    /// one extension (there are none today) is exactly as valid as one
    /// matching a single extension.
    fn extensions(&self) -> &'static [&'static str];

    fn extract(
        &self,
        bytes: &[u8],
        pdf_timeout_seconds: u64,
        on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>,
    ) -> Option<Vec<String>>;

    /// Only `PdfExtractor` overrides this - see
    /// `pdf_extraction_looks_reliable`'s doc comment for what it detects.
    fn low_confidence(&self, _lines: &[String]) -> bool {
        false
    }
}

struct DocxExtractor;
impl Extractor for DocxExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[".docx"]
    }
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>) -> Option<Vec<String>> {
        extract_docx_lines(bytes)
    }
}

struct PptxExtractor;
impl Extractor for PptxExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[".pptx"]
    }
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>) -> Option<Vec<String>> {
        extract_pptx_lines(bytes)
    }
}

struct XlsxExtractor;
impl Extractor for XlsxExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[".xlsx"]
    }
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>) -> Option<Vec<String>> {
        extract_xlsx_lines(bytes)
    }
}

struct ZipExtractor;
impl Extractor for ZipExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[".zip"]
    }
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>) -> Option<Vec<String>> {
        extract_zip_archive_lines(bytes, 2)
    }
}

struct PdfExtractor;
impl Extractor for PdfExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[".pdf"]
    }
    fn extract(
        &self,
        bytes: &[u8],
        pdf_timeout_seconds: u64,
        on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>,
    ) -> Option<Vec<String>> {
        extract_pdf_lines(bytes, pdf_timeout_seconds, on_pdf_progress).0
    }
    fn low_confidence(&self, lines: &[String]) -> bool {
        !pdf_extraction_looks_reliable(lines)
    }
}

struct RtfExtractor;
impl Extractor for RtfExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[".rtf"]
    }
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>) -> Option<Vec<String>> {
        extract_rtf_lines(bytes)
    }
}

/// The fallback for every extension not otherwise registered - not
/// reached via `extensions()` matching at all (see
/// [`extract_lines_by_extension`]'s loop), since "everything else" isn't
/// a finite extension list.
struct PlainTextExtractor;
impl Extractor for PlainTextExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[]
    }
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>) -> Option<Vec<String>> {
        Some(split_lines(&decode_text(bytes)))
    }
}

/// The registry - add a new format by adding an `Extractor` impl above
/// and one entry here, not by editing
/// [`extract_lines_by_extension`]'s dispatch logic itself.
static EXTRACTORS: &[&dyn Extractor] =
    &[&DocxExtractor, &PptxExtractor, &XlsxExtractor, &ZipExtractor, &PdfExtractor, &RtfExtractor];

/// Extension-to-extractor dispatch, factored out of `orchestrator.rs`'s
/// `process_one_file` so the same table backs both the normal search path
/// and the proactive corpus indexer (`native_index.rs`) - one place that
/// knows "which extractor for which extension," not two that could drift
/// apart as formats are added.
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
    let registered = EXTRACTORS.iter().find(|e| e.extensions().contains(&ext)).copied();

    // Binary sniffing only applies to the plain-text fallback path - every
    // registered format extractor above parses its own container format
    // regardless of NUL bytes appearing incidentally in binary structure
    // (a real DOCX/PDF/etc. always has them).
    let extractor: &dyn Extractor = match registered {
        Some(e) => e,
        None => {
            if looks_binary(bytes) {
                return Err(ExtractLinesError::Binary);
            }
            &PlainTextExtractor
        }
    };

    let lines = extractor.extract(bytes, pdf_timeout_seconds, on_pdf_progress);
    match lines {
        Some(l) if !l.is_empty() => {
            let low_confidence_pdf = extractor.low_confidence(&l);
            Ok(ExtractedLines { lines: l, low_confidence_pdf })
        }
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

/// Issue #6 §62 "Security/Safety" - "enforce extracted size limits" for
/// archive extraction. This is the single choke point every DOCX/PPTX/
/// XLSX entry read goes through (word/document.xml, each slide/notes/
/// diagram entry, sheet XML, shared strings) - unlike the nested-zip path
/// in `extract_zip_entries`, these formats were reading each entry with a
/// plain unbounded `read_to_string`, so a malicious file with one of
/// these extensions whose one XML entry has a large *declared*
/// uncompressed size (a classic zip-bomb: a small compressed file, a huge
/// declared/actual decompressed size) could exhaust memory before the
/// orchestrator's `max_file_size_mb` gate ever gets a chance to matter -
/// that gate only sees the small on-disk *compressed* file size. Checks
/// the declared size up front (reject before decompressing at all, same
/// as `extract_zip_entries`'s own per-entry check), then bounds the
/// actual read with `Read::take` as defense-in-depth against a
/// deliberately-wrong declared size, not just a trusted-header check.
fn read_zip_entry_to_string(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let entry = zip.by_name(name).ok()?;
    if entry.size() as i64 > ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES {
        return None;
    }
    let mut s = String::new();
    entry.take(ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES as u64).read_to_string(&mut s).ok()?;
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
            let entry = match zip.by_index(i) {
                Ok(e) => e,
                Err(_) => continue,
            };
            // Bounded read, not just the declared-size precheck above -
            // defense-in-depth against a deliberately-wrong declared size
            // (see read_zip_entry_to_string's doc comment for the same
            // reasoning applied to DOCX/PPTX/XLSX entries).
            if entry.take(ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES as u64).read_to_end(&mut entry_bytes).is_err() {
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

/// Kept **only** as the differential-test oracle for
/// [`find_stream_blocks`] (see `extraction::tests::find_stream_blocks_matches_the_original_regex_*`) -
/// no longer used by `extract_pdf_lines` itself. Profiling real PDFs
/// (`docs/benchmarking.md`'s "PDF extraction bottleneck" section) found
/// this one regex responsible for 74-93% of total PDF extraction time,
/// worsening with file size (272KB: 75%, 1.04MB: 74%, 38.6MB: 93%) -
/// `.{0,400}?` bounded repetition compiles to a much larger automaton
/// than a plain literal search in Rust's `regex` crate (it has to track
/// a counter up to 400 through the NFA rather than a simple star/plus),
/// which is almost certainly the real cost driver, not algorithmic
/// complexity (the per-KB rate was roughly constant to slightly
/// *decreasing* with size, not blowing up quadratically). Replaced with
/// `find_stream_blocks`, a manual `str::find`-based scanner proven
/// byte-for-byte equivalent on every real PDF fixture in this repo plus
/// synthetic adversarial cases before the swap was made.
#[cfg(test)]
fn stream_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)(.{0,400}?)stream\r?\n(.*?)endstream").unwrap())
}

/// Manual replacement for the old `stream_re()` regex - see that
/// function's doc comment for why. Finds every non-overlapping
/// `stream\r?\n ... endstream` block in `raw`, yielding
/// `(header, body)` where `header` is up to 400 *bytes* immediately
/// preceding the `stream` keyword (nudged forward to the nearest valid
/// UTF-8 char boundary if a multi-byte Latin-1-mapped character falls
/// exactly on the cut - PDF stream dictionaries are effectively always
/// ASCII in practice, so this only matters for pathological input, but
/// slicing on a non-boundary would panic) and `body` is everything
/// between the newline after `stream` and the next `endstream`.
///
/// Iteration semantics are a deliberate, verified match for what the old
/// regex's `captures_iter` actually did (non-overlapping, leftmost
/// matches; a `stream` keyword not immediately followed by `\r\n`/`\n` is
/// skipped and scanning resumes right after it, matching substrings like
/// "bit**stream**\n" included - the old regex had no word-boundary
/// assertion around the literal either, so this preserves that exact,
/// slightly odd but pre-existing behavior rather than "fixing" it as an
/// incidental side effect of a performance change).
fn find_stream_blocks(raw: &str) -> impl Iterator<Item = (&str, &str)> {
    // Two positions, deliberately not one - this distinction is exactly
    // the bug the differential tests below caught in an earlier draft.
    // `anchor` is the *lower bound* the header capture can't reach behind
    // (only moves forward once a full match completes, matching
    // `captures_iter`'s non-overlapping-match semantics). `probe` is
    // where to resume looking for the next literal "stream" text after a
    // candidate turns out not to be followed by a newline - it must NOT
    // also drag `anchor` forward, because the real regex's lazily-
    // quantified header can absorb a failed "stream" candidate as
    // ordinary header text and keep extending past it in the *same*
    // overall match attempt (e.g. "bitstream ... stream\n": the "stream"
    // inside "bitstream" fails the newline check, but the header for the
    // real match still starts from the original anchor, not from just
    // after the failed candidate).
    let mut anchor = 0usize;
    let mut probe = 0usize;
    std::iter::from_fn(move || loop {
        let rel = raw.get(probe..)?.find("stream")?;
        let stream_abs = probe + rel;
        let after_stream = stream_abs + "stream".len();

        let body_start = if raw.as_bytes().get(after_stream..after_stream + 2) == Some(b"\r\n") {
            after_stream + 2
        } else if raw.as_bytes().get(after_stream).copied() == Some(b'\n') {
            after_stream + 1
        } else {
            probe = after_stream;
            continue;
        };

        // Exactly 400 *characters* back, not bytes - matching the old
        // regex's `.{0,400}?` (Rust's `regex` crate matches `.` against
        // one Unicode scalar value by default). `decode_latin1` maps
        // every byte 0-255 to the same-valued codepoint, so bytes
        // 0x80-0xFF become 2-byte UTF-8 sequences - a byte-count cutoff
        // would disagree with the regex (and risk landing mid-character)
        // the moment any such byte appears in the 400 bytes before a
        // stream keyword. `char_indices().rev().nth(399)` only walks
        // back up to 400 characters regardless of file size, so this
        // stays cheap even on huge files.
        let header_start = raw[..stream_abs].char_indices().rev().nth(399).map(|(idx, _)| idx).unwrap_or(0).max(anchor);
        let header = &raw[header_start..stream_abs];

        return match raw.get(body_start..)?.find("endstream") {
            Some(end_rel) => {
                let end_abs = body_start + end_rel;
                let body = &raw[body_start..end_abs];
                anchor = end_abs + "endstream".len();
                probe = anchor;
                Some((header, body))
            }
            None => None,
        };
    })
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

    for (header, stream_text) in find_stream_blocks(&raw) {
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

        if skip_marker_re().is_match(header) {
            continue;
        }

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

/// Issue #6 §62 "Security/Safety" - a PDF content stream declares itself
/// `/FlateDecode`d but there's no header field bounding how large the
/// *inflated* output is before decompression actually runs (unlike a zip
/// entry's declared `uncompressed_size`) - a small on-disk PDF containing
/// a deliberately extreme-ratio deflate stream ("deflate bomb") could
/// otherwise make `read_to_end` allocate without bound, before
/// `extract_pdf_lines`'s own `MAX_CONTENT_CHARS` truncation - which only
/// trims the buffer *after* it's already fully inflated - ever gets a
/// chance to matter.
const PDF_MAX_INFLATED_STREAM_BYTES: u64 = 20_000_000;

fn inflate_raw_deflate(data: &[u8]) -> Option<Vec<u8>> {
    use flate2::read::DeflateDecoder;
    let decoder = DeflateDecoder::new(data);
    let mut out = Vec::new();
    decoder.take(PDF_MAX_INFLATED_STREAM_BYTES).read_to_end(&mut out).ok()?;
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

    /// Issue #6 §62 "Security/Safety" - "enforce extracted size limits".
    /// Builds a real (not fabricated) zip whose `word/document.xml` entry
    /// declares more than `ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES` of
    /// uncompressed content (highly compressible repeated bytes, so the
    /// on-disk/compressed size stays tiny and the test runs instantly) -
    /// exactly the shape a small-file, huge-decompressed-content zip bomb
    /// would take. `extract_docx_lines` must reject it via the declared-
    /// size precheck in `read_zip_entry_to_string`, never attempt to
    /// materialize the full decompressed content in memory.
    #[test]
    fn extract_docx_lines_rejects_an_entry_declaring_more_than_the_size_cap() {
        use std::io::Write;
        let oversized_len = (ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES + 1) as usize;
        let mut buf = Vec::new();
        {
            let cursor = Cursor::new(&mut buf);
            let mut writer = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("word/document.xml", options).unwrap();
            writer.write_all(&vec![b'a'; oversized_len]).unwrap();
            writer.finish().unwrap();
        }
        assert!(extract_docx_lines(&buf).is_none(), "an entry over the declared-size cap must be rejected, not decompressed");
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

    // ---- find_stream_blocks vs. the original regex (issue #8 follow-up,
    // 2026-08-26) - the differential proof the swap in extract_pdf_lines
    // is safe. Profiling found the old `stream_re()` regex responsible
    // for 74-93% of total PDF extraction time; `find_stream_blocks` is
    // its manual-scan replacement. Every case below asserts BYTE-FOR-BYTE
    // identical (header, body) pairs, in the same order, from both
    // implementations - not "close enough."

    fn old_regex_blocks(raw: &str) -> Vec<(&str, &str)> {
        stream_re().captures_iter(raw).map(|c| (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str())).collect()
    }

    fn assert_same_blocks(raw: &str) {
        let old: Vec<(&str, &str)> = old_regex_blocks(raw);
        let new: Vec<(&str, &str)> = find_stream_blocks(raw).collect();
        assert_eq!(old, new, "find_stream_blocks must match the original regex exactly for: {raw:?}");
    }

    #[test]
    fn find_stream_blocks_matches_the_original_regex_on_simple_cases() {
        assert_same_blocks("BT stream\nhello world endstream ET");
        assert_same_blocks("no stream markers here at all");
        assert_same_blocks("");
        assert_same_blocks("stream\n\nendstream"); // empty body
        assert_same_blocks("/Filter/FlateDecode stream\r\nBODY1endstream junk /Filter2 stream\nBODY2endstream");
    }

    #[test]
    fn find_stream_blocks_matches_the_original_regex_on_adversarial_cases() {
        // "stream" as a substring of another word, not followed by a
        // newline at all - the old regex has no word-boundary assertion,
        // so "bitstream" partially matches the literal "stream" but fails
        // the \r?\n requirement immediately after; scanning must resume
        // and still find the real block later.
        assert_same_blocks("bitstream text here, then /Filter stream\nREAL BODYendstream");
        // "stream" appearing but never followed by endstream at all - no
        // match should be produced (old regex finds nothing either).
        assert_same_blocks("garbage /Filter stream\nbody never closes");
        // Multiple candidate "stream" occurrences before a valid one -
        // the first "stream" isn't followed by a newline, scanning must
        // skip it and find the second.
        assert_same_blocks("stream stream\nactual body endstream");
        // \r\n vs bare \n line endings after the "stream" keyword.
        assert_same_blocks("hdr stream\r\nCRLF bodyendstream hdr2 stream\nLF bodyendstream");
        // Header longer than 400 chars before "stream" - must be capped
        // to (at most) 400 in both implementations.
        let long_header = "x".repeat(500);
        assert_same_blocks(&format!("{long_header} stream\nbody endstream"));
        // Back-to-back blocks with no separating text at all.
        assert_same_blocks("stream\nAendstreamstream\nBendstream");
        // A multi-byte (non-ASCII, Latin-1-mapped) character sitting near
        // the 400-byte header cutoff - proves the char-boundary nudge in
        // find_stream_blocks doesn't panic and still agrees with the
        // regex (which counts *characters*, not bytes, for its {0,400}
        // bound - decode_latin1 maps every byte 0-255 to the same-valued
        // Unicode scalar, so bytes 0x80-0xFF become 2-byte UTF-8
        // sequences; Latin-1 byte 0xE9 was chosen arbitrarily as one such
        // character).
        let padding_with_high_byte: String = std::iter::repeat('a').take(398).chain(std::iter::once('\u{e9}')).collect();
        assert_same_blocks(&format!("{padding_with_high_byte} stream\nbody endstream"));
    }

    #[test]
    fn find_stream_blocks_matches_the_original_regex_on_real_pdf_fixtures() {
        let fixtures_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/TextInFilesSearch.Tests/Fixtures");
        let data_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/data");
        for path in [
            format!("{fixtures_dir}/test.pdf"),
            format!("{data_dir}/medium.pdf"),
            format!("{data_dir}/large.pdf"),
        ] {
            let Ok(bytes) = std::fs::read(&path) else {
                continue; // benches/data/ files aren't always present (e.g. a fresh checkout before fetching them) - skip, don't fail
            };
            let raw = decode_latin1(&bytes);
            let old = old_regex_blocks(&raw);
            let new: Vec<(&str, &str)> = find_stream_blocks(&raw).collect();
            assert_eq!(old.len(), new.len(), "block count must match for {path}");
            assert_eq!(old, new, "blocks must be byte-for-byte identical for {path}");
        }
    }

    /// Same proof as `find_stream_blocks_matches_the_original_regex_on_real_pdf_fixtures`,
    /// for `xlarge-scanned.pdf` (38.6MB, the original scanned/image-only
    /// PDF this test was written and verified against - renamed from
    /// `xlarge.pdf` once a genuine text-bearing ~10MB PDF was sourced and
    /// took the `xlarge.pdf` name instead, see `benches/data/README.md`)
    /// specifically - `#[ignore]`d and kept separate because running the
    /// *old* regex (the whole reason for this rewrite) against a file
    /// this size takes seconds even in release mode and far longer in the
    /// default debug test profile - not something every `cargo test` run
    /// should pay for. Run on demand: `cargo test -p search-core
    /// --release -- --ignored
    /// find_stream_blocks_matches_the_original_regex_on_xlarge_pdf`.
    #[test]
    #[ignore]
    fn find_stream_blocks_matches_the_original_regex_on_xlarge_pdf() {
        let data_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/data");
        let path = format!("{data_dir}/xlarge-scanned.pdf");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("could not read {path}: {e}"));
        let raw = decode_latin1(&bytes);
        let old = old_regex_blocks(&raw);
        let new: Vec<(&str, &str)> = find_stream_blocks(&raw).collect();
        assert_eq!(old.len(), new.len(), "block count must match for xlarge-scanned.pdf");
        assert_eq!(old, new, "blocks must be byte-for-byte identical for xlarge-scanned.pdf");
    }

    /// `xlarge-scanned.pdf` is a real, legitimately-sourced ~38.6MB PDF
    /// that is image-only (scanned pages, `/Im1 Do` content streams, zero
    /// `Tj`/`TJ` text-showing operators) - this extractor has no OCR, so
    /// it correctly returns no text for it, not a bug. `xlarge-
    /// recordheavy.xlsx` is Apache Tika's `testRecordSizeExceeded.xlsx`,
    /// whose single worksheet entry decompresses from 12.4MB to ~328MB -
    /// exactly what `ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES` (20MB) exists to
    /// reject, so it correctly returns no text too. Both are kept as
    /// permanent, `#[ignore]`d (large file I/O) regression proof that
    /// these real-world pathological documents degrade gracefully -
    /// extraction returns `None`/no lines - rather than crashing, hanging,
    /// or silently producing wrong output.
    #[test]
    #[ignore]
    fn diagnose_why_xlarge_pdf_and_xlsx_fail_extraction() {
        let data_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/data");
        let pdf_bytes = std::fs::read(format!("{data_dir}/xlarge-scanned.pdf")).unwrap();
        let xlsx_bytes = std::fs::read(format!("{data_dir}/xlarge-recordheavy.xlsx")).unwrap();

        let raw = decode_latin1(&pdf_bytes);
        let blocks: Vec<_> = find_stream_blocks(&raw).collect();
        eprintln!("xlarge-scanned.pdf: {} stream blocks total", blocks.len());
        let mut flate_count = 0;
        let mut flate_rejected_by_size_cap = 0;
        let mut flate_produced_no_text = 0;
        let mut skipped_by_marker = 0;
        for (header, body) in &blocks {
            if skip_marker_re().is_match(header) {
                skipped_by_marker += 1;
                continue;
            }
            if header.contains("/FlateDecode") {
                flate_count += 1;
                let working = encode_latin1(body);
                if working.len() > 2 {
                    match inflate_raw_deflate(&working[2..]) {
                        Some(inflated) => {
                            if inflated.is_empty() {
                                flate_produced_no_text += 1;
                            } else if flate_count <= 3 {
                                let content = decode_latin1(&inflated[..inflated.len().min(500)]);
                                eprintln!("  sample stream #{flate_count} (first 500 chars of inflated content):\n{content}\n---");
                            }
                        }
                        None => flate_rejected_by_size_cap += 1,
                    }
                }
            }
        }
        eprintln!(
            "  skipped_by_marker={skipped_by_marker} flate_streams={flate_count} rejected_by_20MB_cap={flate_rejected_by_size_cap} inflate_empty={flate_produced_no_text}"
        );
        let (pdf_result, _) = extract_pdf_lines(&pdf_bytes, 15, None);
        eprintln!("  extract_pdf_lines final: {:?} lines", pdf_result.as_ref().map(|l| l.len()));
        assert!(
            pdf_result.map(|l| l.is_empty()).unwrap_or(true),
            "xlarge-scanned.pdf is image-only (no Tj/TJ operators) - extraction has no OCR, so this must stay empty/None, not regress into a panic or spurious text"
        );

        eprintln!("\nxlarge-recordheavy.xlsx: {} bytes on disk", xlsx_bytes.len());
        let xlsx_result = extract_xlsx_lines(&xlsx_bytes);
        eprintln!("  extract_xlsx_lines result: {:?}", xlsx_result.as_ref().map(|l| l.len()));
        assert!(
            xlsx_result.is_none(),
            "xlarge-recordheavy.xlsx's worksheet entry decompresses to ~328MB - the ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES guard must keep rejecting it, not regress into decompressing it"
        );
    }

    // ---- PDF extraction profiling (issue #8 follow-up, 2026-08-26) ----
    // `docs/benchmarking.md`'s corrected numbers showed PDF extraction
    // costing 33.6-112ms on real 272KB-1.04MB documents, orders of
    // magnitude above the other formats. Before deciding whether that
    // justifies replacing the regex/content-stream scanner with a real
    // structural parser (a substantial, parity-risking rewrite), find out
    // *where inside the current algorithm* the time actually goes - a
    // cheap, safe, no-dependency-change fix might get most of the benefit.
    // #[ignore]d (needs the real fixture files in benches/data/, and
    // prints a report rather than asserting anything) - matches this
    // project's established pattern for opt-in diagnostic/stress tests
    // (`stress_test_100k_files`).

    #[derive(Default)]
    struct PdfPhaseBreakdown {
        decode_latin1_whole_file: std::time::Duration,
        stream_regex_scan: std::time::Duration,
        ascii85_decode: std::time::Duration,
        inflate: std::time::Duration,
        decode_latin1_per_stream: std::time::Duration,
        tj_probe_regex: std::time::Duration,
        text_extract_regex: std::time::Duration,
        unescape: std::time::Duration,
        streams_total: usize,
        streams_skipped_by_marker: usize,
        streams_with_no_tj: usize,
    }

    /// Exact same algorithm as `extract_pdf_lines`, instrumented with a
    /// `std::time::Instant` around each conceptual phase instead of just
    /// running them - calls the real private helper functions this module
    /// already has (`find_stream_blocks`, `decode_ascii85`,
    /// `inflate_raw_deflate`, etc.), not a reimplemented copy that could
    /// silently drift from the real behavior and give a misleading
    /// profile.
    fn profile_pdf_extraction(bytes: &[u8]) -> PdfPhaseBreakdown {
        let mut b = PdfPhaseBreakdown::default();

        let t = std::time::Instant::now();
        let raw = decode_latin1(bytes);
        b.decode_latin1_whole_file += t.elapsed();

        let t = std::time::Instant::now();
        let blocks: Vec<_> = find_stream_blocks(&raw).collect();
        b.stream_regex_scan += t.elapsed();

        for (header, stream_text) in blocks {
            b.streams_total += 1;
            if skip_marker_re().is_match(header) {
                b.streams_skipped_by_marker += 1;
                continue;
            }
            if stream_text.is_empty() {
                continue;
            }
            let has_ascii85 = header.contains("/ASCII85Decode");
            let has_flate = header.contains("/FlateDecode");

            let t = std::time::Instant::now();
            let working_bytes: Vec<u8> =
                if has_ascii85 { decode_ascii85(stream_text) } else { encode_latin1(stream_text) };
            if has_ascii85 {
                b.ascii85_decode += t.elapsed();
            }

            let t = std::time::Instant::now();
            let content_bytes: Option<Vec<u8>> = if !working_bytes.is_empty() {
                if has_flate {
                    if working_bytes.len() > 2 { inflate_raw_deflate(&working_bytes[2..]) } else { None }
                } else {
                    Some(working_bytes)
                }
            } else {
                None
            };
            if has_flate {
                b.inflate += t.elapsed();
            }

            if let Some(cb_bytes) = content_bytes {
                if !cb_bytes.is_empty() {
                    const MAX_CONTENT_CHARS: usize = 2_000_000;
                    let content_len = cb_bytes.len().min(MAX_CONTENT_CHARS);

                    let t = std::time::Instant::now();
                    let content = decode_latin1(&cb_bytes[..content_len]);
                    b.decode_latin1_per_stream += t.elapsed();

                    let t = std::time::Instant::now();
                    let has_tj = tj_re().is_match(&content);
                    b.tj_probe_regex += t.elapsed();

                    if has_tj {
                        let t = std::time::Instant::now();
                        let matches: Vec<String> = text_re().find_iter(&content).map(|m| m.as_str().to_string()).collect();
                        b.text_extract_regex += t.elapsed();

                        let t = std::time::Instant::now();
                        for s in &matches {
                            let raw_inner = &s[1..s.len() - 1];
                            let _ = unescape_pdf_string(raw_inner);
                        }
                        b.unescape += t.elapsed();
                    } else {
                        b.streams_with_no_tj += 1;
                    }
                }
            }
        }

        b
    }

    fn print_pdf_profile(label: &str, path: &str) {
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("{label}: could not read {path}, skipping");
            return;
        };
        let total_start = std::time::Instant::now();
        let b = profile_pdf_extraction(&bytes);
        let total = total_start.elapsed();

        eprintln!("\n=== {label} ({} bytes) - total {:.2}ms ===", bytes.len(), total.as_secs_f64() * 1000.0);
        eprintln!(
            "  streams: {} total, {} skipped (image/font/ICC/metadata marker), {} decoded-but-no-Tj/TJ",
            b.streams_total, b.streams_skipped_by_marker, b.streams_with_no_tj
        );
        let phases: &[(&str, std::time::Duration)] = &[
            ("decode_latin1 (whole file, once)", b.decode_latin1_whole_file),
            ("find_stream_blocks scan (find all stream..endstream)", b.stream_regex_scan),
            ("ascii85 decode (per ASCII85Decode stream)", b.ascii85_decode),
            ("inflate (per FlateDecode stream)", b.inflate),
            ("decode_latin1 (per decoded stream)", b.decode_latin1_per_stream),
            ("Tj/TJ probe regex (per stream)", b.tj_probe_regex),
            ("text_re find_iter (per stream with Tj/TJ)", b.text_extract_regex),
            ("unescape_pdf_string (per matched text run)", b.unescape),
        ];
        for (name, dur) in phases {
            let pct = 100.0 * dur.as_secs_f64() / total.as_secs_f64();
            eprintln!("  {:<45} {:>8.2}ms  ({:>5.1}%)", name, dur.as_secs_f64() * 1000.0, pct);
        }
    }

    #[test]
    #[ignore]
    fn profile_pdf_extraction_phases_on_real_documents() {
        let data_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/data");
        let fixtures_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/TextInFilesSearch.Tests/Fixtures");
        print_pdf_profile("tiny.pdf", &format!("{fixtures_dir}/test.pdf"));
        print_pdf_profile("medium.pdf", &format!("{data_dir}/medium.pdf"));
        print_pdf_profile("large.pdf", &format!("{data_dir}/large.pdf"));
        print_pdf_profile("xlarge.pdf (real text, arXiv paper)", &format!("{data_dir}/xlarge.pdf"));
        print_pdf_profile("xlarge-scanned.pdf (image-only, no text)", &format!("{data_dir}/xlarge-scanned.pdf"));
    }
}
