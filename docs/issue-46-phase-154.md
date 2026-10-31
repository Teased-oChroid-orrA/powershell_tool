# Phase 154: keep distant tangent faces out of feature snapping

Issue #63, excluding NEMA17 by owner request. The common volume mesher now limits which positive face hits can use the wide feature-snap reach when a boundary vertex is materially off the surface. The remaining OCCT seam-area failure and retirement gates are unchanged.

## Cause and change

At a 0.85 mm element size, the filleted-box lattice vertex `[7.5, 14, 14.4]` is 0.039664 mm from the curved face. The flat face beside that fillet is 0.4 mm away. Both were active because the positive-hit rule admitted any face within 1.1 local edge lengths. Alternating projections pulled the vertex to `[7.5, 14.037332, 14.003677]`, still 0.037513 mm off the tessellated surface. After guarded snap passes it remained outside the box allowance at `[7.5, 14.026184, 14.002579]`. The input surface itself has exact box bounds and no out-of-box vertex ([Phase 152](issue-46-phase-152.md)).

The positive-hit reach was added in Phase 85 to find a bore wall when a vertex is on an end face but still outside the bore. Removing it would restore that defect. `snap_to_features` now admits a positive face through the wide reach only when its trimmed-face distance is within `eps` of the signed distance to the solid. A vertex already within `eps` of the surface retains the earlier selection, preserving the existing near-wall and reentrant behavior. Negative hits retain the half-edge reach.

The new focused regression runs the 0.85 mm fillet independently of the existing three-size face-area test. Before the change it failed at the node above. After the change it passes through both OCCT and native surface paths. The existing 1 mm OCCT face-area test still fails at 80.42372283704822 mm² against the 80.4 mm² limit. That error comes from whole boundary triangles crossing the flat/fillet seam ([Phase 150](issue-46-phase-150.md)); this face-selection change does not repair their topology.

## Local qualification

Tests expanded only after the focused case passed. All runs used locked offline dependencies and the current checkout's Cargo target.

| Check | Result |
|---|---|
| `fea-core` snap unit tests | 10 passed, including the Phase 85 bore and reentrant cases. |
| `fea-problem` 0.85 mm fillet regression | Passed with and without `occt`. |
| Filleted cross-bore, mesh only | OCCT and native pass with zero cells below scaled Jacobian 0.1; native minimum 0.120. Earlier fallback candidates lost this gate ([Phase 152](issue-46-phase-152.md)). |
| T-junction reentrant repair | Passed on both surface paths. |
| Curved-body convergence and boundary repair | 6 passed, 4 ignored. |
| Same-STEP mesh table, NEMA17 omitted, `NO_SOLVE=1` | 36 of 36 rows pass the unchanged quality gates. No previously passing row lost its gate against [Phase 149](issue-46-phase-149-parity.txt). |
| Full feature-off touched-crate suites | `fea-core`: 620 passed, 37 ignored across 67 suites. `fea-problem`: 217 passed, 6 ignored across 27 suites. |

The 36-row table did not run static solves, and changed mesh counts in 16 rows. It is mesh-quality evidence, not a complete solver, load-area, runtime, or Windows parity certificate. NEMA17 remains deferred. The ABC chart and small-angle refusals, Windows native-only mesh and solve, and the 1 mm loaded-face seam remain open. `occt_retirement_approved` stays false.
