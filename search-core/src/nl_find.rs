//! Natural-language Find orchestration.
//!
//! A sentence is split into conditions by a local language model (`search_llm`).
//! Each condition is retrieved on its own through the hybrid semantic index, the
//! model judges the strongest passages of the most promising files, and a file
//! is labelled by its weakest condition. The model never writes answers: every
//! match carries the unchanged cited passage and a quote verified against it.
//! Negated conditions only exclude files.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use search_llm::{Condition, ConditionModel};

use crate::semantic_index::SemanticHit;

/// Passages retrieved per condition; files lacking a hit inside it are not candidates.
const RETRIEVE_PER_CONDITION: usize = 100;
/// Files judged per search: the model is slow, so only the best-ranked files are checked.
const MAX_JUDGED_FILES: usize = 10;
/// Passages judged per (file, condition) pair.
const PASSAGES_PER_PAIR: usize = 3;

/// How well a file satisfies the positive conditions. Not a percentage on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    Partial,
    OneWeak,
    AllMatched,
}

impl Fit {
    pub fn label(self) -> &'static str {
        match self {
            Fit::AllMatched => "all matched",
            Fit::OneWeak => "one weak",
            Fit::Partial => "partial",
        }
    }
}

/// A passage the model confirmed states a condition.
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence {
    pub hit: SemanticHit,
    /// Words copied from the passage by the model; verified to occur in it.
    pub quote: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileMatch {
    pub path: String,
    pub fit: Fit,
    /// One entry per positive condition, in order; `None` where nothing confirmed it.
    pub evidence: Vec<Option<Evidence>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindError {
    NoPositiveCondition,
    Retrieval(String),
    Model(String),
}

impl std::fmt::Display for FindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindError::NoPositiveCondition => {
                f.write_str("at least one condition must not be negated")
            }
            FindError::Retrieval(error) => write!(f, "search failed: {error}"),
            FindError::Model(error) => write!(f, "language model failed: {error}"),
        }
    }
}
impl std::error::Error for FindError {}

/// Longest focused passage handed to the model, in characters.
const FOCUS_CHARS: usize = 600;
const FOCUS_SENTENCES: usize = 3;

/// The distinct word forms of a condition's terms, as the keyword scorer reads them.
fn wanted_stems(condition: &Condition) -> Vec<String> {
    let mut stems: Vec<String> = condition
        .terms
        .iter()
        .flat_map(|term| crate::lexical::terms(term))
        .collect();
    stems.sort();
    stems.dedup();
    stems
}

/// How many of `wanted` occur in `text`.
fn coverage(text: &str, wanted: &[String]) -> usize {
    let words = crate::lexical::terms(text);
    wanted.iter().filter(|w| words.contains(w)).count()
}

/// The part of a retrieved passage that bears on the condition. A passage is a fixed-size cut of the document
/// and often starts mid-sentence or holds a neighbouring topic, which a small model loses the sentence in. The
/// sentences that share the most word forms with the condition's terms are kept, in their original order;
/// a passage sharing none is handed over whole. Line-end hyphens (`re­ moved`) are rejoined.
fn focus(passage: &str, condition: &Condition) -> String {
    let text = passage.replace("\u{ad} ", "").replace('\u{ad}', "");
    let wanted = wanted_stems(condition);
    let mut sentences: Vec<(usize, usize, &str)> = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        if matches!(c, '.' | '?' | '!' | ';') && text[end..].starts_with(char::is_whitespace) {
            sentences.push((0, sentences.len(), text[start..end].trim()));
            start = end;
        }
    }
    sentences.push((0, sentences.len(), text[start..].trim()));
    for sentence in &mut sentences {
        sentence.0 = coverage(sentence.2, &wanted);
    }
    sentences.retain(|sentence| sentence.0 > 0 && !sentence.2.is_empty());
    if sentences.is_empty() {
        return text;
    }
    sentences.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    sentences.truncate(FOCUS_SENTENCES);
    sentences.sort_by_key(|sentence| sentence.1);
    let joined = sentences.iter().map(|s| s.2).collect::<Vec<_>>().join(" ");
    joined.chars().take(FOCUS_CHARS).collect()
}

fn judge_pair<M: ConditionModel + ?Sized>(
    model: &M,
    condition: &Condition,
    hits: &[&SemanticHit],
) -> Result<Option<Evidence>, FindError> {
    // The passages that hold the most of the condition's word forms are judged first: a short query's
    // nearest passages by meaning alone are often a contents page or a neighbouring topic.
    let wanted = wanted_stems(condition);
    let mut order: Vec<&&SemanticHit> = hits.iter().collect();
    order.sort_by_key(|hit| std::cmp::Reverse(coverage(&hit.excerpt, &wanted)));
    for hit in order.into_iter().take(PASSAGES_PER_PAIR) {
        let verdict = model
            .judge(&condition.text, &focus(&hit.excerpt, condition))
            .map_err(FindError::Model)?;
        if verdict.matches {
            return Ok(Some(Evidence {
                hit: (*hit).clone(),
                quote: verdict.quote,
            }));
        }
    }
    Ok(None)
}

