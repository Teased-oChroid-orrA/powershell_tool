//! Learned sparse retrieval (SPLADE) against the shipped ranking, on a BEIR-format folder (issue #240 P1-8,
//! `docs/search-benchmarks-splade.md`).
//!
//!   cargo run -p search-core --release --features semantic-search --example splade_eval -- <folder> --model <dir> [--queries N]
//!
//! `<dir>` holds `model.onnx` (a BERT masked-language-model head: inputs `input_ids`, `input_mask`, `segment_ids`, output
//! token logits) and `tokenizer.json`. A text becomes a sparse vector: for every vocabulary term the maximum over the
//! text's tokens of `ln(1 + relu(logit))` (SPLADE-max). A document is scored by the dot product of its vector with the
//! query's. Documents are encoded whole (truncated at 256 tokens, like the dense model), not chunked.
//!
//! The shipped rankings come from the existing `<folder>/.eval-index` (built by `retrieval_eval`), through
//! `SemanticIndex::rank_features`: `bm25` is the best chunk's BM25, `default` the best chunk's min-max fused score at
//! keyword share 0.35, the same way `ranking_experiments.rs` scores them. Fusions with SPLADE are convex sums of min-max
//! normalised document scores. Document vectors are cached in `<folder>/.eval-splade-<tag>/docs.jsonl`.
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use rten::{Model, ValueView};
use search_core::semantic_index::SemanticIndex;
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

const MAX_TOKENS: usize = 256;
const DOC_BATCH: usize = 8;
const QUERY_BATCH: usize = 16;

type Sparse = Vec<(u32, f32)>;

struct Encoder {
    model: Model,
    tokenizer: Tokenizer,
    ids: rten::NodeId,
    mask: rten::NodeId,
    types: rten::NodeId,
    output: rten::NodeId,
}

impl Encoder {
    fn load(dir: &Path) -> Self {
        let model = Model::load(std::fs::read(dir.join("model.onnx")).expect("model.onnx")).expect("load model");
        let mut tokenizer = Tokenizer::from_bytes(std::fs::read(dir.join("tokenizer.json")).expect("tokenizer.json")).expect("tokenizer");
        tokenizer.with_truncation(Some(TruncationParams { max_length: MAX_TOKENS, ..Default::default() })).unwrap();
        tokenizer.with_padding(Some(PaddingParams::default()));
        let node = |name: &str| model.node_id(name).unwrap_or_else(|e| panic!("input {name}: {e}"));
        let (ids, mask, types) = (node("input_ids"), node("input_mask"), node("segment_ids"));
        let output = *model.output_ids().first().expect("output");
        Self { model, tokenizer, ids, mask, types, output }
    }

    fn encode(&self, texts: &[&str]) -> Vec<Sparse> {
        let encodings = self.tokenizer.encode_batch(texts.to_vec(), true).expect("tokenize");
        let (batch, seq) = (encodings.len(), encodings[0].len());
        let flat = |f: &dyn Fn(&tokenizers::Encoding) -> &[u32]| -> Vec<i32> { encodings.iter().flat_map(|e| f(e).iter().map(|&v| v as i32)).collect() };
        let (ids, mask, types) = (flat(&|e| e.get_ids()), flat(&|e| e.get_attention_mask()), flat(&|e| e.get_type_ids()));
        fn view(v: &[i32], shape: [usize; 2]) -> ValueView<'_> {
            ValueView::from_shape(shape, v).unwrap()
        }
        let out = self
            .model
            .run(vec![(self.ids, view(&ids, [batch, seq]).into()), (self.mask, view(&mask, [batch, seq]).into()), (self.types, view(&types, [batch, seq]).into())], &[self.output], None)
            .expect("run")
            .into_iter()
            .next()
            .unwrap();
        let (shape, logits) = out.into_shape_vec::<f32, 3>().expect("logits");
        let vocab = shape[2];
        assert_eq!(&shape[..2], &[batch, seq]);
        let mut result = Vec::with_capacity(batch);
        for b in 0..batch {
            let mut best = vec![0f32; vocab];
            for t in 0..seq {
                if mask[b * seq + t] == 0 {
                    continue;
                }
                for (slot, &x) in best.iter_mut().zip(&logits[(b * seq + t) * vocab..(b * seq + t + 1) * vocab]) {
                    if x > 0.0 {
                        *slot = slot.max((1.0 + x).ln());
                    }
                }
            }
            result.push(best.iter().enumerate().filter(|(_, &w)| w > 0.0).map(|(i, &w)| (i as u32, w)).collect());
        }
        result
    }
}

fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).expect("json line")).collect()
}

