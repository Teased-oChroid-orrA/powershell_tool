//! Ports `TextInFilesSearch.Core/Services/MatchingEngine.cs`. See that
//! file's doc comments for the "why" behind `passes_mode` being an
//! explicit result field (not inferred from an empty hits list),
//! per-filter-*slot* indexing (not per filter text, so case-variant
//! duplicate filters don't collapse into one proximity-range entry), and
//! the whole-word lookaround boundary - ported verbatim here, not
//! re-derived.
//!
//! Uses `fancy-regex` (not the `regex` crate) for every regex path in this
//! file, including plain regex-mode filters: whole-word matching needs
//! lookaround, which `regex` deliberately doesn't support (no
//! backtracking, by design), and `fancy-regex` was chosen over a
//! hand-rolled boundary scan after verifying it against the C# whole-word
//! test cases (Program.cs Test 6/23) - see docs/rust-rewrite-status.md.
//!
//! ## Catastrophic-backtracking safety (issue #9 epic §40)
//!
//! Regex mode compiles the user's own filter text directly (unlike
//! whole-word mode, which only ever wraps an *escaped* literal in a fixed
//! lookaround template - no user-controlled quantifiers there, so it
//! can't backtrack pathologically regardless of filter content). A
//! hand-written classic ReDoS pattern (`(a+)+$`-style nested/ambiguous
//! quantifiers) run against adversarial input in regex mode could in
//! principle cost exponential backtracking work.
//!
//! Verified (not assumed) that this is already bounded: every `FancyRegex`
//! in this file is built via plain `Regex::new`/`RegexBuilder::new`
//! (`compile_case_insensitive`, `build_combined`), and fancy-regex 0.19's
//! own `RegexOptions::default()`/`RegexOptionsBuilder::new()` both set
//! `backtrack_limit: 1_000_000` (confirmed by reading
//! `fancy-regex-0.19.0/src/lib.rs`'s `HardRegexRuntimeOptions::default()`
//! directly, not assumed from documentation) - nothing in this file
//! overrides it. `is_match` returns `Err` once that many backtrack steps
//! are spent, rather than continuing unboundedly - see
//! `regex_backtrack_limit_bounds_a_classic_redos_pattern_instead_of_hanging`
//! below for a real proof against an adversarial pattern+input pair, not
//! just a reading of the dependency's source.
//!
//! What that error becomes matters for correctness, and is a real,
//! previously-undocumented asymmetry: the cheap combined pre-check
//! (`build_combined`/`apply_line_matching`'s `candidate_line`/
//! `exclude_candidate`) fails OPEN (`unwrap_or(true)` - "treat as a
//! candidate, fall through to the real per-filter check"), the safe
//! direction. The authoritative per-filter check in `is_hit` fails CLOSED
//! (`unwrap_or(false)` - "not a hit"), meaning a filter that hits the
//! backtrack limit against a specific line is silently treated as *not
//! matching* that line, rather than erroring the whole search or being
//! reported as a low-confidence result. This requires a genuinely
//! pathological pattern (1,000,000 backtrack steps is not something an
//! ordinary regex filter hits by accident) - not a new risk introduced by
//! anything in this file, but worth this explicit record since nothing in
//! CLAUDE.md's regex-engine rationale previously mentioned this bound or
//! its failure direction.

use std::collections::HashMap;

use fancy_regex::{Regex as FancyRegex, RegexBuilder as FancyRegexBuilder};

use crate::models::{ExcludeScope, LineHit, MatchMode, SearchSettings};

/// One regex filter that failed to compile, paired with why - mirrors the
/// C# side's `(string Filter, string Error)` tuple list.
#[derive(Debug, Clone)]
pub struct InvalidFilter {
    pub filter: String,
    pub error: String,
}

/// One or more regex-mode filters failed to compile. Named so callers can
/// report exactly which filter(s) were bad, not a bare parse error - same
/// motivation as the C# `InvalidFilterRegexException`.
#[derive(Debug, Clone)]
pub struct InvalidFilterRegexError {
    pub invalid_filters: Vec<InvalidFilter>,
}

impl std::fmt::Display for InvalidFilterRegexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self
            .invalid_filters
            .iter()
            .map(|i| format!("\"{}\" ({})", i.filter, i.error))
            .collect();
        write!(f, "Invalid regex filter(s): {}", parts.join("; "))
    }
}

impl std::error::Error for InvalidFilterRegexError {}

