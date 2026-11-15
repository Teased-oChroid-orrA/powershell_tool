//! End to end on a folder of real PDFs with human-judged pages (ViDoRe v3 Industrial source PDFs): every PDF is read
//! by the product's extraction (scanner or yolowpdf), indexed with its page numbers through `index_document_with_pages`,
//! and each judged English query is searched with the default hybrid ranking. Reports document-level and page-level
//! recall, hit rate and nDCG@10 (binary relevance), so extraction and page mapping errors count against the result.
//!
//!   python3 scripts/prepare_vidore_folder.py <dir>
//!   cargo run -p search-core --release --features semantic-search,pdf-yolow --example pdf_folder_eval -- <work-dir> <dir> <scanner|yolow> [max-pdfs]
use std::collections::HashSet;
use std::num::NonZeroU32;
use std::path::Path;
use std::time::Instant;

use search_core::lexical::LexicalConfig;
use search_core::semantic_index::SemanticIndex;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (work, dir, backend) = (&a[1], &a[2], a[3].as_str());
    let max: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    std::fs::create_dir_all(work).unwrap();
    let db = Path::new(work).join(format!("folder-{backend}.sqlite"));
    let _ = std::fs::remove_file(&db);
    let index = SemanticIndex::open(&db).unwrap();
    let mut pdfs: Vec<_> = std::fs::read_dir(Path::new(dir).join("pdfs")).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "pdf")).collect();
    pdfs.sort();
    pdfs.truncate(max);
    let (mut lines_total, mut chunks_total, mut no_text) = (0usize, 0usize, Vec::new());
    let t0 = Instant::now();
    for p in &pdfs {
        let bytes = std::fs::read(p).unwrap();
        let t = Instant::now();
        let Some(cited) = search_core::extraction::eval_pdf_cited_lines(&bytes, backend == "yolow") else {
            no_text.push(p.file_name().unwrap().to_string_lossy().into_owned());
            continue;
        };
        let (lines, pages): (Vec<String>, Vec<Option<NonZeroU32>>) = cited.into_iter().map(|(l, pg)| (l, pg.and_then(NonZeroU32::new))).unzip();
        let extract = t.elapsed().as_secs_f64();
        lines_total += lines.len();
        let path = p.to_string_lossy().into_owned();
        // The index serves only files that still match their indexed mtime and size, so store the real ones.
        let modified = search_core::cache::ticks_from_modified(chrono::DateTime::<chrono::Utc>::from(std::fs::metadata(p).unwrap().modified().unwrap()).with_timezone(&chrono::Local));
        chunks_total += index.index_document_with_pages(&path, modified, bytes.len() as i64, &lines, Some(&pages)).unwrap();
        eprintln!("{} {} lines {:.1}s extract, {:.0}s elapsed", p.file_name().unwrap().to_string_lossy(), lines.len(), extract, t0.elapsed().as_secs_f64());
    }
    println!("{backend}: {} pdfs, {lines_total} lines, {chunks_total} chunks, no text: {no_text:?}, {:.0}s", pdfs.len(), t0.elapsed().as_secs_f64());
    let queries: Vec<serde_json::Value> = std::fs::read_to_string(Path::new(dir).join("folder_queries.jsonl")).unwrap().lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let present: HashSet<String> = pdfs.iter().map(|p| p.file_stem().unwrap().to_string_lossy().into_owned()).collect();
    let config = LexicalConfig::default();
    let (mut n, mut dh1, mut dh5, mut ph1, mut ph10, mut pr100, mut ndcg) = (0usize, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for q in &queries {
        let rel: HashSet<(String, u32)> = q["pages"].as_array().unwrap().iter().map(|p| (p[0].as_str().unwrap().to_string(), p[1].as_u64().unwrap() as u32)).filter(|(d, _)| present.contains(d)).collect();
        if rel.is_empty() {
            continue;
        }
        let rel_docs: HashSet<&String> = rel.iter().map(|(d, _)| d).collect();
        let hits = index.search_tuned(q["text"].as_str().unwrap(), 100, true, &config).unwrap();
        let doc_of =|h: &search_core::semantic_index::SemanticHit| Path::new(&h.path).file_stem().unwrap().to_string_lossy().into_owned();
        let mut docs: Vec<String> = Vec::new();
        for h in &hits {
            let d = doc_of(h);
            if !docs.contains(&d) {
                docs.push(d);
            }
        }
        let hit_at = |k: usize| docs.iter().take(k).any(|d| rel_docs.contains(d)) as u8 as f64;
        let page_rel: Vec<bool> = hits.iter().map(|h| h.page.is_some_and(|pg| rel.contains(&(doc_of(h), pg.get())))).collect();
        // distinct relevant pages found, in rank order, for recall and nDCG
        let mut seen = HashSet::new();
        let gains: Vec<f64> = hits.iter().zip(&page_rel).map(|(h, ok)| if *ok && seen.insert((doc_of(h), h.page.unwrap().get())) { 1.0 } else { 0.0 }).collect();
        let dcg: f64 = gains.iter().take(10).enumerate().map(|(i, g)| g / ((i + 2) as f64).log2()).sum();
        let ideal: f64 = (0..rel.len().min(10)).map(|i| 1.0 / ((i + 2) as f64).log2()).sum();
        n += 1;
        dh1 += hit_at(1);
        dh5 += hit_at(5);
        ph1 += page_rel.iter().take(1).any(|x| *x) as u8 as f64;
        ph10 += page_rel.iter().take(10).any(|x| *x) as u8 as f64;
        pr100 += gains.iter().sum::<f64>() / rel.len() as f64;
        ndcg += dcg / ideal;
    }
    let f = |x: f64| x / n as f64;
    println!("queries {n}\tdoc hit@1 {:.3}\tdoc hit@5 {:.3}\tpage hit@1 {:.3}\tpage hit@10 {:.3}\tpage nDCG@10 {:.3}\tpage recall@100 {:.3}", f(dh1), f(dh5), f(ph1), f(ph10), f(ndcg), f(pr100));
}
