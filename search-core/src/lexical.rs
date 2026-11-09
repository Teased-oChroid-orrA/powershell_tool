//! Keyword scoring fused with embedding similarity for semantic retrieval.
//!
//! Embeddings miss bare numbers, spec codes and part numbers; keyword scoring
//! misses paraphrase. Reciprocal-rank fusion of the two beats either alone
//! (`search-ml/examples/data/retrieval_eval.tsv`, `docs/` audit notes).

use std::collections::HashMap;

/// Longest query (in distinct terms, pieces of part numbers included) that is fused. Longer text, such as a
/// whole document used for related-document discovery, stays dense-only.
pub const MAX_QUERY_TERMS: usize = 24;
const RRF_K: f32 = 60.0;
const LEXICAL_WEIGHT: f32 = 2.0;
const CONVEX_KEYWORD_SHARE: f32 = 0.35;

/// The knobs of keyword scoring and fusion. The defaults are what search uses; benchmarks (`examples/retrieval_eval.rs`,
/// BEIR and MS MARCO) vary them to measure a change before it is adopted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LexicalConfig {
    /// Reduce word forms to a common root (blended = blending).
    pub stem: bool,
    /// Weight of the keyword rank against the similarity rank in the fusion.
    pub weight: f32,
    /// Reciprocal-rank-fusion constant.
    pub rrf_k: f32,
    /// How the two scores are combined.
    pub fusion: Fusion,
    /// BM25 term-frequency saturation and length normalisation.
    pub bm25_k1: f32,
    pub bm25_b: f32,
}

/// How similarity and keyword scores are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fusion {
    /// Reciprocal-rank fusion: only the ranks count.
    Rrf,
    /// Weighted sum of the scores after min-max normalisation over the candidates; `weight` is then the keyword
    /// share, a number in 0 to 1.
    Convex,
}

impl Default for LexicalConfig {
    /// What retrieval over indexed passages uses: min-max normalised scores with a keyword share of 0.35. Measured on
    /// BEIR (SciFact, NFCorpus, TREC-COVID subset), an MS MARCO subset and Cranfield, it equals or beats the
    /// rank fusion below on every set (mean nDCG@10 0.4886 to 0.5092, `docs/search-benchmarks.md`).
    fn default() -> Self {
        Self { fusion: Fusion::Convex, weight: CONVEX_KEYWORD_SHARE, ..Self::rrf() }
    }
}

impl LexicalConfig {
    /// Reciprocal-rank fusion with the keyword rank weighted 2. Kept for ranking the short lines of the Search help
    /// (Ask), whose retrieval was validated with it and is not covered by the benchmarks.
    pub fn rrf() -> Self {
        Self { stem: true, weight: LEXICAL_WEIGHT, rrf_k: RRF_K, fusion: Fusion::Rrf, bm25_k1: BM25_K1, bm25_b: BM25_B }
    }
}
const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "how",
    "in", "is", "it", "of", "on", "or", "that", "the", "to", "was", "what", "when", "where",
    "which", "who", "why", "with",
];

/// Cuts a plural or verb ending off a plain word, so `blended`, `blending` and `blends` all become `blend`
/// and a search for one finds the others. Applied to query and text alike, so it need not give real stems.
/// Words with digits, and short words, are left as they are.
pub fn stem(word: &str) -> String {
    if word.len() < 5 || !word.chars().all(|c| c.is_ascii_alphabetic()) {
        return word.to_string();
    }
    for (ending, replacement) in [("ies", "y"), ("ied", "y"), ("ing", ""), ("ed", ""), ("es", ""), ("s", "")] {
        if let Some(root) = word.strip_suffix(ending) {
            let sibilant = ["s", "x", "z", "ch", "sh"].iter().any(|e| root.ends_with(e));
            if root.len() >= 3 && !(ending == "s" && root.ends_with('s')) && (ending != "es" || sibilant) {
                return format!("{root}{replacement}");
            }
        }
    }
    word.to_string()
}

/// Lowercased terms. A mixed letter/digit token such as `m8x1.25` also yields
/// its pieces (`m`, `8`, `x`, `1.25`) and its first letters+digits prefix
/// (`m8`) so a query for `M8` finds the full part number.
pub fn terms(text: &str) -> Vec<String> {
    terms_with(text, true)
}

