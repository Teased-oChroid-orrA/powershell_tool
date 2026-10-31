# Phase 156: bisect spline-edge grading intervals

Issue #63, excluding NEMA17 by owner request. The official ABC v00 STEP archive contains `00006600_2516c959a43348f89c02cae9_step_000.step` and `00007062_a7b44768091a464bb7beff7f_step_000.step`. The archive's MD5 matched the publisher's `695388be7a278c7798c8c8ae239772ac`. The two files were extracted only to temporary local storage. The [ABC publisher](https://deep-geometry.github.io/abc-dataset/) says CAD copyright belongs to the creators, so the files are not committed here.

## Chart-grading diagnosis and change

On `00006600`, `AnalyticModel::graded` took a constrained-chart split fraction and inserted it at the same fraction of a spline edge's parameter interval. Repeated grading produced world points about `6.8e-10` mm apart while the edge-to-face projection residual was about `2.7e-6` mm. Independent face-chart projection then mapped adjacent edge points to the same chart point. The unchanged baseline reproduced `adjacent sampled trim segments overlap in bound #1066 segment 156`.

A diagnostic switch changed only spline-edge split placement to the parameter midpoint of each requested segment. On the same STEP file, grading passed and the 327-face shell was sampled; PLC recovery then cancelled as deliberately requested with `--conform-timeout 0`. The branch candidate uses that bisection without the diagnostic switch. Lines, circles and ellipses keep their chart-requested fractions, as does `insert_edge_splits` for its other caller. No overlap validator, model tolerance, solver-quality gate or PLC budget was relaxed.

The focused `chart_grade` and `spline_surfaces` tests passed on the candidate (3 tests, release, locked/offline). The `curved_edges` and `curved_step` suites also passed (6 tests). The candidate's `abc-corpus --one` run returned the same 327-face, 133,509-vertex, 267,030-triangle shell, with minimum shell angle `0.0855005°`, followed by intentional PLC cancellation. This is a chart-grading improvement, not a conforming or solver-quality mesh. A redistributable synthetic file that reproduces this edge/face projection mismatch has not yet been established; the actual-file A/B result and the existing fillet/spline grading tests are the regression evidence. Full touched-crate tests remain to run before any merge.

## Other findings and limits

The `00007062` graded shell reproduced the prior 48-face, 110,215-vertex, 220,430-triangle result with minimum angle `0.0316063°`. This does not justify relaxing the 18° input gate. With the shell cached, a protected PLC run used 17,644 Steiner points and cancelled at its 120-second limit. An unprotected run reached 352 Steiner points in 42.8 seconds, then refused because refinement descended below the feature-size floor around a small-angle input vertex. The shell and recovery times were measured separately; neither path returned a conforming mesh or a solve. Raising the timeout or relaxing the quality gate is not supported by this result.

Two compiler warnings in `fea-core::tetmesh` came from declaring `order` and `local` before a loop that always overwrote both. Their declarations now sit at the point of assignment inside that loop; snapping behavior is unchanged. OCCT retirement remains unapproved, and the broader native/OCCT solver and Windows native-only gates remain open.
