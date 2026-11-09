//! Reciprocal-rank fusion of an existing ranking with a second scoring of its leading items.
//!
//! One implementation for the opt-in "Refine results" step of the Search window and for
//! `examples/retrieval_eval.rs` (`rerank-rrf`, benchmarked in `docs/search-benchmarks.md`).

/// The reciprocal-rank constant used by the benchmarked variant.
pub const RRF_K: f64 = 60.0;
/// How many leading results the Refine step re-scores.
pub const REFINE_TOP: usize = 30;

/// The new order of `total` ranked items after fusing the first `scores.len()` of them.
///
/// Item `i` is the one at original rank `i`; `scores[i]` is the second model's score for it (higher is better).
/// Each scored item gets `1/(K + original rank) + 1/(K + score rank)`; those items are sorted by that sum, best
/// first (ties keep the original order). Items beyond `scores.len()` keep their place. Returns the original
/// indices in their new order; a `scores` longer than `total` is clipped to `total`.
pub fn fuse_top(scores: &[f32], total: usize) -> Vec<usize> {
    let n = scores.len().min(total);
    let mut by_score: Vec<usize> = (0..n).collect();
    by_score.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    let mut score_rank = vec![0usize; n];
    for (rank, &i) in by_score.iter().enumerate() {
        score_rank[i] = rank;
    }
    let fused = |i: usize| 1.0 / (RRF_K + i as f64) + 1.0 / (RRF_K + score_rank[i] as f64);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| fused(b).total_cmp(&fused(a)));
    order.extend(n..total);
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_gives_the_untouched_tail() {
        assert!(fuse_top(&[], 0).is_empty());
        assert_eq!(fuse_top(&[], 3), vec![0, 1, 2]);
    }

    #[test]
    fn agreeing_scores_keep_the_order() {
        assert_eq!(fuse_top(&[0.9, 0.5, 0.1], 3), vec![0, 1, 2]);
    }

    #[test]
    fn a_strong_second_score_lifts_an_item_but_not_above_a_double_winner() {
        // Item 2 is last originally but best by score; item 0 is first originally and second by score.
        let order = fuse_top(&[0.8, 0.1, 0.9], 3);
        assert_eq!(order, vec![0, 2, 1]);
        // Fusion is a compromise, not a replacement: a plain score sort would put item 2 first.
    }

    #[test]
    fn only_the_scored_head_moves() {
        let order = fuse_top(&[0.1, 0.2, 0.9], 6);
        assert_eq!(&order[3..], &[3, 4, 5]);
        let mut head = order[..3].to_vec();
        head.sort();
        assert_eq!(head, vec![0, 1, 2]);
        // Item 0 (first originally, last by score) and item 2 (the reverse) tie; the original order wins.
        assert_eq!(&order[..3], &[0, 2, 1]);
    }

    #[test]
    fn ties_keep_the_original_order() {
        assert_eq!(fuse_top(&[0.5, 0.5, 0.5], 3), vec![0, 1, 2]);
    }

    #[test]
    fn extra_scores_are_clipped_to_the_total() {
        assert_eq!(fuse_top(&[0.1, 0.9, 0.5], 2), vec![0, 1]);
    }

    #[test]
    fn matches_the_reference_formula() {
        let scores = [0.2f32, 0.7, 0.4, 0.9, 0.1];
        let order = fuse_top(&scores, 5);
        let mut by_score: Vec<usize> = (0..5).collect();
        by_score.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
        let rank_of = |i: usize| by_score.iter().position(|&x| x == i).unwrap();
        let f = |i: usize| 1.0 / (60.0 + i as f64) + 1.0 / (60.0 + rank_of(i) as f64);
        for w in order.windows(2) {
            assert!(f(w[0]) >= f(w[1]));
        }
    }
}
