//! Exercises the raw `extern "C"` surface directly (not the safe `engine`
//! API) to prove the ABI itself round-trips correctly - separate from
//! `src/engine.rs`'s unit tests, which cover indexing/search logic through
//! the safe Rust API.

use std::ffi::CString;
use std::os::raw::c_void;
use std::ptr;

use native_search::ffi::*;

struct TestIndex {
    handle: *mut c_void,
    _dir: tempfile::TempDir,
}

impl TestIndex {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let dir_c = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut handle: *mut c_void = ptr::null_mut();
        let status = unsafe { ns_create(dir_c.as_ptr(), &mut handle) };
        assert_eq!(status, 0, "ns_create failed");
        assert!(!handle.is_null());
        Self { handle, _dir: dir }
    }
}

impl Drop for TestIndex {
    fn drop(&mut self) {
        unsafe { ns_destroy(self.handle) };
    }
}

fn last_error_message() -> String {
    let mut buf: *mut u8 = ptr::null_mut();
    let mut len: usize = 0;
    let status = unsafe { ns_last_error(&mut buf, &mut len) };
    assert_eq!(status, 0);
    if buf.is_null() {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(buf, len) }.to_vec();
    unsafe { ns_free_buffer(buf, len) };
    String::from_utf8(bytes).unwrap()
}

#[test]
fn create_index_search_destroy_round_trip() {
    let idx = TestIndex::new();

    let id = CString::new("1").unwrap();
    let path = CString::new("C:\\docs\\report.txt").unwrap();
    let filename = CString::new("report.txt").unwrap();
    let extension = CString::new(".txt").unwrap();
    let title = CString::new("").unwrap();
    let body = b"torque spec deviation on aft mount bolts";

    let status = unsafe {
        ns_index_document(
            idx.handle,
            id.as_ptr(),
            path.as_ptr(),
            filename.as_ptr(),
            extension.as_ptr(),
            title.as_ptr(),
            1_700_000_000,
            1_600_000_000,
            42,
            body.as_ptr(),
            body.len(),
        )
    };
    assert_eq!(status, 0);

    let status = unsafe { ns_commit(idx.handle) };
    assert_eq!(status, 0);

    let query = CString::new("torque").unwrap();
    let mut out_buf: *mut u8 = ptr::null_mut();
    let mut out_len: usize = 0;
    let status = unsafe {
        ns_search(
            idx.handle,
            query.as_ptr(),
            10,
            ptr::null_mut(),
            &mut out_buf,
            &mut out_len,
        )
    };
    assert_eq!(status, 0);
    assert!(!out_buf.is_null());

    let json_bytes = unsafe { std::slice::from_raw_parts(out_buf, out_len) };
    let json: serde_json::Value = serde_json::from_slice(json_bytes).unwrap();
    unsafe { ns_free_buffer(out_buf, out_len) };

    let hits = json.as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["id"], "1");
    assert_eq!(hits[0]["path"], "C:\\docs\\report.txt");
}

#[test]
fn null_handle_returns_invalid_argument_not_crash() {
    let query = CString::new("anything").unwrap();
    let mut out_buf: *mut u8 = ptr::null_mut();
    let mut out_len: usize = 0;
    let status = unsafe {
        ns_search(
            ptr::null_mut(),
            query.as_ptr(),
            10,
            ptr::null_mut(),
            &mut out_buf,
            &mut out_len,
        )
    };
    assert_eq!(status, 1 /* InvalidArgument */);
    assert!(out_buf.is_null());
    assert_eq!(last_error_message(), "handle must not be null");
}

#[test]
fn null_index_dir_returns_invalid_argument_not_crash() {
    let mut handle: *mut c_void = ptr::null_mut();
    let status = unsafe { ns_create(ptr::null(), &mut handle) };
    assert_eq!(status, 1 /* InvalidArgument */);
    assert!(handle.is_null());
}

#[test]
fn destroy_null_handle_is_a_documented_noop() {
    unsafe { ns_destroy(ptr::null_mut()) };
}

#[test]
fn invalid_utf8_body_is_rejected_not_crash() {
    let idx = TestIndex::new();
    let id = CString::new("1").unwrap();
    let path = CString::new("p").unwrap();
    let filename = CString::new("f").unwrap();
    let extension = CString::new("e").unwrap();
    let title = CString::new("").unwrap();
    let bad_utf8: [u8; 2] = [0xff, 0xfe];

    let status = unsafe {
        ns_index_document(
            idx.handle,
            id.as_ptr(),
            path.as_ptr(),
            filename.as_ptr(),
            extension.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            bad_utf8.as_ptr(),
            bad_utf8.len(),
        )
    };
    assert_eq!(status, 1 /* InvalidArgument */);
}

