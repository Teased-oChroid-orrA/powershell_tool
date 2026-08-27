# Text In Files Search

A native Windows desktop app that recursively searches a folder for
keyword filters across `.txt`, `.log`, `.docx`, `.pptx` (slides, speaker
notes, and SmartArt diagram text), `.xlsx`, `.zip` (recursing into
entries, including nested zips), `.rtf`, `.pdf`, and dozens of other
code/config/data extensions - producing a live-updating HTML report plus
optional CSV/JSON/JSONL export.

The GUI (`app/`) is a multi-tool dashboard shell ("Toolbench") - a left
rail switches between tools, and this search feature is the first,
fully-functional one inside it. A few more tool slots exist in the rail
today as "Coming soon" placeholders (Duplicate Finder, Batch Rename, Log
Analyzer) with no logic behind them yet - see
[`docs/toolbench-status.md`](docs/toolbench-status.md).

## Project status: mid-migration, Rust is the active implementation

This project is being migrated from **C#/WinUI 3** to **Rust/Dioxus**.
The Rust stack (`native-search/`, `search-core/`, `app/`, `cli/`) is
where active development happens; the original C#/WinUI app (`src/`) and
the PowerShell tool it was itself migrated from (`powershell/`) are kept
as working references only - not built on, not deleted. See
[`docs/rust-rewrite-status.md`](docs/rust-rewrite-status.md) for exactly
why (short version: WinUI 3 cannot be built, run, or debugged on a
non-Windows machine at all, turning every UI iteration into a
tens-of-minutes-per-attempt Windows CI round-trip) and what's done vs.
still open.

The Rust app is **functionally complete** - full parity with the C#
app's settings/matching/export behavior, plus features the C# app never
had (a persistent Tantivy-backed search index for fast re-search, a
headless CLI, SQLite-backed extraction-failure tracking, streaming
export for large result sets, and more - see "Features" below).

## Architecture at a glance

```
native-search/    Tantivy-backed indexing/search engine ("Fast re-search")
search-core/      Plain Rust library - all business logic, zero GUI dependency
app/              Dioxus desktop GUI (dioxus-native/Blitz - no WebView2 dependency)
cli/              Headless CLI entry point, same search-core engine
src/              C#/WinUI 3 app - reference only, not actively developed
powershell/       The original PowerShell tool - reference only
```

