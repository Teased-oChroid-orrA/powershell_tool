//! Trial text-layer PDF backend (`pdf-yolow` feature, opt in per run with `SearchSettings::pdf_yolow`).
//!
//! Measured on the KG-RAG corpus (docs/search-benchmarks.md, 2026-11-12) `yolowpdf`
//! recovers ~0.976 of poppler's words against ~0.866 for the byte scanner. It returns
//! Markdown per page, so lines are cleaned of table and heading markup here. `None`
//! means "use the scanner": backend off, parse error, panic, scanned document
//! (the scanner owns OCR) or no text.

use std::sync::atomic::{AtomicBool, Ordering};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

const OVERALL_TIMEOUT: Duration = Duration::from_secs(300);

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Process-wide, like the OCR cache folder: set at the start of each run from the settings.
pub(crate) fn configure(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn extract_lines(bytes: &[u8]) -> Option<Vec<String>> {
    if enabled() {
        run(bytes).map(|paged| paged.into_iter().map(|(_, line)| line).collect())
    } else {
        None
    }
}

/// Like [`extract_lines`] but with the 1-based page of each line (the backend's own page split).
pub(crate) fn extract_paged_if_enabled(bytes: &[u8]) -> Option<Vec<(usize, String)>> {
    if enabled() { run(bytes) } else { None }
}

/// Cleaned lines with the 1-based page each came from; ignores the on/off switch (evaluation).
#[doc(hidden)]
pub fn extract_paged(bytes: &[u8]) -> Option<Vec<(usize, String)>> {
    run(bytes)
}

fn run(bytes: &[u8]) -> Option<Vec<(usize, String)>> {
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
    let lines: Vec<(usize, String)> = doc
        .pages
        .iter()
        .flat_map(|page| {
            page.markdown
                .lines()
                .filter_map(clean_line)
                .map(move |line| (page.number, line))
        })
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
    fn real_pdf_gives_text_lines() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/test.pdf")).unwrap();
        let lines = run(&bytes).expect("text layer");
        assert!(lines.iter().any(|(_, l)| l.contains("apple")), "{lines:?}");
    }

    #[test]
    fn paged_lines_carry_one_based_pages() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/test.pdf")).unwrap();
        let paged = extract_paged(&bytes).expect("text layer");
        assert!(!paged.is_empty() && paged.iter().all(|(p, _)| *p >= 1));
    }
}