/// [`terms`], with word forms reduced to a root only when `stem` is set.
pub fn terms_with(text: &str, stem_words: bool) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '.') {
        let token = raw.trim_matches('.').to_lowercase();
        if token.is_empty() {
            continue;
        }
        out.push(if stem_words { stem(&token) } else { token.clone() });
        let mut pieces = Vec::new();
        let mut piece = String::new();
        let mut last_digit = None;
        for c in token.chars() {
            let digit = c.is_ascii_digit() || c == '.';
            if last_digit.is_some_and(|previous| previous != digit) {
                pieces.push(std::mem::take(&mut piece));
            }
            piece.push(c);
            last_digit = Some(digit);
        }
        pieces.push(piece);
        if pieces.len() > 1 {
            if pieces.len() >= 2 && pieces[1].starts_with(|c: char| c.is_ascii_digit()) {
                out.push(format!("{}{}", pieces[0], pieces[1]));
            }
            out.extend(
                pieces
                    .into_iter()
                    .map(|p| p.trim_matches('.').to_string())
                    .filter(|p| !p.is_empty()),
            );
        }
    }
    out
}

/// Distinct, non-stop-word query terms; `None` when the query is empty of
/// terms or longer than [`MAX_QUERY_TERMS`] (it then stays dense-only).
pub fn query_terms(query: &str) -> Option<Vec<String>> {
    query_terms_with(query, true)
}

/// [`query_terms`] under a given stemming choice.
pub fn query_terms_with(query: &str, stem_words: bool) -> Option<Vec<String>> {
    let mut distinct: Vec<String> = Vec::new();
    for term in terms_with(query, stem_words) {
        if !STOP_WORDS.contains(&term.as_str()) && !distinct.contains(&term) {
            distinct.push(term);
        }
    }
    (!distinct.is_empty() && distinct.len() <= MAX_QUERY_TERMS).then_some(distinct)
}

/// `query_terms` of `text`, empty when there are none or too many.
pub fn query_terms_or_empty(text: &str) -> Vec<String> {
    query_terms(text).unwrap_or_default()
}

/// Per-query-term counts in `text` plus its length in terms.
pub fn count_terms(text: &str, query: &[String]) -> (Vec<u16>, u32) {
    count_terms_with(text, query, true)
}

/// [`count_terms`] under a given stemming choice (it must match the one the query terms were made with).
pub fn count_terms_with(text: &str, query: &[String], stem_words: bool) -> (Vec<u16>, u32) {
    let index: HashMap<&str, usize> = query
        .iter()
        .enumerate()
        .map(|(i, t)| (t.as_str(), i))
        .collect();
    let mut counts = vec![0u16; query.len()];
    let doc = terms_with(text, stem_words);
    for term in &doc {
        if let Some(&i) = index.get(term.as_str()) {
            counts[i] = counts[i].saturating_add(1);
        }
    }
    (counts, doc.len() as u32)
}

/// BM25 over documents given as (term counts, length). Zero when no query term occurs.
pub fn bm25(documents: &[(Vec<u16>, u32)], query_len: usize) -> Vec<f32> {
    bm25_with(documents, query_len, BM25_K1, BM25_B)
}

/// [`bm25`] with its two parameters given.
pub fn bm25_with(documents: &[(Vec<u16>, u32)], query_len: usize, k1: f32, b: f32) -> Vec<f32> {
    let n = documents.len() as f32;
    if documents.is_empty() {
        return Vec::new();
    }
    let average = (documents.iter().map(|d| d.1 as f32).sum::<f32>() / n).max(1.0);
    let mut df = vec![0f32; query_len];
    for (counts, _) in documents {
        for (i, &c) in counts.iter().enumerate() {
            if c > 0 {
                df[i] += 1.0;
            }
        }
    }
    let idf: Vec<f32> = df
        .iter()
        .map(|&d| ((n - d + 0.5) / (d + 0.5) + 1.0).ln())
        .collect();
    documents
        .iter()
        .map(|(counts, length)| {
            let norm = k1 * (1.0 - b + b * *length as f32 / average);
            counts
                .iter()
                .zip(&idf)
                .filter(|(&c, _)| c > 0)
                .map(|(&c, &idf)| idf * c as f32 * (k1 + 1.0) / (c as f32 + norm))
                .sum()
        })
        .collect()
}

