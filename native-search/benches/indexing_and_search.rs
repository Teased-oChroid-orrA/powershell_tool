//! Minimal benchmark/validation harness (issue #2 Section 13). Deliberately
//! not criterion or any other benchmarking crate - "a small benchmark
//! harness where practical" (Section 4), not a permanent perf-tracking
//! suite, matching Section 23's "don't overengineer" guidance. Plain manual
//! timing, `cargo bench` (harness = false, see Cargo.toml) or
//! `cargo run --release --bin bench_report` equivalent.
//!
//! IMPORTANT: numbers this prints are real, measured on whatever machine
//! runs it - never fabricated - but they characterize *this* machine, not
//! the win-x64 target hardware this app actually ships on. Treat them as
//! "does this look pathological" evidence, not a performance SLA. See
//! docs/benchmarking.md for the actual measured baseline and that caveat
//! stated again in context.

use std::path::Path;
use std::time::Instant;

use native_search::engine::{DocumentInput, NativeSearchEngine};

const DOC_COUNT: usize = 5_000;
const SEARCH_ITERATIONS: usize = 200;

const VOCAB: &[&str] = &[
    "torque",
    "spec",
    "deviation",
    "corrosion",
    "inspection",
    "aircraft",
    "engine",
    "mount",
    "bolt",
    "fastener",
    "airframe",
    "wing",
    "empennage",
    "fuselage",
    "workorder",
    "aog",
    "sustainment",
    "maintenance",
    "report",
    "finding",
    "structural",
    "panel",
    "rework",
    "schedule",
    "quarterly",
    "annual",
    "compliance",
    "directive",
    "bulletin",
    "revision",
];

/// Deterministic, varied synthetic body text - no `rand` dependency needed
/// (keeps this crate's dependency footprint unchanged just for a benchmark).
fn synthetic_body(doc_index: usize) -> String {
    let mut words = Vec::with_capacity(40);
    for i in 0..40 {
        let idx = (doc_index * 7 + i * 13 + i * i) % VOCAB.len();
        words.push(VOCAB[idx]);
    }
    words.join(" ")
}

fn main() {
    println!("native-search benchmark harness (issue #2 Section 13)");
    println!("Measured on THIS machine only - not the win-x64 target hardware.");
    println!("Corpus: {DOC_COUNT} synthetic documents, ~40 words each.\n");

    let dir = tempfile_dir();
    let engine = NativeSearchEngine::open_or_create(&dir).expect("open_or_create");

    let bodies: Vec<String> = (0..DOC_COUNT).map(synthetic_body).collect();
    let total_bytes: usize = bodies.iter().map(|b| b.len()).sum();

    let index_start = Instant::now();
    for (i, body) in bodies.iter().enumerate() {
        let id = i.to_string();
        engine
            .index_document(DocumentInput {
                id: &id,
                path: "C:\\bench\\synthetic.txt",
                filename: "synthetic.txt",
                extension: ".txt",
                title: "",
                modified_unix: 0,
                created_unix: 0,
                size: body.len() as i64,
                body,
            })
            .expect("index_document");
    }
    let index_elapsed = index_start.elapsed();

    let commit_start = Instant::now();
    engine.commit().expect("commit");
    let commit_elapsed = commit_start.elapsed();

    println!("Indexing:");
    println!(
        "  {:.0} docs/sec ({} docs in {:.3}s)",
        DOC_COUNT as f64 / index_elapsed.as_secs_f64(),
        DOC_COUNT,
        index_elapsed.as_secs_f64()
    );
    println!(
        "  {:.2} MB/sec ({:.2} MB total)",
        (total_bytes as f64 / 1_000_000.0) / index_elapsed.as_secs_f64(),
        total_bytes as f64 / 1_000_000.0
    );
    println!("  commit: {:.3}s\n", commit_elapsed.as_secs_f64());

    // Search latency: a mix of query shapes, matching Section 9's semantics
    // (single term, phrase, field filter) rather than only the easiest case.
    let queries = [
        "torque",
        "\"corrosion inspection\"",
        "extension:.txt",
        "aog OR workorder",
    ];
    for query in queries {
        let mut latencies_us: Vec<u128> = Vec::with_capacity(SEARCH_ITERATIONS);
        for _ in 0..SEARCH_ITERATIONS {
            let start = Instant::now();
            let hits = engine.search(query, 50, None).expect("search");
            latencies_us.push(start.elapsed().as_micros());
            std::hint::black_box(&hits);
        }
        latencies_us.sort_unstable();
        let median = latencies_us[latencies_us.len() / 2];
        let p95 = latencies_us[(latencies_us.len() as f64 * 0.95) as usize];
        println!(
            "Search {:>24}: median {:>5}us, p95 {:>5}us ({} iterations)",
            format!("{query:?}"),
            median,
            p95,
            SEARCH_ITERATIONS
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("native-search-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create bench dir");
    Path::new(&dir).to_path_buf()
}
