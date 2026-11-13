//! Opt-in relevance ordering for exact search results.

use search_ml::MlError;

const BATCH_SIZE: usize = 16;
const MATCH_TEXT_WEIGHT: f32 = 0.45;
const FILENAME_WEIGHT: f32 = 0.10;
const EXACT_EVIDENCE_WEIGHT: f32 = 0.20;
const PROXIMITY_WEIGHT: f32 = 0.15;
const RECENCY_WEIGHT: f32 = 0.10;
pub const MAX_FEEDBACK_EXAMPLES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackLabel {
    Relevant,
    NotRelevant,
}

/// A user's explicit judgment of the existing neural ranking signals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelevanceFeedback {
    pub label: FeedbackLabel,
    pub match_text_similarity: f32,
    pub filename_similarity: f32,
    pub exact_evidence: f32,
    pub proximity: Option<f32>,
    pub recency: f32,
}

/// Compact signals copied from an exact result before background ranking.
pub struct RelevanceCandidate {
    pub filename: String,
    pub matched_text: String,
    pub hit_count: usize,
    pub proximity_min_range: Option<i32>,
    pub modified_unix: i64,
}

/// Raw factors and combined score for one exact result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelevanceExplanation {
    pub index: usize,
    pub score: f32,
    pub base_score: f32,
    pub feedback_applied: bool,
    pub match_text_similarity: f32,
    pub filename_similarity: f32,
    pub exact_evidence: f32,
    pub proximity: Option<f32>,
    pub recency: f32,
}

/// Rank only supplied exact hits. Missing proximity data does not count as a
/// zero score; its weight is removed before the available factors are combined.
pub fn rank_results(
    query: &str,
    candidates: &[RelevanceCandidate],
) -> Result<Vec<RelevanceExplanation>, MlError> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let query_embedding = search_ml::embed_queries(&[query])?
        .into_iter()
        .next()
        .expect("one query produces one embedding");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let mut ranked = Vec::with_capacity(candidates.len());

    for (batch_index, batch) in candidates.chunks(BATCH_SIZE).enumerate() {
        let texts = batch
            .iter()
            .flat_map(|candidate| [candidate.filename.as_str(), candidate.matched_text.as_str()])
            .collect::<Vec<_>>();
        let embeddings = search_ml::embed_batch(&texts)?;
        for (offset, candidate) in batch.iter().enumerate() {
            let filename_similarity = normalize_similarity(search_ml::cosine_similarity(
                &query_embedding,
                &embeddings[offset * 2],
            ));
            let match_text_similarity = normalize_similarity(search_ml::cosine_similarity(
                &query_embedding,
                &embeddings[offset * 2 + 1],
            ));
            let exact_evidence = candidate.hit_count as f32 / (candidate.hit_count as f32 + 1.0);
            let proximity = candidate
                .proximity_min_range
                .map(|range| 1.0 / (range.max(0) as f32 + 1.0));
            let age_days = now.saturating_sub(candidate.modified_unix).max(0) as f32 / 86_400.0;
            let recency = 1.0 / (1.0 + age_days / 365.0);
            let mut weighted = match_text_similarity * MATCH_TEXT_WEIGHT
                + filename_similarity * FILENAME_WEIGHT
                + exact_evidence * EXACT_EVIDENCE_WEIGHT
                + recency * RECENCY_WEIGHT;
            let mut weights =
                MATCH_TEXT_WEIGHT + FILENAME_WEIGHT + EXACT_EVIDENCE_WEIGHT + RECENCY_WEIGHT;
            if let Some(value) = proximity {
                weighted += value * PROXIMITY_WEIGHT;
                weights += PROXIMITY_WEIGHT;
            }
            ranked.push(RelevanceExplanation {
                index: batch_index * BATCH_SIZE + offset,
                score: weighted / weights,
                base_score: weighted / weights,
                feedback_applied: false,
                match_text_similarity,
                filename_similarity,
                exact_evidence,
                proximity,
                recency,
            });
        }
    }
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.index.cmp(&right.index))
    });
    Ok(ranked)
}

/// Adjust ranking weights from explicit local judgments. This only reorders
/// the supplied explanations; it cannot add or remove exact-search results.
pub fn apply_feedback(scores: &mut [RelevanceExplanation], examples: &[RelevanceFeedback]) {
    let examples = examples
        .iter()
        .rev()
        .take(MAX_FEEDBACK_EXAMPLES)
        .filter(|example| valid_feedback(example))
        .copied()
        .collect::<Vec<_>>();
    let positive = examples
        .iter()
        .filter(|example| example.label == FeedbackLabel::Relevant)
        .collect::<Vec<_>>();
    let negative = examples
        .iter()
        .filter(|example| example.label == FeedbackLabel::NotRelevant)
        .collect::<Vec<_>>();
    if positive.is_empty() || negative.is_empty() {
        return;
    }

    for score in scores.iter_mut() {
        let mut weighted = 0.0;
        let mut total_weight = 0.0;
        for (value, base_weight, positive_mean, negative_mean) in [
            (
                Some(score.match_text_similarity),
                MATCH_TEXT_WEIGHT,
                mean(&positive, |e| Some(e.match_text_similarity)),
                mean(&negative, |e| Some(e.match_text_similarity)),
            ),
            (
                Some(score.filename_similarity),
                FILENAME_WEIGHT,
                mean(&positive, |e| Some(e.filename_similarity)),
                mean(&negative, |e| Some(e.filename_similarity)),
            ),
            (
                Some(score.exact_evidence),
                EXACT_EVIDENCE_WEIGHT,
                mean(&positive, |e| Some(e.exact_evidence)),
                mean(&negative, |e| Some(e.exact_evidence)),
            ),
            (
                score.proximity,
                PROXIMITY_WEIGHT,
                mean(&positive, |e| e.proximity),
                mean(&negative, |e| e.proximity),
            ),
            (
                Some(score.recency),
                RECENCY_WEIGHT,
                mean(&positive, |e| Some(e.recency)),
                mean(&negative, |e| Some(e.recency)),
            ),
        ] {
            if let Some(value) = value {
                let scale = (1.0 + 2.0 * (positive_mean - negative_mean)).clamp(0.2, 3.0);
                let weight = base_weight * scale;
                weighted += value * weight;
                total_weight += weight;
            }
        }
        score.score = (weighted / total_weight).clamp(0.0, 1.0);
        score.feedback_applied = true;
    }
    scores.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.index.cmp(&right.index))
    });
}

