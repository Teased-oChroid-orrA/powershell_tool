//! Deterministic noise-line filter for text taken from PDF text layers and
//! OCR (`docs/search-ocr-line-quality.md` has the measurements behind every
//! rule here).
//!
//! The index and the result snippets are built from the extracted lines, so a
//! line that is binary residue (`ÿÎ\x04i¹ô!`), an OCR fragment (`l1|~ . ,; Il`,
//! a lone `T`) or a glyph run that never decoded is a false hit waiting to
//! happen and wastes embedding and index space. This module scores one line at
//! a time with plain character statistics - no model, no per-line cost beyond
//! a pass over its characters, nothing to download - and drops only lines that
//! have **no** plausible content. The bias is deliberate: engineering text that
//! merely looks odd (`MIL-STD-1553`, `AN3-5A`, `0.125 in`, `Ftu = 62 ksi`,
//! `3/8-24 UNF`, `σ = F/A`) must survive, so a line is kept as soon as one
//! token looks like a word, a number or a part number, and is dropped only when
//! nothing does or when symbols and fragments clearly dominate.

use std::borrow::Cow;
use std::collections::HashSet;

/// Why a line was judged noise (for tests and diagnostics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Noise {
    /// Control characters, C1 bytes, replacement characters or Latin-1
    /// symbol soup: undecoded binary.
    Binary,
    /// Nothing in the line looks like a word, a number or a part number.
    NoContent,
    /// One or two plausible tokens next to a mass of fragments and junk.
    Sparse,
}

/// Where a line came from. A PDF text layer holds what the generator drew, so a
/// lone capital letter there is usually a real symbol; OCR output has
/// no such guarantee and lone letters are mostly speckle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    TextLayer,
    Ocr,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Token {
    /// Word, number, part number or unit.
    Good,
    /// One capital letter (a callout or column label): counts only in a line
    /// with at least two tokens.
    Label,
    /// Operators and punctuation that are fine anywhere but are no content.
    Neutral,
    /// Fragment, stroke confusion (`l1`, `Il`), or stray symbols.
    Noise,
}

/// Two-letter units and abbreviations without a vowel.
const SHORT_UNITS: &[&str] = &[
    "mm", "cm", "km", "kg", "ft", "lb", "pa", "hz", "db", "kn", "kw", "mw", "mv", "ma", "ph", "mg", "ms", "ns", "kv", "nm", "rh", "st",
    "nd", "rd", "th", "vs", "tm", "pc", "cc", "hr", "yr", "dc", "ac", "gs", "gp",
];

fn is_vowel(c: char) -> bool {
    matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y') || (!c.is_ascii() && c.is_alphabetic())
}

/// Characters OCR and broken text layers scatter through short runs of letters
/// (ruled lines, speckle). Only counted *inside* a token: `Why?`, `|x|` are fine.
fn is_weird(c: char) -> bool {
    matches!(c, '|' | '~' | '¦' | '¬' | '?' | '!')
}

/// Latin-1 symbols (U+0080-U+00BF) that real engineering/prose text uses.
fn is_legit_latin1_symbol(c: char) -> bool {
    matches!(c, '°' | '±' | '²' | '³' | 'µ' | '·' | '¼' | '½' | '¾' | 'º' | 'ª' | '«' | '»' | '©' | '®' | '§' | '¶' | '¿' | '¡' | '\u{a0}' | '\u{ad}' | '´' | '¹' | '¨' | '¸' | '¯')
}

/// `true` for a token made only of I, l, |, !, 1 with at least one stroke
/// letter: how OCR renders speckle, ruled lines and stems (`l1`, `Il`, `I1l`).
fn stroke_only(core: &str) -> bool {
    // `II`, `III` are roman numerals (Type II anodize), not speckle.
    !matches!(core, "II" | "III") && core.chars().count() <= 5
        && core.chars().all(|c| matches!(c, 'I' | 'l' | '|' | '!' | '1'))
        && core.chars().any(|c| matches!(c, 'I' | 'l' | '|' | '!'))
}

