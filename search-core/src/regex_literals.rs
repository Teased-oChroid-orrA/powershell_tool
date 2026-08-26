//! Conservative literal-substring extraction from a regex filter pattern
//! (issue #6 §24 "Regex Candidate Filtering" - "do not regex-scan the
//! entire filesystem unless explicitly required").
//!
//! `required_literal_chunks` finds substrings that are *guaranteed* to
//! appear, contiguously, in any string the pattern matches - a necessary
//! (not sufficient) condition, exactly like the plain-filter trigram
//! narrowing in native-search's `trigram_candidate_paths`. The two combine
//! at the call site (`app/src/state.rs`): turn each chunk into its
//! trigrams, require all of them, same safe-superset reasoning.
//!
//! This is deliberately NOT a general regex analyzer. It handles a
//! restricted, easily-verified-safe subset of syntax and returns `None`
//! (meaning "don't narrow, fall back to a full scan") the moment it sees
//! anything it isn't sure about - groups, character classes, alternation,
//! and bounded quantifiers (`(`, `)`, `[`, `]`, `{`, `}`, `|`) all bail
//! immediately rather than being partially/incorrectly analyzed. `None`
//! is always the safe answer; getting a chunk *wrong* (claiming a
//! substring is required when it isn't) would silently drop real matches,
//! which this whole feature exists to never do.
//!
//! ## Why quantifiers need a flush, not just a drop
//!
//! A naive version might just drop the single atom immediately before
//! `*`/`+`/`?` and keep building the literal run around it. That's wrong:
//! `ab+c` matches "abc" AND "abbc" - the latter does NOT contain "abc" as
//! a contiguous substring (it contains "abb" then "bc"). So a quantified
//! atom must not just be excluded from the run - the run must be *split*
//! there, so characters before and after the quantifier are never treated
//! as one contiguously-required chunk. See the adversarial tests below
//! (`colou?r`, `ab+c`) for the exact failure mode this guards against.

