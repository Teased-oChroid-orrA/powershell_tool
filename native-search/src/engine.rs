//! Safe Rust core: schema, indexing, and search. No `unsafe`, no FFI types -
//! testable on its own (see `tests/engine.rs`), matching issue #2 Section 2's
//! "testable independently from the .NET UI" requirement.
//!
//! Scope is indexing/search only (ADR-001) - callers hand over text .NET has
//! already extracted; this module never reads a file from disk itself.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tantivy::collector::{Collector, SegmentCollector, TopDocs};
use tantivy::directory::MmapDirectory;
use tantivy::query::QueryParser;
use tantivy::schema::{Schema, Value, FAST, INDEXED, STORED, STRING, TEXT};
use tantivy::{
    doc, Index, IndexReader, IndexWriter, ReloadPolicy, SegmentOrdinal, SegmentReader,
    TantivyDocument, TantivyError, Term,
};

use crate::error::{NsError, NsResult, CANCELLED_SENTINEL};

/// A cheaply-cloneable flag a caller can hand to
/// [`NativeSearchEngine::search`] and set from another thread to abort an
/// in-flight search (issue #2 Section 17). Checked before the search starts
/// and again before each segment is scanned (`CancellableCollector`) - a
/// large single-segment index can't be interrupted mid-scan (Tantivy's
/// `SegmentCollector::collect` has no early-exit hook), but this is real,
/// working, best-effort cancellation for the common multi-segment case, not
/// a no-op placeholder.
#[derive(Clone, Default)]
pub struct CancellationFlag(Arc<AtomicBool>);

impl CancellationFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Wraps any `Collector` so `for_segment` bails out with a distinguishable
/// error once `flag` is set, aborting the search before scanning the next
/// segment. See `error::CANCELLED_SENTINEL` for how `engine::search`
/// recognizes this specific error and reports `NsStatus::Cancelled` rather
/// than a generic index error.
struct CancellableCollector<C> {
    inner: C,
    flag: CancellationFlag,
}

impl<C: Collector> Collector for CancellableCollector<C> {
    type Fruit = C::Fruit;
    type Child = C::Child;

    fn for_segment(
        &self,
        segment_local_id: SegmentOrdinal,
        segment: &SegmentReader,
    ) -> tantivy::Result<Self::Child> {
        if self.flag.is_cancelled() {
            return Err(TantivyError::InvalidArgument(
                CANCELLED_SENTINEL.to_string(),
            ));
        }
        self.inner.for_segment(segment_local_id, segment)
    }

    fn requires_scoring(&self) -> bool {
        self.inner.requires_scoring()
    }

    fn merge_fruits(
        &self,
        segment_fruits: Vec<<Self::Child as SegmentCollector>::Fruit>,
    ) -> tantivy::Result<Self::Fruit> {
        self.inner.merge_fruits(segment_fruits)
    }
}

/// A document to index. All fields are already-extracted text/metadata from
/// the .NET side (ADR-001/ADR-003) - this module does no file I/O of its own.
pub struct DocumentInput<'a> {
    pub id: &'a str,
    pub path: &'a str,
    pub filename: &'a str,
    pub extension: &'a str,
    pub title: &'a str,
    pub modified_unix: i64,
    pub created_unix: i64,
    pub size: i64,
    pub body: &'a str,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub id: String,
    pub path: String,
    pub filename: String,
    pub extension: String,
    pub title: String,
    pub modified_unix: i64,
    pub created_unix: i64,
    pub size: i64,
    pub score: f32,
}

struct Fields {
    id: tantivy::schema::Field,
    path: tantivy::schema::Field,
    filename: tantivy::schema::Field,
    extension: tantivy::schema::Field,
    title: tantivy::schema::Field,
    modified: tantivy::schema::Field,
    created: tantivy::schema::Field,
    size: tantivy::schema::Field,
    body: tantivy::schema::Field,
}

fn build_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let id = builder.add_text_field("id", STRING | STORED);
    let path = builder.add_text_field("path", STRING | STORED);
    let filename = builder.add_text_field("filename", TEXT | STORED);
    let extension = builder.add_text_field("extension", STRING | STORED);
    let title = builder.add_text_field("title", TEXT | STORED);
    let modified = builder.add_i64_field("modified", INDEXED | STORED | FAST);
    let created = builder.add_i64_field("created", INDEXED | STORED | FAST);
    let size = builder.add_i64_field("size", INDEXED | STORED | FAST);
    // Not STORED: the extracted body already lives on the .NET side
    // (FileSearchResult.LinesCache) - duplicating it here would double the
    // index's on-disk footprint for no reader who needs it back out.
    let body = builder.add_text_field("body", TEXT);
    let schema = builder.build();
    (
        schema,
        Fields {
            id,
            path,
            filename,
            extension,
            title,
            modified,
            created,
            size,
            body,
        },
    )
}

pub struct NativeSearchEngine {
    index: Index,
    fields: Fields,
    writer: Mutex<IndexWriter>,
    reader: IndexReader,
}