fn ranks(scores: &[f32]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then_with(|| a.cmp(&b)));
    let mut rank = vec![0; scores.len()];
    for (position, index) in order.into_iter().enumerate() {
        rank[index] = position;
    }
    rank
}

/// Reciprocal-rank fusion. A document with no keyword match gets no keyword share.
pub fn fuse(dense: &[f32], lexical: &[f32]) -> Vec<f32> {
    fuse_with(dense, lexical, &LexicalConfig::rrf())
}

/// [`fuse`] under a given weight and constant.
pub fn fuse_with(dense: &[f32], lexical: &[f32], config: &LexicalConfig) -> Vec<f32> {
    if config.fusion == Fusion::Convex {
        let scale = |scores: &[f32]| {
            let (low, high) = scores.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), &x| (l.min(x), h.max(x)));
            let span = (high - low).max(f32::EPSILON);
            scores.iter().map(|&x| (x - low) / span).collect::<Vec<f32>>()
        };
        let (dense, lexical) = (scale(dense), scale(lexical));
        let share = config.weight.clamp(0.0, 1.0);
        return dense.iter().zip(&lexical).map(|(d, l)| (1.0 - share) * d + share * l).collect();
    }
    let dense_rank = ranks(dense);
    let lexical_rank = ranks(lexical);
    (0..dense.len())
        .map(|i| {
            1.0 / (config.rrf_k + dense_rank[i] as f32)
                + if lexical[i] > 0.0 {
                    config.weight / (config.rrf_k + lexical_rank[i] as f32)
                } else {
                    0.0
                }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_numbers_yield_searchable_prefixes_and_numbers() {
        let t = terms("Bolt M8x1.25 at 4500 N, e/D 1.5");
        for want in ["m8x1.25", "m8", "1.25", "4500", "e", "d", "1.5"] {
            assert!(t.iter().any(|x| x == want), "missing {want}: {t:?}");
        }
    }

    #[test]
    fn word_forms_match_each_other() {
        for (a, b) in [("blended", "blending"), ("faired", "fairing"), ("fills", "filled"), ("holes", "hole"), ("bodies", "bodied")] {
            assert_eq!(terms(a), terms(b), "{a} vs {b}");
        }
        assert_ne!(terms("glass"), terms("glas"));
        let q = query_terms("surfaces blended").unwrap();
        let docs = [
            count_terms("Fairing or blending reworked areas into the surface", &q),
            count_terms("The bonding surfaces should be machined smooth", &q),
        ];
        let s = bm25(&docs, q.len());
        assert!(s[0] > s[1], "{s:?}");
    }

    #[test]
    fn m8_query_matches_m8x1_25_passage_not_m6() {
        let q = query_terms("preload for M8 bolts").unwrap();
        assert!(!q.contains(&"for".to_string()));
        let docs = [
            count_terms("Bolt preload is 4500 N for M6x1 fasteners", &q),
            count_terms("Bolt preload is 8200 N for M8x1.25 fasteners", &q),
        ];
        let s = bm25(&docs, q.len());
        assert!(s[1] > s[0], "{s:?}");
    }

    #[test]
    fn empty_and_overlong_queries_stay_dense_only() {
        assert!(query_terms("the of is").is_none());
        let long = (0..40)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(query_terms(&long).is_none());
    }

    #[test]
    fn fusion_lifts_keyword_match_over_a_dense_near_miss() {
        let dense = [0.9, 0.5, 0.4];
        let lexical = [0.0, 3.0, 0.0];
        let fused = fuse(&dense, &lexical);
        assert!(fused[1] > fused[0]);
        // The same under score fusion (the default for passages): a close second by meaning that also matches wins.
        let close = [0.9, 0.8, 0.4];
        let fused = fuse_with(&close, &lexical, &LexicalConfig::default());
        assert!(fused[1] > fused[0], "{fused:?}");
    }
}
