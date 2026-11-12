//! Relationship edges (#240 P3-1): what the catalog can say about how two files relate, with the
//! evidence for it, a class and a confidence. Derived and additive, like the rest of the catalog:
//! `rebuild_edges` recomputes every edge from the `files` and `entities` tables, so deleting the
//! `relationship` table loses nothing.
//!
//! Types and classes (the class says what a reader may do with the edge):
//! - `FILE_IN_FOLDER` (metadata, 1.0): the file's parent folder. Pure path metadata.
//! - `DUPLICATE_OF` (verified, 1.0): equal content hash, i.e. identical extracted text (and PDF pages).
//!   Each extra copy points at the smallest path of its group, so a group of n costs n-1 edges.
//! - `VERSION_OF` (metadata, 0.9 or 0.7): same folder, same extension, same file-name stem once version
//!   tokens (`v2`, `rev B`, `r3`, a date, `(2)`, `copy`, `final` ...) are removed. Names only; the
//!   evidence is both names and the tokens found. 0.9 when an explicit version token or a copy marker
//!   is involved, 0.7 when only a status word (draft, final, old, new, backup, latest) is. The numbers
//!   are ordinal tiers, not probabilities. Groups of more than 30 files are skipped (generic stems).
//! - `SAME_ENTITY` (statistical): two files hold the same stored entity (P1-2, pattern matches). The
//!   confidence is a noisy-or of 1/(files holding the entity - 1) over the shared entities, so an
//!   identifier in two files is strong and one in many files is weak; an entity in more than
//!   `MAX_ENTITY_FILES` files links nothing (it identifies nobody). Never a fact.
//!
//! `SIMILAR_TO` is not built: it needs file-level vectors from `semantic.sqlite`, whose storage is
//! changing, and nothing here may depend on that format.
//!
//! Only `metadata` and `verified` edges may be shown as fact (`facts_for`); `statistical` edges come
//! from `statistical_for` with an explicit confidence threshold and are labelled as such.
//! Invalidation: an edge's `source_version` holds the stamp of each end (content hash, or size and
//! mtime when there is none); a rebuild drops edges whose ends are gone and rewrites edges whose
//! stamp, evidence, class or confidence changed. `created` is kept for edges that did not change.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use regex::Regex;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::catalog::Catalog;

/// An entity held by more files than this links nothing: it is a common term, not an identifier.
pub const MAX_ENTITY_FILES: usize = 25;
/// A version group larger than this is a generic stem ("image"), not a document's versions.
pub const MAX_VERSION_GROUP: usize = 30;
const MAX_EVIDENCE_ENTITIES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeType {
    FileInFolder,
    SameEntity,
    DuplicateOf,
    VersionOf,
}

impl EdgeType {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeType::FileInFolder => "FILE_IN_FOLDER",
            EdgeType::SameEntity => "SAME_ENTITY",
            EdgeType::DuplicateOf => "DUPLICATE_OF",
            EdgeType::VersionOf => "VERSION_OF",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeClass {
    Metadata,
    Verified,
    Statistical,
}

impl EdgeClass {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeClass::Metadata => "metadata",
            EdgeClass::Verified => "verified",
            EdgeClass::Statistical => "statistical",
        }
    }
}

/// An edge as computed, before storage. Symmetric types have `src < dst`.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub src: String,
    pub dst: String,
    pub kind: EdgeType,
    pub confidence: f64,
    pub evidence: Value,
    pub class: EdgeClass,
    pub source_version: String,
}

/// An edge as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEdge {
    pub src: String,
    pub dst: String,
    pub kind: String,
    pub confidence: f64,
    pub evidence: Value,
    pub class: String,
    pub created: i64,
    pub source_version: String,
}

/// What an edge builder needs to know about a file.
#[derive(Debug, Clone)]
pub struct FileRow {
    pub path: String,
    pub size: i64,
    pub modified: i64,
    pub content_hash: Option<Vec<u8>>,
}

