//! Persistent, local embedding index for optional semantic Search results.
//! Exact matching remains owned by `matching` and `native_index`.

use std::{
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, HashMap},
    path::{Path, PathBuf},
    sync::Mutex,
};

use rusqlite::{params, Connection, OptionalExtension};
use search_ml::{Embedding, MlError};

const EMBEDDING_VERSION: i64 = search_ml::EMBEDDING_MODEL_VERSION;
/// Per-PDF stamp: 1 = page citations, 2 = line quality repair and noise filter, 4 = hyphen-break repair.
const CITATION_VERSION: i64 = 4;
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
    pub page: Option<std::num::NonZeroU32>,
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
    page: Option<std::num::NonZeroU32>,
    embedding: Embedding,
}

fn ensure_column(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    let exists = {
        let mut statement = tx.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        columns.iter().any(|name| name == column)
    };
    if !exists {
        tx.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
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
    path: PathBuf,
    /// Above this many chunks a query streams from SQLite instead of using the in-memory snapshot.
    cache_limit: usize,
}

/// Chunks (about 1.5 KB of vector each) up to which an index is searched from memory (about 400 MB).
const MAX_CACHED_CHUNKS: usize = 250_000;

/// Record that the stored chunks changed, so cached snapshots of them are stale.
fn bump_generation(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO semantic_meta(key,value) VALUES('generation',1)
         ON CONFLICT(key) DO UPDATE SET value=value+1",
        [],
    )
    .map(|_| ())
}

/// SHA-256 of the extracted lines and their PDF pages: what the stored vectors were made from.
fn content_hash(lines: &[String], pages: Option<&[Option<std::num::NonZeroU32>]>) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for (index, line) in lines.iter().enumerate() {
        hasher.update((line.len() as u64).to_le_bytes());
        hasher.update(line.as_bytes());
        hasher.update(pages.and_then(|pages| pages[index]).map_or(0, |page| page.get()).to_le_bytes());
    }
    hasher.finalize().to_vec()
}

fn generation(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT value FROM semantic_meta WHERE key='generation'", [], |row| row.get(0))
        .optional()
        .map(|value| value.unwrap_or(0))
        .map_err(|error| error.to_string())
}

/// `true` when `path` holds a readable SQLite database that passes `PRAGMA quick_check`.
fn database_is_sound(path: &Path) -> bool {
    Connection::open(path)
        .and_then(|conn| conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0)))
        .map(|verdict| verdict == "ok")
        .unwrap_or(false)
}

