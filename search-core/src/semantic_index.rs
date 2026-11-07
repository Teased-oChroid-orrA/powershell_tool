//! Persistent, local embedding index for optional semantic Search results.
//! Exact matching remains owned by `matching` and `native_index`.

use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
    path::Path,
    sync::Mutex,
};

use rusqlite::{params, Connection, OptionalExtension};
use search_ml::{Embedding, MlError};

const EMBEDDING_VERSION: i64 = 1;
const EMBEDDING_DIMENSIONS: usize = 384;
const CHUNK_CHARS: usize = 800;
const CHUNK_TOKEN_BUDGET: usize = 240;
const EMBED_BATCH: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticHit {
    pub path: String,
    pub first_line: usize,
    pub last_line: usize,
    pub excerpt: String,
    pub similarity: f32,
}

/// Keep the strongest cited chunk for each possible related document.
pub fn group_similar_documents(
    source_path: &str,
    hits: &[SemanticHit],
    limit: usize,
) -> Vec<SemanticHit> {
    let mut documents = std::collections::HashMap::<String, SemanticHit>::new();
    for hit in hits.iter().filter(|hit| hit.path != source_path) {
        documents
            .entry(hit.path.clone())
            .and_modify(|current| {
                if hit.similarity > current.similarity
                    || (hit.similarity == current.similarity
                        && (hit.first_line, hit.last_line, &hit.excerpt)
                            < (current.first_line, current.last_line, &current.excerpt))
                {
                    *current = hit.clone();
                }
            })
            .or_insert_with(|| hit.clone());
    }
    let mut documents: Vec<_> = documents.into_values().collect();
    documents.sort_by(|left, right| {
        right
            .similarity
            .total_cmp(&left.similarity)
            .then_with(|| left.path.cmp(&right.path))
    });
    documents.truncate(limit.min(10));
    documents
}

struct TextChunk {
    first_line: usize,
    last_line: usize,
    text: String,
    embedding: Embedding,
}

#[derive(Debug)]
struct HeapHit(SemanticHit);

impl PartialEq for HeapHit {
    fn eq(&self, other: &Self) -> bool {
        self.0.similarity.to_bits() == other.0.similarity.to_bits()
            && self.0.path == other.0.path
            && self.0.first_line == other.0.first_line
    }
}
impl Eq for HeapHit {}
impl PartialOrd for HeapHit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapHit {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .similarity
            .total_cmp(&other.0.similarity)
            .then_with(|| self.0.path.cmp(&other.0.path))
            .then_with(|| self.0.first_line.cmp(&other.0.first_line))
    }
}

/// One SQLite sidecar per native-search folder. Vectors stream from SQLite
/// during retrieval, so query memory stays proportional to `limit`.
pub struct SemanticIndex {
    conn: Mutex<Connection>,
}

impl SemanticIndex {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS semantic_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS semantic_documents (
                 path TEXT PRIMARY KEY, modified INTEGER NOT NULL, size INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS semantic_chunks (
                 path TEXT NOT NULL REFERENCES semantic_documents(path) ON DELETE CASCADE,
                 chunk_index INTEGER NOT NULL,
                 first_line INTEGER NOT NULL, last_line INTEGER NOT NULL,
                 excerpt TEXT NOT NULL, embedding BLOB NOT NULL,
                 PRIMARY KEY(path, chunk_index)
             );
             CREATE INDEX IF NOT EXISTS semantic_chunks_path ON semantic_chunks(path);",
        )?;
        let version: Option<i64> = conn
            .query_row(
                "SELECT value FROM semantic_meta WHERE key = 'embedding_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if version != Some(EMBEDDING_VERSION) {
            conn.execute_batch("DELETE FROM semantic_chunks; DELETE FROM semantic_documents;")?;
            conn.execute(
                "INSERT INTO semantic_meta(key,value) VALUES('embedding_version',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [EMBEDDING_VERSION],
            )?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Store bounded, line-cited chunks for a document. Replacing a document
    /// is atomic, so interruption never leaves a half-updated vector set.
    pub fn index_document(
        &self,
        path: &str,
        modified: i64,
        size: i64,
        lines: &[String],
    ) -> Result<usize, String> {
        let chunks = embed_lines(lines).map_err(|error| error.to_string())?;
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| "semantic index lock poisoned".to_string())?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM semantic_documents WHERE path=?1", [path])
            .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO semantic_documents(path,modified,size) VALUES(?1,?2,?3)",
            params![path, modified, size],
        )
        .map_err(|error| error.to_string())?;
        for (chunk_index, chunk) in chunks.iter().enumerate() {
            let vector = embedding_bytes(&chunk.embedding);
            tx.execute(
                "INSERT INTO semantic_chunks(path,chunk_index,first_line,last_line,excerpt,embedding) VALUES(?1,?2,?3,?4,?5,?6)",
                params![path, chunk_index as i64, chunk.first_line as i64, chunk.last_line as i64, chunk.text, vector],
            ).map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(chunks.len())
    }