/// Builds the "whole word" match pattern used everywhere whole-word mode is
/// needed (`MatchingEngine` here, and the report highlighter later - one
/// implementation instead of two that could drift). Uses lookaround
/// against letter/digit/underscore rather than `\b`: `\b` only asserts a
/// transition between a word and non-word character, so a filter whose own
/// first or last character is itself non-word (e.g. "C#") can fail to
/// match even standing alone between spaces, because neither side of that
/// boundary is a word-to-non-word transition. Asserting "not adjacent to a
/// letter/digit/underscore" on both sides instead gives the intuitive
/// "isolated token" behavior regardless of what character the filter
/// starts or ends with.
pub fn whole_word_pattern(filter: &str) -> String {
    format!(
        r"(?<![\p{{L}}\p{{N}}_]){}(?![\p{{L}}\p{{N}}_])",
        fancy_regex::escape(filter)
    )
}

fn compile_case_insensitive(pattern: &str) -> Result<FancyRegex, fancy_regex::Error> {
    FancyRegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
}

/// Precompiled regex state built once per search run (not per file, not
/// per line) and reused across every file - the same optimization as the
/// PowerShell version's compiled-regex cache and combined pre-check
/// pattern.
pub struct CompiledMatchState {
    compiled_filter_regex: HashMap<String, FancyRegex>,
    compiled_exclude_regex: HashMap<String, FancyRegex>,
    whole_word_filter_regex: HashMap<String, FancyRegex>,
    whole_word_exclude_regex: HashMap<String, FancyRegex>,

    /// One alternation of every filter, for a cheap single check per line
    /// before the slower per-filter loop. Only populated for regex/
    /// whole-word mode - literal mode uses `filters_lower`/
    /// `exclude_filters_lower` instead, see the doc comment on those.
    combined_filter_regex: Option<FancyRegex>,
    combined_exclude_regex: Option<FancyRegex>,

    /// Literal-mode filters/excludes, lowercased once here rather than on
    /// every line of every file. Positional (aligned 1:1 with
    /// `settings.filters`/`exclude_filters`), not a HashMap, for the same
    /// reason `per_filter_hit_lines` in `apply_line_matching` is
    /// slot-indexed - two filters differing only by case are distinct
    /// slots, and a map would collapse them.
    ///
    /// Literal mode deliberately does NOT go through `fancy_regex` at all
    /// (unlike whole-word/regex mode, which both need it - see this
    /// module's top doc comment). A plain case-insensitive substring check
    /// has no regex semantics to diverge on, so there's no "two engines
    /// might disagree" risk to guard against by routing it through a regex
    /// engine anyway - and fancy-regex's backtracking-VM `is_match` on an
    /// escaped-literal-alternation pattern is real, measurable per-line
    /// overhead compared to `str::contains` when it's invoked on every
    /// line of every file in a large search. This was found while
    /// investigating a reported "large-folder search is slower than the
    /// old PowerShell tool" regression.
    filters_lower: Vec<String>,
    exclude_filters_lower: Vec<String>,
}

impl CompiledMatchState {
    pub fn build(settings: &SearchSettings) -> Result<Self, InvalidFilterRegexError> {
        let mut state = CompiledMatchState {
            compiled_filter_regex: HashMap::new(),
            compiled_exclude_regex: HashMap::new(),
            whole_word_filter_regex: HashMap::new(),
            whole_word_exclude_regex: HashMap::new(),
            combined_filter_regex: None,
            combined_exclude_regex: None,
            filters_lower: Vec::new(),
            exclude_filters_lower: Vec::new(),
        };

        if settings.use_regex {
            let mut invalid = Vec::new();

            for f in &settings.filters {
                match compile_case_insensitive(f) {
                    Ok(rx) => {
                        state.compiled_filter_regex.insert(f.clone(), rx);
                    }
                    Err(e) => invalid.push(InvalidFilter {
                        filter: f.clone(),
                        error: e.to_string(),
                    }),
                }
            }
            for f in &settings.exclude_filters {
                match compile_case_insensitive(f) {
                    Ok(rx) => {
                        state.compiled_exclude_regex.insert(f.clone(), rx);
                    }
                    Err(e) => invalid.push(InvalidFilter {
                        filter: f.clone(),
                        error: e.to_string(),
                    }),
                }
            }

            if !invalid.is_empty() {
                return Err(InvalidFilterRegexError {
                    invalid_filters: invalid,
                });
            }
        } else if settings.whole_word {
            // Built from an escaped filter dropped into a fixed lookaround
            // skeleton - structurally always a valid pattern, so unlike
            // regex mode there's no user-facing invalid-filter case here
            // (matches the C# side, which doesn't try/catch this either).
            for f in &settings.filters {
                let rx = compile_case_insensitive(&whole_word_pattern(f))
                    .expect("whole-word pattern is always valid regex");
                state.whole_word_filter_regex.insert(f.clone(), rx);
            }
            for f in &settings.exclude_filters {
                let rx = compile_case_insensitive(&whole_word_pattern(f))
                    .expect("whole-word pattern is always valid regex");
                state.whole_word_exclude_regex.insert(f.clone(), rx);
            }
        } else {
            // Literal mode - no fancy_regex involved at all, see the doc
            // comment on `filters_lower`/`exclude_filters_lower`.
            state.filters_lower = settings.filters.iter().map(|f| f.to_lowercase()).collect();
            state.exclude_filters_lower =
                settings.exclude_filters.iter().map(|f| f.to_lowercase()).collect();
        }

        if settings.use_regex || settings.whole_word {
            state.combined_filter_regex =
                build_combined(&settings.filters, settings.use_regex, settings.whole_word);
            state.combined_exclude_regex = if !settings.exclude_filters.is_empty() {
                build_combined(
                    &settings.exclude_filters,
                    settings.use_regex,
                    settings.whole_word,
                )
            } else {
                None
            };
        }

        Ok(state)
    }
}

