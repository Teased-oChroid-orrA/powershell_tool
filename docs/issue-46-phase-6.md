# Issue 46 — Phase 6: bounded cylindrical trim charts

Source `05e9f0ee77f5f04b6afb5c616ed6f167c64c9019` adds `cylindrical_step` and `AnalyticModel::sampled_cylindrical_trims`. Shared edge samples unwrap into physical circumferential/axial millimetres. Integer-period reconstruction avoids cumulative remainder drift. A single explicitly seamed loop must close in its chart, span at most one turn and pass the existing sampled-polygon validity checks. Half-turn ambiguities, winding/unseamed bands, multiple bounds and work-budget exhaustion fail explicitly. No native triangulation or production importer switch is included.

All ten cylindrical bracket faces pass, complementing the 28 planar faces from Phase 5. An independently exported translated cylinder (radius 2 mm, height 3 mm) yields the developed lateral area 12π mm² within the unchanged 1e-12 test bound. Negative cases retain hard work caps and ambiguous periodic-step rejection.

A tilted OCCT cylinder export initially failed the single-turn certificate: observed chart width 12.566370614360283 mm versus circumference 12.566370614359172 mm, beyond the arithmetic-only closure/span bound. Integer-period reconstruction removes accumulation as an implementation risk but does not qualify that reference. The refusal is retained as an explicit regression; no threshold was raised or chart clamped. Analytic consistency alone is therefore insufficient to promise periodic trim certification for every rotated/export-rounded model. Future work needs explicit geometric uncertainty/canonicalization with modification reports, not silent seam snapping. This limitation is part of the supported subset.

**40 CAD-enabled tests and 31 no-default-feature tests passed** at the committed source. [Verification](issue-46-phase-6-verification.json). No dependencies, lockfile, numerical criteria, production import or main branch changed. No additional hosted CI run was requested. Draft [PR #65](https://github.com/Teased-oChroid-orrA/powershell_tool/pull/65) tracks these foundations; CI skip markers respect the requested usage restraint, without replacing eventual platform/integration gates.

## Current native diagnostic support matrix

| Area | Implemented subset / refusal |
|---|---|
| Physical STEP file | Bounded inventory, complete retained references; unknown entity declarations preserved |
| Units | Explicit assigned SI / positive conversion factors; direct body representation association; normalized mm |
| Body topology | One simple manifold-body graph for analytic evaluation; raw multi-body extraction has cumulative limits |
| Curves / surfaces | Referenced LINE / CIRCLE and PLANE / CYLINDRICAL_SURFACE; unsupported geometry refuses the whole analytic model |
| Planar trims | Sampled polygons: one outer + unnested holes; order-independent roles; crossing/touching rejection |
| Cylindrical trims | One chart-closing seamed boundary; conservative single-turn certificate; tilted reference above refused |
| Placements / assemblies | Local analytic frames supported; mapped/assembly transformations explicitly refused |
| Exact trims / solid validity | Not certified: sampling may hide exact curve intersections; no global shell/self-intersection certificate |
| Native tessellation / volume | Not implemented; point-set hull Delaunay is not CAD/PLC recovery |
| IGES / healing / stable association | Not implemented; no silent repairs or persistent naming guarantee |

The complete epic remains open. Next gates: preserve canonical shared-boundary sample identities through constrained chart triangulation, explicitly handle periodic uncertainty/holes, verify global shell orientation/vertex links/geometric intersections and independent volume/face measures, then constrained 3D recovery and sliver quality. General B-splines, assemblies/healing and later P2/P3 scopes remain open. OCCT stays the production backend; retirement and future merge remain separately gated.
