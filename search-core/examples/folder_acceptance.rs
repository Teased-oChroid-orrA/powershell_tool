//! Acceptance check of Search on a real folder (#240): indexes `<folder>` through the product path (exact index plus
//! semantic index), asks every question in `<questions.tsv>` of the semantic index exactly as the Search window does,
//! and reports file-level hit@1/3/10 and MRR@10 for answerable questions, plus how well the best passage's similarity
//! separates answerable from unanswerable questions (the evidence an abstention threshold would need).
//!
//! Questions file: `query<TAB>file names separated by |` (a file counts when its path ends with the name), or `-`
//! when the folder holds no answer. Lines starting with `#` are ignored. Build your own from real questions about your
//! folder; `data/faa_ad_questions.tsv` is a machine-authored seed set over public FAA directives, not human judged.
//!
//!   cargo run -p search-core --release --features semantic-search --example folder_acceptance -- \
//!       <work-dir> <folder> <questions.tsv> [.ext ...]
use search_core::models::SearchSettings;
use search_core::native_index;
use search_core::semantic_index::SemanticIndex;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (work, root, questions) = (&args[0], std::fs::canonicalize(&args[1]).unwrap().to_string_lossy().into_owned(), &args[2]);
    let extensions: Vec<String> = args[3..].to_vec();
    std::fs::create_dir_all(work).unwrap();
    let idx_dir = std::path::Path::new(work).join("index");
    native_index::ensure_index_directory_exists(&idx_dir).unwrap();
    let mut exclude = Vec::new();
    native_index::ensure_index_folder_excluded(&mut exclude);
    let settings = SearchSettings {
        search_path: root.clone(),
        output_folder: work.clone(),
        exclude_folders: exclude,
        extensions: (!extensions.is_empty()).then_some(extensions),
        parallel: true,
        semantic_index: true,
        ..Default::default()
    };
    let t = std::time::Instant::now();
    {
        let engine = native_index::open_or_create_with_rebuild(&idx_dir).unwrap();
        let out = native_index::build_or_update_corpus_index(&settings, &engine, &CancellationToken::new(), None).await.unwrap();
        eprintln!("indexed in {:.0} s ({} embedded)", t.elapsed().as_secs_f64(), out.semantic_embedded_count);
    }
    let index = SemanticIndex::open(&idx_dir.join("semantic.sqlite")).unwrap();

    let (mut n, mut h1, mut h3, mut h10, mut mrr) = (0usize, 0usize, 0usize, 0usize, 0f64);
    let (mut answerable_top, mut unanswerable_top): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
    for line in std::fs::read_to_string(questions).unwrap().lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (query, gold) = line.split_once('\t').expect("query<TAB>gold");
        let hits = index.search_with_settings(query, 50, &settings).unwrap_or_default();
        // File ranking: a file's rank is the rank of its best passage.
        let mut files: Vec<(&str, f32)> = Vec::new();
        for hit in &hits {
            if !files.iter().any(|(p, _)| *p == hit.path) {
                files.push((hit.path.as_str(), hit.similarity));
            }
        }
        let top = files.first().map_or(0.0, |f| f.1);
        if gold.trim() == "-" {
            unanswerable_top.push(top);
            println!("NONE\ttop {:.3}\t{}\t{}", top, files.first().map_or("", |f| short(f.0)), query);
            continue;
        }
        let wanted: Vec<&str> = gold.split('|').map(str::trim).filter(|g| !g.is_empty()).collect();
        let rank = files.iter().position(|(p, _)| wanted.iter().any(|g| p.ends_with(g)));
        n += 1;
        answerable_top.push(top);
        if rank == Some(0) {
            h1 += 1;
        }
        if rank.is_some_and(|r| r < 3) {
            h3 += 1;
        }
        if rank.is_some_and(|r| r < 10) {
            h10 += 1;
        }
        if let Some(r) = rank.filter(|r| *r < 10) {
            mrr += 1.0 / (r + 1) as f64;
        }
        println!("{}\ttop {:.3}\t{}\t{}", rank.map_or("MISS".to_string(), |r| format!("rank {}", r + 1)), top, files.first().map_or("", |f| short(f.0)), query);
    }
    let f = |x: usize| x as f64 / n.max(1) as f64;
    println!("\nanswerable {n}: hit@1 {:.3}  hit@3 {:.3}  hit@10 {:.3}  MRR@10 {:.3}", f(h1), f(h3), f(h10), mrr / n.max(1) as f64);
    if !unanswerable_top.is_empty() {
        // AUC: chance that an answerable question's best passage scores above an unanswerable one's.
        let mut wins = 0.0;
        for a in &answerable_top {
            for u in &unanswerable_top {
                wins += if a > u { 1.0 } else if a == u { 0.5 } else { 0.0 };
            }
        }
        let auc = wins / (answerable_top.len() * unanswerable_top.len()) as f64;
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        println!(
            "unanswerable {}: mean top similarity {:.3} (answerable {:.3}); AUC separating them {:.3}",
            unanswerable_top.len(),
            mean(&unanswerable_top),
            mean(&answerable_top),
            auc
        );
    }
}

fn short(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}
