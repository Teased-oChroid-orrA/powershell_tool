//! Code-navigation probe (#240 P3-4, "evaluate need first"): how well does the current search answer "where is X
//! defined / what calls Y / which file does Z" on real code folders, and what would a symbol table add?
//! Paths: exact whole-word search with `relevance_order`, the semantic+keyword hybrid over a real `SemanticIndex`
//! (natural-language question and bare identifier), and a regex ctags-style symbol table (the minimal extractor).
//!   cargo run --release -p search-core --features semantic-search --example code_nav_probe -- \
//!     <projects dir> <scratch dir> [questions.tsv]
//! Corpora: sc = this repo's search-core/src, fc = fea-core/src, gm = geometry_mesh (no vendored SDK),
//! gs = the gmsh API example scripts (py, cpp, jl, c).
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::Local;
use regex::Regex;
use search_core::lexical::LexicalConfig;
use search_core::matching::{apply_line_matching, CompiledMatchState};
use search_core::models::{FileSearchResult, FileSearchStatus, SearchSettings};
use search_core::ordering::relevance_order;
use search_core::semantic_index::SemanticIndex;

const STOP: &[&str] = &["where", "which", "what", "the", "is", "are", "of", "in", "to", "a", "an", "for", "by", "does", "script", "file", "module", "calls"];

struct File {
    path: String,
    lines: Vec<String>,
}

fn walk(dir: &Path, exts: &[&str], skip: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !skip.contains(&name.as_str()) {
                walk(&p, exts, skip, out);
            }
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| exts.contains(&x)) && p.metadata().map_or(false, |m| m.len() < 1_500_000) {
            out.push(p);
        }
    }
}

fn load(root: &Path, exts: &[&str]) -> Vec<File> {
    let mut paths = Vec::new();
    walk(root, exts, &["target", ".git", ".gmsh-sdk-cache", "node_modules", ".eval-index"], &mut paths);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|p| {
            let t = std::fs::read_to_string(&p).ok()?;
            Some(File { path: p.to_string_lossy().into_owned(), lines: t.lines().map(String::from).collect() })
        })
        .collect()
}

