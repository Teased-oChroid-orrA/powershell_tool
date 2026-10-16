# GS Engineering Toolbench

A Rust engineering application with native GPU workbenches for modelling,
finite-element analysis, aircraft frame/truss members, bushings and eccentric
bushings. The dashboard also opens the terminal tools for search, fastener holes,
lug analysis, pressure vessels, preload and material lookup.

The search engine handles text, Office documents, archives and PDFs, with live
progress, an optional persistent index and streamed reports. Rust is the sole
implementation; the earlier PowerShell and C# applications remain in git history.

## Native engineering workbenches

Start the native dashboard with `cargo run --release -p app-tui -- --gui`.
It opens FEA modelling and aircraft frame/truss, Bushing and Eccentric Bushing
workbenches, and launches terminal tools where those remain the interface.
The viewport uses direct Rust wgpu with egui controls. The terminal interface
remains available with `cargo run -p app-tui`.

For STEP import and solid editing, build with `--features occt` (OpenCASCADE).
On Windows with the self-contained static CRT, use `--features occt-source`.
The release folder must include `THIRD-PARTY-NOTICES.md` and `licenses/`.
A default build can display and solve a saved tessellation without the CAD kernel.

Choose the model's unit system in the Units dropdown; conversion preserves the
physical model. STEP coordinates are normalized by OpenCASCADE to millimetres.
Results remain subject to the displayed convergence, equilibrium and formulation
checks; an exaggerated deformation is labelled separately from true scale.

Run `python scripts/qualify.py` for the complete workspace and terminal-only
checks. Add `--gpu` on a machine with a native GPU adapter, and `--cad` to include
STEP kernel tests. The JSON report records the tested commit, commands and exits.
Native desktop interaction and clean-machine packaged startup still require a
platform smoke check; a successful compile alone does not establish those.

## Project status: Rust only

This project was migrated from **C#/WinUI 3** to **Rust**.
The Rust stack (`native-search/`, `search-core/`, `app-tui/`, `cli/` and
the solver crates) is the whole implementation; the original C#/WinUI app and
the PowerShell tool it was itself migrated from have been removed from the tree
(still in git history). See
[`docs/rust-rewrite-status.md`](docs/rust-rewrite-status.md) for exactly
why (short version: WinUI 3 cannot be built, run, or debugged on a
non-Windows machine at all, turning every UI iteration into a
tens-of-minutes-per-attempt Windows CI round-trip) and what's done vs.
still open.

The earlier Rust GUI heads (`app/` on dioxus-native and `app-egui/`) were
deleted once `app-tui/` reached parity on what it implements.

The Rust app is **functionally complete** - full parity with the C#
app's settings/matching/export behavior, plus features the C# app never
had (a persistent Tantivy-backed search index for fast re-search, a
headless CLI, SQLite-backed extraction-failure tracking, streaming
export for large result sets, and more - see "Features" below).

## Architecture at a glance

```
native-search/    Tantivy-backed indexing/search engine ("Fast re-search")
search-core/      Plain Rust library - all business logic, zero GUI dependency
app-tui/          Native wgpu/egui workbenches and ratatui terminal tools
cli/              Headless CLI entry point, same search-core engine
bushing-solver/, fastened-joint-solver/, pressure-vessel-solver/,
engineering-math/, mechanics-core/   Engineering solvers and shared math
```

`search-core` is a plain Rust library with no GUI dependency at all - it
builds and its full test suite runs on any platform's toolchain
(including this repo's own CI, on Linux, for speed). `app-tui` and `cli`
are both thin heads built on top of it; all real logic lives in
`search-core`, not duplicated in either head. Engineering workbenches call the
FEA and domain solver crates directly; their modelling and native controls live
in `app-tui/src/gpu_viewer`. See
[`CLAUDE.md`](CLAUDE.md) for the project map and the `AGENTS.md` file in
each crate for its contracts and pitfalls (why `fancy-regex` not
`regex`, why hand-rolled OOXML/PDF extraction instead of a parser crate,
and more), and
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
  folder, auto-excluded from future scans, skip-reindex-if-unchanged
  by modified time + size, brought up to date by an explicit build/update
  - there is no filesystem watcher; a search always re-checks the folder
  and includes any file changed since the build) that narrows full scans
  via a trigram candidate filter - always a safe superset pre-filter, never a
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

Requires the Rust toolchain (`rustup`). Native workbenches run on Windows, macOS
and Linux/X11 with a supported graphics driver; default builds include wgpu.
Use `--no-default-features` for the terminal-only application. Optional STEP
editing needs the OpenCASCADE feature described above.

```sh
# Run the GUI locally (any platform)
cargo run -p app-tui

# Run the headless CLI
cargo run -p search-cli -- --help
cargo run -p search-cli -- /path/to/folder --filter foo --filter bar
# bare invocation (no folder/filters) drops into an interactive menu:
cargo run -p search-cli

# Build a release binary
cargo build --release -p app-tui      # target/release/app-tui(.exe)
cargo build --release -p search-cli   # target/release/search-cli(.exe)
```

Windows-specific release builds cross-compile the same way:
`cargo build --release -p app-tui --target x86_64-pc-windows-msvc` (this is
exactly what CI does - see "CI" below).

## Running the tests

```sh
cargo test --workspace      # everything (what CI runs)
cargo test -p search-core   # zero GUI dependency - runs anywhere
cargo test -p search-cli    # also GUI-free
```

`search-core`'s suite covers all three match modes, exclude scopes,
whole-word/regex matching (including highlight-span correctness),
invalid-regex-filter error reporting, real DOCX/PPTX/XLSX/ZIP/PDF
fixture parity tests, the incremental cache lifecycle, CSV
formula-injection neutralization, the Windows-1252 encoding path, the
fast-search index's auto-exclude/skip-if-unchanged policy, and full
end-to-end orchestrator runs. The `app-tui` crate's rendering needs a
real terminal run to verify beyond its `TestBackend` tests - `cargo run
-p app-tui` locally (or in `tmux`) is the fast feedback loop for that.

## CI

`.github/workflows/qualification.yml` runs workspace and terminal-only checks on
Linux, macOS and Windows for relevant pull requests, or by manual dispatch.
`python scripts/qualify.py` runs the same checks locally. Native GPU and CAD
checks are explicit options because they require a graphics adapter or CAD kernel.
The manually dispatched/tagged `app-tui-build.yml` and `rust-build.yml` workflows
produce the Windows application and search CLI release artifacts. Native desktop
interaction and clean-machine startup must also be verified for a release.

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
verification for the Rust heads), `deployment.md`/`offline-build.md`
(historical, for the removed C# app), `toolbench-status.md` (history of
the multi-tool dashboard shell - the old `app/` head's view; obsolete), and a series of
`issue-*-status.md`/`issue-6-phase-*.md` docs recording the
evidence-driven investigation and decisions behind specific features and
rejected optimizations - read those before assuming something wasn't
considered.
