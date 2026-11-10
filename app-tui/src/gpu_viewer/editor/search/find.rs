//! One Find box for everything: what was typed decides how it is searched. A question (ends with `?`, or opens with a
//! question word) is answered with cited lines; a sentence is read as conditions; anything else is an exact search of
//! the words and phrases. The user can pin the reading instead of leaving it on Auto.

/// How the Find text is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FindAs {
    Auto,
    Words,
    Sentence,
    Question,
}

impl FindAs {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Auto => Self::Words,
            Self::Words => Self::Sentence,
            Self::Sentence => Self::Question,
            Self::Question => Self::Auto,
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Auto => "Find as: auto",
            Self::Words => "Find as: words",
            Self::Sentence => "Find as: sentence",
            Self::Question => "Find as: question",
        }
    }
}

/// What a Find text is, once Auto has looked at it (never `Auto`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Intent {
    Words,
    #[cfg_attr(not(feature = "nl-find"), allow(dead_code))]
    Sentence,
    Question,
}

const QUESTION_WORDS: &[&str] = &[
    "how", "what", "where", "which", "who", "when", "why", "does", "do", "is", "are", "can", "should",
];

/// Whether `text` is phrased as a question: it ends with `?`, or has at least three words and opens with a question
/// word, and in neither case uses the filter syntax (`,` `;` `[` `]` `(` `)` `"`), which marks a keyword search.
pub(super) fn is_question(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text.contains([',', ';', '[', ']', '(', ')', '"']) {
        return false;
    }
    if text.ends_with('?') {
        return true;
    }
    let mut words = text.split_whitespace();
    let first = words.next().map(str::to_lowercase);
    words.count() >= 2 && first.is_some_and(|w| QUESTION_WORDS.contains(&w.as_str()))
}

pub(super) fn classify(text: &str, pinned: FindAs) -> Intent {
    match pinned {
        FindAs::Words => Intent::Words,
        FindAs::Sentence => Intent::Sentence,
        FindAs::Question => Intent::Question,
        FindAs::Auto => {
            if is_question(text) {
                Intent::Question
            } else if cfg!(feature = "nl-find") && super::nl::looks_like_sentence(text) {
                Intent::Sentence
            } else {
                Intent::Words
            }
        }
    }
}

/// The question as the Ask engine wants it: every part ends with `?`.
pub(super) fn as_question(text: &str) -> String {
    let text = text.trim();
    if text.ends_with('?') {
        text.to_string()
    } else {
        format!("{text}?")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_are_told_from_keyword_searches() {
        assert!(is_question("What is the pressure test duration?"));
        assert!(is_question("how many pdf files are there"));
        assert!(is_question("where is the torque table?"));
        assert!(!is_question("torque, bolt"));
        assert!(!is_question("how"));
        assert!(!is_question("what is, torque"));
        assert!(!is_question("\"is this exact?\""));
        assert!(!is_question(""));
    }

    #[test]
    fn auto_reads_a_question_a_sentence_or_words_and_a_pin_overrides() {
        assert_eq!(classify("how do I exclude a folder?", FindAs::Auto), Intent::Question);
        assert_eq!(classify("torque, bolt", FindAs::Auto), Intent::Words);
        assert_eq!(classify("torque, bolt", FindAs::Question), Intent::Question);
        assert_eq!(classify("how do I exclude a folder?", FindAs::Words), Intent::Words);
        #[cfg(feature = "nl-find")]
        assert_eq!(classify("surfaces that were blended and filled and fair", FindAs::Auto), Intent::Sentence);
    }

    #[test]
    fn a_pinned_reading_cycles_and_a_question_always_ends_with_a_mark() {
        let mut m = FindAs::Auto;
        for _ in 0..4 {
            m = m.next();
        }
        assert_eq!(m, FindAs::Auto);
        assert_eq!(as_question("how many files"), "how many files?");
        assert_eq!(as_question("how many files?"), "how many files?");
    }
}
