//! Closes issue #8's "result processing breakdown" Known Gap
//! (`docs/issue-8-status.md`): candidate generation, verification, snippet
//! generation, and result serialization, timed as four *separate* phases
//! against real fixture files - not the tiny in-memory `str::contains`
//! proxy `native-search/benches/trigram_candidate_reduction.rs` uses for
//! its "full-scan" baseline (that benchmark says so itself: "That is *not*
//! what production verification costs").
//!
//! Same deliberate choices as this crate's other harnesses: plain manual
//! timing (`cargo bench`, `harness = false`), no criterion. Read
//! `docs/benchmarking.md`'s standing caveats (wrong hardware, small corpus)
//! before citing any number this prints elsewhere.
//!
//! ## Where the four phase boundaries actually are (read before editing this file)
//!
//! - **Candidate generation** = `native_search::engine::NativeSearchEngine::trigram_candidate_paths`
//!   only - the safe-superset pre-filter query, nothing else.
//! - **Verification** = `search_core::matching::apply_line_matching` against
//!   the REAL lines a REAL file extracts to (via
//!   `search_core::extraction::extract_lines_by_extension` in the untimed
//!   setup phase - extraction itself is already benchmarked separately in
//!   `discovery_and_extraction.rs` and is deliberately NOT re-timed here).
//! - **Snippet generation** = the highlighted-match-span computation
//!   `search_core::report`'s `append_file_block` calls per hit line. That
//!   function (`highlight_matches`) is a private `fn`, not `pub` - there is
//!   no way to call it directly from this external bench binary without
//!   making production code `pub` (out of scope: this investigation is
//!   measurement-only). Instead, `bench_highlight_line` below is a
//!   line-for-line copy of its logic, built only from pieces `report.rs`
//!   itself uses and that already ARE public (`fancy_regex` - already a
//!   direct `search-core` dependency - and `matching::whole_word_pattern`).
//!   This is not a cost proxy the way the trigram benchmark's
//!   `str::contains` stand-in is: it is the *same* regex compile +
//!   `find_iter` + range-merge work, not a cheaper approximation of it. Its
//!   output is differentially checked against the real, `pub`
//!   `report::build_html_report`'s actual output in `setup_and_verify`
//!   below before any timing happens, on real hit lines, precisely the
//!   "differential testing discipline this repo already established for
//!   the stream_re fix" the task asks for - if the two ever disagree, the
//!   verification panics before a single timing number is printed, rather
//!   than silently reporting a wrong number.
//! - **Result serialization** = `report::build_export_rows` +
//!   `report::write_csv`/`write_json`/`write_jsonl` (pure serialization,
//!   never touch snippet highlighting) and, separately,
//!   `report::write_html_report` (reported on its own, explicitly labeled,
//!   because it necessarily *also* re-runs snippet generation internally -
//!   there's no production entry point that serializes HTML without
//!   highlighting, so that combined number is reported honestly as
//!   combined, not misrepresented as pure serialization).
//!
//! All four phases run against the same real hit set, built once from real
//! DOCX/PPTX/XLSX/RTF/PDF fixtures (`tests/TextInFilesSearch.Tests/Fixtures/`
//! and `search-core/benches/data/`, per the task's own instruction to reuse
//! those rather than trivial synthetic strings) - not four disconnected
//! synthetic experiments.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use native_search::engine::{DocumentInput, NativeSearchEngine};
use search_core::extraction::extract_lines_by_extension;
use search_core::matching::{apply_line_matching, CompiledMatchState};
use search_core::models::{
    FileSearchResult, FileSearchStatus, LineHit, SearchRunResult, SearchRunSummary, SearchSettings,
};
use search_core::report::{
    build_export_rows, build_html_report, write_csv, write_html_report, write_json, write_jsonl,
};

/// One real fixture file, already read and extracted (untimed setup - the
/// extraction cost itself is `discovery_and_extraction.rs`'s job, not
/// this benchmark's).
struct RealFile {
    label: String,
    lines: Vec<String>,
}

