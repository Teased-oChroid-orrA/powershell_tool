# native_search FFI contract

Phase 3 vertical slice for issue #2. See `docs/adr/ADR-001` through `ADR-003`
for why the boundary sits where it does (indexing/search only, not
extraction) and `docs/native-search-assessment.md` for the wider context.

## Where things live

```
native-search/                  Rust crate (cdylib "native_search" + lib)
  src/error.rs                    NsStatus, NsError - Section 18 error classes
  src/engine.rs                   Safe Rust core: schema, indexing, search.
                                   No unsafe, no FFI types - unit-tested on
                                   its own (7 tests, cargo test).
  src/ffi.rs                      The extern "C" surface. Every export is
                                   wrapped in catch_unwind - a Rust panic
                                   must never cross into .NET (Section 18).
  tests/ffi_smoke.rs               Exercises the raw extern "C" functions
                                   directly (5 tests) - proves the ABI shape
                                   itself round-trips, not just the Rust API.

src/TextInFilesSearch.Core/Native/
  NativeSearchInterop.cs          [LibraryImport] declarations, must match
                                   ffi.rs exactly.
  NativeSearchHandle.cs            SafeHandle wrapper around the opaque
                                   handle ns_create returns.
  NativeSearchStatus.cs           C# mirror of NsStatus - keep in sync.

src/TextInFilesSearch.Core/Services/NativeSearchService.cs
                                   Public, safe, IDisposable wrapper. This is
                                   the only type anything outside Native/
                                   should call.

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
| `ns_search(handle, query, limit) -> JSON buffer` | `NsStatus` | `ns_search(...)` |
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
passed including the four `native_search:` checks above, and
`native_search.dll` was confirmed present in the self-contained publish
output. The `[LibraryImport]`-generated marshalling, `SafeHandle` lifetime,
and UTF-8/byte-buffer conventions described in this document link and work
correctly across the real Rust/.NET boundary on Windows - not just asserted
by this document, actually run.

## What still isn't done

- Cancellation (Section 17) — not wired up. `ns_search`/`ns_commit` run to
  completion; there's no cancel token in this slice.
- `native_search.dll` is copied into place by CI shell steps, not a real
  MSBuild `Content`/reference item in `TextInFilesSearch.csproj` — a
  `dotnet publish` run outside this workflow won't include it yet.
- Schema-evolution and corruption-recovery hardening flagged as open in
  ADR-002 (items 9/10) — not addressed by this slice.
- A `%LOCALAPPDATA%` index-location convention (ADR-007, not yet written).
- Nothing in the WinUI head (`MainViewModel`/`MainWindow.xaml`) calls
  `NativeSearchService` yet — it exists as a capability, not a wired-up
  feature. The two search paths (existing line scan vs. native index) are
  still not reconciled, per ADR-001.

## What was actually verified locally (before CI ever ran)

Rust: fully built and tested (`cargo build`, `cargo clippy --all-targets` —
zero warnings, `cargo test` — 12/12 passing, covering round-trip indexing/
search, re-indexing replacing not duplicating, delete, index persistence
across reopen, and that a null handle / null pointer / invalid UTF-8 /
malformed query string all return a typed error status instead of
crashing).

C#: not locally compiled (no SDK on this machine), but confirmed compiling
and working correctly via the real CI run described above.
