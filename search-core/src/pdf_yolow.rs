//! Trial text-layer PDF backend (`pdf-yolow` feature, opt in with `SEARCH_PDF_YOLOW=1`).
//!
//! Measured on the KG-RAG corpus (docs/search-benchmarks.md, 2026-11-12) `yolowpdf`
//! recovers ~0.976 of poppler's words against ~0.866 for the byte scanner. It returns
//! Markdown per page, so lines are cleaned of table and heading markup here. `None`
//! means "use the scanner": backend off, parse error, panic, scanned document
//! (the scanner owns OCR) or no text.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

const OVERALL_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) fn enabled() -> bool {
    std::env::var_os("SEARCH_PDF_YOLOW").is_some_and(|v| v == "1")
}

pub(crate) fn extract_lines(bytes: &[u8]) -> Option<Vec<String>> {
    if !enabled() {
        return None;
    }
    let opts = yolowpdf::Options {
        timeout: Some(OVERALL_TIMEOUT),
        ..Default::default()
    };
    let doc = catch_unwind(AssertUnwindSafe(|| yolowpdf::extract_with(bytes, &opts)))
        .ok()?
        .ok()?;
    if doc.scanned {
        return None;
    }
    let lines: Vec<String> = doc
        .pages
        .iter()
        .flat_map(|page| page.markdown.lines())
        .filter_map(clean_line)
        .collect();
    (!lines.is_empty()).then_some(lines)
}

fn clean_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("![") || is_scanned_note(line) {
        return None;
    }
    let line = line.trim_start_matches('#').trim_start();
    let cleaned = line.replace('|', " ");
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    // Table rule rows ("--- ---") carry no text.
    (cleaned.chars().any(char::is_alphanumeric)).then_some(cleaned)
}

fn is_scanned_note(line: &str) -> bool {
    line.starts_with("*Page ") && line.ends_with("is a scanned image with no text layer.*")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_and_scan_notes_are_dropped() {
        assert_eq!(clean_line("## Heading"), Some("Heading".into()));
        assert_eq!(clean_line("| a | b |"), Some("a b".into()));
        assert_eq!(clean_line("| --- | --- |"), None);
        assert_eq!(clean_line("![](images/x.png)"), None);
        assert_eq!(
            clean_line("*Page 3 is a scanned image with no text layer.*"),
            None
        );
    }

    #[test]
    fn real_pdf_gives_lines_when_enabled_and_defers_when_not() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/test.pdf")).unwrap();
        assert!(extract_lines(&bytes).is_none(), "off unless SEARCH_PDF_YOLOW=1");
        // SAFETY: test-only env mutation; no other test reads this variable.
        unsafe { std::env::set_var("SEARCH_PDF_YOLOW", "1") };
        let lines = extract_lines(&bytes).expect("text layer");
        unsafe { std::env::remove_var("SEARCH_PDF_YOLOW") };
        assert!(lines.iter().any(|l| l.contains("apple")), "{lines:?}");
    }
}
