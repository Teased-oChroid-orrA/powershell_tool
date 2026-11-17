//! First query after a small write (the in-memory snapshot has to follow the write): times one query after adding,
//! editing and removing a single document in an existing semantic index, against the steady query time.
//!
//!   cargo run -p search-core --release --features semantic-search --example semantic_snapshot_update -- <semantic.sqlite>...
//!
//! Each index is copied to a temporary file first; the original is never written. Query times include the query
//! embedding (the steady column is the same query with nothing to rebuild).
use std::time::Instant;

use search_core::lexical::LexicalConfig;
use search_core::semantic_index::SemanticIndex;

fn main() {
    let config = LexicalConfig::default();
    let query = "torque limit for the flange bolts";
    println!("index\tchunks\tfull_build_ms\tafter_add_ms\tafter_edit_ms\tafter_remove_ms\tsteady_ms");
    for source in std::env::args().skip(1) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db = dir.path().join("semantic.sqlite");
        std::fs::copy(&source, &db).expect("copy index");
        let index = SemanticIndex::open(&db).expect("open");
        let ms = |f: &dyn Fn()| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.0
        };
        let search = || {
            index.search_tuned(query, 100, true, &config).expect("query");
        };
        index.search_tuned("load the model", 1, false, &config).ok();
        let path = dir.path().join("probe.txt").to_string_lossy().into_owned();
        let full = {
            // Force a full build: a fresh handle on a fresh copy has no snapshot.
            let dir2 = tempfile::tempdir().expect("temp dir");
            let db2 = dir2.path().join("semantic.sqlite");
            std::fs::copy(&source, &db2).expect("copy index");
            let cold = SemanticIndex::open(&db2).expect("open");
            let t = Instant::now();
            cold.warm().expect("warm");
            t.elapsed().as_secs_f64() * 1000.0
        };
        let best = |write: &dyn Fn(usize)| {
            // Three rounds, the median: one write, then the first query after it.
            let mut times: Vec<f64> = (0..3)
                .map(|round| {
                    write(round);
                    ms(&search)
                })
                .collect();
            times.sort_by(f64::total_cmp);
            times[1]
        };
        let add = best(&|round| {
            index.index_document(&format!("{path}.{round}"), 1, 1, &[format!("Hydrostatic test pressure record number {round}.")]).expect("add");
        });
        let edit = best(&|round| {
            index.index_document(&format!("{path}.{round}"), 2, 2, &[format!("Proof load certificate number {round} for the lifting lug.")]).expect("edit");
        });
        let remove = best(&|round| {
            index.remove_document(&format!("{path}.{round}")).expect("remove");
        });
        let steady = (0..20).map(|_| ms(&search)).sum::<f64>() / 20.0;
        let count = rusqlite::Connection::open(&db)
            .and_then(|c| c.query_row("SELECT COUNT(*) FROM semantic_chunks", [], |r| r.get::<_, i64>(0)))
            .unwrap_or(0);
        let name = std::path::Path::new(&source).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        println!("{name}\t{count}\t{full:.0}\t{add:.1}\t{edit:.1}\t{remove:.1}\t{steady:.1}");
    }
}
