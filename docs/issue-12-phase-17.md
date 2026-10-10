# Issue #12 — Phase 17 performance and optional advisory ML

2026-10-10 UTC. `tests/bench.rs::roadmap_profiles_and_advisory_selection` is ignored by the
ordinary suite and run explicitly. It profiles representative frame chains and 2D/3D continuum
solves before changing production selection. Three repetitions/minimum per workload, two independent
process repeats; workspace dev profile with fea-core/dependencies optimized, Linux x86_64, 5 CPUs,
stable Rust 1.99.0. Independent analytical response tolerance is 1e-6, residual/equilibrium acceptance
unchanged; frame results also reproduce bit for bit. Benchmark times include solving/verification
and advisory inference, exclude offline training and compilation. No optimization is asserted.

Final core measurement, repeat 1:

| Workload | DOFs/free DOFs | Time (ms) | Accuracy |
|---|---:|---:|---:|
| Frame 8 elements | 54 | 0.569 | 4.50e-13 relative analytical error |
| Frame 32 elements | 198 | 2.358 | 2.91e-11 |
| Frame 128 elements | 774 | 9.600 | 1.87e-9 |
| Hex8 4³ training | 300 free | direct 2.264 / iterative 2.176 | independently verified axial response |
| Hex8 8³ training | 1,944 free | direct 14.160 / iterative 29.777 | same |
| Hex8 12³ training | 6,084 free | direct 100.261 / iterative 89.857 | same |
| Hex8 10³ held out | 3,630 free | direct 58.325 / advised 47.791 | displacement difference 6.97e-12 |

Peak process RSS (Linux wait4 ru_maxrss): 51,004 / 48,480 KiB in the two independent repeats;
whole benchmark wall time 1.351 / 1.297 s, both exit 0. Complete per-case timings/CPU/memory and
commands: `/workspace/phase12-evidence/roadmap-performance-final-{1,2}.{log,json}`.
This is process peak memory, not a claim of per-matrix allocation or Windows/release performance.

A benchmark-only cost-trained one-feature decision stump selected iterative solving above 1,945
free DOFs within the training domain. The held-out isotropic 3D case gained about 1.22x in repeat 1;
small cases and out-of-domain aspect ratios (25 and 0.04) / 2D physics revert to deterministic direct
selection and show timing noise, not a generalized speed gain. All predictions still run the same
numerical/physical acceptance and bounded direct fallback. A recorded strategy event is not itself
a fallback: forced iterative selection also records an event. No learned production default is
justified by this small homogeneous corpus; anisotropy, contrast, contact and conditioning need
broader independent data and measured end-to-end robustness before adoption.

Optional-ML evaluation:

| Advisory role | Evidence and decision |
|---|---|
| Solver / preconditioner selection | Timing-trained experiment above. Small held-out benefit; OOD deterministic fallback. Keep benchmark-only; do not alter production crossover thresholds from this corpus. |
| Initial guesses | Existing exact implicit factor solves/eigen shift-invert dominate these workloads; no validated warm-start corpus or measured benefit. Do not add a learned initial guess without measuring iterations and total time under unchanged acceptance. |
| Mesh prioritization | Existing finite ZZ estimates already drive bounded remeshing. A surrogate must beat estimator+mesher total time without missing high-error regions/singularities; no such evidence here. Retain deterministic estimator. |
| Anomaly diagnostics | Finite-state, residual, equilibrium, heat balance, conditional energy and independent benchmark checks are deterministic. Learned warnings may be advisory only; no labels justify a detector replacing those checks. |

ML remains optional, off in production, and never supplies acceptance status. Neural stress/PINN
solvers are not defaults or replacements. The performance/advisory evaluation phase is incorporated;
production ML deployment is deliberately gated on broader measured benefit and unchanged accuracy.