/// ctags-style symbol table by regex: name -> (file index, 1-based line).
fn extract_symbols(files: &[File]) -> BTreeMap<String, Vec<(usize, usize)>> {
    let rs = Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?(?:async\s+)?(?:const\s+)?(?:fn|struct|enum|trait|type|mod|union|static)\s+(\w+)").unwrap();
    let rs_const = Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+(\w+)\s*:").unwrap();
    let rs_macro = Regex::new(r"^\s*macro_rules!\s+(\w+)").unwrap();
    let py = Regex::new(r"^\s*(?:async\s+)?(?:def|class)\s+(\w+)").unwrap();
    let jl = Regex::new(r"^\s*(?:function|struct|mutable struct|module)\s+(\w+)").unwrap();
    let cpp_type = Regex::new(r"^\s*(?:class|struct|namespace|enum)\s+(\w+)").unwrap();
    let cpp_fn = Regex::new(r"^[A-Za-z_][\w:<>\*&, ]*[\s\*&](\w+)\s*\([^;]*$").unwrap();
    let mut table: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (fi, f) in files.iter().enumerate() {
        let ext = Path::new(&f.path).extension().and_then(|x| x.to_str()).unwrap_or("");
        let pats: Vec<&Regex> = match ext {
            "rs" => vec![&rs, &rs_const, &rs_macro],
            "py" => vec![&py],
            "jl" => vec![&jl],
            _ => vec![&cpp_type, &cpp_fn],
        };
        for (li, line) in f.lines.iter().enumerate() {
            for p in &pats {
                if let Some(c) = p.captures(line) {
                    table.entry(c[1].to_string()).or_default().push((fi, li + 1));
                    break;
                }
            }
        }
    }
    table
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let projects = PathBuf::from(&args[1]);
    let scratch = PathBuf::from(&args[2]);
    let qfile = args.get(3).cloned().unwrap_or_else(|| "search-core/examples/code_nav_questions.tsv".into());
    let corpora: Vec<(&str, PathBuf, Vec<&str>)> = vec![
        ("sc", projects.join("powerShell/powershell_tool/search-core/src"), vec!["rs"]),
        ("fc", projects.join("powerShell/powershell_tool/fea-core/src"), vec!["rs"]),
        ("gm", projects.join("geometry_mesh"), vec!["rs"]),
        ("gs", projects.join("geometry_mesh/mesh_kernel/.gmsh-sdk-cache/gmsh-4.15.2-macos/share/doc/gmsh/examples/api"), vec!["py", "cpp", "jl", "c"]),
    ];
    let mut loaded: BTreeMap<&str, (Vec<File>, SemanticIndex, BTreeMap<String, Vec<(usize, usize)>>)> = BTreeMap::new();
    for (key, root, exts) in &corpora {
        let files = if *key == "gs" {
            // the example folder is under the vendored SDK cache, which `load` skips by name: walk it directly
            let mut paths = Vec::new();
            walk(root, exts, &[], &mut paths);
            paths.sort();
            paths.into_iter().filter_map(|p| Some(File { path: p.to_string_lossy().into_owned(), lines: std::fs::read_to_string(&p).ok()?.lines().map(String::from).collect() })).collect()
        } else {
            load(root, exts)
        };
        let bytes: usize = files.iter().map(|f| f.lines.iter().map(|l| l.len() + 1).sum::<usize>()).sum();
        let t = Instant::now();
        let table = extract_symbols(&files);
        let ex_ms = t.elapsed().as_secs_f64() * 1000.0;
        let dir = scratch.join(format!("idx-{key}"));
        std::fs::create_dir_all(&dir).unwrap();
        let index = SemanticIndex::open(&dir.join("semantic.sqlite")).unwrap();
        let t = Instant::now();
        let mut built = 0;
        for f in &files {
            let md = std::fs::metadata(&f.path).unwrap();
            let modified = search_core::cache::ticks_from_modified(chrono::DateTime::<Local>::from(md.modified().unwrap()));
            if index.is_current(&f.path, modified, md.len() as i64).unwrap_or(false) {
                continue;
            }
            index.index_document(&f.path, modified, md.len() as i64, &f.lines).unwrap();
            built += 1;
        }
        eprintln!(
            "{key}: {} files {:.2} MB, {} symbols; extractor {:.1} ms ({:.0} MB/s); embedded {built} files in {:.0}s",
            files.len(), bytes as f64 / 1e6, table.len(), ex_ms, bytes as f64 / 1e6 / (ex_ms / 1000.0), t.elapsed().as_secs_f64()
        );
        loaded.insert(key, (files, index, table));
    }

    println!("id\tcorpus\tkind\tgold_files\tex_rank\tex_hitfiles\tex_hitlines\tex_lines_to_gold\tex_prec\tex_r10\tsem_nl_rank\tsem_nl_r10\tsem_id_rank\tsem_id_r10\ttbl_found\ttbl_entries\ttbl_files\ttbl_gold_hit");
    let text = std::fs::read_to_string(&qfile).unwrap();
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let c: Vec<&str> = line.split('\t').collect();
        let (id, key, kind, question, exact_q, gold) = (c[0], c[1], c[2], c[3], c[4], c[5]);
        let (files, index, table) = &loaded[key];
        // gold files and gold lines
        let mut gold_files: BTreeSet<usize> = BTreeSet::new();
        let mut gold_lines: BTreeSet<(usize, usize)> = BTreeSet::new();
        if let Some(re) = gold.strip_prefix("def:") {
            let re = Regex::new(re).unwrap();
            for (fi, f) in files.iter().enumerate() {
                for (li, l) in f.lines.iter().enumerate() {
                    if re.is_match(l) {
                        gold_files.insert(fi);
                        gold_lines.insert((fi, li + 1));
                    }
                }
            }
        } else if let Some(re) = gold.strip_prefix("use:") {
            let re = Regex::new(re).unwrap();
            let def = Regex::new(&format!(r"\b(fn|def|function|struct|class)\s+{}\b", regex::escape(exact_q))).unwrap();
            for (fi, f) in files.iter().enumerate() {
                for (li, l) in f.lines.iter().enumerate() {
                    if re.is_match(l) && !def.is_match(l) {
                        gold_files.insert(fi);
                        gold_lines.insert((fi, li + 1));
                    }
                }
            }
        } else if let Some(sfx) = gold.strip_prefix("file:") {
            for (fi, f) in files.iter().enumerate() {
                if sfx.split(',').any(|s| f.path.ends_with(s)) {
                    gold_files.insert(fi);
                }
            }
        }
        if gold_files.is_empty() {
            println!("{id}\t{key}\t{kind}\tGOLD EMPTY");
            continue;
        }
        let ngold = gold_files.len();
        let recall10 = |ranked: &[usize]| ranked.iter().take(10).filter(|f| gold_files.contains(f)).count() as f64 / ngold.min(10) as f64;
        let rank_of = |ranked: &[usize]| ranked.iter().position(|f| gold_files.contains(f)).map(|r| r + 1);

        // exact
        let filters: Vec<String> = if kind == "concept" {
            exact_q.to_lowercase().split_whitespace().filter(|w| !STOP.contains(w)).map(String::from).collect()
        } else {
            vec![exact_q.to_string()]
        };
        let settings = SearchSettings { filters, whole_word: true,..Default::default() };
        let state = CompiledMatchState::build(&settings).unwrap();
        let now = Local::now();
        let results: Vec<FileSearchResult> = files
            .iter()
            .map(|f| {
                let out = apply_line_matching(&f.lines, &settings, &state);
                FileSearchResult {
                    full_name: f.path.clone(),
                    status: if out.hits.is_empty() { FileSearchStatus::NoHit } else { FileSearchStatus::Hit },
                    hits: out.hits,
                    created: now,
                    modified: now,
                    file_length: 0,
                    lines_cache: Vec::new(),
                    total_line_count: f.lines.len() as i32,
                    proximity_min_range: None,
                    low_confidence_pdf: false,
                    error_message: None,
                }
            })
            .collect();
        let order: Vec<usize> = relevance_order(&results, &settings).into_iter().filter(|&i| results[i].status == FileSearchStatus::Hit).collect();
        let hit_lines: usize = order.iter().map(|&i| results[i].hits.len()).sum();
        let ex_rank = rank_of(&order);
        // lines a reader scans before reaching the first gold line (concept: first hit of the gold file)
        let lines_to_gold = ex_rank.map(|r| {
            let before: usize = order[..r - 1].iter().map(|&i| results[i].hits.len()).sum();
            let gi = order[r - 1];
            let pos = results[gi]
                .hits
                .iter()
                .position(|h| gold_lines.is_empty() || gold_lines.contains(&(gi, h.line_number as usize)))
                .map_or(results[gi].hits.len(), |p| p + 1);
            before + pos
        });
        let prec = if order.is_empty() { 0.0 } else { order.iter().filter(|f| gold_files.contains(f)).count() as f64 / order.len() as f64 };

        // semantic hybrid: natural-language question, then bare identifier
        let by_path: BTreeMap<&str, usize> = files.iter().enumerate().map(|(i, f)| (f.path.as_str(), i)).collect();
        let sem = |q: &str| -> Vec<usize> {
            let mut ranked: Vec<usize> = Vec::new();
            for h in index.search_tuned(q, 300, true, &LexicalConfig::default()).unwrap_or_default() {
                if let Some(&i) = by_path.get(h.path.as_str()) {
                    if !ranked.contains(&i) {
                        ranked.push(i);
                    }
                }
            }
            ranked
        };
        let nl = sem(question);
        let idr = if kind == "concept" { nl.clone() } else { sem(exact_q) };

        // symbol table (answers definition questions only)
        let (tbl_found, tbl_entries, tbl_files, tbl_gold) = if kind == "def" {
            let e = table.get(exact_q).cloned().unwrap_or_default();
            let tf: BTreeSet<usize> = e.iter().map(|x| x.0).collect();
            let hit = gold_lines.iter().filter(|g| e.contains(g)).count();
            (
                (hit == gold_lines.len()).to_string(),
                e.len().to_string(),
                tf.len().to_string(),
                format!("{hit}/{}", gold_lines.len()),
            )
        } else {
            ("-".into(), "-".into(), "-".into(), "-".into())
        };
        let f = |r: Option<usize>| r.map_or("-".to_string(), |v| v.to_string());
        println!(
            "{id}\t{key}\t{kind}\t{ngold}\t{}\t{}\t{hit_lines}\t{}\t{prec:.2}\t{:.2}\t{}\t{:.2}\t{}\t{:.2}\t{tbl_found}\t{tbl_entries}\t{tbl_files}\t{tbl_gold}",
            f(ex_rank), order.len(), f(lines_to_gold), recall10(&order), f(rank_of(&nl)), recall10(&nl), f(rank_of(&idr)), recall10(&idr)
        );
    }
}
