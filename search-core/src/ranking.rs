//! Query-time ranking over a semantic snapshot: raw per-chunk scores (`feature_set`) that ranking ideas are measured on
//! offline (`examples/support/ranking_experiments.rs`, `docs/search-benchmarks.md`).

use std::collections::HashSet;

use crate::semantic_cache::Snapshot;

/// Word-3-gram Jaccard similarity at or above which two passages count as the same text.
pub const NEAR_DUPLICATE: f32 = 0.7;

/// Hashes of the lowercase word 3-grams of `text` (the whole text as one gram when it has fewer than three words).
pub fn shingles(text: &str) -> HashSet<u64> {
    use std::hash::{Hash, Hasher};
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    let hash = |grams: &[String]| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        grams.hash(&mut hasher);
        hasher.finish()
    };
    if words.len() < 3 {
        return HashSet::from([hash(&words)]);
    }
    words.windows(3).map(hash).collect()
}

/// Jaccard similarity of two shingle sets (0 when both are empty).
pub fn jaccard(a: &HashSet<u64>, b: &HashSet<u64>) -> f32 {
    let inter = a.intersection(b).count() as f32;
    let union = (a.len() + b.len()) as f32 - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// One chunk's raw scores for one query.
#[derive(Debug, Clone, Copy)]
pub struct ChunkFeatures {
    /// Index into [`FeatureSet::docs`].
    pub doc: u32,
    /// Position of the chunk in its file (0 = first) and the number of chunks the file has.
    pub ordinal: u32,
    pub chunks_in_doc: u32,
    pub rowid: i64,
    /// Cosine similarity of the query and the chunk.
    pub dense: f32,
    /// BM25 of the query terms over all chunks (0 where none occurs or the query has no terms).
    pub bm25: f32,
    /// Chunk length in terms.
    pub length: u32,
}

#[derive(Debug, Clone)]
pub struct FeatureSet {
    pub docs: Vec<String>,
    pub chunks: Vec<ChunkFeatures>,
    /// Distinct non-stop-word terms of the query (0 when the query stays dense-only).
    pub query_terms: usize,
}

pub(crate) fn feature_set(snapshot: &Snapshot, query_values: &[f32], query: &str, stem: bool) -> Option<FeatureSet> {
    let terms = crate::lexical::query_terms_with(query, stem);
    let candidates: Vec<u32> = (0..snapshot.len() as u32).collect();
    let bm25 = terms
        .as_ref()
        .map(|t| snapshot.bm25(t, &candidates, crate::lexical::LexicalConfig::default().bm25_k1, crate::lexical::LexicalConfig::default().bm25_b));
    let mut counts = vec![0u32; snapshot.docs.len()];
    for &d in &snapshot.doc_of {
        counts[d as usize] += 1;
    }
    let mut seen = vec![0u32; snapshot.docs.len()];
    let mut chunks = Vec::with_capacity(snapshot.len());
    for i in 0..snapshot.len() {
        let doc = snapshot.doc_of[i];
        let dense = snapshot.similarity(query_values, i);
        if !dense.is_finite() {
            continue;
        }
        chunks.push(ChunkFeatures {
            doc,
            ordinal: seen[doc as usize],
            chunks_in_doc: counts[doc as usize],
            rowid: snapshot.rowids[i],
            dense,
            bm25: bm25.as_ref().map_or(0.0, |b| b[i]),
            length: snapshot.lengths[i],
        });
        seen[doc as usize] += 1;
    }
    Some(FeatureSet { docs: snapshot.docs.iter().map(|d| d.path.clone()).collect(), chunks, query_terms: terms.map_or(0, |t| t.len()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexical::{fuse_with, LexicalConfig};
    use crate::semantic_index::SemanticIndex;

    #[test]
    fn shingle_similarity_separates_copies_from_different_text() {
        let a = shingles("the stud torque for the m8 flange bolts is 25 newton metres");
        let copy = shingles("The stud torque for the M8 flange bolts is 25 newton metres (rev 2)");
        let other = shingles("gasket thickness was measured at three points around the rim");
        assert!(jaccard(&a, &copy) >= NEAR_DUPLICATE);
        assert_eq!(jaccard(&a, &other), 0.0);
        assert_eq!(jaccard(&shingles(""), &shingles("")), 1.0);
    }

    /// The raw scores `rank_features` returns, fused with the default configuration, give the order search returns.
    #[test]
    fn features_rebuild_the_search_order() {
        let dir = tempfile::tempdir().unwrap();
        let index = SemanticIndex::open(&dir.path().join("semantic.sqlite")).unwrap();
        for (name, text) in [
            ("a.txt", "Torque the M8x1.25 bolt to 25 newton metres."),
            ("b.txt", "The shaft experiences torsional stress under load."),
            ("c.txt", "Corrosion was blended out and the surface filled."),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            let metadata = std::fs::metadata(&path).unwrap();
            let modified = crate::cache::ticks_from_modified(chrono::DateTime::<chrono::Local>::from(metadata.modified().unwrap()));
            index.index_document(&path.to_string_lossy(), modified, metadata.len() as i64, &[text.to_string()]).unwrap();
        }
        let config = LexicalConfig::default();
        for query in ["M8 bolt torque", "shaft stress", "blended corrosion"] {
            let features = index.rank_features(query, config.stem).unwrap().unwrap();
            assert_eq!(features.chunks.len(), 3);
            assert!(features.query_terms > 0);
            let dense: Vec<f32> = features.chunks.iter().map(|c| c.dense).collect();
            let keyword: Vec<f32> = features.chunks.iter().map(|c| c.bm25).collect();
            let scores = fuse_with(&dense, &keyword, &config);
            let best = (0..scores.len()).max_by(|&a, &b| scores[a].total_cmp(&scores[b])).unwrap();
            let searched = index.search_tuned(query, 3, true, &config).unwrap();
            assert_eq!(features.docs[features.chunks[best].doc as usize], searched[0].path, "{query}");
        }
    }
}
