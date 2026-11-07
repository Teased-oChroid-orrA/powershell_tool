//! Search-specific ranking over candidates produced by existing local context.

/// Candidate index and neural similarity to the supplied search context.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CandidateScore {
    pub index: usize,
    pub similarity: f32,
}

/// Unchanged source line selected for an extractive document summary.
#[derive(Clone, Debug, PartialEq)]
pub struct CitedSummaryLine {
    pub line_number: usize,
    pub text: String,
}

/// Bounded summary output with explicit sampled-source coverage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExtractiveSummary {
    pub lines: Vec<CitedSummaryLine>,
    pub source_lines: usize,
    pub sampled_lines: usize,
    pub skipped_long_lines: usize,
}

const SUMMARY_MAX_BYTES: usize = 1_048_576;
const SUMMARY_MAX_SAMPLES: usize = 256;
const SUMMARY_MAX_OUTPUT_LINES: usize = 5;
const SUMMARY_MAX_LINE_BYTES: usize = 4_096;
const SUMMARY_BATCH_SIZE: usize = 16;
const SUMMARY_REDUNDANCY_WEIGHT: f32 = 0.25;

/// Select representative original lines using bundled MiniLM sentence vectors.
/// Returns source citations only; this function never generates text.
pub fn extractive_summary(
    text: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<ExtractiveSummary, String> {
    if text.len() > SUMMARY_MAX_BYTES {
        return Err("Selected document exceeds the 1 MiB summary input limit.".into());
    }
    if cancel.is_cancelled() {
        return Err("Extractive summary cancelled.".into());
    }

    let source_lines = text.lines().filter(|line| !line.trim().is_empty()).count();
    if source_lines == 0 {
        return Ok(ExtractiveSummary::default());
    }
    let sample_count = source_lines.min(SUMMARY_MAX_SAMPLES);
    let mut sampled = Vec::with_capacity(sample_count);
    let mut skipped_long_lines = 0;
    let mut seen = 0usize;
    let mut next_sample = 0usize;
    for (index, line) in text.lines().enumerate() {
        if index % 64 == 0 && cancel.is_cancelled() {
            return Err("Extractive summary cancelled.".into());
        }
        if line.trim().is_empty() {
            continue;
        }
        let wanted = next_sample < sample_count
            && seen == next_sample.saturating_mul(source_lines) / sample_count;
        seen += 1;
        if !wanted {
            continue;
        }
        next_sample += 1;
        if line.len() > SUMMARY_MAX_LINE_BYTES
            || search_ml::token_count(line).map_err(|error| error.to_string())?
                > search_ml::MAX_INPUT_TOKENS
        {
            skipped_long_lines += 1;
            continue;
        }
        sampled.push((index + 1, line.to_string()));
    }

    let mut embeddings = Vec::with_capacity(sampled.len());
    for batch in sampled.chunks(SUMMARY_BATCH_SIZE) {
        if cancel.is_cancelled() {
            return Err("Extractive summary cancelled.".into());
        }
        let inputs = batch
            .iter()
            .map(|(_, line)| line.as_str())
            .collect::<Vec<_>>();
        embeddings.extend(search_ml::embed_batch(&inputs).map_err(|error| error.to_string())?);
    }
    if embeddings.is_empty() {
        return Ok(ExtractiveSummary {
            source_lines,
            skipped_long_lines,
            ..Default::default()
        });
    }

    let mut center = vec![0.0; 384];
    for embedding in &embeddings {
        for (sum, value) in center.iter_mut().zip(embedding.values()) {
            *sum += value / embeddings.len() as f32;
        }
    }
    let norm = center.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Err("The model returned an invalid document summary vector.".into());
    }
    center.iter_mut().for_each(|value| *value /= norm);
    let center = search_ml::Embedding::from_values(center).map_err(|error| error.to_string())?;

    let mut selected = Vec::with_capacity(SUMMARY_MAX_OUTPUT_LINES.min(embeddings.len()));
    while selected.len() < SUMMARY_MAX_OUTPUT_LINES && selected.len() < embeddings.len() {
        if cancel.is_cancelled() {
            return Err("Extractive summary cancelled.".into());
        }
        let mut best: Option<(usize, f32)> = None;
        for (index, embedding) in embeddings.iter().enumerate() {
            if selected.contains(&index) {
                continue;
            }
            let salience =
                search_ml::cosine_similarity(&center, embedding).unwrap_or(f32::NEG_INFINITY);
            let redundancy = selected
                .iter()
                .map(|&chosen| {
                    search_ml::cosine_similarity(embedding, &embeddings[chosen]).unwrap_or(0.0)
                })
                .fold(0.0f32, f32::max);
            let score = salience - SUMMARY_REDUNDANCY_WEIGHT * redundancy;
            if best.is_none_or(|(best_index, best_score)| {
                score > best_score
                    || (score == best_score && sampled[index].0 < sampled[best_index].0)
            }) {
                best = Some((index, score));
            }
        }
        selected.push(best.expect("at least one unselected embedding").0);
    }
    selected.sort_by_key(|&index| sampled[index].0);
    let lines = selected
        .into_iter()
        .map(|index| CitedSummaryLine {
            line_number: sampled[index].0,
            text: sampled[index].1.clone(),
        })
        .collect();
    Ok(ExtractiveSummary {
        lines,
        source_lines,
        sampled_lines: sampled.len(),
        skipped_long_lines,
    })
}

