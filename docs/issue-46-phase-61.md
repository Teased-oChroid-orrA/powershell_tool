# Issue 46 — Phase 61: registry entry and input-angle gate for the native constrained strategy

Stacked on Phase 60 (PR #88).

`fea_core::meshing::compatibility` (registry version 2) now knows `Strategy::NativeConstrained`: available for a general solid, linear tetrahedra (`Tet4` only; the recovery produces no Tet10), linear-elastic 3D, within the resource budget, and never when exact curved CAD is demanded. The request gains `shell_min_angle_deg`; `None` is `ShellAngleUnknown` and anything below `NATIVE_MIN_SHELL_ANGLE_DEG` (18 degrees, the smallest input angle with recovery-plus-refinement evidence in Phases 58-59) or NaN is `ShellAngleTooSmall`. A fixed native choice with a bad shell is an error, never a silent switch to the lattice mesher; `select` still keeps the audited input order. `GeneralHybrid3d` stays unavailable.

`solid_kernel::plc_conform::min_shell_angle_deg(vertices, facets)` measures that input angle (smallest triangle corner or dihedral angle on either side of any edge; checked on the cube 45, 3x3x1 slab 18.43, 4x4x0.5 slab 7.12, ring plate 14.04 degrees). `step-inventory --conform` prints the registry verdict first: the NEMA17 sampled shell has a smallest angle of 0.012 degrees, so the registry answers `ShellAngleTooSmall` and the attempt (made anyway on request) exhausts the Steiner budget, which explains the earlier non-convergence deterministically.

Evidence: three registry tests (including the new native one with the version pin); `solid-kernel` and `fea-core` meshing suites pass. Not claimed: that angles above 18 degrees always converge, or that plain recovery fails below it (a 7-degree slab recovers without refinement).
