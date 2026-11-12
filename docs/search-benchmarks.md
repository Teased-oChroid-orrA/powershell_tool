# Search retrieval benchmarks (2026-11-08)

Owner rule: any change to retrieval or ranking (embeddings, keyword scoring, fusion, chunking, query expansion) is
measured on public benchmarks before it is adopted. The hand-made 14-file corpus used for the Natural-language Find
work cannot show whether an idea helps.

## Data

Kept outside the repository in `~/Claude/Projects/powerShell/benchmarks/`, in BEIR format (`corpus.jsonl`,
`queries.jsonl`, `qrels/test.tsv`). None of it is committed.

| Set | Source | Size used | Notes |
|---|---|---|---|
| Cranfield | `ir.dcs.gla.ac.uk/resources/test_collections/cran` (Cleverdon) | 1,400 aeronautical abstracts, 225 queries, 1,612 judgements | Closest to the owner's field. Gain = 5 minus Cleverdon's code (1 to 4); the 225 `-1` lines are dropped; query numbers are the order in `cran.qry`. |
| SciFact | BEIR | 5,183 abstracts, 300 test queries | Scientific claims. |
| NFCorpus | BEIR | 3,633 documents, 323 test queries | Medical, graded judgements. |
| TREC-COVID | BEIR (TREC) | 2,859 documents, 50 queries | Subsample: per query up to 40 relevant and 20 judged non-relevant documents; qrels restricted to those. Easier than the full 171k-document task. |
| MS MARCO | Hugging Face `microsoft/ms_marco` v1.1 validation | 4,144 passages, 484 queries | First 500 queries with their candidate passages pooled into one corpus, `is_selected` as relevance. Not the 8.8M-passage task: numbers do not compare with the leaderboard. |

Absolute numbers on the subsets are not comparable with published ones except as a sanity check (SciFact dense
nDCG@10 0.661 here against 0.645 published for all-MiniLM-L6-v2); use them to compare variants.

## Harness

`search-core/examples/retrieval_eval.rs` indexes a dataset once through the real `SemanticIndex` (each document is
written as a file under `.eval-docs`, because search only returns documents that are real files with matching size and
time) and prints nDCG@10, Recall@100 and MRR@10 for named variants of `lexical::LexicalConfig`. Documents are ranked
by their best chunk.

```
cargo build -p search-core --release --features semantic-search --example retrieval_eval
target/release/examples/retrieval_eval <folder> [--queries N] [--variants dense,default,rrf-w2] [--model <folder>]
```

`--model` takes a folder with `model.onnx` and `tokenizer.json` (384-dimension, mean-pooled; `search_ml::use_embedding_model`)
and keeps its index apart. Run the five sets in parallel; each variant costs about two minutes per set.

## Results

nDCG@10, mean of the five sets (per-set numbers in the tables below):

| Variant | Mean nDCG@10 | Mean Recall@100 |
|---|---|---|
| similarity only (dense) | 0.4657 | 0.690 |
| keyword-led | 0.4540 | 0.680 |
| rank fusion, keyword weight 2, no stemming | 0.4803 | 0.700 |
| rank fusion, keyword weight 2 (until 2026-11-08) | 0.4886 | 0.710 |
| rank fusion, weight 1.5 | 0.4924 | 0.717 |
| score fusion, keyword share 0.35 (**current**) | **0.5092** | **0.724** |

| nDCG@10 | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID |
|---|---|---|---|---|---|
| dense | 0.3564 | 0.6614 | 0.2897 | 0.6675 | 0.3534 |
| rank fusion w2 (old) | 0.3772 | 0.7089 | 0.3421 | 0.6278 | 0.3870 |
| score fusion 0.35 (current) | 0.3936 | 0.7315 | 0.3456 | 0.6632 | 0.4121 |
| current without stemming | 0.3898 | 0.7275 | 0.3336 | 0.6574 | 0.3995 |

Findings:

- Score fusion (min-max normalised similarity and BM25, keyword share 0.35) equals or beats the old rank fusion on
  every set and adds 0.021 mean nDCG@10. A share of 0.25 to 0.40 is flat (0.507 to 0.509); 0.5 and above lose.
- Word-form stemming (`lexical::stem`, blended = blending) helps on all five sets under the current fusion (mean +0.008).
- BM25 `k1` and `b` do not matter (0.4922 to 0.4940 around the old default); RRF constant 30 to 100 is flat; left alone.
- The Search help ranking behind Ask still uses rank fusion (`LexicalConfig::rrf`): it ranks short lines of one
  document, which these sets do not cover. Benchmark it separately before changing it.

## Larger embedding model: gte-small, not adopted

`thenlper/gte-small` (MIT, 33M parameters, 384 dimensions, mean pooling: a drop-in for the current pooling code, 133 MB
fp32 against 90 MB for MiniLM-L6) on the same five sets, with the current score fusion:

| nDCG@10 | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | mean |
|---|---|---|---|---|---|---|
| MiniLM-L6, similarity only | 0.3564 | 0.6614 | 0.2897 | 0.6675 | 0.3534 | 0.4657 |
| gte-small, similarity only | 0.3852 | 0.7239 | 0.3278 | 0.6803 | 0.4233 | 0.5081 |
| MiniLM-L6, score fusion (current) | 0.3936 | 0.7315 | 0.3456 | 0.6632 | 0.4121 | 0.5092 |
| gte-small, score fusion | 0.4060 | 0.7492 | 0.3604 | 0.6547 | 0.4354 | 0.5211 |

The model helps similarity-only retrieval a lot (+0.042) but the keyword fusion already recovers most of it, so the
fused gain is +0.012 (2.3 percent), with a loss on MS MARCO (-0.009). Costs: 43 MB more in the app, about twice the
indexing time (a 21 MB PDF already takes minutes) and a full re-index on upgrade. Under the rule of adopting only what
loses on no set it is not adopted. `bge-small-en-v1.5` (CLS pooling and a query prefix) was not tried. To repeat:
download `model.onnx` and `tokenizer.json` into a folder and pass it as `--model`.

## Query speed: in-memory snapshot (2026-11-08)

Per query (embedding ~8 to 20 ms included), 40 queries, release build, before and after `semantic_cache.rs`:

| Set (chunks) | hybrid before | hybrid after | nDCG@10 before and after |
|---|---|---|---|
| Cranfield (1.4k) | 172 ms | 17 ms | 0.3996 and 0.3996 |
| SciFact (5.2k) | 638 ms | 26 ms | 0.8713 and 0.8713 |

The streaming path re-decoded every vector and re-tokenized every chunk on each query. The snapshot (contiguous vectors,
term postings, keyed by the `semantic_meta.generation` write counter) does that once per index state. Dense-only time
dropped on Cranfield (40 to 24 ms); the SciFact dense row (60 ms) includes the one-off snapshot build. Indexes above
250,000 chunks use the streaming path (memory: about 1.5 KB per chunk).
Test: `snapshot_scores_equal_streaming_scores_and_follow_writes`.

## Not yet measured

Pseudo-relevance feedback (expanding the query with terms from the best passages), chunk size and overlap, a larger
embedding model (see below), query-length-dependent weights.

## 2026-11-08: embedding model bake-off (MiniLM-L6-v2 vs bge-small, arctic-embed-xs, e5-small)

Harness: `search-core/examples/retrieval_eval.rs` with `--pooling`, `--query-prefix`, `--passage-prefix`; 200 queries per set (50 for TREC-COVID subset). Numbers are nDCG@10 of the shipped fused `default` variant (keyword share 0.35, stemming). Timings were taken while other builds ran, so ms figures are only relative.

| Set | MiniLM (bundled) | bge-small-en-v1.5 | arctic-embed-xs | e5-small-v2 |
|---|---|---|---|---|
| Cranfield | 0.3988 | 0.3977 | 0.4042 | 0.3870 |
| SciFact | 0.7518 | 0.7653 | 0.7289 | 0.7357 |
| NFCorpus | 0.3844 | 0.4050 | 0.3839 | 0.3838 |
| MS MARCO | 0.6674 | 0.6749 | 0.6574 | 0.6653 |
| TREC-COVID | 0.4121 | 0.4337 | 0.4041 | 0.4234 |

Dense-only, bge-small beats MiniLM on all five sets. Fused, it wins four and trails on Cranfield by 0.0011 nDCG (Recall@100 0.7900 vs 0.8020), within noise but a loss under the "loses on no set" rule. arctic-xs and e5-small each lose on at least three sets. Cost of bge-small: 133 MB fp32 vs 90 MB, about 3x slower query embedding (31 vs 9 ms cold on Cranfield) and about 2-3x slower indexing.

Verdict: keep MiniLM. bge-small is the only credible replacement; adopt it only if the owner accepts the Cranfield tie and the cost, and then bump `EMBEDDING_MODEL_VERSION` together with the index version constants. int8 variants were not measured on these five sets.

## 2026-11-08: int8 models on the five sets

Same harness and settings as the bake-off above (fused `default` variant, nDCG@10); models were quantised with onnxruntime `quantize_dynamic`. Not adopted: neither int8 model beats the shipped MiniLM fp32 on all five sets.

| Set | MiniLM fp32 (shipped) | MiniLM int8 | bge-small fp32 | bge-small int8 |
|---|---|---|---|---|
| Cranfield | 0.3988 | 0.3912 | 0.3977 | 0.3955 |
| SciFact | 0.7518 | 0.7479 | 0.7653 | 0.7652 |
| NFCorpus | 0.3844 | 0.3813 | 0.4050 | 0.4058 |
| MS MARCO | 0.6674 | 0.6623 | 0.6749 | 0.6761 |
| TREC-COVID | 0.4121 | 0.4215 | 0.4337 | 0.4370 |

MiniLM int8 loses to its own fp32 on four of five sets (it wins TREC-COVID by 0.0094), so it fails the rule outright. bge-small int8 is level with its fp32 (lower on Cranfield by 0.0022 and SciFact by 0.0001, higher on the other three), but it still trails shipped MiniLM on Cranfield (0.3955 vs 0.3988), so the bge-small verdict above is unchanged. Earlier note that the bge-small model costs about 3x in query time and 2-3x in indexing holds for int8 too (indexing NFCorpus took 703 s vs 188 s for MiniLM int8). Negative result recorded; `EMBEDDING_MODEL_VERSION` stays as is.

