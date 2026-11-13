//! Stored entities (#240 P1-2): identifying strings found in catalogued files (part numbers, standards,
//! fastener callouts, materials), kept with their evidence so a later step can answer "which files
//! share part number X" and say why.
//!
//! Every row is a pattern match from `entities::extract_candidates` (class `pattern`, extractor
//! `pattern-v2`): file, 1-based line, the text as written and a normalised value. Nothing here is a
//! verified fact or a statistical score. Resolution is by normalisation only (case, spacing, trailing
//! punctuation); two spellings that normalise differently are different entities. Dimensions and
//! units are not stored: they do not identify anything.
//!
//! Rare terms (P1-7, kind `rare_term`, same class and evidence): identifier-like words the patterns
//! above do not cover, found by shape alone (see [`is_rare_shape`]): letters mixed with digits
//! (`xk7741b`, `HX-2290`) or an inner capital after a lower-case letter (`GsToolbench`). Shape says
//! "looks like an identifier", not "is rare": rarity comes from document frequency, which the edge
//! builder applies (an entity in more than `MAX_ENTITY_FILES` files links nothing). At most
//! [`MAX_RARE_PER_FILE`] per file, in text order, so a log of unique ids cannot flood the table.
//! Evidence only: nothing ranks with them.

use std::path::Path;

use rusqlite::params;

use crate::catalog::Catalog;
use crate::entities::{extract_candidates, EntityKind};
use crate::extraction;

const EXTRACTOR: &str = "pattern-v2";
const CLASS: &str = "pattern";
/// Kind string of rare-term rows (not an [`EntityKind`]: the extractor in `entities` knows nothing of it).
pub const RARE_TERM_KIND: &str = "rare_term";
/// Most rare terms stored per file.
pub const MAX_RARE_PER_FILE: usize = 200;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Returns a file's text by path; stands in for reading the disk.
pub type TextReader<'a> = dyn Fn(&str) -> Option<String> + 'a;

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

/// Whether a word looks like an identifier: 5 to 40 characters of letters, digits, `-` and `_`, with a digit
/// and at least two letters, or an upper-case letter right after a lower-case one. A number with a short unit
/// (`100mm`, `3rd`, `2024q3` is not: the suffix is over three letters) is a quantity, not an identifier.
pub fn is_rare_shape(word: &str) -> bool {
    if !(5..=40 * 4).contains(&word.len()) {
        return false;
    }
    let (mut n, mut letters, mut digits, mut inner_capital, mut prev_lower) = (0usize, 0usize, 0usize, false, false);
    for c in word.chars() {
        n += 1;
        if c.is_ascii_digit() {
            digits += 1;
            prev_lower = false;
        } else if c.is_alphabetic() {
            letters += 1;
            inner_capital |= prev_lower && c.is_uppercase();
            prev_lower = c.is_lowercase();
        } else {
            prev_lower = false;
        }
    }
    if !(5..=40).contains(&n) || !((digits > 0 && letters >= 2) || inner_capital) {
        return false;
    }
    // number followed by at most three letters: 100mm, 25kg, 3rd
    let tail = word.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
    let is_quantity = tail.len() < word.len() && tail.len() <= 3 && tail.chars().all(|c| c.is_alphabetic());
    !is_quantity
}

/// Rare-term occurrences of `text` not already in `taken` (normalised values of the pattern entities):
/// `(normalised value, text as written, line)`, first line of each value, at most [`MAX_RARE_PER_FILE`].
fn rare_occurrences(text: &str, taken: &std::collections::HashSet<String>) -> Vec<(String, String, usize)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    'lines: for (i, line) in text.lines().enumerate() {
        for word in line.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')) {
            let word = word.trim_matches(['-', '_']);
            if !is_rare_shape(word) {
                continue;
            }
            let value = normalize(word);
            if !taken.contains(&value) && seen.insert(value.clone()) {
                out.push((value, word.to_string(), i + 1));
                if out.len() == MAX_RARE_PER_FILE {
                    break 'lines;
                }
            }
        }
    }
    out
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

/// What `index_entities` stores for one file's text: `(kind, normalised value, text as written, first
/// line)`, one per (kind, value). Shared with the relationship evaluation so both see the same entities.
pub(crate) fn entity_occurrences(text: &str) -> Vec<(&'static str, String, String, usize)> {
    let extraction = extract_candidates(text);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for candidate in &extraction.candidates {
        let shown = &text[candidate.bytes.clone()];
        for kind in candidate.shape_hints.iter().copied().filter(|k| identifies(*k, shown)) {
            let value = normalize(shown);
            // One row per (kind, value) per file: the first line is the evidence.
            if value.is_empty() || !seen.insert((kind.as_str(), value.clone())) {
                continue;
            }
            out.push((kind.as_str(), value, shown.to_string(), candidate.line_number));
        }
    }
    let taken: std::collections::HashSet<String> = out.iter().map(|o| o.1.clone()).collect();
    out.extend(rare_occurrences(text, &taken).into_iter().map(|(value, shown, line)| (RARE_TERM_KIND, value, shown, line)));
    out
}

