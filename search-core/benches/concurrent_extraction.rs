//! Concurrent/mixed-corpus extraction benchmark (user follow-up to issue
//! #8's PDF-extraction finding): runs the real, production
//! `orchestrator::run` - not an isolated extraction-function call like
//! `discovery_and_extraction.rs`'s per-format section - over real folders
//! built from `search-core/benches/data/`'s real documents, both a
//! same-format-only folder and a mixed-format folder, at parallel
//! throttle limits matching this app's own defaults. Same deliberate
//! choices as this crate's other benchmarks: plain manual timing,
//! `cargo bench`, `harness = false`, no criterion. See
//! `docs/benchmarking.md` for the standing "wrong hardware" caveat and
//! for how these numbers relate to the single-file numbers in
//! `discovery_and_extraction.rs`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use search_core::models::SearchSettings;
use search_core::orchestrator;
use tokio_util::sync::CancellationToken;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/data")
}

/// Copies `src` into `dest_dir` `count` times as `{stem}-{i}{ext}`, e.g.
/// `medium-0.pdf`, `medium-1.pdf`, ... - real file content each time, not
/// a placeholder, so extraction cost is genuine for every copy.
fn copy_n(src: &Path, dest_dir: &Path, count: usize) {
    let stem = src.file_stem().unwrap().to_string_lossy().into_owned();
    let ext = src.extension().unwrap().to_string_lossy().into_owned();
    for i in 0..count {
        let dest = dest_dir.join(format!("{stem}-{i}.{ext}"));
        std::fs::copy(src, &dest).unwrap_or_else(|e| panic!("copy {} -> {}: {e}", src.display(), dest.display()));
    }
}

fn settings_for(dir: &Path, parallel: bool) -> SearchSettings {
    SearchSettings {
        search_path: dir.to_string_lossy().into_owned(),
        output_folder: dir.to_string_lossy().into_owned(),
        filters: vec!["the".to_string()],
        parallel,
        ..SearchSettings::default()
    }
}

async fn run_and_time(dir: &Path, parallel: bool) -> (std::time::Duration, i32, i32) {
    let settings = settings_for(dir, parallel);
    let start = Instant::now();
    let result = orchestrator::run(settings, None, CancellationToken::new()).await.expect("orchestrator::run");
    (start.elapsed(), result.summary.files_searched, result.summary.skipped_read_error)
}

fn print_scenario_result(label: &str, file_count: usize, total_bytes: u64, seq: (std::time::Duration, i32, i32), par: (std::time::Duration, i32, i32)) {
    let mb = total_bytes as f64 / 1_000_000.0;
    println!("\n{label}: {file_count} files, {mb:.1} MB total");
    println!(
        "  sequential: {:>7.1}ms ({} searched, {} read errors)",
        seq.0.as_secs_f64() * 1000.0,
        seq.1,
        seq.2
    );
    println!(
        "  parallel:   {:>7.1}ms ({} searched, {} read errors) - {:.1}x vs. sequential",
        par.0.as_secs_f64() * 1000.0,
        par.1,
        par.2,
        seq.0.as_secs_f64() / par.0.as_secs_f64().max(0.0001)
    );
}

fn dir_total_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

