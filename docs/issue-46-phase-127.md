# Phase 127 — focused OCCT parity and ABC chart topology (2026-10-21)

This is a focused checkpoint after Phase 125, not a full retirement qualification. The worktree was `main` at `753404c`; no production code changed. The OCCT-enabled `fea-problem` comparison was built from the same checkout, with locked offline dependencies on macOS arm64. Each case ran with `CASE='<name>' rtk bypass cargo test -p fea-problem --features occt --release --locked --offline --test occt_vs_native -- --ignored --nocapture table` and exited 0.

| Model and path | Tets | DOFs | Min scaled Jacobian | Worst radius ratio | Mesh time | Solve time | Strain energy | Loaded-face reaction |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Box, OCCT | 32,256 | 146,319 | .512 | .638 | .58 s | 7250.0 ms | 5.89037 | -6000 |
| Box, native | 32,256 | 146,319 | .512 | .638 | .56 s | 6693.4 ms | 5.89037 | -6000 |
| Cylinder filleted top, OCCT | 51,677 | 233,469 | .122 | .037 | 7.87 s | 10215.9 ms | .0558567 | -382.016 |
| Cylinder filleted top, native | 51,842 | 234,684 | .120 | .040 | 8.42 s | 9568.3 ms | .0560105 | -382.571 |

Both models and paths passed the default mesh quality gate, with zero elements below scaled Jacobian .1. The box matched exactly in maximum displacement (1.98304e-3), energy and reaction. The filleted cylinder's maximum displacement was 3.30827e-4 / 3.31205e-4 (OCCT/native); edge ratio was 9.3 / 8.9 and size jump 1.8 / 1.8. Equilibrium error was at most 1e-12. The test execution times were 17.71 s for the box and 54.15 s for the filleted cylinder. These single-run timings are indicative only. In particular, the filleted cylinder now passes the absolute quality gate on both paths; the older Phase 124 result predates the Phase 125 boundary repair. The other 17 models were not rerun.

Two ABC chart refusals remain separate and correctly fail validation:

- `00006600`, face #379: `boundary_splits` requests splits by chart length; `insert_edge_splits` evaluates exact points on spline edge #11215; `SplineSurface::chart_loop` projects them onto the face. Repeated grading produced adjacent edge points about 6.8e-10 mm apart while their face projection residual was about 2.7e-6 mm. They map to one chart boundary point. A generic world-spacing filter previously failed the bracket quality or bounded-runtime checks; no safe replacement or redistributable regression was established here.
- `00007062`, face #92: this full surface of revolution has two one-edge closed circular bounds and no meridian seam edge. Bound #225's circle traverses opposite the revolution parameter; its chart path jumps across the full-period seam and then decreases. The current single planar-polygon trim cannot represent the annular band. Moving the initial seam image only moves the discontinuity. A valid repair needs a periodic-band representation or paired artificial meridian seam sides that preserve the original shared-edge topology, with a synthetic two-circle STEP regression.

Neither ABC source file is redistributed. No chart validator was relaxed and `occt_retirement_approved` remains false. Remaining evidence includes the full post-Phase-125 per-model comparison, Windows native-only qualification, and a supported design and proof for both chart cases.
