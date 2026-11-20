//! The local reader: serves the viewer page of a large plain text file and its blocks of lines, so the page never
//! holds the file whole.
//!
//! Bound to `127.0.0.1` on an ephemeral port. Every URL starts with a per-run random token (`/r/<token>/...`), the
//! `Host` header must name this exact address (a DNS-rebinding page cannot pass it), nothing is writable and the
//! responses are never cached. The page itself comes from here too, so its fetches are same-origin and need no CORS.
//!
//! Lines are read by byte offset from a sparse index (one checkpoint per [`STRIDE`] lines). The split rules are the
//! extractor's (`extraction::split_lines`): `\r\n`, a lone `\r` and `\n` each end a line, and text after the last break
//! is a line (so a file ending in a newline has a final empty line).

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::HitSet;
use crate::extraction::decode_text;
use crate::models::{FileSearchResult, SearchSettings};

/// One index checkpoint per this many lines.
pub const STRIDE: u64 = 256;
/// Most lines one `lines` request returns.
const MAX_LINES: usize = 2000;
/// Time one `find` request scans before it hands back what it has and where to continue.
const FIND_SLICE: Duration = Duration::from_millis(120);

/// Byte offsets of every [`STRIDE`]th line of a file, plus the line count.
pub struct LineIndex {
    checkpoints: Vec<u64>,
    pub total: u64,
    pub size: u64,
}

impl LineIndex {
    /// Scan the whole file once for line starts (measured 0.9 to 1.0 GB/s on a 305 MB, 4 million line file from the file
    /// cache; a cold disk is the limit). A UTF-8 byte order mark is skipped.
    pub fn build(path: &Path) -> std::io::Result<Self> {
        let mut f = std::fs::File::open(path)?;
        let size = f.metadata()?.len();
        let mut buf = vec![0u8; 1 << 20];
        let mut checkpoints = vec![0u64];
        let (mut count, mut prev_cr, mut pos, mut first) = (0u64, false, 0u64, true);
        if size == 0 {
            return Ok(Self { checkpoints: Vec::new(), total: 0, size });
        }
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            let mut chunk = &buf[..n];
            if first {
                first = false;
                if chunk.starts_with(&[0xEF, 0xBB, 0xBF]) {
                    chunk = &chunk[3..];
                    pos = 3;
                    checkpoints[0] = 3;
                }
            }
            for &b in chunk {
                pos += 1;
                match b {
                    b'\r' => {
                        count += 1;
                        if count % STRIDE == 0 {
                            checkpoints.push(pos);
                        }
                        prev_cr = true;
                    }
                    b'\n' if prev_cr => {
                        // the second half of CRLF: the line that began after the CR begins after the LF instead
                        if count % STRIDE == 0 {
                            *checkpoints.last_mut().expect("a checkpoint exists") = pos;
                        }
                        prev_cr = false;
                    }
                    b'\n' => {
                        count += 1;
                        if count % STRIDE == 0 {
                            checkpoints.push(pos);
                        }
                    }
                    _ => prev_cr = false,
                }
            }
        }
        Ok(Self { checkpoints, total: count + 1, size })
    }
}

/// Yields the lines of a byte stream with the extractor's break rules.
struct LineReader<R: BufRead> {
    r: R,
    done: bool,
}

impl<R: BufRead> LineReader<R> {
    fn next_line(&mut self, out: &mut Vec<u8>) -> std::io::Result<bool> {
        if self.done {
            return Ok(false);
        }
        out.clear();
        loop {
            let (used, hit) = {
                let buf = self.r.fill_buf()?;
                if buf.is_empty() {
                    self.done = true;
                    return Ok(true);
                }
                match buf.iter().position(|&b| b == b'\n' || b == b'\r') {
                    Some(i) => {
                        out.extend_from_slice(&buf[..i]);
                        (i + 1, Some(buf[i]))
                    }
                    None => {
                        out.extend_from_slice(buf);
                        (buf.len(), None)
                    }
                }
            };
            self.r.consume(used);
            if let Some(b) = hit {
                if b == b'\r' && self.r.fill_buf()?.first() == Some(&b'\n') {
                    self.r.consume(1);
                }
                return Ok(true);
            }
        }
    }
}