/// One stored entity occurrence.
#[derive(Debug, Clone)]
pub struct EntityRow {
    pub path: String,
    pub kind: String,
    pub value: String,
    pub shown: String,
    pub line: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeStats {
    pub total: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Freshness stamp of one end of an edge: the content hash when known, else size and mtime.
pub fn stamp(file: &FileRow) -> String {
    match &file.content_hash {
        Some(hash) => format!("h:{}", hex(hash)),
        None => format!("s:{}:{}", file.size, file.modified),
    }
}

/// Parent folder of an indexed path (either separator); `/` for a top-level file.
pub fn parent_folder(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(0) => &path[..1],
        Some(i) => &path[..i],
        None => "",
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn pair_version(stamps: &BTreeMap<&str, String>, src: &str, dst: &str) -> String {
    format!("{}|{}", stamps[src], stamps[dst])
}

pub fn folder_edges(files: &[FileRow]) -> Vec<Edge> {
    files
        .iter()
        .filter(|f| !parent_folder(&f.path).is_empty())
        .map(|f| {
            let folder = parent_folder(&f.path).to_string();
            Edge {
                src: f.path.clone(),
                dst: folder.clone(),
                kind: EdgeType::FileInFolder,
                confidence: 1.0,
                evidence: json!({ "folder": folder }),
                class: EdgeClass::Metadata,
                source_version: stamp(f),
            }
        })
        .collect()
}

/// SHA-256 of no bytes: the hash of a file with no extractable text, which all such files share.
const EMPTY_SHA256: [u8; 32] = [
    0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95,
    0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
];

pub fn duplicate_edges(files: &[FileRow]) -> Vec<Edge> {
    let mut groups: BTreeMap<&[u8], Vec<&FileRow>> = BTreeMap::new();
    for f in files {
        // Files with no text all hash alike; having nothing in common is not a relationship.
        if let Some(h) = f.content_hash.as_ref().filter(|h| h.as_slice() != EMPTY_SHA256) {
            groups.entry(h.as_slice()).or_default().push(f);
        }
    }
    let mut out = Vec::new();
    for (hash, mut group) in groups {
        if group.len() < 2 {
            continue;
        }
        group.sort_by(|a, b| a.path.cmp(&b.path));
        let canonical = group[0];
        for copy in &group[1..] {
            out.push(Edge {
                src: copy.path.clone(),
                dst: canonical.path.clone(),
                kind: EdgeType::DuplicateOf,
                confidence: 1.0,
                evidence: json!({
                    "basis": "identical extracted text (SHA-256)",
                    "content_hash": hex(hash),
                    "group_size": group.len(),
                }),
                class: EdgeClass::Verified,
                source_version: format!("{}|{}", stamp(copy), stamp(canonical)),
            });
        }
    }
    out
}

fn version_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\(\d{1,2}\)|\bv\d+(?:\.\d+){0,3}\b|\brev[ \-]?[a-z0-9]{1,2}\b|\br\d{1,3}\b|\b\d{4}-\d{2}-\d{2}\b|\b\d{8}\b|\bcopy(?: of)?(?: \d+)?\b|\b(?:draft|final|old|new|backup|bak|latest)(?: ?\d+)?\b",
        )
        .expect("version token pattern")
    })
}

/// File name split into the stem that identifies the document, its extension and the version tokens
/// removed from it. `None` when nothing is left of the stem.
pub fn version_parts(name: &str) -> Option<(String, String, Vec<String>)> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], name[i + 1..].to_lowercase()),
        _ => (name, String::new()),
    };
    let spaced = stem.replace('_', " ");
    let mut tokens = Vec::new();
    let rest = version_token_re().replace_all(&spaced, |c: &regex::Captures| {
        tokens.push(c[0].to_lowercase());
        " "
    });
    let base: String = rest
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_lowercase().collect::<String>() } else { " ".into() })
        .collect();
    let base = base.split_whitespace().collect::<Vec<_>>().join(" ");
    (!base.is_empty()).then_some((base, ext, tokens))
}

fn is_status_word(token: &str) -> bool {
    ["draft", "final", "old", "new", "backup", "bak", "latest"].iter().any(|w| token.starts_with(w))
}