fn main() {
    println!("search-core concurrent/mixed-corpus extraction benchmark");
    println!("Measured on THIS machine only - not the win-x64 target hardware. See docs/benchmarking.md.");

    let data = data_dir();
    if !data.join("medium.pdf").exists() {
        eprintln!("benches/data/ fixtures not found - run this from the search-core crate root after fetching them.");
        return;
    }

    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");
    let tmp_root = std::env::temp_dir().join(format!("search-core-concurrent-bench-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_root).unwrap();

    // ---- Scenario 1: same-type-only folder (PDF - the format issue #8's
    // fix targeted) - several medium + large copies, no xlarge (kept
    // separate below since one 38.6MB PDF alone already takes hundreds of
    // ms; mixing tiers here keeps this scenario's total runtime sane).
    {
        let dir = tmp_root.join("same-type-pdf");
        std::fs::create_dir_all(&dir).unwrap();
        copy_n(&data.join("medium.pdf"), &dir, 6);
        copy_n(&data.join("large.pdf"), &dir, 4);
        let file_count = std::fs::read_dir(&dir).unwrap().count();
        let total_bytes = dir_total_bytes(&dir);

        let seq = rt.block_on(run_and_time(&dir, false));
        let par = rt.block_on(run_and_time(&dir, true));
        print_scenario_result("Same-type (PDF only, 6x medium + 4x large)", file_count, total_bytes, seq, par);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- Scenario 2: same-type-only folder, a different format (XLSX)
    // for comparison - proves the concurrency behavior isn't PDF-specific.
    {
        let dir = tmp_root.join("same-type-xlsx");
        std::fs::create_dir_all(&dir).unwrap();
        copy_n(&data.join("medium.xlsx"), &dir, 6);
        copy_n(&data.join("large.xlsx"), &dir, 4);
        let file_count = std::fs::read_dir(&dir).unwrap().count();
        let total_bytes = dir_total_bytes(&dir);

        let seq = rt.block_on(run_and_time(&dir, false));
        let par = rt.block_on(run_and_time(&dir, true));
        print_scenario_result("Same-type (XLSX only, 6x medium + 4x large)", file_count, total_bytes, seq, par);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- Scenario 3: mixed-format folder - one of every real fixture
    // this benchmark has, all formats together, the shape of a real
    // user's folder far more than any single-format scenario is.
    {
        let dir = tmp_root.join("mixed-format");
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["medium.docx", "large.docx", "medium.pptx", "large.pptx", "medium.xlsx", "large.xlsx", "medium.rtf", "large.rtf", "medium.pdf", "large.pdf"] {
            let src = data.join(name);
            if src.exists() {
                std::fs::copy(&src, dir.join(name)).unwrap();
            }
        }
        let file_count = std::fs::read_dir(&dir).unwrap().count();
        let total_bytes = dir_total_bytes(&dir);

        let seq = rt.block_on(run_and_time(&dir, false));
        let par = rt.block_on(run_and_time(&dir, true));
        print_scenario_result("Mixed format (one of each: docx/pptx/xlsx/rtf/pdf, medium+large)", file_count, total_bytes, seq, par);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- Scenario 4: concurrent xlarge (~10MB+) files, mixed formats -
    // the worst-case realistic scenario this whole investigation started
    // from: several genuinely large, genuinely expensive-to-extract real
    // documents competing for the heavy-resource-class throttle at once.
    // `xlarge.docx`/`xlarge.pdf` are real, representative documents with
    // genuine extractable text. `xlarge-scanned.pdf` (image-only, no Tj/TJ
    // operators - no OCR in this extractor) and `xlarge-recordheavy.xlsx`
    // (Tika's testRecordSizeExceeded.xlsx, decompresses to ~328MB) are
    // deliberately-included pathological edge cases: both correctly
    // extract zero text (no-OCR limitation / zip-bomb guard respectively,
    // not bugs, not caused by this session's find_stream_blocks fix - see
    // benches/data/README.md) and are here specifically to prove the
    // orchestrator handles real-world pathological files gracefully under
    // concurrency (no crash/hang), not to measure representative
    // extraction throughput.
    {
        let dir = tmp_root.join("xlarge-mixed");
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["xlarge.docx", "xlarge.pdf", "xlarge-scanned.pdf", "xlarge-recordheavy.xlsx"] {
            let src = data.join(name);
            if src.exists() {
                std::fs::copy(&src, dir.join(name)).unwrap();
            }
        }
        // Duplicate the real-text PDF (the format this investigation's
        // fix targeted) so there's real concurrent contention on it
        // specifically, not just one of each format.
        if data.join("xlarge.pdf").exists() {
            std::fs::copy(data.join("xlarge.pdf"), dir.join("xlarge-pdf-2.pdf")).unwrap();
        }
        let file_count = std::fs::read_dir(&dir).unwrap().count();
        let total_bytes = dir_total_bytes(&dir);

        let seq = rt.block_on(run_and_time(&dir, false));
        let par = rt.block_on(run_and_time(&dir, true));
        print_scenario_result(
            "~10MB+ files, mixed format (2x xlarge.pdf real-text + xlarge.docx + xlarge-scanned.pdf[no text, expected] + xlarge-recordheavy.xlsx[rejected, expected])",
            file_count,
            total_bytes,
            seq,
            par,
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    std::fs::remove_dir_all(&tmp_root).ok();
}
