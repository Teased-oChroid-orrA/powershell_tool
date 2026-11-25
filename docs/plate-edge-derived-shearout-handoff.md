# Handoff: Plate Edge derived shear-out plane and elastic-pin interference FE

Written 2026-11-24 (manager session). Self-contained: the next engineer or assistant needs nothing from the earlier chats. Tracking issue: https://github.com/Teased-oChroid-orrA/powershell_tool/issues/381 (Plate Edge: finish derived shear-out plane and elastic-pin interference FE).

## 1. Where things stand

| Item | State |
|---|---|
| PR 378 (Plate Edge workbench), PR 379 (window layout) | merged |
| PR 380 (in-plane free-edge distance in `solid-kernel`, Assembly Joints shows it) | merged as `0e65205` |
| This task (derived shear-out plane + elastic pin interference) | branch `plate-edge-derived-shearout` on origin, commit `33e7f4d` (WIP, not for merge). No PR. 4 `app-tui` tests fail. No docs, snapshots or CI yet. |

Branch is cut from `main` at `d36a101`; `main` has since moved (PR 380 touches `solid-kernel`, `fea-problem`, `docs/edge-distance-crosscheck.md`, none of which this branch edits except possibly the docs file): rebase is NOT allowed by repo rules on shared branches, so merge `origin/main` into the branch (plain merge commit) or open the PR as is and let GitHub report conflicts.

Repo rules (root `CLAUDE.md`): Fix-Everything Policy (fix verified bugs found on the way, report them separately), squash-merge after tests pass, never force-push, never rewrite history, never delete under `docs/`, never touch other crates' frozen behaviour. CI = `qualify` on ubuntu/macOS/windows via `scripts/qualify.py` (25-65 min, restarts on every push: batch commits). `qualify` also compiles app-tui lib tests WITHOUT the native-window feature, so any test using egui must be `#[cfg(feature = "gpu-viewer")]`; run `cargo check -p app-tui --no-default-features --bins --lib --tests` locally before pushing.

## 2. The task (what the owner asked)

(1) Derive the shear-out plane from computed fields instead of the assumed 40 deg (`PLANE_ANGLE = 40.0` in `app-tui/src/gpu_viewer/editor/plate_edge/inputs.rs`):
 - (a) edge-check: scan plane inclination and where it leaves the hole wall over the elastic field; take the plane of least shear-out capacity; test the scan against the closed form on a limiting case and that the angle is stable under mesh refinement.
 - (b) from the elastic-plastic FE (2D plane strain / plane stress bracket, and 3D) extract the shear path (ridge of equivalent plastic strain), report its angle and the load at which mean shear along it reaches Fsu; if the FE cannot carry what is needed, say exactly what is missing.
 - (c) draw the planes in the picture and result view at the derived geometry (both halves, mirrored); show angle and its source in the shear-out row ("derived from elastic field" / "derived from FE"; the handbook 40 deg stays as the handbook column, labelled so).
(2) Remove the window restriction "FE not run: the 2D/3D bracket FE models a clearance or line-to-line pin ..." as far as physics allows: model the pin as an elastic body pressed in by interference contact, then push it toward the edge as the rigid pin does today. 2D first (plane strain + plane stress); 3D: try with a cost cap and report honestly whether it converges.

Constraints: the legacy `Fbru + 0.8 p` edge check stays the governing check; nothing is promoted (the FE is advisory). Numerical changes need numerical proof (limiting cases, differential tests); never loosen tolerances to hide a failure.

## 3. What is built (all in commit `33e7f4d`, compiles)

Read the code with `git show 33e7f4d --stat`. Per area:

