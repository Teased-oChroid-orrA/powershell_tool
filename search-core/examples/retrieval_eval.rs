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

#[path = "support/ranking_experiments.rs"]
mod ranking_experiments;

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
        // Experiments built on `default`, handled in `main`: pseudo-relevance feedback.
        v("prf", true, LexicalConfig::default()),
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

    // `--rank-experiments <outdir>`: offline query-time ranking ideas (examples/support/ranking_experiments.rs).
    if let Some(outdir) = flag("--rank-experiments") {
        let titles: HashMap<String, String> = corpus.iter().filter_map(|d| Some((d["_id"].as_str()?.to_string(), d["title"].as_str()?.to_string()))).collect();
        let set = folder.to_string_lossy().rsplit('/').find(|p| !p.is_empty() && *p != "sub").unwrap_or("set").to_string();
        let weights = flag("--weights").map(Path::new);
        ranking_experiments::run(&ranking_experiments::Inputs { set: &set, index: &index, queries: &queries, qrels: &qrels, titles: &titles, outdir: Path::new(outdir), weights });
        return;
    }

    // `--lazy 25,50,75,100` (P2-3): enrich a second index in steps, in a fixed pseudo-random document order (the sets have
    // no folders or extensions, so priority cannot be tested; this is the case where priority carries no information),
    // and after each step report the hybrid ranking over the embedded part alone (`partial`) and a simulated combined
    // view (`+exact`): that ranking merged by reciprocal rank (k 60) with the keyword-led ranking of the not yet
    // embedded documents, taken from the fully built index. The shipped app shows the two lists separately; `+exact`
    // only estimates what a reader gets from both.
    if let Some(steps) = flag("--lazy") {
        let fractions: Vec<usize> = steps.split(',').filter_map(|v| v.parse().ok()).collect();
        let mut docs: Vec<(String, String)> = corpus
            .iter()
            .filter_map(|doc| {
                let id = doc["_id"].as_str()?.to_string();
                let text = format!("{} {}", doc["title"].as_str().unwrap_or(""), doc["text"].as_str().unwrap_or("")).trim().to_string();
                (!text.is_empty()).then_some((id, text))
            })
            .collect();
        let fnv = |id: &str| id.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        docs.sort_by_key(|(id, _)| fnv(id));
        let lazy_dir = folder.join(".eval-index-lazy");
        let _ = std::fs::remove_dir_all(&lazy_dir);
        std::fs::create_dir_all(&lazy_dir).unwrap();
        let lazy = SemanticIndex::open(&lazy_dir.join("semantic.sqlite")).expect("open lazy index");
        let ranked_ids = |hits: Vec<search_core::semantic_index::SemanticHit>| {
            let mut ranked: Vec<String> = Vec::new();
            for hit in hits {
                let id = Path::new(&hit.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if !ranked.contains(&id) {
                    ranked.push(id);
                }
            }
            ranked
        };
        let score = |ranked: &[String], gains: &HashMap<String, f64>| {
            (ndcg_at(ranked, gains, 10), ranked.iter().take(100).filter(|d| gains.contains_key(*d)).count() as f64 / gains.len() as f64)
        };
        let keyword = LexicalConfig { weight: 1000.0, ..LexicalConfig::rrf() };
        let default = LexicalConfig::default();
        println!("{:<8} {:>6} {:>9} | {:>8} {:>10} | {:>8} {:>10}", "embedded", "docs", "index s", "nDCG@10", "Recall@100", "+exact N", "+exact R");
        let (mut done, mut spent) = (0usize, 0f64);
        for pct in fractions {
            let upto = (docs.len() * pct).div_ceil(100);
            let step = std::time::Instant::now();
            for (id, text) in &docs[done..upto] {
                let path = docs_dir.join(format!("{id}.txt"));
                let metadata = std::fs::metadata(&path).unwrap();
                let modified = search_core::cache::ticks_from_modified(chrono::DateTime::<chrono::Local>::from(metadata.modified().unwrap()));
                lazy.index_document(&path.to_string_lossy(), modified, metadata.len() as i64, &[text.clone()]).expect("index document");
            }
            spent += step.elapsed().as_secs_f64();
            done = upto;
            let embedded: std::collections::HashSet<&str> = docs[..done].iter().map(|(id, _)| id.as_str()).collect();
            let (mut n, mut r, mut n2, mut r2) = (0.0, 0.0, 0.0, 0.0);
            for (qid, text) in &queries {
                let gains = &qrels[qid];
                let partial = ranked_ids(lazy.search_tuned(text, 600, true, &default).expect("search"));
                let exact_rest: Vec<String> = ranked_ids(index.search_tuned(text, 600, true, &keyword).expect("search"))
                    .into_iter()
                    .filter(|id| !embedded.contains(id.as_str()))
                    .collect();
                let mut fused: HashMap<&str, f64> = HashMap::new();
                for list in [&partial, &exact_rest] {
                    for (rank, id) in list.iter().enumerate() {
                        *fused.entry(id.as_str()).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
                    }
                }
                let mut merged: Vec<(&str, f64)> = fused.into_iter().collect();
                merged.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
                let merged: Vec<String> = merged.into_iter().map(|(id, _)| id.to_string()).collect();
                let (a, b) = score(&partial, gains);
                let (c, d) = score(&merged, gains);
                n += a; r += b; n2 += c; r2 += d;
            }
            let q = queries.len() as f64;
            println!("{:>7}% {:>6} {:>9.1} | {:>8.4} {:>10.4} | {:>8.4} {:>10.4}", pct, done, spent, n / q, r / q, n2 / q, r2 / q);
        }
        return;
    }

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
            let gains = &qrels[qid];
            ndcg += ndcg_at(&ranked, gains, 10);
            recall += ranked.iter().take(100).filter(|d| gains.contains_key(*d)).count() as f64 / gains.len() as f64;
            mrr += ranked.iter().take(10).position(|d| gains.contains_key(d)).map_or(0.0, |p| 1.0 / (p + 1) as f64);
        }
        let n = queries.len() as f64;
        println!("{:<12} {:>8.4} {:>10.4} {:>8.4} {:>10.1}", variant.name, ndcg / n, recall / n, mrr / n, variant_started.elapsed().as_secs_f64() * 1000.0 / n);
    }
}
