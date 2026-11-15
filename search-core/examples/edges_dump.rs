//! Dumps every catalog edge of a real folder as TSV (src, dst, type, confidence, class) after building a fast
//! index, catalog, entities and edges for `<folder>` in `<work-dir>`; `scripts/score_faa_ad_edges.py` scores it.
//!
//!   cargo run -p search-core --release --features neural-search --example edges_dump -- <work-dir> <folder> [.ext ...]
use search_core::catalog::Catalog;
use search_core::models::SearchSettings;
use search_core::native_index;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (work, root) = (&args[0], std::fs::canonicalize(&args[1]).unwrap().to_string_lossy().into_owned());
    let extensions: Vec<String> = args[2..].to_vec();
    let _ = std::fs::remove_dir_all(work);
    std::fs::create_dir_all(work).unwrap();
    let mut exclude = Vec::new();
    native_index::ensure_index_folder_excluded(&mut exclude);
    let settings = SearchSettings {
        search_path: root.clone(),
        output_folder: work.clone(),
        exclude_folders: exclude,
        extensions: (!extensions.is_empty()).then_some(extensions),
        parallel: true,
        ..Default::default()
    };
    let idx_dir = std::path::Path::new(work).join("index");
    native_index::ensure_index_directory_exists(&idx_dir).unwrap();
    {
        let engine = native_index::open_or_create_with_rebuild(&idx_dir).unwrap();
        native_index::build_or_update_corpus_index(&settings, &engine, &CancellationToken::new(), None).await.unwrap();
    }
    let mut c = Catalog::open(&std::path::Path::new(work).join("catalog.sqlite")).unwrap();
    c.register_root(&root, &idx_dir).unwrap();
    let files = c.sync_root(&root).unwrap();
    c.index_entities(&root, None).unwrap();
    let edges = c.rebuild_edges().unwrap();
    println!("{files} files, {} edges", edges.total);
    for f in c.files().unwrap() {
        for e in c.edges_for(&f.path).unwrap() {
            println!("{}\t{}\t{}\t{:.3}\t{}", e.src.replace(&root, "."), e.dst.replace(&root, "."), e.kind, e.confidence, e.class);
        }
    }
}
