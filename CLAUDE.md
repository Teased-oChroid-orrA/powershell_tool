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

- **The Stress Solver toolbox's PINN training is ~20-30x slower in a debug
  build than a release build - measured, not assumed.** A user reported
  `single_hole_plate.toml` running at ~2.64s/step (a smaller network than
  the documented Kirsch baseline's ~150-200ms/step, yet ~13-17x slower -
  ruled out problem size immediately). Investigation (via direct Cargo
  feature-graph inspection, `cargo tree`, and a real timed
  `#[ignore]`d smoke test - `pinn-solver/src/runner.rs::tests::
  run_training_user_problem_step_time_smoke`, run explicitly with
  `--release`/without) found three real, independent factors, in order of
  actual measured impact:
  1. **Debug vs release build is the dominant factor by far (~23x alone)**:
     same code, same input, 2.18s/step in a debug build vs 93ms/step in
     release - already at/below the GPU baseline once built in release.
     `cargo run -p app-egui` (no `--release`) is fine for UI/layout
     iteration but is NOT representative of real training performance -
     don't judge Stress Solver perf from a debug run. `stress_solver.rs`
     now shows an in-app warning banner (`cfg!(debug_assertions)`) for
     exactly this reason - don't remove it without replacing the warning
     some other way.
  2. `run_training_user_problem` (`pinn-solver/src/runner.rs`) rejection-
     sampled the interior/boundary collocation points and rebuilt the
     per-hole `HashMap` of named point sets from scratch EVERY STEP, for
     potentially thousands of steps - `UserSamplingStrategy::
     sample_interior` reseeds a fixed-seed `LcgRng` every call, so this was
     pure waste (byte-identical output either way), not a stochastic-
     resampling design choice. Fixed by hoisting the sampling/`HashMap`
     construction to run once before the loop, mirroring the Kirsch path's
     (`run_training`) own established `int_norm` dirty-flag cache pattern.
  3. `burn-ndarray` (the CPU-only backend `pinn-solver`'s `ndarray-backend`
     feature selects, to avoid a GPU-driver dependency for a bundled
     desktop tool) compiled with neither its `simd` (macerator) nor
     `multi-threads` (rayon) Cargo features - the workspace's own `burn`
     dependency (`NeuralNetwork-Stress-Solver/Cargo.toml`) uses
     `default-features = false` with an explicit list that doesn't forward
     either to burn-ndarray (only burn's own disabled top-level `default`
     feature would). Fixed by adding a direct, `optional`, feature-only
     `burn-ndarray` dependency edge in `pinn-solver/Cargo.toml` (Cargo
     unifies features across every crate depending on the same package,
     so this activates them on the shared instance without pinn-solver
     needing to call the `burn_ndarray` crate's API directly). Measured
     effect was modest on its own (~5.5% on the `training_step` criterion
     bench) - most of the gain came from #1 and #2, not this.
  - `blas-accelerate`/`blas-openblas`/`blas-netlib` (burn-ndarray's BLAS
    acceleration features) were deliberately NOT enabled: Accelerate is
    macOS-only (this project's target is Windows-first per "Target
    environment" below) and OpenBLAS/Netlib need a build toolchain or a
    system-installed library, which would violate the "no pre-installed
    runtime of any kind" requirement for the *shipped* app (build-time
    dependencies are fine; this is a runtime-linking concern).
  - Manually fusing the physics residual (FD stencil → strain → energy in
    fewer tensor passes, avoiding duplicate derivative computation) was
    considered and is a real, valid technique for CPU perf in general -
    but the measured numbers above already reach GPU parity for this
    problem size without it, so it's deferred as a follow-up, not treated
    as required.

- **`step_physics_multi` (`pinn-solver/src/training_core.rs`) - the shared
  training-step function `UserDefinedProblem`/pin-lug both use - had NO
  constitutive-consistency mechanism at all, unlike Kirsch's own dedicated
  `step_physics`.** A user reported the Stress Solver toolbox's Von Mises
  field looking wrong for `single_hole_plate.toml` (elevated values near
  outer edges/corners, a smooth featureless blob everywhere else,
  including at the hole where a real stress concentration should be
  sharp). Investigation confirmed the Von Mises FORMULA itself was
  correct (`sqrt(sxx²−sxx·syy+syy²+3·sxy²)`, matching plane-stress theory
  exactly) - the bug was upstream, in what constrains the mDEM network's
  direct σxx/σyy/σxy output columns. `step_physics_multi`'s `active_terms`
  filters `"constitutive_consistency"` out of `ctx.problem.loss_terms()`
  (mirroring `step_physics`'s identical filter) but - unlike
  `step_physics`, which explicitly adds `const_loss.mul_scalar(lam_const)`
  back into the total loss - had nothing that ever added an equivalent
  contribution back. Net effect: for any mDEM domain (`output_dim == 5`)
  driven through this function, σxx/σyy/σxy were constrained ONLY by
  whatever boundary/interface term happened to read them directly (e.g.
  `UserDefinedProblem`'s `HoleBcTerm`, sampling just the 64 hole-ring
  points) - nowhere in the domain interior or at the outer boundary.
  Fixed by adding the identical mechanism `step_physics` already uses
  (construct `kirsch_problem::ConstitutiveConsistencyTerm` ad-hoc, reuse
  the domain's already-computed `"interior"` forward pass - no extra
  forward pass needed since `InteriorEnergyTerm` already requires that
  same pair) generically for every `output_dim == 5` domain, with the
  same fixed weight (`5.0`, outside SAW-BRDR) Kirsch uses. This also
  fixes pin-lug (same missing mechanism, unverified until this pass -
  confirmed via the full test suite, 229/229 passed including pin-lug's
  own convergence tests, zero regression).
  - **`StepOutput::*_scalar` fields are always UNWEIGHTED term values -
    weighting lives separately in `lam_*`/`lam_by_name`.** First attempt
    at this fix folded the fixed weight directly into `const_scalar`,
    caught by `step_physics_multi_single_domain_matches_step_physics_
    kirsch`'s `rel_close` assertion failing at exactly a 5.0x ratio (the
    fixed weight value itself) - a precise, diagnostic signature for this
    exact class of mistake. If you add a new fixed-weight (non-SAW) term
    to `step_physics_multi` in the future, keep its `StepOutput` scalar
    field unweighted, matching every other field's convention.
  - That same test's hand-rolled `model_single` reference
    (`FourTermKirschProblem` - deliberately only 4 terms, by its own doc
    comment) needed the identical constitutive-consistency term added to
    stay a genuine parity check once `step_physics_multi`'s real behavior
    grew a 5th term - a real, expected consequence of fixing a real bug,
    not a sign the fix was wrong. If you touch this area again and hit a
    similar hand-rolled-reference-vs-real-code divergence, check whether
    the reference itself needs updating before assuming the new code is
    broken.
  - Percentile-clipped colorbar (`stress_solver.rs`'s `field_to_pixels`)
    was fixed alongside this as a separate, independently-real
    contributing factor (a smooth wide-area max elsewhere in the field
    can visually wash out a hole's small, sharp high-value region even
    when the underlying physics is correct) - color mapping now clips to
    P2/P98 while the true min/max are still always reported numerically.

- **`UserDefinedProblem` had no adaptive mesh refinement (AMR) at all,
  unlike Kirsch's problem - a second, real contributing factor to the same
  Von Mises investigation above.** Fixed generically, not with Kirsch-
  specific code copied into a new problem: `pinn_core::amr::AdaptiveGrid`
  was hardcoded to `GeometryConfig` (Kirsch/pin-lug's single-hole-at-origin
  geometry), so `UserGeometry` (the N-hole type this toolbox uses) had no
  equivalent hook. Generalized via a new `AmrDomain` trait (`x_range`/
  `y_range`/`contains`/`lock_zones` - zero or more `(cx, cy, radius)`
  "must-stay-refined" zones, generalizing the old single-origin-hole
  assumption to N zones or none) with `AdaptiveGrid<G: AmrDomain =
  GeometryConfig>` (default type param - every existing Kirsch/pin-lug call
  site, always written as bare `AdaptiveGrid`, keeps compiling unchanged).
  `derive_amr_config(bounds, lock_zones)` replaces the depth-derivation
  formula duplicated per-problem, proven to reproduce Kirsch's own
  `engine.rs` numbers exactly for its default geometry (regression test).
  **Nothing about AMR eligibility is gated on "does this problem have a
  hole"** - a feature-less geometry just gets zero lock zones and runs
  pure residual-driven refine/coarsen with no permanent floor; this is the
  generic behavior the AMR machinery already had, just previously
  unreachable outside Kirsch's own hardcoded type.
  - Wired into BOTH `run_training_user_problem` (`UserGeometry`, single
    domain) and `run_training_pinlug` (`GeometryConfig` ×2 domains - the
    lug domain's real hole gets a genuine lock zone identical in shape to
    Kirsch's own; the pin domain, `HoleType::None`, gets zero zones and
    "just works" with no special-case code, which is the generic design
    paying off). Kirsch's own `run_training` AMR sweep was deliberately
    left untouched (already working/tested) - migrating its hand-rolled
    residual probe onto the new generic `probe_interior_energy_residuals`
    helper is a noted future consolidation, not done.
  - New generic helper: `training_core::probe_interior_energy_residuals`
    wraps the already-generic, ansatz-aware `compute_domain_forwards` +
    `energy::dem_energy_per_point` - works identically for
    `UserDefinedProblem`'s `IdentityAnsatz` and Kirsch's `QuarterSymmAnsatz`
    with zero problem-specific code, one call returning every domain's
    residuals at once.
  - **Natural byproduct, not scope creep**: wiring periodic AMR-driven
    resampling into `run_training_pinlug` required first fixing the exact
    same redundant-resampling bug found and fixed in `UserDefinedProblem`
    earlier - `PinLugSamplingStrategy::sample_interior` also reseeds a
    fixed-seed `LcgRng` every call, so `pin_int`/`lug_int` were being
    rebuilt from scratch, unconditionally, every step, forever. Extracted
    to `resample_pinlug_domains` (called once before the loop, and again
    inside `WarmStart` - the one case pin-lug's geometry can actually
    change mid-run).
  - Both new `#[ignore]`d integration tests (`run_training_user_problem_
    amr_sweep_changes_collocation_count`, `run_training_pinlug_amr_sweep_
    changes_collocation_count`) assert `n_colloc` CHANGES after the sweep,
    not spatial density near a hole - the stronger spatial claim is already
    covered, faster and more precisely, by `pinn-core`'s own unit tests
    (`amr::tests::adaptive_grid_over_user_geometry_refines_near_each_hole_
    zone`). `run_training_pinlug`'s own version measured ~345s even at
    `tiny_pinlug_config`'s tiny network size (1300 steps needed to cross
    warmup + one sweep interval) - too slow for the default suite.

- **"Neural-Network-Wide Adaptive Collocation" epic (Phases 0-2, 3-7, 12)
  audited and hardened the AMR work above - real gaps found, fixed, and
  verified, not assumed.** Full audit (Phase 0) confirmed: the indicator
  (`|dem_energy_per_point|`) is principled for a DEM-family solver, hole/
  boundary/domain geometry validity is solid, and duplicate points/minimum
  spacing are guaranteed by the quadtree's own structure - but the TRIGGER
  was pure step-count (no residual-magnitude/nonuniformity gating), NO
  point-count safety net was ever active (`max_active_cells` existed but
  was never set by any wired caller), and there was ZERO AMR observability
  (no logging, no telemetry) for either new wiring site. Attempting to
  measure single-sweep cost externally (`T(200)` vs `T(201)` steps) at
  ~115ms/step release baseline was NOISE-DOMINATED across separate process
  runs - a real finding: per-sweep cost needs in-process
  `Instant::now()` bracketing, not external before/after timing, and that
  instrumentation doesn't exist yet (tracked, not built - UI/instrumentation
  phases were explicitly deferred, see below).
  - **Phase 3 (spatial behavior)**: 6 new fast unit tests prove the
    residual-driven refine/coarsen mechanism itself - localized error
    concentrates refinement (2x+ density), uniform error stays uniform
    (no pathological clustering), multiple regions both refine
    independently, a boundary-hugging region refines like any other, a
    synthetic ring around a circular hole refines FROM THE RESIDUAL SIGNAL
    (not just the structural hole-zone floor - deliberately set a tiny
    `hole_zone_factor` to isolate this), and a single pathological spike
    doesn't starve the rest of the domain.
  - **Phase 4 (global coverage)**: new `AdaptiveGrid::coverage_stats()`
    (`min_local_density`/`max_local_density`/`density_ratio`/
    `mean_spacing`, all derived exactly from the quadtree's own known cell
    sizes - `mean_spacing` is a fast quadtree-native PROXY for nearest-
    neighbor distance, not a rigorous k-d-tree search, documented as such).
    Opt-in diagnostic only - never called in the training hot path.
  - **Phase 5 (indicator)**: investigated PDE residual (doesn't exist
    anywhere in this DEM-based codebase - would need new physics, not a
    signal swap), solution gradient (redundant with energy density, which
    is already a function of strain = `∇u`), and stress gradient (a real,
    well-motivated alternative for mDEM domains specifically - the
    constitutive-consistency residual `|σ_net - C:ε_fd|` this session's
    earlier fix trains against - identified as the concrete future
    candidate, NOT implemented without a measured before/after comparison
    per this epic's own Scientific Rule).
  - **Phase 7 (smart activation)**: new `AmrtConfig::residual_threshold`/
    `nonuniformity_threshold` + `AdaptiveGrid::should_adapt(&residuals)`,
    wired into both `run_training_user_problem`/`run_training_pinlug`
    (`update_residuals` always runs; `adapt()`+resample is gated). Both
    thresholds default to `None` EVERYWHERE, including `derive_amr_config`
    - deliberately NOT given a real default, since the residual signal's
    physical scale differs per problem (`ref_energy`-normalized) and a
    universal absolute threshold would be exactly the "arbitrary
    threshold" this epic's own rules warn against. "Cooldown" is already
    provided by `interval_steps` (no separate mechanism added); "expected
    benefit" is folded into the nonuniformity check itself, not a separate
    metric.
  - **Phase 12 (runaway prevention)**: `derive_amr_config` now sets a REAL
    `max_active_cells` (`4^(max_level-1)`, derived transparently from the
    same `max_level` already computed, not an independent arbitrary
    constant) and a new `AmrtConfig::max_growth_fraction` (`Some(1.0)` =
    at most double per single sweep, enforced via a new shared
    `enforce_cap`/`enforce_growth_fraction_cap` primitive refactored out of
    the existing `enforce_active_cell_cap`). Kirsch's own hand-tuned
    `engine.rs` config deliberately keeps both `None` (zero behavior
    change - out of scope, Preservation Rule).
  - Verification: 32/32 `pinn-core` amr tests (up from 25), full
    `pinn-solver` suite still 229/229 (0 failures, 6 ignored - one new:
    `run_training_user_problem_amr_overhead_baseline`), BOTH existing
    slow AMR integration tests independently re-run and confirmed passing
    after the Phase 12 default-cap change landed.
  - Explicitly deferred, by direction, not oversight: Phases 8-11 and
    13-22 (Von Mises pipeline re-audit, hole-boundary stress profiling,
    AMR effectiveness/ROI tracking, all UI/dashboard work, the parametric-
    PINN generalization/inference-guardrail phases) - large, separate
    scopes, not folded into this hardening pass.

- **Epic follow-up: Phases 9-11 (Von Mises re-audit, hole-boundary
  profiling, AMR effectiveness telemetry) and Phases 13/15/16 (AMR status
  UI, training-timeline markers, hole-stress results card) closed - Phase
  9 is a "verified correct, no rewrite" outcome, the rest are new,
  tested, generic functionality.**
  - **Phase 9 (Von Mises pipeline audit)**: independently re-derived the
    tensor-vs-engineering shear convention end to end and found it
    already correct and consistent - `compute_strains` uses the tensor
    convention (`eps_xy = ½(∂u/∂y+∂v/∂x)`), `compute_stress` converts via
    `σxy = εxy·E/(1+ν)` (correctly bakes in the tensor→engineering
    factor), and `strain_energy_density` uses `2·σxy·εxy` (correct for
    the tensor convention it's paired with). Von Mises formula, hole
    masking (`geometry.contains` excludes points - never interpolates
    across a hole), and unit scaling (`u_ref`/`px_pa` applied identically
    in training vs. visualization) all checked and correct. Per this
    epic's own explicit rule ("do not rewrite working code simply because
    another architecture appears better"), nothing was changed here -
    this is a documented negative finding, not a gap.
  - **Phase 10 (hole boundary stress profile)**: new, generic
    `user_problem::probe_hole_boundary_profile` (samples the solution at
    fine angular resolution around a hole's circumference via the same
    `assemble_stencil`+`compute_strains` pair `compute_domain_forwards`
    uses, physical-scale-first) and
    `stress_concentration_from_profile` (`Kt = max_von_mises /
    nominal_stress`, computed from live model output every time - the
    epic's own "do not hard-code Kt = 3" rule is enforced by construction,
    not just convention, and covered by a dedicated regression test
    asserting the result is NOT 3.0).
  - **Phase 11 (AMR effectiveness/ROI)**: real, in-process, per-sweep
    telemetry - `AmrSweepReport` (`pinn_core::messages`:
    `domain_label`/`step`/`points_before`/`points_after`/
    `residual_rms_before`/`residual_max_before`/`residual_rms_after`/
    `residual_max_after`/`sweep_duration_ms`) bracketed with
    `Instant::now()` directly around each sweep in both
    `run_training_user_problem` and `run_training_pinlug`, replacing the
    Phase 0 finding that external before/after step timing is
    noise-dominated at this problem size. Flows through
    `TrainingUpdate.amr_sweep`/`PinLugTrainingUpdate.amr_sweep` to the UI.
  - **Phase 13/15 (AMR status + training timeline UI)**: `app-egui`'s
    `stress_solver.rs` gained an "Adaptive Refinement" status card
    (ACTIVE/IDLE, last sweep step, collocation count before→after, sweep
    cost, residual RMS before→after and % change, sweep count) and
    `VLine` markers on the loss-vs-step chart at each real AMR sweep.
    **The chart's x-axis is message-arrival index, not real training
    step** - `apply_msg`'s bounded(1) `try_send` channel silently drops
    the newest queued `TrainingUpdate` under UI-poll contention (the
    OLD queued message survives, not the new one), so some real steps
    never reach the UI at all. Marker x-positions are therefore tracked
    separately (`amr_marker_positions: Vec<usize>`, pushed as
    `self.total_loss.len() - 1` at the moment a sweep report arrives)
    rather than computed from `AmrSweepReport.step` directly - if you
    add another per-step chart annotation, follow this pattern, not a
    step-number lookup.
  - **Phase 16 (hole stress analysis results card)**: new
    `HoleAnalysis`/`HoleBoundaryPoint`/`StressConcentration` types,
    real-computed per-hole in `run_training_user_problem`'s existing
    `send_vis` cadence (not a separate poll), rendered as a
    "Hole Stress Analysis" card on the Results step (nominal stress, max
    Von Mises, Kt, peak angle, plus a Von Mises-vs-θ plot that wraps the
    first point to θ=360° to close the circle visually).
  - **A real cross-crate dependency-direction constraint surfaced mid-
    Phase-16**: `HoleBoundaryPoint`/`StressConcentration` originally lived
    in `pinn-solver::user_problem`, but `TrainingUpdate` (which needs to
    carry `HoleAnalysis`) lives in `pinn-core`, and `pinn-core` cannot
    depend on `pinn-solver`. Resolved by relocating both types into
    `pinn_core::messages` - the same "solver computes the value, core
    owns the message shape" split this codebase already uses for
    `VisFields`. If you add another solver-computed diagnostic that needs
    to ride on `TrainingUpdate`/`PinLugTrainingUpdate`, its result type
    belongs in `pinn-core::messages`, not in the solver crate that
    produces it.
  - Verification: full `pinn-solver` suite re-run after all of the above
    (including the type relocation) - 234/234 passed, 0 failed, 6
    ignored, zero regressions. `pinn-gui` and `app-egui` both rebuild
    clean. `app-egui` launched as a real background process post-change
    (`cargo build -p app-egui && ./target/debug/app-egui &`) - stayed
    alive, empty stderr/stdout (no panic), menu bar showed it as the
    frontmost app ("GS Engineering - Toolbench"); the window itself did
    not appear in a `screencapture -x` capture and an `osascript`
    focus/activate attempt hung on a permission prompt and had to be
    backgrounded - the same window-focus limitation already on record in
    this file, not a new one. Treat this as "confirmed: doesn't crash on
    startup with the new UI code paths active," not as a full visual
    confirmation of the new cards' layout - that still needs a session
    with working `osascript`/screen-recording permission, or the user's
    own eyes.
  - At the time this section was first written, Phase 8 and Phase 14 were
    considered "substance covered elsewhere, not built standalone" and
    Phases 17-22 were unstarted. A follow-up pass (documented in the new
    section immediately below) built all of them for real - this
    paragraph is kept only as a historical record of that earlier,
    narrower state; see below for what actually exists now.

## Epic completion: Phases 8, 14, 17-22 ("Neural-Network-Wide Adaptive Collocation")

Closes out every remaining phase of the epic audited/hardened above. Per the
epic's own rules, nothing here was declared done because it compiled or a
test passed without a real, independent check backing it - see each
subsection for what was actually measured/verified, not assumed.

### Phase 8 - Plate-with-hole validation: hole-zone density diagnostic

The epic's core Phase 8 question - "does the AMR engine identify the hole
region as requiring additional resolution" - now has a real, numeric answer
instead of a plausible-looking picture. `pinn_core::amr::AdaptiveGrid::
zone_density(cx, cy, r)` (new) computes mean local point density (leaves
overlapping a circular zone) directly from the quadtree; `lock_zone_density()`
averages this over `self.geom.lock_zones()` (`0.0` for a feature-less
geometry - no hole to concentrate near, not a missing value). `AmrSweepReport`
gained `hole_zone_density_before/after` and `domain_mean_density_before/after`
- every AMR sweep on every domain now reports a `hole-zone / domain-average`
density ratio before and after, surfaced in `app-egui`'s "Adaptive
Refinement" card as `Hole-zone density: {before}x -> {after}x domain
average`. Verified two ways: a new `pinn-core` unit test
(`lock_zone_density_rises_after_a_sweep_driven_by_high_residual_at_the_hole_
ring`, isolates the residual signal from the structural hole-zone floor via
`hole_zone_factor: 0.0`, same technique `spatial_test_e` already established)
asserts the ratio actually rises under a synthetic hole-ring residual signal;
the real training-loop wiring reuses the same `AdaptiveGrid` instance already
driving each domain's real sweeps, not a parallel/duplicate computation.

Full before/after field snapshots (displacement/strain/stress/Von Mises
grids at the exact moment of each sweep) were considered and NOT built as a
separate capture - `VisFields` (extended for Phase 14 below) already carries
every one of those fields on its existing cadence, and pairing a full grid
snapshot to every sweep event would duplicate that data for a real cost
(another full forward pass) with no additional question it uniquely answers
beyond what the density-ratio number already answers directly. If a future
need specifically requires a visual before/after diff (not just the number),
build it by capturing two `VisFields` bracketing a known sweep step - the
data needed is already flowing.

### Phase 14 - Spatial diagnostic visualization: field selector + overlays

`VisFields` (`pinn-core::messages`) grew six fields: `eps_xx`/`eps_yy`/
`eps_xy` (strain), `pde_residual`, `amr_score`, `collocation_density` -
same `(ny, nx)` shape and NaN-outside-domain masking as the original six.

- **`eps_xx`/`eps_yy`/`eps_xy`**: Kirsch's path already computed these via
  FD stencil to derive stress - simply retained instead of discarded. The
  mDEM paths (`evaluate_vis_grid_mdem` in `runner.rs`, `evaluate_user_vis_
  grid` in `user_problem.rs`) did NOT have an FD stencil at all before this
  (direct network stress-column read, no derivative needed for stress) -
  both gained one, reusing the exact "physical scale before the FD
  derivative" convention `training_core::compute_domain_forwards`'s
  `is_mdem` branch and `user_problem::probe_hole_boundary_profile` already
  established, not a new convention invented for display.
- **`pde_residual`**: for mDEM domains (sigma is a direct, independently-
  learned network output), this is `|sigma_net - C:eps_fd|` - literally the
  same per-point quantity `step_physics_multi`'s `constitutive_consistency`
  training term already penalizes (`energy::compute_stress`/
  `constitutive_consistency_loss`, both already unit-tested in `energy.rs`),
  now surfaced for display instead of only existing inside a training loss.
  For Kirsch's `QuarterSymmAnsatz` (stress is analytically derived FROM
  strain, never an independent output) this is trivially and correctly
  `0.0` inside the domain, not NaN - there is nothing for a constitutive
  check to disagree with there, and the field says so explicitly rather
  than looking like missing data.
- **`amr_score`**: `|dem_energy_per_point|` evaluated on the vis grid - the
  literal signal `AdaptiveGrid`'s real residual-driven refine/coarsen
  decision uses (`training_core::probe_interior_energy_residuals`), not a
  new indicator invented for the picture.
- **`collocation_density`**: a genuine 2D histogram (`runner::
  bin_collocation_density`, `pub(crate)`) of the domain's current `int_norm`
  point set binned into the same grid - raw counts, not an estimate.

`app-egui/src/stress_solver.rs` gained a `SpatialField` selector (13 real
fields + `BoundaryResidual`, which has NO backing data on this DEM-based
solver architecture - see the Phase 9 audit above - and says so explicitly
when selected rather than being silently omitted or fabricated), a
`ColorScale` mode (`Linear`/`Log`/`PercentileClipped`, default
`PercentileClipped` = the pre-existing behavior, unchanged), and max/min
value markers on the heatmap. The hole-boundary overlay (color-coded
Free/Fixed circles) already existed and needed no change. **Not built**:
literal scatter overlays for individual collocation/AMR-refinement points
(the epic's "☑ Collocation points"/"☑ AMR refinement points" checkboxes) -
`TrainingUpdate` never carries raw point coordinates, only the binned
density field and scalar counts; adding per-point scatter would mean
streaming every point's (x, y) over the channel every vis cadence, a real
new data-volume decision, not a wiring gap. `collocation_density` (the
field type) already surfaces this information in aggregate today.

New composition tests (`user_problem.rs`): mask-consistency across every
new field vs. the pre-existing `von_mises` mask, `pde_residual`
finite/non-negative and NOT trivially zero everywhere (catches the "compared
a value against itself" bug class directly - a real, freshly-initialized
network's direct stress essentially never exactly equals its own FD-derived
counterpart), `amr_score` non-negative everywhere, and an exact
hand-computed `collocation_density` histogram check.

### Phases 17-18 - Inference envelope: parameter classification + guardrails

New `pinn_core::inference_envelope` module (pure data/logic, no burn/IO
dependency - fits `pinn-core`'s existing "plain data-type crate" role).

**The load-bearing finding, verified by reading `pinn_solver::engine::
EngineParams::net_input_dim` directly, not assumed**: `ElasticityNet`'s
input is `net_input_dim()` - either `3` (raw `x, y, z=0`) or `4 * n_fourier`
(a Fourier embedding of `x, y`). **No geometry, material, or load value is
ever part of the network's input.** Every other `ProblemSpec` field is baked
into training only - geometry shapes the collocation sampling domain and
hole boundary-condition loss terms, material shapes the constitutive law
used to normalize/interpret the network's stress-column outputs, load
shapes the traction targets and the stress-column physical scale. There is
also no model checkpoint save/load anywhere in this codebase (verified: no
`Recorder`/`.mpk` usage in `pinn-solver` at all) - every trained model lives
and dies within one training run's process lifetime.

Direct consequence, stated per the epic's own explicit rule ("do not claim
`Train once, change anything instantly`"): **a trained `UserDefinedProblem`
model is a solution to exactly one `ProblemSpec`, not a parametric
surrogate over a family of them.** `user_problem_parameter_envelope()`
classifies every `ProblemSpec` field accordingly - `SafeForInference`
(query point `(x, y)` only), `RequiresGeometryRegeneration` (plate
extents, hole count/center/radius), `RequiresNewBoundaryConditionTraining`
(a hole's `Free`/`Fixed` flag alone), `RequiresRetraining` (material, load),
`Unsupported` (network architecture - a differently-shaped model, nothing
to reuse at all). `training.*` fields are absent from the table on purpose
- they have no meaning at inference time.

`classify_inference(trained, requested, query_point) -> InferenceClass`
(`Green`/`Yellow(reason)`/`Red(reason)`) is the Phase 18 guardrail: checks
network -> material -> load -> geometry in severity order (most structurally
broken reported first), then - if a query point was given - whether it
falls inside the trained geometry's validated region (`Yellow` if not,
`Green` otherwise). 12 fast unit tests cover every branch, including that a
simultaneous network+material mismatch reports the network break (the more
severe one), not whichever was checked last.

**Not wired into a live save/load-based inference workflow** - there is no
such workflow in this codebase yet (see the checkpoint-persistence finding
above). This module is ready to gate one the moment model persistence is
added; until then, its real, tested use is guarding any in-process
"what if I changed X" comparison against a live trained model (see Phase 19
immediately below, which does exactly that).

### Phase 19 - Generalization validation: measured, not claimed

`runner::tests::generalization_perturbing_material_after_training_
measurably_degrades_constitutive_residual_and_is_flagged_red` (`#[ignore]`d,
real ~300-step training, ~60s in debug - run explicitly with `--ignored`):
trains a small `UserDefinedProblem`, then calls `evaluate_user_vis_grid`
twice on the SAME trained model - once with the original material, once
with `material.e` perturbed 3x, `u_ref`/`px_pa` held at their ORIGINAL
trained values (simulating "reuse this trained model with a changed
material, without retraining", consistent with Phase 17's classification
that displacement/stress scale is itself baked into training). Asserts the
`pde_residual` RMS measurably worsens (>20%) under the perturbation - a
real, measured degradation, not an assumed one - AND that `classify_
inference` independently flags the same change `Red`/`RequiresRetraining`.
This is the epic's Phase 19 requirement satisfied honestly: it does NOT
claim "train once, change anything instantly" works within some validated
envelope - it demonstrates concretely why it currently doesn't for material,
and that the Phase 18 guardrail catches exactly that case.

### Phase 20 - Performance protection: re-verified after all epic work

**`training_core::step_physics` (Kirsch's real per-step function) and
`step_physics_multi` (`UserDefinedProblem`/pin-lug's shared per-step
function) - the two functions that actually dominate the documented
~80-second/950-step Kirsch release baseline and the debug/release
investigation's own measured per-step costs - have zero lines changed
anywhere across this epic's Phase 8-19 work.** Confirmed by inspection: every
edit in this pass touched `pinn-core::amr`/`pinn-core::messages`/
`pinn-core::inference_envelope` (new), `pinn-solver::user_problem`'s vis-grid
evaluator, and `pinn-solver::runner`'s AMR-sweep-adjacent code and vis-grid
evaluators - never the per-step hot-loop functions themselves. No per-step
regression is possible from this pass by construction, not by assumption.

The one place real per-call cost was added to an EXISTING operation: the
Phase 14 field extension, on vis-grid evaluation (already periodic before
this epic - every 10 steps for `UserDefinedProblem`, every 50 for
Kirsch/pin-lug - never the per-step path). Measured directly (`runner::
tests::evaluate_user_vis_grid_call_cost_is_small_relative_to_the_vis_
cadence`, `#[ignore]`d, release + `ndarray-backend`, 64x64 grid, 2048
collocation points): **40.78ms per call.** Against the documented ~93ms/step
release baseline and a 10-step vis cadence (~930ms between vis calls for
`UserDefinedProblem`), that's roughly 4.4% of the inter-vis budget - real,
measured, and small, not assumed negligible.

The `step_physics` criterion bench (`benches/training_step.rs`, `medium`
tier) was also run post-epic as an extra check; its absolute number isn't
directly comparable to the documented ndarray-backend baseline (the
workspace's default features build against the `wgpu` backend, and no
`--features ndarray-backend` flag was passed for that particular run) so it
is NOT the evidence this section relies on - the zero-lines-changed fact
above is. The bench run itself completed without error, confirming the
benchmarked code path still executes correctly post-epic.

### Phase 21 - Regression test architecture: gaps closed

Per the epic's own fast-test list (scoring, selection, localized/multi-region
refinement, coverage, duplicate suppression, geometry validity, point
limits, trigger logic, cooldown, determinism), everything already existed
from earlier phases EXCEPT duplicate suppression and determinism, which had
no dedicated test despite both being real, checkable properties of
`AdaptiveGrid`. Added: `sample_points_has_no_exact_duplicates_after_several_
adapt_cycles` (exact-bits `HashSet` check across several real adapt cycles)
and `adapt_is_deterministic_given_the_same_residual_sequence` (two freshly
constructed grids fed the identical residual sequence must produce
bit-for-bit identical `sample_points()` output). "Cooldown" has no dedicated
unit test - it's `interval_steps`-gated inline inside each training loop
(`(step - AMR_WARMUP_STEPS) % amr_interval == 0`), not a separately callable
function, and extracting one purely for testability would be exactly the
"rewrite working code because another architecture appears better" the epic
warns against; its behavior is still exercised end-to-end by every slow
integration test that crosses multiple sweep intervals. Slow-integration-test
side of the split was already complete (`run_training_user_problem_amr_
sweep_changes_collocation_count`, `run_training_pinlug_amr_sweep_changes_
collocation_count`, both `#[ignore]`d; the new Phase 19 generalization test
joins this category for the same reason).

**A pre-existing flaky test was found (and left alone, not "fixed") during
this pass's full-suite verification**: `headless::tests::run_headless_
width_growth_disabled_is_byte_identical_to_pre_change` failed once in a full
`cargo test -p pinn-solver` run (`a=246.56345 b=246.6421`,
`rel_err=0.00032`), a genuine numerical divergence between two live
`run_headless` calls being compared against each other, not against a
hardcoded golden value. Investigated per this file's own evidence-before-
assumptions standard, not assumed innocent: this file's own earlier
performance-investigation entry already documents that `burn-ndarray`'s
`multi-threads` (rayon) feature was deliberately enabled - a well-known
source of run-to-run float-summation-order nondeterminism under any kind of
scheduling/contention variance, completely independent of program logic.
Re-ran the single test in isolation 3/3 clean, then re-ran the FULL suite a
second time end-to-end and got 238/238 clean (0 failures) - same code, same
suite, one flake in two full runs. This session's own Phase 8/14/17-21 work
never touches `headless.rs` or `width_growth` code at all (verified by
inspection, not assumed) - there is no plausible causal path from anything
in this pass to that specific test's numerics. Left as a known, real,
pre-existing flake rather than "fixed" - the epic's own rule is to fix a
demonstrated defect in the smallest appropriate layer, not to silence a
flaky assertion by loosening its tolerance without understanding why it's
flaky first (a separate, legitimate investigation this pass did not have
grounds to start).

### Phase 22 - This section

Documents exactly what Phases 8/14/17-21 above actually built, per the
epic's own "document only after implementation has been verified" rule -
nothing here describes intended behavior the code doesn't actually have.
Full verification after this pass: `cargo test -p pinn-core` 77/77 (up from
72 before this pass, via `inference_envelope`'s 12 new tests plus `amr::`
growing to 37), `cargo test -p pinn-solver` 238/238 (0 failed, 8 ignored -
up from 234/0/6; see the flaky-test paragraph above for the one test that
failed on a DIFFERENT full-suite run and was confirmed unrelated), `cargo
build -p pinn-gui`, `cargo build -p app-egui`, and `cargo build --workspace`
from `NeuralNetwork-Stress-Solver` (confirms the default/`wgpu`-backend
build path - not just the `ndarray-backend` path this project's own app
actually ships - also stays green). A real `app-egui` launch (`cargo build
-p app-egui && ./target/debug/app-egui &`) with every new UI code path
active (field selector, color scale, extrema markers, hole-zone density
line) stayed alive 20s with an empty log - no panic - matching this
project's own established verification discipline.

## Follow-up pass: scroll fix, engineering-dashboard items, and a real parametric PINN

A later session picked up two things: a real, reported UI bug (the Adaptive
Refinement card was unreachable - no way to scroll to it), and a review of
`/Users/nautilus/Desktop/enhancement.txt` (an external critique of the
Results screen, proposing an engineering dashboard and - most
substantially - a genuinely PARAMETRIC PINN: `x,y,E,nu,Load -> u` instead
of `x,y -> u`, trained across ranges, with instant re-inference at a new
point plus an honest validity check). The user explicitly chose to build
the parametric architecture now, not defer it.

### Bug: `stress_solver.rs`'s content column had no `ScrollArea`

Confirmed by inspection: `stress_solver.rs` had zero `ScrollArea` usage
anywhere, unlike `search.rs`/`bushing.rs`/`components.rs`, which already
use one. Once the Phase 8/13/14/16 cards stacked (Stress Field, Adaptive
Refinement, Hole Stress Analysis, and now Solution Summary/Model
Contract/Model Validity Envelope), the column could exceed the window
height with no way to reach what scrolled off - exactly the reported
symptom. Fixed by wrapping `step_content`'s render call in
`egui::ScrollArea::vertical().auto_shrink([false, false])` inside the
`side_by_side` content closure - `auto_shrink([false,false])` matters:
the default shrinks the area to fit its content instead of filling the
available height and showing a scrollbar on overflow, which would silently
undo the fix. `pressure_vessel.rs`'s `step_content` has the same
missing-`ScrollArea` shape and was NOT touched here (out of scope - only
the reported tool was fixed).

### Engineering-dashboard items built directly (no architecture change needed)

- **Solution Summary card** (Results step): max/min displacement, max/avg
  Von Mises, max principal stress (`(sxx+syy)/2 + sqrt(((sxx-syy)/2)^2 +
  sxy^2)`), energy (final loss value), PDE residual RMS/max - every value
  computed directly from the existing `VisFields` grid already sent on the
  normal vis cadence, no new solver-side plumbing. **Explicitly NOT
  included**: reaction force and force-equilibrium error - both need an
  integral of stress over the actual boundary POINT SET, not the
  visualization grid, a genuinely separate computation this pass didn't
  build. Said so directly in the card rather than faking a number.
- **Model Contract card** (Results step, non-parametric `Plate` specs
  only): lists the trained spec's exact E/nu/Load/geometry/network values
  next to what `pinn_core::classify_inference`/`ParameterClass` actually
  says about changing each one - real classification output (see the
  earlier Phase 17/18 section above), not a hand-written summary. Points
  the user at the parametric spec format for the case they actually want
  "change a number, get a new solution."
- Field selector, color scale, extrema markers, hole-zone density line,
  training-timeline AMR markers: already existed from the earlier pass
  documented above - re-verified working after this pass's changes, not
  re-built.
- **Not built this pass** (real, stated gaps, not silently dropped):
  gradient-norm tracking (would require instrumenting
  `step_physics_multi`'s `backward()` call, a solver-side change this pass
  chose not to make), a rasterized boundary-condition-residual field (the
  `BoundaryResidual` field-selector entry still honestly reports "no
  backing data" - see the earlier Phase 9/14 section), and a training-
  case-vs-new-case comparison/difference-field view (needs a stored
  baseline-run mechanism that doesn't exist yet).

### A real parametric PINN: `pinn_core::parametric_spec` + `pinn_solver::parametric_problem`

**The core architecture change**: `ElasticityNet`'s input grows from 3
columns (`x, y, z=0`) to 6 (`x, y, z=0, e_n, nu_n, p_n` - `E`/`nu`/`Px`
each linearly mapped to `[-1,1]` via the new `ParamRange::normalize`).
Every forward pass now optionally carries the CURRENT `(E, nu, Px)` sample
tiled onto every row (`parametric_problem::tile_params`/`with_params`) -
this is what makes the network actually condition on the physical
parameters, not just have them affect which loss target it's trained
against.

**Deliberately built as a new, self-contained module
(`pinn_solver::parametric_problem`), NOT by modifying `step_physics_multi`/
`compute_domain_forwards`.** Those functions assume ONE fixed
material/load baked into `DomainSpec` at construction time - there's no
seam for a per-step-varying, network-INPUT-conditioning value without
touching the shared forward-pass pipeline `UserDefinedProblem`/pin-lug/
Kirsch all depend on. Per this project's own established discipline (a new
sibling instead of touching working, tested code - `AdaptiveGrid`,
`derive_amr_config`, `resample_pinlug_domains` all did the same thing),
`parametric_problem.rs` reuses only the PURE, already-tested primitives
that don't care where their inputs come from - `assemble_stencil`/
`compute_strains`/`norm_pts_to_tensor` (`fd_stencil.rs`), every function in
`energy.rs`, `SawBrdr`, `LrSchedule`, `WeightOptim`/`BiasOptim`/`GateOptim`,
`UserSamplingStrategy` (geometry is fixed in v1, so its `(x,y)` point
generation needs no change) - and writes its own linear step function
(`step_parametric`) mirroring `step_physics_multi`'s actual SAW-BRDR/
constitutive-consistency/optimizer-step structure by hand, not by calling
it. `training_core::step_physics`/`step_physics_multi` have zero lines
changed by this work.

**v1 scope, stated explicitly** (`pinn_core::parametric_spec`'s own doc
comment): only `material.e`, `material.nu`, and `load.px` are parametric.
Geometry (plate extents, hole count/position/radius/bc), `material.density`/
`ultimate_strength_pa`, and `load.py` are all FIXED - matching
`enhancement.txt`'s own item 13 risk ranking (geometry/topology changes are
"much more dangerous" than a continuous material/load value). One `(E, nu,
Px)` sample is drawn PER STEP (not per point) via a seeded `LcgRng` - a
real, stated simplification (not per-point-varying) chosen specifically so
every loss term's formula stays byte-identical to `UserDefinedProblem`'s
existing math, just fed a step-local `MaterialProps`/`LoadConfig` instead
of a spec-fixed one. Reference scales (`u_ref`, stress scale, `ref_energy`,
`ref_stress2`) are fixed once for the whole run from each range's
worst-case magnitude (`ParamRange::max_abs`) - a property of the
normalization scheme, not recomputed per step, matching how every other
training path in this codebase already treats these scales.

**Instant inference after training - no checkpoint persistence exists, so
the training thread stays alive.** Confirmed (again) that this codebase
has no model save/load anywhere (see the Phase 17/18 section's finding).
`run_training_parametric` therefore does not return after
`training.max_steps` - it sends `TrainingMsg::ParametricReady` once, then
blocks on `stop_rx.recv()` answering `ControlMsg::ParametricInfer { e, nu,
px }` requests (each answered with a fresh `evaluate_parametric_vis_grid`+
`probe_hole_profile_parametric` pass against the SAME trained model,
wrapped in `TrainingMsg::ParametricInferResult`) until `ControlMsg::Stop`.
Validity is a min/max range check (`ParametricProblemSpec::in_range`) -
`enhancement.txt` item 10's own explicit note that a real
distance-to-training-distribution metric is a future refinement, not
required for a first, honest version, so this pass didn't build one.

**UI** (`app-egui/src/stress_solver.rs`): `LoadedSpec` gained a
`Parametric(ParametricProblemSpec)` variant - `load_spec` tries
`ParametricProblemSpec` before `ProblemSpec` (distinct required fields:
`e_range`/`nu_range`/`load_range`), so pointing the spec-path field at a
parametric TOML just works. Once `parametric_ready`, the Results step shows
a "Model Validity Envelope" card: sliders for E/ν/Load (bounded to the
trained ranges but draggable ~25% past either end via
`egui::SliderClamping::Never`, deliberately, so the user CAN ask for an
out-of-range value and see the honest RED result), a "Calculate New
Solution" button (`run_instant_inference`, sends `ControlMsg::
ParametricInfer`), and the returned field heatmap/hole analysis rendered
through the SAME `field_heatmap`/`SpatialField` machinery the training view
uses (`field_heatmap` was refactored to take `vis: &VisFields` as an
explicit parameter instead of reading `self.vis`, specifically so it could
serve both the live training snapshot and an instant-inference result
without duplicating the renderer).

**A real TOML gotcha, caught by a test before it could bite the UI**: the
shipped example (`examples/problems/parametric_single_hole_plate.toml`)
originally placed `e_range`/`nu_range`/`load_range`/`density`/
`ultimate_strength_pa` AFTER `[[geometry.holes]]` - TOML parses any bare
`key = value` line following an array-of-tables header as belonging to
that array's CURRENT entry, not the document root, so all five fields
silently vanished into `geometry.holes[0]` and `toml::from_str` reported
"missing field `e_range`" at the document's own first line. Fixed by moving
every top-level field before `[[geometry.holes]]`, with a comment in the
file itself explaining why. Caught by
`parametric_spec::tests::shipped_parametric_example_spec_parses` (mirrors
`problem_spec.rs`'s own shipped-example regression-guard pattern) - written
and run BEFORE attempting any real training via the UI, exactly so this
class of mistake couldn't surface as a confusing runtime error instead.

**A real backend-speed flake, found and fixed properly, not silenced**: the
first version of `run_training_parametric_completes_and_sends_updates_and_
ready` (15 steps, polling `rx.try_recv()` for up to a fixed `2000 * 5ms =
10s`) passed reliably under `--features ndarray-backend` but failed once in
a full default-feature (`wgpu` backend) suite run with "must send
ParametricReady after training completes" - not a logic bug: `wgpu`'s
fixed per-dispatch overhead (already documented elsewhere in this file as
large even for tiny tensors) pushed 15 steps' wall-clock past the fixed
10-second budget. Confirmed by isolating the test (passed reliably alone)
and re-running the full suite a second time (241/241 clean, zero repeats of
the failure). Fixed properly: reduced to 6 steps (still enough to prove
different steps sample different `(E,nu,Px)` triples) AND replaced every
fixed-iteration poll loop across all three parametric tests with a genuine
60-second WALL-CLOCK deadline (`std::time::Instant`), so correctness no
longer depends on guessing a backend's speed. Verified clean under both
`--features ndarray-backend` (9/9 pinn-core parametric_spec tests, 3/3
parametric_problem tests) and the default `wgpu` build (241/241 full
suite, two independent runs).

**Verification for this pass**: `cargo test -p pinn-core` 86/86 (up from
77 - 9 new `parametric_spec` tests), `cargo test -p pinn-solver` 241/241 (0
failed, 8 ignored - up from 238/0/8, +3 new `parametric_problem` tests),
`cargo build -p pinn-gui` (needed one new no-op `TrainingMsg` match arm for
the three new parametric variants - this GUI has no parametric mode, same
treatment as the existing `BeamUpdate` no-op), `cargo build -p app-egui`,
and a real `app-egui` launch (all-new UI active: scroll fix, Solution
Summary, Model Contract, Model Validity Envelope) stayed alive 17s with an
empty log - no panic.

## Follow-up pass: closing enhancement.txt's remaining gaps (grad norm, BC residual, tri-state validity, comparison view)

A later session re-audited `/Users/nautilus/Desktop/enhancement.txt` item-by-
item against the actual code (not against the prior pass's own summary) and
found four real, previously-honestly-stated gaps still open: no gradient-norm
telemetry, no real boundary-condition residual (distinct from the pre-existing
interior/PDE residual), the GREEN/YELLOW/RED validity classification described
in the earlier "Model Validity Envelope" section was actually just a range
check (no physics-based residual comparison), and no training-case-vs-new-case
comparison/difference-field view. All four closed this pass, plus two
incidental real bugs found while extending this code.

- **Gradient norm** (`training_core::StepOutput::grad_norm: Option<f32>`):
  computed in `step_physics` (Kirsch) and `step_physics_multi` (`UserDefined
  Problem`/pin-lug) from the already-built `GradientsParams` (weight/bias/gate)
  via the existing `flatten_grads` helper, READ BY REFERENCE before those
  values are moved into the optimizer `.step()` calls - a provably pure,
  zero-effect addition. Verified via this project's own most-protected
  byte-identical-trajectory regression tests
  (`step_physics_compute_skip_disabled_by_default_matches_pre_change_
  trajectory`, `step_physics_multi_single_domain_matches_step_physics_kirsch`,
  `step_physics_trait_driven_matches_independently_reimplemented_old_formula`,
  `kirsch_regression_matches_hardcoded_step_physics`) - all still pass
  byte-identical. `None` on every L-BFGS/synthetic path (no comparable
  single-step gradient norm exists there) - stated honestly, not faked as
  `0.0`. Parametric path's own `grad_norm: f32` (always real, computed inside
  `step_parametric` the same way) - see below.
- **BC residual RMS/max** - real per-point traction/displacement residual at
  the outer boundary and every hole ring, DISTINCT from `VisFields::
  pde_residual` (the pre-existing interior constitutive-consistency check).
  Added for the parametric path (`parametric_problem::bc_residual_stats`,
  generic, shared between periodic training updates and on-demand inference
  queries) and for `UserDefinedProblem` (new `user_problem::
  probe_boundary_residuals`, mirrors the parametric path's approach, does NOT
  touch `step_physics_multi`). Both combine outer-boundary Neumann-traction
  residual and every hole's Free (traction magnitude)/Fixed (displacement
  magnitude) residual into one RMS/max pair via the existing
  `training_core::residual_stats`. **Deliberately NOT added for Kirsch's own
  path or pin-lug** - Kirsch's BC residual would need new plumbing this pass
  didn't build (left `0.0`/`0.0`, stated as a real deferred gap, not silently
  faked); pin-lug's BC is Signorini contact/KKT, not a simple prescribed
  traction, so "BC residual" isn't the same well-defined quantity there -
  inventing one would be a new metric definition, out of scope.
- **GREEN/YELLOW/RED validity, now actually physics-based**
  (`stress_solver.rs::classify_infer_result` -> `Verdict`): RED whenever
  `!in_range` unconditionally (parameter-space extrapolation always flagged).
  Otherwise compares the instant-inference query's PDE/BC residual RMS
  against THIS RUN's OWN trained-baseline residual (a ratio against the
  model's own observed training-time residual, not an arbitrary hardcoded
  absolute threshold - `>10x` baseline -> RED even if in-range, `>3x` ->
  YELLOW, else GREEN) - satisfies `enhancement.txt`'s own explicit caution
  against unearned precision in thresholds. Baseline comes from
  `TrainingCaseSnapshot`, a stable snapshot of the latest real training-time
  `(E,nu,Px,vis,hole_analyses)` overwritten every vis-cadence update -
  deliberately separate from `self.vis` (which could be stale or belong to a
  different run) so the comparison always has a coherent reference point.
- **Training Case vs New Case comparison** (`training_vs_new_case_card`):
  metrics table (E/nu/Load/max sigma_VM/Kt for hole 0) plus a computed
  New-minus-Original difference field (`difference_vis`, elementwise
  `ndarray::Zip` subtraction, NaN-propagating) rendered through the SAME
  `field_heatmap` machinery the training view uses - no new renderer.
- **Solution Summary** gained PDE residual P95, BC residual RMS/max, and
  last-step gradient norm - all real solver-side telemetry now, not derived
  client-side approximations.
- **Boundary point color overlay**: hole-ring points colored by Von Mises via
  the existing `viridis` colormap, using the profile's OWN local min/max (not
  the currently-selected field's `colorbar_range`, which could belong to a
  different field entirely - caught and fixed before finalizing).
- **Incidental bug #1, found while extending this code**: the pre-existing
  hole-boundary circle-stroke overlay in `field_heatmap` only ever matched
  `LoadedSpec::Plate` - parametric runs got ZERO hole overlay, silently.
  Fixed by making the block match both `Plate` and `Parametric`.
- **Incidental bug #2, a real TOML gotcha, same class as the one already on
  record above for the shipped parametric example**: none new this pass, but
  worth restating why `shipped_parametric_example_spec_parses`-style
  regression guards matter - this codebase's spec files are hand-edited TOML
  and this exact array-of-tables field-ordering mistake is easy to reintroduce.
- **A real test-authoring bug, not a production bug**: the first version of
  `run_training_parametric_completes_and_sends_updates_and_ready` called
  `run_training_parametric` SYNCHRONOUSLY with an unused control-sender,
  hanging forever in the post-training `stop_rx.recv()` loop - that loop
  staying alive after training IS the intended, correct instant-inference
  design (see the parametric section above), not a bug; the TEST needed to
  run it on a thread and explicitly send `Stop`. Diagnosed via targeted
  `eprintln!` instrumentation (confirmed training itself completed in ~1-2s;
  the hang was specifically in the intentional post-training serving loop),
  then fixed and the instrumentation removed.
- **A real backend-speed test flake, same class as this file's already-
  documented `wgpu`-vs-`ndarray-backend` timing note**: the same test's fixed
  `2000*5ms=10s` poll budget was insufficient under the default `wgpu`
  backend even though it passed reliably under `--features ndarray-backend`.
  Fixed properly (not by loosening an assertion): reduced step count and
  replaced every fixed-iteration poll loop across all three parametric tests
  with a genuine `std::time::Instant`-based 60-second wall-clock deadline
  (shared `wait_for_ready`/`wait_for_result` helpers).
- **A real background-task pitfall worth recording for future verification
  passes**: a "full suite" background test run can silently use an
  ALREADY-BUILT, STALE test binary if cargo's freshness check finds nothing
  changed relative to what it last compiled for that exact feature set - this
  happened here (a `wgpu`-backend full-suite run reported a clean "241
  passed, 0 failed" that, on inspection, simply didn't contain the two newest
  `probe_boundary_residuals` tests at all, because that specific binary had
  last been built before those tests were added, and a `--features
  ndarray-backend` build in between doesn't share the same fingerprint/
  target). A "0 failed" result is not proof of freshness by itself - if a
  just-added test's name doesn't appear in the full output at all, that's a
  stronger tell than the pass/fail count. Confirmed via a second, deliberately
  fresh full run (compiled from a clean state) showing all 243 tests
  including both new ones, 0 failed, 8 ignored.

**Final verification for this pass**: `cargo test -p pinn-core` (unchanged by
this pass, not re-run), `cargo test -p pinn-solver` (fresh rebuild) 243/243
passed, 0 failed, 8 ignored (up from 241/0/8 - +2 `probe_boundary_residuals`
tests; the parametric path's own 3 tests already counted in the prior pass's
241), `cargo test -p pinn-gui --no-run` and `cargo test -p pinn-core --no-run`
both clean, `cargo build -p app-egui` clean/zero-warnings, a real `app-egui`
launch (`cargo build -p app-egui && ./target/debug/app-egui &`) stayed alive
15s+ with an empty log (no panic) with every new UI code path active
(GREEN/YELLOW/RED card, Training Case vs New Case comparison, boundary point
overlay, Solution Summary's new rows) - `osascript`/`screencapture` focus
attempt hung on the same permission-prompt limitation already on record in
this file, so this is "confirmed alive, no panic," not a full visual
confirmation; that still needs a session with working screen-recording
permission or the user's own eyes.

## Follow-up pass: enhancement.md's full epic - units, force/energy validation, distance metric, in-app editing, model persistence, live network visualization

The user replaced `enhancement.txt` with a larger, restructured `enhancement.md` (same critique,
formalized into ~65 phases plus a full Unit-System addendum) and asked for a genuine gap audit
against the ACTUAL code, then implementation of everything still missing, plus four explicit
follow-up asks made mid-session: in-app editing of problem inputs (no TOML round-trip), model
checkpoint save/load, and a live network-evolution visualization beside the heatmap. Two parallel
read-only audits (not assumption) established real ground truth before any code changed - see
the plan file this pass worked from for the full citation trail. Confirmed already-done: Phases
0-42's substance (training dashboard, physics residuals, AMR observability, parametric PINN,
GREEN/YELLOW/RED validity, comparison view - all documented in the sections above). Confirmed
genuinely missing: a real unit system (only 5 one-way SI->USCS display constants existed, no
`UnitSystem` toggle anywhere), a diverging colormap for the difference field, force-equilibrium
validation, a real energy balance (vs. the raw optimizer loss), and a distance-to-training-
distribution metric (only min/max rectangle containment existed).

### Stage A - Unit system (`pinn_core::units` + app-egui display layer)

Extended the existing `pinn-core/src/units.rs` (already the established SI<->USCS conversion-
constant module) rather than starting fresh: added `UnitSystem` (`Uscs`/`Si`, `Uscs` default per
the doc's explicit requirement), `PhysicalQuantity` (`Length`/`Stress`/`Force`/`Energy`/`Strain`/
`Dimensionless`), and `pick_unit`/`to_unit`/`convert_for_display`/`convert_from_display`/
`format_value` - a magnitude-based engineering-unit picker (ksi vs psi, mm vs m, etc.), split so
several related values (a slider's current value AND its min/max range) can share ONE picked
unit rather than each independently rounding to a different one. **Confirmed by direct
inspection, not assumed**: `ProblemSpec`/`ParametricProblemSpec` raw field storage is already
genuine SI (Pa/m/kg*m^-3) - the doc's Phase 47 "canonical internal representation independent of
display units" requirement was already structurally satisfied; this pass only added the display/
input conversion layer, never touching TOML spec parsing or the training-time normalization
scheme (`u_ref`/`ref_energy`/`ref_stress2`, all still computed from raw SI as before).

`app-egui`: `StressSolverTool.unit_system` (persisted via a new flat `PersistedState.unit_system`
field, mirroring `IndexLocation`'s same-crate-enum-persisted-natively pattern, not the heavier
`SearchFieldsSnap` bag pattern - this is a single global toggle). A segmented USCS/SI control at
the top of every step. Every hardcoded `"{:.3e} Pa"`/`"{} m"` literal across the spec summaries,
Solution Summary, Model Contract, parametric envelope/comparison cards, and the heatmap colorbar
now goes through `units::format_value` - the colorbar sweep closed a real, separate gap (it was
previously completely bare regardless of which field/dimension was selected). The parametric
inference sliders were made genuinely unit-aware (not just relabeled): dragging now happens in
the picked display unit, converting back to canonical SI on change, never mutating the
underlying value except through an explicit user edit.

### Stage B - Diverging colormap for the difference field

New `fn diverging(t: f32) -> Color32` (blue->white->red, zero-centered at `t=0.5`) beside the
existing single-hue `viridis`, plus a `Colormap` enum threaded through `field_to_pixels`/
`field_heatmap`. `training_vs_new_case_card`'s New-minus-Original difference field now renders
through `Diverging` with a symmetric (`-max(|min|,|max|)..+max(|min|,|max|)`) range - the
existing P2/P98 asymmetric clip would put zero at an arbitrary off-center position, defeating a
diverging colormap's entire point. Every raw-field call site keeps `Viridis` unchanged.

### Stage C - Force equilibrium (`enhancement.md` Phase 9)

New `pinn_core::messages::ReactionForce { net_fx, net_fy, reference_force, equilibrium_error }` -
lives in `pinn-core` for the same reason `HoleBoundaryPoint`/`VisFields` do (the solver computes
it, `pinn-core` owns the shape crossing the `TrainingUpdate` boundary). **A real, non-obvious
finding shaped the design**: the far-field traction target this problem applies is, by
construction, self-canceling around the whole closed rectangle (`px` pulls the right edge one way
and the left edge the other) - so the target/applied resultant is analytically always zero, and
there is nothing meaningful to compare a prediction against there. The actual meaningful check is
whether the network's own PREDICTED traction integral is *also* close to zero; any nonzero net
predicted force is a real, physically-meaningful inconsistency. `user_problem::
probe_reaction_force` (arc-length-weighted `traction * ds * thickness` integral over the real
outer-boundary point set, `ds` derived from `UserSamplingStrategy::sample_boundary`'s own known
even-spacing formula - no changes to that shared, tested function) and `parametric_problem::
reaction_force_stats` (generic-over-backend analogue, shared between the periodic training probe
and the on-demand inference query, same dedup precedent as `bc_residual_stats`). Wired into
`TrainingUpdate`/`ParametricTrainingUpdate` (vis-cadence only, `None` between updates - a real
absence, not a `0.0` sentinel) and `ParametricInferenceResult` (always computed there). Kirsch/
pin-lug deliberately left `None` - same documented-gap treatment as BC residual's own deferral.

### Stage D - Real energy balance (`enhancement.md` Phase 10)

New `pinn_core::messages::EnergyBalance { internal_energy, external_work, energy_balance_error }`,
explicitly documented as DISTINCT from `energy_loss` (confirmed by direct inspection: that field
is literally the raw, un-integrated, SAW-BRDR-weighted optimizer loss term - not a physical
energy in joules, exactly the conflation the doc's own Phase 10 warns against). `user_problem::
probe_energy_balance`/`parametric_problem::energy_balance_stats`: internal energy is a genuine
Monte-Carlo domain integral (`mean(strain energy density) * area * thickness`, area computed as
`4*half_w*half_h` minus hole areas); external work is `sum(t . u * ds * thickness)` over the same
weighted boundary point set Stage C built, halved for the same quasi-static-linear-loading `1/2`
factor `energy::dem_energy_per_point`'s own formula already carries. The existing "Energy
(final)" Solution Summary row was relabeled "Optimization loss (energy term)" so the two
quantities are never visually conflated. Same vis-cadence/`None`-between-updates/Kirsch-and-pin-
lug-deferred treatment as Stage C.

### Stage E - Distance-to-training-distribution (`enhancement.md` Phase 21, parametric only)

New `pinn_core::param_distance` module (pure logic, no `burn`/tensor dependency, fits alongside
`sampling.rs`'s existing plain-math role): `nearest_neighbor_distance`/`median_nn_spacing` over
normalized `[-1,1]^3` `(e_n,nu_n,p_n)` triples. `run_training_parametric` now keeps a bounded
(`VecDeque`, cap 500, FIFO-evicted) reservoir of every triple actually drawn during training -
memory stays flat for a long run while still reflecting "recent" coverage. On `ParametricInfer`,
computes the query's nearest-neighbor distance against the reservoir plus the reservoir's OWN
median pairwise spacing as a self-baseline (ratio-based, the same "compare against this run's own
observed baseline, not an arbitrary absolute constant" convention the PDE/BC residual ratios
already use). Folded into `classify_infer_result`'s existing `Verdict.reasons` as one additional
line/YELLOW-tier bump when coverage is sparse (>3x typical spacing) - extends the existing tri-
state logic rather than adding a fourth axis.

### Stage G - In-app spec editing (user's explicit follow-up ask)

**Confirmed by audit**: zero editable numeric widgets existed anywhere for geometry/material/load
- every spec value was read-only text, requiring a TOML edit + reload to change anything. Fixed
by replacing the read-only summary blocks (Load Spec step, both `Plate` and `Parametric`
variants) with unit-aware `DragValue` widgets bound directly to the loaded spec's fields (width/
height edited as the full dimension the user sees, halved on write to the underlying `half_w`/
`half_h`; a `unit_drag` closure mirrors Stage A's slider technique - pick one unit from the
current value's magnitude, convert on display, convert back on change). Editable only while
`Status::Idle` (training/a served checkpoint has already captured whatever spec it started with -
`run_training_*`/checkpoint-load read the spec once at thread-spawn time, so edits mid-run would
silently do nothing; disabling them is honest, not cosmetic). No changes needed to `start_training`
itself - only to what the UI now hands it.

### Stage H - Model checkpoint save/load (user's explicit follow-up ask)

**No model persistence existed anywhere in this codebase before this pass** (confirmed
independently by both audits, and already on record in this file's own Phase 17/18 section) -
genuinely new infrastructure, not a gap-closing tweak.

- New `pinn_solver::checkpoint` module: `save_checkpoint`/`load_checkpoint` using burn's own
  `NamedMpkGzFileRecorder<HalfPrecisionSettings>` (gzip'd, half-precision binary - an existing,
  idiomatic burn facility that directly minimizes file size per this pass's own "keep generated
  file size to a minimum" instruction, not a hand-rolled format). Only the model's weights are
  saved (no optimizer momentum state - inference reuse, not resume-training, a stated scope
  choice). A `.meta.json` sidecar (`CheckpointMeta`/`CheckpointSpec::{Plate,Parametric}`) carries
  the exact spec trained, steps completed, final loss, and a save timestamp - real, run-derived
  Model Contract persistence (this file's own Phase 20/26/39 section), not hand-authored.
- New `ControlMsg::SaveCheckpoint { path, saved_at_unix }` / `TrainingMsg::CheckpointSaved(Result
  <String,String>)`. `saved_at_unix` is stamped by the UI thread (natural clock access for a real
  user-triggered action), not read inside the solver thread - keeps every solver-side probe in
  this codebase a pure function of its arguments, per this project's own established convention.
- **`run_training_user_problem` (the plain Plate path) previously exited right after `Done`** -
  unlike the parametric path, already redesigned in an earlier pass to stay alive for instant
  inference. Extended it with the identical stay-alive-and-serve-`SaveCheckpoint` shape (the same
  pattern applied a second time, not a new architecture) so a save can be requested at any later
  moment, not only in the instant training finishes.
- **Loading skips training entirely**: a new `pinn_solver::runner::serve_loaded_plate_checkpoint`
  (evaluates the loaded model once via the exact same standalone probes the vis-cadence block
  already calls - `evaluate_user_vis_grid`/`probe_hole_boundary_profile`/`probe_boundary_
  residuals`/`probe_reaction_force`/`probe_energy_balance`, all pure functions of `(model, spec,
  device)` - then sends one `Update`+`Done` pair so the existing Results cards populate exactly
  as a real run would) and `pinn_solver::parametric_problem::serve_loaded_checkpoint` (sends
  `ParametricReady` immediately, no training). **Refactored `run_training_parametric`'s own tail
  loop into a shared `serve_parametric_inference` function** used by both the post-training path
  and the loaded-checkpoint path - avoids duplicating the `ParametricInfer`/`SaveCheckpoint`
  handler logic across two entry points.
- UI: "Load Trained Model..." next to the spec-path field (Load Spec step, `rfd::FileDialog::
  pick_file`), "Save Trained Model..." next to Start/Stop (Results-reachable, enabled at
  `Status::Done`, `rfd::FileDialog::save_file`), inline success/failure status for both.
- **Two real bugs found and fixed via the round-trip test itself, not assumed correct**: (1) the
  "compute the exact written path to report back" logic used a malformed `with_extension` call
  that never matched burn's own `set_extension`-based convention - fixed by replicating burn's
  own `record/file.rs` macro exactly (`weights_path.set_extension("mpk.gz")`) instead of guessing.
  (2) a supplementary architecture-check test asserted the wrong tensor dimension convention (`5
  rows` instead of `5 columns` for a single input point) - a test-authoring mistake, not a
  production bug, caught the same way. (3) **A real test-only hang, same class already on record
  in this file for the parametric path**: the first version of `serve_loaded_plate_checkpoint`'s
  own test used `run_and_drain` (which `join()`s assuming the function returns after `Done`) -
  but this function deliberately stays alive after `Done`, so the test deadlocked waiting for a
  `join()` that would never return. Fixed by switching to the same manual-thread-plus-explicit-
  `Stop` pattern the parametric tests already established, not by touching the production code.

### Stage I - Live network-evolution visualization (user's explicit follow-up ask)

**Hard constraint honored by construction**: nothing here touches the per-step hot loop
(`step_physics`/`step_physics_multi`/`step_parametric`) - it reads only `model_val`, the CPU-
mirrored snapshot ALREADY pulled at the existing vis-cadence throttle for vis-grid evaluation, so
it adds zero forward passes and zero per-step training-loop cost. New `ElasticityNet::
layer_weight_stats` (per-`layers` entry `(mean |weight|, max |weight|)`, a pure read of already-
computed parameter tensors - no forward pass, no gradient computation) and `network::
network_snapshot` (wraps that plus the existing `awake_mask` into `pinn_core::messages::
NetworkSnapshot`). Wired into `TrainingUpdate` (all three paths - Kirsch, `UserDefinedProblem`,
and the loaded-checkpoint server - since this is generic and cheap regardless of problem type,
unlike the problem-specific BC-residual/reaction-force/energy-balance probes) and
`ParametricTrainingUpdate`, same vis-cadence/`None`-between-updates convention as every other
`Option` field. **Confirmed by inspection**: `NetworkSpec` (both `ProblemSpec` and
`ParametricProblemSpec`) has no `use_piratenet` field, so `awake_mask` is always empty for every
problem type this toolbox actually trains - the type stays correct if that ever changes, but the
"awake/dormant" half of the UI panel will show nothing until it does, which is accurate, not a
bug. UI: a compact per-layer strip (colored by mean |weight|, normalized against this snapshot's
own min/max - a per-network-state relative view, since weight magnitude has no natural fixed
scale) rendered beside (not below) the Stress Field heatmap, per the user's own explicit
placement ask - the heatmap's column width is now explicitly capped so the two panels coexist
rather than the heatmap claiming the full row.

### Stage F - Exportable analysis report (`enhancement.md` Phase 40)

**No export/report code existed anywhere in `app-egui` for the Stress Solver** (confirmed by
audit; unrelated to `search-core`'s own HTML report for the Search tool). New "Export Analysis
Report..." button (Results step) assembles a single JSON document from data ALREADY flowing
through `self.*` fields - problem definition, training/final-loss summary, engineering results
(re-derived from `self.vis` via the same aggregate computation `solution_summary_card` already
does - no new solver-side computation), physics validation (PDE/BC residual, reaction force,
energy balance), AMR summary, hole analyses, and the active parametric query + verdict if one was
run - via `serde_json::to_string_pretty`, written through `rfd::FileDialog::save_file`.
Deliberately excludes raw `VisFields` 2D arrays (engineering scalars/summaries only) to keep the
file small, consistent with this pass's own size-minimization instruction.

### Verification for this pass

`cargo test -p pinn-core`: 99/99 (up from 86 - +7 `units` tests, +6 `param_distance` tests).
`cargo test -p pinn-solver` (fresh, default `wgpu` backend, the full ~16-minute suite, not a
filtered subset): **257/257 passed, 0 failed, 8 ignored** (up from 243 - +14: 3 `probe_reaction_
force` + 2 `probe_energy_balance` + 2 `checkpoint` + 2 `serve_loaded_checkpoint` (parametric) + 2
`serve_loaded_plate_checkpoint` (runner) + 3 `network` Stage-I tests - the arithmetic matches
exactly, independently confirming this was a genuine complete run and not the stale-binary
pitfall documented in this file's own prior section). `cargo build -p pinn-gui` and `cargo test -p
pinn-gui --no-run` both clean (one new no-op `TrainingMsg::CheckpointSaved(_)` match arm needed,
same treatment as the existing `BeamUpdate`/`ParametricUpdate` no-ops there). `cargo build -p
app-egui` clean, zero warnings, at every stage boundary (nine separate clean builds across this
pass, not just a final one). Real `app-egui` launches (`cargo build -p app-egui && ./target/
debug/app-egui &`) after Stages G, H, I, and F each stayed alive 6-8s with an empty log - no
panic - with that stage's new UI code path active.

**A real environment lesson from this pass, worth recording**: mid-session, `.shared-cargo-target`
(24GB) plus `powershell_tool/target` (13GB) exhausted the machine's disk entirely, to the point
where even a shell command's own tiny output-capture file could not be written - a hard stop
requiring the user to free space before any further verification could run. `cargo clean` on a
project's own (non-shared-named) `target/` directory is always safe or regenerable; a directory
explicitly named "shared" should not be cleaned without checking first, since other concurrent
sessions may depend on its cache. If disk usage becomes a recurring issue on this machine, an
occasional `cargo clean` on `powershell_tool/target` between long sessions is a reasonable,
low-risk mitigation.

## Follow-up: real network-diagram, not a bar chart, plus reverting the heatmap-size regression

Stage I's first cut (bar-chart panel beside the heatmap) drew two real user complaints: the
heatmap became too small to read (the side-by-side split shrank it to make room), and the panel
itself wasn't actually a network diagram - just colored bars labeled `L0`/`L1`/`L2`. Fixed as one
pass, approved via an artifact mockup before any code changed (per the user's own explicit ask).

- **Heatmap**: the horizontal split introduced in Stage I is fully reverted. `field_heatmap` is
  back to a full-width card, exactly as it was before that pass.
- **Network diagram moved to its own full-width card**, placed after Adaptive Refinement - a real
  layer-by-layer node/edge layout needs horizontal room neither widget had while sharing a row.
- **`NetworkSnapshot` now carries the actual weight matrices**, not just per-layer mean/max: new
  `ElasticityNet::all_weight_matrices()` returns `self.layers` THEN `self.out` (deliberately
  INCLUDING the output projection, unlike `layer_weight_stats`/`awake_mask`, which exclude it to
  mirror `awake_mask`'s own scope - a wiring diagram needs the complete input-to-output path to
  mean anything, a genuinely different requirement, not an oversight in the other two). Payload
  stays small (a 64x64 hidden layer is 16 KB; input/output layers are far smaller) and is still
  read only at the existing vis cadence from `model_val` - zero added cost to the per-step hot
  loop, same as every other Stage I/C/D probe.
- **`app-egui`'s `draw_network_diagram`** renders real neurons (circles) and real connections
  (lines), line width/opacity encoding `|weight|` normalized per-layer-transition (each matrix's
  own min/max among the displayed edges - different layers can sit at very different weight
  scales, so one global scale would wash out real variation). Input layer gets real labels when
  the dimension matches this toolbox's own known conventions (`x,y,z` for 3, `+ eₙ,νₙ,pₙ` for 6);
  output layer likewise (`u,v,σxx,σyy,σxy` for the mDEM 5-column case). Wide hidden layers
  (hidden_dim up to 64+) are deterministically stride-sampled down to 10 displayed neurons -
  drawing all of a 64-wide layer's edges would be an unreadable hairball - and the card states
  "N of M" so the reduction is honest, not hidden. Sampling indices are proven safe by
  construction (a new test, `all_weight_matrices_includes_the_output_layer_with_correct_shapes`,
  confirms consecutive matrices chain correctly - `matrices[i].ncols() == matrices[i+1].nrows()`
  - which is what guarantees the UI's row/column index reuse across a matrix boundary can never
  go out of bounds).

**Verification**: `cargo test -p pinn-solver --features ndarray-backend network::` 24/24 (up from
21 - 3 new tests: shape-chaining, cross-check against `layer_weight_stats`, and `NetworkSnapshot`
parity with `all_weight_matrices` directly). `runner::`/`parametric_problem::` full re-runs both
still green (the `NetworkSnapshot` field addition doesn't change how any existing test constructs
one - always via `network_snapshot()`, never a literal). `cargo build -p app-egui` clean, zero
warnings. Real launch stayed alive with an empty log; visual confirmation of the on-screen layout
hit the same pre-existing `screencapture`/window-focus limitation already on record in this file
(process alive and frontmost per the menu bar, but the window itself doesn't appear in the
capture) - correctness here rests on the shape-chaining test plus code review, not a screenshot.

## Follow-up pass: smart adaptive architecture (grow/shrink depth and width during training)

User recalled network resizing as "always a feature" - investigation found `ElasticityNet::
grow_width` (Net2WiderNet-style, function-preserving width growth) was real but wired into
exactly one path (`headless.rs`'s Kirsch-only `run_headless_inner`), gated by a fixed-step
trigger, defaulting off, never reachable from either GUI training loop. No shrink primitive
(width or depth) existed anywhere. This pass built real elasticity - both depth and width,
growing AND shrinking, driven by training-progress signals, reachable from `run_training_
user_problem` and `run_training_parametric` (Kirsch/pin-lug explicitly out of scope, same
precedent as every other BC-residual/reaction-force/energy-balance deferral in this file).

### Design

- **`ElasticityNet` new primitives** (`pinn-solver/src/network.rs`): `append_dormant_layer`
  (appends a `hidden_dim -> hidden_dim` layer + a gate starting at exactly `0.0` - zero-effect
  at insertion, proven by the same `alpha=0` residual-identity technique `shallow_launch_
  matches_single_hidden_layer` already established) and `remove_layer` (removes an already-
  dormant block, proven zero-effect in reverse). Both REQUIRE `gates.len() == layers.len() - 1`
  (fully gated, i.e. `use_piratenet=true`) - the precondition that keeps `forward_masked`'s
  positional `gates[i-1]` lookup aligned with `layers[i]`.
- **`prune_width`** (width shrink) went through a real bug during this pass: the first version
  pruned a single layer's boundary (mirroring how `grow_width` looks superficially), but
  `forward_masked`'s gated residual sum (`h = h + tanh(layers[i](h)) * alpha`) requires EVERY
  gated layer's output width to match `h`'s width - shrinking one layer in isolation desyncs
  that shared width and panics on the next forward pass. Fixed to be a GLOBAL, uniform-hidden_
  dim operation mirroring `grow_width`'s own per-layer axis treatment in reverse (`layers[0]`:
  output columns only; `layers[1..]`: both input rows and output columns; `out`: input rows
  only) - caught by writing `prune_width_on_a_gated_network_does_not_break_forward_masked`
  (a real forward pass on a gated, non-zero-gate network after pruning) BEFORE wiring it into
  training, not after. Lesson: a shrink primitive's tests must exercise the SAME structural
  invariant its sibling growth primitive already depends on, not just its own shape/index
  bookkeeping in isolation.
- **`ElasticityNet::per_neuron_magnitudes`**: one `Vec<f32>` per prunable layer (mean |weight|
  per output column), the per-neuron-granularity input `ArchitectureController::plan_prune`
  needs to rank pruning candidates - `NetworkSnapshot::layer_mean_abs_weight` is coarser
  (one scalar per whole layer), not enough for this.
- **`ArchitectureController`** (new module, `pinn-solver/src/architecture_controller.rs`) -
  pure decision logic, zero burn/tensor dependency (fast to unit test: 10 tests, 0.00s). Reuses
  `controllers::ConvergenceTracker::for_metric` unchanged (already generic over any scalar
  metric). Policy (v1, explicitly heuristic): residual plateau -> grow width, then depth once
  width is capped; any gate dormant for `dormancy_threshold` consecutive observations -> shrink
  depth (always allowed, never watched - provably zero-effect); periodically, if the residual is
  comfortably below its own historical PEAK (not running minimum - an earlier version of this
  check used the running min, which is trivially satisfied by ordinary monotonic improvement and
  would have made pruning fire constantly) -> prune the globally lowest-average-magnitude hidden
  units. Every speculative action (grow/prune, not shrink-depth) is watched for
  `post_action_watch_window` observations; if the residual didn't improve enough, `RevertLast
  Change` restores a kept `(model, hidden_dim, n_hidden)` snapshot.
- **Optimizer migration** (`pinn-solver/src/optim/mod.rs`): `WeightOptim::migrate_for_shrink`
  mirrors the pre-existing `migrate_for_growth` (SoapMuon-only; AdamW-only mode always rebuilds
  fresh - the same "cold-start on shrink" treatment growth already gives every bias `Param`).
  Depth shrink needs NO migration at all (`remove_layer` deletes the whole `ParamId`; the
  orphaned optimizer record is simply never looked up again). `optim::apply_arch_action` is the
  one shared function both training loops call to actually apply an `ArchAction` to a live
  model + optimizer - keeps their adaptive behavior from silently drifting apart.
- **Config surface**: `NetworkSpec.adaptive: bool` / `max_hidden_dim: Option<usize>` /
  `max_n_hidden: Option<usize>`, all `#[serde(default)]` (every existing TOML spec keeps
  parsing unchanged). `adaptive` internally forces `use_piratenet=true` when the model is built
  - there is no separate user-facing "use_piratenet" toggle; PirateNet gating is purely the
  internal mechanism that makes safe depth growth/shrink possible.
