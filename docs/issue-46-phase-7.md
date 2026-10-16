# Issue 46 — Phase 7: native analytic surface triangulation

Committed source `09323716dbf0c217e5530e103a5b5cb5ce0f6eb0` adds `chart_mesh` and `native_surface`, plus the no-OCCT diagnostic `step-inventory FILE.step --surface`. Production import remains OCCT. [Dependency audit](issue-46-phase-7-dependency-audit.md) and [verification](issue-46-phase-7-verification.json).

## Delivered behavior

Validated chart boundaries enter a pinned 2D constrained Delaunay triangulator. Outside/hole regions are classified by crossing constraint parity from the outer face, avoiding rounded centroids of extremely thin triangles. Every source boundary segment must survive with exactly one incident chart triangle; interior edges need two, all nodes must be used, and dependency vertex indexing is verified. Canonical vertex/edge-sample keys preserve identical shared boundary coordinates without proximity welding; interior points have face-local source keys.

Cylindrical interior chords are split at analytically evaluated points until their circumferential span satisfies the same sagitta bound as boundary sampling. Refinement proceeds in batches with bounded region traversals. Every curved edge retains an interior sample, even when its arc is shorter than the requested chord resolution. Assembly uses chart orientation plus declared face sense, rejects locally inverted/degenerate facets, duplicate triangles and inconsistent edge incidences, requires a connected cyclic link at each vertex, and integrates positive volume about a translated origin with compensated summation. Work/vertex/triangle admission limits and cancellable per-face progress return no partial result.

The bracket retains all 38 source faces. Default 0.05 mm chord settings produce **474 shared-edge samples, 469 assembled vertices and 978 triangles**, volume **34654.80701112524 mm³**. The 0.005 mm run gives **1326 vertices / 2692 triangles**, volume **34633.56162623845 mm³** versus **34632.793504982576 mm³** from the independent OCCT tessellation at 0.005 mm. Relative discrepancy drops from about 0.064% to 0.0023%; the unchanged new convergence acceptance is fine error < coarse error and fine relative error < 1e-3. This compares two tessellations, not exact CAD mass properties. The independent 2×3×4 cube retains eight vertices, twelve triangles, six faces and volume 24 mm³ within 1e-12.

**47 CAD-enabled and 36 feature-off solid-kernel tests passed** at the clean committed source. The standalone native command exited zero, reported progress through 38/38 faces, and was measured separately from compilation: 0.014738 seconds / 9344 KiB maximum RSS on this cloud host (one observation, Python wait4). This is diagnostic geometry evidence, not verified solver telemetry or representative training data. No additional hosted CI was dispatched.

## Failures found and corrected

- Rounded centroid classification omitted very thin chart regions. Constraint-parity traversal fixes the cause without tolerance changes.
- Shallow curved strips can have boundary chords coincident with neighbouring planar patches. Analytic interior refinement removes their duplicate straight facets.
- Four edges still had four incident triangles: short arcs and straight CAD edges shared endpoint IDs, and single-chord arc sampling collapsed distinct edges. A mandatory interior sample preserves the curved-edge identity; no welding/deletion repairs this after assembly.
- Fine boundary sampling alone left about 0.54% volume discrepancy because long interior cylinder chords remained coarse. Applying the angular error bound to interior edges fixes the bias.
- Reclassifying regions after every point exhausted even an explicitly larger fine-test work cap. Batched insertion passes the original default cap; no default cap or numerical acceptance was raised. Development also caught missing argument/type annotations before qualification.
- `/usr/bin/time` was absent; the standalone measurement uses the native child’s wait4 resource data instead. This optional measurement-tool absence did not block tests.

## Remaining gates

This milestone certifies sampled connectivity/local orientation and the tested geometric convergence, **not global nonintersection or exact trimmed CAD validity**. Next is a bounded global triangle-intersection gate, then corpus/feature coverage and native-to-solver integration. The tilted-cylinder strict chart refusal, periodic holes/unseamed bands, exact curve intersections, assemblies, B-splines, conservative healing, persistent naming and general PLC recovery remain open. P2/P3 and final OCCT retirement are not complete. Keep production OCCT; future merge and retirement remain separately gated.