impl NativeSearchEngine {
    /// Opens an existing index at `index_dir` if one exists, else creates a
    /// new one. `index_dir` must already exist and be writable - creating
    /// the directory itself is the caller's responsibility (kept out of this
    /// layer so it stays a pure indexing/search concern, not a filesystem
    /// policy one).
    pub fn open_or_create(index_dir: &Path) -> NsResult<Self> {
        let (schema, fields) = build_schema();
        let dir = MmapDirectory::open(index_dir)
            .map_err(|e| NsError::index_error(format!("cannot open directory: {e}")))?;

        let index = if Index::exists(&dir)
            .map_err(|e| NsError::index_error(format!("cannot probe index: {e}")))?
        {
            Index::open(dir).map_err(|e| NsError::index_error(format!("cannot open index: {e}")))?
        } else {
            Index::create(dir, schema.clone(), tantivy::IndexSettings::default())
                .map_err(|e| NsError::index_error(format!("cannot create index: {e}")))?
        };

        // 50 MB indexing buffer is tantivy's own documented minimum-viable
        // default for a single-threaded writer; revisit once Section 13
        // benchmarking exists.
        let writer: IndexWriter = index
            .writer(50_000_000)
            .map_err(|e| NsError::index_error(format!("cannot create writer: {e}")))?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e: tantivy::TantivyError| NsError::index_error(e.to_string()))?;

        Ok(Self {
            index,
            fields,
            writer: Mutex::new(writer),
            reader,
        })
    }

    /// Indexes (or re-indexes) one document. Tantivy documents are immutable
    /// (ADR-002 item 6) - this deletes any existing document with the same
    /// `id` first, so calling this twice for the same file is a safe update,
    /// not a duplicate.
    pub fn index_document(&self, doc: DocumentInput<'_>) -> NsResult<()> {
        if doc.id.is_empty() {
            return Err(NsError::invalid_argument("document id must not be empty"));
        }
        let writer = self.writer.lock().expect("index writer mutex poisoned");
        writer.delete_term(Term::from_field_text(self.fields.id, doc.id));
        writer
            .add_document(doc!(
                self.fields.id => doc.id,
                self.fields.path => doc.path,
                self.fields.filename => doc.filename,
                self.fields.extension => doc.extension,
                self.fields.title => doc.title,
                self.fields.modified => doc.modified_unix,
                self.fields.created => doc.created_unix,
                self.fields.size => doc.size,
                self.fields.body => doc.body,
            ))
            .map_err(|e| NsError::index_error(e.to_string()))?;
        Ok(())
    }

    pub fn delete_document(&self, id: &str) -> NsResult<()> {
        if id.is_empty() {
            return Err(NsError::invalid_argument("document id must not be empty"));
        }
        let writer = self.writer.lock().expect("index writer mutex poisoned");
        writer.delete_term(Term::from_field_text(self.fields.id, id));
        Ok(())
    }

    /// Commits pending changes and reloads the reader so `search` sees them
    /// immediately. Reload is explicit (not the background policy) so tests
    /// and callers get deterministic read-your-writes behavior rather than
    /// racing a reload timer - see the ViewModel test-flakiness lesson
    /// already learned once in this repo (commit 96f00df).
    pub fn commit(&self) -> NsResult<()> {
        {
            let mut writer = self.writer.lock().expect("index writer mutex poisoned");
            writer
                .commit()
                .map_err(|e| NsError::index_error(e.to_string()))?;
        }
        self.reader
            .reload()
            .map_err(|e| NsError::index_error(e.to_string()))?;
        Ok(())
    }

    pub fn search(
        &self,
        query_text: &str,
        limit: usize,
        cancel: Option<&CancellationFlag>,
    ) -> NsResult<Vec<SearchHit>> {
        if query_text.trim().is_empty() {
            return Err(NsError::invalid_argument("query must not be empty"));
        }
        if let Some(flag) = cancel {
            if flag.is_cancelled() {
                return Err(NsError::cancelled("search was cancelled before it started"));
            }
        }
        let searcher = self.reader.searcher();
        let parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.filename, self.fields.title, self.fields.body],
        );
        let query = parser.parse_query(query_text)?;
        let base_collector = TopDocs::with_limit(limit).order_by_score();
        // `?` here (not a manual map_err) so `NsError::from(TantivyError)`
        // gets a chance to recognize CANCELLED_SENTINEL and report
        // NsStatus::Cancelled instead of a generic index error.
        let top_docs = match cancel {
            Some(flag) => searcher.search(
                &query,
                &CancellableCollector {
                    inner: base_collector,
                    flag: flag.clone(),
                },
            )?,
            None => searcher.search(&query, &base_collector)?,
        };

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, addr) in top_docs {
            let retrieved: TantivyDocument = searcher
                .doc(addr)
                .map_err(|e| NsError::index_error(e.to_string()))?;
            hits.push(SearchHit {
                id: text_value(&retrieved, self.fields.id),
                path: text_value(&retrieved, self.fields.path),
                filename: text_value(&retrieved, self.fields.filename),
                extension: text_value(&retrieved, self.fields.extension),
                title: text_value(&retrieved, self.fields.title),
                modified_unix: int_value(&retrieved, self.fields.modified),
                created_unix: int_value(&retrieved, self.fields.created),
                size: int_value(&retrieved, self.fields.size),
                score,
            });
        }
        Ok(hits)
    }

    pub fn num_docs(&self) -> u64 {
        self.reader.searcher().num_docs()
    }
}

