# Issue 46 — Phase 112: a sub-facet circumcentre outside its facet no longer aborts the recovery (#52/#56)

Phase 111 recorded that the cylinder with a cross bore failed in the tetrahedral recovery after 127 s with "sub-facet circumcentre leaves its facet without encroaching a segment".

## Cause and fix

`conform`'s facet recovery inserts the circumcentre of a missing sub-facet into its facet. In exact arithmetic a centre outside the facet always encroaches a boundary segment (and that segment is then split first). On the skinny sub-facets of the saddle edge of the cross bore the computed centre was outside but encroached nothing (rounding), and the recovery gave up. The same situation was already handled when a centre merely *sits on* the boundary ("split the closest boundary sub-segment instead and revisit"); it now applies to a centre outside the facet too: the nearest boundary sub-segment, the one the centre lies beyond, is split, unless it is frozen (then the old error stands). `plc_conform.rs`, `facet recovery`.

## Evidence

The cross-bore body (cylinder R8 x 30, cross bore R3, graded shell with chart quality) now meshes: 87 hexahedra, 124 pyramids, 9957 tetrahedra, conforming (0 non-manifold, 0 mismatched, 0 hanging, one component), 0 inverted, element volume equal to the body volume (1e-9), 128 s in release (`tests/hybrid_region.rs`, `#[ignore]`d study). The next failure it had hit, with the default quality target, is a legitimate one and stays: "refinement descends below the feature-size floor around a vertex (input angles too small for the requested quality)": the saddle's input angles are small (Phase 104: 12 degrees after grading), so radius-edge 2 is unreachable; the study asks for 5 and obtains 4.32. `cargo test -p solid-kernel --no-default-features --locked --offline`: 247 passed, 0 failed; `--features occt`: 272 passed, 0 failed (macOS arm64).

## Limits (recorded)

- 128 s is long for 10 000 tetrahedra; the time is in the facet and segment recovery on the curved shell, not in the voxel chooser or the join. Not optimised here.
- The fallback has no regression test faster than the study (the failing configuration needs this body); it is exercised by the ignored test.
- No Windows or hosted-CI evidence of its own.