## 2026-11-08: pseudo-relevance feedback (PRF) and chunk size

Same harness and settings (bundled MiniLM, fused `default` variant, nDCG@10, 200 queries; 50 for TREC-COVID). Adoption rule: a change must lose on none of the five sets. Neither change is adopted.

**PRF** (`--variants default,prf`: expand the query from the top-ranked passages and search again):

| Set | default | prf | delta |
|---|---|---|---|
| Cranfield | 0.3988 | 0.4071 | +0.0083 |
| SciFact | 0.7518 | 0.7471 | -0.0047 |
| NFCorpus | 0.3844 | 0.3898 | +0.0054 |
| MS MARCO | 0.6674 | 0.6542 | -0.0132 |
| TREC-COVID | 0.4121 | 0.4209 | +0.0088 |

PRF helps three sets but loses on SciFact and MS MARCO, and it costs about 5 to 9 times the query time (Cranfield 86 vs 682 ms/query, SciFact 106 vs 921 ms/query, measured while other jobs ran, so relative only). Not adopted; negative result recorded. The query-expansion step is the part that hurts MS MARCO, where one passage answers the question and feedback terms dilute it.

**Chunk size** (`CHUNK_CHARS` in `search-core/src/semantic_index.rs`, shipped value 800; each size built into its own binary and indexed under its own `--tag`; the 800 column is the shipped `default`):

| Set | 500 | 800 (shipped) | 1200 |
|---|---|---|---|
| Cranfield | 0.3821 | 0.3988 | 0.3989 |
| SciFact | 0.7498 | 0.7518 | 0.7377 |
| NFCorpus | 0.3819 | 0.3844 | 0.3879 |
| MS MARCO | 0.6504 | 0.6674 | 0.6674 |
| TREC-COVID | 0.3871 | 0.4121 | 0.4126 |

500 loses to 800 on every set. 1200 ties or edges 800 on four sets (by at most 0.0035) but loses SciFact by 0.0141, so it fails the rule. Chunk size stays 800; the constant was restored after the experiment.

## 2026-11-08: bge-small rerank of MiniLM's top 30

Bundled MiniLM retrieves, the fused `default` ranking supplies the top 30, and bge-small (CLS pooling, query prefix "Represent this sentence for searching relevant passages: ") re-scores them. `rerank` orders by the bge-small score alone; `rerank-rrf` fuses the bge-small ranking with the `default` ranking by reciprocal rank. nDCG@10, run as `--variants default,rerank,rerank-rrf --tag minilm-rerank --rerank-model <bge-small dir>`. Cranfield and SciFact used 200 queries; **NFCorpus and MS MARCO used 60 and TREC-COVID its 50**, because at 4 to 5 seconds per query a full 200-query run of the last three sets was not practical. Compare rows only within a line (each line carries its own `default`).

| Set (queries) | default | rerank | rerank-rrf |
|---|---|---|---|
| Cranfield (200) | 0.3988 | 0.3845 | 0.4053 |
| SciFact (200) | 0.7518 | 0.7397 | 0.7553 |
| NFCorpus (60) | 0.3300 | 0.3504 | 0.3499 |
| MS MARCO (60) | 0.6519 | 0.6902 | 0.6694 |
| TREC-COVID (50) | 0.4121 | 0.4260 | 0.4466 |

`rerank` alone loses on Cranfield and SciFact, so it fails the five-set rule. `rerank-rrf` wins all five (by 0.0065, 0.0035, 0.0199, 0.0175 and 0.0345), the first change in these experiments to do so. It is **not adopted yet** because of what it costs: about 2.5 to 5.8 seconds per query on this machine against 22 to 52 ms for `default` (50 to 200 times slower, measured while other jobs ran), and a second 133 MB model that would have to ship and load. Those are product decisions for the owner: if rerank is wanted, the likely shape is an opt-in "refine top results" step after the fast results appear, not the default path. The three smaller sets also have few queries, so their gains carry more noise than the 200-query sets.

## 2026-11-09: rerank-rrf is now an opt-in step

The `rerank-rrf` variant above is available in the Search window as a "Refine results" row under the semantic results. It is off until clicked, runs in the background with progress in the status line, and needs the bge-small folder (`model.onnx`, `tokenizer.json`) from `TOOLBENCH_RERANK_MODEL`, `rerank-model/` beside the exe, or `<app data>/fea-editor/rerank-model/`; the model is not bundled. The fusion is `search_core::rank_fusion::fuse_top`, shared with `retrieval_eval.rs`. Window behaviour is not part of the benchmark: the top-30 pool there came from the `default` variant, in the window it comes from the semantic index search. Details: `app-tui/src/gpu_viewer/editor/search/AGENTS.md`.

## 2026-11-09: bigger and int8 second models for rerank-rrf

Question: does a stronger (bge-base-en-v1.5, fp32) or cheaper (bge-small int8) second model beat the fp32 bge-small `rerank-rrf` step above? Same method (`--variants default,rerank-rrf --tag minilm-rerank --rerank-model <dir>`, same query counts), adoption rule: loses on none of the five sets.

**bge-base: not measurable with the current code.** `search-ml` hard-codes `EMBEDDING_DIMENSIONS = 384` (`search-ml/src/lib.rs`, also `search-core/src/semantic_index.rs`); bge-base outputs 768 and the run stops with `unexpected model output shape: [1, 8, 768]`. Measuring it needs a per-model embedding width in `search-ml` (a code change, deliberately not made in this measurement session). The ONNX (436 MB) and tokenizer are in `benchmarks/models/bge-base` for when that is done. No quality or timing number exists for it.

**bge-small int8** (nDCG@10; fp32 column copied from the 2026-11-08 table, same `default` per row):

| Set (queries) | default | rerank-rrf fp32 | rerank-rrf int8 | int8 vs fp32 |
|---|---|---|---|---|
| Cranfield (200) | 0.3988 | 0.4053 | 0.3994 | -0.0059 (loses) |
| SciFact (200) | 0.7518 | 0.7553 | not run | |
| NFCorpus (60) | 0.3300 | 0.3499 | not run | |
| MS MARCO (60) | 0.6519 | 0.6694 | 0.6704 | +0.0010 |
| TREC-COVID (50) | 0.4121 | 0.4466 | 0.4381 | -0.0085 (loses) |

Verdict: **int8 is not adopted.** It loses to fp32 on Cranfield and TREC-COVID, which already fails the five-set rule, so SciFact and NFCorpus were not completed (two runs stopped on a locked shared index while another job used the same `--tag minilm-rerank` folders, and the verdict could not change). On Cranfield int8 keeps only +0.0006 over `default`, i.e. almost all of the rerank gain is lost. Cost: 5.3 s (MS MARCO), 8.4 s (Cranfield) and 11.9 s (TREC-COVID) per query for int8 under heavy concurrent load (load average about 30; the fp32 numbers above were 2.5 to 5.8 s), so no speed advantage was visible; timings here are not comparable with the earlier table. Negative result recorded; do not retry int8 for rerank.

## 2026-11-09: cross-encoder rerank of the default top 30

A cross-encoder reads each (query, passage) pair jointly and outputs one relevance logit, so it can judge matching that a bi-encoder's separate vectors cannot. The new `search_core::cross_encoder::score_pairs` (on `search_ml::CrossEncoder`, the same rten runtime as the embedder; pairs truncated to 512 tokens) scores the same top-30 pool as the bge-small rerank. Two models were tried, each a folder with `model.onnx` and `tokenizer.json`: `cross-encoder/ms-marco-MiniLM-L-6-v2` fp32 (91 MB, `ce-minilm`) and `Xenova/bge-reranker-base` `model_quantized.onnx` int8 (279 MB, `ce-bge-reranker-int8`). `ce` orders the pool by the cross-encoder score alone; `ce-rrf` fuses it with the `default` ranking by `rank_fusion::fuse_top`. Run as `--variants default,ce,ce-rrf --tag minilm-rerank --ce-model <dir>`; nDCG@10, same query counts as the bge-small table (Cranfield 200, SciFact 200, NFCorpus 60, MS MARCO 60, TREC-COVID 50), so compare only within a row.

| Set (queries) | default | MiniLM ce | MiniLM ce-rrf | bge-reranker int8 ce | bge-reranker int8 ce-rrf | (bge-small rerank-rrf) |
|---|---|---|---|---|---|---|
| Cranfield (200) | 0.3988 | 0.3736 | 0.3964 | 0.3631 | 0.3985 | 0.4053 |
| SciFact (200) | 0.7518 | 0.7088 | 0.7506 | 0.6528 | 0.7255 | 0.7553 |
| NFCorpus (60) | 0.3300 | 0.3669 | 0.3649 | 0.2368 | 0.3171 | 0.3499 |
| MS MARCO (60) | 0.6519 | 0.7280 | 0.6897 | 0.7112 | 0.6821 | 0.6694 |
| TREC-COVID (50) | 0.4121 | 0.3768 | 0.4080 | 0.3484 | 0.3994 | 0.4466 |

Verdict by the rule (adopt only if it loses on none of the five sets versus `default`): **neither cross-encoder is adopted, in either variant.** MiniLM `ce` wins NFCorpus (+0.037) and MS MARCO (+0.076) but loses Cranfield, SciFact and TREC-COVID; MiniLM `ce-rrf` wins the same two but sits just under `default` on Cranfield (-0.0024), SciFact (-0.0012) and TREC-COVID (-0.0041). The int8 bge-reranker is worse: it wins only MS MARCO and loses the other four in `ce` and in `ce-rrf`. The ms-marco MiniLM is trained on MS MARCO, so its MS MARCO gain is in-domain and says little about the other sets. The int8 quantization may cost the bge-reranker accuracy (SciFact -0.099 alone); an fp32 bge-reranker (1.1 GB) was not tried. Neither beats the existing bge-small `rerank-rrf`, which won all five sets.