fn mean(examples: &[&RelevanceFeedback], value: impl Fn(&RelevanceFeedback) -> Option<f32>) -> f32 {
    let (sum, count) = examples
        .iter()
        .filter_map(|example| value(example))
        .fold((0.0, 0usize), |(sum, count), value| {
            (sum + value, count + 1)
        });
    if count == 0 {
        0.5
    } else {
        sum / count as f32
    }
}

fn valid_feedback(example: &RelevanceFeedback) -> bool {
    [
        Some(example.match_text_similarity),
        Some(example.filename_similarity),
        Some(example.exact_evidence),
        example.proximity,
        Some(example.recency),
    ]
    .into_iter()
    .flatten()
    .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
}

fn normalize_similarity(similarity: Option<f32>) -> f32 {
    ((similarity.unwrap_or(0.0) + 1.0) / 2.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn explanation(
        index: usize,
        match_text_similarity: f32,
        filename_similarity: f32,
    ) -> RelevanceExplanation {
        RelevanceExplanation {
            index,
            score: 0.5 * match_text_similarity + 0.5 * filename_similarity,
            base_score: 0.5 * match_text_similarity + 0.5 * filename_similarity,
            feedback_applied: false,
            match_text_similarity,
            filename_similarity,
            exact_evidence: 0.5,
            proximity: None,
            recency: 0.5,
        }
    }

    fn feedback(label: FeedbackLabel, text: f32, name: f32) -> RelevanceFeedback {
        RelevanceFeedback {
            label,
            match_text_similarity: text,
            filename_similarity: name,
            exact_evidence: 0.5,
            proximity: None,
            recency: 0.5,
        }
    }

    #[test]
    fn feedback_requires_both_labels_and_only_reorders_supplied_exact_results() {
        let mut scores = vec![explanation(0, 0.9, 0.1), explanation(1, 0.1, 0.9)];
        let baseline = scores.clone();
        apply_feedback(&mut scores, &[feedback(FeedbackLabel::Relevant, 0.1, 0.9)]);
        assert_eq!(
            scores, baseline,
            "one-sided feedback leaves baseline unchanged"
        );

        apply_feedback(
            &mut scores,
            &[
                feedback(FeedbackLabel::Relevant, 0.1, 0.9),
                feedback(FeedbackLabel::NotRelevant, 0.9, 0.1),
            ],
        );
        assert_eq!(
            scores.iter().map(|score| score.index).collect::<Vec<_>>(),
            [1, 0]
        );
        assert!(scores.iter().all(|score| score.feedback_applied));
        assert!(scores
            .iter()
            .all(|score| (0.0..=1.0).contains(&score.score)));
        assert!(scores.iter().all(|score| score.base_score != score.score));
    }

    #[test]
    fn invalid_feedback_does_not_change_baseline_scores() {
        let mut scores = vec![explanation(0, 0.9, 0.1)];
        let baseline = scores.clone();
        apply_feedback(
            &mut scores,
            &[
                feedback(FeedbackLabel::Relevant, f32::NAN, 0.9),
                feedback(FeedbackLabel::NotRelevant, 0.9, 0.1),
            ],
        );
        assert_eq!(scores, baseline);
    }

    #[test]
    fn neural_relevance_orders_exact_candidates_and_exposes_score_factors() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let candidates = vec![
            RelevanceCandidate {
                filename: "tax.txt".into(),
                matched_text: "The annual tax policy changed after the election.".into(),
                hit_count: 1,
                proximity_min_range: None,
                modified_unix: now - 20 * 365 * 86_400,
            },
            RelevanceCandidate {
                filename: "garden-notes.txt".into(),
                matched_text: "A dog plays outside among plants.".into(),
                hit_count: 3,
                proximity_min_range: Some(1),
                modified_unix: now,
            },
        ];
        let ranked = rank_results("A dog is playing outdoors in a garden.", &candidates).unwrap();
        assert_eq!(
            ranked.iter().map(|result| result.index).collect::<Vec<_>>(),
            [1, 0]
        );
        assert!(ranked[0].match_text_similarity > ranked[1].match_text_similarity);
        assert!(ranked
            .iter()
            .all(|result| (0.0..=1.0).contains(&result.score)));
        assert_eq!(ranked[0].proximity, Some(0.5));
        assert_eq!(ranked[1].proximity, None);
    }
}
