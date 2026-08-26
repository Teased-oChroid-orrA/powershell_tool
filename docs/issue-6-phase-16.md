# Issue #6 Phase 16: fix two field-reported crash bugs

User report (2026-08-26, real Windows build, `C:\Users\xo180e\Desktop\organized`):

1. `IndexError: An error occurred in a thread: 'An index writer was
   killed.. A worker thread encountered an error (io::Error most likely)
   or panicked.'` when clicking "Build/update index".
2. Separately, and regardless of whether fast-search indexing is on: the
   app performs a search, shows results, then crashes the whole process
   ~5 seconds later, every time.

## Bug 1: concurrent IndexWriters on the same directory

Three independent code paths each open their own `NativeSearchEngine`
(own Tantivy `IndexWriter`) for the same on-disk index directory, with
zero coordination between them:

- The automatic post-search reindex (`finish_successful_run`, runs
  whenever "Index this folder for fast re-search" is checked).
- The explicit "Build/update index"/"Rebuild from scratch" buttons
  (`build_or_rebuild_corpus_index`).
- The 2-second incremental-reindex flusher
  (`run_incremental_reindex_flusher`/`reindex_changed_paths`), which runs
  in the background the *entire time* indexing is enabled, draining
  filesystem-watcher events.

Any two of these firing close together - very plausible in real use: the
watcher had presumably already queued pending reindex paths for an
actively-changing folder, and the user clicked "Build/update index" - end
up with two `IndexWriter`s open against the same directory
simultaneously. Tantivy's own writer lock doesn't reliably prevent this
across all platforms, and even where it does, the loser fails loudly
rather than queueing - and once one writer's background merge thread hits
a conflict, the whole writer is permanently poisoned ("killed"), matching
the reported error exactly.

Fix: a single process-wide `static INDEX_WRITE_LOCK: tokio::sync::Mutex<()>`
(`app/src/state.rs`), held across the *entire* open+write+commit sequence
at all three call sites - including, for the rebuild path, the delete-old-
directory step, not just the reopen after it (an earlier draft of this fix
left that specific window unlocked; caught and fixed before committing).
Not scoped per index directory - this app's real usage (one folder
actively being searched/indexed/watched at a time) doesn't need that
precision, and a single lock is trivially correct regardless of which two
operations happen to race.

## Bug 2: desktop notification crashing the process

`notify_search_complete` (a best-effort Windows/macOS/Linux toast on
search completion) fires unconditionally after *every* completed run -
the only thing in the whole post-search path that touches an OS-native
notification API regardless of any setting, which lines up with "crashes
every time, whether indexing is on or not." This was flagged as a risk
before it ever shipped - the dependency's own `Cargo.toml` comment: "a
real, known rough edge for toast notifications from a plain win32 exe,
not yet verified against an actual signed build." Unpackaged/unsigned
exes commonly lack the AppUserModelID Windows' WinRT toast API expects;
some of that API's failure modes can cross out of Rust's own panic-
catching entirely, which is why the existing `spawn_blocking` + `let _ =`
(swallowing a `Result`) wasn't actually sufficient protection - a crash
that isn't a catchable Rust panic doesn't care that the `Result` was
discarded.

Fix, given there's no Windows machine in this environment to verify a
real repair against: made it opt-in instead of unconditional.
`AppState.desktop_notification_when_done` (new checkbox, **defaults
false** - unlike every other checkbox added this session, this isn't
preserving prior behavior, it's gating behavior that's now a confirmed
crash source until it can be verified safe on a signed build). Also
wrapped the call itself in `std::panic::catch_unwind` as defense-in-depth
for the subset of failure modes that are ordinary Rust panics - explicitly
not claimed as a fix for the whole problem, since a raw Windows exception
crossing the FFI/COM boundary isn't something `catch_unwind` can intercept.

## Verification

`cargo build --workspace`: clean. `cargo test --workspace`: **191/191
passing, 1 deliberately ignored** (unchanged - neither fix touches
anything the existing suite exercises; the concurrency lock lives
entirely in `app`, which has no automated indexing-race test, and the
notification gate is a pure UI/settings change). `cargo run -p app`: ran
6+s with no panic. Neither bug is reproducible on this development
machine (no Windows, and the race in Bug 1 needs real filesystem-watcher
timing) - both fixes are verified by code reading and reasoning against
the exact reported symptoms, not by reproducing and re-testing the
original crash, which is an honest limitation of fixing a
Windows-specific, timing-dependent bug from a macOS development
environment.
