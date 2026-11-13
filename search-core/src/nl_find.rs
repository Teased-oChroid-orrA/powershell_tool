//! Natural-language Find orchestration.
//!
//! A sentence is split into conditions by rules (`nl_rules`, no model). Each
//! condition is retrieved on its own through the hybrid semantic index, the
//! strongest passages of the most promising files are judged by word-form
//! coverage, and a file is labelled by its weakest condition. Nothing is
//! generated: every match carries the unchanged cited passage and a quote copied
//! from it. Negated conditions only exclude files.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

pub use crate::nl_rules::{judge, parse, Condition, Verdict};

use crate::semantic_index::SemanticHit;

/// Passages retrieved per condition; files lacking a hit inside it are not candidates.
const RETRIEVE_PER_CONDITION: usize = 100;
/// Files judged per search: only the best-ranked files are checked.
const MAX_JUDGED_FILES: usize = 10;
/// Files offered as unconfirmed closest passages when no passage was confirmed, and the length of each shown passage.
const CLOSEST_FILES: usize = 5;
const CLOSEST_QUOTE_CHARS: usize = 200;
/// Passages judged per (file, condition) pair: many for a file that is among the best matches (a question such
/// as "fasteners with a size specified" is satisfied by a few passages in hundreds, and judging stops at the first
/// confirmation), few for a file much less similar than the best one.
const STRONG_PASSAGES: usize = 8;
const WEAK_PASSAGES: usize = 1;
/// A file is "strong" for a condition when its best passage is at least this share of the best file's.
const STRONG_SHARE: f32 = 0.5;

/// How well a file satisfies the positive conditions. Not a percentage on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    /// Nothing was confirmed; these are the best-retrieved passages, shown so a search is never a dead end.
    Unconfirmed,
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
            Fit::Unconfirmed => "not confirmed",
        }
    }
}

/// A passage that states a condition.
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence {
    pub hit: SemanticHit,
    /// Words copied from the passage; verified to occur in it.
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
}

impl std::fmt::Display for FindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindError::NoPositiveCondition => {
                f.write_str("at least one condition must not be negated")
            }
            FindError::Retrieval(error) => write!(f, "search failed: {error}"),
        }
    }
}
impl std::error::Error for FindError {}

/// Longest focused passage handed to the judge, in characters.
const FOCUS_CHARS: usize = 600;
const FOCUS_SENTENCES: usize = 3;