    pub fn is_current(&self, path: &str, modified: i64, size: i64) -> rusqlite::Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let stored: Option<(i64, i64)> = conn
            .query_row(
                "SELECT modified,size FROM semantic_documents WHERE path=?1",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(stored == Some((modified, size)))
    }

    pub fn remove_document(&self, path: &str) -> rusqlite::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        conn.execute("DELETE FROM semantic_documents WHERE path=?1", [path])?;
        Ok(())
    }

    /// Remove deleted source files without treating permission errors as deletion.
    pub fn remove_orphaned_documents(&self) -> Result<usize, String> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| "semantic index lock poisoned".to_string())?;
        let paths = {
            let mut stmt = conn
                .prepare("SELECT path FROM semantic_documents")
                .map_err(|error| error.to_string())?;
            let paths = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            paths
        };
        let orphaned = paths
            .into_iter()
            .filter(|path| Path::new(path).try_exists().is_ok_and(|exists| !exists))
            .collect::<Vec<_>>();
        if orphaned.is_empty() {
            return Ok(0);
        }
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        for path in &orphaned {
            tx.execute("DELETE FROM semantic_documents WHERE path=?1", [path])
                .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(orphaned.len())
    }

    /// Semantic results are separate, cited suggestions. This method never
    /// changes the exact-search result set or query semantics.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SemanticHit>, String> {
        self.search_filtered(query, limit, None, None)
    }

    pub fn search_with_settings(
        &self,
        query: &str,
        limit: usize,
        settings: &crate::models::SearchSettings,
    ) -> Result<Vec<SemanticHit>, String> {
        self.search_filtered(query, limit, Some(settings), None)
    }

    pub fn search_excluding_path_with_settings(
        &self,
        query: &str,
        limit: usize,
        settings: &crate::models::SearchSettings,
        excluded_path: &str,
    ) -> Result<Vec<SemanticHit>, String> {
        self.search_filtered(query, limit, Some(settings), Some(excluded_path))
    }

    fn search_filtered(
        &self,
        query: &str,
        limit: usize,
        settings: Option<&crate::models::SearchSettings>,
        excluded_path: Option<&str>,
    ) -> Result<Vec<SemanticHit>, String> {
        if query.trim().is_empty() {
            return Err("semantic query must not be empty".into());
        }
        let Some(query_embedding) = search_ml::embed_batch(&[query])
            .map_err(|error| error.to_string())?
            .pop()
        else {
            return Err("semantic model returned no embedding".into());
        };
        let conn = self
            .conn
            .lock()
            .map_err(|_| "semantic index lock poisoned".to_string())?;
        let mut stmt = conn
            .prepare("SELECT c.path,c.first_line,c.last_line,c.excerpt,c.embedding,d.modified,d.size FROM semantic_chunks c JOIN semantic_documents d ON d.path=c.path ORDER BY c.path,c.chunk_index")
            .map_err(|error| error.to_string())?;
        let mut rows = stmt.query([]).map_err(|error| error.to_string())?;
        let mut best = BinaryHeap::<Reverse<HeapHit>>::new();
        let limit = limit.min(1000);
        let allowed_extensions = settings.map(|settings| {
            settings
                .extensions
                .clone()
                .unwrap_or_else(crate::models::extension_catalog::all_extensions)
        });
        let mut current_path = String::new();
        let mut current_path_allowed = false;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let path: String = row.get(0).map_err(|error| error.to_string())?;
            if path != current_path {
                current_path_allowed = excluded_path != Some(path.as_str())
                    && semantic_path_allowed(
                        &path,
                        settings,
                        allowed_extensions.as_deref(),
                        row.get(5).map_err(|error| error.to_string())?,
                        row.get(6).map_err(|error| error.to_string())?,
                    );
                current_path = path.clone();
            }
            if !current_path_allowed {
                continue;
            }
            let bytes: Vec<u8> = row.get(4).map_err(|error| error.to_string())?;
            let Some(embedding) = embedding_from_bytes(&bytes) else {
                continue;
            };
            let similarity = search_ml::cosine_similarity(&query_embedding, &embedding)
                .unwrap_or(f32::NEG_INFINITY);
            if !similarity.is_finite() {
                continue;
            }
            let hit = SemanticHit {
                path,
                first_line: row
                    .get::<_, i64>(1)
                    .map_err(|error| error.to_string())?
                    .max(0) as usize,
                last_line: row
                    .get::<_, i64>(2)
                    .map_err(|error| error.to_string())?
                    .max(0) as usize,
                excerpt: row.get(3).map_err(|error| error.to_string())?,
                similarity,
            };
            if limit == 0 {
                continue;
            }
            if best.len() < limit {
                best.push(Reverse(HeapHit(hit)));
            } else if best
                .peek()
                .is_some_and(|smallest| hit.similarity > smallest.0 .0.similarity)
            {
                best.pop();
                best.push(Reverse(HeapHit(hit)));
            }
        }
        Ok(best
            .into_sorted_vec()
            .into_iter()
            .map(|Reverse(hit)| hit.0)
            .collect())
    }
}

