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
/// `pdf_timeout_seconds`/`on_pdf_progress`/`ocr_scanned_pdfs` are part of
/// the shared signature (not PDF-only extra parameters) so every impl has
/// the same shape - only `PdfExtractor` actually reads them, everyone
/// else ignores them, matching this app's standing "PDF extraction must
/// never go silent" progress-reporting requirement without needing a
/// second, PDF-specific trait method.
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
        ocr_scanned_pdfs: bool,
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
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>, _ocr_scanned_pdfs: bool) -> Option<Vec<String>> {
        extract_docx_lines(bytes)
    }
}

struct PptxExtractor;
impl Extractor for PptxExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[".pptx"]
    }
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>, _ocr_scanned_pdfs: bool) -> Option<Vec<String>> {
        extract_pptx_lines(bytes)
    }
}

struct XlsxExtractor;
impl Extractor for XlsxExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[".xlsx"]
    }
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>, _ocr_scanned_pdfs: bool) -> Option<Vec<String>> {
        extract_xlsx_lines(bytes)
    }
}

struct ZipExtractor;
impl Extractor for ZipExtractor {
    fn extensions(&self) -> &'static [&'static str] {
        &[".zip"]
    }
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>, _ocr_scanned_pdfs: bool) -> Option<Vec<String>> {
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
        ocr_scanned_pdfs: bool,
    ) -> Option<Vec<String>> {
        extract_pdf_lines(bytes, pdf_timeout_seconds, on_pdf_progress, ocr_scanned_pdfs).0
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
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>, _ocr_scanned_pdfs: bool) -> Option<Vec<String>> {
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
    fn extract(&self, bytes: &[u8], _pdf_timeout_seconds: u64, _on_pdf_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>, _ocr_scanned_pdfs: bool) -> Option<Vec<String>> {
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
    ocr_scanned_pdfs: bool,
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

    let lines = extractor.extract(bytes, pdf_timeout_seconds, on_pdf_progress, ocr_scanned_pdfs);
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
                // OCR deliberately not offered for a PDF nested inside a
                // zip (or a zip nested inside a zip) - this recursive
                // scanner has no `SearchSettings` to read an opt-in flag
                // from, and OCR-ing every image-only PDF that happens to
                // be archived somewhere would be an unbounded, silent
                // cost with no way for a user to have opted in. Only a
                // standalone top-level `.pdf` file (via `PdfExtractor`)
                // is eligible.
                let (pdf_lines, _truncated) = extract_pdf_lines(&entry_bytes, 5, None, false);
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

/// A PDF hex string operand to `Tj`/`TJ` (e.g. `<0176>` in `<0176> Tj`) -
/// the encoding a CID-keyed/Type0 font (nearly universal for embedded/
/// subsetted fonts from modern PDF generators - web renderers, Chromium
/// print-to-PDF, Stripe's invoice renderer, LaTeX/pdflatex, etc.) uses
/// instead of a parenthesized literal string. Requires whole byte pairs
/// (real PDF hex strings may contain whitespace and an odd trailing
/// nibble per spec; this only recognizes the common, unambiguous case,
/// falling back to "no match" - never guessing - for anything looser).
/// Deliberately cannot match a dictionary's `<<...>>` delimiters: the
/// second character of `<<` is `<`, never a hex digit, so the pattern
/// fails to start there.
fn hex_string_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<((?:[0-9A-Fa-f]{2})+)>").unwrap())
}

fn bfchar_block_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)beginbfchar(.*?)endbfchar").unwrap())
}

fn bfrange_block_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)beginbfrange(.*?)endbfrange").unwrap())
}

fn bfchar_entry_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>").unwrap())
}

/// Only the common `<start> <end> <dstStart>` sequential form of a
/// `bfrange` entry (source CIDs `start..=end` map to `dstStart..`,
/// incrementing) - the alternative `<start> <end> [<dst1> <dst2> ...]`
/// array form is rarer and deliberately not parsed (skipped, not
/// guessed - same "never invent a mapping" rule the whole ToUnicode
/// resolver follows).
fn bfrange_entry_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>").unwrap())
}

/// The other legal `bfrange` form: `<start> <end> [<dst1> <dst2> ...]` -
/// one explicit destination per source CID, not a sequential increment.
/// Never confused with the sequential 3-hex-value form above: the
/// character immediately after the second hex string is `[` here, `<`
/// there, so each regex only ever matches its own real syntax.
fn bfrange_array_entry_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>\s*\[\s*((?:<[0-9A-Fa-f]+>\s*)+)\]").unwrap())
}

