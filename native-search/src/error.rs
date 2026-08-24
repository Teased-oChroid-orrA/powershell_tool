//! Error classification for the FFI boundary. Mirrors issue #2 Section 18.
//! A Rust panic must never cross into .NET, so every status here is a value,
//! never an unwind - see `catch_unwind` usage in `ffi.rs`.

use std::fmt;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NsStatus {
    Ok = 0,
    InvalidArgument = 1,
    FileNotFound = 2,
    AccessDenied = 3,
    UnsupportedFormat = 4,
    ExtractionFailed = 5,
    IndexError = 6,
    QueryError = 7,
    OutOfMemory = 8,
    Cancelled = 9,
    CorruptIndex = 10,
    InternalError = 11,
}

#[derive(Debug)]
pub struct NsError {
    pub status: NsStatus,
    pub message: String,
}

impl NsError {
    pub fn new(status: NsStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(NsStatus::InvalidArgument, message)
    }

    pub fn index_error(message: impl Into<String>) -> Self {
        Self::new(NsStatus::IndexError, message)
    }

    pub fn query_error(message: impl Into<String>) -> Self {
        Self::new(NsStatus::QueryError, message)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(NsStatus::Cancelled, message)
    }
}

/// Sentinel message `CancellableCollector` uses to signal a cancelled
/// search through `TantivyError::InvalidArgument` (the only TantivyError
/// variant that carries an arbitrary `String` and isn't otherwise
/// meaningful mid-search) - checked for in `engine::search` to report
/// `NsStatus::Cancelled` instead of a generic `IndexError`. Not a public
/// contract; both ends live in this crate.
pub(crate) const CANCELLED_SENTINEL: &str = "native-search: search cancelled";

impl fmt::Display for NsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.status, self.message)
    }
}

impl std::error::Error for NsError {}

impl From<tantivy::TantivyError> for NsError {
    fn from(e: tantivy::TantivyError) -> Self {
        if let tantivy::TantivyError::InvalidArgument(ref msg) = e {
            if msg == CANCELLED_SENTINEL {
                return NsError::cancelled("search was cancelled");
            }
        }
        NsError::new(NsStatus::IndexError, e.to_string())
    }
}

impl From<tantivy::query::QueryParserError> for NsError {
    fn from(e: tantivy::query::QueryParserError) -> Self {
        NsError::new(NsStatus::QueryError, e.to_string())
    }
}

pub type NsResult<T> = Result<T, NsError>;