fn classify(raw: &str) -> Token {
    let core = raw.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '|' | '"' | '\'' | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}'));
    if core.is_empty() {
        return Token::Neutral;
    }
    let chars: Vec<char> = core.chars().collect();
    let alnum = chars.iter().filter(|c| c.is_alphanumeric()).count();
    let weird = (1..chars.len().saturating_sub(1))
        .filter(|&i| {
            let c = chars[i];
            // `|` and `~` inside a token are ruled-line speckle; `?` and `!` are
            // prose punctuation unless wedged between letters (`in?tu`).
            matches!(c, '|' | '~' | '\u{a6}' | '\u{ac}') || (is_weird(c) && chars[i - 1].is_alphanumeric() && chars[i + 1].is_alphanumeric())
        })
        .count();
    if alnum == 0 {
        let neutral = chars.iter().all(|c| {
            matches!(c, '=' | '+' | '-' | '\u{2013}' | '\u{2014}' | '/' | '\u{d7}' | '\u{f7}' | '\u{b1}' | '\u{b0}' | '%' | '*' | '<' | '>' | '\u{2264}' | '\u{2265}' | '\u{2248}' | '\u{2260}' | '#' | '&' | '@' | '$' | '_' | '|' | '~' | '\u{2022}' | '\u{b7}' | '\u{2192}' | '\u{221e}' | '\u{2205}' | '\u{d8}')
                || (!c.is_ascii() && !c.is_control() && !c.is_alphanumeric() && !is_legit_latin1_gap(*c))
        });
        return if neutral { Token::Neutral } else { Token::Noise };
    }
    if stroke_only(core) {
        return Token::Noise;
    }
    // Real words carry few Latin-1 accents; binary read as Latin-1 is full of them.
    let hi = chars.iter().filter(|c| (0xC0..0x100).contains(&(**c as u32)) && **c != '\u{d7}' && **c != '\u{f7}').count();
    if hi >= 3 && hi * 8 >= chars.len() {
        return Token::Noise;
    }
    // Symbols jammed inside a short run of letters/digits (`l1|~`, `in?tu`).
    if weird > 0 && (alnum <= 4 || weird * 3 >= alnum) {
        return Token::Noise;
    }
    let letters = chars.iter().filter(|c| c.is_alphabetic()).count();
    let digits = chars.iter().filter(|c| c.is_ascii_digit()).count();
    if letters == 0 {
        // Numbers, ranges, fractions, times, dimensions, money: all content;
        // a digit run salted with `!%#$&` is a glyph-index sequence.
        let body = core.trim_start_matches(|c: char| matches!(c, '$' | '\u{a3}' | '\u{20ac}' | '#' | '~' | '\u{b1}' | '+' | '-' | '\u{2212}' | '\u{2248}' | '<' | '>' | '\u{2264}' | '\u{2265}'));
        let clean = body.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '/' | ':' | '-' | '%' | '+' | '\u{b0}' | '\u{d7}' | '\u{b1}' | '\u{2013}' | '\u{2212}' | '\u{b2}' | '\u{b3}' | '\u{b9}' | '\u{bc}' | '\u{bd}' | '\u{be}' | '\u{2032}' | '\u{2033}' | ')' | '(' | '\'' | '"'));
        return if clean { Token::Good } else { Token::Noise };
    }
    if chars.len() == 1 {
        let c = chars[0];
        if (c as u32) >= 0x100 {
            return Token::Good; // one CJK/Greek/Cyrillic letter is a word or a variable
        }
        return if c.is_ascii_uppercase() && c != 'I' { Token::Label } else { Token::Noise };
    }
    let letter_chars: Vec<char> = chars.iter().copied().filter(|c| c.is_alphabetic()).collect();
    let lower: String = letter_chars.iter().flat_map(|c| c.to_lowercase()).collect();
    let all_upper = letter_chars.iter().all(|c| c.is_uppercase());
    let has_vowel = letter_chars.iter().any(|c| is_vowel(*c));
    // `MLLMs`, `PDEs`, `GPUs`: an acronym with a plural s.
    let plural_acronym = n_plural(&letter_chars);
    let mut run = 0usize;
    let mut max_run = 0usize;
    let mut same = 1usize;
    let mut max_same = 1usize;
    let mut prev: Option<char> = None;
    for &c in &chars {
        if !c.is_alphabetic() {
            run = 0;
            same = 1;
            prev = None;
            continue;
        }
        if is_vowel(c) {
            run = 0;
        } else {
            run += 1;
            max_run = max_run.max(run);
        }
        if Some(c) == prev {
            same += 1;
            max_same = max_same.max(same);
        } else {
            same = 1;
        }
        prev = Some(c);
    }
    let switches = letter_chars.windows(2).filter(|w| w[0].is_uppercase() != w[1].is_uppercase()).count();
    if switches >= 3 && letter_chars.iter().any(|c| !c.is_ascii()) {
        return Token::Noise; // `ÜMžCþJ`: random bytes read as Latin-1 letters
    }
    if max_same >= 4 && chars.iter().any(|c| c.is_lowercase()) {
        return Token::Noise; // `qqqqqq`, `llllll` (all-capital `FFFF` is a hex constant)
    }
    if digits > 0 {
        // Part numbers, symbols with subscripts and code: AN3-5A, R1, M12, 4130N.
        return Token::Good;
    }
    let n = letter_chars.len();
    if n == 2 {
        // `mP`, `mN`, `Wn`, `tx` are fragments; `in`, `of`, `mm`, `ft` are words and units.
        let first_lower = letter_chars[0].is_lowercase();
        let second_upper = letter_chars[1].is_uppercase();
        if first_lower && second_upper {
            return if SHORT_UNITS.contains(&lower.as_str()) || all_upper { Token::Good } else { Token::Noise };
        }
        return if (has_vowel && !stroke_only(core)) || SHORT_UNITS.contains(&lower.as_str()) || all_upper && letter_chars.iter().all(|c| *c != 'I') {
            Token::Good
        } else {
            Token::Noise
        };
    }
    if max_run >= 7 {
        return Token::Noise;
    }
    if has_vowel || all_upper || plural_acronym || n <= 4 {
        Token::Good // short vowel-less lowercase: psf, lbf, rpm, kPa
    } else {
        Token::Noise
    }
}

