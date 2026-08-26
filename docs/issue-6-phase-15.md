# Issue #6 Phase 15: make HTML report generation optional

User-reported UX gap (not an epic #6 section): the HTML report was
written unconditionally on every run, with no checkbox to opt out - CSV
and JSON already had "Export CSV"/"Export JSON" checkboxes, but HTML
(the primary output) had none, even though "Open Report" already existed
as a separate, explicit action. The user's own framing: even with export
unchecked, the "Open Report" button should still work.

## What changed

- New `AppState.export_html: Signal<bool>` (defaults `true` - the app's
  prior behavior was "always generate," so unchecking is an opt-out, not
  a silent behavior change for existing users), new "Generate HTML
  report" checkbox in `SettingsPanel`, next to "Open report when done".
- `finish_successful_run` (`app/src/state.rs`) now branches on it: checked
  writes the report exactly as before (byte-count warning, `open_report_when_done`,
  etc.); unchecked skips the write but stores everything needed to
  generate it later in a new `AppState.pending_report: Signal<Option<PendingReport>>`
  (`PendingReport { path, settings, result }`).
- `open_report()` became async: opens immediately if `last_report_path`
  is set (already written), otherwise generates from `pending_report` on
  the spot, then opens - so unchecking "Generate HTML report" never turns
  "Open Report" into a dead button, matching the user's explicit ask.
  `has_report` (controls whether the button is enabled at all) now checks
  both `last_report_path` and `pending_report`.
- CSV/JSON export is unaffected - it already had its own checkboxes and
  writes independently of whether the HTML report itself was generated
  (both call sites already computed the report's base filename to derive
  `.csv`/`.json` paths from, regardless of whether the `.html` file
  itself gets written).
- `app/src/persistence.rs`: `export_html` persisted like every other
  checkbox, with `#[serde(default = "default_true")]` (not derived
  `bool::default()` = `false`) so an existing user's settings file
  predating this field loads as `true`, not a silent flip to "stop
  generating reports."
- Two call sites needed updating for `open_report()`'s new `async`
  signature: the "Open Report" button itself and the command palette's
  `Command::OpenReport` handler (both now `spawn(state.open_report())`).

## Verification

`cargo build --workspace`: clean. `cargo test --workspace`: **191/191
passing, 1 deliberately ignored** (unchanged from before this phase - no
automated component-rendering tests exist for `app`, same limitation
noted in every other UI-only change this session). `cargo run -p app`:
ran 6+s with no panic - this sandboxed environment still can't open a
real display to visually confirm the checkbox renders/toggles correctly,
the same documented constraint as Phase 13's watcher-indicator change.