fn build_combined(filters: &[String], use_regex: bool, whole_word: bool) -> Option<FancyRegex> {
    if filters.is_empty() {
        return None;
    }
    let parts: Vec<String> = filters
        .iter()
        .map(|f| {
            if use_regex {
                format!("(?:{})", f)
            } else if whole_word {
                whole_word_pattern(f)
            } else {
                fancy_regex::escape(f).into_owned()
            }
        })
        .collect();
    let pattern = format!("(?:{})", parts.join("|"));
    // An invalid combined pattern (e.g. a broken user regex) just means
    // callers fall back to checking every filter on every line - slower,
    // never wrong.
    compile_case_insensitive(&pattern).ok()
}

/// Result of `apply_line_matching` - `passes_mode` is reported explicitly
/// (rather than inferred from an empty hits list) so the caller can
/// correctly distinguish "no per-line matches at all" (NoHit) from
/// "matches existed but the file failed the AllInFile/Proximity gate"
/// (ModeExcluded) - collapsing those two cases was a real bug caught
/// during the original C# port's review.
#[derive(Debug, Default)]
pub struct LineMatchOutcome {
    pub hits: Vec<LineHit>,
    pub excluded_by_file: bool,
    pub passes_mode: bool,
    pub proximity_min_range: Option<i32>,
}

