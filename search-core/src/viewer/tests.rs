use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Local;
use serde_json::Value;

use super::reader::{LargeView, LineIndex, ReaderServer, STRIDE};
use super::*;
use crate::extraction::{decode_text, split_lines};
use crate::models::{FileSearchStatus, LineHit};

fn settings(filters: &[&str]) -> SearchSettings {
    let mut s = SearchSettings::default();
    s.filters = filters.iter().map(|f| f.to_string()).collect();
    s
}

fn hit(line: i32, filters: &[&str]) -> LineHit {
    LineHit { line_number: line, before: None, match_line: String::new(), after: None, matched_filters: filters.iter().map(|f| f.to_string()).collect() }
}

fn result_for(path: &Path, hits: Vec<LineHit>) -> FileSearchResult {
    let meta = std::fs::metadata(path).unwrap();
    FileSearchResult {
        full_name: path.to_string_lossy().into_owned(),
        status: FileSearchStatus::Hit,
        hits,
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

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// What the extractor makes of these bytes: the reference every reader line must equal.
fn expected_lines(bytes: &[u8]) -> Vec<String> {
    split_lines(&decode_text(bytes))
}

fn view_lines(v: &LargeView) -> Vec<String> {
    let total = v.total_lines();
    let mut out: Vec<String> = Vec::new();
    while (out.len() as u64) < total {
        let from = out.len() as u64 + 1;
        let rows = v.lines(from, 1500).unwrap();
        assert!(!rows.is_empty(), "reader returned nothing at line {from} of {total}");
        assert_eq!(rows[0][0].as_u64().unwrap(), from, "line numbers run in order");
        out.extend(rows.iter().map(|r| r[1].as_str().unwrap().to_string()));
    }
    out
}

fn open_view(path: &Path, hits: Vec<LineHit>, filters: &[&str]) -> LargeView {
    let s = settings(filters);
    LargeView::open(path, &result_for(path, hits), &s).unwrap()
}

// ---------------------------------------------------------------- plan

#[test]
fn plan_for_picks_the_viewer_by_type_and_size() {
    let dir = tempfile::tempdir().unwrap();
    let small = write(dir.path(), "a.txt", b"hello\nworld\n");
    let big = write(dir.path(), "big.log", &vec![b'x'; (SMALL_LIMIT + 1) as usize]);
    let binary = write(dir.path(), "b.bin", b"abc\0def");
    let pdf = write(dir.path(), "d.PDF", b"%PDF-1.4");
    let docx = write(dir.path(), "e.docx", b"PK");
    let utf16 = {
        let mut b = vec![0xFF, 0xFE];
        b.extend(std::iter::repeat([b'a', 0u8]).take((SMALL_LIMIT as usize) / 2 + 10).flatten());
        write(dir.path(), "u.txt", &b)
    };
    assert_eq!(plan_for(&small), Plan::Embedded);
    assert_eq!(plan_for(&big), Plan::Reader);
    assert_eq!(plan_for(&binary), Plan::NoViewer);
    assert_eq!(plan_for(&pdf), Plan::Pdf);
    assert_eq!(plan_for(&docx), Plan::Embedded);
    assert_eq!(plan_for(&dir.path().join("missing.txt")), Plan::NoViewer);
    assert_eq!(plan_for(dir.path()), Plan::NoViewer);
    // The reader serves bytes as UTF-8/ANSI lines; a big UTF-16 file has to go through the extractor instead.
    assert_eq!(plan_for(&utf16), Plan::Embedded);
}

// ---------------------------------------------------------------- hits

#[test]
fn hit_spans_are_utf16_units_with_the_report_colour_slots() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "a.txt", b"x\n");
    let s = settings(&["alpha", "beta"]);
    let r = result_for(&p, vec![hit(1, &["alpha", "beta"]), hit(3, &["beta"]), hit(2, &["ALPHA"])]);
    let hs = HitSet::from_result(&r, &s);
    assert_eq!(hs.entries, vec![(1, 0b11), (2, 0b01), (3, 0b10)], "sorted by line, a bit per filter, names matched case-blind");
    assert_eq!(hs.slot_mask(0b11), 0b11);
    assert_eq!(hs.slot_mask(0b10), 0b10);
    // U+1F600 is 2 UTF-16 units but 4 UTF-8 bytes; the e-acute is 1 unit and 2 bytes.
    let text = "\u{e9}\u{1F600} alpha and beta";
    assert_eq!(hs.spans(1, text, &s), vec![[4, 9, 0], [14, 18, 1]]);
    assert_eq!(hs.spans(99, text, &s), Vec::<[u32; 3]>::new(), "not a hit line");
    assert_eq!(hs.chips(), vec![("alpha".to_string(), 0, 2), ("beta".to_string(), 1, 2)]);
}

