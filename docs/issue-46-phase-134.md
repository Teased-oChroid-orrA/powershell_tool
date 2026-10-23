# Phase 134 — boss-rim snap conflict (2026-10-22)

Epic #46 / #63, continuing [Phase 132](issue-46-phase-132.md). This is a bounded diagnosis. OCCT retirement remains unapproved; no mesher code or quality threshold was retained from the probe.

The focused release parity harness (`CASE='plate with a boss' NO_SOLVE=1`, `fea-problem` `occt_vs_native::table`) exited 0. It reports measurements but does not assert solver quality. Temporary tracing in `fea-core::tetmesh::mesh_solid_reported` measured unique edges after lattice compaction, after snapping/reconnection, and after boundary repair. The probe was removed afterward.

| Path | Compact minimum edge / edges below 0.025 mm | After snap | After repair | Final solver gate |
|---|---:|---:|---:|---|
| OCCT | 0.166667 mm / 0 | 0.007913 mm / 38 | 0.011770 mm / 6 | minimum scaled Jacobian 0.066; 6 tets below 0.1 |
| Native | 0.166667 mm / 0 | 0.007825 mm / 49 | 0.012626 mm / 8 | minimum scaled Jacobian 0.063; 20 tets below 0.1 |

The first short edge is the same geometric conflict on both surface paths. On OCCT, adjacent vertices 10993/11001 begin at `(6, 0.833333333, 1.166666667)` and `(6, 0.833333333, 1)`. They end at `(6, 0.807828082, 1.0078125)` and `(6, 0.806573726, 1)`, 0.007912558 mm apart. On native, vertices 140/141 begin at `(7.166666667, 1.833333333, 1)` and `(7.166666667, 1.833333333, 1.166666667)`. They end at `(7.175847170, 1.832013376, 1)` and `(7.175416834, 1.832075249, 1.0078125)`, 0.007824588 mm apart. Both pairs start as vertical lattice edges adjacent to the plate/boss intersection; both endpoints are pulled toward the same rim.

The upper vertex on each path remains roughly 0.0078 mm off the plate face after snapping. Its requested full move to the rim would collapse the edge; the snap loop accepts only fractions that preserve its current neighbor-quality floor. The lower vertex reaches the rim. This establishes that snapping creates these near-short edges, while repair removes most but not all. The current acceptance test prevents an inverted or badly degraded neighboring tet; it cannot resolve two boundary vertices competing for one geometric feature with fixed connectivity. Skipping either snap or imposing a minimum edge length would leave a vertex off the model surface. A safe change needs a topology-aware local cavity operation or a different pre-snap boundary discretization, with a connected boss-rim regression that proves surface fidelity and the unchanged 0.1 solver gate. Neither is established here.

The unmodified source was restored after both probes (`git diff -- fea-core/src/tetmesh.rs` empty). Raw logs: `/private/tmp/boss-edge-trace.log` (SHA-256 `84be93b62df4973187d51cc939844cf94ec77b9dea58756da9ca672692b1720e`) and `/private/tmp/boss-pair-trace.log` (SHA-256 `75adcf4b870238a6485f93579d7e3529a33af8bb753790f4efa587827e5e2b12`). These temporary logs are local diagnostics, not redistributable test fixtures. Static solves, other geometries, and Windows runtime meshing were not run in this phase.
