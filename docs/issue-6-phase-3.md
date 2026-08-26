# Issue #6 Phase 3: streaming HTML export

Epic §35: "The HTML report must support large result sets. Do not
construct massive HTML strings in memory." `build_html_report` used to
build the entire formatted report as one `String` before a single
`std::fs::write`/`tokio::fs::write`.

## Design

`search-core/src/report.rs` gained a `ReportSink` enum (`Buffer(&mut
String)` or `Writer(&mut dyn Write)`) and a shared `write_report_to_sink`
function driving all the existing, unmodified generation logic
(`append_toc`, `append_file_block`, the header/summary building, the
None/Created/Modified grouping branches - none of that changed). The only
change is *when* accumulated text gets committed to the sink:

- `ReportSink::commit` appends-and-clears for `Buffer` (needs everything
  resident anyway) or writes-and-clears for `Writer`.
- Commit points are the natural chunk boundaries the report already has:
  after the header/summary, after the table of contents, and - the one
  that actually bounds memory - **after every single file block**. At
  most one file's worth of formatted HTML (highlighted lines, before/
  after context) is ever resident when streaming to a file, not the
  whole report.

Two public entry points now share this core:

- `build_html_report(settings, run) -> String` - unchanged signature,
  used by every existing test and any caller that genuinely wants the
  whole report as a string.
- `write_html_report(path, settings, run) -> io::Result<u64>` (new) -
  streams straight to a `BufWriter<File>`, returns the real written file
  size (`std::fs::metadata`, not a pre-computed `String::len()`).

`app/src/state.rs`'s `finish_successful_run` switched from
`build_html_report` + `tokio::fs::write` to `write_html_report` (run via
`spawn_blocking`, matching this app's established pattern for other
blocking `native_search`/report calls). The large-report size warning now
checks the real on-disk byte count instead of an in-memory string length.

## Verification

New test `write_html_report_streams_identical_content_to_the_string_builder`
proves the streaming path (committing per file block) and the buffering
path (committing once at the end) produce **byte-for-byte identical**
output despite completely different commit granularity - both drive the
same `write_report_to_sink`, only the sink differs. All 8 pre-existing
`report::tests::html_report_*`/export tests pass unmodified (proving
`build_html_report`'s behavior is unchanged for every existing caller).
search-core: 81 lib tests (was 80, +1) + 10 fixture tests, all passing.
`cargo build -p app` clean, background launch shows no panic.
