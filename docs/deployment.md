# Deployment

## Target requirements (from the migration brief)

- No internet access on the target machine
- No administrator privileges required
- No pre-installed .NET runtime
- No pre-installed Windows App SDK runtime
- No PowerShell modules required
- No installer requiring elevation

## Exact publish command

```powershell
dotnet publish src/TextInFilesSearch/TextInFilesSearch.csproj `
    --configuration Release `
    --runtime win-x64 `
    --self-contained true `
    --output publish/TextInFilesSearch `
    -p:WindowsAppSDKSelfContained=true `
    -p:WindowsPackageType=None `
    -p:PublishSingleFile=false
```

This is exactly what `.github/workflows/build.yml` runs on a clean
`windows-latest` GitHub Actions runner. The output is a **folder**, not a
single EXE - see "Why not a single file?" below.

## What ends up in the publish folder

- `TextInFilesSearch.exe` - the entry point
- `hostfxr.dll`, `coreclr.dll`, and the rest of the .NET runtime - bundled
  because of `--self-contained true`, so the target machine does not need
  .NET installed
- `Microsoft.WindowsAppRuntime.dll` and the other Windows App SDK native
  components - bundled because of `WindowsAppSDKSelfContained=true`, so the
  target machine does not need the Windows App SDK runtime installed
- `vcruntime140.dll`, `vcruntime140_1.dll`, `msvcp140.dll` - the Visual C++
  Redistributable runtime those Windows App SDK native components
  themselves depend on, bundled app-local by an explicit CI step (see
  "First real-machine launch" below for why this exists)
- All managed DLLs, XAML resource files (`.pri`), and content files needed to
  run

The build workflow's verification steps specifically check for the presence
of the runtime files above and fail the build loudly if they're missing,
rather than silently producing a framework-dependent build that would only
fail later on a machine without .NET/Windows App SDK preinstalled.

## Why unpackaged (no MSIX)?

`WindowsPackageType=None` and `EnableMsixTooling=false` mean the app is not
packaged as MSIX. MSIX packages normally need to be installed (registering the
package with the OS), and depending on configuration and provenance, that
installation step can prompt for elevation or a trusted certificate. An
unpackaged, self-contained, folder-based deployment can simply be copied to a
target machine and run directly - no install step, no elevation, no
certificate to trust in advance.

## Why not a single file?

`PublishSingleFile` is deliberately left `false`. WinUI 3's native resource
files and the Windows App SDK's own bundled native libraries have a
documented history of edge cases when extracted from a single-file bundle at
runtime (temp-directory extraction behavior, certain native interop paths not
resolving correctly). A working folder-based deployment was prioritized over a
single EXE that looks cleaner but hasn't actually been confirmed to work -
per the migration brief's own guidance to prefer reliability over a single
file. If a single EXE is wanted later, treat it as something to explicitly
verify on a clean machine, not something to assume works from a green build.

## Clean-machine test procedure

Because this application could not be run in the development sandbox
(no Windows, no Windows App SDK), the following procedure should be the first
real test of the actual application, ideally performed once against the
artifact produced by `.github/workflows/build.yml`:

1. Download the `TextInFilesSearch-win-x64-self-contained` artifact from the
   GitHub Actions run.
2. Copy the extracted folder to a Windows 10 (1809+) or Windows 11 machine
   that:
   - Has no internet connectivity (disconnect it, or use a machine that
     genuinely has none)
   - You do not have administrator rights on (or deliberately run as a
     standard/non-admin user)
   - Does not have the .NET runtime installed (check via `dotnet --info` at
     a command prompt - it should report "not found" or similar)
   - Does not have the Windows App SDK runtime installed
3. Double-click `TextInFilesSearch.exe`. It should launch without any
   dialog about missing dependencies, without requesting elevation, and
   without any network activity (verify with a tool like Resource Monitor's
   network tab if you want to confirm no connections are attempted).
4. Run a search against a small local test folder containing at least one
   `.txt`, `.docx`, `.pptx`, `.pdf`, and `.rtf` file, with a filter you know
   appears in each. Confirm hits are found and the report/CSV/JSON export (if
   enabled) are written to the chosen output folder.
5. Confirm the "in-flight" progress list shows live status while a PDF is
   being processed (this was the specific concern that motivated the live
   progress reporting design - see architecture.md).
6. Close the app and confirm no background process was left running.

If any step fails, that is genuinely new information this migration hasn't
yet accounted for - please report exactly which step failed and the error
text, since it likely points at a WinUI/Windows App SDK configuration detail
that only manifests on a real machine.

## First real-machine launch (2026-08-24): silent startup failure - RESOLVED

The clean-machine test procedure above was finally run for real, for the
first time ever, on an actual Windows machine, and surfaced a real bug
through three rounds of investigation. Recorded here in full because the
process matters as much as the fix: two reasonable-sounding theories were
tried and directly disproven by evidence before the real cause was found,
and the improved diagnostics built along the way are what actually made
that possible.

**Round 1 - no error at all.** SmartScreen's "Run anyway" prompt appeared
and was accepted, then nothing happened: no window, no taskbar entry, no
error dialog. Leading theory at the time: a missing Visual C++
Redistributable dependency of `Microsoft.WindowsAppRuntime.dll`'s own
native components, failing before any of this app's managed code -
including any exception handler - could run. Response: added global
`AppDomain`/`Application.UnhandledException` handlers (writing to
`crash.log` and showing a Win32 `MessageBoxW`, chosen specifically because
it doesn't need a working XAML dispatcher) so a startup failure would at
least become visible; statically linked `native_search.dll`'s own C
runtime; and (once the user restated the actual requirement - fully
self-contained, no host-machine dependency, full stop) bundled
`vcruntime140.dll`/`vcruntime140_1.dll`/`msvcp140.dll` app-local into the
publish output regardless of whether they turned out to be the cause.
**All of this was good defensive work and stayed** - see the sections
above - **but it was not the cause of this bug.**

**Round 2 - a real crash, but a wrong diagnosis.** The exception handler
worked: a `XamlParseException` at `MainWindow.InitializeComponent()`, but
`Exception.ToString()` gave only "XAML parsing failed." with no further
detail (a known limitation of exceptions crossing the WinRT ABI). The one
structural thing that stood out in the newly-added "Fast re-search" XAML -
its `x:DataType` bound against a C# `record` where every other binding in
the file used a plain class - looked like a plausible, well-documented
`x:Bind`-vs-`record` rough edge. `NativeSearchHit` was converted to a
plain class to match. **The exact same exception, same stack trace, same
line number, recurred anyway** - direct proof this theory was wrong, not
just unconfirmed. Response, rather than a third guess: improved the crash
handler to also capture `UnhandledExceptionEventArgs.Message` (a field
separate from `Exception.Message`, populated from native diagnostic text
before it's lost crossing the ABI) and the full `HResult`/inner-exception
chain.

**Round 3 - the real cause.** The improved log finally surfaced it:

```
Framework message (UnhandledExceptionEventArgs.Message): Cannot locate resource from 'ms-appx:///Views/MainWindow.xaml'.
HResult: 0x802B000A
```

**`resources.pri` - the compiled-XAML resource index every `ms-appx:///...`
URI resolves through, including `MainWindow.xaml` itself - was completely
absent from the publish output.** Root cause: `TextInFilesSearch.csproj`
had `EnableMsixTooling=false`, set for the reasonable-looking reason
"unpackaged deployment, so disable the packaging tooling." That property
does not narrowly control MSIX output, though - it gates the *entire*
packaging-tooling MSBuild target chain, and PRI generation rides along
with that chain even though it has nothing to do with MSIX. With it
false, `resources.pri` was never generated at all, so *every* XAML page
in the app - not something specific to the native-search UI work, not
content-dependent in any way, which is exactly why the record fix had zero
effect - was guaranteed to fail identically. This matches a
well-documented WinUI3 unpackaged-deployment gotcha.