/// A bounded extractive answer candidate with its original source citation.
#[cfg(feature = "semantic-search")]
#[derive(Clone, Debug, PartialEq)]
pub struct CitedAnswer {
    pub hit: crate::semantic_index::SemanticHit,
    /// Unchanged source line ranked against the question.
    pub answer: String,
    pub answer_similarity: f32,
}

/// Generated question answer accompanied by the unchanged source lines that
/// grounded its prompt. The answer can still be wrong; callers must show these citations.
#[cfg(feature = "semantic-search")]
#[derive(Clone, Debug, PartialEq)]
pub struct CitedQuestionAnswer {
    pub text: String,
    pub evidence: Vec<CitedAnswer>,
}

#[cfg(feature = "semantic-search")]
const ANSWER_CANDIDATE_BATCH: usize = 128;
#[cfg(feature = "semantic-search")]
const MAX_CITED_ANSWERS: usize = 3;

/// Candidate phrases mined from bounded exact-hit context, in frequency order.
pub fn corpus_term_candidates(query: &str, lines: &[String], limit: usize) -> Vec<String> {
    const STOP: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "of",
        "on", "or", "the", "to", "with",
    ];
    let query = query.trim().to_lowercase();
    let query_words: std::collections::HashSet<&str> = query.split_whitespace().collect();
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for line in lines.iter().take(40) {
        let bounded: String = line.chars().take(2048).collect();
        let words: Vec<String> = bounded
            .split_whitespace()
            .map(|raw| {
                raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '#')
                    .to_lowercase()
            })
            .filter(|word| word.chars().count() >= 2)
            .collect();
        let mut seen = std::collections::HashSet::new();
        for width in 1..=3 {
            for window in words.windows(width) {
                if STOP.contains(&window[0].as_str()) || STOP.contains(&window[width - 1].as_str())
                {
                    continue;
                }
                let candidate = window.join(" ");
                if candidate != query
                    && !(width == 1 && query_words.contains(candidate.as_str()))
                    && seen.insert(candidate.clone())
                {
                    *counts.entry(candidate).or_default() += 1;
                }
            }
        }
    }
    let mut candidates: Vec<_> = counts.into_iter().collect();
    candidates.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| {
                b.0.split_whitespace()
                    .count()
                    .cmp(&a.0.split_whitespace().count())
            })
            .then_with(|| a.0.cmp(&b.0))
    });
    candidates
        .into_iter()
        .take(limit.min(32))
        .map(|(term, _)| term)
        .collect()
}

/// Rank bounded local suggestion candidates by neural sentence similarity.
/// The caller supplies candidates; this does not search files or change exact results.
pub fn rank_suggestions(
    context: &str,
    candidates: &[String],
) -> Result<Vec<CandidateScore>, search_ml::MlError> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut inputs = Vec::with_capacity(candidates.len() + 1);
    inputs.push(context);
    inputs.extend(candidates.iter().map(String::as_str));
    let embeddings = search_ml::embed_batch(&inputs)?;
    let query = &embeddings[0];
    let mut scores = candidates
        .iter()
        .enumerate()
        .map(|(index, _)| CandidateScore {
            index,
            similarity: search_ml::cosine_similarity(query, &embeddings[index + 1])
                .unwrap_or(f32::NEG_INFINITY),
        })
        .collect::<Vec<_>>();
    scores.sort_by(|left, right| {
        right
            .similarity
            .total_cmp(&left.similarity)
            .then_with(|| left.index.cmp(&right.index))
    });
    Ok(scores)
}

