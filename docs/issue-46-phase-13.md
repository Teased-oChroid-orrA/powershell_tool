# Issue 46 — Phase 13: immutable validated triangulated PLC input

Source `2367cf5` introduces `plc::ValidatedPlc`, constructed only from immutable globally checked sampled geometry. It retains the original vertices, every triangular boundary facet, source-face associations and all requested repair reports. Every undirected triangle edge is an explicit constraint, including triangulation diagonals inside sampled CAD faces. This is one closed connected triangulated shell, including its handles; nested cavity shells, multiple components and material-region seeds remain unsupported by the preceding admission gate.

Facet/segment budgets and cancellable progress bound PLC construction. No point deletion, proximity welding, segment splitting, geometry reconstruction or repeated global intersection test occurs. Read-only access preserves the proof; conversion back to the distance-solid bridge retains the original source/report metadata.

The feature-off actual-bracket regression preserves 469 vertices, 978 facets and 1,467 unique segments exactly. Every facet resolves to its original ADVANCED_FACE, every triangle edge is found in the sorted constraint registry, cancellation and both budget failures return explicit errors. The unchanged sampled geometry certificate does not establish exact CAD trims or complete CAD tessellation accuracy.

This is the input representation/admission increment for #52, **not a constrained tetrahedralizer or recovered PLC volume mesh**. Production strategy capabilities remain unchanged; existing point-set Delaunay remains a convex-hull reference only. No dependency, relaxed tolerance, hosted CI, production/main change or OCCT retirement. General constrained recovery/refinement, cavities/regions and broader native CAD coverage remain open.

Phase 13 final checks at `2367cf573c438a018f25072c0095592c7426c2ed`: **63 CAD-enabled / 51 feature-off tests pass**, no failures/ignored checks. The application compilation evidence remains Phase 12; this additive diagnostic PLC API has no app caller. [Verification](issue-46-phase-13-verification.json).
