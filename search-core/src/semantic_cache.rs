//! In-memory snapshot of one semantic index: contiguous embeddings, chunk lengths and term postings.
//!
//! Streaming every chunk out of SQLite on each query, decoding its vector and re-tokenizing its text for keyword
//! scoring, cost time proportional to the index on every search (about 0.1 to 0.6 s per query at 1,400 to 5,000
//! chunks). The snapshot does that work once per index state: a query is then one pass of dot products over
//! contiguous memory plus a walk of the postings of its own few terms. Scores are the same as the streaming path
//! (same tokens, same BM25 over the allowed chunks), see `snapshot_scores_equal_streaming_scores`.
//!
//! A snapshot belongs to one write generation (`semantic_meta.generation`, bumped by every write); a query that sees a
//! different generation rebuilds it. Memory is about 1.5 KB per chunk for vectors plus the postings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::lexical;

/// The document a chunk belongs to, as the index stored it.
pub(crate) struct Doc {
    pub path: String,
    pub modified: i64,
    pub size: i64,
}

pub(crate) struct Snapshot {
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

impl Snapshot {
    /// Read every chunk once, in `(path, chunk_index)` order (the order the streaming path scans in, which ties break on).
    pub fn build(conn: &Connection, generation: i64, stem: bool, dimensions: usize) -> Result<Self, String> {
        let mut snapshot = Snapshot {
            generation,
            rowids: Vec::new(),
            doc_of: Vec::new(),
            first_lines: Vec::new(),
            docs: Vec::new(),
            vectors: Vec::new(),
            lengths: Vec::new(),
            postings: HashMap::new(),
            dimensions,
        };
        let mut stmt = conn
            .prepare("SELECT c.path,c.first_line,c.excerpt,c.embedding,d.modified,d.size,c.rowid FROM semantic_chunks c JOIN semantic_documents d ON d.path=c.path ORDER BY c.path,c.chunk_index")
            .map_err(|error| error.to_string())?;
        let mut rows = stmt.query([]).map_err(|error| error.to_string())?;
        let mut current_path = String::new();
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let bytes: Vec<u8> = row.get(3).map_err(|error| error.to_string())?;
            let Some(vector) = decode(&bytes, dimensions) else { continue };
            let path: String = row.get(0).map_err(|error| error.to_string())?;
            if snapshot.docs.is_empty() || path != current_path {
                snapshot.docs.push(Doc {
                    path: path.clone(),
                    modified: row.get(4).map_err(|error| error.to_string())?,
                    size: row.get(5).map_err(|error| error.to_string())?,
                });
                current_path = path;
            }
            let chunk = snapshot.rowids.len() as u32;
            let excerpt: String = row.get(2).map_err(|error| error.to_string())?;
            let words = lexical::terms_with(&excerpt, stem);
            let mut counts: HashMap<&str, u16> = HashMap::new();
            for word in &words {
                let count = counts.entry(word.as_str()).or_insert(0);
                *count = count.saturating_add(1);
            }
            for (word, count) in counts {
                snapshot.postings.entry(word.to_string()).or_default().push((chunk, count));
            }
            snapshot.rowids.push(row.get(6).map_err(|error| error.to_string())?);
            snapshot.doc_of.push(snapshot.docs.len() as u32 - 1);
            snapshot.first_lines.push(row.get::<_, i64>(1).map_err(|error| error.to_string())?.max(0) as u32);
            snapshot.lengths.push(words.len() as u32);
            snapshot.vectors.extend_from_slice(&vector);
        }
        Ok(snapshot)
    }

    pub fn len(&self) -> usize {
        self.rowids.len()
    }

    /// Dot product of `query` with chunk `index` (cosine similarity: both are unit length).
    pub fn similarity(&self, query: &[f32], index: usize) -> f32 {
        dot(query, &self.vectors[index * self.dimensions..(index + 1) * self.dimensions])
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

pub(crate) fn cached(path: &Path, stem: bool, generation: i64) -> Option<Arc<Snapshot>> {
    let cache = CACHE.lock().ok()?;
    cache
        .iter()
        .find(|(p, s, snap)| p == path && *s == stem && snap.generation == generation)
        .map(|(_, _, snap)| snap.clone())
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
