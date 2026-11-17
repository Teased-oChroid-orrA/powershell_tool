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

use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
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
pub(crate) fn ocr_image(image: &OcrCandidateImage) -> Option<OcrPage> {
    let resized;
    let (nw, nh) = fit_dimensions(image.width, image.height);
    let (rgb, w, h): (&[u8], u32, u32) = if (nw, nh) != (image.width, image.height) {
        let src = image::ImageBuffer::<image::Rgb<u8>, &[u8]>::from_raw(image.width, image.height, &image.rgb[..])?;
        resized = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Triangle).into_raw();
        (&resized, nw, nh)
    } else {
        (&image.rgb, image.width, image.height)
    };
    let lines = recognise(ImageSource::from_bytes(rgb, (w, h)).ok()?)?;
    Some(OcrPage::from_lines(&lines))
}

/// The recognised text of one page image and how sure the model was of it: the sum of the per-character probabilities
/// and the number of characters they cover (kept as a sum so pages combine into a document mean).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct OcrPage {
    pub lines: Vec<String>,
    pub score: OcrScore,
}

impl OcrPage {
    fn from_lines(lines: &[OcrLine]) -> OcrPage {
        let mut score = OcrScore::default();
        for line in lines {
            score.p_sum += line.char_confidence.iter().sum::<f32>();
            score.chars += line.char_confidence.len() as u32;
        }
        OcrPage { lines: lines.iter().map(|l| l.text.clone()).collect(), score }
    }
}

/// Character-weighted mean of the recogniser's per-character probabilities over some pages (0 characters: unknown).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct OcrScore {
    pub p_sum: f32,
    pub chars: u32,
}

impl OcrScore {
    pub fn add(&mut self, other: OcrScore) {
        self.p_sum += other.p_sum;
        self.chars += other.chars;
    }

    /// Mean probability per character, `None` when nothing was recognised.
    pub fn mean(&self) -> Option<f32> {
        (self.chars > 0).then(|| (self.p_sum / self.chars as f32).clamp(0.0, 1.0))
    }
}

/// One recognised line: its text, its box in the pixels of the image that was passed in (y down) and the model's
/// probability for each character of `text` (same order, one value per `char`).
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub text: String,
    pub char_confidence: Vec<f32>,
}

impl OcrLine {
    /// Mean probability over the line's characters (0 for an empty line).
    pub fn mean_confidence(&self) -> f32 {
        if self.char_confidence.is_empty() {
            return 0.0;
        }
        self.char_confidence.iter().sum::<f32>() / self.char_confidence.len() as f32
    }
}

/// Detects, groups and recognises the text of one prepared image: every non-empty line in reading order, with its
/// box in the image's own pixels. `None` when the engine or this image failed; no text is `Some(vec![])`.
fn recognise(source: ImageSource<'_>) -> Option<Vec<OcrLine>> {
    let engine = engine()?;
    let input = engine.prepare_input(source).ok()?;
    let words = engine.detect_words(&input).ok()?;
    let lines = engine.find_text_lines(&input, &words);
    let recognised = engine.recognize_text(&input, &lines).ok()?;
    let mut out = Vec::new();
    for (words, line) in lines.iter().zip(recognised) {
        let Some(line) = line else { continue };
        // Trim like the text always was, keeping one probability per remaining character.
        let chars: Vec<(char, f32)> = line.chars().iter().map(|c| (c.char, c.confidence)).collect();
        let first = chars.iter().position(|(c, _)| !c.is_whitespace());
        let last = chars.iter().rposition(|(c, _)| !c.is_whitespace());
        let (Some(first), Some(last)) = (first, last) else { continue };
        let kept = &chars[first..=last];
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for corner in words.iter().flat_map(|w| w.corners()) {
            x0 = x0.min(corner.x);
            y0 = y0.min(corner.y);
            x1 = x1.max(corner.x);
            y1 = y1.max(corner.y);
        }
        if x0 > x1 {
            continue;
        }
        out.push(OcrLine {
            x0,
            y0,
            x1,
            y1,
            text: kept.iter().map(|(c, _)| *c).collect(),
            char_confidence: kept.iter().map(|(_, p)| *p).collect(),
        });
    }
    Some(out)
}

/// OCR of one 8-bit grayscale image (0 black, 255 white), returning each non-empty line with its box and character
/// probabilities, in reading order. Shrunk to [`MAX_OCR_SIDE`] first like [`ocr_image`]; the boxes are scaled back to
/// the given image. `None` when the engine or this image failed; no text is `Some(vec![])`.
pub fn ocr_gray_lines(width: u32, height: u32, gray: &[u8]) -> Option<Vec<OcrLine>> {
    let (nw, nh) = fit_dimensions(width, height);
    let resized;
    let pixels: &[u8] = if (nw, nh) != (width, height) {
        let src = image::ImageBuffer::<image::Luma<u8>, &[u8]>::from_raw(width, height, gray)?;
        resized = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Triangle).into_raw();
        &resized
    } else {
        gray
    };
    let mut lines = recognise(ImageSource::from_bytes(pixels, (nw, nh)).ok()?)?;
    let (sx, sy) = (width as f32 / nw as f32, height as f32 / nh as f32);
    for line in &mut lines {
        line.x0 *= sx;
        line.x1 *= sx;
        line.y0 *= sy;
        line.y1 *= sy;
    }
    Some(lines)
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
const OCR_CACHE_VERSION: &str = "3";
/// Pages kept in memory; the map is cleared when it fills (cheap, bounded).
const MEMORY_MAX_PAGES: usize = 4096;

