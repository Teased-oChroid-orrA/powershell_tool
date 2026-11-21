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
- Phrase across a wrapped line break: not built at the time of this entry; built 2026-11-14 as the opt-in `match_across_lines` setting (reported on the phrase's first line, so hit line numbers stay real lines; see `search-core/AGENTS.md`).
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

## 2026-11-12: PDF extraction and OCR on the KG-RAG corpus

Corpus: `docugami/KG-RAG-datasets` (MIT), 80 PDFs (20 each: NIH protocols, NTSB reports, SEC 10-Qs, US agency reports), 68 MB. Oracle: poppler `pdftotext -layout` (reference only, never shipped). Scored on the 79 files where poppler finds text. Word recall = multiset of lower-cased alphanumeric tokens found / oracle tokens. Phrase = 40 random 5-word runs per file inside one oracle line, found inside one output line (layout-dependent, so only a rough guide). Time and peak RSS per file via `/usr/bin/time -l`, release builds, one backend at a time (docling was partly contended).

| Backend | Word recall | Phrase on one line | Empty files | Total time | Peak RSS |
|---|---|---|---|---|---|
| ours before fixes | 0.400 | 0.21 | 34 | 3.6 s | 31 MB |
| ours after fixes (fe5f55b) | 0.866 | 0.51 | 1 | 9.5 s | 32 MB |
| `yolowpdf` 0.1.2 | 0.976 | 0.91 | 0 | 10.4 s | 86 MB |
| `pdfium` (via `pdfium-render`) | 0.9975 | 0.94 | 0 | 11.7 s | 35 MB |
| `lopdf` 0.47 `extract_text` | 0.802 | 0.46 | 3 | 9.6 s | 250 MB |
| docling (Python, layout model, no OCR) | 0.974 | 0.88 | 0 | 3,895 s (median 42 s/file) | 2.2 GB |

Causes found in our extractor: 31 files carry `/Encrypt` with an empty user password (all RC4-128) and extracted to nothing (now decrypted with `lopdf`); the 400-character stream-header window crossed into the previous object, so `/ImageB` in a page `/ProcSet` made the `/Image` skip marker drop Word content streams (header now starts at its own `obj`). Still weak after both: 8 files below 0.5 recall, mostly CID TrueType (Identity-H) in NIH and Intel filings; cause not established.

OCR: 8 digital pages sets (2-3 pages each, 200 dpi) turned into image-only JPEG PDFs, clean and noisy (blur, rotation, speckle); ground truth is the pages' own text layer. Recall / precision, clean | noisy: ours `ocrs` 0.12.2 0.965/0.970 | 0.953/0.888, 72 s | 89 s, 1 GB; `ocrs` 0.14.0 (same models) 0.966/0.966 | 0.957/0.805, 89 s | 116 s, 1 GB; `yolowpdf-ocr` small PP-OCRv6 0.974/0.987 | 0.917/0.956, 121 s | 111 s, 0.4 GB; tesseract 0.986/0.981 | 0.973/0.959, 37 s | 35 s, 76 MB. The corpus itself has one scan (CCITT, not decoded by our OCR path).

Decisions: keep `ocrs` 0.12.2 (0.14 adds nothing); do not adopt `yolowpdf-ocr` (no gain on noisy scans, slower); reject docling (Python and torch, 2 GB, 400x slower) and tesseract (C++ runtime) for shipping; `yolowpdf` is the candidate if text-layer recall must reach 0.97 (pure Rust, MIT, 0.1.2 is days old, self-reported benchmarks only) and needs a flagged trial with the current path as fallback. Not measured: Windows, real scans, CCITT/JPEG2000 decoding, page numbers on decrypted files.

### Flagged `yolowpdf` trial (same day)

Built as the `pdf-yolow` cargo feature of `search-core` (off by default there; `app-tui` and `search-cli` enable it, so the shipped app contains it) with a per-run opt-in, `SearchSettings::pdf_yolow`, shown in the Search workbench as "Better PDF text (trial)" (Search ribbon Options group and the settings list). It is in the cache fingerprint and the failure-retry context, so toggling re-reads PDFs. `search-core/src/pdf_yolow.rs`, hooked at the top of `extract_pdf_lines_capture`: `yolowpdf` runs first, under `catch_unwind`, and its Markdown is cleaned to plain lines. The byte scanner (with OCR) takes over when the backend is off, errors, panics, returns no text, or calls the document scanned. It is skipped when page provenance is requested (semantic indexing), so page numbers and the semantic index are unchanged. Measured through `extract_pdf_lines` (release, same 79 files and scorer as above):

| Path | Word recall | Min file recall | Phrase on one line | Empty | Total time | Peak RSS |
|---|---|---|---|---|---|---|
| flag off (scanner) | 0.866 | 0.000 | 0.51 | 1 | 4.7 s | 31 MB |
| flag on (`yolowpdf`, scanner fallback) | 0.972 | 0.855 | 0.91 | 0 | 9.6 s | 86 MB |

The flag removes the CID-font failures (worst file 0.855 against 0), at roughly twice the time and 2.8x the memory. Off by default in the app: 0.1.2 is days old with self-reported benchmarks; no Windows run; scanned and mixed documents (some pages scanned) still go to the scanner only when the whole document is called scanned, so scanned pages inside a mostly-text document get no OCR on the flagged path. Page numbers on the flagged path: see 2026-11-14 below. Next measurement before any default switch: Windows hosted run, and the mixed-document case.

## 2026-11-14: page citations, scanner vs `yolowpdf` (KG-RAG, 80 text-layer PDFs)

Harness: `search-core/examples/pdf_citation_bench.rs` (`--features semantic-search,pdf-yolow`; needs `pdftotext`). Oracle: poppler per-page text. A line is checkable when its first five words occur on some oracle page; it is correct when the page the extractor cites holds them. Per-file means (the corpus total is dominated by a few huge files):

| Backend | Files with checkable lines | Lines carrying a page | Correct / checkable | Files with no page at all |
|---|---|---|---|---|
| scanner (`extract_for_semantic_index`) | 70 | 0.281 | 0.543 | 32 |
| `yolowpdf` (its own page split) | 79 | 0.725 | 0.989 | 0 |

The scanner cites nothing for 32 files (page map not established) and 38 of 70 files score above 0.9. `yolowpdf` scores above 0.9 on 78 of 79 files; the weakest is `EPA2022.pdf` (0.89). Lines without a page under `yolowpdf` are mostly lines under 5 words (not checked) and note rows. Decision: semantic indexing now takes `yolowpdf` lines with their page numbers when "Better PDF text (trial)" is on (scans, errors and empty files still fall to the scanner path); off by default, unchanged. Not measured: Windows, mixed text/scanned files under `yolowpdf`, real scans, retrieval quality of the changed chunk text (gate with `retrieval_eval.rs` before any default switch). Encrypted files go through the same decrypt step first.

## 2026-11-14: cold start of an existing semantic index (P2-1, startup)

Probe: `search-core/examples/semantic_startup.rs` (`cargo run -p search-core --release --features semantic-search --example semantic_startup -- <semantic.sqlite> [query]`), one fresh process per run: time to open the database, to the first query (model load plus building the in-memory snapshot), to a second query, and resident memory after each. macOS, release build, one machine, nothing else heavy running.

| Index | Chunks | DB | Open | First query | Second query | RSS after first query |
|---|---|---|---|---|---|---|
| Cranfield | about 1,400 documents | 21 MB | 234 ms | 243 ms | 3 ms | 214 MB |
| MS MARCO subset | not counted | 27 MB | 266 ms | 306 ms | 5 ms | 223 MB |
| Scale index (the 35,318-chunk run above) | 35,318 | 128 MB | 84 ms (2,058 ms once, cold OS cache) | 2,325 to 2,435 ms (3 runs) | 18 to 19 ms | 173 to 288 MB (3 runs) |

Reading. Opening is cheap. The first semantic query of a session is not: about 0.25 s of it is loading the model (the small indexes), so roughly 2.1 s at 35,318 chunks is building the snapshot, about 60 microseconds per chunk. Every later query is under 20 ms. Extrapolating linearly (not measured; the in-memory limit is 250,000 chunks) the first query could take around 15 s on an index near that limit. Resident memory stays under 300 MB at 35k chunks, with run-to-run spread of 115 MB that was not explained.

Not done, and why. Building the snapshot in the background when the window opens would hide the wait, but it spends memory and CPU for people who never run a semantic query, so it needs a decision on when to pay (on open, on first typing in Find, or never). Not measured: above 35,318 chunks (no more distinct text on this machine; embedding runs at about 20 chunks per second, so 250,000 chunks is about 3.5 hours), the streaming path above the limit, Windows, spinning disks.

### Prewarm on first Find typing (decision taken, measured)

Decision (owner, 2026-11-14): build the snapshot when the user first types in Find, not on open. `SemanticIndex::warm` / `warm_directory` (`search-core/src/semantic_index.rs`) build the in-memory snapshot on the search worker; `warm_semantic` (`app-tui/src/gpu_viewer/editor/search/mod.rs`) triggers it once per built semantic index from `document_field` for the Find input. The probe takes `--warm` to time it.

Measured on the 35,318-chunk scale index (128 MB), macOS release build, one machine: the warm takes about 2.4 s on the worker, and the first query afterwards takes 22 to 25 ms instead of about 2.3 s. Tests: `warm_builds_the_snapshot_once_and_a_write_makes_it_stale`, `typing_in_find_warms_each_built_semantic_index_once`.

Not checked: the trigger in a real window, Windows, indexes above 35k chunks.

## 2026-11-16: warm re-search freshness check reads the index once

Review of the exact path with `examples/exact_bench.rs` (10,000 files x 150 lines, this Mac, release, three runs each). `native_index::narrow_candidates` read every stored document twice per warm re-search: `all_document_metadata` for the (modified, size) map and `all_document_ids` for the removed-file count. The ids are that map's keys, so the second pass is gone.

| | walk | freshness | query |
|---|---|---|---|
| before | 49 to 71 ms | 61, 61, 61 ms | 0 ms |
| after | 37 to 51 ms | 32, 33, 33 ms | 0 ms |

Results are unchanged (same candidates, 104; `native_index` tests pass). One behaviour difference: a failed read of the ids used to leave the removed count at 0 silently; the single read now fails the narrowing, which already falls back to a full scan and reports it.

Also fixed in the harness: the warm cache row printed `cache_reused: 0`, because the cache does not trust files modified within 2 s of its save and the corpus had just been generated; the harness now waits 2.1 s first (10,000 of 10,000 reused, 87 to 180 ms).

Measured, not changed:
- A warm narrowed re-search is now walk plus freshness (about 85 ms of 85 ms at 10,000 files); the trigram query is under 1 ms. The remaining cost scales with the files in scope, not with the hits.
- Literal search with three filters hitting 63 % of lines: 129 ms against 10 ms for one rare filter (30,000 lines x 5), the regression already recorded on 2026-11-10.

### Index build: intermediate commits are time-gated (same day)

`sample` on the build of 10,000 small files showed one thread almost always in `fcntl` (fsync) and little CPU in the trigram postings: the build committed every 200 documents (50 commits), and a commit costs a handful of fsyncs whatever it holds. A count-triggered commit now also waits 2 s since the previous one (`COMMIT_MIN_INTERVAL`); the 32 MB staged-text trigger is unchanged, so replay memory and the text lost by an interrupted build stay bounded, and slow documents (PDF, OCR) still commit every 200.

| files x 150 lines | before | after |
|---|---|---|
| 3,000 | 5.1, 6.2 s | 1.6 s |
| 10,000 | 18.0, 18.7 s | 4.9, 6.6 s (4.8 s with a fixed batch of 2,000) |

Cost: an interrupted (killed, not cancelled) build of small files can lose up to 2 s of indexing instead of 200 files; a cancel still commits what was staged. `cargo test --release -p search-core` (268) and `-p native-search` (32) pass. Not measured: Windows (fsync cost differs), PDF-heavy folders (extraction dominates there, so little change is expected).

## 2026-11-16: literal search with filters that match most lines (the regression recorded on 2026-11-10)

Question: the 2026-11-10 section recorded, as a hypothesis, that the ASCII prefilter is pure overhead when most lines match (`exact_bench`, row "match literal + 3 filters", 63 % of lines hit). Checked by timing each part separately before changing anything.

Method and its limit. macOS, release build, `exact_bench` in-memory rows (200 files x 150 lines, 5 passes; these rows do not depend on the file-count argument). The machine was busy with other builds for the whole session (load average 10 to 30), so single runs vary by a factor of two or more. Numbers below are therefore the lowest and the median of 30 runs, with the old and the new binary run alternately; treat ratios as the result, not the absolute values. Note that only 2,096 of the 30,000 benchmark lines are pure ASCII (the vocabulary has accented words), so this corpus is close to the worst case for anything that depends on a line being ASCII.

Where the time went (per 150,000 lines, parts timed alone, lowest of 5):

| Part | 1 rare filter | 3 filters, 63 % of lines hit |
|---|---|---|
| Prefilter regex on every line | 15 ms | 53 to 64 ms |
| Prefilter, then Unicode `to_lowercase` and substring checks on passing lines (the old path) | 17 to 24 ms | 241 to 268 ms |
| Unicode `to_lowercase` of every line, alone | 200 to 225 ms | same |
| Copy plus ASCII lowercase into a reused buffer, every line | 5.5 to 7 ms | same |
| ASCII-lowercase buffer plus substring checks, no prefilter | 28 to 31 ms | 60 ms |
| Prefilter, then ASCII-lowercase buffer plus substring checks | about 18 to 22 ms | about 110 ms |
| Building the hits (three line copies and the filter names per hit) | 0.3 ms | 139 to 143 ms |

Finding: the hypothesis was wrong as stated. The prefilter was about a fifth of the matching cost; removing it alone would have made the old path slower (every line would then be lowercased). The largest cost was Unicode `to_lowercase`, which allocates and converts character by character on every line that passes, roughly 35 times the cost of an ASCII lowercase into a reused buffer. The prefilter does become a net loss once lowercasing is cheap and most lines pass (110 against 60 ms above), and it stays a win for a rare filter (18 to 22 against 28 to 31 ms).

Change (`search-core/src/matching.rs`), literal mode with all-ASCII filters, no accent folding, no typo tolerance, no NEAR:
1. A line holding neither U+0130 nor U+212A is compared through an ASCII-lowercased copy in one reused buffer. Exact by the existing code-point proof (`lowercase_to_ascii_sources_are_only_kelvin_and_dotted_i`): no other non-ASCII character lowercases to ASCII, so an ASCII filter can only match bytes that ASCII lowercasing produces. Lines with either character take the old full-lowercase path.
2. The prefilter is adaptive within a run: every 32 lines, it is switched off if more than half of the last 32 lines had a hit and on again otherwise; the state at the end of a file is the starting state of the next. The half-way threshold is not from a sweep: one prefilter call costs at least half of the exact check it can save (table above: 15 against 28 to 31 ms for one filter, 53 to 64 against 60 ms for three), so it cannot pay once more than half the lines pass.

Result (`exact_bench`, ms for 30,000 lines x 5; lowest / median of 30 alternated runs; two separate sets because the load changed between them):

| Row | Before, set 1 | After, set 1 | Before, set 2 | After, set 2 |
|---|---|---|---|---|
| match literal | 13.1 / 16.9 | 13.4 / 18.6 | 19.5 / 22.6 | 21.9 / 23.2 |
| match whole-word | 16.1 / 19.8 | 16.2 / 21.8 | 23.6 / 26.8 | 24.3 / 27.9 |
| match regex | 13.3 / 16.1 | 13.5 / 19.3 | 20.6 / 23.9 | 21.9 / 23.9 |
| match literal + 3 filters | 169.2 / 206.9 | 103.9 / 136.3 | 297.6 / 409.6 | 161.2 / 227.7 |

Three plain runs of `exact_bench 3000` each, same load, for the record (before: 365.6, 507.3, 585.2 ms; after: 265.9, 300.3, 218.5 ms on the 3-filter row; the rare rows ranged from 24 to 201 ms in both binaries, which is noise). The 3-filter row is about 1.6 to 1.8 times faster; hit counts are unchanged (15, 15, 15 and 94,770). The rare-filter rows are within the run-to-run spread: lowest values differ by 0.1 to 0.3 ms in set 1 and by 0.7 to 2.4 ms in set 2, medians by 0 to 3 ms, and the whole-word and regex rows execute no new code per line. Contribution of each part on the 3-filter row, measured with a build that kept the prefilter always on: old 420, buffer only 237 to 267, buffer plus adaptive 184 (lowest of 12 to 40 runs under heavier load).

Negative result kept: the first version counted lines and hits per line for the window. That made all three rare-filter rows about 6 % slower (for example 21.1 to 22.6 ms, repeatable in both run orders, also on the regex row), and a build without the counters matched the old binary. The decision now reads the length of the hit list at each window boundary, which removed the difference. Forcing or forbidding inlining of the line loop did not change it.

Not done, and why. Building the hits is now the largest remaining part of the 3-filter row (about 140 of the remaining time): each hit copies the line, its two neighbours and the filter names, which is the shape of `LineHit` and is read by the report, the exports and `app-tui`. Changing it is an API change across crates, not a matcher fix. `memchr`'s substring search in place of `str::contains` measured 60 to 41 ms on the no-prefilter part; not adopted, because it adds a direct dependency for under 10 % of the row. Exclusion filters and whole-word mode still lowercase the old way (whole-word only lowercases pure-ASCII lines, which is already cheap). Not measured: Windows, a quiet machine, real documents (where most lines are ASCII and the buffer path's extra check for U+212A is skipped).

