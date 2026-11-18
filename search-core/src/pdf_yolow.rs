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

/// The cleaned lines and, when pages of the document were recognised by the bundled OCR, its mean character probability.
pub(crate) fn extract_lines_scored(bytes: &[u8], ocr: bool) -> Option<(Vec<String>, Option<f32>)> {
    if !enabled() {
        return None;
    }
    run_scored(bytes, ocr).map(|(sections, confidence)| (sections.into_iter().map(|(_, _, line)| line).collect(), confidence))
}

/// Like [`extract_lines`] but with the 1-based page of each line (the backend's own page split).
pub(crate) fn extract_paged_if_enabled(bytes: &[u8], ocr: bool) -> Option<Vec<(usize, String)>> {
    if enabled() { run(bytes, ocr) } else { None }
}

/// Cleaned lines with the 1-based page each came from; ignores the on/off switch (evaluation).
#[doc(hidden)]
pub fn extract_paged(bytes: &[u8], ocr: bool) -> Option<Vec<(usize, String)>> {
    run(bytes, ocr)
}

/// The bundled OCR (`ocr.rs`) as yolowpdf's page OCR engine: a page with no text layer inside a text document is
/// recognised there, so its lines carry that page's number. Without the `ocr` feature there is no engine.
#[cfg(feature = "ocr")]
#[derive(Debug, Default)]
struct BundledOcr {
    /// Probabilities of every character recognised through this engine during one document's extraction.
    score: std::sync::Mutex<crate::ocr::OcrScore>,
}

#[cfg(feature = "ocr")]
impl BundledOcr {
    fn mean(&self) -> Option<f32> {
        self.score.lock().ok().and_then(|s| s.mean())
    }
}

#[cfg(feature = "ocr")]
impl yolowpdf::PageOcr for BundledOcr {
    fn recognize(
        &self,
        width: u32,
        height: u32,
        gray: &[u8],
        _deadline: Option<std::time::Instant>,
    ) -> Result<yolowpdf::OcrOutput, String> {
        let boxes = crate::ocr::ocr_gray_lines(width, height, gray).ok_or("the bundled OCR failed on this image")?;
        if let Ok(mut total) = self.score.lock() {
            for line in &boxes {
                total.add(crate::ocr::OcrScore { p_sum: line.char_confidence.iter().sum(), chars: line.char_confidence.len() as u32 });
            }
        }
        Ok(yolowpdf::OcrOutput {
            lines: boxes
                .into_iter()
                .map(|b| yolowpdf::OcrLine {
                    x0: b.x0,
                    y0: b.y0,
                    x1: b.x1,
                    y1: b.y1,
                    // The model's mean character probability (vendored ocrs); yolowpdf 0.1.2 only carries it.
                    score: b.mean_confidence(),
                    text: b.text,
                    formula: false,
                })
                .collect(),
            ..Default::default()
        })
    }
}

fn run(bytes: &[u8], ocr: bool) -> Option<Vec<(usize, String)>> {
    run_sections(bytes, ocr).map(|v| v.into_iter().map(|(page, _, line)| (page, line)).collect())
}

/// [`extract_paged`] plus the nearest preceding heading of each line (evaluation of headings as chunk context).
#[doc(hidden)]
pub fn extract_paged_sections(bytes: &[u8], ocr: bool) -> Option<Vec<(usize, Option<String>, String)>> {
    run_sections(bytes, ocr)
}

fn run_sections(bytes: &[u8], ocr: bool) -> Option<Vec<(usize, Option<String>, String)>> {
    run_scored(bytes, ocr).map(|(sections, _)| sections)
}

type Sections = Vec<(usize, Option<String>, String)>;