**edge-check**
- `edge-check/src/shear.rs` (new): `scan(src, geom, mat, fit_pressure) -> ShearScan { best, classical (40 deg, parallel), tangent (90 deg), evaluated }`; `ShearPlane { start_deg (phi), incline_deg (psi), length, capacity, edge_y }`. Grid phi 10..90, psi -20..40, 4 halving refinements. Capacity from mean shear by Gauss quadrature of `Stress::shear_along`, closed-form capacity per the pin-case rule of `FieldResponse`.
- `edge-check/src/modes.rs`: `ModesResult.shear_scan: Result<ShearScan,String>`; the elastic column of the `shear_out` row is now the scan's best capacity (fallback: old tangent value). Handbook column unchanged (40 deg). Legacy `Mode::ShearOut` for the Bushing window untouched.
- Tests (pass, `cargo test -p edge-check`): scan at psi = 0 reproduces `2 Fsu t (e - a cos phi) / ((2 phi + sin 2phi)/pi)` within 1 % (e/D 1.5, 2, 3); tangent plane equals the `FieldResponse` ShearOut capacity; best plane beats 40 deg and tangent references; picked plane stable under mesh refinement (phi within 5 deg, psi within 2.5 deg, capacity within 2 %); analytic and FE fields agree within 5 %; `edge-check/tests/modes.rs::the_shear_out_elastic_column_is_the_derived_plane`.
- FINDING to record in docs: the minimum is not at 40 deg and not at the tangent. e/D 2, 7075, t = 0.125, a = 0.125: best phi 57.5 deg, psi -13.4 deg (planes converge toward the load line), capacity 5128 lbf vs 6400 (handbook 40 deg parallel, same field) vs 6000 (tangent). e/D 1.5: 49.4 / -17.8 deg, 3606 vs 4421 / 4500. e/D 3: 63.8 / -9.1 deg, 8126 vs 10358 / 9001. The minimum sits on the family boundary (planes of the two halves just meet the load line at the free edge, `edge_y` ~ 0). Justification: planes of the two halves must not cross before the edge; diverging planes need the plate to open. Parallel planes (psi 0) minimum is at phi ~ 60-70 deg. The boundary is reported, not hidden.
- CORRECTION to earlier notes: the elastic column did NOT use 40 deg. It integrated xy on the horizontal tangent planes y = +-a, x in [0, e] (`edge-check/src/field.rs`, `FieldResponse`). 40 deg is the angular position on the bore where parallel planes start, used only by the handbook closed form `2 Fsu t (e - a cos th) - p D t` in `modes.rs` and `models/allowable.rs`. The docs (`docs/plate-edge-workbench.md`) must be corrected on this point.

