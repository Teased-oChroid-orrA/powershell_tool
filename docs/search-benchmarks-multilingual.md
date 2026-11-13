# Multilingual embedding evaluation (#240 P1-4, 2026-11-12)

Question: is a small multilingual embedding model worth offering for non-English folders, and does it hold up on the
five English sets of `docs/search-benchmarks.md` against the bundled `all-MiniLM-L6-v2`?

Answer: **not as a replacement** (it loses four of the five English sets, so the owner rule rejects it), **yes as an
optional model for non-English folders** (it lifts hybrid nDCG@10 by 0.10 to 0.61 on five languages). Nothing was
changed in the app; the bundled model stays. Offering a second model is a product decision (see the end).

## Method

- Harness: new `search-core/examples/multilingual_eval.rs`. It indexes a BEIR-format folder through the real
  `SemanticIndex` exactly as `retrieval_eval` does (same index layout) and prints nDCG@10 and Recall@100 for similarity
  only (`dense`) and the shipped hybrid (`default`: score fusion, keyword share 0.35). The bundled model is the
  baseline; candidates go through `search_ml::use_embedding_model` (384 dimensions, mean pooling, `token_type_ids`).
- Multilingual set: MIRACL dev (Hugging Face `mteb/MIRACLRetrieval`, CC BY-SA 4.0; Wikipedia passages, human
  judgements). The full corpora are 1 to 4 GB per language, so each language is a subset built like the TREC-COVID
  subsample: the first N dev queries, and as corpus every passage judged for them (relevant and judged non-relevant).
  Chinese and German use 300 queries (about 3,000 passages; German was cut to its first 100 queries, 1,086 passages,
  for the three-model comparison because the machine was shared and slow); Spanish, French and Russian use 100 queries
  (about 1,000 passages). Japanese was prepared (300 queries) but not run. The subsets are easy (a judged-passage pool
  rather than all of Wikipedia: Recall@100 is 1.00 for the better models), so use them to compare models, not as
  absolute numbers. Data stays outside the repository in `~/Claude/Projects/powerShell/benchmarks/miracl/<lang>`.
- English: the five existing sets, 200 queries each (50 for the TREC-COVID subset), the same settings as the earlier
  model bake-off; the bundled column reproduces that page (Cranfield 0.3988, SciFact 0.7518, NFCorpus 0.3844, MS MARCO
  0.6674, TREC-COVID 0.4121).
- Candidates (both permissive, ONNX from Hugging Face, loaded unchanged by `search-ml`):
  - `intfloat/multilingual-e5-small` (MIT, 118M parameters, XLM-R tokenizer with 250k vocabulary, prefixes `query: `
    and `passage: `). 470 MB fp32.
  - `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` (Apache 2.0, no prefixes). 470 MB fp32.
  Larger models such as bge-m3 (568M parameters, 1024 dimensions) were not tried: `search-ml` fixes 384 dimensions and
  the app would grow to well over 1 GB.

## Results: multilingual (nDCG@10)

| Language (queries, passages) | Bundled dense | Bundled hybrid | multilingual-MiniLM-L12 dense / hybrid | multilingual-e5-small dense / hybrid |
|---|---|---|---|---|
| Chinese (300, 2,930) | 0.0948 | 0.1053 | 0.5881 / 0.5885 | 0.7254 / 0.7187 |
| German (100, 1,086) | 0.4869 | 0.5894 | 0.6521 / 0.6621 | 0.7181 / 0.6887 |
| Spanish (100, 993) | 0.5680 | 0.7142 | not run | 0.8213 / 0.8276 |
| French (100, 998) | 0.5127 | 0.5480 | not run | 0.7280 / 0.6785 |
| Russian (100, 1,039) | 0.1946 | 0.4920 | not run | 0.7898 / 0.7383 |

Recall@100, hybrid: bundled 0.44 (Chinese), 0.96 (German), 0.99 (Spanish), 0.97 (French), 0.83 (Russian);
multilingual-e5-small 1.00, 0.98, 1.00, 1.00, 1.00.