fn ndcg_at(ranked: &[String], gains: &HashMap<String, f64>, k: usize) -> f64 {
    let dcg = |gains: &mut dyn Iterator<Item = f64>| gains.take(k).enumerate().map(|(i, g)| (2f64.powf(g) - 1.0) / ((i + 2) as f64).log2()).sum::<f64>();
    let got = dcg(&mut ranked.iter().map(|d| gains.get(d).copied().unwrap_or(0.0)));
    let mut ideal: Vec<f64> = gains.values().copied().collect();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let best = dcg(&mut ideal.into_iter());
    if best > 0.0 { got / best } else { 0.0 }
}

fn minmax(scores: &[f32]) -> Vec<f32> {
    let (low, high) = scores.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), &x| (l.min(x), h.max(x)));
    let span = (high - low).max(f32::EPSILON);
    scores.iter().map(|&x| (x - low) / span).collect()
}

/// Document ids, best score first (ties: corpus order).
fn rank(ids: &[String], scores: &[f32]) -> Vec<String> {
    let mut order: Vec<usize> = (0..ids.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));
    order.into_iter().take(100).map(|i| ids[i].clone()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let folder = Path::new(&args[1]);
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let max_queries: usize = flag("--queries").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let model_dir = Path::new(flag("--model").expect("--model <dir>"));
    let tag = model_dir.file_name().unwrap().to_string_lossy().into_owned();

    let corpus = read_jsonl(&folder.join("corpus.jsonl"));
    let mut qrels: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for line in std::fs::read_to_string(folder.join("qrels/test.tsv")).expect("qrels").lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        let gain: f64 = cols.get(2).and_then(|g| g.trim().parse().ok()).unwrap_or(0.0);
        if cols.len() >= 3 && gain > 0.0 {
            qrels.entry(cols[0].to_string()).or_default().insert(cols[1].to_string(), gain);
        }
    }
    let queries: Vec<(String, String)> = read_jsonl(&folder.join("queries.jsonl"))
        .into_iter()
        .filter_map(|q| Some((q["_id"].as_str()?.to_string(), q["text"].as_str()?.to_string())))
        .filter(|(id, _)| qrels.contains_key(id))
        .take(max_queries)
        .collect();
    // Same text as `retrieval_eval` indexes.
    let docs: Vec<(String, String)> = corpus
        .iter()
        .filter_map(|d| {
            let text = format!("{} {}", d["title"].as_str().unwrap_or(""), d["text"].as_str().unwrap_or("")).trim().to_string();
            (!text.is_empty()).then(|| (d["_id"].as_str().unwrap_or_default().to_string(), text))
        })
        .collect();

    let encoder = Encoder::load(model_dir);
    // Document vectors: cached after the first run, with the time the encoding took.
    let cache_dir = folder.join(format!(".eval-splade-{tag}"));
    let cache = cache_dir.join("docs.jsonl");
    let (doc_vectors, doc_secs): (Vec<Sparse>, f64) = if cache.is_file() {
        let lines: Vec<serde_json::Value> = read_jsonl(&cache);
        let secs = lines[0]["encode_seconds"].as_f64().unwrap();
        let vectors = lines[1..].iter().map(|l| serde_json::from_value(l["v"].clone()).unwrap()).collect();
        (vectors, secs)
    } else {
        let started = Instant::now();
        let mut order: Vec<usize> = (0..docs.len()).collect();
        // Similar lengths in a batch keep padding small.
        order.sort_by_key(|&i| docs[i].1.len());
        let mut vectors: Vec<Sparse> = vec![Vec::new(); docs.len()];
        for (n, chunk) in order.chunks(DOC_BATCH).enumerate() {
            let texts: Vec<&str> = chunk.iter().map(|&i| docs[i].1.as_str()).collect();
            for (&i, v) in chunk.iter().zip(encoder.encode(&texts)) {
                vectors[i] = v;
            }
            if n % 50 == 0 {
                eprintln!("  encoded {} of {} ({:.0}s)", n * DOC_BATCH, docs.len(), started.elapsed().as_secs_f64());
            }
        }
        let secs = started.elapsed().as_secs_f64();
        std::fs::create_dir_all(&cache_dir).unwrap();
        let mut out = format!("{}\n", serde_json::json!({ "encode_seconds": secs }));
        for v in &vectors {
            out.push_str(&serde_json::json!({ "v": v }).to_string());
            out.push('\n');
        }
        std::fs::write(&cache, out).unwrap();
        (vectors, secs)
    };
    let postings: usize = doc_vectors.iter().map(Vec::len).sum();
    let mut inverted: HashMap<u32, Vec<(u32, f32)>> = HashMap::new();
    for (d, v) in doc_vectors.iter().enumerate() {
        for &(t, w) in v {
            inverted.entry(t).or_default().push((d as u32, w));
        }
    }
    let ids: Vec<String> = docs.iter().map(|(id, _)| id.clone()).collect();
    let position: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();

    let started = Instant::now();
    let mut query_vectors: Vec<Sparse> = Vec::new();
    for chunk in queries.chunks(QUERY_BATCH) {
        let texts: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
        query_vectors.extend(encoder.encode(&texts));
    }
    let query_ms = started.elapsed().as_secs_f64() * 1000.0 / queries.len() as f64;
    let query_terms = query_vectors.iter().map(Vec::len).sum::<usize>() as f64 / queries.len() as f64;

    let index = SemanticIndex::open(&folder.join(".eval-index").join("semantic.sqlite")).expect("open the retrieval_eval index");
    let shares = [0.2f32, 0.35, 0.5, 0.65];
    let mut names: Vec<String> = vec!["bm25".into(), "default".into(), "splade".into()];
    for s in shares {
        names.push(format!("bm25+splade{s}"));
    }
    for s in shares {
        names.push(format!("default+splade{s}"));
    }
    let mut totals: Vec<(f64, f64)> = vec![(0.0, 0.0); names.len()];
    let mut retrieval_ms = 0.0;
    for ((qid, text), qv) in queries.iter().zip(&query_vectors) {
        let gains = &qrels[qid];
        let began = Instant::now();
        let mut splade = vec![0f32; ids.len()];
        for &(t, qw) in qv {
            for &(d, dw) in inverted.get(&t).map(Vec::as_slice).unwrap_or(&[]) {
                splade[d as usize] += qw * dw;
            }
        }
        retrieval_ms += began.elapsed().as_secs_f64() * 1000.0;
        // Best chunk of each file for the shipped scores.
        let fs = index.rank_features(text, true).expect("features").expect("snapshot");
        let (dense, bm): (Vec<f32>, Vec<f32>) = fs.chunks.iter().map(|c| (c.dense, c.bm25)).unzip();
        let (dn, bn) = (minmax(&dense), minmax(&bm));
        let (mut bm25, mut default) = (vec![0f32; ids.len()], vec![0f32; ids.len()]);
        for (i, c) in fs.chunks.iter().enumerate() {
            let stem = Path::new(&fs.docs[c.doc as usize]).file_stem().unwrap().to_string_lossy().into_owned();
            let d = position[stem.as_str()];
            bm25[d] = bm25[d].max(bm[i]);
            default[d] = default[d].max(0.65 * dn[i] + 0.35 * bn[i]);
        }
        let (b, d, s) = (minmax(&bm25), minmax(&default), minmax(&splade));
        let mix = |a: &[f32], share: f32| -> Vec<f32> { a.iter().zip(&s).map(|(x, y)| (1.0 - share) * x + share * y).collect() };
        let mut lists: Vec<Vec<String>> = vec![rank(&ids, &bm25), rank(&ids, &default), rank(&ids, &splade)];
        lists.extend(shares.iter().map(|&sh| rank(&ids, &mix(&b, sh))));
        lists.extend(shares.iter().map(|&sh| rank(&ids, &mix(&d, sh))));
        for (total, ranked) in totals.iter_mut().zip(&lists) {
            total.0 += ndcg_at(ranked, gains, 10);
            total.1 += ranked.iter().filter(|x| gains.contains_key(*x)).count() as f64 / gains.len() as f64;
        }
    }
    let n = queries.len() as f64;
    let model_mb = std::fs::metadata(model_dir.join("model.onnx")).unwrap().len() as f64 / 1e6;
    println!("{}: {} docs, {} queries, model {model_mb:.0} MB", folder.display(), docs.len(), queries.len());
    println!(
        "index: encode {doc_secs:.0} s ({:.1} docs/s), {:.0} terms/doc, {:.2} MB of postings (8 bytes each); query: encode {query_ms:.1} ms ({query_terms:.0} terms), score {:.2} ms",
        docs.len() as f64 / doc_secs,
        postings as f64 / docs.len() as f64,
        postings as f64 * 8.0 / 1e6,
        retrieval_ms / n
    );
    println!("{:<20} {:>8} {:>10}", "variant", "nDCG@10", "Recall@100");
    for (name, (ndcg, recall)) in names.iter().zip(&totals) {
        println!("{:<20} {:>8.4} {:>10.4}", name, ndcg / n, recall / n);
    }
}
