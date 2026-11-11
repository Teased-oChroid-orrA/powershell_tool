//! Stored entities (#240 P1-2): identifying strings found in catalogued files (part numbers, standards,
//! fastener callouts, materials), kept with their evidence so a later step can answer "which files
//! share part number X" and say why.
//!
//! Every row is a pattern match from `entities::extract_candidates` (class `pattern`, extractor
//! `pattern-v1`): file, 1-based line, the text as written and a normalised value. Nothing here is a
//! verified fact or a statistical score. Resolution is by normalisation only (case, spacing, trailing
//! punctuation); two spellings that normalise differently are different entities. Dimensions and
//! units are not stored: they do not identify anything.

use std::path::Path;

use rusqlite::params;

use crate::catalog::Catalog;
use crate::entities::{extract_candidates, EntityKind};
use crate::extraction;

const EXTRACTOR: &str = "pattern-v1";
const CLASS: &str = "pattern";
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// One stored entity occurrence with its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEntity {
    pub root: String,
    pub path: String,
    pub kind: String,
    pub value: String,
    pub shown: String,
    pub line: usize,
    pub class: String,
    pub extractor: String,
}

/// A file that shares entities with another, and which ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedEntities {
    pub root: String,
    pub path: String,
    /// `(kind, normalised value)` pairs present in both files.
    pub shared: Vec<(String, String)>,
}

/// Normal form used to compare two occurrences: trimmed, collapsed spaces, upper case, trailing
/// punctuation removed.
pub fn normalize(shown: &str) -> String {
    let collapsed = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_end_matches(['.', ',', ';', ':']).to_uppercase()
}

/// Whether a candidate is specific enough to identify something. The loose hyphenated part-number
/// pattern also matches dates and plain words, so a part number must mix letters and digits.
fn identifies(kind: EntityKind, shown: &str) -> bool {
    match kind {
        EntityKind::PartNumber => {
            shown.chars().any(|c| c.is_ascii_alphabetic()) && shown.chars().any(|c| c.is_ascii_digit())
        }
        EntityKind::Material | EntityKind::FastenerCallout | EntityKind::Standard => true,
        EntityKind::Dimension | EntityKind::Unit => false,
    }
}

/// Reads and extracts a file the way indexing does; `None` when it cannot be read as text.
fn read_text(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    extraction::extract_lines_by_extension(&ext, &bytes, 30, None, false)
        .ok()
        .map(|doc| doc.lines.join("\n"))
}

