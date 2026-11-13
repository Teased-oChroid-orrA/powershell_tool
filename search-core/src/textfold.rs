//! Opt-in text folding and bounded typo tolerance for the exact (keyword)
//! matcher. Nothing here runs unless `SearchSettings::fold_accents` /
//! `typo_tolerance` is set, so the default hot path is unchanged.
//!
//! Folding is applied identically to a line and to a filter *after*
//! extraction, so plain text, Office, ZIP, RTF and PDF text all match the
//! same way (the extractors differ in how they spell a character, not in
//! how matching compares it): precomposed vs combining accents, `fi`
//! ligature code points, no-break spaces and typographic quotes/dashes.
//!
//! A fold changes the index-time trigram vocabulary's meaning (the index
//! stores unfolded text), so `native_index::narrow_candidates` must not
//! narrow while folding or typo tolerance is on.

use std::borrow::Cow;

/// Pushes the folded form of one already-lowercased char (nothing for a
/// char that folds away, such as a combining accent).
fn push_folded(c: char, out: &mut String) {
    let s: &str = match c {
        '\u{300}'..='\u{36F}' | '\u{AD}' | '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' => return,
        '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => " ",
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' => "'",
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => "\"",
        '\u{2010}'..='\u{2015}' | '\u{2212}' => "-",
        '\u{2026}' => "...",
        '\u{FB00}' => "ff",
        '\u{FB01}' => "fi",
        '\u{FB02}' => "fl",
        '\u{FB03}' => "ffi",
        '\u{FB04}' => "ffl",
        '\u{FB05}' | '\u{FB06}' => "st",
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĳ' => "ij",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' => "t",
        'þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        other => {
            out.push(other);
            return;
        }
    };
    out.push_str(s);
}

/// Lowercases and folds `s` (see the module doc). ASCII input takes a
/// plain ASCII-lowercase fast path.
pub fn fold_lower(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        for lc in c.to_lowercase() {
            push_folded(lc, &mut out);
        }
    }
    out
}

/// [`fold_lower`], plus for each folded char the byte range of the source
/// char it came from, so a match found in the folded text can be shown on
/// the original line. `ranges[i]` belongs to the i-th char of the result.
pub fn fold_lower_mapped(s: &str, fold: bool) -> (String, Vec<(usize, usize)>) {
    let mut out = String::with_capacity(s.len());
    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(s.len());
    for (start, c) in s.char_indices() {
        let end = start + c.len_utf8();
        for lc in c.to_lowercase() {
            let before = out.len();
            if fold {
                push_folded(lc, &mut out);
            } else {
                out.push(lc);
            }
            if out.len() == before {
                // Folded away (a combining accent): the previous char's span grows over it.
                if let Some(last) = ranges.last_mut() {
                    last.1 = end;
                }
            }
            for _ in out[before..].chars() {
                ranges.push((start, end));
            }
        }
    }
    (out, ranges)
}

/// Byte offsets of the start of each char of `s` (for [`map_range`]).
pub fn char_starts(s: &str) -> Vec<usize> {
    s.char_indices().map(|(b, _)| b).collect()
}

/// Converts a byte range `[a, b)` of the folded text to the byte range of
/// the original text it came from. `char_at` is [`char_starts`] of the folded
/// text and `ranges` comes from [`fold_lower_mapped`].
pub fn map_range(char_at: &[usize], ranges: &[(usize, usize)], a: usize, b: usize) -> Option<(usize, usize)> {
    let first = char_at.partition_point(|&x| x < a);
    let last = char_at.partition_point(|&x| x < b).checked_sub(1)?;
    Some((ranges.get(first)?.0, ranges.get(last)?.1))
}

/// Byte offsets of every non-overlapping occurrence of `needle` in
/// `haystack`, converted to ranges on the original text via `ranges`.
pub fn original_spans(haystack: &str, needle: &str, ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let char_at = char_starts(haystack);
    haystack
        .match_indices(needle)
        .filter_map(|(b, m)| map_range(&char_at, ranges, b, b + m.len()))
        .collect()
}

/// Words of an already folded/lowercased line: runs of alphanumerics.
pub fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect()
}

/// A filter prepared for typo-tolerant matching: its words, each with the
/// number of edits it may differ by.
#[derive(Debug, Clone)]
pub struct FuzzyFilter {
    words: Vec<(Vec<char>, usize)>,
}