/// Search one semantic sidecar by its containing native-index directory.
/// A missing sidecar is an error instead of silently creating an empty index.
pub fn search_directory(
    index_directory: &Path,
    query: &str,
    limit: usize,
) -> Result<Vec<SemanticHit>, String> {
    let path = index_directory.join("semantic.sqlite");
    if !path.is_file() {
        return Err("semantic index is not built".into());
    }
    SemanticIndex::open(&path)
        .map_err(|error| error.to_string())?
        .search(query, limit)
}

pub fn search_directory_with_settings(
    index_directory: &Path,
    query: &str,
    limit: usize,
    settings: &crate::models::SearchSettings,
) -> Result<Vec<SemanticHit>, String> {
    let path = index_directory.join("semantic.sqlite");
    if !path.is_file() {
        return Err("semantic index is not built".into());
    }
    SemanticIndex::open(&path)
        .map_err(|error| error.to_string())?
        .search_with_settings(query, limit, settings)
}

pub fn search_directory_excluding_path_with_settings(
    index_directory: &Path,
    query: &str,
    limit: usize,
    settings: &crate::models::SearchSettings,
    excluded_path: &str,
) -> Result<Vec<SemanticHit>, String> {
    let path = index_directory.join("semantic.sqlite");
    if !path.is_file() {
        return Err("semantic index is not built".into());
    }
    SemanticIndex::open(&path)
        .map_err(|error| error.to_string())?
        .search_excluding_path_with_settings(query, limit, settings, excluded_path)
}

