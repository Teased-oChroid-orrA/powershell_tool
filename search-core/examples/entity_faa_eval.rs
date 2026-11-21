//! Part-number discovery on FAA Airworthiness Directives (public domain, no labels shipped).
//!
//!   cargo run -p search-core --release --features neural-search --example entity_faa_eval -- \
//!       <faa-ad dir> [--sample N] [--seed S] [--misses N] [--raw]
//!
//! The files are Federal Register HTML pages (`<pre>` body, anchors, a Cloudflare script tail). By default the page
//! wrapper is stripped (text inside `<pre>`, tags removed, `&#160;` and the XML entities decoded) so the extractor sees
//! what a reader sees; `--raw` keeps the file as downloaded.
//!
//! There is no public labelled set for part numbers, so the gold here is *cue-labelled*: a token is gold when it sits in
//! an explicit part-number list ("P/N 200181 or 704A34112012", "part numbers (P/N) A, B, and C"). The labeller works on
//! the whole file (cues and lists may wrap across lines) and shares no pattern with `entities.rs`, but it is one
//! author's rule, not an annotator's judgement, and it only labels cued part numbers. Two numbers follow:
//!
//! * Recall of the extractor on gold, split into the token right after the cue and the later tokens of a list.
//! * Precision proxy: of the extractor's PartNumber candidates, how many overlap gold. A candidate outside gold is
//!   *unknown*, not wrong (an uncued part number is real). `--sample N` prints N random unknown candidates with their
//!   context so a person can judge them; the reviewed counts are recorded in `docs/knowledge-engine-audit.md`.
use regex::Regex;
use search_core::entities::{extract_candidates, EntityKind};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

/// Words that join items of a list.
const JOINERS: &str = r"(?:\s*,\s*(?:and|or)\b|\s*,|\s+and/or\b|\s+through\b|\s+or\b|\s+and\b|\s*;)";

/// Gold spans of one file: `(byte range, is the first token after its cue)`.
fn gold_spans(text: &str, cue: &Regex, token: &Regex, joiner: &Regex) -> Vec<(Range<usize>, bool)> {
    let mut out: Vec<(Range<usize>, bool)> = Vec::new();
    for cue_match in cue.find_iter(text) {
        let mut at = cue_match.end();
        let mut first = true;
        while let Some(found) = token.find_at(text, at).filter(|m| m.start() == at) {
            let mut start_ok = true;
            // A token must carry a digit (rules out "the", "number") and must not be a date-like AD number.
            if !found.as_str().bytes().any(|b| b.is_ascii_digit()) {
                start_ok = false;
            }
            if !start_ok {
                break;
            }
            if !out.iter().any(|(range, _)| range == &(found.start()..found.end())) {
                out.push((found.start()..found.end(), first));
            }
            first = false;
            let Some(join) = joiner.find_at(text, found.end()).filter(|m| m.start() == found.end()) else {
                break;
            };
            // Skip whitespace after the joiner.
            at = join.end() + text[join.end()..].len() - text[join.end()..].trim_start().len();
        }
    }
    out
}

