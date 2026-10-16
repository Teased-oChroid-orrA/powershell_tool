# Native exchange regression fixtures

`tilted-cylinder.step` was generated for this project through `cadrum` 0.9 /
OpenCASCADE 8.0.1 using `Brep::cylinder([7,-4,2], 2, [1,2,3], 3)` and
`to_step_text()`. It declares millimetres; material is unspecified. The header's
fixed 1979 timestamp is exporter metadata, not provenance of an external design.

The rounded export is accepted by the analytic geometry checks but fails the
strict single-turn chart certificate. Explicit chart canonicalization within the
existing geometry tolerance must report every parameter adjustment, preserve all
world boundary samples, and pass sampled-facet intersection checks. The committed
fixture runs those regressions without requiring an OCCT-enabled test build.
