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
//! anything it isn't sure about - groups, character classes, and
//! alternation (`(`, `)`, `[`, `]`, `|`) all bail immediately rather than
//! being partially/incorrectly analyzed. `None` is always the safe
//! answer; getting a chunk *wrong* (claiming a substring is required when
//! it isn't) would silently drop real matches, which this whole feature
//! exists to never do.
//!
//! Bounded quantifiers (`{n}`, `{n,}`, `{n,m}`) are a narrow, deliberate
//! exception (issue #9 epic §6's own example, `foo.{0,5}bar`): a `{...}`
//! body is only ever treated as a quantifier - and the atom immediately
//! before it dropped/flushed, exactly like `*`/`+`/`?` - when its content
//! is strictly digits, an optional comma, then more digits (Rust regex's
//! actual repetition grammar). Anything else after `{` (non-digit
//! content, no closing `}`) still bails the whole pattern, same as
//! before - this isn't a general brace parser, just enough to recognize
//! the one construct that's unambiguous.
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

    let chars: Vec<char> = pattern.chars().collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut run: Vec<char> = Vec::new();
    let mut i: usize = 0;

    fn flush(run: &mut Vec<char>, chunks: &mut Vec<String>) {
        if !run.is_empty() {
            chunks.push(run.iter().collect());
            run.clear();
        }
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                i += 1;
                match chars.get(i) {
                    None => return None, // trailing backslash - malformed pattern, bail
                    Some(esc) if LITERAL_ESCAPES.contains(esc) => run.push(*esc),
                    // Classes, anchors and control-character escapes
                    // contribute no guaranteed literal text, but don't
                    // themselves break the safety of what's already in `run`.
                    Some('d' | 'D' | 'w' | 'W' | 's' | 'S' | 'b' | 'B' | 'A' | 'z' | 'Z' | 'n' | 'r' | 't' | 'f' | 'v' | 'a') => {
                        flush(&mut run, &mut chunks)
                    }
                    // Any other escape takes operand characters of its own
                    // (`\x41` is "A", `\pL` a letter class, `\1` a backreference,
                    // `\u{..}`, `\k<name>`): the characters after it are not
                    // literal text. Bail rather than guess.
                    Some(_) => return None,
                }
                i += 1;
            }
            '.' | '^' | '$' => {
                flush(&mut run, &mut chunks);
                i += 1;
            }
            '*' | '+' | '?' => {
                // The quantified atom (if any) is not guaranteed to
                // appear contiguously adjacent to what follows - drop it
                // AND split the run here (see module doc for why a split,
                // not just a drop, is required for correctness).
                run.pop();
                flush(&mut run, &mut chunks);
                i += 1;
            }
            '{' => match bounded_quantifier_end(&chars, i) {
                // A recognized `{n}`/`{n,}`/`{n,m}` body - same
                // pop-then-flush treatment as `*`/`+`/`?`, since the atom
                // immediately before `{` is the one being repeated.
                Some(end) => {
                    run.pop();
                    flush(&mut run, &mut chunks);
                    i = end + 1; // past the closing '}'
                }
                // Not a recognized quantifier body (non-digit content, or
                // no closing '}') - unsure, bail the whole pattern rather
                // than guess.
                None => return None,
            },
            // A character class matches exactly one char: it breaks the
            // literal run like `.` does. A nested `[` (set algebra, POSIX
            // names) or an unterminated class bails.
            '[' => match class_end(&chars, i) {
                Some(end) => {
                    flush(&mut run, &mut chunks);
                    i = end + 1;
                }
                None => return None,
            },
            '(' | ')' | ']' | '}' | '|' => return None,
            other => {
                run.push(other);
                i += 1;
            }
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

/// Index of the `]` closing the plain character class opened at `chars[open]`
/// (`[abc]`, `[^a-z]`, `[]a]`), or `None` for anything with a nested `[` or
/// no closing bracket.
fn class_end(chars: &[char], open: usize) -> Option<usize> {
    let mut j = open + 1;
    if chars.get(j) == Some(&'^') {
        j += 1;
    }
    if chars.get(j) == Some(&']') {
        j += 1; // a leading `]` is a literal member
    }
    while j < chars.len() {
        match chars[j] {
            '\\' => j += 2,
            '[' => return None,
            ']' => return Some(j),
            _ => j += 1,
        }
    }
    None
}