Tests: `prefilter_policies_return_identical_hits` (prefilter always on, always off and adaptive against the reference definition `lowercase(line) contains lowercase(filter)`, hit lines and matched filters, on runs of matching and non-matching lines longer than the window with U+0130 and U+212A lines in both, with and without exclusions; it fails if the U+212A guard is removed), `adaptive_prefilter_turns_off_on_dense_lines_and_back_on_when_they_thin_out`. `cargo test --release -p search-core`: 270 passed, 4 ignored.

## 2026-11-17: index freshness reads the document store in order; watcher shortcut not taken

Follow-up to the 2026-11-16 review (warm re-search is walk plus freshness; the trigram query is 1 to 9 ms).

**Not done: skipping the walk through the folder watcher.** `watch.rs` is best effort by its own contract (network shares and some synced folders report nothing, dropped events set `overflow`), and narrowing must never hide a real match. A warm search that trusted "no events" instead of walking could miss an edited file on exactly those folders. The walk itself is at the cost of `read_dir` plus `metadata` (`walk_bench`, `knowledge-engine-audit.md`). Dropped, not built.

**Done: `NativeSearchEngine::all_document_metadata` and `all_document_ids` read each segment's store in document order.** They collected every document address into a set and fetched each by address; the store is block-compressed, so in set order almost every fetch decompressed a block again once the index outgrew the reader's block cache. `for_each_stored_document` iterates `StoreReader::iter` per segment with the segment's alive bitset (deleted documents are skipped as before).

