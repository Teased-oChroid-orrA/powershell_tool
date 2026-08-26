//! Measures issue #8 §7's exact ask: does the trigram candidate filter
//! (native-search's `trigram_candidate_paths`, the safe-superset pre-filter
//! `search-core::orchestrator` routes every non-regex, non-too-short
//! search through) materially reduce total search time, or is it
//! unjustified complexity? Same deliberate choices as this crate's other
//! benchmark (`indexing_and_search.rs`): plain manual timing, no
//! criterion, `cargo bench` / `harness = false`.
//!
//! Reports, per query selectivity tier: total docs, candidate docs after
//! the trigram filter, candidate percentage, and total time for
//! "candidate query + verify only the candidates" vs. "verify every
//! document" (the brute-force baseline the trigram filter exists to
//! avoid). Verification here is a literal `str::contains` check against
//! each document's in-memory body - a direct proxy for
//! `search-core::matching`'s literal-mode check, not a re-implementation
//! of it (this crate has no dependency on `search-core` to call the real
//! thing), but the same O(document length) substring-scan cost shape.
//!
//! IMPORTANT: numbers this prints characterize *this* machine, not the
//! win-x64 target hardware - see docs/benchmarking.md's standing caveat.

use std::path::Path;
use std::time::Instant;

use native_search::engine::{DocumentInput, NativeSearchEngine};

const DOC_COUNT: usize = 10_000;

const FILLER_VOCAB: &[&str] = &[
    "spec", "deviation", "aircraft", "mount", "bolt", "fastener", "airframe", "wing", "empennage", "fuselage",
    "workorder", "aog", "sustainment", "report", "finding", "structural", "panel", "rework", "schedule", "quarterly",
];

/// Builds a corpus with deliberately controlled, known term frequencies -
/// unlike `indexing_and_search.rs`'s corpus (every vocab word appears in
/// almost every doc by construction, which can't exercise selectivity
/// tiers at all). Every document gets filler text plus, per tier, an
/// embedded marker at the documented frequency:
///
/// - "the" (common): every document (~100%).
/// - "corrosion" (medium): every 5th document (~20%).
/// - "zqx9k7f2" (rare): exactly 5 of 10,000 documents (0.05%) - a marker
///   that doesn't collide with any real English trigram distribution.
fn synthetic_body(doc_index: usize) -> String {
    let mut words: Vec<&str> = Vec::with_capacity(40);
    words.push("the");
    if doc_index % 5 == 0 {
        words.push("corrosion");
    }
    if doc_index % 2000 == 0 {
        words.push("zqx9k7f2");
    }
    for i in 0..35 {
        let idx = (doc_index * 7 + i * 13 + i * i) % FILLER_VOCAB.len();
        words.push(FILLER_VOCAB[idx]);
    }
    words.join(" ")
}

struct Tier {
    label: &'static str,
    query: &'static str,
}

const TIERS: &[Tier] = &[
    Tier { label: "very common (~100% of docs)", query: "the" },
    Tier { label: "medium (~20% of docs)", query: "corrosion" },
    Tier { label: "rare (0.05% of docs)", query: "zqx9k7f2" },
    Tier { label: "short (below trigram threshold)", query: "ab" },
];

fn main() {
    println!("native-search trigram candidate-reduction benchmark (issue #8 §7)");
    println!("Measured on THIS machine only - not the win-x64 target hardware. See docs/benchmarking.md.");
    println!("Corpus: {DOC_COUNT} synthetic documents with controlled, known term frequencies.\n");

    let dir = tempfile_dir();
    let engine = NativeSearchEngine::open_or_create(&dir).expect("open_or_create");

    let bodies: Vec<String> = (0..DOC_COUNT).map(synthetic_body).collect();

    for (i, body) in bodies.iter().enumerate() {
        let id = i.to_string();
        engine
            .index_document(DocumentInput {
                id: &id,
                // Unique per doc (not a fixed placeholder path, unlike
                // indexing_and_search.rs's corpus) - trigram_candidate_paths
                // returns the `path` field, and this benchmark needs each
                // candidate mapped back to its specific body in `bodies`.
                path: &id,
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
    engine.commit().expect("commit");

    println!(
        "{:<34} {:>8} {:>10} {:>8} {:>14} {:>14} {:>10}",
        "Tier", "total", "candidates", "cand.%", "full-scan(us)", "narrowed(us)", "speedup"
    );
    for tier in TIERS {
        let candidate_query_start = Instant::now();
        let candidates = engine.trigram_candidate_paths(&[tier.query.to_string()]).expect("trigram_candidate_paths");
        let candidate_query_us = candidate_query_start.elapsed().as_micros();

        // Full-scan baseline: verify every document, exactly what running
        // without the trigram filter costs.
        let full_scan_start = Instant::now();
        let full_scan_hits = bodies.iter().filter(|b| b.contains(tier.query)).count();
        let full_scan_us = full_scan_start.elapsed().as_micros();

        let (candidate_count, narrowed_us, narrowed_hits) = match &candidates {
            Some(paths) => {
                let verify_start = Instant::now();
                // Candidate paths are just "1".."9999" (the doc ids used
                // above) - use them to index back into `bodies` for the
                // narrowed verification pass.
                let hits = paths
                    .iter()
                    .filter_map(|p| p.parse::<usize>().ok())
                    .filter(|&idx| bodies.get(idx).is_some_and(|b| b.contains(tier.query)))
                    .count();
                let verify_us = verify_start.elapsed().as_micros();
                (paths.len(), candidate_query_us + verify_us, hits)
            }
            None => {
                // No narrowing possible (too-short query) - the "narrowed"
                // path degrades to the same full scan, honestly reported
                // as such rather than a fabricated candidate count.
                (DOC_COUNT, full_scan_us, full_scan_hits)
            }
        };
        assert_eq!(full_scan_hits, narrowed_hits, "narrowed path must find exactly the same hits as the full scan");

        let candidate_pct = 100.0 * candidate_count as f64 / DOC_COUNT as f64;
        let speedup = full_scan_us as f64 / narrowed_us.max(1) as f64;
        println!(
            "{:<34} {:>8} {:>10} {:>7.1}% {:>14} {:>14} {:>9.1}x",
            format!("{:?} - {}", tier.query, tier.label),
            DOC_COUNT,
            candidate_count,
            candidate_pct,
            full_scan_us,
            narrowed_us,
            speedup
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("native-search-trigram-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create bench dir");
    Path::new(&dir).to_path_buf()
}
