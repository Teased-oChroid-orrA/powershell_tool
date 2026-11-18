//! Streaming path above the in-memory limit (#240 P2-1): builds a synthetic semantic index larger than the
//! 250,000-chunk snapshot limit by replicating the rows of a real one, then times open, warm, first and steady
//! queries (with and without Search settings, which add one `stat` per indexed file) and resident memory.
//!
//!   cargo run -p search-core --release --features semantic-search --example semantic_stream_probe -- \
//!       <source semantic.sqlite> <out-dir> <copies> [--keep]
//!
//! Each copy re-keys every document under `<out-dir>/files/c<k>/` with a real one-byte file whose size and
//! modified time are stored in the row, because the index drops rows whose file no longer matches. Vectors are
//! copied, not recomputed, so the corpus has `copies` exact repeats of each chunk: scan cost is the real one,
//! ranking quality is not what this measures. Run once per process so nothing is warm.
use std::path::Path;
use std::time::Instant;

use chrono::{DateTime, Local};
use rusqlite::{params, Connection};
use search_core::lexical::LexicalConfig;
use search_core::models::SearchSettings;
use search_core::semantic_index::SemanticIndex;

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().ok();
    out.and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0) / 1024.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let source = Path::new(&args[0]);
    let out = Path::new(&args[1]);
    let copies: usize = args[2].parse().expect("copies");
    let keep = args.iter().any(|a| a == "--keep");
    let db = out.join("semantic.sqlite");
    let files = out.join("files");
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(&files).unwrap();
    std::fs::copy(source, &db).expect("copy source index");

    let t = Instant::now();
    let conn = Connection::open(&db).unwrap();
    let docs: Vec<(String, Vec<u8>, i64)> = conn
        .prepare("SELECT path, content_hash, citation_version FROM semantic_documents")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get::<_, Option<Vec<u8>>>(1)?.unwrap_or_default(), r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let tx = conn.unchecked_transaction().unwrap();
    for k in 1..copies {
        let dir = files.join(format!("c{k}"));
        std::fs::create_dir_all(&dir).unwrap();
        for (i, (path, hash, citation)) in docs.iter().enumerate() {
            let file = dir.join(format!("d{i}.txt"));
            std::fs::write(&file, b"x").unwrap();
            let modified = DateTime::<Local>::from(std::fs::metadata(&file).unwrap().modified().unwrap());
            let ticks = search_core::cache::ticks_from_modified(modified);
            let new_path = file.to_string_lossy().to_string();
            // content_hash differs per copy so the dedupe index sees distinct documents.
            let mut new_hash = hash.clone();
            new_hash.extend_from_slice(&(k as u32).to_le_bytes());
            tx.execute(
                "INSERT INTO semantic_documents(path, modified, size, citation_version, content_hash) VALUES(?,?,?,?,?)",
                params![new_path, ticks, 1i64, citation, new_hash],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO semantic_chunks(path, chunk_index, first_line, last_line, excerpt, embedding, page)
                 SELECT ?, chunk_index, first_line, last_line, excerpt, embedding, page FROM semantic_chunks WHERE path = ?",
                params![new_path, path],
            )
            .unwrap();
        }
    }
    tx.execute("INSERT INTO semantic_meta(key,value) VALUES('generation',1) ON CONFLICT(key) DO UPDATE SET value=value+1", []).unwrap();
    tx.commit().unwrap();
    let chunks: i64 = conn.query_row("SELECT COUNT(*) FROM semantic_chunks", [], |r| r.get(0)).unwrap();
    drop(conn);
    let build_s = t.elapsed().as_secs_f64();
    let mb = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
    println!("built {chunks} chunks, {mb:.0} MB database in {build_s:.0} s");

    let query = "fatigue crack growth in aluminium alloy";
    let config = LexicalConfig::default();
    let rss0 = rss_mb();
    let t = Instant::now();
    let index = SemanticIndex::open(&db).expect("open");
    let open_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let ready = index.warm().expect("warm");
    let warm_ms = t.elapsed().as_secs_f64() * 1000.0;
    let rss_warm = rss_mb();
    let mut timings = Vec::new();
    for label in ["first", "second", "third"] {
        let t = Instant::now();
        let hits = index.search_tuned(&query, 100, true, &config).expect("query");
        timings.push((label, t.elapsed().as_secs_f64() * 1000.0, hits.len()));
    }
    let rss_query = rss_mb();
    let settings = SearchSettings { search_path: files.to_string_lossy().to_string(), ..SearchSettings::default() };
    let t = Instant::now();
    let with_settings = index.search_with_settings(query, 100, &settings).expect("query with settings");
    let settings_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("chunks\tdb_MB\topen_ms\twarm_ready\twarm_ms\trss_start\trss_after_warm\trss_after_queries");
    println!("{chunks}\t{mb:.0}\t{open_ms:.0}\t{ready}\t{warm_ms:.0}\t{rss0:.0}\t{rss_warm:.0}\t{rss_query:.0}");
    for (label, ms, n) in timings {
        println!("{label} query (no settings): {ms:.0} ms, {n} hits");
    }
    println!("query with settings: {settings_ms:.0} ms, {} hits", with_settings.len());
    drop(index);
    if !keep {
        let _ = std::fs::remove_dir_all(out);
    }
}
