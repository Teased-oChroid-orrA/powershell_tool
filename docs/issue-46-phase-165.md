# Phase 165: same-STEP comparison over a licensed real-CAD corpus on the production path

Issues #46 / #63. The owner parked the ABC pair (`00006600`, `00007062`) as known refusals on 2026-11-01 and redirected the work to the OCCT-retirement gate on the production path: the application's `fea-problem::solid_mesh`, measured by `fea-problem/tests/occt_vs_native.rs`. Branch `retirement-corpus-harness`, based on `main` at `1a59333` (Phase 164 docs, PR #214). No gate, tolerance, budget, cap or timeout was changed.

## Corpus

35 STEP files, downloaded with owner approval into temporary local storage. None is committed.

- 33 NIST PMI test models: the CTC 1-5, FTC 6-11 and STC 6-10 parts, each in several AP203 / AP242 exporter variants (`NIST-PMI-STEP-Files.zip` from nist.gov, 13,976,599 bytes). NIST states that these files can be used without restriction. Acknowledgement: the models are courtesy of the National Institute of Standards and Technology (NIST).
- 2 OpenCascade sample files, `screw.step` and `linkrods.step` (`data/step` in the OCCT repository, LGPL 2.1 with the OCCT exception).

That is about 18 distinct parts. Most are machined prismatic parts with holes, pockets, fillets and chamfers; several are multi-part assemblies.

## Harness changes

`fea-problem/tests/occt_vs_native.rs`:

- `STEP_DIR=<folder>` makes the `table` test run every `.step` / `.stp` file in that folder, at each file's own import default size, in place of the 19 built-in models.
- The benchmark holds the lowest flat faces normal to z (else x, else y) and loads the highest ones with a traction of 10 along that axis. Generated models only needed z.
- **All coplanar faces at the extreme level are now held or loaded** (`axis_faces`), not the first one by face number. With ties, the old pick depended on how each kernel numbers and splits faces, so OCCT and native loaded different surfaces. On `linkrods` this produced a 19 % displacement gap and a 13 % reaction gap that came from the harness, not from either mesh (see Results).
- The reaction is reported along the load axis (`R<axis>`), not always along z.
- An OCCT import failure becomes a refusal row instead of a panic.
- The last line is a summary: models, then meshed / default-gate pass / solved for each path.

Run (mesh only, then with solves; note that `NO_SOLVE=` set to an empty string still skips the solves):

```
STEP_DIR=<corpus> NO_SOLVE=1 cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native -- --ignored --nocapture table
env -u NO_SOLVE STEP_DIR=<corpus> cargo test -p fea-problem --features occt --release --locked --offline -j1 --test occt_vs_native -- --ignored --nocapture table
```

## Results

macOS arm64, 8 GB, release. Mesh-only pass 818 s; pass with solves 4,771 s (ftc_11_rb alone took about 49 min of it, both paths, swapping).

Summary: **35 models; OCCT meshed 3, passed the default gate 1, solved 3; native meshed 2, passed the gate 1, solved 2.**

### Files that meshed

| File | Path | Tets | Min sJ | Max edge ratio | Gate | Mesh | Solve | Benchmark |
|---|---|---|---|---|---|---|---|---|
| `linkrods.step` | OCCT | 84,014 | 0.100 | 12.4 | pass | 54 s | 31 s | umax 5.876e-5, U 1.028e-4, R 4.383 |
| | native | 84,011 | 0.100 | 11.3 | pass | 62 s | 34 s | umax 5.962e-5, U 1.039e-4, R 4.430 |
| `nist_ftc_11_asme1_rb.stp` | OCCT | 651,236 | 0.040 | 13.3 | fails (43 cells < 0.1, all on the boundary) | 286 s | 1,191 s | umax 7.3650e-5, U 0.49782 |
| | native | 647,264 | 0.022 | 17.8 | fails (32 cells < 0.1, all on the boundary) | 300 s | 1,147 s | umax 7.3663e-5, U 0.49771 |
| `screw.step` | OCCT | 167,236 | 0.019 | 19.1 | fails (43 cells < 0.1, all on the boundary) | 99 s | 51 s | umax 2.138e-3, U 0.2140 |
| | native | refused at import | | | | | | |

Equilibrium error at most 9.4e-13 on every solve. Where both paths solved, they agree: on `linkrods` (after the face-pick fix) displacement within 1.5 %, strain energy within 1.0 %, reaction within 1.1 %, element count within 0.01 %, native meshing 15 % and solving 12 % slower; on `ftc_11_rb` displacement and energy within 0.03 %. Both paths fail the gate on `ftc_11_rb` with boundary cells only; native's worst cell is worse (0.022 against 0.040) but it has fewer cells under the gate.

Before the face-pick fix, `linkrods` showed native umax 3.19e-5 against OCCT 2.67e-5 and reaction z -0.573 against -0.509. The loaded areas differed because each path took a different one of several coplanar top faces. With every coplanar face loaded, the gap is the discretisation difference above.

