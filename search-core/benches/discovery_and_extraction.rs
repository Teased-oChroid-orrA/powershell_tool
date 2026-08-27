//! Minimal benchmark/validation harness for the two pipeline stages
//! `native-search/benches/indexing_and_search.rs` doesn't cover: directory
//! discovery (epic #6 §54 "Discovery") and text extraction (§54
//! "Extraction"). Same deliberate choices as that harness and
//! `docs/benchmarking.md` explains at length - plain manual timing
//! (`cargo bench`, `harness = false`), not criterion, not a permanent
//! perf-tracking suite. Read `docs/benchmarking.md`'s caveats section
//! before citing any number this prints anywhere - wrong hardware, small
//! corpus, synthetic content, all apply here exactly as they do there.
//!
//! **Methodology correction (issue #8 follow-up, 2026-08-26):** the
//! original per-format extraction section here read each file from disk
//! exactly once, then timed 500 repeated in-memory parse calls against
//! the same bytes - a pure parser-CPU microbenchmark, mislabeled at the
//! time as characterizing "extraction." It never exercised file I/O
//! after the first read, and only ran against 1-4KB correctness
//! fixtures, not realistic document sizes. Two functions now exist:
//! `bench_parse_only_extraction` (the original methodology, honestly
//! relabeled, kept because parser-CPU-only is still a real and useful
//! number) and `bench_full_pipeline_extraction` (new - real file I/O via
//! the actual production `read_file_bytes_robust` function, every
//! iteration, against real medium/large documents pulled from Apache
//! POI/Tika/PDFBox's own test-data corpora - see `benches/data/README.md`).

use std::path::Path;
use std::time::Instant;

use search_core::extraction::extract_lines_by_extension;
use search_core::file_reader::{enumerate_files_safely, read_file_bytes_robust};
use tokio_util::sync::CancellationToken;

const DISCOVERY_FILE_COUNT: usize = 5_000;
const DISCOVERY_DIRS: usize = 50;
const EXTRACTION_FILE_COUNT: usize = 2_000;

const VOCAB: &[&str] = &[
    "torque", "spec", "deviation", "corrosion", "inspection", "aircraft", "engine", "mount", "bolt", "fastener",
    "airframe", "wing", "empennage", "fuselage", "workorder", "aog", "sustainment", "maintenance", "report", "finding",
];

fn synthetic_body(doc_index: usize, word_count: usize) -> String {
    let mut words = Vec::with_capacity(word_count);
    for i in 0..word_count {
        let idx = (doc_index * 7 + i * 13 + i * i) % VOCAB.len();
        words.push(VOCAB[idx]);
    }
    words.join(" ")
}

