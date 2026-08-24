# ADR-002: Tantivy as Primary Search Engine

Status: Accepted (items 9, 10, 12 re-verified directly against source and closed — see "Follow-up verification" below; items 4, 7, 11 remain open, non-blocking)

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
number — but items 9 and 10 above needed re-verification directly against
source before Phase 3 code depended on either schema stability or
corruption recovery, rather than assuming they were resolved by the passage
of time. That re-verification is done — see below.

## Follow-up verification (2026-08-24, direct source inspection)

Items 9, 10, and 12 were re-checked against the actual `tantivy-0.26.1`
source on disk (`~/.cargo/registry/src/.../tantivy-0.26.1/`), not
documentation or the 2019 roadmap issue, and a real local `cargo vendor` +
`cargo build --offline` run — not assumed from a build-step description.

| # | 2026-08-23 status | Re-verified finding | Updated verdict |
|---|---|---|---|
| 9 | EVALUATE FURTHER | **Confirmed unresolved, and now mitigated in our code, not Tantivy's.** `Index::open()` reads whatever schema is on disk with no compatibility check of its own — a schema change in a future `native-search` version would silently load an incompatible index and fail much later, confusingly, at the first `get_field()` lookup. Fixed directly in `engine::NativeSearchEngine::open_or_create`: after opening an existing index, its schema is compared (`Schema: Eq + PartialEq`) against `build_schema()`'s current schema; a mismatch returns `NsStatus::CorruptIndex` naming both schemas, immediately and clearly. Proven by a real test (`opening_index_with_mismatched_schema_is_corrupt_index_not_panic`), not just asserted in a comment. There is still no in-place migration path — a schema change means deleting the index directory and rebuilding from scratch — acceptable for a regenerable local search index, not acceptable if this were a system of record. | **RESOLVED** (mitigated, not a Tantivy gap we're exposed to) |
| 10 | EVALUATE FURTHER / open risk | **Confirmed solid, better than assumed.** `src/directory/footer.rs` and `src/directory/managed_directory.rs` show every managed file gets a footer (magic number `1337`, version, CRC32 via `crc32fast`) appended on write. `ManagedDirectory::open_read` calls `Footer::extract_footer` + `footer.is_compatible()` on **every single file open**, automatically — a truncated file (too short to contain a valid footer) or a version/magic-number mismatch is caught immediately as a typed `OpenReadError`, not a silent misread. Full CRC validation is available via the separate `validate_checksum()` method (not run on every read, for performance — an opt-in integrity check, not a blocker). This directly satisfies the Definition of Done's "index survives application restart" and "corrupt/unreadable documents don't terminate indexing" more solidly than the 2019 roadmap issue suggested was true at the time. | **RESOLVED** |
| 12 | EVALUATE FURTHER | **Not a pure-Rust build — confirmed, and confirmed to still work offline anyway.** `cargo vendor` against this crate's real `Cargo.lock` pulls in `zstd-sys` (C source of zstd, built via the `cc` crate — a genuine C-compiler dependency at build time, not just a transitive Rust crate). This means **Rust alone is not sufficient to build native-search** — a C toolchain must be present too. On the target CI runner (`windows-latest`) this is already satisfied in practice (the existing `.github/workflows/build.yml` run on 2026-08-24 built `native-search` successfully via MSVC, and `microsoft/setup-msbuild`/the runner's bundled Visual Studio installation provides `cl.exe`) — but this ADR was wrong to imply a pure-Rust build in the original pass, and any future minimal-build-environment work (a from-scratch offline installer, a stripped-down CI image) must provision a C compiler alongside Rust, not just Rust. Offline-ness itself is fine: a local `cargo vendor` + `cargo build --offline` against the vendored sources succeeded with zero network access (verified directly, not assumed) — see `docs/offline-build.md`. | **RESOLVED** (revised: needs Rust + a C toolchain, both available on the actual target CI runner; not pure-Rust as first claimed) |

## Decision

Adopt Tantivy as the primary/baseline search engine for the vertical slice
(Section 25, Phase 3), per the epic's own default stance ("Tantivy should be
the baseline, not a religion"). Proceed to a minimal vertical slice
(TXT/MD → Tantivy → search) with items 4, 7, 9, 10, 11, 12 above tracked as
pre-production-hardening follow-ups, not blockers to starting the slice.

## Consequences

- Immutable-document model (item 6) means the incremental-indexing design
  (ADR-008) plans around delete+reinsert per changed file, not a cheaper
  in-place update.
- Corruption-recovery (item 10, now resolved) directly backs the Definition
  of Done's "index survives application restart" claim with source-level
  evidence, not an assumption.
- Schema-mismatch detection (item 9, now resolved) is `native-search`'s own
  code, not a Tantivy feature — a real safeguard that has to be maintained
  going forward whenever `build_schema()` changes.
- The C-toolchain dependency (item 12) is a real build-environment
  requirement to carry into any future CI/offline-installer work, not
  optional — see `docs/offline-build.md`.
- No custom inverted index, BM25, tokenizer, or query parser gets built —
  directly satisfies Section 23's prohibition list for these components.

## Rejected alternatives

None evaluated in depth this pass — Tantivy was the only primary-engine
candidate the epic named, and evidence didn't surface a disqualifying issue.
Revisit only if Phase 3 benchmarking or the item-10 corruption-recovery
follow-up surfaces a real blocker.
