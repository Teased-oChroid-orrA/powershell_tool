# Issue #12 — Phase 14 structural elements

2026-10-10 UTC; branch `codex/fea-core-roadmap`, draft PR #13.

`structural.rs` adds spatial two-node axial trusses and six-DOF/node frames. Frames have
axial, Saint-Venant torsion and exact two-node Timoshenko bending (optional shear areas;
`None` selects Euler-Bernoulli). An explicit reference direction defines the local frame;
loads are local, nodal loads/results are global. Uniform distributed loads use consistent end
forces/moments, subtracted during local end-action recovery. Axial stress is recovered directly.
The fields union permits frames and trusses in one assembly without introducing rotations at
truss-only nodes. Sparse generalized assembly/constraints reuse Phase 13. A separate connected
component support-rank check rejects visible rigid motions before factorization; invisible
rotations of collinear trusses are excluded, and unconstrained orphans are errors.

`plate.rs` adds a five-DOF/node flat rectangular shell/plate: plane-stress membrane plus
MITC4 edge-tied Mindlin shear and bending, 2x2 integration, isotropic constitutive resultants,
consistent pressure loads, and center membrane/moment/shear recovery. This deliberately supports
only flat axis-aligned, counterclockwise rectangular elements. Distortion, curved shells, drilling
rotations, laminates and large rotations are rejected/unsupported. The two transverse rotation
signs are physical global rotations (`gamma_x=w_x+theta_y`, `gamma_y=w_y-theta_x`).

Independent verification: inclined axial bar; axial extension, tip bending including shear,
torsion; cantilever uniform-load deflection/support shear/moment/free-end recovery; rotated
orientation and rigid-motion patch; subdivision exactness for Euler-Bernoulli loads;
flat-shell membrane/bending patches and rigid-motion energy; simply-supported Fourier-mode
Mindlin/Navier response with monotone approximately second-order mesh convergence at thickness/span
0.2, 0.02 and 0.002, final relative error <1%. No test tolerance was relaxed.

Limitations: small displacements, linear elasticity, no member releases, frame warping, geometric
stiffness, member-local buckling, plastic hinge, stress concentration or automatic allowables.
A truss axial stress and frame end actions are not a certified strength/buckling margin.
Full workspace validation and final commit/evidence links are recorded in the roadmap handoff.
