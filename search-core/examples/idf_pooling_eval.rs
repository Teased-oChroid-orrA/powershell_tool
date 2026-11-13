//! P1-6 experiment (docs/search-benchmarks.md, 2026-11-10): does weighting each token's vector by its rarity when
//! pooling a passage (instead of the plain mean) improve retrieval? Own chunking (120-word windows, split further when
//! over the model limit), so every variant, including the uniform control, shares one candidate set; it does not
//! reproduce the index's chunker. Token weight is per-set corpus idf, an upper bound: stored vectors cannot depend on
//! a corpus that changes, so a static weight would do no better.
//!
//!   cargo run -p search-core --release --features semantic-search --example idf_pooling_eval -- <folder> [--queries N]
use std::collections::{HashMap, HashSet};
use std::path::Path;

use search_core::lexical;

fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn ndcg10(ranked: &[String], gains: &HashMap<String, f64>) -> f64 {
    let dcg = |g: &mut dyn Iterator<Item = f64>| g.take(10).enumerate().map(|(i, g)| (2f64.powf(g) - 1.0) / ((i + 2) as f64).log2()).sum::<f64>();
    let got = dcg(&mut ranked.iter().map(|d| gains.get(d).copied().unwrap_or(0.0)));
    let mut ideal: Vec<f64> = gains.values().copied().collect();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let best = dcg(&mut ideal.into_iter());
    if best > 0.0 { got / best } else { 0.0 }
}

fn windows(text: &str, words: usize) -> Vec<String> {
    let w: Vec<&str> = text.split_whitespace().collect();
    if w.is_empty() { return Vec::new(); }
    w.chunks(words).map(|c| c.join(" ")).collect()
}

/// Token vectors with their text, splitting the passage in halves until the model accepts it.
fn tokens(text: &str) -> Vec<(String, Vec<f32>)> {
    match search_ml::embed_tokens(text) {
        Ok(t) => t.into_iter().map(|t| (text[t.byte_range].to_lowercase(), t.embedding.values().to_vec())).collect(),
        Err(_) => {
            let w: Vec<&str> = text.split_whitespace().collect();
            if w.len() < 2 { return Vec::new(); }
            let (a, b) = w.split_at(w.len() / 2);
            let mut out = tokens(&a.join(" "));
            out.extend(tokens(&b.join(" ")));
            out
        }
    }
}

fn pool(tokens: &[(String, Vec<f32>)], weight: impl Fn(&str) -> f32) -> Option<Vec<f32>> {
    let mut v = vec![0f32; 384];
    for (t, e) in tokens {
        let w = weight(t);
        v.iter_mut().zip(e).for_each(|(s, x)| *s += w * x);
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (n.is_finite() && n > 1e-9).then(|| { v.iter_mut().for_each(|x| *x /= n); v })
}

fn minmax(s: &[f32]) -> Vec<f32> {
    let (lo, hi) = s.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), &x| (l.min(x), h.max(x)));
    let span = (hi - lo).max(f32::EPSILON);
    s.iter().map(|&x| (x - lo) / span).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let folder = Path::new(&args[1]);
    let max_q: usize = args.iter().position(|a| a == "--queries").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let mut qrels: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for line in std::fs::read_to_string(folder.join("qrels/test.tsv")).unwrap().lines().skip(1) {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() >= 3 && c[2].trim().parse::<f64>().unwrap_or(0.0) > 0.0 {
            qrels.entry(c[0].to_string()).or_default().insert(c[1].to_string(), c[2].trim().parse().unwrap());
        }
    }
    let queries: Vec<(String, String)> = read_jsonl(&folder.join("queries.jsonl")).into_iter()
        .filter_map(|q| Some((q["_id"].as_str()?.to_string(), q["text"].as_str()?.to_string())))
        .filter(|(id, _)| qrels.contains_key(id)).take(max_q).collect();

    // Chunks and their token vectors.
    let started = std::time::Instant::now();
    let (mut ids, mut texts, mut toks): (Vec<String>, Vec<String>, Vec<Vec<(String, Vec<f32>)>>) = Default::default();
    for d in read_jsonl(&folder.join("corpus.jsonl")) {
        let id = d["_id"].as_str().unwrap_or_default().to_string();
        let text = format!("{} {}", d["title"].as_str().unwrap_or(""), d["text"].as_str().unwrap_or(""));
        for c in windows(&text, 120) {
            let t = tokens(&c);
            if !t.is_empty() { ids.push(id.clone()); texts.push(c); toks.push(t); }
        }
    }
    eprintln!("{}: {} chunks embedded in {:.0}s", folder.display(), ids.len(), started.elapsed().as_secs_f64());
    let n = ids.len() as f32;
    let mut df: HashMap<&str, f32> = HashMap::new();
    for t in &toks {
        for k in t.iter().map(|(k, _)| k.as_str()).collect::<HashSet<_>>() { *df.entry(k).or_default() += 1.0; }
    }
    let idf = |t: &str, p: f32| (1.0 + n / df.get(t).copied().unwrap_or(1.0)).ln().powf(p);

    // Keyword side: BM25 of the query terms over the chunks, as search does.
    let bm25s: Vec<Vec<f32>> = queries.iter().map(|(_, text)| match lexical::query_terms(text) {
        Some(t) => {
            let docs: Vec<(Vec<u16>, u32)> = texts.iter().map(|c| lexical::count_terms(c, &t)).collect();
            lexical::bm25(&docs, t.len())
        }
        None => vec![0.0; texts.len()],
    }).collect();
    let q_toks: Vec<Vec<(String, Vec<f32>)>> = queries.iter().map(|(_, q)| tokens(q)).collect();
    println!("{:<16} {:>9} {:>9}", "variant", "dense", "fused0.35");
    // (name, passage exponent, query exponent): exponent 0 = plain mean of the normalised token vectors (control).
    for (name, pp, qp) in [("uniform", 0.0f32, 0.0f32), ("idf1-both", 1.0, 1.0), ("idf1-passage", 1.0, 0.0), ("idf0.5-both", 0.5, 0.5), ("idf2-both", 2.0, 2.0)] {
        let pooled: Vec<Option<Vec<f32>>> = toks.iter().map(|t| pool(t, |k| idf(k, pp))).collect();
        let (mut dense_sum, mut fused_sum) = (0.0, 0.0);
        for (qi, (qid, _)) in queries.iter().enumerate() {
            let Some(qv) = pool(&q_toks[qi], |k| idf(k, qp)) else { continue };
            let dense: Vec<f32> = pooled.iter().map(|p| p.as_ref().map_or(-1.0, |p| p.iter().zip(&qv).map(|(a, b)| a * b).sum())).collect();
            let bm25 = &bm25s[qi];
            let fused: Vec<f32> = minmax(&dense).iter().zip(minmax(bm25)).map(|(d, b)| 0.65 * d + 0.35 * b).collect();
            let rank = |s: &[f32]| {
                let mut o: Vec<usize> = (0..s.len()).collect();
                o.sort_by(|&a, &b| s[b].total_cmp(&s[a]).then(a.cmp(&b)));
                let mut seen = HashSet::new();
                o.into_iter().filter_map(|i| seen.insert(&ids[i]).then(|| ids[i].clone())).take(10).collect::<Vec<_>>()
            };
            dense_sum += ndcg10(&rank(&dense), &qrels[qid]);
            fused_sum += ndcg10(&rank(&fused), &qrels[qid]);
        }
        let q = queries.len() as f64;
        println!("{:<16} {:>9.4} {:>9.4}", name, dense_sum / q, fused_sum / q);
    }
}
