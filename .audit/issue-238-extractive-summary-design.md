# Extractive summary design

## Goal and rubric

Add a truthful summary to the selected Search document using the bundled MiniLM model. It must work without a Find question, show unchanged source lines and citations, stay bounded, reject stale work, run offline, and leave exact search untouched.

## Candidates

- Candidate A: sample extracted lines across a <=1 MiB preview; rank by similarity to a neural document centroid with MMR diversity. `search-core/neural_search.rs` owns scoring, `SearchSession` owns async generation/cancellation, and Index panel renders original lines and citations.
- Candidate B: reuse the question field to rank preview lines against a query. It provides a query digest but requires a question and overlaps with existing cited evidence ranking.

## Decision

Use A. Both independent designers and the cross-judge favored question-independent summary behavior. Graft B's explicit sample-coverage notice and selected-document identity guard. Keep scores internal; similarity is not calibrated confidence.

## Bounds and semantics

- Refuse source previews above 1 MiB rather than summarize a silently truncated prefix.
- Consider at most 256 evenly sampled nonblank extracted lines. Skip lines beyond the model's token limit before inference.
- Embed in existing MiniLM batches. Return at most five unchanged lines, source ordered, with extracted-line numbers.
- Select by document-centroid similarity with an MMR redundancy penalty; this is extractive salience, not generated prose or an answer.
- One active worker per session; cancellation checks between bounded model batches. Accept only matching preview generation, path, and selected document.
- PDF citations remain flattened extracted-line numbers because current extraction has no page provenance.
