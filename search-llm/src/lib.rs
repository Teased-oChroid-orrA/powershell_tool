//! Local language-model helpers for natural-language Find.
//!
//! The model never writes answers: it splits a sentence into conditions and
//! says whether one passage states one condition. Everything it returns is
//! validated here (conditions must come from the sentence, a "yes" must quote
//! the passage) so a model slip cannot invent a condition or a match.
//! The llama.cpp engine is behind the `llama` feature; the trait, prompts,
//! grammars and validation need no native code.

#[cfg(feature = "llama")]
mod engine;
#[cfg(feature = "llama")]
pub use engine::{LlamaEngine, ModelError};

/// One thing a document must (or must not) say.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub text: String,
    /// Lowercase key words and word-form variants, used by keyword scoring.
    pub terms: Vec<String>,
    /// The document must NOT satisfy this condition.
    pub negated: bool,
    /// Share of the condition's content words found in the user's sentence.
    /// Below 1.0 the condition may be partly invented: the UI asks the user to confirm it.
    pub grounding: f32,
}

impl Condition {
    pub fn needs_review(&self) -> bool {
        self.grounding < 1.0
    }
}

/// A judged passage. `quote` is verified to appear in the passage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub matches: bool,
    pub quote: String,
}

/// What `search-core` needs from a model; tests use a fake.
pub trait ConditionModel {
    fn parse(&self, sentence: &str) -> Result<Vec<Condition>, String>;
    fn judge(&self, condition: &str, passage: &str) -> Result<Verdict, String>;
}

pub const PARSE_SYSTEM: &str = "Split the user's search sentence into the independent conditions a document must satisfy.\n\
For each condition give: text (short phrase), terms (lowercase key words plus word-form variants, keep numbers and codes), negated (true only if the document must NOT satisfy it).\n\
Every condition must be made of words from the sentence. Never add, infer or generalise a condition; if the sentence states one thing, return one condition. When several actions share one subject, repeat the subject in every condition. Example:\n\
Sentence: files where the skin was cracked and repaired but not painted\n\
{\"conditions\":[{\"text\":\"skin cracked\",\"terms\":[\"skin\",\"crack\",\"cracked\"],\"negated\":false},{\"text\":\"skin repaired\",\"terms\":[\"skin\",\"repair\",\"repaired\"],\"negated\":false},{\"text\":\"skin painted\",\"terms\":[\"skin\",\"paint\",\"painted\"],\"negated\":true}]}";

pub const JUDGE_SYSTEM: &str = "Decide whether the passage itself asserts the condition. The passage is a cut-out of a longer document: it may begin or end mid-sentence and hold unrelated text, so look through all of it. First copy the exact words from the passage that state it (empty string if none). Then answer yes if the passage states, requires or instructs the condition for the same item (an instruction such as 'should be blended' counts). Answer no for a negation, a requirement that was not met, a different item, a different property of the same item (a preload is not a torque), or a mere mention of related words.";

