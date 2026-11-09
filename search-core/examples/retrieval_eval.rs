//! Retrieval benchmark over a BEIR-format folder (`corpus.jsonl`, `queries.jsonl`, `qrels/test.tsv`), run through the
//! real semantic index. Datasets: BEIR (SciFact, NFCorpus, TREC-COVID), an MS MARCO subset, Cranfield; see
//! `docs/search-benchmarks.md`.
//!
//!   cargo run -p search-core --release --features semantic-search --example retrieval_eval -- <folder> [--queries N] [--variants a,b]
//!
//! The index is built once in `<folder>/.eval-index` and reused. Each variant is one setting of keyword scoring and
//! fusion (`search_core::lexical::LexicalConfig`); `default` is what search uses.
use std::collections::HashMap;
use std::path::Path;

use search_core::lexical::{Fusion, LexicalConfig};
use search_core::semantic_index::SemanticIndex;

struct Variant {
    name: &'static str,
    hybrid: bool,
    config: LexicalConfig,
}

fn variants() -> Vec<Variant> {
    // The older variants are rank-fusion settings; `default` is what search uses now.
    let d = LexicalConfig::rrf();
    let v = |name, hybrid, config| Variant { name, hybrid, config };
    vec![
        v("dense", false, d),
        v("keyword-led", true, LexicalConfig { weight: 1000.0, ..d }),
        v("default", true, LexicalConfig::default()),
        // Experiments built on `default`, handled in `main`: pseudo-relevance feedback and a second-model re-rank.
        v("prf", true, LexicalConfig::default()),
        v("rerank", true, LexicalConfig::default()),
        v("rerank-rrf", true, LexicalConfig::default()),
        v("rrf-w2", true, LexicalConfig::rrf()),
        v("no-stem", true, LexicalConfig { stem: false, ..LexicalConfig::default() }),
        v("rrf-no-stem", true, LexicalConfig { stem: false, ..d }),
        v("w0.5", true, LexicalConfig { weight: 0.5, ..d }),
        v("w1", true, LexicalConfig { weight: 1.0, ..d }),
        v("w1.5", true, LexicalConfig { weight: 1.5, ..d }),
        v("w3", true, LexicalConfig { weight: 3.0, ..d }),
        v("w5", true, LexicalConfig { weight: 5.0, ..d }),
        v("k10", true, LexicalConfig { rrf_k: 10.0, ..d }),
        v("k30", true, LexicalConfig { rrf_k: 30.0, ..d }),
        v("k100", true, LexicalConfig { rrf_k: 100.0, ..d }),
        v("convex0.3", true, LexicalConfig { fusion: Fusion::Convex, weight: 0.3, ..d }),
        v("convex0.2", true, LexicalConfig { fusion: Fusion::Convex, weight: 0.2, ..d }),
        v("convex0.25", true, LexicalConfig { fusion: Fusion::Convex, weight: 0.25, ..d }),
        v("convex0.35", true, LexicalConfig { fusion: Fusion::Convex, weight: 0.35, ..d }),
        v("convex0.4", true, LexicalConfig { fusion: Fusion::Convex, weight: 0.4, ..d }),
        v("convex0.5", true, LexicalConfig { fusion: Fusion::Convex, weight: 0.5, ..d }),
        v("convex0.7", true, LexicalConfig { fusion: Fusion::Convex, weight: 0.7, ..d }),
        v("w1.5-b0.5", true, LexicalConfig { weight: 1.5, bm25_b: 0.5, ..d }),
        v("w1.5-b0.9", true, LexicalConfig { weight: 1.5, bm25_b: 0.9, ..d }),
        v("w1.5-k1.5", true, LexicalConfig { weight: 1.5, bm25_k1: 1.5, ..d }),
        v("w1.5-k0.9", true, LexicalConfig { weight: 1.5, bm25_k1: 0.9, ..d }),
    ]
}

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
    let wanted: Option<Vec<&str>> = flag("--variants").map(|v| v.split(',').collect());

    // `--model <folder with model.onnx and tokenizer.json>` measures another embedding model (with `--pooling cls|mean`,
    // `--query-prefix`, `--passage-prefix`); its index is kept apart.
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
    // `--rerank-model <dir>` (with `--rerank-pooling`, `--rerank-query-prefix`) loads a second model for the `rerank` variants.
    let reranker = flag("--rerank-model").map(Path::new).map(|dir| {
        let spec = search_ml::ModelSpec {
            pooling: if flag("--rerank-pooling").is_some_and(|p| p == "cls") { search_ml::Pooling::Cls } else { search_ml::Pooling::Mean },
            query_prefix: flag("--rerank-query-prefix").cloned().unwrap_or_default(),
            passage_prefix: String::new(),
        };
        search_ml::Embedder::new(
            std::fs::read(dir.join("model.onnx")).expect("rerank model.onnx"),
            std::fs::read(dir.join("tokenizer.json")).expect("rerank tokenizer.json"),
            spec,
        )
        .expect("load the re-rank model")
    });
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

    // Search only returns documents that are real files whose size and time still match the index, so every
    // document is written as a file (`.eval-docs/<id>.txt`) and indexed through its metadata, as search does.
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
    eprintln!("{}: {} docs ({} newly indexed in {:.0}s), {} queries", folder.display(), corpus.len(), built, started.elapsed().as_secs_f64(), queries.len());

    let embed_started = std::time::Instant::now();
    for (_, text) in &queries {
        search_ml::embed_queries(&[text.as_str()]).expect("embed");
    }
    println!("embedding one query: {:.1} ms", embed_started.elapsed().as_secs_f64() * 1000.0 / queries.len() as f64);
    println!("{:<12} {:>8} {:>10} {:>8} {:>10}", "variant", "nDCG@10", "Recall@100", "MRR@10", "ms/query");
    for variant in variants().into_iter().filter(|v| wanted.as_ref().is_none_or(|w| w.contains(&v.name))) {
        let (mut ndcg, mut recall, mut mrr) = (0.0, 0.0, 0.0);
        let variant_started = std::time::Instant::now();
        for (qid, text) in &queries {
            let hits = if variant.name == "prf" {
                // Rocchio on the dense side: move the query towards the mean of the best three results.
                let first = index.search_tuned(text, 600, true, &variant.config).expect("search");
                let query = search_ml::embed_queries(&[text.as_str()]).expect("embed").remove(0);
                let mut seen = Vec::new();
                let tops: Vec<&str> = first.iter().filter(|h| { let new = !seen.contains(&h.path); seen.push(h.path.clone()); new }).take(3).map(|h| h.excerpt.as_str()).collect();
                let mut moved = query.values().to_vec();
                for vector in search_ml::embed_batch(&tops).expect("embed") {
                    for (m, x) in moved.iter_mut().zip(vector.values()) { *m += 0.5 * x / tops.len() as f32; }
                }
                let norm = moved.iter().map(|x| x * x).sum::<f32>().sqrt();
                moved.iter_mut().for_each(|x| *x /= norm);
                let embedding = search_ml::Embedding::from_values(moved).expect("embedding");
                index.search_tuned_embedded(text, &embedding, 600, true, &variant.config).expect("search")
            } else {
                index.search_tuned(text, 600, variant.hybrid, &variant.config).expect("search")
            };
            let mut ranked: Vec<String> = Vec::new();
            for hit in hits {
                let id = Path::new(&hit.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if !ranked.contains(&id) {
                    ranked.push(id);
                }
            }
            if let (Some(model), true) = (&reranker, variant.name.starts_with("rerank")) {
                // Re-score the first 30 documents with the second model; the rest keep their order.
                let top = ranked.len().min(30);
                let mut excerpts: HashMap<String, String> = HashMap::new();
                for hit in index.search_tuned(text, 600, true, &variant.config).expect("search") {
                    let id = Path::new(&hit.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    excerpts.entry(id).or_insert(hit.excerpt);
                }
                let texts: Vec<&str> = ranked[..top].iter().map(|id| excerpts[id].as_str()).collect();
                let query = model.embed_queries(&[text.as_str()]).expect("embed").remove(0);
                let scores: Vec<f32> = model.embed_passages(&texts).expect("embed").iter().map(|e| search_ml::cosine_similarity(&query, e).unwrap_or(0.0)).collect();
                let mut by_score: Vec<usize> = (0..top).collect();
                by_score.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
                let order: Vec<usize> = if variant.name == "rerank" {
                    by_score
                } else {
                    search_core::rank_fusion::fuse_top(&scores, top)
                };
                let reordered: Vec<String> = order.iter().map(|&i| ranked[i].clone()).collect();
                ranked.splice(..top, reordered);
            }
            let gains = &qrels[qid];
            ndcg += ndcg_at(&ranked, gains, 10);
            recall += ranked.iter().take(100).filter(|d| gains.contains_key(*d)).count() as f64 / gains.len() as f64;
            mrr += ranked.iter().take(10).position(|d| gains.contains_key(d)).map_or(0.0, |p| 1.0 / (p + 1) as f64);
        }
        let n = queries.len() as f64;
        println!("{:<12} {:>8.4} {:>10.4} {:>8.4} {:>10.1}", variant.name, ndcg / n, recall / n, mrr / n, variant_started.elapsed().as_secs_f64() * 1000.0 / n);
    }
}