### OCCT refusals (32 of 35)

- 30 files: the import's own default element size asks for more elements than `solid_mesh` accepts. The refusal is immediate and says so, for example "about 122100062 elements: that is too fine (limit 400000); increase the mesh size". Cause: `finish_import` sets the default size to the smallest of diagonal / 18, wall / 3 and box side / 3, and the curvature floor in `feature_size` (`DEFAULT_TETS`) only bounds the curvature refinement, not this wall/box rule. A thin wall on a large part gives a size the mesher refuses. These files would need a manually coarser size, and that size then does not resolve the thin wall.
- 2 files (`ctc_02_ap242-e2`, `ftc_10_ap242-e2`): OCCT's tessellation is not a closed manifold (52 and 14,176 open or doubled edges).

**Negative result, not kept.** Flooring the default size at the size the 400,000 estimate accepts was tried. It removes the inconsistency between the import and `solid_mesh`'s estimate, but 29 of the 30 files then hit the volume mesher's own hard cap ("more than 2000000 tetrahedra: the size field is too fine"). The real element count is more than 5 times the V / (0.1179 h³) estimate on these parts, so they still do not mesh, and each refusal now costs about 50 s instead of under a second. The change was reverted. The fast, explicit estimate refusal stays. No limit was raised.

### Native refusals (33 of 35), every one at STEP import

| Strict-read refusal | Files |
|---|---|
| assembly/placement mapping is not implemented | 21 |
| multiple body representations require explicit mapping | 3 |
| parallel or unresolved placement directions | 2 |
| declared edge/surface geometry disagrees (4.17e-3 mm and 3.42e-4 mm) | 2 |
| ambiguous plane-angle units | 2 |
| native context scope requires exactly one supported body (`ftc_08_ap242-e1-tg`) | 1 |
| edge endpoint is not on its declared line (`ctc_03_rc`) | 1 |
| a spline edge must cover its whole curve (`screw`) | 1 |

The tolerant re-read refuses with the same classes, except that the two geometry-disagreement files and `ctc_03_rc` then stop on "ambiguous plane-angle units" or "surface chart work budget exceeded".

The assembly class is broad: `step_body::BodyContext::read_all` refuses any file that contains a `MAPPED_ITEM`, `NEXT_ASSEMBLY_USAGE_OCCURRENCE`, `CONTEXT_DEPENDENT_SHAPE_REPRESENTATION` or `REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION` entity, even when the file holds one part wrapped in a product structure, as most exporters write it. The plane-angle class is a file that declares degrees (with a conversion) where `analytic_step::plane_angle_factor` accepts only one explicit SI radian unit.

### Attribution of native-only failures

Every native-only failure on this corpus is at the **import** stage. No native-only failure was found at tessellation, volume meshing or quality on a file that native could read. The two files both paths read (`linkrods`, `ftc_11_rb`) meshed on both paths, with the same gate result on each.

### Built-in parity table

With the face-pick change, both release tests over the 19 built-in models (`table` and `the_native_path_matches_the_opencascade_path_on_quality_size_and_the_benchmark_solve`, `--test-threads=1`, 4,669 s; three other sessions were building on the same machine, so the times are inflated):

- `table`: 19 models; OCCT meshed 19, passed the gate 18, solved 19; native the same. **36 of 38 rows pass**, as on `main`. The two failures are NEMA17 (4 cells under 0.1 on each path, all on the boundary), deferred by owner request.
- The asserted comparison passes on all 12 models. At its 1.6 times coarser size, the edit plate and cross-bore fail the gate on both paths (8 / 8 and 2 / 1 boundary cells), which the assertion allows (native must pass wherever OCCT passes).

The change moves only supports and loads, so every mesh column is unchanged.

## What this shows and does not show

- It shows that on this licensed real-CAD corpus, native STEP import coverage is the dominant blocker for retirement: 33 of 35 files stop there, 21 of them on placement mapping.
- It shows that OCCT's own production path meshes few of these files at their default size (3 of 35), because of the import sizing rule above. A gate based on "native meshes what OCCT meshes" is therefore measured on very few real files at default sizes.
- It does not show native meshing quality on real assemblies, because native cannot read them yet. It was measured on macOS only; [Phase 167](issue-46-phase-167.md), run in parallel, covers feature-off STEP import, mesh and solve of the built-in models on hosted Windows, not this corpus. It does not cover the editor operations or OCCT's healing of the two non-manifold tessellations.

## Follow-ups

- Native STEP placement mapping for single-body files wrapped in a product structure, then for true assemblies (21 files). Degree plane-angle units with an explicit conversion (2 files directly, more after the tolerant read).
- Import sizing for thin-walled large parts: an owner decision, since a coarser default would mesh but would not resolve the wall (see the retirement-gate proposal in `issue-46-abc-and-retirement-gates-plan.md`).
- Editor-operation capability matrix (Phase 166, in progress). Windows feature-off qualification of the built-in models is Phase 167.