/// Rank unchanged lines from indexed source passages against a question.
/// Similarity ranks candidate lines; it does not generate prose or prove an answer.
#[cfg(feature = "semantic-search")]
pub fn rank_cited_answers(
    question: &str,
    hits: &[crate::semantic_index::SemanticHit],
) -> Result<Vec<CitedAnswer>, search_ml::MlError> {
    let selected = crate::evidence::select_evidence(hits);
    let lines = selected
        .iter()
        .enumerate()
        .flat_map(|(hit_index, hit)| {
            hit.excerpt
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(move |line| (hit_index, line.to_string()))
        })
        .collect::<Vec<_>>();
    let mut best = Vec::new();
    for (batch_index, batch) in lines.chunks(ANSWER_CANDIDATE_BATCH).enumerate() {
        let candidates = batch
            .iter()
            .map(|(_, line)| line.clone())
            .collect::<Vec<_>>();
        for score in rank_suggestions(question, &candidates)? {
            best.push((
                score.similarity,
                batch_index * ANSWER_CANDIDATE_BATCH + score.index,
            ));
        }
        best.sort_by(|left, right| {
            right
                .0
                .total_cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
        });
        best.truncate(MAX_CITED_ANSWERS);
    }
    Ok(best
        .into_iter()
        .map(|(similarity, index)| CitedAnswer {
            hit: selected[lines[index].0].clone(),
            answer: lines[index].1.clone(),
            answer_similarity: similarity,
        })
        .collect())
}

