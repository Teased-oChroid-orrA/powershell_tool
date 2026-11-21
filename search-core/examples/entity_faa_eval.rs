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
#[path = "support/gliner.rs"]
mod gliner;
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
    let units_mode = args.iter().any(|a| a == "--units");
    // Dimension candidates by unit word: (count, first context).
    let mut by_unit: BTreeMap<String, (usize, String)> = BTreeMap::new();
    // `--gliner <model dir> [--threshold T] [--gliner-files N]`: run GLiNER with the label "part number" (and four others,
    // as in entity_corpus_eval) on N evenly spaced files and score it next to the pattern extractor on the same files.
    let gl_model = args.windows(2).find(|w| w[0] == "--gliner").map(|w| gliner::Gliner::load(Path::new(&w[1])));
    let gl_threshold = args.windows(2).find(|w| w[0] == "--threshold").and_then(|w| w[1].parse::<f32>().ok()).unwrap_or(0.5);
    let gl_files = flag("--gliner-files").unwrap_or(40) as usize;
    let labels = ["part number", "fastener callout", "standard", "material", "dimension"];
    // (gold first, found first, gold rest, found rest) for GLiNER and the pattern extractor on the GLiNER files.
    let (mut gl_gold, mut gl_hit_first, mut gl_hit_rest, mut gl_gold_first, mut gl_gold_rest) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut pat_hit_first, mut pat_hit_rest) = (0usize, 0usize);
    let (mut gl_cands, mut gl_cands_gold, mut gl_docs, mut gl_words) = (0usize, 0usize, 0usize, 0usize);
    let mut gl_time = std::time::Duration::ZERO;
    let mut gl_unknown: Vec<(String, String, f32)> = Vec::new();
    let mut gl_by_label = [0usize; 5];

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

    for (file_index, path) in files.iter().enumerate() {
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
        if let Some(model) = gl_model.as_ref().filter(|_| file_index % (files.len() / gl_files.max(1)).max(1) == 0 && gl_docs < gl_files) {
            gl_docs += 1;
            gl_words += text.split_whitespace().count();
            let t = std::time::Instant::now();
            let spans = model.extract(&text, &labels, gl_threshold);
            gl_time += t.elapsed();
            let pattern: Vec<Range<usize>> = extraction.candidates.iter().filter(|c| c.shape_hints.contains(&EntityKind::PartNumber)).map(|c| c.bytes.clone()).collect();
            let parts: Vec<&gliner::Span> = spans.iter().filter(|sp| sp.label == 0).collect();
            for sp in &spans {
                gl_by_label[sp.label] += 1;
            }
            for (g, first) in &gold {
                gl_gold += 1;
                let hit = parts.iter().any(|sp| overlaps(&sp.bytes, g));
                let pat = pattern.iter().any(|c| overlaps(c, g));
                if *first {
                    (gl_gold_first, gl_hit_first, pat_hit_first) = (gl_gold_first + 1, gl_hit_first + usize::from(hit), pat_hit_first + usize::from(pat));
                } else {
                    (gl_gold_rest, gl_hit_rest, pat_hit_rest) = (gl_gold_rest + 1, gl_hit_rest + usize::from(hit), pat_hit_rest + usize::from(pat));
                }
            }
            for sp in &parts {
                gl_cands += 1;
                if gold.iter().any(|(g, _)| overlaps(g, &sp.bytes)) {
                    gl_cands_gold += 1;
                } else {
                    gl_unknown.push((path.file_name().unwrap().to_string_lossy().to_string(), context(&text, &sp.bytes), sp.score));
                }
            }
        }
        if units_mode {
            for c in extraction.candidates.iter().filter(|c| c.shape_hints.contains(&EntityKind::Dimension)) {
                let unit = text[c.bytes.clone()]
                    .trim_start_matches(|ch: char| !ch.is_alphabetic() && ch != '°')
                    .trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '.' || ch == '/' || ch == ' ')
                    .to_lowercase();
                let entry = by_unit.entry(unit).or_insert_with(|| (0, context(&text, &c.bytes)));
                entry.0 += 1;
            }
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

    if gl_model.is_some() {
        println!("\nGLiNER threshold {gl_threshold} on {gl_docs} evenly spaced files ({gl_words} words): {:.1} s total, {:.2} s per file, {:.2} ms per 100 words",
            gl_time.as_secs_f64(), gl_time.as_secs_f64() / gl_docs.max(1) as f64, gl_time.as_secs_f64() * 1e5 / gl_words.max(1) as f64);
        println!("  gold on these files {gl_gold}: token after cue {gl_gold_first}, later list tokens {gl_gold_rest}");
        println!("  GLiNER 'part number' recall: after cue {gl_hit_first}/{gl_gold_first} ({:.1}%), later {gl_hit_rest}/{gl_gold_rest} ({:.1}%)", pct(gl_hit_first, gl_gold_first), pct(gl_hit_rest, gl_gold_rest));
        println!("  pattern extractor, same files: after cue {pat_hit_first}/{gl_gold_first} ({:.1}%), later {pat_hit_rest}/{gl_gold_rest} ({:.1}%)", pct(pat_hit_first, gl_gold_first), pct(pat_hit_rest, gl_gold_rest));
        println!("  GLiNER 'part number' spans {gl_cands}: overlap gold {gl_cands_gold} ({:.1}%), unknown {}", pct(gl_cands_gold, gl_cands), gl_cands - gl_cands_gold);
        println!("  GLiNER spans by label {:?}: {:?}", labels, gl_by_label);
        if sample > 0 && !gl_unknown.is_empty() {
            println!("\nrandom GLiNER 'part number' spans outside gold (to read):");
            let mut picked = std::collections::BTreeSet::new();
            while picked.len() < sample.min(gl_unknown.len()) {
                picked.insert((next(&mut rng) % gl_unknown.len() as u64) as usize);
            }
            for i in picked {
                let (file, ctx, score) = &gl_unknown[i];
                println!("#{i}\t{score:.2}\t{file}\t{ctx}");
            }
        }
    }
    if units_mode {
        let mut rows: Vec<_> = by_unit.iter().collect();
        rows.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(b.0)));
        println!("\nDimension candidates by unit text ({} distinct):", rows.len());
        for (unit, (n, ctx)) in rows.iter().take(40) {
            println!("  {n:5}  {unit:12} {ctx}");
        }
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