impl Catalog {
    /// Extracts entities for every file of `root` whose stored entities are missing or out of date
    /// (stamp is the file's size and mtime when it was extracted). `read` returns a file's text; use
    /// `None` for the default reader. Returns the number of files (re)extracted.
    pub fn index_entities(&mut self, root: &str, read: Option<&TextReader<'_>>) -> Result<usize, String> {
        // Rows stamped by an older extractor lack its newer kinds: extract those files again.
        self.conn
            .execute("UPDATE files SET entities_stamp=NULL WHERE entities_stamp IS NOT NULL AND entities_stamp NOT LIKE ?1", params![format!("%:{EXTRACTOR}")])
            .map_err(|e| e.to_string())?;
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
                let mut insert = tx
                    .prepare(
                        "INSERT INTO entities(root_id,path,kind,value,shown,line,class,extractor)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                    )
                    .map_err(|e| e.to_string())?;
                for (kind, value, shown, line) in entity_occurrences(text) {
                    insert
                        .execute(params![root_id, path, kind, value, shown, line, CLASS, EXTRACTOR])
                        .map_err(|e| e.to_string())?;
                }
            }
            // An unreadable file is stamped too (no entities) so it is not retried until it changes.
            tx.execute(
                "UPDATE files SET entities_stamp=?3 WHERE root_id=?1 AND path=?2",
                params![root_id, path, format!("{size}:{modified}:{EXTRACTOR}")],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            done += 1;
        }
        Ok(done)
    }

    /// Every stored occurrence of `value` (normalised first) of `kind`, ordered by root, path.
    pub fn files_with_entity(&self, kind: EntityKind, value: &str) -> rusqlite::Result<Vec<StoredEntity>> {
        self.files_with_kind(kind.as_str(), value)
    }

    /// Every stored occurrence of the rare term `value` (normalised first), ordered by root, path.
    pub fn files_with_rare_term(&self, value: &str) -> rusqlite::Result<Vec<StoredEntity>> {
        self.files_with_kind(RARE_TERM_KIND, value)
    }

    fn files_with_kind(&self, kind: &str, value: &str) -> rusqlite::Result<Vec<StoredEntity>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.path,e.path,e.kind,e.value,e.shown,e.line,e.class,e.extractor
             FROM entities e JOIN roots r ON r.id=e.root_id
             WHERE e.kind=?1 AND e.value=?2 ORDER BY r.path,e.path",
        )?;
        let rows = stmt.query_map(params![kind, normalize(value)], |r| {
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
    fn rare_shapes_are_identifier_like_and_quantities_are_not() {
        for yes in ["xk7741b", "HX-2290", "GsToolbench", "build_7f3a9c", "ab12c"] {
            assert!(is_rare_shape(yes), "{yes}");
        }
        for no in ["100mm", "25kg", "hello", "WARNING", "Title", "2026-11-10", "a1b2", "x".repeat(41).as_str(), "12345", "Mixed-case"] {
            assert!(!is_rare_shape(no), "{no}");
        }
    }

    #[test]
    fn rare_terms_are_stored_once_skip_pattern_entities_and_are_capped() {
        let taken: std::collections::HashSet<String> = ["AX-21-7".to_string()].into();
        let got = rare_occurrences("see AX-21-7 and xk7741b.\nxk7741b again, GsToolbench\n", &taken);
        assert_eq!(got, vec![("XK7741B".into(), "xk7741b".into(), 1), ("GSTOOLBENCH".into(), "GsToolbench".into(), 2)]);
        let many: String = (0..MAX_RARE_PER_FILE + 50).map(|i| format!("id{i:05}x ")).collect();
        assert_eq!(rare_occurrences(&many, &Default::default()).len(), MAX_RARE_PER_FILE);
    }

    #[test]
    fn rare_terms_link_files_across_roots_and_an_older_extractor_stamp_is_redone() {
        let tmp = tempfile::tempdir().unwrap();
        let (ia, ib) = (tmp.path().join("ia"), tmp.path().join("ib"));
        index(&ia, &[("/a/one.txt", "x", 1)]);
        index(&ib, &[("/b/two.txt", "x", 1)]);
        let mut c = Catalog::open_in_memory().unwrap();
        c.register_root("/a", &ia).unwrap();
        c.register_root("/b", &ib).unwrap();
        c.sync_root("/a").unwrap();
        c.sync_root("/b").unwrap();
        let t: std::collections::HashMap<&str, &str> = [("/a/one.txt", "Run tag zq4417r passed."), ("/b/two.txt", "Failure of ZQ4417R seen.")].into();
        let reader = |p: &str| t.get(p).map(|s| s.to_string());
        c.index_entities("/a", Some(&reader)).unwrap();
        c.index_entities("/b", Some(&reader)).unwrap();
        let hits = c.files_with_rare_term("zq4417r").unwrap();
        assert_eq!(hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(), ["/a/one.txt", "/b/two.txt"]);
        assert_eq!((hits[0].class.as_str(), hits[0].extractor.as_str(), hits[0].line), ("pattern", EXTRACTOR, 1));
        // A stamp left by the previous extractor version (no suffix) means "extract again".
        c.conn.execute("UPDATE files SET entities_stamp='1:1'", []).unwrap();
        assert_eq!(c.index_entities("/a", Some(&reader)).unwrap(), 1);
        assert_eq!(c.index_entities("/a", Some(&reader)).unwrap(), 0, "now current");
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
        assert!(hits.iter().all(|h| h.class == "pattern" && h.extractor == EXTRACTOR));

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
