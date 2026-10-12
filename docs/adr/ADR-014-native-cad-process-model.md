# ADR-014: Native CAD window - process model and benches

Status: Accepted (user direction 2026-10-10: the native window becomes a SolidWorks-like front end with separate
benches; the terminal UI stays the launcher and the home of the toolboxes that are not ported)

## Problem

The native window grew three entry points (member editor, solid editor, result viewer) that were reachable only through
terminal keys, each with every menu of its own kind of model. Users could not find the editors (`w` was unadvertised),
the viewer duplicated what the solid editor shows, and a single menu bar carried every study's items.

## Decision

1. **Process model unchanged: the terminal spawns the window as a child process, the window owns the model while it is
   open, files are the interface.** The window is started with `--gpu-editor [file]` (members) or `--gpu-solid [file]`
   (solid); it saves to JSON (`MemberProblem` / `fea_problem::Problem::to_json`, the Workbench's own format); the
   terminal re-reads the solid file when the child exits (`AppEvent::NativeEditorFinished`). Rejected: IPC / shared
   memory (a second protocol to version and secure, no benefit while the hand-off is open -> edit -> close), and a
   resident window service (lifetime and focus problems on Windows, unrelated to the engineering value).
2. **Benches (`gpu_viewer/bench.rs`) are the unit of menu separation.** `Bench::{Members, Static, Modal, Buckling,
   Transient}`. A bench decides which Simulation menu entries and ribbon group exist; the model-kind specific menus
   (File, Edit, Insert, Tools) are the session's own. The four solid benches share one `SolidSession` (same
   `Problem`, same file): `Window > Bench:` switches among them in place, keeping the model. Crossing between the
   member and solid models loads the other model's file beside the current one (`editor::bench_file`) and is refused
   while there are unsaved edits (no silent data loss, no hidden autosave).
3. **One viewer.** Modal, buckling and transient studies run inside the solid editor (`solid/study.rs`, the same
   `fea_problem::dynamics` and `transient` solvers as the Workbench) and are shown by the same window, so the terminal
   keys `g` / `t` / `w` all open the solid editor. The separate result-viewer path (`Effect::OpenGpuScene`) is no longer
   reachable from the keys; its code is retained only for the `--gpu-viewer` file mode.
4. **Benches that need other solver crates** (lug, bushing press fit, eccentric bushing, fastened-joint preload,
   thermal conduction, material lookup) remain terminal toolboxes. Each becomes a bench by implementing `Authoring`
   over its crate's input model; that is a per-crate task (the crates own their inputs and acceptance gates), recorded
   in `docs/issue-12-phase-23.md`. No bench is faked: a bench exists only when its solver runs from the window.
5. **The advisor is per bench.** Its contexts, rules and recordings are member-model specific today; the Advisor menu
   exists in every bench and states why it is unavailable where no rules exist. Recording files are kept per bench
   (never mixed), recording stays opt-in and content-free, a trained model is used only after the measured gate.

## Consequences

* Adding a bench = a `Bench` variant, an `Authoring` impl (or a mode of an existing session), a registry entry in
  `Bench::ALL`; the window, Window menu and switching need no change.
* The terminal UI no longer needs to compute modes/transients for display; `n`/`b` still report them in text.
