//! llama.cpp engine: one loaded GGUF model, grammar-constrained greedy decoding on CPU.

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Mutex;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

use crate::{
    parse_conditions, parse_verdict, Condition, ConditionModel, Verdict, JUDGE_GRAMMAR,
    JUDGE_SYSTEM, PARSE_GRAMMAR, PARSE_SYSTEM,
};

const CONTEXT_TOKENS: u32 = 2048;
const BATCH_TOKENS: usize = 512;
const PARSE_MAX_TOKENS: usize = 400;
const JUDGE_MAX_TOKENS: usize = 160;

#[derive(Debug)]
pub struct ModelError(pub String);

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ModelError {}

fn err(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}

pub struct LlamaEngine {
    backend: LlamaBackend,
    model: LlamaModel,
    threads: i32,
    /// One generation at a time: contexts are created per call and are not thread-safe to share.
    gate: Mutex<()>,
}

impl LlamaEngine {
    /// Load a GGUF model for CPU inference (no GPU layers, memory-mapped).
    pub fn load(path: &Path) -> Result<Self, ModelError> {
        if !path.is_file() {
            return Err(ModelError(format!(
                "language model not found: {}",
                path.display()
            )));
        }
        let mut backend = LlamaBackend::init().map_err(|e| ModelError(err("llama backend", e)))?;
        backend.void_logs();
        let model = LlamaModel::load_from_file(
            &backend,
            path,
            &LlamaModelParams::default()
                .with_n_gpu_layers(0)
                .with_use_mmap(true),
        )
        .map_err(|e| ModelError(err("loading language model", e)))?;
        let threads = std::thread::available_parallelism()
            .map_or(4, |n| n.get() / 2)
            .clamp(2, 8) as i32;
        Ok(Self {
            backend,
            model,
            threads,
            gate: Mutex::new(()),
        })
    }

    fn generate(
        &self,
        system: &str,
        user: &str,
        grammar: &str,
        max_tokens: usize,
    ) -> Result<String, String> {
        let _turn = self
            .gate
            .lock()
            .map_err(|_| "language model lock poisoned".to_string())?;
        let template = self
            .model
            .chat_template(None)
            .map_err(|e| err("chat template", e))?;
        let messages = vec![
            LlamaChatMessage::new("system".into(), system.into()).map_err(|e| err("prompt", e))?,
            LlamaChatMessage::new("user".into(), user.into()).map_err(|e| err("prompt", e))?,
        ];
        let prompt = self
            .model
            .apply_chat_template(&template, &messages, true)
            .map_err(|e| err("prompt", e))?;
        let tokens = self.model.vocab().tokenize(prompt.as_bytes(), false, true);
        if tokens.len() + max_tokens > CONTEXT_TOKENS as usize || tokens.len() > BATCH_TOKENS {
            return Err("prompt is too long for the language model".into());
        }
        let mut ctx = self
            .model
            .new_context(
                &self.backend,
                LlamaContextParams::default()
                    .with_n_ctx(NonZeroU32::new(CONTEXT_TOKENS))
                    .with_n_batch(BATCH_TOKENS as u32)
                    .with_n_threads(self.threads)
                    .with_n_threads_batch(self.threads),
            )
            .map_err(|e| err("language model context", e))?;
        let mut batch = LlamaBatch::new(BATCH_TOKENS, 1);
        let last = tokens.len() - 1;
        for (i, token) in tokens.iter().enumerate() {
            batch
                .add(*token, i as i32, &[0], i == last)
                .map_err(|e| err("prompt", e))?;
        }
        ctx.decode(&mut batch).map_err(|e| err("decode", e))?;
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::grammar(&self.model, grammar, "root").map_err(|e| err("grammar", e))?,
            LlamaSampler::greedy(),
        ]);
        let vocab = self.model.vocab();
        let mut bytes = Vec::new();
        let mut position = tokens.len() as i32;
        for _ in 0..max_tokens {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            // `sample` already accepts the token into the grammar; a second accept aborts (C++ exception).
            if vocab.is_eog(token) {
                break;
            }
            bytes.extend(vocab.token_to_piece(token, false, None));
            batch.clear();
            batch
                .add(token, position, &[0], true)
                .map_err(|e| err("decode", e))?;
            position += 1;
            ctx.decode(&mut batch).map_err(|e| err("decode", e))?;
        }
        String::from_utf8(bytes).map_err(|e| err("model output", e))
    }
}

impl ConditionModel for LlamaEngine {
    fn parse(&self, sentence: &str) -> Result<Vec<Condition>, String> {
        let json = self.generate(
            PARSE_SYSTEM,
            &format!("Sentence: {sentence}"),
            PARSE_GRAMMAR,
            PARSE_MAX_TOKENS,
        )?;
        parse_conditions(sentence, &json)
    }

    fn judge(&self, condition: &str, passage: &str) -> Result<Verdict, String> {
        let json = self.generate(
            JUDGE_SYSTEM,
            &format!("Condition: {condition}\nPassage: {passage}"),
            JUDGE_GRAMMAR,
            JUDGE_MAX_TOKENS,
        )?;
        parse_verdict(passage, &json)
    }
}