/// [`run_sections`] plus the mean OCR character probability of the pages the bundled OCR recognised (`None` when none).
fn run_scored(bytes: &[u8], ocr: bool) -> Option<(Sections, Option<f32>)> {
    #[cfg_attr(not(feature = "ocr"), allow(unused_mut))]
    let mut opts = yolowpdf::Options {
        timeout: Some(OVERALL_TIMEOUT),
        ..Default::default()
    };
    #[cfg(feature = "ocr")]
    let bundled = std::sync::Arc::new(BundledOcr::default());
    #[cfg(feature = "ocr")]
    if ocr {
        opts.ocr = Some(bundled.clone());
    }
    #[cfg(not(feature = "ocr"))]
    let _ = ocr;
    let doc = catch_unwind(AssertUnwindSafe(|| yolowpdf::extract_with(bytes, &opts)))
        .ok()?
        .ok()?;
    if doc.scanned {
        return None;
    }
    // The last heading line ("#" in the backend's Markdown) before a line, across pages.
    let mut heading: Option<String> = None;
    let mut lines = Vec::new();
    for page in &doc.pages {
        for raw in page.markdown.lines() {
            let Some(line) = clean_line(raw) else { continue };
            if raw.trim_start().starts_with('#') {
                heading = Some(line.clone());
            }
            lines.push((page.number, heading.clone(), line));
        }
    }
    #[cfg(feature = "ocr")]
    let confidence = bundled.mean();
    #[cfg(not(feature = "ocr"))]
    let confidence = None;
    (!lines.is_empty()).then_some((lines, confidence))
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
        let lines = run(&bytes, false).expect("text layer");
        assert!(lines.iter().any(|(_, l)| l.contains("apple")), "{lines:?}");
    }

    #[test]
    fn paged_lines_carry_one_based_pages() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/test.pdf")).unwrap();
        let paged = extract_paged(&bytes, false).expect("text layer");
        assert!(!paged.is_empty() && paged.iter().all(|(p, _)| *p >= 1));
    }

    const MIXED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/test-data/mixed-text-scan.pdf");

    #[test]
    fn a_scanned_page_inside_a_text_pdf_has_no_text_without_ocr() {
        let bytes = std::fs::read(MIXED).unwrap();
        let paged = run(&bytes, false).expect("pages 1 and 2 have a text layer");
        assert!(paged.iter().all(|(p, _)| *p <= 2), "{paged:?}");
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn a_scanned_page_inside_a_text_pdf_is_recognised_with_its_page_number() {
        let bytes = std::fs::read(MIXED).unwrap();
        let paged = run(&bytes, true).expect("text");
        assert!(paged.iter().any(|(p, l)| *p == 1 && l.contains("apple")), "{paged:?}");
        let page2: String = paged.iter().filter(|(p, _)| *p == 3).map(|(_, l)| l.to_lowercase()).collect::<Vec<_>>().join(" ");
        for word in ["hydraulic", "relief", "valve", "torque", "ms21250"] {
            assert!(page2.contains(word), "{word} missing from page 3: {page2:?}");
        }
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn gray_ocr_boxes_lie_inside_the_image_and_follow_reading_order() {
        let jpeg = std::fs::read(MIXED).unwrap();
        let doc = yolowpdf::extract_with(&jpeg, &yolowpdf::Options { images: yolowpdf::ImageOptions { export: true, ..Default::default() }, ..Default::default() }).unwrap();
        let image = doc.images.iter().find(|i| i.data.is_some()).expect("the scanned page image");
        let decoded = image::load_from_memory(image.data.as_ref().unwrap()).unwrap().into_luma8();
        let (w, h) = decoded.dimensions();
        let boxes = crate::ocr::ocr_gray_lines(w, h, decoded.as_raw()).expect("ocr");
        assert!(boxes.len() >= 3, "{boxes:?}");
        assert!(boxes.iter().all(|b| b.x0 >= 0.0 && b.y0 >= 0.0 && b.x1 <= w as f32 && b.y1 <= h as f32 && b.x0 < b.x1 && b.y0 < b.y1), "{boxes:?}");
        assert!(boxes.windows(2).all(|p| p[0].y0 <= p[1].y0), "top to bottom: {boxes:?}");
    }

    /// Fixture: the rendered page inside `mixed-text-scan.pdf`. Every recognised character has a probability, a clean page
    /// reads with a high one, and the same page buried in noise reads with a clearly lower one (the vendored `ocrs`
    /// `TextChar::confidence`; `ocr_confidence_eval` measures how well that tracks errors).
    #[cfg(feature = "ocr")]
    #[test]
    fn recognition_confidence_is_high_on_a_clean_scan_and_falls_with_noise() {
        let jpeg = std::fs::read(MIXED).unwrap();
        let doc = yolowpdf::extract_with(&jpeg, &yolowpdf::Options { images: yolowpdf::ImageOptions { export: true, ..Default::default() }, ..Default::default() }).unwrap();
        let image = doc.images.iter().find(|i| i.data.is_some()).expect("the scanned page image");
        let decoded = image::load_from_memory(image.data.as_ref().unwrap()).unwrap().into_luma8();
        let (w, h) = decoded.dimensions();
        let weighted_mean = |lines: &[crate::ocr::OcrLine]| {
            let chars: usize = lines.iter().map(|l| l.char_confidence.len()).sum();
            assert!(chars > 0);
            lines.iter().map(|l| l.char_confidence.iter().sum::<f32>()).sum::<f32>() / chars as f32
        };
        let clean = crate::ocr::ocr_gray_lines(w, h, decoded.as_raw()).expect("ocr");
        for line in &clean {
            assert_eq!(line.char_confidence.len(), line.text.chars().count(), "{line:?}");
            assert!(line.char_confidence.iter().all(|p| *p > 0.0 && *p <= 1.0), "{line:?}");
        }
        // Deterministic noise: a linear congruential generator, +-60 grey levels on every pixel.
        let mut state = 12345u32;
        let noisy: Vec<u8> = decoded
            .as_raw()
            .iter()
            .map(|p| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (*p as i32 + ((state >> 24) as i32 - 128) * 60 / 128).clamp(0, 255) as u8
            })
            .collect();
        let noisy_lines = crate::ocr::ocr_gray_lines(w, h, &noisy).expect("ocr");
        let (clean_mean, noisy_mean) = (weighted_mean(&clean), if noisy_lines.is_empty() { 0.0 } else { weighted_mean(&noisy_lines) });
        assert!(clean_mean > 0.9, "clean {clean_mean}");
        assert!(noisy_mean < clean_mean - 0.02, "clean {clean_mean}, noisy {noisy_mean}");
    }
}
