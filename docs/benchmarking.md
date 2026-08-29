# Benchmarking (issue #2 Section 13, issue #6 §54-56)

## Scope of what exists today

`native-search/benches/indexing_and_search.rs` and
`search-core/benches/discovery_and_extraction.rs` are minimal, manual-timing
harnesses (`cargo bench`, `harness = false` — not criterion, per Section 23's
"don't overengineer"). Together they cover 4 of epic #6 §54's 6 categories:
indexing throughput, search latency, directory-discovery throughput, and
plain-text extraction throughput. Memory (peak/steady RSS) and UI (result
update latency, scroll performance) are not covered - see "What's
deliberately not benchmarked" below for why, honestly, rather than a
fabricated number.

**This is not** the full tiered benchmark suite Section 13 describes (10k /
100k / 1M file corpora, mixed real file types, Tantivy-vs-FM-index-vs-
suffix-array comparisons). That fuller suite was not built, for two honest
reasons rather than an oversight:

1. **The comparison side is moot.** ADR-004/005/006 already concluded every
   specialized-index candidate is structurally disqualified (no incremental
   update support) — there's nothing to comparison-benchmark against
   Tantivy, because nothing else was adopted (ADR-010). Building a
   benchmark harness to compare Tantivy against candidates already
   rejected on functional grounds would be measuring a question that's
   already answered.
2. **A 1M-file real-corpus benchmark needs infrastructure this environment
   doesn't have** (no Windows machine, no large representative document
   set, and Section 13's own guidance not to draw conclusions from
   synthetic data alone would apply just as much to a fabricated 1M-file
   synthetic run as to a smaller one). Rather than produce numbers that
   look authoritative but aren't representative, this stays scoped to a
   real, honestly-caveated sanity check.

## What was actually measured (2026-08-24, this development machine)

```
$ cargo bench --bench indexing_and_search
native-search benchmark harness (issue #2 Section 13)
Measured on THIS machine only - not the win-x64 target hardware.
Corpus: 5000 synthetic documents, ~40 words each.

Indexing:
  1159936 docs/sec (5000 docs in 0.004s)
  391.66 MB/sec (1.69 MB total)
  commit: 0.167s

Search                 "torque": median    92us, p95   107us (200 iterations)
Search "\"corrosion inspection\"": median    35us, p95    40us (200 iterations)
Search         "extension:.txt": median    65us, p95    66us (200 iterations)
Search       "aog OR workorder": median   127us, p95   130us (200 iterations)
```

Real, measured, not fabricated - but read the caveats below before citing
any of these numbers elsewhere.

## Caveats (read before citing these numbers anywhere)

- **Wrong hardware.** This ran on the development machine (Apple Silicon,
  macOS), not the win-x64 target hardware this app actually ships on. CPU
  architecture, disk (SSD vs. the target user's actual disk), and OS-level
  I/O scheduling all differ. Treat these as "does this look pathological,"
  not a performance SLA for the shipped app.
- **The 1.16M docs/sec indexing number is measuring the in-memory buffering
  step, not full durability.** `index_document` enqueues an add/delete
  against Tantivy's `IndexWriter` buffer; actual segment serialization to
  disk happens at `commit()`, measured separately (167ms for this corpus).
  Don't read "indexing" and "commit" as two independent costs you can
  ignore one of — a real workload pays both, and the commit cost is where
  disk I/O actually happens.
- **Small corpus, single segment.** 5,000 tiny synthetic documents (~40
  words each, ~1.7MB total) is nowhere near Section 13's 10k/100k/1M
  tiers. Search latency in particular is expected to change shape (not
  just scale linearly) once an index spans many segments, since
  `CancellableCollector`'s per-segment cancellation check (ADR from the
  cancellation work) only matters once there's more than one segment to
  check between.
- **Synthetic text, not real documents.** The corpus is generated from a
  small fixed vocabulary (see `VOCAB` in the harness) for determinism and
  to avoid adding a `rand` dependency just for a benchmark — it does not
  reflect the term-frequency distribution, document-length variance, or
  duplicate-content patterns of real PDFs/DOCX/logs this app actually
  searches.

