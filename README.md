# Text In Files Search

A native Windows desktop application (WinUI 3 / C#) that recursively searches
a folder for keyword filters across `.txt`, `.log`, `.docx`, `.pptx`, `.rtf`,
`.pdf`, and many other file types, with a live-updating HTML report, CSV/JSON
export, an incremental cache for repeat searches, and parallel processing.

This is a from-scratch C# port of an earlier PowerShell tool with the same
purpose (kept in `powershell/` for reference only - see that folder's own
README). See `docs/architecture.md` for what moved where and why, and
`docs/deployment.md` for exact publish/verification instructions.

## Requirements to build

- .NET 8 SDK
- Windows 10/11 with the Windows App SDK workload (only needed to *build* the
  `TextInFilesSearch` WinUI project - `TextInFilesSearch.Core` and the test
  project build and run on any platform's .NET 8 SDK)

```powershell
dotnet build TextInFilesSearch.sln
```

## Running the tests

```powershell
dotnet run --project tests\TextInFilesSearch.Tests
```

This is a dependency-free console harness (not xUnit/MSTest) so it can build
and run without any package restore - see the project file for why. Exit code
0 means all checks passed.

## Publishing a self-contained, offline-runnable build

```powershell
dotnet publish src\TextInFilesSearch\TextInFilesSearch.csproj `
    --configuration Release --runtime win-x64 --self-contained true `
    --output publish\TextInFilesSearch `
    -p:WindowsAppSDKSelfContained=true -p:WindowsPackageType=None -p:PublishSingleFile=false
```

The `publish\TextInFilesSearch` folder can then be copied to any Windows
10/11 machine and run directly - no .NET install, no Windows App SDK install,
no admin rights, no internet access required. See `docs/deployment.md` for
the full clean-machine verification procedure.

## CI

`.github/workflows/build.yml` builds the solution, runs the test harness,
publishes the self-contained artifact, and verifies the published output
doesn't accidentally depend on a pre-installed .NET runtime, a pre-installed
Windows App SDK runtime, or MSIX packaging - failing the build loudly if it
does, rather than only discovering that on a target machine later.
