#!/usr/bin/env python3
"""Write the judged English queries of ViDoRe v3 Industrial against the source PDF folder.

  python3 scripts/prepare_vidore_folder.py ../benchmarks/vidore-industrial

Output: <dir>/folder_queries.jsonl, one {"id","text","pages":[[doc_id, page_1_based],...]} per query that has
at least one judged page. Needs pyarrow (conversion only; not an app dependency).
"""
import glob, json, sys
import pyarrow.parquet as pq
d = sys.argv[1]
rd = lambda n, cols=None: pq.read_table(glob.glob(f"{d}/{n}/*.parquet")[0], columns=cols).to_pylist()
corpus = {r["corpus_id"]: (r["doc_id"], r["page_number_in_doc"] + 1) for r in rd("corpus", ["corpus_id", "doc_id", "page_number_in_doc"])}
queries = {r["query_id"]: r["query"] for r in rd("queries", ["query_id", "query", "language"]) if r["language"] == "english"}
rel = {}
for r in rd("qrels", ["query_id", "corpus_id", "score"]):
    if r["score"] >= 1 and r["query_id"] in queries and r["corpus_id"] in corpus:
        rel.setdefault(r["query_id"], set()).add(corpus[r["corpus_id"]])
with open(f"{d}/folder_queries.jsonl", "w") as out:
    for q, pages in sorted(rel.items()):
        out.write(json.dumps({"id": q, "text": queries[q], "pages": sorted([list(p) for p in pages])}) + "\n")
print(len(rel), "queries with judged pages;", len({p[0] for ps in rel.values() for p in ps}), "documents judged")
