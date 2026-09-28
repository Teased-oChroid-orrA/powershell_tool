# src/ (TextInFilesSearch — C#/WinUI reference app)

## Purpose
Owns: the original C#/WinUI 3 desktop app (`TextInFilesSearch/`) and its
service layer (`TextInFilesSearch.Core/`) — the middle stage of a
PowerShell → C#/WinUI → Rust/Dioxus migration. Kept **only** as a
byte-for-byte-tested behavioral reference to diff the active Rust port
against if a discrepancy is ever suspected.

Does not own: any current functionality. The active stack is
`native-search/` + `search-core/` + `app/` (Rust). This app has no runtime
or build dependency *from* that stack, and nothing in Rust ever calls out
to it — see Contracts. `powershell/` at the repo root is a second,
smaller reference tier (the original PowerShell tool) with the same
diff-only treatment; it has no node of its own.

Confirmed via `git log --oneline -- src/`: 14 commits total, all dated
2026-08-23/24, entirely initial-import + bug-fix + the one-off
`native_search` P/Invoke vertical slice (issue #2). Zero commits since.
`search-core/` (31 commits) and `app/` (47 commits) have continued
developing well past that point — `src/` is frozen.

## Code Map

| Looking for... | Go to |
|---|---|
| WinUI shell / window / XAML | `TextInFilesSearch/Views/MainWindow.xaml(.cs)`, `TextInFilesSearch/App.xaml(.cs)` |
| The actual ported business logic | `TextInFilesSearch.Core/Services/*.cs` (`SearchOrchestrator`, `MatchingEngine`, `TextExtractionService`, `FileReaderService`, `CacheService`, `ReportExportService`) — each is the C# source `search-core/src/*.rs` was ported from (see header comments in those Rust files) |
| Models | `TextInFilesSearch.Core/Models/*.cs` (`SearchModels`, `ExtensionCatalog`, `NativeSearchModels`, `InFlightFileStatus`) |
| P/Invoke into the Rust `native_search.dll` cdylib | `TextInFilesSearch.Core/Native/*.cs` + `Services/NativeSearchService.cs` — the one place this app depends on Rust, not the reverse (see Contracts) |
| Build config | `TextInFilesSearch/TextInFilesSearch.csproj`, `TextInFilesSearch.Core/TextInFilesSearch.Core.csproj`, root `TextInFilesSearch.sln` |

## Entry Points

| Task | Start Here |
|---|---|
| Verify this app still behaves as documented / diff a suspected Rust regression | `dotnet run --project tests/TextInFilesSearch.Tests` (dependency-free harness, `Program.cs`) |
| CI verification gate for this app | `.github/workflows/build.yml` (`workflow_dispatch`) — builds `native-search` + `TextInFilesSearch.sln`, runs the same test harness, publishes self-contained win-x64, then asserts the publish output contains `hostfxr.dll`, `coreclr.dll`, `Microsoft.WindowsAppRuntime.dll`, `resources.pri`, `native_search.dll` and contains **no** `*.msix*` output. Confirmed present and matching this description as of this writing. |

## Contracts

- **No new features here.** If something is missing from the Rust port,
  port it into `search-core/`/`app/`, not into this C# app. This directory
  exists to be diffed against, not extended.
- **Nothing in Rust ever calls into `src/` or `powershell/`.** No
  PowerShell invocation, no .NET reference, no shell-out from
  `native-search/`, `search-core/`, `app/`, or `cli/` into either
  reference tier. The one cross-language dependency runs the other way:
  this C# app P/Invokes the built `native_search.dll` (Rust cdylib) via
  `NativeSearchService`/`NativeSearchInterop.cs` — a one-off vertical
  slice from issue #2 (commit `af7651b`), not a pattern to invert.
- **Test fixtures are shared byte-identical with the Rust port.**
  `tests/TextInFilesSearch.Tests/Fixtures/*.{docx,pptx,pdf,xlsx,zip}` are
  `include_bytes!`'d directly by `search-core/tests/fixtures.rs` (see its
  header comment referencing "Program.cs Tests 14/27/28/29") and by
  `search-core/src/extraction.rs`. Do not regenerate, replace, or modify
  a fixture without checking both consumers still pass.

## Pitfalls

- `EnableMsixTooling=false` in `TextInFilesSearch.csproj` silently disabled
  the entire MSBuild PRI resource-generation target chain — the publish
  output shipped with zero `resources.pri`, and every `ms-appx:///...`
  XAML URI (including `MainWindow.xaml`'s own first line) failed at
  runtime with a `XamlParseException` on a clean machine, with no build
  error. `WindowsPackageType=None` (a separate setting) is what actually
  controls "no MSIX output" and was unaffected. Fixed by setting
  `EnableMsixTooling=true`; CI now asserts `resources.pri` is present in
  the publish output. (commit `a5b40c1`)
- A model type bound via `x:Bind` in XAML must be a mutable class, not a
  `record` — `NativeSearchHit` as a `record` compiled fine but broke
  `x:Bind` at runtime with no compile-time warning. (commit `34ac14a`)
- **WinUI 3 cannot build, run, or debug on a non-Windows machine at all**
  — this is part of why the Rust/Dioxus migration happened. Any change
  here can only be verified via `.github/workflows/build.yml` on a
  Windows runner, never locally on a Mac/Linux dev machine.
- (Historical) An XML comment containing `--` once broke a `.csproj` file
  outright. `.csproj` files here have long hand-written prose comments
  (see `TextInFilesSearch.csproj`) — avoid `--` inside any XML comment
  added to `.csproj`/`.xaml` files.

## Boundaries

### Never
- Add features, endpoints, or new behavior to `TextInFilesSearch`/`TextInFilesSearch.Core` — port to `search-core`/`app` instead.
- Add a PowerShell invocation, .NET/C# reference, or shell-out to `src/` or `powershell/` from any Rust crate.

### Verify First
- Any change to `tests/TextInFilesSearch.Tests/Fixtures/*` → confirm `search-core/tests/fixtures.rs` still builds/passes (`cargo test -p search-core`) before committing.
