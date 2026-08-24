//! The C ABI. Issue #2 Section 16/18: opaque handles only, no Rust types
//! (`String`, `Vec<T>`, `Result<T>`, trait objects) cross this boundary, and
//! no panic is allowed to unwind into .NET - every exported function's body
//! runs inside `catch_unwind` and converts a caught panic into
//! `NsStatus::InternalError` plus a message retrievable via `ns_last_error`.
//!
//! String convention: short, well-formed identifiers (id/path/filename/
//! extension/title/query) are passed as NUL-terminated UTF-8 C strings.
//! `body` is passed as an explicit (ptr, len) byte slice instead, because it
//! is extracted document text that issue #2 Section 19 asks us to treat as
//! hostile input - a NUL-terminated string would silently truncate on the
//! first embedded NUL byte a corrupted document might contain.

use std::cell::RefCell;
use std::ffi::{c_char, CStr};
use std::os::raw::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::ptr;
use std::slice;

use crate::engine::{CancellationFlag, DocumentInput, NativeSearchEngine};
use crate::error::{NsError, NsStatus};

thread_local! {
    /// Last error on this thread, across any `ns_*` call. Simpler than a
    /// per-handle slot (sqlite3_errmsg-style) and sufficient for a single
    /// native-search handle per process, which is this app's actual usage
    /// pattern (one index, one .NET process).
    static LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_last_error(message: impl Into<String>) {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(message.into()));
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Runs `body`, converting any Rust panic into `NsStatus::InternalError`
/// instead of unwinding across the FFI boundary. Clears the thread-local
/// last-error slot first, so every `ns_*` call except `ns_last_error`
/// itself goes through this (see `guard_readonly` for that one - reading
/// the last error must not erase it before the caller sees it).
fn guard(body: impl FnOnce() -> Result<(), NsError>) -> i32 {
    clear_last_error();
    guard_readonly(body)
}

/// Same panic/error handling as `guard`, without clearing the last-error
/// slot beforehand. Used only by `ns_last_error`.
fn guard_readonly(body: impl FnOnce() -> Result<(), NsError>) -> i32 {
    let result = panic::catch_unwind(AssertUnwindSafe(body));
    match result {
        Ok(Ok(())) => NsStatus::Ok as i32,
        Ok(Err(e)) => {
            set_last_error(e.message.clone());
            e.status as i32
        }
        Err(panic_payload) => {
            let msg = panic_payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "native-search panicked with a non-string payload".to_string());
            set_last_error(format!("internal panic: {msg}"));
            NsStatus::InternalError as i32
        }
    }
}

/// # Safety
/// `ptr` must be a valid, NUL-terminated UTF-8 C string, or null.
unsafe fn cstr_to_str<'a>(ptr: *const c_char, field_name: &str) -> Result<&'a str, NsError> {
    if ptr.is_null() {
        return Err(NsError::invalid_argument(format!(
            "{field_name} must not be null"
        )));
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map_err(|_| NsError::invalid_argument(format!("{field_name} is not valid UTF-8")))
}

fn alloc_buffer(bytes: Vec<u8>) -> (*mut u8, usize) {
    let len = bytes.len();
    let boxed = bytes.into_boxed_slice();
    (Box::into_raw(boxed) as *mut u8, len)
}

/// Opens (or creates) an index at `index_dir` and returns an opaque handle
/// via `out_handle`. The directory must already exist - this layer stays a
/// pure indexing concern, not a filesystem-provisioning one (see
/// `engine::NativeSearchEngine::open_or_create`).
///
/// # Safety
/// `index_dir` must be a valid NUL-terminated UTF-8 C string. `out_handle`
/// must be a valid, writable pointer.
#[no_mangle]
pub unsafe extern "C" fn ns_create(index_dir: *const c_char, out_handle: *mut *mut c_void) -> i32 {
    guard(|| {
        if out_handle.is_null() {
            return Err(NsError::invalid_argument("out_handle must not be null"));
        }
        *out_handle = ptr::null_mut();
        let dir_str = cstr_to_str(index_dir, "index_dir")?;
        let engine = NativeSearchEngine::open_or_create(Path::new(dir_str))?;
        *out_handle = Box::into_raw(Box::new(engine)) as *mut c_void;
        Ok(())
    })
}

/// # Safety
/// `handle` must be a pointer previously returned by `ns_create` and not yet
/// destroyed. Passing null is a documented no-op.
#[no_mangle]
pub unsafe extern "C" fn ns_destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    // Deliberately outside `guard`: dropping a `Box` we already own cannot
    // meaningfully fail, and destroy must be safe to call unconditionally
    // during .NET shutdown/cleanup paths (Section 2's "safe under repeated
    // initialization/shutdown").
    drop(Box::from_raw(handle as *mut NativeSearchEngine));
}