pub fn version_edges(files: &[FileRow]) -> Vec<Edge> {
    let stamps: BTreeMap<&str, String> = files.iter().map(|f| (f.path.as_str(), stamp(f))).collect();
    let mut groups: BTreeMap<(String, String, String), Vec<(&FileRow, Vec<String>)>> = BTreeMap::new();
    for f in files {
        if let Some((base, ext, tokens)) = version_parts(file_name(&f.path)) {
            groups.entry((parent_folder(&f.path).to_string(), ext, base)).or_default().push((f, tokens));
        }
    }
    let mut out = Vec::new();
    for ((_, _, base), mut group) in groups {
        if group.len() < 2 || group.len() > MAX_VERSION_GROUP {
            continue;
        }
        group.sort_by(|a, b| a.0.path.cmp(&b.0.path));
        for i in 0..group.len() {
            for j in i + 1..group.len() {
                let (a, ta) = (&group[i].0, &group[i].1);
                let (b, tb) = (&group[j].0, &group[j].1);
                let only_status = ta.iter().chain(tb).all(|t| is_status_word(t));
                out.push(Edge {
                    src: a.path.clone(),
                    dst: b.path.clone(),
                    kind: EdgeType::VersionOf,
                    confidence: if only_status { 0.7 } else { 0.9 },
                    evidence: json!({
                        "basis": "same folder, extension and file-name stem after removing version tokens",
                        "stem": base,
                        "names": [file_name(&a.path), file_name(&b.path)],
                        "tokens": [ta, tb],
                    }),
                    class: EdgeClass::Metadata,
                    source_version: pair_version(&stamps, &a.path, &b.path),
                });
            }
        }
    }
    out
}

pub fn entity_edges(files: &[FileRow], entities: &[EntityRow]) -> Vec<Edge> {
    let stamps: BTreeMap<&str, String> = files.iter().map(|f| (f.path.as_str(), stamp(f))).collect();
    // (kind, value) -> one occurrence per file.
    let mut by_entity: BTreeMap<(&str, &str), BTreeMap<&str, &EntityRow>> = BTreeMap::new();
    for e in entities.iter().filter(|e| stamps.contains_key(e.path.as_str())) {
        by_entity.entry((&e.kind, &e.value)).or_default().entry(&e.path).or_insert(e);
    }
    // (a, b) -> shared entities as (1/(df-1), evidence)
    let mut pairs: BTreeMap<(&str, &str), Vec<(f64, Value)>> = BTreeMap::new();
    for ((kind, value), holders) in &by_entity {
        let df = holders.len();
        if df < 2 || df > MAX_ENTITY_FILES {
            continue;
        }
        let weight = 1.0 / (df - 1) as f64;
        let list: Vec<&&EntityRow> = holders.values().collect();
        for i in 0..list.len() {
            for j in i + 1..list.len() {
                let (a, b) = (list[i], list[j]);
                pairs.entry((&a.path, &b.path)).or_default().push((
                    weight,
                    json!({
                        "kind": kind, "value": value, "files_with_entity": df,
                        "src_line": a.line, "src_text": a.shown, "dst_line": b.line, "dst_text": b.shown,
                    }),
                ));
            }
        }
    }
    pairs
        .into_iter()
        .map(|((a, b), mut shared)| {
            let confidence = 1.0 - shared.iter().map(|(w, _)| 1.0 - w).product::<f64>();
            shared.sort_by(|x, y| y.0.total_cmp(&x.0));
            let count = shared.len();
            let shown: Vec<Value> = shared.into_iter().take(MAX_EVIDENCE_ENTITIES).map(|(_, v)| v).collect();
            Edge {
                src: a.to_string(),
                dst: b.to_string(),
                kind: EdgeType::SameEntity,
                confidence,
                evidence: json!({
                    "basis": "same stored entity (pattern match, not verified)",
                    "shared_count": count, "shared": shown,
                }),
                class: EdgeClass::Statistical,
                source_version: pair_version(&stamps, a, b),
            }
        })
        .collect()
}

/// Entity rows for one file's text, exactly as `Catalog::index_entities` would store them.
#[cfg(feature = "neural-search")]
pub fn entity_rows(path: &str, text: &str) -> Vec<EntityRow> {
    crate::catalog_entities::entity_occurrences(text)
        .into_iter()
        .map(|(kind, value, shown, line)| EntityRow { path: path.to_string(), kind: kind.to_string(), value, shown, line: line as i64 })
        .collect()
}

/// Every edge the catalog's current files and entities support, in a deterministic order.
pub fn compute_edges(files: &[FileRow], entities: &[EntityRow]) -> Vec<Edge> {
    let mut edges = folder_edges(files);
    edges.extend(duplicate_edges(files));
    edges.extend(version_edges(files));
    edges.extend(entity_edges(files, entities));
    // Paths are the keys; a path catalogued under two roots counts once, and nothing links a file to itself.
    edges.retain(|e| e.src != e.dst);
    edges.sort_by(|a, b| (&a.src, &a.dst, a.kind).cmp(&(&b.src, &b.dst, b.kind)));
    edges.dedup_by(|b, a| (&a.src, &a.dst, a.kind) == (&b.src, &b.dst, b.kind));
    edges
}

