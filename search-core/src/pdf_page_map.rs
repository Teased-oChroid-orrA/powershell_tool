//! Conservative page attribution for the classic-xref PDF subset.
//!
//! This module does not extract text. It maps validated stream body offsets to
//! page-tree ordinals and refuses unsupported or ambiguous files.

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    rc::Rc,
};

const MAX_PDF_BYTES: usize = 64 * 1024 * 1024;
const MAX_XREF_OBJECTS: usize = 250_000;
const MAX_PAGES: usize = 10_000;
const MAX_DEPTH: usize = 128;
const MAX_REFERENCES: usize = 250_000;
const MAX_TOKENS: usize = 2_000_000;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Ref {
    id: u32,
    generation: u16,
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Number(i64),
    Ref(Ref),
    Name(String),
    Array(Vec<Value>),
    Dict(HashMap<String, Value>),
    String,
    Other,
}

#[derive(Clone)]
struct Object {
    value: Value,
    stream_start: Option<usize>,
}

#[derive(Clone, Copy)]
struct XrefEntry {
    offset: usize,
    generation: u16,
}

/// Return exact stream-body offsets mapped to 1-based page-tree ordinals.
/// Returns `None` when the PDF is unsupported, malformed, or over budget.
pub(crate) fn page_by_stream_offset(bytes: &[u8]) -> Option<HashMap<usize, NonZeroU32>> {
    if bytes.is_empty() || bytes.len() > MAX_PDF_BYTES || !bytes.starts_with(b"%PDF-") {
        return None;
    }
    let mut pdf = Pdf::open(bytes)?;
    let root = match pdf.trailer.get("Root")? {
        Value::Ref(reference) => *reference,
        _ => return None,
    };
    let catalog = pdf.object(root, &mut HashSet::new())?;
    if !matches!(dict_get(&catalog.value, "Type"), Some(Value::Name(name)) if name == "Catalog") {
        return None;
    }
    let pages = match dict_get(&catalog.value, "Pages")? {
        Value::Ref(reference) => *reference,
        _ => return None,
    };
    let mut owners = HashMap::<usize, Option<NonZeroU32>>::new();
    let mut seen = HashSet::<Ref>::new();
    let mut active = HashSet::<Ref>::new();
    let mut references = 0usize;
    let mut next_page = 0u32;
    let count = pdf.walk_pages(
        pages,
        None,
        0,
        &mut seen,
        &mut active,
        &mut owners,
        &mut next_page,
        &mut references,
    )?;
    if count == 0 || count > MAX_PAGES as u32 {
        return None;
    }
    Some(
        owners
            .into_iter()
            .filter_map(|(offset, page)| page.map(|page| (offset, page)))
            .collect(),
    )
}

struct Pdf<'a> {
    bytes: &'a [u8],
    xref: HashMap<u32, XrefEntry>,
    trailer: HashMap<String, Value>,
    objects: HashMap<Ref, Rc<Object>>,
    tokens: usize,
    references: usize,
}