## Discovery and extraction (2026-08-26, this development machine)

**Methodology correction:** the per-format section below originally ran
against 1-4KB correctness fixtures only, and only measured in-memory
parse cost (one disk read, then 500 repeated calls against the same
bytes - no I/O on any iteration after the first). That was mislabeled as
characterizing "extraction" when it only isolated parser CPU cost at toy
document sizes. Two corrections: (1) added `medium`/`large`/`xlarge`
real-document tiers per format, pulled from Apache POI/Tika/PDFBox's own
test-data corpora plus sample-files.com/arXiv.org for the ~10MB+ tier
(real documents, not synthetic filler - see
`search-core/benches/data/README.md` for exact provenance/license), and
(2) added a second benchmark function that calls the actual production
`file_reader::read_file_bytes_robust` for real file I/O on *every*
iteration (not just the first), the number issue #8 actually asked for.

**Fix applied since the numbers below were first captured:** the
methodology correction surfaced a real bottleneck in `extract_pdf_lines`
(profiled to `stream_re`, a bounded-repetition regex, at 74-93% of PDF
extraction cost - see "PDF extraction fix" below). That fix is already
reflected in the numbers in this section; the "roughly two to three
orders of magnitude" discrepancy noted below is against the *pre-fix*
correction, not the original toy-fixture number twice over.

```
$ cargo bench -p search-core --bench discovery_and_extraction
search-core discovery/extraction benchmark harness (issue #6 §54, issue #8 §2)
Measured on THIS machine only - not the win-x64 target hardware. See docs/benchmarking.md.

Discovery:
  283923 files/sec (5000 files across 50 dirs in 0.018s, 0 enumeration errors)

Extraction (.txt path, 2000 files, ~200 words each):
  547402 files/sec, 902.67 MB/sec (3.30 MB total in 0.004s)
  latency: median 1us, p95 1us

Parse-only extraction (in-memory, no file I/O after the initial read, 200 iterations each):
  .docx  tiny         1363 bytes, median      6us, p95      6us
  .docx  medium     144959 bytes, median    361us, p95    374us
  .docx  large     2959626 bytes, median     44us, p95     46us
  .docx  xlarge   11317142 bytes, median    626us, p95    687us
  .pptx  tiny         2383 bytes, median      8us, p95      8us
  .pptx  medium     322325 bytes, median    493us, p95    504us
  .pptx  large     2282394 bytes, median    148us, p95    168us
  .xlsx  tiny         2857 bytes, median     12us, p95     13us
  .xlsx  medium     428616 bytes, median    131us, p95    133us
  .xlsx  large     2698159 bytes, median     80us, p95     82us
  .xlsx  xlarge (rejected by zip-bomb guard)  12364136 bytes, median      7us, p95      7us
  .rtf   medium     172843 bytes, median    666us, p95    679us
  .rtf   large     1234084 bytes, median   3495us, p95   3729us
  .pdf   tiny          684 bytes, median      9us, p95     10us
  .pdf   medium     272008 bytes, median  13874us, p95  15790us
  .pdf   large     1040970 bytes, median  32552us, p95  32909us
  .pdf   xlarge    5853703 bytes, median 123243us, p95 126394us
  .pdf   xlarge-scanned (image-only, no text)  38589556 bytes, median 225855us, p95 227765us

Full pipeline extraction (real file I/O via file_reader::read_file_bytes_robust + parse, every iteration, 50 iterations each):
  .docx  tiny         1363 bytes, 1st-read   1025us, warm-reread median     51us, p95     79us
  .docx  medium     144959 bytes, 1st-read    914us, warm-reread median    441us, p95    564us
  .docx  large     2959626 bytes, 1st-read   2529us, warm-reread median    634us, p95    820us
  .docx  xlarge   11317142 bytes, 1st-read   6815us, warm-reread median   3028us, p95   3502us
  .pptx  tiny         2383 bytes, 1st-read    904us, warm-reread median     53us, p95     65us
  .pptx  medium     322325 bytes, 1st-read   1106us, warm-reread median    580us, p95    860us
  .pptx  large     2282394 bytes, 1st-read   2132us, warm-reread median    700us, p95   1043us
  .xlsx  tiny         2857 bytes, 1st-read    338us, warm-reread median     59us, p95     67us
  .xlsx  medium     428616 bytes, 1st-read   1924us, warm-reread median    228us, p95    277us
  .xlsx  large     2698159 bytes, 1st-read   4359us, warm-reread median    830us, p95    972us
  .xlsx  xlarge (rejected by zip-bomb guard)  12364136 bytes, 1st-read  12661us, warm-reread median   2349us, p95   2710us
  .rtf   medium     172843 bytes, 1st-read   1797us, warm-reread median    750us, p95    776us
  .rtf   large     1234084 bytes, 1st-read   5581us, warm-reread median   3854us, p95   4034us
  .pdf   tiny          684 bytes, 1st-read   1028us, warm-reread median     53us, p95    141us
  .pdf   medium     272008 bytes, 1st-read  16476us, warm-reread median  13993us, p95  14292us
  .pdf   large     1040970 bytes, 1st-read  36284us, warm-reread median  33018us, p95  33396us
  .pdf   xlarge    5853703 bytes, 1st-read 131204us, warm-reread median 125175us, p95 127113us
  .pdf   xlarge-scanned (image-only, no text)  38589556 bytes, 1st-read 246445us, warm-reread median 233875us, p95 236725us
```

