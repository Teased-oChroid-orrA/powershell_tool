//! Realistic heterogeneous corpus stress test (issue #8's Known Gap /
//! epic §22 - "mixed real-world proportions of TXT/LOG/DOCX/PPTX/RTF/PDF
//! at meaningful volume").
//!
//! This is deliberately distinct from the two benchmarks that already
//! exist and could be mistaken for covering this:
//!
//! - `orchestrator::tests::stress_test_100k_files`
//!   (`docs/issue-6-phase-14.md`) proves scale (100K files) but every
//!   file is `.txt` with a one-line synthetic body ("apple pie recipe" /
//!   "nothing relevant here") - single format, not realistic per-file
//!   sizes, no format mix at all.
//! - `search-core/benches/concurrent_extraction.rs`'s "Mixed format"
//!   scenario uses real per-format fixtures at real sizes (closing the
//!   "real sizes" half of the gap), but only 10 files - one of each
//!   format, equal weight - not realistic real-world *proportions*
//!   (most real folders are overwhelmingly .txt/.log, not one-seventh
//!   PDF) and not "meaningful volume."
//!
//! Neither combines realistic format *proportions* + meaningful *volume*
//! (thousands of files) + real per-format *sizes*, run through the real
//! `orchestrator::run` pipeline (not an isolated extraction call). This
//! test does, reusing the same real documents from
//! `search-core/benches/data/` (see that directory's README for
//! provenance) that `discovery_and_extraction.rs`/`concurrent_extraction.rs`
//! already established as this repo's real-size fixtures, duplicated to
//! realistic proportions rather than regenerated.
//!
//! `#[ignore]`d for the same reason `stress_test_100k_files` is: writing/
//! copying ~2,500 files (several hundred MB, since real DOCX/PPTX/XLSX/
//! PDF copies are genuinely megabyte-sized) on every `cargo test` run
//! would make the default suite slow and disk-heavy for a scale this
//! app's real use case only occasionally approaches. Run on demand:
//!
//! ```text
//! cargo test -p search-core --release -- --ignored realistic_mixed_corpus_reflects_real_world_proportions --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use search_core::models::SearchSettings;
use search_core::orchestrator;
use tokio_util::sync::CancellationToken;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/data")
}

/// A pool of ordinary, varied English sentences - not a single repeated
/// token - used to build realistic-looking `.txt`/`.log` bodies. Real
/// searched folders contain prose/log lines with real word-frequency
/// variety, not one string repeated a million times; using a rotating
/// pool (instead of e.g. `"lorem ipsum ".repeat(n)`) keeps per-file
/// extraction/matching work honest (the line-scanner has to look at
/// varied content, not exploit a single repeated pattern).
const SENTENCE_POOL: &[&str] = &[
    "The quarterly report was filed after the finance team reconciled every open invoice.",
    "Server logs showed intermittent latency spikes during the overnight batch window.",
    "Please review the attached contract and return your signed copy by Friday.",
    "The migration from the legacy database completed without any data loss.",
    "Customer support escalated three tickets related to the failed payment gateway.",
    "Our team completed the annual security audit ahead of the compliance deadline.",
    "The build pipeline failed twice before the flaky integration test was quarantined.",
    "Marketing requested updated assets for the upcoming product launch campaign.",
    "The warehouse inventory count did not match the numbers in the tracking system.",
    "Engineering proposed a new caching layer to reduce average response times.",
    "The onboarding documentation was rewritten to reflect the current workflow.",
    "A routine backup job ran successfully at two in the morning as scheduled.",
    "The client meeting was rescheduled twice due to conflicting travel plans.",
    "Load testing revealed a memory leak under sustained concurrent traffic.",
    "The design review surfaced several accessibility issues in the new layout.",
];

/// A pool of realistic log-line fragments (level + subsystem + message),
/// assembled with a synthetic-but-varied timestamp - closer to a real
/// application log than prose, since `.log` files in practice look
/// different from `.txt` files even though both extensions are treated
/// the same by extraction.
const LOG_LEVELS: &[&str] = &["INFO", "WARN", "ERROR", "DEBUG"];
const LOG_SUBSYSTEMS: &[&str] = &["auth", "db", "cache", "scheduler", "api", "worker"];
const LOG_MESSAGES: &[&str] = &[
    "connection established",
    "request completed in 42ms",
    "retrying after transient failure",
    "cache miss, falling back to source",
    "shutting down gracefully",
    "configuration reloaded",
    "rate limit threshold approached",
    "background job finished",
];

fn pseudo_rand(seed: usize, modulus: usize) -> usize {
    // Deterministic, dependency-free pseudo-randomness (same technique as
    // `regex_query_shapes_at_scale.rs`'s `pseudo_rand`) - reproducible
    // across runs without pulling in a `rand` dev-dependency for one test.
    (seed.wrapping_mul(2654435761).wrapping_add(0x9E3779B9)) % modulus.max(1)
}

