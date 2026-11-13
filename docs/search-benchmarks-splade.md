# Learned sparse retrieval (SPLADE): measured, not adopted (2026-11-12, issue #240 P1-8)

Question: does a SPLADE-style learned sparse model, run offline in Rust, beat our BM25 and our shipped ranking
(`LexicalConfig::default`: MiniLM similarity plus BM25, min-max fused, keyword share 0.35, stemming) under the owner
rule (`docs/search-benchmarks.md`: five sets, measured before adoption)?

## Model

`prithivida/Splade_PP_en_v1` (SPLADE++ style, BERT-base, 110M parameters, Apache-2.0, ONNX 532 MB fp32). It runs on
the `rten` 0.24.0 that `search-ml` already pins, with the `onnx_format` feature, with no changes: every operator
loads. The ONNX file ends in the masked-language-model head (inputs `input_ids`, `input_mask`, `segment_ids`, output
token logits over the 30,522-term vocabulary). A text becomes a sparse vector: for each term, the maximum over the
text's tokens of `ln(1 + relu(logit))`. Documents and queries use the same model; a document is scored by the dot
product of the two vectors.

Other candidates seen and not run: the naver SPLADE models (CC BY-NC-SA, not usable); `rasyosef/splade-tiny` and
`-mini` (MIT, small, but no ONNX export on the hub and no torch here to make one); the OpenSearch doc-only models
(Apache-2.0, 630 MB `gte` variant, would need an idf-lookup query side). A smaller model that is good enough is the
only thing that could change the verdict below on cost.

## Harness

`search-core/examples/splade_eval.rs` (new; `retrieval_eval.rs` is untouched). It encodes each set's documents
whole (256 tokens, like the dense model, no chunking), caches the vectors in `<set>/.eval-splade-splade-pp/`, and scores
every query. The shipped scores come from the existing `<set>/.eval-index` through `SemanticIndex::rank_features`
(best chunk per file), the way `ranking_experiments.rs` does: `bm25` = best chunk's BM25, `default` = best chunk's fused
score at share 0.35. `x+spladeS` is a convex sum `(1-S) * x + S * splade` of min-max normalised document scores.
Same five sets, same queries, same nDCG@10 and Recall@100 as `retrieval_eval`.

```
cargo build -p search-core --release --features semantic-search --example splade_eval
RTEN_NUM_THREADS=2 target/release/examples/splade_eval <set> --model <folder with model.onnx and tokenizer.json>
```

`search-core/Cargo.toml` gets `rten` and `tokenizers` as dev-dependencies (same versions and features as `search-ml`)
for the example.

## Results

nDCG@10 (Recall@100 in the second table).

| nDCG@10 | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | Mean |
|---|---|---|---|---|---|---|
| BM25 (best chunk, stemmed) | 0.3444 | 0.6817 | 0.3162 | 0.5871 | 0.3325 | 0.4524 |
| shipped `default` | 0.3936 | 0.7315 | 0.3456 | 0.6632 | 0.4121 | 0.5092 |
| SPLADE alone | 0.3740 | 0.6932 | 0.3389 | **0.7149** | 0.3555 | 0.4953 |
| BM25 + SPLADE 0.5 | 0.3833 | 0.7201 | 0.3442 | 0.6617 | 0.3724 | 0.4963 |
| `default` + SPLADE 0.2 | 0.4016 | 0.7494 | 0.3576 | 0.6824 | 0.4189 | 0.5220 |
| `default` + SPLADE 0.35 | 0.4029 | 0.7465 | 0.3586 | 0.7007 | 0.4106 | 0.5239 |
| `default` + SPLADE 0.5 | 0.4009 | 0.7319 | 0.3556 | 0.7075 | 0.3977 | 0.5187 |

| Recall@100 | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID |
|---|---|---|---|---|---|
| BM25 | 0.7238 | 0.9070 | 0.2439 | 0.9886 | 0.4945 |
| shipped `default` | 0.7943 | 0.9600 | 0.3110 | 1.0000 | 0.5520 |
| SPLADE alone | 0.7669 | 0.8970 | 0.2841 | 0.9979 | 0.4500 |
| `default` + SPLADE 0.2 | 0.7989 | 0.9593 | 0.3177 | 1.0000 | 0.5495 |

(Other shares, 0.65 included, are printed by the tool; the full grid was run, nothing else was tried.)

Findings:

- SPLADE alone beats BM25 on every set (mean 0.4953 against 0.4524) but loses to the shipped `default` on four of five.
  It wins MS MARCO by a wide margin (0.7149 against 0.6632), which is the set the model was trained on (MS MARCO
  passages): that set is the least trustworthy for generalisation to engineering documents.
- Fused with `default`, share 0.2 beats `default` on all five sets (mean +0.0128 nDCG@10), share 0.35 on four
  (it loses TREC-COVID by 0.0015; mean +0.0147). Recall@100 is flat: slightly lower on SciFact (-0.0007) and TREC-COVID
  (-0.0025), higher on Cranfield and NFCorpus.
- The share was chosen on these test sets from four values, so the gain is optimistic; no held-out check was done.
- Closest to the owner's field (Cranfield): +0.008 to +0.009.

## Cost

Timings were taken on a machine loaded by other builds (load average 30 to 130 on 8 cores) with 1 to 2 `rten`
threads, so they are upper bounds; the ratio against MiniLM is what matters.

| | MiniLM (shipped) | SPLADE++ |
|---|---|---|
| Model size | 90 MB | 532 MB (about 6x) |
| Index encode | NFCorpus 188 s (int8; fp32 similar order) | NFCorpus 8,532 s; SciFact 10,452 s; MS MARCO 5,075 s; TREC-COVID 6,474 s; Cranfield 1,912 s (0.4 to 0.8 documents/s) |
| Query encode | about 9 ms | 89 to 370 ms |
| Stored per document | 384 floats per chunk | 100 to 187 non-zero terms, 8 bytes each (1.8 to 7.8 MB per set) |
| Scoring one query | n/a | 0.04 to 0.13 ms (inverted lists) |

Indexing costs roughly 40 times the dense model's and a 6x larger offline download for a machine with no GPU and no
admin rights, and every query would add 0.1 to 0.4 s of encoding to a path that is a few milliseconds today. Postings
are small and scoring is fast; the model is the cost.

## Verdict

Not adopted. It beats BM25 but not the shipped ranking alone; as a fusion partner it adds about +0.013 mean nDCG@10
(selected on the test sets, recall flat) for a 6x model, about 40x indexing time and 10 to 40x query latency. That does
not justify a second model, a second index and a version bump. No code outside the example and its dev-dependencies
changed; `EMBEDDING_MODEL_VERSION` stays 1.

Reopen only with a model of MiniLM's size or smaller that holds the fused gain on held-out shares (for instance a
distilled or int8 SPLADE with an ONNX export), and then choose the share on half the queries and report the other half.
