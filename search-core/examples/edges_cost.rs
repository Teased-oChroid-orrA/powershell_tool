//! Cost of the catalog relationship pass at scale (#240 P3-1 / P2-1): generates N small files, then times
//! `sync_root`, `index_entities` and `rebuild_edges` (first build and an unchanged rebuild).
//!
//!   cargo run -p search-core --release --example edges_cost -- <work-dir> <files>
use std::time::Instant;

use search_core::catalog::Catalog;
use search_core::models::SearchSettings;
use search_core::native_index;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (work, n): (&str, usize) = (&args[0], args[1].parse().unwrap());
    let root = std::path::Path::new(work).join("tree");
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(&root).unwrap();
    let t = Instant::now();
    for i in 0..n {
        let dir = root.join(format!("d{:04}", i / 250));
        if i % 250 == 0 {
            std::fs::create_dir_all(&dir).unwrap();
        }
        // Every 20th file is a revision of the previous one; every 50th a verbatim copy; every 7th names a part number.
        let name = if i % 20 == 1 { format!("spec{}_v2.txt", i - 1) } else { format!("spec{i}.txt") };
        let body = if i % 50 == 49 { format!("shared body {}", (i / 50) * 50) } else { format!("document {i} about brackets and fasteners {}", if i % 7 == 0 { format!("PN-{}", i % 3000) } else { String::new() }) };
        std::fs::write(dir.join(name), body).unwrap();
    }
    eprintln!("generated {n} files in {:.1}s", t.elapsed().as_secs_f64());
    // The catalog reads the file list from the fast index, so build that first (not timed here).
    let mut exclude = Vec::new();
    native_index::ensure_index_folder_excluded(&mut exclude);
    let settings = SearchSettings {
        search_path: root.to_string_lossy().into_owned(),
        output_folder: root.to_string_lossy().into_owned(),
        exclude_folders: exclude,
        extensions: Some(vec![".txt".into()]),
        parallel: true,
        ..Default::default()
    };
    let idx_dir = native_index::index_directory(&root.to_string_lossy());
    native_index::ensure_index_directory_exists(&idx_dir).unwrap();
    {
        let engine = native_index::open_or_create_with_rebuild(&idx_dir).unwrap();
        let t = Instant::now();
        let o = native_index::build_or_update_corpus_index(&settings, &engine, &CancellationToken::new(), None).await.unwrap();
        eprintln!("fast index built in {:.1}s: indexed {} docs {} failed {}", t.elapsed().as_secs_f64(), o.indexed_count, o.index_docs, o.failed_count);
    }
    let mut c = Catalog::open(&std::path::Path::new(work).join("catalog.sqlite")).unwrap();
    let r = root.to_string_lossy().to_string();
    c.register_root(&r, &idx_dir).unwrap();
    let t = Instant::now();
    let files = c.sync_root(&r).unwrap();
    println!("sync_root\t{files} files\t{:.2}s", t.elapsed().as_secs_f64());
    let t = Instant::now();
    let ents = c.index_entities(&r, None).unwrap();
    println!("index_entities\t{ents} files\t{:.2}s", t.elapsed().as_secs_f64());
    for label in ["rebuild_edges first", "rebuild_edges unchanged"] {
        let t = Instant::now();
        let s = c.rebuild_edges().unwrap();
        println!("{label}\t{s:?}\t{:.2}s", t.elapsed().as_secs_f64());
    }
}