fn n_plural(letters: &[char]) -> bool {
    match letters.split_last() {
        Some(('s', stem)) => stem.len() >= 2 && stem.iter().all(|c| c.is_uppercase()),
        _ => false,
    }
}

fn is_legit_latin1_gap(c: char) -> bool {
    (c as u32) < 0xC0 && !is_legit_latin1_symbol(c)
}

/// `Some(reason)` when `line` has no plausible content; `None` when it stays.
pub fn assess(line: &str, source: Source) -> Option<Noise> {
    let mut non_space = 0usize;
    let mut c0 = 0usize; // control codes and undefined C1 bytes
    let mut c1 = 0usize; // C1 bytes that are Windows-1252 punctuation when decoded properly
    let mut sym = 0usize;
    let mut hi = 0usize; // accented Latin-1 letters and odd symbols
    for c in line.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            continue;
        }
        non_space += 1;
        let cp = c as u32;
        if (0xC0..0x100).contains(&cp) && cp != 0xD7 && cp != 0xF7 {
            hi += 1;
        }
        if (0x80..0xA0).contains(&cp) && !matches!(cp, 0x81 | 0x8d | 0x8f | 0x90 | 0x9d) {
            c1 += 1;
        } else if c.is_control() || c == '\u{fffd}' {
            c0 += 1;
        } else if (0xA1..0xC0).contains(&cp) && !is_legit_latin1_symbol(c) {
            sym += 1;
        }
    }
    if non_space == 0 {
        return Some(Noise::NoContent);
    }
    let fixed = repair_encoding(line);
    let line = fixed.as_ref();
    let mut tokens = 0usize;
    let mut good = 0usize;
    let mut labels = 0usize;
    let mut noise_chars = 0usize;
    for raw in line.split_whitespace() {
        tokens += 1;
        match classify(raw) {
            Token::Good => {
                good += 1;
            }
            Token::Label => {
                labels += 1;
            }
            Token::Noise => noise_chars += raw.chars().count(),
            Token::Neutral => {}
        }
    }
    let content = good + if tokens >= 2 || source == Source::TextLayer { labels } else { 0 };
    if content == 0 {
        return Some(Noise::NoContent);
    }
    if content <= 2 && noise_chars * 2 > non_space {
        return Some(Noise::Sparse);
    }
    // Judged on the line as extracted, once its words are known: one or two
    // stray glyph codes in a line of real words are a font quirk
    // (`the \x01nancial \x01eld.`) and curly quotes arrive as C1 bytes, but a
    // line mostly made of them, or with hardly any words, is undecoded binary.
    // A control code counts double: C1 bytes are mostly honest punctuation.
    let bad = 2 * c0 + c1;
    if bad * 10 > non_space * 6 || (bad >= 3 && bad * 10 > non_space * 2 && content < 2) || (non_space >= 4 && sym * 3 > non_space) || (hi + sym >= 5 && (hi + sym) * 5 > non_space) {
        return Some(Noise::Binary);
    }
    None
}