/// The distinct word forms of a condition's terms, as the keyword scorer reads them.
fn wanted_stems(condition: &Condition) -> Vec<String> {
    let mut stems: Vec<String> = condition
        .terms
        .iter()
        .map(String::as_str)
        .flat_map(crate::lexical::query_terms_or_empty)
        .collect();
    // Single characters and bare numbers (the `x` and `8` of "M8 x 1.25") match scraps of every passage.
    stems.retain(|stem| stem.chars().count() >= 3 && !stem.chars().all(|c| c.is_ascii_digit() || c == '.'));
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
/// and often starts mid-sentence or holds a neighbouring topic, which loses the sentence in. The
/// sentences that share the most word forms with the condition's terms are kept with the sentence on either side,
/// in their original order; a passage sharing none is handed over whole. Line-end hyphens (`re­ moved`) are rejoined.
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
    let all: Vec<String> = sentences.iter().map(|sentence| sentence.2.to_string()).collect();
    sentences.retain(|sentence| sentence.0 > 0 && !sentence.2.is_empty());
    if sentences.is_empty() {
        return text;
    }
    // Best sentences first (a fragment of a few characters is rarely the answer), each with its neighbours: a
    // passage is cut from running text, so the sentence beside a match often completes it.
    let total = all.len();
    sentences.sort_by(|a, b| {
        (b.0, b.2.len().min(40)).cmp(&(a.0, a.2.len().min(40))).then(a.1.cmp(&b.1))
    });
    sentences.truncate(FOCUS_SENTENCES);
    let mut wanted_indices: Vec<usize> = Vec::new();
    for sentence in &sentences {
        for index in sentence.1.saturating_sub(1)..=(sentence.1 + 1).min(total - 1) {
            if !wanted_indices.contains(&index) {
                wanted_indices.push(index);
            }
        }
    }
    wanted_indices.sort_unstable();
    let joined = wanted_indices
        .iter()
        .map(|&i| all[i].as_str())
        .filter(|sentence| !sentence.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    joined.chars().take(FOCUS_CHARS).collect()
}

fn judge_pair(
    condition: &Condition,
    hits: &[&SemanticHit],
    limit: usize,
) -> Option<Evidence> {
    // Two signals pick the passages to judge. Retrieval order is right when the condition is
    // concrete; word-form coverage is right when meaning alone surfaces a contents page or a neighbouring topic.
    // They are taken alternately, best first, until `limit` passages are chosen.
    let wanted = wanted_stems(condition);
    let mut by_coverage: Vec<&SemanticHit> = hits.to_vec();
    by_coverage.sort_by_key(|hit| std::cmp::Reverse(coverage(&hit.excerpt, &wanted)));
    let mut order: Vec<&SemanticHit> = Vec::new();
    'pick: for rank in 0..hits.len() {
        for hit in [hits[rank], by_coverage[rank]] {
            if !order.iter().any(|seen| std::ptr::eq(*seen, hit)) {
                order.push(hit);
                if order.len() >= limit {
                    break 'pick;
                }
            }
        }
    }
    for hit in order {
        let focused = focus(&hit.excerpt, condition);
        let verdict = judge(&condition.text, &focused);
        if verdict.matches {
            return Some(Evidence {
                hit: hit.clone(),
                quote: verdict.quote,
            });
        }
    }
    None
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
/// before the next passage is judged. Returns files best first (fit, then weakest evidence).
pub fn find<R>(
    conditions: &[Condition],
    retrieve: R,
    cancel: &AtomicBool,
    mut on_file: impl FnMut(&FileMatch),
) -> Result<Vec<FileMatch>, FindError>
where
    R: Fn(&str, usize) -> Result<Vec<SemanticHit>, String>,
{
    let positive: Vec<&Condition> = conditions.iter().filter(|c| !c.negated).collect();
    if positive.is_empty() {
        return Err(FindError::NoPositiveCondition);
    }
    let negative: Vec<&Condition> = conditions.iter().filter(|c| c.negated).collect();
    let retrieve_for = |condition: &Condition| {
        let hits = retrieve(&condition.text, RETRIEVE_PER_CONDITION).map_err(FindError::Retrieval)?;
        Ok(by_file(hits))
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

    let candidates: Vec<&str> = ranked.iter().map(|(path, _)| *path).collect();
    // Per condition: how similar a file's best passage must be to count as a strong candidate.
    let strong_floor = |lists: &Vec<HashMap<String, Vec<SemanticHit>>>| -> Vec<f32> {
        lists
            .iter()
            .map(|files| files.values().map(|hits| hits[0].similarity).fold(0.0, f32::max) * STRONG_SHARE)
            .collect()
    };
    let positive_floor = strong_floor(&positive_hits);
    let negative_floor = strong_floor(&negative_hits);
    let passages_for = |hits: &[SemanticHit], floor: f32| {
        if hits[0].similarity >= floor { STRONG_PASSAGES } else { WEAK_PASSAGES }
    };
    let mut matches = Vec::new();
    'files: for (path, _) in ranked {
        // Exclusion first: a confirmed negated condition removes the file.
        for ((condition, files), floor) in negative.iter().zip(&negative_hits).zip(&negative_floor) {
            if let Some(hits) = files.get(path) {
                if cancel.load(Ordering::Relaxed) {
                    break 'files;
                }
                let refs: Vec<&SemanticHit> = hits.iter().collect();
                if judge_pair(condition, &refs, passages_for(hits, *floor)).is_some() {
                    continue 'files;
                }
            }
        }
        let mut evidence = Vec::with_capacity(positive.len());
        for ((condition, files), floor) in positive.iter().zip(&positive_hits).zip(&positive_floor) {
            let found = match files.get(path) {
                Some(hits) => {
                    if cancel.load(Ordering::Relaxed) {
                        break 'files;
                    }
                    let refs: Vec<&SemanticHit> = hits.iter().collect();
                    judge_pair(condition, &refs, passages_for(hits, *floor))
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
    if matches.is_empty() && !cancel.load(Ordering::Relaxed) {
        // Nothing confirmed: offer the best passage of the best files, marked as unconfirmed.
        let best_of = |path: &str| {
            positive_hits
                .iter()
                .filter_map(|files| files.get(path).and_then(|hits| hits.first()))
                .map(|hit| hit.similarity)
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let top = candidates.iter().map(|path| best_of(path)).fold(f32::NEG_INFINITY, f32::max);
        // A file whose best passage is far less like the sentence than the best file's is noise, not a near miss.
        let floor = (top * 0.5).max(0.2);
        for path in candidates.into_iter().filter(|path| best_of(path) >= floor).take(CLOSEST_FILES) {
            let evidence: Vec<Option<Evidence>> = positive_hits
                .iter()
                .zip(&positive)
                .map(|(files, condition)| {
                    files.get(path).and_then(|hits| hits.first()).map(|hit| Evidence {
                        hit: hit.clone(),
                        quote: focus(&hit.excerpt, condition).chars().take(CLOSEST_QUOTE_CHARS).collect(),
                    })
                })
                .collect();
            matches.push(FileMatch {
                path: path.to_string(),
                fit: Fit::Unconfirmed,
                evidence,
            });
        }
        return Ok(matches);
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

    fn cond(text: &str, negated: bool) -> Condition {
        Condition { text: text.into(), terms: vec![text.into()], negated, }
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
        // Retrieval is loose: every passage comes back, the judge decides.
        let _ = query;
        Ok(all.to_vec())
    }

    fn run(conditions: &[Condition]) -> Vec<FileMatch> {
        find(conditions, corpus, &AtomicBool::new(false), |_| {}).unwrap()
    }

    #[test]
    fn focus_keeps_the_sentences_about_the_condition() {
        let passage = "Remove the battery. Drain the tank. Check the tires. Protect the surfaces next to rework areas. 6-115. All depressions should be faired or blended with the surrounding surface. Fairing can be done as follows: remove edges";
        let blended = Condition { text: "surfaces blended".into(), terms: vec!["surface".into(), "blend".into(), "blended".into()], negated: false };
        let focused = focus(passage, &blended);
        assert!(focused.contains("faired or blended with the surrounding surface"), "{focused}");
        assert!(!focused.contains("battery"), "{focused}");
        let nothing = Condition { text: "paint".into(), terms: vec!["paint".into()], negated: false };
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
    fn when_nothing_is_confirmed_the_closest_passages_are_offered_as_unconfirmed() {
        let found = run(&[cond("zzzzzz", false)]);
        assert!(!found.is_empty());
        assert!(found.iter().all(|m| m.fit == Fit::Unconfirmed), "{found:?}");
        assert!(found.len() <= CLOSEST_FILES);
        let evidence = found[0].evidence[0].as_ref().expect("the best passage is shown");
        assert!(!evidence.quote.is_empty());
        // A confirmed match suppresses them.
        assert!(run(&[cond("blended", false)]).iter().all(|m| m.fit != Fit::Unconfirmed));
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
        assert_eq!(evidence.quote, "blended filled faired");
    }

    #[test]
    fn only_negated_conditions_is_an_error() {
        let result = find(
            &[cond("painted", true)],
            corpus,
            &AtomicBool::new(false),
            |_| {},
        );
        assert_eq!(result.unwrap_err(), FindError::NoPositiveCondition);
    }

    #[test]
    fn cancel_stops_before_judging_and_progress_streams() {
        let cancel = AtomicBool::new(true);
        assert!(find(
            &[cond("blended", false)],
            corpus,
            &cancel,
            |_| panic!()
        )
        .unwrap()
        .is_empty());
        let mut seen = 0;
        find(
            &[cond("blended", false)],
            corpus,
            &AtomicBool::new(false),
            |_| seen += 1,
        )
        .unwrap();
        assert_eq!(seen, 2);
    }
}
