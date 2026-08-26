# Issue #6 Phase 4: CLI/headless entry point

Epic §60: "Where practical, make the core engine usable without Dioxus...
possible to have search-engine -> Dioxus GUI / CLI / future API/service.
This also makes automated testing easier."

## Design

New workspace member `cli/` (binary `search-cli`), not a `[[bin]]` inside
`search-core` itself - keeps search-core's own `[dependencies]` exactly
what the library needs (no `clap`/arg-parsing pulled into every consumer,
including `app/`'s GUI build). `cli/` depends on `search-core` the same
way `app/` does - a second, independent consumer proving the core is
genuinely usable without Dioxus, not just theoretically GUI-free.

Exposes a reasonable, useful subset of `SearchSettings` as flags (search
path, filters/excludes, match mode, proximity lines, regex/whole-word,
extensions, exclude-folders, hidden files, max file size, group-by,
parallel + both throttle limits, dry-run, CSV/JSON export, and
`--index` to also build/update the fast-search index) - not a literal
port of every `SettingsPanel` field, with everything not exposed
defaulting to `SearchSettings::default()`. Runs `orchestrator::run`
directly (no progress channel - a terminal can't usefully redraw a live
progress display the way the GUI does, so a single summary line prints
at the end instead), writes the report via the same `report::
write_html_report`/`write_csv`/`write_json` the GUI uses, and can also
call `native_index::build_or_update_corpus_index` when `--index` is
passed.

## Verification

Real end-to-end tests (`cli/tests/cli_smoke.rs`) spawn the actual
compiled binary (`env!("CARGO_BIN_EXE_search-cli")`, no extra
`assert_cmd`-style dependency needed) against real files on disk - not a
synthetic shortcut into internals:

- Finds a real hit, writes a real report, the report actually contains
  the highlighted match.
- `--dry-run` reads/writes nothing (output folder doesn't even get
  created).
- `--csv`/`--json` write those files alongside the HTML report.
- An invalid regex filter in `--regex` mode is a clean non-zero exit with
  a real error message on stderr, not a panic.

Manually verified against real files first (before writing the automated
tests) - found a hit, wrote an 86KB report with the match actually
highlighted, `--dry-run`/`--csv`/`--json`/`--index` all behave correctly
including a real Tantivy index getting built on disk.

Full workspace (`cargo build --workspace` / `cargo test --workspace`):
app 8/8, native-search 38/38, search-cli 4/4 (new), search-core 91/91 -
all green, zero changes to any pre-existing test.
