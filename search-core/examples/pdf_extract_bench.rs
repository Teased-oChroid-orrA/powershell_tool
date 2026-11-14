//! Times `extract_pdf_lines` per PDF and reports progress-callback behaviour. Usage:
//! cargo run --release -p search-core [--features ocr] --example pdf_extract_bench -- [--ocr] [--timeout=N] [--cache=DIR] file.pdf...
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ocr = args.iter().any(|a| a == "--ocr");
    let timeout = args.iter().find_map(|a| a.strip_prefix("--timeout=")).and_then(|v| v.parse().ok()).unwrap_or(600u64);
    search_core::extraction::configure_ocr_cache(args.iter().find_map(|a| a.strip_prefix("--cache=")));
    for path in args.iter().filter(|a| !a.starts_with("--")) {
        let bytes = std::fs::read(path).unwrap();
        let t = Instant::now();
        let (mut calls, mut first, mut longest_gap, mut last) = (0u32, None::<Duration>, Duration::ZERO, Duration::ZERO);
        let mut progress = |_n: i32, at: Duration| {
            calls += 1;
            first.get_or_insert(at);
            longest_gap = longest_gap.max(at - last);
            last = at;
        };
        let (lines, truncated) = search_core::extraction::extract_pdf_lines(&bytes, timeout, Some(&mut progress), ocr);
        let dt = t.elapsed().as_secs_f64();
        let n = lines.as_ref().map_or(0, |l| l.len());
        let text_bytes: usize = lines.as_ref().map_or(0, |l| l.iter().map(|s| s.len()).sum());
        println!(
            "{path}: {:.1} MB, {dt:.3}s, {n} lines, {text_bytes} UTF-8 bytes, truncated={truncated}, progress calls={calls}, first at {:.2}s, longest gap {:.2}s",
            bytes.len() as f64 / 1e6,
            first.map_or(0.0, |d| d.as_secs_f64()),
            longest_gap.as_secs_f64()
        );
    }
}
