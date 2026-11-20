//! The structure behind the viewer's "Structured" view of Office files.
//!
//! The extractors flatten a document to lines, and the search's line numbers refer to those lines. The structured view
//! must therefore not change the lines: it adds one descriptor per line (a heading, a list item, a table cell) and the
//! images that sit on a line. For `.docx` the descriptors come from the same XML transformation the extractor runs, with
//! private-use marker characters inserted at the tags that matter, so line `n` here is line `n` of
//! [`crate::extraction::extract_docx_lines`] by construction (a test compares them). `.pptx` slides and `.xlsx` sheets
//! carry their own `--- Slide N ---` / `--- Sheet N ---` marker lines, which the page reads directly; this module only
//! finds the images of a slide.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::OnceLock;

use base64::Engine;
use regex::Regex;
use serde_json::{json, Value};

use crate::extraction::{decode_xml_entities, read_zip_entry_to_string, split_lines, strip_tags};

/// One picture larger than this is left out (the page would carry it as base64).
const IMAGE_MAX_BYTES: u64 = 3 << 20;
/// All pictures of a page together.
const IMAGES_MAX_TOTAL: u64 = 12 << 20;
const IMAGES_MAX_COUNT: usize = 60;

// Marker characters (Unicode private use), inserted before the extractor's tag stripping and removed from every line.
const M_PARA: char = '\u{E010}';
const M_STYLE: char = '\u{E011}';
const M_STYLE_END: char = '\u{E012}';
const M_LIST: char = '\u{E013}';
const M_TBL: char = '\u{E014}';
const M_ROW: char = '\u{E015}';
const M_CELL: char = '\u{E016}';
const M_TBL_END: char = '\u{E017}';
const M_IMG: char = '\u{E018}';
const M_IMG_END: char = '\u{E019}';

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("static pattern"))
}

/// What the page needs besides the lines.
#[derive(Debug, Default, PartialEq)]
pub struct Structure {
    pub kind: &'static str,
    /// Per line, `.docx` only: `""` paragraph, `"h1"`..`"h9"` heading, `"title"`, `"li"` list item, or
    /// `"c<table>.<row>.<column>"` for a paragraph inside a table cell (the heading and list codes are not used
    /// inside a table).
    pub meta: Vec<String>,
    /// Picture sources as `data:` URIs.
    pub images: Vec<String>,
    /// `(line, index into images)`; the picture is drawn at that line.
    pub refs: Vec<(u32, usize)>,
}

impl Structure {
    pub fn to_json(&self) -> Value {
        json!({ "kind": self.kind, "meta": self.meta, "imgs": self.images, "refs": self.refs.iter().map(|r| json!([r.0, r.1])).collect::<Vec<_>>() })
    }
}

type Zip<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

fn mime_of(name: &str) -> Option<&'static str> {
    match name.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        "webp" => Some("image/webp"),
        _ => None, // emf, wmf, svg, tiff: a browser either cannot draw them or they can carry active content
    }
}

/// Resolves relationship ids to the picture part they point at, reading pictures lazily under the size limits.
struct Pictures<'a> {
    zip: Zip<'a>,
    rels: HashMap<String, String>,
    base: &'static str,
    images: Vec<String>,
    by_part: HashMap<String, Option<usize>>,
    total: u64,
}

impl<'a> Pictures<'a> {
    fn new(zip: Zip<'a>) -> Self {
        Self { zip, rels: HashMap::new(), base: "", images: Vec::new(), by_part: HashMap::new(), total: 0 }
    }