Cost per query (re-scoring 30 pairs, measured on 20 SciFact queries while other jobs ran, about 6 load average, including model load): MiniLM about 1.8 s, bge-reranker int8 about 5.0 s; during the full runs under load of 20 to 30 these were 12 to 40 s. `default` is 22 to 52 ms. Model size on disk: 91 MB and 279 MB, against 133 MB for the bge-small rerank. Numbers in the printed `ce-rrf` ms/query column are not meaningful, because the eval reuses the `ce` scores for `ce-rrf` within a run.

## 2026-11-09: bge-base-en-v1.5 as the second model for rerank-rrf

`search-ml` used to assume a 384-wide hidden state, so only MiniLM-sized models could load. The embedder now reads the width from the model's output shape (the stored-index path still requires 384, so the bundled MiniLM index is unchanged). That made `bge-base-en-v1.5` fp32 (436 MB, 768 wide, CLS pooling, same query prefix as bge-small) loadable as the `--rerank-model`. Same method as the bge-small table: `--variants default,rerank-rrf --tag minilm-rerank --rerank-model <bge-base dir>`, Cranfield and SciFact 200 queries, NFCorpus 60, MS MARCO 60, TREC-COVID 50 (the `trec-covid/sub` folder). The `default` column was rebuilt in this session's index folders and differs from the 2026-11-08 one by up to 0.004 on some sets, so compare gains (rerank-rrf minus its own `default`), not absolute rows across tables.

| Set (queries) | default | bge-base rerank-rrf | gain | bge-small gain (2026-11-08) |
|---|---|---|---|---|
| Cranfield (200) | 0.3988 | 0.4154 | +0.0166 | +0.0065 |
| SciFact (200) | 0.7497 | 0.7616 | +0.0119 | +0.0035 |
| NFCorpus (60) | 0.3340 | 0.3550 | +0.0210 | +0.0199 |
| MS MARCO (60) | 0.6519 | 0.6846 | +0.0327 | +0.0175 |
| TREC-COVID (50) | 0.4121 | 0.4421 | +0.0300 | +0.0345 |

Verdict by the rule: bge-base `rerank-rrf` **wins all five sets against `default`**, so it passes. Against bge-small `rerank-rrf` it gains more on four sets and less on TREC-COVID (-0.0045, a 50-query set, within noise). It is not adopted as the shipped Refine model because of cost: 436 MB against 133 MB, and 9 to 18 s per query under load (load average 6 to 30, so not comparable with the bge-small timings) against 25 to 175 ms for `default`. The owner can use it today without a code change: put its `model.onnx` and `tokenizer.json` in the `rerank-model` folder. Whether to ship it, or recommend it as an optional download, is a product decision.

## 2026-11-10: Ask, Refine, cross-encoder and second-model code removed

