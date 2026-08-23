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
