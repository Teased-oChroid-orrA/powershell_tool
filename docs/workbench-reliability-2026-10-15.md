# Workbench reliability and performance — 2026-10-15

User-authorized implementation on `codex/workbench-reliability`, based on main `c1e2881`. No merge is authorized.
The user subsequently supplied `nema17-motor-bracket.step` in main commit `9b90aa7`; the identical file is carried here.
Its original CAD authoring tool is unknown. Its STEP context explicitly declares millimetres; the OpenCASCADE header is
not evidence of its original authoring tool. Do not infer source units from a user's CAD working preferences.

## Scope and acceptance

1. STEP import: render its cached surface before generating a volume mesh, fit the camera on replacement, keep CAD source
   separate from the saved `.fea.json`, and test the supplied bracket. Inspect closure/welding and incomplete volume meshes;
   never weaken physical acceptance to make a picture appear.
2. Units: a native dropdown converts the complete physical model, including materials, region materials, prescribed
   displacements, mechanical/thermal loads, history and cached/external geometry coordinates. STEP coordinates are normalized
   to millimetres by the kernel; imported mesh files retain an explicit cumulative scale instead of being rewritten.
   Bushing handoffs normalize to the Bushing solver's inch/lbf convention. Material readouts include unit labels.
3. Idle drawing: honor egui repaint deadlines, animate at a bounded cadence, keep worker/file polling separate from drawing,
   and avoid a past-deadline busy loop when occluded. Verify with actual desktop interaction and process measurements.
4. Workers: expose true stages/elapsed time, keep cancellation visible until the thread exits, and check cooperative cancellation
   between static solve phases and transient steps. Factorizations themselves remain non-interruptible. A cancelled answer
   cannot be published, and a worker slot cannot start another thread until its old computation finishes.
5. Learning: background automatic/manual refits retain the current advisor until the replacement is ready and gated. Both
   journal implementations stream bounded records, ignore torn/oversized lines, retain a bounded byte window and compact
   large files. Preserve held-out session evaluation and deterministic fallback; do not claim simulated evidence is real.
6. Files: stage beside the destination, sync before atomic replacement, preserve the previous complete file, reject external
   edits before saving, and periodically write separate recovery snapshots. Recovery is explicit and undoable. Closing a dirty
   model offers Save and close / Discard changes / Keep editing; repeated Close cannot silently discard it.
7. Picking: cache a screen-space boundary-face BVH by model revision/camera/viewport; preserve brute-force depth/edge semantics.
   Record construction cost and repeated-pick timing separately, with exact result agreement.
8. Release qualification: `python scripts/qualify.py` runs workspace tests and terminal-only compilation and records commands,
   commit and exits; `--gpu` explicitly runs native adapter checks (snapshot-only utilities excluded), `--cad` includes OCCT.
   A manually dispatchable/PR workflow covers Linux/macOS/Windows. Clean-machine startup and real native interaction remain
   separate evidence. The README exposes the dashboard, workbenches, CAD feature and qualification process.

## Unit contract

Systems are inch/lbf/psi/degree F, mm/N/MPa/degree C and m/N/Pa/degree C; seconds are unchanged. Density is force*time^2/length^4
(mm/N uses tonne/mm^3, not kg/mm^3). Thermal power is force*length/time. Conductivity scales as force/temperature difference;
heat flux as force/length, volumetric source as force/length^2, and film coefficient as force/(length*temperature difference).
Absolute temperature uses the Fahrenheit/Celsius offset; expansion uses differences only. “Declare” remains an explicit
advanced reinterpretation, distinct from the automatic physical conversion dropdown.

Legacy STEP projects with unrecorded source-unit assumptions cannot be safely repaired by guessing. Re-import the source or
review its declared units and materials. Likewise, old thermal values entered under an undocumented convention require review.
A valid residual does not establish small-displacement validity, mesh convergence or correct input units: retain those warnings.

## Validation log (in progress)

The first broad run overlapped the initial implementation; it is **not** an untouched-main baseline. It completed 137 targets:
2,538 passed, 5 failed, 85 ignored, exit 101. Three failures assumed synchronous advisor training or the old unit buttons;
those tests are being updated for the new behavior. Two existing dashboard input tests discarded unapplied egui texture deltas;
the test harness now explicitly clears those deltas because it has no renderer.

