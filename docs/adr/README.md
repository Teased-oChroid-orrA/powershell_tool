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
| [012](ADR-012-general-fea-kernel.md) | General FEA kernel (`fea-core`) alongside the special-purpose solvers | Accepted (direct user direction; not part of issue #2) |
| [013](ADR-013-eigen-and-transient-methods.md) | Eigenvalue (modal / buckling), transient and adaptive-strategy methods of `fea-core` | Accepted (direct user direction) |
| [014](ADR-014-native-cad-process-model.md) | Native CAD window: process model and benches | Accepted (direct user direction) |
| [015](ADR-015-parametric-sketch-history.md) | Parametric sketch + feature history model | Accepted (direct user direction) |
| [016](ADR-016-manual-mesh-model-lifecycle-and-learning-prior.md) | Manual meshing, explicit model lifecycle, refinement zones, simulated prior for the rankers | Accepted (direct user direction) |
| [017](ADR-017-egui-wgpu-front-end-and-real-3d-modelling.md) | egui on wgpu 30 for the native window, B-rep path for real 3D | Accepted in direction (kernel open) |
| [018](ADR-018-contextual-inspector-and-adaptive-layout.md) | Contextual inspector cards instead of a permanent dock; tree-row menus; adaptive layout | Accepted (direct user direction) |
| [019](ADR-019-solid-kernel-opencascade.md) | OpenCascade (via `opencascade`) is the B-rep kernel; tetrahedral volume mesher in `fea-core` | Accepted (spike evidence); binding changed to `cadrum` by addendum 2026-10-11 |
| [020](ADR-020-failure-guidance-diagnosis-remedies-questions.md) | Failure guidance: state-based diagnosis, remedies and questions as data, ranking only behind the measured gate | Proposed (core built, UI awaits approval) |
| [021](ADR-021-thermal-studies.md) | Thermal studies (steady conduction, thermal stress) as a study of the same `Problem`, not a second schema | Accepted |
