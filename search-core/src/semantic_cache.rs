//! In-memory snapshot of one semantic index: contiguous embeddings, chunk lengths and term postings.
//!
//! Streaming every chunk out of SQLite on each query, decoding its vector and re-tokenizing its text for keyword
//! scoring, cost time proportional to the index on every search (about 0.1 to 0.6 s per query at 1,400 to 5,000
//! chunks). The snapshot does that work once per index state: a query is then one pass of dot products over
//! contiguous memory plus a walk of the postings of its own few terms. Scores are the same as the streaming path
//! (same tokens, same BM25 over the allowed chunks), see `snapshot_scores_equal_streaming_scores`.
//!
//! A snapshot belongs to one write generation (`semantic_meta.generation`, bumped by every write) of one database
//! (`semantic_meta.instance`). A query that sees a later generation of the same database brings the snapshot up to
//! date with `Snapshot::updated`: every document row carries the generation it was written at, so only the documents
//! written or removed since are read again, and the result is field for field what `Snapshot::build` returns. Memory
//! is about 1.5 KB per chunk for vectors plus the postings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::lexical;

/// The document a chunk belongs to, as the index stored it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Doc {
    pub path: String,
    pub modified: i64,
    pub size: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Snapshot {
    /// Which database this was read from (`semantic_meta.instance`): a recreated index file is another one.
    pub instance: i64,
    pub generation: i64,
    pub rowids: Vec<i64>,
    pub doc_of: Vec<u32>,
    pub first_lines: Vec<u32>,
    pub docs: Vec<Doc>,
    vectors: Vec<f32>,
    pub lengths: Vec<u32>,
    postings: HashMap<String, Vec<(u32, u16)>>,
    dimensions: usize,
}

/// One row per chunk with its document's stamp; callers add the filter and the `(path, chunk_index)` order.
const CHUNK_ROWS: &str = "SELECT c.path,c.first_line,c.excerpt,c.embedding,d.modified,d.size,c.rowid FROM semantic_chunks c JOIN semantic_documents d ON d.path=c.path";