#[test]
fn highlight_ranges_merge_overlaps_and_keep_the_earlier_slot() {
    let s = settings(&["tor", "torque"]);
    let colors = FilterColors::new(&s);
    let ranges = highlight_ranges("the torque value", &["tor".to_string(), "torque".to_string()], &s, &colors);
    assert_eq!(ranges, vec![(4, 10, 0)]);
}

// ---------------------------------------------------------------- pages

#[test]
fn embedded_page_carries_lines_hits_and_cannot_be_closed_early_by_file_text() {
    let dir = tempfile::tempdir().unwrap();
    let text = "first alpha line\n</script><b>boom</b> beta\nlast\n";
    let p = write(dir.path(), "a.txt", text.as_bytes());
    let s = settings(&["alpha", "beta"]);
    let r = result_for(&p, vec![hit(1, &["alpha"]), hit(2, &["beta"])]);
    let page = build_page(&p, &r, &s).unwrap();
    assert_eq!(page.plan, Plan::Embedded);
    assert_eq!(page.lines, 4, "three lines and the empty one after the final newline, as the extractor splits");
    assert!(!page.html.contains("</script><b>"), "file text must not close the config script");
    assert_eq!(page.html.matches("</script").count(), 2, "only the config script and the app script end");
    let cfg = cfg_of(&page.html);
    assert_eq!(cfg["name"], "a.txt");
    assert_eq!(cfg["kind"], "lines");
    assert_eq!(cfg["total"], 4);
    assert_eq!(cfg["capped"], false);
    assert!(cfg["stale"].is_null());
    assert_eq!(cfg["embedded"]["lines"][1], "</script><b>boom</b> beta");
    assert_eq!(cfg["embedded"]["hits"][0], serde_json::json!([1, 1, [[6, 11, 0]]]));
    assert_eq!(cfg["embedded"]["hits"][1][1], 2);
    assert_eq!(cfg["filters"][1]["slot"], 1);
    assert!(cfg["uri"].as_str().unwrap().starts_with("file:///"));
}

#[test]
fn page_notes_a_file_that_changed_since_the_search() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "a.txt", b"alpha\n");
    let s = settings(&["alpha"]);
    let mut r = result_for(&p, vec![hit(1, &["alpha"])]);
    r.file_length = 999_999;
    let page = build_page(&p, &r, &s).unwrap();
    let note = cfg_of(&page.html)["stale"].as_str().unwrap().to_string();
    assert!(note.starts_with("was "), "{note}");
    assert!(note.contains("now"), "{note}");
}

#[test]
fn build_page_refuses_binaries_and_marks_pdf_pages() {
    let dir = tempfile::tempdir().unwrap();
    let bin = write(dir.path(), "b.bin", b"a\0b");
    let s = settings(&["a"]);
    assert!(matches!(build_page(&bin, &result_for(&bin, vec![]), &s), Err(ViewerError::NoViewer)));
    let pdf = write(dir.path(), "x.pdf", b"%PDF-1.4 not a real pdf");
    let page = build_page(&pdf, &result_for(&pdf, vec![]), &s).unwrap();
    let cfg = cfg_of(&page.html);
    assert_eq!(page.plan, Plan::Pdf);
    assert_eq!(cfg["kind"], "pdf");
    assert!(cfg["pdf"].as_str().unwrap().ends_with("x.pdf"));
}

