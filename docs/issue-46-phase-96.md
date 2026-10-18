# Issue 46 — Phase 96: native-versus-OpenCASCADE parity matrix on OpenCASCADE-built shapes (#63 groundwork)

With OpenCASCADE built and running (Phase 95), `tests/parity_occt.rs` (feature `occt`) builds nine shapes with OpenCASCADE, exports STEP, and sends the file through the native pipeline (`AnalyticModel::read`, `sampled_surface`), comparing with the exact volume and with OpenCASCADE's own tessellation (deflection 0.005). The matrix is pinned: what is supported and what is refused is asserted, so a change is a visible decision.

| shape | exact | OpenCASCADE tessellation | native (chord 0.005 mm) |
|---|---|---|---|
| box 2x3x4 | 24 | 24 | 24 (exact) |
| cylinder r 2, h 3 | 37.6991 | 37.6366 | 37.6080 (-0.24 %); at the default chord of 0.05 mm: 36.8971 (**-2.13 %**) |
| extruded L profile | 12 | 12 | 12 (exact) |
| plate with a through hole | 11.8037 | 11.8049 | 11.8054 (+0.015 %) |
| plate with two holes | 11.6073 | 11.6098 | 11.6109 (+0.031 %) |
| plate with a boss | 15.5186 | 15.5041 | 15.4982 (-0.13 %) |
| plate with a pocket | 11.4 | 11.4 | 11.4 (exact) |
| union of two boxes (L) | 6 | 6 | 6 (exact) |
| box with every edge filleted | 23.7017 | 23.6927 | **refused**: unsupported native analytic surface |

Reading: for planar solids the native volume is exact; for planes-and-cylinders solids it equals OpenCASCADE's to the chord tolerance (an inscribed sampling is a little small for a solid cylinder and boss, a little large for a hole, as asserted). Two honest gaps: (1) the **default chord of 0.05 mm gives a 2.1 % volume error on a 2 mm radius cylinder**, ten times OpenCASCADE's own tessellation error at its default; callers that need volume parity must pass a finer chord (the matrix uses 0.005 mm); (2) **only planes and cylinders are importable**: a filleted box (cylindrical edge blends and spherical corner patches) is refused, and so are cones, tori and spheres in general; the native import cannot replace OpenCASCADE for models with those faces. No third-party CAD file beyond the bracket was available.

This is groundwork for #63, not parity: production parity needs the unsupported surface types, the healing and exact-trim gates of #51, an independent acceptance on representative models, and the owner's separate approval; `occt_retirement_approved` stays false and OpenCASCADE stays production.
