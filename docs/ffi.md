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

## What Phase 3 does *not* cover yet

- Cancellation (Section 17) — not wired up. `ns_search`/`ns_commit` run to
  completion; there's no cancel token in this slice.
- Build/publish integration — `native_search.dll` is not yet copied into the
  WinUI head's publish output, and `.github/workflows/build.yml` does not
  yet build the Rust crate. Both are real work before this is runnable
  end-to-end on Windows; tracked as a follow-up, not done here.
- Schema-evolution and corruption-recovery hardening flagged as open in
  ADR-002 (items 9/10) — not addressed by this slice.
- A `%LOCALAPPDATA%` index-location convention (ADR-007, not yet written).

## What was actually verified this pass

This machine has no Windows install and no .NET SDK, so the C# side above
is written to contract but **not compiled**. The Rust side was fully built
and tested locally (`cargo build`, `cargo clippy --all-targets`, `cargo
test` — 12/12 passing, covering round-trip indexing/search, re-indexing
replacing not duplicating, delete, index persistence across reopen, and
that a null handle / null pointer / invalid UTF-8 / malformed query string
all return a typed error status instead of crashing). The C# side needs a
real `dotnet build` on the next machine that has the SDK before it can be
trusted — don't treat it as verified.