    /// Read the picture relationships of one part: `rels_part` is its `.rels` file (`word/_rels/document.xml.rels`) and
    /// `base` the folder its targets are relative to (`word/`). Pictures already loaded stay loaded.
    fn use_rels(&mut self, rels_part: &str, base: &'static str) {
        static REL: OnceLock<Regex> = OnceLock::new();
        self.rels.clear();
        self.base = base;
        let Some(xml) = read_zip_entry_to_string(&mut self.zip, rels_part) else { return };
        for m in re(&REL, r#"<Relationship\b[^>]*>"#).find_iter(&xml) {
            let tag = m.as_str();
            let attr = |k: &str| {
                let p = format!(r#"{k}=""#);
                tag.find(&p).and_then(|i| tag[i + p.len()..].split('"').next()).map(str::to_string)
            };
            if let (Some(id), Some(target)) = (attr("Id"), attr("Target")) {
                if attr("Type").is_some_and(|t| t.ends_with("/image")) && attr("TargetMode").as_deref() != Some("External") {
                    self.rels.insert(id, target);
                }
            }
        }
    }

    /// The index in `images` of the picture behind relationship `rid`, loading it the first time.
    fn get(&mut self, rid: &str) -> Option<usize> {
        let target = self.rels.get(rid)?.clone();
        let part = resolve(self.base, &target);
        if let Some(found) = self.by_part.get(&part) {
            return *found;
        }
        let loaded = self.load(&part);
        self.by_part.insert(part, loaded);
        loaded
    }

    fn load(&mut self, part: &str) -> Option<usize> {
        let mime = mime_of(part)?;
        if self.images.len() >= IMAGES_MAX_COUNT {
            return None;
        }
        let entry = self.zip.by_name(part).ok()?;
        if entry.size() > IMAGE_MAX_BYTES || self.total + entry.size() > IMAGES_MAX_TOTAL {
            return None;
        }
        let mut bytes = Vec::new();
        entry.take(IMAGE_MAX_BYTES).read_to_end(&mut bytes).ok()?;
        self.total += bytes.len() as u64;
        self.images.push(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)));
        Some(self.images.len() - 1)
    }
}

