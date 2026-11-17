//! Embedding throughput on fixed text: chunks per second of the product path (`index_document`: chunk, embed, store)
//! and of the model alone, plus a checksum of every stored vector so a speed change can be shown to leave the
//! vectors bit-identical.
//!
//!   cargo run -p search-core --release --features semantic-search --example embed_throughput -- <documents> <folder>...
//!
//! Documents are the first `<documents>` distinct `title + text` of the folders' `corpus.jsonl`, taken round-robin.
use std::path::Path;
use std::time::Instant;

use search_core::semantic_index::SemanticIndex;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (count, folders) = args.split_first().expect("usage: <documents> <folder>...");
    let count: usize = count.parse().expect("document count");
    let mut seen = std::collections::HashSet::new();
    let sets: Vec<Vec<String>> = folders
        .iter()
        .map(|folder| {
            std::fs::read_to_string(Path::new(folder).join("corpus.jsonl"))
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .map(|d| format!("{} {}", d["title"].as_str().unwrap_or(""), d["text"].as_str().unwrap_or("")).trim().to_string())
                .filter(|text| !text.is_empty() && seen.insert(text.clone()))
                .take(count)
                .collect()
        })
        .collect();
    let mut docs: Vec<&String> = Vec::new();
    for i in 0..count {
        docs.extend(sets.iter().filter_map(|set| set.get(i)));
    }
    docs.truncate(count);
    search_ml::embed_batch(&["load the model"]).expect("model");

    let dir = tempfile::tempdir().expect("temp dir");
    let db = dir.path().join("semantic.sqlite");
    let index = SemanticIndex::open(&db).expect("open");
    let t = Instant::now();
    let mut chunks = 0usize;
    // `DOCS_PER_FILE=<n>` indexes the texts n to a file (one line each), so a file has many chunks, as a long
    // document does; the default is one text per file.
    let per_file: usize = std::env::var("DOCS_PER_FILE").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    for (i, group) in docs.chunks(per_file).enumerate() {
        let doc = group.iter().map(|d| d.replace('\n', " ")).collect::<Vec<_>>().join("\n");
        let lines: Vec<String> = doc.lines().map(String::from).collect();
        chunks += index.index_document(&format!("/throughput/{i}.txt"), 1, doc.len() as i64, &lines).expect("index");
    }
    let stored = t.elapsed().as_secs_f64();

    let conn = rusqlite::Connection::open(&db).expect("reopen");
    let mut stmt = conn.prepare("SELECT excerpt,embedding FROM semantic_chunks ORDER BY path,chunk_index").expect("select");
    let rows: Vec<(String, Vec<u8>)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).expect("rows").filter_map(Result::ok).collect();
    // FNV-1a over every vector byte in a fixed order.
    let checksum = rows.iter().flat_map(|(_, v)| v).fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3));

    // The model alone on the same chunk texts, in calls of `batch` texts from `threads` threads at once. Each row
    // says whether every vector has the same bits as the stored one.
    // `SHORT_TEXTS=<chars>` cuts every text to that many characters first (line-sized inputs, as the other callers
    // of the model send); the comparison is then against the same short texts embedded one per call.
    let short: Option<usize> = std::env::var("SHORT_TEXTS").ok().and_then(|v| v.parse().ok());
    let texts: Vec<&str> = rows
        .iter()
        .map(|(text, _)| short.map_or(text.as_str(), |n| text.char_indices().nth(n).map_or(text.as_str(), |(at, _)| &text[..at])))
        .collect();
    let stored_bits: Vec<Vec<u8>> = match short {
        None => rows.iter().map(|(_, v)| v.clone()).collect(),
        Some(_) => texts
            .iter()
            .map(|text| search_ml::embed_batch(&[text]).expect("embed")[0].values().iter().flat_map(|v| v.to_le_bytes()).collect())
            .collect(),
    };
    println!("documents\tchunks\tindex_document_chunks_per_s\tvector_checksum");
    println!("{}\t{chunks}\t{:.1}\t{checksum:016x}", docs.len(), chunks as f64 / stored);
    if std::env::var("INDEX_ONLY").is_ok() {
        return;
    }
    println!("model only, {ROUNDS} interleaved rounds: batch\tthreads\tchunks_per_s median\tmin\tmax\tvectors_vs_stored\tmax_abs_difference");
    const ROUNDS: usize = 3;
    let shapes = [(1usize, 1usize), (2, 1), (4, 1), (16, 1), (1, 2), (1, 4), (4, 4)];
    let mut rates = vec![Vec::new(); shapes.len()];
    let mut identical = vec![(usize::MAX, 0f32); shapes.len()];
    for round in 0..ROUNDS * shapes.len() {
        let shape = round % shapes.len();
        let (batch, threads) = shapes[shape];
        let calls: Vec<&[&str]> = texts.chunks(batch).collect();
        let t = Instant::now();
        let per_thread = calls.len().div_ceil(threads);
        let vectors: Vec<Vec<f32>> = std::thread::scope(|scope| {
            let handles: Vec<_> = calls
                .chunks(per_thread)
                .map(|part| {
                    scope.spawn(move || {
                        part.iter()
                            .flat_map(|call| search_ml::embed_batch(call).expect("embed"))
                            .map(|e| e.values().to_vec())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("thread")).collect()
        });
        let secs = t.elapsed().as_secs_f64();
        let (mut same, mut worst) = (0usize, 0f32);
        for (vector, bits) in vectors.iter().zip(&stored_bits) {
            let bytes: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
            same += (bytes == *bits) as usize;
            for (v, b) in vector.iter().zip(bits.chunks_exact(4)) {
                worst = worst.max((v - f32::from_le_bytes([b[0], b[1], b[2], b[3]])).abs());
            }
        }
        rates[shape].push(texts.len() as f64 / secs);
        identical[shape] = (identical[shape].0.min(same), identical[shape].1.max(worst));
    }
    for (((batch, threads), rates), (same, worst)) in shapes.iter().zip(&mut rates).zip(&identical) {
        rates.sort_by(f64::total_cmp);
        println!("{batch}\t{threads}\t{:.1}\t{:.1}\t{:.1}\t{same} of {} identical\t{worst:e}", rates[ROUNDS / 2], rates[0], rates[ROUNDS - 1], texts.len());
    }
}