Findings:

- The bundled model is nearly blind outside Latin-script, English-like text: Chinese 0.09 and Russian 0.19 dense. Its
  English-only vocabulary shreds other scripts into pieces. For Latin-script languages the keyword side carries it
  (Spanish hybrid 0.714), which is why the hybrid hides much of the gap.
- multilingual-e5-small beats multilingual-MiniLM-L12 on both languages where both ran (Chinese +0.13, German +0.03
  hybrid; +0.07 dense), so MiniLM-L12 is dominated and dropped.
- With e5 the hybrid is below dense-only on German, French and Russian (by 0.03 to 0.05), as it is on the bundled
  model for English-free languages: the keyword side uses English stemming (`lexical::stem`) and whitespace words, and
  the 0.35 share was tuned on English sets. Chinese and Japanese have no spaces, so keyword matching degrades to whole
  runs of characters. A non-English mode would want a smaller keyword share or a language-aware tokenizer; not
  attempted (it would need its own measurement).

## Results: English regression (nDCG@10, hybrid `default`; dense in brackets)

| Set | Bundled MiniLM-L6 | multilingual-e5-small | Change |
|---|---|---|---|
| Cranfield | 0.3988 (0.3637) | 0.3792 (0.3320) | -0.0196 |
| SciFact | 0.7518 (0.6634) | 0.7298 (0.5710) | -0.0220 |
| NFCorpus | 0.3844 (0.3115) | 0.3771 (0.2637) | -0.0073 |
| MS MARCO | 0.6674 (0.6610) | 0.6613 (0.6754) | -0.0061 |
| TREC-COVID | 0.4121 (0.3534) | 0.4129 (0.4069) | +0.0008 |

multilingual-e5-small loses four of five sets (mean -0.0108), and its Recall@100 is also lower on Cranfield, SciFact
and NFCorpus. Under the owner rule (adopt only what loses on no set) it cannot replace the bundled model for English
folders. multilingual-MiniLM-L12 was not run on the English sets: it was already dominated on the multilingual ones.

## Cost

- Size: 470 MB fp32 (model) plus a 17 MB tokenizer, against 90 MB and 0.7 MB for the bundled model: 5.2 times the
  model, 12 layers against 6. int8 was not measured; the earlier int8 runs (`docs/search-benchmarks.md`) lost quality
  on MiniLM, so it would need its own run before a smaller download is claimed.
- Speed: not reliably measured. The machine ran other heavy jobs throughout (load average 20 to 110 on 8 cores), so
  the indexing rates the harness printed (0.6 to 1.5 passages per second for the candidates, 1.9 to 4.7 for the bundled
  model, each under different load) and query embedding times (26 to 138 ms, no consistent order) say nothing about the
  models. From the architecture, expect roughly twice the compute per token (12 layers, same width), and more tokens
  per passage for non-Latin text, so indexing about two to three times slower, in line with the 2 to 3 times seen for
  bge-small. Re-measure on an idle machine before quoting a figure.

## Verdict and options

- Not adopted: replacing the bundled model fails the rule (four English sets lose). The bundled model, index version
  constants and `search-ml` are untouched.
- Recorded as a positive result for a different use: an **optional, user-selected multilingual model**
  (multilingual-e5-small) for folders whose text is not English. It would need no code in `search-ml` beyond choosing the
  model and its prefixes (`use_embedding_model` already does that), but it also needs a distribution decision (470 MB
  download or a quantised variant), a per-index model record so vectors of two models never mix, and a non-English
  keyword setting. These are product decisions and are not started here.
- Not tried: language-aware keyword scoring for the hybrid, int8 e5, Japanese and the remaining MIRACL languages.

To repeat: download `model.onnx` and `tokenizer.json` of `intfloat/multilingual-e5-small` (the `onnx/` folder) into a
folder and run

```
cargo build -p search-core --release --features semantic-search --example multilingual_eval
target/release/examples/multilingual_eval <set folder> --model <folder> --query-prefix "query: " --passage-prefix "passage: " --queries 100 --header
```
