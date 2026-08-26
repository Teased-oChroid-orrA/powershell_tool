//! Minimal benchmark/validation harness for the two pipeline stages
//! `native-search/benches/indexing_and_search.rs` doesn't cover: directory
//! discovery (epic #6 §54 "Discovery") and text extraction (§54
//! "Extraction"). Same deliberate choices as that harness and
//! `docs/benchmarking.md` explains at length - plain manual timing
//! (`cargo bench`, `harness = false`), not criterion, not a permanent
//! perf-tracking suite. Read `docs/benchmarking.md`'s caveats section
//! before citing any number this prints anywhere - wrong hardware, small
//! corpus, synthetic content, all apply here exactly as they do there.

use std::path::Path;
use std::time::Instant;

use search_core::extraction::extract_lines_by_extension;
use search_core::file_reader::enumerate_files_safely;
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

/// Per-format extraction timing (issue #8 §2 - "measure separately by
/// format: TXT/LOG/DOCX/PPTX/RTF/PDF"), against real fixture files
/// (`tests/TextInFilesSearch.Tests/Fixtures/`) rather than synthetic
/// bytes - DOCX/PPTX/PDF extraction is real container-format/regex
/// parsing, not reproducible with a fabricated byte string the way plain
/// text is. These fixtures are small (1-4 KB, the same ones
/// `search-core/tests/fixtures.rs` uses for correctness), so this reports
/// per-file latency at realistic small-document sizes, run many times for
/// a stable median/p95 - not a throughput number, since one tiny file
/// repeated isn't a volume claim.
const FORMAT_ITERATIONS: usize = 500;

fn bench_per_format_extraction() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/TextInFilesSearch.Tests/Fixtures");
    let files: &[(&str, &str)] =
        &[(".docx", "test.docx"), (".pptx", "test.pptx"), (".xlsx", "test.xlsx"), (".zip", "test.zip"), (".pdf", "test.pdf")];

    println!("\nPer-format extraction (real fixtures, {FORMAT_ITERATIONS} iterations each):");
    for (ext, filename) in files {
        let path = fixtures_dir.join(filename);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                println!("  {ext:<6} skipped - could not read {}: {e}", path.display());
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
        println!("  {:<6} {:>7} bytes on disk, median {:>5}us, p95 {:>5}us", ext, bytes.len(), median, p95);
    }
}

fn main() {
    println!("search-core discovery/extraction benchmark harness (issue #6 §54, issue #8 §2)");
    println!("Measured on THIS machine only - not the win-x64 target hardware. See docs/benchmarking.md.\n");

    let tmp_root = std::env::temp_dir().join(format!("search-core-bench-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_root).unwrap();

    bench_discovery(&tmp_root);
    bench_extraction(&tmp_root);
    bench_per_format_extraction();

    std::fs::remove_dir_all(&tmp_root).ok();
}