#[test]
fn page_file_names_are_stable_and_filesystem_safe() {
    let a = page_file_name(Path::new("/tmp/My Report (v2).txt"));
    assert_eq!(a, page_file_name(Path::new("/tmp/My Report (v2).txt")));
    assert_ne!(a, page_file_name(Path::new("/other/My Report (v2).txt")));
    assert!(a.ends_with(".html") && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)), "{a}");
}

/// The JSON of the `<script id="cfg">` block.
fn cfg_of(html: &str) -> Value {
    let start = html.find("<script id=\"cfg\" type=\"application/json\">").unwrap() + "<script id=\"cfg\" type=\"application/json\">".len();
    let end = start + html[start..].find("</script>").unwrap();
    serde_json::from_str(&html[start..end]).unwrap()
}

// ---------------------------------------------------------------- line index and reader

fn check_parity(name: &str, bytes: &[u8]) {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), name, bytes);
    let v = open_view(&p, vec![], &["x"]);
    let want = expected_lines(bytes);
    assert_eq!(v.total_lines() as usize, want.len(), "{name}: line count");
    assert_eq!(view_lines(&v), want, "{name}: line text");
}

#[test]
fn line_index_splits_exactly_like_the_extractor() {
    check_parity("lf.txt", b"a\nb\nc");
    check_parity("crlf.txt", b"a\r\nb\r\nc\r\n");
    check_parity("cr.txt", b"a\rb\rc");
    check_parity("mixed.txt", b"a\r\nb\nc\rd\r\n\r\ne\n\n");
    check_parity("trailing.txt", b"a\nb\n");
    check_parity("bom.txt", b"\xEF\xBB\xBFfirst\nsecond");
    check_parity("empty.txt", b"");
    check_parity("blank.txt", b"\n");
    check_parity("crcr.txt", b"\r\r\n\n\r");
}

/// `n` lines of the form `line <i>` ending with `eol`, with the first break placed so that its first byte is the last
/// byte of the index's 1 MiB read buffer (the CR of a CRLF straddles two reads).
fn straddling(eol: &[u8], lines: usize) -> Vec<u8> {
    let boundary = (1usize << 20) - 1;
    let mut out = vec![b'p'; boundary];
    out.extend_from_slice(eol);
    for i in 0..lines {
        out.extend_from_slice(format!("line {i}").as_bytes());
        out.extend_from_slice(eol);
    }
    out
}

#[test]
fn line_index_handles_a_break_split_across_read_buffers_and_many_checkpoints() {
    for eol in [&b"\r\n"[..], b"\r", b"\n"] {
        let bytes = straddling(eol, STRIDE as usize * 5 + 17);
        check_parity("straddle.txt", &bytes);
    }
}

