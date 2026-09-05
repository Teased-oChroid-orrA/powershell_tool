# CLAUDE.md

Context for Claude Code (or any fresh agent) picking up this repository with
no memory of how it got here. Read this before making changes.

## What this project is

A native Windows desktop app that recursively searches a folder for keyword
filters across `.txt`, `.log`, `.docx`, `.pptx` (slides, speaker notes, and
SmartArt diagram text), `.xlsx`, `.zip` (recursing into entries, including
nested zips), `.rtf`, `.pdf`, and dozens of other code/config/data
extensions (see `search-core/src/models.rs`'s `extension_catalog` module -
the single source of truth for both the engine's default extension list and
the UI's type-to-filter/tick-list extension picker), producing an HTML
report plus optional CSV/JSON export.

**The project is mid-migration from C#/WinUI to Rust/Dioxus.** The Rust
stack (`native-search/`, `search-core/`, `app/`) is the actively developed
implementation; the C#/WinUI app (`src/`) is kept as a working reference
during the transition - same treatment this repo already gives `powershell/`
(the original PowerShell tool the C# version was itself migrated from). See
"Why the migration happened" below before assuming either older stack is
dead weight to delete.

## Architecture: three-crate Cargo workspace

```
Cargo.toml                     Workspace root. [profile.*] and
                               .cargo/config.toml live HERE, not in a
                               member crate - Cargo silently ignores
                               per-crate [profile.*] sections for non-root
                               workspace members.

native-search/                 Tantivy-backed indexing/search engine
                               (issue #2's "Fast re-search" feature).
    src/engine.rs                NativeSearchEngine: open_or_create,
                               index_document, delete_document, commit,
                               search, get_document_metadata. Called
                               in-process by search-core/app - no FFI
                               involved on the Rust side of this repo.
    src/ffi.rs                  A C ABI (extern "C", catch_unwind-guarded)
                               that exists ONLY to serve the legacy C#/
                               WinUI app's P/Invoke layer. Dead weight for
                               the Rust app; do not remove until the C# app
                               is retired (see docs/ffi.md).

search-core/                   Plain Rust library. Zero GUI dependency -
                               ports TextInFilesSearch.Core 1:1, and keeps
                               the same "buildable/testable on any
                               platform's toolchain, no GUI libs needed"
                               property that made the C# Core valuable.
    src/models.rs                 SearchSettings, FileSearchResult,
                               LineHit, SearchRunResult, the match-mode/
                               exclude-scope/group-by enums, and the
                               extension_catalog module.
    src/matching.rs                MatchingEngine port. Uses `fancy-regex`
                               (not the `regex` crate) everywhere, even for
                               plain literal/regex-mode filters - whole-word
                               matching needs lookaround, which `regex`
                               doesn't support by design, and using one
                               regex engine throughout avoids two subtly
                               different matching semantics coexisting.
    src/extraction.rs              TextExtractionService port. DOCX/PPTX/
                               XLSX/ZIP read via `zip` + regex tag-stripping
                               (matching the C# original's own dependency-
                               free ZipArchive+Regex approach, not a real
                               OOXML parser) rather than adopting
                               `calamine`/`docx-rust`/etc., whose extraction
                               algorithms would silently diverge from the
                               byte-for-byte-tested original. PDF via a
                               hand-rolled stream/ASCII85/FlateDecode walker
                               (flate2 for the raw-deflate part).
    src/file_reader.rs              FileReaderService port. Async
                               (tokio) robust file reads with retry/
                               timeout, plus a sync, cancellable,
                               symlink-safe directory walk.
    src/cache.rs                    CacheService port. The incremental
                               JSON cache, fingerprinted by the settings
                               that affect matching.
    src/report.rs                    ReportExportService port. HTML/CSV/
                               JSON export - the HTML report's CSS and
                               structure are copied verbatim from the C#
                               original so old and new reports stay
                               visually identical.
    src/orchestrator.rs               SearchOrchestrator port. Async,
                               tokio-based; throttled parallel processing
                               via a `Semaphore` + `JoinSet` (not literally
                               `Parallel.ForEachAsync`, but the same
                               throttle-limit semantics).
    src/native_index.rs               Policy layer over native-search's
                               `engine.rs` (index-per-searched-folder
                               placement at `.native-search-index/`
                               inside the searched folder - ADR-011 -
                               auto-exclusion of that folder, and
                               skip-reindex-if-unchanged). No FFI, no
                               SafeHandle - native-search is a normal
                               in-process library dependency here.
    src/ocr.rs                       Optional OCR fallback for image-only/
                               scanned PDFs (`ocr` Cargo feature, off by
                               default for the bare library, on for
                               `app`/`cli`). `ocrs`+`rten` - pure-Rust
                               ONNX-model execution, no system runtime
                               dependency - chosen after evaluating
                               alternatives specifically against this
                               project's "no pre-installed runtime, fully
                               offline-capable" constraint. Two `.rten`
                               model files (~12MB, `assets/ocr/`) are
                               embedded via `include_bytes!` rather than
                               downloaded at runtime. Only attempted when
                               a PDF has no text-showing operators at all
                               AND `SearchSettings.ocr_scanned_pdfs` is
                               explicitly enabled (real per-page latency,
                               not the millisecond range the rest of
                               extraction runs in) - time-bounded against
                               the same `overall_timeout_seconds` the rest
                               of `extract_pdf_lines` respects, never run
                               unconditionally.
    tests/fixtures.rs                  Integration tests against the SAME
                               real DOCX/PPTX/XLSX/ZIP/PDF fixture files
                               the old C# test harness used (reused
                               byte-identical from
                               tests/TextInFilesSearch.Tests/Fixtures/,
                               not regenerated).

app/                            Dioxus desktop head. Keep this THIN - all
                               business logic belongs in search-core. Now
                               a multi-tool dashboard shell ("Toolbench" -
                               see docs/toolbench-status.md) with a left
                               tool-switcher rail, not a single-purpose
                               window - the search feature described
                               throughout this file is the first, fully-
                               functional tool inside it; a few more rail
                               slots exist as inert "Coming soon"
                               placeholders (Duplicate Finder/Batch
                               Rename/Log Analyzer).
    src/main.rs                    Entry point AND the dashboard shell
                               itself (`App()`: `.rail` tool switcher +
                               `.main` topbar/stage, `ToolId` enum,
                               `PlaceholderTool` component, hand-written
                               inline-SVG icons). Launches via
                               `dioxus_native::launch_cfg` (NOT
                               `dioxus::launch`/the "desktop" feature -
                               see "Why dioxus-native, not dioxus-desktop"
                               below).
    src/state.rs                    AppState: one Dioxus `Signal<T>` per
                               setting (mirrors the old MainViewModel's
                               properties 1:1), plus the Run/Cancel/
                               Native-Search async command logic
                               (`run_search`, `run_native_search`,
                               `browse_search_folder`, etc.) as methods on
                               it. Calls `rfd` (folder picker) and `open`
                               (report opening) directly - unlike the C#
                               ViewModel, there's no separate-testability
                               reason to inject these as delegates, since
                               all the actually-testable logic already
                               lives in search-core.
    src/components.rs                The rsx UI: `SettingsPanel` (mirrors
                               MainWindow.xaml's Required / Matching /
                               Scope and output / Performance and
                               robustness / Fast re-search sections, each
                               as a `<details>`/`<summary>` - the HTML
                               equivalent of WinUI's `Expander`) and
                               `ResultsPanel` (progress bar, in-flight
                               file list, results list).

src/TextInFilesSearch(.Core)/   The C#/WinUI app. Reference only during
                               the transition - see "Why the migration
                               happened" below. Do not add new features
                               here; port them into search-core/app
                               instead.
tests/TextInFilesSearch.Tests/  The C# app's own dependency-free test
                               harness (Program.cs) - still the
                               verification gate for src/TextInFilesSearch*
                               while that app remains in the repo.
                               Fixtures/ is also reused by
                               search-core/tests/fixtures.rs.
docs/, GS_Engineering_Brand_Assets/, powershell/   Unchanged - see the
                               longer-form docs and the "reference only,
                               never wire up" treatment already established
                               for powershell/.
```

## Why the migration happened

WinUI 3 cannot run, build, or be debugged on a non-Windows development
machine at all - every UI iteration had to go through a Windows CI
round-trip (tens of minutes each). A real bug (`EnableMsixTooling=false`
silently disabling `resources.pri` generation, causing "app launches, no
window appears, no error") took three separate CI round-trips to diagnose
blind, something local reproduction would have caught in seconds. Rust +
Dioxus was chosen specifically so the whole app - business logic AND UI -
can be built, run, and debugged locally on any platform, closing that loop.

## Why `dioxus-native` (Blitz/WGPU/winit), not `dioxus-desktop` (wry/WebView2)

`app/Cargo.toml` enables dioxus's `"native"` feature, not `"desktop"`. This
was a deliberate, verified decision, not a default: `wry` (the webview
backend `"desktop"` uses) hardcodes `browserExecutableFolder` to null in its
`CreateCoreWebView2EnvironmentWithOptions` call (confirmed by reading
`wry-0.53.5/src/webview2/mod.rs` directly in the local Cargo registry cache,
not assumed from documentation) - there is no supported way to bundle a
Fixed Version WebView2 Runtime app-locally with it. Every `"desktop"` build
would therefore depend on a machine-wide WebView2 install, which directly
violates this project's standing "fully self-contained, no host-machine
dependency" requirement (the same requirement that drove bundling the VC++
Redistributable for the WinUI build - see `docs/deployment.md`).
`dioxus-native` has no WebView dependency at all: Windows' bundled D3D12
(always present) is the only runtime graphics dependency. If you ever
consider switching back to `"desktop"`, re-verify that constraint hasn't
changed upstream first - don't just flip the feature flag.
`.github/workflows/rust-build.yml` has a regression check that fails the
build if `WebView2Loader.dll` ends up linked into `app.exe`.

## `powershell/` and `src/TextInFilesSearch(.Core)/` are reference-only

Neither has a runtime or build dependency from `native-search/`,
`search-core/`, or `app/`. They exist so behavior can be diffed against if a
discrepancy is ever suspected between the Rust port and the (twice-migrated)
original. Do not add a PowerShell invocation, a C#/`.NET` reference, or any
shell-out to either from Rust code. If a feature seems missing from the Rust
port, port it into `search-core` - don't fall back to calling out to an
older implementation.

## Design decisions worth knowing before you change them

- **`fancy-regex`, not `regex`, throughout `matching.rs` and the report
  highlighter.** Whole-word matching needs lookaround
  (`(?<![\p{L}\p{N}_])...(?![\p{L}\p{N}_])`, so punctuation-edged filters
  like "C#" work standing alone between spaces) - the `regex` crate
  deliberately doesn't support lookaround (no backtracking, by design).
  Verified against the C# whole-word test cases before adopting, not
  assumed. Using `fancy-regex` for plain/regex-mode filters too (not just
  whole-word) avoids two different regex engines' matching semantics
  quietly diverging on edge cases.
- **DOCX/PPTX/XLSX/ZIP extraction is hand-rolled (`zip` + regex
  tag-stripping), not a real OOXML parser crate.** The C# original is
  itself dependency-free (`ZipArchive` + `Regex`, no OOXML library) - this
  is a deliberate parity choice, not an oversight. A "better" library
  (`calamine`, `docx-rust`, ...) would extract text differently in edge
  cases and silently drift from the byte-for-byte-tested original.
- **`InFlightMap` (orchestrator.rs) is `std::sync::Mutex`, not
  `tokio::sync::Mutex`.** The PDF-progress and retry-status callbacks
  extraction.rs/file_reader.rs accept are plain synchronous `FnMut`
  closures (not async) - a std Mutex lets them lock/update/unlock without
  needing to be async themselves, and the critical sections are always
  short (a HashMap insert). Don't "upgrade" this to an async mutex without
  also making those callback signatures async.
- **`AppState` (app/src/state.rs) is a flat `Copy` struct of `Signal<T>`
  fields**, not a context-provided struct or a nested tree of smaller
  state objects. `Signal<T>` is itself `Copy`, so this is the idiomatic
  Dioxus pattern for a single-window app - passing `AppState` into a
  component or an async task just copies a handful of cheap handles, no
  `Arc`/context-provider plumbing needed.
- **Numeric `<input>` handlers must only call `.set()` on a successful
  parse**, never fall back to a hardcoded default on invalid/partial input.
  Dioxus's controlled inputs re-render the `value` attribute on every
  signal change - calling `.set()` with a fallback default on every
  keystroke (including while the field is transiently empty mid-edit)
  fights the user's typing with a visible snap-back. This was a real bug
  caught and fixed during the initial port; if you add a new numeric field,
  match the existing pattern (`if let Ok(v) = evt.value().parse() { ... }`,
  no `else` branch that sets anything).

## Testing requirements - do not skip these

- **Before considering any `search-core` change done**, run:
  ```
  cargo test -p search-core
  ```
  Zero GUI dependency, runs anywhere (developed and verified without a
  Windows machine, same as the old C# `Core` was). Covers all three match
  modes, exclude scopes (including that `exclude_folders` matches whole
  path segments, not a raw substring), whole-word/regex matching (including
  the punctuation-edged "C#" case and highlight-span correctness),
  invalid-regex-filter error reporting (naming the bad filter), the
  ASCII85 decoder, RTF extraction, real DOCX/PPTX (slides + speaker notes +
  SmartArt diagram)/XLSX/ZIP (including a nested DOCX entry)/PDF fixtures
  (the PDF case specifically exercises an ASCII85Decode+FlateDecode filter
  chain), parallel-vs-sequential consistency, cancellable/progress-reported
  directory enumeration, the full incremental cache lifecycle, CSV
  formula-injection neutralization, the Windows-1252 encoding path, the
  native_search index-per-folder/auto-exclude/skip-if-unchanged policy, and
  full end-to-end orchestrator runs against every real fixture. Add a new
  test here for any new behavior rather than trusting a passing build.
- **The `app`/`app-egui` crates (Dioxus/egui UI) cannot be verified by
  `cargo check`/`cargo test` alone** - the actual rendered window needs a
  real run. `dx serve` (or `cargo run -p app`) / `cargo run -p app-egui`
  locally is the fast feedback loop; unlike the old WinUI head, this works
  on any platform including macOS/Linux, since neither `dioxus-native` nor
  `egui`/`eframe` has a Windows-only rendering dependency.
  **Many prior phases in this project's history claimed "not independently
  verified on-screen, no local GUI capability in this environment" - that
  assumption was WRONG for at least one real session** (confirmed:
  `screencapture` and a real display are available here on macOS).
  Multiple real, screenshot-confirmed layout bugs shipped across several
  phases specifically because that assumption went unquestioned - see
  `docs/app-egui-parity-checklist.md`'s top section for the two exact bug
  classes this cost. **Before claiming any `app-egui`/`app` layout or
  rendering fix works, actually try:** `cargo build -p app-egui &&
  ./target/debug/app-egui &` then `screencapture -x <path>` and read the
  result back - don't assume this is unavailable without testing it in
  the current session first. If it genuinely isn't available in a given
  environment, say so explicitly rather than reusing this note's old
  wording as if it were still an untested assumption.
  `.github/workflows/rust-build.yml` is the CI gate for the actual win-x64
  build: it builds `app`/`app-egui` for `x86_64-pc-windows-msvc` on a
  Windows runner and checks the published exe for an accidental
  `WebView2Loader.dll` dependency creeping back in (see "Why
  dioxus-native" above - `app-egui` never had this dependency to begin
  with, but the same CI check stays harmless to keep passing for it too).
- **`src/TextInFilesSearch(.Core)/` (the C#/WinUI reference app)** still
  has its own gate: `dotnet run --project tests/TextInFilesSearch.Tests`
  locally, `.github/workflows/build.yml` in CI (builds, tests, publishes
  self-contained, and checks the publish output for `hostfxr.dll`/
  `coreclr.dll`/`Microsoft.WindowsAppRuntime.dll`/`resources.pri`/
  `native_search.dll`, and the absence of MSIX output). Only relevant if
  you're deliberately still touching that app during the transition.

## Live progress reporting is a hard requirement, not a nice-to-have

This project exists partly because of a specific, explicit complaint: PDF
processing in the original PowerShell tool would go silent for many seconds
with no way to tell "still working" from "actually stuck." Any future
change to `search-core::orchestrator` or `extraction::extract_pdf_lines`
MUST preserve:
- Per-file progress reporting during extraction, not just on file
  completion (the PDF progress callback fires roughly every 150ms with
  streams-scanned + elapsed time).
- A background ticker during parallel runs (`orchestrator.rs`'s
  `ticker_handle`) so elapsed-time displays keep moving between file
  completions, not just when a file finishes.
- Per-file in-flight status visible in the UI (`AppState.in_flight_files`),
  not just an aggregate progress bar - a user should be able to see which
  specific file is slow and what it's doing.

Don't refactor this into a simpler "start/done" event model even if it
looks cleaner - that regresses the exact problem this app was built to fix.

## Bug classes already found and fixed once - watch for recurrences

- A mode-gating bug: "no hits at all" and "hits existed but failed
  AllInFile/Proximity gating" were briefly conflated by inferring pass/fail
  from whether the hits list was empty (the original C# bug). The Rust
  port's `matching::apply_line_matching` reports `passes_mode` as an
  explicit struct field for exactly this reason - keep that distinction
  explicit rather than re-deriving it from list state if you touch this
  function.
- A numeric `<input>` snap-back bug (see "Design decisions" above) -
  calling `.set()` with a fallback default on every keystroke instead of
  only on successful parse.
- **`onchange` never fires on this renderer - use `oninput` for
  everything, including checkboxes.** `dioxus-native`/`blitz-dom` has no
  `Change` DOM event at all (`blitz-traits::events::DomEventData` has no
  such variant); a checkbox click dispatches only an `Input` event. Every
  checkbox in `SettingsPanel` used `onchange` from the original port
  onward, so none of them ever actually updated app state - Blitz's own
  internal visual toggle would flip on click, then the next re-render's
  controlled `checked: {signal}` binding (holding the never-updated old
  value) would snap it straight back, reading as "the checkbox doesn't
  respond to clicks." Silent for a long time because it degrades
  gracefully-looking (a flicker, not a crash) rather than erroring. Fixed
  by switching every `onchange` to `oninput` (see
  `docs/epic-ui-performance-and-design.md`'s platform-constraints table
  for the full source trail) - `FormData::checked()` reads the same
  `value` string either way, so this is a pure rename, not a logic
  change. If you add a new checkbox/radio/any form control, use `oninput`
  from the start, not `onchange`.
- **`<details>`/`<summary>` never toggles on click either - use
  `components.rs`'s `Expander` component, never a raw `details`/
  `summary`.** Same root cause and bug class as the `onchange`/`<select>`
  gaps above: `blitz-dom`'s click dispatcher
  (`blitz-dom-0.2.4/src/events/mouse.rs`'s `handle_click`) only special-
  cases `checkbox`/`radio`/`label`/`a`/`submit`/file `input` elements -
  clicking a `<summary>` falls through to `_ => {}` and does nothing, and
  no code anywhere in `blitz-dom` ever mutates a `details` element's
  `open` attribute in response to any event. Every `<details>` in this
  app (the four `SettingsPanel` sections, all seven original
  `BushingSection`s) was therefore permanently stuck at whatever `open`
  state it was given at render time - `SettingsPanel`'s were stuck
  closed, `bushing_workbench.rs`'s were hardcoded `open: true` (stuck
  open) specifically because an earlier pass already needed a workaround
  for this. Found while investigating a scroll-cumbersomeness request:
  making bushing's sections default-closed to shorten scroll distance
  would have made critical fields permanently inaccessible without this
  fix landing first. Fixed by `Expander` (`components.rs`): keeps real
  `<details>`/`<summary>` markup (so the existing `details`/`summary`/
  `details[open]` CSS in `main.rs` still applies unchanged) but drives
  `open` from an explicit signal and toggles it via a manual `onclick` on
  `summary`, the same "replicate the missing native behavior by hand"
  fix shape as `Dropdown`. If you add a new collapsible section, use
  `Expander`, not a bare `details`/`summary`.
- **`position: sticky` is parsed but never actually implemented on this
  renderer - it behaves exactly like `position: static`.** Confirmed by
  reading `blitz-dom` directly (`~/.cargo/registry/src/index.crates.io-.../
  blitz-dom-0.2.4/src/layout/damage.rs:368` and `src/node/node.rs:190`):
  both bucket `Position::Sticky` in with `Static`/`Relative` for paint/
  z-ordering purposes only - there is no offset-on-scroll logic anywhere
  in the crate. Found chasing a real screenshot report that a status
  rail wasn't staying visible while its page scrolled; don't reach for
  `position: sticky` to pin an element during scroll on this renderer -
  it will silently do nothing. If you need something to stay fixed while
  a sibling scrolls, give the scrolling sibling its own bounded height +
  `overflow-y: auto` (see `.bushing-workspace` in `main.rs`, or the
  pre-existing `.settings-column`/`.results-column` pattern) rather than
  trying to pin the other element in place.
- **CSS `transform` is invisible to hit-testing (hover/click) on this
  renderer - only `final_layout.location`/`final_layout.size` are
  consulted, never the transform matrix.** Confirmed by reading
  `blitz-dom-0.2.4/src/node/node.rs:716`'s `Node::hit()` directly. An
  element hidden via `transform: translateX(...)` (or any other
  transform) keeps its full untransformed hit-test box exactly where it
  would sit if the transform weren't applied - clicks/hover landing in
  that invisible region still fire as if the element were sitting there
  visibly. Found building an auto-hiding sidebar: `.rail:hover {
  transform: translateX(0) }` never actually stayed collapsed, because
  the always-present, always-full-size hit box kept re-triggering
  `:hover` the moment the cursor crossed where the rail *would* be if
  open. If you need a show/hide interaction driven by `:hover` (which
  does work - see `.nav-item`/`.add-tool-btn`/`.theme-toggle`/`.rail`),
  animate a real layout property (`width`, `height`, `max-height`), not
  `transform`.
- (Historical, C#-era, preserved for context) An XML comment containing
  `--` broke a `.csproj` file outright; a `zip -x "*.git*"` packaging
  command once silently excluded the entire `.github/` folder via
  substring wildcard matching. Worth remembering if you ever script an
  archive/export of this repo - use exact path exclusions, not bare
  substring wildcards, and diff the result.
- **PDF text extraction silently returned zero text for CID-keyed/Type0-font
  PDFs (hex-string `<0176> Tj` operands) even though `extract_pdf_lines`
  correctly located and decompressed the actual content stream** - found
  investigating a real user-reported PDF (a Stripe-generated invoice) that
  extracted nothing. Root cause: `text_re` (`extraction.rs`) only ever
  matched parenthesized-literal Tj operands (`(...)  Tj`), never
  hex-string operands (`<...> Tj`) - the encoding a CID-keyed embedded/
  subsetted font uses, which is the *default* for most modern PDF
  generators (headless-browser/Chromium print-to-PDF, many invoicing/web
  tools, LaTeX/pdflatex), not a rare edge case. Fixed by adding
  `hex_string_re`/`parse_tounicode_cmap`/`hex_string_to_unicode`: resolves
  hex CIDs through the file's own `/ToUnicode` CMap (`beginbfchar`/
  sequential-`beginbfrange` forms) rather than treating the raw CID as a
  Unicode codepoint (which would produce wrong, not just missing, text).
  A second, non-obvious pitfall caught in the same investigation: many
  real generators emit one `Tj` call *per glyph*, not per word - naively
  pushing one `lines` entry per hex-string match fragments every word into
  one character per line, silently breaking substring/whole-word search
  even though the mapped text is byte-for-byte correct. Fixed by
  concatenating all hex-derived characters within one content *stream*
  into a single line before pushing. If you touch PDF text extraction
  again, keep both fixes in mind - correct character mapping alone isn't
  enough if the result gets fragmented back into unsearchable pieces.

- **A tantivy `IndexWriter` that hits `TantivyError::ErrorInThread` is
  permanently dead - the same instance never recovers, only a fresh
  `index.writer(budget)` does.** Confirmed by reading tantivy 0.26.1's own
  source: a background segment-writer thread dying (io error, panic) sets
  `index_writer_status` to not-alive, and every subsequent
  `send_add_documents_batch` call raises exactly the user-reported
  `"An index writer was killed."` text - this is a distinct failure mode
  from Rust `Mutex` poisoning (already handled elsewhere via
  `lock_writer()`'s `unwrap_or_else(|poisoned| poisoned.into_inner())`) and
  must not be conflated with it. Fixed in `native-search/src/engine.rs` by
  `with_writer_retry`: on `ErrorInThread`, rebuild the writer once and
  retry; `search-core/src/native_index.rs`'s indexing loop also now
  tolerates a per-document/per-commit failure
  (`CorpusIndexOutcome::failed_files`) instead of aborting the whole run,
  since a single dead worker thread shouldn't lose all indexing progress
  made before it died.
- **`#![windows_subsystem = "windows"]` is a per-binary-crate attribute,
  not something a new binary target inherits from a sibling.** `app/`
  (the dioxus-native predecessor) already carries
  `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`,
  with its own doc comment recording the exact bug it fixes (a Windows
  console-owner-process window opening alongside the GUI window; closing
  the console kills the whole process). `app-egui/` never got this
  attribute when scaffolded as a new binary target - a real regression a
  user hit on a real Windows machine, since the two binaries don't share
  a `main.rs`. If this project ever adds a third binary target, check for
  this attribute explicitly rather than assuming a fix already shipped in
  one binary automatically covers a new one.

- **`app-egui` now bundles real fonts (Inter + JetBrains Mono,
  `assets/fonts/`, wired in `design/typography.rs`) - egui's bundled
  default fonts (`Ubuntu-Light`/`Hack-Regular`/`NotoEmoji-Regular`/
  `emoji-icon-font`) are kept installed as FALLBACKS, never removed.**
  Google Fonts only distributes Inter/JetBrains Mono as variable fonts
  now, and `ab_glyph` (egui/epaint's rasterizer) has no variable-font
  axis support - it would render every weight at the font's single
  default named instance. Static Regular/Medium/SemiBold/Bold (Inter)
  and Regular/Bold (JetBrains Mono) instances were produced with
  `fonttools varLib.instancer` (`pip install fonttools` in a throwaway
  venv) before bundling via `include_bytes!` - don't bundle a variable
  font file directly here, it won't do what it looks like it should. The
  fallback-fonts-stay-installed part matters for a reason already on
  record: the "verify glyph coverage before using an uncommon Unicode
  symbol" bug class below found that ✔/⚙/🖊/🌙 don't exist in Inter or
  JetBrains Mono at all (re-confirmed via `fontTools` cmap inspection
  before adopting these faces) - removing the old bundled fonts instead
  of layering the new ones in front would have reintroduced that exact
  bug for four already-fixed symbols.

- **`resvg`/`usvg`/`tiny-skia` were declared in `app-egui/Cargo.toml` since
  early in this crate's history but sat completely unused until
  `design/icons.rs`** - confirmed by grepping for any `resvg::`/`usvg::`/
  `tiny_skia::` call anywhere in the crate before this landed; there
  were none. If you're hunting for "why is this dependency here", check
  `design/icons.rs` first now, not just `Cargo.toml`'s own comment (which
  itself was aspirational, not a description of working code, until this
  pass). Two non-obvious integration details if you touch icon
  rasterization again: (1) every bundled Lucide SVG uses
  `stroke="currentColor"` - `usvg` has no CSS cascade to resolve that
  against, so it must be string-replaced with a real hex color before
  `Tree::from_str`; (2) `tiny_skia::Pixmap::data()` is **premultiplied**
  alpha - feed it to `egui::ColorImage::from_rgba_premultiplied`, not
  `from_rgba_unmultiplied`, or every anti-aliased edge pixel renders too
  dark.

## `app-egui` parity checklist is the tracked source of truth, not phase docs

`app-egui/` (the egui/eframe migration target replacing `app/`'s
dioxus-native UI - see `docs/issue-11-phase-11..13.md` for why) accumulated
8 straight commits of chrome/cosmetic fixes (rail hover threshold, card
sizing, title size, tile-status chip, sketches, stepper widgets) after
Phase 14 explicitly documented a per-tool deferred-functionality list -
zero of those commits touched that list, and it aged silently because it
only ever lived as prose in `docs/issue-11-phase-14.md`/`-15.md`. See
`docs/app-egui-parity-checklist.md` for the tracked table this replaces.

**Standing rule: no chrome/cosmetic-only `app-egui` commit lands while a
`P0` row in that checklist is OPEN**, unless the user explicitly asked for
cosmetic work specifically. Update the checklist row-by-row as each item
closes, citing the closing commit - it is the thing to check before
starting new `app-egui` work, not the phase docs.

- **A fixed bug recurred in a second UI implementation of the same
  feature.** `app/src/state.rs::notify_search_complete`'s own doc comment
  records a real, confirmed-on-Windows crash (~5s after every search
  completes) from firing `notify_rust::Notification::...show()`
  unconditionally on the async task that just finished a search - fixed
  there by making it opt-in (`desktop_notification_when_done`, defaults
  OFF) plus `spawn_blocking` + `catch_unwind`. `app-egui/src/search.rs`'s
  first working build (Phase 14) reintroduced the exact same
  unconditional, unprotected call when it built its own completion
  notification from scratch instead of checking whether this feature
  already had a documented fix elsewhere in the repo. Fixed by porting the
  real fix (see `app-egui/src/search.rs::notify_search_complete`), not
  re-deriving one. If you port a feature that already exists in the other
  UI stack, check that stack's own doc comments for a "found and fixed"
  history first - a feature name matching isn't enough assurance its
  first implementation didn't already teach the project something.

- **A `#[derive(Default)]` on a `#[serde(default)]`-heavy struct silently
  discards every field's intended default.** `app-egui/src/persistence.rs`'s
  `SearchFieldsSnap` gives each field a specific default via
  `#[serde(default = "fn")]` (e.g. `max_file_size_mb` → 50.0,
  `throttle_limit` → `default_throttle_limit()`) - but those attributes
  only fire when a field is missing from an ALREADY-PRESENT JSON object.
  When the whole `search` key is absent (any settings file saved before
  this struct existed), `PersistedState.search`'s own `#[serde(default)]`
  falls back to `SearchFieldsSnap::default()` - and a *derived* `Default`
  impl ignores every `#[serde(default = "fn")]` function entirely, giving
  `0`/`0.0`/`false` instead. Screenshot-confirmed real bug: every
  Performance-section field read `0` on first load after this struct grew
  past its original three fields. Fixed by hand-writing
  `impl Default for SearchFieldsSnap` to call the same `default_*()`
  functions, and having `SearchTool::new()` build its initial state from
  `SearchFieldsSnap::default()` rather than a second, separately
  maintained default list - one source of truth, so the two paths can't
  diverge again. If you add a field with a non-zero/non-empty default to
  a `#[serde(default)]`-per-field struct, verify `Self::default()` matches
  by testing the "whole struct missing" path, not just individual missing
  fields.
- **An unannotated `dyn FnMut(...)` (or any `dyn Trait`) parameter is
  never `Send`, even when every real closure passed to it is** - this
  makes an `async fn` taking `Option<&mut dyn FnMut(...)>` structurally
  `!Send` for ALL callers, a property of the function's own body, not of
  what any particular caller passes. `search_core::native_index::
  build_or_update_corpus_index` hit this: `app/`'s Dioxus caller works
  fine (Dioxus's task spawner doesn't require `Send`), but `app-egui`'s
  real `tokio::spawn` (a multi-thread `Runtime` always requires it) can't
  await it at all - not a bug in the closure passed, a structural
  mismatch between the `dyn`-typed API and a `Send`-requiring runtime.
  Adding `+ Send` to the trait object bound would have fixed `app-egui`
  but broken `app/`'s caller (its closure captures a Dioxus `Signal`,
  which is `!Send` by design). Resolved by adding
  `build_or_update_corpus_index_send<F: FnMut(...) + Send>` as a generic
  sibling (both delegate to a shared `?Sized`-generic private impl) rather
  than changing the shared function's bound either direction. If a
  `search-core` function needs to serve both this project's UI stacks,
  remember they have genuinely different `Send` requirements - don't
  assume adding or removing `Send` is a no-op for the other caller.
- **`app-egui` has no custom `FontDefinitions` - it renders with
  egui's bundled default fonts only (`Ubuntu-Light`/`Hack-Regular`/
  `NotoEmoji-Regular`/`emoji-icon-font`), and several Unicode symbols
  used throughout the UI don't exist in ANY of them.** Confirmed by
  directly inspecting each font's cmap with `fontTools` (a Python
  venv + `TTFont(...).getBestCmap()`), not assumed from how a
  screenshot happened to look: `⌀` U+2300 (diameter - used 10× across
  Bushing/Pressure-Vessel dimension labels), `✕` U+2715, `⬔` U+2B14
  (was the Pressure Vessel nav icon), `✓` U+2713, `✎` U+270E (was the
  Rename nav icon), `☽` U+263D (was the dark-mode toggle icon) all
  render as tofu boxes. `◉` U+25C9 (the Bushing nav icon) was present
  in `Hack-Regular` only - broken too, since nav icon labels render
  with the default proportional style, not monospace. Fixed by
  substituting each for a confirmed-present equivalent rather than
  bundling a new font asset for a handful of symbols: `Ø` U+00D8, `×`
  U+00D7, `■` U+25A0, `✔` U+2714, `🖊` U+1F58A, `🌙` U+1F319, `⚙`
  U+2699. **Before using any Unicode symbol outside common Latin-1/
  typography punctuation in this crate, verify it against egui's
  actual bundled fonts (`epaint_default_fonts` in the Cargo registry
  cache) rather than assuming it will render** - this bug was old and
  present across nearly every sketch label before anyone checked.

## Feature parity checklist (from the original PowerShell tool, via the C# port)

If refactoring search/matching/reporting, confirm none of these regress:
match modes (AnyLine / AllInFile / Proximity), exclude filters with
Line/File scope, exclude-folder matching by whole path segment (never raw
substring), whole-word matching (lookaround-based, not `\b`, so
punctuation-edged filters like "C#" work), regex mode (with a typed error
naming the bad filter instead of a bare crash), group-by (Created/Modified/
None), the extension type-to-filter/tick-list picker
(`search-core::models::extension_catalog` is the single source of truth
backing both the picker and the engine's default list) plus a
custom-extension add path, parallel processing with a throttle limit, the
incremental cache (fingerprinted by settings, keyed by path + size +
mtime) including that cache-reused files still stream through progress,
dry run, retry-with-backoff plus per-file timeout for locked/slow files
(including detecting a file truncated by a concurrent write mid-read),
symlink-safe and cancellable directory walking with periodic enumeration
progress, encoding detection (BOM → strict UTF-8 → Windows-1252 fallback),
CSV export's formula-injection guard, live streaming of results into the
UI as each file completes (not just after the whole run finishes), the
HTML report's dark-mode CSS, table of contents, per-filter bar chart, PDF
low-confidence flagging, and match highlighting, and (issue #2) the
native_search fast re-search index: per-folder placement, auto-exclusion,
and skip-reindex-if-unchanged.

## Target environment (do not relax these without discussion)

Windows 10 1809+ / Windows 11, `win-x64`. No internet access, no admin
rights, no pre-installed runtime of any kind required on the machine
running the built app. Build-time internet access (crates.io/NuGet restore
in CI) is fine and expected - it's only the *published, running
application* that must be fully self-contained and offline-capable. See
"Why dioxus-native, not dioxus-desktop" above for the one place this
requirement actively shaped a dependency choice.
