//! Edge tests against a real catalog: invalidation, idempotence, evidence on every edge, and the
//! labelled precision/recall set (#240 P3-1; numbers in docs/knowledge-engine-audit.md).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use native_search::engine::DocumentInput;
use rusqlite::params;
use sha2::{Digest, Sha256};

use super::*;
use crate::native_index;

/// Files on disk, indexed by the fast index, registered in a catalog.
struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    index_dir: PathBuf,
    catalog: Catalog,
    /// Per-path change counter, used as the indexed mtime (bumped by every write).
    modified: std::cell::RefCell<BTreeMap<String, i64>>,
}

fn text_of(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
    crate::extraction::extract_lines_by_extension(&ext, &bytes, 30, None, false).map(|d| d.lines.join("\n")).unwrap_or_default()
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let index_dir = tmp.path().join("index");
        std::fs::create_dir_all(&root).unwrap();
        native_index::ensure_index_directory_exists(&index_dir).unwrap();
        let catalog = Catalog::open_in_memory().unwrap();
        catalog.register_root(root.to_str().unwrap(), &index_dir).unwrap();
        Env { _tmp: tmp, root, index_dir, catalog, modified: Default::default() }
    }

    fn path(&self, rel: &str) -> String {
        self.root.join(rel).to_string_lossy().into_owned()
    }

    fn write(&self, rel: &str, body: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        *self.modified.borrow_mut().entry(p.to_string_lossy().into_owned()).or_insert(0) += 1;
        std::fs::write(p, body).unwrap();
    }

    fn copy_in(&self, from: &Path, rel: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        *self.modified.borrow_mut().entry(p.to_string_lossy().into_owned()).or_insert(0) += 1;
        std::fs::copy(from, p).unwrap();
    }

    /// Indexes every file under the root (stale entries for deleted files are removed), syncs the
    /// catalog, stores the content hash (SHA-256 of the extracted text; the real flow takes it from the
    /// semantic index) and extracts entities.
    fn refresh(&mut self) {
        let engine = native_index::open_or_create_with_rebuild(&self.index_dir).unwrap();
        let mut on_disk = BTreeSet::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    on_disk.insert(p.to_string_lossy().into_owned());
                }
            }
        }
        for known in engine.all_document_metadata().unwrap().keys() {
            if !on_disk.contains(known) {
                engine.delete_document(known).unwrap();
            }
        }
        for path in &on_disk {
            let size = std::fs::metadata(path).unwrap().len() as i64;
            engine
                .index_document(DocumentInput {
                    id: path,
                    path,
                    filename: path,
                    extension: ".txt",
                    title: "",
                    modified_unix: self.modified.borrow()[path],
                    created_unix: 0,
                    size,
                    body: "x",
                })
                .unwrap();
        }
        engine.commit().unwrap();
        drop(engine);
        let root = self.root.to_string_lossy().into_owned();
        self.catalog.sync_root(&root).unwrap();
        for path in &on_disk {
            let hash = Sha256::digest(text_of(Path::new(path)).as_bytes()).to_vec();
            self.catalog.conn.execute("UPDATE files SET content_hash=?2 WHERE path=?1", params![path, hash]).unwrap();
        }
        self.catalog.index_entities(&root, None).unwrap();
    }

    fn edges(&self) -> Vec<StoredEdge> {
        let mut stmt = self
            .catalog
            .conn
            .prepare("SELECT src,dst,type,confidence,evidence_json,class,created,source_version FROM relationship ORDER BY type,src,dst")
            .unwrap();
        stmt.query_map([], |r| {
            Ok(StoredEdge {
                src: r.get(0)?,
                dst: r.get(1)?,
                kind: r.get(2)?,
                confidence: r.get(3)?,
                evidence: serde_json::from_str::<Value>(&r.get::<_, String>(4)?).unwrap(),
                class: r.get(5)?,
                created: r.get(6)?,
                source_version: r.get(7)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }
}

fn words(seed: u64, count: usize) -> String {
    const SYL: [&str; 16] = ["ka", "to", "mi", "re", "su", "no", "la", "pe", "di", "vo", "ru", "ne", "sa", "gi", "bo", "zu"];
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut out = Vec::new();
    for _ in 0..count {
        let mut w = String::new();
        for _ in 0..3 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            w.push_str(SYL[(x >> 33) as usize % 16]);
        }
        out.push(w);
    }
    out.join(" ")
}

fn body(seed: u64) -> String {
    (0..25).map(|l| words(seed * 1000 + l, 8)).collect::<Vec<_>>().join("\n")
}

fn changed(text: &str, salt: u64) -> String {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    for i in [3usize, 11, 19] {
        lines[i] = words(900_000 + salt * 100 + i as u64, 8);
    }
    lines.join("\n")
}

#[test]
fn rebuild_is_idempotent_and_every_edge_has_evidence_and_class() {
    let mut env = Env::new();
    env.write("a/spec.txt", "Part number AX-21-7 here\nsome text");
    env.write("a/spec_v2.txt", "Part number AX-21-7 here\nsome other text");
    env.write("b/same.txt", "Part number AX-21-7 here\nsome text");
    env.refresh();
    let first = env.catalog.rebuild_edges_at(100).unwrap();
    assert!(first.added > 0 && first.updated == 0 && first.removed == 0);
    let before = env.edges();
    assert_eq!(before.len(), first.total);
    let second = env.catalog.rebuild_edges_at(200).unwrap();
    assert_eq!((second.added, second.updated, second.removed), (0, 0, 0));
    assert_eq!(env.edges(), before, "second rebuild changes nothing, `created` included");

    let kinds: BTreeSet<&str> = before.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, BTreeSet::from(["DUPLICATE_OF", "FILE_IN_FOLDER", "SAME_ENTITY", "VERSION_OF"]));
    for e in &before {
        assert!(e.evidence.as_object().is_some_and(|o| !o.is_empty()), "no edge without evidence: {e:?}");
        assert!(["metadata", "verified", "statistical"].contains(&e.class.as_str()));
    }
    // The table itself refuses an edge without evidence or with an unknown class.
    let bad = |evidence: &str, class: &str| {
        env.catalog.conn.execute(
            "INSERT INTO relationship VALUES('x','y','T',1.0,?1,?2,0,'v')",
            params![evidence, class],
        )
    };
    assert!(bad("", "metadata").is_err());
    assert!(bad("{}", "metadata").is_err());
    assert!(bad("{\"a\":1}", "guess").is_err());
    assert!(bad("{\"a\":1}", "verified").is_ok());
}

#[test]
fn facts_exclude_statistical_edges_and_statistical_edges_are_thresholded() {
    let mut env = Env::new();
    env.write("a/one.txt", "ref AX-21-7 and BX-30-1\nfiller");
    env.write("b/two.txt", "ref AX-21-7 only\nfiller");
    env.write("c/three.txt", "ref BX-30-1 too\nfiller");
    env.write("d/four.txt", "ref BX-30-1 again\nfiller");
    env.refresh();
    env.catalog.rebuild_edges().unwrap();
    let one = env.path("a/one.txt");
    assert!(env.catalog.facts_for(&one).unwrap().iter().all(|e| e.class != "statistical"));
    assert_eq!(env.catalog.facts_for(&one).unwrap().len(), 1, "only FILE_IN_FOLDER");
    let all = env.catalog.statistical_for(&one, 0.0).unwrap();
    assert_eq!(all.len(), 3, "AX-21-7 with two.txt, BX-30-1 with three.txt and four.txt");
    let strong = env.catalog.statistical_for(&one, 0.9).unwrap();
    assert_eq!(strong.len(), 1);
    assert_eq!(strong[0].dst, env.path("b/two.txt"));
    assert_eq!(strong[0].evidence["shared"][0]["value"], "AX-21-7");
    assert_eq!(env.catalog.edges_for(&one).unwrap().len(), 4);
}

#[test]
fn edges_are_invalidated_when_a_file_changes_or_disappears() {
    let mut env = Env::new();
    let base = body(1);
    env.write("a/doc.txt", &format!("{base}\nPart AX-21-7"));
    env.write("b/doc-copy.txt", &format!("{base}\nPart AX-21-7"));
    env.write("a/doc_v2.txt", &format!("{base}\nPart AX-21-7\nmore"));
    env.refresh();
    env.catalog.rebuild_edges_at(1).unwrap();
    let (a, b, v2) = (env.path("a/doc.txt"), env.path("b/doc-copy.txt"), env.path("a/doc_v2.txt"));
    let has = |env: &Env, kind: &str, x: &str, y: &str| {
        env.edges().iter().any(|e| e.kind == kind && ((e.src == x && e.dst == y) || (e.src == y && e.dst == x)))
    };
    assert!(has(&env, "DUPLICATE_OF", &a, &b) && has(&env, "VERSION_OF", &a, &v2) && has(&env, "SAME_ENTITY", &a, &b));
    let folder_version = |env: &Env| env.edges().into_iter().find(|e| e.kind == "FILE_IN_FOLDER" && e.src == b).unwrap();
    let old = folder_version(&env);

    // doc-copy.txt changes on disk: no longer a duplicate and no longer shares the part number.
    env.write("b/doc-copy.txt", &format!("{}\nPart ZZ-99-9", body(2)));
    env.refresh();
    let stats = env.catalog.rebuild_edges_at(2).unwrap();
    assert!(!has(&env, "DUPLICATE_OF", &a, &b), "{stats:?}");
    assert!(!has(&env, "SAME_ENTITY", &a, &b));
    assert!(has(&env, "VERSION_OF", &a, &v2), "an unrelated file's edge is untouched");
    let new = folder_version(&env);
    assert_ne!(new.source_version, old.source_version, "the stamp follows the content hash");
    assert_eq!(new.created, 2, "a rewritten edge restarts `created`");
    let kept = env.edges().into_iter().find(|e| e.kind == "VERSION_OF").unwrap();
    assert_eq!(kept.created, 1, "an unchanged edge keeps `created`");

    // doc_v2.txt is deleted: every edge that touched it goes.
    std::fs::remove_file(env.path("a/doc_v2.txt")).unwrap();
    env.refresh();
    env.catalog.rebuild_edges_at(3).unwrap();
    assert!(env.edges().iter().all(|e| e.src != v2 && e.dst != v2 && e.kind != "VERSION_OF"));
}

/// Unordered pairs `(a, b)` with a < b, from a set of file names.
fn pairs(of: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    of.iter().map(|(a, b)| if a <= b { (a.to_string(), b.to_string()) } else { (b.to_string(), a.to_string()) }).collect()
}

/// Union-find closure: every pair inside each connected group.
fn closure(edges: &BTreeSet<(String, String)>) -> BTreeSet<(String, String)> {
    let mut group: BTreeMap<String, usize> = BTreeMap::new();
    let mut members: Vec<BTreeSet<String>> = Vec::new();
    for (a, b) in edges {
        let (ga, gb) = (group.get(a).copied(), group.get(b).copied());
        let target = match (ga, gb) {
            (Some(x), Some(y)) if x != y => {
                let moved = std::mem::take(&mut members[y]);
                for m in &moved {
                    group.insert(m.clone(), x);
                }
                members[x].extend(moved);
                x
            }
            (Some(x), _) | (_, Some(x)) => x,
            _ => {
                members.push(BTreeSet::new());
                members.len() - 1
            }
        };
        for n in [a, b] {
            group.insert(n.clone(), target);
            members[target].insert(n.clone());
        }
    }
    let mut out = BTreeSet::new();
    for set in &members {
        let v: Vec<&String> = set.iter().collect();
        for i in 0..v.len() {
            for j in i + 1..v.len() {
                out.insert((v[i].clone(), v[j].clone()));
            }
        }
    }
    out
}

fn report(name: &str, got: &BTreeSet<(String, String)>, truth: &BTreeSet<(String, String)>) -> (usize, usize, usize) {
    let tp = got.intersection(truth).count();
    println!(
        "{name:<14} edges {:>4}  truth {:>4}  correct {:>4}  precision {:>6.3}  recall {:>6.3}",
        got.len(),
        truth.len(),
        tp,
        if got.is_empty() { 1.0 } else { tp as f64 / got.len() as f64 },
        if truth.is_empty() { 1.0 } else { tp as f64 / truth.len() as f64 },
    );
    (got.len(), truth.len(), tp)
}

/// Labelled set built by construction (see the audit log for the design and the numbers).
#[test]
fn precision_and_recall_on_a_labelled_set() {
    let mut env = Env::new();
    let mut dup: Vec<(String, String)> = Vec::new();
    let mut version: Vec<(String, String)> = Vec::new();
    let mut entity: Vec<(String, String)> = Vec::new();
    let rel = |env: &Env, r: &str| env.path(r);

    // 30 unrelated base documents.
    let stems: Vec<String> = (0..30).map(|i| format!("{}{i:02}", &words(500 + i, 1)[..4])).collect();
    for (i, stem) in stems.iter().enumerate() {
        env.write(&format!("projA/{stem}.txt"), &body(i as u64 + 1));
    }
    // Copies under other names in another folder (renames), one real Office file included.
    for i in 0..10 {
        env.write(&format!("projB/renamed-{}.txt", stems[i]), &body(i as u64 + 1));
        dup.push((rel(&env, &format!("projA/{}.txt", stems[i])), rel(&env, &format!("projB/renamed-{}.txt", stems[i]))));
    }
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data");
    env.copy_in(&data.join("test.docx"), "projA/test.docx");
    env.copy_in(&data.join("test.docx"), "projB/handbook-final-copy.docx");
    dup.push((rel(&env, "projA/test.docx"), rel(&env, "projB/handbook-final-copy.docx")));
    for f in ["test.pdf", "test.pptx", "test.xlsx", "test.zip"] {
        env.copy_in(&data.join(f), &format!("projA/{f}"));
    }
    // Same-folder copies: both a duplicate and a version by name.
    for i in 10..13 {
        env.write(&format!("projA/{} - Copy.txt", stems[i]), &body(i as u64 + 1));
        let (a, b) = (rel(&env, &format!("projA/{}.txt", stems[i])), rel(&env, &format!("projA/{} - Copy.txt", stems[i])));
        dup.push((a.clone(), b.clone()));
        version.push((a, b));
    }
    // Real versions: edited content under a version token; three-file chains for 13..16.
    for i in 13..23 {
        let tokens: &[&str] = match i {
            13..=15 => &["_v2", "_v3"],
            16..=19 => &[" rev B"],
            _ => &["-2026-02-01"],
        };
        let mut chain = vec![rel(&env, &format!("projA/{}.txt", stems[i]))];
        for (n, t) in tokens.iter().enumerate() {
            env.write(&format!("projA/{}{t}.txt", stems[i]), &changed(&body(i as u64 + 1), n as u64 + 1));
            chain.push(rel(&env, &format!("projA/{}{t}.txt", stems[i])));
        }
        for x in 0..chain.len() {
            for y in x + 1..chain.len() {
                version.push((chain[x].clone(), chain[y].clone()));
            }
        }
    }
    // Versions kept in an archive folder: related by construction, outside the same-folder rule.
    for i in 23..26 {
        env.write(&format!("archive/{}_final.txt", stems[i]), &changed(&body(i as u64 + 1), 1));
        version.push((rel(&env, &format!("projA/{}.txt", stems[i])), rel(&env, &format!("archive/{}_final.txt", stems[i]))));
    }
    // Decoys: same stem plus a token but unrelated content (a limit of name matching); and numbered
    // names without a version token (no edge expected).
    let mut decoy_name_pairs = Vec::new();
    for k in 0..3 {
        env.write(&format!("notes/memo{k}.txt"), &body(700 + k));
        env.write(&format!("notes/memo{k}_v2.txt"), &body(800 + k));
        decoy_name_pairs.push((rel(&env, &format!("notes/memo{k}.txt")), rel(&env, &format!("notes/memo{k}_v2.txt"))));
        env.write(&format!("tables/table_{k}.txt"), &body(900 + k));
    }
    env.write("notes/invoice1_v2.txt", &body(950));
    env.write("notes/report9_v2.txt", &body(951));
    // Entities: 10 identifiers in two files, 3 in three files, a near-miss pair, and one common standard.
    let host = |env: &Env, name: &str, ids: &[&str], seed: u64| {
        let lines: Vec<String> = ids.iter().map(|id| format!("see P/N {id} for details")).collect();
        env.write(name, &format!("{}\n{}", lines.join("\n"), body(seed)));
    };
    for k in 0..10u64 {
        let id = format!("KR-{}-{}", 40 + k, 7 + k);
        host(&env, &format!("ent/two-{k}a.txt"), &[&id, "ASTM A36"], 2000 + k * 2);
        host(&env, &format!("ent/two-{k}b.txt"), &[&id, "ASTM A36"], 2001 + k * 2);
        entity.push((rel(&env, &format!("ent/two-{k}a.txt")), rel(&env, &format!("ent/two-{k}b.txt"))));
    }
    for k in 0..3u64 {
        let id = format!("MZ-{}-{}", 70 + k, 3 + k);
        let names: Vec<String> = (0..3).map(|j| format!("ent/three-{k}{j}.txt")).collect();
        for (j, n) in names.iter().enumerate() {
            host(&env, n, &[&id, "ASTM A36"], 3000 + k * 3 + j as u64);
        }
        for x in 0..3 {
            for y in x + 1..3 {
                entity.push((rel(&env, &names[x]), rel(&env, &names[y])));
            }
        }
    }
    host(&env, "ent/near-a.txt", &["BX-21-7"], 4000);
    host(&env, "ent/near-b.txt", &["BX-21-8"], 4001);
    for j in 0..30u64 {
        host(&env, &format!("ent/hub-{j:02}.txt"), &["ASTM A36"], 5000 + j);
    }

    env.refresh();
    env.catalog.rebuild_edges().unwrap();

    let files = env.edges().iter().filter(|e| e.kind == "FILE_IN_FOLDER").count();
    let total_files: i64 = env.catalog.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap();
    assert_eq!(files as i64, total_files, "FILE_IN_FOLDER: one edge per file");

    let by = |kind: &str| -> BTreeSet<(String, String)> {
        env.edges().into_iter().filter(|e| e.kind == kind).map(|e| if e.src <= e.dst { (e.src, e.dst) } else { (e.dst, e.src) }).collect()
    };
    let dup_truth = pairs(&dup.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect::<Vec<_>>());
    let ver_truth = pairs(&version.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect::<Vec<_>>());
    let ent_truth = pairs(&entity.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect::<Vec<_>>());

    println!("-- labelled set: {total_files} files, {} edges", env.edges().len());
    println!("false duplicates: {:?}", closure(&by("DUPLICATE_OF")).difference(&dup_truth).collect::<Vec<_>>());
    let (_, _, dup_ok) = report("DUPLICATE_OF", &closure(&by("DUPLICATE_OF")), &dup_truth);
    let (v_got, _, v_ok) = report("VERSION_OF", &by("VERSION_OF"), &ver_truth);
    let (s_got, _, s_ok) = report("SAME_ENTITY", &by("SAME_ENTITY"), &ent_truth);
    // SAME_ENTITY by confidence band.
    for floor in [0.0, 0.5, 0.9] {
        let kept: BTreeSet<(String, String)> =
            env.edges().into_iter().filter(|e| e.kind == "SAME_ENTITY" && e.confidence >= floor).map(|e| (e.src, e.dst)).collect();
        report(&format!("SAME_ENT>={floor}"), &kept, &ent_truth);
    }

    // Hard guarantees: verified edges are exact; metadata edges are right except the labelled decoys.
    assert_eq!(dup_ok, closure(&by("DUPLICATE_OF")).len(), "DUPLICATE_OF precision is 1.0");
    assert_eq!(dup_ok, dup_truth.len(), "DUPLICATE_OF recall is 1.0");
    let decoys = pairs(&decoy_name_pairs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect::<Vec<_>>());
    let wrong: BTreeSet<_> = by("VERSION_OF").difference(&ver_truth).cloned().collect();
    assert_eq!(wrong, decoys, "the only false VERSION_OF edges are the memo decoys");
    assert!(v_ok > 0 && s_ok > 0 && v_got > 0 && s_got > 0);
    // The common standard in 33 files and the near-miss identifiers link nothing.
    assert!(!by("SAME_ENTITY").contains(&(rel(&env, "ent/near-a.txt"), rel(&env, "ent/near-b.txt"))));
    assert!(!by("SAME_ENTITY").iter().any(|(a, b)| a.contains("hub-") && b.contains("hub-")));
    // Numbered names without a version token produce no edge.
    assert!(!by("VERSION_OF").iter().any(|(a, _)| a.contains("table_")));
}

/// Project discovery (#240 P3-2) on a labelled synthetic tree: 10 projects (two of them under one
/// `old/` folder, each with a doc/ and a data/ subfolder), plus decoys. Prints precision and recall.
#[test]
fn project_discovery_precision_and_recall_on_a_labelled_tree() {
    let mut env = Env::new();
    let host = |env: &Env, name: &str, ids: &[String], seed: u64| {
        let lines: Vec<String> = ids.iter().map(|id| format!("see P/N {id} for details")).collect();
        env.write(name, &format!("{}\n{}", lines.join("\n"), body(seed)));
    };
    let mut truth: BTreeSet<String> = BTreeSet::new();
    let mut seed = 10_000u64;
    let places: Vec<String> = (0..8).map(|p| format!("work/pr{p}")).chain(["old/prx".to_string(), "old/pry".to_string()]).collect();
    for (p, place) in places.iter().enumerate() {
        truth.insert(env.path(place));
        let ids: Vec<String> = (0..3).map(|k| format!("QX-{}{}-{}", p + 1, k + 1, 100 + p * 7 + k)).collect();
        for k in 0..7usize {
            let sub = if k % 2 == 0 { "doc" } else { "data" };
            // Each file carries two of the project's three identifiers: a connected chain.
            host(&env, &format!("{place}/{sub}/f{k}.txt"), &[ids[k % 3].clone(), ids[(k + 1) % 3].clone(), "ASTM A36".into()], seed);
            seed += 1;
        }
    }
    // Decoys: an inbox of unrelated files, a folder of one-off identifiers, and a reference folder whose
    // files all cite one standard (a real cluster, but not a project: the expected false positive).
    for k in 0..8u64 {
        env.write(&format!("inbox/note{k}.txt"), &body(60_000 + k));
    }
    for k in 0..6usize {
        host(&env, &format!("scraps/s{k}.txt"), &[format!("ZZ-{}-{}", 50 + k, 9 + k)], seed);
        seed += 1;
    }
    for k in 0..6usize {
        host(&env, &format!("refs/r{k}.txt"), &["MIL-STD-810G".to_string()], seed);
        seed += 1;
    }
    env.refresh();
    env.catalog.rebuild_edges().unwrap();

    let got = env.catalog.project_suggestions().unwrap();
    let suggested: BTreeSet<String> = got.iter().map(|s| s.folder.clone()).collect();
    let hits = suggested.intersection(&truth).count();
    let (precision, recall) = (hits as f64 / suggested.len() as f64, hits as f64 / truth.len() as f64);
    eprintln!("project discovery: {} suggested, {hits} of {} true projects; precision {precision:.2}, recall {recall:.2}", suggested.len(), truth.len());
    for s in &got {
        eprintln!("  {} files={} linked={} cohesion={:.2} evidence={:?}", s.folder.replace(&env.path(""), ""), s.files, s.linked, s.cohesion, s.evidence);
    }
    // Never a subfolder of a project, a wrapper of two, a decoy without shared identifiers, or the root.
    for bad in ["inbox", "scraps", "old", "work"] {
        assert!(!suggested.contains(&env.path(bad)), "{bad} must not be suggested");
    }
    assert!(suggested.iter().all(|f| !f.ends_with("/doc") && !f.ends_with("/data")));
    assert!(got.iter().all(|s| !s.evidence.is_empty()), "every suggestion carries evidence");
    assert!(precision >= 0.8 && recall >= 0.8, "precision {precision} recall {recall}");
}

/// Knowledge gaps (#240 P4-4) on a labelled tree; the labels are planted by construction.
#[test]
fn knowledge_gaps_precision_and_recall_on_a_labelled_tree() {
    use crate::catalog_gaps::GapKind;
    let mut env = Env::new();
    let seed = std::cell::Cell::new(200_000u64);
    let next = || {
        seed.set(seed.get() + 1);
        seed.get()
    };
    let cite = |ids: &[&str]| ids.iter().map(|id| format!("per {id} requirements")).collect::<Vec<_>>().join("\n");

    // Missing documents. True gaps: standards cited by three files with no file named after them.
    let absent = ["AMS 5643", "NAS1352", "MIL-DTL-5541", "ASME B18.2"];
    for (i, std) in absent.iter().enumerate() {
        for k in 0..3 {
            env.write(&format!("jobs/j{i}/plan{k}.txt"), &format!("{}\n{}", cite(&[std]), body(next())));
        }
    }
    // Not gaps: the document is present (named after it, in any spacing/case), or one file cites it.
    for (std, name) in [("ASTM A36", "ASTM_A36_spec.txt"), ("MIL-STD-810G", "mil-std-810g.txt"), ("AMS 2759", "AMS2759 rev C.txt")] {
        env.write(&format!("library/{name}"), &body(next()));
        for k in 0..3 {
            env.write(&format!("jobs/cites_{}_{k}.txt", name.len()), &format!("{}\n{}", cite(&[std]), body(next())));
        }
    }
    env.write("jobs/once.txt", &format!("{}\n{}", cite(&["SAE J429"]), body(next())));
    // Planted limit: the document is present but its name does not carry the number (a false positive).
    env.write("library/socket_head_screws.txt", &body(next()));
    for k in 0..2 {
        env.write(&format!("jobs/screws{k}.txt"), &format!("{}\n{}", cite(&["NAS1351"]), body(next())));
    }
    let missing_truth: BTreeSet<String> = absent.iter().map(|s| s.to_string()).collect();

    // Dropped identifiers: version groups of four. `true` = the newest dropped an identifier every
    // earlier version holds and nothing explains it.
    let chain = |env: &Env, stem: &str, ids: [&[&str]; 4], newest_body: bool| {
        for (v, ids) in ids.iter().enumerate() {
            let text = if v == 3 && !newest_body { String::new() } else { format!("{}\n{}", cite(ids), body(next())) };
            env.write(&format!("specs/{stem}_v{}.txt", v + 1), &text);
        }
    };
    let keep: [&[&str]; 4] = [&["QX-100-A", "QX-200-B"]; 4];
    let drop_b: [&[&str]; 4] = [&["QX-100-A", "QX-200-B"], &["QX-100-A", "QX-200-B"], &["QX-100-A", "QX-200-B"], &["QX-100-A"]];
    let mut dropped_truth: BTreeSet<String> = BTreeSet::new();
    for (n, id) in ["QX-310-B", "QX-320-B", "QX-330-B"].iter().enumerate() {
        let stem = format!("drop{n}");
        chain(&env, &stem, [&["QX-100-A", id], &["QX-100-A", id], &["QX-100-A", id], &["QX-100-A"]], true);
        dropped_truth.insert(format!("{stem}:{id}"));
    }
    chain(&env, "keepa", keep, true);
    chain(&env, "keepb", keep, true);
    chain(&env, "unread", drop_b, false); // newest has no text: nothing to compare
    // Renumbered: the old identifier is replaced, not lost (false positives by construction).
    for n in 0..2 {
        let (old, new) = (format!("QX-9{n}1-C"), format!("QX-9{n}2-C"));
        chain(&env, &format!("renum{n}"), [&["QX-100-A", &old], &["QX-100-A", &old], &["QX-100-A", &old], &["QX-100-A", &new]], true);
    }
    // Held by only two of the three earlier versions: not a unanimous drop.
    chain(&env, "partial", [&["QX-100-A", "QX-440-D"], &["QX-100-A"], &["QX-100-A", "QX-440-D"], &["QX-100-A"]], true);
    // Ordered by file time only (status words): the order is weak, so nothing is claimed.
    for (k, name) in ["draft", "final", "latest", "backup"].iter().enumerate() {
        let ids: &[&str] = if k == 3 { &["QX-100-A"] } else { &["QX-100-A", "QX-550-E"] };
        env.write(&format!("specs/weak_{name}.txt"), &format!("{}\n{}", cite(ids), body(next())));
    }
    env.refresh();
    env.catalog.rebuild_edges().unwrap();

    let gaps = env.catalog.knowledge_gaps().unwrap();
    let missing: BTreeSet<String> = gaps.iter().filter(|g| g.kind == GapKind::MissingDocument).map(|g| g.subject.clone()).collect();
    let dropped: BTreeSet<String> = gaps
        .iter()
        .filter(|g| g.kind == GapKind::DroppedIdentifier)
        .map(|g| format!("{}:{}", Path::new(&g.files[0]).file_name().unwrap().to_string_lossy().split('_').next().unwrap(), g.subject))
        .collect();
    let score = |name: &str, got: &BTreeSet<String>, truth: &BTreeSet<String>| {
        let tp = got.intersection(truth).count();
        let (p, r) = (tp as f64 / got.len().max(1) as f64, tp as f64 / truth.len() as f64);
        eprintln!("{name}: {} flagged, {tp} of {} true; precision {p:.2}, recall {r:.2}; flagged {got:?}", got.len(), truth.len());
        (p, r)
    };
    let (mp, mr) = score("missing documents", &missing, &missing_truth);
    let (dp, dr) = score("dropped identifiers", &dropped, &dropped_truth);
    // The planted limits are the only expected errors.
    assert!(missing.contains("NAS1351"), "document not named by number is a known false positive");
    assert!(dropped.iter().any(|d| d.starts_with("renum")), "a renumbering is a known false positive");
    for never in ["ASTM A36", "MIL-STD-810G", "AMS 2759", "SAE J429"] {
        assert!(!missing.contains(never), "{never}");
    }
    assert!(dropped.iter().all(|d| !d.starts_with("keep") && !d.starts_with("unread") && !d.starts_with("partial") && !d.starts_with("weak")), "{dropped:?}");
    assert!(gaps.iter().all(|g| !g.files.is_empty() && !g.detail.is_empty()));
    assert!(mr == 1.0 && dr == 1.0, "recall {mr} {dr}");
    assert!(mp >= 0.8 && dp >= 0.5, "precision {mp} {dp}");
}
