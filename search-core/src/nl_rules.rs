//! Rule-based reading of a natural-language Find sentence: no model, no network.
//!
//! [`parse`] splits a sentence into the conditions a document must (or must not) state, using only the sentence's own
//! words, so a condition can never be invented. [`judge`] decides whether one passage states one condition: every
//! key word (as a word form) must occur within one or two neighbouring sentences and not be negated there, and the
//! quote returned is those sentences, copied unchanged.

use crate::lexical;

pub const MAX_CONDITIONS: usize = 6;

/// One thing a document must (or must not) say.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    /// The sentence's own content words for this condition, in order.
    pub text: String,
    /// Lowercase key words and their stems, used to pick passages.
    pub terms: Vec<String>,
    /// The document must NOT satisfy this condition.
    pub negated: bool,
}

/// A judged passage. `quote` is a span of the passage, unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub matches: bool,
    pub quote: String,
}

/// Words that open a negated clause ("but not painted", "without a doubler").
const NEGATORS: &[&str] = &["not", "no", "never", "without", "except", "excluding", "nor", "cannot", "unable"];
/// Words that end one clause and start the next.
const CLAUSE_BREAKS: &[&str] = &["and", "but", "while", "plus", "also", "then", "yet"];
/// Words that start a description of the noun before them ("fasteners which size is specified").
const RELATIVE: &[&str] = &["which", "that", "where", "whose", "with", "having", "has", "have"];
/// Words that carry no condition by themselves.
const FILLER: &[&str] = &[
    "a", "an", "are", "as", "at", "be", "been", "by", "did", "do", "does", "for", "from", "had", "has", "have", "if",
    "in", "into", "is", "it", "its", "of", "on", "or", "so", "than", "that", "the", "there", "these", "this", "to",
    "was", "were", "where", "which", "with", "files", "file", "documents", "document", "records", "reports", "notes",
    "find", "search", "list", "show", "me", "any", "anything", "mention", "mentioning", "mentions", "about", "discuss",
];

