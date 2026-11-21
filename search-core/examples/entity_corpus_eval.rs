//! Entity category ranking on two public, expert-labelled corpora (aeroBERT-NER, SOFC-Exp).
//!
//!   cargo run -p search-core --release --features neural-search --example entity_corpus_eval -- \
//!       <aerobert-ner dir> <sofc-exp-corpus dir> [--misses N]
//!
//! Neither corpus uses our six categories, so each gold label is mapped to one kind (`MAPPING` below) and a gold span
//! counts as *found* when an extracted candidate overlaps it. For the found spans the table compares four rankers: the
//! first pattern hint, any pattern hint (the ceiling for the constrained ranker), raw MiniLM top-one, and MiniLM top-one
//! restricted to the pattern hints (what the Search window shows). Candidates outside gold spans are not scored: both
//! corpora label only some categories, so a candidate without a gold span is not a known false positive.
#[path = "support/gliner.rs"]
mod gliner;
use search_core::entities::{extract_entities, EntityCandidate, EntityKind};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;
use tokio_util::sync::CancellationToken;

/// (dataset, gold label, kind it is scored as). VAL / VALUE are quantities with a unit ("400 feet", "700 °C"): the
/// nearest of our kinds is `Dimension`. RES is a regulatory reference ("Section 25-341"): the nearest is `Standard`.
const MAPPING: &[(&str, &str, EntityKind)] = &[
    ("aeroBERT", "VAL", EntityKind::Dimension),
    ("aeroBERT", "RES", EntityKind::Standard),
    ("SOFC-Exp", "VALUE", EntityKind::Dimension),
    ("SOFC-Exp", "MATERIAL", EntityKind::Material),
];

struct Gold {
    label: String,
    /// Byte range into the sentence.
    bytes: Range<usize>,
}

struct Sentence {
    text: String,
    gold: Vec<Gold>,
}

#[derive(Default)]
struct Tally {
    gold: usize,
    found: usize,
    exact: usize,
    first_hint: usize,
    any_hint: usize,
    raw_top1: usize,
    constrained: usize,
    /// GLiNER (`--gliner`): any span overlapping the gold, a span with the mapped label, and the exact gold span.
    gl_any: usize,
    gl_label: usize,
    gl_exact: usize,
}

