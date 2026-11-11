//! Relationship edges on a BEIR-format retrieval set (#240 P3-1; docs/search-benchmarks.md, 2026-11-11).
//! Builds the edges the catalog would hold for the set's documents (`catalog_edges::compute_edges` over the
//! entities `Catalog::index_entities` would store and the SHA-256 of each text), then asks two questions:
//! how often is an edge's other end relevant to a common query (a proxy, not a label), and does expanding the
//! query with terms of files related to its top hits change nDCG@10 under the default ranking.
//!
//!   cargo run -p search-core --release --features semantic-search --example edges_eval -- <folder> [--queries N] [--top K]
//!
//! The set's `.eval-index` must exist (built by `retrieval_eval`).
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use search_core::catalog_edges::{compute_edges, entity_rows, Edge, EdgeType, FileRow};
use search_core::lexical::LexicalConfig;
use search_core::semantic_index::SemanticIndex;
use sha2::{Digest, Sha256};

fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("json line"))
        .collect()
}

fn ndcg_at(ranked: &[String], gains: &HashMap<String, f64>, k: usize) -> f64 {
    let dcg = |gains: &mut dyn Iterator<Item = f64>| gains.take(k).enumerate().map(|(i, g)| (2f64.powf(g) - 1.0) / ((i + 2) as f64).log2()).sum::<f64>();
    let got = dcg(&mut ranked.iter().map(|d| gains.get(d).copied().unwrap_or(0.0)));
    let mut ideal: Vec<f64> = gains.values().copied().collect();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let best = dcg(&mut ideal.into_iter());
    if best > 0.0 { got / best } else { 0.0 }
}

fn terms(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| w.len() >= 3 && w.chars().next().is_some_and(|c| c.is_ascii_alphabetic())).map(String::from).collect()
}

fn id_of(path: &str) -> &str {
    Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or_default()
}

struct Ctx<'a> {
    index: &'a SemanticIndex,
    config: LexicalConfig,
    qrels: &'a HashMap<String, HashMap<String, f64>>,
}

