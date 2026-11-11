//! Optional relevance ordering of exact-search results. The default order of
//! a run stays "found order" (`SearchRunResult::file_results`); this module
//! only computes a ranking the caller may show instead. It uses nothing but
//! the run's own results, so it costs one pass over the hits.
//!
//! The score is BM25 over the matched lines: each filter is a term, its
//! term frequency in a file is the number of hit lines that matched it, its
//! document frequency is the number of files that matched it (inverse
//! document frequency over every file the run searched), and file length is
//! the line count. So a file with many hits for a rare filter outranks one
//! with a single hit for a common word, and a file matching more of the
//! filters outranks one matching fewer. A filter that appears in the file
//! name adds a bonus.
//!
//! Measured against found order and plain hit counts on Cranfield, SciFact
//! and NFCorpus by `examples/exact_order_eval.rs` (numbers in
//! `docs/search-benchmarks.md` once recorded, and `AGENTS.md`).

use std::collections::HashMap;

use crate::models::{FileSearchResult, FileSearchStatus, SearchSettings};
use crate::textfold::fold_lower;

const K1: f64 = 1.2;
const B: f64 = 0.75;
/// Weight of a filter appearing in the file name, as a multiple of that
/// filter's idf (a file *named* for the thing searched for is rarely wrong).
pub const FILENAME_WEIGHT: f64 = 0.6;

/// Why a file ranks where it does; the pieces of its score.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OrderScore {
    pub score: f64,
    /// Filters (as typed) that matched at least one line, in filter order.
    pub matched_filters: Vec<String>,
    /// Filters that appear in the file name.
    pub filename_filters: Vec<String>,
    /// Hit lines in the file.
    pub hit_lines: usize,
}

/// Scores every `Hit` file of a run. Returns one entry per result, in the
/// same order as `results`; files that are not hits score 0 with no
/// evidence.
pub fn score_results(results: &[FileSearchResult], settings: &SearchSettings) -> Vec<OrderScore> {
    let filters: Vec<&String> = settings.all_filters().collect();
    let n_files = results.len().max(1) as f64;
    let is_hit = |r: &FileSearchResult| r.status == FileSearchStatus::Hit;

    // Per file: hit lines per filter.
    let per_file: Vec<Vec<usize>> = results
        .iter()
        .map(|r| {
            let mut tf = vec![0usize; filters.len()];
            if is_hit(r) {
                let index: HashMap<&str, usize> = filters.iter().enumerate().map(|(i, f)| (f.as_str(), i)).collect();
                for h in &r.hits {
                    for f in &h.matched_filters {
                        if let Some(&i) = index.get(f.as_str()) {
                            tf[i] += 1;
                        }
                    }
                }
            }
            tf
        })
        .collect();

    let mut df = vec![0usize; filters.len()];
    for tf in &per_file {
        for (i, &t) in tf.iter().enumerate() {
            if t > 0 {
                df[i] += 1;
            }
        }
    }
    let idf: Vec<f64> = df
        .iter()
        .map(|&d| (1.0 + (n_files - d as f64 + 0.5) / (d as f64 + 0.5)).ln())
        .collect();

    let hit_lens: Vec<f64> = results.iter().filter(|r| is_hit(r)).map(|r| f64::from(r.total_line_count.max(1))).collect();
    let avg_len = if hit_lens.is_empty() { 1.0 } else { hit_lens.iter().sum::<f64>() / hit_lens.len() as f64 };
    let folded_filters: Vec<String> = filters.iter().map(|f| fold_lower(f)).collect();

    results
        .iter()
        .zip(&per_file)
        .map(|(r, tf)| {
            if !is_hit(r) {
                return OrderScore::default();
            }
            let len = f64::from(r.total_line_count.max(1));
            let name = fold_lower(
                std::path::Path::new(&r.full_name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
                    .as_str(),
            );
            let mut out = OrderScore { hit_lines: r.hits.len(), ..Default::default() };
            for (i, &t) in tf.iter().enumerate() {
                if t > 0 {
                    let t = t as f64;
                    out.score += idf[i] * t * (K1 + 1.0) / (t + K1 * (1.0 - B + B * len / avg_len));
                    out.matched_filters.push(filters[i].clone());
                }
                if !folded_filters[i].is_empty() && name.contains(&folded_filters[i]) {
                    out.score += idf[i] * FILENAME_WEIGHT;
                    out.filename_filters.push(filters[i].clone());
                }
            }
            out
        })
        .collect()
}

/// Indices into `results`, best first: `Hit` files by descending score,
/// ties kept in found order; non-hit files follow in found order.
pub fn relevance_order(results: &[FileSearchResult], settings: &SearchSettings) -> Vec<usize> {
    let scores = score_results(results, settings);
    let mut idx: Vec<usize> = (0..results.len()).collect();
    idx.sort_by(|&a, &b| {
        let (ha, hb) = (results[a].status == FileSearchStatus::Hit, results[b].status == FileSearchStatus::Hit);
        hb.cmp(&ha).then_with(|| scores[b].score.total_cmp(&scores[a].score))
    });
    idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LineHit;
    use chrono::Local;

    fn file(name: &str, lines: i32, hits: &[(i32, &[&str])]) -> FileSearchResult {
        FileSearchResult {
            full_name: format!("/docs/{name}"),
            status: if hits.is_empty() { FileSearchStatus::NoHit } else { FileSearchStatus::Hit },
            hits: hits
                .iter()
                .map(|(n, fs)| LineHit {
                    line_number: *n,
                    before: None,
                    match_line: String::new(),
                    after: None,
                    matched_filters: fs.iter().map(|s| s.to_string()).collect(),
                })
                .collect(),
            created: Local::now(),
            modified: Local::now(),
            file_length: 0,
            lines_cache: Vec::new(),
            total_line_count: lines,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }
    }

    fn settings(filters: &[&str]) -> SearchSettings {
        SearchSettings { filters: filters.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn rare_filters_and_more_filters_outrank_one_common_hit() {
        let results = vec![
            file("a.txt", 100, &[(1, &["the"])]),
            file("b.txt", 100, &[(1, &["the"]), (5, &["flutter"])]),
            file("c.txt", 100, &[(2, &["the"])]),
            file("d.txt", 100, &[]),
            file("e.txt", 100, &[(3, &["the"])]),
        ];
        let order = relevance_order(&results, &settings(&["the", "flutter"]));
        assert_eq!(order[0], 1, "the only file with the rare word comes first");
        assert_eq!(order[4], 3, "non-hit files go last");
        assert_eq!(&order[1..4], &[0, 2, 4], "equal scores keep found order");
    }

    #[test]
    fn density_beats_a_stray_hit_in_a_long_file() {
        let results = vec![
            file("long.txt", 5000, &[(10, &["torque"])]),
            file("short.txt", 40, &[(1, &["torque"]), (2, &["torque"]), (9, &["torque"])]),
        ];
        assert_eq!(relevance_order(&results, &settings(&["torque"])), vec![1, 0]);
    }

    #[test]
    fn a_file_named_for_the_filter_gets_a_bonus_and_it_is_reported() {
        let results = vec![file("notes.txt", 50, &[(1, &["fastener"])]), file("Fastener list.txt", 50, &[(1, &["fastener"])])];
        let s = score_results(&results, &settings(&["fastener"]));
        assert!(s[1].score > s[0].score);
        assert_eq!(s[1].filename_filters, vec!["fastener"]);
        assert_eq!(s[1].matched_filters, vec!["fastener"]);
        assert_eq!(relevance_order(&results, &settings(&["fastener"])), vec![1, 0]);
    }
}
