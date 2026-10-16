# Issue 46 — Phase 25: shared native body/unit binding

Source `c6bd290` extracts the existing strict single-body representation/context admission from AnalyticModel into `step_body::BodyContext`. Both paths now use one implementation. The immutable context exposes raw source-unit topology, a document-scoped length-context reference and its declared millimetres-per-source-unit factor. It does not normalize the raw coordinates or certify geometry.

Existing assembly/mapping, multiple-body, ambiguous representation, unsupported item and absent length-context refusals remain. No inferred units or fallback scale is introduced. Two new regressions verify millimetre/metre factors with unchanged raw point identities/coordinates, exact-document rejection after source edits, and duplicate/missing direct representation refusal. Existing analytic importer tests exercise the extracted path.

An initial compile error used Result-style `?` on the source-reference Option; corrected with an explicit missing-context error before qualification. No final code change to dependencies, tolerances, production backend or main. This is reusable context admission for subsequent spline declaration integration, not full spline-backed B-Rep support or assembly placement support.

Source-specific final results are recorded in [verification](issue-46-phase-25-verification.json). Native spline-body sampling/projection/trims and broader CAD/PLC/quality/P2/P3/final gates remain open. No hosted CI was dispatched.

Final checks at `c6bd290ca89f85320db2c7f6dcc57ce342c42967`: **106 CAD-enabled / 89 feature-off tests and CAD-enabled app lib/tests compilation pass**, zero final failures/ignored checks.
