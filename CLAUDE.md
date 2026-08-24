# CLAUDE.md

Context for Claude Code (or any fresh agent) picking up this repository with
no memory of how it got here. Read this before making changes.

## What this project is

A native Windows desktop app (WinUI 3 / C# / .NET 8) that recursively
searches a folder for keyword filters across `.txt`, `.log`, `.docx`,
`.pptx` (slides, speaker notes, and SmartArt diagram text), `.xlsx`, `.zip`
(recursing into entries, including nested zips), `.rtf`, `.pdf`, and dozens
of other code/config/data extensions (see `Models/ExtensionCatalog.cs` - the
single source of truth for both the engine's default extension list and the
UI's type-to-filter/tick-list extension picker), producing an HTML report
plus optional CSV/JSON export. It is a from-scratch C# migration of an
earlier PowerShell tool (kept in `powershell/` for reference only - see
below).

## Architecture: Core/head split - do not merge these back together

```
src/TextInFilesSearch.Core/   Plain net8.0 class library. Zero WinUI reference.
    Models/                    SearchSettings, FileSearchResult, LineHit, progress-report types
    Services/                  TextExtractionService, FileReaderService, MatchingEngine,
                               SearchOrchestrator, CacheService, ReportExportService
    ViewModels/MainViewModel.cs  All UI state + commands. Folder-picking and
                               report-opening are injected as delegates
                               (Func<Task<string?>> / Action<string>) specifically
                               so this file has NO WinUI dependency and can be
                               unit tested with fakes.
    Helpers/                   Dependency-free ObservableObject + RelayCommand
                               (see "No MVVM Toolkit" below).

src/TextInFilesSearch/        The WinUI 3 head. Keep this THIN.
    App.xaml(.cs)               Entry point; registers CodePagesEncodingProvider
                               once at startup (required for the Windows-1252
                               fallback in TextExtractionService to work at all).
    Views/MainWindow.xaml(.cs)   The ONLY two files in the whole solution that
                               should touch a WinUI/Win32 API directly
                               (FolderPicker + window-handle interop,
                               AppWindow.SetIcon for the window/taskbar icon,
                               Process.Start to open the report).
    Assets/AppIcon.ico, Assets/Banner.png   Production-sized brand assets for
                               the head project - exe icon (baked in via
                               ApplicationIcon + loaded at runtime via
                               AppWindow.SetIcon) and the in-app title-bar
                               banner. Derived from GS_Engineering_Brand_Assets/
                               (see below), not hand-edited directly.

src/TextInFilesSearch.Core/Assets/Banner.jpg   The same banner, separately
                               sized/compressed for embedding as a base64
                               data URI in the HTML report (ReportExportService)
                               - kept here rather than referencing the head
                               project's copy so Core stays zero-WinUI-dependency
                               and the report stays a single portable file.

tests/TextInFilesSearch.Tests/Program.cs   Dependency-free integration harness.
docs/architecture.md, docs/deployment.md   Longer-form design rationale.
powershell/                    Original scripts. Reference only - see below.
GS_Engineering_Brand_Assets/    Source brand assets (master-resolution icons,
                               banners, README) the Assets/ folders above were
                               derived from. Reference only, like powershell/ -
                               don't point csproj/XAML/report code at this
                               folder directly; regenerate the sized/compressed
                               Assets/ copies from it instead if the brand
                               assets ever change.
```

**Why the split matters:** everything in `Core` builds and runs on any
platform's .NET 8 SDK with zero NuGet restore needed for the class library
itself. That's not incidental - it's how this codebase was actually developed
and verified (no Windows machine was available during initial development).
Any new business logic goes in `Core`. If you find yourself adding a
`using Microsoft.UI...` anywhere outside `src/TextInFilesSearch/Views` or
`App.xaml.cs`, stop - that logic belongs in `Core` instead, with the WinUI
dependency injected in from the head project.

## `powershell/` is reference-only - never wire it up

The app has zero runtime or build dependency on the `.ps1` files. They exist
so the original implementation can be diffed against if a behavior
discrepancy is ever suspected. Do not add a PowerShell invocation, a
`System.Management.Automation` reference, or any shell-out to `powershell.exe`
anywhere in `src/`. If a feature seems missing, port it into `Core` as C# -
don't fall back to calling out to the old script.

## Design decisions worth knowing before you change them

- **No CommunityToolkit.Mvvm.** `Helpers/ObservableObject.cs` and
  `Helpers/RelayCommand.cs` are small hand-rolled `INotifyPropertyChanged`/
  `ICommand` implementations. This was originally a workaround for no NuGet
  access in the dev sandbox, not a hard architectural requirement - swapping
  to the MVVM Toolkit later is a fine, low-risk change if you want the
  source-generator ergonomics (`[ObservableProperty]` etc.). Just don't do it
  silently; it changes the dependency footprint of a self-contained build.
- **Folder pickers and "open report" are injected, not called directly** from
  `MainViewModel`. Keep it that way - it's what lets the ViewModel be unit
  tested without a Windows App SDK reference. If you add a new
  Windows-API-dependent action, inject it the same way rather than reaching
  into WinUI types from the ViewModel.
- **`PublishSingleFile` is deliberately `false`.** WinUI 3's native resources
  and the Windows App SDK's bundled DLLs have a documented history of
  single-file-publish edge cases. Don't flip this on without actually
  verifying the result runs on a clean machine (see `docs/deployment.md`) -
  a green build is not the same as a working publish here.
- **`WindowsPackageType=None` (unpackaged, no MSIX)** and
  **`SelfContained=true` + `WindowsAppSDKSelfContained=true`** are both load-
  bearing for the "runs on a machine with no internet, no admin rights, no
  pre-installed runtime" requirement. Don't remove these to "simplify" the
  csproj without re-reading `docs/deployment.md` first.

## Testing requirements - do not skip these

- **Before considering any `Core` or `ViewModels` change done**, run:
  ```
  dotnet run --project tests/TextInFilesSearch.Tests
  ```
  This is a plain console harness (not xUnit/MSTest, deliberately - zero
  package restore needed), currently 60 checks covering all three match
  modes, exclude scopes (including that ExcludeFolders matches whole path
  segments, not a raw substring - excluding "bin" must not exclude "robin"),
  whole-word/regex matching (including the lookaround-based whole-word
  boundary that correctly handles punctuation-edged filters like "C#", and
  highlight-span correctness), invalid-regex-filter error reporting, the
  ASCII85 decoder, RTF extraction, real DOCX/PPTX (slides + speaker notes +
  SmartArt diagram)/XLSX/ZIP (including a nested DOCX entry)/PDF files (the
  PDF case specifically exercises an ASCII85Decode+FlateDecode filter chain
  that silently failed before that bug was caught), parallel-vs-sequential
  consistency, cancellable/progress-reported directory enumeration, the full
  incremental cache lifecycle, CSV formula-injection neutralization, the
  Windows-1252 encoding path, ViewModel numeric-setting clamps and output-name
  sanitization, the extension type-to-filter/tick-list picker, and the
  ViewModel run/cancel/progress/streamed-results lifecycle. Add a new check
  here for any new behavior rather than trusting a passing build.
- **The WinUI layer (`src/TextInFilesSearch/Views`, the `.csproj`'s publish
  config) cannot be verified this way** - it needs an actual Windows build.
  `.github/workflows/build.yml` is the real verification gate: it builds,
  runs the test harness above, publishes self-contained, then explicitly
  checks the published output for `hostfxr.dll`/`coreclr.dll` (proves the
  .NET runtime is bundled) and `Microsoft.WindowsAppRuntime.dll` (proves the
  Windows App SDK runtime is bundled), and fails loudly if either is
  missing rather than letting a framework-dependent build slip through. If
  you change `RuntimeIdentifier`, `SelfContained`, or
  `WindowsAppSDKSelfContained` in the `.csproj`, update the matching
  verification step in `build.yml` too - they're meant to stay in sync.
- If you have access to a real Windows machine, that's a faster feedback
  loop than round-tripping through CI for XAML/WinUI changes specifically.

## Live progress reporting is a hard requirement, not a nice-to-have

This project exists partly because of a specific, explicit complaint: PDF
processing in the original tool would go silent for many seconds with no way
to tell "still working" from "actually stuck." Any future change to
`SearchOrchestrator` or `TextExtractionService.ExtractPdfLines` MUST preserve:
- Per-file progress reporting during extraction, not just on file completion
  (`PdfProgressCallback` fires roughly every 150ms with streams-scanned +
  elapsed time).
- A background ticker during parallel runs so elapsed-time displays keep
  moving between file completions, not just when a file finishes.
- Per-file in-flight status visible in the UI (`ViewModel.InFlightFiles`),
  not just an aggregate progress bar - a user should be able to see which
  specific file is slow and what it's doing.

Don't refactor this into a simpler "start/done" event model even if it looks
cleaner - that regresses the exact problem this app was built to fix.

## Bug classes already found and fixed once - watch for recurrences

- A mode-gating bug in `MatchingEngine.ApplyLineMatching`: "no hits at all"
  and "hits existed but failed AllInFile/Proximity gating" were briefly
  conflated by inferring pass/fail from whether the hits list was empty.
  Fixed by making `passesMode` an explicit `out` parameter. If you touch this
  method, keep that distinction explicit rather than re-deriving it from list
  state.
- An XML comment containing `--` broke a `.csproj` file outright (XML
  disallows `--` inside comments). Worth remembering when writing comments in
  any `.csproj`/`.xaml`/`.yml` file in this repo.
- A `zip -x "*.git*"` packaging command once silently excluded the entire
  `.github/` folder, because the wildcard substring-matched ".git" inside
  ".github". If you ever script an archive/export of this repo, use an exact
  path exclusion (e.g. `-x "*/.git/*"`) rather than a bare substring wildcard,
  and diff the archive's file list against the source tree afterward rather
  than assuming the exclusion did only what you intended.

## Feature parity checklist (from the original PowerShell tool)

If refactoring search/matching/reporting, confirm none of these regress:
match modes (AnyLine / AllInFile / Proximity), exclude filters with Line/File
scope, exclude-folder matching by whole path segment (never raw substring -
see the bug class note above), whole-word matching (lookaround-based, not
`\b`, so punctuation-edged filters like "C#" work), regex mode (with a typed
`InvalidFilterRegexException` naming the bad filter instead of a bare crash),
GroupBy (Created/Modified/None), the extension type-to-filter/tick-list
picker (`Models/ExtensionCatalog.cs` is the single source of truth backing
both the picker and the engine's default list) plus a custom-extension add
path, parallel processing via `Parallel.ForEachAsync` with a throttle limit,
the incremental cache (fingerprinted by settings, keyed by path + size +
mtime) including that cache-reused files still stream through progress, dry
run, retry-with-backoff plus per-file timeout for locked/slow files
(including detecting a file truncated by a concurrent write mid-read),
symlink-safe and cancellable directory walking with periodic enumeration
progress, encoding detection (BOM → strict UTF-8 → Windows-1252 fallback),
CSV export's formula-injection guard, live streaming of results into the
UI as each file completes (not just after the whole run finishes), and the
HTML report's dark-mode CSS, table of contents, per-filter bar chart, PDF
low-confidence flagging, and match highlighting.

## Target environment (do not relax these without discussion)

Windows 10 1809+ / Windows 11, `win-x64`. No internet access, no admin
rights, no pre-installed .NET runtime, no pre-installed Windows App SDK
runtime required on the machine running the built app. Build-time internet
access (NuGet restore in CI) is fine and expected - it's only the *published,
running application* that must be fully self-contained and offline-capable.
