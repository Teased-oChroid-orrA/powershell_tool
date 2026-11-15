//! Cold start of an existing semantic index (#240 P2-1): time to open the database, to the first query (which
//! loads the model and builds the in-memory snapshot) and to a steady query, with resident memory after each.
//! Run it once per index in a fresh process so nothing is warm.
//!
//!   cargo run -p search-core --release --features semantic-search --example semantic_startup -- <semantic.sqlite> [query] [--warm]
//!
//! With `--warm`, `SemanticIndex::warm` runs first (what the Search window does when Find is first typed in) and is
//! timed on its own, so the first query then shows what the person waits for.
use std::time::Instant;

use search_core::lexical::LexicalConfig;
use search_core::semantic_index::SemanticIndex;

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().ok();
    out.and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0) / 1024.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let db = std::path::Path::new(&args[0]);
    let warm = args.iter().any(|a| a == "--warm");
    let query = args.get(1).filter(|a| *a != "--warm").cloned().unwrap_or_else(|| "fatigue crack growth in aluminium alloy".into());
    let mb = std::fs::metadata(db).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
    let start = rss_mb();
    let t = Instant::now();
    let index = SemanticIndex::open(db).expect("open");
    let open_ms = t.elapsed().as_secs_f64() * 1000.0;
    let after_open = rss_mb();
    let config = LexicalConfig::default();
    if warm {
        let t = Instant::now();
        let ready = index.warm().expect("warm");
        println!("warm: {ready} in {:.0} ms, rss {:.0} MB", t.elapsed().as_secs_f64() * 1000.0, rss_mb());
    }
    let t = Instant::now();
    index.search_tuned(&query, 100, true, &config).expect("first query");
    let first_ms = t.elapsed().as_secs_f64() * 1000.0;
    let after_first = rss_mb();
    let t = Instant::now();
    index.search_tuned(&query, 100, true, &config).expect("second query");
    let second_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("db_MB\topen_ms\tfirst_query_ms\tsecond_query_ms\trss_start\trss_after_open\trss_after_first\trss_after_second");
    println!("{mb:.0}\t{open_ms:.0}\t{first_ms:.0}\t{second_ms:.0}\t{start:.0}\t{after_open:.0}\t{after_first:.0}\t{:.0}", rss_mb());
}
