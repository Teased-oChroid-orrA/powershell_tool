# Issue 46 — Phase 52: shared source parameter-map queries

PR #79 merged at `c3adced`; tracking-only main `43d506d` is the base of `feature/native-cad-shared-parameter-queries`.

`step_parameter_map::ParameterMapRef` borrows an already source-bound plane, cylinder or spline map. World bounds dispatch to the actual map without geometric approximation, preserving each map's original unit/domain behavior. A shared declaration-to-UV-to-world path checks the exact source basis and retains declaration error offsets. Spline composition now delegates to that path instead of duplicating it.

`step_pcurve_distance::distance` accepts any of the three maps while preserving existing `&SplineParameterMap` call sites. The same bounded global search, decoded-once declarations, knot-cell partition, progress/cancellation and source checks serve analytic maps. Analytic maps consume no spline surface patches; spline maps retain their hard patch cap. Results remain global objective brackets/candidate enclosures, not unique projection or finite trim/master/body certificates.

Targeted tests pass for all 228 actual bracket p-curves (200 LINE / 13 CIRCLE / 15 spline) on their actual planes/cylinders, including explicit finite caller intervals, endpoint-near query bounds and monotonic work. An independent translated, axis-aligned plane query brackets exactly 2 mm and 2000 mm after source unit conversion with unchanged parameters. Basis/source mismatch, zero work/pending budgets and initial/final cancellation refuse. All eight prior spline-map tests pass. No development failure or tolerance relaxation; the known-distance fixture changes only its local map declaration and does not claim validity of the edited solid.

[Frozen-source qualification](issue-46-phase-52-verification.json) at `b3f2fcc`: 183 CAD-enabled / 166 feature-off tests, zero failures/ignored/warnings; app CAD library/tests compilation passes. OCCT remains production; no dependency or hosted CI. Next: shared finite source p-curve endpoint agreement, then complete master/trim/body and general meshing/quality/later gates.