/// `true` when `line` should not be indexed or shown.
pub fn is_noise(line: &str, source: Source) -> bool {
    assess(line, source).is_some()
}

/// Computer Modern (TeX) text fonts encode ligatures as control codes:
/// `\x0b` ff, `\x0c` fi, `\x0d` fl, `\x0e` ffi, `\x0f` ffl. Inside a word the
/// code is unambiguous enough to restore (`\x0cles` -> `files`), which both
/// repairs search for those words and keeps the line from reading as binary.
fn repair_ligature_controls(line: &str) -> Cow<'_, str> {
    if !line.chars().any(|c| ('\u{b}'..='\u{f}').contains(&c)) {
        return Cow::Borrowed(line);
    }
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        let lig = match c {
            '\u{b}' => "ff",
            '\u{c}' => "fi",
            '\u{d}' => "fl",
            '\u{e}' => "ffi",
            '\u{f}' => "ffl",
            _ => {
                out.push(c);
                continue;
            }
        };
        let letter_before = i > 0 && chars[i - 1].is_alphabetic();
        let letter_after = chars.get(i + 1).is_some_and(|n| n.is_alphabetic());
        if letter_before || letter_after {
            out.push_str(lig);
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// PDF `WinAnsiEncoding` is Windows-1252, but the extractor reads bytes as
/// Latin-1, so curly quotes, dashes and bullets arrive as C1 controls
/// (`\x93models,\x94`). Maps them back; the five undefined bytes stay.
fn repair_cp1252(line: &str) -> Cow<'_, str> {
    const MAP: [char; 32] = [
        '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}', '\u{8f}',
        '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}', '\u{178}',
    ];
    if !line.chars().any(|c| ('\u{80}'..='\u{9f}').contains(&c)) {
        return Cow::Borrowed(line);
    }
    Cow::Owned(line.chars().map(|c| if ('\u{80}'..='\u{9f}').contains(&c) { MAP[c as usize - 0x80] } else { c }).collect())
}

/// Control characters that carry no text (tab, newline and carriage return stay).
fn is_stray_control(c: char) -> bool {
    c.is_control() && !matches!(c, '\t' | '\n' | '\r')
}

fn repair_encoding(line: &str) -> Cow<'_, str> {
    if !line.chars().any(char::is_control) {
        return Cow::Borrowed(line); // both repairs only touch control codes
    }
    let fixed = repair_cp1252(&repair_ligature_controls(line)).into_owned();
    if fixed == line { Cow::Borrowed(line) } else { Cow::Owned(fixed) }
}