**PDF extraction fix (2026-08-26): `find_stream_blocks` replaces
`stream_re`.** Profiling (`cargo test -p search-core --release --
--ignored --nocapture profile_pdf_extraction_phases_on_real_documents`)
found the original `stream_re` regex (matching `stream ... endstream`
blocks with a bounded `.{0,400}?` header lookback) was 74-93% of total
PDF extraction time - not the inflate/text-extraction work that's
inherent to the format, but the *block-finding scan itself*. Root cause:
Rust's `regex` crate compiles bounded repetition into an NFA that tracks
a counter up to 400, which is far more expensive per byte than a simple
scan even though it stays linear (not quadratic) in file size.
`find_stream_blocks` (`search-core/src/extraction.rs`) replaces it with a
hand-rolled two-position scan (`anchor`/`probe`) that reproduces the same
lazy-match "absorb a failed candidate into the header" semantics via
plain substring search, with char-count-based (not byte-count) header
truncation to stay faithful to the original `.{0,400}?`'s
Unicode-scalar-counting semantics. Proven behaviorally identical via
differential testing against the original regex kept as a
`#[cfg(test)]`-only oracle (`old_regex_blocks`) - exact match required on
every existing correctness fixture, 7 hand-built adversarial edge cases
(lazy-match boundary, multi-byte-char header cutoff, nested/overlapping
candidates, etc.), and the real `medium.pdf`/`large.pdf`/
`xlarge-scanned.pdf` fixtures, including the full ~38.6MB file
(`find_stream_blocks_matches_the_original_regex_on_xlarge_pdf`,
`#[ignore]`d - run with `--release -- --ignored` since the *old* regex
being used as the comparison oracle is itself slow at this size).

Before/after (same real files, same benchmark):

| File | Before | After | Speedup |
|---|---|---|---|
| `medium.pdf` (272KB) | 33.6ms | 13.9ms | 2.4x |
| `large.pdf` (1.04MB) | 112ms | 32.6ms | 3.4x |
| `xlarge-scanned.pdf` (38.6MB, the file profiling was done against) | ~3.08s | 229ms | 13.5x |