/// Real fixture files this benchmark reuses, per the task's own
/// instruction - the tiny correctness fixtures
/// `search-core/tests/fixtures.rs` uses (via the same
/// `tests/TextInFilesSearch.Tests/Fixtures/` directory
/// `discovery_and_extraction.rs` already reads from) plus the real
/// medium/large/xlarge documents in `search-core/benches/data/` (Apache
/// POI/Tika/PDFBox test-data, sample-files.com, arXiv.org - see that
/// directory's README for exact provenance). `xlarge-scanned.pdf` (image-
/// only, no OCR feature here) and `xlarge-recordheavy.xlsx` (correctly
/// zip-bomb-rejected) are deliberately excluded - both extract zero real
/// text, so they contribute nothing to a benchmark whose whole point is
/// real per-line matching/highlighting/serialization content.
fn real_fixture_files() -> Vec<(&'static str, PathBuf)> {
    let tiny_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/TextInFilesSearch.Tests/Fixtures");
    let sized_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/data");
    vec![
        (".docx", tiny_dir.join("test.docx")),
        (".pptx", tiny_dir.join("test.pptx")),
        (".pptx", tiny_dir.join("test_notes.pptx")),
        (".xlsx", tiny_dir.join("test.xlsx")),
        (".pdf", tiny_dir.join("test.pdf")),
        (".docx", sized_dir.join("medium.docx")),
        (".docx", sized_dir.join("large.docx")),
        (".docx", sized_dir.join("xlarge.docx")),
        (".pptx", sized_dir.join("medium.pptx")),
        (".pptx", sized_dir.join("large.pptx")),
        (".xlsx", sized_dir.join("medium.xlsx")),
        (".xlsx", sized_dir.join("large.xlsx")),
        (".rtf", sized_dir.join("medium.rtf")),
        (".rtf", sized_dir.join("large.rtf")),
        (".pdf", sized_dir.join("medium.pdf")),
        (".pdf", sized_dir.join("large.pdf")),
        (".pdf", sized_dir.join("xlarge.pdf")),
    ]
}

/// Reads and extracts every real fixture (untimed setup). A file that
/// can't be read or extracted is skipped with a printed reason, not a
/// panic - matches this app's own "a bad file must never stop the rest of
/// a run" philosophy, applied here to benchmark setup rather than a real
/// search.
fn load_real_files() -> Vec<RealFile> {
    let mut out = Vec::new();
    for (ext, path) in real_fixture_files() {
        if !path.exists() {
            println!("  setup: skipping missing fixture {}", path.display());
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                println!("  setup: skipping {} - read error: {e}", path.display());
                continue;
            }
        };
        match extract_lines_by_extension(ext, &bytes, 30, None, false) {
            Ok(extracted) if !extracted.lines.is_empty() => {
                out.push(RealFile {
                    label: format!("{ext}:{}", path.file_name().unwrap().to_string_lossy()),
                    lines: extracted.lines,
                });
            }
            Ok(_) => println!(
                "  setup: skipping {} - extracted zero lines",
                path.display()
            ),
            Err(e) => println!(
                "  setup: skipping {} - extraction failed: {e:?}",
                path.display()
            ),
        }
    }
    out
}

/// Splits words on anything that isn't alphanumeric, lowercased - good
/// enough for frequency counting, not a tokenizer that needs to match
/// `matching.rs`'s own semantics (that's exercised for real by the timed
/// `apply_line_matching` calls below, not by this word-picking helper).
fn words_of(line: &str) -> impl Iterator<Item = String> + '_ {
    line.split(|c: char| !c.is_alphanumeric())
        // Upper bound excludes a real, known PDF-extraction artifact: a
        // font's missing inter-word spacing metadata can make
        // `extract_pdf_lines` glue several real words into one long
        // run with no separators (still real, extracted text - just not
        // a real single "word" to use as a filter term).
        .filter(|w| (4..25).contains(&w.len()) && w.chars().all(|c| c.is_ascii_alphabetic()))
        .map(|w| w.to_lowercase())
}

/// Picks two real, guaranteed-present filter terms directly from the real
/// corpus content, instead of hardcoding a word that might not survive a
/// future fixture change: `common` is the most frequent word overall
/// (spread across the most distinct files); `rare` is the longest word
/// that appears in exactly one file (a real, naturally-selective term,
/// the same shape as the existing trigram benchmark's synthetic
/// "zqx9k7f2" marker but drawn from real content instead of invented).
fn pick_terms(files: &[RealFile]) -> (String, String) {
    let mut file_count: HashMap<String, HashSet<usize>> = HashMap::new();
    let mut total_count: HashMap<String, u32> = HashMap::new();
    for (fi, f) in files.iter().enumerate() {
        for line in &f.lines {
            for w in words_of(line) {
                *total_count.entry(w.clone()).or_insert(0) += 1;
                file_count.entry(w).or_default().insert(fi);
            }
        }
    }

    let common = total_count
        .iter()
        .max_by_key(|(w, &c)| (file_count.get(*w).map(|s| s.len()).unwrap_or(0), c))
        .map(|(w, _)| w.clone())
        .expect("at least one word must exist across all real fixtures");

    // "Rare" candidates: real words confined to exactly one file. Prefers a
    // natural-looking word (moderate length, not one character repeated -
    // real test fixtures do contain both degenerate filler runs like
    // "qqqqqqqqqqqqqqqqqqqqqqq" and non-Latin real words, both real but
    // poor choices for a legible example in a report) with the lowest
    // total in-corpus frequency (most selective), falling back to any
    // file-unique word, then to `common` itself, so this never panics
    // regardless of what real content future fixture changes bring in.
    let is_natural_word =
        |w: &str| w.len() >= 5 && w.len() <= 15 && w.chars().collect::<HashSet<_>>().len() > 2;
    let rare = file_count
        .iter()
        .filter(|(w, files)| files.len() == 1 && is_natural_word(w))
        .min_by_key(|(w, _)| total_count.get(*w).copied().unwrap_or(u32::MAX))
        .or_else(|| {
            file_count
                .iter()
                .filter(|(_, files)| files.len() == 1)
                .max_by_key(|(w, _)| w.len())
        })
        .map(|(w, _)| w.clone())
        .unwrap_or_else(|| common.clone());

    (common, rare)
}

