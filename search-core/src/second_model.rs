//! A second, optional embedding model (bge-small-en-v1.5) that scores passages against a query, for the opt-in
//! "Refine results" step. It is never bundled: the caller names a folder holding `model.onnx` and `tokenizer.json`.
//! The loaded model is kept for the process because `search_ml::Embedder::new` leaks the model bytes.
use search_ml::{Embedder, ModelSpec, Pooling};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
/// Passages embedded between two progress reports.
const CHUNK: usize = 5;

static LOADED: Mutex<Option<(PathBuf, Arc<Embedder>)>> = Mutex::new(None);

fn embedder(dir: &Path) -> Result<Arc<Embedder>, String> {
    let mut loaded = LOADED.lock().map_err(|_| "the second model's loader failed earlier".to_string())?;
    if let Some((loaded_dir, embedder)) = loaded.as_ref() {
        if loaded_dir == dir {
            return Ok(Arc::clone(embedder));
        }
    }
    let read = |name: &str| std::fs::read(dir.join(name)).map_err(|e| format!("cannot read {}: {e}", dir.join(name).display()));
    let spec = ModelSpec { pooling: Pooling::Cls, query_prefix: QUERY_PREFIX.into(), passage_prefix: String::new() };
    let embedder = Arc::new(
        Embedder::new(read("model.onnx")?, read("tokenizer.json")?, spec).map_err(|e| format!("cannot load the second model: {e}"))?,
    );
    *loaded = Some((dir.to_path_buf(), Arc::clone(&embedder)));
    Ok(embedder)
}

/// Cosine score of each passage against `query` with the model in `dir`. `progress(done, total)` is called after
/// each chunk of passages.
pub fn score_passages(dir: &Path, query: &str, passages: &[&str], mut progress: impl FnMut(usize, usize)) -> Result<Vec<f32>, String> {
    let model = embedder(dir)?;
    let query_vec = model.embed_queries(&[query]).map_err(|e| e.to_string())?.remove(0);
    let mut scores = Vec::with_capacity(passages.len());
    for chunk in passages.chunks(CHUNK) {
        for passage in model.embed_passages(chunk).map_err(|e| e.to_string())? {
            scores.push(search_ml::cosine_similarity(&query_vec, &passage).unwrap_or(0.0));
        }
        progress(scores.len(), passages.len());
    }
    Ok(scores)
}
