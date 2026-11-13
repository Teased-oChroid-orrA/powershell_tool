//! Does ordering exact-search results by relevance beat found order? Runs the
//! real exact matcher over a BEIR-format folder (`corpus.jsonl`,
//! `queries.jsonl`, `qrels/test.tsv`): each query's content words become the
//! filters, every document is split into sentence lines, and the files with a
//! hit are ordered three ways and scored with nDCG@10.
//!   cargo run --release -p search-core --example exact_order_eval -- <folder> [--queries N] [--literal]
use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::Local;
use search_core::matching::{apply_line_matching, CompiledMatchState};
use search_core::models::{FileSearchResult, FileSearchStatus, SearchSettings};
use search_core::ordering::{relevance_order, FILENAME_WEIGHT};

const STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "are", "was", "were", "from", "what", "which", "how", "does", "can", "has", "have",
    "been", "into", "their", "there", "than", "then", "its", "who", "when", "where", "why", "any", "all", "such", "not", "but", "may",
    "also", "used", "use", "using", "between", "about", "these", "those", "they", "them", "will", "would", "could", "should", "more",
    "most", "some", "other", "each", "very", "over", "only", "under", "being", "during", "while", "is", "of", "in", "to", "a", "an", "on",
    "by", "as", "at", "be", "or", "it", "do", "if", "so", "no", "we",
];

fn read_jsonl(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("json line"))
        .collect()
}

fn ndcg_at(ranked: &[&str], gains: &HashMap<String, f64>, k: usize) -> f64 {
    let dcg = |it: &mut dyn Iterator<Item = f64>| {
        it.take(k).enumerate().map(|(i, g)| (2f64.powf(g) - 1.0) / ((i + 2) as f64).log2()).sum::<f64>()
    };
    let got = dcg(&mut ranked.iter().map(|d| gains.get(*d).copied().unwrap_or(0.0)));
    let mut ideal: Vec<f64> = gains.values().copied().collect();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let best = dcg(&mut ideal.into_iter());
    if best > 0.0 { got / best } else { 0.0 }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let folder = Path::new(&args[1]);
    let max_q: usize = args.iter().position(|a| a == "--queries").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let literal = args.iter().any(|a| a == "--literal");
    let mut qrels: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for line in std::fs::read_to_string(folder.join("qrels/test.tsv")).unwrap().lines().skip(1) {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() >= 3 {
            let g: f64 = c[2].trim().parse().unwrap_or(0.0);
            if g > 0.0 {
                qrels.entry(c[0].to_string()).or_default().insert(c[1].to_string(), g);
            }
        }
    }
    let docs: Vec<(String, Vec<String>)> = read_jsonl(&folder.join("corpus.jsonl"))
        .into_iter()
        .map(|d| {
            let id = d["_id"].as_str().unwrap_or_default().to_string();
            let text = format!("{}. {}", d["title"].as_str().unwrap_or(""), d["text"].as_str().unwrap_or(""));
            let lines = text.split(". ").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            (id, lines)
        })
        .collect();
    let queries: Vec<(String, String)> = read_jsonl(&folder.join("queries.jsonl"))
        .into_iter()
        .filter_map(|q| Some((q["_id"].as_str()?.to_string(), q["text"].as_str()?.to_string())))
        .filter(|(id, _)| qrels.contains_key(id))
        .take(max_q)
        .collect();

    let names = ["found order", "hit lines", "relevance"];
    let mut sums = [0.0f64; 3];
    let mut counted = 0usize;
    let mut with_hits = 0usize;
    for (qid, text) in &queries {
        let mut seen = HashSet::new();
        let filters: Vec<String> = text
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 3 && !STOP.contains(w))
            .filter(|w| seen.insert(w.to_string()))
            .map(String::from)
            .collect();
        if filters.is_empty() {
            continue;
        }
        let settings = SearchSettings { filters, whole_word: !literal, ..Default::default() };
        let state = CompiledMatchState::build(&settings).unwrap();
        let now = Local::now();
        let results: Vec<FileSearchResult> = docs
            .iter()
            .map(|(id, lines)| {
                let out = apply_line_matching(lines, &settings, &state);
                FileSearchResult {
                    full_name: format!("/corpus/{id}.txt"),
                    status: if out.hits.is_empty() { FileSearchStatus::NoHit } else { FileSearchStatus::Hit },
                    hits: out.hits,
                    created: now,
                    modified: now,
                    file_length: 0,
                    lines_cache: Vec::new(),
                    total_line_count: lines.len() as i32,
                    proximity_min_range: None,
                    low_confidence_pdf: false,
                    error_message: None,
                }
            })
            .collect();
        let hit_ids: Vec<usize> = (0..results.len()).filter(|&i| results[i].status == FileSearchStatus::Hit).collect();
        if hit_ids.is_empty() {
            continue;
        }
        with_hits += 1;
        let id_of = |i: usize| docs[i].0.as_str();
        let found: Vec<&str> = hit_ids.iter().map(|&i| id_of(i)).collect();
        let mut by_hits = hit_ids.clone();
        by_hits.sort_by_key(|&i| std::cmp::Reverse(results[i].hits.len()));
        let by_hits: Vec<&str> = by_hits.into_iter().map(id_of).collect();
        let rel: Vec<&str> = relevance_order(&results, &settings).into_iter().filter(|&i| results[i].status == FileSearchStatus::Hit).map(id_of).collect();
        let gains = &qrels[qid];
        for (s, ranked) in sums.iter_mut().zip([&found, &by_hits, &rel]) {
            *s += ndcg_at(ranked, gains, 10);
        }
        counted += 1;
    }
    println!("{} queries ({} with hits), filename weight {FILENAME_WEIGHT}, {}", queries.len(), with_hits, if literal { "substring" } else { "whole word" });
    for (n, s) in names.iter().zip(sums) {
        println!("  nDCG@10 {n:<12} {:.4}", s / counted.max(1) as f64);
    }
}