/// The readable text of one Federal Register page: the `<pre>` body without tags.
fn clean(raw: &str, tags: &Regex) -> String {
    let body = raw.split_once("<pre>").map_or(raw, |(_, rest)| rest.split("</pre>").next().unwrap_or(rest));
    tags.replace_all(body, "")
        .replace("&#160;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Small deterministic generator, so a sample is reproducible without a dependency.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn context(text: &str, range: &Range<usize>) -> String {
    let mut from = range.start.saturating_sub(70);
    while !text.is_char_boundary(from) {
        from -= 1;
    }
    let mut to = (range.end + 50).min(text.len());
    while !text.is_char_boundary(to) {
        to += 1;
    }
    format!("{}[[{}]]{}", &text[from..range.start], &text[range.clone()], &text[range.end..to]).replace('\n', " ")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.first().expect("usage: entity_faa_eval <faa-ad dir> [--sample N] [--seed S]");
    let flag = |name: &str| args.windows(2).find(|w| w[0] == name).and_then(|w| w[1].parse::<u64>().ok());
    let sample = flag("--sample").unwrap_or(0) as usize;
    let misses = flag("--misses").unwrap_or(0) as usize;
    let mut missed_first: Vec<(String, String)> = Vec::new();
    let mut rng = flag("--seed").unwrap_or(1).max(1);

    let cue = Regex::new(
        r"(?i)\b(?:P\s*/\s*Ns?|PNs?|part\s+(?:numbers?|nos?\.?)(?:\s*\(\s*P\s*/\s*Ns?\s*\))?)\s*[:#]?\s*",
    )
    .unwrap();
    // One token: letters, digits and . / - joined; a hyphen at a line end continues on the next line.
    let token = Regex::new(r"[A-Za-z0-9](?:[A-Za-z0-9./]|-\s*)*[A-Za-z0-9]|[A-Za-z0-9]").unwrap();
    let joiner = Regex::new(JOINERS).unwrap();
    let tags = Regex::new(r"<[^>]*>").unwrap();
    let raw_mode = args.iter().any(|a| a == "--raw");

    let mut files: Vec<_> = std::fs::read_dir(Path::new(dir))
        .expect("faa-ad dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .collect();
    files.sort();

    let (mut gold_total, mut gold_first, mut gold_rest) = (0usize, 0usize, 0usize);
    let (mut found_first, mut found_rest, mut exact_first, mut exact_rest) = (0usize, 0usize, 0usize, 0usize);
    let (mut cands, mut cands_gold) = (0usize, 0usize);
    let mut unknown: Vec<(String, Range<usize>, String)> = Vec::new();
    let mut unknown_text: BTreeMap<String, usize> = BTreeMap::new();
    let mut missed: BTreeMap<String, usize> = BTreeMap::new();
    let mut files_with_gold = 0usize;
    let mut capped_files = 0usize;

    for path in &files {
        let text = std::fs::read_to_string(path).expect("read");
        let text = if raw_mode { text } else { clean(&text, &tags) };
        let gold = gold_spans(&text, &cue, &token, &joiner);
        if !gold.is_empty() {
            files_with_gold += 1;
        }
        let extraction = extract_candidates(&text);
        if extraction.coverage.candidate_limit_reached {
            capped_files += 1;
        }
        let part: Vec<Range<usize>> = extraction
            .candidates
            .into_iter()
            .filter(|c| c.shape_hints.contains(&EntityKind::PartNumber))
            .map(|c| c.bytes)
            .collect();
        for (span, first) in &gold {
            gold_total += 1;
            let hit = part.iter().find(|c| overlaps(c, span));
            if *first {
                gold_first += 1;
            } else {
                gold_rest += 1;
            }
            match hit {
                Some(c) => {
                    let exact = c == span;
                    match (first, exact) {
                        (true, true) => (found_first, exact_first) = (found_first + 1, exact_first + 1),
                        (true, false) => found_first += 1,
                        (false, true) => (found_rest, exact_rest) = (found_rest + 1, exact_rest + 1),
                        (false, false) => found_rest += 1,
                    }
                }
                None => {
                    *missed.entry(text[span.clone()].replace('\n', " ")).or_default() += 1;
                    if *first {
                        let name = path.file_name().unwrap().to_string_lossy().to_string();
                        missed_first.push((name, context(&text, span)));
                    }
                }
            }
        }
        for c in &part {
            cands += 1;
            if gold.iter().any(|(g, _)| overlaps(g, c)) {
                cands_gold += 1;
            } else {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                unknown.push((name, c.clone(), context(&text, c)));
                *unknown_text.entry(text[c.clone()].to_string()).or_default() += 1;
            }
        }
    }

    let pct = |a: usize, b: usize| if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 };
    println!("files {} (with gold {}), gold spans {gold_total}", files.len(), files_with_gold);
    println!("files that hit the candidate cap (later text unscanned): {capped_files}");
    println!("recall   token after cue : gold {gold_first:5} found {found_first:5} ({:.1}%) exact {exact_first:5} ({:.1}%)", pct(found_first, gold_first), pct(exact_first, gold_first));
    println!("recall   later list token: gold {gold_rest:5} found {found_rest:5} ({:.1}%) exact {exact_rest:5} ({:.1}%)", pct(found_rest, gold_rest), pct(exact_rest, gold_rest));
    println!("candidates with a PartNumber hint {cands}: overlap gold {cands_gold} ({:.1}%), unknown {}", pct(cands_gold, cands), cands - cands_gold);

    let mut top: Vec<_> = unknown_text.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("\nmost frequent unknown candidates (text, count, distinct {}):", top.len());
    for (t, n) in top.iter().take(25) {
        println!("  {n:4}  {t}");
    }
    let mut top: Vec<_> = missed.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    println!("\nmissed gold tokens (text, count, distinct {}):", top.len());
    for (t, n) in top.iter().take(25) {
        println!("  {n:4}  {t}");
    }

    if misses > 0 {
        println!("\nmissed first tokens: {} (showing {})", missed_first.len(), misses.min(missed_first.len()));
        let step = (missed_first.len() / misses.max(1)).max(1);
        for (file, ctx) in missed_first.iter().step_by(step).take(misses) {
            println!("  {file}\t{ctx}");
        }
    }
    if sample > 0 && !unknown.is_empty() {
        println!("\nrandom unknown candidates:");
        let mut picked = std::collections::BTreeSet::new();
        while picked.len() < sample.min(unknown.len()) {
            picked.insert((next(&mut rng) % unknown.len() as u64) as usize);
        }
        for i in picked {
            let (file, _, ctx) = &unknown[i];
            println!("#{i}\t{file}\t{ctx}");
        }
    }
}
