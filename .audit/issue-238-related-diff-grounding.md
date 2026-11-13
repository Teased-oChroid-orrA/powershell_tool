# Related-document comparison grounding

Caller: Search Index panel presents up to three related SemanticHit candidates. SearchAct::OpenRelated only opens a candidate externally. `SearchSession::find_related_documents` builds candidates from the selected preview text and binds them to RelatedSearchKey { source_path, preview_generation, scope }.

Source: `SearchSession::refresh_preview` caches full extracted lines or asynchronously loads them through `read_file_bytes_robust` + `extract_lines_by_extension`. `Preview` holds the full `DocText`. Candidate extraction must reuse those existing reader/extractor APIs and enforce the current max file size and cancellation behavior.

Ownership: candidates and event acceptance belong to `app-tui/src/gpu_viewer/editor/search/mod.rs`; Index panel actions and bounded line rendering belong to `view.rs`; behavioral proof belongs to `tests.rs`. No search-core matcher/result membership changes. Similarity stays a candidate signal; exact textual comparison makes no duplicate/revision classification.

Constraints: offline self-contained runtime; avoid blocking the UI; stale worker results must be discarded if source preview or query scope changes; bound bytes/lines and rendered diff rows; PDF line numbering is flattened because current extraction has no page provenance.
