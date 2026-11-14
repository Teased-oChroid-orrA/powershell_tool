//! Page-citation accuracy of PDF extraction backends against poppler's per-page text.
//!
//! usage: pdf_citation_bench <scanner|yolow> <pdf>...   (needs `pdftotext` on PATH)
//! Prints one TSV row per file: lines, cited, correct, anywhere. A line is "checkable" when
//! its first 5 words occur on some oracle page; "correct" when the cited page holds them.
use std::collections::HashSet;
use std::process::Command;

fn toks(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let yolow = a[1] == "yolow";
    let (mut tl, mut tc, mut tk, mut tr) = (0, 0, 0, 0);
    println!("file\tlines\tcited\tcheckable\tcorrect");
    for path in &a[2..] {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Some(lines) = search_core::extraction::eval_pdf_cited_lines(&bytes, yolow) else {
            println!("{path}\t-\t-\t-\t-");
            continue;
        };
        let Ok(out) = Command::new("pdftotext").args(["-enc", "UTF-8", path, "-"]).output() else { continue };
        let text = String::from_utf8_lossy(&out.stdout);
        // Per page: the set of 5-word windows.
        let pages: Vec<HashSet<String>> = text
            .split('\x0c')
            .map(|p| {
                let t = toks(p);
                t.windows(5).map(|w| w.join(" ")).collect()
            })
            .collect();
        let (mut cited, mut check, mut ok) = (0, 0, 0);
        for (line, page) in &lines {
            let t = toks(line);
            if t.len() < 5 {
                continue;
            }
            let key = t[..5].join(" ");
            if page.is_some() {
                cited += 1;
            }
            if pages.iter().any(|p| p.contains(&key)) {
                check += 1;
                if let Some(n) = page {
                    if pages.get(*n as usize - 1).is_some_and(|p| p.contains(&key)) {
                        ok += 1;
                    }
                }
            }
        }
        println!("{path}\t{}\t{cited}\t{check}\t{ok}", lines.len());
        tl += lines.len(); tc += cited; tk += check; tr += ok;
    }
    println!("TOTAL\t{tl}\t{tc}\t{tk}\t{tr}\tcorrect/checkable={:.4}", tr as f64 / tk.max(1) as f64);
}