`exact_bench`, release, this Mac, other builds running at the time (so compare within a pair, runs alternated):

| files | freshness before | freshness after |
|---|---|---|
| 10,000 (three alternated pairs) | 117, 74, 71 ms | 18, 37, 40 ms |
| 100,000 (one run each) | 490 ms | 184 ms |

Candidates are identical (104 and 1,031). `cargo test --release -p native-search` (32) and `-p search-core` pass. The 100,000-file walk in the same runs was 898 and 383 ms: directory-cache state, not this change. Not measured on Windows; `.github/workflows/search-bench-windows.yml` now runs `exact_bench` at 3,000 and 10,000 files on hosted Windows for every search PR and prints the rows in the job summary (evidence for that host, not a gate).

Still open from the review list: narrowing with Ignore accents or Typo tolerance on (needs a folded trigram field and an index rebuild), and same-size same-mtime edits that the freshness check cannot see.

## 2026-11-16: incremental snapshot and embedding throughput (semantic index speed)

### Snapshot follows writes instead of being rebuilt

Before, the first query after any write rebuilt the whole in-memory snapshot (decode every vector, tokenize every chunk). Now every `semantic_documents` row carries the write generation it was written at, and `Snapshot::updated` (`search-core/src/semantic_cache.rs`) re-reads only the documents newer than the snapshot, drops the ones no longer stored, renumbers the rest in one pass over memory, and returns exactly what a full build returns (same chunk order, vectors, lengths and postings, so scores are the same by construction).