pub(crate) fn create_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS relationship(
             src TEXT NOT NULL,
             dst TEXT NOT NULL,
             type TEXT NOT NULL,
             confidence REAL NOT NULL CHECK(confidence >= 0 AND confidence <= 1),
             evidence_json TEXT NOT NULL CHECK(length(evidence_json) > 2),
             class TEXT NOT NULL CHECK(class IN ('metadata','verified','statistical')),
             created INTEGER NOT NULL,
             source_version TEXT NOT NULL,
             PRIMARY KEY(src,dst,type)
         );
         CREATE INDEX IF NOT EXISTS relationship_dst ON relationship(dst,type);",
    )
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl Catalog {
    /// Recomputes every relationship edge from the catalogued files and stored entities.
    pub fn rebuild_edges(&mut self) -> Result<EdgeStats, String> {
        self.rebuild_edges_at(now_unix())
    }

    /// `rebuild_edges` with an explicit `created` time for new and rewritten edges.
    pub fn rebuild_edges_at(&mut self, now: i64) -> Result<EdgeStats, String> {
        let err = |e: rusqlite::Error| e.to_string();
        let files: Vec<FileRow> = {
            let mut stmt = self
                .conn
                .prepare("SELECT f.path,f.size,f.modified,f.content_hash FROM files f JOIN roots r ON r.id=f.root_id ORDER BY r.path,f.path")
                .map_err(err)?;
            let rows = stmt
                .query_map([], |r| Ok(FileRow { path: r.get(0)?, size: r.get(1)?, modified: r.get(2)?, content_hash: r.get(3)? }))
                .map_err(err)?;
            let mut seen = BTreeSet::new();
            rows.collect::<Result<Vec<_>, _>>().map_err(err)?.into_iter().filter(|f| seen.insert(f.path.clone())).collect()
        };
        let entities: Vec<EntityRow> = {
            let mut stmt = self
                .conn
                .prepare("SELECT path,kind,value,shown,line FROM entities ORDER BY kind,value,path")
                .map_err(err)?;
            let rows = stmt
                .query_map([], |r| Ok(EntityRow { path: r.get(0)?, kind: r.get(1)?, value: r.get(2)?, shown: r.get(3)?, line: r.get(4)? }))
                .map_err(err)?;
            rows.collect::<Result<_, _>>().map_err(err)?
        };
        let wanted = compute_edges(&files, &entities);

        let tx = self.conn.transaction().map_err(err)?;
        let mut existing: BTreeMap<(String, String, String), (f64, String, String, String)> = BTreeMap::new();
        {
            let mut stmt = tx
                .prepare("SELECT src,dst,type,confidence,evidence_json,class,source_version FROM relationship")
                .map_err(err)?;
            let rows = stmt
                .query_map([], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), (r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))))
                .map_err(err)?;
            for row in rows {
                let (key, value) = row.map_err(err)?;
                existing.insert(key, value);
            }
        }
        let mut stats = EdgeStats { total: wanted.len(), ..Default::default() };
        {
            let mut put = tx
                .prepare(
                    "INSERT OR REPLACE INTO relationship(src,dst,type,confidence,evidence_json,class,created,source_version)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                )
                .map_err(err)?;
            for edge in &wanted {
                let key = (edge.src.clone(), edge.dst.clone(), edge.kind.as_str().to_string());
                let evidence = edge.evidence.to_string();
                match existing.remove(&key) {
                    Some((c, ev, class, version))
                        if (c - edge.confidence).abs() < 1e-12 && ev == evidence && class == edge.class.as_str() && version == edge.source_version => {}
                    previous => {
                        // Rewritten edges are new claims: `created` restarts.
                        put.execute(params![edge.src, edge.dst, key.2, edge.confidence, evidence, edge.class.as_str(), now, edge.source_version])
                            .map_err(err)?;
                        if previous.is_some() { stats.updated += 1 } else { stats.added += 1 }
                    }
                }
            }
        }
        for (src, dst, kind) in existing.keys() {
            tx.execute("DELETE FROM relationship WHERE src=?1 AND dst=?2 AND type=?3", params![src, dst, kind]).map_err(err)?;
            stats.removed += 1;
        }
        tx.commit().map_err(err)?;
        Ok(stats)
    }

    /// Every stored edge that has `path` at either end, ordered by type, confidence (best first), then path.
    pub fn edges_for(&self, path: &str) -> rusqlite::Result<Vec<StoredEdge>> {
        self.query_edges("WHERE src=?1 OR dst=?1", params![path])
    }

    /// Edges that may be presented as fact: class `metadata` or `verified`.
    pub fn facts_for(&self, path: &str) -> rusqlite::Result<Vec<StoredEdge>> {
        self.query_edges("WHERE (src=?1 OR dst=?1) AND class IN ('metadata','verified')", params![path])
    }

    /// Statistical edges with at least `min_confidence`; to be shown labelled as inferred, never as fact.
    pub fn statistical_for(&self, path: &str, min_confidence: f64) -> rusqlite::Result<Vec<StoredEdge>> {
        self.query_edges("WHERE (src=?1 OR dst=?1) AND class='statistical' AND confidence>=?2", params![path, min_confidence])
    }

    fn query_edges(&self, filter: &str, args: impl rusqlite::Params) -> rusqlite::Result<Vec<StoredEdge>> {
        let sql = format!(
            "SELECT src,dst,type,confidence,evidence_json,class,created,source_version FROM relationship {filter}
             ORDER BY type,confidence DESC,src,dst"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(args, |r| {
            let evidence: String = r.get(4)?;
            Ok(StoredEdge {
                src: r.get(0)?,
                dst: r.get(1)?,
                kind: r.get(2)?,
                confidence: r.get(3)?,
                evidence: serde_json::from_str(&evidence).unwrap_or(Value::Null),
                class: r.get(5)?,
                created: r.get(6)?,
                source_version: r.get(7)?,
            })
        })?;
        rows.collect()
    }
}