fn bench_discovery(tmp_root: &Path) {
    let root = tmp_root.join("discovery");
    std::fs::create_dir_all(&root).unwrap();
    for d in 0..DISCOVERY_DIRS {
        let dir = root.join(format!("dir{d}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..(DISCOVERY_FILE_COUNT / DISCOVERY_DIRS) {
            std::fs::write(dir.join(format!("f{f}.txt")), b"x").unwrap();
        }
    }

    let cancellation = CancellationToken::new();
    let start = Instant::now();
    let (files, errors) = enumerate_files_safely(&root.to_string_lossy(), false, &[], &cancellation, None).unwrap();
    let elapsed = start.elapsed();

    println!("Discovery:");
    println!(
        "  {:.0} files/sec ({} files across {} dirs in {:.3}s, {} enumeration errors)",
        files.len() as f64 / elapsed.as_secs_f64(),
        files.len(),
        DISCOVERY_DIRS,
        elapsed.as_secs_f64(),
        errors
    );

    std::fs::remove_dir_all(&root).ok();
}

fn bench_extraction(_tmp_root: &Path) {
    // Plain-text extraction throughput - the fast path most files in a
    // real corpus actually take (.txt/.log dominate typical searched
    // folders far more than .docx/.pptx/.pdf do). Format-specific
    // extractor throughput (DOCX/PPTX/PDF) is exercised for correctness
    // by search-core/tests/fixtures.rs against real fixture files, but
    // isn't separately benchmarked here - those fixtures are a handful of
    // small files, not a volume large enough to produce a meaningful
    // throughput number, and generating a large synthetic corpus of valid
    // DOCX/PPTX/PDF bytes is significant extra machinery for a "does this
    // look pathological" sanity check, not proportionate to the ask.
    let bodies: Vec<String> = (0..EXTRACTION_FILE_COUNT).map(|i| synthetic_body(i, 200)).collect();
    let total_bytes: usize = bodies.iter().map(|b| b.len()).sum();

    let mut latencies_us: Vec<u128> = Vec::with_capacity(EXTRACTION_FILE_COUNT);
    let start = Instant::now();
    for body in &bodies {
        let t0 = Instant::now();
        let result = extract_lines_by_extension(".txt", body.as_bytes(), 30, None);
        latencies_us.push(t0.elapsed().as_micros());
        std::hint::black_box(&result);
    }
    let elapsed = start.elapsed();

    latencies_us.sort_unstable();
    let median = latencies_us[latencies_us.len() / 2];
    let p95 = latencies_us[(latencies_us.len() as f64 * 0.95) as usize];

    println!("\nExtraction (.txt path, {EXTRACTION_FILE_COUNT} files, ~200 words each):");
    println!(
        "  {:.0} files/sec, {:.2} MB/sec ({:.2} MB total in {:.3}s)",
        EXTRACTION_FILE_COUNT as f64 / elapsed.as_secs_f64(),
        (total_bytes as f64 / 1_000_000.0) / elapsed.as_secs_f64(),
        total_bytes as f64 / 1_000_000.0,
        elapsed.as_secs_f64()
    );
    println!("  latency: median {median}us, p95 {p95}us");
}

/// Fixture set for the per-format benchmarks below: 3 real size tiers per
/// format, not one. `tiny` reuses the same 1-4KB files
/// `search-core/tests/fixtures.rs` uses for correctness (so this can be
/// read directly as "how much slower does a realistic-size document get
/// vs. the tiny correctness fixture"); `medium`/`large` are real
/// documents (not synthetic filler) pulled from the Apache POI/Tika/
/// PDFBox projects' own test-data corpora specifically for parser
/// benchmarking - see `search-core/benches/data/README.md` for exact
/// provenance/license/original filenames.
fn format_fixtures() -> Vec<(&'static str, &'static str, std::path::PathBuf)> {
    let tiny_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/TextInFilesSearch.Tests/Fixtures");
    let sized_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/data");
    vec![
        (".docx", "tiny", tiny_dir.join("test.docx")),
        (".docx", "medium", sized_dir.join("medium.docx")),
        (".docx", "large", sized_dir.join("large.docx")),
        (".docx", "xlarge", sized_dir.join("xlarge.docx")),
        (".pptx", "tiny", tiny_dir.join("test.pptx")),
        (".pptx", "medium", sized_dir.join("medium.pptx")),
        (".pptx", "large", sized_dir.join("large.pptx")),
        // No real ~10MB+ PPTX found from a source this project treats as
        // legitimate (Apache POI's own test-data tops out at 2.28MB for
        // PPTX) - `large` (2.28MB) stays the biggest real tier for this
        // format. Documented as a real gap, not silently skipped.
        (".xlsx", "tiny", tiny_dir.join("test.xlsx")),
        (".xlsx", "medium", sized_dir.join("medium.xlsx")),
        (".xlsx", "large", sized_dir.join("large.xlsx")),
        // No real, representative ~10MB+ XLSX with genuine extractable
        // content was found - the only real ~10MB+ XLSX sourced
        // (`xlarge-recordheavy.xlsx`, Apache Tika's own
        // testRecordSizeExceeded.xlsx) is a deliberate pathological-
        // compression stress fixture whose single worksheet decompresses
        // to ~328MB, correctly rejected by the zip-bomb guard before any
        // real parse work happens - timing it here mostly measures the
        // guard's early-reject path, not representative extraction cost.
        // Included anyway so this stays an honest, labeled gap rather
        // than a silently-missing tier; see benches/data/README.md.
        (".xlsx", "xlarge (rejected by zip-bomb guard)", sized_dir.join("xlarge-recordheavy.xlsx")),
        (".rtf", "medium", sized_dir.join("medium.rtf")),
        (".rtf", "large", sized_dir.join("large.rtf")),
        // Same gap as PPTX - no real ~10MB+ RTF found; `large` (1.23MB)
        // is the biggest real tier available.
        (".pdf", "tiny", tiny_dir.join("test.pdf")),
        (".pdf", "medium", sized_dir.join("medium.pdf")),
        (".pdf", "large", sized_dir.join("large.pdf")),
        (".pdf", "xlarge", sized_dir.join("xlarge.pdf")),
        // The other real ~10MB+ PDF sourced, `xlarge-scanned.pdf`
        // (sample-files.com's large-doc.pdf, 38.6MB), is image-only
        // (scanned pages, no Tj/TJ text operators) - included as its own
        // tier so the benchmark also reports the cost of scanning a large
        // file that correctly extracts zero lines, not just the
        // real-text case above.
        (".pdf", "xlarge-scanned (image-only, no text)", sized_dir.join("xlarge-scanned.pdf")),
    ]
}

/// **In-memory parse cost only** (issue #8's methodology concern: this
/// reads each file from disk exactly ONCE before timing starts, then
/// times `FORMAT_ITERATIONS` repeated calls to `extract_lines_by_extension`
/// against the same already-in-memory `Vec<u8>` - it deliberately
/// excludes file I/O entirely, on every iteration after the first. This
/// isolates the parser/extractor's own CPU cost, but must never be read
/// as "the extraction pipeline's cost" - see `bench_full_pipeline_extraction`
/// below for the number that actually includes I/O, which is what issue
/// #8 asked for and this function alone does not provide.
const FORMAT_ITERATIONS: usize = 200;

fn bench_parse_only_extraction() {
    println!("\nParse-only extraction (in-memory, no file I/O after the initial read, {FORMAT_ITERATIONS} iterations each):");
    println!("  ** Isolates parser CPU cost. Does NOT include file I/O - see 'full pipeline' section below for that. **");
    for (ext, tier, path) in format_fixtures() {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                println!("  {ext:<6} {tier:<7} skipped - could not read {}: {e}", path.display());
                continue;
            }
        };

        let mut latencies_us: Vec<u128> = Vec::with_capacity(FORMAT_ITERATIONS);
        for _ in 0..FORMAT_ITERATIONS {
            let t0 = Instant::now();
            let result = extract_lines_by_extension(ext, &bytes, 30, None);
            latencies_us.push(t0.elapsed().as_micros());
            std::hint::black_box(&result);
        }
        latencies_us.sort_unstable();
        let median = latencies_us[latencies_us.len() / 2];
        let p95 = latencies_us[(latencies_us.len() as f64 * 0.95) as usize];
        println!("  {:<6} {:<7} {:>9} bytes, median {:>6}us, p95 {:>6}us", ext, tier, bytes.len(), median, p95);
    }
}

