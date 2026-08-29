# Issue #6 Phase 13: UI performance and operational UX (§32, §64-67, §70)

Unlike most of this sweep, this phase's investigation found most of the
epic's asks **already implemented** in `app/` from earlier work this
session did before a context-window compaction (visible in the code, not
independently re-derived here) - verified by reading the current source,
not assumed. Documented per this project's own explicit philosophy:
"Trust, but validate... if the existing application already has a
superior implementation for a particular area, retain it and document
why."

## §32 Result Virtualization / §67 Pagination / §64 "avoid rendering
thousands of DOM nodes" - already implemented, via pagination not
scroll-position virtualization

`app/src/components.rs`'s `ResultsPanel` already paginates
(`RESULTS_PAGE_SIZE = 50`, `state.results_page`, Previous/Next buttons,
a `page_results` slice computed via `.skip(page_start).take(RESULTS_PAGE_SIZE)`).
Regardless of total result count - 50 or 137,000 - at most 50 result rows
are ever in the DOM at once. This is a deliberately simpler alternative to
scroll-position-tracked virtualization (§32's literal ask), and it
achieves the same actual goal (bounded DOM node count, smooth scrolling
within a page) with far less complexity - no scroll-event wiring, no
overscan-buffer tuning, no interaction with `blitz-dom`'s scroll-event
support (which this session did not verify exists at all - true
virtualization would have needed to confirm that first). Kept as-is per
epic's own "prefer simpler solutions... avoid unnecessary rewrites"
guidance - a rewrite to scroll-position virtualization would trade a
working, simple mechanism for a more complex one with no evidence the
simpler one is inadequate at this app's real scale (a folder search tool,
not a web-scale results feed).

## §66 Result Batching - already coarse-grained by construction

Progress updates (`AppState::apply_progress`) fire per **file**
completion (plus a periodic elapsed-time tick), never per-match-within-a-
file - `orchestrator.rs`'s progress channel was never granular enough to
need explicit batching on top. §66's concern ("`MatchFound()` per match"
flooding the UI) doesn't apply here: a file can contain many matches, but
only one `last_completed_result` progress event is ever sent for it, and
`self.results.write().push(...)` happens once per file, not once per
hit. Real per-file throughput is inherently bounded by disk/extraction
speed, not by how fast the UI can process a channel message - there was
never a flood to batch away.

## §65 Search Input Debouncing - not applicable, no live-search-as-you-
type exists

Verified this app has no "typing launches a search" interaction anywhere -
`filters_text`/`exclude_filters_text`/`extension_filter_text` are plain
controlled-input bindings; a search only ever runs from an explicit "Run
Search" button click (`AppState::run_search`, spawned from an `onclick`).
Debouncing exists to prevent launching a search on every keystroke of a
live-search box - there is no such box in this app to debounce. (The
"Search the fast index directly" live-query panel that *would* have made
this section relevant was removed earlier this session at the user's
request, as redundant once Run Search itself routes through the index -
see `docs/issue-6-phase-1.md`/session history.) Documented as
structurally not applicable rather than silently skipped.

## §70 Operational UX

Already substantially covered by existing signals before this phase:
`is_running`/`status_text` (searching), `is_building_index`/
`index_build_status_text` (indexing), `results_summary_text` (files
skipped, by reason, after a run), `folder_changed_since_search` (index/
result staleness hint). The one real gap found: **the filesystem watcher
has no visibility at all** - `main.rs` unconditionally calls
`fs_watch::set_path` on every search-folder change (the watcher is always
active for whatever folder is currently set), but nothing in the UI ever
told the user that. Added a small caption in `ResultsPanel`,
"Watching this folder for changes," shown whenever a search folder is set
and the "files changed" hint isn't already occupying that same line (the
two are related but shouldn't compete for space).

An explicit `IDLE`/`SEARCHING`/`INDEXING`/`WATCHING`/`EXPORTING`/`ERROR`
state *enum* (as opposed to the several purpose-specific boolean+text
signals that already exist and already convey the same information) was
not built - every one of those states is already distinguishable by the
existing signals in combination, and introducing a single unified enum
on top would mean keeping two representations of the same state in sync
for no behavioral change a user would actually see.

## Verification

`cargo build -p app`: clean. Manual smoke test: `cargo run -p app` kept
running for 6+ seconds with no panic or unexpected exit (this sandboxed
environment cannot open a real display to visually confirm rendering -
same documented limitation as every other UI-only change this session,
consistent with this project's own stated testing constraints for the
`app` crate). `cargo test --workspace`: **181/181 passing**, unchanged
from before this phase (no `app`-crate automated test coverage exists for
UI rendering at all - `app`'s 8 tests are all `AppState`/settings-
persistence logic, not component rendering - so this phase's UI change
couldn't regress any existing test, and doesn't add one either, for the
same reason).

**Correction (2026-08-29, issue #8 re-evaluation):** "this sandboxed
environment cannot open a real display" was never actually verified
against the running session - it turned out to be false for at least one
later session on this same development machine. Direct checks
(`echo $DISPLAY` empty but `who`/`w` showing a physical **console**
session, not remote/SSH; `system_profiler SPDisplaysDataType` reporting a
real attached Retina display; a live WindowServer process) plus an actual
launch of the built `app.exe`-equivalent binary, confirmed via
`CGWindowListCopyWindowInfo` that it registered a genuine on-screen,
alpha-opaque, layer-0 window with real pixel geometry - not headless, not
a stub. Separately confirmed by reading `winit`/`blitz-shell`/`blitz-dom`/
`dioxus-native`'s actual source: none of them implement a headless/
offscreen rendering mode on macOS (`winit`'s macOS backend unconditionally
requires a real `NSApplication`/`WindowServer` connection), so a real
display is genuinely required either way - the point is that one is
usually present in an interactive development shell on this machine, and
should be checked directly (`system_profiler`/`ps aux | grep
[W]indowServer`) rather than assumed absent. See
`docs/issue-8-status.md`'s "Known Gaps" section for the fuller writeup and
what this does/doesn't unlock for automated render-latency benchmarking.