/// Group hits by file, keeping the retrieval order within each file.
fn by_file(hits: Vec<SemanticHit>) -> HashMap<String, Vec<SemanticHit>> {
    let mut files: HashMap<String, Vec<SemanticHit>> = HashMap::new();
    for hit in hits {
        files.entry(hit.path.clone()).or_default().push(hit);
    }
    files
}

/// Run a parsed sentence. `retrieve` returns scored passages for one condition
/// text (normally `SemanticIndex::search_with_settings`). `on_file` is called as each
/// judged file with at least one confirmed condition is finished; `cancel` stops
/// before the next model call. Returns files best first (fit, then weakest evidence).
pub fn find<M, R>(
    model: &M,
    conditions: &[Condition],
    retrieve: R,
    cancel: &AtomicBool,
    mut on_file: impl FnMut(&FileMatch),
) -> Result<Vec<FileMatch>, FindError>
where
    M: ConditionModel + ?Sized,
    R: Fn(&str, usize) -> Result<Vec<SemanticHit>, String>,
{
    let positive: Vec<&Condition> = conditions.iter().filter(|c| !c.negated).collect();
    if positive.is_empty() {
        return Err(FindError::NoPositiveCondition);
    }
    let negative: Vec<&Condition> = conditions.iter().filter(|c| c.negated).collect();
    let retrieve_for = |condition: &Condition| {
        retrieve(&condition.text, RETRIEVE_PER_CONDITION)
            .map(by_file)
            .map_err(FindError::Retrieval)
    };
    let positive_hits = positive
        .iter()
        .map(|c| retrieve_for(c))
        .collect::<Result<Vec<_>, _>>()?;
    let negative_hits = negative
        .iter()
        .map(|c| retrieve_for(c))
        .collect::<Result<Vec<_>, _>>()?;

    // Rank candidate files: most conditions retrieved, then strongest weakest-link similarity.
    let mut candidates: HashMap<&str, (usize, f32)> = HashMap::new();
    for files in &positive_hits {
        for (path, hits) in files {
            let entry = candidates
                .entry(path.as_str())
                .or_insert((0, f32::INFINITY));
            entry.0 += 1;
            entry.1 = entry.1.min(hits[0].similarity);
        }
    }
    let mut ranked: Vec<(&str, (usize, f32))> = candidates.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1 .0
            .cmp(&a.1 .0)
            .then(b.1 .1.total_cmp(&a.1 .1))
            .then(a.0.cmp(b.0))
    });
    ranked.truncate(MAX_JUDGED_FILES);

    let mut matches = Vec::new();
    'files: for (path, _) in ranked {
        // Exclusion first: a confirmed negated condition removes the file.
        for (condition, files) in negative.iter().zip(&negative_hits) {
            if let Some(hits) = files.get(path) {
                if cancel.load(Ordering::Relaxed) {
                    break 'files;
                }
                let refs: Vec<&SemanticHit> = hits.iter().collect();
                if judge_pair(model, condition, &refs)?.is_some() {
                    continue 'files;
                }
            }
        }
        let mut evidence = Vec::with_capacity(positive.len());
        for (condition, files) in positive.iter().zip(&positive_hits) {
            let found = match files.get(path) {
                Some(hits) => {
                    if cancel.load(Ordering::Relaxed) {
                        break 'files;
                    }
                    let refs: Vec<&SemanticHit> = hits.iter().collect();
                    judge_pair(model, condition, &refs)?
                }
                None => None,
            };
            evidence.push(found);
        }
        let confirmed = evidence.iter().flatten().count();
        if confirmed == 0 {
            continue;
        }
        let fit = if confirmed == positive.len() {
            Fit::AllMatched
        } else if positive.len() >= 3 && confirmed + 1 == positive.len() {
            Fit::OneWeak
        } else {
            Fit::Partial
        };
        let file = FileMatch {
            path: path.to_string(),
            fit,
            evidence,
        };
        on_file(&file);
        matches.push(file);
    }
    let weakest = |m: &FileMatch| {
        m.evidence
            .iter()
            .flatten()
            .map(|e| e.hit.similarity)
            .fold(f32::INFINITY, f32::min)
    };
    matches.sort_by(|a, b| {
        b.fit
            .cmp(&a.fit)
            .then(weakest(b).total_cmp(&weakest(a)))
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use search_llm::Verdict;

    /// Judges "yes" when the passage contains every word of the condition's first term.
    struct Fake;
    impl ConditionModel for Fake {
        fn parse(&self, _: &str) -> Result<Vec<Condition>, String> {
            unreachable!()
        }
        fn judge(&self, condition: &str, passage: &str) -> Result<Verdict, String> {
            let hit = passage.contains(condition);
            Ok(Verdict {
                matches: hit,
                quote: if hit {
                    condition.to_string()
                } else {
                    String::new()
                },
            })
        }
    }

    fn cond(text: &str, negated: bool) -> Condition {
        Condition {
            text: text.into(),
            terms: vec![text.into()],
            negated,
            grounding: 1.0,
        }
    }

    fn hit(path: &str, excerpt: &str, similarity: f32) -> SemanticHit {
        SemanticHit {
            path: path.into(),
            first_line: 1,
            last_line: 1,
            excerpt: excerpt.into(),
            similarity,
            page: None,
        }
    }

    fn corpus(query: &str, _: usize) -> Result<Vec<SemanticHit>, String> {
        let all = [
            hit("a.txt", "blended filled faired", 0.9),
            hit("b.txt", "blended only", 0.8),
            hit("b.txt", "painted over", 0.4),
            hit("c.txt", "filled faired", 0.7),
        ];
        // Retrieval is loose: every passage comes back, the model decides.
        let _ = query;
        Ok(all.to_vec())
    }

    fn run(conditions: &[Condition]) -> Vec<FileMatch> {
        find(&Fake, conditions, corpus, &AtomicBool::new(false), |_| {}).unwrap()
    }

    #[test]
    fn focus_keeps_the_sentences_about_the_condition() {
        let passage = "Remove the battery. Protect the surfaces next to rework areas. 6-115. All depressions should be faired or blended with the surrounding surface. Fairing can be done as follows: remove edges";
        let blended = Condition { text: "surfaces blended".into(), terms: vec!["surface".into(), "blend".into(), "blended".into()], negated: false, grounding: 1.0 };
        let focused = focus(passage, &blended);
        assert!(focused.contains("faired or blended with the surrounding surface"), "{focused}");
        assert!(!focused.contains("battery"), "{focused}");
        let nothing = Condition { text: "paint".into(), terms: vec!["paint".into()], negated: false, grounding: 1.0 };
        assert_eq!(focus(passage, &nothing), passage);
        assert_eq!(focus("a re\u{ad} moved edge is blended", &blended), "a removed edge is blended");
    }

    #[test]
    fn labels_files_by_how_many_positive_conditions_are_confirmed() {
        let found = run(&[cond("blended", false), cond("filled", false)]);
        let fits: Vec<(&str, Fit)> = found.iter().map(|m| (m.path.as_str(), m.fit)).collect();
        assert_eq!(fits[0], ("a.txt", Fit::AllMatched));
        assert!(fits[1..].iter().all(|(_, fit)| *fit == Fit::Partial));
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn one_weak_needs_three_conditions() {
        let found = run(&[
            cond("blended", false),
            cond("filled", false),
            cond("faired", false),
        ]);
        assert_eq!(found[0].path, "a.txt");
        assert_eq!(found[0].fit, Fit::AllMatched);
        // c.txt confirms filled + faired but not blended: one condition weak.
        assert_eq!(
            found.iter().find(|m| m.path == "c.txt").unwrap().fit,
            Fit::OneWeak
        );
    }

    #[test]
    fn negated_condition_excludes_files() {
        let found = run(&[cond("blended", false), cond("painted", true)]);
        assert!(found.iter().all(|m| m.path != "b.txt"));
        assert!(found.iter().any(|m| m.path == "a.txt"));
    }

    #[test]
    fn evidence_is_the_cited_passage_with_quote() {
        let found = run(&[cond("blended", false)]);
        let evidence = found[0].evidence[0].as_ref().unwrap();
        assert_eq!(evidence.hit.path, "a.txt");
        assert_eq!(evidence.quote, "blended");
    }

    #[test]
    fn only_negated_conditions_is_an_error() {
        let result = find(
            &Fake,
            &[cond("painted", true)],
            corpus,
            &AtomicBool::new(false),
            |_| {},
        );
        assert_eq!(result.unwrap_err(), FindError::NoPositiveCondition);
    }

    #[test]
    fn cancel_stops_before_model_calls_and_progress_streams() {
        let cancel = AtomicBool::new(true);
        assert!(find(
            &Fake,
            &[cond("blended", false)],
            corpus,
            &cancel,
            |_| panic!()
        )
        .unwrap()
        .is_empty());
        let mut seen = 0;
        find(
            &Fake,
            &[cond("blended", false)],
            corpus,
            &AtomicBool::new(false),
            |_| seen += 1,
        )
        .unwrap();
        assert_eq!(seen, 2);
    }
}
