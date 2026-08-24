# ADR-011: In-Folder Index Location (supersedes ADR-007)

Status: Accepted (direct user direction)

## Problem

ADR-007 put the native_search index at a single global,
per-machine location (`%LOCALAPPDATA%\TextInFilesSearch\native-index\`),
shared across every folder the app ever searched. After that shipped and
was CI-verified working, the user who owns this feature gave explicit,
direct direction to change it: the index should live **at the root of the
folder being searched** instead, and that folder must be automatically
excluded from the normal (non-native) search so it's never walked into as
if it were user content.

## Evidence / reasoning for the reversal

- A single global index conflates every folder ever searched into one
  index. "Fast re-search" then means "search everything I've ever indexed
  from any folder," which is a different, broader capability than "quickly
  re-search *this* folder" — and the user's direction makes clear the
  latter is what's wanted.
- Putting the index inside the searched folder makes the relationship
  between a folder and its index visible and self-managing: delete the
  folder, the index goes with it; copy/move the folder (e.g. to another
  machine), the index comes along too, no separate `%LOCALAPPDATA%` state
  to lose track of.
- `MatchingEngine`'s `ExcludeFolders` already matches by whole path
  segment, not substring (`CLAUDE.md`'s own documented bug-class note) —
  the mechanism to safely auto-exclude a folder by exact name already
  existed and needed no changes, only a caller to actually add the name to
  the list.

## Decision

- Index directory: `<SearchPath>\.native-search-index\` — a subfolder at
  the root of the folder being searched. Dot-prefixed to read as
  "tool-owned, not a document" at a glance (same convention as `.git`).
  Implemented as `NativeSearchPaths.GetIndexDirectory(string searchPath)`
  and the shared `NativeSearchPaths.IndexFolderName` constant
  (`src/TextInFilesSearch.Core/Native/NativeSearchPaths.cs`).
- `MainViewModel.BuildSettings()` always appends `IndexFolderName` to
  `ExcludeFolders` — unconditionally, not only when `IndexForFastSearch` is
  on for the current run, since a folder indexed by an *earlier* run must
  still be excluded even if this particular run isn't building/updating it.
- `MainViewModel.GetOrCreateNativeSearch(string searchPath)` now resolves
  and, if necessary, swaps the open `NativeSearchService` whenever the
  folder being searched changes between calls — disposing the previous
  instance rather than leaking it. See that method's own doc comment for
  the thread-safety reasoning (a lock, plus `ObjectDisposedException`
  handling in both `IndexHitsForFastSearch` and `RunNativeSearchAsync` for
  the rare cross-thread folder-change race this makes possible).

## Consequences

- One index per searched folder, not one global index — "Fast re-search"
  now only ever searches documents indexed from the *current* `SearchPath`,
  a narrower and more predictable scope than ADR-007's global index. A
  session that searches multiple different folders with `IndexForFastSearch`
  on ends up with multiple separate `.native-search-index` folders, one per
  searched location, not one shared one.
- `NativeSearchCommand`'s `CanExecute` now also requires a non-blank
  `SearchPath` (it didn't need one under the global-index design) - there's
  no folder to resolve an index location against otherwise.
- Copying, moving, or deleting a searched folder now visibly takes its
  index with it - a `.native-search-index` subfolder inside whatever the
  user does with the parent folder, not orphaned state in `%LOCALAPPDATA%`
  that ADR-007's design would have left behind indefinitely.
- `%LOCALAPPDATA%\TextInFilesSearch\native-index\` (ADR-007's location) is
  simply unused now - nothing reads or writes it. No migration of any
  pre-existing index at that path is implemented (this feature has had no
  end users yet, so there is nothing to migrate from in practice).

## Rejected alternatives

- **Keep ADR-007's global index and add a per-folder filter to search
  queries instead** (e.g. store `search_root` as an indexed field, filter
  every query by it) — rejected: more moving parts for a worse result. It
  still leaves stray global state building up forever in `%LOCALAPPDATA%`
  regardless of what happens to the folders that fed it, which is exactly
  what this ADR's evidence section argues against; the direct user
  instruction was also unambiguous about the folder-root location itself,
  not just query-time scoping.