/// Scans every line of an already-extracted file for filter/exclude
/// matches, then applies the AllInFile/Proximity gating rules. A direct
/// port of `Invoke-SingleFileSearch`'s line-matching loop.
pub fn apply_line_matching(
    lines: &[String],
    settings: &SearchSettings,
    state: &CompiledMatchState,
) -> LineMatchOutcome {
    let mut outcome = LineMatchOutcome {
        passes_mode: true,
        ..Default::default()
    };

    // Indexed by filter *slot* (position in settings.filters), not by
    // filter text - two filters differing only by case (or literal
    // duplicates) are distinct slots. Keying this by string would let
    // case-variant duplicate filters silently collapse into one entry,
    // skewing the proximity range calculation.
    let filter_count = settings.filters.len();
    let mut per_filter_hit_lines: Vec<Vec<i32>> = vec![Vec::new(); filter_count];

    // Literal mode never needs fancy_regex - see the doc comment on
    // `CompiledMatchState::filters_lower`. Lowercasing each line once here
    // (instead of once per combined-check call plus once per per-filter
    // `is_hit` call, as the regex-routed path effectively did before) is
    // itself part of the fix: same substring semantics, far fewer
    // allocations and no backtracking-VM invocation per line.
    let is_literal = !settings.use_regex && !settings.whole_word;

    for (i, line) in lines.iter().enumerate() {
        let line_number = (i + 1) as i32;
        let line_lower = if is_literal { Some(line.to_lowercase()) } else { None };

        if !settings.exclude_filters.is_empty() {
            let exclude_candidate = if is_literal {
                let ll = line_lower.as_deref().unwrap();
                state.exclude_filters_lower.iter().any(|f| ll.contains(f.as_str()))
            } else {
                state
                    .combined_exclude_regex
                    .as_ref()
                    .map(|rx| rx.is_match(line).unwrap_or(true))
                    .unwrap_or(true)
            };

            if exclude_candidate {
                let is_excluded_line = if is_literal {
                    let ll = line_lower.as_deref().unwrap();
                    state.exclude_filters_lower.iter().any(|f| ll.contains(f.as_str()))
                } else {
                    settings.exclude_filters.iter().any(|xf| {
                        is_hit(
                            line,
                            xf,
                            settings,
                            &state.compiled_exclude_regex,
                            &state.whole_word_exclude_regex,
                        )
                    })
                };
                if is_excluded_line {
                    if settings.exclude_scope == ExcludeScope::File {
                        outcome.excluded_by_file = true;
                    }
                    continue;
                }
            }
        }

        let mut matched_filters = Vec::new();
        let candidate_line = if is_literal {
            let ll = line_lower.as_deref().unwrap();
            state.filters_lower.iter().any(|f| ll.contains(f.as_str()))
        } else {
            state
                .combined_filter_regex
                .as_ref()
                .map(|rx| rx.is_match(line).unwrap_or(true))
                .unwrap_or(true)
        };

        if candidate_line {
            for (fi, f) in settings.filters.iter().enumerate() {
                let hit = if is_literal {
                    line_lower.as_deref().unwrap().contains(state.filters_lower[fi].as_str())
                } else {
                    is_hit(
                        line,
                        f,
                        settings,
                        &state.compiled_filter_regex,
                        &state.whole_word_filter_regex,
                    )
                };
                if hit {
                    matched_filters.push(f.clone());
                    per_filter_hit_lines[fi].push(line_number);
                }
            }
        }

        if !matched_filters.is_empty() {
            outcome.hits.push(LineHit {
                line_number,
                before: if i > 0 { lines.get(i - 1).cloned() } else { None },
                match_line: line.clone(),
                after: lines.get(i + 1).cloned(),
                matched_filters,
            });
        }
    }

    if outcome.excluded_by_file {
        return outcome;
    }

    if matches!(settings.match_mode, MatchMode::AllInFile | MatchMode::Proximity) {
        for hits in &per_filter_hit_lines {
            if hits.is_empty() {
                outcome.passes_mode = false;
                break;
            }
        }
    }

    if outcome.passes_mode && settings.match_mode == MatchMode::Proximity {
        // Each per-filter list was appended in increasing line order by
        // construction, so it's already sorted with no duplicates.
        let min_range = get_min_line_range_across_filters(&per_filter_hit_lines);
        outcome.proximity_min_range = Some(min_range);
        if min_range > settings.proximity_lines {
            outcome.passes_mode = false;
        }
    }

    outcome
}

fn is_hit(
    line: &str,
    filter: &str,
    settings: &SearchSettings,
    compiled_regex: &HashMap<String, FancyRegex>,
    whole_word_regex: &HashMap<String, FancyRegex>,
) -> bool {
    if settings.use_regex {
        compiled_regex
            .get(filter)
            .map(|rx| rx.is_match(line).unwrap_or(false))
            .unwrap_or(false)
    } else if settings.whole_word {
        whole_word_regex
            .get(filter)
            .map(|rx| rx.is_match(line).unwrap_or(false))
            .unwrap_or(false)
    } else {
        // Ordinal (not culture-aware) case-insensitive substring search -
        // mirrors the C# side's `IndexOf(filter, StringComparison.OrdinalIgnoreCase)`.
        line.to_lowercase().contains(&filter.to_lowercase())
    }
}