/// Repairs ligature codes, then removes noise lines in place, keeping the
/// parallel per-line `origins` (PDF stream offsets for page citation) aligned.
/// Returns how many lines were dropped.
pub(crate) fn clean_lines(lines: &mut Vec<String>, source: Source, origins: Option<&mut Vec<Option<usize>>>) -> usize {
    let before = lines.len();
    // Judged as extracted: repairing first would hide the binary signal.
    let mut keep: Vec<bool> = lines.iter().map(|l| !is_noise(l, source)).collect();
    // Lone capital letters are real labels beside text, but a "document" of
    // nothing else is not text (a font program that happened to decode).
    let lone_label = |l: &str| {
        let mut chars = l.trim().chars();
        matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_uppercase())
    };
    if keep.iter().zip(lines.iter()).all(|(k, l)| !*k || lone_label(l)) {
        keep.iter_mut().for_each(|k| *k = false);
    }
    let mut flags = keep.iter().copied();
    lines.retain(|_| flags.next().unwrap_or(true));
    for line in lines.iter_mut() {
        if let Cow::Owned(fixed) = repair_encoding(line) {
            *line = fixed;
        }
        if line.chars().any(is_stray_control) {
            line.retain(|c| !is_stray_control(c));
        }
    }
    let mut origins = origins;
    if let Some(origins) = origins.as_deref_mut() {
        if origins.len() == keep.len() {
            let mut flags = keep.iter().copied();
            origins.retain(|_| flags.next().unwrap_or(true));
        }
    }
    let dropped = before - lines.len();
    join_soft_hyphens(lines, origins.as_deref_mut());
    dehyphenate(lines, origins);
    dropped
}

/// Whole words (lowercase letter runs) and hyphenated compounds (`a-b`, lowercase) used as the document's own dictionary.
fn word_sets(lines: &[String]) -> (HashSet<String>, HashSet<String>) {
    let mut words = HashSet::new();
    let mut compounds = HashSet::new();
    for line in lines {
        let mut prev_word: Option<&str> = None;
        let mut prev_end = 0;
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_alphabetic() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word = &line[start..i];
                // A word that ends the line with a hyphen is a break, not a whole word.
                let broken = i == line.trim_end().len() - 1 && line.trim_end().ends_with('-');
                if !broken {
                    words.insert(word.to_ascii_lowercase());
                }
                if let Some(prev) = prev_word {
                    if prev_end + 1 == start && bytes[prev_end] == b'-' {
                        compounds.insert(format!("{}-{}", prev.to_ascii_lowercase(), word.to_ascii_lowercase()));
                    }
                }
                prev_word = Some(word);
                prev_end = i;
            } else {
                i += 1;
            }
        }
    }
    (words, compounds)
}