/// Weakest statistical edge worth showing a reader (an entity in at most three files, or several shared).
pub const EXPLAIN_MIN_CONFIDENCE: f64 = 0.5;
const EXPLAIN_PER_KIND: usize = 3;

/// One sentence of why a file relates to another, backed by one stored edge. `inferred` edges are statistical
/// (pattern matches, not verified) and must be shown as such; the rest are facts about names or content hashes.
#[derive(Debug, Clone, PartialEq)]
pub struct Reason {
    /// The other end of the edge.
    pub other: String,
    pub text: String,
    pub inferred: bool,
}

/// The other end of `edge` as seen from `path`.
fn other_end<'a>(edge: &'a StoredEdge, path: &str) -> &'a str {
    if edge.src == path { &edge.dst } else { &edge.src }
}

/// Reasons for `path` from its stored edges: duplicates and versions (facts) first, then shared entities
/// (inferred, at least [`EXPLAIN_MIN_CONFIDENCE`]). Folder edges are skipped (every file has one). At most
/// three per kind, best first, plus a count of the rest. Only reads the edges' own evidence; no text is generated
/// beyond naming the other file and what the evidence says.
pub fn explain(path: &str, edges: &[StoredEdge]) -> Vec<Reason> {
    let mut out = Vec::new();
    let kinds = [("DUPLICATE_OF", false), ("VERSION_OF", false), ("SAME_ENTITY", true)];
    for (kind, inferred) in kinds {
        let mut of_kind: Vec<&StoredEdge> = edges
            .iter()
            .filter(|e| e.kind == kind && (e.src == path || e.dst == path))
            .filter(|e| !inferred || e.confidence >= EXPLAIN_MIN_CONFIDENCE)
            .collect();
        of_kind.sort_by(|a, b| b.confidence.total_cmp(&a.confidence).then_with(|| other_end(a, path).cmp(other_end(b, path))));
        for edge in of_kind.iter().take(EXPLAIN_PER_KIND) {
            let other = other_end(edge, path);
            let name = file_name(other);
            let text = match kind {
                "DUPLICATE_OF" => format!("Identical text to {name}"),
                "VERSION_OF" => format!("Another version of the same document: {name}"),
                _ => {
                    let shared = edge.evidence["shared_count"].as_u64().unwrap_or(1);
                    let first = edge.evidence["shared"][0]["src_text"].as_str().unwrap_or("");
                    let first = if edge.src == path { first } else { edge.evidence["shared"][0]["dst_text"].as_str().unwrap_or(first) };
                    let e = if shared == 1 { "entity" } else { "entities" };
                    if first.is_empty() { format!("Shares {shared} {e} with {name}") } else { format!("Shares {shared} {e} with {name}, e.g. {first}") }
                }
            };
            out.push(Reason { other: other.to_string(), text, inferred });
        }
        if of_kind.len() > EXPLAIN_PER_KIND {
            let more = of_kind.len() - EXPLAIN_PER_KIND;
            let what = match kind {
                "DUPLICATE_OF" => "identical copies",
                "VERSION_OF" => "versions",
                _ => "files sharing entities",
            };
            out.push(Reason { other: String::new(), text: format!("and {more} more {what}"), inferred });
        }
    }
    out
}

