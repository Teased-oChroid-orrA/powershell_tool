# Rust/Dioxus rewrite status

Referenced from several `search-core`/`app` source doc comments. This is
the narrative/status companion to `CLAUDE.md`'s architecture section - read
that first for the "what goes where" map; this doc is "what's done, what
isn't, and why specific choices were made."

## Why this rewrite exists

WinUI 3 (the original `src/TextInFilesSearch` head) cannot run, build, or
be debugged on a non-Windows machine at all. Every UI iteration had to
round-trip through Windows CI - tens of minutes per attempt. A real bug
(`EnableMsixTooling=false` silently disabling `resources.pri` generation,
causing "app launches, no window appears, no error") took three blind CI
round-trips to diagnose, something local reproduction would have caught in
seconds. Rust + Dioxus was chosen specifically to close that loop: the
whole app - business logic and UI alike - builds, runs, and debugs locally
on any platform.

## Status: functionally complete, not yet feature-frozen

All of `TextInFilesSearch.Core`'s business logic is ported to `search-core`
and passes its own test suite (`cargo test -p search-core`), including
integration tests against the same real DOCX/PPTX/XLSX/ZIP/PDF fixtures the
C# test harness used. The `app` crate is a working Dioxus desktop UI with
full functional parity against `MainWindow.xaml`/`MainViewModel.cs` - every
setting, the extension picker, live progress, results, and the
native_search "Fast re-search" panel.

Not yet done:
- The C#/WinUI app (`src/TextInFilesSearch(.Core)/`) has not been retired.
  It stays as a working reference until the Rust app has had real-world
  runtime on Windows (this session's local verification was on macOS -
  the win-x64 CI build has been verified to compile and link correctly,
  but not yet run by a human on an actual Windows machine).
- Visual polish - **superseded, see `docs/epic-ui-performance-and-design.md`**.
  The first functional-parity CSS pass shipped real Blitz rendering bugs
  (garbled `<select>`, overlapping list rows, a horizontal-scroll bug, and
  vertical text-clipping in plain `<input>`s), all found and fixed via a
  full redesign pass documented in that epic - graphite "Instrument"
  palette (following the sibling `profile_capabilities` Dioxus app's
  design direction), real design tokens, a dark/light toggle, and a UX
  pass (empty states, recent searches, per-row actions, a stat breakdown).
  Not yet a pixel-accurate recreation of anything in particular - it's
  this app's own visual identity now, not a WinUI/Fluent recreation
  target.
- `CLAUDE.md`'s bundled-asset section (`GS_Engineering_Brand_Assets/` →
  `Assets/AppIcon.ico`/`Banner.png`) hasn't been re-pointed at an `app/`
  equivalent - the Dioxus window currently has no custom icon.

## Key decisions and why (chronological)

1. **Cargo workspace, three members.** `native-search/` (pre-existing,
   unchanged), `search-core/` (new - the ported business logic),
   `app/` (new - the Dioxus UI). Mirrors the C# Core/head split
   conceptually: `search-core` stays independently `cargo test`-able with
   zero GUI dependency, the same property that made the C# `Core` valuable.

2. **`fancy-regex`, not `regex`, for `matching.rs`.** Whole-word matching
   needs lookaround (`(?<![\p{L}\p{N}_])...(?![\p{L}\p{N}_])` - so a filter
   like "C#" matches standing alone between spaces even though `\b` alone
   wouldn't). The `regex` crate deliberately doesn't support lookaround (no
   backtracking, by design - that's also *why* it needs no ReDoS timeout,
   unlike .NET's `Regex`). Verified against the C# whole-word test cases
   (real compile-and-test, not assumed) before adopting. Used for regex-mode
   and literal filters too, not just whole-word, so there's exactly one
   regex engine's matching semantics in play, not two that could quietly
   diverge on edge cases.

3. **Extraction is hand-rolled (`zip` + `flate2` + `regex`), not a format
   library.** The C# original is itself dependency-free (`ZipArchive` +
   `Regex`, no real OOXML parser) - matching that approach exactly, rather
   than adopting `calamine`/`docx-rust`/`pdf-extract`/etc., avoids those
   libraries' different extraction algorithms silently drifting from the
   byte-for-byte-tested original. Verified against the real embedded test
   fixtures (`tests/TextInFilesSearch.Tests/Fixtures/*`, reused
   byte-identical), not synthetic inputs.

4. **`orchestrator.rs` is `tokio`-based**, with a `Semaphore` + `JoinSet`
   for throttled parallelism (not literally `Parallel.ForEachAsync`, but
   equivalent throttle-limit semantics) and an `mpsc` channel for progress
   reporting (the Rust analogue of `IProgress<T>`).

5. **`native_index.rs` calls `native-search::engine` directly, in-process**
   - no FFI. The C# app's `Native/` folder, `NativeSearchService.cs`, and
   `NativeSearchCancellationToken.cs` are pure FFI-boundary plumbing
   (SafeHandle marshaling, a `DangerousAddRef`/`DangerousGetHandle`/
   `DangerousRelease` workaround for a `LibraryImport` marshaller gap) that
   becomes entirely unnecessary once the caller is Rust too. `ffi.rs` in
   `native-search` stays, unchanged, purely to keep serving the legacy C#
   app during the transition.

6. **`dioxus-native` (Blitz/WGPU/winit), not `dioxus-desktop` (wry/
   WebView2).** Originally planned as "bundle the WebView2 Fixed Version
   Runtime app-local," the same app-local-deployment pattern already used
   for the VC++ Redistributable. That plan turned out to be unimplementable:
   `wry` hardcodes `browserExecutableFolder` to null in its
   `CreateCoreWebView2EnvironmentWithOptions` call (verified by reading
   `wry-0.53.5/src/webview2/mod.rs` in the local registry cache directly,
   not assumed from documentation) - there is no supported way to point it
   at a bundled runtime folder. Every `wry`-based build would therefore
   depend on a machine-wide WebView2 install, directly violating this
   project's standing "fully self-contained, no host-machine dependency"
   requirement. `dioxus-native` (first-party as of dioxus 0.7.10, not an
   unofficial fork) has no WebView dependency at all - Windows' bundled
   D3D12 is the only runtime graphics dependency, and it's always present.
   `.github/workflows/rust-build.yml` has a CI regression check for
   `WebView2Loader.dll` accidentally linking back in.

7. **`AppState` is a flat `Copy` struct of `Signal<T>` fields**, one per
   `MainViewModel` property, rather than a context-provided object or a
   tree of smaller state structs. `Signal<T>` is itself `Copy`, so this is
   the idiomatic single-window Dioxus pattern - passing `AppState` into a
   component or spawning it into an async task just copies a handful of
   cheap handles.

## A real bug caught and fixed during the initial port

Numeric `<input>` `oninput` handlers were originally written as
`state.field.set(s.parse().unwrap_or(some_hardcoded_default).max(min))` -
called on *every* keystroke, including while the field was transiently
empty or partially typed mid-edit. Because Dioxus re-renders a controlled
input's `value` attribute on every signal change, this caused a visible
snap-back to the hardcoded default while the user was still typing (clear
the field to retype a value, watch it immediately jump to e.g. `5` before
you can enter the new number). Fixed by only calling `.set()` on a
successful parse (`if let Ok(v) = evt.value().parse() { state.field.set(v)
}`), leaving the signal - and therefore the rendered value - unchanged on
invalid/partial input instead of forcing it back to a default. See
`CLAUDE.md`'s "Design decisions" section for the standing rule this
established.
