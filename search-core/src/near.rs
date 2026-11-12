//! `word NEAR/n word` filters: both words on the same line, at most `n` word
//! positions apart (adjacent words are distance 1), in either order.
//!
//! A filter is a NEAR query only when it is exactly three whitespace-separated
//! pieces - a word, `NEAR/n` (any case, `n >= 1`), a word - and both words are
//! alphanumeric runs. Anything else (a phrase, `C# NEAR/3 bolt`, `NEAR/0`) is
//! an ordinary filter, exactly as before, and regex mode never uses NEAR.
//!
//! Words are the same runs of alphanumerics the typo-tolerant matcher uses
//! ([`textfold::words`]). A term hits a word the way a plain filter hits text:
//! as a substring (`bolt` hits `bolts`), or as the whole word in whole-word
//! mode, after lowercasing (and accent folding when it is on).

use crate::textfold;

/// A parsed `a NEAR/n b` filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearQuery {
    pub first: String,
    pub second: String,
    pub distance: usize,
}

/// `Some` when `filter` is a NEAR query (see the module doc).
pub fn parse(filter: &str) -> Option<NearQuery> {
    let mut parts = filter.split_whitespace();
    let (first, op, second) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || op.len() < 6 || !op.is_char_boundary(5) || !op[..5].eq_ignore_ascii_case("near/") {
        return None;
    }
    let distance: usize = op[5..].parse().ok().filter(|&n| n >= 1)?;
    let word = |w: &str| !w.is_empty() && w.chars().all(char::is_alphanumeric);
    (word(first) && word(second)).then(|| NearQuery {
        first: first.to_string(),
        second: second.to_string(),
        distance,
    })
}

/// The texts a filter looks for: both words of a NEAR query, else the filter
/// itself. Used where only presence matters (highlighting, index narrowing).
pub fn terms(filter: &str) -> Vec<String> {
    match parse(filter) {
        Some(q) => vec![q.first, q.second],
        None => vec![filter.to_string()],
    }
}

/// What an index (trigram) narrowing may require of a file for this filter:
/// the filter itself, or for a NEAR query its longer word (a file lacking
/// either word cannot match, and the longer word narrows more).
pub fn narrowing_text(filter: &str) -> String {
    match parse(filter) {
        Some(q) if q.second.chars().count() > q.first.chars().count() => q.second,
        Some(q) => q.first,
        None => filter.to_string(),
    }
}

/// `filters` with every NEAR query replaced by its words (regex-mode filters
/// are never NEAR queries and pass through).
pub fn expand_terms(filters: &[String], use_regex: bool) -> Vec<String> {
    if use_regex {
        return filters.to_vec();
    }
    filters.iter().flat_map(|f| terms(f)).collect()
}

/// A NEAR query ready to test lines: words lowercased/folded as the matcher
/// prepares plain filters.
#[derive(Debug, Clone)]
pub struct CompiledNear {
    first: String,
    second: String,
    distance: usize,
    whole_word: bool,
}

impl CompiledNear {
    pub fn new(q: &NearQuery, fold: bool, whole_word: bool) -> Self {
        let prep = |t: &str| if fold { textfold::fold_lower(t) } else { t.to_lowercase() };
        Self { first: prep(&q.first), second: prep(&q.second), distance: q.distance, whole_word }
    }

    /// The text a cheap "might this line match" pre-check can look for.
    pub fn probe(&self) -> &str {
        &self.first
    }