impl<'a> Pdf<'a> {
    fn open(bytes: &'a [u8]) -> Option<Self> {
        let start = bytes.len().saturating_sub(2048);
        let tail = &bytes[start..];
        let eof = rfind_bytes(tail, b"%%EOF")? + start;
        let marker = rfind_bytes(&bytes[start..eof], b"startxref")? + start;
        let mut trailer_lex = Lexer::new(bytes, marker + 9);
        let xref_offset = trailer_lex.integer()?;
        if xref_offset < 0 {
            return None;
        }
        let xref_offset = usize::try_from(xref_offset).ok()?;
        while trailer_lex
            .bytes
            .get(trailer_lex.pos)
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            trailer_lex.pos += 1;
        }
        if trailer_lex.bytes.get(trailer_lex.pos..trailer_lex.pos + 5) != Some(b"%%EOF") {
            return None;
        }
        let mut pdf = Self {
            bytes,
            xref: HashMap::new(),
            trailer: HashMap::new(),
            objects: HashMap::new(),
            tokens: 0,
            references: 0,
        };
        let mut lex = Lexer::new(bytes, xref_offset);
        if !lex.keyword(b"xref")? {
            return None;
        }
        let mut entries = 0usize;
        let mut object_ids = HashSet::new();
        loop {
            if lex.keyword(b"trailer")? {
                break;
            }
            let first = lex.integer()?;
            let count = lex.integer()?;
            if first < 0 || count < 0 {
                return None;
            }
            let first = u32::try_from(first).ok()?;
            let count = usize::try_from(count).ok()?;
            entries = entries.checked_add(count)?;
            if entries > MAX_XREF_OBJECTS {
                return None;
            }
            for delta in 0..count {
                let offset = lex.integer()?;
                let generation = lex.integer()?;
                let in_use = lex.word()?;
                if offset < 0 || !(0..=u16::MAX as i64).contains(&generation) {
                    return None;
                }
                let id = first.checked_add(u32::try_from(delta).ok()?)?;
                if !object_ids.insert(id) {
                    return None;
                }
                if in_use == b"n" {
                    let entry = XrefEntry {
                        offset: usize::try_from(offset).ok()?,
                        generation: generation as u16,
                    };
                    pdf.xref.insert(id, entry);
                } else if in_use != b"f" {
                    return None;
                }
            }
        }
        let trailer = match lex.value(&mut pdf.tokens, 0)? {
            Value::Dict(dict) => dict,
            _ => return None,
        };
        if trailer.contains_key("Prev")
            || trailer.contains_key("XRefStm")
            || trailer.contains_key("Encrypt")
        {
            return None;
        }
        let size = match trailer.get("Size")? {
            Value::Number(n) if *n > 0 => usize::try_from(*n).ok()?,
            _ => return None,
        };
        if size > MAX_XREF_OBJECTS || entries > size {
            return None;
        }
        pdf.trailer = trailer;
        Some(pdf)
    }

    fn object(&mut self, reference: Ref, active: &mut HashSet<Ref>) -> Option<Rc<Object>> {
        if let Some(object) = self.objects.get(&reference) {
            return Some(Rc::clone(object));
        }
        let entry = *self.xref.get(&reference.id)?;
        if entry.generation != reference.generation || !active.insert(reference) {
            return None;
        }
        let mut lex = Lexer::new(self.bytes, entry.offset);
        let id = u32::try_from(lex.integer()?).ok()?;
        let generation = u16::try_from(lex.integer()?).ok()?;
        if id != reference.id || generation != reference.generation || !lex.keyword(b"obj")? {
            active.remove(&reference);
            return None;
        }
        let value = lex.value(&mut self.tokens, 0)?;
        let mut stream_start = None;
        if matches!(value, Value::Dict(_)) {
            lex.skip_space();
            if lex.keyword(b"stream")? {
                let body = if lex.bytes.get(lex.pos..lex.pos + 2) == Some(b"\r\n") {
                    lex.pos + 2
                } else if lex.bytes.get(lex.pos) == Some(&b'\n') {
                    lex.pos + 1
                } else {
                    active.remove(&reference);
                    return None;
                };
                let length_value = dict_get(&value, "Length")?;
                let length = match length_value {
                    Value::Number(n) if *n >= 0 => usize::try_from(*n).ok()?,
                    Value::Ref(length_ref) => {
                        self.references += 1;
                        if self.references > MAX_REFERENCES {
                            active.remove(&reference);
                            return None;
                        }
                        let length_object = self.object(*length_ref, active)?;
                        match length_object.value {
                            Value::Number(n) if n >= 0 => usize::try_from(n).ok()?,
                            _ => {
                                active.remove(&reference);
                                return None;
                            }
                        }
                    }
                    _ => {
                        active.remove(&reference);
                        return None;
                    }
                };
                let end = body.checked_add(length)?;
                if end > self.bytes.len() {
                    active.remove(&reference);
                    return None;
                }
                let mut tail = Lexer::new(self.bytes, end);
                tail.skip_space();
                if !tail.keyword(b"endstream")? {
                    active.remove(&reference);
                    return None;
                }
                tail.skip_space();
                if !tail.keyword(b"endobj")? {
                    active.remove(&reference);
                    return None;
                }
                stream_start = Some(body);
            } else if !lex.keyword(b"endobj")? {
                active.remove(&reference);
                return None;
            }
        } else if !lex.keyword(b"endobj")? {
            active.remove(&reference);
            return None;
        }
        active.remove(&reference);
        if matches!(dict_get(&value, "Type"), Some(Value::Name(name)) if name == "ObjStm") {
            return None;
        }
        let object = Rc::new(Object {
            value,
            stream_start,
        });
        self.objects.insert(reference, Rc::clone(&object));
        Some(object)
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_pages(
        &mut self,
        reference: Ref,
        parent: Option<Ref>,
        depth: usize,
        seen: &mut HashSet<Ref>,
        active: &mut HashSet<Ref>,
        owners: &mut HashMap<usize, Option<NonZeroU32>>,
        page_number: &mut u32,
        references: &mut usize,
    ) -> Option<u32> {
        if depth > MAX_DEPTH || !seen.insert(reference) || !active.insert(reference) {
            return None;
        }
        *references += 1;
        if *references > MAX_REFERENCES || self.references + *references > MAX_REFERENCES {
            return None;
        }
        let object = self.object(reference, &mut HashSet::new())?;
        let kind = match dict_get(&object.value, "Type")? {
            Value::Name(name) => name.as_str(),
            _ => return None,
        };
        let result = if kind == "Page" {
            if dict_get(&object.value, "Parent")? != &Value::Ref(parent?) {
                return None;
            }
            *page_number = page_number.checked_add(1)?;
            if *page_number as usize > MAX_PAGES {
                return None;
            }
            let page = NonZeroU32::new(*page_number)?;
            if let Some(contents) = dict_get(&object.value, "Contents") {
                let mut refs = Vec::new();
                collect_refs(contents, &mut refs)?;
                for content in refs {
                    *references += 1;
                    if *references > MAX_REFERENCES {
                        return None;
                    }
                    let stream = self.object(content, &mut HashSet::new())?;
                    let offset = stream.stream_start?;
                    match owners.get_mut(&offset) {
                        None => {
                            owners.insert(offset, Some(page));
                        }
                        Some(owner) if *owner == Some(page) => {}
                        Some(owner) => *owner = None,
                    }
                }
            }
            1
        } else if kind == "Pages" {
            if let Some(parent) = parent {
                if dict_get(&object.value, "Parent")? != &Value::Ref(parent) {
                    return None;
                }
            } else if object.value.as_dict()?.contains_key("Parent") {
                return None;
            }
            let expected = match dict_get(&object.value, "Count")? {
                Value::Number(n) if *n >= 0 => u32::try_from(*n).ok()?,
                _ => return None,
            };
            let kids = match dict_get(&object.value, "Kids")? {
                Value::Array(kids) => kids,
                _ => return None,
            };
            let mut actual = 0u32;
            for kid in kids {
                let kid = match kid {
                    Value::Ref(reference) => *reference,
                    _ => return None,
                };
                actual = actual.checked_add(self.walk_pages(
                    kid,
                    Some(reference),
                    depth + 1,
                    seen,
                    active,
                    owners,
                    page_number,
                    references,
                )?)?;
            }
            if expected != actual {
                return None;
            }
            actual
        } else {
            return None;
        };
        active.remove(&reference);
        Some(result)
    }
}