#[test]
fn reader_serves_any_window_and_listed_lines_of_a_multi_stride_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut text = String::new();
    for i in 1..=(STRIDE * 6 + 40) {
        text.push_str(&format!("row {i} {}\r\n", if i % 100 == 0 { "needle" } else { "hay" }));
    }
    let p = write(dir.path(), "w.log", text.as_bytes());
    let v = open_view(&p, vec![hit(100, &["needle"]), hit(300, &["needle"])], &["needle"]);
    let want = expected_lines(text.as_bytes());
    assert_eq!(v.total_lines() as usize, want.len());
    for from in [1u64, 2, STRIDE - 1, STRIDE, STRIDE + 1, STRIDE * 3, v.total_lines() - 1, v.total_lines()] {
        let rows = v.lines(from, 7).unwrap();
        for (k, r) in rows.iter().enumerate() {
            let n = from as usize + k;
            assert_eq!(r[0].as_u64().unwrap() as usize, n);
            assert_eq!(r[1].as_str().unwrap(), want[n - 1], "line {n}");
        }
        assert_eq!(rows.len(), 7.min((v.total_lines() - from + 1) as usize));
    }
    assert!(v.lines(v.total_lines() + 1, 5).unwrap().is_empty());
    assert!(v.lines(0, 5).unwrap().is_empty());
    // listed lines come back ascending with one entry each, hits carry mask and UTF-16 spans
    let rows = v.lines_of(&[300, 5, 5, 100, STRIDE * 6, 9_999_999]).unwrap();
    let got: Vec<u64> = rows.iter().map(|r| r[0].as_u64().unwrap()).collect();
    assert_eq!(got, vec![5, 100, 300, STRIDE * 6]);
    assert_eq!(rows[1][2], 1);
    assert_eq!(rows[1][3], serde_json::json!([[8, 14, 0]]));
    assert_eq!(rows[0].as_array().unwrap().len(), 2, "a non-hit line is just [n, text]");
    assert_eq!(v.hit_list(), vec![100, 1, 300, 1]);
}

#[test]
fn reader_find_scans_in_slices_and_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let mut text = String::new();
    for i in 1..=2000 {
        text.push_str(&format!("row {i} {}\n", if i % 250 == 0 { "Needle" } else { "hay" }));
    }
    let p = write(dir.path(), "f.log", text.as_bytes());
    let v = open_view(&p, vec![], &["x"]);
    let re = regex::RegexBuilder::new("needle").case_insensitive(true).build().unwrap();
    let (all, next) = v.find(&re, 1, 100).unwrap();
    assert_eq!(all, vec![250, 500, 750, 1000, 1250, 1500, 1750, 2000]);
    assert_eq!(next, 0, "reached the end");
    let (first, next) = v.find(&re, 1, 3).unwrap();
    assert_eq!(first, vec![250, 500, 750]);
    assert!(next > 750);
    let (rest, next) = v.find(&re, next, 100).unwrap();
    assert_eq!(rest, vec![1000, 1250, 1500, 1750, 2000]);
    assert_eq!(next, 0);
}

// ---------------------------------------------------------------- server

fn http(port: u16, host: &str, method: &str, target: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let code = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
    (code, raw.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("").to_string())
}

fn server_with_file() -> (ReaderServer, String, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut text = String::new();
    for i in 1..=600 {
        text.push_str(&format!("row {i} {}\n", if i % 100 == 0 { "needle" } else { "hay" }));
    }
    let p = write(dir.path(), "s.log", text.as_bytes());
    let server = ReaderServer::start().unwrap();
    let url = server.register(open_view(&p, vec![hit(100, &["needle"]), hit(200, &["needle"])], &["needle"]));
    (server, url, dir, p)
}

fn path_of(url: &str) -> String {
    url.split_once("127.0.0.1:").unwrap().1.split_once('/').map(|x| format!("/{}", x.1)).unwrap()
}