fn decode_line(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => decode_text(bytes),
    }
}

/// A file registered with the reader.
pub struct LargeView {
    path: PathBuf,
    index: LineIndex,
    hits: HitSet,
    settings: SearchSettings,
    result: FileSearchResult,
}

impl LargeView {
    /// Open `path` for reading: builds the line index (reads the whole file once, without keeping it).
    pub fn open(path: &Path, result: &FileSearchResult, settings: &SearchSettings) -> std::io::Result<Self> {
        let index = LineIndex::build(path)?;
        Ok(Self { path: path.to_path_buf(), index, hits: HitSet::from_result(result, settings), settings: settings.clone(), result: result.clone() })
    }

    pub fn total_lines(&self) -> u64 {
        self.index.total
    }

    /// A reader positioned at the start of 1-based line `from`, with the lines before it already skipped.
    fn lines_from(&self, from: u64) -> std::io::Result<LineReader<BufReader<std::fs::File>>> {
        let l0 = from.saturating_sub(1).min(self.index.total.saturating_sub(1));
        let cp = (l0 / STRIDE) as usize;
        let mut f = std::fs::File::open(&self.path)?;
        f.seek(SeekFrom::Start(self.index.checkpoints.get(cp).copied().unwrap_or(0)))?;
        let mut lr = LineReader { r: BufReader::with_capacity(64 * 1024, f), done: false };
        let mut scratch = Vec::new();
        for _ in 0..(l0 - cp as u64 * STRIDE) {
            if !lr.next_line(&mut scratch)? {
                break;
            }
        }
        Ok(lr)
    }

    fn row(&self, n: u64, text: String) -> Value {
        match self.hits.entries.binary_search_by_key(&(n as u32), |e| e.0) {
            Ok(i) => {
                let spans = self.hits.spans(n as u32, &text, &self.settings);
                json!([n, text, self.hits.slot_mask(self.hits.entries[i].1), spans])
            }
            Err(_) => json!([n, text]),
        }
    }

    /// Lines `from..from + count` (1-based), each `[n, text]` or `[n, text, slotMask, spans]` for a hit line.
    pub fn lines(&self, from: u64, count: usize) -> std::io::Result<Vec<Value>> {
        let count = count.min(MAX_LINES);
        if from == 0 || from > self.index.total || count == 0 {
            return Ok(Vec::new());
        }
        let mut lr = self.lines_from(from)?;
        let (mut out, mut buf) = (Vec::with_capacity(count), Vec::new());
        let mut n = from;
        while out.len() < count && n <= self.index.total && lr.next_line(&mut buf)? {
            out.push(self.row(n, decode_line(&buf)));
            n += 1;
        }
        Ok(out)
    }

    /// The listed lines (ascending or not), at most [`MAX_LINES`].
    pub fn lines_of(&self, wanted: &[u64]) -> std::io::Result<Vec<Value>> {
        let mut sorted: Vec<u64> = wanted.iter().copied().filter(|n| *n >= 1 && *n <= self.index.total).take(MAX_LINES).collect();
        sorted.sort_unstable();
        sorted.dedup();
        let mut out = Vec::with_capacity(sorted.len());
        let mut buf = Vec::new();
        let mut i = 0;
        while i < sorted.len() {
            let mut lr = self.lines_from(sorted[i])?;
            let mut n = sorted[i];
            // keep reading forward while the next wanted line is close, so a run of nearby lines costs one seek
            while i < sorted.len() && sorted[i] - n <= STRIDE && lr.next_line(&mut buf)? {
                if n == sorted[i] {
                    out.push(self.row(n, decode_line(&buf)));
                    i += 1;
                }
                n += 1;
            }
            if i < sorted.len() && sorted[i] < n {
                i += 1; // a line the file no longer has
            }
        }
        Ok(out)
    }