fn semantic_path_allowed(
    path: &str,
    settings: Option<&crate::models::SearchSettings>,
    allowed_extensions: Option<&[String]>,
    indexed_modified: i64,
    indexed_size: i64,
) -> bool {
    let path = Path::new(path);
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    if metadata.len() as i64 != indexed_size
        || crate::cache::ticks_from_modified(crate::file_reader::system_time_to_local(modified))
            != indexed_modified
    {
        return false;
    }
    if let Some(settings) = settings {
        let root = Path::new(&settings.search_path);
        if !path.starts_with(root)
            || crate::file_reader::is_excluded_path(path, root, &settings.exclude_folders)
        {
            return false;
        }
        if !settings.include_hidden {
            let mut current = path.to_path_buf();
            while current.starts_with(root) && current != root {
                if crate::file_reader::is_hidden(&current) {
                    return false;
                }
                if !current.pop() {
                    break;
                }
            }
        }
        let max_bytes = (settings.max_file_size_mb * 1024.0 * 1024.0) as i64;
        if metadata.len() as i64 > max_bytes {
            return false;
        }
        let extension = path
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()))
            .unwrap_or_default();
        if !allowed_extensions.is_some_and(|allowed| {
            allowed
                .iter()
                .any(|candidate| candidate == "*" || candidate.eq_ignore_ascii_case(&extension))
        }) {
            return false;
        }
    }
    true
}

fn embed_lines(lines: &[String]) -> Result<Vec<TextChunk>, MlError> {
    let mut raw = Vec::<(usize, usize, String)>::new();
    let (mut first, mut last, mut count, mut text) = (0usize, 0usize, 0usize, String::new());
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        for ch in line.chars().chain(std::iter::once('\n')) {
            if count == CHUNK_CHARS {
                raw.push((first + 1, last + 1, std::mem::take(&mut text)));
                count = 0;
                first = i;
            }
            if count == 0 {
                first = i;
            }
            text.push(ch);
            count += 1;
            last = i;
        }
    }
    if !text.is_empty() {
        raw.push((first + 1, last + 1, text));
    }

    let mut bounded = Vec::with_capacity(raw.len());
    for (first, last, text) in raw {
        split_to_token_budget(first, last, text, &mut bounded)?;
    }
    let mut embedded = Vec::with_capacity(bounded.len());
    for batch in bounded.chunks(EMBED_BATCH) {
        let inputs = batch
            .iter()
            .map(|(_, _, text)| text.as_str())
            .collect::<Vec<_>>();
        let vectors = search_ml::embed_batch(&inputs)?;
        embedded.extend(batch.iter().zip(vectors).map(
            |((first_line, last_line, text), embedding)| TextChunk {
                first_line: *first_line,
                last_line: *last_line,
                text: text.clone(),
                embedding,
            },
        ));
    }
    Ok(embedded)
}

fn split_to_token_budget(
    first: usize,
    last: usize,
    text: String,
    output: &mut Vec<(usize, usize, String)>,
) -> Result<(), MlError> {
    if search_ml::token_count(&text)? <= CHUNK_TOKEN_BUDGET || text.chars().count() <= 1 {
        output.push((first, last, text));
        return Ok(());
    }
    let midpoint = text.chars().count() / 2;
    let split = text
        .char_indices()
        .nth(midpoint)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len());
    let (left, right) = text.split_at(split);
    split_to_token_budget(first, last, left.to_string(), output)?;
    split_to_token_budget(first, last, right.to_string(), output)
}