/// Parses a `/ToUnicode` CMap stream's decompressed PostScript-ish text
/// (`beginbfchar`/`endbfchar` and `beginbfrange`/`endbfrange` blocks - see
/// PDF32000-1:2008 §9.10.3) into `cid -> unicode text` entries, merging
/// into `map`. A destination value can itself be more than one UTF-16BE
/// code unit (a ligature mapping to multiple real characters); each dest
/// hex string is decoded as UTF-16BE and any unpaired/invalid code units
/// are dropped rather than guessed.
fn parse_tounicode_cmap(content: &str, map: &mut HashMap<u32, String>) {
    for block in bfchar_block_re().captures_iter(content) {
        for entry in bfchar_entry_re().captures_iter(&block[1]) {
            if let Ok(cid) = u32::from_str_radix(&entry[1], 16) {
                if let Some(text) = utf16be_hex_to_string(&entry[2]) {
                    map.insert(cid, text);
                }
            }
        }
    }
    for block in bfrange_block_re().captures_iter(content) {
        for entry in bfrange_entry_re().captures_iter(&block[1]) {
            let (Ok(start), Ok(end)) = (u32::from_str_radix(&entry[1], 16), u32::from_str_radix(&entry[2], 16)) else { continue };
            // A malformed range (end before start, or absurdly wide -
            // real fonts never legitimately need more than a few hundred
            // thousand code points) is skipped rather than looped, so a
            // corrupt/adversarial CMap can't turn this into an unbounded
            // allocation loop.
            if end < start || end - start > 200_000 {
                continue;
            }
            let dst_hex = &entry[3];
            // The dest is a single UTF-16BE code point that increments by
            // one per source CID in the range (the sequential form this
            // function supports) - only valid when dst_hex is exactly one
            // 4-hex-digit code unit; a longer dest here is the ligature
            // case, which has no well-defined "increment," so it's
            // skipped rather than guessed.
            if dst_hex.len() != 4 {
                continue;
            }
            let Ok(dst_start) = u32::from_str_radix(dst_hex, 16) else { continue };
            for (i, cid) in (start..=end).enumerate() {
                if let Some(ch) = char::from_u32(dst_start + i as u32) {
                    map.insert(cid, ch.to_string());
                }
            }
        }
        // Array form: one explicit destination per source CID (not a
        // sequential increment) - `zip` naturally stops at whichever of
        // the range or the array is shorter, so a claimed `end` far
        // beyond the array's actual length can't turn this into an
        // unbounded or out-of-range loop; no separate sanity cap needed
        // here the way the sequential form above requires one.
        for entry in bfrange_array_entry_re().captures_iter(&block[1]) {
            let (Ok(start), Ok(end)) = (u32::from_str_radix(&entry[1], 16), u32::from_str_radix(&entry[2], 16)) else { continue };
            if end < start {
                continue;
            }
            let dsts: Vec<&str> = hex_string_re().find_iter(&entry[3]).map(|m| &m.as_str()[1..m.as_str().len() - 1]).collect();
            for (cid, dst_hex) in (start..=end).zip(dsts) {
                if let Some(text) = utf16be_hex_to_string(dst_hex) {
                    map.insert(cid, text);
                }
            }
        }
    }
}

/// Decodes a hex string (as it appears inside `<...>`, e.g. `"0041"`) as
/// UTF-16BE text. Returns `None` if the hex itself is malformed or if
/// decoding produces no valid text at all (an odd number of hex digits,
/// or content that isn't valid UTF-16) - callers treat `None` as "skip,"
/// never as a reason to guess.
fn utf16be_hex_to_string(hex: &str) -> Option<String> {
    if hex.len() % 4 != 0 {
        return None;
    }
    let mut units = Vec::with_capacity(hex.len() / 4);
    for chunk in hex.as_bytes().chunks(4) {
        let chunk_str = std::str::from_utf8(chunk).ok()?;
        units.push(u16::from_str_radix(chunk_str, 16).ok()?);
    }
    String::from_utf16(&units).ok().filter(|s| !s.is_empty())
}

