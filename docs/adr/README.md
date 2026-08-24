# Architecture Decision Records — issue #2

Index of ADRs for the native offline search engine epic (issue #2), in the
order the epic's own Section 21 names them. See `docs/native-search-assessment.md`
for the Phase 1 repo reconnaissance these build on, `docs/ffi.md` for the
implementation contract, `docs/benchmarking.md` and `docs/offline-build.md`
for Sections 13 and 15.

| ADR | Title | Status |
|---|---|---|
| [001](ADR-001-rust-native-search-boundary.md) | Rust Native Search Boundary | Accepted |
| [002](ADR-002-tantivy-primary-search-engine.md) | Tantivy as Primary Search Engine | Accepted |
| [003](ADR-003-unified-extraction-architecture.md) | Unified Extraction Architecture | Accepted (deferred: reuse existing C# extraction) |
| [004](ADR-004-fm-index-evaluation.md) | FM-Index Evaluation | Rejected (no incremental update support) |
| [005](ADR-005-suffix-array-evaluation.md) | Suffix Array Evaluation | Rejected (no incremental update support) |
| [006](ADR-006-bioinformatics-indexing-library-evaluation.md) | Bioinformatics Indexing Library Evaluation | Rejected (no incremental update support) |
| [007](ADR-007-index-persistence-location.md) | Index Persistence Location | Superseded by 011 |
| [008](ADR-008-incremental-indexing-strategy.md) | Incremental Indexing Strategy | Accepted |
| [009](ADR-009-ffi-serialization-strategy.md) | FFI Serialization Strategy | Accepted (JSON) |
| [010](ADR-010-multi-index-vs-tantivy-only-architecture.md) | Multi-Index vs. Tantivy-Only Architecture | Accepted (Tantivy-only) |
| [011](ADR-011-in-folder-index-location.md) | In-Folder Index Location | Accepted (`<SearchPath>\.native-search-index\`, by direct user direction) |