/// # Safety
/// `handle` must be a live handle from `ns_create`. All C-string arguments
/// must be valid NUL-terminated UTF-8 or null. `body` must point to at least
/// `body_len` valid bytes of UTF-8 (or `body_len` may be 0).
#[no_mangle]
pub unsafe extern "C" fn ns_index_document(
    handle: *mut c_void,
    id: *const c_char,
    path: *const c_char,
    filename: *const c_char,
    extension: *const c_char,
    title: *const c_char,
    modified_unix: i64,
    created_unix: i64,
    size: i64,
    body: *const u8,
    body_len: usize,
) -> i32 {
    guard(|| {
        let engine = handle_ref(handle)?;
        let id = cstr_to_str(id, "id")?;
        let path = cstr_to_str(path, "path")?;
        let filename = cstr_to_str(filename, "filename")?;
        let extension = cstr_to_str(extension, "extension")?;
        let title = if title.is_null() {
            ""
        } else {
            cstr_to_str(title, "title")?
        };
        let body_bytes = if body.is_null() || body_len == 0 {
            &[][..]
        } else {
            slice::from_raw_parts(body, body_len)
        };
        let body_str = std::str::from_utf8(body_bytes)
            .map_err(|_| NsError::invalid_argument("body is not valid UTF-8"))?;

        engine.index_document(DocumentInput {
            id,
            path,
            filename,
            extension,
            title,
            modified_unix,
            created_unix,
            size,
            body: body_str,
        })
    })
}

/// # Safety
/// `handle` must be a live handle from `ns_create`. `id` must be a valid
/// NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn ns_delete_document(handle: *mut c_void, id: *const c_char) -> i32 {
    guard(|| {
        let engine = handle_ref(handle)?;
        let id = cstr_to_str(id, "id")?;
        engine.delete_document(id)
    })
}

/// Looks up the `(modified_unix, size)` stored for `id`, so a caller can
/// decide whether re-indexing it is actually necessary (issue #2 - "only
/// re-index if different"). `*out_found` is `0`/`1`; `*out_modified_unix`/
/// `*out_size` are only meaningful when `*out_found == 1`.
///
/// # Safety
/// `handle` must be a live handle from `ns_create`. `id` must be a valid
/// NUL-terminated UTF-8 C string. `out_found`/`out_modified_unix`/`out_size`
/// must be valid, writable pointers.
#[no_mangle]
pub unsafe extern "C" fn ns_get_document_metadata(
    handle: *mut c_void,
    id: *const c_char,
    out_found: *mut i32,
    out_modified_unix: *mut i64,
    out_size: *mut i64,
) -> i32 {
    guard(|| {
        if out_found.is_null() || out_modified_unix.is_null() || out_size.is_null() {
            return Err(NsError::invalid_argument(
                "out_found/out_modified_unix/out_size must not be null",
            ));
        }
        *out_found = 0;
        *out_modified_unix = 0;
        *out_size = 0;
        let engine = handle_ref(handle)?;
        let id_str = cstr_to_str(id, "id")?;
        if let Some((modified, size)) = engine.get_document_metadata(id_str)? {
            *out_found = 1;
            *out_modified_unix = modified;
            *out_size = size;
        }
        Ok(())
    })
}

/// # Safety
/// `handle` must be a live handle from `ns_create`.
#[no_mangle]
pub unsafe extern "C" fn ns_commit(handle: *mut c_void) -> i32 {
    guard(|| handle_ref(handle)?.commit())
}

/// Searches the index and writes a JSON array of hits (see
/// `engine::SearchHit`) into a newly allocated buffer via `out_buffer`/
/// `out_len`. The caller must release it with `ns_free_buffer`.
///
/// `cancel_token` is optional (pass null for no cancellation support) - a
/// live token from `ns_cancel_token_create`, cancelled from another thread
/// via `ns_cancel_token_cancel`, aborts this call with `NsStatus::Cancelled`
/// once the check fires (before the search starts, and before each segment
/// scan - see `engine::CancellableCollector`, not a guarantee of instant
/// mid-scan interruption).
///
/// # Safety
/// `handle` must be a live handle from `ns_create`. `query` must be a valid
/// NUL-terminated UTF-8 C string. `out_buffer`/`out_len` must be valid,
/// writable pointers. `cancel_token`, if non-null, must be a live handle
/// from `ns_cancel_token_create` not yet destroyed.
#[no_mangle]
pub unsafe extern "C" fn ns_search(
    handle: *mut c_void,
    query: *const c_char,
    limit: u32,
    cancel_token: *mut c_void,
    out_buffer: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    guard(|| {
        if out_buffer.is_null() || out_len.is_null() {
            return Err(NsError::invalid_argument(
                "out_buffer/out_len must not be null",
            ));
        }
        *out_buffer = ptr::null_mut();
        *out_len = 0;
        let engine = handle_ref(handle)?;
        let query_str = cstr_to_str(query, "query")?;
        let cancel = cancel_token_ref(cancel_token);
        let hits = engine.search(query_str, limit as usize, cancel)?;
        let json = serde_json::to_vec(&hits)
            .map_err(|e| NsError::index_error(format!("failed to serialize results: {e}")))?;
        let (ptr, len) = alloc_buffer(json);
        *out_buffer = ptr;
        *out_len = len;
        Ok(())
    })
}

