//! GLiNER (zero-shot span extraction) on `rten`, for the entity evals only. Not a product dependency.
//!
//! Model: `onnx-community/gliner_small-v2.1` (DeBERTa-v3-small encoder + LSTM + span head, Apache-2.0), fp32 or
//! `onnxruntime` dynamic int8. Inputs follow the GLiNER reference: the prompt `<<ENT>> label <<ENT>> label <<SEP>>`
//! followed by the text words, each word tokenized alone (first sub-token marks the word), all spans up to
//! `MAX_WIDTH` words, sigmoid scores, greedy non-overlapping selection (`flat_ner`).
use regex::Regex;
use rten::{Model, ValueView};
use std::ops::Range;
use std::path::Path;
use tokenizers::Tokenizer;

const MAX_WIDTH: usize = 12;
/// Text words per window; the prompt plus 5 labels is about 25 tokens and the model was trained to 384.
const WINDOW_WORDS: usize = 160;

pub struct Gliner {
    model: Model,
    tokenizer: Tokenizer,
    words: Regex,
    ids: [rten::NodeId; 6],
    output: rten::NodeId,
    pub ent: u32,
    pub sep: u32,
    cls: u32,
    end: u32,
}

#[derive(Clone, Debug)]
pub struct Span {
    pub bytes: Range<usize>,
    pub label: usize,
    pub score: f32,
}

impl Gliner {
    pub fn load(dir: &Path) -> Self {
        let model = Model::load(std::fs::read(dir.join("model.onnx")).expect("model.onnx")).expect("load model on rten");
        let tokenizer = Tokenizer::from_bytes(std::fs::read(dir.join("tokenizer.json")).expect("tokenizer.json")).expect("tokenizer");
        let node = |name: &str| model.node_id(name).unwrap_or_else(|e| panic!("input {name}: {e}"));
        let ids = ["input_ids", "attention_mask", "words_mask", "text_lengths", "span_idx", "span_mask"].map(node);
        let output = node("logits");
        let id = |t: &str| tokenizer.token_to_id(t).unwrap_or_else(|| panic!("token {t}"));
        Self {
            ent: id("<<ENT>>"),
            sep: id("<<SEP>>"),
            cls: id("[CLS]"),
            end: id("[SEP]"),
            model,
            tokenizer,
            words: Regex::new(r"\w+(?:[-_]\w+)*|\S").unwrap(),
            ids,
            output,
        }
    }

    fn word_ids(&self, word: &str) -> Vec<i32> {
        self.tokenizer.encode(word, false).expect("tokenize").get_ids().iter().map(|&v| v as i32).collect()
    }

    /// Spans with score at least `threshold`, non-overlapping, in text order. `labels` are the zero-shot type names.
    pub fn extract(&self, text: &str, labels: &[&str], threshold: f32) -> Vec<Span> {
        let words: Vec<Range<usize>> = self.words.find_iter(text).map(|m| m.range()).collect();
        let mut out = Vec::new();
        for window in words.chunks(WINDOW_WORDS) {
            out.extend(self.window(text, window, labels, threshold));
        }
        out
    }

    fn window(&self, text: &str, words: &[Range<usize>], labels: &[&str], threshold: f32) -> Vec<Span> {
        let n = words.len();
        let mut ids: Vec<i32> = vec![self.cls as i32];
        for label in labels {
            ids.push(self.ent as i32);
            for w in label.split_whitespace() {
                ids.extend(self.word_ids(w));
            }
        }
        ids.push(self.sep as i32);
        let mut words_mask = vec![0i32; ids.len()];
        for (i, w) in words.iter().enumerate() {
            let pieces = self.word_ids(&text[w.clone()]);
            words_mask.push(i as i32 + 1);
            words_mask.extend(std::iter::repeat(0).take(pieces.len().saturating_sub(1)));
            ids.extend(pieces);
        }
        ids.push(self.end as i32);
        words_mask.push(0);
        let len = ids.len();
        let attention = vec![1i32; len];
        let lengths = [n as i32];
        let mut span_idx = Vec::with_capacity(n * MAX_WIDTH * 2);
        let mut span_mask = Vec::with_capacity(n * MAX_WIDTH);
        for start in 0..n {
            for width in 0..MAX_WIDTH {
                span_idx.extend([start as i32, (start + width) as i32]);
                span_mask.push(i32::from(start + width < n));
            }
        }
        fn v1(s: &[i32], shape: [usize; 2]) -> ValueView<'_> {
            ValueView::from_shape(shape, s).unwrap()
        }
        let out = self
            .model
            .run(
                vec![
                    (self.ids[0], v1(&ids, [1, len]).into()),
                    (self.ids[1], v1(&attention, [1, len]).into()),
                    (self.ids[2], v1(&words_mask, [1, len]).into()),
                    (self.ids[3], v1(&lengths, [1, 1]).into()),
                    (self.ids[4], ValueView::from_shape([1, n * MAX_WIDTH, 2], span_idx.as_slice()).unwrap().into()),
                    (self.ids[5], v1(&span_mask, [1, n * MAX_WIDTH]).into()),
                ],
                &[self.output],
                None,
            )
            .expect("run gliner")
            .into_iter()
            .next()
            .unwrap();
        let (shape, logits) = out.into_shape_vec::<f32, 4>().expect("logits");
        let classes = shape[3];
        let mut found: Vec<(usize, usize, usize, f32)> = Vec::new(); // (start word, end word, label, score)
        for start in 0..n {
            for width in 0..MAX_WIDTH.min(n - start) {
                let base = ((start * MAX_WIDTH) + width) * classes;
                for c in 0..classes.min(labels.len()) {
                    let p = 1.0 / (1.0 + (-logits[base + c]).exp());
                    if p >= threshold {
                        found.push((start, start + width, c, p));
                    }
                }
            }
        }
        found.sort_by(|a, b| b.3.total_cmp(&a.3));
        let mut kept: Vec<(usize, usize, usize, f32)> = Vec::new();
        for f in found {
            if kept.iter().all(|k| f.1 < k.0 || f.0 > k.1) {
                kept.push(f);
            }
        }
        kept.sort_by_key(|k| k.0);
        kept.into_iter().map(|(s, e, label, score)| Span { bytes: words[s].start..words[e].end, label, score }).collect()
    }
}
