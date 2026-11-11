//! Small, local neural inference building blocks for Search.

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::{Mutex, OnceLock};

use rten::{Model, ValueView};
use tokenizers::{Encoding, PaddingParams, Tokenizer, TruncationParams};

/// Identity of the bundled sentence-embedding weights. Stored vectors are only
/// comparable within one value: bump it whenever `all-minilm-l6-v2.onnx` (or
/// its pooling/normalisation) changes, and `search-core` rebuilds every index.
/// 1 = MiniLM-L6-v2 fp32.
pub const EMBEDDING_MODEL_VERSION: i64 = 1;
const MODEL_BYTES: &[u8] = include_bytes!("../assets/all-minilm-l6-v2.onnx");
const TOKENIZER_BYTES: &[u8] = include_bytes!("../assets/all-minilm-l6-v2-tokenizer.json");
pub const MAX_INPUT_TOKENS: usize = 256;
const EMBEDDING_DIMENSIONS: usize = 384;
const TOKEN_EMBED_BATCH_SIZE: usize = 16;

#[derive(Clone, Debug)]
pub struct MlError(String);

impl std::fmt::Display for MlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MlError {}

#[derive(Clone, Debug)]
pub struct Embedding(Vec<f32>);

#[derive(Clone, Debug)]
pub struct TokenEmbedding {
    pub byte_range: Range<usize>,
    pub embedding: Embedding,
}

impl Embedding {
    pub fn values(&self) -> &[f32] {
        &self.0
    }

    /// Reconstruct a normalized embedding loaded from local storage.
    pub fn from_values(values: Vec<f32>) -> Result<Self, MlError> {
        if values.len() != EMBEDDING_DIMENSIONS
            || values.iter().any(|value| !value.is_finite())
            || (values.iter().map(|value| value * value).sum::<f32>().sqrt() - 1.0).abs() > 1e-3
        {
            return Err(MlError("stored embedding is invalid".into()));
        }
        Ok(Self(values))
    }
}

/// How a sentence vector is taken from the token vectors of the encoder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pooling {
    /// Average of the attended tokens (MiniLM, gte, e5).
    #[default]
    Mean,
    /// The first (`[CLS]`) token (bge, snowflake-arctic).
    Cls,
}

/// What a candidate model needs around the encoder: pooling and the text it expects before queries and passages.
#[derive(Clone, Debug, Default)]
pub struct ModelSpec {
    pub pooling: Pooling,
    pub query_prefix: String,
    pub passage_prefix: String,
}

/// A model used instead of the bundled one: leaked weights and tokenizer, set once before the first embedding.
static MODEL_OVERRIDE: OnceLock<(&'static [u8], &'static [u8], ModelSpec)> = OnceLock::new();

/// Use another 384-dimension sentence encoder (an ONNX file with `input_ids`, `attention_mask` and
/// `token_type_ids`, and its `tokenizer.json`) instead of the bundled MiniLM. For benchmarking a candidate model
/// (`search-core/examples/retrieval_eval.rs`); it must be called before anything is embedded, and the vectors it
/// makes are not comparable with a stored index of another model.
pub fn use_embedding_model(onnx: Vec<u8>, tokenizer_json: Vec<u8>, spec: ModelSpec) -> Result<(), MlError> {
    MODEL_OVERRIDE
        .set((Box::leak(onnx.into_boxed_slice()), Box::leak(tokenizer_json.into_boxed_slice()), spec))
        .map_err(|_| MlError("an embedding model is already set".into()))
}

fn model_bytes() -> (&'static [u8], &'static [u8], ModelSpec) {
    MODEL_OVERRIDE
        .get()
        .cloned()
        .unwrap_or((MODEL_BYTES, TOKENIZER_BYTES, ModelSpec::default()))
}

/// Which side of a search a text is on; some models expect a different prefix for each.
#[derive(Clone, Copy)]
enum Side {
    Query,
    Passage,
}

struct TextEncoder {
    model: Model,
    tokenizer: Tokenizer,
    counter: Tokenizer,
    input_ids: rten::NodeId,
    attention_mask: rten::NodeId,
    token_type_ids: rten::NodeId,
    output: rten::NodeId,
    spec: ModelSpec,
}

impl TextEncoder {
    fn load() -> Result<Self, MlError> {
        let (model_bytes, tokenizer_bytes, spec) = model_bytes();
        Self::load_from(model_bytes, tokenizer_bytes, spec)
    }

