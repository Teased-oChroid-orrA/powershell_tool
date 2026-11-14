"""Focused adapter check. Run with pyarrow on PYTHONPATH."""

import json
import tempfile
import unittest
from pathlib import Path

import pyarrow as arrow
import pyarrow.parquet as parquet

from prepare_vidore_industrial import prepare


class PrepareVidoreTest(unittest.TestCase):
    def test_keeps_english_complete_judgments_and_original_grades(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source"
            output = Path(directory) / "output"
            for name, rows in {
                "corpus": [
                    {"corpus_id": 1, "markdown": "fuel shutoff valve"},
                    {"corpus_id": 2, "markdown": ""},
                    {"corpus_id": 3, "markdown": "hydraulic line"},
                ],
                "qrels": [
                    {"query_id": 10, "corpus_id": 1, "score": 2},
                    {"query_id": 11, "corpus_id": 1, "score": 1},
                    {"query_id": 12, "corpus_id": 2, "score": 2},
                    {"query_id": 13, "corpus_id": 3, "score": 1},
                    {"query_id": 13, "corpus_id": 4, "score": 2},
                ],
                "queries": [
                    {"query_id": 10, "query": "fuel shutoff?", "language": "english"},
                    {"query_id": 11, "query": "arrêt du carburant?", "language": "french"},
                    {"query_id": 12, "query": "blank page?", "language": "english"},
                    {"query_id": 13, "query": "two pages?", "language": "english"},
                ],
            }.items():
                path = source / name / "test.parquet"
                path.parent.mkdir(parents=True)
                parquet.write_table(arrow.Table.from_pylist(rows), path)

            prepare(source, output)
            queries = [json.loads(line)["_id"] for line in (output / "queries.jsonl").read_text(encoding="utf-8").splitlines()]
            self.assertEqual(queries, ["10"])
            self.assertEqual((output / "qrels" / "test.tsv").read_text(encoding="utf-8").splitlines()[1:], ["10\t1\t2"])

            prepare(source, output, all_languages=True)
            queries = [json.loads(line)["_id"] for line in (output / "queries.jsonl").read_text(encoding="utf-8").splitlines()]
            self.assertEqual(queries, ["10", "11"])


if __name__ == "__main__":
    unittest.main()