**Real finding, not noise: PDF extraction cost is still genuinely
significant at realistic document sizes even after the fix** - 13.9ms
for a 272KB PDF, 32.6ms for a 1.04MB PDF, ~123ms for a real 5.85MB
text-heavy PDF (both real documents, not synthetic). This remains one to
two *orders of magnitude* higher than what the original tiny-fixture-only
benchmark showed (28µs, on a 684-byte fixture) even after the
`find_stream_blocks` fix - the earlier "PDF is slightly slower but still
trivial" conclusion was wrong at realistic sizes, and this correction is
why, though the fix substantially narrowed the gap. DOCX/PPTX/XLSX/RTF
all stay in the sub-millisecond to low-single-digit-millisecond range
even at 1-3MB - PDF is still the outlier, consistent with it being the
one format without a real structural parser (a regex/content-stream
scanner re-scanning the whole byte stream, not walking a parsed object
graph - `docs/issue-6-phase-6.md`'s PDF-page-awareness limitation note
has the same root cause). The `xlarge-scanned.pdf`/`xlarge.docx`/
`xlarge-recordheavy.xlsx` rows above are real, legitimately-sourced files,
not synthetic stress fixtures - see `search-core/benches/data/README.md`
for exactly why each is included and what it demonstrates (one genuine
large document, one image-only PDF correctly returning no text since
this extractor has no OCR, one pathological-compression XLSX correctly
rejected by the zip-bomb guard).

**This is not a newly-discovered, unaddressed bottleneck** - it's
measured confirmation of a characteristic this project has known about
and specifically designed around since before this benchmark existed.
CLAUDE.md's own "Live progress reporting is a hard requirement" section
states plainly: "This project exists partly because of a specific,
explicit complaint: PDF processing in the original PowerShell tool would
go silent for many seconds with no way to tell 'still working' from
'actually stuck.'" That's exactly this cost, already measured
qualitatively by a real user before this benchmark ever quantified it -
and the mitigations already exist: `extract_pdf_lines`'s progress
callback fires ~every 150ms during extraction (not just on completion),
the per-file timeout bounds worst-case wait, and the heavy/light
resource-class throttle (`docs/issue-6-phase-8.md`) keeps several slow
PDFs from starving light-format files in the same run. Per this epic's
own §3/§24 ("do not implement an optimization simply because it is
theoretically faster... every architectural change must be justified by
a measurable bottleneck, comparison against the existing implementation,
and improvement that justifies the complexity"), the `find_stream_blocks`
fix above is exactly that: a measured bottleneck (74-93% of cost),
proven via differential testing, that achieved up to 13.5x real speedup
without touching the extraction *algorithm's* output at all (byte-for-byte
identical stream blocks, proven, not assumed). Replacing the regex-scan
PDF extractor with a full structural parser remains a real, legitimate
future option now backed by concrete post-fix numbers instead of a
hunch - but it is still a substantial, parity-risking rewrite (see
CLAUDE.md's own rationale for why the hand-rolled approach was chosen
over a real parser crate in the first place), and this investigation
stops at surfacing the number honestly rather than making that call
unilaterally. See `docs/issue-8-status.md`'s updated bottleneck section.

## Concurrent / mixed-format extraction (2026-08-26, this development machine)

Follow-up to the PDF finding above: does the same bottleneck show up
under real concurrent/mixed-format load via the actual production
`orchestrator::run` (not an isolated per-format function call), and what
happens with several genuinely large (~10MB+) real files competing for
the parallel throttle at once?

```
$ cargo bench -p search-core --bench concurrent_extraction
search-core concurrent/mixed-corpus extraction benchmark
Measured on THIS machine only - not the win-x64 target hardware. See docs/benchmarking.md.

Same-type (PDF only, 6x medium + 4x large): 10 files, 5.8 MB total
  sequential:   275.3ms (10 searched, 0 read errors)
  parallel:      79.7ms (10 searched, 0 read errors) - 3.5x vs. sequential

Same-type (XLSX only, 6x medium + 4x large): 10 files, 13.4 MB total
  sequential:    34.5ms (10 searched, 0 read errors)
  parallel:       2.4ms (10 searched, 0 read errors) - 14.5x vs. sequential

Mixed format (one of each: docx/pptx/xlsx/rtf/pdf, medium+large): 10 files, 11.6 MB total
  sequential:    78.6ms (10 searched, 0 read errors)
  parallel:      38.7ms (10 searched, 0 read errors) - 2.0x vs. sequential

~10MB+ files, mixed format (2x xlarge.pdf real-text + xlarge.docx + xlarge-scanned.pdf[no text, expected] + xlarge-recordheavy.xlsx[rejected, expected]): 5 files, 74.0 MB total
  sequential:   558.6ms (3 searched, 2 read errors)
  parallel:     246.2ms (3 searched, 2 read errors) - 2.3x vs. sequential
```