    /// `line_lower` is the line already lowercased (and folded) like the terms.
    pub fn matches(&self, line_lower: &str) -> bool {
        let words = textfold::words(line_lower);
        let positions = |term: &str| -> Vec<usize> {
            words
                .iter()
                .enumerate()
                .filter(|(_, w)| if self.whole_word { **w == term } else { w.contains(term) })
                .map(|(i, _)| i)
                .collect()
        };
        let a = positions(&self.first);
        if a.is_empty() {
            return false;
        }
        let b = if self.second == self.first { a.clone() } else { positions(&self.second) };
        // Some a-position and some b-position, different words, within `distance`.
        a.iter().any(|&i| {
            let from = b.partition_point(|&j| j + self.distance < i);
            b[from..].iter().take_while(|&&j| j <= i + self.distance).any(|&j| j != i)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(filter: &str, whole_word: bool) -> CompiledNear {
        CompiledNear::new(&parse(filter).expect("near query"), false, whole_word)
    }

    #[test]
    fn parses_only_word_near_word() {
        let q = parse("torque NEAR/5 bolt").unwrap();
        assert_eq!((q.first.as_str(), q.second.as_str(), q.distance), ("torque", "bolt", 5));
        assert!(parse("torque near/12 bolt").is_some());
        assert!(parse("  torque   NeAr/2   bolt ").is_some());
        for plain in [
            "torque", "torque bolt", "torque NEAR bolt", "torque NEAR/0 bolt", "torque NEAR/x bolt",
            "torque NEAR/-1 bolt", "torque NEAR/ bolt", "a b NEAR/3 c", "torque NEAR/5 bolt nut",
            "C# NEAR/3 bolt", "torque NEAR/3 bo-lt", "NEAR/5", "",
        ] {
            assert!(parse(plain).is_none(), "{plain:?} must stay a plain filter");
        }
        assert!(parse("torque NEAR/99999999999999999999 bolt").is_none(), "overflow is not a distance");
    }

    #[test]
    fn distance_boundary_and_order() {
        let q = near("torque NEAR/3 bolt", false);
        assert!(q.matches("torque bolt"));
        assert!(q.matches("torque a b bolt"), "3 apart is within 3");
        assert!(!q.matches("torque a b c bolt"), "4 apart is not");
        assert!(q.matches("bolt a b torque"), "order does not matter");
        assert!(!q.matches("bolt a b c torque"));
        assert!(!q.matches("torque only"));
        assert!(!q.matches("only bolt"));
    }

    #[test]
    fn any_pair_of_occurrences_counts() {
        let q = near("torque NEAR/2 bolt", false);
        assert!(q.matches("torque x x x x x x bolt x torque"), "second torque is adjacent to bolt");
        assert!(q.matches("bolt x x x x x torque x bolt"));
        assert!(!q.matches("torque x x x bolt x x x torque"));
    }

    #[test]
    fn same_word_needs_two_occurrences() {
        let q = near("bolt NEAR/3 bolt", false);
        assert!(!q.matches("one bolt only"));
        assert!(q.matches("bolt and bolt"));
        assert!(!q.matches("bolt a b c bolt"));
    }

    #[test]
    fn punctuation_does_not_count_as_a_word() {
        let q = near("torque NEAR/1 bolt", false);
        assert!(q.matches("torque, bolt"));
        assert!(q.matches("torque -- (bolt)"));
    }

    #[test]
    fn substring_versus_whole_word() {
        assert!(near("bolt NEAR/2 torque", false).matches("bolts and torque"));
        assert!(!near("bolt NEAR/2 torque", true).matches("bolts and torque"));
        assert!(near("bolt NEAR/2 torque", true).matches("bolt and torque"));
    }

    #[test]
    fn case_and_accents_follow_the_matcher() {
        let q = near("Torque NEAR/2 BOLT", false);
        assert!(q.matches(&"TORQUE the Bolt".to_lowercase()));
        let accented = NearQuery { first: "café".into(), second: "bolt".into(), distance: 2 };
        assert!(!CompiledNear::new(&accented, false, false).matches("cafe bolt"));
        assert!(CompiledNear::new(&accented, true, false).matches(&textfold::fold_lower("CAFÉ bolt")));
    }

    #[test]
    fn terms_expand_only_near_queries() {
        assert_eq!(terms("a NEAR/2 b"), vec!["a", "b"]);
        assert_eq!(terms("plain filter"), vec!["plain filter"]);
        let fs = vec!["a NEAR/2 b".to_string(), "c".to_string()];
        assert_eq!(expand_terms(&fs, false), vec!["a", "b", "c"]);
        assert_eq!(expand_terms(&fs, true), fs);
        assert_eq!(narrowing_text("ab NEAR/2 torque"), "torque");
        assert_eq!(narrowing_text("torque NEAR/2 ab"), "torque");
        assert_eq!(narrowing_text("plain text"), "plain text");
    }
}
