# Issue 46 — Phase 80: does AMR starting-mesh advice help? Measured on a plate with a hole (#62)

`fea-core/tests/amr_advice_classes.rs` (class chosen from `issue-46-benchmarks.md` section D): a 12x4 plate in tension with a circular hole (r 0.6), goal = weighted sigma_xx at the hole top, reference = a uniform h = 0.05 solve (J = 2337.92); the default start is h = 1.0. Advice comes from `adaptivity::feedback` over three (deterministic, identical) converged runs, applied as a finer start (multiplier within 1.2 of the hole centre), then the same AMR loop runs to the same tolerance. A plain plate (no hole) is the smooth control.

Measured (macOS arm64, release):

| tolerance (relative to J) | advice | default start | advised start (depth 0.5 / 1.0) |
|---|---|---|---|
| 2e-3 | Singular x0.955 | 2 cycles, 1794 dofs, 0.19 s | 2 cycles, 1786 dofs, 0.18 s / same |
| 5e-4 | Singular x0.955 | 2 cycles, 1794 dofs, 0.18 s | 2 cycles, 1786 dofs, 0.18 s / same |
| 1e-4 | GeomDiscontinuity x0.898, Singular x0.889, Smooth x0.962 | 4 cycles, 7588 dofs, 0.72 s | 4 cycles, 7308 dofs, 0.70 s / same (x0.790 at depth 1) |

The plain plate converges in 1 cycle (700 dofs) and yields no advice, as required of the control.

**Result: no time-to-accuracy gain; the #62 gate is still not met.** The advised start changes dofs by about 4 % and time by about 3 %, never the number of cycles. Strengthening the pre-applied depth (0.5 to 1.0) changed the advised multiplier (0.889 to 0.790) but not the resulting mesh (7308 dofs both): the mesher's own size control at the hole boundary dominates the start size there, so no start-size advice can matter on this class. The class is therefore a poor discriminator, for the same reason as the L-plate (the mesher already resolves the geometric feature). A class is needed where the demand is *not* implied by geometry features the mesher already honours, for example a crack tip or a load singularity inside the domain; the benchmarks doc lists the Mode I crack.

Side observation (not a defect): at the two looser tolerances the loop stops after the second cycle with a true goal error (2.5) above the requested tolerance 1.17 at 5e-4: the estimator's effectivity on this goal is about 0.4-0.5 (the existing L-plate test only bounds effectivity to 0.2-4). A requested tolerance is on the estimate, not the true error.

Limits: one geometry, one goal, one machine; zone-to-size mapping was the caller's choice (the strongest advised zone factor within 1.2 of the hole); the crack class (needs a slit/crack-tip geometry in `fea-problem`/`mesh2d`) and Lamé remain to be run; no Windows or hosted-CI evidence.