impl FuzzyFilter {
    /// `None` when typo tolerance cannot apply (no words, or no word long
    /// enough to be tolerant - the exact matcher already covers those).
    /// Words under 4 characters and words containing a digit stay exact
    /// (`M8` must never match `M9`); 8+ characters allow up to `max_edits`
    /// (at most 2), 4-7 allow one.
    pub fn new(folded_filter: &str, max_edits: u8) -> Option<Self> {
        let max = usize::from(max_edits.min(2));
        if max == 0 {
            return None;
        }
        let parts: Vec<(Vec<char>, usize)> = words(folded_filter)
            .into_iter()
            .map(|w| {
                let chars: Vec<char> = w.chars().collect();
                let allowed = if w.chars().any(|c| c.is_ascii_digit()) || chars.len() < 4 {
                    0
                } else if chars.len() >= 8 {
                    max
                } else {
                    1
                };
                (chars, allowed)
            })
            .collect();
        (parts.iter().any(|(_, k)| *k > 0)).then_some(Self { words: parts })
    }

    /// Number of words in the filter.
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// True when consecutive `line_words` match this filter's words in
    /// order, each within its allowed edit distance.
    pub fn matches(&self, line_words: &[&str]) -> bool {
        let n = self.words.len();
        if line_words.len() < n {
            return false;
        }
        line_words.windows(n).any(|win| {
            self.words
                .iter()
                .zip(win)
                .all(|((fw, k), lw)| within_edits(fw, lw, *k))
        })
    }
}

/// Optimal-string-alignment distance (insert, delete, substitute, swap two
/// neighbours) of `a` and `b` is at most `max`.
pub fn within_edits(a: &[char], b: &str, max: usize) -> bool {
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return false;
    }
    if max == 0 {
        return a == b.as_slice();
    }
    let (n, m) = (a.len(), b.len());
    let mut prev2: Vec<usize> = vec![0; m + 1];
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(prev2[j - 2] + 1);
            }
            cur[j] = d;
            row_min = row_min.min(d);
        }
        if row_min > max {
            return false;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m] <= max
}

/// Cheap borrow-or-fold used where a line may already be folded.
pub fn folded<'a>(s: &'a str, fold: bool) -> Cow<'a, str> {
    if fold {
        Cow::Owned(fold_lower(s))
    } else {
        Cow::Borrowed(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precomposed_and_combining_accents_fold_to_the_same_text() {
        assert_eq!(fold_lower("Caf\u{e9}"), "cafe");
        assert_eq!(fold_lower("Cafe\u{301}"), "cafe");
        assert_eq!(fold_lower("R\u{c9}SUM\u{c9}"), "resume");
        assert_eq!(fold_lower("Stra\u{df}e"), "strasse");
        assert_eq!(fold_lower("\u{130}stanbul"), "istanbul");
    }

    #[test]
    fn ligatures_spaces_quotes_and_dashes_fold() {
        assert_eq!(fold_lower("\u{fb01}nish"), "finish");
        assert_eq!(fold_lower("pre\u{a0}load"), "pre load");
        assert_eq!(fold_lower("don\u{2019}t"), "don't");
        assert_eq!(fold_lower("a\u{2013}b"), "a-b");
        assert_eq!(fold_lower("ABC def"), "abc def");
    }

    #[test]
    fn mapped_fold_points_back_at_the_original_chars() {
        let line = "un Cafe\u{301} chaud";
        let (folded, ranges) = fold_lower_mapped(line, true);
        assert_eq!(folded, "un cafe chaud");
        let spans = original_spans(&folded, "cafe", &ranges);
        assert_eq!(spans.len(), 1);
        // The span covers C, a, f, e and the combining accent that follows.
        assert_eq!(&line[spans[0].0..spans[0].1], "Cafe\u{301}");
        let (l2, r2) = fold_lower_mapped("\u{c9}t\u{e9} \u{c9}T\u{c9}", true);
        assert_eq!(original_spans(&l2, "ete", &r2).len(), 2);
    }

    #[test]
    fn edit_distance_is_bounded_and_counts_swaps_once() {
        let a: Vec<char> = "torque".chars().collect();
        assert!(within_edits(&a, "torque", 1));
        assert!(within_edits(&a, "toruqe", 1), "adjacent swap is one edit");
        assert!(within_edits(&a, "torqe", 1));
        assert!(within_edits(&a, "torques", 1));
        assert!(!within_edits(&a, "tork", 1));
        assert!(!within_edits(&a, "tarqua", 1));
        assert!(within_edits(&a, "tarque", 1));
    }

    #[test]
    fn short_and_numeric_words_stay_exact() {
        assert!(FuzzyFilter::new("m8", 2).is_none());
        assert!(FuzzyFilter::new("bolt", 0).is_none());
        let f = FuzzyFilter::new("bolt m8", 1).unwrap();
        assert!(f.matches(&["a", "boot", "m8"]));
        assert!(!f.matches(&["a", "boot", "m9"]), "a numeric word must match exactly");
        let g = FuzzyFilter::new("corrosion", 2).unwrap();
        assert!(g.matches(&["corosion"]));
        assert!(g.matches(&["corrossoin"]), "two edits allowed on 8+ letter words");
        let h = FuzzyFilter::new("corrosion", 1).unwrap();
        assert!(!h.matches(&["corrossoin"]));
    }
}
