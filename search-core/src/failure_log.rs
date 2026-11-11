//! Persistent extraction-failure log (issue #6 §12/§16/§17/§50) -
//! SQLite-backed, deliberately scoped narrower than "replace `cache.rs`
//! with SQLite" or "duplicate what Tantivy's document metadata already
//! tracks":
//!
//! - `cache.rs`'s JSON `CacheFile` already does incremental result
//!   caching, fingerprinted by the *settings* that affect matching
//!   (different filters/match-mode need different cached results) -
//!   working, tested, a genuinely different concern from a corpus-wide
//!   metadata store. Not touched here.
//! - `native_index`/Tantivy's own stored fields (`id`/`path`/`modified`/
//!   `size`) already function as a metadata store for the *indexed*
//!   corpus, with `get_document_metadata` already serving skip-if-
//!   unchanged. Duplicating that into a parallel SQLite table would be
//!   exactly the redundancy epic §12 itself warns against ("do not
//!   duplicate large text unnecessarily between SQLite and the search
//!   index" - the same principle extends to metadata that's already
//!   tracked somewhere real).
//!
//! What neither of those covers, and what this module adds: a
//! **persistent, queryable record of files that failed extraction** -
//! epic §16 ("avoid repeatedly attempting the same corrupt/unreadable
//! file on every startup"), §17 ("failed extraction state should be
//! persisted and visible to the user"), §50 ("inspect failed
//! documents"). Today a `ReadError`/extraction failure only exists
//! transiently inside one run's `SearchRunResult` - never persisted,
//! never browsable, and a permanently-corrupt file gets a full read +
//! extraction attempt on every single run forever.
//!
//! Deliberately narrow: only genuine *extraction* failures (a format's
//! extractor ran and produced nothing usable - `ExtractLinesError::
//! Failed`) are recorded as "known failures" worth skipping on an
//! unchanged fingerprint. Transient failures (a locked file, a timeout)
//! are NOT recorded here - those are worth retrying every run regardless
//! of fingerprint, unlike a genuinely malformed file.

use std::sync::Mutex;

use rusqlite::Connection;

/// Bumped when extraction logic changes enough that a previously-recorded
/// failure might now succeed - a record made under an older version is
/// ignored by `known_failure_reason` (epic §15's "extractor versioning"),
/// so a parser fix doesn't silently leave affected files skipped forever.
const EXTRACTOR_VERSION: i32 = 1;

#[derive(Debug, Clone, serde::Serialize)]
pub struct FailureRecord {
    pub path: String,
    pub size: i64,
    pub modified_unix: i64,
    pub status: String,
    pub reason: String,
    pub failed_at_unix: i64,
}

pub struct FailureLog {
    conn: Mutex<Connection>,
    /// Extraction settings the failures are recorded under (see
    /// [`extraction_context`]). A failure only counts as known when the
    /// current context matches, so enabling OCR or raising the PDF
    /// timeout retries files that failed under the old setting.
    context: i64,
}

/// Folds the settings that can turn an extraction failure into a success
/// (OCR on/off, PDF time budget) into one number stored beside each
/// failure. `0` is the default no-OCR / 15 s context, matching rows that
/// predate this column.
pub fn extraction_context(settings: &crate::models::SearchSettings) -> i64 {
    let timeout = i64::from(settings.pdf_timeout_seconds.max(0)) - 15;
    (timeout << 1) | i64::from(settings.ocr_scanned_pdfs)
}