/// Regex mode in this app always compiles case-insensitively
/// (`matching::compile_case_insensitive`) - chunks are returned in their
/// original case and rely on the caller's trigram tokenizer applying the
/// same `LowerCaser` filter every other trigram lookup already goes
/// through (`native-search`'s `trigrams_of`), not on anything done here.
pub fn required_literal_chunks(pattern: &str) -> Option<Vec<String>> {
    const LITERAL_ESCAPES: &[char] = &['.', '^', '$', '*', '+', '?', '(', ')', '[', ']', '{', '}', '|', '\\', '/', '-'];

    let mut chunks: Vec<String> = Vec::new();
    let mut run: Vec<char> = Vec::new();
    let mut chars = pattern.chars();

    fn flush(run: &mut Vec<char>, chunks: &mut Vec<String>) {
        if !run.is_empty() {
            chunks.push(run.iter().collect());
            run.clear();
        }
    }

    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                None => return None, // trailing backslash - malformed pattern, bail
                Some(esc) if LITERAL_ESCAPES.contains(&esc) => run.push(esc),
                // \d \D \w \W \s \S \b \B \A \z \Z and anything else -
                // contributes no guaranteed literal text, but doesn't
                // itself break the safety of what's already in `run`.
                Some(_) => flush(&mut run, &mut chunks),
            },
            '.' | '^' | '$' => flush(&mut run, &mut chunks),
            '*' | '+' | '?' => {
                // The quantified atom (if any) is not guaranteed to
                // appear contiguously adjacent to what follows - drop it
                // AND split the run here (see module doc for why a split,
                // not just a drop, is required for correctness).
                run.pop();
                flush(&mut run, &mut chunks);
            }
            '(' | ')' | '[' | ']' | '{' | '}' | '|' => return None,
            other => run.push(other),
        }
    }
    flush(&mut run, &mut chunks);

    let usable: Vec<String> = chunks.into_iter().filter(|c| c.chars().count() >= 3).collect();
    if usable.is_empty() {
        None
    } else {
        Some(usable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_literal_pattern_is_kept_whole() {
        assert_eq!(required_literal_chunks("turbofan"), Some(vec!["turbofan".to_string()]));
    }

    #[test]
    fn trailing_digit_class_and_quantifier_keep_the_literal_prefix() {
        // epic #6 §24's own example.
        assert_eq!(required_literal_chunks(r"engine\d+"), Some(vec!["engine".to_string()]));
    }

    #[test]
    fn dot_star_splits_into_two_required_chunks() {
        assert_eq!(required_literal_chunks("foo.*bar"), Some(vec!["foo".to_string(), "bar".to_string()]));
    }

    #[test]
    fn escaped_metacharacters_are_treated_as_literal_text() {
        assert_eq!(required_literal_chunks(r"C\+\+ engine"), Some(vec!["C++ engine".to_string()]));
    }

    #[test]
    fn anchors_are_safe_terminators_not_bail_triggers() {
        assert_eq!(required_literal_chunks("^turbofan$"), Some(vec!["turbofan".to_string()]));
    }

    #[test]
    fn groups_classes_alternation_and_bounded_quantifiers_all_bail() {
        for pattern in ["(engine|motor)", "eng[ei]ne", "eng{1,2}ine", "(?:engine)", "a(bc)d"] {
            assert_eq!(required_literal_chunks(pattern), None, "pattern {pattern:?} must bail, not guess");
        }
    }

    #[test]
    fn trailing_backslash_bails_instead_of_panicking() {
        assert_eq!(required_literal_chunks(r"engine\"), None);
    }

    #[test]
    fn short_or_absent_literal_content_yields_no_narrowing() {
        assert_eq!(required_literal_chunks(r"\d+"), None);
        assert_eq!(required_literal_chunks(r"ab+c"), None); // see module doc: "abc" would be unsafe here
        assert_eq!(required_literal_chunks("ab"), None); // only 2 chars, no trigrams
    }

    /// The adversarial case the module doc's "why a flush" section is
    /// about: naively keeping "colo" + "r" concatenated as "color" would
    /// be WRONG, since "colour" (a real match for `colou?r`) does not
    /// contain "color" as a contiguous substring. The flush-on-quantifier
    /// behavior must produce "colo" as its own chunk, and "r" must never
    /// be glued onto it.
    #[test]
    fn optional_char_mid_pattern_never_produces_a_falsely_contiguous_chunk() {
        let chunks = required_literal_chunks("colou?r").unwrap();
        assert_eq!(chunks, vec!["colo".to_string()]);
        // Sanity: prove the property the whole module exists to guarantee -
        // every real match of the pattern actually contains every chunk.
        let re = fancy_regex::Regex::new("colou?r").unwrap();
        for candidate in ["color", "colour"] {
            assert!(re.is_match(candidate).unwrap(), "test setup: {candidate} should match colou?r");
            for chunk in &chunks {
                assert!(candidate.contains(chunk.as_str()), "{candidate:?} must contain required chunk {chunk:?}");
            }
        }
    }

    /// Same property, proven for the `ab+c` / "abbc" case referenced in
    /// the module doc - since that pattern yields no usable chunks at
    /// all (both runs are too short), there's nothing to falsely assert,
    /// but this locks in that the algorithm doesn't try to be clever
    /// about it either.
    #[test]
    fn repeated_char_mid_pattern_does_not_produce_a_falsely_contiguous_chunk() {
        assert_eq!(required_literal_chunks("ab+c"), None);
        let re = fancy_regex::Regex::new("ab+c").unwrap();
        assert!(re.is_match("abbc").unwrap(), "test setup: abbc should match ab+c");
        assert!(!"abbc".contains("abc"), "test setup sanity: abbc must not contain abc");
    }

    #[test]
    fn empty_pattern_yields_no_narrowing() {
        assert_eq!(required_literal_chunks(""), None);
    }
}