/// A line ending `letters-` whose next line starts with lowercase letters is either a hyphenated compound
/// (`pre-` + `training`) or a word the typesetter broke (`understand-` + `ing`). Without a dictionary the two cannot
/// be told apart from the pair alone, so the document's own text decides: the break is repaired (`understanding` on
/// the first line, the fragment removed from the second) only when the joined word appears whole elsewhere in the
/// document and the hyphenated form appears nowhere. Lines on different pages (different origins) are never joined.
/// Line count and page citations stay as they were, except that a line left empty by the move is dropped.
fn dehyphenate(lines: &mut Vec<String>, mut origins: Option<&mut Vec<Option<usize>>>) {
    let breaks = |l: &String| {
        let t = l.trim_end();
        let b = t.as_bytes();
        b.len() >= 2 && b[b.len() - 1] == b'-' && b[b.len() - 2].is_ascii_alphabetic()
    };
    if !lines.iter().any(breaks) {
        return;
    }
    let (words, compounds) = word_sets(lines);
    let aligned = origins.as_deref().is_some_and(|o| o.len() == lines.len());
    let mut emptied: Vec<usize> = Vec::new();
    for i in 0..lines.len().saturating_sub(1) {
        if !breaks(&lines[i]) || (aligned && origins.as_deref().map(|o| o[i] != o[i + 1]).unwrap_or(false)) {
            continue;
        }
        let first = lines[i].trim_end();
        let before_hyphen = &first[..first.len() - 1];
        let head_len = before_hyphen.bytes().rev().take_while(u8::is_ascii_alphabetic).count();
        let head = &before_hyphen[before_hyphen.len() - head_len..];
        let next = lines[i + 1].trim_start();
        let frag_len = next.bytes().take_while(u8::is_ascii_lowercase).count();
        // A fragment that runs into a non-ASCII letter (`ü`) is not a plain ASCII word; leave it.
        if frag_len == 0 || next[frag_len..].chars().next().is_some_and(char::is_alphabetic) {
            continue;
        }
        let frag = &next[..frag_len];
        let joined = format!("{}{}", head.to_ascii_lowercase(), frag);
        if !words.contains(&joined) || compounds.contains(&format!("{}-{}", head.to_ascii_lowercase(), frag)) {
            continue;
        }
        // Closing punctuation stuck to the fragment (`ment.`) belongs to the word too.
        let moved_len = frag_len + next.bytes().skip(frag_len).take_while(|b| b".,;:)]!?".contains(b)).count();
        let new_first = format!("{}{}", before_hyphen, &next[..moved_len]);
        let rest = next[moved_len..].trim_start().to_string();
        lines[i] = new_first;
        if rest.is_empty() {
            emptied.push(i + 1);
        }
        lines[i + 1] = rest;
    }
    if !emptied.is_empty() {
        let mut idx = 0;
        lines.retain(|_| {
            let keep = !emptied.contains(&idx);
            idx += 1;
            keep
        });
        if aligned {
            if let Some(o) = origins.as_deref_mut() {
                let mut idx = 0;
                o.retain(|_| {
                    let keep = !emptied.contains(&idx);
                    idx += 1;
                    keep
                });
            }
        }
    }
}