An initial CAD compile failed because the dependency's built-in downloader could not resolve GitHub directly. The supported
`OCCT_ROOT` override works with a locally extracted upstream artifact. The 43,619,579-byte Linux artifact was downloaded over
HTTPS and checked against the GitHub release asset SHA256 `f76c341123fc008391d6230b5e9157c5531166ed4cf301f6484f69a44aee59c8`.
The next compile exposed a duplicated `delete_recordings` method introduced during the refactor; corrected. The complete
native bin/lib/tests compile with `--features occt` then passed. Subsequent source changes require fresh checks.

The actual supplied bracket initially failed closed-manifold validation at a small rounded corner (8 open/doubled edges).
This is preserved in `/workspace/reliability-evidence/nema17.log`. Welding previously treated rounded grid buckets as equality,
which can both split near vertices and merge vertices farther apart than the requested weld distance. Neighbor-cell distance
checks and translated-volume regressions are under validation; do not yet claim that they resolve this fixture.

Evidence lives outside the checkout under `/workspace/reliability-evidence/`. Final test counts, measured picking/idle behavior,
actual screenshots, remaining failures and platform limits must be recorded before completion.

## Current implementation and measured checks

The implementation is tracked in `65d9b08` (CAD/physical units) and `df837a1` (native responsiveness, persistence, picking and
advisor workflows). Issue #12 remains closed; this is subsequent reliability work. The fixture is identical to the one the
user added to main; no change to main or merge is authorized.

The bracket's original B-rep passes both OCCT topology and self-interference checks. Its coarse tessellation had coincident
interior curved-face chords, not an established invalid source model. Cleaning, sewing and ShapeFix at the original precision
did not resolve those chords. Global ultra-fine linear/angular trials consumed excessive memory and were stopped. The final
fix projects the midpoints of only coincident interior chords onto the exact trimmed kernel face: 968 -> 976 triangles,
38 faces retained, closed-manifold acceptance unchanged. The surface volume is 34,651.671 mm^3 versus independent OCCT
BRepGProp integration of 34,631.087 mm^3 (0.0594% difference). The default Tet10 mesh has 318,906 nodes / 204,552 elements,
with a 0.093% volume difference from the accepted tessellation. Four tiny faces have no boundary elements at this resolution:
`face31`, `face32`, `face35`, `face37`. The UI/report states this limitation; applying mechanical or thermal conditions to
unresolved faces is refused. Reducing the element size is a real refinement, not a change to numerical acceptance.

The first complete current workspace run executed 137 targets: 2,562 passed, 2 failed, 86 ignored. The failures exposed (1)
Abaqus default section thickness resetting to 1 after conversion, and (2) a default filleted-cube mesh outside the new 5% volume
guard. Abaqus now preserves the caller's default thickness unless the file supplies one; cumulative import scales also cover
embedded isotropic/orthotropic stiffness, compliance and expansion. The default CAD element-size estimate uses diagonal/18
and the wall-thickness cap. Both failing targets then passed: **12 solid tests and 10 units tests**, including analytical bar
response and external section conversion through mm/N/MPa, m/N/Pa and inch/lbf/psi. No test tolerance was loosened.

Native controller verification passed **369 tests**, including actual egui clicks on the unit dropdown, saved physical values,
background advisor replacement and persisted gate rejection. Separate explicit software-Vulkan tests/measurements passed
**9 tests** (snapshot-only utilities excluded). On this Linux CPU, 6,400 faces / 100 picks measured: BVH build 22.76 ms,
brute force 445.49 ms, cached queries 3.06 ms, with identical selections. These are debug-build measurements on a shared cloud
CPU, not a hardware GPU throughput guarantee. Software playback of 2,145 vertices / 4,096 triangles / 241 frames also passed.

Actual native-window screenshots under `/workspace/reliability-evidence/native-smoke/` show the supplied STEP body before
volume meshing, shaded/edge controls, the unit dropdown, generated volume mesh and Save/Discard/Keep editing close modal.
Saving the real mm-to-inch conversion produced E = 29,000,000 psi and CAD coordinate scale = 1/25.4. A settled five-second
post-meshing sample used 0.2% of one CPU core (~414 MiB RSS for that 318k-node model); an earlier three-second settled sample
had no CPU ticks. The initial three-second sample during final mesh/display preparation was 21.7%, so do not describe every
post-click interval as idle. Hover hints no longer hide an active worker's stage/elapsed time.

