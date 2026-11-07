//! Small, local neural inference building blocks for Search.

use std::ops::Range;
use std::sync::{Mutex, OnceLock};

use rten::{Model, ValueView};
use tokenizers::{Encoding, PaddingParams, Tokenizer, TruncationParams};

const MODEL_BYTES: &[u8] = include_bytes!("../assets/all-minilm-l6-v2.onnx");
const TOKENIZER_BYTES: &[u8] = include_bytes!("../assets/all-minilm-l6-v2-tokenizer.json");
const GENERATION_ENCODER_BYTES: &[u8] = include_bytes!("../assets/flan-t5-small-encoder.onnx");
const GENERATION_DECODER_BYTES: &[u8] = include_bytes!("../assets/flan-t5-small-decoder.onnx");
const GENERATION_TOKENIZER_BYTES: &[u8] = include_bytes!("../assets/flan-t5-small-tokenizer.json");
pub const MAX_INPUT_TOKENS: usize = 256;
const EMBEDDING_DIMENSIONS: usize = 384;
const TOKEN_EMBED_BATCH_SIZE: usize = 16;
const MAX_QUESTION_TOKENS: usize = 64;
const MAX_ANSWER_TOKENS: usize = 48;

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

struct TextEncoder {
    model: Model,
    tokenizer: Tokenizer,
    counter: Tokenizer,
    input_ids: rten::NodeId,
    attention_mask: rten::NodeId,
    token_type_ids: rten::NodeId,
    output: rten::NodeId,
}