fn aerobert(dir: &Path) -> Vec<Sentence> {
    let raw = std::fs::read_to_string(dir.join("all_text_annotation_NER.txt")).expect("aeroBERT file");
    let mut out: Vec<Sentence> = Vec::new();
    let mut current = String::new();
    let mut open: Option<(String, usize)> = None;
    let mut gold: Vec<Gold> = Vec::new();
    let mut last_id = String::new();
    let flush = |out: &mut Vec<Sentence>, text: &mut String, gold: &mut Vec<Gold>, open: &mut Option<(String, usize)>| {
        if let Some((label, start)) = open.take() {
            gold.push(Gold { label, bytes: start..text.len() });
        }
        if !text.is_empty() {
            out.push(Sentence { text: std::mem::take(text), gold: std::mem::take(gold) });
        }
    };
    for line in raw.lines().skip(1) {
        let parts: Vec<&str> = line.split('*').collect();
        if parts.len() < 3 {
            continue;
        }
        let (id, tag) = (parts[0], parts[parts.len() - 1]);
        let word = parts[1..parts.len() - 1].join("*");
        if id != last_id {
            flush(&mut out, &mut current, &mut gold, &mut open);
            last_id = id.to_string();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        let start = current.len();
        current.push_str(&word);
        match tag.split_once('-') {
            Some(("B", label)) => {
                if let Some((l, s)) = open.take() {
                    gold.push(Gold { label: l, bytes: s..start - 1 });
                }
                open = Some((label.to_string(), start));
            }
            Some(("I", _)) if open.is_some() => {}
            _ => {
                if let Some((l, s)) = open.take() {
                    gold.push(Gold { label: l, bytes: s..start - 1 });
                }
            }
        }
    }
    flush(&mut out, &mut current, &mut gold, &mut open);
    out
}

fn sofc(dir: &Path) -> Vec<Sentence> {
    let mut out = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(dir.join("texts")).expect("texts dir").flatten().collect();
    names.sort_by_key(|e| e.file_name());
    for entry in names {
        let stem = entry.path().file_stem().unwrap().to_string_lossy().to_string();
        let text: Vec<char> = std::fs::read_to_string(entry.path()).expect("text").chars().collect();
        let read = |sub: &str| std::fs::read_to_string(dir.join("annotations").join(sub).join(format!("{stem}.csv"))).unwrap_or_default();
        let sentences = read("sentences");
        let tokens = read("entity_types_and_slots");
        // Entity spans per sentence number: runs of B-X / I-X tokens, as character offsets from the start of the sentence.
        let mut spans: BTreeMap<usize, Vec<(String, usize, usize)>> = BTreeMap::new();
        for line in tokens.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 6 {
                continue;
            }
            let (Ok(sent), Ok(start), Ok(end)) = (f[0].parse::<usize>(), f[2].parse::<usize>(), f[3].parse::<usize>()) else { continue };
            let list = spans.entry(sent).or_default();
            match f[4].split_once('-') {
                Some(("B", label)) => list.push((label.to_string(), start, end)),
                Some(("I", label)) => {
                    if let Some(last) = list.last_mut().filter(|l| l.0 == label) {
                        last.2 = end;
                    }
                }
                _ => {}
            }
        }
        for line in sentences.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 4 || f[1] != "1" {
                continue; // entity types are only fully annotated on flagged sentences
            }
            let (Ok(sent), Ok(start), Ok(end)) = (f[0].parse::<usize>(), f[2].parse::<usize>(), f[3].parse::<usize>()) else { continue };
            if end > text.len() || start >= end {
                continue;
            }
            let s: String = text[start..end].iter().collect();
            // Token offsets in the annotation files count characters from the start of their sentence.
            let chars = s.chars().count();
            let to_byte = |char_off: usize| -> usize { s.char_indices().nth(char_off).map_or(s.len(), |(b, _)| b) };
            let gold = spans
                .get(&sent)
                .map(|v| {
                    v.iter()
                        .filter(|(_, a, b)| *b <= chars && a < b)
                        .map(|(label, a, b)| Gold { label: label.clone(), bytes: to_byte(*a)..to_byte(*b) })
                        .collect()
                })
                .unwrap_or_default();
            out.push(Sentence { text: s, gold });
        }
    }
    out
}

fn overlap(a: &Range<usize>, b: &Range<usize>) -> usize {
    a.end.min(b.end).saturating_sub(a.start.max(b.start))
}

/// Zero-shot labels offered to GLiNER; `gliner_label` maps a scored kind to its index.
const GLINER_LABELS: [&str; 5] = ["part number", "fastener callout", "standard", "material", "dimension"];

fn gliner_label(kind: EntityKind) -> usize {
    match kind {
        EntityKind::Standard => 2,
        EntityKind::Material => 3,
        _ => 4,
    }
}

struct GlinerRun<'a> {
    model: &'a gliner::Gliner,
    threshold: f32,
    sentences: std::cell::Cell<usize>,
    time: std::cell::Cell<std::time::Duration>,
}

