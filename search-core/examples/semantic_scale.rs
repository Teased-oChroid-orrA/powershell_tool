//! Semantic index at scale (P2-1): builds one real semantic index through the product path (`index_document`) from
//! the text of several BEIR-format folders, and at growing chunk counts reports embedding throughput, database size,
//! resident memory and query latency (first query after a write, which builds the in-memory snapshot, then steady).
//!
//!   cargo run -p search-core --release --features semantic-search --example semantic_scale -- <out-dir> <folder>...
//!
//! Documents are the `title + text` of each `corpus.jsonl`; identical texts are skipped (the index dedupes by content
//! hash, which would otherwise hide the embedding cost). The queries are the first 20 of each folder's `queries.jsonl`.
use std::path::Path;
use std::time::Instant;

use search_core::lexical::LexicalConfig;
use search_core::semantic_index::SemanticIndex;

fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().ok();
    out.and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0) / 1024.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, folders) = args.split_first().expect("usage: <out-dir> <folder>...");
    std::fs::create_dir_all(out).unwrap();
    let db = Path::new(out).join("semantic.sqlite");
    let _ = std::fs::remove_file(&db);
    let index = SemanticIndex::open(&db).expect("open");
    let mut docs: Vec<String> = Vec::new();
    let mut queries: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for f in folders {
        for d in read_jsonl(&Path::new(f).join("corpus.jsonl")) {
            let t = format!("{} {}", d["title"].as_str().unwrap_or(""), d["text"].as_str().unwrap_or("")).trim().to_string();
            if !t.is_empty() && seen.insert(t.clone()) {
                docs.push(t);
            }
        }
        queries.extend(read_jsonl(&Path::new(f).join("queries.jsonl")).iter().take(20).filter_map(|q| q["text"].as_str().map(String::from)));
    }
    eprintln!("{} documents, {} queries; rss at start {:.0} MB", docs.len(), queries.len(), rss_mb());
    let config = LexicalConfig::default();
    let checkpoints = [2_000usize, 5_000, 10_000, 20_000, 35_000, 60_000, 100_000, 150_000, 250_000, 300_000];
    let (mut chunks, mut next, mut embed_secs, mut last_chunks, mut last_secs) = (0usize, 0usize, 0.0f64, 0usize, 0.0f64);
    println!("chunks\tdocs\tchunks_per_s_window\tdb_MB\trss_MB\tfirst_query_ms\tsteady_query_ms");
    for (i, d) in docs.iter().enumerate() {
        let t = Instant::now();
        chunks += index.index_document(&format!("/scale/{i}.txt"), 1_700_000_000 + i as i64, d.len() as i64, &[d.clone()]).expect("index");
        embed_secs += t.elapsed().as_secs_f64();
        if next < checkpoints.len() && chunks >= checkpoints[next] || i + 1 == docs.len() {
            while next < checkpoints.len() && chunks >= checkpoints[next] {
                next += 1;
            }
            let rate = (chunks - last_chunks) as f64 / (embed_secs - last_secs).max(1e-9);
            (last_chunks, last_secs) = (chunks, embed_secs);
            let t = Instant::now();
            index.search_tuned(&queries[0], 100, true, &config).expect("first query");
            let first = t.elapsed().as_secs_f64() * 1000.0;
            let t = Instant::now();
            for q in &queries {
                index.search_tuned(q, 100, true, &config).expect("query");
            }
            let steady = t.elapsed().as_secs_f64() * 1000.0 / queries.len() as f64;
            let mb = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
            println!("{chunks}\t{}\t{rate:.1}\t{mb:.0}\t{:.0}\t{first:.0}\t{steady:.1}", i + 1, rss_mb());
        }
    }
    println!("total embed+store {embed_secs:.0}s for {chunks} chunks = {:.1} chunks/s", chunks as f64 / embed_secs);
}
