# Issue #6 Phase 2: query modes, ranking, extractor abstraction, resource classes

Continuation of `docs/issue-6-status.md`'s original "out of scope for this
phase" list, picked up after Phase 1 (`docs/issue-6-phase-1.md`) shipped.
Four independent items, each verified and tested individually.

## Fuzzy/wildcard query verification (epic §23)

Added `NativeSearchEngine::search_fuzzy` (edit-distance search,
`native-search/src/engine.rs`) - not exposed in the app UI (the raw-query
panel that would have hosted it was trimmed as redundant once Run Search
started routing through the trigram index automatically), but a real,
tested library capability. Verified two things empirically rather than
assumed from source reading:

- **Fuzzy works**: `search_fuzzy("torqe", 1, ...)` (a typo missing one
  character) finds `"torque"` where a plain `search("torqe", ...)` finds
  nothing - proving it isn't accidentally fuzzy by default.
- **Single-word prefix/wildcard does NOT work in this Tantivy version**
  (`0.26.1`) - a real, previously-undocumented limitation found while
  writing the verification test, not the "already worked" assumption the
  task started with. `generate_literals_for_str` only builds a
  `PhrasePrefixQuery` when tokenizing the phrase text produces 2+ tokens;
  a single-token quoted phrase-prefix (`"engin"*`) is a hard
  `QueryParserError` ("does not produce at least two terms"), and a bare
  unquoted `engin*` silently becomes an ordinary exact-term query for the
  literal text "engin" (0 results against "engine"/"engineering" tokens,
  no error at all). Only a *multi-word* quoted phrase ending in `*`
  (`"the engin"*`) actually produces a working prefix match, on its last
  term only. All three behaviors are asserted directly in
  `engine::tests::search_supports_prefix_wildcard_on_multi_word_phrases_only`,
  not just described in a comment - a future Tantivy upgrade that changes
  this gets caught by a failing test.

## Ranking tuning (epic §48)

`filename`/`title` field matches are now boosted (3x/2x) over `body`
matches via `QueryParser::set_field_boost` - a term appearing in a file's
own name is a stronger relevance signal than the same term appearing once
in its body text. Shared by `search`/`search_fuzzy` via one
`build_parser()` helper so the two query modes can't diverge on which
fields matter or by how much. Verified with a real two-document test
(`search_ranks_filename_match_above_body_only_match`) - a term in one
doc's filename outranks the same term appearing once in another doc's
body.

## Extractor trait abstraction (epic §5)

`search-core/src/extraction.rs` gained an `Extractor` trait + a static
`EXTRACTORS` registry (`DocxExtractor`, `PptxExtractor`, `XlsxExtractor`,
`ZipExtractor`, `PdfExtractor`, `RtfExtractor`, plus `PlainTextExtractor`
as the not-registered-by-extension fallback). Adding a format is now
registering a new impl, not editing `extract_lines_by_extension`'s match
arm. Deliberately thin - each impl still calls the same hand-rolled,
byte-for-byte-tested `extract_*_lines` free functions this module always
had (see `CLAUDE.md`'s extraction design notes for why those stay
hand-rolled rather than adopting a generic OOXML/PDF parser crate; this
doesn't change that reasoning, only how the dispatch table is expressed).
Pure refactor - all 89 existing search-core tests (including every real
DOCX/PPTX/XLSX/PDF/ZIP fixture test) pass unchanged, proving byte-for-byte
equivalent dispatch behavior.

## Per-resource-class concurrency limits (epic §19)

New `SearchSettings.heavy_throttle_limit`, separate from the existing
`throttle_limit`. `orchestrator::is_heavy_extension` classifies
`.pdf`/`.docx`/`.pptx`/`.xlsx`/`.zip` as heavy (real container-format
parsing) and everything else (including `.rtf` - plain regex tag-
stripping over already-decoded text, not a container format) as light.
The parallel branch now runs two independent `Semaphore`s instead of one
shared one, so a folder mixing a handful of large PDFs into thousands of
small log files doesn't let the PDFs' extraction cost starve the log
files' throughput, or vice versa. Default heavy limit is closer to the
raw core count (not 2x, like the light default) and clamped to a smaller
[2, 16] range, since container-format parsing is more CPU/memory-bound
per file than plain-text reading. UI: a second throttle field in
"Performance and robustness," both now gated behind the "Parallel
processing" checkbox (previously ungated, a real gap - the field was
visible and editable even though it did nothing with parallel off).

## Verification

search-core: 90/90 (was 89, +1: extension classification, plus the
extractor-trait refactor's zero test changes already counted above).
native-search: 38/38 (was 35, +3: fuzzy, wildcard limitation, ranking).
app: 8/8 unchanged. `cargo build -p app` clean, background launch shows
no panic after each change.

## Still out of scope

Streaming HTML export, CLI/headless entry point, SQLite metadata store -
larger items, tracked separately.
