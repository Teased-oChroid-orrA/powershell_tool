//! The viewer tab behind "Open original" (workbench Open button, report link).
//!
//! One browser page per result file: the file's text with the search's own highlighting (the report's
//! `highlight_ranges`, so look, colours and filtering match the report), a match list, flags beside the scrollbar, a
//! find box, a matches-only view and the other controls of the approved mockup (`.audit/open-original-viewer-mockup.html`,
//! v7). The page is one static script (`assets/viewer.js`) over a JSON config; this module builds the config.
//!
//! - [`Plan::Embedded`]: the extracted lines and the hits are inside the page, so it works offline and survives moving.
//! - [`Plan::Reader`]: a plain text file over [`SMALL_LIMIT`] is never loaded whole. [`reader::ReaderServer`] serves
//!   blocks by byte offset on `127.0.0.1` with a per-run token and the page fetches them as the user scrolls.
//! - [`Plan::Pdf`]: the browser's own PDF viewer shows the pages beside the match list.
//! - [`Plan::NoViewer`]: binaries and types with no text; the caller opens the default app, as before.

pub mod reader;

use std::path::Path;

use serde_json::{json, Value};

use crate::extraction::{extract_lines_by_extension, looks_binary};
use crate::models::{FileSearchResult, SearchSettings};
use crate::report::{file_uri, highlight_ranges, human_size, FilterColors};

/// Plain text files above this many bytes are served by the local reader instead of being embedded.
pub const SMALL_LIMIT: u64 = 1 << 20;
/// An embedded page never carries more than this many lines (the rest is cut and the page says so).
pub const EMBED_LINE_CAP: usize = 200_000;

const TEMPLATE: &str = include_str!("assets/viewer.html");
const CSS: &str = include_str!("assets/viewer.css");
const JS: &str = include_str!("assets/viewer.js");

/// How a file is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Embedded,
    Reader,
    Pdf,
    NoViewer,
}

/// Extensions whose text is extracted from a container (the lines are not the file's bytes), so they are always embedded.
const CONTAINER_EXTS: [&str; 5] = [".docx", ".pptx", ".xlsx", ".zip", ".rtf"];

fn ext_lower(path: &Path) -> String {
    path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default()
}

/// Decide how `path` is shown. Reads at most the first 8 KB (to tell text from a binary).
pub fn plan_for(path: &Path) -> Plan {
    let Ok(meta) = std::fs::metadata(path) else { return Plan::NoViewer };
    if !meta.is_file() {
        return Plan::NoViewer;
    }
    let ext = ext_lower(path);
    if ext == ".pdf" {
        return Plan::Pdf;
    }
    if CONTAINER_EXTS.contains(&ext.as_str()) {
        return Plan::Embedded;
    }
    let mut head = vec![0u8; 8192];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    head.truncate(n);
    // A UTF-16 text file is full of NUL bytes; its byte order mark says it is text, as the extractor reads it.
    let utf16 = head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]);
    if !utf16 && looks_binary(&head) {
        return Plan::NoViewer;
    }
    if meta.len() > SMALL_LIMIT && !utf16 {
        Plan::Reader
    } else {
        Plan::Embedded
    }
}

/// The lines each filter matched, as the viewer needs them: line number and a bit per filter (in `filters` order).
pub struct HitSet {
    /// Every filter of the search (`SearchSettings::all_filters`), in order.
    pub filters: Vec<String>,
    /// `(line number, filter bits)` sorted by line; bit `i` is filter `i`.
    pub entries: Vec<(u32, u64)>,
    colors: FilterColors,
}

impl HitSet {
    pub fn from_result(result: &FileSearchResult, settings: &SearchSettings) -> Self {
        let filters: Vec<String> = settings.all_filters().cloned().collect();
        let lower: Vec<String> = filters.iter().map(|f| f.to_lowercase()).collect();
        let mut entries: Vec<(u32, u64)> = Vec::with_capacity(result.hits.len());
        for h in &result.hits {
            if h.line_number < 1 {
                continue;
            }
            let mut bits = 0u64;
            for f in &h.matched_filters {
                if let Some(i) = lower.iter().position(|x| *x == f.to_lowercase()).filter(|i| *i < 64) {
                    bits |= 1 << i;
                }
            }
            entries.push((h.line_number as u32, bits));
        }
        entries.sort_by_key(|e| e.0);
        entries.dedup_by(|b, a| {
            if a.0 == b.0 {
                a.1 |= b.1;
                true
            } else {
                false
            }
        });
        Self { colors: FilterColors::new(settings), filters, entries }
    }

    /// The colour slot (0..6) of filter `i`, the same one the report gives it.
    fn slot_of(&self, i: usize) -> usize {
        self.colors.slot(&self.filters[i])
    }

