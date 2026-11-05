# Issue 234: corrosion allowance and mill undertolerance

The native Pressure Vessel workbench accepts corrosion allowance in inches and mill undertolerance as a percentage. Both default to zero. Corrosion must be nonnegative; mill undertolerance must satisfy 0 <= value < 100. Invalid inputs or exhausted wall clear checks and the contour.

## Geometry convention

The existing model takes nominal outer diameter and wall thickness. It derives the bore from those inputs. The new solver-owned `WallAllowance` holds validated loss inputs and computes:

```text
remaining wall = nominal wall * (1 - mill undertolerance / 100) - corrosion allowance
analysis outer radius = nominal outer diameter / 2
analysis inner radius = analysis outer radius - remaining wall
```

Mill shortfall applies before corrosion. All loss is assigned to the inside surface, keeping entered OD fixed and enlarging the bore. This models uniform internal wall loss. It does not model external corrosion or localized thinning.

Failure checks, thermal stress, external-pressure buckling, classification, through-wall numbers and the drawn contour share that remaining geometry. Nominal geometry must also be valid before losses are applied. The result card and report distinguish nominal and remaining wall.

## Minimum wall convention

The existing pressure-only thickness solver retains its fixed analyzed bore convention. It changes outer radius to find the minimum remaining wall, then converts that wall to nominal procurement thickness:

```text
minimum nominal wall = (minimum remaining wall + corrosion allowance) / (1 - mill undertolerance / 100)
sizing outer diameter = 2 * solved outer radius
```

The cards and report display both minimum wall values and the sizing outer diameter. The minimum nominal wall belongs with that sizing OD; it is not a replacement wall at the original entered OD. A regression reconstructs the sized nominal vessel with both allowances and verifies all pressure margins meet the requested minimum.

This minimum solve excludes thermal stress and buckling. Those checks still evaluate the entered vessel at its reduced wall. This increment makes no ASME compliance claim.

## Persistence and interaction

Case format 2 stores both new values in the existing named-number map. Older readers reject format 2 rather than silently ignoring wall loss. Format 0/1 cases still open. Missing allowance fields reset to zero even when applying an older case over edited state. Save/reopen, recovery and undo use the same case snapshots.

## Verification

All commands ran in the main checkout on macOS arm64 with its own Cargo target.

- `cargo test -p pressure-vessel-solver`: 51 passed. Includes zero loss, corrosion alone, mill shortfall alone, combined losses, invalid ranges, exhausted wall and nominal inversion.
- `cargo test -p app-tui --lib pressure_vessel`: 30 passed. Includes independent thermal/buckling comparison using the reduced cylinder and native click/type routing.
- `cargo test -p app-tui --lib gpu_viewer::editor::vessel`: 10 passed, including the final nominal-candidate sizing regression.
- Native window input test: passed. It clicks both new rows, types values and verifies remaining wall, bore and changed contour.
- `FEA_SNAPSHOT_VESSEL_OUT=/tmp/pv-corrosion-snapshots cargo test -p app-tui --lib vessel_snapshot -- --ignored`: 1 passed after the final display change. Eight images rendered; final allowance inputs/results and narrow image inspected.
- Full `cargo test -p app-tui`: passed: library 1212 passed and 31 ignored; binary 1 passed; integration 35 passed; doc tests 0. An earlier redundant run was stopped intentionally; the retained full run exited successfully.
- `git diff --check`: passed.

The test filters overlap. Their counts are not additive. Native input is exercised through the window input controller, and snapshots use the real GPU renderer headlessly. This does not establish Windows runtime evidence or an interactive OS-window check.

For OD 6 in, nominal wall 1 in, corrosion 0.125 in and mill undertolerance 12.5%, the remaining wall is 0.7500 in and bore diameter is 4.5000 in. At 5000 psi internal pressure, the analytical Lamé bore hoop stress is 17857.142857 psi. The reduced-wall contour matches within its f32 display tolerance. Inputs and results were visually inspected at 1440 x 900; the default form was also inspected at 860 x 760. Local PNG artifacts live in `.audit/issue-234-corrosion/`.

An additional verified defect was fixed in `CylinderGeometry::new`: it accepted infinite outer radii, allowing nonfinite stress results. It now rejects nonfinite radii. The solver regression covers NaN and both infinities.

Production changes are confined to the Pressure Vessel solver geometry, its UI model/report, native cards/case handling, related native tests and snapshot coverage. No FEA kernel, solid kernel, protected qualification sources, branches, commits or GitHub state were changed.