- **Checkpoint interaction** (Stage H): `CheckpointMeta` now records the LIVE `current_hidden_
  dim`/`current_n_hidden` reached during training, not the original spec's static values - a
  checkpoint saved after adaptation would otherwise reload with the wrong shape. `checkpoint::
  load_checkpoint` also now reads `adaptive` from the saved spec and forces the same `use_
  piratenet` on the rebuilt model, or `load_file` would fail to match the saved record's
  `gates` structure.
- **`app-egui`**: loss-chart `VLine` markers for architecture events (violet, distinct from
  AMR's amber), a status line on the Network Architecture card showing the latest event's
  description, and an "Adaptive architecture" checkbox + `max_hidden_dim`/`max_n_hidden` drag
  fields in Stage G's Idle-only spec editor (shared by the Plate and Parametric branches via
  one `adaptive_architecture_controls` helper, since both use the same `NetworkSpec`).

### The vis-cadence residual signal

The design's original intent was to feed the controller the interior PDE residual RMS. In the
actual code that value (`probe_interior_energy_residuals`) only runs on the AMR sweep cadence,
not the vis cadence `network_snapshot`/`awake_mask` are available on - reusing it would have
meant a new physics probe. Fed `bc_residual_rms` instead (the plate path's boundary-condition
residual, already computed at vis cadence) - `run_training_parametric` already computes its own
`bc_residual_rms` every step regardless. Documented as a deliberate substitution, not an
oversight - caught by reading the actual code instead of trusting the plan's own prose.

