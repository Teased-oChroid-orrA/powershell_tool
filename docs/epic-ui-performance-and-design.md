# Epic: Transform the Search App into an Extremely Fast, Modern, Native-Feeling Dioxus + Blitz Experience

Filed after the first real visual pass of the Dioxus/Blitz rewrite
(`app/` crate - see `docs/rust-rewrite-status.md`): functionally complete
but reported as ugly and sluggish (screenshot: `<select>` dropdowns render
as garbled concatenated option text, results-list rows overlap/stack).
This document is the guideline/tracking epic for fixing that and going
further - a genuine visual and performance overhaul, not just a bug fix.

**Trust but verify.** The vision below (sections 1-50, largely as
originally drafted) is a strong product spec, but several of its
assumptions about what the current `dioxus-native`/`blitz-*` dependency
stack actually supports were **not yet true when checked against the real
crate source** (`~/.cargo/registry/src/.../blitz-*-*`, `dioxus-native-0.7.10`
at time of writing). Read the section below before starting implementation
- it will save you from designing a feature around a platform capability
that doesn't exist yet. Anyone picking this up later: re-run the grep
commands cited here against whatever version is in `Cargo.lock` at the
time, since `dioxus-native` is a young, fast-moving crate and any of these
could change.

## Verified platform constraints (checked against real source, not assumed)

| Claim in the vision doc | Status | Evidence |
|---|---|---|
| `<select>` dropdowns (implicitly assumed to work, used in the current app for Match mode/Exclude scope/Group by) | **Not implemented.** `blitz-dom` has no dropdown/popup widget for `<select>` - it just lays out `<option>` children as flat content, which is why the current app shows `AnyLineAllInFileProximity` as one run-on string. | `blitz-dom-0.2.4/src/form.rs:270` - `// TODO: If the field element is a select element...` is the *only* select-specific code in the crate; zero hits for `"select"` in `document.rs`/`html.rs`. |
| §17/§18 Drag & drop (folders/files dropped onto the window) | **Not wired up.** `winit` delivers `DroppedFile`/`HoveredFile` events to `blitz-shell`, but they're received and discarded. | `blitz-shell-0.2.3/src/window.rs:405-407`: `WindowEvent::DroppedFile(_) => {}` / `HoveredFile(_) => {}` / `HoveredFileCancelled => {}` - empty match arms, no event forwarded into the Dioxus app at all. |
| §35 Context menus ("should feel native and fast") | **No native context-menu API exists** in `blitz-shell`/`dioxus-native`. The only "ContextMenu" references are keyboard `Code`/`Key` enum mappings for the physical Menu key, not an OS context-menu widget. | `blitz-shell-0.2.3/src/convert_events.rs:146,335` - both are `WinitKeyCode`/`WinitNamedKey` mappings, not a menu-creation API. |
| §29 Animation / CSS transitions | **Confirmed working.** Stylo (the CSS engine) actively tracks transition state (`Pending`/`Running`/`Finished`) and Blitz's document layer checks `has_active_transitions` to decide whether to keep re-rendering every frame. | `blitz-dom-0.2.4/src/stylo.rs:91-96,716,728-734`; `blitz-dom-0.2.4/src/document.rs:142`. |
| §2 list-overlap bug already observed in the current app | Root cause not yet pinned to a specific missing CSS feature - flagged for investigation, not to be assumed identical to the `<select>` issue. | See "Phase 0" below for the verification step before fixing blind. |