#[cfg(test)]
thread_local! {
    /// Full builds made on this thread, so a test can tell an update from a rebuild.
    pub(crate) static FULL_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Snapshot {
    fn empty(instance: i64, generation: i64, dimensions: usize) -> Self {
        Snapshot {
            instance,
            generation,
            rowids: Vec::new(),
            doc_of: Vec::new(),
            first_lines: Vec::new(),
            docs: Vec::new(),
            vectors: Vec::new(),
            lengths: Vec::new(),
            postings: HashMap::new(),
            dimensions,
        }
    }

    /// Append the chunks of `rows` (the columns of `CHUNK_ROWS`, in `(path, chunk_index)` order, after every chunk
    /// already held).
    fn push_rows(&mut self, rows: &mut rusqlite::Rows<'_>, stem: bool) -> Result<(), String> {
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let bytes: Vec<u8> = row.get(3).map_err(|error| error.to_string())?;
            let Some(vector) = decode(&bytes, self.dimensions) else { continue };
            let path: String = row.get(0).map_err(|error| error.to_string())?;
            if self.docs.last().is_none_or(|doc| doc.path != path) {
                self.docs.push(Doc {
                    path,
                    modified: row.get(4).map_err(|error| error.to_string())?,
                    size: row.get(5).map_err(|error| error.to_string())?,
                });
            }
            let chunk = self.rowids.len() as u32;
            let excerpt: String = row.get(2).map_err(|error| error.to_string())?;
            let words = lexical::terms_with(&excerpt, stem);
            let mut counts: HashMap<&str, u16> = HashMap::new();
            for word in &words {
                let count = counts.entry(word.as_str()).or_insert(0);
                *count = count.saturating_add(1);
            }
            for (word, count) in counts {
                self.postings.entry(word.to_string()).or_default().push((chunk, count));
            }
            self.rowids.push(row.get(6).map_err(|error| error.to_string())?);
            self.doc_of.push(self.docs.len() as u32 - 1);
            self.first_lines.push(row.get::<_, i64>(1).map_err(|error| error.to_string())?.max(0) as u32);
            self.lengths.push(words.len() as u32);
            self.vectors.extend_from_slice(&vector);
        }
        Ok(())
    }

    /// Read every chunk once, in `(path, chunk_index)` order (the order the streaming path scans in, which ties break on).
    pub fn build(conn: &Connection, instance: i64, generation: i64, stem: bool, dimensions: usize) -> Result<Self, String> {
        #[cfg(test)]
        FULL_BUILDS.with(|builds| builds.set(builds.get() + 1));
        let mut snapshot = Snapshot::empty(instance, generation, dimensions);
        let mut stmt = conn
            .prepare(&format!("{CHUNK_ROWS} ORDER BY c.path,c.chunk_index"))
            .map_err(|error| error.to_string())?;
        snapshot.push_rows(&mut stmt.query([]).map_err(|error| error.to_string())?, stem)?;
        Ok(snapshot)
    }

    /// This snapshot brought up to `generation` of the same database: documents written since `self.generation` are
    /// read again, documents no longer stored are dropped, and everything else is kept as it is. The result equals
    /// `Snapshot::build` at `generation` (same chunk order, same postings), at the cost of the changed documents plus
    /// one pass over memory instead of decoding and tokenizing every chunk.
    pub fn updated(mut self, conn: &Connection, generation: i64, stem: bool) -> Result<Self, String> {
        // Documents written after this snapshot (a rewritten document is a new row with a new stamp).
        let mut stmt = conn
            .prepare("SELECT path FROM semantic_documents WHERE generation>?1")
            .map_err(|error| error.to_string())?;
        let written: std::collections::HashSet<String> = stmt
            .query_map([self.generation], |row| row.get(0))
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<_>>()
            .map_err(|error| error.to_string())?;
        // Every stored path, in order (read from the primary-key index alone): an old document survives when it is
        // still stored and was not written since.
        let mut keep = vec![false; self.docs.len()];
        let mut changed: Vec<String> = Vec::with_capacity(written.len());
        let mut stmt = conn
            .prepare("SELECT path FROM semantic_documents ORDER BY path")
            .map_err(|error| error.to_string())?;
        let mut rows = stmt.query([]).map_err(|error| error.to_string())?;
        let mut old = 0;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let path = row.get_ref(0).map_err(|error| error.to_string())?.as_str().map_err(|error| error.to_string())?;
            while old < self.docs.len() && self.docs[old].path.as_str() < path {
                old += 1;
            }
            if written.contains(path) {
                changed.push(path.to_string());
            } else if old < self.docs.len() && self.docs[old].path == path {
                keep[old] = true;
            }
        }
        self.generation = generation;
        if changed.is_empty() && keep.iter().all(|&kept| kept) {
            return Ok(self);
        }
        let mut delta = Snapshot::empty(self.instance, generation, self.dimensions);
        let mut stmt = conn
            .prepare(&format!("{CHUNK_ROWS} WHERE c.path=?1 ORDER BY c.chunk_index"))
            .map_err(|error| error.to_string())?;
        for path in &changed {
            delta.push_rows(&mut stmt.query([path]).map_err(|error| error.to_string())?, stem)?;
        }

        // Merge the kept documents and the re-read ones in path order. `moved[old chunk]` is its new index.
        const GONE: u32 = u32::MAX;
        let dimensions = self.dimensions;
        let kept_chunks = self.doc_of.iter().filter(|&&doc| keep[doc as usize]).count();
        let total = kept_chunks + delta.len();
        let mut merged = Snapshot::empty(self.instance, generation, dimensions);
        merged.rowids.reserve_exact(total);
        merged.doc_of.reserve_exact(total);
        merged.first_lines.reserve_exact(total);
        merged.lengths.reserve_exact(total);
        merged.vectors.reserve_exact(total * dimensions);
        let mut moved = vec![GONE; self.len()];
        let mut delta_moved = vec![0u32; delta.len()];
        let old_docs = std::mem::take(&mut self.docs);
        let (mut old_chunk, mut delta_chunk) = (0usize, 0usize);
        let mut delta_docs = std::mem::take(&mut delta.docs).into_iter().enumerate().peekable();
        let mut take_delta = |merged: &mut Snapshot, index: usize, doc: Doc, delta_chunk: &mut usize| {
            let start = *delta_chunk;
            while *delta_chunk < delta.doc_of.len() && delta.doc_of[*delta_chunk] as usize == index {
                delta_moved[*delta_chunk] = (merged.rowids.len() + *delta_chunk - start) as u32;
                *delta_chunk += 1;
            }
            merged.docs.push(doc);
            merged.doc_of.extend(std::iter::repeat_n(merged.docs.len() as u32 - 1, *delta_chunk - start));
            merged.rowids.extend_from_slice(&delta.rowids[start..*delta_chunk]);
            merged.first_lines.extend_from_slice(&delta.first_lines[start..*delta_chunk]);
            merged.lengths.extend_from_slice(&delta.lengths[start..*delta_chunk]);
            merged.vectors.extend_from_slice(&delta.vectors[start * dimensions..*delta_chunk * dimensions]);
        };
        for (index, doc) in old_docs.into_iter().enumerate() {
            let start = old_chunk;
            while old_chunk < self.doc_of.len() && self.doc_of[old_chunk] as usize == index {
                old_chunk += 1;
            }
            if !keep[index] {
                continue;
            }
            while let Some((delta_index, delta_doc)) = delta_docs.next_if(|(_, delta_doc)| delta_doc.path < doc.path) {
                take_delta(&mut merged, delta_index, delta_doc, &mut delta_chunk);
            }
            for (offset, slot) in moved[start..old_chunk].iter_mut().enumerate() {
                *slot = (merged.rowids.len() + offset) as u32;
            }
            merged.docs.push(doc);
            merged.doc_of.extend(std::iter::repeat_n(merged.docs.len() as u32 - 1, old_chunk - start));
            merged.rowids.extend_from_slice(&self.rowids[start..old_chunk]);
            merged.first_lines.extend_from_slice(&self.first_lines[start..old_chunk]);
            merged.lengths.extend_from_slice(&self.lengths[start..old_chunk]);
            merged.vectors.extend_from_slice(&self.vectors[start * dimensions..old_chunk * dimensions]);
        }
        for (delta_index, delta_doc) in delta_docs {
            take_delta(&mut merged, delta_index, delta_doc, &mut delta_chunk);
        }

        // Postings: renumber the kept chunks (their order is unchanged, so each list stays sorted), drop the removed
        // ones, then add the re-read chunks where they belong.
        let mut postings = std::mem::take(&mut self.postings);
        if moved.iter().enumerate().any(|(old, &new)| new != old as u32) {
            postings.retain(|_, list| {
                list.retain_mut(|(chunk, _)| {
                    *chunk = moved[*chunk as usize];
                    *chunk != GONE
                });
                !list.is_empty()
            });
        }
        for (term, added) in std::mem::take(&mut delta.postings) {
            let list = postings.entry(term).or_default();
            let in_order = list.last().is_none_or(|&(last, _)| last < delta_moved[added[0].0 as usize]);
            list.extend(added.into_iter().map(|(chunk, count)| (delta_moved[chunk as usize], count)));
            if !in_order {
                list.sort_unstable_by_key(|&(chunk, _)| chunk);
            }
        }
        merged.postings = postings;
        Ok(merged)
    }

    pub fn len(&self) -> usize {
        self.rowids.len()
    }

    /// Dot product of `query` with chunk `index` (cosine similarity: both are unit length).
    pub fn similarity(&self, query: &[f32], index: usize) -> f32 {
        dot(query, &self.vectors[index * self.dimensions..(index + 1) * self.dimensions])
    }

    /// Number of chunks that contain `term`.
    pub(crate) fn doc_freq(&self, term: &str) -> usize {
        self.postings.get(term).map_or(0, Vec::len)
    }

    /// BM25 of each candidate chunk (`candidates` are chunk indices) for `terms`, with document frequency, average
    /// length and `n` taken over the candidates only, as the streaming path does. Zero where no term occurs.
    pub fn bm25(&self, terms: &[String], candidates: &[u32], k1: f32, b: f32) -> Vec<f32> {
        let mut scores = vec![0f32; candidates.len()];
        if candidates.is_empty() {
            return scores;
        }
        let mut position = vec![u32::MAX; self.len()];
        for (i, &chunk) in candidates.iter().enumerate() {
            position[chunk as usize] = i as u32;
        }
        let n = candidates.len() as f32;
        let average = (candidates.iter().map(|&c| self.lengths[c as usize] as f32).sum::<f32>() / n).max(1.0);
        for term in terms {
            let Some(postings) = self.postings.get(term) else { continue };
            let df = postings.iter().filter(|(chunk, _)| position[*chunk as usize] != u32::MAX).count() as f32;
            if df == 0.0 {
                continue;
            }
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for &(chunk, count) in postings {
                let at = position[chunk as usize];
                if at == u32::MAX {
                    continue;
                }
                let norm = k1 * (1.0 - b + b * self.lengths[chunk as usize] as f32 / average);
                let c = count as f32;
                scores[at as usize] += idf * c * (k1 + 1.0) / (c + norm);
            }
        }
        scores
    }
}