### Verification

`cargo test -p pinn-solver --features ndarray-backend` full workspace run: 286 passed, 0 failed,
10 ignored (pre-existing perf/slow-run tests, unrelated to this pass). New tests: 9 in
`network::` (append/remove/prune, including the gated-forward-pass regression test above), 2 in
`network::` for `per_neuron_magnitudes`, 5 in `optim::soap_muon::` for shrink migration, 10 in
`architecture_controller::`. Two new `#[ignore]`d end-to-end integration tests (`runner::tests::
run_training_user_problem_adaptive_wiring_does_not_crash_and_events_are_consistent`,
`parametric_problem::tests::run_training_parametric_adaptive_wiring_...`) - both run for real in
release mode, 450 steps each, `adaptive: true`: no panic, no NaN, zero architecture events fired
either run (an honest result, not a failure - `ConvergenceTracker::check_plateau` needs 400
steps' worth of real, non-fully-seeded readings before it can even evaluate a plateau; these
tests deliberately don't hard-require an event, only that whichever fire are internally
consistent). `cargo build -p app-egui` clean, zero warnings. Real launch stayed alive with an
empty log (same screencapture/window-focus limitation on record elsewhere in this file - no
visual screenshot, correctness rests on the test suite plus code review).

By construction this pass never touches `step_physics`/`step_physics_multi`/`step_parametric`
themselves - every architecture mutation happens BETWEEN steps, at the existing vis-cadence
checkpoint, so every byte-identical-trajectory regression test (`step_physics_trait_driven_
matches_independently_reimplemented_old_formula` and siblings) stayed green with zero changes.

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
