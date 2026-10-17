# Issue 46 — Phase 56: finite arcs for circle and cylinder circle edges

Stacked on Phase 55 (PR #83).

`step_pcurve_arc::select`/`resolve` now accept the other two circle carriers in the bracket, in addition to closed single-winding planar splines:
- plane parameter CIRCLE p-curves (period 2π in the circle angle, single-turn window [-π, π]);
- cylinder constant-V parameter LINE p-curves (raw U is the angle in radians, period 2π/|magnitude|, window centred on t = 0).

Periodic carriers need no winding proof: one period is exactly one turn. The unwrapped complement (`Arc::Wrapping`) is reported with an end parameter one period away from the interval (`|end - start|` is its parameter span) and is itself checked against both source vertices with the endpoint verifier; if the unwrapped interval leaves the map's raw angular range [-8, 8] rad the same arc one period over is used. Complement witnesses fall back one period lower for the same reason. Splines keep the domain-end semantics of Phase 55. `select`/`resolve` take any `ParameterMapRef`.

`step_curve_audit::Options::finite_arcs` (off by default) now resolves every plane-spline, plane-circle and cylinder-circle association: on the bracket 52 associations give 32 oriented arcs (20 interior, 12 wrapping; 14 on cylinders) and 20 closed edges. Every one of the 32 arcs is checked by the independent f64 oracle (400 samples, atan2 in a circle frame parsed from the STEP text, monotone traversal from start to end vertex inside the edge arc). A mutation that swaps the interior/wrapping decision fails the oracle tests.

Not claimed: line/ellipse/other carriers, spline-surface faces, loop closure, master or body validity, parameter uniqueness.

Evidence (macOS arm64, unchanged tolerances): `solid-kernel` 192 passed with `--features occt`, 175 passed with `--no-default-features`, 0 failed/ignored; `step-inventory --finite-arcs` reports 32 oriented arcs and 20 closed edges; `cargo check -p app-tui --lib --tests` passes.
