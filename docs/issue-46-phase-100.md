# Issue 46 — Phase 100: graded boundary sampling and chart quality; OpenCASCADE as the reference for the bracket gate (#52/#53/#51)

The question was what drives the NEMA17 bracket's failure of the 18 degree input-angle gate (`NATIVE_MIN_SHELL_ANGLE_DEG`), whether the gate must change, and what OpenCASCADE does with the same shell. Measured with `solid-kernel/examples/shell-angle-probe.rs` (OpenCASCADE 8.0.1 reads the STEP and tessellates it) and `shell-sampling-probe.rs` (native sampling, per source face).

## 1. OpenCASCADE is not a better reference for the shell angle

`Brep::read_step` + `tessellate(deflection)` on the bracket, smallest angle by the registry's own measure (`plc_conform::min_shell_angle_deg`): **0.0061 degrees at every deflection from 0.5 mm to 0.005 mm** (a corner of the cylinder fillet face, tag 16). The native base shell is 0.0123 degrees. OpenCASCADE's tessellation is therefore no better than the native one (worse by half), and the bracket has four faces of area 0.0536 mm^2 (5 mm x 0.0107 mm) in the file itself. The production OpenCASCADE path never needed a good shell: `mesh_volume` meshes from a signed-distance field and does not conform to the surface triangles. The gate is a property of the native *conforming* route only, so OpenCASCADE gives no number the gate should be moved to.

## 2. What drives the failure

Per source face of the base native shell (`shell-sampling-probe`, triangles below 18 degrees): **33 of 38 faces** have them (472 of 978 triangles), not just the sliver region. Causes, in order of weight:

1. Each face is triangulated on its own (constrained Delaunay of the boundary polygon, `chart_mesh`) with **no interior points and no grading of the shared boundary**. A 60 mm planar face with a circular hole, or a 54 x 49 mm face whose boundary has only its corner vertices, gets triangles with a 54 mm base and a near-zero height (0.0145 degrees); cylinder strips 50 mm long and 4 mm wide get 0.0123 degrees. Straight edges carry only their two end points.
2. Near-degenerate CAD: the fillet cylinders have radius 3.994642 where the corner they round is 4.0, leaving 5.4 micrometre sliver faces (the four 0.0536 mm^2 faces) and cap faces whose boundary arc meets a straight edge at 0.1 degrees. No tessellator that keeps the faces can grade a face 0.0107 mm wide.
3. The surface Steiner refinement of Phase 77 (terminal-edge bisection) cannot improve a thin triangle whose neighbours are not coplanar: the bisection of a thin right triangle yields similar thin triangles (the longest-edge classes), which is why 40 times the budget stayed at 1.446 degrees.

## 3. Graded boundary sampling and chart quality (new, opt-in)

- `AnalyticModel::graded(GradeOptions, max_work)` (`src/chart_grade.rs`): per face, the chart's Delaunay refinement is run on a scratch triangulation with boundary splitting allowed (`chart_mesh::shape_chart`, the same chord rule and interior pass as the real triangulation); the boundary points it wants are collected per shared edge, inserted on the edge's **own curve** (lines interpolated, circles by angle, `AnalyticModel::insert_edge_splits`), and the rounds repeat until no face asks for more. Segments shorter than twice `floor_mm` (default 0.1) are never split: that bounds the grading around tiny features. The neighbouring faces agree on every shared sample by construction.
- `native_surface::Limits::chart_quality = Some(ChartQuality { min_angle_deg, max_points })`: the final per-face triangulation inserts interior points evaluated on the exact surface (spade refinement with the boundary kept).
- `analytic_step::Options::max_line_mm`: optional equal subdivision of straight edges (a coarser alternative; grading supersedes it).
- `step-inventory --grade DEG[,FLOOR] --chart-quality DEG --max-line-mm MM` runs them.

Results (macOS arm64, release; OpenCASCADE-built shapes via `examples/write-shapes.rs`; smallest registry angle, plain -> graded):

| shape | plain | graded | triangles |
|---|---|---|---|
| block with a through hole | 1.896 | 25.058 | 166 -> 1940 |
| block with a pocket | 9.462 | 26.565 | 28 -> 76 |
| thin plate (12 x 4 x 0.25) with two holes | 1.193 | 25.039 | 104 -> 858 |
| rod (R 2, 60 long) | 0.799 | 25.034 | 86 -> 4248 |

