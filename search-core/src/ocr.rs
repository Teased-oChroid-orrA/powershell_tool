//! Optional OCR support for image-only/scanned PDFs - compiled in only
//! when this crate is built with the `ocr` Cargo feature (see
//! `Cargo.toml`'s `ocrs`/`rten` dependency comment for the full
//! evaluation/reasoning behind that specific library choice: pure-Rust
//! ONNX-model execution, no system runtime dependency, models embedded
//! into the binary rather than downloaded at runtime).
//!
//! This module owns machine-learning inference only - finding and
//! decoding the actual image bytes out of a PDF (JPEG via `/DCTDecode`,
//! or raw pixel data via a plain `/FlateDecode`) is `extraction.rs`'s job
//! (`find_jpeg_image_streams`/`find_raw_flate_image_streams`), matching
//! this crate's existing "PDF parsing knowledge lives in one place"
//! discipline - this module only ever sees already-decoded RGB8 pixel
//! buffers (`extraction::OcrCandidateImage`), never a PDF filter name.

use std::sync::OnceLock;

use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;

use crate::extraction::OcrCandidateImage;

/// The two model files `ocrs`' own CLI would otherwise download at first
/// run (`~/.cache/ocrs`) - embedded directly into the binary instead,
/// since this app's standing requirement is "no internet access on the
/// target machine, ever" (see CLAUDE.md's "Target environment"), not
/// just "no internet access most of the time." `Model::load_static_slice`
/// is `rten`'s own API for exactly this `include_bytes!` pattern - not a
/// workaround.
static DETECTION_MODEL_BYTES: &[u8] = include_bytes!("../assets/ocr/text-detection.rten");
static RECOGNITION_MODEL_BYTES: &[u8] = include_bytes!("../assets/ocr/text-recognition.rten");

/// Builds the OCR engine once per process (model loading has real,
/// non-trivial cost) and reuses it for every subsequent call. `None` if
/// the bundled model bytes ever fail to load - a defensive fallback that
/// should never actually trigger against the models this crate ships
/// (they're loaded from `include_bytes!`, not user input), but a
/// panic-free `None`/skip is still the right failure mode over an
/// `unwrap` if it somehow did.
fn engine() -> Option<&'static OcrEngine> {
    static ENGINE: OnceLock<Option<OcrEngine>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let detection_model = Model::load_static_slice(DETECTION_MODEL_BYTES).ok()?;
            let recognition_model = Model::load_static_slice(RECOGNITION_MODEL_BYTES).ok()?;
            OcrEngine::new(OcrEngineParams { detection_model: Some(detection_model), recognition_model: Some(recognition_model), ..Default::default() }).ok()
        })
        .as_ref()
}

/// Runs OCR on one already-decoded RGB8 image (typically one page image
/// `extraction::find_jpeg_image_streams`/`find_raw_flate_image_streams`
/// found in a scanned PDF) and returns its non-empty recognized lines.
/// `None` if the OCR engine itself couldn't be built at all (should not
/// happen against the bundled models - see `engine`'s doc comment) or if
/// this specific image failed to process; an image that processes fine
/// but has no recognizable text returns `Some(vec![])`, not `None` - a
/// blank/mostly-blank scanned page is a legitimate, non-exceptional
/// result, not a failure.
///
/// Deliberately one image at a time, not a batch - real full-page OCR
/// measured around 0.6-1s per page against a real scanned document (see
/// `docs/deployment-rust.md`'s OCR section for the actual numbers), so a
/// multi-page scanned PDF needs the caller to bound total work against
/// `extract_pdf_lines`'s own `overall_timeout_seconds` between images,
/// not after committing to OCR-ing every page in the file.
pub(crate) fn ocr_image(image: &OcrCandidateImage) -> Option<Vec<String>> {
    let engine = engine()?;
    let img_source = ImageSource::from_bytes(&image.rgb, (image.width, image.height)).ok()?;
    let ocr_input = engine.prepare_input(img_source).ok()?;
    let text = engine.get_text(&ocr_input).ok()?;
    Some(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}
