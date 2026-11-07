# Related-document exact comparison

## Problem

SearchSession already owns the selected source preview, scoped related-document candidates, and asynchronous event acceptance. The UI needs an exact comparison path while preserving exact result membership and avoiding a new public core API. The bundled model discovers candidates only; this comparison assigns no duplicate or revision verdict.

## Usage

In Index > Possible related documents, click **Compare exact lines** beside a candidate. The current extracted text from source and candidate is read for that request. Index shows added and removed lines with source/candidate line numbers. Similarity stays a discovery signal.

```rust
SearchAct::CompareRelated(index)
SearchSession::compare_related(index)
view::cards(session) // bounded result rows
```

## Shape

Keep orchestration in `SearchSession`; put the pure exact comparison helper in a private `search/diff.rs` module. Reuse `read_file_bytes_robust` and `extract_lines_by_extension`. Do not export comparison policy from search-core for one UI caller.

```rust
RelatedDiffKey { related: RelatedSearchKey, candidate_path: String }
CompareRequest { key, source_path, candidate_path, config }
CompareEvent { generation, key, result: Result<LineDiff, String> }
LineDiff { added, removed, unchanged, changes: Vec<DiffRow>, omitted_changes }
DiffRow { side: Source | Candidate, line_number, text, clipped }
```

The private diff helper compares extracted lines byte-for-byte after existing extractor normalization. It ignores final-newline and container-byte differences; it makes no file identity claim. Reject either input above 1 MiB, 1,500 extracted lines, or 2,250,000 comparison cells. Intern line strings once, then use a bounded `u16` LCS table. Keep at most 100 changed rows, clip each displayed line to 240 Unicode scalar values, and report omitted changes. Never truncate inputs before comparison.

One active comparison worker only. A new request cancels the active request cooperatively and replaces one pending request; after the worker returns, launch the latest pending request only if its source and candidate remain current. File extraction runs on `spawn_blocking`; it cannot be interrupted, so file-size and extractor limits apply before work. Diff checks the cancellation token within its matrix/reconstruction loops.

Read both source and candidate fresh for each request. Before accepting a result, require its generation and `RelatedDiffKey` to match and confirm the candidate remains in `current_related_hits()`. Selection, preview, query/scope changes, new requests, and session drop invalidate results. The UI labels lines as extracted-text line numbers (PDF pages are flattened).

## Tests

Pure helper: identical, insertion, deletion, replacement, repeated lines, work/byte/line caps, clipping, omitted changes, and cooperative cancellation. Session: exact source/candidate paths shown, exact result selection unchanged, stale scope/selection discarded, and replacing a request launches only the latest pending candidate.

## Synthesis

Use Candidate A's SearchSession ownership and private helper, with Candidate B's explicit bounded LCS table and work cap. Add the cross-judge's required byte cap before and after reads, changed-row-only output, exact extracted-line semantics, fresh source/candidate reads, cooperative diff cancellation, and single active worker with one latest pending request. Reject both candidates' uncapped post-extraction memory assumptions. No external diff dependency.

## Tradeoffs accepted

- Inputs above the fixed limits are refused rather than partially compared.
- PDF page provenance stays unavailable; UI reports flattened extracted-line numbers.
- Similarity finds candidates but does not classify duplicate or revision status.
- A currently running PDF extraction cannot be interrupted; only one comparison job runs at a time.

## Alternatives rejected

- Compare only the semantic excerpt: it cannot support a whole-document diff.
- Export a general diff API from search-core: only the Search Workbench consumes it today.
- Add `similar`: it adds a dependency while a small, testable bounded LCS helper meets the scoped need.

## Open risks

The current extractor may allocate expanded text before the post-extraction byte cap. Keep per-file raw bytes bounded to 1 MiB and honor existing extractor limits. The output is an exact comparison of the two extracted snapshots, not a byte-level file comparison.