Two broad build attempts failed from the 32 GB writable-layer limit (linker bus errors followed by ENOSPC), not numerical
failures. Obsolete generated executables and failed linker temporaries were removed; debug sections were stripped from local
cached archives/executables without changing code or optimization. The final local qualification uses one job and a local cc
wrapper adding `-Wl,--strip-debug`. Hosted qualification uses supported Cargo profile debug=0 settings and two jobs to bound
artifact size. Logs preserve every failed attempt. A focused native compile also caught two test constructors still using a
scalar import scale after the typed `ImportScale` change; corrected. Final complete qualification is running and must be
recorded below before claiming completion.

The cloud environment's install/start draft was refreshed with the supported verified OCCT override, signed-package Mesa
Vulkan driver, current branch/handoff and runtime variables. Saving the draft is confirmed; publication and fresh-task
restoration are not claimed.

## Claude Code handoff

- Continue on `codex/workbench-reliability`. Preserve user files and keep main untouched. Inspect the latest commits and this
  document's final validation section before rerunning work.
- Read the root and crate AGENTS instructions. CodeGraph was unavailable; all call paths were inspected directly.
- Numerical contracts: unit conversion is physical; Declare deliberately reinterprets numbers. Keep thermal offsets,
  imported constitutive matrices/section thickness and STEP scale together. Existing ambiguous legacy unit inputs require
  review/re-import, not guessed magnitude corrections. The default material of an imported STEP remains generic steel and
  must be reviewed for the actual part.
- Cancellation is cooperative at documented safe boundaries. A running factorization/mesher can finish its current phase;
  the cancelled result cannot publish and its worker slot remains occupied. Keep progress visible through that interval.
- Persistence detects external content changes before saving and keeps atomic replacements, previous files and separate
  recovery snapshots. This is not a cross-process database transaction; do not claim simultaneous-writer serializability.
- Actual Linux software-window and adapter checks do not prove native macOS/Windows hardware or packaged startup. Hosted
  matrix results, if available, are compilation/tests, not hardware interaction. Do not label unrun snapshots or ignored
  numerical soaks as passed.
- Future product breadth is proposed in [the breadth roadmap](workbench-breadth-2026-10-15.md). It is not permission to merge
  or an assertion those capabilities are implemented. Camera fit/padding for rotated models in small windows is a future
  navigation increment; the current Reset camera remains a fixed preset. No camera preference redesign was bundled here.

## Completed local qualification and final verification ledger

The broad CAD-enabled Linux qualification completed successfully: **137 targets, 2,568 passed, 0 failed, 86 intentionally
ignored**. Both the workspace command and the terminal-only bin/lib/tests compile exited 0. The report records its starting
commit `df837a1` and `dirty: true` honestly: documentation and later follow-up work were present during implementation.
Do not describe that as an untouched-main baseline or an immutable final-head run.

A final performance review found that selection changes were invalidating the picking cache because it used the whole UI
revision. The cache now uses immutable mesh identity (a Weak reference, preventing address reuse without retaining obsolete
mesh buffers), camera and viewport. **370 native tests passed** after that change. An additional focused regression passed
for selection reuse, camera invalidation, replacement-mesh invalidation and releasing the previous mesh's buffers. The final
software-Vulkan command passed **9 tests** again. Profiling now reports the actual **208-byte** uniform payload; the earlier
64-byte printout was stale after display settings expanded. This changes measurement output, not GPU upload behavior.
The optional qualification GPU phase retains the selected CAD feature to avoid an unnecessary feature-switch rebuild.
The final terminal-only compile also passed after the cache change.

The final validation sequence freezes the committed worktree and runs `scripts/qualify.py --cad --gpu --jobs 1` once more.
Its exact-head, clean-worktree result is recorded in `/workspace/reliability-evidence/qualification-clean.json` and in draft
[PR #45](https://github.com/Teased-oChroid-orrA/powershell_tool/pull/45), alongside the current hosted Linux/macOS/Windows checks.
Read the PR's current validation status for that last run; do not substitute the earlier dirty-worktree report. No further
implementation edits are intended during this frozen run. Platform smoke limitations and optional ignored soaks remain as
stated above. The draft PR remains unmerged, and issue #12 remains closed.