/// GBNF forcing the parse JSON shape (fixed key order, no whitespace).
pub const PARSE_GRAMMAR: &str = r#"root ::= "{\"conditions\":[" cond ("," cond){0,5} "]}"
cond ::= "{\"text\":" str ",\"terms\":[" str ("," str){0,11} "],\"negated\":" bool "}"
str ::= "\"" [^"\\\n]{0,80} "\""
bool ::= "true" | "false""#;

pub const JUDGE_GRAMMAR: &str = r#"root ::= "{ \"quote\": " str ", \"verdict\": " ("\"yes\"" | "\"no\"") "}"
str ::= "\"" [^"\\\n]{0,300} "\"""#;

pub const MAX_CONDITIONS: usize = 6;

const FILLER: &[&str] = &[
    "a",
    "an",
    "and",
    "are",
    "as",
    "at",
    "be",
    "been",
    "but",
    "by",
    "did",
    "do",
    "does",
    "for",
    "from",
    "had",
    "has",
    "have",
    "if",
    "in",
    "into",
    "is",
    "it",
    "its",
    "no",
    "not",
    "of",
    "on",
    "or",
    "so",
    "than",
    "that",
    "the",
    "then",
    "there",
    "these",
    "this",
    "to",
    "was",
    "were",
    "where",
    "which",
    "while",
    "with",
    "files",
    "file",
    "documents",
    "document",
    "records",
    "reports",
    "notes",
    "find",
    "show",
    "me",
    "any",
    "anything",
    "mention",
    "mentioning",
    "mentions",
    "about",
    "discuss",
];

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Words that carry meaning: not filler, and not a lone letter.
fn content_words(text: &str) -> Vec<String> {
    words(text)
        .into_iter()
        .filter(|w| {
            !FILLER.contains(&w.as_str()) && (w.len() > 1 || w.chars().all(|c| c.is_ascii_digit()))
        })
        .collect()
}

/// Shared leading letters, at least 4, so "blended" ties to "blend".
fn related(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let common = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    common >= 4
        || (common >= 2
            && common == a.len().min(b.len())
            && a.len().min(b.len()) >= 2
            && a.chars().any(|c| c.is_ascii_digit()))
}

/// Parse the model's JSON and keep only conditions grounded in the sentence:
/// a condition survives when at least half of its own content words are related
/// to words of the sentence, and its terms are limited to word forms of the
/// sentence's words (a model-added synonym is dropped, never searched). Invented conditions (the bake-off's main failure) are dropped.
pub fn parse_conditions(sentence: &str, json: &str) -> Result<Vec<Condition>, String> {
    #[derive(serde::Deserialize)]
    struct Raw {
        conditions: Vec<RawCondition>,
    }
    #[derive(serde::Deserialize)]
    struct RawCondition {
        text: String,
        terms: Vec<String>,
        negated: bool,
    }
    let raw: Raw = serde_json::from_str(json)
        .map_err(|error| format!("model returned invalid JSON: {error}"))?;
    let sentence_words = content_words(sentence);
    let mut out: Vec<Condition> = Vec::new();
    for condition in raw.conditions.into_iter().take(MAX_CONDITIONS) {
        let mut terms: Vec<String> = Vec::new();
        for term in condition
            .terms
            .iter()
            .chain(std::iter::once(&condition.text))
            .flat_map(|t| content_words(t))
        {
            if !terms.contains(&term) && sentence_words.iter().any(|s| related(&term, s)) {
                terms.push(term);
            }
        }
        // Grounded when at least half of the condition's own content words
        // appear (as the same word or a word form) in the user's sentence.
        let own = content_words(&condition.text);
        let hits = own
            .iter()
            .filter(|w| sentence_words.iter().any(|s| related(w, s)))
            .count();
        let grounded = !own.is_empty() && hits * 2 >= own.len();
        let text = condition.text.trim().to_string();
        if !grounded || text.is_empty() || out.iter().any(|c| c.text.eq_ignore_ascii_case(&text)) {
            continue;
        }
        out.push(Condition {
            text,
            terms,
            negated: condition.negated,
            grounding: hits as f32 / own.len() as f32,
        });
    }
    if out.is_empty() {
        return Err("no condition in the sentence could be identified".into());
    }
    Ok(out)
}

fn normalise(text: &str) -> String {
    words(text).join(" ")
}

/// Parse the judge JSON. A "yes" counts only when its quote is in the passage.
pub fn parse_verdict(passage: &str, json: &str) -> Result<Verdict, String> {
    #[derive(serde::Deserialize)]
    struct Raw {
        quote: String,
        verdict: String,
    }
    let raw: Raw = serde_json::from_str(json)
        .map_err(|error| format!("model returned invalid JSON: {error}"))?;
    let quote_found =
        !raw.quote.trim().is_empty() && normalise(passage).contains(&normalise(&raw.quote));
    Ok(Verdict {
        matches: raw.verdict == "yes" && quote_found,
        quote: if quote_found {
            raw.quote
        } else {
            String::new()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invented_conditions_are_dropped_and_variants_kept() {
        let json = r#"{"conditions":[
            {"text":"corrosion blended","terms":["corrosion","blend","blended"],"negated":false},
            {"text":"surface filled and faired","terms":["surface","fill","fair"],"negated":false},
            {"text":"paint is fresh","terms":["paint","fresh"],"negated":false}]}"#;
        let found = parse_conditions(
            "files where corrosion is blended, and the surface filled and faired",
            json,
        )
        .unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text, "corrosion blended");
        assert!(found[0].terms.contains(&"blend".to_string()));
    }

    #[test]
    fn model_added_synonyms_are_not_searched() {
        let json = r#"{"conditions":[{"text":"corrosion blended","terms":["corrosion","blend","grind","remove"],"negated":false}]}"#;
        let found = parse_conditions("corrosion blended", json).unwrap();
        assert_eq!(found[0].terms, ["corrosion", "blend", "blended"]);
    }

    #[test]
    fn partly_grounded_conditions_are_flagged_for_review() {
        let json = r#"{"conditions":[
            {"text":"corrosion blended","terms":["corrosion"],"negated":false},
            {"text":"surface sanded","terms":["surface"],"negated":false},
            {"text":"surface sanded smooth","terms":["surface"],"negated":false}]}"#;
        let found = parse_conditions("corrosion blended and surface filled", json).unwrap();
        assert!(!found[0].needs_review());
        // Half of "surface sanded" is the user's: kept, but marked for review.
        assert!(found[1].needs_review(), "{found:?}");
        // A third of "surface sanded smooth" is the user's: an invented condition, dropped.
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn filler_words_never_ground_a_condition() {
        let json = r#"{"conditions":[{"text":"is the","terms":["is"],"negated":false}]}"#;
        assert!(parse_conditions("where it is the case", json).is_err());
    }

    #[test]
    fn negation_and_duplicates() {
        let json = r#"{"conditions":[
            {"text":"corrosion allowance","terms":["corrosion","allowance"],"negated":true},
            {"text":"Corrosion allowance","terms":["corrosion"],"negated":true}]}"#;
        let found = parse_conditions("hydrostatic test but not corrosion allowance", json).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].negated);
    }

    #[test]
    fn nothing_grounded_is_an_error_not_an_empty_search() {
        let json = r#"{"conditions":[{"text":"unrelated","terms":["zebra"],"negated":false}]}"#;
        assert!(parse_conditions("corrosion blended", json).is_err());
        assert!(parse_conditions("corrosion", "not json").is_err());
    }

    #[test]
    fn yes_needs_a_quote_from_the_passage() {
        let passage = "Light surface corrosion was removed by blending with abrasive pad.";
        let ok = parse_verdict(
            passage,
            r#"{"quote":"corrosion was removed by blending","verdict":"yes"}"#,
        )
        .unwrap();
        assert!(ok.matches);
        let invented = parse_verdict(
            passage,
            r#"{"quote":"corrosion was blended flush","verdict":"yes"}"#,
        )
        .unwrap();
        assert!(!invented.matches);
        let no = parse_verdict(passage, r#"{"quote":"","verdict":"no"}"#).unwrap();
        assert!(!no.matches);
    }
}