**What this means for the plan below:** treat §35 (context menus) and
§17/§18 (drag & drop) as **build-it-yourself** work, not "wire up the
platform feature" work - same shape of fix as the `<select>` replacement
(a custom Rust/rsx component standing in for a native widget the renderer
doesn't provide yet). Drag & drop additionally needs the `blitz-shell`
event forwarding gap addressed first (either patch/fork `blitz-shell` to
forward `DroppedFile`/`HoveredFile` into the Dioxus event system, or find
another intercept point) before any app-level drag & drop code can do
anything at all - don't start on the app-level UI for this until that
plumbing exists and is verified with a real dropped file logged somewhere.
Everything else in the vision doc (virtualization, incremental search,
theming, keyboard shortcuts, CSS transitions/animation, layout) is either
confirmed supported above or is ordinary Dioxus/Rust application logic
with no Blitz-specific platform dependency - build those with normal
confidence, verifying only if something behaves unexpectedly.

## Immediate bugs (do these first - Phase 0)

These were the two concrete, screenshotted issues that prompted this epic.
Fix them before starting the broader redesign; they block any real
before/after comparison of later work.

1. **`<select>` garbled rendering.** Replace every `select { ... }` in
   `app/src/components.rs` (Match mode, Exclude scope, Group by - three
   call sites) with a custom dropdown component (`<button>` to open + a
   styled `<div>` list of clickable options, closed on selection/outside
   click). Verify each of the three dropdowns visually - opens, shows
   distinct readable options, updates the right signal on click, closes
   after selection.
2. **Results/list rows overlapping.** Before fixing: build a minimal
   repro (a scratch example or a temporarily stripped-down `ResultsPanel`)
   with just one `overflow-y: auto` flex column and confirm whether rows
   visually separate correctly in isolation. If they still overlap in the
   minimal repro, this is a `blitz-paint`/`anyrender_vello` clipping
   limitation - check https://github.com/DioxusLabs/blitz's issue tracker
   for an existing report before filing a new one, and apply a workaround
   (e.g. `display: block` rows with real margin instead of `flex` +
   `gap`) regardless of root cause. If rows separate fine in isolation,
   the bug is in this app's specific CSS (check for a missing `min-height:
   0` on a flex child - a classic overflow gotcha unrelated to Blitz).

## Sluggishness - investigate before optimizing blind

Not yet root-caused. Check candidates in order of cost to rule out:

1. **Whole-panel re-render on every keystroke.** `AppState`
   (`app/src/state.rs`) is one flat struct of ~35 signals, and
   `SettingsPanel` reads most of them directly in one large function body
   rather than each field being scoped to its own small component -
   typing in "Search folder" may be re-rendering the entire settings
   panel, including the extension catalog list. Verify with logging or
   `dioxus-devtools` before restructuring anything.
2. **`filtered_extensions`/`selected_extensions_summary` recomputed
   unconditionally on every render** (`components.rs`) - a full `Vec`
   filter/clone over the ~50-entry extension catalog on every keystroke
   anywhere in the panel, not just when the extension filter changes.
   Should be `use_memo`'d against its actual inputs.
3. **WGPU software-rasterizer fallback.** If the dev machine's GPU driver
   doesn't support the backend Vello wants, it silently falls back to
   software rendering. Check with `RUST_LOG=wgpu=info cargo run -p app` -
   if it's using a software adapter, that's an environment issue, not an
   app bug.

---

# 1. Core Product Principles

Everything in the implementation should follow these principles.

### 1.1 Speed is a feature

The application should feel instantaneous.

Target:

* Search input → first visible results: effectively immediate
* Keyboard navigation: no perceptible delay
* Opening/closing overlays: instantaneous
* Filtering: instantaneous
* Result selection: instantaneous
* Preview updates: instantaneous
* Scrolling: consistently smooth
* UI should remain responsive while indexing/searching

Never block the UI thread with:

* filesystem traversal
* indexing
* searching
* parsing
* file loading
* syntax highlighting
* expensive computation

Use asynchronous/background Rust work and stream results into the UI.
`search-core::orchestrator` already does this (tokio-based, throttled
parallel processing, `mpsc`-channel progress reporting) - the UI layer
must not block on it.

---

# 2. Performance Architecture

Before adding visual complexity, audit the existing architecture.

The UI must remain a thin presentation layer over efficient Rust services
(this is already true - `app/` is a thin Dioxus head over `search-core`,
mirroring the old C# Core/head split's own invariant. Keep it that way).

Recommended conceptual architecture:

```text
┌──────────────────────────────────────────────────────────┐
│                         Dioxus UI                        │
│                                                          │
│ SearchBar / Results / Preview / Sidebar / Command UI    │
└───────────────────────────┬──────────────────────────────┘
                            │
                     lightweight state
                            │
┌───────────────────────────▼──────────────────────────────┐
│                    Application Layer                     │
│                                                          │
│ SearchController                                         │
│ WorkspaceController                                      │
│ PreviewController                                        │
│ CommandController                                        │
└───────────────┬───────────────────────┬──────────────────┘
                │                       │
        background tasks          background tasks
                │                       │
┌───────────────▼───────────────┐ ┌────▼───────────────────┐
│       Search Engine           │ │     Index Manager      │
│                               │ │                         │
│ query parsing                 │ │ filesystem watching    │
│ ranking                       │ │ incremental indexing   │
│ filtering                     │ │ persistence             │
│ result streaming              │ │ cache                   │
└───────────────────────────────┘ └─────────────────────────┘
```

Do not put search-engine logic inside Dioxus components. It belongs in
`search-core`, tested with `cargo test -p search-core` independent of the
UI - that separation is the whole reason `search-core` exists.

---

# 3. Search Must Be Incremental

Do not wait until every result has been calculated.

For a query:

```text
authentication
```

the UI should receive results progressively:

```text
authentication

8 results
↓
42 results
↓
184 results
↓
1,284 results
```

But don't visually thrash the interface on every individual result.

Batch updates intelligently.

For example:

```text
search engine
    ↓
small batches
    ↓
UI update
    ↓
render
```

The UI should receive enough information to feel live without causing
excessive reconciliation. `orchestrator::run`'s existing `SearchProgressReport`
channel already streams per-file completions - this section is about the
UI consuming that well (batching signal updates), not about building new
plumbing from scratch.

---

# 4. Debouncing Without Making Search Feel Slow

Avoid naive long debounce timers.

The search experience should feel immediate.

Use a very short debounce only when necessary to prevent excessive queries
during rapid typing.

Cancel stale queries.

Example:

```text
query #41: "auth"
query #42: "authe"
query #43: "authen"
query #44: "authenti"
```

If query #41 is still running when #44 arrives:

```text
CANCEL #41
CANCEL #42
CANCEL #43
RUN #44
```

Never allow stale results to overwrite newer results.

Every search operation should have a generation/request ID.
`search-core::orchestrator::run` already takes a `CancellationToken` -
reuse that plumbing (cancel the old token, start a new one) rather than
inventing a separate generation-ID mechanism.

---

# 5. Virtualize Large Result Sets

This is mandatory.

Never render thousands of result components simultaneously.

If there are:

```text
128,492 results
```

the DOM/UI should only contain approximately the visible viewport plus a
small overscan region.

Conceptually:

```text
128,492 results
        │
        ▼
┌──────────────────────┐
│ virtualized viewport │
│                      │
│ result 421           │
│ result 422           │
│ result 423           │
│ result 424           │
│ result 425           │
│ ...                  │
└──────────────────────┘
```

Scrolling must remain smooth even with enormous result sets. This is
ordinary Dioxus application logic (render only the visible slice of a
`Vec`, driven by scroll position) - no Blitz-specific constraint found
against it, but verify actual scroll-position event delivery works
smoothly in Blitz before committing to a specific virtualization library,
since this app's result lists are typically far smaller (dozens to low
hundreds of file hits, not hundreds of thousands) than the scale this
section is written for - don't over-build this relative to actual result
sizes search-core produces.

---

# 6. Avoid Allocation Hotspots

Audit hot paths for:

* unnecessary `String` cloning
* repeated `PathBuf` cloning
* repeated formatting
* unnecessary `Vec` allocations
* temporary strings
* repeated parsing
* duplicate metadata
* redundant serialization/deserialization

Prefer:

* borrowed data where practical
* `Arc`
* immutable shared structures
* compact result representations
* cached metadata
* reusable buffers
* lazy computation

Do not optimize blindly; profile first.

But search and scrolling hot paths should receive particular scrutiny -
`search-core::extraction`/`matching` already avoid unnecessary allocation
in a few places worth preserving (e.g. `OnceLock`-cached compiled regexes
in `extraction.rs`) - don't regress those while touching this code.

---

# 7. Search Result Model

`search-core::models::FileSearchResult` and `LineHit` already exist and
are the real result model - this section's sketch (`SearchResult` with
`SmallVec<[Match; 4]>` etc.) should be reconciled against what actually
exists rather than introduced as a parallel type. If the UI needs a
lighter view model, that's `app/src/state.rs`'s `FileResultView` (already
exists) - extend it there, keep `search-core`'s model as the source of
truth.

Avoid storing large snippets unnecessarily.

Generate preview/context lazily when required.

---

# 8. Search Syntax

Make the search bar substantially more powerful.

Support queries such as:

```text
authentication
```

```text
authentication type:rust
```

```text
authentication ext:rs
```

```text
authentication path:src/
```

```text
authentication -test
```

```text
"exact phrase"
```

```text
authentication modified:today
```

```text
authentication size:<10mb
```

The parser should be fast and deterministic. Note: `search-core` already
has two distinct search paths - the per-run line scan
(`orchestrator::run`, plain filter-list matching) and the native_search
Tantivy index (`native_index.rs`, real query-parser syntax already via
`tantivy::query::QueryParser`). Decide which of these this section's
`type:`/`ext:`/`path:` syntax targets before implementing - Tantivy's
query parser may already give you most of this for free on the Fast
re-search path, whereas the primary line-scan path has no query parser at
all today and would need one built from scratch.

The UI should visually distinguish query terms from filters:

```text
authentication   type:rust   path:src/
───────────────   ─────────   ─────────
 search term       filter      filter
```

---

# 9. Search Bar

The search bar is the most important UI element.

Make it beautiful and extremely responsive.

Requirements:

* prominent
* keyboard accessible
* large click target
* excellent typography
* subtle border
* subtle focus treatment
* no unnecessary decoration
* search icon
* keyboard shortcut hint
* filter chips
* clear button
* optional result count

Example:

```text
┌───────────────────────────────────────────────────────────────┐
│  ◉  authentication type:rust path:src/            1,284  ⌘K │
└───────────────────────────────────────────────────────────────┘
```

Do not make it gigantic.

It should consume enough visual attention without dominating the entire
application.

---

# 10. Command Palette

Implement a first-class command palette.

Shortcut:

```text
Ctrl/Cmd + K
```

Commands should include:

```text
Search
Open folder
Open file
Add workspace
Remove workspace
Reindex
Refresh index
Toggle preview
Toggle sidebar
Change theme
Open settings
Keyboard shortcuts
Clear search
```

Commands should be searchable.

Support keyboard navigation:

```text
↑ ↓
Enter
Esc
```

The command palette must open and close without noticeable latency.
Global keyboard shortcuts need a real keydown listener at the window/app
root (`dioxus-native`'s `winit`-backed event loop delivers keyboard events
normally - no platform gap found here) - verify `Ctrl/Cmd` modifier
detection specifically, since cross-platform modifier-key handling is a
common source of subtle bugs.

---

# 11. Quick Open

Implement a second extremely fast workflow:

```text
Ctrl/Cmd + P
```

Allow users to quickly find:

* files
* folders
* recent searches
* recent files
* workspaces

This should not require the full search engine if a lightweight filename
index can answer the request faster.

---

# 12. Three-Pane Desktop Layout

Use a flexible desktop layout:

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Search                                                         ⌘K    │
├───────────────┬──────────────────────────────────┬───────────────────┤
│               │                                  │                   │
│ WORKSPACE     │ RESULTS                          │ PREVIEW           │
│               │                                  │                   │
│ project       │ config.rs                  98%  │ config.rs         │
│ documents     │ parser.rs                  94%  │                   │
│               │ search.rs                  91%  │ 138 │ ...         │
│ FILTERS       │ index.rs                   88%  │ 139 │ ...         │
│               │                                  │ 140 │ ...         │
│ Rust    2.8K  │                                  │                   │
│ MD      1.2K  │                                  │                   │
│ JSON     842  │                                  │                   │
│               │                                  │                   │
└───────────────┴──────────────────────────────────┴───────────────────┘
```

All panes should be resizable.

Persist pane sizes.

Allow:

* hide sidebar
* hide preview
* fullscreen results
* restore default layout

This is a significant departure from the current two-column
(settings-panel + results-panel) layout in `app/src/components.rs` -
treat as a real redesign, not an incremental CSS tweak. Resizable panes
need pointer-drag handling verified against Blitz's mouse-event delivery
before committing to the interaction design.

---

# 13. Result Cards Should Be Information-Dense

Avoid giant rounded cards.

Each result should communicate:

1. file
2. path
3. relevance
4. matching context
5. match count
6. file type
7. useful metadata

Example:

```text
┌──────────────────────────────────────────────────────────────┐
│ 🦀 config.rs                                      98%        │
│ src/config.rs                                                │
│                                                              │
│ 142  let config = load_<mark>config</mark>();                │
│ 143  if <mark>config</mark>.enabled {                        │
│ 144      start_service(&<mark>config</mark>);                │
│                                                              │
│ 3 matches                                      1.8 KB       │
└──────────────────────────────────────────────────────────────┘
```

Keep the visual hierarchy extremely clean.

---

# 14. Preview Pane

Selecting a result should instantly update the preview.

Do not open a new window.

Preview should support:

* syntax highlighting
* line numbers
* highlighted matches
* context around matches
* file metadata
* path breadcrumbs
* copy
* open externally
* reveal in file manager
* jump to match

Lazy-load file contents.

Never load enormous files entirely into memory just because they were
selected. Note `search-core::models::SearchSettings.max_file_size_mb`
already exists as a search-time cap; a preview-time lazy-load is a
separate, new concern from that setting - don't conflate the two.

---

# 15. Match Highlighting

Highlight the actual search match rather than the entire line.

Bad:

```text
████████████████████████████████████
```

Good:

```text
let config = load_<mark>config</mark>();
```

Use a restrained accent color.

Avoid neon/high-saturation highlighting. Note: `search-core::report`
already implements real match-highlighting logic (byte-range detection,
overlap merging, `<mark>` wrapping) for the HTML report - the preview
pane's highlighting should reuse that logic (or a shared helper extracted
from it) rather than reimplementing it from scratch in the UI layer.

