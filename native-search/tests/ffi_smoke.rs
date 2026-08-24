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
    let status = unsafe { ns_search(idx.handle, query.as_ptr(), 10, &mut out_buf, &mut out_len) };
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