/// OR of [`required_literal_chunks`] over the top-level alternatives of
/// `pattern` (`foo.*bar|baz`): every match contains all chunks of at least
/// one returned set. `None` when any alternative has no usable chunk, or the
/// pattern has groups (parentheses are not analysed). A pattern without `|`
/// returns one set - the same chunks `required_literal_chunks` gives.
pub fn required_literal_alternatives(pattern: &str) -> Option<Vec<Vec<String>>> {
    if pattern.contains('(') || pattern.contains(')') {
        return required_literal_chunks(pattern).map(|c| vec![c]);
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut branches: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                current.push('\\');
                if let Some(c) = chars.get(i + 1) {
                    current.push(*c);
                }
                i += 2;
                continue;
            }
            '[' => match class_end(&chars, i) {
                Some(end) => {
                    current.extend(&chars[i..=end]);
                    i = end + 1;
                    continue;
                }
                None => return None,
            },
            '|' => branches.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
        i += 1;
    }
    branches.push(current);
    branches.iter().map(|b| required_literal_chunks(b)).collect()
}

/// If `chars[open_brace_idx..]` starts with a syntactically valid bounded-
/// repetition body - `{` then one-or-more digits, then optionally a comma
/// and zero-or-more more digits, then `}` (Rust regex's actual `{n}`/
/// `{n,}`/`{n,m}` grammar) - returns the index of the closing `}`.
/// Anything else (non-digit content, an empty `{}`, no closing brace at
/// all) returns `None`, the signal to bail the whole pattern rather than
/// misinterpret a `{` that wasn't actually a quantifier.
fn bounded_quantifier_end(chars: &[char], open_brace_idx: usize) -> Option<usize> {
    let mut j = open_brace_idx + 1;
    let digits_start = j;
    while j < chars.len() && chars[j].is_ascii_digit() {
        j += 1;
    }
    if j == digits_start {
        return None; // `{` must be followed by at least one digit
    }
    if j < chars.len() && chars[j] == ',' {
        j += 1;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
    }
    if j < chars.len() && chars[j] == '}' {
        Some(j)
    } else {
        None
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
    fn groups_and_alternation_bail() {
        for pattern in ["(engine|motor)", "(?:engine)", "a(bc)d", "foo|bar"] {
            assert_eq!(required_literal_chunks(pattern), None, "pattern {pattern:?} must bail, not guess");
        }
    }

    #[test]
    fn a_character_class_breaks_the_literal_run_like_a_dot() {
        assert_eq!(required_literal_chunks("eng[ei]ne"), Some(vec!["eng".to_string()]));
        assert_eq!(required_literal_chunks("torque[ _-]+value"), Some(vec!["torque".to_string(), "value".to_string()]));
        assert_eq!(required_literal_chunks(r"abc[\]x]def"), Some(vec!["abc".to_string(), "def".to_string()]));
        assert_eq!(required_literal_chunks("abc[]x]def"), Some(vec!["abc".to_string(), "def".to_string()]));
        assert_eq!(required_literal_chunks("abc[a-z&&[^aeiou]]def"), None, "nested class bails");
        assert_eq!(required_literal_chunks("abc[unclosed"), None);
    }

    #[test]
    fn escapes_with_operands_bail_instead_of_leaking_operand_characters() {
        // `\x41` is "A": the old code kept "41" as required literal text and would have
        // dropped every real match of `\x41BC` (text "ABC"). Same for `\pL`, `\1`.
        for pattern in [r"\x41BCD", r"\pLfoo", r"foo\1bar", r"\u{41}BCD", r"\k<n>abc"] {
            assert_eq!(required_literal_chunks(pattern), None, "{pattern}");
        }
        assert_eq!(required_literal_chunks(r"abc\tdef\n"), Some(vec!["abc".to_string(), "def".to_string()]));
    }

    #[test]
    fn chunks_are_present_in_every_real_match_of_class_and_alternation_patterns() {
        let patterns = [
            "eng[ei]ne", "torque[ _-]+value", "abc[]x]def", r"abc[\]x]def", "torque.*value|bolt load", "[a|b]cde|fgh",
            r"\x41BCD", r"foo\d+bar", r"load\s*path|shear\b", "colou?r[s]? chart", "x{2}yzw[0-9]{3}",
        ];
        let texts = [
            "engine", "engene", "torque value", "torque - value", "TORQUE_VALUE", "abcxdef", "abc]def", "bolt load",
            "ABCD", "ABCDE", "foo12bar", "load  path", "shear force", "colour charts", "color chart", "xxyzw123", "acde", "bcde", "|cde", "fgh",
        ];
        for p in patterns {
            let rx = compile_ci(p);
            let alts = required_literal_alternatives(p);
            for t in texts {
                if rx.is_match(t).unwrap_or(false) {
                    if let Some(alts) = &alts {
                        let low = t.to_lowercase();
                        assert!(
                            alts.iter().any(|set| set.iter().all(|c| low.contains(&c.to_lowercase()))),
                            "{p:?} matches {t:?} but no chunk set is contained in it: {alts:?}"
                        );
                    }
                }
            }
        }
    }

    fn compile_ci(p: &str) -> fancy_regex::Regex {
        fancy_regex::RegexBuilder::new(p).case_insensitive(true).build().unwrap()
    }

    #[test]
    fn top_level_alternatives_become_or_of_chunk_sets() {
        assert_eq!(
            required_literal_alternatives("torque.*value|bolt load"),
            Some(vec![vec!["torque".to_string(), "value".to_string()], vec!["bolt load".to_string()]])
        );
        assert_eq!(required_literal_alternatives("foo|ab"), None, "one unusable branch means no narrowing");
        assert_eq!(required_literal_alternatives("turbofan"), Some(vec![vec!["turbofan".to_string()]]));
        assert_eq!(required_literal_alternatives("[a|b]cde|fgh"), Some(vec![vec!["cde".to_string()], vec!["fgh".to_string()]]));
        assert_eq!(required_literal_alternatives("(foo|bar)baz"), None, "groups are not analysed");
        assert_eq!(required_literal_alternatives(r"a\|bcd"), Some(vec![vec!["a|bcd".to_string()]]));
    }

    /// Issue #9 epic §6's own example: `foo.{0,5}bar` must narrow to
    /// `["foo", "bar"]`, not bail to a full scan - a recognized bounded
    /// quantifier is treated exactly like `*`/`+`/`?` (split, don't glue).
    #[test]
    fn bounded_quantifier_splits_into_required_chunks_like_other_quantifiers() {
        assert_eq!(required_literal_chunks("foo.{0,5}bar"), Some(vec!["foo".to_string(), "bar".to_string()]));
        assert_eq!(required_literal_chunks("foo.{3}bar"), Some(vec!["foo".to_string(), "bar".to_string()]));
        assert_eq!(required_literal_chunks("foo.{2,}bar"), Some(vec!["foo".to_string(), "bar".to_string()]));
    }

    /// The atom immediately before `{n,m}` is the one being repeated -
    /// must be popped off the run before flushing, same as `x+`/`x*`.
    /// "eng{1,2}ine" repeats only the "g", so "en" (what's left of "eng"
    /// after popping it) is too short to be a usable 3+ char chunk and is
    /// correctly dropped, leaving only "ine".
    #[test]
    fn bounded_quantifier_pops_the_immediately_preceding_atom() {
        assert_eq!(required_literal_chunks("eng{1,2}ine"), Some(vec!["ine".to_string()]));
    }

    /// Same safety property this whole module exists to guarantee, proven
    /// for the new bounded-quantifier path specifically: every real match
    /// of `x{n,m}`-containing patterns actually contains every returned
    /// chunk, across both ends of the repetition range.
    #[test]
    fn bounded_quantifier_chunks_are_present_in_every_real_match() {
        let re = fancy_regex::Regex::new("eng{1,3}ine").unwrap();
        let chunks = required_literal_chunks("eng{1,3}ine").unwrap();
        for candidate in ["enggine", "engggine", "engggggine"] {
            let is_match = re.is_match(candidate).unwrap();
            for chunk in &chunks {
                let contains = candidate.contains(chunk.as_str());
                assert!(
                    !is_match || contains,
                    "{candidate:?} matched eng{{1,3}}ine but is missing required chunk {chunk:?}"
                );
            }
        }
    }

    /// `{` bodies this module doesn't recognize (non-digit content, an
    /// unterminated brace, or an empty `{}`) must still bail the whole
    /// pattern - the new quantifier handling must not become a general
    /// brace parser.
    #[test]
    fn unrecognized_or_unterminated_brace_bodies_still_bail() {
        for pattern in ["eng{abc}ine", "eng{ine", "eng{}ine", "eng{1,2"] {
            assert_eq!(required_literal_chunks(pattern), None, "pattern {pattern:?} must bail, not guess");
        }
    }

    /// An escaped `\{` is a literal brace character, not the start of a
    /// quantifier - must never be routed through the new lookahead logic.
    #[test]
    fn escaped_brace_is_literal_not_a_quantifier() {
        assert_eq!(required_literal_chunks(r"eng\{1,2\}ine"), Some(vec!["eng{1,2}ine".to_string()]));
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
