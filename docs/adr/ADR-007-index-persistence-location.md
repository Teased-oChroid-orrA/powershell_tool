# ADR-007: Index Persistence Location

Status: **Superseded by [ADR-011](ADR-011-in-folder-index-location.md)**.
The `%LOCALAPPDATA%` decision below shipped and was wired into
`MainViewModel`, then explicitly reversed by direct user direction to an
in-folder location instead. Kept here for the reasoning trail (the
per-machine-vs-per-folder tradeoff this ADR weighed is still real context
for ADR-011), not as the current behavior.

## Problem

`NativeSearchEngine::open_or_create` (native-search/src/engine.rs) requires
its caller to already have a writable directory and does no filesystem
provisioning itself (ADR-001) — a deliberate choice to keep the Rust module
a pure indexing/search concern, not a filesystem-policy one. Something on
the .NET side needs to decide where that directory actually is.

## Evidence

- The existing app has no established "app data" convention to inherit —
  confirmed in `docs/native-search-assessment.md`: no `%LocalAppData%`/
  `ApplicationData` usage exists anywhere in `src/` today. The one thing
  that resembles persistent app state, the optional incremental cache
  (`CacheService`), writes to a path the *user* types into a text field, not
  an app-owned location — a different pattern (explicit user choice) than
  what a search index needs (implicit, app-managed, not something a user
  should be prompted to place).
- A Tantivy index is app-owned, per-machine, regenerable state, not a
  user document — the standard Windows convention for exactly this shape
  of data is `%LOCALAPPDATA%\<AppName>\...`, which requires no admin rights
  and is writable by the current user by default (matches the "no admin
  rights" target-environment requirement in `CLAUDE.md`).
- The app is unpackaged (`WindowsPackageType=None`), so there's no MSIX
  per-app isolated storage identity to use instead — a plain folder under
  `%LOCALAPPDATA%` is the correct unpackaged-app equivalent.

## Decision

Index location is `%LOCALAPPDATA%\TextInFilesSearch\native-index\`
(`Environment.SpecialFolder.LocalApplicationData` + `TextInFilesSearch` +
`native-index`), created on first use if it doesn't exist.

Implemented as `NativeSearchPaths.GetDefaultIndexDirectory()` /
`NativeSearchPaths.EnsureIndexDirectoryExists()` in
`src/TextInFilesSearch.Core/Native/NativeSearchPaths.cs` — a thin helper
the eventual caller of `new NativeSearchService(...)` uses to get a
guaranteed-to-exist directory, keeping `NativeSearchEngine` itself free of
this policy per ADR-001.

This is a default, not a hard-coded requirement — nothing prevents a future
settings UI from letting a user point the index elsewhere (mirroring the
existing `CacheFilePath` pattern), but no such UI exists yet, so there is
exactly one caller-facing entry point to change later rather than a path
computed ad hoc in multiple places.

## Consequences

- One index per machine/user by default, not per search-folder or
  per-project — matches Tantivy's own single-writer-per-index model
  (ADR-002 item 7) and avoids a proliferation of small indexes with no
  clear lifecycle.
- Nothing calls `NativeSearchPaths` yet — this ADR and its implementation
  exist so the eventual WinUI wiring has an unambiguous, already-decided
  answer to "where does the index live," rather than that decision being
  made ad hoc inside a UI change later.
- Index growth/cleanup (what happens if the user re-points `SearchPath` to
  an entirely different folder tree, or deletes files that were indexed) is
  explicitly out of scope for this ADR — it's a consequence of *when* and
  *how* indexing gets triggered, which is still undecided (see ADR-001's
  open item on reconciling the two search paths).

## Rejected alternatives

- **A path next to the executable** — rejected: the app is published
  unpackaged and self-contained; writing app state next to the exe risks
  needing elevated permissions depending on install location (e.g.
  `Program Files`) and conflates "installed program" with "per-user data,"
  which `%LOCALAPPDATA%` exists specifically to separate.
- **Reusing the existing `CacheFilePath` convention (user-specified, no
  default)** — rejected: that field solves a different problem (an
  *optional* incremental-extraction cache the user explicitly opts into and
  places), not an index the app needs to always be able to find without
  asking. Forcing a location prompt for something as foundational as "where
  is the search index" would be worse UX than a sensible default.