All four scenarios parallelize (2.0x-14.5x depending on format mix - PDF
and mixed-format scale less than XLSX because PDF's per-file cost is CPU-
bound on a single large regex/scan pass, giving the throttle less
opportunity to overlap I/O and compute across files of that type
specifically). The "3 searched, 2 read errors" in the xlarge scenario is
expected, not a regression: 3 real files (`xlarge.docx`, 2x `xlarge.pdf`)
extract genuine text; the other 2 (`xlarge-scanned.pdf`,
`xlarge-recordheavy.xlsx`) are the deliberately-included pathological
edge cases from `benches/data/README.md` - correctly rejected (no OCR /
zip-bomb guard), not crashed or hung. No panics, no timeouts, no silent
data loss across any scenario - the orchestrator's error-isolation
philosophy ("a bad file must never stop indexing") holds under real
concurrent contention with real large files, which is what this scenario
exists to prove.

Same "wrong hardware" caveat as everywhere else applies. The "1st-read"
vs. "warm-reread" split is the closest honest proxy for cold-vs-warm this
benchmark can produce without OS-level cache-dropping privileges (macOS's
`purge`/Linux's `/proc/sys/vm/drop_caches` both need elevated rights this
benchmark shouldn't require) - it's a real signal (e.g. XLSX medium: 2280µs
first read vs. 257µs warm reread, a genuine cache effect), not a claimed
true-cold measurement. `.txt`/`.log` still dominate typical searched
folders far more than `.docx`/`.pptx`/`.pdf` do in practice, so the
plain-text path remains the one most real corpora spend most of their
time on - but a folder containing many/large PDFs specifically will feel
that cost, which is exactly what the existing progress-reporting design
already answers for.

## Trigram candidate-set reduction (2026-08-26, this development machine)

```
$ cargo bench -p native-search --bench trigram_candidate_reduction
Tier                                  total candidates   cand.%  full-scan(us)   narrowed(us)    speedup
"the" (~100% of docs)                 10000      10000   100.0%            273          12782       0.0x
"corrosion" (~20% of docs)            10000       2000    20.0%            230           2575       0.1x
"zqx9k7f2" (rare, 0.05% of docs)      10000          5     0.1%            225             28       8.0x
"ab" (below trigram threshold)        10000      10000   100.0%            220            220       1.0x
```

Added for issue #8 §7. Full discussion (why the "full-scan" baseline here
is deliberately the *cheapest possible* verification cost, and why that
makes common/medium terms look like a net loss here despite being a real
win in production, where verification means real file I/O + parsing, not
an in-memory string check) is in `docs/issue-8-status.md`'s "Optimization
Results" section - read that before citing the 0.0x/0.1x numbers as
evidence the trigram filter isn't worthwhile; it deliberately isolates
overhead from reduction, and is not itself a production-shaped measurement.

## Result-processing phase breakdown (2026-08-29, this development machine)

