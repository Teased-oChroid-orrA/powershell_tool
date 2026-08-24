# native_search FFI contract

Phase 3 vertical slice for issue #2. See `docs/adr/` (ADR-001 through
ADR-011) for the architectural decisions behind this — ADR-001/003 for why
the boundary sits where it does (indexing/search only, not extraction),
ADR-008/009/010 for incremental-indexing, serialization, and single-vs-
multi-index strategy, ADR-011 for the in-folder index location (supersedes
ADR-007) — and `docs/native-search-assessment.md` for the wider context.
`docs/benchmarking.md` and `docs/offline-build.md` cover Sections 13 and 15
respectively.

## Where things live

```
native-search/                  Rust crate (cdylib "native_search" + lib)
  src/error.rs                    NsStatus, NsError - Section 18 error classes
  src/engine.rs                   Safe Rust core: schema, indexing, search,
                                   cancellation (CancellationFlag,
                                   CancellableCollector), schema-mismatch
                                   detection on open (ADR-002 item 9),
                                   get_document_metadata for change
                                   detection (issue #2/ADR-011 - "only
                                   re-index if different"). No unsafe, no
                                   FFI types - unit-tested on its own (17
                                   tests, cargo test).
  src/ffi.rs                      The extern "C" surface. Every export is
                                   wrapped in catch_unwind - a Rust panic
                                   must never cross into .NET (Section 18).
  tests/ffi_smoke.rs               Exercises the raw extern "C" functions
                                   directly (13 tests) - proves the ABI shape
                                   itself round-trips, not just the Rust API.
  benches/indexing_and_search.rs  Section 13 benchmark harness - manual
                                   timing, not criterion. See
                                   docs/benchmarking.md for measured numbers
                                   and their caveats.

src/TextInFilesSearch.Core/Native/
  NativeSearchInterop.cs          [LibraryImport] declarations, must match
                                   ffi.rs exactly.
  NativeSearchHandle.cs            SafeHandle wrapper around the opaque
                                   handle ns_create returns.
  NativeSearchCancellationHandle.cs SafeHandle wrapper around the opaque
                                   handle ns_cancel_token_create returns.
  NativeSearchStatus.cs           C# mirror of NsStatus - keep in sync.
  NativeSearchPaths.cs            In-folder index location (ADR-011).

src/TextInFilesSearch.Core/Services/
  NativeSearchService.cs           Public, safe, IDisposable wrapper. This is
                                   the only type anything outside Native/
                                   should call.
  NativeSearchCancellationToken.cs Public wrapper around a cancellation
                                   handle - pass to Search(), call Cancel()
                                   from another thread.

src/TextInFilesSearch.Core/Models/NativeSearchModels.cs
                                   NativeDocumentInput (what you index),
                                   NativeSearchHit (what Search returns).
```

## Functions

| Rust (`ffi.rs`) | Returns | C# (`NativeSearchInterop`) |
|---|---|---|
| `ns_create(index_dir) -> handle` | `NsStatus` | `ns_create(string, out NativeSearchHandle)` |
| `ns_destroy(handle)` | — | `ns_destroy(IntPtr)` (SafeHandle.ReleaseHandle only) |
| `ns_index_document(handle, id, path, filename, extension, title, modified_unix, created_unix, size, body, body_len)` | `NsStatus` | `ns_index_document(...)` |
| `ns_delete_document(handle, id)` | `NsStatus` | `ns_delete_document(...)` |
| `ns_commit(handle)` | `NsStatus` | `ns_commit(...)` |
| `ns_get_document_metadata(handle, id) -> (found, modified_unix, size)` | `NsStatus` | `ns_get_document_metadata(...)` |
| `ns_search(handle, query, limit, cancel_token) -> JSON buffer` | `NsStatus` | `ns_search(...)` |
| `ns_cancel_token_create() -> token` | `NsStatus` | `ns_cancel_token_create(out NativeSearchCancellationHandle)` |
| `ns_cancel_token_cancel(token)` | `NsStatus` | `ns_cancel_token_cancel(...)` |
| `ns_cancel_token_destroy(token)` | — | `ns_cancel_token_destroy(IntPtr)` (SafeHandle.ReleaseHandle only) |
| `ns_last_error() -> buffer` | `NsStatus` | `NativeSearchInterop.TakeLastError()` |
| `ns_free_buffer(ptr, len)` | — | called internally by `CopyAndFreeBuffer` |

## Conventions