impl TextEncoder {
    fn load() -> Result<Self, MlError> {
        let model = Model::load_static_slice(MODEL_BYTES).map_err(|e| MlError(e.to_string()))?;
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
            Tokenizer::from_bytes(TOKENIZER_BYTES).map_err(|e| MlError(e.to_string()))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_INPUT_TOKENS,
                ..Default::default()
            }))
            .map_err(|e| MlError(e.to_string()))?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        let mut counter =
            Tokenizer::from_bytes(TOKENIZER_BYTES).map_err(|e| MlError(e.to_string()))?;
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
        })
    }

    fn token_count(&self, text: &str) -> Result<usize, MlError> {
        self.counter
            .encode(text, true)
            .map(|encoding| encoding.get_ids().len())
            .map_err(|e| MlError(e.to_string()))
    }

    fn encode_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>, MlError> {
        if texts.is_empty() {
            return Ok(Vec::new());
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

        let mut embeddings = Vec::with_capacity(batch);
        for (batch_index, encoding) in encodings.iter().enumerate() {
            let mask = encoding.get_attention_mask();
            let mut vector = vec![0.0f32; EMBEDDING_DIMENSIONS];
            let mut token_count = 0.0f32;
            for (token_index, &attend) in mask.iter().enumerate() {
                if attend == 0 {
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

struct TextGenerator {
    encoder: Model,
    decoder: Model,
    tokenizer: Tokenizer,
    encoder_input_ids: rten::NodeId,
    encoder_attention_mask: rten::NodeId,
    encoder_hidden_state: rten::NodeId,
    decoder_input_ids: rten::NodeId,
    decoder_attention_mask: rten::NodeId,
    decoder_encoder_hidden_state: rten::NodeId,
    decoder_logits: rten::NodeId,
}

impl TextGenerator {
    fn load() -> Result<Self, MlError> {
        let encoder = Model::load_static_slice(GENERATION_ENCODER_BYTES)
            .map_err(|error| MlError(error.to_string()))?;
        let decoder = Model::load_static_slice(GENERATION_DECODER_BYTES)
            .map_err(|error| MlError(error.to_string()))?;
        let tokenizer = Tokenizer::from_bytes(GENERATION_TOKENIZER_BYTES)
            .map_err(|error| MlError(error.to_string()))?;

        Ok(Self {
            encoder_input_ids: encoder
                .node_id("input_ids")
                .map_err(|error| MlError(error.to_string()))?,
            encoder_attention_mask: encoder
                .node_id("attention_mask")
                .map_err(|error| MlError(error.to_string()))?,
            encoder_hidden_state: encoder
                .node_id("last_hidden_state")
                .map_err(|error| MlError(error.to_string()))?,
            decoder_input_ids: decoder
                .node_id("input_ids")
                .map_err(|error| MlError(error.to_string()))?,
            decoder_attention_mask: decoder
                .node_id("encoder_attention_mask")
                .map_err(|error| MlError(error.to_string()))?,
            decoder_encoder_hidden_state: decoder
                .node_id("encoder_hidden_states")
                .map_err(|error| MlError(error.to_string()))?,
            decoder_logits: decoder
                .node_id("logits")
                .map_err(|error| MlError(error.to_string()))?,
            encoder,
            decoder,
            tokenizer,
        })
    }

    fn generate(&self, question: &str, context: &[&str]) -> Result<(String, usize), MlError> {
        let question = question.trim();
        if question.is_empty() {
            return Err(MlError("question must not be empty".into()));
        }
        let question_tokens = self
            .tokenizer
            .encode(question, true)
            .map_err(|error| MlError(error.to_string()))?
            .get_ids()
            .len();
        if question_tokens > MAX_QUESTION_TOKENS {
            return Err(MlError(format!(
                "question exceeds the {MAX_QUESTION_TOKENS}-token limit"
            )));
        }

        let prefix = format!("Answer the question using only the context. Question: {question} Context: ");
        let mut selected = Vec::new();
        for passage in context {
            let joined = selected
                .iter()
                .copied()
                .chain(std::iter::once(*passage))
                .collect::<Vec<_>>()
                .join("\n");
            let candidate = format!("{prefix}{joined} Answer:");
            let token_count = self
                .tokenizer
                .encode(candidate, true)
                .map_err(|error| MlError(error.to_string()))?
                .get_ids()
                .len();
            if token_count > MAX_INPUT_TOKENS {
                break;
            }
            selected.push(*passage);
        }
        if selected.is_empty() {
            return Err(MlError(
                "no cited source line fits the generator input limit".into(),
            ));
        }
        let joined = selected.join("\n");
        let prompt = format!("{prefix}{joined} Answer:");
        let encoding = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|error| MlError(error.to_string()))?;
        let ids = encoding
            .get_ids()
            .iter()
            .map(|&id| i32::try_from(id).map_err(|error| MlError(error.to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        let mask = encoding
            .get_attention_mask()
            .iter()
            .map(|&value| value as i32)
            .collect::<Vec<_>>();
        if ids.is_empty() || ids.len() != mask.len() {
            return Err(MlError("tokenizer returned invalid generator input".into()));
        }
        if ids.len() > MAX_INPUT_TOKENS {
            return Err(MlError(format!(
                "generator input exceeds the {MAX_INPUT_TOKENS}-token limit"
            )));
        }
        let hidden = self
            .encoder
            .run(
                vec![
                    (
                        self.encoder_input_ids,
                        ValueView::from_shape([1, ids.len()], &ids)
                            .map_err(|error| MlError(error.to_string()))?
                            .into(),
                    ),
                    (
                        self.encoder_attention_mask,
                        ValueView::from_shape([1, mask.len()], &mask)
                            .map_err(|error| MlError(error.to_string()))?
                            .into(),
                    ),
                ],
                &[self.encoder_hidden_state],
                None,
            )
            .map_err(|error| MlError(error.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| MlError("generator encoder returned no output".into()))?;

        let mut output_ids = vec![0_i32];
        for _ in 0..MAX_ANSWER_TOKENS {
            let outputs = self
                .decoder
                .run(
                    vec![
                        (
                            self.decoder_input_ids,
                            ValueView::from_shape([1, output_ids.len()], &output_ids)
                                .map_err(|error| MlError(error.to_string()))?
                                .into(),
                        ),
                        (
                            self.decoder_attention_mask,
                            ValueView::from_shape([1, mask.len()], &mask)
                                .map_err(|error| MlError(error.to_string()))?
                                .into(),
                        ),
                        (self.decoder_encoder_hidden_state, (&hidden).into()),
                    ],
                    &[self.decoder_logits],
                    None,
                )
                .map_err(|error| MlError(error.to_string()))?;
            let logits = outputs
                .into_iter()
                .next()
                .ok_or_else(|| MlError("generator decoder returned no output".into()))?;
            let (shape, logits) = logits
                .into_shape_vec::<f32, 3>()
                .map_err(|error| MlError(error.to_string()))?;
            if shape[0] != 1 || shape[1] != output_ids.len() || shape[2] == 0 {
                return Err(MlError(format!("unexpected decoder output shape: {shape:?}")));
            }
            let offset = (shape[1] - 1) * shape[2];
            let next = logits[offset..offset + shape[2]]
                .iter()
                .enumerate()
                .filter(|(_, value)| value.is_finite())
                .max_by(|left, right| left.1.total_cmp(right.1))
                .map(|(index, _)| index)
                .ok_or_else(|| MlError("generator returned no finite token scores".into()))?;
            if next == 0 || next == 1 {
                break;
            }
            output_ids.push(
                i32::try_from(next).map_err(|error| MlError(error.to_string()))?,
            );
        }
        let generated = output_ids
            .into_iter()
            .skip(1)
            .map(|id| u32::try_from(id).map_err(|error| MlError(error.to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        let answer = self
            .tokenizer
            .decode(&generated, true)
            .map_err(|error| MlError(error.to_string()))?
            .trim()
            .to_string();
        if answer.is_empty() {
            return Err(MlError("generator produced an empty answer".into()));
        }
        Ok((answer, selected.len()))
    }
}

/// Generate a bounded answer from local evidence using bundled Flan-T5-small.
/// The caller must show the evidence beside the answer; generation can be wrong.
pub fn generate_answer(question: &str, context: &[&str]) -> Result<(String, usize), MlError> {
    static GENERATOR: OnceLock<Result<Mutex<TextGenerator>, MlError>> = OnceLock::new();
    let generator = GENERATOR
        .get_or_init(|| TextGenerator::load().map(Mutex::new))
        .as_ref()
        .map_err(Clone::clone)?;
    generator
        .lock()
        .map_err(|_| MlError("generator lock poisoned".into()))?
        .generate(question, context)
}

fn encoder() -> Result<&'static TextEncoder, MlError> {
    static ENCODER: OnceLock<Result<TextEncoder, MlError>> = OnceLock::new();
    ENCODER
        .get_or_init(TextEncoder::load)
        .as_ref()
        .map_err(Clone::clone)
}

/// Embed texts with the bundled, offline sentence encoder.
pub fn embed_batch(texts: &[&str]) -> Result<Vec<Embedding>, MlError> {
    encoder()?.encode_batch(texts)
}

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
    fn bundled_generator_answers_from_local_evidence() {
        let answer = generate_answer(
            "What causes torsional stress?",
            &["The shaft experiences torsional stress under applied torque."],
        )
        .unwrap();
        assert!(answer.0.to_lowercase().contains("torque"), "{answer:?}");
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
