# Issue 46 — #63: is OpenCASCADE ready to be retired? Readiness assessment (2026-10-18, `main` d0431e6 + Phase 113)

> Historical assessment. See the 2026-10-20 addendum below for the current routing and measured retirement gaps. The original observations remain here as evidence of the earlier state.

**Verdict: not ready.** Native STEP import is a credible replacement for *reading* simple analytic and spline parts, but OpenCASCADE (OCCT) is the only implementation of everything the application does with a STEP solid, and the native pipeline has not yet passed on a real third-party file. This document advises; `occt_retirement_approved` stays `false` and flipping it needs the owner's separate approval (root `CLAUDE.md`, "Ask First").

## What was checked (this session, macOS arm64)

| Check | Result |
|---|---|
| `cargo test --release -p solid-kernel --features occt` `parity_occt`, `grade_occt`, `occt` | 1 + 2 + 5 passed. Native volume vs OCCT tessellation on 14 OCCT-built shapes: exact on planar solids; -0.24 % cylinder, +0.015 % / +0.031 % plates with holes, -0.13 % boss, -0.08 % fillets, -0.19 % torus, -0.08 % frustum (native, chord 0.005 mm). |
| Default chord (0.05 mm) | Native volume off by -2.13 % on the 2 mm cylinder, -0.88 % on the filleted cylinder: the default is too coarse for volume parity. |
| Registry shell-angle gate (`NATIVE_MIN_SHELL_ANGLE_DEG` = 18) on OCCT's *own* tessellation | 1.2-9.5 degrees on the three plates / blocks; only graded shells reach 25 degrees. The plain pocket shell is refused by the conforming mesher ("refinement descends below the feature-size floor"). |
| Real third-party file 1: `nema17-motor-bracket.step` (unknown provenance) | Fails the native 18 degree gate (5.4 micrometre sliver faces; OCCT's own tessellation: 0.0061 degrees). |
| Real third-party file 2: a TraceParts / Halder standard part (`231110746__.stp`, 31 KB, AP214, planes + cylinders + cones + tori, 20 faces; local only, not committed: licence unknown) | Reads natively (all surface types supported), but the native sampled shell's smallest corner is 5.81 degrees (faces with 80 triangles), below the 18 degree gate. OCCT's own tessellation of it: 1.3-4.2 degrees. |
| Hosted CI | Phase 111's run passed on macOS and **Windows** (native hex-core join) and failed only on ubuntu in the recorded-OCCT-volume tests, the 1e-6 tolerance fixed by #147; the run of #147 and of Phase 112 was still in progress at the time of writing. |

## Who consumes what

- `app-tui` and `fea-problem` never call the native pipeline (`AnalyticModel`, `sampled_surface`, `NativeConstrained`, `plc_conform`, `hybrid_region`: no references in `app-tui/src`, `fea-problem/src`, `cli/src`). Reading, editing and tessellating a STEP solid in the application all go through `fea_problem::solid` / `stepops.rs`, which return "needs the OpenCascade build" without the `occt` feature.
- The compatibility registry that would select the native path is gated on the 18 degree shell angle and is not selected by production strategy.
- Windows release builds that include OCCT need `--features occt-source` (static CRT) and ship `THIRD-PARTY-NOTICES.md` + `licenses/` (LGPL 2.1 + exception).

## Gaps that block retirement (verified)

1. **Modelling has no native replacement**: booleans, fillets / chamfers, extrusion / lathe of profiles and STEP *export* exist only in `solid-kernel/src/occt.rs` (the application's solid editor uses them).
2. **Real-file robustness is 0 of 2.** Both real third-party parts read natively but fail the shell-quality gate; there is no corpus run (ABC, NIST PMI not downloaded, licences unverified).
3. **Exact trims and B-rep healing** (#51) are open; sampled trims are not a certificate.
4. **Volume parity needs a finer chord than the default** and a 18 degree shell the native mesher cannot get from ordinary CAD faces without grading.
5. **Cross-bore hybrid** (the saddle intersection) takes 128 s and needs `max_radius_edge: Some(5.0)`.
6. **No Windows evidence** for native-only operation of the later phases beyond Phase 111's one run.

## What is already true (a defensible partial claim)

Native import can replace OCCT *for reading* planar / cylindrical / conical / spherical / toroidal / B-spline / revolution / extrusion bodies and mapped-item assemblies, with volume within 0.1-0.3 % of OCCT's tessellation at a fine chord. Retirement would also need: native modelling or a decision to drop the editor operations, a real-file corpus pass, wiring of the native path into `fea_problem::solid`, and hosted Windows green with the feature off.

## Criteria to flip the question to "ready" (proposal for the owner)

1. Native read + tessellation + meshing is selected in `fea_problem::solid` with OCCT as fallback, and a corpus of at least 20 permissively licensed real STEP files (not NEMA17 / Halder alone) passes with a measured success rate the owner accepts.
2. Each OCCT-only editor operation is either implemented natively or explicitly dropped by a product decision.
3. `cargo test` of `solid-kernel` and `fea-problem` without `occt` is green on hosted ubuntu, macOS and Windows.
4. Owner approval recorded, then `occt_retirement_approved` set by the owner.


## Owner decision, 2026-10-19: the retirement criterion

The owner wants OCCT retired, on this condition: **the mesh quality of the model at hand matches what the OCCT path gives, the solver can use the mesh, and it is efficient.** No percentage of a corpus is required; the test is per model, against the OCCT result. Operationalised (the agent's reading, to be confirmed by the owner when the evidence is in):

1. *Usable*: the native mesh passes the solver's own gates (`QualityThresholds::default()`: scaled Jacobian >= 0.1, edge ratio <= 50, size jump <= 6, conformity) and a static solve converges.
2. *Matches*: for the same model and element-size target, the native mesh's quality (minimum scaled Jacobian, worst radius ratio) is no worse than the OCCT path's within a stated margin, and a benchmark solve agrees with the OCCT-mesh solve to the discretisation error.
3. *Efficient*: element and degree-of-freedom counts and solve time are within a stated factor of the OCCT mesh for the same accuracy.
4. Each editor operation native or dropped; tests green without `occt` on ubuntu, macOS and Windows.

A file the native path cannot mesh to this standard keeps OCCT as fallback until fixed; OCCT is removed when the measured evidence satisfies 1-4 on the agreed corpus. `occt_retirement_approved` is set only then, in its own PR with the evidence (the owner's conditional approval is recorded here; it does not by itself set the flag).

### Evidence against the criterion, 2026-10-19 (`docs/issue-46-phase-124.md`, `fea-problem/tests/occt_vs_native.rs`)

OCCT supplies only the surface triangulation here: both paths then use the same `mesh_volume` and the same solve, so the comparison isolates the native tessellation. 19 models (OCCT-built, native-built, the `solid-kernel` fixtures and the real NEMA17 bracket), same STEP text, same element size, linear static benchmark (fixed face, traction on the opposite face).

1. *Usable*: all 19 mesh and solve (equilibrium <= 1e-12) through the native tessellation, including NEMA17 (204 686 tets against 204 552, strain energy 0.3 % apart). **Absolute gate not met by either path**: `QualityThresholds::default()` (sJ >= 0.1) fails on every curved model, 0.2-5 % of the elements, all in the boundary layer of `fea_core::tetmesh` (OCCT kept would fail identically). Met relative to OCCT.
2. *Matches*: minimum sJ within x1.2, radius ratio within x1.8, compliance within 1.5-5 %, energy 0.1-6 %; asserted by `the_native_path_matches_the_opencascade_path_on_quality_size_and_the_benchmark_solve`. Weak spot: models with B-spline intersection edges (cross-bore, t-junction, tilted hole) get a native surface triangulation 5-45 x larger than OCCT's because `analytic_step::sample_spline_adaptive` seeds `2 * degree` samples per knot span (169 points on a 9.5 mm edge where about 20 suffice): up to 6.8 x more sub-gate elements on the plain cross-bore at the default size.
3. *Efficient*: tets and DOFs within 5 %, solve time equal, meshing 0.9-1.2 x OCCT's on most models and 2.2-2.7 x on the spline-edge ones.

Also found, shared by both paths: a load face meeting a tangent fillet gets +16 % (OCCT) / +26 % (native) of its area at element = fillet radius / 2 (`face_of_triangle` tolerance, hypothesis in the phase doc); the import's default element size ignores fillets and both paths refuse such a part until the size is reduced.

## Addendum, 2026-10-20: current implementation and retirement gate

The 2026-10-18 claims that the application never calls the native pipeline, that STEP import/edit requires OCCT, and that native modelling has no replacement are superseded. Without the `occt` feature, `fea-problem::solid::{tessellate_step,import_step,apply_op}` delegates to `native_solid`. `Problem::from_step`, `Problem::apply_solid_op` and `solid_mesh` use that route, and the solid editor calls those APIs. Native edits cover a through-hole or boss on a flat face and filleting all convex edges of a supported solid. They do not establish parity with all of `solid-kernel::occt`'s general booleans, profile modelling, fillets/chamfers and export. With the `occt` feature, the application still selects OCCT directly; there is no per-model native-quality certification or automatic OCCT fallback.

The Phase 124 19-model comparison predates Phase 125's opt-in CAD boundary repair and fillet-rim correction. Its absolute quality-gate and loaded-face failures must not be presented as current measurements. Phase 125's focused sphere/cylinder/torus repair and filleted-box tests passed locally without OCCT, but the 19-model OCCT-versus-native comparison has **not** been rerun after that change. No current per-model OCCT parity or Windows native-only qualification is established here.

The ABC 300-file sample now provides broader, separate diagnostic evidence (`issue-46-abc-corpus-2026-10-18.md`). With PLC refinement, 149 files conformed, 30 passed the solver-quality gate and all 30 completed the static benchmark solve; 14 timed out. This is not the `fea-problem::solid_mesh` production path and does not meet the owner's per-model quality/efficiency criterion. The corpus models cannot be redistributed in the repository under the available licence information. The earlier proposed minimum of 20 licensed files was superseded by the owner's per-model criterion above; any eventual retirement decision still needs a defined, reviewable model set and measured outcomes.

Remaining work for #63: run the post-Phase-125 same-STEP OCCT/native mesh-and-solve comparison on a machine with OCCT, including quality, face loads, element/DOF counts and time; fix confirmed failures without relaxing gates; qualify native-only builds and solves on target platforms; resolve unsupported editor operations explicitly; and obtain the separate owner approval before removing OCCT or setting `occt_retirement_approved`. The flag remains `false`.

The [2026-10-21 chart diagnosis](issue-46-phase-126.md) narrows two ABC refusals to projected B-spline/revolution trim overlaps; it does not change this retirement verdict.

## Addendum, 2026-10-21: focused post-Phase-125 comparison

[Phase 127](issue-46-phase-127.md) reran the same-STEP comparison on the planar box and filleted cylinder. Both OCCT and native paths passed the default mesh quality gate; the box matched exactly, and the filleted cylinder differed by about 0.3% in elements and strain energy. This supersedes the older absolute-gate result for that filleted-cylinder case only. The other 17 models, Windows native-only builds and solves, and the two ABC chart refusals remain open. The retirement verdict and approval flag are unchanged.

## Addendum, 2026-10-21: full comparison and bounded repairs

[Phase 128](issue-46-phase-128.md) records the full post-Phase-125 19-model comparison and three focused repairs. All OCCT/native paths meshed and solved, but 12 of 38 meshes failed the default quality gate. The tilted hole is a native-only gate failure; the plain cross-bore is an OCCT-only gate failure. Both paths fail on five other models. A hosted feature-off Windows executable build passed, but no Windows mesh or solve ran. Synthetic seams now chart three revolution faces of local ABC file `00007062`; the full file still stops on later cone/torus band topology, and `00006600` still refuses during grading. The retirement verdict and separate approval requirement are unchanged.

## Addendum, 2026-10-21: shallow cone seam and focused quality diagnosis

[Phase 129](issue-46-phase-129.md) repairs the next `00007062` cone-band seam refusal with a synthetic regression. The file now reaches sampled-shell/PLC recovery, where the diagnostic zero-second timeout cancels it. The tilted-hole native-only quality failure and the shared box-with-boss quality failure were reproduced; a proposed stagnation fix failed a controlled test and was reverted. `00006600` remains a projected-chart overlap. No new per-model solver-quality or Windows mesh/solve evidence is established; the retirement verdict and approval flag are unchanged.

## Addendum, 2026-10-22: repair gain and cancellation

[Phase 130](issue-46-phase-130.md) makes the native box-with-boss mesh cross the default scaled-Jacobian gate with a verified boundary-repair gain. Its OCCT path still fails that gate, and the native tilted-hole failure remains; a controlled chart toggle proved causality but was reverted to preserve an overlap guard. PLC now checks cancellation before initial Delaunay construction. These scoped changes do not qualify OCCT retirement or change its approval flag.

## Addendum, 2026-10-22: unresolved chart and boss-rim failures

[Phase 131](issue-46-phase-131.md) traces the tilted-hole's single sub-gate tet to a cylinder chart refinement chain seeded between opposite rim edges. Two narrower chord rules left its quality unchanged and were reverted. [Phase 132](issue-46-phase-132.md) localizes shared plate-with-boss failures to the boss/plate rim; its measurements used a temporary Phase 131 source snapshot and establish no production improvement. The default quality gates, Windows mesh/solve requirement, and separate retirement approval remain open.
