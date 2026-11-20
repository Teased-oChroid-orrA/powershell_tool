use std::io::Write;

use super::*;
use crate::extraction::{extract_docx_lines, extract_pptx_lines};

/// A zip with the given parts.
fn zip_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        for (name, data) in parts {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

/// 1x1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

const DOC_RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/>
<Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.emf"/>
<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.com/x.png" TargetMode="External"/>
</Relationships>"#;

fn docx(body: &str) -> Vec<u8> {
    let xml = format!(r#"<?xml version="1.0"?><w:document xmlns:w="w" xmlns:a="a" xmlns:r="r"><w:body>{body}</w:body></w:document>"#);
    zip_of(&[("word/document.xml", xml.as_bytes()), ("word/_rels/document.xml.rels", DOC_RELS.as_bytes()), ("word/media/pic.png", PNG), ("word/media/pic.emf", b"emf")])
}

const BODY: &str = r#"
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Scope</w:t></w:r></w:p>
<w:p><w:r><w:t>Plain text</w:t></w:r><w:br/><w:r><w:t>second row of it</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>item</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B1</w:t></w:r></w:p><w:p><w:r><w:t>B1 second</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>A2</w:t></w:r></w:p></w:tc><w:tc><w:p/><w:p><w:r><w:t>B2</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>After the table</w:t></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="rId7"/></w:drawing></w:r><w:r><w:t>caption &amp; more</w:t></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="rId8"/></w:drawing></w:r></w:p>
<w:p><w:r><w:drawing><a:blip r:embed="rId9"/></w:drawing></w:r></w:p>
"#;

#[test]
fn docx_descriptors_line_up_with_the_extractor_and_name_headings_lists_and_cells() {
    let bytes = docx(BODY);
    let lines = extract_docx_lines(&bytes).unwrap();
    let st = docx_structure(&bytes).unwrap();
    assert_eq!(st.meta.len(), lines.len(), "one descriptor per extracted line");
    let at = |text: &str| lines.iter().position(|l| l.trim() == text).unwrap_or_else(|| panic!("no line {text:?} in {lines:?}"));
    assert_eq!(st.meta[at("Scope")], "h1");
    assert_eq!(st.meta[at("Plain text")], "");
    assert_eq!(st.meta[at("second row of it")], "", "a line break keeps the paragraph's own style");
    assert_eq!(st.meta[at("item")], "li");
    assert_eq!(st.meta[at("A1")], "c1.0.0");
    assert_eq!(st.meta[at("B1")], "c1.0.1");
    assert_eq!(st.meta[at("B1 second")], "c1.0.1");
    assert_eq!(st.meta[at("A2")], "c1.1.0");
    assert_eq!(st.meta[at("B2")], "c1.1.1");
    assert_eq!(st.meta[at("After the table")], "h2", "the table's cell context ends with the table");
    assert!(st.meta.iter().all(|m| !m.contains(['\u{E010}', '\u{E011}'])), "no marker leaks into a descriptor");
}

#[test]
fn docx_pictures_are_carried_once_and_only_when_safe_to_show() {
    let bytes = docx(BODY);
    let lines = extract_docx_lines(&bytes).unwrap();
    let st = docx_structure(&bytes).unwrap();
    assert_eq!(st.images.len(), 1, "the png only: emf cannot be drawn and the external one is never fetched");
    assert!(st.images[0].starts_with("data:image/png;base64,"));
    assert_eq!(st.refs.len(), 1);
    let caption = lines.iter().position(|l| l.trim() == "caption & more").unwrap() as u32 + 1;
    assert_eq!(st.refs[0], (caption, 0), "the picture sits on its paragraph's line");
}

#[test]
fn docx_text_is_untouched_by_the_markers_even_in_pretty_printed_xml() {
    let pretty = "<w:p>\n  <w:pPr>\n    <w:pStyle w:val=\"Title\"/>\n  </w:pPr>\n  <w:r><w:t>Name</w:t></w:r>\n</w:p>\n<w:p><w:r><w:t>x</w:t></w:r></w:p>";
    let bytes = docx(pretty);
    let lines = extract_docx_lines(&bytes).unwrap();
    let st = docx_structure(&bytes).unwrap();
    assert_eq!(st.meta.len(), lines.len());
    assert_eq!(st.meta[lines.iter().position(|l| l.trim() == "Name").unwrap()], "title");
}

#[test]
fn docx_that_already_uses_the_marker_range_gets_no_structure() {
    assert!(docx_structure(&docx("<w:p><w:r><w:t>a\u{E010}b</w:t></w:r></w:p>")).is_none());
    assert!(docx_structure(b"not a zip").is_none());
}

#[test]
fn nested_tables_stay_in_the_outer_cell() {
    let body = "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>outer</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>inner</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>back</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>next</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>out</w:t></w:r></w:p>";
    let bytes = docx(body);
    let lines = extract_docx_lines(&bytes).unwrap();
    let st = docx_structure(&bytes).unwrap();
    let at = |t: &str| st.meta[lines.iter().position(|l| l == t).unwrap()].clone();
    assert_eq!(at("outer"), "c1.0.0");
    assert_eq!(at("inner"), "c1.0.0");
    assert_eq!(at("back"), "c1.0.0");
    assert_eq!(at("next"), "c1.0.1");
    assert_eq!(at("out"), "");
}

fn slide(text: &str, pics: &[&str]) -> String {
    let blips: String = pics.iter().map(|r| format!(r#"<p:pic><a:blip r:embed="{r}"/></p:pic>"#)).collect();
    format!(r#"<p:sld xmlns:a="a" xmlns:p="p" xmlns:r="r"><p:txBody><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody>{blips}</p:sld>"#)
}

#[test]
fn pptx_pictures_attach_to_their_slide_line_and_a_shared_one_is_carried_once() {
    let rels = |ids: &[(&str, &str)]| {
        let items: String = ids.iter().map(|(i, t)| format!(r#"<Relationship Id="{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{t}"/>"#)).collect();
        format!("<Relationships>{items}</Relationships>")
    };
    let (s1, s2) = (slide("One", &["rId2"]), slide("Two", &["rId5", "rId6"]));
    let (r1, r2) = (rels(&[("rId2", "../media/logo.png")]), rels(&[("rId5", "../media/logo.png"), ("rId6", "../media/b.png")]));
    let bytes = zip_of(&[
        ("ppt/slides/slide1.xml", s1.as_bytes()),
        ("ppt/slides/slide2.xml", s2.as_bytes()),
        ("ppt/slides/_rels/slide1.xml.rels", r1.as_bytes()),
        ("ppt/slides/_rels/slide2.xml.rels", r2.as_bytes()),
        ("ppt/media/logo.png", PNG),
        ("ppt/media/b.png", PNG),
    ]);
    let lines = extract_pptx_lines(&bytes).unwrap();
    assert_eq!(lines, ["--- Slide 1 ---", "One", "--- Slide 2 ---", "Two"]);
    let st = pptx_structure(&bytes, &lines).unwrap();
    assert_eq!(st.images.len(), 2, "logo.png once, b.png once");
    assert_eq!(st.refs, vec![(1, 0), (3, 0), (3, 1)]);
}

#[test]
fn relationship_targets_resolve_against_their_folder() {
    assert_eq!(resolve("word/", "media/a.png"), "word/media/a.png");
    assert_eq!(resolve("ppt/slides/", "../media/a.png"), "ppt/media/a.png");
    assert_eq!(resolve("word/", "/word/media/a.png"), "word/media/a.png");
    assert_eq!(resolve("word/", "./media/../media/a.png"), "word/media/a.png");
}