impl Ctx<'_> {
    fn ranked(&self, query: &str) -> Vec<String> {
        let hits = self.index.search_tuned(query, 600, true, &self.config).expect("search");
        let mut out: Vec<String> = Vec::new();
        for hit in hits {
            let id = id_of(&hit.path).to_string();
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let folder = Path::new(&args[1]);
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let max_queries: usize = flag("--queries").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let top: usize = flag("--top").and_then(|v| v.parse().ok()).unwrap_or(3);
    let corpus = read_jsonl(&folder.join("corpus.jsonl"));
    let mut qrels: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for line in std::fs::read_to_string(folder.join("qrels/test.tsv")).expect("qrels").lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() >= 3 && cols[2].trim().parse::<f64>().unwrap_or(0.0) > 0.0 {
            qrels.entry(cols[0].to_string()).or_default().insert(cols[1].to_string(), cols[2].trim().parse().unwrap());
        }
    }
    let queries: Vec<(String, String)> = read_jsonl(&folder.join("queries.jsonl"))
        .into_iter()
        .filter_map(|q| Some((q["_id"].as_str()?.to_string(), q["text"].as_str()?.to_string())))
        .filter(|(id, _)| qrels.contains_key(id))
        .take(max_queries)
        .collect();

    // Graph over the corpus documents.
    let mut text_of: BTreeMap<String, String> = BTreeMap::new();
    for doc in &corpus {
        let id = doc["_id"].as_str().unwrap_or_default().to_string();
        let text = format!("{} {}", doc["title"].as_str().unwrap_or(""), doc["text"].as_str().unwrap_or("")).trim().to_string();
        if !text.is_empty() {
            text_of.insert(id, text);
        }
    }
    let key = |id: &str| format!("docs/{id}.txt");
    let files: Vec<FileRow> = text_of
        .iter()
        .map(|(id, t)| FileRow { path: key(id), size: t.len() as i64, modified: 1, content_hash: Some(Sha256::digest(t.as_bytes()).to_vec()) })
        .collect();
    let entities: Vec<_> = text_of.iter().flat_map(|(id, t)| entity_rows(&key(id), t)).collect();
    let started = std::time::Instant::now();
    let edges: Vec<Edge> = compute_edges(&files, &entities).into_iter().filter(|e| e.kind != EdgeType::FileInFolder).collect();
    let build_ms = started.elapsed().as_secs_f64() * 1000.0;
    let set = folder.to_string_lossy().rsplit('/').find(|p| !p.is_empty() && *p != "sub").unwrap_or("set").to_string();
    let n_docs = text_of.len();
    println!("== {set}: {n_docs} docs, {} queries, {} entity rows, edges built in {build_ms:.0} ms", queries.len(), entities.len());

    // Proxy label: two documents are co-relevant when some query judges both relevant.
    let mut co: BTreeSet<(String, String)> = BTreeSet::new();
    let mut relevant_docs: BTreeSet<&str> = BTreeSet::new();
    for gains in qrels.values() {
        let ids: Vec<&String> = gains.keys().collect();
        for i in 0..ids.len() {
            relevant_docs.insert(ids[i]);
            for j in i + 1..ids.len() {
                let (a, b) = if ids[i] <= ids[j] { (ids[i], ids[j]) } else { (ids[j], ids[i]) };
                co.insert((a.clone(), b.clone()));
            }
        }
    }
    let judged = relevant_docs.len() as f64;
    let base_rate = co.len() as f64 / (judged * (judged - 1.0) / 2.0).max(1.0);
    println!("co-relevance proxy: {} judged-relevant docs, {} co-relevant pairs, base rate among judged pairs {:.4}", relevant_docs.len(), co.len(), base_rate);
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &edges {
        *counts.entry(e.kind.as_str()).or_default() += 1;
    }
    println!("edges by type: {counts:?}");
    for (label, floor) in [("SAME_ENTITY all", 0.0), ("SAME_ENTITY >=0.5", 0.5), ("SAME_ENTITY >=0.9", 0.9)] {
        let chosen: Vec<&Edge> = edges.iter().filter(|e| e.kind == EdgeType::SameEntity && e.confidence >= floor).collect();
        // Only edges between two judged-relevant docs can be scored by the proxy.
        let scorable: Vec<&&Edge> = chosen.iter().filter(|e| relevant_docs.contains(id_of(&e.src)) && relevant_docs.contains(id_of(&e.dst))).collect();
        let hit = scorable.iter().filter(|e| {
            let (a, b) = (id_of(&e.src).to_string(), id_of(&e.dst).to_string());
            co.contains(&if a <= b { (a, b) } else { (b, a) })
        }).count();
        println!("{label:<18} edges {:>6}  between two judged docs {:>5}  co-relevant {:>5}  ({:.3} vs base {:.4})", chosen.len(), scorable.len(), hit, if scorable.is_empty() { 0.0 } else { hit as f64 / scorable.len() as f64 }, base_rate);
    }
    for kind in ["DUPLICATE_OF", "VERSION_OF"] {
        let n = edges.iter().filter(|e| e.kind.as_str() == kind).count();
        println!("{kind}: {n} edges");
    }

    // Query expansion.
    let index = SemanticIndex::open(&folder.join(".eval-index/semantic.sqlite")).expect("open index");
    let ctx = Ctx { index: &index, config: LexicalConfig::default(), qrels: &qrels };
    let mut df: HashMap<String, usize> = HashMap::new();
    for t in text_of.values() {
        for w in terms(t).into_iter().collect::<BTreeSet<_>>() {
            *df.entry(w).or_default() += 1;
        }
    }
    let mut neighbours: HashMap<&str, Vec<(&Edge, &str)>> = HashMap::new();
    for e in &edges {
        neighbours.entry(&e.src).or_default().push((e, &e.dst));
        neighbours.entry(&e.dst).or_default().push((e, &e.src));
    }
    struct Variant { name: &'static str, kinds: &'static [EdgeType], floor: f64, control: bool }
    let variants = [
        Variant { name: "graph-facts (dup,version)", kinds: &[EdgeType::DuplicateOf, EdgeType::VersionOf], floor: 0.0, control: false },
        Variant { name: "graph-entity>=0.9", kinds: &[EdgeType::SameEntity], floor: 0.9, control: false },
        Variant { name: "graph-entity>=0.5", kinds: &[EdgeType::SameEntity], floor: 0.5, control: false },
        Variant { name: "graph-all>=0.5", kinds: &[EdgeType::SameEntity, EdgeType::DuplicateOf, EdgeType::VersionOf], floor: 0.5, control: false },
        Variant { name: "control: top-hit terms", kinds: &[], floor: 0.0, control: true },
    ];
    let expansion_terms = |docs: &[&str], query: &str| -> String {
        let own: BTreeSet<String> = terms(query).into_iter().collect();
        let mut score: HashMap<String, f64> = HashMap::new();
        for d in docs {
            if let Some(t) = text_of.get(*d) {
                for w in terms(t) {
                    let f = df[&w];
                    if f >= 2 && !own.contains(&w) {
                        *score.entry(w).or_default() += (n_docs as f64 / f as f64).ln();
                    }
                }
            }
        }
        let mut ranked: Vec<(String, f64)> = score.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked.into_iter().take(8).map(|(w, _)| w).collect::<Vec<_>>().join(" ")
    };
    let mut base_scores: Vec<f64> = Vec::new();
    let mut base_ranked: Vec<Vec<String>> = Vec::new();
    for (qid, text) in &queries {
        let r = ctx.ranked(text);
        base_scores.push(ndcg_at(&r, &ctx.qrels[qid], 10));
        base_ranked.push(r);
    }
    let n = queries.len() as f64;
    let base_mean = base_scores.iter().sum::<f64>() / n;
    println!("{:<28} {:>9} {:>9} {:>9} {:>8} {:>8} {:>8}", "variant", "affected", "nDCG@10", "delta", "aff.base", "aff.delta", "up/down");
    println!("{:<28} {:>9} {:>9.4} {:>9} {:>8} {:>8} {:>8}", "default (baseline)", "-", base_mean, "-", "-", "-", "-");
    // Affected = queries where some top-hit has a neighbour under that variant's edges; the control uses the same
    // queries as the graph-all variant so the comparison is paired.
    let affected_by = |v: &Variant, ranked: &[String]| -> Vec<String> {
        let mut out = Vec::new();
        for seed in ranked.iter().take(top) {
            for (e, other) in neighbours.get(key(seed).as_str()).map(|v| v.as_slice()).unwrap_or(&[]) {
                if v.kinds.contains(&e.kind) && e.confidence >= v.floor {
                    out.push(id_of(other).to_string());
                }
            }
        }
        out.sort();
        out.dedup();
        out
    };
    let all = &variants[3];
    for v in &variants {
        let (mut sum, mut aff, mut aff_base, mut aff_sum, mut up, mut down) = (0.0, 0usize, 0.0, 0.0, 0usize, 0usize);
        for (i, (qid, text)) in queries.iter().enumerate() {
            let probe = if v.control { all } else { v };
            let related = affected_by(probe, &base_ranked[i]);
            let score = if related.is_empty() {
                base_scores[i]
            } else {
                let docs: Vec<&str> = if v.control { base_ranked[i].iter().take(top).map(|s| s.as_str()).collect() } else { related.iter().map(|s| s.as_str()).collect() };
                let extra = expansion_terms(&docs, text);
                let r = ctx.ranked(&format!("{text} {extra}"));
                let s = ndcg_at(&r, &ctx.qrels[qid], 10);
                aff += 1;
                aff_base += base_scores[i];
                aff_sum += s;
                if s > base_scores[i] + 1e-9 { up += 1 } else if s < base_scores[i] - 1e-9 { down += 1 }
                s
            };
            sum += score;
        }
        let mean = sum / n;
        let (ab, ad) = if aff > 0 { (aff_base / aff as f64, (aff_sum - aff_base) / aff as f64) } else { (0.0, 0.0) };
        println!("{:<28} {:>9} {:>9.4} {:>+9.4} {:>8.4} {:>+8.4} {:>4}/{:<3}", v.name, aff, mean, mean - base_mean, ab, ad, up, down);
    }
}