---

# 16. Keyboard-First UX

Everything important should be accessible without the mouse.

Minimum:

| Shortcut               | Action                |
| ---------------------- | --------------------- |
| `Ctrl/Cmd + K`         | Command palette       |
| `Ctrl/Cmd + P`         | Quick open            |
| `/`                    | Focus search          |
| `↑ ↓`                  | Navigate              |
| `Enter`                | Select                |
| `Esc`                  | Close                 |
| `Ctrl/Cmd + F`         | Search within preview |
| `Ctrl/Cmd + ,`         | Settings              |
| `Ctrl/Cmd + Shift + F` | Global search         |
| `Ctrl/Cmd + Shift + R` | Reindex               |
| `?`                    | Keyboard shortcuts    |

Support platform-appropriate modifier keys.

---

# 17. Mouse UX

Also support:

* right-click context menus - **verify feasibility first**: no native
  context-menu API exists in `blitz-shell`/`dioxus-native` today (see
  "Verified platform constraints" above) - this means a custom
  Rust/rsx-rendered menu positioned at the click point, not an OS-native
  menu. Scope accordingly.
* double-click
* drag and drop - **verify feasibility first**: `blitz-shell` currently
  discards `DroppedFile`/`HoveredFile` winit events entirely (see
  "Verified platform constraints" above) - this is blocked on either
  patching `blitz-shell` to forward those events or finding another
  intercept point, not app-layer work alone.