#[test]
fn cancelled_before_search_starts_returns_cancelled_status() {
    let idx = TestIndex::new();
    let id = CString::new("1").unwrap();
    let path = CString::new("p").unwrap();
    let filename = CString::new("f").unwrap();
    let extension = CString::new("e").unwrap();
    let title = CString::new("").unwrap();
    let body = b"findable text";
    unsafe {
        ns_index_document(
            idx.handle,
            id.as_ptr(),
            path.as_ptr(),
            filename.as_ptr(),
            extension.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            body.as_ptr(),
            body.len(),
        )
    };
    unsafe { ns_commit(idx.handle) };

    let mut token: *mut c_void = ptr::null_mut();
    let status = unsafe { ns_cancel_token_create(&mut token) };
    assert_eq!(status, 0);
    assert!(!token.is_null());

    let status = unsafe { ns_cancel_token_cancel(token) };
    assert_eq!(status, 0);

    let query = CString::new("findable").unwrap();
    let mut out_buf: *mut u8 = ptr::null_mut();
    let mut out_len: usize = 0;
    let status = unsafe {
        ns_search(
            idx.handle,
            query.as_ptr(),
            10,
            token,
            &mut out_buf,
            &mut out_len,
        )
    };
    assert_eq!(status, 9 /* Cancelled */);
    assert!(out_buf.is_null());

    unsafe { ns_cancel_token_destroy(token) };
}

#[test]
fn uncancelled_token_does_not_block_search() {
    let idx = TestIndex::new();
    let id = CString::new("1").unwrap();
    let path = CString::new("p").unwrap();
    let filename = CString::new("f").unwrap();
    let extension = CString::new("e").unwrap();
    let title = CString::new("").unwrap();
    let body = b"findable text";
    unsafe {
        ns_index_document(
            idx.handle,
            id.as_ptr(),
            path.as_ptr(),
            filename.as_ptr(),
            extension.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            body.as_ptr(),
            body.len(),
        )
    };
    unsafe { ns_commit(idx.handle) };

    let mut token: *mut c_void = ptr::null_mut();
    unsafe { ns_cancel_token_create(&mut token) };

    let query = CString::new("findable").unwrap();
    let mut out_buf: *mut u8 = ptr::null_mut();
    let mut out_len: usize = 0;
    let status = unsafe {
        ns_search(
            idx.handle,
            query.as_ptr(),
            10,
            token,
            &mut out_buf,
            &mut out_len,
        )
    };
    assert_eq!(status, 0);
    assert!(!out_buf.is_null());
    unsafe { ns_free_buffer(out_buf, out_len) };
    unsafe { ns_cancel_token_destroy(token) };
}

#[test]
fn cancel_token_null_create_out_param_is_invalid_argument() {
    let status = unsafe { ns_cancel_token_create(ptr::null_mut()) };
    assert_eq!(status, 1 /* InvalidArgument */);
}

#[test]
fn cancel_token_null_cancel_is_invalid_argument_not_crash() {
    let status = unsafe { ns_cancel_token_cancel(ptr::null_mut()) };
    assert_eq!(status, 1 /* InvalidArgument */);
}

#[test]
fn cancel_token_destroy_null_is_a_documented_noop() {
    unsafe { ns_cancel_token_destroy(ptr::null_mut()) };
}

#[test]
fn get_document_metadata_round_trips_through_the_raw_abi() {
    let idx = TestIndex::new();
    let id = CString::new("1").unwrap();
    let path = CString::new("p").unwrap();
    let filename = CString::new("f").unwrap();
    let extension = CString::new("e").unwrap();
    let title = CString::new("").unwrap();
    let body = b"some body";

    unsafe {
        ns_index_document(
            idx.handle,
            id.as_ptr(),
            path.as_ptr(),
            filename.as_ptr(),
            extension.as_ptr(),
            title.as_ptr(),
            1_700_000_123,
            1_600_000_000,
            9999,
            body.as_ptr(),
            body.len(),
        )
    };
    unsafe { ns_commit(idx.handle) };

    let mut found: i32 = -1;
    let mut modified_unix: i64 = 0;
    let mut size: i64 = 0;
    let status = unsafe {
        ns_get_document_metadata(
            idx.handle,
            id.as_ptr(),
            &mut found,
            &mut modified_unix,
            &mut size,
        )
    };
    assert_eq!(status, 0);
    assert_eq!(found, 1);
    assert_eq!(modified_unix, 1_700_000_123);
    assert_eq!(size, 9999);
}

#[test]
fn get_document_metadata_unknown_id_reports_not_found_not_an_error() {
    let idx = TestIndex::new();
    let id = CString::new("nope").unwrap();
    let mut found: i32 = -1;
    let mut modified_unix: i64 = 0;
    let mut size: i64 = 0;
    let status = unsafe {
        ns_get_document_metadata(
            idx.handle,
            id.as_ptr(),
            &mut found,
            &mut modified_unix,
            &mut size,
        )
    };
    assert_eq!(status, 0);
    assert_eq!(found, 0);
    assert_eq!(modified_unix, 0);
    assert_eq!(size, 0);
}

#[test]
fn get_document_metadata_null_handle_is_invalid_argument_not_crash() {
    let id = CString::new("1").unwrap();
    let mut found: i32 = -1;
    let mut modified_unix: i64 = 0;
    let mut size: i64 = 0;
    let status = unsafe {
        ns_get_document_metadata(
            ptr::null_mut(),
            id.as_ptr(),
            &mut found,
            &mut modified_unix,
            &mut size,
        )
    };
    assert_eq!(status, 1 /* InvalidArgument */);
}