fn decode(bytes: &[u8], dimensions: usize) -> Option<Vec<f32>> {
    if bytes.len() != dimensions * 4 {
        return None;
    }
    let values: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    // The same acceptance as `Embedding::from_values`: finite values only.
    values.iter().all(|v| v.is_finite()).then_some(values)
}

/// Dot product in eight independent lanes, which the compiler turns into vector instructions.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut lanes = [0f32; 8];
    let (a_chunks, b_chunks) = (a.chunks_exact(8), b.chunks_exact(8));
    let (a_rest, b_rest) = (a_chunks.remainder(), b_chunks.remainder());
    for (x, y) in a_chunks.zip(b_chunks) {
        for i in 0..8 {
            lanes[i] += x[i] * y[i];
        }
    }
    let tail: f32 = a_rest.iter().zip(b_rest).map(|(x, y)| x * y).sum();
    lanes.iter().sum::<f32>() + tail
}

/// Snapshots kept per (index file, stemming), the two most recently built.
static CACHE: Mutex<Vec<(PathBuf, bool, Arc<Snapshot>)>> = Mutex::new(Vec::new());
const KEPT: usize = 2;

pub(crate) fn cached(path: &Path, stem: bool, instance: i64, generation: i64) -> Option<Arc<Snapshot>> {
    let cache = CACHE.lock().ok()?;
    cache
        .iter()
        .find(|(p, s, snap)| p == path && *s == stem && snap.instance == instance && snap.generation == generation)
        .map(|(_, _, snap)| snap.clone())
}

/// Remove and return the snapshot held for this index file, whatever its generation, so it can be updated.
pub(crate) fn take(path: &Path, stem: bool) -> Option<Arc<Snapshot>> {
    let mut cache = CACHE.lock().ok()?;
    let at = cache.iter().position(|(p, s, _)| p == path && *s == stem)?;
    Some(cache.remove(at).2)
}

pub(crate) fn store(path: &Path, stem: bool, snapshot: Arc<Snapshot>) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|(p, s, _)| !(p == path && *s == stem));
        cache.push((path.to_path_buf(), stem, snapshot));
        while cache.len() > KEPT {
            cache.remove(0);
        }
    }
}
