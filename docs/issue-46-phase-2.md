# Issue 46 — Phase 2: native STEP inventory and declared length units

Source `b78068c8269cd5d949cfd3a28fed34fbecf8147f` adds a native, read-only physical-file foundation for #51. It does **not** implement native geometric B-Rep import, validate a solid or replace the OCCT adapter. [Verification](issue-46-phase-2-verification.json) records the exact checks.

## Implemented

`solid-kernel::step::inventory` preserves all parsed simple/complex entity components, source IDs, entity offsets and header values. It checks mandatory header records, duplicate IDs, forward/dangling references and termination, and bounds file bytes, entities, values, nesting and strings before accepting a result. Unknown entity names remain in the inventory; unsupported physical-file syntax is an explicit error rather than silently dropping data. Integers retain i64 precision, reals must be finite and nonzero literals cannot silently underflow to zero. STEP-encoded strings are retained without pretending to decode them or establish CAD authoring provenance.

`Inventory::length_contexts` follows assigned-unit references, handles the supported SI/metre and explicit conversion-based length forms, verifies length dimensions and rejects missing/ambiguous/cyclic/unsupported definitions, nonpositive factors and overflow/underflow. Conversion labels do not determine factors. Multiple contexts remain separate; a later geometry adapter must bind each representation/body to the appropriate context and placement. Offset/derived subtypes are refused instead of ignoring their semantics.

The pure-Rust audit command runs without the OCCT feature:

```sh
cargo run -p solid-kernel --example step-inventory --locked -- nema17-motor-bracket.step
```

Its output explicitly says geometry/topology/import validity is not certified. It reports **3,088 entities, 38 ADVANCED_FACE declarations and context #3083 → unit #3084 → 0.001 metres per source length unit** for the supplied bracket. This independently confirms the declared millimetre scale; the authoring tool and material remain unknown. The input reader itself uses a bounded read.

## Validation and limits

The CAD-enabled solid-kernel suite passed **23 checks**, including nine new inventory/unit cases, existing surface checks, CAD booleans/STEP round trips and the CAD-to-FEA patch solve. The default-feature example also ran against the actual bracket. Tests cover quoted/comment/complex syntax, exact large integers, nonfinite/underflow numbers, input budgets, missing headers/references, separate unit contexts, conversion factors independent of labels, invalid dimensions, cycles and unsupported unit subtypes. One development regression caught a missing reference's incorrect source offset; it was corrected without weakening the check.

No additional hosted CI was dispatched for this portable read-only increment. The previous single Windows foundations job remains the Phase 1 evidence, not proof that these later tests ran on Windows. No application importer or existing unit conversion was switched to this new reader, and Cargo.lock/dependencies remain unchanged.

Remaining #51 gates include canonical geometric B-Rep construction, exact curve/surface/trim evaluation, body/context and assembly mapping, geometry and closed-manifold validation, conservative healing, entity/support corpus qualification and persistent assignment identity. Named DATA/external physical-file sections, binary literals and encoded-string interpretation are not implemented. Retain OCCT and its acceptance checks. Parsing and unit metadata are useful foundations, not production CAD acceptance.
