# Issue 46 — Phase 84: convergence and cost of second-order tetrahedra on a curved body (#55)

`fea-core/tests/high_order_convergence.rs`: a quarter of a thick cylinder (a = 1, b = 2, length 0.5) under internal pressure, plane strain, meshed by `tetmesh::mesh_solid` from a signed distance function, Tet4 against Tet10 (midside nodes of boundary edges projected onto the exact surface), scored against the Lame solution. A fast test (two sizes, debug) pins that Tet10 has the lower rms radial error; the study (four sizes, `--ignored`, release) prints everything below (macOS arm64).

| h | Tet4 dofs / rms err / max err | Tet10 dofs / rms err / max err |
|---|---|---|
| 0.40 | 681 / 3.2e-2 / 4.6e-2 | 3945 / 6.5e-3 / 9.1e-3 |
| 0.28 | 2430 / 1.1e-2 / 1.7e-2 | 14721 / 1.2e-3 / 2.6e-3 |
| 0.20 | 3918 / 5.7e-3 / 1.2e-2 | 26187 / 9.5e-4 / **9.3e-3** |
| 0.14 | 14358 / 3.3e-3 / 6.8e-3 | 93126 / 4.3e-4 / 3.5e-3 |

Cost (best of three, ms; assemble / factor / solve): Tet4 15 / 38 / 1.8 at 14k dofs (factor 1.7 M entries); Tet10 31 / 863 / 41 at 93k dofs (factor 36 M entries). Per unknown Tet10 is far dearer (factor dominated), per unit of error it wins: Tet10 at 3945 dofs (rms 6.5e-3) beats Tet4 at 3918 dofs (5.7e-3) only marginally, but Tet10 at 14.7k dofs (1.2e-3) is 4.7x better than Tet4 at 14.4k (3.3e-3) for 1.5x the assembly and 1.5x the factor time.

Findings:
1. **Tet10 has the lower rms error at every size** (5x to 8x), but **its maximum nodal error is erratic** (9.1e-3, 2.6e-3, 9.3e-3, 3.5e-3) and its observed rate in the maximum norm (0.9) is below Tet4's (1.8): local defects, not smooth convergence. At h = 0.20 the worst nodes sit 1.4 % inside the bore at mid-height (r = 1.0138), symmetric about 45 degrees.
2. **The snapping leaves boundary vertices off the surface on this body** (earlier evidence was the unit sphere): `refused` 28 / 27 / 62 / 41 of 192 / 438 / 744 / 1724 boundary vertices (15 % at h = 0.4, 2.4 % at 0.14), up to **11 %** of the bore radius from the surface (h = 0.28), and a Tet10 mesh then has 109 to 241 nodes outside the body (the midside nodes of edges touching those vertices and of edges spanning the concave bore). The pinned floor `MIN_QUALITY = 0.05` is reached in every mesh.
3. **Not a budget problem.** More snap passes (6 to 12) and more reconnect rounds (4 to 10) leave the counts unchanged. Widening the feature-snap reach (`edge_reach` factor 0.5 to 0.75 and 1.0) makes it worse (refused at h = 0.14: 41, 110, 243), so the obvious knob is wrong; the refusals are structural (the quality guard against the neighbouring tetrahedra), which is why the open item is a *validity-guarded exact projection* that moves the neighbours too, not a larger budget.
4. A scoring pitfall found and fixed in the study: the Lame solution exists only inside the body, so nodes off the surface (r < a) must be excluded from the error (they were scored at an extrapolated radius and read as a 3.6 % "error" that grew with refinement); they are counted separately.

#55 stays open: the exact CAD-parameter projection with neighbour relaxation, and a convergence study showing a clean higher-order rate (asymptotic, in the maximum norm), are not delivered. This phase supplies the verified diagnosis and the benchmark to hold such a change to.

Limits: one body, one load, one machine; Tet10 only (no Hex20/Wedge15 curved study); the cost of the iterative regime is not part of the table; no Windows or hosted-CI evidence.