    /// Flat `[line, slotMask, line, slotMask, ...]` of every hit.
    pub fn hit_list(&self) -> Vec<u32> {
        self.hits.entries.iter().flat_map(|&(n, bits)| [n, self.hits.slot_mask(bits) as u32]).collect()
    }

    /// Scan for lines matching `re` from line `from`, for about [`FIND_SLICE`]. Returns the matching line numbers and
    /// the line to continue from (`0` when the end was reached).
    pub fn find(&self, re: &regex::Regex, from: u64, max: usize) -> std::io::Result<(Vec<u64>, u64)> {
        let start = Instant::now();
        let (mut found, mut buf) = (Vec::new(), Vec::new());
        if from == 0 || from > self.index.total {
            return Ok((found, 0));
        }
        let mut lr = self.lines_from(from)?;
        let mut n = from;
        while n <= self.index.total && lr.next_line(&mut buf)? {
            if re.is_match(&decode_line(&buf)) {
                found.push(n);
            }
            n += 1;
            if found.len() >= max || (n & 255 == 0 && start.elapsed() >= FIND_SLICE) {
                break;
            }
        }
        Ok((found, if n > self.index.total { 0 } else { n }))
    }
}

struct Shared {
    port: u16,
    token: String,
    views: Mutex<HashMap<String, Arc<LargeView>>>,
    stop: AtomicBool,
    /// Opens a file in the default app (the `native` route); search-core has no such dependency, the host supplies it.
    opener: Mutex<Option<Opener>>,
}

/// Host callback that opens a path in the system's default app.
pub type Opener = Box<dyn Fn(&Path) + Send + Sync>;

/// The server. Cheap to keep for the life of the program; [`ReaderServer::shutdown`] stops it.
pub struct ReaderServer {
    shared: Arc<Shared>,
}

fn random_hex(bytes: usize) -> String {
    let mut s = String::new();
    while s.len() < bytes * 2 {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        s.push_str(&format!("{:016x}", h.finish()));
    }
    s.truncate(bytes * 2);
    s
}

impl ReaderServer {
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let shared = Arc::new(Shared { port: listener.local_addr()?.port(), token: random_hex(16), views: Mutex::new(HashMap::new()), stop: AtomicBool::new(false), opener: Mutex::new(None) });
        let acceptor = shared.clone();
        std::thread::Builder::new().name("viewer-reader".into()).spawn(move || {
            for stream in listener.incoming() {
                if acceptor.stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(stream) = stream {
                    let s = acceptor.clone();
                    let _ = std::thread::Builder::new().name("viewer-reader-conn".into()).spawn(move || {
                        let _ = handle(stream, &s);
                    });
                }
            }
        })?;
        Ok(Self { shared })
    }

    /// Set what the `native` route calls ("Open the file itself" in the viewer menu).
    pub fn set_opener(&self, opener: Opener) {
        *self.shared.opener.lock().expect("opener lock") = Some(opener);
    }

    pub fn port(&self) -> u16 {
        self.shared.port
    }

    /// Register a large file; returns the URL that opens its viewer page.
    pub fn register(&self, view: LargeView) -> String {
        let id = random_hex(6);
        self.shared.views.lock().expect("views lock").insert(id.clone(), Arc::new(view));
        format!("http://127.0.0.1:{}/r/{}/v/{}", self.shared.port, self.shared.token, id)
    }

    /// Stop accepting connections and forget every view.
    pub fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.views.lock().expect("views lock").clear();
        let _ = TcpStream::connect(("127.0.0.1", self.shared.port)); // wake the accept loop
    }
}