fn literal_settings(term: &str) -> SearchSettings {
    SearchSettings {
        filters: vec![term.to_string()],
        ..Default::default()
    }
}

/// Line-for-line copy of `report.rs`'s private `highlight_matches` -
/// see this file's top doc comment for why this can't just call the real
/// (non-`pub`) function, and why this is a faithful reimplementation, not
/// a cheap proxy: same `fancy_regex` compile, same `find_iter` scan, same
/// overlapping-range merge, same HTML escaping. Differentially verified
/// against the real `report::build_html_report`'s actual output in
/// `verify_snippet_generation_matches_production` before any timing runs.
fn bench_highlight_line(
    line: &str,
    matched_filters: &[String],
    settings: &SearchSettings,
) -> String {
    if line.is_empty() {
        return bench_html_escape(line);
    }

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for f in matched_filters {
        if f.is_empty() {
            continue;
        }
        let pattern = if settings.use_regex {
            f.clone()
        } else if settings.whole_word {
            search_core::matching::whole_word_pattern(f)
        } else {
            fancy_regex::escape(f).into_owned()
        };

        let rx = match fancy_regex::RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .build()
        {
            Ok(r) => r,
            Err(_) => continue,
        };

        for m in rx.find_iter(line).flatten() {
            if !m.as_str().is_empty() {
                ranges.push((m.start(), m.end()));
            }
        }
    }

    if ranges.is_empty() {
        return bench_html_escape(line);
    }

    ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in ranges {
        if let Some(last) = merged.last_mut() {
            if r.0 <= last.1 {
                if r.1 > last.1 {
                    last.1 = r.1;
                }
                continue;
            }
        }
        merged.push(r);
    }

    let mut out = String::new();
    let mut pos = 0usize;
    for (start, end) in merged {
        if start > pos {
            out.push_str(&bench_html_escape(&line[pos..start]));
        }
        out.push_str("<mark>");
        out.push_str(&bench_html_escape(&line[start..end]));
        out.push_str("</mark>");
        pos = end;
    }
    if pos < line.len() {
        out.push_str(&bench_html_escape(&line[pos..]));
    }
    out
}

