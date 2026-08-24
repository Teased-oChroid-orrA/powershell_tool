# native_search FFI contract

Phase 3 vertical slice for issue #2. See `docs/adr/ADR-001` through `ADR-003`
for why the boundary sits where it does (indexing/search only, not
extraction) and `docs/native-search-assessment.md` for the wider context.

## Where things live

```
native-search/                  Rust crate (cdylib "native_search" + lib)
  src/error.rs                    NsStatus, NsError - Section 18 error classes
  src/engine.rs                   Safe Rust core: schema, indexing, search,
                                   cancellation (CancellationFlag,
                                   CancellableCollector). No unsafe, no FFI
                                   types - unit-tested on its own (10 tests,
                                   cargo test).
  src/ffi.rs                      The extern "C" surface. Every export is
                                   wrapped in catch_unwind - a Rust panic
                                   must never cross into .NET (Section 18).
  tests/ffi_smoke.rs               Exercises the raw extern "C" functions
                                   directly (10 tests) - proves the ABI shape
                                   itself round-trips, not just the Rust API.

src/TextInFilesSearch.Core/Native/
  NativeSearchInterop.cs          [LibraryImport] declarations, must match
                                   ffi.rs exactly.
  NativeSearchHandle.cs            SafeHandle wrapper around the opaque
                                   handle ns_create returns.
  NativeSearchCancellationHandle.cs SafeHandle wrapper around the opaque
                                   handle ns_cancel_token_create returns.
  NativeSearchStatus.cs           C# mirror of NsStatus - keep in sync.
  NativeSearchPaths.cs            Default index location (ADR-007).

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
- **Handle lifetime**: `NativeSearchHandle` is a `SafeHandle`. Every
  handle-taking export marshals it via ref-counted `DangerousAddRef`/
  `DangerousRelease`, so a concurrent `Dispose()` can't race a call already
  in flight into a use-after-free. Don't add a raw-`IntPtr` overload for any
  function except `ns_destroy` (which exists only for `ReleaseHandle` itself
  to call, since a handle can't pass itself as an argument mid-release).
- **Panics**: every `ffi.rs` export runs its body inside `catch_unwind`
  and converts a caught panic to `NsStatus::InternalError` plus a message —
  verified by the Rust test suite, not just asserted in a comment.
- **Documents are immutable in Tantivy** (ADR-002 item 6): `index_document`
  deletes any existing document with the same `id` first, so re-indexing a
  changed file is a safe call, not a duplicate — this happens inside
  `engine.rs`, callers don't need to delete-then-add themselves.
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

**Not yet re-confirmed on CI**: the cancellation support (`ns_cancel_token_*`,
`NativeSearchCancellationToken`, the real MSBuild `Content` item replacing
the old CI-only DLL copy) and the ADR-007 index-path helper were added
*after* that run. They're fully covered by the local Rust suite (`cargo
test`, 20/20 — see below) and written to the same contract on the C# side,
but per-commit CI runs are being batched rather than triggered on every
change (GitHub Actions minutes) — the next full CI run is the one that will
confirm these compile and work on real hardware too. Don't treat this
section's "confirmed on real hardware" claim as covering anything added
after 2026-08-24.

## What still isn't done

- Schema-evolution and corruption-recovery hardening flagged as open in
  ADR-002 (items 9/10) — not addressed by this slice.
- Nothing in the WinUI head (`MainViewModel`/`MainWindow.xaml`) calls
  `NativeSearchService` yet — it exists as a capability, not a wired-up
  feature. The two search paths (existing line scan vs. native index) are
  still not reconciled, per ADR-001. This also means nothing in the app
  actually threads a .NET `CancellationToken` through to
  `NativeSearchCancellationToken.Cancel()` yet — the mechanism exists and
  is tested, but nothing calls it from a real cancel button.
- Index growth/cleanup semantics when `SearchPath` changes (ADR-007's open
  item) — undecided until the above wiring happens.
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
- Index location decided and implemented: `%LOCALAPPDATA%\TextInFilesSearch\native-index\`,
  see `docs/adr/ADR-007-index-persistence-location.md` and
  `NativeSearchPaths.GetDefaultIndexDirectory()`/`EnsureIndexDirectoryExists()`.
  Not called by anything yet (same caveat as the WinUI-wiring item above).
- Cancellation (Section 17) for `ns_search` - see the Conventions section
  above and `NativeSearchCancellationToken`. Not yet CI-verified (see note
  above); locally Rust-tested only.

## What was actually verified locally

Rust: fully built and tested (`cargo build`, `cargo clippy --all-targets` —
zero warnings, `cargo test` — 20/20 passing: round-trip indexing/search,
re-indexing replacing not duplicating, delete, index persistence across
reopen, that a null handle / null pointer / invalid UTF-8 / malformed query
string all return a typed error status instead of crashing, and (added
after the CI run above) that a pre-cancelled token reports
`NsStatus::Cancelled` from both the safe Rust API and the raw FFI surface,
an un-cancelled token doesn't block a search, and cancelling a token after
a search already succeeded doesn't retroactively fail that completed call).

C#: not locally compiled (no SDK on this machine). Everything through the
2026-08-24 CI run is confirmed compiling and working correctly; the
cancellation/MSBuild-integration/ADR-007 additions since then are written
to the same contract but await the next CI run for confirmation.