/// A word without surrounding punctuation, lowercased.
fn clean(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn is_content(word: &str) -> bool {
    !word.is_empty()
        && !FILLER.contains(&word)
        && !NEGATORS.contains(&word)
        && !CLAUSE_BREAKS.contains(&word)
        && (word.chars().count() > 1 || word.chars().all(|c| c.is_ascii_digit()))
}

/// Shared leading letters, at least 4, so "blended" ties to "blend".
fn related(a: &str, b: &str) -> bool {
    a == b || a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count() >= 4
}

fn condition(words: &[String], negated: bool) -> Option<Condition> {
    let content: Vec<&String> = words.iter().filter(|w| is_content(w)).collect();
    if content.is_empty() {
        return None;
    }
    let mut terms: Vec<String> = Vec::new();
    for word in &content {
        for form in [word.to_string(), lexical::stem(word)] {
            if !terms.contains(&form) {
                terms.push(form);
            }
        }
    }
    Some(Condition { text: content.iter().map(|w| w.as_str()).collect::<Vec<_>>().join(" "), terms, negated })
}

/// Split a sentence into conditions. A clause ends at a comma, semicolon or a joining word (and, but, while ...);
/// a clause opened by "not", "no", "without" ... is negated; a clause of one verb-like word ("repaired") takes the
/// subject of the clause before it; the noun before "which"/"that"/"where" is put back in front of a clause that lost it.
pub fn parse(sentence: &str) -> Result<Vec<Condition>, String> {
    let mut clauses: Vec<(Vec<String>, bool)> = vec![(Vec::new(), false)];
    for token in sentence.split_whitespace() {
        let word = clean(token);
        let breaks_after = token.ends_with([',', ';', ':']);
        if CLAUSE_BREAKS.contains(&word.as_str()) {
            clauses.push((Vec::new(), false));
        } else if NEGATORS.contains(&word.as_str()) {
            if clauses.last().is_some_and(|(words, _)| words.iter().any(|w| is_content(w))) {
                clauses.push((Vec::new(), true));
            } else if let Some(last) = clauses.last_mut() {
                last.1 = true;
            }
        } else if !word.is_empty() {
            if let Some(last) = clauses.last_mut() {
                last.0.push(word);
            }
        }
        if breaks_after {
            clauses.push((Vec::new(), false));
        }
    }
    let mut out: Vec<Condition> = Vec::new();
    for (words, negated) in &clauses {
        let Some(mut found) = condition(words, *negated) else { continue };
        // "cracked and repaired": the second clause is one verb and shares the previous clause's subject.
        let verb_like = |w: &str| w.len() > 4 && (w.ends_with("ed") || w.ends_with("ing"));
        if !out.is_empty() && !found.text.contains(' ') && verb_like(&found.text) {
            let previous = &out[out.len() - 1].text;
            if let Some(subject) = previous.split(' ').next().filter(|s| !verb_like(s) && previous.contains(' ')) {
                let subject = subject.to_string();
                let with_subject: Vec<String> = std::iter::once(subject).chain(found.text.split(' ').map(str::to_string)).collect();
                found = condition(&with_subject, *negated).unwrap_or(found);
            }
        }
        if !out.iter().any(|c| c.text == found.text && c.negated == found.negated) && out.len() < MAX_CONDITIONS {
            out.push(found);
        }
    }
    if out.is_empty() {
        return Err("no condition in the sentence could be identified".into());
    }
    restore_subject(sentence, &mut out);
    Ok(out)
}

/// "fasteners which size is specified" is about fasteners, but the clause alone ("size specified") would match any
/// size anywhere. When the noun in front of the first relative word appears in no condition, it is put back in front
/// of each positive one.
fn restore_subject(sentence: &str, conditions: &mut [Condition]) {
    let all: Vec<String> = sentence.split_whitespace().map(clean).filter(|w| !w.is_empty()).collect();
    let Some(split) = all.iter().position(|w| RELATIVE.contains(&w.as_str())) else { return };
    let Some(head) = all[..split].iter().rev().find(|w| is_content(w)).cloned() else { return };
    if conditions.iter().any(|c| c.text.split(' ').any(|w| related(w, &head))) {
        return;
    }
    let singular = head.strip_suffix('s').filter(|root| root.len() > 2).map(str::to_string);
    for condition in conditions.iter_mut().filter(|c| !c.negated) {
        condition.text = format!("{head} {}", condition.text);
        for form in singular.iter().chain(std::iter::once(&head)) {
            if !condition.terms.contains(form) {
                condition.terms.push(form.clone());
            }
        }
    }
}

/// Sentences of `text`, ending at `.`, `?`, `!` or `;` followed by white space.
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        if matches!(c, '.' | '?' | '!' | ';') && text[end..].starts_with(char::is_whitespace) {
            out.push(text[start..end].trim());
            start = end;
        }
    }
    out.push(text[start..].trim());
    out.retain(|s| !s.is_empty());
    out
}

/// Whether a key word in `window` is negated ("was not blended", "without a doubler"): a negator within three words in
/// front of it.
fn negated_in(window: &str, wanted: &[String]) -> bool {
    let words: Vec<String> = window.split_whitespace().map(clean).collect();
    words.iter().enumerate().any(|(i, word)| {
        wanted.contains(&lexical::stem(word))
            && words[i.saturating_sub(3)..i].iter().any(|w| NEGATORS.contains(&w.as_str()) || w.ends_with("n't"))
    })
}

/// Longest quote returned, in characters.
const QUOTE_CHARS: usize = 300;