- **Strings**: `id`/`path`/`filename`/`extension`/`title`/`query` are
  NUL-terminated UTF-8 C strings (`StringMarshalling.Utf8` on the C# side).
- **`body`** is passed as an explicit `(pointer, length)` byte pair instead,
  not a C string — issue #2 Section 19 treats extracted document text as
  hostile input, and a NUL-terminated string would silently truncate at the
  first embedded NUL byte a corrupted document might contain.
- **Errors**: every function returns an `NsStatus` (`int`/`NativeSearchStatus`).
  On anything other than `Ok`, call `ns_last_error` (wrapped by
  `NativeSearchService` into `NativeSearchException`) for the message. The
  slot is thread-local and cleared at the start of the *next* `ns_*` call on
  that thread — read it immediately after the failing call, don't defer.
- **Buffers**: `ns_search` and `ns_last_error` both allocate a buffer the
  Rust side owns until `ns_free_buffer` releases it. `NativeSearchInterop.CopyAndFreeBuffer`
  is the single choke point on the C# side — nothing else should call
  `ns_free_buffer` directly.
- **Handle lifetime**: `NativeSearchHandle`/`NativeSearchCancellationHandle`
  are `SafeHandle`s. A **required** handle-taking export (`ns_index_document`,
  `ns_delete_document`, `ns_commit`, `ns_cancel_token_cancel`, and `ns_search`'s
  own `handle` parameter) takes the `SafeHandle`-derived type directly -
  LibraryImport marshals it via ref-counted `DangerousAddRef`/`DangerousRelease`
  automatically, so a concurrent `Dispose()` can't race a call already in
  flight into a use-after-free. An **optional** handle (`ns_search`'s
  `cancel_token`) cannot use that same pattern: the generated
  `SafeHandleMarshaller` does not null-check before dereferencing, so
  passing `null` throws `NullReferenceException` instead of meaning "no
  token" — confirmed by a real CI failure on Windows (`SafeHandleMarshaller<T>
  .ManagedToUnmanagedIn.FromManaged`), not assumed. `ns_search`'s
  `cancelToken` parameter is `IntPtr` instead, and
  `NativeSearchService.Search` performs the same `DangerousAddRef`/
  `DangerousGetHandle`/`DangerousRelease` sequence by hand, passing
  `IntPtr.Zero` for "no token." Don't add a raw-`IntPtr` overload for a
  *required* handle parameter — `ns_destroy`/`ns_cancel_token_destroy`
  exist only because `ReleaseHandle` can't pass itself as an argument
  mid-release; a raw `IntPtr` for an *optional* handle parameter (like
  `ns_search`'s) is the correct, deliberate pattern instead.
- **Panics**: every `ffi.rs` export runs its body inside `catch_unwind`
  and converts a caught panic to `NsStatus::InternalError` plus a message —
  verified by the Rust test suite, not just asserted in a comment.
- **Documents are immutable in Tantivy** (ADR-002 item 6): `index_document`
  deletes any existing document with the same `id` first, so re-indexing a
  changed file is a safe call, not a duplicate — this happens inside
  `engine.rs`, callers don't need to delete-then-add themselves.
- **Change detection** (issue #2/ADR-008/ADR-011): `ns_get_document_metadata`
  looks up the `(modified_unix, size)` stored for an `id` via an exact
  `TermQuery`, deliberately **not** `QueryParser::parse_query` — `id` is an
  arbitrary caller-supplied string (a file path), and the free-text parser
  would mis-parse anything containing query syntax characters (a Windows
  path like `C:\...` would be read as field `C` with value `\...`).
  `NativeSearchService.TryGetDocumentMetadata`/`MainViewModel.IndexHitsForFastSearch`
  use this to skip re-indexing a file whose modified time and size haven't
  changed, instead of unconditionally re-indexing every hit on every run.
- **Cancellation** (Section 17): `ns_search`'s `cancel_token` parameter is
  optional (null = no cancellation). A live token from
  `ns_cancel_token_create`, cancelled via `ns_cancel_token_cancel` from
  another thread, aborts the search with `NsStatus::Cancelled` — checked
  before the search starts and again before each index segment is scanned
  (`engine::CancellableCollector`). This is real, working cancellation for
  the common multi-segment case, **not** a guarantee of instant interruption
  mid-scan of one large segment — Tantivy's `SegmentCollector::collect` has
  no early-exit hook, so a single huge segment can't be interrupted once
  its scan has started. Don't oversell this as hard real-time cancellation
  in anything built on top of it.

## CI (`.github/workflows/build.yml`)

Since this development machine has neither Windows nor a .NET SDK, GitHub
Actions' `windows-latest` runner is where the C# side actually gets
compiled and exercised for the first time — not an afterthought. The
workflow now:

1. Installs a pinned Rust toolchain (`dtolnay/rust-toolchain@stable`,
   `x86_64-pc-windows-msvc`) and runs `cargo build --release` + `cargo test`
   for `native-search/` — the same 12-test suite verified locally.
2. Builds the whole `.sln` (this is what actually proves the
   `[LibraryImport]`-generated marshalling code in `NativeSearchInterop.cs`
   compiles — the biggest previously-unverified risk).
3. Copies the built `native_search.dll` next to the test harness's own
   output, then runs the dependency-free suite (`Program.cs`), which
   includes a **Test 35** that round-trips a real `NativeSearchService`
   call through the actual process boundary: index two documents, commit,
   search, assert the right one comes back with fields intact, delete,
   commit, search again, and confirm an empty query surfaces as a typed
   `NativeSearchException` rather than crashing. If `native_search.dll`
   isn't present (e.g. a developer running the harness locally without the
   Rust toolchain installed), this prints `SKIP`, not `FAIL` — it never
   blocks unrelated C# iteration.
4. Copies `native_search.dll` into the self-contained publish output and
   verifies it's actually there, alongside the existing hostfxr/coreclr/
   WindowsAppRuntime checks.

This closes the gap flagged after Phase 3: the C# side is no longer
"written to contract but unverified" once this workflow runs.

**Confirmed on real hardware** (run
[32688244936](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/32688244936),
2026-08-24): all 19 steps green, all 68 checks in the dependency-free suite
passed including the four `native_search:` checks that existed at that
point, and `native_search.dll` was confirmed present in the self-contained
publish output. The `[LibraryImport]`-generated marshalling, `SafeHandle`
lifetime, and UTF-8/byte-buffer conventions described in this document link
and work correctly across the real Rust/.NET boundary on Windows for
everything that run covered - not just asserted by this document, actually
run.

**Second CI run** (run
[32690778016](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/32690778016),
2026-08-24, after cancellation/ADR-007/schema-check/benchmark/WinUI-wiring
work): Rust build+test passed on Windows, and — significant on its own —
the **entire `.sln` compiled clean, including the new `MainWindow.xaml`/
`MainViewModel.cs` wiring**. The test harness itself then failed with a
real, genuine bug: `NativeSearchInterop.ns_search`'s optional
`NativeSearchCancellationHandle?` parameter threw `NullReferenceException`
from the generated `SafeHandleMarshaller` when passed `null` (Test 35's
"empty query" check calls `Search` without a cancellation token). This
directly contradicted this document's own prior "Don't add a raw-`IntPtr`
overload for any function except `ns_destroy`" guidance — that guidance
was correct for *required* handles, wrong for *optional* ones, and CI is
what caught the difference, not a compile error or local test (the Rust
side had no way to exercise the .NET marshalling layer). **Fixed**:
`ns_search`'s `cancelToken` is now `IntPtr`, with `NativeSearchService.Search`
doing the `DangerousAddRef`/`DangerousGetHandle`/`DangerousRelease`
sequence by hand — see the Conventions section's "Handle lifetime" bullet
for the corrected guidance. Publish/publish-verification steps were
skipped as a downstream consequence of the test-harness failure, not
independently tested this run.

**Third CI run — fully green** (run
[32691063260](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/32691063260),
2026-08-24): all 19 steps passed, including the exact check that failed
before (`native_search: an empty query surfaces as a typed
NativeSearchException, not a crash`), the cancellation checks, and all
four new ViewModel-level checks
(`ViewModel: NativeSearchCommand's underlying search finds the file
indexed by the run above` among them — the WinUI/`MainViewModel` wiring's
first real proof, not just a clean compile). `native_search.dll` (5.8MB)
confirmed present in the self-contained publish output. Everything in this
document and in `docs/issue-2-status.md` as of this run is real,
CI-confirmed-on-Windows work, not a claim awaiting verification.

## What still isn't done

- The two search paths (existing line scan vs. native index) are
  deliberately kept visibly separate in the UI (a labeled "experimental"
  panel), not unified into one search experience — see
  `docs/issue-2-status.md`'s "WinUI wiring" section for why that
  unification is left as a real, undecided product-design question rather
  than guessed at here.
- Index growth/cleanup semantics: a file that's deleted or moved out of
  `SearchPath` between runs stays in the index indefinitely (nothing calls
  `DeleteDocument` for files that no longer exist) - `IndexHitsForFastSearch`
  only ever adds/updates from the current run's hits, never prunes.
  Undecided; ADR-011's in-folder location makes this lower-stakes than
  ADR-007's global index would have (deleting the whole searched folder
  takes its index with it), but a file deleted from an otherwise-still-
  searched folder is still a real gap.
- `ns_commit`/`ns_index_document` have no cancellation support - only
  `ns_search` does. Per-file indexing cancellation is effectively already
  covered at the .NET orchestration layer instead (whatever eventually
  calls `IndexDocument` per file can just stop calling it once its own
  `CancellationToken` fires, mirroring how `SearchOrchestrator` already
  works) — interrupting a `commit()` mid-flight is also a correctness risk
  worth avoiding deliberately, not an oversight.

## Resolved since the initial Phase 3 pass

- `native_search.dll` is now a real MSBuild `Content` item in
  `TextInFilesSearch.csproj` (`Condition="Exists(...)"`, so a Rust-less
  local `dotnet build` still succeeds), not a CI-only shell copy — CI's
  publish-output check now verifies that pipeline actually worked rather
  than trusting a manual copy step.
- Index location: **revised from ADR-007's global `%LOCALAPPDATA%` design
  to ADR-011's in-folder `<SearchPath>\.native-search-index\`**, by direct
  user direction after ADR-007 had already shipped and been CI-verified.
  `NativeSearchPaths.GetIndexDirectory(searchPath)` + the shared
  `IndexFolderName` constant; `BuildSettings()` auto-excludes that folder
  name so the normal line-scan search never walks into it.
- Change detection (issue #2): re-indexing now skips files whose modified
  time and size match what's already stored - `ns_get_document_metadata`/
  `TryGetDocumentMetadata`, wired into `IndexHitsForFastSearch`.
- **WinUI wiring**: `MainViewModel`/`MainWindow.xaml` now call
  `NativeSearchService` directly — `IndexForFastSearch` toggle, a "Fast
  re-search" panel with its own query box/results list/cancel button. See
  `docs/issue-2-status.md`'s "WinUI wiring" section for the full picture,
  including two real thread-safety bugs (UI-thread blocking, a lazy-init
  race) caught and fixed while building it, and this section's own
  "not yet re-confirmed on CI" caveat below, which applies to this too.
- Cancellation (Section 17) for `ns_search` - see the Conventions section
  above and `NativeSearchCancellationToken`. Not yet CI-verified (see note
  above); locally Rust-tested only.
- ADR-002 items 9 (schema evolution) and 10 (corruption recovery) -
  re-verified directly against Tantivy's source and resolved: schema
  mismatches on open now fail fast with `NsStatus::CorruptIndex` (our own
  code, tested), and Tantivy's own per-file footer/CRC corruption detection
  was confirmed to already exist and run automatically on every file open,
  stronger than the 2019-roadmap-issue uncertainty originally suggested.
  See ADR-002's "Follow-up verification" section.
- ADR-002 item 12 (offline/vendored build) - re-verified with a real local
  `cargo vendor` + `cargo build --offline` run (succeeded), but corrected:
  this is **not** a pure-Rust build (`zstd-sys` needs a C compiler) as
  first assumed. See `docs/offline-build.md`.
- A Section 13 benchmark harness (`native-search/benches/`) with real,
  measured (not fabricated) numbers from this development machine - see
  `docs/benchmarking.md` for the numbers and why they're explicitly not a
  performance SLA for the target hardware.
- ADR-008 (incremental indexing strategy), ADR-009 (FFI serialization
  strategy), ADR-010 (multi-index vs. Tantivy-only architecture) - all
  three formalize decisions already implemented in code, closing out the
  ADR checklist from the epic's Section 21.
- Two more Definition-of-Done items proven with tests, not just designed:
  structured field filters (`extension:.pdf`-style queries -
  `structured_extension_filter_scopes_results`) and that one rejected
  document doesn't affect others indexed around it
  (`a_rejected_document_does_not_affect_documents_indexed_around_it`).

## What was actually verified locally

Rust: fully built and tested (`cargo build`, `cargo clippy --all-targets`
(including `--benches`) — zero warnings, `cargo test` — 30/30 passing,
adding to the 23/23 baseline above: `get_document_metadata` returns the
stored `(modified_unix, size)` for a known id, `None` for an unknown one,
reflects the *latest* re-index rather than the stale original, rejects an
empty id, and round-trips correctly through the raw `extern "C"` surface
(found/not-found/null-handle cases).

C#: not locally compiled (no SDK on this machine), but confirmed on real
Windows hardware — run
[32726269929](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/32726269929),
2026-08-24: all 19 steps green, including the change-detection check
(`ViewModel: re-running over unchanged files reports them as already up
to date, not re-indexed`) and the auto-exclusion checks
(`ViewModel: BuildSettings() automatically excludes the native_search
index folder`, `...normal search never descends into the auto-excluded
native_search index folder`). One real bug was caught and fixed along the
way in the prior run
([32725890735](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/32725890735)):
`NativeSearchCommand.CanExecute` gained a `SearchPath` requirement
(ADR-011 — the index lives inside `SearchPath`), and Test 36 checked
"becomes true once a query is typed" before `SearchPath` was set, which
the old contract never required. Fixed by reordering the test and adding
an explicit intermediate-state assertion — a genuine test bug caught by
CI, not a product bug.