Conforming recovery with radius-edge refinement (`--conform-radius-edge 2`, Steiner budget 100 000): the block with a pocket reaches **1.870** and the plate **1.998** (graded); on the *plain* shells both are refused (`sub-facet circumcentre leaves its facet without encroaching a segment`, `facet insertion cavity is not star-shaped`). Phase 83 had recorded that radius-edge refinement fails at 18 degrees and below; this is the first time it succeeds, and the cause was the input shell, not the refinement. Without radius-edge refinement the maximum radius-edge ratio of the recovered mesh falls from 16.7 to 3.2 (pocket) and 25.1 to 7.1 (plate).

Sampling is cheap (bracket: 4 s to grade, 0.6 s to sample) but the shell grows (bracket 978 -> 23 842 triangles; the Steiner demand of the recovery grows with the vertex count: 4358 for the graded block with a hole, versus 1875 for the plain one, i.e. 4.5 against 22.6 per input vertex).

## 4. The gate stays at 18 degrees

The gate is an evidence bound and the new data keep it meaningful: shells below it are exactly the ones whose recovery needs 5x more Steiner points per vertex and whose radius-edge refinement fails; shells at 25 degrees recover and refine. What was wrong was the producer of the shell, not the threshold. Changing the number would hide that. The registry text now says so.

## 5. Not fixed (measured, recorded)

- **The bracket itself is still at 0.0385 degrees** (graded). The remaining thin triangles are on the faces around the 5.4 micrometre slivers (tags 16, 17, 29, 33, 35). Shell-level defeaturing (Phase 76) works on the base shell (0.0123 -> 0.735) but **rejects every cluster on the graded shell** (`rejected_zero_area` 10-12 of 23-42 clusters; a cap has no flippable neighbour or the flip would duplicate an edge). The consistent remedy is B-rep level: collapse faces and edges below a tolerance (sewing the two rails of a sliver face into one edge) before sampling, i.e. the healing of #51. Phase 101 onwards.
- Plain shells below the gate may be refused by the radius-edge pass with the two messages above; that is the gate's job, not a new defect.

## Defects fixed on the way

- **`plc_conform` failed on every long cylinder** (a rod R 2, 60 mm: `point lies outside the hull and outside every conflicting tetrahedron` after 310 Steiner points). Cause (measured with a convexity check after every insertion): `Delaunay3` extends its hull only through the hull faces of the conflict cavity ("such a point replaces the hull faces it sees, which can leave the hull slightly non-convex", its own module note). A convex shell has every point on the hull of its vertices, so a Steiner point on a rim chord or lateral facet, rounded a few ulps outside, dented the hull (the first dent was 2.4e-3 mm deep after point 336), and a later legitimate shell point lay 1.03 mm outside the dented hull. Two local repairs (a visibility fallback, nudging the point inward) only changed the error. Fix: `conform` now tetrahedralizes the shell vertices together with **eight far points** (a box 64 half-extents out), so every shell and Steiner point is an interior point and the engine's hull case never arises; the far points are in no inside cell, facet or segment and are dropped from the result (`Conforming::points` numbering is unchanged). The block, pocket, plate and tilted-wedge results are bit-identical to before; the rod (plain and graded) now recovers. Regression: `tests/plc_conform.rs::a_prism_over_a_regular_polygon_recovers_although_every_point_is_on_the_hull` (22-, 24- and 40-gon prisms; fails on the old code). `hull_cells` now counts the cells around the far points too.
- `planar_step::sign` called the 3D predicate `orientation` with `z = 0`, whose arithmetic envelope refuses any non-zero coordinate below 1e-30: a chart value of 8.7e-32 (rounding noise around zero from a sampled cylinder seam point) made the face refuse with `planar predicate arithmetic range exceeded`. It now uses the exact planar `orientation_2d` with a 1e-100..1e100 envelope; chart values below 1e-25 are snapped to 0 when a chart is built.
- The chord loop of the cylinder chart and the quality refinement alternate until neither adds a point (a midpoint inserted by the chord rule reshaped the Delaunay triangulation and could leave a thin triangle on the seam).

## Evidence

`cargo test -p solid-kernel --no-default-features --locked --offline`: 215 passed, 0 failed, 3 ignored (default features); 236 with `--features occt` (new: `tests/chart_grade.rs`, the bracket: needles on at least 40 percent of the plain triangles, under 10 percent graded, volume within 5e-4, a converged model asks for nothing more, source faces unchanged). `cargo test -p solid-kernel --features occt --test grade_occt`: 2 passed (angles of three OpenCASCADE-built shapes plain below and graded above the gate; graded pocket reaches radius-edge 2 while the plain one is refused). Clippy clean for the new code. No Windows or hosted-CI evidence.