    fn load_from(model_bytes: &'static [u8], tokenizer_bytes: &'static [u8], spec: ModelSpec) -> Result<Self, MlError> {
        let model = Model::load_static_slice(model_bytes).map_err(|e| MlError(e.to_string()))?;
        let input_ids = model
            .node_id("input_ids")
            .map_err(|e| MlError(e.to_string()))?;
        let attention_mask = model
            .node_id("attention_mask")
            .map_err(|e| MlError(e.to_string()))?;
        let token_type_ids = model
            .node_id("token_type_ids")
            .map_err(|e| MlError(e.to_string()))?;
        let output = *model
            .output_ids()
            .first()
            .ok_or_else(|| MlError("model has no output".into()))?;

        let mut tokenizer =
            Tokenizer::from_bytes(tokenizer_bytes).map_err(|e| MlError(e.to_string()))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_INPUT_TOKENS,
                ..Default::default()
            }))
            .map_err(|e| MlError(e.to_string()))?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        let mut counter =
            Tokenizer::from_bytes(tokenizer_bytes).map_err(|e| MlError(e.to_string()))?;
        counter
            .with_truncation(None)
            .map_err(|e| MlError(e.to_string()))?;
        counter.with_padding(None);

        Ok(Self {
            model,
            tokenizer,
            counter,
            input_ids,
            attention_mask,
            token_type_ids,
            output,
            spec,
        })
    }

    fn token_count(&self, text: &str) -> Result<usize, MlError> {
        self.counter
            .encode(text, true)
            .map(|encoding| encoding.get_ids().len())
            .map_err(|e| MlError(e.to_string()))
    }

    fn encode_batch(&self, texts: &[&str], side: Side) -> Result<Vec<Embedding>, MlError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let prefix = match side {
            Side::Query => &self.spec.query_prefix,
            Side::Passage => &self.spec.passage_prefix,
        };
        let prefixed: Vec<String>;
        let texts: &[&str] = if prefix.is_empty() {
            texts
        } else {
            prefixed = texts.iter().map(|text| format!("{prefix}{text}")).collect();
            &prefixed.iter().map(String::as_str).collect::<Vec<_>>()
        };

        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| MlError(e.to_string()))?;
        let batch = encodings.len();
        let sequence = encodings
            .first()
            .map_or(0, |encoding| encoding.get_ids().len());
        if sequence == 0
            || encodings
                .iter()
                .any(|encoding| encoding.get_ids().len() != sequence)
        {
            return Err(MlError("tokenizer returned invalid batch shapes".into()));
        }

        let hidden = self.hidden_batch(&encodings, batch, sequence)?;

        let mut embeddings = Vec::with_capacity(batch);
        for (batch_index, encoding) in encodings.iter().enumerate() {
            let mask = encoding.get_attention_mask();
            let mut vector = vec![0.0f32; EMBEDDING_DIMENSIONS];
            let mut token_count = 0.0f32;
            for (token_index, &attend) in mask.iter().enumerate() {
                if attend == 0 || (self.spec.pooling == Pooling::Cls && token_index > 0) {
                    continue;
                }
                token_count += 1.0;
                let start = (batch_index * sequence + token_index) * EMBEDDING_DIMENSIONS;
                for (sum, value) in vector
                    .iter_mut()
                    .zip(&hidden[start..start + EMBEDDING_DIMENSIONS])
                {
                    *sum += *value;
                }
            }
            if token_count == 0.0 {
                return Err(MlError("tokenizer produced an empty sequence".into()));
            }
            let norm = vector
                .iter()
                .map(|value| (value / token_count).powi(2))
                .sum::<f32>()
                .sqrt();
            if !norm.is_finite() || norm <= f32::EPSILON {
                return Err(MlError(
                    "model returned a zero or non-finite embedding".into(),
                ));
            }
            vector
                .iter_mut()
                .for_each(|value| *value = (*value / token_count) / norm);
            embeddings.push(Embedding(vector));
        }
        Ok(embeddings)
    }

    fn hidden_batch(
        &self,
        encodings: &[Encoding],
        batch: usize,
        sequence: usize,
    ) -> Result<Vec<f32>, MlError> {
        let ids = encodings
            .iter()
            .flat_map(|encoding| encoding.get_ids())
            .map(|&id| id as i32)
            .collect::<Vec<_>>();
        let masks = encodings
            .iter()
            .flat_map(|encoding| encoding.get_attention_mask())
            .map(|&mask| mask as i32)
            .collect::<Vec<_>>();
        let types = encodings
            .iter()
            .flat_map(|encoding| encoding.get_type_ids())
            .map(|&kind| kind as i32)
            .collect::<Vec<_>>();
        let shape = [batch, sequence];
        let ids = ValueView::from_shape(shape, &ids).map_err(|e| MlError(e.to_string()))?;
        let masks = ValueView::from_shape(shape, &masks).map_err(|e| MlError(e.to_string()))?;
        let types = ValueView::from_shape(shape, &types).map_err(|e| MlError(e.to_string()))?;
        let values = self
            .model
            .run(
                vec![
                    (self.input_ids, ids.into()),
                    (self.attention_mask, masks.into()),
                    (self.token_type_ids, types.into()),
                ],
                &[self.output],
                None,
            )
            .map_err(|e| MlError(e.to_string()))?;
        let output = values
            .into_iter()
            .next()
            .ok_or_else(|| MlError("model returned no output".into()))?;
        let (output_shape, hidden) = output
            .into_shape_vec::<f32, 3>()
            .map_err(|e| MlError(e.to_string()))?;
        if output_shape != [batch, sequence, EMBEDDING_DIMENSIONS] {
            return Err(MlError(format!(
                "unexpected model output shape: {output_shape:?}"
            )));
        }
        Ok(hidden)
    }

    fn encode_tokens_batch(&self, texts: &[&str]) -> Result<Vec<Vec<TokenEmbedding>>, MlError> {
        if texts.len() > TOKEN_EMBED_BATCH_SIZE {
            return Err(MlError(format!(
                "token batch exceeds {TOKEN_EMBED_BATCH_SIZE} inputs"
            )));
        }
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        for text in texts {
            let full_count = self.token_count(text)?;
            if full_count > MAX_INPUT_TOKENS {
                return Err(MlError(format!(
                    "input has {full_count} tokens; maximum is {MAX_INPUT_TOKENS}"
                )));
            }
        }
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| MlError(e.to_string()))?;
        let batch = encodings.len();
        let sequence = encodings
            .first()
            .map_or(0, |encoding| encoding.get_ids().len());
        if sequence == 0
            || encodings
                .iter()
                .any(|encoding| encoding.get_ids().len() != sequence)
        {
            return Err(MlError("tokenizer returned invalid batch shapes".into()));
        }
        let hidden = self.hidden_batch(&encodings, batch, sequence)?;
        encodings
            .iter()
            .zip(texts)
            .enumerate()
            .map(|(batch_index, (encoding, text))| {
                let mut tokens = Vec::new();
                for (index, &(start, end)) in encoding.get_offsets().iter().enumerate() {
                    if start == end || encoding.get_attention_mask()[index] == 0 {
                        continue;
                    }
                    if text.get(start..end).is_none() {
                        return Err(MlError("tokenizer returned a non-UTF-8 byte range".into()));
                    }
                    let offset = (batch_index * sequence + index) * EMBEDDING_DIMENSIONS;
                    let mut values = hidden[offset..offset + EMBEDDING_DIMENSIONS].to_vec();
                    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
                    if !norm.is_finite() || norm <= f32::EPSILON {
                        return Err(MlError(
                            "model returned a zero or non-finite token embedding".into(),
                        ));
                    }
                    values.iter_mut().for_each(|value| *value /= norm);
                    tokens.push(TokenEmbedding {
                        byte_range: start..end,
                        embedding: Embedding(values),
                    });
                }
                Ok(tokens)
            })
            .collect()
    }
}