Probe: `search-core/examples/semantic_snapshot_update.rs` (`cargo run -p search-core --release --features semantic-search --example semantic_snapshot_update -- <semantic.sqlite>...`). It copies the index, then times the first hybrid query after adding, editing and removing one single-chunk document (median of 3 each) and the steady query. Query times include the query embedding. Indexes: SciFact's evaluation index (12,601 chunks); SciFact + NFCorpus + MS MARCO + Cranfield merged (28,765); that merged index four times under different paths (115,060; real text and vectors, repeated). The indexed files do not exist on this machine, so the per-query freshness check fails fast for every document; it costs the same before and after. macOS, release build, load average 3 to 7 from other work.

| chunks | full build | first query after add / edit / remove, before | after | steady query |
|---|---|---|---|---|
| 12,601 | 0.82 s | 831 / 835 / 834 ms | 13 / 16 / 15 ms | 8 ms |
| 28,765 | 1.78 s | 1,821 / 1,817 / 1,826 ms | 30 / 35 / 34 ms | 21 to 28 ms |
| 115,060 | 6.77 s | 6,783 / 6,674 / 6,746 ms | 117 / 123 / 115 ms | 53 to 63 ms |

The update itself (first query minus steady) is about 6 ms at 12.6k chunks, 10 ms at 29k and 60 ms at 115k: the copy of the vector block and the renumbering of the postings, both linear in the index but at memory speed. A run on the same machine under heavy load (load average 17 to 25) gave 3.5 s / 5.1 s / 11.6 s before and 22 / 58 / 268 ms after.

Tests: `incremental_snapshot_equals_a_rebuilt_one_through_adds_edits_and_deletes` (after each of 13 writes the served snapshot equals a fresh `Snapshot::build` field for field, no full build happened, and hits and scores equal the streaming path), `snapshot_scores_equal_streaming_scores_and_follow_writes` (unchanged), `a_recreated_database_never_updates_the_old_snapshot`.

Two defects found and fixed on the way, each with a test that fails without the fix:
- A file whose time changed but whose text did not was re-stamped without bumping the write counter. The cached snapshot kept the old time, so the per-query freshness check dropped that file's hits until some other write. The re-stamp now counts as a write (`a_touched_file_with_unchanged_text_stays_in_cached_results`).
- Snapshots were matched to an index file by path and write counter only. A database deleted and recreated at the same path could be served the old snapshot when the counters met. Each database now has a random `semantic_meta.instance`, and a snapshot is only reused or updated for the same instance.

Not done: the update still copies the vector block (about 1.5 KB per chunk), so its cost grows with the index (extrapolated, not measured: about 150 ms at 250,000 chunks). Not measured: above 115,060 chunks, a write of many documents between two queries (cost is the changed documents at full price plus the pass above), Windows.

### Embedding throughput

Profile (`sample <pid> 8` on `search-core/examples/embed_throughput.rs`, which runs `index_document` on fixed text): 97.5% of the main thread's time is inside the model call (`rten::Model::run`), tokenization about 1% (counting tokens for the chunk budget plus encoding), SQLite 0.5%, hashing and the rest below that. The model's pool has 4 threads (the performance cores); one of them runs the graph and most of the matrix products, the other three spend about 45% of their samples yielding and 15% in the product kernel.

Model alone on the 235 chunks of 120 documents (SciFact, MS MARCO, Cranfield, NFCorpus, round robin), 3 interleaved rounds, median chunks/s; every vector compared bit for bit with the stored one:

| texts per call | threads calling | full chunks (up to 800 chars) | cut to 400 chars | 200 chars | 80 chars |
|---|---|---|---|---|---|
| 1 | 1 | 28.2 | 44.2 | 70.1 | 106.0 |
| 2 | 1 | 24.0 | 41.9 | 73.5 | 141.1 |
| 4 | 1 | 22.2 | 41.1 | 77.0 | 156.4 |
| 16 | 1 | 19.4 | 37.6 | 72.6 | 157.5 |
| 1 | 2 | 29.5 | 45.2 | 77.1 | 127.1 |
| 1 | 4 | 29.8 | 47.8 | 79.7 | 142.0 |
| 4 | 4 | 22.5 | 41.3 | 78.6 | 165.5 |