static CACHE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
static MEMORY: Mutex<Option<HashMap<String, OcrPage>>> = Mutex::new(None);

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

pub(crate) fn cached_page(key: &str) -> Option<OcrPage> {
    let dir = CACHE_DIR.lock().ok().and_then(|d| d.clone());
    read_page(dir.as_deref(), key)
}

pub(crate) fn store_page(key: &str, page: &OcrPage) {
    let dir = CACHE_DIR.lock().ok().and_then(|d| d.clone());
    write_page(dir.as_deref(), key, page);
}

/// File format: a first line `#ocr3 <sum of probabilities> <characters>`, then one recognised line per line.
const PAGE_HEADER: &str = "#ocr3 ";

/// Memory first, then `dir` (when there is one). Takes the folder as a parameter so tests need not touch the
/// process-wide setting that every search run reconfigures.
fn read_page(dir: Option<&std::path::Path>, key: &str) -> Option<OcrPage> {
    if let Some(page) = MEMORY.lock().ok()?.as_ref().and_then(|m| m.get(key)) {
        return Some(page.clone());
    }
    let text = std::fs::read_to_string(dir?.join(format!("{key}.txt"))).ok()?;
    let mut rows = text.lines();
    let mut head = rows.next()?.strip_prefix(PAGE_HEADER)?.split(' ');
    let score = OcrScore { p_sum: head.next()?.parse().ok()?, chars: head.next()?.parse().ok()? };
    let page = OcrPage { lines: rows.map(str::to_string).collect(), score };
    remember(key, &page);
    Some(page)
}

fn write_page(dir: Option<&std::path::Path>, key: &str, page: &OcrPage) {
    remember(key, page);
    let Some(dir) = dir else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    // Temp file then rename, so a crash never leaves a truncated page behind.
    let tmp = dir.join(format!("{key}.tmp"));
    let mut body = format!("{PAGE_HEADER}{} {}", page.score.p_sum, page.score.chars);
    for line in &page.lines {
        body.push('\n');
        body.push_str(line);
    }
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(format!("{key}.txt")));
    }
}

fn remember(key: &str, page: &OcrPage) {
    if let Ok(mut guard) = MEMORY.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        if map.len() >= MEMORY_MAX_PAGES {
            map.clear();
        }
        map.insert(key.to_string(), page.clone());
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

    fn page(lines: &[&str], p_sum: f32, chars: u32) -> OcrPage {
        OcrPage { lines: lines.iter().map(|l| l.to_string()).collect(), score: OcrScore { p_sum, chars } }
    }

    #[test]
    fn page_text_and_score_round_trip_through_memory_and_disk() {
        let dir = tempfile::tempdir().unwrap();
        let folder = Some(dir.path());
        let key = cache_key("<< /Subtype /Image >>", "pixels one");
        assert_ne!(key, cache_key("<< /Subtype /Image >>", "pixels two"));
        assert!(read_page(folder, &key).is_none());
        let first = page(&["first line", "second line"], 19.25, 20);
        write_page(folder, &key, &first);
        assert_eq!(read_page(folder, &key).unwrap(), first);
        // Disk alone is enough: drop the in-memory copy.
        MEMORY.lock().unwrap().as_mut().map(|m| m.remove(&key));
        assert_eq!(read_page(folder, &key).unwrap(), first);
        // A page with no text is a real, cached answer.
        let blank = cache_key("h", "blank page");
        write_page(folder, &blank, &OcrPage::default());
        MEMORY.lock().unwrap().as_mut().map(|m| m.remove(&blank));
        assert_eq!(read_page(folder, &blank), Some(OcrPage::default()));
        // Without a folder nothing survives the memory copy.
        MEMORY.lock().unwrap().as_mut().map(|m| m.remove(&key));
        std::fs::remove_file(dir.path().join(format!("{key}.txt"))).unwrap();
        MEMORY.lock().unwrap().as_mut().map(|m| m.remove(&key));
        assert!(read_page(folder, &key).is_none());
    }

    #[test]
    fn a_page_file_without_the_score_header_is_not_a_hit() {
        let dir = tempfile::tempdir().unwrap();
        let key = cache_key("h", "header-less page");
        std::fs::write(dir.path().join(format!("{key}.txt")), "text from an older cache\nsecond line").unwrap();
        assert!(read_page(Some(dir.path()), &key).is_none());
        std::fs::write(dir.path().join(format!("{key}.txt")), "#ocr3 not-a-number 4\nx").unwrap();
        assert!(read_page(Some(dir.path()), &key).is_none());
    }

    #[test]
    fn pages_combine_into_a_character_weighted_mean() {
        let mut total = OcrScore::default();
        assert_eq!(total.mean(), None);
        total.add(OcrScore { p_sum: 9.0, chars: 10 });
        total.add(OcrScore { p_sum: 27.0, chars: 30 });
        assert!((total.mean().unwrap() - 0.9).abs() < 1e-6);
        total.add(OcrScore { p_sum: 0.0, chars: 60 });
        assert!((total.mean().unwrap() - 0.36).abs() < 1e-6);
    }
}
