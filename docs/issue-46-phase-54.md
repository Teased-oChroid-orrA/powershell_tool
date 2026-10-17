# Issue 46 — Phase 54: bounded p-curve endpoint candidate discovery

PR #81 (Phase 53) merged at `58c054c`. `step_pcurve_candidates::discover` turns the Phase 53 read-only probe into a library API.

For a caller-selected finite search interval of one source p-curve it validates exact-document edge/p-curve/face association (shared helper), runs the bounded world-trace distance search (`step_pcurve_distance`) once per source topological vertex, and proposes the sorted candidate parameters plus the order that maps them to topological start/end. The proposal is then rechecked by `step_pcurve_endpoints::verify` (unchanged 1e-7 mm tolerance, outward source unit conversion). Search queries use the midpoint of the outward vertex box; the recheck, not the search, carries certification.

Closed edges (one topological vertex, or identical candidates) report `Proposal::Coincident`; no empty or inferred arc is built. A geometric refusal of a proposal is returned as data (`recheck: Err`); `Limit` and `Cancelled` always propagate as errors. Distance-search enclosure budget is cumulative over both vertices; the recheck keeps its own budget. Progress is monotone (`search_enclosures`, `searched_vertices`, `recheck_enclosures`); cancellation at start, mid-search and final is atomic.

Not claimed: parameter uniqueness, correct oriented arc among the two arcs of a closed circle trace, winding/wrap, master semantics, trim or body validity.

Evidence (macOS, feature-off, unchanged tolerances): all 15 planar spline p-curves of the bracket — 10 open edges give a proposal whose recheck passes, 5 closed edges are `Coincident` (matches the Phase 53 probe); cumulative-limit, recheck-limit, cancellation (start/mid/final), invalid/out-of-domain interval and stale-source refusals pass. `solid-kernel --no-default-features`: 170 passed, 0 failed, 0 ignored; `cargo check -p app-tui --lib --tests` passes. CAD-enabled (`--features occt`, macOS arm64): 187 passed, 0 failed, 0 ignored.

Next: choose and prove the oriented finite arc (sense/wrap) from candidates, then master/trim/body gates, constrained tetrahedralization and the remaining roadmap.
