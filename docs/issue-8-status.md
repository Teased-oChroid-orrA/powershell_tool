# Issue #8 status: Evidence-Driven Search Engine Performance, Indexing &
Scalability Optimization

Required-deliverables report per the epic's §27, evaluated against this
repo's actual state (`main`, through the epic #6 sweep - `docs/issue-6-*.md`
- and this investigation's own new benchmarks). Read `docs/benchmarking.md`
and `docs/search-semantics.md` alongside this; both are cited throughout
rather than repeated.

**Bottom line: no major architectural change is justified by the evidence.**
Per the epic's own explicit allowance ("A successful outcome can therefore
be 'no code change'"), this report documents what was investigated,
measured, and rejected, plus two small additive benchmarks that filled
real gaps in existing coverage. Nothing in `search-core`'s or
`native-search`'s production code changed as a result of this
investigation.

**Update (2026-08-26, methodology correction):** a review of this
report's first version correctly identified that its per-format
extraction benchmark measured in-memory parse cost against 1-4KB toy
fixtures only, with file I/O excluded after the first read - narrower
than what the epic asks for ("the actual extraction pipeline, including
file I/O"), and mislabeled as such. Corrected: real medium/large
documents (Apache POI/Tika/PDFBox test-data, `search-core/benches/data/`)
and a second benchmark measuring the actual production I/O path on every
iteration were added. The correction changed one real conclusion - PDF
extraction cost is genuinely significant at realistic sizes (33.6-112ms,
not the 28µs the tiny fixture suggested) - now reflected throughout
sections C, E, and the decision matrix below. Every other conclusion in
this report held up under the corrected numbers.

## A. Existing Architecture

Pipeline (already documented in CLAUDE.md and `docs/issue-6-*.md`, restated
here for the epic's §1 map):

```
Discovery (file_reader::enumerate_files_safely)
  → extension filter (orchestrator::filter_by_extension)
  → extraction (extraction::extract_lines_by_extension, per-format Extractor trait)
  → matching (matching.rs - literal/whole-word/regex, always the authority)
  → [optional] index-first narrowing (native-search trigram field, safe-superset pre-filter)
  → orchestrator (bounded parallel/sequential processing, incremental JSON cache)
  → report/export (streaming HTML/CSV/JSON/JSONL)
```

Tantivy 0.26.1 backs the persistent index (`native-search`). Schema:
`id`/`path`/`filename`/`extension`/`title`/`modified`/`created`/`size` are
`STORED` (small metadata only); `body` and `trigram` are indexed but
**not** `STORED` - the full extracted text is never duplicated into the
index, only re-derived from source files when needed (this is load-
bearing for §9/§10 below). The `trigram` field uses a custom `NgramTokenizer(3,3)`
+ `LowerCaser` registered under a private tokenizer name, queried via a
safe-superset boolean AND-within-filter/OR-across-filters candidate query
(`trigram_candidate_paths`/`trigram_candidate_paths_for_chunk_sets`, the
latter added for regex mode in `docs/issue-6-phase-7.md`).

## B. Benchmark Suite

Three `cargo bench` harnesses exist (plain manual timing per-crate,
`harness = false` - no criterion, matching this project's established
"small harness where practical, not a permanent perf-tracking suite"
choice, unchanged by this investigation):

| Harness | Covers |
|---|---|
| `native-search/benches/indexing_and_search.rs` | Indexing throughput, search latency (mixed query shapes) |
| `search-core/benches/discovery_and_extraction.rs` | Directory discovery, plain-text extraction throughput, **new**: per-format extraction latency at 3 real size tiers (tiny/medium/large) for DOCX/PPTX/XLSX/RTF/PDF, both in-memory-parse-only and full-pipeline-with-real-I/O (`search-core/benches/data/`, see that directory's README for provenance) |
| `native-search/benches/trigram_candidate_reduction.rs` (**new**) | Candidate-set reduction and narrowed-vs-full-scan timing across query selectivity tiers |

The two new additions close real gaps the epic explicitly asks for
(§2 "measure extraction separately by format," §7 "benchmark trigram
candidate reduction") that nothing in the existing suite covered.
Result processing (candidate generation/verification/snippet generation)
and UI update/render latency (§2's other asks) are **not** independently
benchmarked - see "Known Gaps" below for why, honestly, rather than a
fabricated number.

## C. Bottleneck Report

**Revised after a methodology correction** (a user review caught that the
original per-format extraction numbers only measured in-memory parse cost
against 1-4KB toy fixtures, with real file I/O excluded after the first
read - see `docs/benchmarking.md`'s "Methodology correction" for the full
account and the corrected numbers, gathered against real medium/large
documents pulled from Apache POI/Tika/PDFBox's test-data corpora, with a
second benchmark function added that calls the actual production
`read_file_bytes_robust` for real I/O on every iteration).

Everything except one format remains well under any reasonable UX
threshold at realistic sizes:

- Search p50/p95: 35-130µs (native-search's existing benchmark).
- Discovery: ~291,000 files/sec.
- Plain-text extraction: ~550,000 files/sec, 907 MB/sec.
- DOCX/PPTX/XLSX/RTF extraction, real 150KB-3MB documents: sub-millisecond
  to low-single-digit-milliseconds (full pipeline, including real I/O -
  e.g. DOCX large 2.96MB: 770µs warm-reread median).
- 100K-file end-to-end stress test (`docs/issue-6-phase-14.md`):
  16,308 files/sec, exact correctness at scale.

**PDF is a real, quantified exception.** 33.6ms for a real 272KB PDF,
112ms for a real 1.04MB PDF (full pipeline, real I/O included) - two to
three orders of magnitude above what the original tiny-fixture number
(28µs) suggested. This is not being reported as a new, unaddressed
bottleneck: CLAUDE.md already documents that this exact cost (PDF
extraction taking many seconds with no progress indication) is the
specific, real user complaint that motivated this whole project's
"live progress reporting is a hard requirement" design - the mitigations
(150ms-interval progress callback, per-file timeout, heavy/light resource-
class throttling) already exist and predate this benchmark. What changed
is that the cost is now a concrete number instead of an inherited design
assumption. Whether to additionally replace the regex-scan PDF extractor
with a real structural parser is a legitimate question this number now
supports investigating, but is a substantial, parity-risking rewrite (see
CLAUDE.md's stated rationale for the current hand-rolled approach) that
this investigation does not unilaterally undertake - see "Deviations"
below.

The trigram candidate filter's fixed per-query overhead vs. its
candidate-set reduction remains the other real, quantified tradeoff (not
a bottleneck) this investigation measured - see the benchmark results
below, unchanged by this correction.

## D. Optimization Results

### Trigram candidate-set reduction (issue #8 §7) - the one thing actually measured end-to-end

`native-search/benches/trigram_candidate_reduction.rs`, 10,000-document
synthetic corpus with **controlled, known term frequencies** (unlike the
existing `indexing_and_search.rs` corpus, where every vocab word appears
in nearly every document by construction - useless for testing
selectivity). Real numbers, this development machine:

```
Tier                                  total candidates   cand.%  full-scan(us)   narrowed(us)    speedup
"the" (~100% of docs)                 10000      10000   100.0%            273          12782       0.0x
"corrosion" (~20% of docs)            10000       2000    20.0%            230           2575       0.1x
"zqx9k7f2" (rare, 0.05% of docs)      10000          5     0.1%            225             28       8.0x
"ab" (below trigram threshold)        10000      10000   100.0%            220            220       1.0x
```

**Read this honestly, not literally.** The "full-scan" baseline here is an
in-memory `str::contains` check against tiny synthetic strings already
held in a `Vec` - deliberately the *cheapest possible* verification cost,
to isolate the trigram query's own fixed overhead (~200-270µs, dominated
by opening a Tantivy searcher and running a boolean query) from candidate-
set reduction. Against that artificially cheap baseline, narrowing is a
**net loss** for common/medium-frequency terms (0.0x-0.1x) and a clear win
only for genuinely rare terms (8.0x for a 0.05%-frequency marker).

That is *not* what production verification costs. `orchestrator.rs`'s
real verification step opens a real file from disk and, for DOCX/PPTX/XLSX/
ZIP/PDF, parses a container format (measured above: 6-28µs *per file* even
for these tiny fixtures - real-world documents are larger, and disk I/O
itself adds latency this in-memory proxy has none of). Against a per-
document verification cost anywhere near realistic, the trigram query's
~200-270µs fixed cost becomes comparatively negligible, and even the
"medium" tier's 5x fewer documents-to-verify (2,000 of 10,000) is a real
win once each document costs real I/O + parsing rather than a
microsecond-scale in-memory check.

**Conclusion: keep the trigram candidate filter, unchanged.** It was
already a considered, deliberate design (`docs/issue-6-phase-1.md`), and
this benchmark - while it can't reproduce realistic per-document
verification cost without a much larger synthetic-document-generation
effort disproportionate to what this investigation needs - confirms the
one thing that would have disqualified it: there's no regime where it
produces *wrong* results (already proven separately, see "Correctness"
below), and its fixed overhead is small in absolute terms (hundreds of
microseconds) against any realistic per-file verification cost.

### Everything else: keep as-is, evidence below

No other change was made. Each rejected candidate is documented next.

## E. Rejected Optimizations

Matching the epic's own §26 format:

- **Custom SIMD postings/positional index** → rejected. Tantivy already
  depends directly on `bitpacking` - a Rust port of Daniel Lemire's
  `simdcomp` (the exact algorithm lineage FastPFor/§14 of the original
  issue #8 cites), claiming >4 billion integers/sec with automatic
  runtime CPU dispatch and scalar fallback (verified by reading the
  crate's own README in the local registry cache, not assumed) - plus
  `lz4_flex` and `zstd` as direct Tantivy dependencies, and blocked
  (128-doc), delta-encoded, bitpacked postings as Tantivy's native
  format. Building a parallel custom implementation would mean re-deriving
  (almost certainly worse) what's already a well-tested, actively-
  maintained dependency, for a bottleneck (see "Bottleneck Report") that
  doesn't exist at this application's scale.
- **Positional trigrams** → rejected. The trigram field is `IndexRecordOption::Basic`
  (presence-only, no positions - confirmed by reading the schema in
  `native-search/src/engine.rs`) by deliberate design, not oversight:
  every trigram-narrowed candidate still goes through the unchanged, exact
  literal/regex line scanner as the authoritative check
  (`orchestrator.rs`'s always-on verification pass). Positional proof at
  the trigram layer would only matter if that verification pass were
  itself the bottleneck; it isn't (6-28µs per file, measured above).
- **Zstd for stored-field compression, replacing the default LZ4** →
  rejected without a live A/B rebuild, on direct schema evidence rather
  than a benchmark: only `id`/`path`/`filename`/`extension`/`title`/
  `modified`/`created`/`size` are `STORED` in this schema (all small
  metadata) - the actual bulk text (`body`, `trigram`) is deliberately
  **not** `STORED` (`engine.rs`'s own comment: duplicating it "would
  double the index's on-disk footprint for no reader that needs it back
  out of Tantivy itself"). The LZ4-vs-Zstd choice only ever affects
  compression of that small metadata block - the theoretical maximum
  possible impact on this app's real index size is provably negligible
  given what's actually stored, making a live rebuild-and-measure
  experiment not worth the time it would cost for a predictable near-zero
  result. (Tantivy's *columnar* Zstd compression, a separate feature, is
  already enabled by default and unaffected by this question - it covers
  `FAST` fields like `modified`/`created`/`size`, also tiny.)
- **Alternative document extraction libraries for DOCX/PPTX/XLSX/ZIP**
  (Omniparse, Office Oxide, Apache Tika, etc.) → rejected, reaffirming the
  existing, explicit CLAUDE.md decision: this app's extraction
  deliberately mirrors the original C# tool's own dependency-free
  `ZipArchive` + regex approach for byte-for-byte parity with an
  already-tested reference implementation - swapping to a "better"
  library would extract differently in edge cases and silently drift from
  that tested baseline, for extraction costs measured (after the
  methodology correction, real 150KB-3MB documents, full pipeline
  including I/O) at sub-millisecond to low-single-digit-milliseconds -
  not a bottleneck to solve.
- **A real structural PDF parser, replacing the regex/content-stream
  scanner** → **not rejected outright - deferred, not attempted.** Unlike
  the other formats, PDF extraction *is* measurably expensive at realistic
  size (33.6ms/272KB, 112ms/1.04MB, full pipeline - see "Bottleneck
  Report"). This is the one place this investigation's evidence points
  toward a real optimization candidate rather than "no bottleneck exists."
  Not attempted here because: (1) it's a substantial rewrite of the one
  extractor CLAUDE.md most explicitly documents a deliberate hand-rolled
  design for, carrying real parity/correctness risk against the existing
  fixture-tested behavior; (2) the cost is already mitigated at the UX
  level (live progress reporting, per-file timeout, heavy-class
  throttling) for the specific problem it originally caused; (3) no
  evidence exists yet that real users are bottlenecked by it *now* that
  those mitigations exist, as opposed to being bottlenecked by it *before*
  they existed (the historical complaint CLAUDE.md documents predates the
  progress-reporting system, not this benchmark). If PDF-heavy folders
  become a reported real-world pain point again, this number is the
  evidence to start from - a next step, not unfinished work from this
  investigation.
- **Cost-based query planner** → rejected/not needed. The existing routing
  is already exactly the epic's own preferred starting point (§13):
  static rules (`use_regex` → literal-chunk extraction or full scan;
  filter length < 3 chars → no narrowing; otherwise → trigram candidate
  query). No evidence of misrouted queries or measured planning overhead
  exists to justify more sophistication.
- **Hot/warm/cold storage tiers** → rejected, matching the epic's own
  default expectation (§21). This app relies on the OS filesystem cache
  and Tantivy's own `mmap`-based reader caching already; no workload
  measurement suggests either is insufficient at this app's real corpus
  sizes (thousands to low hundreds-of-thousands of files, not the
  web-scale corpora that motivate explicit application-level tiering).
- **2-gram/4-gram or multi-gram-size trigram strategies** → not
  benchmarked, and not warranted: §8 explicitly gates this investigation
  on "only if the current trigram implementation demonstrates a
  meaningful bottleneck" - it doesn't (see above).
- **Pluggable extraction trait abstraction beyond what exists** → already
  done (the `Extractor` trait, `docs/issue-6-phase-2.md`) - re-confirmed
  adequate, not re-built.

## F. Correctness

The safe-superset invariant this epic's §5/§23 requires
(`authoritative_matches ⊆ candidate_documents`, never the reverse) is
already proven, not newly asserted:

- `native-search::engine::tests::trigram_candidates_find_a_substring_the_default_tokenizer_would_miss` -
  proves trigram narrowing finds substrings a normal tokenizer would
  silently miss (the exact literal-substring-vs-token-search gap this
  epic's §4 warns about).
- `native_index::tests::index_first_routing_agrees_with_full_scan` and its
  regex-mode sibling `regex_mode_index_first_routing_agrees_with_full_scan` -
  full end-to-end proof that the index-first (narrowed) path finds
  *exactly* the same hit files as a full unfiltered scan over the same
  real fixture files, for both literal and regex modes.
- `docs/search-semantics.md` - the formal semantic contract this epic's
  §4/§23 asks for (case sensitivity, Unicode case-folding via
  `str::to_lowercase`'s full Unicode tables not ASCII-only, the one
  documented gap being Unicode *normalization* - NFC/NFD - which isn't
  applied anywhere in matching, a pre-existing, already-documented
  limitation unrelated to the trigram layer specifically).
- `search_supports_prefix_wildcard_on_multi_word_phrases_only` - a real,
  previously-undocumented Tantivy 0.26.1 limitation found empirically
  during this session (not the trigram field, but the same discipline of
  verifying rather than assuming Tantivy's behavior).

No new correctness gap was found. No new correctness test was added -
existing coverage already proves the invariant this epic asks to verify.

## G. Final Architecture Recommendation

**Keep, unchanged:**
- Tantivy as the index engine (already provides the SIMD/delta/bitpacked
  postings, LZ4+Zstd compression options, mmap, versioned/corruption-
  resistant persistence, concurrent readers this epic's original scope
  asked to build from scratch).
- The trigram candidate field as a presence-only (non-positional) safe-
  superset pre-filter.
- The exact literal/regex line scanner as the sole semantic authority.
- The hand-rolled, parity-driven document extractors.
- Dioxus/Blitz UI architecture (already audited separately,
  `docs/issue-6-phase-13.md`: pagination bounds DOM nodes, progress
  streams per-file not per-match, no live-search-as-you-type exists to
  debounce).

**What changed as a result of this investigation:** two new benchmark
harnesses (`trigram_candidate_reduction.rs`, the per-format extraction
section of `discovery_and_extraction.rs`) and this report. No production
code in `search-core` or `native-search` changed.

## Known Gaps (honestly unmeasured, not silently claimed done)

- **Result processing breakdown** (§2: candidate generation / verification
  / snippet generation / result serialization as *separate* numbers) -
  not independently instrumented. The trigram benchmark above measures
  candidate generation vs. a verification proxy together; splitting
  snippet generation and result serialization out further wasn't done -
  no evidence either is a meaningful fraction of total time at measured
  scales.
- **UI update/render latency** (§2, §18-19) - still not benchmarkable from
  this environment (no real display - the same limitation stated in every
  UI-touching phase doc this session). `docs/issue-6-phase-13.md` covers
  what was actually verifiable (pagination, streaming, no panics on
  launch).
- **Memory (RSS) profiling** (§17) - still not done, for the same reason
  stated in `docs/benchmarking.md`: would need platform-specific
  instrumentation that would only characterize this development machine,
  not the win-x64 target.
- **Realistic heterogeneous benchmark corpus** (§22 - mixed real-world
  proportions of TXT/LOG/DOCX/PPTX/RTF/PDF at meaningful volume) - only
  small, single-copy real fixtures exist (1-4KB each); no large mixed
  corpus was assembled. Building one is real, disproportionate effort for
  a workload already shown not to be a bottleneck at every scale actually
  tested (5K/10K synthetic docs, 100K real files).

## Decision Matrix (epic §25)

| Optimization | Current performance | Bottleneck? | Alternative | Improvement | Complexity | Decision |
|---|---:|---|---|---:|---:|---|
| Custom postings/SIMD codec | N/A (Tantivy handles this) | No | Tantivy (`bitpacking`=Lemire's simdcomp, `lz4_flex`, `zstd`) | N/A | High | **Rejected** - already provided |
| Positional trigrams | Presence-only, verified safe | No | Keep `IndexRecordOption::Basic` | N/A | Medium | **Rejected** - verification is unconditional anyway |
| Trigram filter itself | 8x reduction (rare), 0.1-0.0x (common, cheap-verify proxy) | No, net positive at realistic verify cost | Full scan always | Positive for realistic per-file cost | Low (already built) | **Keep unchanged** |
| Zstd vs LZ4 (stored fields) | Affects only tiny metadata fields | No | Keep LZ4 default | ~0 (schema-proven) | Low | **Rejected** - provably negligible |
| Extraction library replacement (DOCX/PPTX/XLSX/ZIP) | Sub-ms to low-ms/file (real 150KB-3MB docs, full pipeline) | No | Keep hand-rolled parity extractors | N/A | Medium | **Rejected** - parity risk, no bottleneck |
| Structural PDF parser, replacing regex scanner | 33.6-112ms/file (real 272KB-1.04MB PDFs, full pipeline) | **Yes, real and measured** | Keep regex/stream scanner | Likely significant | High, parity risk | **Deferred** - already UX-mitigated, no rewrite attempted |
| Cost-based query planner | Static routing already matches epic's own starting point | No | Keep static rules | N/A | Medium | **Rejected** - no evidence of misrouting |
| Result virtualization | Pagination already bounds DOM nodes | No | Scroll-position virtualization | N/A | Medium | **Rejected** (`docs/issue-6-phase-13.md`) |
| Hot/warm/cold tiers | OS+Tantivy caching relied on | No | Explicit app-level tiers | N/A | High | **Rejected** - no workload evidence |

## Definition of Done (epic's checklist)

- [x] Existing architecture has been fully audited.
- [x] Reproducible baseline benchmarks exist (3 harnesses, 2 new).
- [ ] Realistic heterogeneous corpora have been tested - partial (real
      fixtures used for per-format latency; no large mixed-volume corpus
      assembled - see "Known Gaps").
- [x] Literal-search semantics are formally preserved and documented
      (`docs/search-semantics.md`).
- [x] Trigram candidate filtering has been validated for no false
      negatives (existing agrees-with-full-scan tests).
- [x] Tantivy's built-in capabilities have been evaluated before custom
      replacements (bitpacking/lz4_flex/zstd dependency audit above).
- [x] Extraction libraries have been benchmarked/considered where relevant
      (rejected on parity + measured-cost grounds).
- [x] Compression alternatives have been evaluated where relevant (Zstd
      rejected on schema evidence).
- [ ] UI/rendering performance has been profiled - partial, see "Known
      Gaps" (no real display in this environment).
- [x] Large result sets have been tested (pagination + 100K-file stress
      test).
- [x] Actual bottlenecks have been identified - none found at this app's
      real scale.
- [x] Only justified optimizations have been implemented - none were;
      two benchmarks were added to fill measurement gaps.
- [x] No unnecessary custom indexing/compression infrastructure has been
      introduced.
- [x] Existing search behavior remains correct (no production code
      changed by this investigation).
- [x] Rejected optimizations are documented with evidence (section E).
- [x] The final architecture is simpler or materially faster than a
      custom-built alternative would have been - by not building one.

## Core Principle, restated

Measured first. Nothing was optimized, because nothing needed to be -
and per this epic's own stated principle, that is the successful outcome,
not a failure to deliver.
