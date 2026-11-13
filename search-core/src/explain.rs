//! Why a result matched, computed on demand from data a run already has (no
//! extra storage in the result model): which filters hit a line and where in
//! the line ([`LineExplainer::spans`]), and which filters and how often in a
//! file ([`explain_file`]). The UI calls these only for what it displays.

use fancy_regex::Regex;

use crate::models::{FileSearchResult, FileSearchStatus, SearchSettings};
use crate::ordering::{score_results, OrderScore};
use crate::textfold::{self, FuzzyFilter};

/// One matched stretch of a line: bytes `start..end` of the ORIGINAL line
/// (a char boundary on both ends), and the filter, as typed, that found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSpan {
    pub start: usize,
    pub end: usize,
    pub filter: String,
}

enum Matcher {
    Regex(Regex),
    /// Lowercased/folded filter text for substring search, optionally typo-tolerant.
    Text { needle: String, whole_word: Option<Regex>, fuzzy: Option<FuzzyFilter> },
}

/// Finds match positions the way the matcher decides matches (same case
/// rules, folding, whole-word boundary, regex flags, typo tolerance).
pub struct LineExplainer {
    filters: Vec<(String, Matcher)>,
    fold: bool,
}

impl LineExplainer {
    /// `None` if a regex filter does not compile (the run would have failed
    /// with `InvalidFilterRegex` before any result existed).
    pub fn new(settings: &SearchSettings) -> Option<Self> {
        let fold = settings.fold_accents && !settings.use_regex;
        let mut filters = Vec::new();
        for f in settings.all_filters() {
            // A NEAR query shows both of its words (labelled with the whole filter).
            let texts = if settings.use_regex { vec![f.clone()] } else { crate::near::terms(f) };
            for text in &texts {
                let m = if settings.use_regex {
                    Matcher::Regex(case_insensitive(text)?)
                } else {
                    let needle = if fold { textfold::fold_lower(text) } else { text.to_lowercase() };
                    let whole_word = if settings.whole_word {
                        Some(case_insensitive(&crate::matching::whole_word_pattern(&needle))?)
                    } else {
                        None
                    };
                    let fuzzy = (settings.typo_tolerance > 0 && texts.len() == 1)
                        .then(|| FuzzyFilter::new(&needle, settings.typo_tolerance))
                        .flatten();
                    Matcher::Text { needle, whole_word, fuzzy }
                };
                filters.push((f.clone(), m));
            }
        }
        Some(Self { filters, fold })
    }

    /// Every match of every filter in `line`, in line order.
    pub fn spans(&self, line: &str) -> Vec<MatchSpan> {
        let mut out = Vec::new();
        let mut mapped: Option<(String, Vec<(usize, usize)>, Vec<usize>)> = None;
        for (filter, m) in &self.filters {
            match m {
                Matcher::Regex(rx) => {
                    for found in rx.find_iter(line).flatten() {
                        if found.start() < found.end() {
                            out.push(MatchSpan { start: found.start(), end: found.end(), filter: filter.clone() });
                        }
                    }
                }
                Matcher::Text { needle, whole_word, fuzzy } => {
                    let (low, ranges, char_at) = mapped.get_or_insert_with(|| {
                        let (low, ranges) = textfold::fold_lower_mapped(line, self.fold);
                        let char_at = textfold::char_starts(&low);
                        (low, ranges, char_at)
                    });
                    let mut found: Vec<(usize, usize)> = match whole_word {
                        Some(rx) => rx.find_iter(low).flatten().map(|f| (f.start(), f.end())).collect(),
                        None => low.match_indices(needle.as_str()).map(|(b, t)| (b, b + t.len())).collect(),
                    };
                    if found.is_empty() {
                        if let Some(ff) = fuzzy {
                            found = fuzzy_ranges(low, ff);
                        }
                    }
                    for (a, b) in found {
                        if a < b {
                            if let Some((start, end)) = textfold::map_range(char_at, ranges, a, b) {
                                out.push(MatchSpan { start, end, filter: filter.clone() });
                            }
                        }
                    }
                }
            }
        }
        out.sort_by_key(|s| (s.start, s.end));
        out
    }
}

fn case_insensitive(pattern: &str) -> Option<Regex> {
    fancy_regex::RegexBuilder::new(pattern).case_insensitive(true).build().ok()
}

/// Byte ranges of runs of consecutive words of `low` that `ff` accepts.
fn fuzzy_ranges(low: &str, ff: &FuzzyFilter) -> Vec<(usize, usize)> {
    let mut pos: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (i, c) in low.char_indices() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                pos.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        pos.push((s, low.len()));
    }
    let n = ff.word_count();
    let mut out = Vec::new();
    let mut i = 0;
    while i + n <= pos.len() {
        let words: Vec<&str> = pos[i..i + n].iter().map(|&(a, b)| &low[a..b]).collect();
        if ff.matches(&words) {
            out.push((pos[i].0, pos[i + n - 1].1));
            i += n;
        } else {
            i += 1;
        }
    }
    out
}