    /// Filter bits folded onto the six colour slots.
    pub fn slot_mask(&self, bits: u64) -> u8 {
        (0..self.filters.len().min(64)).filter(|i| bits >> i & 1 == 1).fold(0u8, |m, i| m | 1 << self.slot_of(i))
    }

    fn bits_of(&self, line: u32) -> Option<u64> {
        self.entries.binary_search_by_key(&line, |e| e.0).ok().map(|i| self.entries[i].1)
    }

    /// `[start, end, slot]` highlight ranges of `text` for a hit line, in UTF-16 units (what JavaScript indexes by).
    pub fn spans(&self, line: u32, text: &str, settings: &SearchSettings) -> Vec<[u32; 3]> {
        let Some(bits) = self.bits_of(line) else { return Vec::new() };
        let names: Vec<String> = (0..self.filters.len().min(64)).filter(|i| bits >> i & 1 == 1).map(|i| self.filters[i].clone()).collect();
        let to16 = |byte: usize| text[..byte].encode_utf16().count() as u32;
        highlight_ranges(text, &names, settings, &self.colors).into_iter().map(|(s, e, slot)| [to16(s), to16(e), slot as u32]).collect()
    }

    /// Hits per filter, for the chips: `(name, slot, count)`.
    pub fn chips(&self) -> Vec<(String, usize, usize)> {
        self.filters
            .iter()
            .enumerate()
            .map(|(i, f)| (f.clone(), self.slot_of(i), self.entries.iter().filter(|e| e.1 >> i & 1 == 1).count()))
            .filter(|c| c.2 > 0)
            .collect()
    }
}

/// A page ready to write.
pub struct ViewerPage {
    pub html: String,
    pub plan: Plan,
    /// Lines in the page (0 for a PDF page or the reader).
    pub lines: usize,
}

#[derive(Debug)]
pub enum ViewerError {
    NoViewer,
    Read(String),
}

impl std::fmt::Display for ViewerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewerError::NoViewer => write!(f, "this file type has no viewer"),
            ViewerError::Read(e) => write!(f, "{e}"),
        }
    }
}

fn mod_time(meta: &std::fs::Metadata) -> Option<chrono::DateTime<chrono::Local>> {
    meta.modified().ok().map(chrono::DateTime::<chrono::Local>::from)
}

/// The yellow banner text when the file no longer matches what the search recorded.
fn stale_note(path: &Path, result: &FileSearchResult) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let now_mod = mod_time(&meta)?;
    let same_len = result.file_length < 0 || meta.len() as i64 == result.file_length;
    let same_time = (now_mod.timestamp() - result.modified.timestamp()).abs() <= 1;
    if same_len && same_time {
        return None;
    }
    Some(format!(
        "was {}, modified {}; now {}, modified {}",
        human_size(result.file_length),
        result.modified.format("%Y-%m-%d %H:%M"),
        human_size(meta.len() as i64),
        now_mod.format("%Y-%m-%d %H:%M")
    ))
}

fn badge_of(path: &Path) -> String {
    let e = ext_lower(path);
    if e.is_empty() { "TXT".into() } else { e[1..].to_uppercase().chars().take(4).collect() }
}

/// Fields every page shares.
fn base_config(path: &Path, result: &FileSearchResult, hits: &HitSet, size: u64) -> serde_json::Map<String, Value> {
    let meta = std::fs::metadata(path).ok();
    let mut m = serde_json::Map::new();
    m.insert("name".into(), json!(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    m.insert("path".into(), json!(path.to_string_lossy()));
    m.insert("size".into(), json!(human_size(size as i64)));
    m.insert("mod".into(), json!(meta.as_ref().and_then(mod_time).map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()));
    m.insert("badge".into(), json!(badge_of(path)));
    m.insert("uri".into(), json!(file_uri(&path.to_string_lossy())));
    m.insert("stale".into(), json!(stale_note(path, result)));
    m.insert("filters".into(), json!(hits.chips().iter().map(|(n, s, c)| json!({"name": n, "slot": s, "count": c})).collect::<Vec<_>>()));
    m.insert("ocr".into(), json!(if result.low_confidence_pdf { result.pdf_warning() } else { None }));
    m
}

/// Line ending and encoding label of a plain text file, from its first 64 KB.
fn text_label(path: &Path, size: u64) -> String {
    let mut head = vec![0u8; 65536];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    head.truncate(n);
    let enc = if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
        "UTF-8 BOM"
    } else if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        "UTF-16"
    } else if std::str::from_utf8(&head).is_ok() || head.len() == 65536 {
        "UTF-8"
    } else {
        "Windows-1252"
    };
    let crlf = head.windows(2).filter(|w| w == b"\r\n").count();
    let lf = head.iter().filter(|b| **b == b'\n').count();
    let eol = if lf == 0 { "no line break" } else if crlf * 2 >= lf { "CRLF" } else { "LF" };
    format!("{enc} \u{b7} {eol} \u{b7} {}", human_size(size as i64))
}