fn evaluate(name: &str, sentences: &[Sentence], misses: usize, gl: Option<&GlinerRun>) {
    let cancel = CancellationToken::new();
    let mut tallies: BTreeMap<(&str, EntityKind), Tally> = BTreeMap::new();
    let mut shown = 0;
    for sentence in sentences {
        if !sentence.gold.iter().any(|g| MAPPING.iter().any(|m| m.0 == name && m.1 == g.label)) {
            continue;
        }
        let extraction = extract_entities(&sentence.text, &cancel).expect("entity extraction");
        let spans = gl.map(|g| {
            let t = std::time::Instant::now();
            let spans = g.model.extract(&sentence.text, &GLINER_LABELS, g.threshold);
            g.time.set(g.time.get() + t.elapsed());
            g.sentences.set(g.sentences.get() + 1);
            spans
        });
        for gold in &sentence.gold {
            let Some(&(_, label, kind)) = MAPPING.iter().find(|m| m.0 == name && m.1 == gold.label) else { continue };
            let tally = tallies.entry((label, kind)).or_default();
            tally.gold += 1;
            if let Some(spans) = &spans {
                let hits: Vec<_> = spans.iter().filter(|sp| overlap(&sp.bytes, &gold.bytes) > 0).collect();
                tally.gl_any += usize::from(!hits.is_empty());
                tally.gl_label += usize::from(hits.iter().any(|sp| sp.label == gliner_label(kind)));
                tally.gl_exact += usize::from(hits.iter().any(|sp| sp.label == gliner_label(kind) && sp.bytes == gold.bytes));
            }
            let best: Option<&EntityCandidate> = extraction
                .candidates
                .iter()
                .filter(|c| overlap(&c.bytes, &gold.bytes) > 0)
                .max_by_key(|c| (overlap(&c.bytes, &gold.bytes), c.bytes.len()));
            let Some(candidate) = best else {
                if shown < misses {
                    shown += 1;
                    println!("miss {name}/{label} -> {kind:?}: {:?} in {:?}", &sentence.text[gold.bytes.clone()], truncate(&sentence.text));
                }
                continue;
            };
            tally.found += 1;
            tally.exact += usize::from(candidate.bytes == gold.bytes);
            tally.first_hint += usize::from(candidate.shape_hints.first() == Some(&kind));
            tally.any_hint += usize::from(candidate.shape_hints.contains(&kind));
            tally.raw_top1 += usize::from(candidate.category_scores.first().map(|s| s.kind) == Some(kind));
            tally.constrained += usize::from(candidate.neural_suggestion().map(|s| s.kind) == Some(kind));
        }
    }
    for ((label, kind), t) in &tallies {
        println!(
            "{name:9} {label:9} as {kind:?}: gold={} found={} exact_span={} | of found: first_hint={} any_hint={} raw_minilm={} minilm_in_hints={}",
            t.gold, t.found, t.exact, t.first_hint, t.any_hint, t.raw_top1, t.constrained
        );
        if gl.is_some() {
            println!("{:30}GLiNER: found_any_label={} found_right_label={} exact_span_right_label={}", "", t.gl_any, t.gl_label, t.gl_exact);
        }
    }
}

fn truncate(text: &str) -> String {
    text.chars().take(110).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: entity_corpus_eval <aerobert-ner dir> <sofc-exp-corpus dir> [--misses N]");
        std::process::exit(2);
    }
    let misses = args.iter().position(|a| a == "--misses").and_then(|i| args.get(i + 1)).and_then(|n| n.parse().ok()).unwrap_or(0);
    let aero = aerobert(Path::new(&args[0]));
    let fuel = sofc(Path::new(&args[1]));
    println!("aeroBERT sentences {}, SOFC-Exp flagged sentences {}", aero.len(), fuel.len());
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1));
    let run = flag("--gliner").map(|dir| GlinerRun {
        model: Box::leak(Box::new(gliner::Gliner::load(Path::new(dir)))),
        threshold: flag("--threshold").and_then(|t| t.parse().ok()).unwrap_or(0.5),
        sentences: Default::default(),
        time: Default::default(),
    });
    for (name, sentences) in [("aeroBERT", &aero), ("SOFC-Exp", &fuel)] {
        evaluate(name, sentences, misses, run.as_ref());
        if let Some(g) = &run {
            let n = g.sentences.take().max(1);
            println!("{name}: GLiNER threshold {} over {n} sentences, {:.1} ms per sentence", g.threshold, g.time.take().as_secs_f64() * 1000.0 / n as f64);
        }
    }
}