impl Catalog {
    /// [`explain`] for `path` from the stored edges.
    pub fn reasons_for(&self, path: &str) -> rusqlite::Result<Vec<Reason>> {
        Ok(explain(path, &self.edges_for(path)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, hash: Option<u8>) -> FileRow {
        FileRow { path: path.into(), size: 1, modified: 1, content_hash: hash.map(|h| vec![h]) }
    }

    fn entity(path: &str, value: &str, line: i64) -> EntityRow {
        EntityRow { path: path.into(), kind: "part_number".into(), value: value.into(), shown: value.to_lowercase(), line }
    }

    #[test]
    fn version_names_reduce_to_a_stem_and_tokens() {
        let parts = |n| version_parts(n).map(|(s, e, t)| (s, e, t));
        assert_eq!(parts("Spec_v2.docx"), Some(("spec".into(), "docx".into(), vec!["v2".into()])));
        assert_eq!(parts("Spec rev B.docx"), Some(("spec".into(), "docx".into(), vec!["rev b".into()])));
        assert_eq!(parts("plan-2026-01-05.txt"), Some(("plan".into(), "txt".into(), vec!["2026-01-05".into()])));
        assert_eq!(parts("plan - Copy (2).txt").map(|p| p.0), Some("plan".into()));
        assert_eq!(parts("plan.txt").map(|p| p.2.len()), Some(0));
        // Words that merely start like a token are not tokens.
        assert_eq!(parts("revenue.txt").map(|p| p.0), Some("revenue".into()));
        assert_eq!(parts("newton.txt").map(|p| p.0), Some("newton".into()));
        assert_eq!(parts("v2.txt"), None);
    }

    #[test]
    fn every_edge_type_carries_evidence_class_and_a_stamp() {
        let files = vec![
            file("/a/spec.txt", Some(1)),
            file("/a/spec_v2.txt", Some(2)),
            file("/b/copy-of-spec.txt", Some(1)),
            file("/a/other.txt", None),
        ];
        let entities = vec![entity("/a/spec.txt", "AX-1", 3), entity("/a/other.txt", "AX-1", 9)];
        let edges = compute_edges(&files, &entities);
        let kinds: BTreeSet<&str> = edges.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, BTreeSet::from(["DUPLICATE_OF", "FILE_IN_FOLDER", "SAME_ENTITY", "VERSION_OF"]));
        for e in &edges {
            assert!(e.evidence.as_object().is_some_and(|o| !o.is_empty()), "{e:?}");
            assert!(!e.source_version.is_empty() && (0.0..=1.0).contains(&e.confidence));
        }
        let dup = edges.iter().find(|e| e.kind == EdgeType::DuplicateOf).unwrap();
        assert_eq!((dup.src.as_str(), dup.dst.as_str(), dup.class), ("/b/copy-of-spec.txt", "/a/spec.txt", EdgeClass::Verified));
        let ver = edges.iter().find(|e| e.kind == EdgeType::VersionOf).unwrap();
        assert_eq!((ver.src.as_str(), ver.dst.as_str(), ver.class), ("/a/spec.txt", "/a/spec_v2.txt", EdgeClass::Metadata));
        let same = edges.iter().find(|e| e.kind == EdgeType::SameEntity).unwrap();
        assert_eq!(same.class, EdgeClass::Statistical);
        assert_eq!(same.confidence, 1.0);
        assert_eq!((&same.evidence["shared"][0]["src_line"], &same.evidence["shared"][0]["dst_line"]), (&json!(9), &json!(3)));
        // Different folders or different stems never form a version pair.
        assert_eq!(edges.iter().filter(|e| e.kind == EdgeType::VersionOf).count(), 1);
    }

    #[test]
    fn reasons_name_the_other_file_label_inference_and_skip_folders_and_weak_edges() {
        let stored = |src: &str, dst: &str, kind: &str, confidence: f64, evidence: Value, class: &str| StoredEdge {
            src: src.into(), dst: dst.into(), kind: kind.into(), confidence, evidence, class: class.into(), created: 0, source_version: String::new(),
        };
        let edges = vec![
            stored("/d/a.txt", "/d", "FILE_IN_FOLDER", 1.0, json!({"folder": "/d"}), "metadata"),
            stored("/d/b.txt", "/d/a.txt", "DUPLICATE_OF", 1.0, json!({}), "verified"),
            stored("/d/a.txt", "/d/spec v2.txt", "VERSION_OF", 0.9, json!({}), "metadata"),
            stored("/d/a.txt", "/d/c.txt", "SAME_ENTITY", 0.9, json!({"shared_count": 2, "shared": [{"src_text": "AN3-5A", "dst_text": "an3-5a"}]}), "statistical"),
            stored("/d/a.txt", "/d/weak.txt", "SAME_ENTITY", 0.2, json!({"shared_count": 1, "shared": []}), "statistical"),
            stored("/x/y.txt", "/x/z.txt", "DUPLICATE_OF", 1.0, json!({}), "verified"),
        ];
        let got = explain("/d/a.txt", &edges);
        let texts: Vec<(&str, bool)> = got.iter().map(|r| (r.text.as_str(), r.inferred)).collect();
        assert_eq!(texts, [
            ("Identical text to b.txt", false),
            ("Another version of the same document: spec v2.txt", false),
            ("Shares 2 entities with c.txt, e.g. AN3-5A", true),
        ]);
        // Seen from the other end of an entity edge, the other file's own spelling is quoted.
        let dst_side = explain("/d/c.txt", &edges);
        assert_eq!(dst_side[0].text, "Shares 2 entities with a.txt, e.g. an3-5a");
        // More than three of a kind are counted, not listed.
        let many: Vec<StoredEdge> = (0..5).map(|i| stored("/d/a.txt", &format!("/d/c{i}.txt"), "DUPLICATE_OF", 1.0, json!({}), "verified")).collect();
        let got = explain("/d/a.txt", &many);
        assert_eq!(got.len(), 4);
        assert_eq!(got[3].text, "and 2 more identical copies");
    }

    #[test]
    fn files_without_text_are_not_duplicates_of_each_other() {
        use sha2::{Digest, Sha256};
        assert_eq!(Sha256::digest(b"").as_slice(), EMPTY_SHA256);
        let empty = |p: &str| FileRow { path: p.into(), size: 0, modified: 1, content_hash: Some(EMPTY_SHA256.to_vec()) };
        assert!(duplicate_edges(&[empty("/a/x.pdf"), empty("/b/y.pdf")]).is_empty());
    }

    #[test]
    fn a_common_entity_links_nothing_and_confidence_falls_with_frequency() {
        let files: Vec<FileRow> = (0..40).map(|i| file(&format!("/d/f{i:02}.txt"), None)).collect();
        let mut entities: Vec<EntityRow> = files.iter().map(|f| entity(&f.path, "COMMON-1", 1)).collect();
        entities.extend([entity("/d/f00.txt", "RARE-2", 5), entity("/d/f01.txt", "RARE-2", 6)]);
        entities.extend(["/d/f00.txt", "/d/f01.txt", "/d/f02.txt"].iter().map(|p| entity(p, "MID-3", 7)));
        let edges: Vec<Edge> = entity_edges(&files, &entities);
        let pair = |a: &str, b: &str| edges.iter().find(|e| e.src == a && e.dst == b).map(|e| e.confidence);
        // f00-f01 share RARE (df 2 -> 1.0) and MID (df 3 -> 0.5): noisy-or is 1.0.
        assert_eq!(pair("/d/f00.txt", "/d/f01.txt"), Some(1.0));
        assert_eq!(pair("/d/f00.txt", "/d/f02.txt"), Some(0.5));
        assert_eq!(pair("/d/f10.txt", "/d/f11.txt"), None, "40 files hold COMMON-1");
    }
}

#[cfg(all(test, feature = "neural-search"))]
#[path = "catalog_edges_tests.rs"]
mod catalog_tests;