fn text_value(doc: &TantivyDocument, field: tantivy::schema::Field) -> String {
    doc.get_first(field)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn int_value(doc: &TantivyDocument, field: tantivy::schema::Field) -> i64 {
    doc.get_first(field)
        .and_then(|v| v.as_i64())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample<'a>(id: &'a str, body: &'a str) -> DocumentInput<'a> {
        DocumentInput {
            id,
            path: "C:\\docs\\a.txt",
            filename: "a.txt",
            extension: ".txt",
            title: "",
            modified_unix: 1_700_000_000,
            created_unix: 1_600_000_000,
            size: 42,
            body,
        }
    }

    #[test]
    fn index_and_search_round_trip() {
        let dir = tempdir().unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        engine
            .index_document(sample("1", "torque spec deviation on engine mount"))
            .unwrap();
        engine
            .index_document(sample("2", "unrelated corrosion inspection notes"))
            .unwrap();
        engine.commit().unwrap();

        let hits = engine.search("torque", 10, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "1");
    }

    #[test]
    fn reindexing_same_id_replaces_not_duplicates() {
        let dir = tempdir().unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        engine
            .index_document(sample("1", "original body text"))
            .unwrap();
        engine.commit().unwrap();
        engine
            .index_document(sample("1", "updated body text"))
            .unwrap();
        engine.commit().unwrap();

        assert_eq!(engine.num_docs(), 1);
        let hits = engine.search("updated", 10, None).unwrap();
        assert_eq!(hits.len(), 1);
        let stale = engine.search("original", 10, None).unwrap();
        assert_eq!(stale.len(), 0);
    }

    #[test]
    fn delete_removes_document() {
        let dir = tempdir().unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        engine.index_document(sample("1", "findable text")).unwrap();
        engine.commit().unwrap();
        assert_eq!(engine.search("findable", 10, None).unwrap().len(), 1);

        engine.delete_document("1").unwrap();
        engine.commit().unwrap();
        assert_eq!(engine.search("findable", 10, None).unwrap().len(), 0);
    }

    #[test]
    fn reopening_existing_index_preserves_documents() {
        let dir = tempdir().unwrap();
        {
            let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
            engine
                .index_document(sample("1", "persisted across reopen"))
                .unwrap();
            engine.commit().unwrap();
        }
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        let hits = engine.search("persisted", 10, None).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn empty_query_is_invalid_argument_not_panic() {
        let dir = tempdir().unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        let err = engine.search("", 10, None).unwrap_err();
        assert_eq!(err.status, crate::error::NsStatus::InvalidArgument);
    }

    #[test]
    fn malformed_query_syntax_is_query_error_not_panic() {
        let dir = tempdir().unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        engine.index_document(sample("1", "some body")).unwrap();
        engine.commit().unwrap();
        let err = engine.search("title:(unclosed", 10, None).unwrap_err();
        assert_eq!(err.status, crate::error::NsStatus::QueryError);
    }

    #[test]
    fn empty_id_is_invalid_argument() {
        let dir = tempdir().unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        let err = engine.index_document(sample("", "body")).unwrap_err();
        assert_eq!(err.status, crate::error::NsStatus::InvalidArgument);
    }

    #[test]
    fn search_cancelled_before_it_starts_reports_cancelled_not_index_error() {
        let dir = tempdir().unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        engine.index_document(sample("1", "findable text")).unwrap();
        engine.commit().unwrap();

        let cancel = CancellationFlag::new();
        cancel.cancel();
        let err = engine.search("findable", 10, Some(&cancel)).unwrap_err();
        assert_eq!(err.status, crate::error::NsStatus::Cancelled);
    }

    #[test]
    fn uncancelled_flag_does_not_affect_search() {
        let dir = tempdir().unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        engine.index_document(sample("1", "findable text")).unwrap();
        engine.commit().unwrap();

        let cancel = CancellationFlag::new();
        assert!(!cancel.is_cancelled());
        let hits = engine.search("findable", 10, Some(&cancel)).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn cancelling_after_a_successful_search_does_not_retroactively_fail_it() {
        let dir = tempdir().unwrap();
        let engine = NativeSearchEngine::open_or_create(dir.path()).unwrap();
        engine.index_document(sample("1", "findable text")).unwrap();
        engine.commit().unwrap();

        let cancel = CancellationFlag::new();
        let hits = engine.search("findable", 10, Some(&cancel)).unwrap();
        assert_eq!(hits.len(), 1);
        cancel.cancel();
        // A second, independent search using the now-cancelled flag correctly fails -
        // the flag isn't a one-shot "did anything ever get cancelled" latch tied to the
        // engine, it just reflects the caller's own token state at call time.
        let err = engine.search("findable", 10, Some(&cancel)).unwrap_err();
        assert_eq!(err.status, crate::error::NsStatus::Cancelled);
    }
}