fn embedding_bytes(embedding: &Embedding) -> Vec<u8> {
    embedding
        .values()
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn embedding_from_bytes(bytes: &[u8]) -> Option<Embedding> {
    if bytes.len() != EMBEDDING_DIMENSIONS * std::mem::size_of::<f32>() {
        return None;
    }
    let values = bytes
        .chunks_exact(4)
        .map(|chunk| Some(f32::from_le_bytes(chunk.try_into().ok()?)))
        .collect::<Option<Vec<_>>>()?;
    Embedding::from_values(values).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_metadata(path: &Path) -> (i64, i64) {
        let metadata = std::fs::metadata(path).unwrap();
        let modified = crate::file_reader::system_time_to_local(metadata.modified().unwrap());
        (
            crate::cache::ticks_from_modified(modified),
            metadata.len() as i64,
        )
    }

    #[test]
    fn scoped_semantic_results_obey_root_extension_and_folder_filters() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("skip")).unwrap();
        let txt = dir.path().join("keep.txt");
        let rs = dir.path().join("keep.rs");
        let excluded = dir.path().join("skip").join("hidden.txt");
        for path in [&txt, &rs, &excluded] {
            std::fs::write(path, "text").unwrap();
        }
        let txt_metadata = source_metadata(&txt);
        let rs_metadata = source_metadata(&rs);
        let excluded_metadata = source_metadata(&excluded);
        let settings = crate::models::SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            extensions: Some(vec![".txt".into()]),
            exclude_folders: vec!["skip".into()],
            ..Default::default()
        };

        let allowed = vec![".txt".to_string()];
        assert!(semantic_path_allowed(
            txt.to_str().unwrap(),
            Some(&settings),
            Some(&allowed),
            txt_metadata.0,
            txt_metadata.1
        ));
        assert!(!semantic_path_allowed(
            rs.to_str().unwrap(),
            Some(&settings),
            Some(&allowed),
            rs_metadata.0,
            rs_metadata.1
        ));
        assert!(!semantic_path_allowed(
            excluded.to_str().unwrap(),
            Some(&settings),
            Some(&allowed),
            excluded_metadata.0,
            excluded_metadata.1
        ));
        assert!(!semantic_path_allowed(
            "/outside/keep.txt",
            Some(&settings),
            Some(&allowed),
            0,
            0
        ));
        let zero_size_limit = crate::models::SearchSettings {
            max_file_size_mb: 0.0,
            ..settings.clone()
        };
        assert!(!semantic_path_allowed(
            txt.to_str().unwrap(),
            Some(&zero_size_limit),
            Some(&allowed),
            txt_metadata.0,
            txt_metadata.1
        ));

        let all_extensions = crate::models::SearchSettings {
            extensions: Some(vec!["*".into()]),
            ..settings
        };
        assert!(semantic_path_allowed(
            rs.to_str().unwrap(),
            Some(&all_extensions),
            Some(&["*".to_string()]),
            rs_metadata.0,
            rs_metadata.1
        ));
    }

    #[test]
    fn semantic_index_persists_cited_results_and_leaves_exact_index_separate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("semantic.sqlite");
        let relevant = vec!["The shaft rotates under torque and torsional load.".to_string()];
        let unrelated = vec!["The annual tax policy changed after the election.".to_string()];
        let relevant_path = dir.path().join("shaft.txt");
        let unrelated_path = dir.path().join("tax.txt");
        std::fs::write(&relevant_path, relevant.join("\n")).unwrap();
        std::fs::write(&unrelated_path, unrelated.join("\n")).unwrap();
        let relevant_meta = source_metadata(&relevant_path);
        let unrelated_meta = source_metadata(&unrelated_path);
        let relevant_path = relevant_path.to_string_lossy().into_owned();
        let unrelated_path = unrelated_path.to_string_lossy().into_owned();
        {
            let index = SemanticIndex::open(&path).unwrap();
            index
                .index_document(&relevant_path, relevant_meta.0, relevant_meta.1, &relevant)
                .unwrap();
            index
                .index_document(
                    &unrelated_path,
                    unrelated_meta.0,
                    unrelated_meta.1,
                    &unrelated,
                )
                .unwrap();
            assert!(!index
                .is_current(&relevant_path, relevant_meta.0 - 1, relevant_meta.1)
                .unwrap());
            assert!(index
                .is_current(&relevant_path, relevant_meta.0, relevant_meta.1)
                .unwrap());
        }
        let index = SemanticIndex::open(&path).unwrap();
        let hits = index.search("rotational force", 1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, relevant_path);
        assert_eq!((hits[0].first_line, hits[0].last_line), (1, 1));
        assert!(hits[0].excerpt.contains("torsional load"));

        let similar_scope = crate::models::SearchSettings {
            search_path: dir.path().to_string_lossy().into_owned(),
            output_folder: dir.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let similar = index
            .search_excluding_path_with_settings(
                "rotational force",
                10,
                &similar_scope,
                &relevant_path,
            )
            .unwrap();
        assert!(similar.iter().all(|hit| hit.path != relevant_path));
        assert!(similar.iter().any(|hit| hit.path == unrelated_path));

        std::fs::write(
            &relevant_path,
            "changed content with a much larger size and no shaft or torque context",
        )
        .unwrap();
        assert!(index
            .search("rotational force", 10)
            .unwrap()
            .iter()
            .all(|hit| hit.path != relevant_path));
        assert!(index.search("", 10).is_err());
    }

    #[test]
    fn similar_document_grouping_keeps_one_strongest_cited_chunk_per_other_file() {
        let hits = vec![
            SemanticHit {
                path: "source.txt".into(),
                first_line: 1,
                last_line: 1,
                excerpt: "same source".into(),
                similarity: 1.0,
            },
            SemanticHit {
                path: "b.txt".into(),
                first_line: 1,
                last_line: 2,
                excerpt: "weaker".into(),
                similarity: 0.6,
            },
            SemanticHit {
                path: "b.txt".into(),
                first_line: 3,
                last_line: 4,
                excerpt: "stronger".into(),
                similarity: 0.8,
            },
            SemanticHit {
                path: "a.txt".into(),
                first_line: 5,
                last_line: 6,
                excerpt: "other file".into(),
                similarity: 0.7,
            },
        ];
        let grouped = group_similar_documents("source.txt", &hits, 50);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].path, "b.txt");
        assert_eq!(grouped[0].excerpt, "stronger");
        assert_eq!(grouped[1].path, "a.txt");
    }

    #[test]
    fn semantic_chunks_preserve_line_ranges_and_keep_limits() {
        let lines = (0..100)
            .map(|i| format!("line {i} {}", "engineering text ".repeat(20)))
            .collect::<Vec<_>>();
        let chunks = embed_lines(&lines).unwrap();
        assert!(chunks.len() > 1);
        assert_eq!(chunks.first().unwrap().first_line, 1);
        assert_eq!(chunks.last().unwrap().last_line, 100);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.text.chars().count() <= CHUNK_CHARS + 1));
    }

    #[test]
    fn searching_a_missing_sidecar_does_not_create_it() {
        let dir = tempfile::tempdir().unwrap();
        assert!(search_directory(dir.path(), "query", 10).is_err());
        assert!(!dir.path().join("semantic.sqlite").exists());
    }

    #[test]
    fn orphan_cleanup_removes_deleted_sources_and_keeps_live_sources() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.txt");
        let deleted = dir.path().join("deleted.txt");
        std::fs::write(&live, "torsional load on a shaft").unwrap();
        std::fs::write(&deleted, "torsional load on a shaft").unwrap();
        let index = SemanticIndex::open(&dir.path().join("semantic.sqlite")).unwrap();
        let lines = vec!["torsional load on a shaft".to_string()];
        let live_path = live.to_string_lossy().into_owned();
        let deleted_path = deleted.to_string_lossy().into_owned();
        index.index_document(&live_path, 1, 10, &lines).unwrap();
        index.index_document(&deleted_path, 1, 10, &lines).unwrap();
        std::fs::remove_file(&deleted).unwrap();

        assert_eq!(index.remove_orphaned_documents().unwrap(), 1);
        assert!(index.is_current(&live_path, 1, 10).unwrap());
        assert!(!index.is_current(&deleted_path, 1, 10).unwrap());
    }

    #[test]
    fn semantic_chunking_preserves_long_line_tail_and_bounds_token_count() {
        let mut line = "ordinary technical text ".repeat(100);
        line.push_str("tail_marker");
        let chunks = embed_lines(&[line]).unwrap();
        assert!(chunks
            .iter()
            .any(|chunk| chunk.text.contains("tail_marker")));
        assert!(chunks
            .iter()
            .all(|chunk| search_ml::token_count(&chunk.text).unwrap() <= CHUNK_TOKEN_BUDGET));
        assert!(chunks
            .iter()
            .all(|chunk| (chunk.first_line, chunk.last_line) == (1, 1)));
    }
}