/// A soft hyphen (U+00AD) at the end of a line marks a word the typesetter broke over two lines, so the two
/// halves are one word: `re` + soft hyphen, then `moved.`, becomes `removed.`. (An ordinary hyphen is left alone:
/// without a dictionary `high-` + `temperature` cannot be told from `optimiza-` + `tion`.) The joined line keeps
/// the origin of its first half.
fn join_soft_hyphens(lines: &mut Vec<String>, mut origins: Option<&mut Vec<Option<usize>>>) {
    const SOFT_HYPHEN: char = '\u{ad}';
    if !lines.iter().any(|l| l.trim_end().ends_with(SOFT_HYPHEN)) {
        return;
    }
    let aligned = origins.as_deref().is_some_and(|o| o.len() == lines.len());
    let mut joined: Vec<String> = Vec::with_capacity(lines.len());
    let mut joined_origins: Vec<Option<usize>> = Vec::new();
    let mut open = false;
    for (i, line) in lines.drain(..).enumerate() {
        if open {
            let last = joined.last_mut().expect("an open line");
            last.push_str(line.trim_start());
        } else {
            joined.push(line);
            if aligned {
                joined_origins.push(origins.as_deref().and_then(|o| o[i]));
            }
        }
        let last = joined.last_mut().expect("a line");
        open = last.trim_end().ends_with(SOFT_HYPHEN);
        if open {
            let kept = last.trim_end().trim_end_matches(SOFT_HYPHEN).len();
            last.truncate(kept);
        }
    }
    *lines = joined;
    if aligned {
        if let Some(origins) = origins.as_deref_mut() {
            *origins = joined_origins;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn kept(line: &str) -> bool {
        assess(line, Source::TextLayer).is_none()
    }

    #[test]
    fn engineering_text_that_looks_odd_is_kept() {
        for line in [
            "MIL-STD-1553",
            "AN3-5A",
            "AN960-10L washer",
            "0.125 in",
            "Ftu = 62 ksi",
            "Fcy = 53 ksi, Fsu = 37 ksi",
            "3/8-24 UNF-3A",
            "\u{3c3} = F/A (1 + 2e/d)",
            "4130N steel, 0.063 in wall",
            "t = P r / (S E - 0.6 P)",
            "Type II anodize per MIL-A-8625",
            "Kt = 3.14",
            "-65 to +250 F",
            "psf",
            "12 rpm",
            "kPa",
            "M8x1.25",
            "Table 3-2",
            "Fig. 4",
            "7075-T6",
            "e.g.,",
            "a.k.a.,",
            "ASTM A36 / A572 Gr 50",
            "see [12], pp. 3-7",
            "PDEs and GPUs",
            "d\u{e9}veloppement et \u{e9}t\u{e9}",
            "Gl\u{fc}cklich \u{fc}ber alles",
            "\u{4f60}\u{597d}",
            "\u{398} = 45\u{b0}",
            "\\def\\lheader{\\headertrue\\mainfont}",
            "if(buffer[1]=='x')break;",
            "https://doi.org/10.48550/arXiv.2304.12244",
            "\u{201c}water\u{201d}, \u{201c}tea\u{201d}",
            "\u{93}models,\u{94} and \u{97}more",
            "the \u{1}nancial \u{1}eld.",
        ] {
            assert!(kept(line), "false drop: {line:?} -> {:?}", assess(line, Source::TextLayer));
        }
    }

    #[test]
    fn gibberish_is_dropped() {
        for line in [
            "l1|~ . ,; Il",
            "Il",
            "|",
            "- - - -",
            "= &",
            "!\"#\"$%",
            "\u{c0}\u{c3}\u{c5}\u{c7}",
            "\u{ff}\u{ce}\u{4}i\u{b9}\u{f4}!f\u{dc}\u{e6}\u{ec}F",
            "\u{3}",
            "\u{fffd}\u{fffd}\u{fffd}",
            "mP",
            "qqqqqqqqqq##5#\"'&5476",
            "BSI\u{fe}\u{fe}\u{fe}\u{fe}\u{e4}\u{e4}\u{e4}",
            "",
            "   ",
            "...... ......",
        ] {
            assert!(!kept(line), "kept noise: {line:?}");
        }
        // OCR only: a lone letter is speckle, but in a text layer it is a label.
        assert!(assess("T", Source::Ocr).is_some());
        assert!(assess("T", Source::TextLayer).is_none());
        assert!(assess("l", Source::TextLayer).is_some());
        assert!(assess("x y z", Source::Ocr).is_some());
    }

    #[test]
    fn ligature_controls_and_cp1252_are_repaired() {
        assert_eq!(repair_encoding("\u{c}les and \u{c}elds"), "files and fields");
        assert_eq!(repair_encoding("sta\u{b}ng"), "staffng");
        assert_eq!(repair_encoding("\u{93}x\u{94} \u{96} \u{95}"), "\u{201c}x\u{201d} \u{2013} \u{2022}");
        // A control code standing alone is not a ligature.
        assert_eq!(repair_encoding("a \u{c} b"), "a \u{c} b");
        assert_eq!(repair_encoding("plain"), "plain");
    }

    #[test]
    fn soft_hyphenated_words_are_joined_across_lines_with_origins_kept() {
        let mut lines: Vec<String> = ["whether all corrosion has been re\u{ad}", "moved. Inspect", "the area", "for corro\u{ad}", " sion."]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut origins = vec![Some(10), Some(20), Some(30), Some(40), Some(50)];
        clean_lines(&mut lines, Source::TextLayer, Some(&mut origins));
        assert_eq!(lines, ["whether all corrosion has been removed. Inspect", "the area", "for corrosion."]);
        assert_eq!(origins, [Some(10), Some(30), Some(40)]);
        let mut plain: Vec<String> = vec!["a high-".into(), "temperature seal".into()];
        clean_lines(&mut plain, Source::TextLayer, None);
        assert_eq!(plain, ["a high-", "temperature seal"]);
    }

    #[test]
    fn ordinary_hyphen_breaks_are_repaired_only_when_the_document_confirms_the_word() {
        let mut lines: Vec<String> = [
            "Models of understand-",
            "ing and communicating in the field",
            "Good understanding helps. Pre-training is common,",
            "and pre-",
            "training data is large. A high-",
            "temperature seal. Next the de-",
            "veloped parts",
            "More developed parts",
            "Pairs end with perfor-",
            "mance. Performance is whole.",
            "The last line of a page ends with measure-",
            "ment on the next page and measurement is whole here",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut origins: Vec<Option<usize>> = vec![Some(1); 12];
        origins[11] = Some(2);
        clean_lines(&mut lines, Source::TextLayer, Some(&mut origins));
        assert_eq!(lines[0], "Models of understanding");
        assert_eq!(lines[1], "and communicating in the field");
        // "pre-training" is written with its hyphen elsewhere, "high-temperature" never as one word.
        assert_eq!(lines[3], "and pre-");
        assert_eq!(lines[4], "training data is large. A high-");
        assert_eq!(lines[5], "temperature seal. Next the developed");
        assert_eq!(lines[6], "parts");
        // Across a page boundary (different origins) nothing is joined.
        assert_eq!(&lines[8..10], ["Pairs end with performance.", "Performance is whole."]);
        assert_eq!(&lines[10..], ["The last line of a page ends with measure-", "ment on the next page and measurement is whole here"]);
        assert_eq!(origins.len(), lines.len());
    }

    #[test]
    fn clean_lines_keeps_origins_aligned_and_strips_controls() {
        let mut lines: Vec<String> = ["real text here", "|", "the \u{2}rst page", "\u{3}", "Ftu = 62 ksi"].iter().map(|s| s.to_string()).collect();
        let mut origins: Vec<Option<usize>> = vec![Some(10), Some(20), Some(30), Some(40), None];
        let dropped = clean_lines(&mut lines, Source::TextLayer, Some(&mut origins));
        assert_eq!(dropped, 2);
        assert_eq!(lines, ["real text here", "the rst page", "Ftu = 62 ksi"]);
        assert_eq!(origins, [Some(10), Some(30), None]);
    }

    #[test]
    fn a_document_of_only_lone_capitals_is_not_text() {
        let mut lines: Vec<String> = vec!["W".into(), "W".into(), "S".into()];
        assert_eq!(clean_lines(&mut lines, Source::TextLayer, None), 3);
        let mut lines: Vec<String> = vec!["W".into(), "Allowable stress".into()];
        assert_eq!(clean_lines(&mut lines, Source::TextLayer, None), 0);
    }

    fn unescape(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some('x') => {
                    let hex: String = chars.by_ref().take(2).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                }
                other => panic!("bad escape {other:?} in {s:?}"),
            }
        }
        out
    }

    /// Lines sampled from the real fixtures (`docs/search-ocr-line-quality.md`):
    /// every K must survive, every D must go, and the M lines are the noise the
    /// filter is known to keep (a count that may only shrink).
    #[test]
    fn labelled_real_lines() {
        let (mut keep_ok, mut keep_total, mut drop_ok, mut drop_total, mut miss_kept, mut miss_total) = (0, 0, 0, 0, 0, 0);
        let mut failures = Vec::new();
        for row in include_str!("../test-data/line-quality-labels.tsv").lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let mut parts = row.splitn(3, '\t');
            let (label, source, line) = (parts.next().unwrap(), parts.next().unwrap(), unescape(parts.next().unwrap()));
            let source = if source == "ocr" { Source::Ocr } else { Source::TextLayer };
            let keeps = assess(&line, source).is_none();
            match label {
                "K" => {
                    keep_total += 1;
                    if keeps { keep_ok += 1 } else { failures.push(format!("false drop {line:?}")) }
                }
                "D" => {
                    drop_total += 1;
                    if !keeps { drop_ok += 1 } else { failures.push(format!("kept noise {line:?}")) }
                }
                "M" => {
                    miss_total += 1;
                    if keeps { miss_kept += 1 }
                }
                other => panic!("bad label {other}"),
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!((keep_ok, drop_ok), (keep_total, drop_total));
        assert!(keep_total >= 150 && drop_total >= 60, "label set shrank");
        assert!(miss_kept <= miss_total && miss_total >= 15, "known-miss list changed: {miss_kept}/{miss_total}");
    }
}
