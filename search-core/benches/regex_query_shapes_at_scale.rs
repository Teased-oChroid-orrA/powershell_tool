//! Issue #9 task: does "Level 3" indexed regex execution (regex AST →
//! automaton → positional index execution → cost-based planner) have room
//! to help at the user's real reported scale - thousands of folders,
//! 100,000+ files - without regressing current speed? Per the epic's own
//! §42 ("Claude Code MUST establish a current baseline... report actual
//! results... the architecture should be accepted based on measured
//! end-to-end improvement"), this benchmark exists to answer that
//! *before* any automaton/positional-index engineering is attempted, not
//! after.
//!
//! Runs the epic's own §44 benchmark-matrix query shapes (simple/rare/
//! common/long literal, no-match, `foo.*bar`, `foo.{0,5}bar`,
//! `(foo|bar)baz`, anchored regex, regex-with-no-useful-literal) against a
//! real `orchestrator::run` over a real ~110,000-file, 2,000-directory
//! corpus - not a toy fixture, not an isolated function call. Same
//! deliberate choices as this crate's other benchmarks: plain manual
//! timing, `cargo bench`, `harness = false`, no criterion.
//!
//! Each query shape is labeled with whether the *existing* architecture
//! (trigram candidate filter + `regex_literals::required_literal_chunks`
//! mandatory-literal extraction, including this session's bounded-
//! quantifier extension) narrows it before the full per-line regex scan,
//! or falls back to a full scan - so the printed numbers double as a
//! direct measurement of what a hypothetical Level 3 engine could
//! possibly improve on (the full-scan rows) versus what's already solved
//! (the narrowed rows).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use search_core::models::SearchSettings;
use search_core::orchestrator;
use tokio_util::sync::CancellationToken;

const DIR_COUNT: usize = 2_000;
const FILES_PER_DIR: usize = 55; // 110,000 files total
const LINES_PER_FILE: usize = 40;

const VOCAB: &[&str] = &[
    "torque", "spec", "deviation", "corrosion", "inspection", "aircraft", "engine", "mount", "bolt", "fastener",
    "airframe", "wing", "empennage", "fuselage", "workorder", "aog", "sustainment", "the", "and", "was", "with",
    "system", "component", "assembly", "panel", "surface", "check", "report", "field", "unit",
];

/// Deterministic pseudo-randomness without a `rand` dependency - same
/// choice this crate's other benchmarks already made (see
/// `native-search/benches/indexing_and_search.rs`'s own VOCAB comment).
fn pseudo_rand(seed: usize) -> usize {
    seed.wrapping_mul(2_654_435_761).wrapping_add(0x9E3779B9)
}

/// Builds one file's body. `i` is the file's global index (0..FILE_COUNT).
/// Deliberately embeds specific substrings in a *known fraction* of files
/// so every query shape below has real, non-trivial candidate/verification
/// work to do - not just "scan everything, find nothing."
fn file_body(i: usize) -> String {
    let mut lines = Vec::with_capacity(LINES_PER_FILE);
    for l in 0..LINES_PER_FILE {
        let mut words = Vec::with_capacity(8);
        for w in 0..8 {
            let idx = pseudo_rand(i * 97 + l * 13 + w) % VOCAB.len();
            words.push(VOCAB[idx]);
        }
        let mut line = words.join(" ");

        // "apple" - rare literal, ~1 in 5000 files, one occurrence.
        if i % 5000 == 0 && l == 3 {
            line = format!("{line} apple");
        }
        // A unique long literal, ~1 in 1000 files.
        if i % 1000 == 0 && l == 5 {
            line = format!("{line} unique-long-literal-marker-zz9x7");
        }
        // "start"..."finish" with variable-length filler between, for
        // `start.*bar`-equivalent - ~1 in 200 files.
        if i % 200 == 0 && l == 10 {
            line = format!("start {line} finish");
        }
        // "mid" then 0-5 chars then "point", for `mid.{{0,5}}point` -
        // ~1 in 150 files.
        if i % 150 == 0 && l == 15 {
            let gap = "x".repeat(i % 6);
            line = format!("mid{gap}point {line}");
        }
        // "redflag" or "blueflag" - the one gap in current architecture
        // (alternation isn't decomposed by `required_literal_chunks`,
        // which bails the instant it sees `|`) - ~1 in 100 files.
        if i % 100 == 0 && l == 20 {
            let word = if i % 200 == 0 { "redflag" } else { "blueflag" };
            line = format!("{line} {word}");
        }
        // A line starting with "SECTION", for the anchored-regex case -
        // ~1 in 300 files.
        if i % 300 == 0 && l == 0 {
            line = format!("SECTION {line}");
        }
        lines.push(line);
    }
    lines.join("\n") + "\n"
}

