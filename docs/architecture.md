# Architecture

## Project structure

```
src/
  TextInFilesSearch.Core/   Plain net8.0 class library - zero WinUI dependency.
    Models/                 Data shapes: SearchSettings, FileSearchResult, LineHit, progress reports.
    Services/                TextExtractionService, FileReaderService, MatchingEngine,
                              SearchOrchestrator, CacheService, ReportExportService.
    ViewModels/               MainViewModel - all UI state and command logic, with
                              folder-picking and report-launching injected as delegates
                              so this whole layer needs no WinUI/Windows App SDK reference.
    Helpers/                  Dependency-free ObservableObject / RelayCommand (no
                              CommunityToolkit.Mvvm - see "Why no MVVM Toolkit" below).

  TextInFilesSearch/         The WinUI 3 "head" project - thin by design.
    App.xaml(.cs)             Application entry point; registers the legacy
                              code-pages provider once at startup.
    Views/MainWindow.xaml(.cs) The only two files in the whole solution that touch
                              a WinUI/Win32 API directly (FolderPicker + window
                              handle interop, and Process.Start to open the report).

tests/
  TextInFilesSearch.Tests/   Dependency-free integration test harness (36 checks) -
                              see "Verification" below.

powershell/                  The original PowerShell scripts, kept for reference and
                              historical context only. The WinUI 3 application does
                              NOT load, call, or depend on these at runtime or build
                              time in any way.

docs/                        This file, plus deployment.md.
```

## Why the Core/head split

Every line of the original PowerShell tool's actual logic - file walking, retry/
timeout, encoding detection, DOCX/PPTX/RTF/PDF extraction, regex matching,
AllInFile/Proximity gating, the incremental cache, and HTML/CSV/JSON report
generation - was already built entirely on plain .NET APIs (`System.IO`,
`System.IO.Compression.ZipArchive`, `System.Text.RegularExpressions`), not
anything PowerShell-specific. That meant it could be ported to C# as an
ordinary, portable class library with **no WinUI/Windows App SDK reference at
all**.

Keeping that code in its own `TextInFilesSearch.Core` project (rather than
mixed into the WinUI head project) means:

- It compiles and runs on any platform's .NET 8 SDK, which is exactly how it
  was actually developed and verified - the whole engine and ViewModel layer
  were built and tested with a real, offline .NET 8 SDK before this repository
  ever touched a Windows machine.
- The WinUI head project only needs to compile two files (`App.xaml.cs`,
  `MainWindow.xaml.cs`) against it - everything else is inherited, tested,
  logic. This sharply reduces how much of the app is genuinely unverified at
  the point this reaches your CI.
- If a future maintainer wants a different front end (a CLI, a web UI, another
  desktop framework), `TextInFilesSearch.Core` is already a clean, reusable
  unit with no UI assumptions baked in.

## Why no MVVM Toolkit (CommunityToolkit.Mvvm)

`CommunityToolkit.Mvvm` is a perfectly good, idiomatic choice for WinUI 3 MVVM
and would work fine at both build time and runtime here (it gets bundled into
the self-contained publish output like any other dependency). It was
deliberately not used for one practical reason: NuGet package restore was not
available in the sandbox this was developed in, so pulling in that package
would have made the entire ViewModel layer impossible to compile-check before
handing it off. Instead, `Helpers/ObservableObject.cs` and
`Helpers/RelayCommand.cs` are two small, dependency-free files implementing
the same `INotifyPropertyChanged` / `ICommand` patterns by hand. This is a
reasonable permanent choice (one less external dependency in a
self-contained-deployment-focused app), but swapping to `CommunityToolkit.Mvvm`
later is a mechanical, low-risk change if preferred - it wasn't a hard
architectural requirement, just what made local verification possible.

## What was migrated vs. retained

**Everything was migrated to native C#.** There is no PowerShell dependency
anywhere in the running application - it doesn't shell out to `powershell.exe`,
doesn't load `.ps1` files, and doesn't require PowerShell modules of any kind.
The `powershell/` folder exists purely so the original scripts remain
available as a historical reference and a second, independent implementation
to diff behavior against if a discrepancy is ever suspected.