/// Evidence for one filter in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterEvidence {
    pub filter: String,
    /// Hit lines this filter matched.
    pub lines: usize,
    pub first_line: i32,
}

/// Why one file is in the results.
#[derive(Debug, Clone, PartialEq)]
pub struct FileWhy {
    pub filters: Vec<FilterEvidence>,
    /// Filters found in the file name.
    pub in_filename: Vec<String>,
    pub proximity_min_range: Option<i32>,
    /// Relevance score within `results` (see `ordering`).
    pub score: f64,
}

/// Explains every hit file of a run (same order as `results`; `None` for
/// files that are not hits). One pass over the hits, no re-matching.
pub fn explain_file(results: &[FileSearchResult], settings: &SearchSettings) -> Vec<Option<FileWhy>> {
    let scores: Vec<OrderScore> = score_results(results, settings);
    results
        .iter()
        .zip(scores)
        .map(|(r, sc)| {
            if r.status != FileSearchStatus::Hit {
                return None;
            }
            let filters = sc
                .matched_filters
                .iter()
                .map(|f| {
                    let lines: Vec<i32> = r.hits.iter().filter(|h| h.matched_filters.contains(f)).map(|h| h.line_number).collect();
                    FilterEvidence { filter: f.clone(), lines: lines.len(), first_line: lines.first().copied().unwrap_or(0) }
                })
                .collect();
            Some(FileWhy { filters, in_filename: sc.filename_filters, proximity_min_range: r.proximity_min_range, score: sc.score })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans_of(settings: &SearchSettings, line: &str) -> Vec<(String, String)> {
        LineExplainer::new(settings)
            .unwrap()
            .spans(line)
            .into_iter()
            .map(|s| (line[s.start..s.end].to_string(), s.filter))
            .collect()
    }

    fn s(filters: &[&str]) -> SearchSettings {
        SearchSettings { filters: filters.iter().map(|f| f.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn literal_spans_keep_the_original_case_and_cover_every_occurrence() {
        let got = spans_of(&s(&["torque", "BOLT"]), "Torque the bolt, then TORQUE it");
        assert_eq!(
            got,
            vec![
                ("Torque".to_string(), "torque".to_string()),
                ("bolt".to_string(), "BOLT".to_string()),
                ("TORQUE".to_string(), "torque".to_string()),
            ]
        );
    }

    #[test]
    fn whole_word_regex_and_folded_spans_point_at_the_original_text() {
        let mut ww = s(&["cat"]);
        ww.whole_word = true;
        assert_eq!(spans_of(&ww, "concatenate the Cat"), vec![("Cat".to_string(), "cat".to_string())]);

        let mut rx = s(&[r"\d{3}-\d{4}"]);
        rx.use_regex = true;
        assert_eq!(spans_of(&rx, "call 555-1234 now"), vec![("555-1234".to_string(), r"\d{3}-\d{4}".to_string())]);

        let mut fold = s(&["cafe"]);
        fold.fold_accents = true;
        let line = "un Cafe\u{301} et un caf\u{e9}";
        let got = spans_of(&fold, line);
        assert_eq!(got.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), vec!["Cafe\u{301}", "caf\u{e9}"]);
    }

    #[test]
    fn typo_tolerant_spans_cover_the_misspelled_word() {
        let mut t = s(&["corrosion"]);
        t.typo_tolerance = 1;
        assert_eq!(spans_of(&t, "found corosion on the wing"), vec![("corosion".to_string(), "corrosion".to_string())]);
        assert!(spans_of(&t, "nothing here").is_empty());
    }

    #[test]
    fn file_why_lists_filters_lines_and_filename() {
        use crate::models::LineHit;
        let hit = |n: i32, fs: &[&str]| LineHit { line_number: n, matched_filters: fs.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let r = FileSearchResult {
            full_name: "/x/torque notes.txt".into(),
            status: FileSearchStatus::Hit,
            hits: vec![hit(4, &["torque"]), hit(9, &["torque", "bolt"])],
            created: chrono::Local::now(),
            modified: chrono::Local::now(),
            file_length: 0,
            lines_cache: vec![],
            total_line_count: 20,
            proximity_min_range: Some(5),
            low_confidence_pdf: false,
            error_message: None,
        };
        let why = explain_file(&[r], &s(&["torque", "bolt"])).remove(0).unwrap();
        assert_eq!(why.filters[0], FilterEvidence { filter: "torque".into(), lines: 2, first_line: 4 });
        assert_eq!(why.filters[1], FilterEvidence { filter: "bolt".into(), lines: 1, first_line: 9 });
        assert_eq!(why.in_filename, vec!["torque"]);
        assert_eq!(why.proximity_min_range, Some(5));
    }
}