* selection
* text copying
* path copying
* file reveal
* opening in external editor

Dragging a folder onto the application should be a first-class workflow -
subject to the drag-and-drop platform gap noted above.

---

# 18. Drag & Drop

**See the platform-constraint note in §17 before starting this section -
`blitz-shell` does not currently forward drop events to the app at all.**

Support:

```text
Drop folder
Drop files
Drop multiple folders
```

Show an elegant drop overlay:

```text
┌─────────────────────────────────────────────┐
│                                             │
│              ↓ Drop to search               │
│                                             │
│        files, folders or workspaces         │
│                                             │
└─────────────────────────────────────────────┘
```

Do not interrupt the current search unnecessarily.

---

# 19. Empty States

Never display generic:

```text
No results.
```

Use contextual empty states.

Example:

```text
                    ◌

                No matches

       Nothing matched your current query.

       Try removing a filter or searching
       for a broader term.

       /  Focus search
```

Initial state:

```text
                    ◉

              Search anything

        Add a folder or workspace to begin.

             [ Add workspace ]

        Drop a folder anywhere to get started.
```

(The "drop a folder anywhere" hint in the initial empty state should be
removed or gated behind the drag-and-drop platform gap being resolved
first - don't promise a capability that doesn't exist yet.)

---

# 20. Loading States

