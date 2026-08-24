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

## First real-machine launch (2026-08-24): silent startup failure

The clean-machine test procedure above was finally run for real, for the
first time ever, on an actual Windows machine. Result: SmartScreen's "Run
anyway" prompt appeared and was accepted, then nothing happened - no
window, no taskbar entry, no error dialog. Root cause not yet confirmed
(no Windows access to reproduce with), but the most likely explanation:

**`WindowsAppSDKSelfContained=true` bundles the Windows App SDK's own
managed+native components, but does not bundle the Visual C++
Redistributable those native components themselves depend on**
(`vcruntime140.dll`, `msvcp140.dll`, `vcruntime140_1.dll`). These are
extremely commonly present on real Windows machines (many other
applications install them) but are not guaranteed on a genuinely clean
one, and this project's own target requirement is "nothing pre-installed."
A missing dependency of a *native* module can fail before any of this
app's own managed code - including a global exception handler - ever runs,
which matches the reported symptom (no error, not even a crash dialog)
better than a managed-code exception would.

**Immediate thing to try**: install the
[VC++ Redistributable (x64)](https://aka.ms/vs/17/release/vc_redist.x64.exe)
(official Microsoft installer) on the affected machine and re-launch. If
that fixes it, this is confirmed.

**What was changed in response, blind (no way to verify locally or via
CI - GitHub Actions has no interactive desktop session to launch a WinUI
GUI in)**:

1. `App.xaml.cs` now wires up `AppDomain.CurrentDomain.UnhandledException`
   and `Application.UnhandledException` at the very start of the `App`
   constructor, before anything else runs. Any *managed* exception during
   startup now writes to `crash.log` next to the executable and shows a
   plain Win32 `MessageBoxW` (not a WinUI `ContentDialog`, which needs a
   working XAML dispatcher that might not exist yet) instead of failing
   silently. This does **not** catch a true native-module-load failure
   happening before managed code starts - if the VC++ Redistributable
   theory above is correct, this specific fix won't surface anything,
   because the process never gets far enough to run it. It's still
   unconditionally worth having: it turns every *other* class of startup
   failure from silent into diagnosable.
2. `native-search/.cargo/config.toml` now statically links the MSVC C
   runtime into `native_search.dll` (`-C target-feature=+crt-static`), so
   *that* component at least has zero dependency on the VC++
   Redistributable being present. This doesn't touch
   `Microsoft.WindowsAppRuntime.dll` (a prebuilt Microsoft binary, not
   something this project compiles), which remains the prime suspect if
   the redistributable theory is correct.

**Update**: rather than wait for confirmation, the redistributable is now
bundled directly, by explicit direction - "self-contained" was always the
actual requirement, and depending on the VC++ Redistributable being
pre-installed contradicts it regardless of whether it turns out to be the
cause of this specific bug. `.github/workflows/build.yml` now copies
`vcruntime140.dll`, `vcruntime140_1.dll`, and `msvcp140.dll` from the
build runner's `System32` (present there because `windows-latest` ships a
full Visual Studio install, which installs the redistributable
system-wide) into the publish output as loose files next to the exe, with
a verification step that fails the build loudly if any are missing —
same pattern as the existing hostfxr/coreclr and
Microsoft.WindowsAppRuntime.dll checks. This is Microsoft's own documented
"local" (a.k.a. "app-local") deployment technique for these specific
DLLs: no installer, no registration, no elevation - Windows' DLL search
order checks the application's own directory before `System32`, so they're
picked up automatically. The DLLs themselves aren't vendored into the git
repo (they come from the build runner at build time, same as the .NET
runtime and Windows App SDK components already do), consistent with how
every other bundled runtime component in this publish pipeline is sourced.

This still hasn't been confirmed against the original symptom on a real
machine - it addresses the requirement ("must not depend on the host
machine") unconditionally, independent of whether it turns out to be the
actual root cause of the reported silent-startup bug.

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
