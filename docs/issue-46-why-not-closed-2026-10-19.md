# Issue 46 — why the epic is far from closed (root-cause analysis, 2026-10-19)

Scope: epic #46 / #63 (replace OpenCASCADE with a native STEP -> mesh pipeline, then retire OCCT). Evidence: `git log`, `docs/issue-46-completion-checklist.md`, `docs/issue-46-occt-retirement-readiness.md`, the ABC survey (`docs/issue-46-abc-corpus-2026-10-18.md`, step 9).

## The measurement that exposes it

The first run on real third-party files came at Phase 114. On 300 ABC files the native pipeline then produced a mesh for 11 (3.7 %). Phases 114-119 raised that to 108 (36 %). The pipeline is a serial funnel, so stage rates multiply:

| stage | files out of 300 | note |
|---|---|---|
| read the STEP model | 211 | 83 reader refusals + 6 entity/byte-budget refusals |
| sample a closed shell | 154 | 57 sampling refusals |
| conform a tetrahedral mesh (`--merge`) | 108 | 44 shell failures (25 feature-size floor, 19 cancelled at 100 s) + 2 timeouts |

Before Phase 114 nobody knew these numbers, because the work before it was measured on synthetic and OCCT-built fixtures plus two real files (0 of 2 passed).

## Root causes, in order of weight

1. **Validation on representative data came last, not first.** About 110 phases (50-113) were qualified on fixtures chosen by the same author as the code. The failure classes that dominate real files (diagonals of planar faces, near-flat creases, needle sampling, spline-surface closest-point failures, torus bands, multi-body files, multiple `FACE_OUTER_BOUND`) were invisible until a corpus was run. Each class took one phase to find; there are still about ten open.
2. **Effort went to research tiers that are not on the retirement path.** By title, roughly half of Phases 50-113 address #55-#62 (high-order curved elements, anisotropic adaptation, ALE motion, learned cost models, AMR advice, benchmark records). Retirement (#63) needs: read, sample, mesh, and the editor operations. The checklist counts these tiers as progress, so activity looked like convergence.
3. **The largest items on the critical path were not started.** OCCT is the only implementation of booleans, fillets/chamfers, profile extrude/lathe, STEP export and tessellation in the editor (`solid-kernel/src/occt.rs`). A native boolean/fillet kernel is a project of its own, larger than everything delivered for reading. It has no phase yet. It was deferred while the cheaper read-side items consumed the schedule.
4. **The native path has no consumer.** `app-tui`, `fea-problem` and `cli` never call `AnalyticModel`, `plc_conform` or `hybrid_region`. There is no product feedback loop, no hosted-CI evidence for native-only operation beyond Phase 111, and no user-visible failure that would rank classes by importance.
5. **The exit criterion is not a number.** The readiness proposal says "a measured success rate the owner accepts". Without a target (for example >= 90 % of a held-out corpus, per operation), no amount of work can finish: every survey leaves a long tail (the 25 floor failures are individually different). The checklist then keeps broad gates unchecked by design ("phase numbers count increments, not tiers").
6. **Fixes land at the symptom stage, but causes live upstream.** The conform stage fails on needle triangles and acute input angles that the sampler (stage 2) or the reader's face handling (stage 1) produced; 11 of 25 classified floor cascades were planar-face diagonals created by sampling. Improving a downstream stage in isolation moves the funnel less than fixing the generating stage.

## What follows from this (the plan the evidence supports)

- Fix the exit criterion first (owner decision): proposed >= 90 % of a held-out ABC chunk conforms, plus parity within 0.3 % volume on the OCCT-built set, plus each editor operation native or dropped.
- Work the funnel by file count, upstream first: reader refusals (83), sampling (57), then conform (44 + 2). Re-run the survey after every fix; a drop of the 108 baseline blocks the merge.
- Start the modelling kernel now as its own track (extrude/lathe and STEP write first: closed-form proofs; booleans/fillets last and largest).
- Wire the native path into `fea_problem::solid` behind the compatibility registry with OCCT as fallback as soon as the success rate is meaningful, so failures surface in the product.
- Pause new work on #55-#62 tiers until the retirement path is closed, unless it blocks a retirement item.

This session started that plan: three parallel tracks (conform floor/cancelled, STEP reader and sampling refusals, native modelling operations); their results are recorded in their own phase documents and in the corpus document.