#[test]
fn reader_server_serves_page_and_data_only_with_the_token_host_and_get() {
    let (server, url, _dir, _p) = server_with_file();
    let port = server.port();
    let host = format!("127.0.0.1:{port}");
    let page_path = path_of(&url); // /r/<token>/v/<id>
    let segs: Vec<&str> = page_path.trim_start_matches('/').split('/').collect();
    let (token, id) = (segs[1], segs[3]);

    let (code, body) = http(port, &host, "GET", &page_path);
    assert_eq!(code, 200);
    assert!(body.contains("<script id=\"cfg\""), "the page itself is served");
    let cfg = cfg_of(&body);
    assert_eq!(cfg["total"], 601, "600 lines plus the empty one after the final newline");
    assert_eq!(cfg["reader"]["base"], format!("/r/{token}/f/{id}"));
    assert_eq!(cfg["kind"], "lines");
    assert!(cfg.get("embedded").is_none(), "a reader page never carries the text");

    let base = format!("/r/{token}/f/{id}");
    let (code, body) = http(port, &host, "GET", &format!("{base}/lines?from=99&count=3"));
    assert_eq!(code, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["l"][1], serde_json::json!([100, "row 100 needle", 1, [[8, 14, 0]]]));
    assert_eq!(v["l"][0], serde_json::json!([99, "row 99 hay"]));

    let (_, body) = http(port, &host, "GET", &format!("{base}/lines?l=3,200"));
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["l"].as_array().unwrap().len(), 2);

    let (_, body) = http(port, &host, "GET", &format!("{base}/hits"));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["h"], serde_json::json!([100, 1, 200, 1]));

    let (_, body) = http(port, &host, "GET", &format!("{base}/find?q=NEEDLE&cs=0&from=1&max=10"));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), serde_json::json!({"m": [100, 200, 300, 400, 500, 600], "next": 0}));
    let (_, body) = http(port, &host, "GET", &format!("{base}/find?q=NEEDLE&cs=1&from=1&max=10"));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["m"], serde_json::json!([]), "case-sensitive");
    let (_, body) = http(port, &host, "GET", &format!("{base}/find?q=%28&rx=1&from=1"));
    assert!(serde_json::from_str::<Value>(&body).unwrap()["err"].is_string(), "a bad pattern is reported, not a 500");
    let (_, body) = http(port, &host, "GET", &format!("{base}/find?q=row%20%281%29&rx=0&from=1&max=1"));
    assert!(serde_json::from_str::<Value>(&body).unwrap()["m"].as_array().unwrap().is_empty(), "plain text is escaped, not a pattern");

    // refusals
    assert_eq!(http(port, "evil.example:80", "GET", &page_path).0, 403, "a rebinding page cannot pass the Host check");
    assert_eq!(http(port, &host, "GET", &page_path.replace(token, "0000")).0, 404, "wrong token");
    assert_eq!(http(port, &host, "GET", &format!("/r/{token}/v/nope")).0, 404, "unknown file");
    assert_eq!(http(port, &host, "POST", &page_path).0, 405);
    assert_eq!(http(port, &host, "GET", "/").0, 404);
    assert_eq!(http(port, &host, "GET", &format!("{base}/native")).0, 501, "no opener set");
}

#[test]
fn reader_native_route_calls_the_host_opener_with_the_registered_path() {
    let (server, url, _dir, p) = server_with_file();
    let opened = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(std::sync::Mutex::new(PathBuf::new()));
    let (o, sn) = (opened.clone(), seen.clone());
    server.set_opener(Box::new(move |path| {
        o.store(true, Ordering::SeqCst);
        *sn.lock().unwrap() = path.to_path_buf();
    }));
    let page_path = path_of(&url);
    let native = page_path.replace("/v/", "/f/") + "/native";
    let (code, body) = http(server.port(), &format!("127.0.0.1:{}", server.port()), "GET", &native);
    assert_eq!(code, 200, "{body}");
    assert!(opened.load(Ordering::SeqCst));
    assert_eq!(*seen.lock().unwrap(), p);
}

#[test]
fn reader_shutdown_forgets_files_and_refuses_new_requests() {
    let (server, url, _dir, _p) = server_with_file();
    let port = server.port();
    let page_path = path_of(&url);
    server.shutdown();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let refused = match TcpStream::connect(("127.0.0.1", port)) {
        Err(_) => true,
        Ok(mut s) => {
            let _ = write!(s, "GET {page_path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
            let mut raw = String::new();
            let _ = s.read_to_string(&mut raw);
            !raw.starts_with("HTTP/1.1 200")
        }
    };
    assert!(refused, "no page after shutdown");
}

#[test]
fn line_index_build_reports_a_missing_file() {
    assert!(LineIndex::build(Path::new("/definitely/not/here.log")).is_err());
}