Avoid blocking spinners.

Prefer progressive UI.

During indexing:

```text
Indexing workspace…

██████████████████░░░░░░  74%

42,821 files indexed
```

But keep the rest of the application usable.

Users should be able to search while indexing whenever technically
possible.

---

# 21. Indexing Architecture

Make indexing incremental.

Do not rebuild everything unnecessarily. `search-core::native_index`
already implements skip-reindex-if-unchanged via
`NativeSearchEngine::get_document_metadata` - the incremental piece
described here largely already exists for the Fast re-search path; this
section's filesystem-watcher/rename-handling asks are the genuinely new
part.

Use:

* filesystem watchers
* modification timestamps
* file identity where appropriate
* content hashes where appropriate
* persistent index
* incremental updates
* deletion detection
* rename handling

On startup:

```text
existing index
      ↓
load immediately
      ↓
application becomes usable
      ↓
background validation/update
```

The app should not make the user wait for a full re-index on every
launch.

---

# 22. Persistence

Persist:

* workspaces
* recent searches
* selected result
* sidebar state
* preview state
* pane widths
* window size
* window position
* theme
* filters
* indexing configuration

But don't persist excessive transient UI state.

---

# 23. Recent Searches

Make search history useful:

```text
Recent

authentication type:rust
database connection
TODO path:src/
error handling ext:rs
```

Allow keyboard selection.

---

# 24. Search Statistics

Provide useful statistics without overwhelming the user.

Example:

```text
1,284 matches
across 143 files
```

Optional compact breakdown:

```text
Rust       842  ━━━━━━━━━━━━━━━
Markdown   293  ━━━━━
JSON       149  ━━━
```

Use microvisualizations rather than large charts.

---

# 25. Modern Visual Design

Use a restrained dark theme by default.

Suggested palette:

```text
Background       #0B0D10
Surface          #101318
Elevated         #151922
Hover            #1A202A
Selected         #202735
Border           #252C38

Primary text     #F4F6F8
Secondary text   #98A2B3
Tertiary text    #606A79

Accent           #7C5CFF

Success          #42D392
Warning          #F2B84B
Error            #FF5D72
```

Avoid pure black and pure white.

Use extremely subtle borders.

---

# 26. Typography

Use a high-quality UI font where available.

UI:

* Inter
* Geist
* system UI stack

Code:

* JetBrains Mono
* Iosevka
* system monospace

