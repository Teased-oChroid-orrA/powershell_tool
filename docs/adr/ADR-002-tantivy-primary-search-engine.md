# ADR-002: Tantivy as Primary Search Engine

Status: Proposed (baseline adopted; three items flagged EVALUATE FURTHER before Phase 3 code depends on them)

## Problem

Issue #2 treats Tantivy as the default candidate for BM25/relevance-ranked
search but requires verification before adoption ("trust but verify" —
Section 22), not documentation-claim trust.

## Evidence (verified 2026-08-23 against crates.io/GitHub/docs.rs, not training-data recall)

| # | Question | Finding | Verdict |
|---|---|---|---|
| 1 | Current version / maintenance | `tantivy` 0.26.1, released 2026-04-21. 340 open issues, 16k GitHub stars, active. | ADOPT |
| 2 | License | MIT (quickwit-oss/tantivy). | ADOPT |
| 3 | Windows support | README explicitly lists Linux/macOS/Windows. Only Windows compile-failure issue found (#69) is from 2017, closed. No open Windows-blocking issues found. | ADOPT |
| 4 | ARM64 | Not addressed in README/docs; no evidence either way. | EVALUATE FURTHER — not a blocker (app is win-x64-only today, see ADR-001/assessment doc), but don't assume ARM64 works before targeting it |
| 5 | mmap / persistence | "Mmap directory" listed as a feature; a `RamDirectory`/custom `Directory` trait appears to exist for non-mmap use but was not independently confirmed. No admin/elevated-privilege requirement found for user-file mmap on Windows. | ADOPT, with the non-mmap `Directory` claim to be confirmed directly before citing as fact in implementation |
| 6 | Incremental indexing | Confirmed feature. **Documents are immutable** — updates are delete-by-term + re-add, not in-place mutation. | ADOPT, with this caveat carried into the (not-yet-written) ADR-008 incremental-indexing-strategy decision |
| 7 | Concurrency | "Multithreaded indexing" confirmed; single-writer `IndexWriter` model implied by design, not independently re-verified in source. | EVALUATE FURTHER before Phase 3 code assumes a specific concurrent-writer guarantee |
| 8 | Query capabilities | Confirmed present in 0.26.1 docs: `BooleanQuery`, `TermQuery`, `TermSetQuery`, `PhraseQuery`, `PhrasePrefixQuery`, `RegexPhraseQuery`, `RangeQuery`, `FastFieldRangeQuery`, `FuzzyTermQuery`, `RegexQuery`, `ExistsQuery`, `BoostQuery`, `DisjunctionMaxQuery`, `MoreLikeThisQuery`. BM25 scoring confirmed ("same as Lucene" per README). Faceting confirmed. Highlighting/snippets not independently confirmed this pass. | ADOPT for the confirmed set; verify snippet/highlighting API directly before Phase 3 relies on it |
| 9 | Schema evolution | Unresolved as of the 2019 "Roadmap to 1.0" issue (#638), which lists schema-modification strategy as a pre-1.0 blocker. Current (0.26.x) status not re-confirmed. | EVALUATE FURTHER — treat schema as fixed-at-index-creation until proven otherwise |
| 10 | Corruption/crash recovery | No documentation found. The same 2019 roadmap issue lists "file checksums and footers" as an unresolved pre-1.0 requirement at the time. | **EVALUATE FURTHER / open risk** — directly relevant to Definition of Done's "index survives application restart"; needs a direct source-level check before shipping |
| 11 | Memory usage | No concrete numbers found in docs. | EVALUATE FURTHER — needs the Section 13 benchmark harness, not documentation |
| 12 | Offline/vendored build | Plain `cargo test` build step, no C compiler/cmake/protoc/codegen mentioned. Strong signal of a pure-Rust build. Full transitive dependency tree (e.g. compression crates that may have C backends) not exhaustively audited. | EVALUATE FURTHER — run `cargo vendor` against a real `Cargo.lock` and inspect it before claiming a zero-C-toolchain build in the offline-build doc |
| 13 | Naming/forks | Canonical repo: `quickwit-oss/tantivy`. No rename/archive signal. `tantivy-py` (official Python bindings) and several personal forks exist but are not competing projects. | ADOPT (no confusion risk) |

**Surprise worth carrying forward:** Tantivy is still pre-1.0 (0.26.x) seven
years after the 2019 roadmap first listed schema-change strategy and
checksums/footers as 1.0 blockers. This doesn't disqualify it — active
maintenance and a real production user base (Quickwit) offset a raw version
number — but items 9 and 10 above should be re-verified directly against
source/changelog before Phase 3 code depends on either schema stability or
corruption recovery, rather than assumed resolved by the passage of time.

## Decision

Adopt Tantivy as the primary/baseline search engine for the vertical slice
(Section 25, Phase 3), per the epic's own default stance ("Tantivy should be
the baseline, not a religion"). Proceed to a minimal vertical slice
(TXT/MD → Tantivy → search) with items 4, 7, 9, 10, 11, 12 above tracked as
pre-production-hardening follow-ups, not blockers to starting the slice.

## Consequences

- Immutable-document model (item 6) means the incremental-indexing design
  (future ADR-008) must plan around delete+reinsert per changed file, not a
  cheaper in-place update.
- Corruption-recovery gap (item 10) means the vertical slice should not yet
  claim "index survives application restart" as done (Definition of Done
  checklist item) until re-verified — track separately, don't mark complete
  prematurely.
- No custom inverted index, BM25, tokenizer, or query parser gets built —
  directly satisfies Section 23's prohibition list for these components.

## Rejected alternatives

None evaluated in depth this pass — Tantivy was the only primary-engine
candidate the epic named, and evidence didn't surface a disqualifying issue.
Revisit only if Phase 3 benchmarking or the item-10 corruption-recovery
follow-up surfaces a real blocker.