**fea-core**
- `fea-core/src/nonlinear.rs`: `StepEvent<'a>` now carries `u: &[f64]` (end-of-step displacements) and `before: &NlState` (material state at step start); `StepObserver` is `Arc<dyn for<'a> Fn(&StepEvent<'a>) + Send + Sync>`. Other user: `eccentric-bushing/src/control.rs` (must still compile: `cargo check -p eccentric-bushing`).
- `fea-core/src/locate.rs`: `Locator::nl_stress_at(model, u, old, x) -> Option<[f64;6]>` (plastic return map at a point from the nearest Gauss point's old state).

**lug-solver**
- `lug-solver/src/shear_path.rs` (new): `ridge(mesh, plastic, a, edge)` (strip maxima of equivalent plastic strain across the ligament, weighted line fit, intersect with wall; candidates within 15 deg of the load line are excluded as bearing crush; keeps 0.15 of peak; needs strips spanning >= 50 % of the ligament else `Err("the plastic zone off the load line spans N % of the ligament: it has not reached the free edge, so there is no shear path to read")`) and `trace(model, steps, ridge, half_t)` (mean shear along the path at every step; 3D averaged through thickness with Simpson z = 0, h/2, 0.999h). `ShearPath { start_deg, incline_deg, length, wall, edge_pt, peak_plastic, curve }`.
- `lug-solver/src/bracket.rs`: `Out.shear` and `Refinement.shear`; `try_run_with(case, ideal, Interrupt)` with a deadline; the pin stage retries on Newton stall over `PENALTY_LADDER = [10, 3, 30]` (units E/a; interruption is not retried). Reason (measured): plane-stress bushed collapse stalled at one penalty and converged at its neighbour (delta 0.0008 stalled at 10, converged at 3 after 47 s; delta 0.003 converged at 10, stalled at 3, 30, 100); loads of converging penalties agree within 0.3 % (2693 vs 2686 lbf).
- `lug-solver/tests/edge_3d_bracket.rs`: struct literals fixed (`shear: Err(String::new())`).
- Throwaway probes still in the tree, delete before the PR: `lug-solver/tests/zz_probe_ridge.rs`, `lug-solver/tests/zz_probe_pin.rs` (turn their useful parts into the real tests below).

**app-tui window** (`app-tui/src/gpu_viewer/editor/plate_edge/`)
- `scene.rs`: `Planes{wall, edge, source: PlaneSource::{Elastic, Fe(label), Tangent}}`, `planes()`, `caption()`; dashed planes wall to edge in picture and result view, both halves mirrored. `mod.rs`: `PlateEdgeSession::planes()`, caption "shear-out planes (derived from elastic field)" / "(derived from FE, strain)" / "(tangents, not derived yet)".
- `cards.rs`: new "Shear-out plane" card (handbook 40 deg row, derived-from-elastic row with 40 deg / tangent comparison and the boundary note, one derived-from-FE row per FE line or "no path: why"); shear-out row of the allowables card labels columns "handbook 40 deg planes" / "elastic field, derived plane" and fills the FE cells (load at Fsu, or "N % Fsu", or "-").
- `inputs.rs`: `bracket_refusal` now refuses only a custom contact pressure (no diametral interference to impose; converting by Lame would give a pressure 7 % off what was typed); `bracket_case` builds `Bush{ inner_dia: PIN_CORE * D, pin E/nu, interference_dia: delta, mu: friction }` for delta > 0. `PLANE_ANGLE` kept for the handbook column only.
- `run.rs`: each idealisation stands alone (a stalled plane stress no longer blocks 3D).

## 4. Measurements so far (use them in docs and tests)

FE shear path (default case 7075 plate, pin 0.25 in, release build):
- Plane strain, e = 0.5: path start 42.8 deg, incline +7.8 deg, mean shear 28.9 ksi of Fsu 48 ksi (60 %) at 0.1 D, load at Fsu not reached. e = 0.375: 61.4 deg, -2.4 deg, 38.1 ksi (79 %).
- Plane stress (e = 0.5): no path (plastic zone off the load line spans 22 % of the ligament). 3D unbushed (e = 0.5): no path (30 %).
- So the FE says: bearing governs, shear-out not reached within 0.1 D travel; 2D plane strain gives an angle and %Fsu. Limits: the FE has equivalent plastic strain only (no plastic strain tensor); the ridge is at mesh-node resolution; the first-step mean shear is within ~16 % of an equilibrium estimate and `trace` has NOT been validated further (test missing).

Elastic-pin interference, 2D, e = 0.5, 7075, pin E 29e6 psi nu 0.32, ring around a 0.4 a rigid core, mu 0.12:
- line-to-line rigid pin: plane strain 4128 lbf, plane stress 2690 lbf.
- delta 1e-5: 4127 / 2694 (fit 244 psi) so the zero-interference limit reproduces the rigid pin within 0.1-0.2 %.
- delta 0.0008: 4132 / 2693 (fit 19.6 / 18.6 ksi); delta 0.003: 4146 / 2686 (fit 64.4 / 61.3 ksi). Collapse barely moves with interference.
- Lame solid-pin fit pressure (`edge_check::modes::fit_pressure`): 21.2 ksi at 0.0008 vs FE 19.6 ksi (-7 %, the ring with a 0.4 a rigid core is more compliant than a solid pin; edge-check's contact FE uses the same idealisation); 79 ksi at 0.003 vs FE 64 ksi (housing yields).
- `delta = 0` exactly with a bushing stagnates in the fit stage (residual 1e-10): never build a `Bush` for delta <= 0 (the window already does not).
- 3D unbushed default: collapse 3155 lbf, 17205 dof, 71 s.

3D elastic-pin interference (new, decides the 3D question): default case, delta 0.0008, 2 layers, 25-minute deadline: model 8140 nodes / 24420 dof, fit stage 5.1 s (fit pressure 18645 psi), pin stage did NOT finish: "FAILED after 1503.0s: the pin run stopped early: Interrupted". Decision: keep 3D restricted for interference with that measured reason; 2D runs.

## 5. What is wrong or missing (the checklist; the issue mirrors it)

1. **4 `app-tui` tests fail** (`cargo test -p app-tui --lib plate_edge`, 23 pass, 4 fail; log of the run: tests in `plate_edge/tests.rs`):
   - `an_interference_case_runs_in_the_bracket_as_an_elastic_pin` (line ~22): "the run did not finish". Cause: the run includes 3D for interference, which never finishes in test time (see 3D result above). Fix by implementing the 3D restriction in `run.rs` (item 3), then rerun; also strengthen its weak `assert!(fit > 0.0)` and drop the stale comment (e.g. assert plane strain collapse within ~5 % of the line-to-line numbers).
   - `a_custom_pressure_is_not_modelled_by_the_bracket_and_the_run_says_why` (line 215): an assertion got `None` where a refusal reason was expected. Check `bracket_refusal` against how the test builds the custom pressure input.
   - `the_window_is_a_boxed_workbench_with_inputs_results_and_menus` (line 271): the card list now has `plate-edge-shear`; update the expected list to `["plate-edge-allowables", "plate-edge-shear", "plate-edge-check", "plate-edge-notes"]`.
   - `the_results_name_each_collapse_number_by_its_model_and_show_plot_and_tables` (line 508): shear-out row now reads `("63 % Fsu", "-")` (the plane-strain FE cell has a value); update the expectation if the number is right (verify it against the Section 4 numbers: 60-79 % depending on edge distance).
2. **3D interference restriction**: in `run.rs` skip 3D for delta > 0 with the note "3D elastic-pin interference did not converge in 25 min (24420 dof): ..." and restore a 3D-only refusal text; keep 2D. Update the Limits text and the fit card.
3. **Missing real tests in `lug-solver`** (`tests/edge_3d_bracket.rs` style; use `--release` if debug is too slow): (a) elastic-pin fit pressure vs `fit_pressure()` Lame (state the -7 % ring deviation) and vs edge-check's contact FE (reuse the `default_bushing` pattern); (b) tiny delta (1e-5) reproduces the rigid line-to-line pin within ~0.5 % in plane strain and plane stress (measured 0.02 % / 0.15 %); (c) collapse trend over delta 1e-5 / 0.0008 / 0.003 (4127 / 4132 / 4146 in plane strain); (d) `trace` validation: first-step mean shear along a parallel plane vs the equilibrium `share(phi)` identity for a line-to-line pin, mu 0, tolerance measured; (e) `ridge` unit tests with a synthetic plastic field (band at a known angle; crush on the load line ignored; the "has not reached the free edge" error); (f) a test (or documented `#[ignore]`d ~50 s release test) that the penalty ladder retries (delta 0.0008 plane stress stalls at 10, converges at 3). Then delete the two `zz_probe_*.rs` files.
4. **Docs and AGENTS.md (never delete under `docs/`)**: `docs/plate-edge-workbench.md` (Evidence + Limits; correct the statement about what the elastic column integrates; add the derived plane and the converging-boundary finding with the numbers above; FE ridge results and limits; interference evidence, the plane-stress penalty ladder, the 3D result), `app-tui/src/gpu_viewer/editor/plate_edge/AGENTS.md`, `edge-check/AGENTS.md` (`shear.rs`), `lug-solver/AGENTS.md` (`shear_path.rs`, ladder, observer, interference), `fea-core/AGENTS.md` (`StepEvent` fields, `Locator::nl_stress_at`), optionally the last section of `docs/edge-distance-crosscheck.md`.
5. **Verification not yet run**: `cargo test -p edge-check -p lug-solver`, `cargo test -p app-tui --lib`, `cargo test -p fea-core` (observer/locate change), `cargo check -p eccentric-bushing`, `cargo check -p app-tui --no-default-features --bins --lib --tests`.
6. **Snapshots not re-rendered/viewed**: `FEA_SNAPSHOT_PLATE_EDGE_OUT=<dir outside the repo> cargo test -p app-tui --lib plate_edge_snapshot -- --ignored`, open the PNGs: planes at the derived geometry in picture and result view, both halves, caption names the source, shear card and shear-out row render; delete the `.rgba` files. Layout reference: `.audit/edge-workbench-mockup.html` on the main checkout (untracked). Offscreen render is the only visual check available.
7. **Process**: commit in few batches, push once, open the PR (no force), wait for all three OS in `qualify`, squash-merge, delete branch, remove the worktree `target/` (5-7 GB).
8. **Fix-Everything items to report in the PR**: the penalty ladder, "a stalled idealisation no longer blocks the others", the `Out`/`Refinement` literals, the corrected documentation of the elastic column.

## 6. Limits to restate in the final report

Offscreen snapshots only; the fastener library has spec Ftu/Fsu only; 3D with friction takes minutes (3D elastic-pin interference did not converge in 25 min); the legacy `Fbru + 0.8 p` check governs and nothing is promoted; the FE shear path exists only where the plastic zone reaches the free edge.

## 7. Open product question relayed from PR 371 (owner decides, no product code changed)

Keep the 2D lug and use 3D only as a bracketing/calibration reference. The 2D plane-strain lug over-predicts the 2 % offset yield by 13-20 % at friction 0.12; the 3D model is within -0.4 to +7.1 %; the requested 3D mesh took 54 min and 0.78 GB.

## 8. Other open item (not this task)

The free-edge worker's session cannot be archived by the app ("live work"); archive it from the sidebar. Old sessions of this task (`local_80a47576...`, `local_b5b7a00a...`) are idle and can be archived once this lands.

## 9. Practical notes

macOS, BSD `sed` (use `sed -i ''` or perl/python). Work in the worktree `.claude/worktrees/plate-edge-derived-shearout` (or a fresh clone of the branch); the main checkout may hold other uncommitted work. Disk was tight (94 % used): each worktree needs its own 5-7 GB `target/`; do not share one `target/` between checkouts with different source. Slice big files by explicit offsets when patching with scripts (a python replace to end-of-file once deleted `try_run`; recovered from git).
