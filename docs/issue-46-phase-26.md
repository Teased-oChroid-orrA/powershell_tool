# Issue 46 — Phase 26: source-bound spline surface declarations

Source `0b19cd0` adds `BodyContext::read_spline_surface`. The caller supplies a document-scoped body-face reference, not an unqualified surface number or guessed unit multiplier. Both context and face references resolve against the exact source document before numeric topology lookup. Non-face references refuse; the surface reader uses the bound representation length factor. The returned declaration retains its actual source surface identity.

The existing equivalent planar-spline bracket regression now exercises this route. It verifies declared mm and metre scaling, unchanged knot domains, coordinate scaling at three independent parameter locations, non-face refusal, stale context and stale face refusal. Full AnalyticModel import still rejects the exact unsupported spline surface; no partial success is advertised. All previous declaration parsing/limits tests remain.

This binds declarations only: trims, regularity, exact surface/edge agreement, sampled error bounds and full B-Rep validation remain required. No production backend switch, dependency/tolerance change, hosted CI, main modification or OCCT retirement. Source-specific tests and consumer compilation are recorded in [verification](issue-46-phase-26-verification.json).

Final checks at `0b19cd0ac3bd3c57ecf4f808d9daec280673a81d`: **106 CAD-enabled / 89 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero failures/ignored checks.
