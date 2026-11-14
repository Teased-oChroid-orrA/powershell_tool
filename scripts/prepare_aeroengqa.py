"""Convert the four AeroEngQA JSON sets to the existing BEIR evaluation shape.

Usage: python3 scripts/prepare_aeroengqa.py ../benchmarks/aeroengqa
The source JSON files belong in <folder>/raw; keep downloaded data outside git.
"""

import hashlib
import json
import sys
from pathlib import Path


SETS = (
    ("single-hop", True),
    ("multi-hop", True),
    ("single-hop-unanswerable", False),
    ("multi-hop-unanswerable", False),
)


def write_jsonl(path, rows):
    path.write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows),
        encoding="utf-8",
    )


def prepare(folder):
    corpus = {}
    queries = []
    qrels = []
    unanswerable = []
    for name, answerable in SETS:
        records = json.loads(
            (folder / "raw" / f"AeroEngQA_{name}.json").read_text(encoding="utf-8")
        )
        for record in records:
            question = record["question"].strip()
            contexts = [record[key].strip() for key in ("context",) if key in record]
            contexts += [record[key].strip() for key in ("context-1", "context-2") if key in record]
            assert question and contexts and all(contexts)
            assert bool(record.get("answer")) == answerable
            query_id = f"{name}-{record['id']:03d}"
            queries.append({"_id": query_id, "text": question})
            if not answerable:
                unanswerable.append(query_id)
            for context in contexts:
                doc_id = hashlib.sha256(context.encode()).hexdigest()[:24]
                assert doc_id not in corpus or corpus[doc_id]["text"] == context
                corpus[doc_id] = {"_id": doc_id, "title": "", "text": context}
                if answerable:
                    qrels.append((query_id, doc_id, 1))

    assert len(queries) == len({query["_id"] for query in queries})
    assert len(qrels) == 60 and len(unanswerable) == 40
    write_jsonl(folder / "corpus.jsonl", [corpus[key] for key in sorted(corpus)])
    write_jsonl(folder / "queries.jsonl", queries)
    write_jsonl(folder / "unanswerable.jsonl", [{"_id": key} for key in unanswerable])
    (folder / "qrels").mkdir(exist_ok=True)
    (folder / "qrels" / "test.tsv").write_text(
        "query-id\tcorpus-id\tscore\n"
        + "".join(f"{q}\t{d}\t{score}\n" for q, d, score in qrels),
        encoding="utf-8",
    )
    print(
        f"{len(corpus)} contexts, {len(queries)} queries, "
        f"{len(qrels)} answerable judgments, {len(unanswerable)} unanswerable questions"
    )


if __name__ == "__main__":
    prepare(Path(sys.argv[1]))
