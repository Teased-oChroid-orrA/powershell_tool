//! Offline ranking experiments for `retrieval_eval --rank-experiments <outdir>` (docs/search-benchmarks.md, 2026-11-10).
//! Every variant is scored per query from the raw chunk scores of `SemanticIndex::rank_features`, so one pass over a
//! set measures them all; results go to `<outdir>/perquery.tsv` (set, query number, id, query terms, variant, nDCG@10,
//! Recall@100) for pooled and held-out analysis, and the candidates of the default ranking to `<outdir>/candidates.tsv`
//! for fitting. `--weights <file>` adds the cross-fitted `learned` variant (fold k is scored with the weights trained
//! on the other fold).
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

use search_core::lexical::{self, LexicalConfig};
use search_core::ranking::{jaccard, shingles, FeatureSet};
use search_core::semantic_index::SemanticIndex;

type Qrels = HashMap<String, HashMap<String, f64>>;

pub struct Inputs<'a> {
    pub set: &'a str,
    pub index: &'a SemanticIndex,
    pub queries: &'a [(String, String)],
    pub qrels: &'a Qrels,
    /// Document id -> title (empty where the set has none).
    pub titles: &'a HashMap<String, String>,
    pub outdir: &'a Path,
    pub weights: Option<&'a Path>,
}

fn minmax(scores: &[f32]) -> Vec<f32> {
    let (low, high) = scores.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), &x| (l.min(x), h.max(x)));
    let span = (high - low).max(f32::EPSILON);
    scores.iter().map(|&x| (x - low) / span).collect()
}

fn doc_id(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn ndcg_at(ranked: &[String], gains: &HashMap<String, f64>, k: usize) -> f64 {
    let dcg = |gains: &mut dyn Iterator<Item = f64>| gains.take(k).enumerate().map(|(i, g)| (2f64.powf(g) - 1.0) / ((i + 2) as f64).log2()).sum::<f64>();
    let got = dcg(&mut ranked.iter().map(|d| gains.get(d).copied().unwrap_or(0.0)));
    let mut ideal: Vec<f64> = gains.values().copied().collect();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let best = dcg(&mut ideal.into_iter());
    if best > 0.0 { got / best } else { 0.0 }
}

fn recall_at(ranked: &[String], gains: &HashMap<String, f64>, k: usize) -> f64 {
    ranked.iter().take(k).filter(|d| gains.contains_key(*d)).count() as f64 / gains.len() as f64
}

/// Files in order of their best chunk (ties: similarity, then row id), the harness' ranking.
fn rank_docs(fs: &FeatureSet, scores: &[f32]) -> Vec<String> {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| {
        scores[b].total_cmp(&scores[a]).then_with(|| fs.chunks[b].dense.total_cmp(&fs.chunks[a].dense)).then_with(|| fs.chunks[a].rowid.cmp(&fs.chunks[b].rowid))
    });
    let mut seen = HashSet::new();
    order.into_iter().filter(|&i| seen.insert(fs.chunks[i].doc)).map(|i| doc_id(&fs.docs[fs.chunks[i].doc as usize])).collect()
}

/// Files ranked by a document score computed from each file's chunk scores (best first).
fn rank_docs_by(fs: &FeatureSet, scores: &[f32], doc_score: impl Fn(&[f32]) -> f32) -> Vec<String> {
    let mut per_doc: HashMap<u32, Vec<f32>> = HashMap::new();
    for (c, &s) in fs.chunks.iter().zip(scores) {
        per_doc.entry(c.doc).or_default().push(s);
    }
    let mut docs: Vec<(u32, f32)> = per_doc
        .into_iter()
        .map(|(d, mut v)| {
            v.sort_by(|a, b| b.total_cmp(a));
            (d, doc_score(&v))
        })
        .collect();
    docs.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    docs.into_iter().map(|(d, _)| doc_id(&fs.docs[d as usize])).collect()
}

fn fuse(dn: &[f32], bn: &[f32], share: f32) -> Vec<f32> {
    dn.iter().zip(bn).map(|(d, b)| (1.0 - share) * d + share * b).collect()
}

/// Features of the cross-fitted linear model; the fit lives in the analysis script, the evaluation here.
pub fn features(dn: f32, bn: f32, terms: usize, length: u32, ordinal: u32, chunks: u32) -> [f32; 8] {
    let q = (terms.min(24) as f32) / 24.0;
    [dn, bn, dn * q, bn * q, (1.0 + length as f32).ln() / 6.0, (ordinal == 0) as u8 as f32, ordinal as f32 / (chunks.max(2) - 1) as f32, (bn > 0.0) as u8 as f32]
}

