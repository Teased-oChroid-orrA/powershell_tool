//! Multilingual embedding evaluation (#240 P1-4): one BEIR-format folder (`corpus.jsonl`, `queries.jsonl`,
//! `qrels/test.tsv`; MIRACL subsets, see `docs/search-benchmarks-multilingual.md`), run through the real semantic
//! index with the bundled model or a candidate. Prints one tab-separated row: indexing speed, query embedding time,
//! and nDCG@10 / Recall@100 for keyword only, similarity only and the shipped hybrid (`default`).
//!
//!   cargo run -p search-core --release --features semantic-search --example multilingual_eval -- <folder> \
//!     [--model <folder with model.onnx and tokenizer.json> --pooling mean|cls --query-prefix "query: " --passage-prefix "passage: "] \
//!     [--tag name] [--queries N] [--header]
//!
//! Indexes are kept in `<folder>/.eval-index[-tag]`, the same layout as `retrieval_eval`, so the two share them. The
//! index is built once; the rate printed is for documents indexed in this run (`-` when the index already existed).
use std::collections::HashMap;
use std::path::Path;

use search_core::lexical::LexicalConfig;
use search_core::semantic_index::SemanticIndex;

fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("json line"))
        .collect()
}

fn ndcg_at(ranked: &[String], gains: &HashMap<String, f64>, k: usize) -> f64 {
    let dcg = |gains: &mut dyn Iterator<Item = f64>| {
        gains.take(k).enumerate().map(|(i, g)| (2f64.powf(g) - 1.0) / ((i + 2) as f64).log2()).sum::<f64>()
    };
    let got = dcg(&mut ranked.iter().map(|d| gains.get(d).copied().unwrap_or(0.0)));
    let mut ideal: Vec<f64> = gains.values().copied().collect();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let best = dcg(&mut ideal.into_iter());
    if best > 0.0 { got / best } else { 0.0 }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let folder = Path::new(&args[1]);
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let max_queries: usize = flag("--queries").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let model_dir = flag("--model").map(Path::new);
    let tag = flag("--tag").cloned().unwrap_or_else(|| {
        model_dir.and_then(|d| d.file_name()).map_or("minilm".to_string(), |n| n.to_string_lossy().into_owned())
    });
    if let Some(dir) = model_dir {
        let spec = search_ml::ModelSpec {
            pooling: if flag("--pooling").is_some_and(|p| p == "cls") { search_ml::Pooling::Cls } else { search_ml::Pooling::Mean },
            query_prefix: flag("--query-prefix").cloned().unwrap_or_default(),
            passage_prefix: flag("--passage-prefix").cloned().unwrap_or_default(),
        };
        search_ml::use_embedding_model(
            std::fs::read(dir.join("model.onnx")).expect("model.onnx"),
            std::fs::read(dir.join("tokenizer.json")).expect("tokenizer.json"),
            spec,
        )
        .expect("set the embedding model");
    }

    let corpus = read_jsonl(&folder.join("corpus.jsonl"));
    let mut qrels: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for line in std::fs::read_to_string(folder.join("qrels/test.tsv")).expect("qrels").lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() >= 3 {
            let gain: f64 = cols[2].trim().parse().unwrap_or(0.0);
            if gain > 0.0 {
                qrels.entry(cols[0].to_string()).or_default().insert(cols[1].to_string(), gain);
            }
        }
    }
    let queries: Vec<(String, String)> = read_jsonl(&folder.join("queries.jsonl"))
        .into_iter()
        .filter_map(|q| Some((q["_id"].as_str()?.to_string(), q["text"].as_str()?.to_string())))
        .filter(|(id, _)| qrels.contains_key(id))
        .take(max_queries)
        .collect();

    // As in `retrieval_eval`: every document is a real file, indexed through its metadata.
    let docs_dir = folder.join(".eval-docs");
    let index_dir = folder.join(if tag == "minilm" { ".eval-index".to_string() } else { format!(".eval-index-{tag}") });
    std::fs::create_dir_all(&docs_dir).unwrap();
    std::fs::create_dir_all(&index_dir).unwrap();
    let index = SemanticIndex::open(&index_dir.join("semantic.sqlite")).expect("open index");
    let started = std::time::Instant::now();
    let mut built = 0usize;
    for doc in &corpus {
        let id = doc["_id"].as_str().unwrap_or_default();
        let text = format!("{} {}", doc["title"].as_str().unwrap_or(""), doc["text"].as_str().unwrap_or("")).trim().to_string();
        if text.is_empty() {
            continue;
        }
        let path = docs_dir.join(format!("{id}.txt"));
        if !path.is_file() {
            std::fs::write(&path, &text).unwrap();
        }
        let metadata = std::fs::metadata(&path).unwrap();
        let modified = search_core::cache::ticks_from_modified(chrono::DateTime::<chrono::Local>::from(metadata.modified().unwrap()));
        let key = path.to_string_lossy().into_owned();
        if index.is_current(&key, modified, metadata.len() as i64).unwrap_or(false) {
            continue;
        }
        index.index_document(&key, modified, metadata.len() as i64, &[text]).expect("index document");
        built += 1;
        if built % 500 == 0 {
            eprintln!("  indexed {built} of {} ({:.0}s)", corpus.len(), started.elapsed().as_secs_f64());
        }
    }
    let index_rate = if built > 0 { format!("{:.1}", built as f64 / started.elapsed().as_secs_f64()) } else { "-".to_string() };

    let embed_started = std::time::Instant::now();
    for (_, text) in &queries {
        search_ml::embed_queries(&[text.as_str()]).expect("embed");
    }
    let query_ms = embed_started.elapsed().as_secs_f64() * 1000.0 / queries.len() as f64;

    // keyword only = rank fusion with a keyword weight so large the similarity side cannot reorder it.
    let rrf = LexicalConfig::rrf();
    let variants: [(&str, bool, LexicalConfig); 3] = [
        ("keyword", true, LexicalConfig { weight: 1000.0, ..rrf }),
        ("dense", false, rrf),
        ("default", true, LexicalConfig::default()),
    ];
    let mut cells: Vec<(f64, f64)> = Vec::new();
    for (_, hybrid, config) in &variants {
        let (mut ndcg, mut recall) = (0.0, 0.0);
        for (qid, text) in &queries {
            let hits = index.search_tuned(text, 600, *hybrid, config).expect("search");
            let mut ranked: Vec<String> = Vec::new();
            for hit in hits {
                let id = Path::new(&hit.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if !ranked.contains(&id) {
                    ranked.push(id);
                }
            }
            let gains = &qrels[qid];
            ndcg += ndcg_at(&ranked, gains, 10);
            recall += ranked.iter().take(100).filter(|d| gains.contains_key(*d)).count() as f64 / gains.len() as f64;
        }
        let n = queries.len() as f64;
        cells.push((ndcg / n, recall / n));
    }
    if args.iter().any(|a| a == "--header") {
        println!("set\tmodel\tdocs\tqueries\tdocs/s\tquery ms\tkw nDCG\tdense nDCG\tdefault nDCG\tkw R@100\tdense R@100\tdefault R@100");
    }
    let set = folder.to_string_lossy().rsplit('/').find(|p| !p.is_empty() && *p != "sub").unwrap_or("set").to_string();
    println!(
        "{set}\t{tag}\t{}\t{}\t{index_rate}\t{query_ms:.1}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{:.4}",
        corpus.len(), queries.len(), cells[0].0, cells[1].0, cells[2].0, cells[0].1, cells[1].1, cells[2].1
    );
}