/// Generate an answer from neural-ranked source lines and retain their citations.
#[cfg(feature = "semantic-search")]
pub fn answer_cited(
    question: &str,
    hits: &[crate::semantic_index::SemanticHit],
) -> Result<CitedQuestionAnswer, String> {
    let evidence = rank_cited_answers(question, hits).map_err(|error| error.to_string())?;
    if evidence.is_empty() {
        return Err("No cited source lines are available to answer this question.".into());
    }
    let context = evidence
        .iter()
        .map(|item| item.answer.as_str())
        .collect::<Vec<_>>();
    let (text, used_source_count) =
        search_ml::generate_answer(question, &context).map_err(|error| error.to_string())?;
    let evidence = evidence.into_iter().take(used_source_count).collect();
    Ok(CitedQuestionAnswer { text, evidence })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neural_ranking_places_semantic_suggestion_ahead_of_unrelated_candidate() {
        let candidates = vec![
            "government tax policy".to_string(),
            "A dog plays outside among plants.".to_string(),
        ];
        let ranked =
            rank_suggestions("A dog is playing outdoors in a garden.", &candidates).unwrap();
        assert_eq!(
            ranked.iter().map(|score| score.index).collect::<Vec<_>>(),
            [1, 0]
        );
        assert!(ranked[0].similarity > ranked[1].similarity);
    }

    #[test]
    fn extractive_summary_returns_original_cited_lines_in_source_order() {
        let text = "The titanium fitting transfers torque into the shaft.\nPaint the outside surface blue.\nTorque produces torsional stress in the shaft.";
        let summary =
            extractive_summary(text, &tokio_util::sync::CancellationToken::new()).unwrap();
        assert!(!summary.lines.is_empty());
        assert!(summary.lines.len() <= 5);
        assert!(summary
            .lines
            .windows(2)
            .all(|pair| pair[0].line_number < pair[1].line_number));
        for cited in summary.lines {
            assert_eq!(
                text.lines().nth(cited.line_number - 1),
                Some(cited.text.as_str())
            );
        }
    }

    #[test]
    fn extractive_summary_bounds_input_and_samples_lines_evenly() {
        let too_large = "x".repeat(1_048_577);
        assert!(
            extractive_summary(&too_large, &tokio_util::sync::CancellationToken::new()).is_err()
        );

        let text = (0..300)
            .map(|line| format!("Engineering note {line}: shaft load transfers torque."))
            .collect::<Vec<_>>()
            .join("\n");
        let summary =
            extractive_summary(&text, &tokio_util::sync::CancellationToken::new()).unwrap();
        assert_eq!(summary.source_lines, 300);
        assert_eq!(summary.sampled_lines, 256);
    }

    #[test]
    fn extractive_summary_handles_empty_and_cancelled_input() {
        let cancel = tokio_util::sync::CancellationToken::new();
        assert_eq!(
            extractive_summary(" \n\t\n", &cancel).unwrap(),
            ExtractiveSummary::default()
        );
        cancel.cancel();
        assert!(extractive_summary("Torque loads the shaft.", &cancel)
            .unwrap_err()
            .contains("cancelled"));
    }

    #[test]
    fn corpus_candidates_keep_engineering_phrases_and_are_bounded() {
        let lines = vec!["The M6 titanium bolt has high fatigue strength.".to_string(); 50];
        let candidates = corpus_term_candidates("bolt", &lines, 100);
        assert!(candidates.contains(&"titanium bolt".to_string()));
        assert!(candidates.contains(&"fatigue strength".to_string()));
        assert!(!candidates.iter().any(|term| term == "the" || term == "of"));
        assert!(candidates.len() <= 32);
    }

    #[cfg(feature = "semantic-search")]
    #[test]
    fn cited_answer_ranking_keeps_source_lines_and_does_not_invent_text() {
        let hits = vec![
            crate::semantic_index::SemanticHit {
                path: "tax.txt".into(),
                first_line: 20,
                last_line: 21,
                excerpt: "The tax code changed this year.".into(),
                similarity: 0.99,
            },
            crate::semantic_index::SemanticHit {
                path: "shaft.txt".into(),
                first_line: 8,
                last_line: 9,
                excerpt: "Torque causes torsional stress in the shaft.".into(),
                similarity: 0.6,
            },
        ];
        let answers = rank_cited_answers("What creates torsional stress?", &hits).unwrap();
        assert_eq!(answers[0].hit.path, "shaft.txt");
        assert_eq!(
            (answers[0].hit.first_line, answers[0].hit.last_line),
            (8, 9)
        );
        assert_eq!(answers[0].hit.excerpt, hits[1].excerpt);
    }

    #[cfg(feature = "semantic-search")]
    #[test]
    fn cited_question_answers_rank_unchanged_lines_and_keep_their_source_citation() {
        let hits = vec![
            crate::semantic_index::SemanticHit {
                path: "tax.txt".into(),
                first_line: 20,
                last_line: 21,
                excerpt: "The tax code changed this year.\nThe budget is approved in June.".into(),
                similarity: 0.99,
            },
            crate::semantic_index::SemanticHit {
                path: "shaft.txt".into(),
                first_line: 8,
                last_line: 9,
                excerpt: "The shaft carries torque.\nTorsional stress comes from applied torque.".into(),
                similarity: 0.6,
            },
        ];

        let answers = rank_cited_answers("What causes torsional stress?", &hits).unwrap();

        assert_eq!(answers[0].answer, "Torsional stress comes from applied torque.");
        assert_eq!(answers[0].hit.path, "shaft.txt");
        assert_eq!((answers[0].hit.first_line, answers[0].hit.last_line), (8, 9));
        assert_eq!(answers[0].hit.excerpt, hits[1].excerpt);
    }

    #[cfg(feature = "semantic-search")]
    #[test]
    fn generated_question_answer_keeps_ranked_source_citations() {
        let hit = crate::semantic_index::SemanticHit {
            path: "shaft.txt".into(),
            first_line: 8,
            last_line: 9,
            excerpt: "The shaft carries torque.\nTorsional stress comes from applied torque.".into(),
            similarity: 0.6,
        };

        let answer = answer_cited("What causes torsional stress?", &[hit]).unwrap();

        assert!(answer.text.to_lowercase().contains("torque"), "{}", answer.text);
        assert_eq!(answer.evidence[0].hit.path, "shaft.txt");
        assert_eq!(answer.evidence[0].answer, "Torsional stress comes from applied torque.");
    }

    #[cfg(feature = "semantic-search")]
    #[test]
    fn cited_question_answer_batches_do_not_drop_late_relevant_lines() {
        let mut lines = vec!["x".to_string(); 128];
        lines.push("Torque causes torsional stress.".into());
        let hit = crate::semantic_index::SemanticHit {
            path: "shaft.txt".into(),
            first_line: 1,
            last_line: 129,
            excerpt: lines.join("\n"),
            similarity: 0.6,
        };

        let answers = rank_cited_answers("What causes torsional stress?", &[hit]).unwrap();

        assert_eq!(answers[0].answer, "Torque causes torsional stress.");
        assert_eq!((answers[0].hit.first_line, answers[0].hit.last_line), (1, 129));
    }
}