impl Value {
    fn as_dict(&self) -> Option<&HashMap<String, Value>> {
        if let Value::Dict(dict) = self {
            Some(dict)
        } else {
            None
        }
    }
}
fn dict_get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_dict()?.get(key)
}
fn collect_refs(value: &Value, refs: &mut Vec<Ref>) -> Option<()> {
    match value {
        Value::Ref(reference) => refs.push(*reference),
        Value::Array(values) => {
            for value in values {
                collect_refs(value, refs)?;
            }
        }
        _ => return None,
    }
    Some(())
}
fn rfind_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

struct Lexer<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Lexer<'a> {
    fn new(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }
    fn skip_space(&mut self) {
        loop {
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|b| b.is_ascii_whitespace() || *b == 0)
            {
                self.pos += 1;
            }
            if self.bytes.get(self.pos) != Some(&b'%') {
                break;
            }
            while self
                .bytes
                .get(self.pos)
                .is_some_and(|b| *b != b'\n' && *b != b'\r')
            {
                self.pos += 1;
            }
        }
    }
    fn keyword(&mut self, expected: &[u8]) -> Option<bool> {
        self.skip_space();
        if self.bytes.get(self.pos..self.pos + expected.len()) == Some(expected)
            && self.boundary(self.pos + expected.len())
        {
            self.pos += expected.len();
            Some(true)
        } else {
            Some(false)
        }
    }
    fn boundary(&self, pos: usize) -> bool {
        self.bytes.get(pos).is_none_or(|b| is_delimiter(*b))
    }
    fn word(&mut self) -> Option<&'a [u8]> {
        self.skip_space();
        let start = self.pos;
        while self.bytes.get(self.pos).is_some_and(|b| !is_delimiter(*b)) {
            self.pos += 1;
        }
        (self.pos > start).then_some(&self.bytes[start..self.pos])
    }
    fn integer(&mut self) -> Option<i64> {
        self.skip_space();
        let start = self.pos;
        if self
            .bytes
            .get(self.pos)
            .is_some_and(|b| *b == b'+' || *b == b'-')
        {
            self.pos += 1;
        }
        let digits = self.pos;
        while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
        if self.pos == digits {
            self.pos = start;
            return None;
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()?
            .parse()
            .ok()
    }
    fn value(&mut self, tokens: &mut usize, depth: usize) -> Option<Value> {
        *tokens += 1;
        if *tokens > MAX_TOKENS || depth > MAX_DEPTH {
            return None;
        }
        self.skip_space();
        match self.bytes.get(self.pos..self.pos + 2)? {
            b"<<" => self.dictionary(tokens, depth + 1),
            b"[ " | b"[\n" | b"[\r" => self.array(tokens, depth + 1),
            _ if self.bytes.get(self.pos) == Some(&b'[') => self.array(tokens, depth + 1),
            _ if self.bytes.get(self.pos) == Some(&b'/') => self.name(),
            _ if self.bytes.get(self.pos) == Some(&b'(') => {
                self.string()?;
                Some(Value::String)
            }
            _ if self.bytes.get(self.pos) == Some(&b'<') => {
                self.hex_string()?;
                Some(Value::String)
            }
            _ if self
                .bytes
                .get(self.pos)
                .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.')) =>
            {
                let token = self.word()?;
                let magnitude = if matches!(token.first(), Some(b'+' | b'-')) {
                    &token[1..]
                } else {
                    token
                };
                if magnitude.contains(&b'.') {
                    return (magnitude.iter().filter(|&&byte| byte == b'.').count() == 1
                        && magnitude.iter().any(u8::is_ascii_digit)
                        && magnitude
                            .iter()
                            .all(|byte| byte.is_ascii_digit() || *byte == b'.'))
                    .then_some(Value::Other);
                }
                let number = std::str::from_utf8(token).ok()?.parse::<i64>().ok()?;
                let after_first = self.pos;
                if number >= 0 {
                    if let Some(second) = self.integer() {
                        if second >= 0 && self.word() == Some(b"R") {
                            return Some(Value::Ref(Ref {
                                id: u32::try_from(number).ok()?,
                                generation: u16::try_from(second).ok()?,
                            }));
                        }
                    }
                }
                self.pos = after_first;
                Some(Value::Number(number))
            }
            _ => {
                let word = self.word()?;
                Some(match word {
                    b"true" | b"false" | b"null" => Value::Other,
                    _ => return None,
                })
            }
        }
    }
    fn dictionary(&mut self, tokens: &mut usize, depth: usize) -> Option<Value> {
        self.pos += 2;
        let mut dict = HashMap::new();
        loop {
            self.skip_space();
            if self.bytes.get(self.pos..self.pos + 2)? == b">>" {
                self.pos += 2;
                return Some(Value::Dict(dict));
            }
            let key = match self.name()? {
                Value::Name(name) => name,
                _ => unreachable!(),
            };
            let value = self.value(tokens, depth)?;
            if dict.insert(key, value).is_some() {
                return None;
            }
        }
    }
    fn array(&mut self, tokens: &mut usize, depth: usize) -> Option<Value> {
        self.pos += 1;
        let mut values = Vec::new();
        loop {
            self.skip_space();
            if self.bytes.get(self.pos) == Some(&b']') {
                self.pos += 1;
                return Some(Value::Array(values));
            }
            if values.len() >= MAX_REFERENCES {
                return None;
            }
            values.push(self.value(tokens, depth)?);
        }
    }
    fn name(&mut self) -> Option<Value> {
        self.pos += 1;
        let start = self.pos;
        while self.bytes.get(self.pos).is_some_and(|b| !is_delimiter(*b)) {
            self.pos += 1;
        }
        if self.pos == start || self.pos - start > 1024 {
            return None;
        }
        let name = String::from_utf8(self.bytes[start..self.pos].to_vec()).ok()?;
        Some(Value::Name(name))
    }
    fn string(&mut self) -> Option<()> {
        self.pos += 1;
        let mut nesting = 1usize;
        while let Some(&b) = self.bytes.get(self.pos) {
            self.pos += 1;
            match b {
                b'\\' => {
                    self.pos += usize::from(
                        self.bytes.get(self.pos) == Some(&b'\r')
                            && self.bytes.get(self.pos + 1) == Some(&b'\n'),
                    ) + 1;
                }
                b'(' => nesting += 1,
                b')' => {
                    nesting -= 1;
                    if nesting == 0 {
                        return Some(());
                    }
                }
                _ => {}
            }
        }
        None
    }
    fn hex_string(&mut self) -> Option<()> {
        self.pos += 1;
        while let Some(&b) = self.bytes.get(self.pos) {
            self.pos += 1;
            if b == b'>' {
                return Some(());
            }
        }
        None
    }
}
fn is_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_metadata_does_not_become_structural_integer() {
        for token in ["595.276", "841.89", "-.5", "4."] {
            let mut lexer = Lexer::new(token.as_bytes(), 0);
            assert_eq!(lexer.value(&mut 0, 0), Some(Value::Other), "{token}");
        }
        for token in ["2.5.6", ".", "12e3"] {
            let mut lexer = Lexer::new(token.as_bytes(), 0);
            assert_eq!(lexer.value(&mut 0, 0), None, "{token}");
        }
    }

    fn two_page_pdf() -> (Vec<u8>, usize, usize) {
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = HashMap::<u32, usize>::new();
        fn object(id: u32, body: &[u8], offsets: &mut HashMap<u32, usize>, out: &mut Vec<u8>) {
            offsets.insert(id, out.len());
            out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        object(
            1,
            b"<< /Type /Catalog /Pages 2 0 R >>",
            &mut offsets,
            &mut bytes,
        );
        object(
            2,
            b"<< /Type /Pages /Kids [5 0 R 3 0 R] /Count 2 >>",
            &mut offsets,
            &mut bytes,
        );
        let second_body = b"BT (PAGE TWO) Tj ET";
        object(
            3,
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
            &mut offsets,
            &mut bytes,
        );
        offsets.insert(4, bytes.len());
        bytes.extend_from_slice(
            format!("4 0 obj\n<< /Length {} >>\nstream\n", second_body.len()).as_bytes(),
        );
        let second = bytes.len();
        bytes.extend_from_slice(second_body);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let first_body = b"BT (PAGE ONE) Tj ET";
        object(
            5,
            b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>",
            &mut offsets,
            &mut bytes,
        );
        offsets.insert(6, bytes.len());
        bytes.extend_from_slice(
            format!("6 0 obj\n<< /Length {} >>\nstream\n", first_body.len()).as_bytes(),
        );
        let first = bytes.len();
        bytes.extend_from_slice(first_body);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for id in 1..=6 {
            bytes.extend_from_slice(format!("{:010} 00000 n \n", offsets[&id]).as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        (bytes, first, second)
    }

    #[test]
    fn maps_streams_by_page_tree_order_not_object_order() {
        let (pdf, first, second) = two_page_pdf();
        let map = page_by_stream_offset(&pdf).unwrap();
        assert_eq!(map.get(&first).unwrap().get(), 1);
        assert_eq!(map.get(&second).unwrap().get(), 2);
    }

    #[test]
    fn extracted_pdf_lines_keep_verified_page_provenance() {
        let (pdf, _, _) = two_page_pdf();
        let extracted =
            crate::extraction::extract_for_semantic_index(".pdf", &pdf, 10, false).unwrap();
        assert_eq!(extracted.lines, ["PAGE TWO", "PAGE ONE"]);
        assert_eq!(
            extracted.pages.unwrap(),
            [
                Some(NonZeroU32::new(2).unwrap()),
                Some(NonZeroU32::new(1).unwrap())
            ]
        );
    }

    #[test]
    fn real_multipage_pdf_keeps_line_page_alignment() {
        let pdf = include_bytes!("../benches/data/medium.pdf");
        let extracted = crate::extraction::extract_for_semantic_index(".pdf", pdf, 30, false)
            .expect("real PDF text extraction");
        let pages = extracted.pages.expect("classic-xref page map");
        assert_eq!(pages.len(), extracted.lines.len());
        assert!(pages.iter().flatten().all(|page| page.get() <= 28));
        assert!(pages
            .iter()
            .any(|page| page.is_some_and(|page| page.get() == 1)));
        assert!(pages
            .iter()
            .any(|page| page.is_some_and(|page| page.get() == 28)));
        let first_words = |page| {
            extracted
                .lines
                .iter()
                .zip(&pages)
                .filter(|(_, source)| source.is_some_and(|source| source.get() == page))
                .take(12)
                .map(|(line, _)| line.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(&first_words(1)[..3], ["The", "CWEB", "System of Structured Documentation"]);
        assert_eq!(&first_words(28)[..2], ["APPENDIX C | NOTES ON FORMATTING", "27"]);
    }

    #[test]
    fn real_large_pdf_keeps_bounded_page_numbers() {
        let pdf = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/benches/data/xlarge.pdf"
        ))
        .unwrap();
        // Generous timeout: a debug build needs ~14 s here, and a time-truncated
        // extraction (no page 144, no map) is what once looked like a mapper reject.
        let extracted = crate::extraction::extract_for_semantic_index(".pdf", &pdf, 300, false)
            .expect("large PDF text extraction");
        let pages = extracted.pages.expect("classic-xref page map");
        assert_eq!(pages.len(), extracted.lines.len());
        assert!(pages.iter().flatten().all(|page| page.get() <= 144));
        assert!(pages
            .iter()
            .any(|page| page.is_some_and(|page| page.get() == 1)));
        assert!(pages
            .iter()
            .any(|page| page.is_some_and(|page| page.get() == 144)));
    }

    #[test]
    fn refuses_incremental_or_malformed_documents() {
        let (mut pdf, _, _) = two_page_pdf();
        let xref = pdf.len() - 20;
        let _ = xref;
        let trailer = pdf
            .windows(b"/Size 7".len())
            .position(|w| w == b"/Size 7")
            .unwrap();
        pdf.splice(trailer..trailer, b"/Prev 1 ".iter().copied());
        assert!(page_by_stream_offset(&pdf).is_none());
        assert!(page_by_stream_offset(b"%PDF-1.4\nstartxref\nnot-an-offset\n%%EOF").is_none());
    }

    #[test]
    fn shared_streams_and_inconsistent_page_counts_do_not_get_citations() {
        let (mut shared, _, _) = two_page_pdf();
        let page_one = shared
            .windows(b"/Contents 6 0 R".len())
            .position(|window| window == b"/Contents 6 0 R")
            .unwrap();
        shared[page_one + b"/Contents ".len()] = b'4';
        assert!(page_by_stream_offset(&shared).unwrap().is_empty());

        let (mut bad_count, _, _) = two_page_pdf();
        let count = bad_count
            .windows(b"/Count 2".len())
            .position(|window| window == b"/Count 2")
            .unwrap();
        bad_count[count + b"/Count ".len()] = b'3';
        assert!(page_by_stream_offset(&bad_count).is_none());
    }
}

