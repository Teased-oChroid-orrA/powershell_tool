//! Measurement harness for `build_or_update_corpus_index` scheduling
//! (head-of-line blocking between slow and fast extractions).
//!
//! `#[ignore]`d: builds a mixed corpus of many small `.txt` files with a
//! few multi-megabyte real DOCX/PDF fixtures interleaved, then times a
//! cold index build. Run on demand:
//!
//! ```text
//! cargo test -p search-core --release --test index_scheduling -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use search_core::models::SearchSettings;
use search_core::native_index;
use tokio_util::sync::CancellationToken;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/data")
}

const TXT_FILES: usize = 960;
/// One heavy fixture after every this-many small files.
const DEFAULT_HEAVY_EVERY: usize = 60;

fn heavy_every() -> usize {
    std::env::var("HEAVY_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_HEAVY_EVERY)
}

fn build_corpus(root: &Path) -> usize {
    let heavy = ["xlarge.pdf", "xlarge-scanned.pdf", "large.pdf", "xlarge.pdf"];
    let mut heavy_count = 0;
    for i in 0..TXT_FILES {
        let sub = root.join(format!("d{:03}", i / 20));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("f{i:05}.txt")), format!("small file {i} with some searchable prose\n").repeat(40)).unwrap();
        if std::env::var_os("NO_HEAVY").is_none() && i % heavy_every() == 0 {
            let name = heavy[heavy_count % heavy.len()];
            std::fs::copy(data_dir().join(name), sub.join(format!("f{i:05}-heavy-{name}"))).unwrap();
            heavy_count += 1;
        }
    }
    heavy_count
}

/// Total bytes under `p`. Background merges delete segment files while this
/// walks, so a vanished entry is skipped rather than an error.
fn dir_size(p: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(p) else { return 0 };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok().map(|m| if m.is_dir() { dir_size(&e.path()) } else { m.len() }))
        .sum()
}

fn settings(root: &Path, throttle: i32) -> SearchSettings {
    SearchSettings {
        search_path: root.to_string_lossy().into_owned(),
        parallel: true,
        throttle_limit: throttle,
        ..Default::default()
    }
}

#[test]
#[ignore]
fn cold_index_build_on_mixed_corpus() {
    let dir = tempfile::tempdir().unwrap();
    let heavy = build_corpus(dir.path());
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    for throttle in [4, 8] {
        let index_dir = tempfile::tempdir().unwrap();
        let engine = native_index::open_or_create_with_rebuild(index_dir.path()).unwrap();
        let s = settings(dir.path(), throttle);
        let t = Instant::now();
        let out = rt
            .block_on(native_index::build_or_update_corpus_index(&s, &engine, &CancellationToken::new(), None))
            .unwrap();
        println!(
            "throttle={throttle} txt={TXT_FILES} heavy={heavy} indexed={} failed={} elapsed_ms={} (outcome.elapsed_ms={}) index_bytes={}",
            out.indexed_count,
            out.failed_count,
            t.elapsed().as_millis(),
            out.elapsed_ms,
            dir_size(index_dir.path())
        );
        assert_eq!(out.failed_count, 0, "{:?}", out.failed_files);
    }
}

#[test]
#[ignore]
fn extraction_cost_per_fixture() {
    for name in std::fs::read_dir(data_dir()).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()) {
        let Some(ext) = Path::new(&name).extension().map(|e| format!(".{}", e.to_string_lossy())) else { continue };
        if ext == ".md" {
            continue;
        }
        let bytes = std::fs::read(data_dir().join(&name)).unwrap();
        let t = Instant::now();
        let r = search_core::extraction::extract_lines_by_extension(&ext, &bytes, 60, None, false);
        println!("{name}: {} ms ({} lines)", t.elapsed().as_millis(), r.map(|e| e.lines.len()).unwrap_or(0));
    }
}

/// Evidence for "does the index ever cost more than it saves?": the worst
/// case is a filter that every file contains (candidate ratio 100%). The
/// fixed overhead of narrowing (freshness check + trigram lookup, on top of
/// the directory walk a full scan needs anyway) is what a planner would
/// avoid, so it is the number that decides whether a planner is worth having.
#[test]
#[ignore]
fn narrowing_overhead_when_every_file_is_a_candidate() {
    let n: usize = std::env::var("N").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let dir = tempfile::tempdir().unwrap();
    for i in 0..n {
        let sub = dir.path().join(format!("d{:04}", i / 100));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(format!("f{i:06}.txt")), format!("common marker line {i}\n")).unwrap();
    }
    let index_dir = tempfile::tempdir().unwrap();
    let engine = native_index::open_or_create_with_rebuild(index_dir.path()).unwrap();
    let mut s = settings(dir.path(), 8);
    s.filters = vec!["common marker".to_string()];
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let cancel = CancellationToken::new();
    rt.block_on(native_index::build_or_update_corpus_index(&s, &engine, &cancel, None)).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(20)); // let background merges settle
    for _ in 0..3 {
        let t = Instant::now();
        let out = rt.block_on(native_index::narrow_candidates(&s, &engine, &cancel)).unwrap();
        println!(
            "files={n} candidates={} total_ms={} walk_ms={} freshness_ms={} lookup_ms={} segments={}",
            out.candidates.as_ref().map_or(0, |c| c.len()),
            t.elapsed().as_millis(),
            out.walk_ms,
            out.freshness_ms,
            out.query_ms,
            out.segments
        );
    }
}