- Vectors are bit-identical for every batch size and thread count (0 of 235 differ, also 0 of 387 and 0 of 643 in two earlier runs). Batch composition is free to change.
- The index embedded 16 chunks per call. The model pads a call to its longest text, and for full chunks one per call is 45% faster than 16 per call (28.2 against 19.4). Short texts are the other way round (80 characters: 157.5 against 106.0).
- Adopted: `semantic_index::embed_calls` groups the chunks of a document into calls of about one full chunk of padded text (`texts x longest <= 800` characters, at most 16): full chunks go alone, tails and page fragments together.
- Not adopted, measured: calling the model from 2 or 4 threads at once gains 5% on full chunks (28.2 to 29.8). The calls share the same 4-thread pool, which is already the limit. No change made.

Product path (`index_document`, chunk + embed + store) before and after on the same text, 2 runs each, same vector checksum before and after:

| text | chunks | before | after | checksum |
|---|---|---|---|---|
| 240 documents, one per file | 474 | 24.0, 24.1 chunks/s | 28.1, 27.8 | `8bd840a085d96ef6` both |
| the same 240, 24 per file (long documents) | 356 | 18.8, 18.8 chunks/s | 21.6, 21.5 | `15e9f9dd01ddb42b` both |

That is 15% to 17% faster with identical vectors, so no `retrieval_eval` run and no `EMBEDDING_MODEL_VERSION` bump. Tests: `model_calls_hold_one_full_chunk_or_several_short_ones`, `batching_does_not_change_a_vector`.

What remains: inference itself is the cost, and nothing else in the loop is worth changing (database writes are 0.5%). Not tried: a different inference runtime or a newer `rten` (would need the bit comparison above, and the five-set run if any vector changes), the other callers of `search_ml::embed_batch` (`alerts.rs`, `neural_search.rs`, `relevance.rs` send lines; the 80 and 200 character columns say their batching is already the right way round), Windows.

## 2026-11-17: first hosted Windows rows (`search-bench-windows.yml`, PR #305)

Host: AMD EPYC 9V74 (hosted runner, `windows-latest`), release, one run. Evidence for that host only.

| row | 3,000 files | 10,000 files |
|---|---|---|
| match literal (30,000 lines x5) | 14.0 ms | 11.1 ms |
| match literal + 3 filters | 47.0 ms | 42.1 ms |
| cold full scan, sequential | 659 ms | 1,671 ms |
| cold full scan, parallel (+cache write) | 196 ms | 609 ms |
| warm re-run, cache hit | 89 ms | 258 ms |
| index build | 2.0 s | 6.5 s |
| index narrow (walk / freshness / query) | 67 ms (60 / 5 / 0) | 219 ms (194 / 20 / 0) |
| run_candidates (verify) | 4.3 ms | 10.7 ms |

Reading: on Windows the warm re-search is the directory walk (194 of 219 ms at 10,000 files, about 19 microseconds a file, roughly four times this Mac); the freshness check after the in-order store read is 20 ms. The walk, not the index, is what is left to gain on the shipped target.

## 2026-11-17: index narrowing with Ignore accents (item 4); same-size edits (item 9) not adopted

### Item 4: a folded trigram field

Before: turning on Ignore accents or Typo tolerance made `narrow_candidates` return no candidates (the index stores unfolded text), so a warm re-search scanned every file. No set of raw trigrams of the folded filter is safe (`e` may come from `e`, `é`, `è`, a combining mark or a ligature), so the only sound narrowing is a second trigram field built from the folded text.

Change: `trigram_folded` (schema change, `INDEX_SEMANTIC_VERSION` 5 to 6, so existing indexes rebuild once). It is written only for a document whose folded text differs from its lowercased text (never for ASCII), through one helper (`native_index::folded_trigram_text`) used by both file-index writers. With Ignore accents on, the candidates are the files whose own trigrams hold the folded filter's trigrams OR whose folded trigrams do. Typo tolerance still scans (a word one edit from the filter bounds no trigram set).

Proof: `folded_narrowing_keeps_every_file_the_folding_matcher_finds` runs 16 filters (plain, accented, upper case, ligature, curly quote, no-break space, `ss`), whole word and not, over nine files (precomposed and combining accents, ligatures, typographic characters, `ß`, none) and requires every file the real matcher finds to be a candidate, and the exact shortlist for `cafe`. With the folded text switched off the test fails (`pre.txt` not a candidate), so it detects the failure it guards.

Measured (`exact_bench` 10,000 files, this Mac, other builds running): a search ignoring accents narrows to 104 candidates in 55 ms against a full scan of 1.0 to 1.2 s (about 20 times; the 10,000 files were on disk, hot). Cost: this bench corpus is a worst case (every document holds a character that folds), and the index build took 1.75 to 1.85 times as long (5.4 and 9.6 s before, 9.3 and 17.8 s after, alternated, loaded machine). Share of real documents with a foldable character (approximate fold; English, first 20,000): Cranfield 0 %, SciFact 9.8 %, NFCorpus 14.3 %, MS MARCO 15.0 %, TREC-COVID 19.9 %, AeroEngQA 37.8 %, a Spanish set 99.3 %, an industrial PDF set 0.4 %. For typical English engineering text the extra index work is the folded share of the trigram field (10 to 20 %); a Spanish or German corpus approaches the bench's worst case. Index size was not measured. Not measured on Windows.

### Item 9: same-size, same-mtime edits: built as an on-request content check

Freshness is `(modified, size)` per document. Two ways an edit can hide: (1) the file system's timestamp tick is coarse (FAT/exFAT 2 s, some network shares 1 to 2 s) and the edit happens inside the tick of the last index, (2) a tool preserves the old mtime (copy with preserved times, archive extraction, `touch -r`) and the size is the same. No timestamp rule protects the second case; only reading the content does, which is the cost the index exists to avoid. Options compared (mockup, owner choice 2026-11-18): A keep as is, B racy window (per-document index time), C verify on demand, D hash on every search. **C was built.**