/// Given each filter slot's sorted hit-line-numbers within one file,
/// returns the smallest line span covering at least one line per filter -
/// the classic "smallest range covering one element from each list"
/// problem, solved by always advancing whichever list sits at the current
/// minimum. Assumes every filter has at least one entry (callers only call
/// this after confirming that via the AllInFile-style gate above).
pub fn get_min_line_range_across_filters(filter_line_lists: &[Vec<i32>]) -> i32 {
    let k = filter_line_lists.len();
    if k == 0 {
        return i32::MAX;
    }

    let mut ptr = vec![0usize; k];
    let mut best_range = i32::MAX;

    loop {
        let mut vals = vec![0i32; k];
        let mut exhausted = false;
        for i in 0..k {
            if ptr[i] >= filter_line_lists[i].len() {
                exhausted = true;
                break;
            }
            vals[i] = filter_line_lists[i][ptr[i]];
        }
        if exhausted {
            break;
        }

        let mut min_val = vals[0];
        let mut max_val = vals[0];
        let mut min_idx = 0;
        for i in 1..k {
            if vals[i] < min_val {
                min_val = vals[i];
                min_idx = i;
            }
            if vals[i] > max_val {
                max_val = vals[i];
            }
        }

        let range = max_val - min_val;
        if range < best_range {
            best_range = range;
        }

        ptr[min_idx] += 1;
    }

    best_range
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SearchSettings;

    fn settings_with_filters(filters: &[&str]) -> SearchSettings {
        SearchSettings {
            filters: filters.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn lines_of(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn any_line_mode_finds_case_insensitive_substring_hits() {
        let settings = settings_with_filters(&["hello"]);
        let state = CompiledMatchState::build(&settings).unwrap();
        let lines = lines_of(&["nothing here", "say HELLO world", "bye"]);
        let outcome = apply_line_matching(&lines, &settings, &state);
        assert_eq!(outcome.hits.len(), 1);
        assert_eq!(outcome.hits[0].line_number, 2);
        assert!(outcome.passes_mode);
        assert!(!outcome.excluded_by_file);
    }

    #[test]
    fn all_in_file_mode_requires_every_filter_present() {
        let mut settings = settings_with_filters(&["alpha", "beta"]);
        settings.match_mode = MatchMode::AllInFile;
        let state = CompiledMatchState::build(&settings).unwrap();

        let lines_missing_beta = lines_of(&["alpha only here"]);
        let outcome = apply_line_matching(&lines_missing_beta, &settings, &state);
        assert!(!outcome.passes_mode);
        assert!(!outcome.hits.is_empty(), "hits exist even though mode gate fails");

        let lines_with_both = lines_of(&["alpha here", "beta here"]);
        let outcome2 = apply_line_matching(&lines_with_both, &settings, &state);
        assert!(outcome2.passes_mode);
    }

    #[test]
    fn proximity_mode_computes_min_range_and_gates_on_it() {
        let mut settings = settings_with_filters(&["alpha", "beta"]);
        settings.match_mode = MatchMode::Proximity;
        settings.proximity_lines = 1;
        let state = CompiledMatchState::build(&settings).unwrap();

        let close = lines_of(&["alpha", "beta"]);
        let outcome = apply_line_matching(&close, &settings, &state);
        assert!(outcome.passes_mode);
        assert_eq!(outcome.proximity_min_range, Some(1));

        let far = lines_of(&["alpha", "x", "x", "x", "beta"]);
        let outcome2 = apply_line_matching(&far, &settings, &state);
        assert!(!outcome2.passes_mode);
        assert_eq!(outcome2.proximity_min_range, Some(4));
    }

    #[test]
    fn exclude_scope_file_excludes_whole_file_on_any_excluded_line() {
        let mut settings = settings_with_filters(&["target"]);
        settings.exclude_filters = vec!["skip".to_string()];
        settings.exclude_scope = ExcludeScope::File;
        let state = CompiledMatchState::build(&settings).unwrap();

        let lines = lines_of(&["target here", "skip this line"]);
        let outcome = apply_line_matching(&lines, &settings, &state);
        assert!(outcome.excluded_by_file);
    }

    #[test]
    fn exclude_scope_line_only_drops_the_matching_line() {
        let mut settings = settings_with_filters(&["target"]);
        settings.exclude_filters = vec!["skip".to_string()];
        settings.exclude_scope = ExcludeScope::Line;
        let state = CompiledMatchState::build(&settings).unwrap();

        let lines = lines_of(&["target skip", "target ok"]);
        let outcome = apply_line_matching(&lines, &settings, &state);
        assert!(!outcome.excluded_by_file);
        assert_eq!(outcome.hits.len(), 1);
        assert_eq!(outcome.hits[0].line_number, 2);
    }

    #[test]
    fn whole_word_matches_punctuation_edged_filter_like_c_sharp() {
        let mut settings = settings_with_filters(&["C#"]);
        settings.whole_word = true;
        let state = CompiledMatchState::build(&settings).unwrap();

        let hit = lines_of(&["I write C# for a living"]);
        let outcome = apply_line_matching(&hit, &settings, &state);
        assert_eq!(outcome.hits.len(), 1);

        let no_hit = lines_of(&["I write C#Sharp for a living"]);
        let outcome2 = apply_line_matching(&no_hit, &settings, &state);
        assert_eq!(outcome2.hits.len(), 0);
    }

    #[test]
    fn whole_word_rejects_substring_of_a_longer_word() {
        let mut settings = settings_with_filters(&["cat"]);
        settings.whole_word = true;
        let state = CompiledMatchState::build(&settings).unwrap();

        let lines = lines_of(&["concatenate this", "a cat sat"]);
        let outcome = apply_line_matching(&lines, &settings, &state);
        assert_eq!(outcome.hits.len(), 1);
        assert_eq!(outcome.hits[0].line_number, 2);
    }

    #[test]
    fn regex_mode_matches_pattern() {
        let mut settings = settings_with_filters(&[r"\d{3}-\d{4}"]);
        settings.use_regex = true;
        let state = CompiledMatchState::build(&settings).unwrap();

        let lines = lines_of(&["call 555-1234 now", "no number here"]);
        let outcome = apply_line_matching(&lines, &settings, &state);
        assert_eq!(outcome.hits.len(), 1);
        assert_eq!(outcome.hits[0].line_number, 1);
    }

    #[test]
    fn invalid_regex_filter_reports_which_filter_and_why() {
        let mut settings = settings_with_filters(&["(unclosed"]);
        settings.use_regex = true;
        let err = match CompiledMatchState::build(&settings) {
            Err(e) => e,
            Ok(_) => panic!("expected invalid regex filter to be rejected"),
        };
        assert_eq!(err.invalid_filters.len(), 1);
        assert_eq!(err.invalid_filters[0].filter, "(unclosed");
    }

    #[test]
    fn duplicate_filters_differing_only_by_case_are_distinct_slots_for_proximity() {
        // Two filters that are case-variants of each other must not collapse
        // into one proximity-range entry (that was a real bug: keying
        // per-filter hit lines by filter *text* instead of *slot*).
        let mut settings = settings_with_filters(&["Alpha", "alpha"]);
        settings.match_mode = MatchMode::AllInFile;
        let state = CompiledMatchState::build(&settings).unwrap();

        let lines = lines_of(&["alpha appears once"]);
        let outcome = apply_line_matching(&lines, &settings, &state);
        // Both slots matched the same single line, so AllInFile still passes.
        assert!(outcome.passes_mode);
        assert_eq!(outcome.hits[0].matched_filters.len(), 2);
    }

    #[test]
    fn min_line_range_across_filters_matches_expected_span() {
        // Every combo must include line 5 (list 1's only entry); the
        // closest neighbors to it are 6 (list 2) and either 1 or 10 (list
        // 0) - both give a span of 5, which is the true minimum.
        let lists = vec![vec![1, 10], vec![5], vec![6, 20]];
        assert_eq!(get_min_line_range_across_filters(&lists), 5);
    }

    /// Issue #9 epic §40: "Claude Code MUST explicitly evaluate regex
    /// denial-of-service risks." A classic ReDoS pattern
    /// (nested/ambiguous quantifiers) run in regex mode against
    /// adversarial input must not hang - fancy-regex's default 1,000,000
    /// backtrack limit (see this module's top doc comment) must kick in
    /// and bound the work, proven here with a real wall-clock assertion
    /// rather than just trusting the dependency's documented default.
    #[test]
    fn regex_backtrack_limit_bounds_a_classic_redos_pattern_instead_of_hanging() {
        let mut settings = settings_with_filters(&["(a+)+$"]);
        settings.use_regex = true;
        let state = CompiledMatchState::build(&settings).unwrap();

        // The textbook ReDoS trigger: a long run of the repeated
        // character followed by one character that can never complete
        // the match, forcing a naive backtracking engine through
        // exponentially many ways to partition the "a" run before it can
        // conclude there's no match.
        let adversarial_line = format!("{}!", "a".repeat(40));
        let lines = lines_of(&[adversarial_line.as_str()]);

        let start = std::time::Instant::now();
        let outcome = apply_line_matching(&lines, &settings, &state);
        let elapsed = start.elapsed();

        // Bounded by the backtrack limit, not exponential in input length -
        // generous enough to be robust on slow CI hardware, tight enough
        // that an unbounded hang would still fail this test rather than
        // stalling the suite.
        assert!(elapsed < std::time::Duration::from_secs(10), "regex match took {elapsed:?} - backtrack limit did not bound the work");
        // The backtrack limit is a *safety* bound, not a correctness
        // guarantee for pathological patterns - hitting it fails closed
        // (see this module's top doc comment), so "no hit" here is the
        // documented, acceptable outcome, not an assertion this specific
        // pattern must match.
        let _ = outcome;
    }
}