`search-core` is a plain Rust library with no GUI dependency at all - it
builds and its full test suite runs on any platform's toolchain
(including this repo's own CI, on Linux, for speed). `app` and `cli` are
both thin heads built on top of it; all real logic lives in
`search-core`, not duplicated in either head. See
[`CLAUDE.md`](CLAUDE.md) for the full per-file architecture map and the
reasoning behind each major design decision (why `fancy-regex` not
`regex`, why hand-rolled OOXML/PDF extraction instead of a parser crate,
why `dioxus-native` instead of `dioxus-desktop`, and more), and
[`docs/architecture.md`](docs/architecture.md) for the original
PowerShell → C# migration map.

## Features

- **Match modes**: any line, all filters present somewhere in the file,
  or proximity (all filters within N lines of each other)
- **Filter semantics**: case-insensitive literal substring (default),
  whole-word/token matching (lookaround-based, so punctuation-edged
  filters like `C#` work standing alone), or full regex mode (with
  narrowing via mandatory-literal extraction against the fast-search
  index when available, and a typed error naming the exact bad filter on
  invalid regex input, never a bare crash)
- **Exclude filters** with line or whole-file scope
- **File types**: `.txt`/`.log`/plain text (encoding-detected: BOM →
  strict UTF-8 → Windows-1252 fallback), `.docx`, `.pptx` (slides +
  speaker notes + SmartArt diagrams), `.xlsx`, `.zip` (including nested
  zips and nested Office documents), `.rtf`, `.pdf` (including
  CID-keyed/Type0-font PDFs via `/ToUnicode` CMap resolution - the
  encoding most modern PDF generators use, plus an opt-in OCR fallback
  for image-only/scanned PDFs, pure-Rust and fully offline - no
  internet access or system OCR runtime needed), and dozens of other
  extensions - the full default list plus a type-to-filter tick-list
  picker and custom-extension add path
- **Fast re-search**: an optional persistent Tantivy index (per searched
  folder, auto-excluded from future scans, skip-reindex-if-unchanged,
  kept current by a filesystem watcher) that narrows full scans via a
  trigram candidate filter - always a safe superset pre-filter, never a
  replacement for the exact literal/regex line scanner, which stays the
  sole authority on what's actually a match
- **Live progress**: per-file in-flight status (not just an aggregate
  bar), PDF extraction progress reported every ~150ms during long
  extractions specifically so a slow file never looks like a frozen app
- **Robustness**: retry-with-backoff and a per-file timeout for
  locked/slow files, symlink-safe and cancellable directory walking,
  dry-run mode, an incremental result cache (fingerprinted by the
  settings that affect matching) so unchanged files are skipped on
  repeat searches, and a persistent extraction-failure log so a known-bad
  file isn't re-attempted every run
- **Export**: streaming HTML (dark-mode CSS, table of contents,
  per-filter bar chart, match highlighting, PDF low-confidence flagging)
  plus optional CSV (with formula-injection neutralization), JSON, and
  JSONL, all streamed rather than built fully in memory first
- **Security**: zip-bomb/deflate-bomb guards (bounded entry-count and
  inflated-size caps) on every archive-based format, so a pathological
  file degrades gracefully instead of hanging or exhausting memory

See [`docs/search-semantics.md`](docs/search-semantics.md) for the
formal semantic contract (case sensitivity, Unicode handling, known
limitations) and [`docs/benchmarking.md`](docs/benchmarking.md) for real,
measured performance numbers - not fabricated ones.

## Building and running (Rust stack)

Requires the Rust toolchain (`rustup`) - nothing else. Works on macOS,
Linux, or Windows; the GUI (`dioxus-native`/Blitz) has no
Windows-only rendering dependency, so the full app - not just
`search-core` - is developable and runnable on any platform.

```sh
# Run the GUI locally (any platform)
cargo run -p app
# or, for hot-reload during UI development, from app/:
cd app && dx serve

# Run the headless CLI
cargo run -p search-cli -- --help
cargo run -p search-cli -- /path/to/folder --filter foo --filter bar
# bare invocation (no folder/filters) drops into an interactive menu:
cargo run -p search-cli

# Build a release binary
cargo build --release -p app          # target/release/app(.exe)
cargo build --release -p search-cli   # target/release/search-cli(.exe)
```

Windows-specific release builds cross-compile the same way:
`cargo build --release -p app --target x86_64-pc-windows-msvc` (this is
exactly what CI does - see "CI" below).

## Running the tests

```sh
cargo test -p search-core   # zero GUI dependency - runs anywhere
cargo test -p search-cli    # also GUI-free
```

`search-core`'s suite covers all three match modes, exclude scopes,
whole-word/regex matching (including highlight-span correctness),
invalid-regex-filter error reporting, real DOCX/PPTX/XLSX/ZIP/PDF
fixture parity tests, the incremental cache lifecycle, CSV
formula-injection neutralization, the Windows-1252 encoding path, the
fast-search index's auto-exclude/skip-if-unchanged policy, and full
end-to-end orchestrator runs. The `app` crate (the actual rendered
window) needs a real run to verify beyond type-checking - `cargo run -p
app` locally is the fast feedback loop for that, on any platform.

## CI

`.github/workflows/rust-build.yml` runs `search-core`'s and `search-cli`'s
test suites (Linux, for speed), then builds `app` and `search-cli` for
`x86_64-pc-windows-msvc` on a Windows runner and uploads both as
artifacts - `app`'s published exe is also scanned to confirm it doesn't
accidentally link `WebView2Loader.dll` (a real regression guard: this
app deliberately avoids any WebView2 dependency, see `CLAUDE.md`).
`.github/workflows/build.yml` is the separate CI gate for the C#/WinUI
reference app, only relevant if that app is being deliberately touched
during the migration.

## Target environment

Windows 10 1809+ / Windows 11, `win-x64`. No internet access, no admin
rights, and no pre-installed runtime of any kind required on the machine
running the built app (build-time internet access - crates.io in CI - is
fine and expected; it's only the published, running application that
must be fully self-contained and offline-capable).

## More documentation

`docs/` has the full detail behind every design decision referenced
above - `rust-rewrite-status.md` (migration status), `architecture.md`
(the original PowerShell → C# map), `benchmarking.md` (real, measured
performance numbers with caveats), `search-semantics.md` (the formal
matching contract), `deployment-rust.md` (build/publish/clean-machine
verification for `app`/`cli`), `deployment.md`/`offline-build.md`
(the same for the C# reference app), `toolbench-status.md` (the
multi-tool dashboard shell `app/` is becoming), and a series of
`issue-*-status.md`/`issue-6-phase-*.md` docs recording the
evidence-driven investigation and decisions behind specific features and
rejected optimizations - read those before assuming something wasn't
considered.