impl Catalog {
    /// Extracts entities for every file of `root` whose stored entities are missing or out of date
    /// (stamp is the file's size and mtime when it was extracted). `read` returns a file's text; use
    /// `None` for the default reader. Returns the number of files (re)extracted.
    pub fn index_entities(&mut self, root: &str, read: Option<&dyn Fn(&str) -> Option<String>>) -> Result<usize, String> {
        let pending: Vec<(i64, String, i64, i64)> = {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT r.id,f.path,f.size,f.modified FROM files f JOIN roots r ON r.id=f.root_id
                     WHERE r.path=?1 AND f.entities_stamp IS NULL ORDER BY f.path",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([root], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        let mut done = 0;
        for (root_id, path, size, modified) in pending {
            let text = match read {
                Some(read) => read(&path),
                None => read_text(Path::new(&path)),
            };
            let tx = self.conn.transaction().map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM entities WHERE root_id=?1 AND path=?2", params![root_id, path])
                .map_err(|e| e.to_string())?;
            if let Some(text) = &text {
                let extraction = extract_candidates(text);
                let mut insert = tx
                    .prepare(
                        "INSERT INTO entities(root_id,path,kind,value,shown,line,class,extractor)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                    )
                    .map_err(|e| e.to_string())?;
                let mut seen = std::collections::HashSet::new();
                for candidate in &extraction.candidates {
                    let shown = &text[candidate.bytes.clone()];
                    for kind in candidate.shape_hints.iter().copied().filter(|k| identifies(*k, shown)) {
                        let value = normalize(shown);
                        // One row per (kind, value) per file: the first line is the evidence.
                        if value.is_empty() || !seen.insert((kind.as_str(), value.clone())) {
                            continue;
                        }
                        insert
                            .execute(params![root_id, path, kind.as_str(), value, shown, candidate.line_number, CLASS, EXTRACTOR])
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
            // An unreadable file is stamped too (no entities) so it is not retried until it changes.
            tx.execute(
                "UPDATE files SET entities_stamp=?3 WHERE root_id=?1 AND path=?2",
                params![root_id, path, format!("{size}:{modified}")],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            done += 1;
        }
        Ok(done)
    }

    /// Every stored occurrence of `value` (normalised first) of `kind`, ordered by root, path.
    pub fn files_with_entity(&self, kind: EntityKind, value: &str) -> rusqlite::Result<Vec<StoredEntity>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.path,e.path,e.kind,e.value,e.shown,e.line,e.class,e.extractor
             FROM entities e JOIN roots r ON r.id=e.root_id
             WHERE e.kind=?1 AND e.value=?2 ORDER BY r.path,e.path",
        )?;
        let rows = stmt.query_map(params![kind.as_str(), normalize(value)], |r| {
            Ok(StoredEntity {
                root: r.get(0)?,
                path: r.get(1)?,
                kind: r.get(2)?,
                value: r.get(3)?,
                shown: r.get(4)?,
                line: r.get(5)?,
                class: r.get(6)?,
                extractor: r.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// Other files that share at least one stored entity with `path` of `root`, most shared first
    /// (ties by path), each with the shared `(kind, value)` pairs as evidence.
    pub fn files_sharing_entities(&self, root: &str, path: &str) -> rusqlite::Result<Vec<SharedEntities>> {
        let mut stmt = self.conn.prepare(
            "SELECT r2.path,o.path,o.kind,o.value
             FROM entities m
             JOIN roots r1 ON r1.id=m.root_id
             JOIN entities o ON o.kind=m.kind AND o.value=m.value
             JOIN roots r2 ON r2.id=o.root_id
             WHERE r1.path=?1 AND m.path=?2 AND NOT (o.root_id=m.root_id AND o.path=m.path)
             ORDER BY r2.path,o.path,o.kind,o.value",
        )?;
        let mut out: Vec<SharedEntities> = Vec::new();
        let rows = stmt.query_map(params![root, path], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })?;
        for row in rows {
            let (other_root, other_path, kind, value) = row?;
            match out.last_mut() {
                Some(last) if last.root == other_root && last.path == other_path => last.shared.push((kind, value)),
                _ => out.push(SharedEntities { root: other_root, path: other_path, shared: vec![(kind, value)] }),
            }
        }
        out.sort_by(|a, b| b.shared.len().cmp(&a.shared.len()).then_with(|| a.path.cmp(&b.path)));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_index;
    use native_search::engine::DocumentInput;

    fn index(dir: &Path, docs: &[(&str, &str, i64)]) {
        native_index::ensure_index_directory_exists(dir).unwrap();
        let engine = native_index::open_or_create_with_rebuild(dir).unwrap();
        for (path, body, modified) in docs {
            engine
                .index_document(DocumentInput {
                    id: path,
                    path,
                    filename: path,
                    extension: ".txt",
                    title: "",
                    modified_unix: *modified,
                    created_unix: 0,
                    size: body.len() as i64,
                    body,
                })
                .unwrap();
        }
        engine.commit().unwrap();
    }

    fn texts() -> std::collections::HashMap<&'static str, &'static str> {
        [
            ("/a/spec.txt", "Title\nP/N AX-21-7 bracket per ASTM A36.\nIssued 2026-11-10.\nGap 0.25 in."),
            ("/b/order.txt", "Order list\nPart number ax-21-7  qty 4\nUse M6x1 screws."),
            ("/b/other.txt", "Nothing relevant. Dated 2025-01-02."),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn entities_are_stored_with_evidence_and_shared_across_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let (ia, ib) = (tmp.path().join("ia"), tmp.path().join("ib"));
        index(&ia, &[("/a/spec.txt", "x", 1)]);
        index(&ib, &[("/b/order.txt", "x", 1), ("/b/other.txt", "x", 1)]);
        let mut c = Catalog::open_in_memory().unwrap();
        c.register_root("/a", &ia).unwrap();
        c.register_root("/b", &ib).unwrap();
        c.sync_root("/a").unwrap();
        c.sync_root("/b").unwrap();
        let t = texts();
        let reader = |p: &str| t.get(p).map(|s| s.to_string());
        assert_eq!(c.index_entities("/a", Some(&reader)).unwrap(), 1);
        assert_eq!(c.index_entities("/b", Some(&reader)).unwrap(), 2);

        // Same part number written two ways resolves to one entity in two files, with line evidence.
        let hits = c.files_with_entity(EntityKind::PartNumber, "AX-21-7").unwrap();
        assert_eq!(hits.iter().map(|h| (h.path.as_str(), h.line)).collect::<Vec<_>>(), vec![("/a/spec.txt", 2), ("/b/order.txt", 2)]);
        assert_eq!(hits[1].shown, "ax-21-7");
        assert!(hits.iter().all(|h| h.class == "pattern" && h.extractor == "pattern-v1"));

        // Dates are not part numbers; dimensions are not stored.
        assert!(c.files_with_entity(EntityKind::PartNumber, "2026-11-10").unwrap().is_empty());
        let stored: i64 = c.conn.query_row("SELECT COUNT(*) FROM entities WHERE kind IN ('dimension','unit')", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, 0);

        let related = c.files_sharing_entities("/a", "/a/spec.txt").unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!((related[0].root.as_str(), related[0].path.as_str()), ("/b", "/b/order.txt"));
        assert_eq!(related[0].shared, vec![("part_number".to_string(), "AX-21-7".to_string())]);

        // Unchanged files are not re-extracted.
        assert_eq!(c.index_entities("/a", Some(&reader)).unwrap(), 0);
    }

    #[test]
    fn a_changed_file_loses_stale_entities_and_a_removed_file_loses_all() {
        let tmp = tempfile::tempdir().unwrap();
        let ia = tmp.path().join("ia");
        index(&ia, &[("/a/f.txt", "x", 1), ("/a/g.txt", "x", 1)]);
        let mut c = Catalog::open_in_memory().unwrap();
        c.register_root("/a", &ia).unwrap();
        c.sync_root("/a").unwrap();
        let v1 = |_: &str| Some("P/N QQ-11-9".to_string());
        assert_eq!(c.index_entities("/a", Some(&v1)).unwrap(), 2);
        assert_eq!(c.files_with_entity(EntityKind::PartNumber, "QQ-11-9").unwrap().len(), 2);

        // f.txt changes on disk (new mtime) and g.txt is removed from the index.
        index(&ia, &[("/a/f.txt", "y", 2)]);
        let engine = native_index::open_or_create_with_rebuild(&ia).unwrap();
        engine.delete_document("/a/g.txt").unwrap();
        engine.commit().unwrap();
        drop(engine);
        c.sync_root("/a").unwrap();
        assert!(
            c.files_with_entity(EntityKind::PartNumber, "QQ-11-9").unwrap().is_empty(),
            "changed file's old entities and removed file's entities are gone"
        );
        let v2 = |_: &str| Some("P/N ZZ-22-3".to_string());
        assert_eq!(c.index_entities("/a", Some(&v2)).unwrap(), 1, "only the changed file is re-extracted");
        let now = c.files_with_entity(EntityKind::PartNumber, "ZZ-22-3").unwrap();
        assert_eq!(now.len(), 1);
        assert_eq!(now[0].path, "/a/f.txt");
    }
}