/// Does `passage` state `condition`? Every key word (all but one when there are four or more) must occur, as a word
/// form, in one sentence or two neighbouring ones that do not negate it. The best-covering shortest span is the quote.
pub fn judge(condition: &str, passage: &str) -> Verdict {
    let none = Verdict { matches: false, quote: String::new() };
    let mut wanted = lexical::query_terms_or_empty(condition);
    wanted.retain(|w| w.chars().count() >= 2 || w.chars().all(|c| c.is_ascii_digit()));
    if wanted.is_empty() {
        return none;
    }
    let need = if wanted.len() >= 4 { wanted.len() - 1 } else { wanted.len() };
    let text = passage.replace("\u{ad} ", "").replace('\u{ad}', "");
    let parts = sentences(&text);
    let mut best: Option<(usize, usize, String)> = None;
    for width in 1..=2usize {
        for window in parts.windows(width) {
            let span = window.join(" ");
            let words = lexical::terms(&span);
            let covered = wanted.iter().filter(|w| words.contains(w)).count();
            if covered < need || negated_in(&span, &wanted) {
                continue;
            }
            if best.as_ref().is_none_or(|(c, w, _)| (covered, std::cmp::Reverse(width)) > (*c, std::cmp::Reverse(*w))) {
                best = Some((covered, width, span));
            }
        }
    }
    match best {
        Some((_, _, span)) => Verdict { matches: true, quote: span.chars().take(QUOTE_CHARS).collect() },
        None => none,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sentence: &str) -> Vec<(String, bool)> {
        parse(sentence).unwrap().into_iter().map(|c| (c.text, c.negated)).collect()
    }

    #[test]
    fn splits_at_joining_words_and_keeps_only_the_sentences_words() {
        assert_eq!(texts("files where corrosion is blended, and the surface filled and faired"), [
            ("corrosion blended".to_string(), false),
            ("surface filled".to_string(), false),
            ("surface faired".to_string(), false),
        ]);
    }

    #[test]
    fn a_lone_verb_takes_the_subject_of_the_first_clause_but_a_lone_noun_does_not() {
        assert_eq!(texts("files where the skin was cracked and repaired but not painted"), [
            ("skin cracked".to_string(), false),
            ("skin repaired".to_string(), false),
            ("skin painted".to_string(), true),
        ]);
        assert_eq!(texts("bolts and nuts"), [("bolts".to_string(), false), ("nuts".to_string(), false)]);
    }

    #[test]
    fn negators_open_a_negated_clause() {
        assert_eq!(texts("hydrostatic test without corrosion allowance"), [
            ("hydrostatic test".to_string(), false),
            ("corrosion allowance".to_string(), true),
        ]);
        assert_eq!(texts("not painted"), [("painted".to_string(), true)]);
    }

    #[test]
    fn a_dropped_subject_is_put_back_in_front_of_the_clause() {
        let found = parse("search fasteners which size is specified?").unwrap();
        assert_eq!(found[0].text, "fasteners size specified");
        assert!(found[0].terms.contains(&"fastener".to_string()));
        assert_eq!(texts("find holes that have a diameter given")[0].0, "holes diameter given");
        assert_eq!(texts("files where the skin was cracked")[0].0, "skin cracked");
    }

    #[test]
    fn stems_join_the_terms_and_filler_alone_is_an_error() {
        let found = parse("corrosion blended").unwrap();
        assert_eq!(found[0].terms, ["corrosion", "blended", "blend"]);
        assert!(parse("find the files where it is").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn duplicates_collapse_and_conditions_are_capped() {
        assert_eq!(parse("rust and rust").unwrap().len(), 1);
        assert_eq!(parse("a1 b2 and c3 and d4 and e5 and f6 and g7 and h8").unwrap().len(), MAX_CONDITIONS);
    }

    #[test]
    fn a_passage_states_a_condition_when_all_words_share_a_sentence() {
        let passage = "Remove the battery. Light surface corrosion was removed by blending with an abrasive pad. Check the tires.";
        let verdict = judge("corrosion blended", passage);
        assert!(verdict.matches, "{verdict:?}");
        assert!(verdict.quote.contains("corrosion was removed by blending"));
        assert!(passage.contains(&verdict.quote));
        // Words spread over unrelated sentences do not count.
        assert!(!judge("battery tires", passage).matches);
        assert!(!judge("paint", passage).matches);
    }

    #[test]
    fn two_neighbouring_sentences_may_share_a_condition() {
        assert!(judge("skin repaired", "The skin cracked near the flange. It was repaired with a doubler.").matches);
        assert!(!judge("skin repaired", "The skin cracked. Check the tires. It was repaired.").matches);
        assert!(judge("skin repaired", "The skin was inspected. The skin repair used a doubler.").matches);
        assert!(judge("flange doubler", "A crack ran along the flange. A doubler was fitted.").matches);
    }

    #[test]
    fn a_negated_statement_does_not_confirm_the_condition() {
        assert!(!judge("skin painted", "The skin was not painted.").matches);
        assert!(!judge("doubler fitted", "A repair without a doubler fitted.").matches);
        assert!(judge("skin painted", "The skin was painted.").matches);
    }

    #[test]
    fn four_or_more_key_words_may_miss_one() {
        let passage = "Install the AN3-5A bolt with a washer under the nut.";
        assert!(judge("install bolt washer nut torque", passage).matches);
        assert!(!judge("install bolt washer torque sealant", passage).matches);
    }

    #[test]
    fn line_end_hyphens_are_rejoined() {
        assert!(judge("corrosion removed", "The corrosion was re\u{ad} moved by hand.").matches);
    }
}