impl Drop for ReaderServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query(q: &str) -> HashMap<String, String> {
    q.split('&').filter(|p| !p.is_empty()).map(|p| match p.split_once('=') {
        Some((k, v)) => (percent_decode(k), percent_decode(v)),
        None => (percent_decode(p), String::new()),
    }).collect()
}

fn respond(stream: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn json_ok(stream: &mut TcpStream, v: &Value) -> std::io::Result<()> {
    respond(stream, "200 OK", "application/json; charset=utf-8", v.to_string().as_bytes())
}

fn handle(mut stream: TcpStream, sh: &Shared) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut host = String::new();
    for _ in 0..64 {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("host") {
                host = v.trim().to_string();
            }
        }
    }
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method != "GET" {
        return respond(&mut stream, "405 Method Not Allowed", "text/plain", b"GET only");
    }
    if host != format!("127.0.0.1:{}", sh.port) {
        return respond(&mut stream, "403 Forbidden", "text/plain", b"wrong host");
    }
    let (path, q) = target.split_once('?').unwrap_or((target, ""));
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if segs.len() < 4 || segs[0] != "r" || segs[1] != sh.token {
        return respond(&mut stream, "404 Not Found", "text/plain", b"not found");
    }
    let view = sh.views.lock().expect("views lock").get(segs[3]).cloned();
    let Some(view) = view else { return respond(&mut stream, "404 Not Found", "text/plain", b"unknown file") };
    let args = query(q);
    let num = |k: &str, d: u64| args.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(d);
    match (segs[2], segs.get(4).copied()) {
        ("v", None) => {
            let base = format!("/r/{}/f/{}", sh.token, segs[3]);
            let html = super::reader_page(&view.path, &view.result, &view.hits, view.index.total, view.index.size, &base);
            respond(&mut stream, "200 OK", "text/html; charset=utf-8", html.as_bytes())
        }
        ("f", Some("lines")) => {
            let rows = if let Some(list) = args.get("l") {
                let wanted: Vec<u64> = list.split(',').filter_map(|s| s.parse().ok()).collect();
                view.lines_of(&wanted)
            } else {
                view.lines(num("from", 1), num("count", 100) as usize)
            };
            match rows {
                Ok(r) => json_ok(&mut stream, &json!({ "l": r })),
                Err(e) => respond(&mut stream, "500 Internal Server Error", "text/plain", e.to_string().as_bytes()),
            }
        }
        ("f", Some("native")) => {
            let opener = sh.opener.lock().expect("opener lock");
            match opener.as_ref() {
                Some(open) => {
                    open(&view.path);
                    json_ok(&mut stream, &json!({ "ok": true }))
                }
                None => respond(&mut stream, "501 Not Implemented", "text/plain", b"no opener"),
            }
        }
        ("f", Some("hits")) => json_ok(&mut stream, &json!({ "h": view.hit_list() })),
        ("f", Some("find")) => {
            let mut pat = args.get("q").cloned().unwrap_or_default();
            if args.get("rx").map(|v| v.as_str()) != Some("1") {
                pat = regex::escape(&pat);
            }
            if args.get("ww").map(|v| v.as_str()) == Some("1") {
                pat = format!(r"\b(?:{pat})\b");
            }
            let cs = args.get("cs").map(|v| v.as_str()) == Some("1");
            match regex::RegexBuilder::new(&pat).case_insensitive(!cs).size_limit(1 << 22).build() {
                Err(e) => json_ok(&mut stream, &json!({ "err": e.to_string().lines().last().unwrap_or("invalid pattern") })),
                Ok(re) => match view.find(&re, num("from", 1), num("max", 5000) as usize) {
                    Ok((m, next)) => json_ok(&mut stream, &json!({ "m": m, "next": next })),
                    Err(e) => respond(&mut stream, "500 Internal Server Error", "text/plain", e.to_string().as_bytes()),
                },
            }
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", b"not found"),
    }?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}
