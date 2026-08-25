# native_search benchmarking (issue #2 Section 13)

## Scope of what exists today

`native-search/benches/indexing_and_search.rs` is a minimal, manual-timing
harness (`cargo bench`, `harness = false` — not criterion, per Section 23's
"don't overengineer"). It measures indexing throughput and search latency
against a single synthetic corpus.

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

## Re-running this

```
cd native-search
cargo bench --bench indexing_and_search
```

No special setup beyond what `cargo build`/`cargo test` already need (see
`docs/ffi.md`). Numbers will vary machine to machine — that's expected and
is exactly why this document states the caveats above rather than
presenting the numbers as a fixed target.
