# Issue 46 — Phase 55: oriented finite arc selection

Stacked on Phase 54 (PR #82, unmerged when written).

`step_pcurve_arc::select` decides which of the two arcs between a candidate interval's endpoint parameters is the edge's arc, for a planar closed spline p-curve on a source CIRCLE edge. Preconditions are checked, not assumed: exact-document edge/p-curve/face association, a spline p-curve whose active domain contains the interval, distinct start/end vertices, and the existing closed single-winding proof against the edge circle (`verify_spline_circle_deviation` with coverage and single winding).

Method: vertices and two interior witness points (one per arc, at the interval's parameter midpoint and the midpoint of the complementary parameter span) are expressed in the edge circle's frame with outward interval arithmetic. `EDGE_CURVE.same_sense` fixes the counter-clockwise arc `from -> to`. Certified half-plane sign tests place each witness inside or outside that arc (arcs below, above and exactly half a turn are handled; half-turn endpoints must be antipodal within 1e-6 relative, and witnesses near an endpoint refuse). Exactly one witness must be certified inside and the other outside; otherwise the selection refuses. The report gives `Arc::{Interior, Wrapping}`, start/end parameters and whether the parameter increases (through the domain end for `Wrapping`) from the start vertex to the end vertex.

Not claimed: master semantics, loop closure, trim or body validity, non-circle edge carriers, non-planar faces.

Evidence (macOS arm64, unchanged tolerances): all 10 open bracket spline edges select `Interior`, checked by an independent f64 oracle (dense 400-sample atan2 monotone traversal from start to end vertex inside the edge arc, circle frame parsed separately from the STEP text; one edge is an exact semicircle). Flipping `EDGE_CURVE.same_sense` in the source text for each of those edges selects `Wrapping` and the same oracle passes. Closed edges, reversed/outside/full-domain intervals, witness budgets (0 and 1), initial/mid cancellation, a weakened circle proof and a stale source refuse. `solid-kernel`: 190 passed with `--features occt`, 173 passed with `--no-default-features`, 0 failed/ignored; clippy clean for the new files.

Test fixtures shared by the p-curve integration tests now live in `solid-kernel/tests/common/mod.rs`.
