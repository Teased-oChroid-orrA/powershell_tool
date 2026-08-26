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
document sizes. Two corrections: (1) added `medium`/`large` real-document
tiers per format, pulled from Apache POI/Tika/PDFBox's own test-data
corpora (real documents, not synthetic filler - see
`search-core/benches/data/README.md` for exact provenance/license), and
(2) added a second benchmark function that calls the actual production
`file_reader::read_file_bytes_robust` for real file I/O on *every*
iteration (not just the first), the number issue #8 actually asked for.

```
$ cargo bench -p search-core --bench discovery_and_extraction
search-core discovery/extraction benchmark harness (issue #6 §54, issue #8 §2)
Measured on THIS machine only - not the win-x64 target hardware. See docs/benchmarking.md.

Discovery:
  291389 files/sec (5000 files across 50 dirs in 0.017s, 0 enumeration errors)

Extraction (.txt path, 2000 files, ~200 words each):
  550351 files/sec, 907.53 MB/sec (3.30 MB total in 0.004s)
  latency: median 1us, p95 1us

Parse-only extraction (in-memory, no file I/O after the initial read, 200 iterations each):
  .docx  tiny         1363 bytes, median      6us, p95      6us
  .docx  medium     144959 bytes, median    351us, p95    364us
  .docx  large     2959626 bytes, median     44us, p95     45us
  .pptx  tiny         2383 bytes, median      8us, p95      8us
  .pptx  medium     322325 bytes, median    497us, p95    518us
  .pptx  large     2282394 bytes, median    145us, p95    165us
  .xlsx  tiny         2857 bytes, median     12us, p95     13us
  .xlsx  medium     428616 bytes, median    132us, p95    134us
  .xlsx  large     2698159 bytes, median     80us, p95     84us
  .rtf   medium     172843 bytes, median    611us, p95    617us
  .rtf   large     1234084 bytes, median   3581us, p95   3787us
  .pdf   tiny          684 bytes, median     28us, p95     28us
  .pdf   medium     272008 bytes, median  33567us, p95  33984us
  .pdf   large     1040970 bytes, median 112128us, p95 113490us

Full pipeline extraction (real file I/O via file_reader::read_file_bytes_robust + parse, every iteration, 50 iterations each):
  .docx  tiny         1363 bytes, 1st-read    357us, warm-reread median     51us, p95     72us
  .docx  medium     144959 bytes, 1st-read    888us, warm-reread median    423us, p95    490us
  .docx  large     2959626 bytes, 1st-read   2392us, warm-reread median    770us, p95   1197us
  .pptx  tiny         2383 bytes, 1st-read    273us, warm-reread median     50us, p95     71us
  .pptx  medium     322325 bytes, 1st-read   1108us, warm-reread median    679us, p95   1029us
  .pptx  large     2282394 bytes, 1st-read   2197us, warm-reread median    721us, p95    932us
  .xlsx  tiny         2857 bytes, 1st-read    236us, warm-reread median     59us, p95     74us
  .xlsx  medium     428616 bytes, 1st-read   2280us, warm-reread median    257us, p95    342us
  .xlsx  large     2698159 bytes, 1st-read   3272us, warm-reread median    665us, p95    733us
  .rtf   medium     172843 bytes, 1st-read   1049us, warm-reread median    754us, p95    785us
  .rtf   large     1234084 bytes, 1st-read   6497us, warm-reread median   3831us, p95   4069us
  .pdf   tiny          684 bytes, 1st-read    355us, warm-reread median     66us, p95     85us
  .pdf   medium     272008 bytes, 1st-read  35991us, warm-reread median  33867us, p95  34320us
  .pdf   large     1040970 bytes, 1st-read 115883us, warm-reread median 112661us, p95 113309us
```

**Real finding, not noise: PDF extraction cost is genuinely significant
at realistic document sizes** - 33.6ms for a 272KB PDF, 112ms for a
1.04MB PDF (both real documents, not synthetic). This is roughly two to
three *orders of magnitude* higher than what the original tiny-fixture-only
benchmark showed (28µs, on a 684-byte fixture) - the earlier "PDF is
slightly slower but still trivial" conclusion was wrong at realistic
sizes, and this correction is why. DOCX/PPTX/XLSX/RTF all stay in the
sub-millisecond to low-single-digit-millisecond range even at 1-3MB -
PDF is the outlier, consistent with it being the one format without a
real structural parser (a regex/content-stream scanner re-scanning the
whole byte stream, not walking a parsed object graph - `docs/issue-6-phase-6.md`'s
PDF-page-awareness limitation note has the same root cause).

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
and improvement that justifies the complexity"), replacing the regex-scan
PDF extractor with a structural parser is a real, legitimate future
option now backed by a concrete number instead of a hunch - but it is a
substantial, parity-risking rewrite (see CLAUDE.md's own rationale for
why the hand-rolled approach was chosen over a real parser crate in the
first place), and this investigation stops at surfacing the number
honestly rather than making that call unilaterally. See
`docs/issue-8-status.md`'s updated bottleneck section.

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
```

No special setup beyond what `cargo build`/`cargo test` already need (see
`docs/ffi.md`). Numbers will vary machine to machine — that's expected and
is exactly why this document states the caveats above rather than
presenting the numbers as a fixed target.
