# Issue 46 — Phase 57: finite trims for every supported bracket edge

Stacked on Phase 56 (PR #84).

`step_pcurve_arc::resolve` also handles straight p-curve images (plane LINE and cylinder generators): each vertex's line parameter is proposed by projecting its source point onto the mapped world line (binary64), and `step_pcurve_endpoints::verify` certifies the proposal with outward rounding (`Resolved::Segment`). With `step_curve_audit::Options::finite_arcs` the body audit now gives a finite trim for all 228 bracket associations: 176 segments, 32 oriented arcs (12 wrapping) and 20 closed edges without an arc. `step-inventory --finite-arcs` prints the same counts.

Not claimed: loop closure, face-level orientation or region validity, master semantics beyond the existing locus agreement, edge carriers other than LINE/CIRCLE, spline-surface faces, body validity.

Evidence (macOS arm64): `solid-kernel` 192 passed with `--features occt`, 175 with `--no-default-features`, 0 failed/ignored; clippy clean for the touched modules.