Closes issue #8's "result processing breakdown" Known Gap: candidate
generation, verification, snippet generation, and result serialization,
timed as four *separate* phases against 17 real fixture files (198,518
real extracted lines total) - not the tiny in-memory `str::contains`
proxy the trigram benchmark above uses for its "full-scan" baseline. Full
phase-boundary reasoning and the differential-testing proof that the
snippet-generation reimplementation matches production
(`report::build_html_report`'s real output) are in
`docs/issue-8-status.md`'s "Result processing breakdown" section - this
is just the raw numbers.

```
$ cargo bench -p search-core --bench result_processing_phases
Filter terms picked dynamically from real fixture content (not hardcoded):
  common = "this"   rare = "transformed"

Phase 1 - candidate generation (real-content corpus, 3320 documents):
  common "this": 685us, 399 of 3320 documents (12.0%)
  rare   "transformed": 106us, 4 of 3320 documents (0.1%)

Phase 2 - verification (real extracted lines, real per-file cost):
  "this" (common): 17 file(s), total 13869us, per-file min 0us / median 1us / max 8893us
  "transformed" (rare): 17 file(s), total 12453us, per-file min 0us / median 0us / max 7564us

Phase 3 - snippet generation (highlight-span computation per real hit line):
  differential check: 471 real hit line(s) confirmed byte-for-byte identical to production output
  highlight per line: 471 file(s), total 60326us, per-file min 126us / median 127us / max 177us

Phase 4 - result serialization:
  build_export_rows: 73us (471 row(s))
  write_csv:   935us
  write_json:  962us
  write_jsonl: 692us
  write_html_report (includes phase 3's snippet generation): 158457us, 2695001 bytes written
```

**One real, per-unit disproportion found:** snippet generation
(`report.rs`'s `highlight_matches`) recompiles a `FancyRegex` per matched
filter on *every* hit line it's called on, with no equivalent of
`matching.rs`'s `CompiledMatchState` (compiled once per run, reused
everywhere). Measured at ~127us/line, essentially flat regardless of line
content - roughly 1,800x verification's real per-line cost (~70ns/line
average across the same 198,518 real lines). Still small in absolute
terms at this benchmark's scale (60ms for 471 real hits), so this is
reported as an evidence-backed recommendation for a future investigation
(cache a compiled regex per filter across a report's hit lines), not
implemented here - this investigation's scope was measurement-only. The
exact "rare" term printed varies run to run (picked dynamically from
whichever real word happens to be file-unique in the fixture set that
run, to avoid a hardcoded term silently going stale if fixtures change) -
the cost *shape* (flat ~127us/line for highlighting, bimodal per-file cost
for verification dominated by the largest real documents) is what's
significant, not the specific word.

## What's deliberately not benchmarked

- **Memory (peak/steady RSS).** Would need a platform-specific RSS query
  (`/proc/self/status` on Linux, `GetProcessMemoryInfo` on Windows - the
  actual target platform, not this development machine) wired into the
  harness - real work, and the resulting number would still only describe
  this machine's allocator behavior, not the target Windows machine's.
  `search-core`'s architecture (Tantivy's disk-backed index, streaming
  HTML/CSV/JSON export, bounded per-resource-class concurrency) is what
  actually keeps memory bounded - see this doc's sibling phase docs
  (`issue-6-phase-3.md` streaming export, `issue-6-phase-8.md`
  concurrency) for the design reasoning, not a fabricated RSS figure.
- **UI (result update latency, scroll performance).** Needs a real running
  `dioxus-native`/Blitz window - not something a `cargo bench` binary can
  measure. `docs/epic-ui-performance-and-design.md` (if present) or
  manual `dx serve`/`cargo run -p app` verification is the actual
  verification path for this, same as every other UI-only concern in this
  repo (this project's own testing requirements already say the `app`
  crate "cannot be fully verified this way").

## Re-running this

```
cd native-search && cargo bench --bench indexing_and_search
cd native-search && cargo bench --bench trigram_candidate_reduction
cd search-core && cargo bench --bench discovery_and_extraction
cd search-core && cargo bench --bench concurrent_extraction
cd search-core && cargo bench --bench regex_query_shapes_at_scale
cd search-core && cargo bench --bench result_processing_phases
```

Fixtures for `discovery_and_extraction`/`concurrent_extraction` live in
`search-core/benches/data/`, committed to the repo like the rest of that
directory (see its `README.md` for exact source/provenance); both
benchmarks skip themselves with a message if the fixtures aren't present
rather than failing. `regex_query_shapes_at_scale` (issue #9's Level-3
justification check - see `docs/issue-9-status.md`) generates its own
110,000-file, 2,000-directory synthetic corpus under the OS temp dir at
run time and deletes it afterward; no fixture setup needed, but expect it
to take ~1-2 minutes (corpus generation plus ten full 110,000-file scans).

No special setup beyond what `cargo build`/`cargo test` already need (see
`docs/ffi.md`). Numbers will vary machine to machine — that's expected and
is exactly why this document states the caveats above rather than
presenting the numbers as a fixed target.