/// Builds a realistic-sized `.txt` body: a few paragraphs of rotating
/// real sentences, sized in the hundreds-of-bytes-to-tens-of-KB range
/// typical logs/notes/text dumps actually are - not a single line.
/// Every `hit_every_nth`-th file gets a rare marker word injected so the
/// resulting hit count is exact and checkable, matching
/// `stress_test_100k_files`'s "exact count, not roughly right" standard.
fn txt_body(i: usize, is_hit: bool) -> String {
    let paragraph_count = 3 + pseudo_rand(i, 12); // 3-14 short paragraphs
    let mut body = String::new();
    for p in 0..paragraph_count {
        let sentence_count = 2 + pseudo_rand(i * 31 + p, 6); // 2-7 sentences/paragraph
        for s in 0..sentence_count {
            let idx = pseudo_rand(i * 97 + p * 13 + s, SENTENCE_POOL.len());
            body.push_str(SENTENCE_POOL[idx]);
            body.push(' ');
        }
        body.push('\n');
    }
    if is_hit {
        body.push_str("gizmoquark distinctive marker line for search verification\n");
    }
    body
}

fn log_body(i: usize, is_hit: bool) -> String {
    let line_count = 40 + pseudo_rand(i, 300); // 40-340 lines, realistic log-file range
    let mut body = String::new();
    for l in 0..line_count {
        let level = LOG_LEVELS[pseudo_rand(i * 7 + l, LOG_LEVELS.len())];
        let subsystem = LOG_SUBSYSTEMS[pseudo_rand(i * 11 + l, LOG_SUBSYSTEMS.len())];
        let message = LOG_MESSAGES[pseudo_rand(i * 13 + l, LOG_MESSAGES.len())];
        body.push_str(&format!("2026-08-{:02}T{:02}:{:02}:{:02}Z {level} [{subsystem}] {message}\n", 1 + (l % 28), l % 24, (l * 7) % 60, (l * 13) % 60));
    }
    if is_hit {
        body.push_str("2026-08-29T00:00:00Z ERROR [worker] gizmoquark distinctive marker line for search verification\n");
    }
    body
}

/// Copies `src` into `dest_dir` `count` times as `{stem}-{i}.{ext}` - real
/// document bytes each time (same technique as
/// `concurrent_extraction.rs::copy_n`), not a placeholder.
fn copy_n(src: &Path, dest_dir: &Path, count: usize) {
    if !src.exists() {
        return;
    }
    let stem = src.file_stem().unwrap().to_string_lossy().into_owned();
    let ext = src.extension().unwrap().to_string_lossy().into_owned();
    for i in 0..count {
        let dest = dest_dir.join(format!("{stem}-{i}.{ext}"));
        std::fs::copy(src, &dest).unwrap_or_else(|e| panic!("copy {} -> {}: {e}", src.display(), dest.display()));
    }
}

fn settings_for(dir: &Path, filters: &[&str]) -> SearchSettings {
    SearchSettings {
        search_path: dir.to_string_lossy().into_owned(),
        output_folder: dir.to_string_lossy().into_owned(),
        filters: filters.iter().map(|s| s.to_string()).collect(),
        parallel: true,
        throttle_limit: 8,
        ..SearchSettings::default()
    }
}

// Realistic real-world format proportions (issue #8 §22's own wording):
// heavily txt/log-dominated, a meaningful minority of office documents,
// and a small minority of RTF/PDF - the shape a genuine "search my
// documents folder" run actually has, not an equal-weight one-of-each
// folder. Total: 2,500 files.
const TXT_COUNT: usize = 1125; // 45%
const LOG_COUNT: usize = 625; // 25%  -> 70% plain text combined
const DOCX_MEDIUM: usize = 200;
const DOCX_LARGE: usize = 50; // 250 total, 10%
const PPTX_MEDIUM: usize = 160;
const PPTX_LARGE: usize = 40; // 200 total, 8%
const XLSX_MEDIUM: usize = 140;
const XLSX_LARGE: usize = 35; // 175 total, 7%
const RTF_MEDIUM: usize = 60;
const RTF_LARGE: usize = 15; // 75 total, 3%
const PDF_MEDIUM: usize = 40;
const PDF_LARGE: usize = 10; // 50 total, 2%
const HIT_EVERY_NTH: usize = 6;