Built: every indexed document stores the SHA-256 of the bytes it was indexed from (`content_hash`, `INDEX_SEMANTIC_VERSION` 7, so existing indexes rebuild once). The hash is taken over bytes the builder already reads, so it adds no file read. `native_index::verify_content` (button `Verify content` in the Search window's Index card) reads every indexed file once, compares, drops the differing ones from the index and the semantic index, and the build that follows re-indexes them; the status line reports `content check: N changed, M without a new time or size`. Not run automatically and not run on search, so searches cost the same as before.

Proof: `native_index::tests::verify_content_finds_a_same_size_same_time_edit_the_freshness_check_misses` rewrites a file with the same size and restored mtime: the normal update indexes 0 files and the old text stays searchable (the gap); `verify_content` reports 1 changed, 1 hidden; the next update indexes 1 and the new text is found. `verify_content_finds_an_edit_that_build_update_cannot_see` does the same through the Search window action.

Measured cost of the check (5,839 files, 828 MB, repo folder, warm cache, Python script, not the Rust code): time and size 16 ms, sampled hash (three 4 KiB reads) 221 ms, full SHA-256 901 ms, so about 154 us per 138 KiB file; a cold disk is bound by reading every byte and was not measured, nor was Windows, nor the added index-build time of hashing in Rust. B (racy window) was not built: it protects only coarse-timestamp file systems for 2 s after indexing and needs a per-document index time.

## 2026-11-17: OCR confidence from a vendored `ocrs`

`ocrs` 0.12.2 discards the recognition model's character probabilities, so the PDF warning could only say "may be misread". `third_party/ocrs` is the 0.12.2 source with one change: `TextChar::confidence`, the model's probability for each recognised character (`step_confidences` in `recognition.rs`: the exponential of the best log-probability of the emitted label over the frames from that step to the next; unit test inside the crate). `search-core::ocr` carries it per line and per page, the OCR cache is version 3, `FileSearchResult.ocr_confidence` is the document mean, and the Search window and HTML report show it. The text that OCR returns is unchanged.

**Method.** `scripts/make_ocr_confidence_set.py` renders 80 pages (5 documents of prose, 16 degradations: clean, 100 and 300 dpi, blur, low contrast, rotation, JPEG quality 20, shading, noise, speckle and combinations) with known text. `search-core/examples/ocr_confidence_eval.rs` runs OCR, aligns recognised lines with the truth lines, and compares mean probability with character error rate (CER). Release build, Apple silicon, 616 s: 3,353 truth lines, 5,838 recognised lines (the extra ones are specks the detector read as text).

**Lines** (3,290 matched). Spearman of (1 - mean p) with CER 0.816. AUC of mean p for lines with CER above 2%, 10% and 25%: 0.857, 0.964, 0.987. The minimum probability in a line is worse (0.909 for CER above 10%); geometric mean and the share of characters under 0.9 are about equal to the mean. A line threshold of 0.90 flags 18.5% of lines at 99.3% precision and 56.6% recall for CER above 10%; 0.95 flags 31% at 89% and 85%.

**Pages** (error = (edits + missed + spurious characters) / truth characters; bad = error above 5%). Spearman 0.949, AUC 0.988. Threshold on page mean probability:

| Threshold | Pages flagged | Precision | Recall |
|---|---|---|---|
| 0.90 | 22 of 80 | 100% | 67% |
| 0.97 (chosen, `POOR_OCR_CONFIDENCE`) | 29 of 80 | 100% | 88% |
| 0.98 | 39 of 80 | 82% | 97% |
| 0.99 | 67 of 80 | 49% | 100% |

Ordinary-quality conditions (clean, 100 and 300 dpi, blur 1, low contrast, rotation 1 and 2, JPEG 20, shading): page mean 0.972 to 0.994, page error 0.011 to 0.052. Noise-heavy 0.761 to 0.926; speckle 0.76 to 0.85 (error 0.38 to 1.1); combined-heavy 0.38 to 0.40 (error about 0.95).

**Decision.** The warning stays on every OCR PDF: the clean baseline reads about 3% of characters wrong (code-like prose, underscores), so no score makes OCR text exact. The score only changes the label ("OCR 98%: may be misread" or "OCR 62%: poorly read" below 0.97) and the detail text.

**Not adopted: dropping low-confidence lines.** The 2,087 spurious lines have mean line probability 0.408 (98% under 0.9), and 191 of the 3,290 matched lines under 0.6 have mean CER 0.93, so removing lines under about 0.6 would remove junk from the index. It changes indexed text, so by the owner rule it needs `retrieval_eval` on BEIR, MS MARCO, Cranfield and TREC first. Recorded as a finding only.

**Caveats.** The set is synthetic (document prose, macOS fonts). Real scanners, CCITT scans and Windows were not run. The score is the model's certainty, not accuracy, and cannot see text the detector missed (which is why pages with missed lines can still score high). Reproduce: `python3 scripts/make_ocr_confidence_set.py OUT_DIR`, then `cargo run -p search-core --release --features ocr --example ocr_confidence_eval -- OUT_DIR [--lines lines.tsv]`.

Tests: `models::tests::{ocr_percent_floors_and_clamps, ocr_is_poor_switches_at_the_threshold, pdf_warning_*}`, `report::tests::html_report_note_states_the_ocr_confidence_when_known`, `cache::tests::{ocr_confidence_survives_save_and_load, entry_without_the_ocr_confidence_field_still_loads}`, `pdf_yolow::tests::recognition_confidence_is_high_on_a_clean_scan_and_falls_with_noise`, and the app-tui `pdf_warning_shows_the_ocr_percent_in_the_ribbon_and_the_detail_note`.


## 2026-11-17: page citations for xref-stream PDFs on the scanner path (#127, #240)

The scanner's page mapper (`pdf_page_map.rs`) read only classic cross-reference tables, so a PDF written as 1.5 or later (cross-reference streams, compressed object streams) got no page at all (page 5 above: 32 of 80 KG-RAG files). `page_by_stream_offset` now falls back to `lopdf`'s page tree when the strict reader refuses. `lopdf` does not keep where a stream body sat in the file, so the body offset is rebuilt from the xref entry and accepted only when the bytes there equal the loaded stream content.

`pdf_citation_bench scanner` over the 27 readable ViDoRe Industrial source PDFs (poppler per-page text as oracle; a line is checkable when its first five words occur on some page), one machine, release build:

| | lines | lines citing a page | checkable | cited correctly | files with any page |
|---|---:|---:|---:|---:|---:|
| before (classic reader only) | 512,110 | 0 | 62,227 | 0 | 0 of 27 |
| after (lopdf fallback) | 512,110 | 111,229 | 62,227 | 57,965 (93.2%) | 25 of 27 |

Uncited lines are mostly fragments from streams no page draws from (fonts, metadata); of lines poppler can check, 93.2% now carry the right page. Mapping Form XObject streams as well added one line (111,230), so it is not in the code. Two files stay uncited (`TO_35-1-18`, `TO_36-1-191`, the second also yields nothing under yolowpdf). Cost: the fallback loads the file with `lopdf` (21 s for all 27 files together, including the extraction). Tests: `pdf_page_map::tests::{xref_stream_pdfs_are_mapped_through_lopdf, both_readers_agree_on_a_classic_file, a_stream_shared_by_two_pages_is_not_attributed_by_the_fallback}`. #127: not reproduced (see 2026-11-10); the mapper limit it was filed for is now removed.

## 2026-11-17: semantic index above the snapshot limit (streaming path, #240 P2-1)

Probe: `search-core/examples/semantic_stream_probe.rs` (`cargo run -p search-core --release --features semantic-search --example semantic_stream_probe -- <source semantic.sqlite> <out-dir> <copies>`). It copies the rows of a real index (the 15,075-chunk scanner index of the 27 ViDoRe PDFs, 26 documents) `copies` times under new paths, each with a real one-byte file whose size and mtime match its row (otherwise `semantic_path_fresh` drops the row), then opens it in a fresh process and queries. Vectors are copied, so scan cost is real and ranking quality is not measured. Release build, one machine (Apple silicon Mac, SSD), query "fatigue crack growth in aluminium alloy", hybrid, limit 100.

| Chunks | Database | Path | `open` | `warm` | First query | Steady query | Resident memory after queries |
|---:|---:|---|---:|---:|---:|---:|---:|
| 241,200 | 1,048 MB | snapshot (under the limit) | 6.6 s | 18.1 s | 226 ms | 41 to 46 ms | 600 MB |
| 286,425 | 1,248 MB | streaming (over the limit) | 10.3 s | 2.0 s (returns `false`, builds nothing) | 17.7 s | 18.4 to 19.8 s | 30 MB |

Reading. The cliff at `MAX_CACHED_CHUNKS` is about 450 times in steady query time, not the 35 ms exact-scan cost the ANN probe measured: the streaming path decodes every vector and tokenises every chunk on each query. With Search settings (`search_with_settings`, one `stat` per document; 468 documents here) the time is the same, so the per-file `stat` is not the cost at this scale. Memory is the opposite: the streaming path holds 30 MB where the snapshot holds 600 MB. `open` is 6.6 to 10.3 s on an index this size (it runs `PRAGMA quick_check` over the whole file), which a person waits for at the first Search of a session; this was not isolated further. The snapshot build (`warm`) takes 18 s at 241k chunks, close to the linear extrapolation (15 to 20 s) in earlier entries.

Limits. Replicated rows (each chunk repeated 16 to 19 times, so the keyword postings are repetitive and the cost split between vectors and text may differ on real text); one machine; one query; Windows and spinning disks not run; `open` time split between `quick_check` and everything else not measured.

Not changed, and why: an index over 250,000 chunks is hours of embedding at 20 chunks per second (earlier entry), so this is a limit for large corpora, not a common case. Making the streaming path fast (a vector-only pass with the keyword side from a postings table, or a larger or quantised snapshot) changes the exact-equality contract between the two paths and the memory budget on an 8 GB machine (see the int8 entries above), which is a product decision about index size, not a defect fix. The measurement is the input to that decision.



## 2026-11-20: a better stemmer than `lexical::stem` (Porter2, silent-e fix): not adopted

Question: `lexical::stem` is a light suffix stripper (`blended`, `blending`, `blends` meet at `blend`). It misses silent-e pairs (`cause` / `causing` / `caused`) and derivational and irregular forms. Does a stronger stemmer help the hybrid lexical side?

Method: `retrieval_eval <set> --variants default` (200 queries, 50 for TREC-COVID), one process per stemmer, same indexes, hybrid `default` variant, release build. Temporary switch in `lexical::stem` (not committed): `light` (shipped), `lighte` (shipped stemmer, then a trailing `e` dropped from roots of 4+ letters), `porter2` (Snowball English via `rust-stemmers` 1.2, already in `Cargo.lock`). Exact search does not use these stemmers and was not touched.

nDCG@10 (Recall@100 moved by at most 0.006 on every set):

| Set | light (shipped) | lighte | porter2 |
|---|---|---|---|
| SciFact | 0.7518 | 0.7493 | 0.7572 |
| NFCorpus | 0.3844 | 0.3854 | 0.3886 |
| Cranfield | 0.3988 | 0.3994 | 0.4041 |
| MS MARCO | 0.6674 | 0.6658 | 0.6630 |
| TREC-COVID | 0.4121 | 0.4160 | 0.4201 |
| mean | 0.5229 | 0.5232 | 0.5266 |

Result: `lighte` loses SciFact and MS MARCO (mean +0.0003). Porter2 wins four sets (+0.0042 to +0.0080) and loses MS MARCO (-0.0044); mean +0.0037. It also costs about 2 ms more per query (22.4 to 24.7 ms SciFact, 24.0 to 27.8 ms TREC-COVID; not isolated from background load). The owner rule adopts only what loses on none of the five sets, so neither is adopted. Porter2's gain is real on the scientific and biomedical sets and its loss is on the short, everyday-language queries, which fits an over-conflating stemmer; it was not tuned further (no per-length or per-suffix variants), since each variant would be fit to the same five sets.

Not measured: engineering folders (no labelled set), non-English text, and lemmatization with a dictionary (irregular forms such as `broke` / `broken`); `rust-stemmers` is English-only. The shipped stemmer stays.

### Addendum: nlprule lemmatization and the engineering sets

Question: does dictionary lemmatization (`nlprule` 0.6.4, English tagger) beat Snowball Porter2 (`rust-stemmers` English) and the shipped stemmer? `rust-bert` was excluded: it has no stemmer or lemmatizer and needs libtorch or ONNX, which breaks the clean-Windows, nothing-pre-installed rule.

Method: as above, one process per mode, same indexes, hybrid `default` variant, release build, one rerun of all four modes for a like-for-like table. Temporary switch in `lexical::stem` (not committed): `light` (shipped), `porter2`, `nlp` (first tagger lemma of an all-ASCII-letter word, else the word unchanged), `nlpfb` (same, but untagged words go through the shipped stemmer). Two engineering sets added as tiebreakers, not part of the gate: AeroEngQA and the ViDoRe industrial shard 0.

nDCG@10:

| Set | light (shipped) | porter2 | nlp | nlpfb |
|---|---|---|---|---|
| SciFact | 0.7518 | 0.7572 | 0.7420 | 0.7437 |
| NFCorpus | 0.3844 | 0.3886 | 0.3829 | 0.3830 |
| Cranfield | 0.3988 | 0.4041 | 0.4024 | 0.4024 |
| MS MARCO | 0.6674 | 0.6630 | 0.6691 | 0.6677 |
| TREC-COVID | 0.4121 | 0.4201 | 0.3983 | 0.4006 |
| gate mean (5) | 0.5229 | 0.5266 (+0.0037) | 0.5189 (-0.0035) | 0.5195 (-0.0029) |
| AeroEngQA | 0.9462 | 0.9537 | 0.9462 | 0.9462 |
| ViDoRe industrial | 0.4023 | 0.3992 | 0.3996 | 0.4005 |
| mean (7) | 0.5661 | 0.5694 (+0.0033) | 0.5629 (-0.0032) | 0.5634 (-0.0027) |

ViDoRe Recall@100: light 0.7822, porter2 0.7975, nlp 0.7826, nlpfb 0.7822. MRR@10: 0.4948, 0.4981, 0.4966, 0.5017.

Result: nothing is adopted; the shipped stemmer stays. Under the five-set rule Porter2 loses MS MARCO (-0.0044), `nlp` loses SciFact (-0.0098), NFCorpus (-0.0015) and TREC-COVID (-0.0138), and `nlpfb` loses the same three (-0.0081, -0.0014, -0.0115). Lemmatization wins only Cranfield and MS MARCO, by small margins, and the first-lemma simplification (ambiguous words such as `leaves` have several lemmas) is a likely reason it trails on the scientific and biomedical sets; the shipped-stemmer fallback recovers only about 0.0006 of the gate mean.

Ranking of the three: Porter2 (`rust-stemmers`) is best on retrieval (highest mean on both the five and seven sets, wins four gate sets and AeroEngQA, loses MS MARCO and ViDoRe nDCG); the shipped stemmer is second and is the only one with no data or dependency cost; `nlprule` is last. `nlprule` also needs an 11.6 MB tokenizer file whose language data is LGPL 2.1 (LanguageTool-derived), which the project avoided by retiring OpenCascade (ADR-023); the latest release is 0.6.4 and its maintenance is unconfirmed. Adopting it would have been an owner decision even if it had won.

Latency: not comparable. The reruns ran while other builds and tests loaded the machine (shipped stemmer SciFact 108.8 ms/query here against 22.4 ms in the first run), so no per-query cost is claimed for any mode. `nlprule` also loads the tokenizer once per process.

Not measured: engineering folders with real relevance labels beyond the two sets above, non-English text, and a lemma chooser that uses part of speech instead of the first tag.

## 2026-11-20: term clusters for the "Related from hits" chips

Question: the Find suggestion popover offers terms mined from the hits (`neural_search::corpus_term_candidates`) ranked by similarity to the query. Near-duplicates (`outer diameter`, `outside diameter`) fill the three slots. Can the neural embeddings that already rank them also group them by meaning?

Built: `neural_search::rank_and_group` embeds the context and candidates once (`rank_suggestions` is now a wrapper) and groups the candidates by greedy average-link clustering in input order (`group_by_similarity`, threshold `GROUP_SIMILARITY` = 0.60). The Find popover folds a mined term that is grouped with one already shown into that chip ("Related from hits: outer diameter (also outside diameter)"), so a slot goes to a different idea. Display only: exact search, results, ranking and the order of the chips are unchanged; only mined terms fold (synonyms and corrections never do); clicking a chip adds only the term it names.

Measurement: `search-core/examples/term_cluster_eval.rs` on `search-core/examples/data/term_clusters.tsv` (65 terms: 13 ideas of 3 or 4 terms and 10 singleton distractors, 81 true pairs), 20 shuffled input orders per threshold, pairwise precision and recall against the labels:

| Threshold | Precision | Recall | False merges per run | Groups per run |
|---|---|---|---|---|
| 0.50 | 0.853 | 0.376 | 5.2 | 42.8 |
| 0.55 | 0.921 | 0.361 | 2.5 | 44.8 |
| 0.60 | 0.983 | 0.326 | 0.4 | 47.4 |
| 0.65 | 0.975 | 0.286 | 0.6 | - |
| 0.70 | 0.964 | 0.180 | - | - |
| 0.75 | 1.000 | 0.074 | - | - |

0.60 is the lowest threshold with precision of at least 0.95: a wrong merge hides a distinct chip, while a missed merge only leaves a duplicate. Result: about a third of the true pairs merge, with about 98% of merges correct.

Limits: the 65 terms were written by the assistant and not reviewed by a person; the threshold was tuned on the same set (in-sample); English engineering phrases only; MiniLM-L6 on short phrases; no evaluation on real folders. Recall is low by design. Reproduce: `cargo run --release -p search-core --features neural-search --example term_cluster_eval`. No five-set retrieval gate applies because ranking and results do not change.

Found on the way: `SearchSession::corpus_term_suggestions` (n-gram mining over up to 40 hit lines) ran on every call of `query_suggestions`, which the Find popover makes several times per redraw. Measured in release with 40 hit lines of about 190 characters: 3.0 ms per call, so roughly 10 ms per frame while the popover is open with results. It is now cached on the query and the exact hit lines it was mined from (`corpus_term_cache`, same pattern as the typo cache); test `corpus_terms_are_mined_once_per_query_and_hits`. Re-timed on 2026-11-21 (ignored test `document_rebuild_cost`, release, 40 distinct hit lines of about 150 characters, 300 builds of the whole page document): 0.14 ms per redraw with the cache, 2.4 ms with the cache cleared before every build. The cache removes the mining from every redraw but the first after a query or hit change.