Use typography to establish hierarchy rather than excessive boxes. Verify
font loading works as expected in Blitz (Parley is the text-shaping layer
- system font fallback should work, but a bundled custom font file's
loading path should be verified with a real render before relying on it
for the whole app's type system).

---

# 27. Spacing

Create a consistent spacing scale.

For example:

```text
4
6
8
12
16
20
24
32
```

Do not randomly use:

```text
7px
11px
13px
19px
27px
```

Consistency is one of the easiest ways to make the application look
professionally designed.

---

# 28. Borders and Radius

Use subtle radius.

Avoid:

```text
border-radius: 24px
```

everywhere.

Prefer:

```text
4–8px
```

for most UI.

Use larger radius only for:

* command palette
* major overlays
* floating surfaces

---

# 29. Animation

**Confirmed working** - Stylo tracks CSS transition state and Blitz
re-renders while transitions are active (see "Verified platform
constraints" above). Build with normal confidence here.

Animation must never compromise performance.

Use short transitions for:

* hover
* focus
* dropdowns
* command palette
* sidebar
* preview

Avoid expensive animations involving large DOM trees.

Prefer opacity/transform/background changes.

Respect reduced-motion preferences where applicable.

---

# 30. Rendering Discipline

Audit every component for unnecessary rerenders.

Important:

* keep state as local as possible
* don't put rapidly changing state at the root
* avoid global state updates for individual result changes
* memoize derived expensive data where appropriate
* don't recompute formatting on every render
* don't parse search queries repeatedly
* don't regenerate unchanged result snippets
* don't recreate huge vectors unnecessarily

The UI should update only what actually changed. This is the sluggishness
investigation from "Immediate bugs" above, generalized into a standing
discipline - see item 1/2 there for the two concrete candidates already
identified in the current codebase.

---

# 31. No Giant DOM

Do not create thousands of nodes for:

```text
results
syntax highlighting
statistics
file trees
```

Use virtualization and lazy rendering.

The number of rendered UI nodes should scale approximately with the
viewport, not the total dataset. (See §5's note on right-sizing this
against this app's actual result-set scale before over-building it.)

---

# 32. File Preview Performance

Preview large files intelligently.

For large files:

* read only the required region
* seek to relevant lines
* display a bounded context window
* lazy-load surrounding content
* avoid loading entire files unnecessarily

Never freeze the application because someone selected a 500 MB log file.

---

# 33. Error Handling

Errors should be useful and non-destructive.

Instead of:

```text
Error
```

show:

```text
Couldn't read file

The file may have been removed or is no longer accessible.

[ Retry ]  [ Remove from results ]
```

Don't panic because one file disappears during indexing.
`search-core::file_reader::read_file_bytes_robust` already distinguishes
retryable vs. non-retryable I/O errors with real retry-with-backoff - the
UI-level error messaging here should surface that existing detail, not
collapse it into a generic error string.

---

# 34. Accessibility

Maintain:

* keyboard accessibility
* visible focus states
* sufficient contrast
* semantic controls
* tooltip descriptions
* predictable keyboard navigation

Keyboard accessibility is especially important because this is a
search-oriented desktop application. Verify Blitz's accessibility-tree
support (it depends on `accesskit` per its `Cargo.toml` dependencies) with
a real screen reader before assuming full parity with a browser-based
implementation - `accesskit` integration maturity varies by platform.

---

# 35. Context Menus

**Not natively supported - see "Verified platform constraints" above.**
No native context-menu creation API exists in the current
`blitz-shell`/`dioxus-native` stack. Build a custom in-app menu component
(positioned `div` at the cursor location on right-click, closed on
outside click/Escape) - the same pattern as the `<select>` replacement in
"Immediate bugs" above, not a wrapper around an OS API.

Results should expose:

```text
Open
Open externally
Copy path
Copy relative path
Reveal in file manager
Search in this folder
Search similar
Add folder to workspace
Exclude this path
```

Context menus should feel native and fast even though they're custom-built
- match the OS's visual conventions (shadow, fast open/close, dismiss on
outside click) rather than looking like a generic web dropdown.

---

# 36. Smart Search Actions

When appropriate, search results should expose actions.

Example:

```text
authentication

143 files
────────────────────────────

Search in folder
Open containing folder
Filter to Rust
Copy paths
Export results
```

Don't make users manually construct every operation.

---

# 37. Theme System

Support at least:

* Dark
* Light
* System

Design the entire UI around semantic tokens rather than hard-coded
colors.

Example:

```rust
struct Theme {
    background: Color,
    surface: Color,
    elevated: Color,
    text_primary: Color,
    text_secondary: Color,
    accent: Color,
}
```

This makes future themes easy. `app/src/main.rs`'s current `APP_CSS`
already uses CSS custom properties (`--fg`, `--bg`, etc.) with a
`prefers-color-scheme` media query - this section is an extension of that
existing pattern (add a `System`/explicit-override toggle, not a rewrite
of the token approach), matching the theming discipline already
established for the HTML report in `search-core::report`'s `CSS_BLOCK`.

---

# 38. Avoid Web-App Patterns

Do NOT turn this into:

* huge hero sections
* excessive gradients
* giant cards
* excessive shadows
* excessive rounded corners
* animated backgrounds
* unnecessary dashboards
* enormous empty spaces
* giant charts
* decorative UI that doesn't improve search

This is a **desktop tool**, not a marketing website.

---

# 39. Native Desktop Details

Take advantage of the desktop environment.

Support:

* native window behavior
* platform-appropriate shortcuts
* file manager integration
* drag/drop - subject to the platform gap noted in §17/§18
* clipboard
* external editor
* native file dialogs - already true (`rfd` crate, already integrated for
  folder browsing)
* window persistence
* system theme

Blitz should feel like part of the operating system rather than a
website - and structurally already avoids the one thing that would have
prevented that: no WebView involved at all (see
`docs/rust-rewrite-status.md` item 6 for why `dioxus-native` was chosen
over `dioxus-desktop`/wry specifically).

---

# 40. Performance Budgets

Establish explicit performance goals.

### Interaction

```text
Keyboard input → UI response       < 16ms target
Selection → preview update          < 16ms target
Command palette open                < 16ms target
```

### Rendering

Maintain smooth scrolling at approximately:

```text
60 FPS minimum target
```

where the environment allows it.

### Search

Optimize for:

```text
keystroke → first useful results
```

rather than:

```text
keystroke → complete search finished
```

The first result should arrive as quickly as possible.

### Memory

Do not retain:

* unnecessary complete file contents
* duplicate strings
* stale result sets
* abandoned search tasks
* unnecessary preview buffers

---

# 41. Cancellation Is Mandatory

Every expensive asynchronous operation should be cancellable where
practical.

Examples:

```text
SearchTask
IndexTask
PreviewTask
SyntaxHighlightTask
FilesystemScanTask
```

If the user changes their query, old work should stop contributing to the
UI. `search-core`'s `orchestrator::run` and `file_reader::enumerate_files_safely`
already accept a `CancellationToken` - reuse that plumbing for any new
task type introduced by this epic rather than inventing a second
cancellation mechanism.

---

# 42. Instrument Everything

Add lightweight instrumentation around:

* search latency
* indexing throughput
* first-result latency
* total result latency
* preview latency
* filesystem scan duration
* allocations where measurable
* memory usage
* UI update frequency

Do not guess about performance.

Measure.

Use profiling to identify actual bottlenecks before introducing
complexity - this is the same discipline "Sluggishness" above already
calls for; don't skip straight to a fix without measuring which of the
candidate causes is actually responsible.

---

# 43. Avoid Premature Heavy Dependencies

Prefer small, focused Rust crates.

Every dependency should have a reason.

Before adding a dependency, consider:

```text
Does this materially improve UX?
Does it materially simplify maintenance?
What does it add to compile time?
What does it add to binary size?
Does it allocate?
Can the same thing be implemented cheaply?
```

Do not add a massive UI framework on top of Dioxus. Also apply this
scrutiny to the `blitz-*`/`dioxus-native` stack itself where relevant -
e.g. patching `blitz-shell` for drag-and-drop (§17/§18) is real ongoing
maintenance burden if forked; weigh that against just not shipping drag &
drop for v1.

---

# 44. Component Architecture

Create a clean component hierarchy.

Suggested structure:

```text
App
├── AppShell
│   ├── Sidebar
│   │   ├── WorkspaceList
│   │   ├── FilterPanel
│   │   └── SearchStats
│   │
│   ├── MainContent
│   │   ├── SearchBar
│   │   ├── SearchToolbar
│   │   └── SearchResults
│   │       └── VirtualizedResultList
│   │           └── SearchResult
│   │
│   └── PreviewPane
│       ├── PreviewHeader
│       ├── Breadcrumbs
│       └── CodePreview
│
├── CommandPalette
├── QuickOpen
├── ContextMenu
├── Settings
└── ToastLayer
```

Keep components focused. This replaces `app/src/components.rs`'s current
two-component (`SettingsPanel`/`ResultsPanel`) structure - a real
refactor, not additive.

---

# 45. UX Polish

Add subtle details that make the application feel finished:

* animated result-count changes
* selection indicator
* subtle hover states
* keyboard shortcut hints
* breadcrumbs
* file-type indicators
* relative timestamps
* smart truncation
* tooltip for long paths
* copy confirmation
* "indexed X files" status
* background indexing indicator
* recent searches
* recently opened files
* preserved selection
* preserved scroll position where appropriate

Every interaction should have an intentional state.

---

# 46. Status Bar

Use a tiny unobtrusive status bar:

```text
143 files · 1,284 matches                    Indexed 42,821 files
```

During indexing:

```text
143 files · 1,284 matches                    Indexing 74%…
```

Don't make it visually dominant.

---

# 47. Toasts

Use toasts sparingly.

Examples:

```text
✓ Copied path
✓ Workspace added
✓ Index refreshed
```

Never use toasts for important errors that require user action.

---

# 48. Search Result Ranking

Improve result quality as much as performance.

Ranking can consider:

```text
exact filename match
exact phrase
filename match
path match
content match
word boundary
proximity
frequency
file type
recency
```

But keep ranking deterministic and fast.

Users should trust that the best result appears first. Note: the native_search
(Tantivy) path already has real relevance scoring
(`SearchHit.score`, BM25 via Tantivy) - the primary line-scan path
(`orchestrator::run`) currently has no ranking at all (results ordered by
file-completion order). Decide which path this section targets before
implementing.

---

# 49. Make "Open" Extremely Fast

Selecting a result should not cause:

```text
search → loading → modal → loading → file
```

Instead:

```text
select
  ↓
preview immediately
  ↓
Enter
  ↓
open externally
```

---

# 50. Final UX Goal

When complete, the application should feel like:

```text
Press key
    ↓
type
    ↓
results instantly appear
    ↓
↑ ↓
    ↓
preview
    ↓
Enter
    ↓
done
```

No unnecessary dialogs.

No waiting.

No visual clutter.

No giant UI framework.

No excessive animation.

No unnecessary network activity.

No WebView dependency. (Already true - see `docs/rust-rewrite-status.md`
item 6.)

No blocking work.

The application should feel **fast because the architecture is fast**,
not because it hides latency behind animations.

---

# Implementation Strategy

Do this incrementally rather than rewriting everything at once.

### Phase 0 — Immediate bug fixes (do first, see "Immediate bugs" above)

* [ ] Replace all three `<select>` uses with a custom dropdown component
* [ ] Root-cause and fix the results-list row overlap (minimal repro first)
* [ ] Investigate sluggishness (three candidates listed above, measure
      before fixing)

### Phase 1 — Performance foundation

* [ ] Profile current application
* [ ] Identify UI-thread blocking work
* [ ] Move search/indexing/file I/O to background tasks (already true at
      the `search-core` layer - verify the `app` layer doesn't
      accidentally block on anything synchronously)
* [ ] Implement search cancellation (reuse `CancellationToken` plumbing
      already in `search-core`)
* [ ] Add request/generation IDs (or confirm the existing
      `CancellationToken`-per-run approach already covers this)
* [ ] Implement incremental result streaming (already true via
      `SearchProgressReport` - verify UI consumption is batched well)
* [ ] Audit allocations in search hot paths
* [ ] Remove unnecessary cloning
* [ ] Establish performance instrumentation

### Phase 2 — Search UX

* [ ] Redesign search bar
* [ ] Add query parser (decide primary-scan vs. Tantivy path first - see §8)
* [ ] Add search filters
* [ ] Add result ranking (decide primary-scan vs. Tantivy path first - see §48)
* [ ] Add keyboard navigation
* [ ] Add recent searches
* [ ] Add Quick Open
* [ ] Add command palette

### Phase 3 — Results

* [ ] Implement virtualized results (right-size against actual result
      volumes - see §5/§31 notes)
* [ ] Redesign result rows
* [ ] Add match highlighting (reuse `search-core::report`'s existing
      highlight logic - see §15 note)
* [ ] Add metadata
* [ ] Add file-type indicators
* [ ] Add result statistics
* [ ] Add context menus (custom-built - see §35, not a native API)

### Phase 4 — Preview

* [ ] Build preview pane
* [ ] Add lazy file loading
* [ ] Add syntax highlighting
* [ ] Add line numbers
* [ ] Add match navigation
* [ ] Add copy/open/reveal actions
* [ ] Ensure huge files cannot freeze the UI

### Phase 5 — Workspace/indexing

* [ ] Persistent index (already true for native_search - see §21 note)
* [ ] Incremental indexing (already true for native_search - see §21 note)
* [ ] Filesystem watching (genuinely new)
* [ ] Rename/delete handling (genuinely new)
* [ ] Background indexing
* [ ] Index status UI
* [ ] Drag/drop workspace support - **blocked on `blitz-shell` event
      forwarding (see §17/§18) - resolve that first, verify with a real
      dropped-file log, before building any app-level UI for this**

### Phase 6 — Visual system

* [ ] Establish design tokens (extend the existing CSS-custom-property
      pattern in `app/src/main.rs`'s `APP_CSS`, don't replace it)
* [ ] Dark/light/system themes
* [ ] Typography system
* [ ] Spacing system
* [ ] Component primitives
* [ ] Consistent borders/radius
* [ ] Animation system (confirmed supported - see §29)
* [ ] Empty/loading/error states

### Phase 7 — Polish

* [ ] Custom context menus (see §35 - not native)
* [ ] Tooltips
* [ ] Keyboard shortcut overlay
* [ ] Window state persistence
* [ ] Workspace persistence
* [ ] Accessibility pass (verify `accesskit` integration maturity first - see §34)
* [ ] Reduced-motion support
* [ ] Cross-platform UX pass

### Phase 8 — Performance verification

* [ ] Benchmark search latency
* [ ] Benchmark first-result latency
* [ ] Benchmark indexing throughput
* [ ] Test 10K+ files
* [ ] Test 100K+ files
* [ ] Test millions of matches
* [ ] Test extremely large files
* [ ] Test rapid query changes
* [ ] Test rapid scrolling
* [ ] Test low-memory scenarios
* [ ] Profile CPU
* [ ] Profile memory
* [ ] Verify no UI freezes

---

# Definition of Done

The application should satisfy all of the following:

* Feels instantaneous during normal use.
* Search never blocks the UI.
* Stale searches cannot overwrite current results.
* Results are virtualized (or explicitly deemed unnecessary at this
  app's actual result-set scale, with that judgment call recorded here).
* Large result sets remain smooth.
* Large files cannot freeze the application.
* Indexing occurs incrementally.
* Existing indexes are reused on startup.
* Search results stream progressively.
* Keyboard navigation is first-class.
* Command palette exists.
* Quick Open exists.
* Preview is immediate.
* Match highlighting is precise.
* UI has a coherent modern design system.
* Dark/light/system themes work.
* Drag/drop works - **or is explicitly deferred with the `blitz-shell`
  platform gap documented as the blocker, not silently dropped.**
* Context menus work (custom-built, not native - see §35).
* Window/layout state persists.
* Error states are actionable.
* Empty states are polished.
* Accessibility is considered.
* Animations never become a performance bottleneck.
* Dependencies remain lean.
* Performance is measured rather than assumed.
* The `<select>` and list-overlap bugs from the original screenshot are
  fixed and visually verified.
* The application feels like a **native, purpose-built desktop search
  tool**, not a web application wrapped in a window.

## Most Important Rule

**Do not sacrifice performance for visual polish.**

Every UI improvement must preserve the application's core characteristic:

> **Type → search → results → preview → open, with essentially no
> perceived waiting.**

If a visual effect, abstraction, dependency, component, or feature makes
that interaction slower, more memory-intensive, or more complicated
without substantial UX value, don't add it.

**And per this epic's own "trust but verify" framing: before implementing
any section that assumes a platform capability, check it against the real
`blitz-*`/`dioxus-native` source first (the way the constraints table at
the top of this document was built), the same way every other technical
claim in this codebase's `CLAUDE.md`/`docs/adr/` is expected to be
grounded in real verification, not assumption.**
