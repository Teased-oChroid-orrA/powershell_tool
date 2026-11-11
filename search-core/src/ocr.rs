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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

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

/// Longest image side handed to the recognizer (an A4 page at 300 dpi is 3508 px, so this is about 210 dpi).
/// Larger scans are shrunk first: on generated 600 dpi and noisy pages word recall rose from 0.974 to 0.997 and
/// from 0.966 to 0.981, and 150-300 dpi pages were unchanged (`search-core/AGENTS.md`).
const MAX_OCR_SIDE: u32 = 2480;

/// Dimensions after shrinking so the longest side is at most [`MAX_OCR_SIDE`] (unchanged when already smaller).
fn fit_dimensions(width: u32, height: u32) -> (u32, u32) {
    let big = width.max(height);
    if big <= MAX_OCR_SIDE {
        return (width, height);
    }
    let scale = MAX_OCR_SIDE as f64 / big as f64;
    (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1))
}

/// Runs OCR on one already-decoded RGB8 image (typically one page image
/// `extraction::jpeg_image_from_stream`/`raw_flate_image_from_stream`
/// decoded from a scanned PDF) and returns its non-empty recognized lines.
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
    let resized;
    let (nw, nh) = fit_dimensions(image.width, image.height);
    let (rgb, w, h): (&[u8], u32, u32) = if (nw, nh) != (image.width, image.height) {
        let src = image::ImageBuffer::<image::Rgb<u8>, &[u8]>::from_raw(image.width, image.height, &image.rgb[..])?;
        resized = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Triangle).into_raw();
        (&resized, nw, nh)
    } else {
        (&image.rgb, image.width, image.height)
    };
    let img_source = ImageSource::from_bytes(rgb, (w, h)).ok()?;
    let ocr_input = engine.prepare_input(img_source).ok()?;
    let text = engine.get_text(&ocr_input).ok()?;
    Some(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

// --------------------------------------------------------------------
// Recognised-text cache
// --------------------------------------------------------------------
//
// Recognition costs roughly a second per page, so the text of a page image is
// kept, keyed by a SHA-256 of the image's PDF stream (dictionary text plus
// encoded bytes, taken before any decoding so a hit also skips the JPEG or
// inflate work) and of the bundled models. It lives in memory for the process
// and, when a folder is configured, on disk as `<key>.txt` (one recognised line
// per line; an empty file is a page that had no text). Raw recognised lines are
// stored, not the line-quality-cleaned ones, so later changes to that filter
// still apply.

/// Bump when preprocessing or the way lines are produced changes.
const OCR_CACHE_VERSION: &str = "2";
/// Pages kept in memory; the map is cleared when it fills (cheap, bounded).
const MEMORY_MAX_PAGES: usize = 4096;

static CACHE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
static MEMORY: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);

/// Sets (or clears) the persistent cache folder. The folder is created on first write.
pub(crate) fn set_cache_dir(dir: Option<&str>) {
    let dir = dir.map(str::trim).filter(|d| !d.is_empty()).map(PathBuf::from);
    if let Ok(mut guard) = CACHE_DIR.lock() {
        *guard = dir;
    }
}

/// Cache key for one PDF image stream: hex SHA-256 over version, model sizes, header and body.
pub(crate) fn cache_key(header: &str, body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(OCR_CACHE_VERSION.as_bytes());
    hasher.update((DETECTION_MODEL_BYTES.len() as u64).to_le_bytes());
    hasher.update((RECOGNITION_MODEL_BYTES.len() as u64).to_le_bytes());
    hasher.update((header.len() as u64).to_le_bytes());
    hasher.update(header.as_bytes());
    hasher.update(body.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn cached_lines(key: &str) -> Option<Vec<String>> {
    if let Some(lines) = MEMORY.lock().ok()?.as_ref().and_then(|m| m.get(key)) {
        return Some(lines.clone());
    }
    let dir = CACHE_DIR.lock().ok()?.clone()?;
    let text = std::fs::read_to_string(dir.join(format!("{key}.txt"))).ok()?;
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    remember(key, &lines);
    Some(lines)
}

pub(crate) fn store_lines(key: &str, lines: &[String]) {
    remember(key, lines);
    let Some(dir) = CACHE_DIR.lock().ok().and_then(|d| d.clone()) else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    // Temp file then rename, so a crash never leaves a truncated page behind.
    let tmp = dir.join(format!("{key}.tmp"));
    if std::fs::write(&tmp, lines.join("\n")).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(format!("{key}.txt")));
    }
}

fn remember(key: &str, lines: &[String]) {
    if let Ok(mut guard) = MEMORY.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        if map.len() >= MEMORY_MAX_PAGES {
            map.clear();
        }
        map.insert(key.to_string(), lines.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_scans_are_shrunk_to_the_working_size_and_small_ones_left_alone() {
        assert_eq!(fit_dimensions(2479, 3509), (1752, 2480));
        assert_eq!(fit_dimensions(4961, 7016), (1754, 2480));
        assert_eq!(fit_dimensions(1240, 1754), (1240, 1754));
        assert_eq!(fit_dimensions(2480, 100), (2480, 100));
        assert_eq!(fit_dimensions(1, 100_000), (1, 2480));
    }

    #[test]
    fn page_text_round_trips_through_memory_and_disk() {
        let dir = tempfile::tempdir().unwrap();
        set_cache_dir(Some(dir.path().to_str().unwrap()));
        let key = cache_key("<< /Subtype /Image >>", "pixels one");
        assert_ne!(key, cache_key("<< /Subtype /Image >>", "pixels two"));
        assert!(cached_lines(&key).is_none());
        store_lines(&key, &["first line".into(), "second line".into()]);
        assert_eq!(cached_lines(&key).unwrap(), ["first line", "second line"]);
        // Disk alone is enough: drop the in-memory copy.
        MEMORY.lock().unwrap().take();
        assert_eq!(cached_lines(&key).unwrap(), ["first line", "second line"]);
        // A page with no text is a real, cached answer.
        let blank = cache_key("h", "blank page");
        store_lines(&blank, &[]);
        MEMORY.lock().unwrap().take();
        assert_eq!(cached_lines(&blank), Some(Vec::new()));
        set_cache_dir(None);
        MEMORY.lock().unwrap().take();
        assert!(cached_lines(&key).is_none());
    }
}
