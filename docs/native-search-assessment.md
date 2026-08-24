# Native Search Engine — Architecture Assessment (Issue #2, Phase 1)

Repository reconnaissance for the "Native Offline Search Engine — Tantivy +
Advanced Text Indexing Evaluation" epic. Written before any Rust code exists,
per the epic's own phasing (`Phase 1 — Understand` before `Phase 2 — Verify`
before any implementation). See `docs/adr/` for the decisions this feeds.

## Current application, as it exists today

| Aspect | Finding |
|---|---|
| .NET version | net8.0 (Core library), `net8.0-windows10.0.19041.0` (WinUI head) |
| UI framework | WinUI 3 via Windows App SDK 1.5.240311000, unpackaged (no MSIX) |
| Architecture | Strict Core/head split — `TextInFilesSearch.Core` is a plain, zero-WinUI net8.0 class library; `TextInFilesSearch` is a thin WinUI shell. See root `CLAUDE.md`. |
| Search functionality | None today in the "index" sense. `MatchingEngine` does line-by-line literal/whole-word/regex matching over already-extracted text, per file, per run. No persistent index, no ranking — every search re-reads and re-scans matching files from disk (subject to the incremental cache below). |
| Extraction | `TextExtractionService.cs` (892 lines) — **entirely hand-rolled, zero third-party packages**, in `Core`. Confirmed via `grep PackageReference src/TextInFilesSearch.Core/*.csproj` → no output. Extraction is implemented in-house for: DOCX (zip+XML), PPTX (slides + speaker notes + SmartArt diagrams), XLSX, ZIP (recursive, nested archives), RTF, PDF (including an ASCII85+FlateDecode filter chain implemented by hand), plus the whole `ExtensionCatalog` of ~45 plain-text-ish formats read directly as text (code, config, logs, etc.) via `FileReaderService`. |
| Threading/concurrency | `SearchOrchestrator.cs` uses `Parallel.ForEachAsync` with a user-configurable `ThrottleLimit`, plus a background "ticker" task (`TickProgressAsync`) that keeps elapsed-time UI displays moving between file completions — this exists specifically to solve the "PDF processing goes silent for seconds" complaint (see `CLAUDE.md` → "Live progress reporting is a hard requirement"). Any native module must not regress this. |
| Config/storage | No fixed storage location today — no `%LOCALAPPDATA%` usage found (`grep -rn "LocalAppData\|ApplicationData\|AppData" src/` → no hits). The optional incremental cache (`CacheService.cs`) writes to a **user-specified file path** typed into the "Cache file" field, fingerprinted by settings hash + path + size + mtime, not a fixed app-data folder. A persistent Tantivy index needs to pick a storage convention from scratch — see ADR-007 (open). |
| Test infrastructure | `tests/TextInFilesSearch.Tests/Program.cs` — a dependency-free console harness (deliberately not xUnit/MSTest, to avoid any package restore), currently 60 checks. Runs via `dotnet run --project tests/TextInFilesSearch.Tests` on any platform. |
| Packaging/deployment | Self-contained, unpackaged, `win-x64` only. `SelfContained=true`, `WindowsAppSDKSelfContained=true`, `WindowsPackageType=None`, `PublishSingleFile=false` (deliberately — see csproj comments and `docs/deployment.md`). No admin rights, no internet, no pre-installed runtime required on the target machine. `.github/workflows/build.yml` is the real verification gate (builds, runs the test harness, publishes self-contained, checks for `hostfxr.dll`/`coreclr.dll`/`Microsoft.WindowsAppRuntime.dll` in the output). |
| Target platforms | `win-x64` only today. No ARM64 target exists yet in the csproj (`<Platforms>x64</Platforms>`, `<RuntimeIdentifiers>win-x64</RuntimeIdentifiers>`) — relevant to the epic's ARM64 questions: there is currently nothing to be compatible *with* on ARM64, so this is a green-field decision, not a constraint. |

## What should be preserved, replaced, wrapped, or migrated

- **Preserve as-is:** the Core/head split, the dependency-free test harness, the
  self-contained/unpackaged/no-admin deployment model, the live per-file
  progress reporting requirement, the `ExtensionCatalog` single-source-of-truth
  pattern.
- **Do not duplicate without justification:** the epic's Section 3 ("Unified
  Text Extraction Layer") asks for a Rust extractor trait covering PDF, DOCX,
  PPTX, XLSX, RTF, TXT, MD, CSV, TSV, JSON, XML, YAML, TOML, INI, ENV, LOG,
  HTML, CSS/SCSS/LESS, source code, and ZIP/archive contents. **All of this
  already exists, working, tested, and zero-dependency, in
  `TextExtractionService.cs` / `FileReaderService.cs` today.** Re-implementing
  it in Rust against mature crates (Section 2's own "do not reinvent mature
  algorithms" principle, applied to extraction rather than indexing) is a
  large duplicate effort with real regression risk against 60 existing tests
  and several previously-fixed bug classes (the ASCII85+FlateDecode PDF
  filter chain, the nested-ZIP handling, the whole-word lookaround boundary,
  etc. — see `CLAUDE.md` → "Bug classes already found and fixed once").
  **Recommendation carried into ADR-003:** the Rust module's initial scope
  should be *indexing and search only*, consuming text that C# has already
  extracted (the existing `FileSearchResult`/line-cache shape), rather than
  re-extracting from raw file bytes in Rust. Re-evaluate extraction ownership
  later only if a concrete need appears (e.g. wanting to index files the .NET
  side hasn't opened yet, for a background/always-on indexer).
- **Wrap, don't replace:** `MatchingEngine`'s literal/whole-word/regex/
  proximity/AllInFile matching modes and exclude-scope semantics are specific,
  tested behavior with known-fixed edge cases (folder-segment exclude
  matching, C#-style punctuation-edged whole-word boundaries). Tantivy's own
  query semantics (phrase/boolean/fuzzy/range) are a genuinely new capability
  layer, not a replacement for these modes — see open question in ADR-002
  around how "match mode" UI concepts map onto Tantivy queries, or whether
  they coexist as two distinct search paths (fast indexed search vs. the
  existing line-level scan) rather than one replacing the other.
- **Net-new:** the persistent index itself, the FFI boundary, incremental
  change-detection against a Tantivy index specifically (the existing
  `CacheService` incremental logic solves a different problem — skipping
  re-extraction — and is not a search index).

## Integration point

Proposed seam: a new `NativeSearchService` in `TextInFilesSearch.Core/Services`,
implementing a thin C# wrapper over P/Invoke calls into `NativeSearch.dll`
(built from a new top-level `native-search/` Rust crate, sibling to `src/`).
This keeps the "zero WinUI dependency in Core" property intact — P/Invoke has
no WinUI dependency — while giving the ViewModel layer a service to call
exactly like `SearchOrchestrator` today. Nothing in `src/TextInFilesSearch`
(the WinUI head) needs to know the native module exists.

## Open items carried forward (not resolved in this phase)

- ADR-002 (Tantivy adoption) — pending Phase 2 verification evidence.
- ADR-004/005/006 (FM-index / suffix array / bioinformatics candidates) —
  pending Phase 2 verification evidence.
- ADR-007 (index persistence location) — no existing convention in this app
  to inherit; likely `%LOCALAPPDATA%\TextInFilesSearch\index\`, follows
  standard Windows per-user-writable-without-admin guidance, but not yet
  written up as a decision.
- Whether Tantivy-based search becomes a parallel capability alongside the
  existing per-run line scan, or eventually subsumes it, is explicitly **not**
  decided here — Section 25 of the epic calls for a minimal vertical slice
  before any such call is made.