fn encoder() -> Result<&'static TextEncoder, MlError> {
    static ENCODER: OnceLock<Result<TextEncoder, MlError>> = OnceLock::new();
    ENCODER
        .get_or_init(TextEncoder::load)
        .as_ref()
        .map_err(Clone::clone)
}

/// A second, independent embedding model alongside the process-wide one (for measuring a re-ranker); it shares no state
/// with [`embed_batch`] / [`embed_queries`].
pub struct Embedder(TextEncoder);

impl Embedder {
    pub fn new(onnx: Vec<u8>, tokenizer_json: Vec<u8>, spec: ModelSpec) -> Result<Self, MlError> {
        TextEncoder::load_from(
            Box::leak(onnx.into_boxed_slice()),
            Box::leak(tokenizer_json.into_boxed_slice()),
            spec,
        )
        .map(Self)
    }

    pub fn embed_queries(&self, texts: &[&str]) -> Result<Vec<Embedding>, MlError> {
        self.0.encode_batch(texts, Side::Query)
    }

    pub fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Embedding>, MlError> {
        self.0.encode_batch(texts, Side::Passage)
    }
}

/// Embed passages (documents, lines) with the bundled, offline sentence encoder.
pub fn embed_batch(texts: &[&str]) -> Result<Vec<Embedding>, MlError> {
    encoder()?.encode_batch(texts, Side::Passage)
}

