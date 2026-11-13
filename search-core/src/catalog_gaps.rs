//! Knowledge gaps (#240 P4-4): things the catalog suggests are missing or inconsistent, offered as
//! notes the user may ignore. Nothing here changes a search result or writes anything; it reads
//! stored entities, file names and version groups and returns evidence-bearing candidates.
//!
//! Two kinds, both from facts the catalog already holds:
//! - [`GapKind::MissingDocument`]: a standard (`ASTM A36`, `MIL-STD-810G`, `AMS 5643`: entity kind
//!   `standard`) cited by at least [`MIN_CITING_FILES`] files while no catalogued file has it in its
//!   *name*. Says "cited but not in these folders", not "should be": a standard may live elsewhere.
//! - [`GapKind::DroppedIdentifier`]: a version group ordered by explicit version tokens only (so the
//!   order is strong, see `catalog_chains`) of at least [`MIN_CHAIN`] files, whose newest member lacks
//!   a part number or standard that every earlier member holds, while the newest member does hold
//!   other entities (so its text was read). Says "earlier versions all cite X, this one does not".
//!
//! Not built: "entity mentioned in one file only". Most identifiers occur once by nature (a log of
//! unique ids, any one-off part number), so the signal has no contrast; see
//! `docs/knowledge-engine-audit.md` (2026-11-12, P4-4) for the measurement.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::Catalog;
use crate::catalog_chains::{version_chain, Basis};
use crate::catalog_edges::{file_name, EntityRow, FileRow, StoredEdge};

/// Fewest files that must cite a standard before its absence is worth a note.
pub const MIN_CITING_FILES: usize = 2;
/// Fewest members of a version group for a dropped identifier (two predecessors that agree).
pub const MIN_CHAIN: usize = 3;
/// Most gaps returned.
pub const MAX_GAPS: usize = 20;
const MAX_FILES_SHOWN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GapKind {
    MissingDocument,
    DroppedIdentifier,
}

/// One suggested gap with the evidence behind it.
#[derive(Debug, Clone, PartialEq)]
pub struct Gap {
    pub kind: GapKind,
    /// The identifier as written in its first occurrence.
    pub subject: String,
    /// One sentence naming the evidence, no generated text beyond the counts and names it holds.
    pub detail: String,
    /// Files that make the case (citing files, or the newest version), a few at most.
    pub files: Vec<String>,
    /// How many files back the claim: citing files, or earlier versions holding the identifier.
    pub support: usize,
}

/// Letters and digits only, upper case: `ASTM A36` and `astm_a36.pdf` meet.
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_uppercase).collect()
}

/// Gaps from catalogued `files`, stored standard and part-number `entities`, `read` (paths holding any
/// stored entity of any kind, which proves their text was read) and stored `VERSION_OF` `edges` (other edge
/// types are ignored). Deterministic; strongest first.
pub fn find(files: &[FileRow], entities: &[EntityRow], read: &BTreeSet<String>, edges: &[StoredEdge]) -> Vec<Gap> {
    let known: BTreeSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut gaps = missing_documents(files, entities, &known);
    gaps.extend(dropped_identifiers(files, entities, read, edges, &known));
    gaps.sort_by(|a, b| b.support.cmp(&a.support).then_with(|| a.kind.cmp(&b.kind)).then_with(|| a.subject.cmp(&b.subject)));
    gaps.truncate(MAX_GAPS);
    gaps
}

fn missing_documents(files: &[FileRow], entities: &[EntityRow], known: &BTreeSet<&str>) -> Vec<Gap> {
    let names: Vec<String> = files.iter().map(|f| squash(file_name(&f.path))).collect();
    // value -> (shown, citing paths)
    let mut cited: BTreeMap<&str, (&str, BTreeSet<&str>)> = BTreeMap::new();
    for e in entities.iter().filter(|e| e.kind == "standard" && known.contains(e.path.as_str())) {
        cited.entry(&e.value).or_insert((&e.shown, BTreeSet::new())).1.insert(&e.path);
    }
    cited
        .into_iter()
        .filter(|(_, (_, holders))| holders.len() >= MIN_CITING_FILES)
        .filter_map(|(value, (shown, holders))| {
            let key = squash(value);
            // A document named after the standard is its copy; a file citing it by name is not.
            if key.is_empty() || names.iter().any(|n| n.contains(&key)) {
                return None;
            }
            Some(Gap {
                kind: GapKind::MissingDocument,
                subject: shown.to_string(),
                detail: format!("cited in {} files; no file name contains it", holders.len()),
                files: holders.iter().take(MAX_FILES_SHOWN).map(|p| p.to_string()).collect(),
                support: holders.len(),
            })
        })
        .collect()
}