impl FailureLog {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        Self::open_with_context(path, 0)
    }

    /// Opens the log for one run's extraction settings; see
    /// [`extraction_context`].
    pub fn open_with_context(path: &str, context: i64) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS extraction_failures (
                path TEXT PRIMARY KEY,
                size INTEGER NOT NULL,
                modified_unix INTEGER NOT NULL,
                status TEXT NOT NULL,
                reason TEXT NOT NULL,
                failed_at_unix INTEGER NOT NULL,
                extractor_version INTEGER NOT NULL
            );",
        )?;
        // Databases written before the context column existed: add it (rows default to 0).
        let has_context = conn
            .prepare("SELECT 1 FROM pragma_table_info('extraction_failures') WHERE name = 'context'")?
            .exists([])?;
        if !has_context {
            conn.execute_batch("ALTER TABLE extraction_failures ADD COLUMN context INTEGER NOT NULL DEFAULT 0;")?;
        }
        Ok(Self { conn: Mutex::new(conn), context })
    }

    /// Records (or overwrites) a failure for `path` - a file's failure
    /// state is always about its *current* content, not a history of
    /// every past attempt, so a re-recorded failure replaces the old row
    /// rather than appending a new one.
    pub fn record_failure(&self, path: &str, size: i64, modified_unix: i64, status: &str, reason: &str, now_unix: i64) {
        let Ok(conn) = self.conn.lock() else { return };
        let _ = conn.execute(
            "INSERT INTO extraction_failures (path, size, modified_unix, status, reason, failed_at_unix, extractor_version, context)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(path) DO UPDATE SET
                size = excluded.size,
                modified_unix = excluded.modified_unix,
                status = excluded.status,
                reason = excluded.reason,
                failed_at_unix = excluded.failed_at_unix,
                extractor_version = excluded.extractor_version,
                context = excluded.context",
            rusqlite::params![path, size, modified_unix, status, reason, now_unix, EXTRACTOR_VERSION, self.context],
        );
    }

    /// Clears any recorded failure for `path` - called once a
    /// previously-failing file extracts successfully (fixed, or the
    /// earlier failure was a fluke).
    pub fn clear_failure(&self, path: &str) {
        let Ok(conn) = self.conn.lock() else { return };
        let _ = conn.execute("DELETE FROM extraction_failures WHERE path = ?1", rusqlite::params![path]);
    }

    /// `Some(reason)` when `path` is a known failure whose recorded
    /// size/modified time still match (the file hasn't changed since)
    /// and whose record was made under the current extractor version -
    /// callers use this to skip a doomed read+extraction attempt
    /// entirely. `None` for a genuinely new/changed/never-failed file,
    /// or transparently on any DB error (never blocks a real attempt).
    pub fn known_failure_reason(&self, path: &str, size: i64, modified_unix: i64) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT reason FROM extraction_failures WHERE path = ?1 AND size = ?2 AND modified_unix = ?3 AND extractor_version = ?4 AND context = ?5",
            rusqlite::params![path, size, modified_unix, EXTRACTOR_VERSION, self.context],
            |row| row.get(0),
        )
        .ok()
    }

    /// Every currently-recorded failure, most recent first - epic §50's
    /// "inspect failed documents."
    pub fn list_failures(&self) -> Vec<FailureRecord> {
        let Ok(conn) = self.conn.lock() else { return Vec::new() };
        let Ok(mut stmt) = conn.prepare(
            "SELECT path, size, modified_unix, status, reason, failed_at_unix \
             FROM extraction_failures ORDER BY failed_at_unix DESC",
        ) else {
            return Vec::new();
        };
        let rows = stmt.query_map([], |row| {
            Ok(FailureRecord {
                path: row.get(0)?,
                size: row.get(1)?,
                modified_unix: row.get(2)?,
                status: row.get(3)?,
                reason: row.get(4)?,
                failed_at_unix: row.get(5)?,
            })
        });
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_finds_a_matching_failure() {
        let dir = tempfile::tempdir().unwrap();
        let log = FailureLog::open(dir.path().join("failures.db").to_str().unwrap()).unwrap();

        assert!(log.known_failure_reason("/x/a.pdf", 100, 1_700_000_000).is_none());

        log.record_failure("/x/a.pdf", 100, 1_700_000_000, "ReadError", "malformed PDF stream", 1_700_000_100);
        assert_eq!(
            log.known_failure_reason("/x/a.pdf", 100, 1_700_000_000).as_deref(),
            Some("malformed PDF stream")
        );
    }

    #[test]
    fn a_changed_file_is_not_a_known_failure_even_with_the_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let log = FailureLog::open(dir.path().join("failures.db").to_str().unwrap()).unwrap();
        log.record_failure("/x/a.pdf", 100, 1_700_000_000, "ReadError", "malformed PDF stream", 1_700_000_100);

        // Same path, different size (file was edited/replaced) - must not
        // be treated as the same known failure.
        assert!(log.known_failure_reason("/x/a.pdf", 200, 1_700_000_000).is_none());
        // Same path, different modified time.
        assert!(log.known_failure_reason("/x/a.pdf", 100, 1_700_000_999).is_none());
    }

    #[test]
    fn clearing_a_failure_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let log = FailureLog::open(dir.path().join("failures.db").to_str().unwrap()).unwrap();
        log.record_failure("/x/a.pdf", 100, 1_700_000_000, "ReadError", "malformed PDF stream", 1_700_000_100);
        assert!(log.known_failure_reason("/x/a.pdf", 100, 1_700_000_000).is_some());

        log.clear_failure("/x/a.pdf");
        assert!(log.known_failure_reason("/x/a.pdf", 100, 1_700_000_000).is_none());
    }

    #[test]
    fn re_recording_a_failure_overwrites_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let log = FailureLog::open(dir.path().join("failures.db").to_str().unwrap()).unwrap();
        log.record_failure("/x/a.pdf", 100, 1_700_000_000, "ReadError", "first reason", 1_700_000_100);
        log.record_failure("/x/a.pdf", 150, 1_700_000_050, "ReadError", "second reason", 1_700_000_200);

        assert_eq!(log.list_failures().len(), 1, "must overwrite, not append a second row");
        assert_eq!(
            log.known_failure_reason("/x/a.pdf", 150, 1_700_000_050).as_deref(),
            Some("second reason")
        );
    }

    #[test]
    fn list_failures_returns_everything_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let log = FailureLog::open(dir.path().join("failures.db").to_str().unwrap()).unwrap();
        log.record_failure("/x/a.pdf", 100, 1_700_000_000, "ReadError", "reason a", 1_700_000_100);
        log.record_failure("/x/b.docx", 200, 1_700_000_010, "ReadError", "reason b", 1_700_000_110);

        let failures = log.list_failures();
        assert_eq!(failures.len(), 2);
        assert!(failures.iter().any(|f| f.path == "/x/a.pdf" && f.reason == "reason a"));
        assert!(failures.iter().any(|f| f.path == "/x/b.docx" && f.reason == "reason b"));
    }

    #[test]
    fn reopening_the_same_database_file_preserves_records() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("failures.db");
        {
            let log = FailureLog::open(db_path.to_str().unwrap()).unwrap();
            log.record_failure("/x/a.pdf", 100, 1_700_000_000, "ReadError", "malformed PDF stream", 1_700_000_100);
        }
        let reopened = FailureLog::open(db_path.to_str().unwrap()).unwrap();
        assert!(reopened.known_failure_reason("/x/a.pdf", 100, 1_700_000_000).is_some());
    }

    #[test]
    fn a_failure_is_not_sticky_across_an_ocr_or_timeout_change() {
        use crate::models::SearchSettings;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("failures.db");
        let db = db.to_str().unwrap();
        let off = SearchSettings::default();
        let on = SearchSettings { ocr_scanned_pdfs: true, ..Default::default() };
        let slow = SearchSettings { pdf_timeout_seconds: 60, ..Default::default() };

        let log = FailureLog::open_with_context(db, extraction_context(&off)).unwrap();
        log.record_failure("/x/scan.pdf", 100, 1, "ReadError", "no text", 2);
        assert!(log.known_failure_reason("/x/scan.pdf", 100, 1).is_some());

        for changed in [&on, &slow] {
            let log = FailureLog::open_with_context(db, extraction_context(changed)).unwrap();
            assert!(log.known_failure_reason("/x/scan.pdf", 100, 1).is_none());
        }
        let log = FailureLog::open_with_context(db, extraction_context(&off)).unwrap();
        assert!(log.known_failure_reason("/x/scan.pdf", 100, 1).is_some());
    }

    #[test]
    fn a_database_without_the_context_column_is_upgraded_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("old.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE extraction_failures (path TEXT PRIMARY KEY, size INTEGER NOT NULL, modified_unix INTEGER NOT NULL,
             status TEXT NOT NULL, reason TEXT NOT NULL, failed_at_unix INTEGER NOT NULL, extractor_version INTEGER NOT NULL);
             INSERT INTO extraction_failures VALUES ('/x/a.pdf', 1, 1, 'ReadError', 'old', 0, 1);",
        )
        .unwrap();
        drop(conn);
        let log = FailureLog::open(db.to_str().unwrap()).unwrap();
        assert_eq!(log.known_failure_reason("/x/a.pdf", 1, 1).as_deref(), Some("old"));
    }
}