**Fixed**: `EnableMsixTooling` set back to `true`.
`WindowsPackageType=None` is the actual, independently-verified control
for "no MSIX output" (the existing "Verify no MSIX / packaged-app
artifacts were produced" CI step confirms this every run, unaffected by
this change) - the two properties are not the same control, and conflating
them is exactly what caused this. A new CI step,
"Verify published artifact bundles resources.pri", now fails the build
loudly if this regresses again, the same as every other bundled runtime
component in this pipeline already does - this should have existed from
the start.

## Known limitations

- **PDF text extraction remains best-effort**, exactly as it was in the
  PowerShell version: no OCR, and PDFs with embedded/subsetted fonts (common
  from LaTeX/pdflatex) may extract as garbled or missing text. The UI flags
  files where extraction "looks unreliable" via a heuristic, but this is a
  hint, not a guarantee.
- **The WinUI 3 UI layer (`Views/`) and the self-contained publish
  configuration have not been verified beyond XML/XAML well-formedness
  checks and documented Windows App SDK deployment patterns.** This is the
  one part of the migration that could not be tested during development due
  to the lack of a Windows environment, and it's exactly what the first
  GitHub Actions run and the clean-machine procedure above are for.
- **Architecture is win-x64 only for now.** `RuntimeIdentifiers` is set to
  `win-x64`; ARM64 was left out rather than guessed at, per the brief's
  "make it configurable so ARM64 can be added later" - adding
  `win-arm64` to `RuntimeIdentifiers` and publishing with
  `--runtime win-arm64` should be enough when that's actually needed and
  tested.
- **`Microsoft.WindowsAppSDK` and `Microsoft.Windows.SDK.BuildTools` versions
  are pinned** to specific NuGet package versions in the `.csproj`. These
  should be bumped deliberately (and re-verified via the same clean-machine
  procedure) rather than left to float, since Windows App SDK has a history
  of breaking changes between minor versions.