fn bench_html_escape(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Runs `apply_line_matching` (the real verification phase) against every
/// real file's real lines for one filter term, returning per-file elapsed
/// time and the resulting `FileSearchResult`s for files that hit - reused
/// downstream by the snippet-generation and serialization phases so all
/// four phases share one real, coherent result set instead of four
/// disconnected fixtures.
fn run_verification(files: &[RealFile], term: &str) -> (Vec<u128>, Vec<FileSearchResult>) {
    let settings = literal_settings(term);
    let state = CompiledMatchState::build(&settings)
        .expect("literal mode never produces an invalid-regex error");

    let mut per_file_us = Vec::with_capacity(files.len());
    let mut hit_results = Vec::new();

    for f in files {
        let t0 = Instant::now();
        let outcome = apply_line_matching(&f.lines, &settings, &state);
        per_file_us.push(t0.elapsed().as_micros());

        if !outcome.hits.is_empty() {
            hit_results.push(FileSearchResult {
                full_name: f.label.clone(),
                status: FileSearchStatus::Hit,
                hits: outcome.hits,
                created: chrono::Local::now(),
                modified: chrono::Local::now(),
                file_length: f.lines.iter().map(|l| l.len() as i64).sum(),
                lines_cache: f.lines.clone(),
                total_line_count: f.lines.len() as i32,
                proximity_min_range: None,
                low_confidence_pdf: false,
                error_message: None,
            });
        }
    }

    (per_file_us, hit_results)
}

fn print_latency_stats(label: &str, mut samples_us: Vec<u128>, total_us: u128) {
    if samples_us.is_empty() {
        println!("  {label}: no samples");
        return;
    }
    samples_us.sort_unstable();
    let n = samples_us.len();
    let median = samples_us[n / 2];
    let min = samples_us[0];
    let max = samples_us[n - 1];
    println!(
        "  {label}: {n} file(s), total {total_us}us, per-file min {min}us / median {median}us / max {max}us"
    );
}

/// Real, not fabricated: builds one real `SearchRunResult` from the real
/// hits `run_verification` found, calls the real (`pub`)
/// `report::build_html_report`, and checks that our local
/// `bench_highlight_line` reimplementation's output for each real hit
/// line is byte-for-byte present in the real report's actual HTML output.
/// Panics (failing the whole benchmark loudly, before any timing number
/// is printed) if even one line disagrees - the same "prove equivalence,
/// don't assume it" discipline `find_stream_blocks`'s differential tests
/// used against the original `stream_re` regex.
fn verify_snippet_generation_matches_production(settings: &SearchSettings, run: &SearchRunResult) {
    let html = build_html_report(settings, run);
    let mut checked = 0usize;
    for r in &run.file_results {
        for hit in &r.hits {
            let expected = bench_highlight_line(&hit.match_line, &hit.matched_filters, settings);
            assert!(
                html.contains(&expected),
                "bench_highlight_line's output for a real hit line in {} was not found in report::build_html_report's \
                 real output - the reimplementation has drifted from production and every snippet-generation number \
                 below would be measuring the wrong thing. Line: {:?}",
                r.full_name,
                hit.match_line
            );
            checked += 1;
        }
    }
    println!("  differential check: {checked} real hit line(s) confirmed byte-for-byte identical to production report::build_html_report output");
}

fn main() {
    println!("search-core result-processing phase benchmark (issue #8 \"result processing breakdown\" gap)");
    println!("Measured on THIS machine only - not the win-x64 target hardware. See docs/benchmarking.md.\n");

    println!("Setup (untimed - extraction cost is discovery_and_extraction.rs's job, not this benchmark's):");
    let files = load_real_files();
    if files.is_empty() {
        println!("No real fixture files could be loaded - nothing to benchmark. See search-core/benches/data/README.md.");
        return;
    }
    println!(
        "  loaded {} real file(s), {} total real lines\n",
        files.len(),
        files.iter().map(|f| f.lines.len()).sum::<usize>()
    );

    let (common_term, rare_term) = pick_terms(&files);
    println!("Filter terms picked dynamically from real fixture content (not hardcoded):");
    println!("  common = {common_term:?}   rare = {rare_term:?}\n");

    // ---- Phase 1: candidate generation (native-search trigram query only) ----
    // The corpus indexed here is real content, not synthetic filler: each
    // real file's real extracted lines are chunked into ~60-line blocks
    // (still real text) so the index has enough documents for the trigram
    // query's fixed per-query overhead to be meaningfully measured, the
    // same reason the existing trigram_candidate_reduction.rs benchmark
    // needs a multi-thousand-document corpus at all.
    const CHUNK_LINES: usize = 60;
    let index_dir = std::env::temp_dir().join(format!(
        "search-core-result-phases-bench-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&index_dir).expect("create bench index dir");
    let engine = NativeSearchEngine::open_or_create(&index_dir).expect("open_or_create");

    let mut corpus_size = 0usize;
    for (fi, f) in files.iter().enumerate() {
        for (bi, chunk) in f.lines.chunks(CHUNK_LINES).enumerate() {
            let body = chunk.join("\n");
            if body.trim().is_empty() {
                continue;
            }
            let id = format!("{fi}-{bi}");
            engine
                .index_document(DocumentInput {
                    id: &id,
                    path: &id,
                    filename: &f.label,
                    extension: "",
                    title: "",
                    modified_unix: 0,
                    created_unix: 0,
                    size: body.len() as i64,
                    body: &body,
                })
                .expect("index_document");
            corpus_size += 1;
        }
    }
    engine.commit().expect("commit");

    println!("Phase 1 - candidate generation (native_search::engine::trigram_candidate_paths, real-content corpus, {corpus_size} documents):");
    for (label, term) in [("common", &common_term), ("rare", &rare_term)] {
        let t0 = Instant::now();
        let candidates = engine
            .trigram_candidate_paths(&[term.clone()])
            .expect("trigram_candidate_paths");
        let elapsed_us = t0.elapsed().as_micros();
        match &candidates {
            Some(paths) => println!(
                "  {label:<6} {term:?}: {elapsed_us}us, {} of {corpus_size} documents ({:.1}%)",
                paths.len(),
                100.0 * paths.len() as f64 / corpus_size.max(1) as f64
            ),
            None => println!("  {label:<6} {term:?}: {elapsed_us}us, no narrowing possible (below trigram threshold)"),
        }
    }
    std::fs::remove_dir_all(&index_dir).ok();

    // ---- Phase 2: verification (real matching.rs against real per-file lines) ----
    println!("\nPhase 2 - verification (matching::apply_line_matching, real extracted lines, real per-file cost):");
    let (common_us, common_hits) = run_verification(&files, &common_term);
    let common_total: u128 = common_us.iter().sum();
    print_latency_stats(
        &format!("{common_term:?} (common)"),
        common_us,
        common_total,
    );

    let (rare_us, rare_hits) = run_verification(&files, &rare_term);
    let rare_total: u128 = rare_us.iter().sum();
    print_latency_stats(&format!("{rare_term:?} (rare)"), rare_us, rare_total);

    // The common-term hit set is used for phases 3/4 below - it produces
    // more real hit lines than the rare term does by construction, giving
    // snippet generation/serialization more real content to work over.
    if common_hits.is_empty() {
        println!(
            "\nNo real hits produced for phases 3/4 - cannot proceed (see picked terms above)."
        );
        return;
    }

    let settings = literal_settings(&common_term);
    let mut run = SearchRunResult {
        summary: SearchRunSummary::default(),
        ..Default::default()
    };
    run.file_results = common_hits.clone();

    // ---- Phase 3: snippet generation (differentially verified, then timed in isolation) ----
    println!("\nPhase 3 - snippet generation (highlight-span computation per real hit line):");
    verify_snippet_generation_matches_production(&settings, &run);

    let all_hits: Vec<&LineHit> = run
        .file_results
        .iter()
        .flat_map(|r| r.hits.iter())
        .collect();
    let mut snippet_us = Vec::with_capacity(all_hits.len());
    let t_snippet_total = Instant::now();
    for hit in &all_hits {
        let t0 = Instant::now();
        let highlighted = bench_highlight_line(&hit.match_line, &hit.matched_filters, &settings);
        snippet_us.push(t0.elapsed().as_micros());
        std::hint::black_box(&highlighted);
    }
    let snippet_total_us = t_snippet_total.elapsed().as_micros();
    print_latency_stats("highlight per line", snippet_us, snippet_total_us);

    // ---- Phase 4: result serialization (report.rs's writers) ----
    println!("\nPhase 4 - result serialization (report.rs's row-building + CSV/JSON/JSONL/HTML writers):");
    let out_dir = std::env::temp_dir().join(format!(
        "search-core-result-phases-export-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&out_dir).expect("create export dir");

    let t0 = Instant::now();
    let rows = build_export_rows(&run);
    println!(
        "  build_export_rows: {}us ({} row(s))",
        t0.elapsed().as_micros(),
        rows.len()
    );

    let t0 = Instant::now();
    write_csv(out_dir.join("out.csv").to_str().unwrap(), &rows).expect("write_csv");
    println!("  write_csv:   {}us", t0.elapsed().as_micros());

    let t0 = Instant::now();
    write_json(out_dir.join("out.json").to_str().unwrap(), &rows).expect("write_json");
    println!("  write_json:  {}us", t0.elapsed().as_micros());

    let t0 = Instant::now();
    write_jsonl(out_dir.join("out.jsonl").to_str().unwrap(), &rows).expect("write_jsonl");
    println!("  write_jsonl: {}us", t0.elapsed().as_micros());

    // Reported separately and labeled, not averaged into the pure-
    // serialization numbers above: write_html_report necessarily also
    // performs phase 3's snippet-generation work internally (there is no
    // production entry point that serializes HTML without highlighting),
    // so this number is real but combined, not pure serialization.
    let t0 = Instant::now();
    let html_bytes = write_html_report(out_dir.join("out.html").to_str().unwrap(), &settings, &run)
        .expect("write_html_report");
    println!(
        "  write_html_report (includes phase 3's snippet generation - not pure serialization): {}us, {html_bytes} bytes written",
        t0.elapsed().as_micros()
    );

    std::fs::remove_dir_all(&out_dir).ok();

    // rare_hits only feeds phase-1/2 context above; keep it alive/used so
    // a future edit that adds a rare-term serialization pass has it ready
    // without silently discarding real data computed above.
    std::hint::black_box(&rare_hits);
}