#[tokio::test]
#[ignore]
async fn realistic_mixed_corpus_reflects_real_world_proportions() {
    let data = data_dir();
    if !data.join("medium.pdf").exists() {
        eprintln!("benches/data/ fixtures not found - run from search-core crate root after fetching them (see benches/data/README.md).");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let setup_start = Instant::now();

    let mut txt_expected_hits = 0usize;
    for i in 0..TXT_COUNT {
        let is_hit = i % HIT_EVERY_NTH == 0;
        if is_hit {
            txt_expected_hits += 1;
        }
        std::fs::write(dir.path().join(format!("note-{i}.txt")), txt_body(i, is_hit)).unwrap();
    }
    let mut log_expected_hits = 0usize;
    for i in 0..LOG_COUNT {
        let is_hit = i % HIT_EVERY_NTH == 0;
        if is_hit {
            log_expected_hits += 1;
        }
        std::fs::write(dir.path().join(format!("service-{i}.log")), log_body(i, is_hit)).unwrap();
    }

    copy_n(&data.join("medium.docx"), dir.path(), DOCX_MEDIUM);
    copy_n(&data.join("large.docx"), dir.path(), DOCX_LARGE);
    copy_n(&data.join("medium.pptx"), dir.path(), PPTX_MEDIUM);
    copy_n(&data.join("large.pptx"), dir.path(), PPTX_LARGE);
    copy_n(&data.join("medium.xlsx"), dir.path(), XLSX_MEDIUM);
    copy_n(&data.join("large.xlsx"), dir.path(), XLSX_LARGE);
    copy_n(&data.join("medium.rtf"), dir.path(), RTF_MEDIUM);
    copy_n(&data.join("large.rtf"), dir.path(), RTF_LARGE);
    copy_n(&data.join("medium.pdf"), dir.path(), PDF_MEDIUM);
    copy_n(&data.join("large.pdf"), dir.path(), PDF_LARGE);

    let total_files = TXT_COUNT + LOG_COUNT + DOCX_MEDIUM + DOCX_LARGE + PPTX_MEDIUM + PPTX_LARGE + XLSX_MEDIUM + XLSX_LARGE + RTF_MEDIUM + RTF_LARGE + PDF_MEDIUM + PDF_LARGE;
    let total_bytes: u64 = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();
    eprintln!(
        "realistic_mixed_corpus: wrote/copied {total_files} files ({:.1} MB, {} .txt + {} .log text-dominated, {} office/PDF minority) in {:.2}s",
        total_bytes as f64 / 1_000_000.0,
        TXT_COUNT,
        LOG_COUNT,
        DOCX_MEDIUM + DOCX_LARGE + PPTX_MEDIUM + PPTX_LARGE + XLSX_MEDIUM + XLSX_LARGE + RTF_MEDIUM + RTF_LARGE + PDF_MEDIUM + PDF_LARGE,
        setup_start.elapsed().as_secs_f64()
    );

    // "gizmoquark" is a marker word not present in any real fixture
    // document's actual text (verified: it isn't an English word),
    // so a hit against it should come *only* from the txt/log files we
    // deliberately injected it into - giving an exact, checkable hit
    // count through the real production pipeline, the same discipline
    // `stress_test_100k_files` applies.
    let settings = settings_for(dir.path(), &["gizmoquark"]);
    let run_start = Instant::now();
    let result = orchestrator::run(settings, None, CancellationToken::new()).await.unwrap();
    let elapsed = run_start.elapsed();

    let expected_hits = txt_expected_hits + log_expected_hits;
    let hit_count = result.file_results.iter().filter(|r| r.status == search_core::models::FileSearchStatus::Hit).count();

    eprintln!(
        "realistic_mixed_corpus: searched {} file(s) in {:.3}s ({:.0} files/sec), {} hit(s) (expected {}), {} read errors, {} unexpected errors",
        result.summary.files_searched,
        elapsed.as_secs_f64(),
        total_files as f64 / elapsed.as_secs_f64(),
        hit_count,
        expected_hits,
        result.summary.skipped_read_error,
        result.summary.skipped_unexpected_error,
    );

    assert_eq!(result.file_results.len(), total_files, "every file must be accounted for, none silently dropped");
    assert_eq!(hit_count, expected_hits, "exact hit count must survive at realistic mixed-corpus scale, not just 'roughly right'");
    assert_eq!(result.summary.skipped_unexpected_error, 0, "no format in a realistic mix should produce an unexpected (non-read) error");

    // A second run over the same real-proportioned corpus, searching a
    // common word ("the") that legitimately appears throughout the real
    // office/PDF fixtures too (not just the synthetic txt/log content) -
    // exercises the full pipeline's matching cost against realistic
    // hit-density, not just the sparse marker-word scenario above.
    let settings_common = settings_for(dir.path(), &["the"]);
    let run_start_common = Instant::now();
    let result_common = orchestrator::run(settings_common, None, CancellationToken::new()).await.unwrap();
    let elapsed_common = run_start_common.elapsed();
    let hit_count_common = result_common.file_results.iter().filter(|r| r.status == search_core::models::FileSearchStatus::Hit).count();
    eprintln!(
        "realistic_mixed_corpus (common word \"the\"): searched {} file(s) in {:.3}s ({:.0} files/sec), {} hit(s), {} read errors",
        result_common.summary.files_searched,
        elapsed_common.as_secs_f64(),
        total_files as f64 / elapsed_common.as_secs_f64(),
        hit_count_common,
        result_common.summary.skipped_read_error,
    );
    assert_eq!(result_common.file_results.len(), total_files);
}
