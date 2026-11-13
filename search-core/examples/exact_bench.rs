//! Manual timing harness for the exact (keyword) search path: in-memory line
//! matching, a cold full scan, a warm cache re-run, and a warm index re-search.
//! `cargo run --release -p search-core --example exact_bench [files] [filter] [lines per file, default 150]`
//! Not an SLA - synthetic text, one machine (see docs/benchmarking.md caveats).

use std::path::Path;
use std::time::Instant;

use native_search::engine::NativeSearchEngine;
use search_core::matching::{apply_line_matching, CompiledMatchState};
use search_core::models::SearchSettings;
use search_core::native_index;
use tokio_util::sync::CancellationToken;

const VOCAB: &[&str] = &[
    "torque", "spec", "deviation", "corrosion", "inspection", "aircraft", "engine", "mount", "bolt", "fastener",
    "airframe", "wing", "empennage", "fuselage", "workorder", "sustainment", "maintenance", "report", "finding",
    "Cafe\u{301}", "r\u{e9}sum\u{e9}", "naive", "na\u{ef}ve", "don\u{2019}t", "pre\u{a0}load",
];

fn body(doc: usize, lines: usize) -> String {
    let mut s = String::new();
    let mut x = doc as u64 * 2654435761 + 12345;
    for line in 0..lines {
        for _ in 0..12 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            s.push_str(VOCAB[((x >> 33) as usize) % VOCAB.len()]);
            s.push(' ');
        }
        if doc % 97 == 0 && line == (lines / 2).min(70) {
            s.push_str("needlefound");
        }
        s.push('\n');
    }
    s
}

fn make_corpus(root: &Path, n: usize, lines: usize) {
    for d in 0..40 {
        std::fs::create_dir_all(root.join(format!("d{d}"))).unwrap();
    }
    for i in 0..n {
        std::fs::write(root.join(format!("d{}/f{i}.txt", i % 40)), body(i, lines)).unwrap();
    }
}

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    println!("{label:<48} {:>9.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    r
}

#[tokio::main]
async fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(3000);
    let filter = std::env::args().nth(2).unwrap_or_else(|| "needlefound".to_string());
    let lines: usize = std::env::args().nth(3).and_then(|a| a.parse().ok()).unwrap_or(150);
    let dir = tempfile::tempdir().unwrap();
    time(&format!("generate {n} files"), || make_corpus(dir.path(), n, lines));
    let root = dir.path().to_string_lossy().into_owned();
    let mut exclude = Vec::new();
    native_index::ensure_index_folder_excluded(&mut exclude);
    let base = SearchSettings {
        search_path: root.clone(),
        output_folder: root.clone(),
        filters: vec![filter.clone()],
        exclude_folders: exclude,
        extensions: Some(vec![".txt".into()]),
        ..Default::default()
    };

    // In-memory matching, no I/O: the pure line-scan cost.
    let lines: Vec<Vec<String>> = (0..200).map(|i| body(i, 150).lines().map(String::from).collect()).collect();
    let total: usize = lines.iter().map(Vec::len).sum();
    for (label, tweak) in [
        ("match literal", 0),
        ("match whole-word", 1),
        ("match regex", 2),
        ("match literal + 3 filters", 3),
    ] {
        let mut s = base.clone();
        match tweak {
            1 => s.whole_word = true,
            2 => s.use_regex = true,
            3 => s.filters = vec![filter.clone(), "torque".into(), "bolt".into()],
            _ => {}
        }
        let st = CompiledMatchState::build(&s).unwrap();
        let mut hits = 0;
        time(&format!("{label} ({total} lines x5)"), || {
            for _ in 0..5 {
                for f in &lines {
                    hits += apply_line_matching(f, &s, &st).hits.len();
                }
            }
        });
        println!("    hits: {hits}");
    }

    {
        let paths: Vec<_> = (0..n).map(|i| dir.path().join(format!("d{}/f{i}.txt", i % 40))).collect();
        let t = Instant::now();
        let all: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
        println!("{:<48} {:>9.1} ms", "stage: std::fs::read all files", t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let ex: Vec<_> = all
            .iter()
            .map(|b| search_core::extraction::extract_lines_by_extension(".txt", b, 15, None, false).unwrap())
            .collect();
        println!("{:<48} {:>9.1} ms", "stage: extract lines all files", t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let st = CompiledMatchState::build(&base).unwrap();
        let h: usize = ex.iter().map(|e| apply_line_matching(&e.lines, &base, &st).hits.len()).sum();
        println!("{:<48} {:>9.1} ms  ({h} hits)", "stage: match all files", t.elapsed().as_secs_f64() * 1000.0);
    }
    let cache = dir.path().join("cache.json");
    let mut cached = base.clone();
    cached.cache_file_path = Some(cache.to_string_lossy().into_owned());
    cached.parallel = true;
    let mut seq = base.clone();
    seq.parallel = false;
    macro_rules! timed {
        ($label:expr, $e:expr) => {{
            let t = Instant::now();
            let r = $e.await;
            println!("{:<48} {:>9.1} ms", $label, t.elapsed().as_secs_f64() * 1000.0);
            r
        }};
    }
    let r = timed!("cold full scan, sequential", search_core::orchestrator::run(seq.clone(), None, CancellationToken::new()));
    println!("    results: {}", r.unwrap().file_results.len());
    let _ = timed!("cold full scan, parallel (+cache write)", search_core::orchestrator::run(cached.clone(), None, CancellationToken::new()));
    let warm = timed!("warm re-run, cache hit", search_core::orchestrator::run(cached.clone(), None, CancellationToken::new())).unwrap();
    println!("    cache_reused: {}", warm.summary.cache_reused);

    let idx_dir = native_index::index_directory(&root);
    native_index::ensure_index_directory_exists(&idx_dir).unwrap();
    let engine = NativeSearchEngine::open_or_create(&idx_dir).unwrap();
    let mut par = base.clone();
    par.parallel = true;
    let t = Instant::now();
    native_index::build_or_update_corpus_index(&par, &engine, &CancellationToken::new(), None).await.unwrap();
    println!("{:<48} {:>9.1} ms", "index build", t.elapsed().as_secs_f64() * 1000.0);
    std::thread::sleep(std::time::Duration::from_secs(15)); // let segment merges settle
    let t = Instant::now();
    let narrow = native_index::narrow_candidates(&par, &engine, &CancellationToken::new()).await.unwrap();
    println!(
        "{:<48} {:>9.1} ms  (walk {} fresh {} query {}; candidates {:?})",
        "index narrow",
        t.elapsed().as_secs_f64() * 1000.0,
        narrow.walk_ms,
        narrow.freshness_ms,
        narrow.query_ms,
        narrow.candidates.as_ref().map(Vec::len)
    );
    if let Some(c) = narrow.candidates {
        let t = Instant::now();
        let r = search_core::orchestrator::run_candidates(&c, par.clone(), None, CancellationToken::new())
            .await
            .unwrap();
        println!("{:<48} {:>9.1} ms  ({} files)", "run_candidates (verify)", t.elapsed().as_secs_f64() * 1000.0, r.file_results.len());
    }
}
