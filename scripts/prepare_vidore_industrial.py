"""Make a diagnostic BEIR set from one ViDoRe Industrial parquet shard.

Requires pyarrow in the evaluation environment, not the application.
Usage: python3 scripts/prepare_vidore_industrial.py <download-folder> <output-folder> [--all-languages]
Only queries whose every positive page is in the shard are retained.
"""

import json
import sys
from collections import defaultdict
from pathlib import Path

import pyarrow.parquet as parquet


def prepare(source, output, all_languages=False):
    shard, = (source / "corpus").glob("*.parquet")
    pages = parquet.read_table(shard, columns=["corpus_id", "markdown"]).to_pylist()
    pages = {
        row["corpus_id"]: row["markdown"].strip()
        for row in pages if row["markdown"].strip()
    }
    judgments = parquet.read_table(
        next((source / "qrels").glob("*.parquet")),
        columns=["query_id", "corpus_id", "score"],
    ).to_pylist()
    relevant = defaultdict(dict)
    for row in judgments:
        if row["score"] > 0:
            relevant[row["query_id"]][row["corpus_id"]] = row["score"]
    eligible = {query_id for query_id, ids in relevant.items() if ids.keys() <= pages.keys()}
    queries = parquet.read_table(
        next((source / "queries").glob("*.parquet")),
        columns=["query_id", "query", "language"],
    ).to_pylist()
    if not all_languages:
        queries = [row for row in queries if row["language"] == "english"]
    eligible &= {row["query_id"] for row in queries}
    output.mkdir(parents=True, exist_ok=True)
    (output / "corpus.jsonl").write_text("".join(
        json.dumps({"_id": str(key), "title": "", "text": value}) + "\n"
        for key, value in sorted(pages.items())
    ), encoding="utf-8")
    (output / "queries.jsonl").write_text("".join(
        json.dumps({"_id": str(row["query_id"]), "text": row["query"]}) + "\n"
        for row in queries if row["query_id"] in eligible
    ), encoding="utf-8")
    (output / "qrels").mkdir(exist_ok=True)
    (output / "qrels" / "test.tsv").write_text(
        "query-id\tcorpus-id\tscore\n"
        + "".join(
            f"{query_id}\t{page_id}\t{score}\n"
            for query_id in sorted(eligible)
            for page_id, score in sorted(relevant[query_id].items())
        ),
        encoding="utf-8",
    )
    scope = "all languages" if all_languages else "English"
    print(f"{len(pages)} pages, {len(eligible)} fully judged {scope} queries from {shard.name}")


if __name__ == "__main__":
    prepare(Path(sys.argv[1]), Path(sys.argv[2]), "--all-languages" in sys.argv[3:])
