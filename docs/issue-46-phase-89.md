# Issue 46 — Phase 89: boundary-first refinement converges at strong anisotropy (#58)

Phase 86 found the boundary slivers of anisotropic refinement and fixed the flat-boundary triangulation, but 10:1 still did not converge: 67 edges stayed longer than the metric allows, all running from a cube corner to a vertex in the thin layer. Cause (measured, not assumed): edge bisection splits through the terminal edge of the *volume*, so a boundary edge between a corner and a layer vertex is only split when its cells' longest edge happens to be it; the flips then keep re-creating such edges because the corner vertices are fixed. The surface must follow the metric before the volume does.

`RefineOptions::boundary_first_passes` (default 0 = unchanged behaviour): in the first N passes only boundary edges (edges of a face used by exactly one cell) that are longer than the band are split, each one directly (no terminal-edge walk), then the flips and smoothing run as usual; afterwards the normal terminal-edge refinement continues. Used together with `FlipOptions::planar_boundary` (Phase 86) so the boundary triangulation can follow.

Measured (unit cube, 1 / ratio sized layer across the middle, `quality_against_the_anisotropy_ratio`, macOS arm64 release, 60 passes allowed; metric radius ratio with the edge-band penalty is not used here, plain metric radius ratio):

| ratio | default: cells, converged, worst, below 0.1 | boundary-first 6 + flat flips: cells, passes, worst, median, below 0.1 |
|---|---|---|
| 2 | 211, yes, 0.53, 0 | 257, 15, 0.36, 0.76, 0 |
| 4 | 437, yes, 0.205, 0 | 413, 13, 0.197, 0.70, 0 |
| 6 | 888, yes, 8e-4, 148 | 868, 39, **0.14**, 0.63, 0 |
| 10 | 1884, **no**, 1e-27, 717 | 605, 15, **0.31**, 0.71, 0 |
| 15 | 3641, no, 1e-22, 1845 | 693, 15, **0.25**, 0.73, 0 |
| 20 | 3925, no, 2e-26, 1967 | 770, 15, **0.20**, 0.73, 0 |
| 40 | 8654, no, 5e-28, 5439 | 945, 20, **0.31**, 0.73, 0 |

Every case from 6:1 to 40:1 converges with no cell below 0.1 and a median near 0.7, with 4 to 9 times fewer cells than the default would need (which does not converge above 6:1); at 2:1 and 4:1 the default is as good or slightly better (0.53 against 0.36 at 2:1), so the options are for strong anisotropy, not a new default. Phase 67's "0.0085 at 10:1" is now 0.31. Regression test (debug, 4 s): 10:1 converges in 15 passes, worst cell over 0.2, none below 0.1, band fraction over 0.7, volume 1 and surface area 6 to 1e-12, every boundary vertex on the cube surface; flat-boundary flips without the boundary-first passes do not converge on the same field (control).

Experiment that was rejected: using the band-aware smoothing quality in the flips changed the default variant's worst cell for the worse at 4:1 (0.205 to 0.161) and did not converge 10:1; not adopted.

Limits (not fixed): the geometry is the unit cube (planar faces, coordinate-aligned layer); `planar_boundary` merges coplanar boundary faces regardless of their names, so it needs the caller to know the patches (Phase 86); curved boundaries, CAD-constrained operations and AMR-controller integration (the other open #58 items) are untouched; no Windows or hosted-CI evidence.
