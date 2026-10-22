# Phase 128 — full OCCT parity and bounded chart repairs (2026-10-21)

Epic #46 / #63. The post-Phase-125 default-size OCCT/native comparison ran on macOS arm64 from `main` at `dfb7c02`, before the code changes below. Command: `CASE='' rtk bypass cargo test -p fea-problem --features occt --release --locked --offline --test occt_vs_native -- --ignored --nocapture table`. The 19-model table exited 0 after 2,219.89 seconds; [all 38 path rows](issue-46-phase-128-parity.txt) retain counts, quality, timing, displacement, energy, load reaction and equilibrium. The harness prints these measurements but does not assert the retirement gate.

All 19 models meshed and solved on both paths; maximum equilibrium error was 1.2e-12. Native differed from OCCT by at most 1.03% in tetrahedron count, 0.99% in DOFs, 2.04% in maximum displacement, 4.26% in strain energy, and 2.34% in reaction magnitude. Twenty-six of 38 meshes passed the default quality gate. The failures were:

| Model | OCCT minimum scaled Jacobian / sub-gate tets | Native minimum scaled Jacobian / sub-gate tets | Meaning |
|---|---:|---:|---|
| Plate with boss | .066 / 8 | .043 / 26 | Both fail |
| Native box + boss | .091 / 3 | .096 / 5 | Both fail |
| Native box + boss + hole | .046 / 77 | .042 / 95 | Both fail; one native interior tet |
| Plain cross-bore | .080 / 4 | .119 / 0 | Native passes, OCCT fails |
| T-junction | .032 / 40 | .024 / 53 | Both fail |
| Tilted hole | .121 / 0 | .065 / 1 | **Native-only gate failure**; the bad tet is on the boundary |
| NEMA17 motor bracket | .029 / 70 | .042 / 87 | Both fail; one native interior tet |

The other 12 models passed on both paths, including both filleted-box variants and the filleted cylinder. NEMA17 used 205,416/205,588 tets and 955,101/955,809 DOFs (OCCT/native), with strain energies 4.63904/4.62393 and reactions both -2400. Timings are single runs under concurrent build and test load: native/OCCT mesh-time ratios ranged .302–2.746 and solve-time ratios .404–5.512. The high solve ratio was the native box + boss + hole and is not isolated evidence of a solver-path cost. This table establishes the tilted-hole quality failure against the owner's per-model criterion; it does not qualify retirement.

The feature-off hosted Windows `app-tui` release build [passed](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/39709272962) at `dfb7c02`. It builds an executable but does not execute a Windows STEP mesh or solve.

Three focused code repairs followed the table:

- A revolution face with two full-winding circular bounds now receives a synthetic meridian seam along the generating B-spline interval between the circles. The source circles may bound an interior interval of the profile; endpoint and winding agreement are checked before changing topology. Redistributable vase-derived STEP tests cover full-profile and interior trims, including reversed bound order. On the local ABC `00007062` file, faces #92, #146 and #158 each changed to one seam-joined bound and their chart loops sampled successfully.
- Generated curved-chart boundary and interior midpoints now use Spade's coordinate-underflow mitigation. A real `00007062` midpoint had a chart coordinate 1.028e-43, below Spade's 2^-142 admission limit; this is cancellation, not a small geometric feature. A portable regression checks that the normalized midpoint is accepted.
- `fea-core`'s `REPAIR_TRACE` retry now rolls back a successful diagnostic probe instead of mutating the live mesh and repair counters. A focused test exercises a successful repair and verifies the state is unchanged.

After these changes, `cargo test -p solid-kernel --no-default-features --locked --offline -j1` and `cargo test -p fea-core --no-default-features --locked --offline -j1` exited 0. `cargo test -p solid-kernel --features occt --release --locked --offline --test swept_surfaces --no-run -j1` also compiled. The bounded local `ABC_GRADED_FIRST=1` run of `00007062` (`--foreign --graded --steiner 200000 --conform-timeout 0`) advanced beyond the three revolution faces but still stopped at `grade`: `holes and unseamed bands on cones and tori are not implemented`. This is not a conformed or solver-usable mesh. The ABC source files remain outside the repo because redistribution rights are unestablished.

The `00006600` grading refusal is still unresolved: adjacent spline-edge samples become about 6.8e-10 mm apart, below their roughly 2.7e-6 mm edge-to-face residual, so independently projected chart points can coincide. Changing nearest-point candidate deduplication cannot resolve that mismatch; a generic spacing filter previously harmed quality or runtime. The tilted-hole sub-gate tet also needs a focused mesh diagnostic: the native surface has 1,502 triangles versus OCCT's 144, but the current table cannot identify whether distance, face tagging or repair leaves that element. No tolerance or quality gate was relaxed. `occt_retirement_approved` remains false.
