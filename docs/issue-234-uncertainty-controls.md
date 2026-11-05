# Issue 234: editable Bushing edge-check uncertainty

Implemented on 2026-11-04.

The terminal Results pane exposes **Uncertainty** (`v` / `V`), with percentage
controls for strength CV and each registered model whose default model-error CV
is nonzero. The native Bushing window exposes the same controls on **Checks**.
Defaults remain strength 5%, stress superposition 15%, and contact FE 7%.
Allowables retain their statistical 0% model-error CV.

Controls accept finite values from 0 through 100 percent. A zeroed model CV
remains editable. Values live in edge-check state, separate from engineering
inputs and `NumberTarget`; they last the session and do not enter saved cases or
engineering-input undo. The control labels define CV as standard deviation over
mean and state the percentage scale and session scope.

A changed value immediately makes the previous run stale, drops any in-flight
job, and starts a replacement after the existing 400 ms debounce. The last
chosen quick/contact model set is retained. Unchanged or rejected values do not
invalidate a result. The runner applies each override once in its shared
sampling path for Monte Carlo and recommended edge distances. Nominal margins
and capacity calculations remain unchanged. Reports and model/recommendation
tooltips show the CVs used by the exact run, including fractional percentages.

Also fixed: report export previously appended an edge check after engineering
inputs changed. Export now omits a stale cross-check for either input or CV edits.

## Verification

- `CARGO_INCREMENTAL=0 cargo check -p app-tui --lib --tests --locked` passed.
- `CARGO_INCREMENTAL=0 cargo test -p edge-check --locked` passed 61 tests;
  2 existing tests remain ignored.
- `CARGO_INCREMENTAL=0 cargo test -p app-tui --lib --locked` passed 1209 tests;
  31 existing tests remain ignored.
- `CARGO_INCREMENTAL=0 cargo test -p app-tui --test rendering --bin app-tui --locked`
  passed 36 tests (35 rendering and 1 binary).
- `CARGO_INCREMENTAL=0 cargo build -p app-tui --bin app-tui --locked` passed.
- The existing ignored GPU `bushing_snapshot` test passed with
  `FEA_SNAPSHOT_BUSHING_OUT` set. Inspected dark and light 1360 x 860 snapshots;
  all three CV controls are visible and the long contact label wraps cleanly.
- A fresh binary ran in a real 170 x 45 tmux session. Opened uncertainty using
  `v`, then changed strength to 12.5%, superposition to 25.25%, and contact to
  9.5%. Captured the live terminal before and after edits.
- `git diff --check` passed.

New regression tests cover changed reliability with preserved nominal capacity,
runner/report/tooltip propagation, zero and invalid CVs, cancellation/no-op edits,
in-flight job rejection, debounce, stale export, case serialization isolation,
terminal rendering, and native window clicks followed by typing.

Local visual evidence is under `.audit/issue-234-gpu/`, with terminal captures
`.audit/issue-234-terminal-controls.txt` and `.audit/issue-234-terminal-edited.txt`.
These are local artifacts, not release qualification. A native process reached
`FEA_GPU_READY`; direct CUA attachment timed out and CUA denied Terminal access.
No live CUA interaction is claimed. Native interaction is covered by the window
input-path regression and native drawing by the inspected GPU snapshots.
Windows runtime/packaged-release verification was not performed in this session.
