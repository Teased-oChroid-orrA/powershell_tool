# Issue #238 gap audit

Done predicate: each partial roadmap item has a concrete lowest-risk next unit, verified open constraint, and evidence pointer. Read-only audits; no edits.
Artifact: consolidated roadmap gap report.

- [x] Frame — partition Q&A, entity/related-document, and OCR gaps.
- [x] Fan out — independent read-only source audits.
- [x] Aggregate — reconcile findings with checklist and current source.
- [x] Report — update roadmap and log.

## Findings

- Q&A: current MiniLM ranks extractive evidence only. `search-ml` bundles an embedding encoder, not a text generator. Extraction flattens PDF pages, so page citations cannot be reconstructed safely.
- Entity ranking: 12 manually labeled spans are an integration probe, not held-out quality evidence. No model-quality claim is supported.
- Related documents: candidates are cited and self-excluded; similarity is already labeled as non-proof. Exact diffs and duplicate/revision calibration remain open.
- OCR: bundled OCR exposes text-detection probability, not calibrated recognition confidence. Provenance warning and exact-line highlighting already exist.
- Next unit: implement exact, bounded comparison of two opened document texts; keep semantic similarity as candidate discovery only.
