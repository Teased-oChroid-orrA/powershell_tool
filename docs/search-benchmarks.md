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
