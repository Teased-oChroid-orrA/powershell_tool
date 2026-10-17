# Issue 46 — Phase 36: finite planar bilinear spline faces

Completed PR #66 was merged, as authorized, at `3aebf992ea1861bcc0a349d09af59315fe019a18`; its local/remote branch was deleted. Current work starts at that exact merge on `feature/native-cad-planar-spline-trims`. Prior records refer to the original branches historically.

Source `1ad84b7` admits a strict new diagnostic native surface class: exactly coplanar, strictly convex, two-by-two degree-one spline patches with positive weights. The exact-sign 3D predicate is reused for coplanarity and for projected 2D orientation (z=0 points against z=1); no new predicate dependency or epsilon replaces an exact zero. Noncoplanar controls, including a warp below the ordinary geometry tolerance, refuse rather than being implicitly flattened. Degenerate/concave/folded nets and higher-order nets remain unsupported.

## Geometric domain and orientation

The positive-weight rational bilinear patch maps its rectangle onto the convex control quadrilateral. Its homogeneous Jacobian determinant is bilinear in U/V; each corner sign is a positive weight product times the corresponding adjacent control-triangle sign. Strict convexity makes all corner signs agree, hence the determinant never vanishes. The boundary maps bijectively to the quadrilateral edges; the interior local orientation and simple boundary establish the same complete quadrilateral image. No inverse-UV claim or floating Newton residual is required for this geometric reduction.

Follow-up source `25f6c25` also derives outward-rounded intervals for the exact source-control normal and its length. Point/whole-circle agreement is checked against that control plane directly, rather than relying solely on a rounded facade frame. Unresolved normal intervals refuse. The facade normal must have a certified positive dot product with the source normal, preserving U/V orientation. Shared interval square/nonnegative-square-root helpers include subnormal/rounding tests.

The analytic facade derives a plane frame in natural U/V orientation. It does not rewrite source controls, knots, UVs, face sense, topology, units or IDs. Analysis charts remain physical-mm plane charts, as already documented for the facade; they are not original spline parameter coordinates.

The finite image is enforced independently of the underlying infinite plane. Source endpoints and actual line-segment endpoints must lie inside the convex polygon under exact-sign projected tests. Native circular edges use outward-rounded support-function bounds for **the entire underlying circle** against every polygon side. This conservatively refuses an otherwise valid trimmed arc when its full circle leaves the patch; tangent uncertainty also refuses. No arc is clipped or tolerance silently expanded. Existing whole-locus surface checks, sampled trim classification, shell checks and global facet checks remain required.

## Evidence and remaining scope

The real bracket's plane replaced by an equivalent bilinear patch now builds a native sampled surface with unchanged source-face coverage and volume within 1e-12 relative to the original, and passes global sampled intersections. A tiny finite patch under the same trim loops refuses; a 1e-10 out-of-plane corner refuses. Unit tests verify nonuniform positive weights, exact boundary inclusion, circle/outside rejection, folded/degenerate/warped controls, tilted exact planes, reversed parameter grids and positive local differential orientation.

This remains **3D-locus trim reconstruction**, not full STEP p-curve or master-representation agreement validation. Associated p-curves and their parameter units are not silently certified by the new class. Curved surfaces/curves, general trimmed-curve domains, p-curve/master agreement, robust general CAD exchange and later PLC/quality/P2/P3/final gates remain open. Production stays OCCT; no backend switch, retirement, new dependency or hosted CI dispatch.

A filtered development invocation ran two patch unit tests but zero integration tests; that zero-test target is not validation. The final full feature-off/CAD suites include the actual STEP surface integration cases. No development assertion failures occurred in this phase.

The stale standalone surface-module header was aligned with its existing differential/enclosure APIs; no behavior change accompanied that documentation correction. [Source-specific verification](issue-46-phase-36-verification.json). Next: bounded source p-curve metadata/parameter decoding and verified agreement, retaining angular versus length versus spline-knot parameter distinctions.

Final checks at `25f6c25ece1ef3b7cad04354cdcd709de15bea6b`: **132 CAD-enabled / 115 feature-off tests and CAD-enabled application lib/tests compilation pass**, zero failures/ignored tests.