pub fn run(inputs: &Inputs) {
    std::fs::create_dir_all(inputs.outdir).unwrap();
    let open = |name: &str| std::fs::OpenOptions::new().create(true).append(true).open(inputs.outdir.join(name)).unwrap();
    let (out, mut cand) = (std::cell::RefCell::new(open("perquery.tsv")), open("candidates.tsv"));
    let learned: Option<[Vec<f32>; 2]> = inputs.weights.map(|p| {
        let text = std::fs::read_to_string(p).expect("weights");
        let mut folds = [Vec::new(), Vec::new()];
        for line in text.lines() {
            let mut cols = line.split_whitespace();
            let fold: usize = cols.next().and_then(|f| f.parse().ok()).expect("fold");
            folds[fold] = cols.map(|w| w.parse().expect("weight")).collect();
        }
        folds
    });
    let config = LexicalConfig::default();
    // Title terms per file, for the heading-boost variant.
    let title_terms: Vec<(String, Vec<String>)> = inputs.titles.iter().filter(|(_, t)| !t.trim().is_empty()).map(|(id, t)| (id.clone(), lexical::terms_with(t, true))).collect();
    let shares: Vec<f32> = (0..=16).map(|i| i as f32 * 0.05).collect();
    let mut cost = (0.0f64, 0.0f64, 0usize);
    for (qi, (qid, text)) in inputs.queries.iter().enumerate() {
        let gains = &inputs.qrels[qid];
        let started = std::time::Instant::now();
        let fs = inputs.index.rank_features(text, true).expect("features").expect("snapshot");
        let (dense, bm25): (Vec<f32>, Vec<f32>) = fs.chunks.iter().map(|c| (c.dense, c.bm25)).unzip();
        let (dn, bn) = (minmax(&dense), minmax(&bm25));
        let base_scores = fuse(&dn, &bn, config.weight);
        cost.0 += started.elapsed().as_secs_f64();
        let terms = fs.query_terms;
        let record = |variant: &str, ranked: &[String]| {
            writeln!(out.borrow_mut(), "{}\t{qi}\t{qid}\t{terms}\t{variant}\t{:.6}\t{:.6}", inputs.set, ndcg_at(ranked, gains, 10), recall_at(ranked, gains, 100)).unwrap();
        };
        let default_ranked = rank_docs(&fs, &base_scores);
        record("default", &default_ranked);
        // 1. keyword share grid (dense-only queries without terms keep share 0 behaviour: bn is all zero).
        for &s in &shares {
            record(&format!("share{s:.2}"), &rank_docs(&fs, &fuse(&dn, &bn, s)));
        }
        // 1b. rarity-aware keyword share (P1-5): share = a + b * r, r = mean or max of the query terms' idf over ln(2n + 1)
        // (0 = every term in nearly every chunk, 1 = no chunk has the term); queries without terms keep the default.
        let top_idf = ((2 * fs.chunks.len() + 1) as f32).ln();
        let rarity = |max: bool| -> f32 {
            if fs.term_idf.is_empty() { return 0.0; }
            let v = fs.term_idf.iter().map(|x| x / top_idf);
            if max { v.fold(0.0, f32::max) } else { v.sum::<f32>() / fs.term_idf.len() as f32 }
        };
        for (name, max, a, b) in [
            ("rar-mean-0.15-0.40", false, 0.15f32, 0.40f32), ("rar-mean-0.20-0.30", false, 0.20, 0.30), ("rar-mean-0.25-0.20", false, 0.25, 0.20),
            ("rar-mean-0.45-neg0.20", false, 0.45, -0.20), ("rar-mean-0.55-neg0.40", false, 0.55, -0.40),
            ("rar-max-0.15-0.30", true, 0.15, 0.30), ("rar-max-0.25-0.15", true, 0.25, 0.15), ("rar-max-0.45-neg0.15", true, 0.45, -0.15),
        ] {
            let share = if terms == 0 { config.weight } else { (a + b * rarity(max)).clamp(0.0, 1.0) };
            record(name, &rank_docs(&fs, &fuse(&dn, &bn, share)));
        }
        writeln!(out.borrow_mut(), "{}\t{qi}\t{qid}\t{terms}\trarity-mean\t{:.6}\t{:.6}", inputs.set, rarity(false), rarity(true)).unwrap();
        // 2. document-level scoring.
        for (alpha, k) in [(0.1f32, 1usize), (0.25, 1), (0.5, 1), (0.25, 2), (0.5, 2)] {
            let t = Instant::now();
            let ranked = rank_docs_by(&fs, &base_scores, |v| {
                v[0] + (1..=k).filter_map(|j| v.get(j).map(|s| alpha.powi(j as i32) * s)).sum::<f32>()
            });
            cost.1 += t.elapsed().as_secs_f64();
            cost.2 += 1;
            record(&format!("doc-a{alpha}-k{k}"), &ranked);
        }
        // 3. title boost (proxy for a heading or file-name boost).
        if !title_terms.is_empty() && terms > 0 {
            let query = lexical::query_terms_or_empty(text);
            let docs: Vec<(Vec<u16>, u32)> = title_terms.iter().map(|(_, t)| {
                let counts = query.iter().map(|q| t.iter().filter(|x| *x == q).count() as u16).collect();
                (counts, t.len() as u32)
            }).collect();
            let tb = lexical::bm25_with(&docs, query.len(), config.bm25_k1, config.bm25_b);
            let top = tb.iter().cloned().fold(0.0f32, f32::max);
            let by_doc: HashMap<&str, f32> = title_terms.iter().zip(&tb).map(|((id, _), s)| (id.as_str(), if top > 0.0 { s / top } else { 0.0 })).collect();
            let boost: Vec<f32> = fs.chunks.iter().map(|c| by_doc.get(doc_id(&fs.docs[c.doc as usize]).as_str()).copied().unwrap_or(0.0)).collect();
            for beta in [0.05f32, 0.1, 0.2, 0.4] {
                let scores: Vec<f32> = base_scores.iter().zip(&boost).map(|(s, b)| s + beta * b).collect();
                record(&format!("title-{beta}"), &rank_docs(&fs, &scores));
            }
        } else {
            for beta in [0.05f32, 0.1, 0.2, 0.4] {
                record(&format!("title-{beta}"), &default_ranked);
            }
        }
        // 4. near-duplicate removal among the first 100 passages.
        {
            let hits = inputs.index.search_tuned(text, 100, true, &config).expect("search");
            let sets: Vec<HashSet<u64>> = hits.iter().map(|h| shingles(&h.excerpt)).collect();
            let plain: Vec<String> = dedup_docs(hits.iter().map(|h| doc_id(&h.path)));
            record("dedup-none", &plain);
            for (name, same_file_only, threshold) in [("j0.5", false, 0.5f32), ("j0.7", false, 0.7), ("j0.9", false, 0.9), ("file-j0.7", true, 0.7)] {
                let t = Instant::now();
                let mut kept: Vec<usize> = Vec::new();
                let mut dropped_top30 = 0;
                for i in 0..hits.len() {
                    let duplicate = kept.iter().any(|&k| (!same_file_only || hits[k].path == hits[i].path) && jaccard(&sets[i], &sets[k]) >= threshold);
                    if duplicate {
                        if i < 30 { dropped_top30 += 1; }
                    } else {
                        kept.push(i);
                    }
                }
                cost.1 += t.elapsed().as_secs_f64();
                let ranked = dedup_docs(kept.iter().map(|&i| doc_id(&hits[i].path)));
                record(&format!("dedup-{name}"), &ranked);
                writeln!(out.borrow_mut(), "{}\t{qi}\t{qid}\t{terms}\tdropped-{name}\t{dropped_top30}\t0", inputs.set).unwrap();
            }
            // The evidence brief (3 passages of the best 20): distinct relevant files in it, as a share of 3.
            for (name, limit) in [("none", None), ("j0.5", Some(0.5f32)), ("j0.7", Some(0.7)), ("j0.9", Some(0.9))] {
                let brief = search_core::evidence::select_evidence_with(&hits[..hits.len().min(20)], limit);
                let relevant: HashSet<String> = brief.iter().map(|h| doc_id(&h.path)).filter(|id| gains.contains_key(id)).collect();
                writeln!(out.borrow_mut(), "{}\t{qi}\t{qid}\t{terms}\tbrief-{name}\t{:.6}\t{}", inputs.set, relevant.len() as f64 / 3.0, brief.len()).unwrap();
            }
        }
        // 5. learned fusion: dump the default top 100 chunks, and score with cross-fitted weights when given.
        let mut order: Vec<usize> = (0..base_scores.len()).collect();
        order.sort_by(|&a, &b| base_scores[b].total_cmp(&base_scores[a]).then_with(|| fs.chunks[a].rowid.cmp(&fs.chunks[b].rowid)));
        for &i in order.iter().take(100) {
            let c = &fs.chunks[i];
            let id = doc_id(&fs.docs[c.doc as usize]);
            let f = features(dn[i], bn[i], terms, c.length, c.ordinal, c.chunks_in_doc);
            let gain = gains.get(&id).copied().unwrap_or(0.0);
            let cols: Vec<String> = f.iter().map(|x| format!("{x:.6}")).collect();
            writeln!(cand, "{}\t{qi}\t{qid}\t{}\t{}\t{}\t{gain}\t{}", inputs.set, c.doc, c.rowid, base_scores[i], cols.join("\t")).unwrap();
        }
        if let Some(folds) = &learned {
            let w = &folds[1 - qi % 2];
            let scores: Vec<f32> = fs.chunks.iter().enumerate().map(|(i, c)| {
                let f = features(dn[i], bn[i], terms, c.length, c.ordinal, c.chunks_in_doc);
                f.iter().zip(w).map(|(x, w)| x * w).sum()
            }).collect();
            record("learned", &rank_docs(&fs, &scores));
        }
    }
    let n = inputs.queries.len() as f64;
    eprintln!("{}: features+fusion {:.1} ms/query; doc-level and dedup extras {:.2} ms/query", inputs.set, cost.0 * 1000.0 / n, cost.1 * 1000.0 / n);
}

use std::time::Instant;

fn dedup_docs(ids: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.filter(|id| seen.insert(id.clone())).collect()
}
