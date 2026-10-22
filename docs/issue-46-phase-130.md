# Phase 130 — solver-gate repair gain and early PLC cancellation (2026-10-22)

Epic #46 / #63, continuing [Phase 129](issue-46-phase-129.md). OCCT retirement is still unapproved. No quality threshold, chord tolerance, or geometry validator was relaxed.

## Boundary repair gain

`fea-core` boundary repair aims for scaled Jacobian 0.12, while the default solver admission gate is 0.1. Its previous gain rule rejected a vertex move unless it improved the local minimum by at least 0.01 or reached 0.12. A traced native box-with-boss candidate improved the local minimum from about 0.0982 to 0.1051, clearing the solver gate but missing both old acceptance cases. The rule now also accepts a positive move that crosses the default solver gate. A focused unit regression covers the small gate-crossing gain.

In the focused release mesh-only box-with-boss comparison, native minimum scaled Jacobian improved from 0.096 with 5 sub-gate tets to 0.102 with none (mesh time 6.32 s in the changed run). The OCCT path improved from 0.091 to 0.096 but still has 3 sub-gate tets (6.66 s changed-run mesh time). This qualifies the native mesh on this model's scaled-Jacobian gate only; it does not clear the shared absolute-quality set or establish a static solve for this increment. The tilted-hole case remained at native 0.065 / 1 sub-gate tet and OCCT 0.121 / 0.

## PLC cancellation

`solid-kernel::plc_conform` previously built the initial Delaunay mesh before its first progress/cancellation callback. On ABC `00007062`, that means the `--conform-timeout 0` diagnostic could still spend unbounded time initializing roughly 110,215 shell vertices. The initial `Phase::Delaunay` callback now runs after shell validation and before tetrahedralization. A focused test failed before the change (`Limit` rather than `Cancelled`) and passes after it. This makes an already-cancelled PLC request stop before expensive initialization; it does not bound the preceding STEP grading and surface sampling or prove a conforming mesh.

## Tilted-hole causal diagnosis and limit

A controlled temporary toggle disabled cylinder interior boundary-to-boundary chord refinement for the tilted-hole comparison. Native mesh quality then reached minimum scaled Jacobian 0.120 with no sub-gate tet, versus 0.065 / 1 with refinement; OCCT remained 0.121 / 0. The toggle was reverted. The refinement protects against a known overlap between a curved face's internal chord and a neighboring planar face. `shape_chart` does not receive that plane's trimmed domain, which would be needed to distinguish the overlap from this tilted-hole rim chord. A simple span or endpoint guard would risk restoring the overlap defect. The native tilted-hole failure remains open.

## Verification and remaining work

On macOS arm64 with locked offline dependencies, `cargo test -p fea-core` passed 555 tests with 36 ignored, and `cargo test -p solid-kernel` passed 297 with 4 ignored. Focused regressions passed separately. The ABC files remain outside the repository because redistribution rights are unestablished. Full OCCT/native 19-model parity, Windows mesh/solve, ABC `00007062` PLC completion/solver quality, ABC `00006600` grading, and the remaining shared quality failures were not resolved here. `occt_retirement_approved` remains false.