impl SemanticIndex {
    /// Opens the index. Everything in it is derived from the source files, so a database that is damaged
    /// (not a database, truncated, failing `quick_check`) is deleted and recreated empty: the next index run
    /// re-embeds the documents instead of every semantic search failing.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if path.exists() && !database_is_sound(path) {
            tracing::warn!("semantic index is damaged; rebuilding it from the source files");
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let mut name = path.as_os_str().to_owned();
                name.push(suffix);
                let _ = std::fs::remove_file(std::path::PathBuf::from(name));
            }
        }
        Self::open_checked(path)
    }

    fn open_checked(path: &Path) -> rusqlite::Result<Self> {
        let mut conn = Connection::open(path)?;
        let tx = conn.transaction()?;
        tx.execute_batch(
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
        ensure_column(
            &tx,
            "semantic_documents",
            "citation_version",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(&tx, "semantic_chunks", "page", "INTEGER")?;
        ensure_column(&tx, "semantic_documents", "content_hash", "BLOB")?;
        tx.execute_batch("CREATE INDEX IF NOT EXISTS semantic_documents_hash ON semantic_documents(content_hash);")?;
        tx.commit()?;
        // SQLite ignores this pragma inside a transaction, so it must be set here: replacing or removing a document
        // relies on the cascade to drop its chunks.
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: Option<i64> = conn
            .query_row(
                "SELECT value FROM semantic_meta WHERE key = 'embedding_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if version != Some(EMBEDDING_VERSION) {
            conn.execute_batch("DELETE FROM semantic_chunks; DELETE FROM semantic_documents;")?;
            bump_generation(&conn)?;
            conn.execute(
                "INSERT INTO semantic_meta(key,value) VALUES('embedding_version',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [EMBEDDING_VERSION],
            )?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
            cache_limit: MAX_CACHED_CHUNKS,
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
        self.index_document_with_pages(path, modified, size, lines, None)
    }

    pub fn index_document_with_pages(
        &self,
        path: &str,
        modified: i64,
        size: i64,
        lines: &[String],
        pages: Option<&[Option<std::num::NonZeroU32>]>,
    ) -> Result<usize, String> {
        self.index_document_counting(path, modified, size, lines, pages).map(|(chunks, _)| chunks)
    }

    /// Like `index_document_with_pages`; also returns how many chunks went through the embedding model.
    fn index_document_counting(
        &self,
        path: &str,
        modified: i64,
        size: i64,
        lines: &[String],
        pages: Option<&[Option<std::num::NonZeroU32>]>,
    ) -> Result<(usize, usize), String> {
        let pages = pages.filter(|pages| pages.len() == lines.len());
        let citation_version = if Path::new(path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            CITATION_VERSION
        } else {
            0
        };
        // Embedding is the expensive step. A file whose time changed but whose extracted text did not (a copy, a
        // checkout, a sync tool) keeps its vectors and only gets its new stamp.
        let hash = content_hash(lines, pages);
        {
            let conn = self
                .conn
                .lock()
                .map_err(|_| "semantic index lock poisoned".to_string())?;
            let stored: Option<(Option<Vec<u8>>, i64)> = conn
                .query_row(
                    "SELECT content_hash,citation_version FROM semantic_documents WHERE path=?1",
                    [path],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if stored.is_some_and(|(stored_hash, version)| {
                version == citation_version && stored_hash.as_deref() == Some(hash.as_slice())
            }) {
                conn.execute(
                    "UPDATE semantic_documents SET modified=?2,size=?3 WHERE path=?1",
                    params![path, modified, size],
                )
                .map_err(|error| error.to_string())?;
                return conn
                    .query_row(
                        "SELECT COUNT(*) FROM semantic_chunks WHERE path=?1",
                        [path],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|count| (count as usize, 0))
                    .map_err(|error| error.to_string());
            }
        }
        // A file that grew or was edited in one place still holds most of its old text: chunks whose text is
        // unchanged keep their stored vectors and only the new ones go through the model.
        let mut known = self.stored_chunk_embeddings(path)?;
        // The same text under another path (a copy, a renamed file, a second revision folder) was embedded already.
        if let Some(donor) = self.donor_path(path, &hash, citation_version)? {
            known.extend(self.stored_chunk_embeddings(&donor)?);
        }
        let (chunks, embedded) = embed_lines_with_pages(lines, pages, &known).map_err(|error| error.to_string())?;
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| "semantic index lock poisoned".to_string())?;
        let tx = conn.transaction().map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM semantic_documents WHERE path=?1", [path])
            .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO semantic_documents(path,modified,size,citation_version,content_hash) VALUES(?1,?2,?3,?4,?5)",
            params![path, modified, size, citation_version, hash.as_slice()],
        )
        .map_err(|error| error.to_string())?;
        for (chunk_index, chunk) in chunks.iter().enumerate() {
            let vector = embedding_bytes(&chunk.embedding);
            tx.execute(
                "INSERT INTO semantic_chunks(path,chunk_index,first_line,last_line,excerpt,embedding,page) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![path, chunk_index as i64, chunk.first_line as i64, chunk.last_line as i64, chunk.text, vector, chunk.page.map(|page| page.get() as i64)],
            ).map_err(|error| error.to_string())?;
        }
        bump_generation(&tx).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok((chunks.len(), embedded))
    }

    /// Another indexed document made from exactly the same extracted text (and pages), if any.
    fn donor_path(&self, path: &str, hash: &[u8], citation_version: i64) -> Result<Option<String>, String> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| "semantic index lock poisoned".to_string())?;
        conn.query_row(
            "SELECT path FROM semantic_documents WHERE content_hash=?1 AND citation_version=?2 AND path<>?3 LIMIT 1",
            params![hash, citation_version, path],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
    }

    /// The embeddings stored for `path`, by chunk text.
    fn stored_chunk_embeddings(&self, path: &str) -> Result<HashMap<String, Embedding>, String> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| "semantic index lock poisoned".to_string())?;
        let mut stmt = conn
            .prepare("SELECT excerpt,embedding FROM semantic_chunks WHERE path=?1")
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([path], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))
            .map_err(|error| error.to_string())?;
        let mut known = HashMap::new();
        for row in rows {
            let (text, bytes) = row.map_err(|error| error.to_string())?;
            if let Some(embedding) = embedding_from_bytes(&bytes) {
                known.insert(text, embedding);
            }
        }
        Ok(known)
    }

    pub fn is_current(&self, path: &str, modified: i64, size: i64) -> rusqlite::Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let citation_version = if Path::new(path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            CITATION_VERSION
        } else {
            0
        };
        let stored: Option<(i64, i64, i64)> = conn
            .query_row(
                "SELECT modified,size,citation_version FROM semantic_documents WHERE path=?1",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(stored == Some((modified, size, citation_version)))
    }

    pub fn remove_document(&self, path: &str) -> rusqlite::Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        conn.execute("DELETE FROM semantic_documents WHERE path=?1", [path])?;
        bump_generation(&conn)?;
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
        bump_generation(&tx).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(orphaned.len())
    }

    /// Semantic results are separate, cited suggestions. This method never
    /// changes the exact-search result set or query semantics.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SemanticHit>, String> {
        self.search_filtered(query, limit, None, None, true, &crate::lexical::LexicalConfig::default())
    }

    pub fn search_with_settings(
        &self,
        query: &str,
        limit: usize,
        settings: &crate::models::SearchSettings,
    ) -> Result<Vec<SemanticHit>, String> {
        self.search_filtered(query, limit, Some(settings), None, true, &crate::lexical::LexicalConfig::default())
    }

    pub fn search_excluding_path_with_settings(
        &self,
        query: &str,
        limit: usize,
        settings: &crate::models::SearchSettings,
        excluded_path: &str,
    ) -> Result<Vec<SemanticHit>, String> {
        self.search_filtered(query, limit, Some(settings), Some(excluded_path), true, &crate::lexical::LexicalConfig::default())
    }

    /// Search under explicit keyword and fusion settings (`hybrid` off is similarity alone). Search itself uses
    /// the defaults; this is for measuring a change on a benchmark before adopting it.
    pub fn search_tuned(
        &self,
        query: &str,
        limit: usize,
        hybrid: bool,
        config: &crate::lexical::LexicalConfig,
    ) -> Result<Vec<SemanticHit>, String> {
        self.search_filtered(query, limit, None, None, hybrid, config)
    }

    /// [`Self::search_tuned`] with the query vector given (for measuring feedback or re-weighted queries); `query`
    /// still feeds the keyword side.
    pub fn search_tuned_embedded(
        &self,
        query: &str,
        embedding: &Embedding,
        limit: usize,
        hybrid: bool,
        config: &crate::lexical::LexicalConfig,
    ) -> Result<Vec<SemanticHit>, String> {
        if let Some(hits) = self.search_snapshot(query, embedding, limit, None, None, hybrid, config)? {
            return Ok(hits);
        }
        self.search_streaming(query, embedding, limit, None, None, hybrid, config)
    }

    /// Every chunk's raw scores for `query` (similarity, BM25 over all chunks, length, place in its file), unranked.
    /// For measuring ranking ideas offline on a benchmark (`examples/support/ranking_experiments.rs`); no extension or
    /// freshness filtering. `None` when the index is too large for a snapshot.
    pub fn rank_features(&self, query: &str, stem: bool) -> Result<Option<crate::ranking::FeatureSet>, String> {
        let Some(embedding) = search_ml::embed_queries(&[query]).map_err(|e| e.to_string())?.pop() else {
            return Err("semantic model returned no embedding".into());
        };
        let conn = self.conn.lock().map_err(|_| "semantic index lock poisoned".to_string())?;
        let generation = generation(&conn)?;
        let snapshot = match crate::semantic_cache::cached(&self.path, stem, generation) {
            Some(snapshot) => snapshot,
            None => {
                let snapshot = std::sync::Arc::new(crate::semantic_cache::Snapshot::build(&conn, generation, stem, EMBEDDING_DIMENSIONS)?);
                crate::semantic_cache::store(&self.path, stem, snapshot.clone());
                snapshot
            }
        };
        Ok(crate::ranking::feature_set(&snapshot, embedding.values(), query, stem))
    }

    fn search_filtered(
        &self,
        query: &str,
        limit: usize,
        settings: Option<&crate::models::SearchSettings>,
        excluded_path: Option<&str>,
        hybrid: bool,
        config: &crate::lexical::LexicalConfig,
    ) -> Result<Vec<SemanticHit>, String> {
        if query.trim().is_empty() {
            return Err("semantic query must not be empty".into());
        }
        let Some(query_embedding) = search_ml::embed_queries(&[query])
            .map_err(|error| error.to_string())?
            .pop()
        else {
            return Err("semantic model returned no embedding".into());
        };
        if let Some(hits) = self.search_snapshot(query, &query_embedding, limit, settings, excluded_path, hybrid, config)? {
            return Ok(hits);
        }
        self.search_streaming(query, &query_embedding, limit, settings, excluded_path, hybrid, config)
    }

    /// Search from the in-memory snapshot (rebuilt when the index changed); `None` when the index is too large for one.
    #[allow(clippy::too_many_arguments)]
    fn search_snapshot(
        &self,
        query: &str,
        query_embedding: &Embedding,
        limit: usize,
        settings: Option<&crate::models::SearchSettings>,
        excluded_path: Option<&str>,
        hybrid: bool,
        config: &crate::lexical::LexicalConfig,
    ) -> Result<Option<Vec<SemanticHit>>, String> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| "semantic index lock poisoned".to_string())?;
        // The generation is read before the rows: a write in between leaves a snapshot that is stale, never wrong.
        let generation = generation(&conn)?;
        let snapshot = match crate::semantic_cache::cached(&self.path, config.stem, generation) {
            Some(snapshot) => snapshot,
            None => {
                let chunks: i64 = conn
                    .query_row("SELECT COUNT(*) FROM semantic_chunks", [], |row| row.get(0))
                    .map_err(|error| error.to_string())?;
                if chunks as usize > self.cache_limit {
                    return Ok(None);
                }
                let snapshot = std::sync::Arc::new(crate::semantic_cache::Snapshot::build(
                    &conn,
                    generation,
                    config.stem,
                    EMBEDDING_DIMENSIONS,
                )?);
                crate::semantic_cache::store(&self.path, config.stem, snapshot.clone());
                snapshot
            }
        };
        let query_values = query_embedding.values();
        if query_values.len() != EMBEDDING_DIMENSIONS {
            return Ok(Some(Vec::new()));
        }
        let allowed_extensions = settings.map(|settings| {
            settings
                .extensions
                .clone()
                .unwrap_or_else(crate::models::extension_catalog::all_extensions)
        });
        let mut allowed: Vec<bool> = snapshot
            .docs
            .iter()
            .map(|doc| {
                excluded_path != Some(doc.path.as_str())
                    && semantic_path_listed(&doc.path, settings, allowed_extensions.as_deref(), doc.size)
            })
            .collect();
        // One stat per listed file, spread over the cores: a large index otherwise pays it serially on every query.
        let listed: Vec<usize> = (0..allowed.len()).filter(|&i| allowed[i]).collect();
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
        let chunk = listed.len().div_ceil(threads.max(1)).max(256);
        let fresh: Vec<bool> = std::thread::scope(|scope| {
            let handles: Vec<_> = listed
                .chunks(chunk)
                .map(|part| {
                    let docs = &snapshot.docs;
                    scope.spawn(move || {
                        part.iter()
                            .map(|&i| semantic_path_fresh(&docs[i].path, docs[i].modified, docs[i].size))
                            .collect::<Vec<bool>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        for (&i, ok) in listed.iter().zip(fresh) {
            allowed[i] = ok;
        }
        let mut candidates: Vec<u32> = Vec::new();
        let mut dense: Vec<f32> = Vec::new();
        for chunk in 0..snapshot.len() {
            if !allowed[snapshot.doc_of[chunk] as usize] {
                continue;
            }
            let similarity = snapshot.similarity(query_values, chunk);
            if similarity.is_finite() {
                candidates.push(chunk as u32);
                dense.push(similarity);
            }
        }
        let limit = limit.min(1000);
        let lexical_terms = if hybrid { crate::lexical::query_terms_with(query, config.stem) } else { None };
        let mut order: Vec<usize> = (0..candidates.len()).collect();
        if let Some(terms) = &lexical_terms {
            let lexical = snapshot.bm25(terms, &candidates, config.bm25_k1, config.bm25_b);
            let scores = crate::lexical::fuse_with(&dense, &lexical, config);
            order.sort_by(|&a, &b| {
                scores[b]
                    .total_cmp(&scores[a])
                    .then_with(|| dense[b].total_cmp(&dense[a]))
                    .then_with(|| snapshot.rowids[candidates[a] as usize].cmp(&snapshot.rowids[candidates[b] as usize]))
            });
        } else {
            order.sort_by(|&a, &b| {
                let (ca, cb) = (candidates[a] as usize, candidates[b] as usize);
                dense[b]
                    .total_cmp(&dense[a])
                    .then_with(|| snapshot.docs[snapshot.doc_of[ca] as usize].path.cmp(&snapshot.docs[snapshot.doc_of[cb] as usize].path))
                    .then_with(|| snapshot.first_lines[ca].cmp(&snapshot.first_lines[cb]))
            });
        }
        order.truncate(limit);
        let winners: Vec<(i64, f32)> = order
            .into_iter()
            .map(|i| (snapshot.rowids[candidates[i] as usize], dense[i]))
            .collect();
        load_hits(&conn, &winners).map(Some)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_streaming(
        &self,
        query: &str,
        query_embedding: &Embedding,
        limit: usize,
        settings: Option<&crate::models::SearchSettings>,
        excluded_path: Option<&str>,
        hybrid: bool,
        config: &crate::lexical::LexicalConfig,
    ) -> Result<Vec<SemanticHit>, String> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| "semantic index lock poisoned".to_string())?;
        let mut stmt = conn
            .prepare("SELECT c.path,c.first_line,c.last_line,c.excerpt,c.page,c.embedding,d.modified,d.size,c.rowid FROM semantic_chunks c JOIN semantic_documents d ON d.path=c.path ORDER BY c.path,c.chunk_index")
            .map_err(|error| error.to_string())?;
        let mut rows = stmt.query([]).map_err(|error| error.to_string())?;
        let mut best = BinaryHeap::<Reverse<HeapHit>>::new();
        // Short queries also get keyword scoring fused with similarity; the
        // scan keeps compact per-chunk counts, then fetches only the winners.
        let lexical_terms = if hybrid { crate::lexical::query_terms_with(query, config.stem) } else { None };
        let mut fused = Vec::<FusedCandidate>::new();
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
                        row.get(6).map_err(|error| error.to_string())?,
                        row.get(7).map_err(|error| error.to_string())?,
                    );
                current_path = path.clone();
            }
            if !current_path_allowed {
                continue;
            }
            let bytes: Vec<u8> = row.get(5).map_err(|error| error.to_string())?;
            let Some(embedding) = embedding_from_bytes(&bytes) else {
                continue;
            };
            let similarity = search_ml::cosine_similarity(query_embedding, &embedding)
                .unwrap_or(f32::NEG_INFINITY);
            if !similarity.is_finite() {
                continue;
            }
            if let Some(terms) = &lexical_terms {
                let excerpt: String = row.get(3).map_err(|error| error.to_string())?;
                let (counts, length) = crate::lexical::count_terms_with(&excerpt, terms, config.stem);
                fused.push(FusedCandidate {
                    rowid: row.get(8).map_err(|error| error.to_string())?,
                    similarity,
                    counts,
                    length,
                });
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
                page: row
                    .get::<_, Option<i64>>(4)
                    .map_err(|error| error.to_string())?
                    .and_then(|page| u32::try_from(page).ok())
                    .and_then(std::num::NonZeroU32::new),
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
        if lexical_terms.is_some() {
            drop(rows);
            drop(stmt);
            return fuse_candidates(&conn, fused, lexical_terms.as_deref().map_or(0, <[_]>::len), limit, config);
        }
        Ok(best
            .into_sorted_vec()
            .into_iter()
            .map(|Reverse(hit)| hit.0)
            .collect())
    }
}

/// Compact per-chunk record kept while scanning for fused ranking.
struct FusedCandidate {
    rowid: i64,
    similarity: f32,
    counts: Vec<u16>,
    length: u32,
}

/// Rank by reciprocal-rank fusion of similarity and BM25, then load only the
/// winners. `SemanticHit::similarity` stays the cosine similarity.
fn fuse_candidates(
    conn: &Connection,
    candidates: Vec<FusedCandidate>,
    query_len: usize,
    limit: usize,
    config: &crate::lexical::LexicalConfig,
) -> Result<Vec<SemanticHit>, String> {
    let dense = candidates.iter().map(|c| c.similarity).collect::<Vec<_>>();
    let documents = candidates
        .iter()
        .map(|c| (c.counts.clone(), c.length))
        .collect::<Vec<_>>();
    let scores = crate::lexical::fuse_with(&dense, &crate::lexical::bm25_with(&documents, query_len, config.bm25_k1, config.bm25_b), config);
    let mut order = (0..candidates.len()).collect::<Vec<_>>();
    order.sort_by(|&a, &b| {
        scores[b]
            .total_cmp(&scores[a])
            .then_with(|| candidates[b].similarity.total_cmp(&candidates[a].similarity))
            .then_with(|| candidates[a].rowid.cmp(&candidates[b].rowid))
    });
    order.truncate(limit);
    let winners: Vec<(i64, f32)> = order.into_iter().map(|i| (candidates[i].rowid, candidates[i].similarity)).collect();
    load_hits(conn, &winners)
}

/// Load the cited chunks for `(rowid, cosine similarity)` winners, in the given order.
fn load_hits(conn: &Connection, winners: &[(i64, f32)]) -> Result<Vec<SemanticHit>, String> {
    let mut stmt = conn
        .prepare("SELECT path,first_line,last_line,excerpt,page FROM semantic_chunks WHERE rowid=?1")
        .map_err(|error| error.to_string())?;
    winners
        .iter()
        .map(|&(rowid, similarity)| {
            stmt.query_row(params![rowid], |row| {
                Ok(SemanticHit {
                    path: row.get(0)?,
                    first_line: row.get::<_, i64>(1)?.max(0) as usize,
                    last_line: row.get::<_, i64>(2)?.max(0) as usize,
                    excerpt: row.get(3)?,
                    similarity,
                    page: row
                        .get::<_, Option<i64>>(4)?
                        .and_then(|page| u32::try_from(page).ok())
                        .and_then(std::num::NonZeroU32::new),
                })
            })
            .map_err(|error| error.to_string())
        })
        .collect()
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
    semantic_path_listed(path, settings, allowed_extensions, indexed_size)
        && semantic_path_fresh(path, indexed_modified, indexed_size)
}

/// The indexed file still has the size and time it was indexed with (one `stat`).
fn semantic_path_fresh(path: &str, indexed_modified: i64, indexed_size: i64) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    metadata.len() as i64 == indexed_size
        && crate::cache::ticks_from_modified(crate::file_reader::system_time_to_local(modified))
            == indexed_modified
}

/// The scope checks that need no file access (folder, hidden, size limit, extension). The size limit reads the indexed
/// size: a file whose size changed fails `semantic_path_fresh` anyway.
fn semantic_path_listed(
    path: &str,
    settings: Option<&crate::models::SearchSettings>,
    allowed_extensions: Option<&[String]>,
    indexed_size: i64,
) -> bool {
    let path = Path::new(path);
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
        if indexed_size > max_bytes {
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

#[cfg(test)]
fn embed_lines(lines: &[String]) -> Result<Vec<TextChunk>, MlError> {
    embed_lines_with_pages(lines, None, &HashMap::new()).map(|(chunks, _)| chunks)
}

/// Chunk `lines` and embed every chunk whose text is not in `known`; also returns how many went through the model.
fn embed_lines_with_pages(
    lines: &[String],
    pages: Option<&[Option<std::num::NonZeroU32>]>,
    known: &HashMap<String, Embedding>,
) -> Result<(Vec<TextChunk>, usize), MlError> {
    type RawChunk = (usize, usize, Option<std::num::NonZeroU32>, String);
    let mut raw = Vec::<RawChunk>::new();
    let (mut first, mut last, mut count, mut page, mut text) =
        (0usize, 0usize, 0usize, None, String::new());
    for (i, line) in lines.iter().enumerate() {
        let line_page = pages.and_then(|pages| pages.get(i)).copied().flatten();
        if count > 0 && page != line_page {
            raw.push((first + 1, last + 1, page, std::mem::take(&mut text)));
            count = 0;
        }
        if line.trim().is_empty() {
            continue;
        }
        for ch in line.chars().chain(std::iter::once('\n')) {
            if count == CHUNK_CHARS {
                raw.push((first + 1, last + 1, page, std::mem::take(&mut text)));
                count = 0;
            }
            if count == 0 {
                first = i;
                page = line_page;
            }
            text.push(ch);
            count += 1;
            last = i;
        }
    }
    if !text.is_empty() {
        raw.push((first + 1, last + 1, page, text));
    }

    let mut bounded = Vec::with_capacity(raw.len());
    for (first, last, page, text) in raw {
        split_to_token_budget(first, last, page, text, &mut bounded)?;
    }
    let mut embedded: Vec<Option<TextChunk>> = bounded
        .iter()
        .map(|(first_line, last_line, page, text)| {
            known.get(text).map(|embedding| TextChunk {
                first_line: *first_line,
                last_line: *last_line,
                page: *page,
                text: text.clone(),
                embedding: embedding.clone(),
            })
        })
        .collect();
    let pending: Vec<usize> = (0..bounded.len()).filter(|&i| embedded[i].is_none()).collect();
    for batch in pending.chunks(EMBED_BATCH) {
        let inputs = batch.iter().map(|&i| bounded[i].3.as_str()).collect::<Vec<_>>();
        let vectors = search_ml::embed_batch(&inputs)?;
        for (&i, embedding) in batch.iter().zip(vectors) {
            let (first_line, last_line, page, text) = &bounded[i];
            embedded[i] = Some(TextChunk {
                first_line: *first_line,
                last_line: *last_line,
                page: *page,
                text: text.clone(),
                embedding,
            });
        }
    }
    Ok((embedded.into_iter().flatten().collect(), pending.len()))
}

fn split_to_token_budget(
    first: usize,
    last: usize,
    page: Option<std::num::NonZeroU32>,
    text: String,
    output: &mut Vec<(usize, usize, Option<std::num::NonZeroU32>, String)>,
) -> Result<(), MlError> {
    if search_ml::token_count(&text)? <= CHUNK_TOKEN_BUDGET || text.chars().count() <= 1 {
        output.push((first, last, page, text));
        return Ok(());
    }
    let midpoint = text.chars().count() / 2;
    let split = text
        .char_indices()
        .nth(midpoint)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len());
    let (left, right) = text.split_at(split);
    split_to_token_budget(first, last, page, left.to_string(), output)?;
    split_to_token_budget(first, last, page, right.to_string(), output)
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
                page: None,
            },
            SemanticHit {
                path: "b.txt".into(),
                first_line: 1,
                last_line: 2,
                excerpt: "weaker".into(),
                similarity: 0.6,
                page: None,
            },
            SemanticHit {
                path: "b.txt".into(),
                first_line: 3,
                last_line: 4,
                excerpt: "stronger".into(),
                similarity: 0.8,
                page: None,
            },
            SemanticHit {
                path: "a.txt".into(),
                first_line: 5,
                last_line: 6,
                excerpt: "other file".into(),
                similarity: 0.7,
                page: None,
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

    #[test]
    fn semantic_chunks_never_mix_verified_pdf_pages() {
        let lines = [
            "Page one technical content".to_string(),
            "More page one content".to_string(),
            "Page two technical content".to_string(),
            "More page two content".to_string(),
        ];
        let pages = [
            Some(std::num::NonZeroU32::new(1).unwrap()),
            Some(std::num::NonZeroU32::new(1).unwrap()),
            Some(std::num::NonZeroU32::new(2).unwrap()),
            Some(std::num::NonZeroU32::new(2).unwrap()),
        ];
        let (chunks, _) = embed_lines_with_pages(&lines, Some(&pages), &HashMap::new()).unwrap();

        assert!(chunks.iter().all(|chunk| {
            let page_for_line = |line| if line <= 2 { 1 } else { 2 };
            chunk.page.map(std::num::NonZeroU32::get) == Some(page_for_line(chunk.first_line))
                && chunk.page.map(std::num::NonZeroU32::get) == Some(page_for_line(chunk.last_line))
        }));
        assert!(chunks.iter().any(|chunk| chunk.page.unwrap().get() == 1));
        assert!(chunks.iter().any(|chunk| chunk.page.unwrap().get() == 2));
    }

    /// The in-memory snapshot, the streaming scan and a changed index all agree: same hits in the same order for
    /// dense and fused queries, a write shows up in the next query, and a removal disappears from it.
    #[test]
    fn an_appended_file_embeds_only_the_new_chunks() {
        let mut lines: Vec<String> = (0..40).map(|i| format!("Log entry {i}: torque check passed at station {i}.")).collect();
        let (all, first) = embed_lines_with_pages(&lines, None, &HashMap::new()).unwrap();
        assert!(first == all.len() && all.len() > 1);
        let known: HashMap<String, Embedding> = all.iter().map(|c| (c.text.clone(), c.embedding.clone())).collect();
        // Nothing changed: no model call, identical chunks.
        let (same, calls) = embed_lines_with_pages(&lines, None, &known).unwrap();
        assert_eq!(calls, 0);
        assert_eq!(same.len(), all.len());
        // A line appended: the old chunks are reused and only the tail is embedded.
        lines.push("Log entry 40: new line at the end.".into());
        let (grown, calls) = embed_lines_with_pages(&lines, None, &known).unwrap();
        assert!(calls >= 1 && calls < grown.len(), "{calls} of {}", grown.len());
        assert_eq!(grown[0].embedding.values(), all[0].embedding.values());
    }

    fn chunk_rows(index: &SemanticIndex, path: &str) -> i64 {
        index
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM semantic_chunks WHERE path=?1", [path], |row| row.get(0))
            .unwrap()
    }

    fn log_lines(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("Log entry {i}: torque check passed at station {i}.")).collect()
    }

    /// Re-indexing embeds only what changed; a copy under another path embeds nothing; replacing or removing a
    /// document leaves no chunk rows behind.
    #[test]
    fn incremental_updates_embed_only_changes_and_leave_no_orphan_rows() {
        let dir = tempfile::tempdir().unwrap();
        let index = SemanticIndex::open(&dir.path().join("semantic.sqlite")).unwrap();
        let mut lines = log_lines(40);
        let (total, embedded) = index.index_document_counting("a.txt", 1, 100, &lines, None).unwrap();
        assert!(total > 1 && embedded == total);
        // Same text, new time and size: stamp only.
        assert_eq!(index.index_document_counting("a.txt", 2, 101, &lines, None).unwrap(), (total, 0));
        assert!(index.is_current("a.txt", 2, 101).unwrap() && !index.is_current("a.txt", 1, 100).unwrap());
        // A line appended: the unchanged leading chunks keep their vectors.
        lines.push("Log entry 40: new line at the end.".into());
        let (grown, embedded) = index.index_document_counting("a.txt", 3, 150, &lines, None).unwrap();
        assert!(embedded >= 1 && embedded < grown, "{embedded} of {grown}");
        assert_eq!(chunk_rows(&index, "a.txt"), grown as i64);
        // The same text under another path (copy or rename): no model call.
        let (copied, embedded) = index.index_document_counting("copy/b.txt", 3, 150, &lines, None).unwrap();
        assert_eq!((copied, embedded), (grown, 0));
        // Replaced by shorter text: the old rows are gone, not merged.
        let (short, _) = index.index_document_counting("a.txt", 4, 20, &lines[..3], None).unwrap();
        assert_eq!(chunk_rows(&index, "a.txt"), short as i64);
        // Deleted source files lose their rows; live ones keep theirs.
        let live = dir.path().join("live.txt");
        std::fs::write(&live, "x").unwrap();
        index.index_document(&live.to_string_lossy(), 1, 1, &lines[..3]).unwrap();
        assert_eq!(index.remove_orphaned_documents().unwrap(), 2);
        assert_eq!(chunk_rows(&index, "a.txt") + chunk_rows(&index, "copy/b.txt"), 0);
        assert!(chunk_rows(&index, &live.to_string_lossy()) > 0);
        let stray: i64 = index
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM semantic_chunks WHERE path NOT IN (SELECT path FROM semantic_documents)", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stray, 0);
    }

    /// A build that stops half way keeps the finished documents; the next run skips them and embeds only the rest.
    #[test]
    fn an_interrupted_build_resumes_without_redoing_finished_documents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("semantic.sqlite");
        let docs: Vec<(String, Vec<String>)> =
            (0..6).map(|n| (format!("doc{n}.txt"), vec![format!("Document {n} describes fastener preload case {n}.")])).collect();
        {
            let index = SemanticIndex::open(&path).unwrap();
            for (name, lines) in &docs[..3] {
                index.index_document(name, 1, 10, lines).unwrap();
            }
        } // process ends here
        let index = SemanticIndex::open(&path).unwrap();
        let mut embedded = 0;
        for (name, lines) in &docs {
            if index.is_current(name, 1, 10).unwrap() {
                continue;
            }
            embedded += index.index_document_counting(name, 1, 10, lines, None).unwrap().1;
        }
        assert_eq!(embedded, 3);
        assert!(docs.iter().all(|(name, _)| index.is_current(name, 1, 10).unwrap()));
    }

    #[test]
    fn unchanged_text_with_a_new_time_keeps_its_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let index = SemanticIndex::open(&dir.path().join("semantic.sqlite")).unwrap();
        let lines = vec!["Torque the M8x1.25 bolt to 25 newton metres.".to_string()];
        let generation_of = |index: &SemanticIndex| generation(&index.conn.lock().unwrap()).unwrap();
        index.index_document("a.txt", 100, 40, &lines).unwrap();
        let written = generation_of(&index);
        // Touched, same text: stamp moves, no re-embedding (the write counter stays put), still searchable.
        assert_eq!(index.index_document("a.txt", 200, 40, &lines).unwrap(), 1);
        assert_eq!(generation_of(&index), written);
        assert!(index.is_current("a.txt", 200, 40).unwrap());
        assert!(!index.is_current("a.txt", 100, 40).unwrap());
        // Different text is embedded again.
        let edited = vec!["Torque the M10 bolt to 49 newton metres.".to_string()];
        index.index_document("a.txt", 300, 41, &edited).unwrap();
        assert!(generation_of(&index) > written);
        assert!(index.is_current("a.txt", 300, 41).unwrap());
    }

    #[test]
    fn snapshot_scores_equal_streaming_scores_and_follow_writes() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = SemanticIndex::open(&dir.path().join("semantic.sqlite")).unwrap();
        let add = |index: &SemanticIndex, name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            let (modified, size) = source_metadata(&path);
            index.index_document(&path.to_string_lossy(), modified, size, &[text.to_string()]).unwrap();
            path.to_string_lossy().into_owned()
        };
        add(&index, "a.txt", "Torque the M8x1.25 bolt to 25 newton metres.");
        add(&index, "b.txt", "The shaft experiences torsional stress under load.");
        add(&index, "c.txt", "Corrosion was blended out and the surface filled.");
        let config = crate::lexical::LexicalConfig::default();
        let same = |index: &mut SemanticIndex, query: &str| {
            for hybrid in [true, false] {
                index.cache_limit = MAX_CACHED_CHUNKS;
                let cached = index.search_tuned(query, 10, hybrid, &config).unwrap();
                index.cache_limit = 0;
                let streamed = index.search_tuned(query, 10, hybrid, &config).unwrap();
                assert_eq!(cached.len(), streamed.len(), "{query} hybrid={hybrid}");
                for (c, s) in cached.iter().zip(&streamed) {
                    assert_eq!((&c.path, c.first_line), (&s.path, s.first_line), "{query} hybrid={hybrid}");
                    assert!((c.similarity - s.similarity).abs() < 1e-5, "{query}: {} vs {}", c.similarity, s.similarity);
                }
            }
            index.cache_limit = MAX_CACHED_CHUNKS;
        };
        for query in ["M8 bolt torque", "shaft stress", "blended corrosion", "newton"] {
            same(&mut index, query);
        }
        // A write is visible to the next query, a removal too.
        let d = add(&index, "d.txt", "Hydrostatic test pressure was recorded.");
        same(&mut index, "hydrostatic pressure");
        assert!(index.search("hydrostatic pressure", 1).unwrap()[0].path.ends_with("d.txt"));
        index.remove_document(&d).unwrap();
        assert!(index.search("hydrostatic pressure", 5).unwrap().iter().all(|hit| !hit.path.ends_with("d.txt")));
    }

    #[test]
    fn embedding_model_version_mismatch_discards_stored_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("semantic.sqlite");
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "shaft torque").unwrap();
        let (modified, size) = source_metadata(&file);
        let file = file.to_string_lossy().into_owned();
        {
            let index = SemanticIndex::open(&path).unwrap();
            index
                .index_document(&file, modified, size, &["shaft torque".to_string()])
                .unwrap();
            assert!(index.is_current(&file, modified, size).unwrap());
        }
        // Same model: vectors survive a reopen.
        assert!(SemanticIndex::open(&path)
            .unwrap()
            .is_current(&file, modified, size)
            .unwrap());
        // Vectors from another model are incomparable: reopen must drop them.
        Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE semantic_meta SET value=?1 WHERE key='embedding_version'",
                [EMBEDDING_VERSION + 1],
            )
            .unwrap();
        let index = SemanticIndex::open(&path).unwrap();
        assert!(!index.is_current(&file, modified, size).unwrap());
        assert!(index.search("shaft", 5).unwrap().is_empty());
    }

    #[test]
    fn old_semantic_schema_migrates_without_discarding_documents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("semantic.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE semantic_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             INSERT INTO semantic_meta(key,value) VALUES('embedding_version',1);
             CREATE TABLE semantic_documents (path TEXT PRIMARY KEY, modified INTEGER NOT NULL, size INTEGER NOT NULL);
             CREATE TABLE semantic_chunks (
                 path TEXT NOT NULL REFERENCES semantic_documents(path) ON DELETE CASCADE,
                 chunk_index INTEGER NOT NULL, first_line INTEGER NOT NULL, last_line INTEGER NOT NULL,
                 excerpt TEXT NOT NULL, embedding BLOB NOT NULL, PRIMARY KEY(path,chunk_index)
             );
             INSERT INTO semantic_documents(path,modified,size) VALUES('legacy.pdf',7,9);
             INSERT INTO semantic_chunks(path,chunk_index,first_line,last_line,excerpt,embedding)
                 VALUES('legacy.pdf',0,1,1,'legacy excerpt',x'00');",
        )
        .unwrap();
        drop(conn);

        let index = SemanticIndex::open(&path).unwrap();
        assert!(!index.is_current("legacy.pdf", 7, 9).unwrap());
        drop(index);

        let conn = Connection::open(path).unwrap();
        let (documents, chunks): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM semantic_documents), (SELECT COUNT(*) FROM semantic_chunks)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((documents, chunks), (1, 1));
    }

    /// Hard retrieval set (`search-ml/examples/data/retrieval_eval.tsv`): near-duplicate
    /// part numbers, bare numbers, spec codes, negation. Measured on the bundled
    /// MiniLM: dense 34/40 top-1, fused 37/40 (lexical weight 1.5-3 all give 37).
    #[test]
    fn a_damaged_database_is_rebuilt_empty_and_a_sound_one_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("semantic.sqlite");
        {
            let index = SemanticIndex::open(&path).unwrap();
            index.conn.lock().unwrap().execute("INSERT INTO semantic_documents(path,modified,size) VALUES('a',1,1)", []).unwrap();
        }
        let kept = SemanticIndex::open(&path).unwrap();
        let count = |i: &SemanticIndex| -> i64 {
            i.conn.lock().unwrap().query_row("SELECT COUNT(*) FROM semantic_documents", [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count(&kept), 1, "a sound index is not touched");
        drop(kept);

        std::fs::write(&path, b"this is not a sqlite database at all, just damaged bytes").unwrap();
        let rebuilt = SemanticIndex::open(&path).unwrap();
        assert_eq!(count(&rebuilt), 0, "a damaged index is recreated empty");
    }

    #[test]
    fn fused_ranking_beats_dense_only_on_the_hard_retrieval_set() {
        let data = include_str!("../../search-ml/examples/data/retrieval_eval.tsv");
        let dir = tempfile::tempdir().unwrap();
        let index = SemanticIndex::open(&dir.path().join("semantic.sqlite")).unwrap();
        let mut questions = Vec::new();
        for line in data.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields[0] {
                "P" => {
                    let path = dir.path().join(format!("{}.txt", fields[1]));
                    std::fs::write(&path, fields[2]).unwrap();
                    let (modified, size) = source_metadata(&path);
                    index
                        .index_document(
                            &path.to_string_lossy(),
                            modified,
                            size,
                            &[fields[2].to_string()],
                        )
                        .unwrap();
                }
                "Q" => questions.push((fields[1], fields[2].split(';').collect::<Vec<_>>())),
                _ => {}
            }
        }
        assert_eq!(questions.len(), 40);
        let top1 = |hybrid: bool| {
            questions
                .iter()
                .filter(|(question, gold)| {
                    let hits = index
                        .search_filtered(question, 1, None, None, hybrid, &crate::lexical::LexicalConfig::default())
                        .unwrap();
                    gold.iter().any(|id| hits[0].path.ends_with(&format!("{id}.txt")))
                })
                .count()
        };
        let (dense, fused) = (top1(false), top1(true));
        assert!(fused > dense, "fused {fused} must beat dense {dense}");
        assert!(fused >= 36, "fused top-1 {fused}/40");
    }
}