/// Decodes a Tj/TJ hex-string operand's inner hex text (e.g. `"0176"`)
/// into real characters via a previously-parsed ToUnicode CID map -
/// grouped into 2-byte (4-hex-digit) codes, the near-universal
/// Identity-H/Identity-V encoding for CID-keyed fonts. A code with no
/// entry in `map` is skipped, not guessed (raw CIDs are font-specific
/// glyph indices, not Unicode code points - treating an unmapped CID as
/// its own numeric value as a "character" would produce wrong text, not
/// just incomplete text).
fn hex_string_to_unicode(hex: &str, map: &HashMap<u32, String>) -> String {
    let mut out = String::new();
    for chunk in hex.as_bytes().chunks(4) {
        if chunk.len() != 4 {
            break; // trailing odd nibble(s) - malformed per this function's caller's regex guarantee, but defensive anyway
        }
        let Ok(chunk_str) = std::str::from_utf8(chunk) else { continue };
        let Ok(cid) = u32::from_str_radix(chunk_str, 16) else { continue };
        if let Some(text) = map.get(&cid) {
            out.push_str(text);
        }
    }
    // Some real-world ToUnicode CMaps (observed in a real Stripe-
    // generated invoice this extractor was fixed against) map a
    // word-separator glyph to U+0000 rather than U+0020 - a literal NUL
    // in extracted, searchable text is never useful either way, so it's
    // normalized to a space rather than left in or dropped (dropping it
    // would glue adjacent words together, which is worse for search than
    // an extra space).
    out.replace('\0', " ")
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
/// LIMITATIONS: resolves `/ToUnicode` CMaps for `beginbfchar`, sequential-
/// `beginbfrange` (`<start> <end> <dstStart>`), and array-form
/// `beginbfrange` (`<start> <end> [<dst1> <dst2> ...]`) - including
/// ligature (multi-character) destinations in `bfchar` and array-form
/// `bfrange` entries (see `parse_tounicode_cmap`) - which together cover
/// effectively all real-world `/ToUnicode` CMaps; a CID with no entry in
/// the resolved map (a missing/absent `/ToUnicode` stream, or a
/// ligature destination on the *sequential* `bfrange` form specifically,
/// which has no well-defined "increment" and is skipped rather than
/// guessed) still produces missing, not wrong, text for that glyph.
/// Finds JPEG-encoded (`/DCTDecode`) image XObject streams in a PDF - the
/// filter essentially every real-world scanner/scan-to-PDF tool uses for
/// a page image, and the one format OCR is attempted on (see this
/// module's Cargo.toml comment for why not CCITTFaxDecode/JPXDecode too:
/// "skip what isn't confidently handled" rather than adding more image-
/// codec dependencies for formats not yet seen in a real reported case).
/// Reuses `find_stream_blocks` - the same scanner the text-extraction
/// path uses - rather than a second, parallel PDF-object scanner; the
/// only difference here is which streams are kept (`/Image`+`/DCTDecode`
/// headers, not ones the normal path's `skip_marker_re` would skip).
/// A decoded, OCR-ready RGB8 image found in a PDF, however it was
/// actually encoded on disk - `ocr.rs` only ever deals with this already-
/// decoded shape, never a specific PDF image filter.
#[cfg(feature = "ocr")]
pub(crate) struct OcrCandidateImage {
    pub width: u32,
    pub height: u32,
    /// Tightly-packed RGB8 rows, `width * height * 3` bytes.
    pub rgb: Vec<u8>,
}

#[cfg(feature = "ocr")]
fn find_jpeg_image_streams(raw: &str) -> Vec<OcrCandidateImage> {
    let mut out = Vec::new();
    for (header, stream_text) in find_stream_blocks(raw) {
        if !header.contains("/Image") || !header.contains("/DCTDecode") || stream_text.is_empty() {
            continue;
        }
        let jpeg_bytes = if header.contains("/ASCII85Decode") {
            decode_ascii85(stream_text)
        } else {
            encode_latin1(stream_text)
        };
        if jpeg_bytes.is_empty() {
            continue;
        }
        let Ok(decoded) = image::load_from_memory(&jpeg_bytes) else { continue };
        let rgb_img = decoded.into_rgb8();
        let (width, height) = rgb_img.dimensions();
        out.push(OcrCandidateImage { width, height, rgb: rgb_img.into_raw() });
    }
    out
}

#[cfg(feature = "ocr")]
fn header_u32(header: &str, key: &str) -> Option<u32> {
    let idx = header.find(key)?;
    header[idx + key.len()..].trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// Finds `/Image` streams encoded as raw, uncompressed pixel data behind
/// a plain `/FlateDecode` filter (`/ColorSpace /DeviceRGB` or
/// `/DeviceGray`, `/BitsPerComponent 8`) - a real, legitimately common
/// encoding for scanned-document images from tools that store lossless
/// pixel data rather than JPEG-compressing it (found investigating this
/// exact case in a real scanned-PDF test fixture, `xlarge-scanned.pdf` -
/// see `benches/data/README.md`). Anything not matching this exact,
/// unambiguous shape (missing/unparseable dimensions, a `BitsPerComponent`
/// other than 8, a `ColorSpace` other than the two handled, or an
/// inflated byte count that doesn't match `width * height *
/// bytes_per_pixel` exactly) is skipped, not guessed at.
#[cfg(feature = "ocr")]
fn find_raw_flate_image_streams(raw: &str) -> Vec<OcrCandidateImage> {
    let mut out = Vec::new();
    for (header, stream_text) in find_stream_blocks(raw) {
        if !header.contains("/Image") || !header.contains("/FlateDecode") || header.contains("/DCTDecode") || stream_text.is_empty() {
            continue;
        }
        let (Some(width), Some(height), Some(bits_per_component)) = (header_u32(header, "/Width"), header_u32(header, "/Height"), header_u32(header, "/BitsPerComponent")) else {
            continue;
        };
        if bits_per_component != 8 {
            continue; // 1/2/4/16-bit-per-component images not handled - see doc comment
        }
        let bytes_per_pixel: u32 = if header.contains("/DeviceRGB") {
            3
        } else if header.contains("/DeviceGray") {
            1
        } else {
            continue; // DeviceCMYK and indexed/ICC color spaces not handled
        };
        let Some(expected_len) = (width as u64).checked_mul(height as u64).and_then(|n| n.checked_mul(bytes_per_pixel as u64)) else { continue };
        // A real full-page scan at a few hundred DPI in DeviceRGB (no
        // subsampling) is genuinely 20-100MB of raw pixel data - well
        // past `inflate_raw_deflate`'s shared 20MB deflate-bomb cap
        // (calibrated for arbitrary text-stream content, not a
        // dimension-bounded raster image). `bounded_inflate_to_exact_len`
        // below is deliberately NOT that shared cap: it reads at most
        // `expected_len` bytes - exactly what the declared, already-
        // sanity-checked dimensions require, no more - so a malicious
        // deflate stream can never force more than one legitimately-sized
        // image's worth of allocation regardless of its compression
        // ratio. The separate cap here guards the *other* attack surface
        // (a claimed Width/Height large enough to make even `expected_len`
        // itself an unreasonable allocation, before any inflating starts).
        if expected_len > 300_000_000 {
            continue;
        }

        let working_bytes = encode_latin1(stream_text);
        if working_bytes.len() <= 2 {
            continue;
        }
        let Some(inflated) = bounded_inflate_to_exact_len(&working_bytes[2..], expected_len) else { continue };

        let rgb = if bytes_per_pixel == 3 {
            inflated
        } else {
            // DeviceGray -> RGB (replicate the single channel 3x).
            let mut rgb = Vec::with_capacity(inflated.len() * 3);
            for g in &inflated {
                rgb.extend_from_slice(&[*g, *g, *g]);
            }
            rgb
        };
        out.push(OcrCandidateImage { width, height, rgb });
    }
    out
}

/// An image-only/scanned PDF (no text-showing operators at all, just a
/// drawn page image) extracts no text via the above - unless
/// `ocr_scanned_pdfs` is `true` (and this crate is built with the `ocr`
/// feature - see `ocr.rs`), in which case a whole-page JPEG image XObject
/// is decoded and OCR'd as a fallback, only when the normal pass above
/// found nothing. Filters other than ASCII85Decode/FlateDecode are not
/// handled.
pub fn extract_pdf_lines(
    bytes: &[u8],
    overall_timeout_seconds: u64,
    mut on_progress: Option<&mut (dyn FnMut(i32, Duration) + Send)>,
    ocr_scanned_pdfs: bool,
) -> (Option<Vec<String>>, bool) {
    let raw = decode_latin1(bytes);

    let mut lines: Vec<String> = Vec::new();
    let start = Instant::now();
    let mut last_progress_report = Duration::ZERO;
    let mut streams_scanned: i32 = 0;
    const MAX_CONTENT_CHARS: usize = 2_000_000;
    let mut truncated_by_time = false;

    // A ToUnicode CMap can appear anywhere in the file relative to the
    // content stream(s) that need it - in practice, usually *after*
    // (content streams tend to get lower object numbers than the font
    // resources a PDF writer appends afterward) - so hex-string operands
    // are collected here and resolved only once every stream has been
    // scanned and `cid_to_unicode` is as complete as it's going to get,
    // rather than requiring CMap-before-content file ordering.
    let mut cid_to_unicode: HashMap<u32, String> = HashMap::new();
    let mut pending_hex_strings: Vec<String> = Vec::new();

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

                // A ToUnicode CMap stream (PostScript-ish syntax, no
                // Tj/TJ operators of its own) - parse it for later hex-
                // string resolution rather than treating it as page
                // content.
                if content.contains("beginbfchar") || content.contains("beginbfrange") {
                    parse_tounicode_cmap(&content, &mut cid_to_unicode);
                }

                if tj_re().is_match(&content) {
                    for tm in text_re().find_iter(&content) {
                        let s = tm.as_str();
                        let raw_inner = &s[1..s.len() - 1];
                        let inner = unescape_pdf_string(raw_inner);
                        if !inner.trim().is_empty() {
                            lines.push(inner);
                        }
                    }
                    // CID-keyed/Type0 fonts show text as hex-string
                    // operands (`<0176> Tj`) instead of parenthesized
                    // literals - resolved after the full scan, once
                    // `cid_to_unicode` has seen every CMap in the file
                    // (see the comment on `pending_hex_strings` above).
                    // Many real-world generators (this extractor was
                    // fixed against a real Stripe-generated invoice that
                    // does exactly this) emit one `Tj` call per *glyph*,
                    // not per word - concatenating every hex match in
                    // this stream into one accumulated string (instead of
                    // one `lines` entry per match, which would fragment
                    // every word into single-character lines and break
                    // substring/whole-word search entirely) keeps real
                    // words intact for matching, at the cost of losing
                    // this stream's original line breaks - an accepted
                    // tradeoff, consistent with this being a best-effort
                    // extractor, not a real PDF renderer.
                    let mut stream_hex_text = String::new();
                    for hm in hex_string_re().find_iter(&content) {
                        let s = hm.as_str();
                        stream_hex_text.push_str(&s[1..s.len() - 1]);
                    }
                    if !stream_hex_text.is_empty() {
                        pending_hex_strings.push(stream_hex_text);
                    }
                }
            }
        }
    }

    for hex in &pending_hex_strings {
        let text = hex_string_to_unicode(hex, &cid_to_unicode);
        if !text.trim().is_empty() {
            lines.push(text);
        }
    }

    // OCR fallback (opt-in, feature-gated) - only attempted when every
    // extraction path above found nothing at all. Never runs for a PDF
    // that already has real, extractable text, so the common case pays
    // zero OCR cost.
    if ocr_scanned_pdfs && lines.is_empty() && !truncated_by_time {
        #[cfg(feature = "ocr")]
        {
            let mut candidates = find_jpeg_image_streams(&raw);
            candidates.extend(find_raw_flate_image_streams(&raw));
            // Real full-page OCR measured around 0.6-1s per page against
            // a real scanned document - a multi-page scanned PDF could
            // easily have dozens of page images, which would blow far
            // past `overall_timeout_seconds` if run unconditionally.
            // Checked before *every* image (not just once up front), so
            // a run that's already close to the timeout after the normal
            // extraction pass still gets bounded correctly, and results
            // already found are always kept - this never trades a
            // completed page's real text away because a later page would
            // have run over.
            let mut ocr_truncated = false;
            for image in &candidates {
                if start.elapsed().as_secs_f64() >= overall_timeout_seconds as f64 {
                    ocr_truncated = true;
                    break;
                }
                if let Some(cb) = on_progress.as_deref_mut() {
                    cb(streams_scanned, start.elapsed());
                }
                if let Some(ocr_lines) = crate::ocr::ocr_image(image) {
                    lines.extend(ocr_lines);
                }
            }
            if ocr_truncated {
                truncated_by_time = true;
            }
        }
        #[cfg(not(feature = "ocr"))]
        {
            // This build wasn't compiled with the `ocr` feature - the
            // setting exists (so `SearchSettings`/UI don't need their own
            // feature-conditional compilation) but has no effect here.
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

/// Inflates raw deflate `data`, requiring the result to be *exactly*
/// `expected_len` bytes - not "at most," like `inflate_raw_deflate`'s
/// fixed cap. Used only for dimension-declared raster image data (see
/// `find_raw_flate_image_streams`), where the caller already knows
/// precisely how many bytes a correctly-formed stream must produce.
/// Reads one byte beyond `expected_len` to distinguish "produced exactly
/// `expected_len` bytes" from "produced `expected_len` bytes so far, but
/// there's more" - both `Ok`, until this exact-length check - without
/// that extra byte, a deflate bomb could still be capped at
/// `expected_len` and *appear* to match by truncation alone. Bounding the
/// read at `expected_len + 1` (not an unrelated fixed constant) is what
/// keeps this safe regardless of `expected_len`'s size: the caller
/// already validated it against a sane upper bound before calling this.
#[cfg(feature = "ocr")]
fn bounded_inflate_to_exact_len(data: &[u8], expected_len: u64) -> Option<Vec<u8>> {
    use flate2::read::DeflateDecoder;
    let decoder = DeflateDecoder::new(data);
    let mut out = Vec::new();
    let read = decoder.take(expected_len + 1).read_to_end(&mut out).ok()?;
    if read as u64 == expected_len {
        Some(out)
    } else {
        None
    }
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
        let (pdf_result, _) = extract_pdf_lines(&pdf_bytes, 15, None, false);
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

    // ---- CID-keyed/Type0 font hex-string + ToUnicode CMap support
    // (found and fixed investigating a real user-reported PDF - a
    // Stripe-generated invoice - that extracted zero text despite its
    // content stream being correctly located and decompressed; see
    // `extract_pdf_lines`'s LIMITATIONS doc comment). Root cause: the
    // file's font is CID-keyed, so `Tj`/`TJ` operands are hex strings
    // (`<0176> Tj`), not parenthesized literals (`(...) Tj`) - `text_re`
    // only ever matched the latter. The real invoice file itself is
    // personal financial data and is deliberately NOT committed to this
    // repo; these tests reproduce the same structure (hex-string Tj
    // operands + a `/ToUnicode` CMap) synthetically instead. ----

    #[test]
    fn parse_tounicode_cmap_handles_bfchar_entries() {
        let cmap = "1 beginbfchar\n<0041> <0048>\n<0042> <0065>\nendbfchar\n";
        let mut map = HashMap::new();
        parse_tounicode_cmap(cmap, &mut map);
        assert_eq!(map.get(&0x0041).map(String::as_str), Some("H"));
        assert_eq!(map.get(&0x0042).map(String::as_str), Some("e"));
    }

    #[test]
    fn parse_tounicode_cmap_handles_sequential_bfrange_entries() {
        // <0100> <0102> <0041> means CID 0x0100->'A', 0x0101->'B', 0x0102->'C'.
        let cmap = "1 beginbfrange\n<0100> <0102> <0041>\nendbfrange\n";
        let mut map = HashMap::new();
        parse_tounicode_cmap(cmap, &mut map);
        assert_eq!(map.get(&0x0100).map(String::as_str), Some("A"));
        assert_eq!(map.get(&0x0101).map(String::as_str), Some("B"));
        assert_eq!(map.get(&0x0102).map(String::as_str), Some("C"));
    }

    #[test]
    fn parse_tounicode_cmap_handles_array_form_bfrange_entries() {
        // <0200> <0202> [<0058> <0059> <005A>] means CID 0x0200->'X',
        // 0x0201->'Y', 0x0202->'Z' - explicit per-CID destinations, not a
        // sequential increment from a single starting value.
        let cmap = "1 beginbfrange\n<0200> <0202> [<0058> <0059> <005A>]\nendbfrange\n";
        let mut map = HashMap::new();
        parse_tounicode_cmap(cmap, &mut map);
        assert_eq!(map.get(&0x0200).map(String::as_str), Some("X"));
        assert_eq!(map.get(&0x0201).map(String::as_str), Some("Y"));
        assert_eq!(map.get(&0x0202).map(String::as_str), Some("Z"));
    }

    #[test]
    fn parse_tounicode_cmap_array_form_bfrange_supports_ligature_destinations() {
        // A dest entry longer than one UTF-16 code unit (a ligature, e.g.
        // CID 0x0300 rendering as the two characters "fi") - the array
        // form has no "sequential increment" ambiguity to worry about
        // (unlike the 3-hex-value form), so multi-unit dests just work.
        let cmap = "1 beginbfrange\n<0300> <0300> [<00660069>]\nendbfrange\n";
        let mut map = HashMap::new();
        parse_tounicode_cmap(cmap, &mut map);
        assert_eq!(map.get(&0x0300).map(String::as_str), Some("fi"));
    }

    #[test]
    fn parse_tounicode_cmap_array_form_bfrange_stops_at_the_shorter_of_range_or_array() {
        // A claimed range far wider than the actual array must not loop
        // out of bounds or allocate absurdly - `zip` naturally stops once
        // the array is exhausted, regardless of what `end` claims.
        let cmap = "1 beginbfrange\n<0000> <FFFFFF> [<0041> <0042>]\nendbfrange\n";
        let mut map = HashMap::new();
        parse_tounicode_cmap(cmap, &mut map); // must return promptly
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&0x0000).map(String::as_str), Some("A"));
        assert_eq!(map.get(&0x0001).map(String::as_str), Some("B"));
    }

    #[test]
    fn parse_tounicode_cmap_skips_a_pathologically_wide_bfrange_instead_of_looping() {
        let cmap = "1 beginbfrange\n<0000> <FFFFFF> <0041>\nendbfrange\n";
        let mut map = HashMap::new();
        parse_tounicode_cmap(cmap, &mut map); // must return promptly, not loop ~16M times
        assert!(map.is_empty(), "a range wider than the sanity cap must be skipped entirely, not partially applied");
    }

    #[test]
    fn hex_string_to_unicode_maps_known_cids_and_skips_unknown_ones() {
        let mut map = HashMap::new();
        map.insert(0x0041, "H".to_string());
        map.insert(0x0042, "i".to_string());
        // 0x0043 deliberately absent from the map.
        assert_eq!(hex_string_to_unicode("00410042", &map), "Hi");
        assert_eq!(hex_string_to_unicode("00410043", &map), "H", "an unmapped CID must be skipped, not guessed as a literal codepoint");
    }

    #[test]
    fn hex_string_to_unicode_normalizes_embedded_nul_to_a_space() {
        let mut map = HashMap::new();
        map.insert(0x0041, "a".to_string());
        map.insert(0x0000, "\0".to_string());
        map.insert(0x0042, "b".to_string());
        assert_eq!(hex_string_to_unicode("004100000042", &map), "a b");
    }

    /// End-to-end proof, built from a small synthetic (uncompressed, no
    /// `/Filter`) PDF-like byte stream rather than requiring a real PDF
    /// fixture - `extract_pdf_lines` treats a stream with no `/Filter` as
    /// literal content, so this exercises the exact same code path a
    /// real `/FlateDecode`d CID-font PDF would, without needing to
    /// hand-roll a deflate stream in the test. Mirrors the real
    /// structure that motivated this fix: a content stream with several
    /// single-glyph `Tj` calls (many real-world generators emit one `Tj`
    /// per glyph, not per word) plus a separate `/ToUnicode` CMap stream
    /// - proving both the CID->text mapping AND the "concatenate within
    /// one stream, don't fragment into one line per glyph" behavior that
    /// keeps whole words searchable.
    #[test]
    fn extract_pdf_lines_resolves_cid_hex_text_via_tounicode_cmap() {
        let pdf = concat!(
            "%PDF-1.4\n",
            "10 0 obj\n<< /Length 60 >>\nstream\n",
            "BT <0041> Tj <0042> Tj <0043> Tj <0044> Tj ET\n",
            "endstream\nendobj\n",
            "12 0 obj\n<< /Length 100 >>\nstream\n",
            "1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
            "4 beginbfchar\n<0041> <0048>\n<0042> <0069>\n<0043> <0021>\n<0044> <003F>\nendbfchar\n",
            "endcmap\nendstream\nendobj\n",
        );
        let (result, low_confidence) = extract_pdf_lines(pdf.as_bytes(), 15, None, false);
        assert!(!low_confidence);
        let lines = result.expect("must extract the CID-hex-encoded text via the ToUnicode CMap, not return None");
        assert!(
            lines.iter().any(|l| l.contains("Hi!?")),
            "expected a line containing the concatenated, correctly-mapped text \"Hi!?\" (one line per stream, glyphs joined - not fragmented into one line per Tj call); got: {lines:?}"
        );
    }

    /// Regression guard: a PDF using ordinary parenthesized-literal `Tj`
    /// operands (the common case, unaffected by this fix) must keep
    /// working exactly as before - each literal string its own line,
    /// no interaction with the new hex-string/CMap path.
    #[test]
    fn extract_pdf_lines_still_handles_plain_literal_tj_operands_unchanged() {
        let pdf = concat!("10 0 obj\n<< /Length 40 >>\nstream\n", "BT (Hello) Tj (World) Tj ET\n", "endstream\nendobj\n",);
        let (result, _) = extract_pdf_lines(pdf.as_bytes(), 15, None, false);
        let lines = result.expect("plain literal-string Tj text must still extract");
        assert!(lines.contains(&"Hello".to_string()));
        assert!(lines.contains(&"World".to_string()));
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

    // ---- OCR (feature-gated, "ocr" Cargo feature) ----

    #[cfg(feature = "ocr")]
    fn tiny_jpeg_bytes() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(32, 32, image::Rgb([255, 255, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg).unwrap();
        bytes
    }

    /// Proves the PDF-side half of OCR support - finding and correctly
    /// extracting a `/DCTDecode` image XObject's raw JPEG bytes - without
    /// needing the (slow, model-loading) ML half. Real, arbitrary binary
    /// JPEG bytes (the full 0-255 byte range, not just ASCII text) must
    /// round-trip through `find_stream_blocks`'s Latin-1 decode/encode
    /// exactly, since JPEG's own byte stream would be corrupted otherwise.
    #[cfg(feature = "ocr")]
    #[test]
    fn find_jpeg_image_streams_extracts_dctdecode_image_bytes() {
        let jpeg = tiny_jpeg_bytes();
        let raw_bytes: Vec<u8> = [
            b"10 0 obj\n<< /Type /XObject /Subtype /Image /Width 32 /Height 32 /Filter /DCTDecode /Length ".as_slice(),
            jpeg.len().to_string().as_bytes(),
            b" >>\nstream\n".as_slice(),
            &jpeg,
            b"\nendstream\nendobj\n".as_slice(),
        ]
        .concat();
        let raw = decode_latin1(&raw_bytes);

        let found = find_jpeg_image_streams(&raw);
        assert_eq!(found.len(), 1, "must find exactly the one /Image + /DCTDecode stream");
        // Proves the extracted bytes really did decode as a valid,
        // undamaged JPEG through the real `image` crate decoder (not
        // merely "some bytes came out") - the round-trip through
        // `find_stream_blocks`'s Latin-1 decode/encode must be lossless
        // across JPEG's full 0-255 byte range, not just ASCII text.
        assert_eq!((found[0].width, found[0].height), (32, 32));
        assert_eq!(found[0].rgb.len(), 32 * 32 * 3);
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn find_jpeg_image_streams_skips_non_image_and_non_dctdecode_streams() {
        // A regular FlateDecode content stream (not an image at all) and
        // an /Image stream using a filter other than /DCTDecode (not
        // attempted - see the module's Cargo.toml comment on scope)
        // must both be skipped, not misidentified as OCR candidates.
        let raw = "5 0 obj\n<< /Length 10 >>\nstream\nBT (hi) Tj ET\nendstream\nendobj\n\
             6 0 obj\n<< /Subtype /Image /Filter /FlateDecode /Length 4 >>\nstream\nabcd\nendstream\nendobj\n";
        assert!(find_jpeg_image_streams(raw).is_empty());
    }

    /// `#[ignore]`d - loads the real ~12MB bundled models and runs real
    /// inference, real per-run cost (seconds, not the millisecond range
    /// the rest of this test suite runs in), same reasoning this crate
    /// already applies to its other slow/real-fixture tests. Run on
    /// demand: `cargo test -p search-core --features ocr -- --ignored
    /// ocr_images_runs_without_panicking_on_a_real_image`. Doesn't assert
    /// specific recognized text - a solid-color image has none to find -
    /// only that the full model-load-through-inference pipeline runs to
    /// completion without panicking or returning `None` (which would mean
    /// the bundled models themselves failed to load).
    #[cfg(feature = "ocr")]
    #[test]
    #[ignore]
    fn ocr_images_runs_without_panicking_on_a_real_image() {
        let jpeg = tiny_jpeg_bytes();
        let decoded = image::load_from_memory(&jpeg).unwrap().into_rgb8();
        let (width, height) = decoded.dimensions();
        let candidate = OcrCandidateImage { width, height, rgb: decoded.into_raw() };
        let result = crate::ocr::ocr_image(&candidate);
        assert!(result.is_some(), "the bundled models must load successfully");
    }

    /// `#[ignore]`d (real ~12MB model load + real ~1s/page inference,
    /// same reasoning as this file's other slow/real-fixture tests).
    /// `xlarge-scanned.pdf` (`benches/data/README.md`) is a real,
    /// legitimately-sourced scanned document whose pages are raw
    /// `/DeviceRGB`/`/FlateDecode` images (2479x3509 - the exact shape
    /// that motivated `find_raw_flate_image_streams`/
    /// `bounded_inflate_to_exact_len`, since its ~26MB-per-page raw pixel
    /// data exceeds `inflate_raw_deflate`'s shared 20MB deflate-bomb cap,
    /// which is why a dedicated, dimension-bounded inflate path exists at
    /// all rather than reusing that one). Locks in that real, readable
    /// text keeps coming out - this is the actual case that proved the
    /// feature works, not just a synthetic round-trip.
    #[cfg(feature = "ocr")]
    #[test]
    #[ignore]
    fn ocr_extracts_real_text_from_the_real_scanned_pdf_fixture() {
        let data_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/data");
        let bytes = std::fs::read(format!("{data_dir}/xlarge-scanned.pdf")).unwrap();
        let raw = decode_latin1(&bytes);
        let candidates = find_raw_flate_image_streams(&raw);
        assert!(!candidates.is_empty(), "must find this fixture's raw-FlateDecode page images");
        assert_eq!((candidates[0].width, candidates[0].height), (2479, 3509));

        let lines = crate::ocr::ocr_image(&candidates[0]).expect("OCR must run against the bundled models");
        let joined = lines.join(" ");
        for expected in ["PDF", "Testing", "sample-files.com"] {
            assert!(joined.contains(expected), "expected {expected:?} in real OCR output, got: {lines:?}");
        }
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn find_raw_flate_image_streams_extracts_raw_devicergb_pixels() {
        let (width, height) = (4u32, 3u32);
        let pixels: Vec<u8> = (0..(width * height * 3) as u8).collect(); // deterministic, distinguishable bytes
        let mut deflated = Vec::new();
        {
            use flate2::write::DeflateEncoder;
            use flate2::Compression;
            let mut enc = DeflateEncoder::new(&mut deflated, Compression::default());
            std::io::Write::write_all(&mut enc, &pixels).unwrap();
            enc.finish().unwrap();
        }
        // A real /FlateDecode PDF stream is the zlib-wrapped form (2-byte
        // header + raw deflate body + adler32 trailer) - `inflate_raw_deflate`
        // only reads the raw-deflate body, so the header bytes just need
        // to be present and skippable, matching every other FlateDecode
        // stream in this file's own handling.
        let mut stream_bytes = vec![0x78u8, 0x9c];
        stream_bytes.extend_from_slice(&deflated);

        let raw_bytes: Vec<u8> = [
            format!("10 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n", stream_bytes.len()).into_bytes(),
            stream_bytes,
            b"\nendstream\nendobj\n".to_vec(),
        ]
        .concat();
        let raw = decode_latin1(&raw_bytes);

        let found = find_raw_flate_image_streams(&raw);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].width, found[0].height), (width, height));
        assert_eq!(found[0].rgb, pixels, "decoded pixel bytes must match exactly, not just have the right length");
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn find_raw_flate_image_streams_skips_when_dimensions_dont_match_inflated_length() {
        // Declares 100x100 (30000 bytes expected) but the actual deflated
        // payload is tiny - a real mismatch this function must not guess
        // past.
        let raw = "10 0 obj\n<< /Subtype /Image /Width 100 /Height 100 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length 10 >>\nstream\nxx\nendstream\nendobj\n";
        assert!(find_raw_flate_image_streams(raw).is_empty());
    }

    /// `#[ignore]`d for the same real-model-load reason as above. Builds
    /// a synthetic image-only PDF (an `/Im1 Do` content stream drawing a
    /// `/DCTDecode` image XObject, no text-showing operators anywhere -
    /// the same shape as a real scanned-document PDF) and confirms the
    /// full `extract_pdf_lines` OCR fallback path runs to completion when
    /// `ocr_scanned_pdfs` is enabled, without panicking - the actual
    /// integration point a future refactor could most plausibly break.
    #[cfg(feature = "ocr")]
    #[test]
    #[ignore]
    fn extract_pdf_lines_falls_back_to_ocr_for_an_image_only_pdf_when_enabled() {
        let jpeg = tiny_jpeg_bytes();
        let pdf_bytes: Vec<u8> = [
            b"10 0 obj\n<< /Type /XObject /Subtype /Image /Name /Im1 /Width 32 /Height 32 /Filter /DCTDecode /Length ".as_slice(),
            jpeg.len().to_string().as_bytes(),
            b" >>\nstream\n".as_slice(),
            &jpeg,
            b"\nendstream\nendobj\n11 0 obj\n<< /Length 20 >>\nstream\nq 32 0 0 32 0 0 cm /Im1 Do Q\nendstream\nendobj\n".as_slice(),
        ]
        .concat();

        let (result, truncated) = extract_pdf_lines(&pdf_bytes, 15, None, true);
        assert!(!truncated);
        // No assertion on recognized text content (a solid-color image
        // has none) - this proves the pipeline doesn't panic and
        // completes, which is what this integration point can break.
        let _ = result;
    }
}