/// **Full pipeline: real file I/O + parse, every iteration** (the
/// measurement issue #8 actually asked for). Calls
/// `file_reader::read_file_bytes_robust` - the exact async function
/// `orchestrator::process_one_file` uses in production, not a bare
/// `std::fs::read` standing in for it - then `extract_lines_by_extension`,
/// timed together, for every one of `FULL_PIPELINE_ITERATIONS`
/// iterations. No bytes are cached/reused across iterations; each
/// iteration re-opens and re-reads the file from disk.
///
/// Reports the *first* iteration's latency separately from the median of
/// the remaining iterations - the closest honest proxy for a cold-vs-warm
/// OS page-cache distinction this benchmark can produce without
/// platform-specific cache-dropping privileges (there is no portable,
/// no-root way to force a true cold read on demand; `purge` on macOS and
/// `/proc/sys/vm/drop_caches` on Linux both need elevated privileges this
/// benchmark should not require to run). The first iteration is the only
/// one guaranteed not to already be sitting in the OS page cache from an
/// earlier run of this same benchmark or from the `file()`/download step
/// that fetched it - iterations 2+ are honestly reported as warm-cache
/// repeated reads, not claimed as cold.
const FULL_PIPELINE_ITERATIONS: usize = 50;

fn bench_full_pipeline_extraction() {
    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime for full-pipeline benchmark");
    println!(
        "\nFull pipeline extraction (real file I/O via file_reader::read_file_bytes_robust + parse, every iteration, {FULL_PIPELINE_ITERATIONS} iterations each):"
    );
    println!("  ** This is the number that includes I/O - the one issue #8 asked for. **");
    for (ext, tier, path) in format_fixtures() {
        if !path.exists() {
            println!("  {ext:<6} {tier:<7} skipped - {} does not exist", path.display());
            continue;
        }
        let path_str = path.to_string_lossy().into_owned();

        let mut latencies_us: Vec<u128> = Vec::with_capacity(FULL_PIPELINE_ITERATIONS);
        for _ in 0..FULL_PIPELINE_ITERATIONS {
            let cancellation = CancellationToken::new();
            let t0 = Instant::now();
            let bytes = rt
                .block_on(read_file_bytes_robust(&path_str, 30, 0, 0, None, &cancellation))
                .expect("read_file_bytes_robust");
            let result = extract_lines_by_extension(ext, &bytes, 30, None);
            latencies_us.push(t0.elapsed().as_micros());
            std::hint::black_box(&result);
        }

        let first_us = latencies_us[0];
        let mut rest = latencies_us[1..].to_vec();
        rest.sort_unstable();
        let warm_median = rest[rest.len() / 2];
        let warm_p95 = rest[(rest.len() as f64 * 0.95) as usize];
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        println!(
            "  {:<6} {:<7} {:>9} bytes, 1st-read {:>6}us, warm-reread median {:>6}us, p95 {:>6}us",
            ext, tier, file_size, first_us, warm_median, warm_p95
        );
    }
}

fn main() {
    println!("search-core discovery/extraction benchmark harness (issue #6 §54, issue #8 §2)");
    println!("Measured on THIS machine only - not the win-x64 target hardware. See docs/benchmarking.md.\n");

    let tmp_root = std::env::temp_dir().join(format!("search-core-bench-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_root).unwrap();

    bench_discovery(&tmp_root);
    bench_extraction(&tmp_root);
    bench_parse_only_extraction();
    bench_full_pipeline_extraction();

    std::fs::remove_dir_all(&tmp_root).ok();
}
