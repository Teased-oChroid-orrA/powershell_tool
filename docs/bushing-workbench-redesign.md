# Bushing Workbench native redesign

## Goal

Make the native Bushing Workbench the sole Bushing UI. Keep engineering inputs, checks, recommendations, saved cases, and FEA handoff while retiring its terminal toolbox.

## Implementation

The window has inputs, a 3D model with axial and cross-section sketches, and results. The 3D geometry and Lamé stress display follow the input model. They do not alter the solver verdict. Quick FE runs only on request; an input edit marks its result stale.

The Send dialog writes the existing 2D plane-stress `Problem`. It keeps the case linked so the FEA Workbench can send edited inputs back to the same case. The 3D solid option stays disabled pending #241.

The window still reads saved reamer, material, and Bushing ID libraries. Their editor and import/export controls were terminal-only, so #242 tracks native management. The terminal Eccentric Bushing toolbox currently uses the default Bushing model; #242 also tracks a case handoff.

## Evidence and limits

The focused Bushing snapshot test passed on macOS with a native GPU adapter. It produced dark, light, 980 × 720 narrow, Send dialog, and solved Quick FE frames. The narrow frame keeps toolbar controls inside the center pane and shows both sketches; their small dimension labels remain a limit at that width. The Send dialog obscures model overlays while open. A real 1200 × 760 logical-point native window was launched and captured on macOS after the initial-size fix; its three panes and toolbar did not overlap.

`CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p app-tui --lib --locked -j 2` passed 1,195 tests, with 34 ignored after integrating the Search workbench from #239. The ignored `bushing_snapshot` passed separately with `FEA_SNAPSHOT_BUSHING_OUT=/tmp/bushing-redesign-snapshots`. The Windows executable build is dispatched after merge; macOS rendering cannot establish Windows rendering behavior.