fn dropped_identifiers(files: &[FileRow], entities: &[EntityRow], read: &BTreeSet<String>, edges: &[StoredEdge], known: &BTreeSet<&str>) -> Vec<Gap> {
    // path -> (kind, value) -> shown, for the identifying kinds only.
    let mut held: BTreeMap<&str, BTreeMap<(&str, &str), &str>> = BTreeMap::new();
    for e in entities.iter().filter(|e| known.contains(e.path.as_str())) {
        let slot = held.entry(&e.path).or_default();
        if e.kind == "standard" || e.kind == "part_number" {
            slot.entry((&e.kind, &e.value)).or_insert(&e.shown);
        }
    }
    let version_edges: Vec<StoredEdge> = edges.iter().filter(|e| e.kind == "VERSION_OF").cloned().collect();
    let members: BTreeSet<&str> = version_edges.iter().flat_map(|e| [e.src.as_str(), e.dst.as_str()]).collect();
    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut out = Vec::new();
    for path in members {
        let own: Vec<StoredEdge> = version_edges.iter().filter(|e| e.src == path || e.dst == path).cloned().collect();
        let Some(chain) = version_chain(path, &own, files) else { continue };
        if chain.files.len() < MIN_CHAIN || chain.bases.iter().any(|b| *b != Basis::Token) || !seen.insert(chain.files.clone()) {
            continue;
        }
        let (newest, earlier) = chain.files.split_last().expect("chain has members");
        if !read.contains(newest) {
            continue;
        }
        let empty = BTreeMap::new();
        let newest_has = held.get(newest.as_str()).unwrap_or(&empty);
        // Held by every earlier member, absent from the newest.
        let mut shared: Option<BTreeMap<(&str, &str), &str>> = None;
        for p in earlier {
            let mine = held.get(p.as_str()).unwrap_or(&empty);
            shared = Some(match shared {
                None => mine.clone(),
                Some(s) => s.into_iter().filter(|(k, _)| mine.contains_key(k)).collect(),
            });
        }
        for ((_, _), shown) in shared.unwrap_or_default().into_iter().filter(|(k, _)| !newest_has.contains_key(k)) {
            out.push(Gap {
                kind: GapKind::DroppedIdentifier,
                subject: shown.to_string(),
                detail: format!("in all {} earlier versions, not in the newest, {}", earlier.len(), file_name(newest)),
                files: vec![newest.clone()],
                support: earlier.len(),
            });
        }
    }
    out
}

impl Catalog {
    /// [`find`] over this catalog's files, entities and `VERSION_OF` edges. Reads only.
    pub fn knowledge_gaps(&self) -> rusqlite::Result<Vec<Gap>> {
        let mut seen = BTreeSet::new();
        let files: Vec<FileRow> = self
            .files()?
            .into_iter()
            .filter(|f| seen.insert(f.path.clone()))
            .map(|f| FileRow { path: f.path, size: f.size, modified: f.modified_unix, content_hash: f.content_hash })
            .collect();
        let read: BTreeSet<String> = self
            .conn
            .prepare("SELECT DISTINCT path FROM entities")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut stmt = self
            .conn
            .prepare("SELECT path,kind,value,shown,line FROM entities WHERE kind IN ('standard','part_number') ORDER BY kind,value,path")?;
        let entities = stmt
            .query_map([], |r| Ok(EntityRow { path: r.get(0)?, kind: r.get(1)?, value: r.get(2)?, shown: r.get(3)?, line: r.get(4)? }))?
            .collect::<Result<Vec<_>, _>>()?;
        let edges = self.query_edges("WHERE type='VERSION_OF'", [])?;
        Ok(find(&files, &entities, &read, &edges))
    }
}