/// Embed search queries. Same as [`embed_batch`] for the bundled model, which uses no prefix; models trained with a
/// query instruction (bge, e5, arctic-embed) get theirs here.
///
/// The last [`QUERY_CACHE`] distinct queries are remembered: a refined or repeated search, and the several features that
/// embed the same text (Find, related files), skip the model. The model never changes within a process, so a cached
/// vector cannot go stale.
pub fn embed_queries(texts: &[&str]) -> Result<Vec<Embedding>, MlError> {
    static CACHE: Mutex<VecDeque<(String, Embedding)>> = Mutex::new(VecDeque::new());
    let lookup = |text: &str| {
        CACHE.lock().ok().and_then(|cache| cache.iter().find(|(key, _)| key == text).map(|(_, e)| e.clone()))
    };
    let mut out: Vec<Option<Embedding>> = texts.iter().map(|text| lookup(text)).collect();
    let missing: Vec<usize> = (0..texts.len()).filter(|&i| out[i].is_none()).collect();
    if !missing.is_empty() {
        let batch: Vec<&str> = missing.iter().map(|&i| texts[i]).collect();
        let computed = encoder()?.encode_batch(&batch, Side::Query)?;
        if let Ok(mut cache) = CACHE.lock() {
            for (&i, embedding) in missing.iter().zip(&computed) {
                if !cache.iter().any(|(key, _)| key == texts[i]) {
                    if cache.len() == QUERY_CACHE {
                        cache.pop_front();
                    }
                    cache.push_back((texts[i].to_string(), embedding.clone()));
                }
            }
        }
        for (&i, embedding) in missing.iter().zip(computed) {
            out[i] = Some(embedding);
        }
    }
    Ok(out.into_iter().flatten().collect())
}

/// Distinct queries [`embed_queries`] remembers.
const QUERY_CACHE: usize = 32;

/// Count full input tokens without truncation, including model special tokens.
pub fn token_count(text: &str) -> Result<usize, MlError> {
    encoder()?.token_count(text)
}

/// Return normalized contextual token vectors with byte offsets into the input.
/// Inputs beyond the model's full token limit fail instead of truncating silently.
pub fn embed_tokens(text: &str) -> Result<Vec<TokenEmbedding>, MlError> {
    encoder()?
        .encode_tokens_batch(&[text])?
        .pop()
        .ok_or_else(|| MlError("model returned no token sequence".into()))
}

/// Return normalized contextual token vectors for up to 16 bounded inputs.
pub fn embed_tokens_batch(texts: &[&str]) -> Result<Vec<Vec<TokenEmbedding>>, MlError> {
    encoder()?.encode_tokens_batch(texts)
}

/// Cosine similarity for normalized embeddings. Returns `None` for vectors with different dimensions.
pub fn cosine_similarity(left: &Embedding, right: &Embedding) -> Option<f32> {
    (left.0.len() == right.0.len() && !left.0.is_empty())
        .then(|| left.0.iter().zip(&right.0).map(|(a, b)| a * b).sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_queries_return_the_same_vectors_in_input_order() {
        let first = embed_queries(&["torsional stress", "fuel line"]).unwrap();
        // One cached, one new, in a different order: results must follow the inputs.
        let second = embed_queries(&["hose clamp", "fuel line", "torsional stress"]).unwrap();
        assert_eq!(second.len(), 3);
        assert_eq!(second[1].values(), first[1].values());
        assert_eq!(second[2].values(), first[0].values());
        assert_ne!(second[0].values(), second[1].values());
        let started = std::time::Instant::now();
        let again = embed_queries(&["hose clamp"]).unwrap();
        assert_eq!(again[0].values(), second[0].values());
        assert!(started.elapsed() < std::time::Duration::from_millis(5), "cache hit still ran the model");
    }

    #[test]
    fn bundled_encoder_runs_locally_and_separates_related_sentences() {
        let embeddings = embed_batch(&[
            "A dog is playing outdoors in a garden.",
            "A dog plays outside among the plants.",
            "The government announced a new tax policy.",
        ])
        .unwrap();
        assert_eq!(embeddings.len(), 3);
        assert!(embeddings
            .iter()
            .all(|embedding| embedding.values().len() == EMBEDDING_DIMENSIONS));
        assert!(embeddings.iter().all(|embedding| (embedding
            .values()
            .iter()
            .map(|x| x * x)
            .sum::<f32>()
            .sqrt()
            - 1.0)
            .abs()
            < 1e-4));

        let paraphrase = cosine_similarity(&embeddings[0], &embeddings[1]).unwrap();
        let unrelated = cosine_similarity(&embeddings[0], &embeddings[2]).unwrap();
        assert!(
            paraphrase > unrelated,
            "paraphrase={paraphrase}, unrelated={unrelated}"
        );
    }
}