/// Creates a cancellation token for `ns_search` (issue #2 Section 17).
/// Independent of any search-engine handle, so it can be created and
/// cancelled from a different thread than the one blocked inside
/// `ns_search`.
///
/// # Safety
/// `out_token` must be a valid, writable pointer.
#[no_mangle]
pub unsafe extern "C" fn ns_cancel_token_create(out_token: *mut *mut c_void) -> i32 {
    guard(|| {
        if out_token.is_null() {
            return Err(NsError::invalid_argument("out_token must not be null"));
        }
        *out_token = Box::into_raw(Box::new(CancellationFlag::new())) as *mut c_void;
        Ok(())
    })
}

/// Signals cancellation. Idempotent - cancelling an already-cancelled or
/// not-yet-used token is not an error.
///
/// # Safety
/// `token` must be a live handle from `ns_cancel_token_create`.
#[no_mangle]
pub unsafe extern "C" fn ns_cancel_token_cancel(token: *mut c_void) -> i32 {
    guard(|| {
        let flag = (token as *mut CancellationFlag)
            .as_ref()
            .ok_or_else(|| NsError::invalid_argument("token must not be null"))?;
        flag.cancel();
        Ok(())
    })
}

/// # Safety
/// `token` must be a pointer previously returned by `ns_cancel_token_create`
/// and not yet destroyed. Passing null is a documented no-op. Do not call
/// this while a `ns_search` call using this token may still be in flight on
/// another thread - the same lifetime discipline as `ns_destroy`/
/// `NativeSearchEngine`, just without a `SafeHandle`-equivalent ref-count on
/// the Rust side for this narrower type.
#[no_mangle]
pub unsafe extern "C" fn ns_cancel_token_destroy(token: *mut c_void) {
    if token.is_null() {
        return;
    }
    drop(Box::from_raw(token as *mut CancellationFlag));
}

/// # Safety
/// `token` must be null or a pointer previously returned by
/// `ns_cancel_token_create`.
unsafe fn cancel_token_ref<'a>(token: *mut c_void) -> Option<&'a CancellationFlag> {
    (token as *mut CancellationFlag).as_ref()
}

/// Copies the last error message set on this thread by a prior `ns_*` call
/// into a newly allocated buffer. `*out_len` is 0 (and `*out_buffer` null)
/// if there was no error. Caller must release a non-empty buffer with
/// `ns_free_buffer`.
///
/// # Safety
/// `out_buffer`/`out_len` must be valid, writable pointers.
#[no_mangle]
pub unsafe extern "C" fn ns_last_error(out_buffer: *mut *mut u8, out_len: *mut usize) -> i32 {
    guard_readonly(|| {
        if out_buffer.is_null() || out_len.is_null() {
            return Err(NsError::invalid_argument(
                "out_buffer/out_len must not be null",
            ));
        }
        *out_buffer = ptr::null_mut();
        *out_len = 0;
        LAST_ERROR.with(|slot| {
            if let Some(message) = slot.borrow().as_ref() {
                let (ptr, len) = alloc_buffer(message.clone().into_bytes());
                *out_buffer = ptr;
                *out_len = len;
            }
        });
        Ok(())
    })
}

/// Releases a buffer previously returned by `ns_search` or `ns_last_error`.
///
/// # Safety
/// `ptr`/`len` must be exactly the pair most recently returned by one of
/// those functions for this allocation, not yet freed. Passing null is a
/// documented no-op.
#[no_mangle]
pub unsafe extern "C" fn ns_free_buffer(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    drop(Box::from_raw(ptr::slice_from_raw_parts_mut(ptr, len)));
}

/// # Safety
/// `handle` must be null or a pointer previously returned by `ns_create`.
unsafe fn handle_ref<'a>(handle: *mut c_void) -> Result<&'a NativeSearchEngine, NsError> {
    (handle as *mut NativeSearchEngine)
        .as_ref()
        .ok_or_else(|| NsError::invalid_argument("handle must not be null"))
}