/// `base` + relative `target` with `..` and `.` folded away, or an absolute (`/word/media/x.png`) target as is.
fn resolve(base: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Descriptors for the lines of a `.docx`, aligned with `extract_docx_lines`. `None` when the bytes are not a
/// readable `.docx` (the caller then shows the plain lines only).
pub fn docx_structure(bytes: &[u8]) -> Option<Structure> {
    static P: OnceLock<Regex> = OnceLock::new();
    static STYLE: OnceLock<Regex> = OnceLock::new();
    static NUM: OnceLock<Regex> = OnceLock::new();
    static TBL: OnceLock<Regex> = OnceLock::new();
    static ROW: OnceLock<Regex> = OnceLock::new();
    static CELL: OnceLock<Regex> = OnceLock::new();
    static BLIP: OnceLock<Regex> = OnceLock::new();
    static VML: OnceLock<Regex> = OnceLock::new();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let xml = read_zip_entry_to_string(&mut zip, "word/document.xml")?;
    if xml.chars().any(|c| ('\u{E010}'..='\u{E019}').contains(&c)) {
        return None;
    }
    let mark = |c: char| format!("{c}$0");
    let xml = re(&P, r"<w:p(?:\s[^>]*)?>").replace_all(&xml, mark(M_PARA).as_str()).into_owned();
    let xml = re(&STYLE, r#"<w:pStyle\s+w:val="([^"]*)"[^>]*>"#).replace_all(&xml, format!("$0{M_STYLE}$1{M_STYLE_END}").as_str()).into_owned();
    let xml = re(&NUM, r"<w:numPr>").replace_all(&xml, mark(M_LIST).as_str()).into_owned();
    let xml = re(&TBL, r"<w:tbl>").replace_all(&xml, mark(M_TBL).as_str()).into_owned().replace("</w:tbl>", &format!("</w:tbl>{M_TBL_END}"));
    let xml = re(&ROW, r"<w:tr(?:\s[^>]*)?>").replace_all(&xml, mark(M_ROW).as_str()).into_owned();
    let xml = re(&CELL, r"<w:tc(?:\s[^>]*)?>").replace_all(&xml, mark(M_CELL).as_str()).into_owned();
    let xml = re(&BLIP, r#"<a:blip\b[^>]*?\br:embed="([^"]+)"[^>]*>"#).replace_all(&xml, format!("$0{M_IMG}$1{M_IMG_END}").as_str()).into_owned();
    let xml = re(&VML, r#"<v:imagedata\b[^>]*?\br:id="([^"]+)"[^>]*>"#).replace_all(&xml, format!("$0{M_IMG}$1{M_IMG_END}").as_str()).into_owned();

    // the extractor's own pipeline, unchanged
    let xml = xml.replace("</w:p>", "\n").replace("<w:br/>", "\n").replace("<w:br />", "\n");
    let lines = split_lines(&decode_xml_entities(&strip_tags(&xml)));

    let mut pics = Pictures::new(zip);
    pics.use_rels("word/_rels/document.xml.rels", "word/");
    let (mut meta, mut refs) = (Vec::with_capacity(lines.len()), Vec::new());
    let (mut depth, mut tbl, mut row, mut col) = (0u32, 0i64, -1i64, -1i64);
    let (mut style, mut list) = (String::new(), false);
    for (i, line) in lines.iter().enumerate() {
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                M_PARA => {
                    style.clear();
                    list = false;
                }
                M_STYLE => {
                    style = chars.by_ref().take_while(|c| *c != M_STYLE_END).collect();
                }
                M_LIST => list = true,
                M_TBL => {
                    depth += 1;
                    if depth == 1 {
                        tbl += 1;
                        (row, col) = (-1, -1);
                    }
                }
                M_ROW if depth == 1 => {
                    row += 1;
                    col = -1;
                }
                M_CELL if depth == 1 => col += 1,
                M_TBL_END => depth = depth.saturating_sub(1),
                M_IMG => {
                    let rid: String = chars.by_ref().take_while(|c| *c != M_IMG_END).collect();
                    if let Some(ix) = pics.get(&rid) {
                        refs.push((i as u32 + 1, ix));
                    }
                }
                _ => {}
            }
        }
        meta.push(if depth > 0 && row >= 0 && col >= 0 { format!("c{tbl}.{row}.{col}") } else { style_code(&style, list) });
    }
    Some(Structure { kind: "docx", meta, images: pics.images, refs })
}

fn style_code(style: &str, list: bool) -> String {
    let s = style.to_ascii_lowercase();
    if s == "title" {
        return "title".into();
    }
    if let Some(level) = s.strip_prefix("heading").and_then(|n| n.trim().parse::<u8>().ok()).filter(|n| (1..=9).contains(n)) {
        return format!("h{level}");
    }
    if list || s.starts_with("listparagraph") || s.starts_with("list") {
        return "li".into();
    }
    String::new()
}

/// The pictures of each slide of a `.pptx`, attached to that slide's `--- Slide N ---` line in `lines`. A picture used on
/// several slides (a logo) is carried once.
pub fn pptx_structure(bytes: &[u8], lines: &[String]) -> Option<Structure> {
    static BLIP: OnceLock<Regex> = OnceLock::new();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let slides: Vec<(i32, u32)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| l.strip_prefix("--- Slide ")?.strip_suffix(" ---")?.parse::<i32>().ok().map(|n| (n, i as u32 + 1)))
        .collect();
    let xmls: Vec<Option<String>> = slides.iter().map(|(n, _)| read_zip_entry_to_string(&mut zip, &format!("ppt/slides/slide{n}.xml"))).collect();
    let mut pics = Pictures::new(zip);
    let mut refs = Vec::new();
    for ((num, line), xml) in slides.iter().zip(xmls) {
        let Some(xml) = xml else { continue };
        pics.use_rels(&format!("ppt/slides/_rels/slide{num}.xml.rels"), "ppt/slides/");
        let mut seen = Vec::new();
        for c in re(&BLIP, r#"<a:blip\b[^>]*?\br:embed="([^"]+)""#).captures_iter(&xml) {
            if let Some(ix) = pics.get(&c[1]).filter(|ix| !seen.contains(ix)) {
                seen.push(ix);
                refs.push((*line, ix));
            }
        }
    }
    Some(Structure { kind: "pptx", images: pics.images, refs, ..Default::default() })
}

#[cfg(test)]
mod tests;
