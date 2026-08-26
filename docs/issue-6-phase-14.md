# Issue #6 Phase 14: adversarial test coverage (§53)

Epic §53 lists adversarial cases to test: corrupt PDF, malformed DOCX,
empty files, huge files, binary files with text extensions, invalid
UTF-8, long lines, extremely long paths, Unicode paths, duplicate names,
permission failures, files disappearing during indexing. Audited what
already existed (extraction-level unit tests for malformed zip/DOCX/PPTX/
XLSX bytes, `try_load_returns_none_for_corrupt_json`,
`opening_index_with_mismatched_schema_is_corrupt_index_not_panic`,
`run_candidates_skips_a_path_that_no_longer_exists`) and closed the real
gaps: nothing exercised these cases at the **orchestrator** level - a
full run over a real adversarial file, proving the whole pipeline (not
just one extractor function in isolation) handles it cleanly.

New `orchestrator.rs` tests, one real adversarial file/condition each:

- `empty_file_does_not_crash_and_is_not_a_hit` - a genuinely empty file
  alongside a real one; both get processed, the empty one is never a hit.
- `a_file_over_the_size_limit_is_skipped_as_too_large_not_read` - a real
  file against an artificially tiny `max_file_size_mb`, proving
  `TooLarge` classification without needing to actually write a
  multi-megabyte test fixture.
- `a_binary_file_with_a_text_extension_is_skipped_as_binary_not_crash` -
  NUL-byte content behind a `.txt` extension (the extension lying about
  the content is the adversarial part).
- `invalid_utf8_that_is_also_not_valid_windows_1252_does_not_crash` -
  byte sequences invalid under both UTF-8 and the Windows-1252 fallback,
  proving the full BOM→UTF-8→Windows-1252 chain never panics.
- `a_file_with_a_very_long_path_does_not_crash` - 80 levels of nested
  single-character directories (well past the classic 260-char Windows
  `MAX_PATH`).
- `a_unicode_filename_and_path_component_are_found_and_searched` - a
  directory and filename containing accented characters and an emoji
  (`café_🍎/résumé.txt`), proving Unicode paths are found and searched
  correctly, not mangled or skipped.
- `malformed_docx_bytes_are_a_read_error_not_a_crash` /
  `malformed_pdf_bytes_are_handled_without_a_crash` - garbage bytes with
  real `.docx`/`.pdf` extensions running through the *entire* pipeline
  (not just calling `extract_docx_lines`/`extract_pdf_lines` directly, as
  the existing extraction-level tests already did) - both land cleanly on
  `FileSearchStatus::ReadError`.
- `a_permission_denied_file_is_a_read_error_not_a_crash` (`#[cfg(unix)]`
  only - chmod bits don't translate to Windows ACLs, so this doesn't even
  compile on a Windows target rather than being a runtime `#[ignore]`).
  Skips its own assertion (not the whole test) when running as root,
  since Unix permission bits don't apply to root at all - same "don't
  force a test past what the environment can prove" judgment call as the
  truncation-detection gap already documented in `docs/issue-6-phase-10.md`.

**Not given a dedicated test**: "duplicate names" - two files sharing a
filename in different directories aren't actually an edge case for this
architecture (every `FileSearchResult`/index entry keys off the full
path, never the bare filename), so there's no real risk to prove absent
here. "Files disappearing during indexing" (as opposed to "disappeared
before a candidate-list run started," which
`run_candidates_skips_a_path_that_no_longer_exists` already covers) would
need a real file deleted mid-read - the same genuine, hard-to-do-safely
timing dependency already declined for the truncation-detection test in
Phase 10, for the same reason.

## Stress tests (100K files) - opt-in, not run by default

Added `orchestrator::tests::stress_test_100k_files`, `#[ignore]`d so it
never runs as part of the default `cargo test` suite - generating and
tearing down 100,000 real files on every contributor's every test run
would make the suite itself slow and disk-heavy for a scale this app's
actual use case (a folder search tool, not a corpus-indexing service)
rarely if ever approaches. Run on demand:
`cargo test -p search-core --release -- --ignored stress_test_100k_files --nocapture`.

Real, measured (this development machine, release build, 2026-08-26):

```
stress_test_100k_files: wrote 100000 files in 12.37s
stress_test_100k_files: searched 100000 file(s) in 6.13s (16308 files/sec), 14286 hit(s) (expected 14286)
```

100,000 files across 200 directories, every 7th file containing the
filter text. Asserts every file is accounted for (`file_results.len() ==
100_000`, none silently dropped), the exact expected hit count survives
at this scale (not just "roughly right"), and zero unexpected/read
errors. 500K/1M tiers were not added on top of this - 100K already
exercises the full pipeline (enumerate, extension-filter, bounded-
parallel process, match, tally) at a scale that would surface any
O(n²) behavior or resource exhaustion; going further would mean a much
longer opt-in test for diminishing evidentiary value without a specific
motivating concern, the same "measure first, don't build infrastructure
without evidence it's needed" judgment this project applies elsewhere.

## Verification

`cargo test --workspace`: **190/190 passing** (app 8, native-search 29 +
13 ffi_smoke, search-cli 4, search-core 126 + 10 fixtures) - up from 181
before this phase (9 new adversarial tests).
