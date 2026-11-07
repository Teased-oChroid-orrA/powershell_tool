# Related-document exact comparison

Goal: Let users inspect exact line changes after neural similarity finds a possible related file. Similarity remains discovery only.

- [x] Ground the selected-preview, semantic candidate, extraction, and UI flows.
- [x] Sketch two bounded ownership designs and cross-judge them.
- [x] Wire the bounded comparison helper into SearchSession and the Index panel.
- [x] Add focused helper tests for changed lines, line limits, and output limits.
- [x] Test insertion/deletion/repeated lines, byte/work caps, clipping, cancellation, and stale completion.
- [x] Run focused app tests and review the diff.

Proof: `cargo test -p app-tui --features semantic-search` passed 1,281 tests; 31 ignored. Exact source/candidate extraction runs off the UI thread, bounded to one active worker, and stale scope or preview results are discarded.

Done predicate: Search Index panel can request exact line comparison between selected source and a related candidate; output is bounded and rejected when scope or preview changed.