This was possible because nothing in the original tool actually depended on
PowerShell-specific functionality (cmdlets with no .NET equivalent, the
pipeline object model, etc.) - every operation was already a thin PowerShell
wrapper around a plain .NET API call, which is precisely what let this port be
a faithful, line-for-line logic translation rather than a rewrite-from-scratch
guess at behavior.

## UI functionality vs. business logic (the requested inventory)

| Category | Where it lives now |
|---|---|
| User-interface functionality | `Views/MainWindow.xaml(.cs)` - forms, buttons, progress display, results list |
| Business/application logic | `TextInFilesSearch.Core/Services/*` |
| PowerShell-specific functionality | None retained - see above |
| Windows API interactions | `MainWindow.xaml.cs` only: `FolderPicker` + `WinRT.Interop.WindowNative` (window handle interop required for unpackaged WinUI 3 apps), and `Process.Start` to open the finished report |
| External dependencies | `Microsoft.WindowsAppSDK`, `Microsoft.Windows.SDK.BuildTools` (build-time NuGet, bundled into the self-contained publish output - see deployment.md) |
| Required permissions | None beyond ordinary file read/write in user-selected folders. No elevation is requested anywhere (`app.manifest` has no `requestedExecutionLevel`, so the OS default `asInvoker` applies). |
| Files/registry/services/processes accessed | Reads files under the user-chosen search folder; writes the report/cache files under the user-chosen output folder/cache path; no registry access; launches the OS-associated app for the finished report via `Process.Start` (the only process it ever starts) |
| Functionality that could not reasonably be migrated | None identified |

## Live progress reporting (the specific "PDF looks hung" concern)

The original tool's console progress only updated *between* files, so a
single slow PDF (or several running in parallel) showed no visible change for
however long that file took - it wasn't actually stuck, but there was no way
to tell from the outside. This is addressed structurally, not cosmetically:

- `SearchOrchestrator` reports a live `SearchProgressReport` after every file
  completes, and also runs a 500ms background ticker during parallel runs so
  elapsed-time displays keep moving even between completions.
- `TextExtractionService.ExtractPdfLines` takes a `PdfProgressCallback` that
  fires roughly every 150ms with the number of PDF streams scanned and elapsed
  time - this is surfaced per-file, not just per-run, so a single large PDF
  visibly reports "340 streams scanned, 8.2s" instead of going silent for its
  whole `PdfTimeoutSeconds` budget.
- In parallel mode, every concurrently-running file shows its own live status
  in the UI's in-flight list (`ViewModel.InFlightFiles`), rather than only the
  aggregate file-count progress bar.

## Verification

Everything under `TextInFilesSearch.Core` (and the `MainViewModel` inside it)
was compiled and exercised with a real, offline .NET 8 SDK during development,
using the dependency-free harness in `tests/TextInFilesSearch.Tests`. That
harness currently runs 36 checks covering: single-file/single-line edge cases
across every supported format (a real bug class caught this way during
development), all three match modes, exclude scopes, whole-word and regex
matching (including highlight-span correctness), the ASCII85 decoder's
case-sensitivity fix, RTF extraction, real DOCX/PPTX/PDF files generated by
independent libraries (the PDF specifically exercises the
ASCII85Decode+FlateDecode filter chain that silently failed before that bug
was found), parallel-vs-sequential result consistency, the full incremental
cache lifecycle, the Windows-1252 encoding fallback (including the
provider-registration step that only otherwise happens in the untestable
`App.xaml.cs`), and the ViewModel's run/cancel/progress/folder-picker
lifecycle via injected fakes.

**What was not, and could not be, verified in that environment:** the WinUI 3
`Views/` XAML/code-behind, the `.csproj`'s self-contained publish
configuration, and the actual packaged runtime behavior on a real Windows
machine. That is exactly what `.github/workflows/build.yml` exists to check on
its first real run - see deployment.md for the specific verification steps and
what to look at if that workflow fails.