fn assemble(cfg: &Value) -> String {
    let json = serde_json::to_string(cfg).unwrap_or_else(|_| "{}".into()).replace('<', "\\u003c");
    let title = cfg["name"].as_str().unwrap_or("File");
    TEMPLATE
        .replace("{{TITLE}}", &title.replace('&', "&amp;").replace('<', "&lt;"))
        .replace("{{CSS}}", CSS)
        .replace("{{CFG}}", &json)
        .replace("{{JS}}", JS)
}

/// Build the self-contained page for an embedded or PDF file. Large plain text goes through the reader instead
/// ([`reader::ReaderServer::register`]); asking for it here reads and embeds the whole file.
pub fn build_page(path: &Path, result: &FileSearchResult, settings: &SearchSettings) -> Result<ViewerPage, ViewerError> {
    let plan = plan_for(path);
    if plan == Plan::NoViewer {
        return Err(ViewerError::NoViewer);
    }
    let bytes = std::fs::read(path).map_err(|e| ViewerError::Read(e.to_string()))?;
    let size = bytes.len() as u64;
    let hits = HitSet::from_result(result, settings);
    let mut cfg = base_config(path, result, &hits, size);

    let ext = ext_lower(path);
    let extracted = extract_lines_by_extension(&ext, &bytes, settings.pdf_timeout_seconds.max(1) as u64, None, settings.ocr_scanned_pdfs);
    let mut lines = match extracted {
        Ok(e) => e.lines,
        Err(_) if plan == Plan::Pdf => Vec::new(),
        Err(_) => return Err(ViewerError::Read("no text could be read from this file".into())),
    };
    let total = lines.len();
    let capped = total > EMBED_LINE_CAP;
    lines.truncate(EMBED_LINE_CAP);

    let hit_rows: Vec<Value> = hits
        .entries
        .iter()
        .filter(|e| (e.0 as usize) <= lines.len())
        .map(|&(n, bits)| json!([n, hits.slot_mask(bits), hits.spans(n, &lines[n as usize - 1], settings)]))
        .collect();

    let container = CONTAINER_EXTS.contains(&ext.as_str());
    let (kind, enc, foot) = match plan {
        Plan::Pdf => (
            "pdf",
            format!("PDF \u{b7} {}", human_size(size as i64)),
            "The browser's own PDF viewer shows the original pages. The match list is built from the extracted text.".to_string(),
        ),
        _ if container => (
            "lines",
            format!("Extracted text \u{b7} {}", human_size(size as i64)),
            "Built from the text Search extracts. Fonts, images and page layout are not reproduced.".to_string(),
        ),
        _ => ("lines", text_label(path, size), "Source view. The text is embedded in this page, so it works offline.".to_string()),
    };
    cfg.insert("kind".into(), json!(kind));
    cfg.insert("enc".into(), json!(enc));
    cfg.insert("foot".into(), json!(foot));
    cfg.insert("total".into(), json!(total));
    cfg.insert("capped".into(), json!(capped));
    cfg.insert("embedded".into(), json!({ "lines": lines, "hits": hit_rows }));
    if plan == Plan::Pdf {
        cfg.insert("pdf".into(), json!(file_uri(&path.to_string_lossy())));
    }
    let n = lines.len();
    Ok(ViewerPage { html: assemble(&Value::Object(cfg)), plan, lines: n })
}

/// The page the reader serves for a registered large file; `base` is the URL prefix of its data endpoints.
pub(crate) fn reader_page(path: &Path, result: &FileSearchResult, hits: &HitSet, total: u64, size: u64, base: &str) -> String {
    let mut cfg = base_config(path, result, hits, size);
    cfg.insert("kind".into(), json!("lines"));
    cfg.insert("enc".into(), json!(text_label(path, size)));
    cfg.insert("foot".into(), json!("Read in blocks by byte offset. The file is never loaded whole."));
    cfg.insert("total".into(), json!(total));
    cfg.insert("reader".into(), json!({ "base": base }));
    assemble(&Value::Object(cfg))
}

/// A stable, file-system safe name for the page of `path` (`<stem>-<8 hex>.html`).
pub fn page_file_name(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let stem: String = path
        .file_stem()
        .map(|s| s.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect())
        .unwrap_or_else(|| "file".into());
    format!("{}-{}.html", stem.chars().take(40).collect::<String>(), digest.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>())
}

#[cfg(test)]
mod tests;
