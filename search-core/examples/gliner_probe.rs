//! Does GLiNER load and run on rten, and how fast? `gliner_probe <model dir> <text file or text> [--bench N]`
//! Prints load time, the spans of the first window and, with `--bench N`, the best and median wall time of N runs
//! over the whole text (best-of filters out contention from other processes).
#[path = "support/gliner.rs"]
mod gliner;
use std::time::Instant;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let text = std::fs::read_to_string(&a[1]).unwrap_or_else(|_| a[1].clone());
    let runs = a.windows(2).find(|w| w[0] == "--bench").and_then(|w| w[1].parse().ok()).unwrap_or(1usize);
    let t = Instant::now();
    let g = gliner::Gliner::load(std::path::Path::new(&a[0]));
    println!("load {:?}", t.elapsed());
    let labels = ["part number", "fastener callout", "standard", "material", "dimension"];
    let words = text.split_whitespace().count();
    let mut times = Vec::new();
    let mut spans = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        spans = g.extract(&text, &labels, 0.3);
        times.push(t.elapsed().as_secs_f64());
    }
    times.sort_by(|a, b| a.total_cmp(b));
    println!("{words} words: best {:.0} ms, median {:.0} ms, best {:.1} ms per 100 words", times[0] * 1e3, times[times.len() / 2] * 1e3, times[0] * 1e5 / words as f64);
    for s in spans.iter().take(8) {
        println!("  {:?} {} {:.2}", &text[s.bytes.clone()], labels[s.label], s.score);
    }
}
