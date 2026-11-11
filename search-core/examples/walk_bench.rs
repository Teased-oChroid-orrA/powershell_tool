//! Directory-walk timing (docs/knowledge-engine-audit.md, P2-1): the walk of the exact-search path against bare
//! `read_dir` and `read_dir` plus `metadata`, over N one-line files in D folders, run three times.
//! `cargo run --release -p search-core --example walk_bench -- <files> <folders>`
use std::time::Instant;

use tokio_util::sync::CancellationToken;

fn main() {
    let a: Vec<usize> = std::env::args().skip(1).filter_map(|v| v.parse().ok()).collect();
    let (n, d) = (a[0], a[1]);
    let dir = tempfile::tempdir().unwrap();
    let t = Instant::now();
    for f in 0..d {
        std::fs::create_dir_all(dir.path().join(format!("d{f}"))).unwrap();
    }
    for i in 0..n {
        std::fs::write(dir.path().join(format!("d{}/f{i}.txt", i % d)), "needle\n").unwrap();
    }
    println!("generate {n} files in {d} folders: {:.1} s", t.elapsed().as_secs_f64());
    let root = dir.path().to_string_lossy().into_owned();
    let token = CancellationToken::new();
    for pass in 1..=3 {
        let t = Instant::now();
        let mut names = 0usize;
        for e in std::fs::read_dir(dir.path()).unwrap().flatten() {
            for f in std::fs::read_dir(e.path()).unwrap().flatten() {
                names += f.file_type().map(|t| t.is_file() as usize).unwrap_or(0);
            }
        }
        let list = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let mut bytes = 0u64;
        for e in std::fs::read_dir(dir.path()).unwrap().flatten() {
            for f in std::fs::read_dir(e.path()).unwrap().flatten() {
                bytes += f.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        let stat = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let (files, _) = search_core::file_reader::enumerate_files_safely(&root, false, &[], &token, None).unwrap();
        let walk = t.elapsed().as_secs_f64();
        println!("pass {pass}: read_dir {list:.2} s ({names}), + metadata {stat:.2} s ({bytes} B), enumerate_files_safely {walk:.2} s ({} files)", files.len());
    }
}
