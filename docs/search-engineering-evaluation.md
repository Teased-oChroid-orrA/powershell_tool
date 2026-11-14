# Engineering search evaluation baseline (2026-11-13)

This is a diagnostic baseline for Search Workbench retrieval, not a ranking change. The existing five-set gate in [search-benchmarks.md](search-benchmarks.md) still applies before adopting any retrieval change. Public data and generated indexes stay outside this repository under `../benchmarks/`.

## Data and judgments

| Set | Source and license | What is judged | Important limit |
|---|---|---|---|
| [AeroEngQA](https://zenodo.org/records/14215677), DOI `10.5281/zenodo.14215677` | CC BY 4.0, four JSON files, Zenodo checksums verified on download | 20 single-context and 20 two-context answerable questions; 20 of each are unanswerable | The supplied contexts are already selected and the 119 distinct contexts make a small, easy retrieval pool. The 40 unanswerable questions have no positive relevance judgment and are excluded from ranking metrics. A cited passage is not a verified answer. |
| [ViDoRe v3 Industrial](https://huggingface.co/datasets/vidore/vidore_v3_industrial), revision `e26c864724f5dd71a3d7d739272d95637764cee9` | Qrels and annotations CC BY 4.0; each source PDF and parsed Markdown inherits the publisher license listed in `documents_metadata` | Questions with human-verified relevant pages, graded scores 1/2, and evidence bounding boxes, for aircraft technical manuals | This run uses one of five parquet shards: 974 nonempty pages and only 149 English queries whose every positive judgment is in that shard. It uses published Markdown, not our PDF extraction. Scores are diagnostic and cannot be compared with the full benchmark's leaderboard. |

The [STELLA aerospace corpus](https://huggingface.co/datasets/telepix/STELLA) is the next retrieval stress test: about 2.4 million NASA-report passages, 1,000 synthetic terminology-concordant and paraphrased queries, and BEIR-format judgments (CC BY 4.0). Its processed passages do not test PDF extraction. A representative private engineering folder still needs human-judged queries to test actual use.

## Reproduction

Download the four `AeroEngQA_*.json` files from Zenodo into `../benchmarks/aeroengqa/raw/`. Download ViDoRe's `queries/test-00000-of-00001.parquet`, `qrels/test-00000-of-00001.parquet`, `documents_metadata/test-00000-of-00001.parquet`, and `corpus/test-00000-of-00005.parquet` at the pinned revision into the corresponding directories under `../benchmarks/vidore-industrial/`. The `pyarrow` Python package is needed only to convert parquet for evaluation; it is not an app dependency.

```sh
mkdir -p ../benchmarks/aeroengqa/raw
for name in AeroEngQA_single-hop AeroEngQA_multi-hop AeroEngQA_single-hop-unanswerable AeroEngQA_multi-hop-unanswerable; do
  curl -fsSL "https://zenodo.org/api/records/14215677/files/$name.json/content" -o "../benchmarks/aeroengqa/raw/$name.json"
done
for name in queries/test-00000-of-00001.parquet qrels/test-00000-of-00001.parquet documents_metadata/test-00000-of-00001.parquet corpus/test-00000-of-00005.parquet; do
  mkdir -p "../benchmarks/vidore-industrial/${name%/*}"
  curl -fsSL "https://huggingface.co/datasets/vidore/vidore_v3_industrial/resolve/e26c864724f5dd71a3d7d739272d95637764cee9/$name" -o "../benchmarks/vidore-industrial/$name"
done
python3 -m pip install --target ../benchmarks/.python pyarrow
```

```sh
python3 scripts/prepare_aeroengqa.py ../benchmarks/aeroengqa
PYTHONPATH=../benchmarks/.python python3 scripts/prepare_vidore_industrial.py ../benchmarks/vidore-industrial ../benchmarks/vidore-industrial-shard0
PYTHONPATH=scripts:../benchmarks/.python python3 -m unittest scripts/test_prepare_vidore_industrial.py
cargo build -p search-core --release --features semantic-search --example retrieval_eval
target/release/examples/retrieval_eval ../benchmarks/aeroengqa --variants default,dense,keyword-led
target/release/examples/retrieval_eval ../benchmarks/vidore-industrial-shard0 --variants default,dense,keyword-led
```

Both adapters produce the existing `corpus.jsonl`, `queries.jsonl`, and `qrels/test.tsv` shape. AeroEngQA also writes `unanswerable.jsonl` to keep those cases visible without inventing relevance judgments. `retrieval_eval` indexes each passage as a local text file and queries the actual `SemanticIndex`; it does not exercise the Search window or PDF parser. ViDoRe's original qrel grades are preserved.

ViDoRe publishes each question in six languages. The adapter defaults to English because the bundled MiniLM model is English-trained; `--all-languages` retains all 894 eligible translations for a separate stress test. Mixing those sets into one headline score obscures the English result.

For the per-query slices below, run `retrieval_eval ../benchmarks/vidore-industrial-shard0 --rank-experiments ../benchmarks/vidore-industrial-analysis-english` with a new, empty output directory. `perquery.tsv` has set, query number, query ID, term count, variant, nDCG@10, and Recall@100 columns. Filter variant `default`, join query ID to the published queries parquet, then average each metric by `query_generator` (`human` or `sdg`) and by `content_type == ["Text"]` versus the other content types. The existing experiment harness appends to its output files, so do not reuse an output directory for a second run. For the six-language stress result, convert to a separate output folder with `--all-languages` and run the same retrieval command there.

## Results on this Mac

| Set and variant | nDCG@10 | Recall@100 | MRR@10 | Query time |
|---|---:|---:|---:|---:|
| AeroEngQA dense | 0.9032 | 1.0000 | 0.9363 | 8.3 ms |
| AeroEngQA keyword-led | 0.9213 | 1.0000 | 0.9750 | 8.8 ms |
| AeroEngQA current hybrid | **0.9462** | 1.0000 | **0.9875** | 10.0 ms |
| ViDoRe English shard dense | 0.3403 | 0.7516 | 0.4118 | 14.8 ms |
| ViDoRe English shard keyword-led | 0.3675 | 0.7410 | 0.4644 | 13.5 ms |
| ViDoRe English shard current hybrid | **0.4023** | **0.7822** | **0.4948** | 14.9 ms |

AeroEngQA indexed 119 contexts in 4 seconds. ViDoRe indexed 974 pages in 116 seconds; a second run reused all vectors. AeroEngQA's Recall@100 is weak evidence because its entire corpus has only 119 contexts. ViDoRe's English page recall warrants failure analysis before changing ranking: questions often require visual or table content, and the evaluation uses parsed Markdown rather than the source PDFs. No model or ranker was adopted from these results.

The existing `retrieval_eval --rank-experiments` per-query file shows, for current hybrid ranking on ViDoRe English: 67 human-written questions nDCG@10 0.5166 and Recall@100 0.8583; 82 generated questions 0.3088 and 0.7200. Text-only evidence questions (91) score 0.4456 and 0.7950; questions tagged for tables or visual evidence (58) score 0.3343 and 0.7620. These labels overlap other query attributes, so the differences describe slices, not causes. Across all six languages, hybrid nDCG@10 is 0.1525 and Recall@100 0.4537; the English-only result is the relevant baseline for the shipped model.

Three inspected English, human-written, text-only misses (`query_id` 192, 197 and 207) have answer-bearing text in a judged page's published Markdown, but none places a judged page in the top ten. Query 192's score-2 page states the 15-minute fire-department notice; query 197's judged pages describe Crew Station Supervisor duties; query 207's judged pages include the engine-shutdown procedure. This small check proves that some misses occur after text is available to the retriever. It does not estimate the share caused by ranking across all queries.

One downloaded ViDoRe PDF, `AFD-091005-056.pdf` (18 pages, 0.5 MB), passed the existing extraction timing harness in 0.035 seconds: 398 lines, 14,838 UTF-8 bytes of text, no truncation. This counts extracted text; it does not measure word accuracy or page-citation correctness.

```sh
mkdir -p ../benchmarks/vidore-industrial/pdfs
curl -fsSL "https://huggingface.co/datasets/vidore/vidore_v3_industrial/resolve/e26c864724f5dd71a3d7d739272d95637764cee9/pdfs/AFD-091005-056.pdf" -o ../benchmarks/vidore-industrial/pdfs/AFD-091005-056.pdf
cargo run -p search-core --release --example pdf_extract_bench -- ../benchmarks/vidore-industrial/pdfs/AFD-091005-056.pdf
```

## Next measurement

1. Inspect missed ViDoRe questions by text, table, and visual evidence type. Check whether the judged page's Markdown contains the answer before attributing misses to ranking.
2. Run a bounded PDF extraction and page-citation comparison against source PDFs and judged pages. Include mixed text-and-scan files where a public label exists.
3. Use STELLA only with a feasible full-corpus plan; sampling passages changes the retrieval task. Keep a small private, human-judged engineering folder as the product acceptance set.