fn build_corpus(root: &Path) -> usize {
    let mut i = 0usize;
    for d in 0..DIR_COUNT {
        let sub = root.join(format!("d{d}"));
        std::fs::create_dir_all(&sub).unwrap();
        for f in 0..FILES_PER_DIR {
            std::fs::write(sub.join(format!("f{f}.txt")), file_body(i)).unwrap();
            i += 1;
        }
    }
    i
}

struct QueryShape {
    label: &'static str,
    filter: &'static str,
    use_regex: bool,
    /// Whether `regex_literals::required_literal_chunks` (the existing
    /// mandatory-literal narrowing) is expected to narrow this pattern -
    /// stated up front so the printed table is self-documenting about
    /// which rows are "already solved" vs. "what Level 3 could touch."
    currently_narrowed: bool,
}

const QUERY_SHAPES: &[QueryShape] = &[
    QueryShape { label: "simple literal", filter: "engine", use_regex: false, currently_narrowed: true },
    QueryShape { label: "rare literal", filter: "apple", use_regex: false, currently_narrowed: true },
    QueryShape { label: "common literal", filter: "the", use_regex: false, currently_narrowed: true },
    QueryShape { label: "long literal", filter: "unique-long-literal-marker-zz9x7", use_regex: false, currently_narrowed: true },
    QueryShape { label: "no match", filter: "zzznomatchzzz", use_regex: false, currently_narrowed: true },
    QueryShape { label: r"foo.*bar  (start.*finish)", filter: "start.*finish", use_regex: true, currently_narrowed: true },
    QueryShape { label: r"foo.{0,5}bar  (mid.{0,5}point)", filter: r"mid.{0,5}point", use_regex: true, currently_narrowed: true },
    QueryShape { label: r"(foo|bar)baz  ((red|blue)flag)", filter: "(red|blue)flag", use_regex: true, currently_narrowed: false },
    QueryShape { label: "anchored regex  (^SECTION)", filter: "^SECTION", use_regex: true, currently_narrowed: true },
    QueryShape { label: "regex, no useful literal  (.{10,20})", filter: ".{10,20}", use_regex: true, currently_narrowed: false },
];

fn settings_for(dir: &Path, shape: &QueryShape) -> SearchSettings {
    SearchSettings {
        search_path: dir.to_string_lossy().into_owned(),
        output_folder: dir.to_string_lossy().into_owned(),
        filters: vec![shape.filter.to_string()],
        use_regex: shape.use_regex,
        parallel: true,
        throttle_limit: 8,
        ..SearchSettings::default()
    }
}

async fn run_and_time(dir: &Path, shape: &QueryShape) -> (Duration, i32, usize) {
    let settings = settings_for(dir, shape);
    let start = Instant::now();
    let result = orchestrator::run(settings, None, CancellationToken::new()).await.expect("orchestrator::run");
    let hit_count = result.file_results.iter().filter(|r| r.status == search_core::models::FileSearchStatus::Hit).count();
    (start.elapsed(), result.summary.files_searched, hit_count)
}

fn main() {
    println!("search-core regex query-shape benchmark at scale (issue #9 task: Level 3 justification check)");
    println!("Measured on THIS machine only - not the win-x64 target hardware. See docs/benchmarking.md.");
    println!("Corpus: {DIR_COUNT} directories, {FILES_PER_DIR} files/dir, {LINES_PER_FILE} lines/file.");

    let tmp_root: PathBuf = std::env::temp_dir().join(format!("search-core-regex-scale-bench-{}", std::process::id()));
    std::fs::create_dir_all(&tmp_root).unwrap();

    let setup_start = Instant::now();
    let file_count = build_corpus(&tmp_root);
    println!("Wrote {file_count} files in {:.1}s.\n", setup_start.elapsed().as_secs_f64());

    let rt = tokio::runtime::Runtime::new().expect("build tokio runtime");

    println!(
        "{:<45} {:>8} {:>10} {:>10} {:>9}",
        "Query shape", "narrowed?", "elapsed", "searched", "hits"
    );
    for shape in QUERY_SHAPES {
        let (elapsed, searched, hits) = rt.block_on(run_and_time(&tmp_root, shape));
        println!(
            "{:<45} {:>8} {:>9.0}ms {:>10} {:>9}",
            shape.label,
            if shape.currently_narrowed { "yes" } else { "NO" },
            elapsed.as_secs_f64() * 1000.0,
            searched,
            hits
        );
    }

    std::fs::remove_dir_all(&tmp_root).ok();
}
