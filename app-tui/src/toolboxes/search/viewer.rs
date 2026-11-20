//! Carries out `Effect::OpenViewer`: opens the viewer tab (`search_core::viewer`) for one result file.
//!
//! Embedded and PDF files get a self-contained page written under the temp folder (one stable name per file) that the
//! default browser opens. Large plain text goes through the process-wide [`ReaderServer`] and the browser opens its
//! `127.0.0.1` URL. Anything the viewer cannot show, and any failure, opens the file in its default app instead, so
//! Open never does less than it did before the viewer existed.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use search_core::models::{FileSearchResult, SearchSettings};
use search_core::viewer::reader::{LargeView, ReaderServer};
use search_core::viewer::{self, Plan};

/// The one reader for the life of the process, started on first use (`None`: the loopback socket could not be bound).
fn reader() -> Option<&'static ReaderServer> {
    static READER: OnceLock<Option<ReaderServer>> = OnceLock::new();
    READER
        .get_or_init(|| {
            let server = ReaderServer::start().ok()?;
            // The viewer menu's "Open the file itself" asks the reader to open the original in its default app.
            server.set_opener(Box::new(|p| {
                let _ = open::that(p);
            }));
            Some(server)
        })
        .as_ref()
}

fn page_dir() -> PathBuf {
    std::env::temp_dir().join("toolbench-viewer")
}

/// Open the viewer for `result` in the browser; blocking (builds the page or the line index).
pub fn open(result: &FileSearchResult, settings: &SearchSettings) {
    open_with(result, settings, &page_dir(), reader(), &|target| {
        let _ = open::that(target);
    });
}

/// `open` with its surroundings injected: where pages are written, which reader serves large files, and what opens a
/// target (a page path, a reader URL, or the original file).
pub fn open_with(result: &FileSearchResult, settings: &SearchSettings, dir: &Path, reader: Option<&ReaderServer>, opener: &dyn Fn(&str)) {
    let path = Path::new(&result.full_name);
    let fallback = || opener(&result.full_name);
    match viewer::plan_for(path) {
        Plan::NoViewer => fallback(),
        Plan::Reader => match reader.zip(LargeView::open(path, result, settings).ok()) {
            Some((server, view)) => opener(&server.register(view)),
            None => fallback(),
        },
        Plan::Embedded | Plan::Pdf => {
            let written = viewer::build_page(path, result, settings).ok().and_then(|page| {
                std::fs::create_dir_all(dir).ok()?;
                let file = dir.join(viewer::page_file_name(path));
                std::fs::write(&file, page.html).ok()?;
                Some(file)
            });
            match written {
                Some(file) => opener(&file.to_string_lossy()),
                None => fallback(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use search_core::models::{FileSearchStatus, LineHit};
    use std::cell::RefCell;

    fn result(path: &Path) -> FileSearchResult {
        let meta = std::fs::metadata(path).unwrap();
        FileSearchResult {
            full_name: path.to_string_lossy().into_owned(),
            status: FileSearchStatus::Hit,
            hits: vec![LineHit { line_number: 2, before: None, match_line: "beta apple".into(), after: None, matched_filters: vec!["apple".into()] }],
            created: Local::now(),
            modified: meta.modified().map(chrono::DateTime::<Local>::from).unwrap(),
            file_length: meta.len() as i64,
            lines_cache: Vec::new(),
            total_line_count: 0,
            proximity_min_range: None,
            low_confidence_pdf: false,
            ocr_confidence: None,
            error_message: None,
        }
    }

    fn settings() -> SearchSettings {
        let mut s = SearchSettings::default();
        s.filters = vec!["apple".into()];
        s
    }

    fn opened(path: &Path, dir: &Path, reader: Option<&ReaderServer>) -> Vec<String> {
        let seen = RefCell::new(Vec::new());
        open_with(&result(path), &settings(), dir, reader, &|t| seen.borrow_mut().push(t.to_string()));
        seen.into_inner()
    }

    #[test]
    fn a_small_text_file_opens_its_written_page() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("notes.txt");
        std::fs::write(&file, "alpha\nbeta apple\n").unwrap();
        let seen = opened(&file, &tmp.path().join("pages"), None);
        assert_eq!(seen.len(), 1);
        assert!(seen[0].ends_with(".html") && seen[0].contains("notes-"), "got {seen:?}");
        let html = std::fs::read_to_string(&seen[0]).unwrap();
        assert!(html.contains("beta apple"), "the file's text is embedded in the page");
    }

    #[test]
    fn a_binary_file_opens_in_the_default_app() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("blob.bin");
        std::fs::write(&file, b"abc\0\0def").unwrap();
        assert_eq!(opened(&file, tmp.path(), None), vec![file.to_string_lossy().into_owned()]);
    }

    #[test]
    fn a_large_file_opens_the_reader_url_and_falls_back_without_a_reader() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("big.log");
        std::fs::write(&file, "beta apple\n".repeat(150_000)).unwrap();
        let server = ReaderServer::start().unwrap();
        let seen = opened(&file, tmp.path(), Some(&server));
        assert_eq!(seen.len(), 1);
        assert!(seen[0].starts_with(&format!("http://127.0.0.1:{}/r/", server.port())), "got {seen:?}");
        assert_eq!(opened(&file, tmp.path(), None), vec![file.to_string_lossy().into_owned()], "no reader: default app");
    }

    #[test]
    fn a_page_that_cannot_be_written_falls_back_to_the_default_app() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("notes.txt");
        std::fs::write(&file, "alpha\nbeta apple\n").unwrap();
        let blocker = tmp.path().join("pages");
        std::fs::write(&blocker, "a file where the folder should go").unwrap();
        assert_eq!(opened(&file, &blocker, None), vec![file.to_string_lossy().into_owned()]);
    }
}