By owner decision ("get rid of the Ask and anything related to AI questions, it does not work; keep Semantics where it is
fast and accurate") the Search window no longer has Ask (question answering, the embedded Search help, folder counts),
Refine results (and the `Refine automatically` option), the cross-encoder, the second embedding model
(`search_core::second_model`) or rank fusion over a second model (`search_core::rank_fusion`). The `rerank`,
`rerank-rrf`, `ce` and `ce-rrf` variants and the `--rerank-model` / `--ce-model` flags left `retrieval_eval`; the
hidden-width generalisation of `search-ml` that served them was reverted. A typed question now runs as an ordinary Find
(words, or a sentence when Auto reads it as one). The results above stay as history; none of the removed variants is
part of `default`, so the numbers for `default` are unchanged.

## 2026-11-10: query-time ranking ideas (none adopted)

Five ideas measured on the five sets (Cranfield 200 queries, SciFact 200, NFCorpus 60, MS MARCO 60, TREC-COVID 50), bundled MiniLM, chunk size 800, nDCG@10 over files ranked by best chunk, index folders under `--tag rank-eval`. Adoption rule: a change must lose on no set against `default` (score fusion, keyword share 0.35). **Nothing is adopted; `default` is unchanged and no stored data changed** (no version constant bumped).

Method: `SemanticIndex::rank_features` returns every chunk's raw similarity, BM25, length and place in its file for a query; `retrieval_eval --rank-experiments <outdir>` (`examples/support/ranking_experiments.rs`) scores all variants from those, so one pass measures them all, and writes per-query results for pooled and held-out analysis. Sanity check: the feature path reproduces the shipped `default` exactly (Cranfield 0.3988, SciFact 0.7518, TREC-COVID 0.4121, equal to the numbers of the PRF/chunk-size section; NFCorpus and MS MARCO use 60 queries here, 0.3300 and 0.6519).

Setup trap found on the way: an index folder copied from an earlier run holds paths as written then (relative to the working directory); a run from another directory or with the newer absolute paths re-embeds every document and leaves the old rows beside the new ones. Both copies were fresh files, so each document was ranked twice and nDCG exceeded 1. Delete rows whose path does not start with `/` (or rebuild) before trusting a copied index.

### 1. Query-length-aware keyword share: not adopted

Keyword share per query over the pooled queries (nDCG@10; the 0.35 column is `default`):

| Share | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | Mean |
|---|---|---|---|---|---|---|
| 0.00 | 0.3637 | 0.6634 | 0.3051 | 0.6632 | 0.3534 | 0.4698 |
| 0.10 | 0.3775 | 0.7141 | 0.3190 | 0.6792 | 0.3943 | 0.4968 |
| 0.20 | 0.3911 | 0.7380 | 0.3341 | 0.6802 | 0.4128 | 0.5112 |
| 0.25 | 0.3946 | 0.7470 | 0.3338 | 0.6719 | 0.4180 | 0.5131 |
| 0.30 | 0.3956 | 0.7569 | 0.3301 | 0.6647 | 0.4184 | 0.5131 |
| 0.35 (default) | 0.3988 | 0.7518 | 0.3300 | 0.6519 | 0.4121 | 0.5089 |
| 0.50 | 0.3891 | 0.7419 | 0.3304 | 0.6392 | 0.3907 | 0.4982 |

The best share differs by set (Cranfield 0.35, SciFact 0.30, TREC-COVID 0.30, NFCorpus and MS MARCO 0.20), and the sets' queries differ in length (median distinct query terms: NFCorpus 4, MS MARCO 3, TREC-COVID 7, SciFact and Cranfield 10), so length is the natural handle. To avoid per-set tuning the bucket shares were fit on the pooled queries of one half (even or odd query number) and scored on the other half, the two folds together covering every query; a bucket's best share maximises the mean over sets of the set's mean nDCG in the bucket. Held-out result against `default`, set deltas in the order Cranfield, SciFact, NFCorpus, MS MARCO, TREC-COVID:

| Buckets (query terms) | Mean delta | Set deltas | Fitted shares (fold 0 / fold 1) |
|---|---|---|---|
| <=4, 5+ | +0.0051 | -0.0024 +0.0030 -0.0079 +0.0261 +0.0069 | 0.15, 0.30 / 0.05, 0.30 |
| <=3, 4-6, 7+ | +0.0036 | -0.0024 +0.0002 -0.0018 +0.0159 +0.0062 | 0.25 0.40 0.25 / 0.05 0.40 0.25 |
| <=4, 5-8, 9+ | +0.0046 | +0.0034 -0.0019 -0.0074 +0.0239 +0.0050 | 0.15 0.40 0.35 / 0.05 0.35 0.30 |
| <=3, 4-6, 7-10, 11+ | +0.0021 | +0.0012 +0.0001 -0.0055 +0.0159 -0.0012 | unstable |
| four or five buckets | -0.0005 to -0.0014 | lose on 4 to 5 sets | unstable |

Every bucketing loses on at least one set held-out. The gain comes from the shortest bucket (MS MARCO, whose 60 queries pull it to a share of 0.05 to 0.15 in one fold and the full-data fit gives 0.6 for the same bucket): the fold fits disagree, so there is no stable length effect to encode. A constant share of 0.30 is +0.0042 mean but loses Cranfield by 0.0032; 0.25 loses Cranfield and SciFact. Negative result; shares 0.25 to 0.35 remain flat as recorded before.

### 2. Document-level scoring (max plus decayed next chunks): not adopted

File score = best chunk + alpha times the next chunk (k=1) or + alpha^2 times the one after (k=2); chunk scores are the fused 0 to 1 scores.

| Variant | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | Mean |
|---|---|---|---|---|---|---|
| max (default) | 0.3988 | 0.7518 | 0.3300 | 0.6519 | 0.4121 | 0.5089 |
| alpha 0.1, k=1 | 0.3989 | 0.7558 | 0.3344 | 0.6526 | 0.4021 | 0.5088 |
| alpha 0.25, k=1 | 0.3738 | 0.7460 | 0.3340 | 0.6621 | 0.3853 | 0.5002 |
| alpha 0.5, k=1 | 0.3376 | 0.7318 | 0.3318 | 0.6621 | 0.3810 | 0.4889 |
| alpha 0.25, k=2 | 0.3711 | 0.7386 | 0.3340 | 0.6621 | 0.3712 | 0.4954 |
| alpha 0.5, k=2 | 0.3103 | 0.6855 | 0.3277 | 0.6621 | 0.3375 | 0.4646 |

Every setting loses on TREC-COVID and most on Cranfield; the best (alpha 0.1) is a tie on the mean and loses TREC-COVID by 0.01. Short abstracts (one to three chunks) carry no extra evidence in the following chunks; they only favour longer files. Max stays. Cost was about 1 to 3 ms per query (not significant).

### 3. Heading, title or file-name boost: measured as a proxy, not adopted

The sets have a title field (all but MS MARCO). The boost adds beta times the min-max-normalised BM25 of the query over the file's title to every chunk of the file. MS MARCO has no titles, so it is unchanged and unmeasured. Cranfield's text already begins with its title.

| beta | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | Mean |
|---|---|---|---|---|---|---|
| 0 (default) | 0.3988 | 0.7518 | 0.3300 | 0.6519 | 0.4121 | 0.5089 |
| 0.05 | 0.3993 | 0.7553 | 0.3315 | 0.6519 | 0.4205 | 0.5117 |
| 0.10 | 0.3996 | 0.7551 | 0.3378 | 0.6519 | 0.4216 | 0.5132 |
| 0.20 | 0.4007 | 0.7378 | 0.3241 | 0.6519 | 0.4211 | 0.5071 |
| 0.40 | 0.3929 | 0.7029 | 0.3145 | 0.6519 | 0.4207 | 0.4966 |

Beta 0.05 to 0.1 loses on no set (+0.003 to +0.004 mean), larger weights lose SciFact. This measures the benefit of a clean title field. The product has no title: it would have to take the first line of a file or its name, and the stored index has no per-file heading (a schema change, chunking and indexing are owned elsewhere). Whether the first line of real files (reports, PDFs, code) behaves like these titles is unmeasured, and no benchmark here has file names or first lines separate from the text, so no ranking code was added. To adopt it: store or derive a heading per file, build a benchmark set whose documents begin with a title line (the harness writes `title text` on one line today), and require beta 0.05 to 0.1 to hold there.

### 4. Near-duplicate removal: not adopted

Word 3-gram Jaccard on excerpts, among the first 100 passages of `default`. Files ranked by nDCG@10 after dropping a passage that repeats an earlier one:

| Variant | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | Mean |
|---|---|---|---|---|---|---|
| none | 0.3988 | 0.7518 | 0.3300 | 0.6519 | 0.4121 | 0.5089 |
| Jaccard 0.5 | 0.3982 | 0.7518 | 0.3348 | 0.6304 | 0.4096 | 0.5050 |
| Jaccard 0.7 | 0.3982 | 0.7518 | 0.3348 | 0.6282 | 0.4102 | 0.5046 |
| Jaccard 0.9 | 0.3988 | 0.7518 | 0.3348 | 0.6457 | 0.4092 | 0.5081 |
| Jaccard 0.7, same file only | 0.3988 | 0.7518 | 0.3300 | 0.6519 | 0.4121 | 0.5089 |

Passages dropped from the first 30 (all queries): Jaccard 0.7 drops 4 (Cranfield), 0 (SciFact), 52 (NFCorpus), 39 (MS MARCO), 44 (TREC-COVID); same-file only drops none on any set (these documents are one to three chunks without overlap). The losses are real duplicates: the sets contain the same text as different judged-relevant files (MS MARCO pools each query's candidates, TREC-COVID repeats records), so dropping the later copy drops a relevant file. As a presentation fix for the evidence brief (3 passages of the best 20), counting distinct relevant files in it divided by 3: none 0.3750 / 0.2483 / 0.3444 / 0.2444 / 0.5267 (mean 0.3478); Jaccard 0.7: 0.3750 / 0.2483 / 0.3444 / 0.2333 / 0.5333 (0.3469); Jaccard 0.9: 0.3750 / 0.2483 / 0.3444 / 0.2389 / 0.5333 (0.3480). Neutral on the mean but it loses MS MARCO, so by the rule it is not applied. What was kept: `evidence::select_evidence_with(hits, Some(limit))` (the default `select_evidence` is unchanged: it still drops only identical passages) and `ranking::{shingles, jaccard, NEAR_DUPLICATE}`, with a unit test, so the measurement is reproducible and a UI that wants to collapse copies (show "also in: ..." rather than hide a file) can use them. Collapsing in the display keeps the file ranking unchanged by construction.

### 5. Small learned fusion: not adopted

Pointwise logistic regression (L2, set-balanced) on the top 100 chunks of `default` per query, label = file judged relevant; features: normalised dense and BM25 scores, both times query length, log chunk length, first-chunk flag, relative position in file, BM25 > 0 (8 weights). Trained on the queries of one parity, scored on the other, both folds together covering all queries.

| nDCG@10 | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | Mean |
|---|---|---|---|---|---|---|
| default | 0.3988 | 0.7518 | 0.3300 | 0.6519 | 0.4121 | 0.5089 |
| learned (held out) | 0.3911 | 0.7425 | 0.3390 | 0.6541 | 0.4114 | 0.5076 |

Recall@100 0.7176 to 0.7153. Loses on three sets; the weights also moved between folds (the dense weight stayed near 5.4 to 6.1, query-length interactions changed sign), i.e. no stable constants to put in code. Negative; the fused score with one keyword share is already close to what these features can give, and chunk length and position carry no signal on documents of one to three chunks.

### Cost and code

Query-time cost of the experiments is not a product cost since nothing is adopted. Reference: `rank_features` plus 17 share fusions plus the other variants took 70 to 110 ms per query on this loaded machine (load average 35), of which the fusion itself is under 1 ms per share.

Left in the code: `SemanticIndex::rank_features` and `ranking.rs` (raw per-chunk scores, shingle helpers; no filtering, no extension or freshness checks, measurement only), `evidence::select_evidence_with`, `retrieval_eval --rank-experiments <outdir> [--weights file]` with `examples/support/ranking_experiments.rs`. Tests: `ranking::tests::{shingle_similarity_separates_copies_from_different_text, features_rebuild_the_search_order}`, `evidence::tests::near_duplicate_limit_is_opt_in_and_search_keeps_copies`.

## 2026-11-10: semantic indexing (chunking, incremental updates, large indexes)

Bundled MiniLM, fused `default` variant, nDCG@10, harness `search-core/examples/retrieval_eval.rs`, one binary for all runs so ranking code was identical across variants. Machine was shared with other jobs, so timings are rough; quality numbers are exact.

### 1. Boundary-aware chunking: not adopted

Shipped chunking cuts the character stream every 800 characters wherever that falls. Tried two variants, each under its own tag (`chunk-eval-b`, `chunk-eval-bo`), selected by a temporary environment switch (removed again):

- `b`: split each line into sentences (after `.`, `!`, `?` plus closing quote or bracket, whitespace, and a next character that is not lower case), pack whole sentences and lines into chunks of at most 800 characters, close a chunk at a blank line once it holds 400 characters, never cross a PDF page. Over-long sentences are cut at the last space in their final quarter. Cited first/last line come from the pieces themselves, so they are exact by construction.
- `bo`: the same plus an overlap: a chunk closed because the next sentence did not fit starts with the trailing whole sentences of its predecessor, up to 100 characters (about 12 percent).

Cranfield, 200 queries (baseline `default` reproduces the documented 0.3988):

| Variant | nDCG@10 | Recall@100 | MRR@10 |
|---|---|---|---|
| fixed 800 (shipped) | 0.3988 | 0.8020 | 0.5573 |
| sentence boundaries | 0.3947 | 0.8046 | 0.5517 |
| boundaries + 100-char overlap | 0.3947 | 0.8041 | 0.5517 |

Both variants lose 0.0041 nDCG@10 on Cranfield, which fails the owner rule (a change must lose on none of the five sets). The other four sets were not completed: the run was stopped once the rule was already failed (SciFact indexing under CPU contention takes tens of minutes per variant, and a result there cannot change the verdict). Likely reason: Cranfield abstracts are about 1,000 characters, so fixed chunking gives an 800 plus a 200 character tail, which the dense model and keyword scorer both handle fine, while sentence packing splits them into two roughly equal halves that each carry less of the vocabulary. Unit tests for the chunker (sentence ends, 800-character cap, overlap, exact cited lines on 30 line input, paragraph break, page separation, unbroken text, abbreviation followed by lower case) passed while it was in the tree; the code and tests were removed with the negative result, so chunking is byte-for-byte the shipped behavior and no index version constant was bumped.

Side effect worth knowing: with fixed boundaries an insert near the top of a file re-embeds every later chunk. A boundary-aware chunker would reduce that, but the benefit does not outweigh the retrieval loss.

### 2. Incremental updates: audit, two gaps fixed, tests added

Already correct: per-file freshness is `(modified, size)` (`is_current`), then a SHA-256 of the extracted lines and pages decides whether vectors are kept; unchanged chunk text keeps its vector within a file; each document is one transaction (so an interrupted build keeps finished documents and the next run embeds only the rest); deleted source files are removed at the end of a build (`remove_orphaned_documents`, skipped if the build was cancelled).

Gaps fixed in `search-core/src/semantic_index.rs`:

1. A copy, renamed or moved file, or the same document under a second revision folder was embedded again in full. Now a document whose content hash and citation version equal another indexed document's reuses that document's chunk vectors (new index `semantic_documents_hash`; no schema break, old databases get the index on open).
2. `PRAGMA foreign_keys = ON` was executed inside the schema transaction, where SQLite ignores it, so the chunk cascade on replace and delete worked only because the bundled SQLite happens to default foreign keys on. It is now set after the transaction on every connection.

New tests: `incremental_updates_embed_only_changes_and_leave_no_orphan_rows` (restamp embeds 0, appended line embeds only the new tail, copy embeds 0, replacement leaves no stale rows, orphan cleanup keeps live files, no chunk row without a document) and `an_interrupted_build_resumes_without_redoing_finished_documents`. A private `index_document_counting` returns the number of chunks that went through the model so tests can assert it.

Known and left alone: a same-size edit that restores the original modification time to the nanosecond is not detected (same as the exact index); a single very large document is lost if cancelled mid-embedding (atomic per document).

### 3. Large-index search: approximate nearest neighbour not adopted

Probe: `search-core/examples/ann_probe.rs` (`cargo run -p search-core --release --example ann_probe -- <N> <semantic.sqlite>...`). Corpus: the 35,449 real chunk vectors of the five benchmark indexes seed N synthetic chunks; 200 held-out real vectors are the queries (never in the corpus). Two corpus shapes: diffuse (each chunk a random 0.3 to 0.7 mix of two seeds plus noise, the hard case) and clustered (one seed plus noise of norm 0.8, the easy case for IVF). IVF is spherical k-means (500 lists, 8 iterations on a sample), probe the nearest `nprobe` lists, exact dot products inside them. Recall is recall@100 against the exact f32 top 100.

N = 250,000 (the current snapshot limit), exact scan: 36 to 41 ms per query on one thread, 19 to 20 ms on 8 threads (the snapshot code does the same lane-wise dot product); 100,000 chunks: 14 ms single thread.

| nprobe | diffuse recall@100 | diffuse share scanned | clustered recall@100 | clustered share scanned |
|---|---|---|---|---|
| 8 | 0.738 | 2.7% | 0.849 | 2.2% |
| 16 | 0.831 | 5.1% | 0.918 | 4.2% |
| 32 | 0.897 | 9.4% | 0.964 | 8.1% |
| 64 | 0.939 | 17.3% | 0.985 | 15.1% |
| 128 | 0.966 | 31.3% | 0.995 | 28.5% |
| 256 | 0.987 | 57.3% | 1.000 | 54.4% |

Reaching 0.98 needs 15 percent of the corpus in the easy case and 57 percent in the hard case. Timings of the IVF runs were distorted by CPU contention and random access into the lists, and did not beat the exact scan at equal recall; the best case is about 6 times fewer dot products on a scan that is already 20 to 40 ms. K-means build took 85 to 230 seconds at 250,000 vectors and would have to be redone on every write generation. IVF also does not carry the hybrid keyword side: lexical matches that are far from the query vector in embedding space fall outside the probed lists, so the fused ranking would change. Verdict: not adopted (recall at equal speed fails the 0.98 bar on the diffuse corpus; the speed gain is small on the easy one). The 250,000 limit is a memory limit (about 1.5 KB per chunk plus postings), the snapshot build time and the per-file `stat`, not the vector scan.

Side measurement: an int8 copy of the vectors (per-vector scale) kept recall@100 at 0.995 against exact f32 at 100,000 and 250,000 chunks with a quarter of the memory. My probe's int8 loop was not vectorised, so its 250 to 700 ms is not a real speed figure. If large indexes matter, quantised snapshot vectors (and a higher `MAX_CACHED_CHUNKS`) is the better lever than ANN; not built, because it touches the snapshot scoring owned by the ranking work and would break the exact-equality test against the streaming path.

## Regular (exact) search: findings, measurements, decisions (2026-11-10)

Scope: search-core (matching, cache, native_index narrowing, regex_literals, new textfold / ordering / explain) and nothing in app-tui or native-search code.

### New settings fields (SearchSettings, both with serde default, defaults keep today's behaviour)
- `fold_accents: bool` (default false). Literal and whole-word modes only (ignored with regex). Folds precomposed vs combining accents (`Caf\u{e9}` = `Cafe\u{301}` = `cafe`), ss/ae/oe/th letters, ligature code points (fi, fl, ffi...), no-break and other Unicode spaces to a space, curly quotes and en/em dashes to ' " -, soft hyphen and zero-width characters dropped. Applies to filters, excludes and lines identically, so plain text, Office, ZIP, RTF and PDF text all compare the same way.
- `typo_tolerance: u8` (default 0 = off). 1 = one edit on words of 4+ letters; 2 = up to two edits on words of 8+ letters (Damerau, adjacent swap counts once). Words under 4 letters and words containing a digit stay exact (M8 never matches M9). Multi-word filters match consecutive words, each within its bound. Search filters only, never excludes. Literal and whole-word modes.
- Both turn off index narrowing (the index stores unfolded text and exact words), so a warm re-search with either option scans all files in scope. Both are part of the cache fingerprint only when non-default, so existing caches stay valid.
- UI notes: the Ignore accents toggle and Typo tolerance (0-2) are wired as Search settings 32 and 33. Highlighting should use `explain::LineExplainer::spans(line)` (byte spans in the original line) instead of a substring search, or folded or fuzzy hits will not highlight.

### New APIs (no change to LineHit / FileSearchResult, so app-tui literals still compile)
- `ordering::relevance_order(results, settings) -> Vec<usize>` and `score_results` (score, matched filters, filename filters, hit lines). Found order stays the default (`file_results` is untouched).
- `explain::LineExplainer::new(settings)?.spans(line) -> Vec<MatchSpan{start,end,filter}>`; `explain::explain_file(results, settings) -> Vec<Option<FileWhy>>` (per filter: hit lines and first line; filters found in the file name; proximity range; score). Computed on demand, nothing stored, no tokens.
- `regex_literals::required_literal_alternatives`, `textfold::{fold_lower, FuzzyFilter}`.
- Harnesses: `examples/exact_bench.rs` (speed), `examples/exact_order_eval.rs` (ordering, BEIR-format folders).

### Speed (3000 files x 150 lines, this Mac, release; not an SLA)
In-memory matching, 150k lines: literal 437 -> 71 ms (6x); whole word 5236 -> 181 ms (29x); regex unchanged (62-77 ms, noise); literal with three filters that hit 63 % of lines 836 -> ~1000 ms (the prefilter is pure overhead when most lines match; the only regression found). Cause of the old costs: literal lowercased and allocated for every line; whole word ran a lookaround regex (about 20-35 us a line) once for the combined check and again per filter.
How: an ASCII case-insensitive prefilter regex rejects lines before lowercasing (exact: only U+212A and U+0130 lowercase to ASCII; U+212A is matched by (?i), lines with U+0130 skip the prefilter; proved by an exhaustive test over all code points), and whole word on ASCII lines checks word boundaries by hand (differential test against the regex).
Stages for 3000 files: std read 190 ms, extract 357 ms, match 158 ms (was ~600+); cold full scan sequential 0.9-2.5 s (noisy, other builds ran concurrently), parallel 0.33-0.45 s; warm re-run through the JSON cache 60-100 ms; index narrow 63-72 ms (walk 31, freshness 27, query 1) then verify 12 ms for 31 candidates; index build 7-8 s.
Open question: the UI default `parallel: false` makes a cold scan about 3-7x slower than it needs to be on this corpus; consider defaulting to `parallel: true` (SearchSettings::default is also false; I did not change it because it is not a pure speed knob, progress ordering changes).

### Cache and index correctness (bugs fixed)
1. Cached `ReadError`/`UnexpectedError` results were replayed on every later run while (size, mtime) matched, so a file locked once stayed "unreadable" until edited. Now never reused (orchestrator.rs). Test: `a_cached_read_error_is_retried_not_reused`.
2. Racily-clean entries: with a coarse filesystem mtime and an unchanged size, an edit made right after a run was invisible. The cache now stores `saved_at_ns` and drops entries whose mtime is within 2 s of it on load (old cache files without it are unaffected). Test: `a_file_modified_just_before_the_cache_was_saved_is_not_trusted`. Existing test `incremental_cache_reuses_unchanged_files` now ages its file.
3. `max_embed_lines` (lines stored in cached results) was not in the fingerprint; added at non-default values. (OCR and PDF timeout were added to the fingerprint concurrently by another agent; left as is.)
4. Files changing during a run were already safe (the cache stores the walk-time mtime, so a mid-run edit mismatches next run).
5. `regex_literals`: escapes with operands (`\x41`, `\pL`, `\1`, `\u{..}`) kept their operand characters as required literal text, so index narrowing could drop real matches (`\x41BCD` demanded "41BCD"). Now bail. Test checks chunk sets against real matches.
Index freshness check itself (size + mtime ns) is unchanged: same-size same-mtime edits remain invisible (documented in AGENTS.md).

### Narrowing coverage added
Regex filters with character classes (`eng[ei]ne` -> "eng") and top-level alternation (`torque.*value|bolt load`) now narrow through the index (OR of chunk sets); groups still bail. Not measured separately: it only turns a full scan into a trigram lookup for those patterns, which the earlier numbers (63 ms narrow + 12 ms verify vs 330-900 ms scan) bound.

### Ordering evaluation (the new `ordering` module)
Method: exact matcher with the query's content words as whole-word filters (any-line), every document split into sentence lines, hit files ordered three ways, nDCG@10 averaged over queries with hits. Found order = corpus order (arbitrary, as completion order is), "hit lines" = most matched lines first.
- Cranfield (225 queries): found 0.0096, hit lines 0.0970, relevance 0.3372
- SciFact (300 queries): found 0.0083, hit lines 0.1320, relevance 0.6663
- NFCorpus: found 0.1156, hit lines 0.2156, relevance 0.3337 (wins on all three sets)
Score = BM25 over hit lines per filter (tf = lines matching the filter, idf over all files searched, line count as length) plus a filename-match bonus (0.6 x idf; not exercised by these corpora, whose file names are ids, so that weight is unevaluated and conservative). Adopted as an opt-in order; found order unchanged by default. Caveat: queries are natural-language questions turned into any-line filters, a harder case than a few hand-typed keywords; the gap to found order is the point, the absolute numbers are not an SLA. A hand-labelled engineering fixture was not built: the three public sets are labelled and larger.

### Rejected / not done, and why
- Word-distance proximity (`a NEAR/5 b`): the existing syntax is `[near N]` in LINES (`N = 0` means the same line) and phrases are plain substrings with spaces. Same-line adjacency is already `[near 0]`; a word-distance operator needs parser changes in app-tui (Filters field), which I was told not to touch, and would not change which files match in practice. Not built then; built later as `search-core/src/near.rs` (see `knowledge-engine-audit.md` section 8, 2026-11-11 NEAR/n).
- Phrase across a wrapped line break: not built; PDF line joining belongs to the extraction agent, and joining line pairs would change hit line numbers. Worth a look once extraction output is stable.
- Dropping the Greek final-sigma / Turkish-i edge cases: noted only, no engineering corpus impact.
- Folding in regex mode: a folded pattern would break regex syntax; ignored there by design.
- Proximity bonus in the order score: not evaluated (the corpora have no filter proximity signal beyond co-occurrence), left out rather than guessed.

## 2026-11-10: OCR and PDF indexing in search-core: findings, measurements, decisions

Machine note: all timings were taken with a load average around 30 (other agents building), so absolute numbers are inflated; only before/after ratios taken in the same session mean anything.

### Open issues from the log

**#0127 page mapper.** The mapper does not reject the bundled 144-page `benches/data/xlarge.pdf`. A trace (the module compiled standalone) shows `Pdf::open` succeeds, all 7109 xref objects parse, and `page_by_stream_offset` returns 147 stream offsets. The previously ignored test `real_large_pdf_keeps_bounded_page_numbers` passes unchanged in release (1.8 s) and debug (13.8 s). I could not reproduce the reject; the likeliest cause is that extraction hit the test's 30 s budget in a loaded debug build, which cuts the text before page 144 (a hypothesis, not proven). The test is un-ignored and now gives extraction 300 s. Known real limit: the mapper only handles classic xref tables (no xref streams, object streams, `/Prev`, `/Encrypt`), so most PDFs written as 1.5+ with compressed object streams get line-only citations. Supporting them would mean parsing xref streams, PNG predictors and object streams; not done.

**#0119 sticky OCR results.** Confirmed two real bugs. (1) `cache::compute_fingerprint` ignored `ocr_scanned_pdfs` and `pdf_timeout_seconds`, so a result cached without OCR was reused after switching OCR on. Both now enter the fingerprint (omitted at their defaults so old caches stay valid). (2) `failure_log` recorded "extractor produced no usable text" for a scanned PDF and `known_failure_reason` then skipped the file forever, even with OCR on. Failures now carry a context number (`failure_log::extraction_context`: OCR flag and PDF timeout); a mismatching context is not a known failure. Old databases get the new column on open. Tests: `fingerprint_changes_with_ocr_and_pdf_timeout_but_not_at_defaults`, `a_failure_is_not_sticky_across_an_ocr_or_timeout_change`, `a_database_without_the_context_column_is_upgraded_in_place`. Not fixed (other agent's file): `native_index` skips a PDF whose path, size and time are unchanged, so a PDF indexed without OCR stays text-less in the fast index after OCR is switched on; the index stamp would need the OCR flag.

**#0116 recognition confidence.** Not obtainable. ocrs 0.12 `TextChar` is a character plus a box, the recogniser module is private, and the CTC decode drops the score. A real score needs a patched ocrs. Nothing in search-core or the report text calls detection probability "confidence"; the only user-visible wording is the pre-existing `low_confidence_pdf` flag (set for any OCR output and for the text-layer heuristic) and the report note "may be incomplete or misrecognized". No change made, negative result recorded.

### Measurements

Extraction (release, no OCR): `medium.pdf` 0.3 MB 0.09 s (3349 lines); `xlarge.pdf` 5.9 MB, 144 pages, 0.78 s (about 5 ms per page, 23391 lines); `large.pdf` 0 lines is correct (a form PDF; the `Tj` matches are inside font programs); `xlarge-scanned.pdf` 38.6 MB 0.84 s, 0 lines without OCR.

OCR on `xlarge-scanned.pdf` (100 pages, 2479x3509 raw RGB): before my change a 600 s timeout run produced 1290 lines because every page image was decoded first (about 26 MB each held at once, no progress until the first page finished). Now 8 pages in 46 s under load with the first progress call at 0.14 s and the longest gap one page (8 s). A second run over the same file recognised the first 8 pages from the cache instantly and continued.

Cache: generated 600 dpi page, cold 34.7 s, warm 0.09 s; 300 dpi page 31.2 s to 0.07 s.

Preprocessing (generated scans with known text, word recall against ground truth, same engine): full size / longest side capped at 2480 -- clean 300 dpi 0.997 / 0.998; noisy small type 0.966 / 0.981; 200 dpi 0.994 / 0.994; 600 dpi 0.974 / 0.997; 150 dpi 0.965 / 0.965. Capped at 1800: clean 0.997, noisy 0.981, 200 dpi 0.999, other pages not finished, speed not measured reliably under load; not adopted. Adopted the 2480 cap (images already below it are untouched, so the real 100-page fixture is unchanged).

De-hyphenation: `xlarge.pdf` has 2315 lines ending in a letter and a hyphen followed by a lowercase line; 1495 were repaired (the rest are real compounds such as pre-training/in-context, or words never written whole elsewhere in the document). Samples checked by eye: espe-/cially, anno-/tations, Trans-/formers all correct; compounds kept. A first version left a stray ". " at the start of the next line; closing punctuation now moves with the fragment.

### Added

- `line_quality::dehyphenate` (document-vocabulary rule, page-boundary safe), versions bumped: `cache::EXTRACTION_VERSION` 3, `native_index::INDEX_SEMANTIC_VERSION` 5, `semantic_index::CITATION_VERSION` 4.
- Lazy page-by-page OCR with per-page timeout and progress, per-image OCR cache (memory, plus disk when `SearchSettings::ocr_cache_dir` is set; wired in the orchestrator via `extraction::configure_ocr_cache`), image cap 2480 px.
- Fingerprint and failure-log context for OCR and PDF timeout.
- Robustness test for truncated, encrypted-looking and stream-flooded PDFs (all finish; no hang found, so no code change).
- Examples `pdf_extract_bench` (timing, progress behaviour, `--ocr`, `--cache=DIR`, `--timeout=N`) and `pdf_dump` / `pdf_dump_ocr`.

### Not done, with reasons

- OCR only when the whole text layer is empty; PDFs mixing text pages and scanned pages are not OCR'd. A per-page decision needs a page map for non-classic PDFs; no evidence gathered.
- Column reading order: no fixture or measurement; the extractor emits text in content-stream order.
- Encrypted PDFs report the generic "no usable text" reason; a distinct reason needs a new `ExtractLinesError` variant touching other agents' code.
- Hashing a whole file (rather than an image stream) for the OCR cache: the stream hash already skips decoding and recognition, which are the costs.

## 2026-11-10: rarity of words (P1-5 and P1-6): neither adopted

Question from the owner: can the rarity of words improve semantic search? The keyword side already weights terms by BM25 idf, so the tests are about the dense side and the balance between the two. Adoption rule as before: no loss on any of the five sets (Cranfield 200 queries, SciFact 200, NFCorpus 60, MS MARCO 60, TREC-COVID 50). **Nothing adopted; `default` unchanged; no stored data changed, `EMBEDDING_MODEL_VERSION` stays 1.**

### P1-5 rarity-aware keyword share: not adopted

Per query, share = a + b * r, clamped to 0..1, where r is the mean (or the maximum) over the query terms of idf / ln(2n + 1) computed over all chunks of the index with the keyword score's own formula (0 = term in nearly every chunk, 1 = in none). Queries without terms keep share 0.35. Run through `retrieval_eval --rank-experiments` against `default`; the feature path reproduces `default` exactly (0.3988 / 0.7518 / 0.3300 / 0.6519 / 0.4121). nDCG@10 and the change against `default`:

| Variant (r, a, b) | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | Mean | Sets lost |
|---|---|---|---|---|---|---|---|
| default | 0.3988 | 0.7518 | 0.3300 | 0.6519 | 0.4121 | 0.5089 | 0 |
| mean, 0.15, +0.40 | -0.0045 | +0.0017 | +0.0014 | -0.0049 | +0.0021 | 0.5080 | 2 |
| mean, 0.20, +0.30 | -0.0035 | +0.0026 | +0.0019 | -0.0046 | +0.0031 | 0.5088 | 2 |
| mean, 0.25, +0.20 | -0.0023 | +0.0022 | +0.0020 | -0.0056 | +0.0036 | 0.5089 | 2 |
| mean, 0.45, -0.20 | -0.0005 | -0.0016 | -0.0018 | +0.0014 | +0.0021 | 0.5088 | 3 |
| mean, 0.55, -0.40 | -0.0047 | -0.0038 | -0.0022 | +0.0039 | -0.0007 | 0.5074 | 4 |
| max, 0.15, +0.30 | -0.0068 | -0.0010 | -0.0000 | +0.0080 | +0.0014 | 0.5092 | 3 |
| max, 0.25, +0.15 | -0.0023 | -0.0000 | +0.0049 | -0.0003 | +0.0034 | 0.5100 | 3 |
| max, 0.45, -0.15 | +0.0020 | +0.0019 | +0.0017 | -0.0005 | -0.0018 | 0.5096 | 2 |

Both signs of the slope were tried (more keyword weight for rare queries and less) and neither is consistent: the best means (+0.0003 to +0.0011) sit inside the set-to-set noise, and every variant loses at least one set. Mean rarity per set differs (Cranfield 0.37, SciFact 0.45, NFCorpus 0.53, MS MARCO 0.53, TREC-COVID 0.28) but the best slope does not follow it. Consistent with the query-length result above: one keyword share is already close to what a per-query signal can give. Negative.

### P1-6 idf-weighted pooling of token vectors: not adopted

`examples/idf_pooling_eval.rs` re-embeds each set with token vectors (`search_ml::embed_tokens`), pools them with weight idf^p, where idf is the WordPiece token's corpus idf in that set (an upper bound: a stored vector cannot depend on a corpus that changes, so a static weight would do no better), and fuses with BM25 at share 0.35. It uses its own 120-word chunker, so the control (`uniform`, plain mean of the normalised token vectors) is compared with the variants on identical candidates; the control's fused numbers differ slightly from `default` (0.3912 / 0.7373 / 0.3344 / 0.6679 / 0.4045). nDCG@10 fused, change against `uniform`:

| Variant | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | Sets lost |
|---|---|---|---|---|---|---|
| uniform (control) | 0.3912 | 0.7373 | 0.3344 | 0.6679 | 0.4045 | 0 |
| idf, passage and query | -0.0012 | +0.0050 | -0.0012 | -0.0111 | -0.0085 | 4 |
| idf, passage only | +0.0015 | +0.0015 | +0.0027 | +0.0089 | -0.0109 | 1 |
| idf^0.5, passage and query | +0.0029 | +0.0045 | +0.0018 | -0.0124 | -0.0115 | 2 |
| idf^2, passage and query | -0.0042 | +0.0026 | -0.0002 | -0.0119 | -0.0082 | 4 |

Dense only (control 0.3485 / 0.6690 / 0.3264 / 0.6611 / 0.3432): idf-weighted passage and query gains on MS MARCO (+0.0240) and TREC-COVID (+0.0170) but loses NFCorpus (-0.0058); once fused with BM25 (which already carries rarity) the gain disappears and TREC-COVID loses 0.009 to 0.011 in every variant. The best (passage only) loses TREC-COVID by 0.011. Negative; with a corpus-dependent weight it would also need a re-embed on every index change, so it is dropped rather than refined.

Left in the code: `FeatureSet::term_idf` (measurement only), the `rar-*` variants in `ranking_experiments.rs` (they also write a `rarity-mean` row per query to `perquery.tsv`), `examples/idf_pooling_eval.rs`. Reproduce: `retrieval_eval <set> --queries N --tag rank-eval --rank-experiments <outdir>`; `idf_pooling_eval <set> --queries N` (about 90 s on Cranfield, 8 minutes on SciFact, this Mac).

## 2026-11-11: graph-based query expansion from relationship edges (P3-1): not adopted

Question: do relationship edges (`search-core/src/catalog_edges.rs`) help retrieval? Method (`examples/edges_eval.rs`, debug build, whole query sets, `LexicalConfig::default()` through `SemanticIndex::search_tuned` on each set's existing `.eval-index`): build the edges for each set's documents (entities as `Catalog::index_entities` stores them, content hash = SHA-256 of the text; names are ids, so there are no `VERSION_OF` edges); take the top 3 documents of the default ranking; collect their neighbours over the variant's edges; append to the query the 8 highest-idf terms (tf times ln(N/df), df >= 2, not already in the query) of those neighbours; search again. Queries with no neighbour keep their baseline ranking ("affected" counts the rest). The control takes the same terms from the top hits themselves (plain pseudo-relevance feedback, same queries as `graph-all`). Baselines reproduce the table above (Cranfield 0.3936, SciFact 0.7315, NFCorpus 0.3456, MS MARCO 0.6632, TREC-COVID 0.4121).

Mean nDCG@10 over all queries (change against `default`), affected queries in brackets, and the mean change on the affected queries; "up/down" counts queries whose nDCG rose and fell:

| Variant | Cranfield (225) | SciFact (300) | NFCorpus (323) | MS MARCO (484) | TREC-COVID (50) | Sets lost |
|---|---|---|---|---|---|---|
| default | 0.3936 | 0.7315 | 0.3456 | 0.6632 | 0.4121 | 0 |
| graph-facts (duplicate, version) | +0.0000 [0] | +0.0000 [0] | -0.0008 [15] (aff. -0.0181, 7 up, 6 down) | +0.0013 [7] (aff. +0.0912, 3 up, 0 down) | +0.0000 [0] | 1 |
| graph-entity >= 0.9 | -0.0017 [10] | -0.0484 [69] (aff. -0.2103, 5 up, 39 down) | -0.0252 [52] (aff. -0.1567, 6/29) | -0.0045 [10] (aff. -0.2175, 2/8) | -0.0025 [4] | 5 |
| graph-entity >= 0.5 | -0.0017 [11] | -0.0616 [100] (aff. -0.1848, 8/52) | -0.0353 [73] (aff. -0.1560, 8/42) | -0.0065 [14] (aff. -0.2260, 3/11) | -0.0154 [8] (aff. -0.0961, 2/6) | 5 |
| graph-all >= 0.5 | -0.0017 [11] | -0.0616 [100] | -0.0369 [87] (aff. -0.1369, 14/48) | -0.0052 [21] (aff. -0.1203, 6/11) | -0.0154 [8] | 5 |
| control: terms of the top hits | -0.0016 [11] (aff. -0.0332, 4/4) | -0.0242 [100] (aff. -0.0725, 17/37) | -0.0028 [87] (aff. -0.0102, 27/31) | +0.0015 [21] (aff. +0.0357, 7/6) | +0.0082 [8] (aff. +0.0514, 4/4) | 3 |

Result: every graph variant that uses shared entities loses on all five sets and by a large margin on the affected queries (-0.12 to -0.23 nDCG@10), clearly worse than plain pseudo-relevance feedback from the hits themselves, so the loss is not just the cost of expansion: a shared stored entity between two scientific abstracts (gene or chemical names, section labels, "CD4", "IL-6") is a weak signal of the same topic, and the neighbours' terms pull the query away from the question. The only variant that does not lose clearly uses duplicate edges, and it is positive on one set (MS MARCO, 7 queries with an exact duplicate passage among the top 3, +0.0912 on those) and negative on another (NFCorpus -0.0008). It fails the rule (lose on no set) and the evidence is 22 affected queries in all. **Nothing adopted; defaults and stored data unchanged.**

Edge quality on the same sets, using co-relevance as a proxy label (two documents judged relevant to a common query; a proxy, not a label of "related"), only for edges whose two ends are both judged relevant to something: SAME_ENTITY edges link co-relevant documents more often than random judged pairs do, and more so at higher confidence, e.g. NFCorpus 0.336 (all, 2926 scorable edges) / 0.378 (>= 0.5) / 0.422 (>= 0.9) against base rate 0.173; TREC-COVID 0.078 / 0.302 / 0.333 against 0.021; Cranfield 0.147 (102 scorable) against 0.019; SciFact 0.250 (12 scorable) against 0.0015 and MS MARCO 0 of 1 are too few to read. So the edges carry real signal for "which documents belong together", which is what navigation and explanation need; the signal does not survive being turned into extra query terms. Edge counts: Cranfield 161, SciFact 5,403, NFCorpus 3,969 (40 duplicate), MS MARCO 180 (14 duplicate), TREC-COVID 1,835; edge build 0.2 to 0.6 s per set. What the public sets cannot show: they contain no file-name versions and almost no duplicates, so expansion over true duplicates or versions, and the value of edges for navigation (related-file panel, result explanations), are untested here. Reproduce: `cargo build -p search-core --features semantic-search --example edges_eval`, then `edges_eval <set folder>` (about 2 to 10 minutes per set in a debug build on this Mac). The labelled-set precision and recall of the edge types are in `docs/knowledge-engine-audit.md`, section 8, 2026-11-11.

## 2026-11-11: P2-4 int8 vectors in the in-memory snapshot: not adopted

Idea: hold the query-time snapshot (`semantic_cache::Snapshot`) with one signed byte per dimension plus a scale per vector, so an index above the 250,000-chunk limit could still be searched from memory (earlier ANN probe: quarter vector memory at recall@100 0.995). Built and tested: scores of the int8 snapshot stayed within 0.01 of the exact ones and gave the same chunk order on a three-chunk index (bundled MiniLM). Memory was measured on a synthetic index of 100,002 chunks of 120 words each (20,000-word vocabulary), release build, resident-set change across `Snapshot::build`: f32 3.87 KB per chunk, int8 3.81 KB per chunk. Peak process memory (`/usr/bin/time -l`, debug) was 404 MB against 417 MB.

Reading: the vectors are only part of a snapshot; the term postings for the keyword side are the rest, and they do not shrink. By arithmetic the vector part is 1,536 B (f32) or 388 B (int8) per chunk, a possible saving of about 1.1 KB of roughly 3.9 KB (28%), and the measurement did not show even that (resident-set deltas include allocator slack, so the figure is an upper bound on the gain, not a precise one). Raising the in-memory limit four times would put 1,000,000 chunks at about 3.8 GB, which is not acceptable on the 8 GB reference Mac, and the limit of 250,000 chunks (about 1 GB at this chunk size, not the 400 MB the old comment says) was left alone. Not adopted; the code was reverted, nothing stored changed. A real answer for indexes beyond the limit has to shrink the postings (for example a disk-backed postings list or a keyword path off the snapshot) or use the ANN probe's IVF; neither was attempted. Note for later: the comment on `MAX_CACHED_CHUNKS` ("about 400 MB") understates the snapshot for 120-word chunks.

## 2026-11-11: embedding only part of the documents (P2-3, lazy embedding)

Question: what does a reader lose while only a fraction of the documents has vectors, and how much indexing time does the fraction save? `retrieval_eval ... --lazy 25,50,75,100` builds a second index in steps through the real `SemanticIndex::index_document`, in a fixed pseudo-random document order (FNV hash of the id). The sets have no folders or extensions, so this is the case where priority carries no information; a real priority order (prose before code, important folders first) can only do better than this when it correlates with relevance, and that was not measured. After each step it reports the shipped `default` hybrid ranking over the embedded part alone (`partial`) and `+exact`: that ranking merged by reciprocal rank (k = 60) with the keyword-led ranking (keyword weight 1000, rank fusion) of the not yet embedded documents taken from the fully built index. `+exact` is a simulation of what exact search adds, not shipped ranking; the app shows the exact and semantic lists separately. 200 queries per set (50 for TREC-COVID). Run serially on a Mac that was also running other builds, so seconds are relative; they are the embedding and write time of the step only.

nDCG@10 / Recall@100, `partial` (hybrid over the embedded documents only):

| Embedded | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | mean nDCG | mean R@100 |
|---|---|---|---|---|---|---|---|
| 25% | 0.1464 / 0.2284 | 0.1883 / 0.2039 | 0.1539 / 0.1118 | 0.1229 / 0.2000 | 0.3254 / 0.2080 | 0.1874 | 0.1904 |
| 50% | 0.2083 / 0.3600 | 0.4199 / 0.5137 | 0.2418 / 0.1581 | 0.2867 / 0.4600 | 0.3684 / 0.3430 | 0.3050 | 0.3670 |
| 75% | 0.3155 / 0.6024 | 0.5754 / 0.7351 | 0.3171 / 0.2299 | 0.4902 / 0.7350 | 0.4036 / 0.4580 | 0.4204 | 0.5521 |
| 100% | 0.3988 / 0.8020 | 0.7518 / 0.9700 | 0.3836 / 0.2970 | 0.6670 / 1.0000 | 0.4121 / 0.5520 | 0.5227 | 0.7242 |

`+exact` (simulated union with the keyword ranking of the pending documents):

| Embedded | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | mean nDCG | mean R@100 |
|---|---|---|---|---|---|---|---|
| 25% | 0.2885 / 0.7369 | 0.6097 / 0.9278 | 0.3213 / 0.2756 | 0.3854 / 0.9975 | 0.3518 / 0.4805 | 0.3913 | 0.6837 |
| 50% | 0.3307 / 0.7633 | 0.6374 / 0.9480 | 0.3240 / 0.2797 | 0.4094 / 0.9975 | 0.3803 / 0.5235 | 0.4164 | 0.7024 |
| 75% | 0.3328 / 0.7567 | 0.6262 / 0.9515 | 0.3135 / 0.2883 | 0.4392 / 0.9975 | 0.3570 / 0.5035 | 0.4137 | 0.6995 |

Cumulative indexing seconds (embedding plus write) to reach each step, with the share of the full time saved by stopping there:

| Embedded | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID |
|---|---|---|---|---|---|
| 25% | 36.6 (74%) | 141.1 (74%) | 103.7 (74%) | 26.9 (74%) | 63.2 (76%) |
| 50% | 71.3 (50%) | 267.8 (50%) | 201.1 (49%) | 52.3 (50%) | 124.9 (52%) |
| 75% | 108.9 (23%) | 403.0 (25%) | 300.4 (25%) | 78.1 (25%) | 197.7 (25%) |
| 100% | 141.8 | 538.6 | 398.0 | 104.3 | 262.0 |

Findings (all numbers above, wins and losses):
- Time is linear in documents embedded: stopping at 25% saves 74 to 76% of the embedding time on every set, 50% saves about half, 75% about a quarter.
- Embedded-only recall falls faster than linearly, so a half-embedded index is not "half as good": mean nDCG@10 is 0.305 at 50% against 0.523 at 100% (58%), and 0.187 at 25% (36%). Recall@100 falls roughly in proportion to the embedded share (0.19 / 0.37 / 0.55 / 0.72 for 25 / 50 / 75 / 100%). Whatever is not embedded cannot appear in the semantic list.
- Keyword results cover most of the gap, unevenly. The simulated union recovers Recall@100 to 0.68 to 0.70 mean at every fraction (full 0.724) and gets mean nDCG@10 to 0.39 to 0.42 (full 0.523); on SciFact and MS MARCO (short, keyword-rich) it reaches 0.93 to 1.00 Recall@100 at 25%, on NFCorpus (medical, vocabulary mismatch) it stays near 0.28 at every fraction because the keyword side alone is that weak there (full dense-plus-keyword 0.297). The union is not monotonic in the embedded share (mean nDCG 0.4164 at 50%, 0.4137 at 75%; Cranfield 0.3307 then 0.3328, SciFact 0.6374 then 0.6262): the reciprocal-rank merge of two differently scaled lists is crude, and this is the simulation's noise, not a property of the policy.
- Full-index control: the 100% step reproduces the documented default (Cranfield 0.3988 / 0.8020, SciFact 0.7518 / 0.9700, MS MARCO 0.6670 / 1.0000, TREC-COVID 0.4121 / 0.5520). NFCorpus gives 0.3836 where the 2026-11-08 table has 0.3844; the cause of the 0.0008 was not investigated.
- Not measured: a priority order correlated with relevance (needs a corpus with folders and extension classes and labelled queries; the cross-folder set of the audit's plan), query latency during enrichment, Windows.
- Ranking defaults and stored data are unchanged; nothing here changes what the full index returns. Policy and counters: `search-core/src/embed_policy.rs`, `native_index::build_or_update_corpus_index_lazy_send`, `enrich_semantic_index_send`.

## Semantic index at scale (2026-11-11, P2-1)

Probe: `search-core/examples/semantic_scale.rs` (`cargo run -p search-core --release --features semantic-search --example semantic_scale -- <out-dir> <folder>...`). One real semantic index built through the product path (`index_document`, MiniLM, hybrid query) from the distinct `title + text` of Cranfield, SciFact, NFCorpus, MS MARCO and TREC-COVID (17,161 documents, 35,318 chunks, about 1 chunk per document), on the loaded 16 GB Mac; queries are the first 20 of each set (100). Measured at growing sizes; the query time includes the query embedding (about 8 to 20 ms).

| chunks | embed+store, chunks/s (window) | db MB | RSS MB | first query after a write (snapshot build) | steady query |
|---|---|---|---|---|---|
| 2,000 | 16.5 | 8 | 298 | 232 ms | 10.4 ms |
| 5,000 | 17.4 | 19 | 367 | 450 ms | 10.7 ms |
| 10,001 | 17.5 | 38 | 304 | 946 ms | 13.1 ms |
| 20,001 | 17.9 | 76 | 327 | 1,552 ms | 13.3 ms |
| 35,000 | 25.2 | 127 | 272 | 3,497 ms | 23.5 ms |

Whole run: 1,746 s for 35,318 chunks, 20.2 chunks/s (the rate varied with other load on the machine; the last window was faster). Reading:
- Embedding is the cost that scales: at 17 to 25 chunks/s, 100,000 chunks take 1.1 to 1.6 hours and the 250,000-chunk snapshot limit 2.9 to 4.1 hours. This is the case for P2-3 lazy embedding (exact search is complete long before).
- The database is about 3.6 KB per chunk (vector, text, keyword data); 250,000 chunks would be about 0.9 GB.
- The first query after any write rebuilds the in-memory snapshot, about 0.07 to 0.1 ms per chunk and growing a little faster than linear here; extrapolated (not measured) about 20 s at 250,000 chunks. Every watcher update or enrichment pass that writes invalidates it, so lazy passes pay it once per pass. Steady queries stay near the embedding cost; the ANN probe measured the dense scan itself at 36 to 41 ms for 250,000 chunks.
- RSS stayed under 370 MB through 35,000 chunks (the process also holds the model); snapshot memory is about 1.5 KB of vector per chunk plus postings, so 250,000 chunks is about 400 MB plus postings, not measured here.
- Not measured: anything above 35,318 chunks (only that much distinct real text is on this machine), the streaming path above the 250,000 limit (the limit is private to `SemanticIndex`), startup time, Windows.


## 2026-11-11: context-aware ranking prior (P1-3): not adopted

Question (#240 P1-3): does a small prior built from query-time context (the folder of each file and where the best files sit) help ranking? Rule as before: adopted only if it loses on none of the five sets. **Not adopted; `default` unchanged, no stored data changed.**

Prior (`--context-prior`, `ranking_experiments::run_context`, offline on the chunk scores of `rank_features`): every chunk of a file gets `beta * m(folder)` added to its min-max fused score, where `m` is the share of the top-k files of the default ranking (rank weight 1/(rank+1), normalised) that live in that folder. The boost is capped at `beta` (0.02, 0.05, 0.10; k 3, and k 10 at 0.05). Recency was not built: every eval file is written within seconds of the others, so any signal would be corpus order, not information.

**What the public sets can and cannot show.** They have no folders: every document is `.eval-docs/<id>.txt`. With real folders (`flat`) the boost is the same constant for every file, so the ranking cannot change; the table confirms it (0 queries changed, every number identical to `default`). "No loss on all five sets" is therefore true by construction and says nothing about benefit. The benefit case (relevant files cluster in one folder) needs the cross-folder set of the audit (section 4), which does not exist yet. As a control for the downside, `rand20` assigns each file to one of 20 folders by hash of its id: folders carry no information, so any change is pure cost.

nDCG@10 change against `default` (default: Cranfield 0.3936, SciFact 0.7315, NFCorpus 0.3456, MS MARCO 0.6632, TREC-COVID 0.4121; the five sets are the full query sets, 225 / 300 / 323 / 484 / 50):

| Variant | Cranfield | SciFact | NFCorpus | MS MARCO | TREC-COVID | Sets lost |
|---|---|---|---|---|---|---|
| flat, any beta or k | 0 | 0 | 0 | 0 | 0 | 0 (identity) |
| rand20 k3 beta 0.02 | -0.0009 | -0.0016 | +0.0007 | -0.0000 | -0.0023 | 4 |
| rand20 k3 beta 0.05 | -0.0010 | -0.0024 | +0.0007 | +0.0003 | -0.0040 | 3 |
| rand20 k3 beta 0.10 | -0.0028 | -0.0027 | -0.0006 | +0.0008 | -0.0076 | 4 |
| rand20 k10 beta 0.05 | -0.0005 | -0.0012 | +0.0004 | +0.0001 | -0.0041 | 3 |

Recall@100 moves by at most 0.004 (SciFact, Cranfield, NFCorpus) and 0.0015 (TREC-COVID). So a prior that fires on structure unrelated to relevance costs 0.001 to 0.008 nDCG@10 and loses three or four sets at every strength; it fails the rule even at beta 0.02. Whether real folders are informative enough to turn that into a gain is unmeasured. My analysis does not support trying a stronger or different prior on these sets: they cannot answer it. Not tried: neighbour context from content similarity, because pseudo-relevance feedback from the hits (`prf`) and graph expansion from shared entities (P3-1) are already measured and negative; a same-folder prior that is useful would have to beat that on the cross-folder set.

Left in the code (does nothing unless `--context-prior` is passed): `run_context` in `examples/support/ranking_experiments.rs` and the flag in `retrieval_eval.rs`. Reproduce: `retrieval_eval <set> --context-prior` (about two minutes per set, this Mac, release).

**Data hygiene finding (fixed in the benchmark data, outside the repo).** The first run printed default nDCG@10 above 1 (SciFact 1.15). Cause: every set's `.eval-index` held each document twice, once under its absolute path and once under a relative path (`cranfield/.eval-docs/1.txt`), left by an earlier run started with a relative folder argument. `doc_id` is the file stem, so rankings listed relevant files twice. I deleted the relative-path rows from the five `.eval-index` databases (rows with a path not starting with `/`; chunks cascade); defaults now reproduce the documented numbers to four decimals. Any experiment run on those indexes between that stray run and today would be inflated; always pass an absolute folder to `retrieval_eval`.
