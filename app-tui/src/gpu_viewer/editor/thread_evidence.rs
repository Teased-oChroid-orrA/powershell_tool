//! Local, app-owned browser view of published thread evidence.
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

const PAGE: &str = include_str!("thread_evidence/index.html");
const DATA: &str = include_str!("thread_evidence/nasa-coil.json");
const TAPPED_PAGE: &str = include_str!("thread_evidence/tapped.html");
const TAPPED_DATA: &str = include_str!("thread_evidence/nasa-tapped.json");
const TULSA_DATA: &str = include_str!("thread_evidence/tulsa-tapped.json");
const ZHANG_DATA: &str = include_str!("thread_evidence/zhang-tapped.json");
const ZHANG_CURVES: &str = include_str!("thread_evidence/zhang-figure5.json");
const KRAUS_DATA: &str = include_str!("thread_evidence/kraus-blind.json");
const MAX_HEAD: usize = 8192;
const MAX_BODY: usize = 16384;

#[derive(Deserialize)]
struct SourceRow {
    id: String,
    load: [Option<f64>; 5],
    eq: [Option<f64>; 5],
}

#[derive(Deserialize)]
struct TappedRow {
    id: String,
    limit: Option<f64>,
    pullout: Option<f64>,
}

#[derive(Deserialize)]
struct TulsaRow {
    id: String,
    hole_in: f64,
    load_lbf: f64,
}

#[derive(Deserialize)]
struct ZhangRow {
    id: String,
    d_in: f64,
    p_in: f64,
    dh_in: f64,
    th_in: f64,
    fu_lbf: f64,
    cov_pct: f64,
    du_in: f64,
    ku_lbf_per_in: f64,
}

#[derive(Deserialize)]
struct ZhangCurve {
    id: String,
    points: Vec<ZhangPoint>,
}

#[derive(Deserialize)]
struct ZhangPoint {
    travel_in: f64,
    load_lbf: f64,
}

#[derive(Deserialize)]
struct KrausRow {
    id: String,
    engagement_in: f64,
    mean_lbf: f64,
    sd_lbf: f64,
}

fn kraus_rows() -> &'static [KrausRow] {
    static ROWS: OnceLock<Vec<KrausRow>> = OnceLock::new();
    ROWS.get_or_init(|| serde_json::from_str(KRAUS_DATA).expect("bundled Kraus evidence data"))
}

fn zhang_curves() -> &'static [ZhangCurve] {
    static CURVES: OnceLock<Vec<ZhangCurve>> = OnceLock::new();
    CURVES.get_or_init(|| serde_json::from_str(ZHANG_CURVES).expect("bundled Zhang curve data"))
}

fn zhang_rows() -> &'static [ZhangRow] {
    static ROWS: OnceLock<Vec<ZhangRow>> = OnceLock::new();
    ROWS.get_or_init(|| serde_json::from_str(ZHANG_DATA).expect("bundled Zhang evidence data"))
}

fn tulsa_rows() -> &'static [TulsaRow] {
    static ROWS: OnceLock<Vec<TulsaRow>> = OnceLock::new();
    ROWS.get_or_init(|| serde_json::from_str(TULSA_DATA).expect("bundled Tulsa evidence data"))
}

fn tapped_rows() -> &'static [TappedRow] {
    static ROWS: OnceLock<Vec<TappedRow>> = OnceLock::new();
    ROWS.get_or_init(|| serde_json::from_str(TAPPED_DATA).expect("bundled tapped evidence data"))
}

fn source_rows() -> &'static [SourceRow] {
    static ROWS: OnceLock<Vec<SourceRow>> = OnceLock::new();
    ROWS.get_or_init(|| serde_json::from_str(DATA).expect("bundled evidence data"))
}

#[derive(Default, Deserialize, Serialize)]
struct Saved {
    version: u32,
    edits: BTreeMap<String, f64>,
}

#[derive(Deserialize)]
struct Update {
    revision: Option<String>,
    edits: BTreeMap<String, f64>,
}

fn valid_edits(edits: &BTreeMap<String, f64>) -> bool {
    if edits.len() > source_rows().len() * 10 + tapped_rows().len() * 2 + tulsa_rows().len() * 2 + zhang_rows().len() * 8 + zhang_curves().iter().map(|c| c.points.len() * 2).sum::<usize>() + kraus_rows().len() * 3 {
        return false;
    }
    edits.iter().all(|(key, value)| {
        let mut parts = key.split(':');
        let Some(kind) = parts.next() else {
            return false;
        };
        let Some(series) = parts.next() else {
            return false;
        };
        let Some(index) = parts.next().and_then(|x| x.parse::<usize>().ok()) else {
            return false;
        };
        if parts.next().is_some() || !value.is_finite() {
            return false;
        }
        match kind {
            "load" | "eq" => {
                let Some(row) = source_rows().iter().find(|row| row.id == series) else { return false; };
                if index >= row.load.len() { return false; }
                match kind {
                    "load" => row.load[index].is_some() && (0.0..=1_000_000_000.0).contains(value),
                    _ => row.eq[index].is_some() && (-15.0..=15.0).contains(value),
                }
            }
            "limit" | "pullout" => {
                let Some(row) = tapped_rows().iter().find(|row| row.id == series) else { return false; };
                index == 0 && (0.0..=1_000_000_000.0).contains(value)
                    && (if kind == "limit" { row.limit } else { row.pullout }).is_some()
            }
            "tulsa_hole" | "tulsa_load" => {
                let Some(row) = tulsa_rows().iter().find(|row| row.id == series) else { return false; };
                index == 0 && if kind == "tulsa_hole" {
                    row.hole_in > 0.0 && (0.5..=0.7).contains(value)
                } else {
                    row.load_lbf > 0.0 && (0.0..=1_000_000_000.0).contains(value)
                }
            }
            "zhang_d" | "zhang_p" | "zhang_dh" | "zhang_th" | "zhang_fu" | "zhang_cov" | "zhang_du" | "zhang_ku" => {
                let Some(row) = zhang_rows().iter().find(|row| row.id == series) else { return false; };
                if index != 0 { return false; }
                let (published, max) = match kind {
                    "zhang_d" => (row.d_in, 10.0),
                    "zhang_p" => (row.p_in, 10.0),
                    "zhang_dh" => (row.dh_in, 10.0),
                    "zhang_th" => (row.th_in, 10.0),
                    "zhang_fu" => (row.fu_lbf, 1_000_000_000.0),
                    "zhang_cov" => (row.cov_pct, 100.0),
                    "zhang_du" => (row.du_in, 100.0),
                    _ => (row.ku_lbf_per_in, 1_000_000_000.0),
                };
                published > 0.0 && (0.0..=max).contains(value)
            }
            "zhang_curve_x" | "zhang_curve_y" => {
                let Some(curve) = zhang_curves().iter().find(|curve| curve.id == series) else { return false; };
                let Some(point) = curve.points.get(index) else { return false; };
                let max = if kind == "zhang_curve_x" { 1.0 } else { 100_000.0 };
                (0.0..=max).contains(value) && point.travel_in >= 0.0 && point.load_lbf >= 0.0
            }
            "kraus_depth" | "kraus_mean" | "kraus_sd" => {
                let Some(row) = kraus_rows().iter().find(|row| row.id == series) else { return false; };
                if index != 0 { return false; }
                let (published, max) = match kind {
                    "kraus_depth" => (row.engagement_in, 10.0),
                    "kraus_mean" => (row.mean_lbf, 1_000_000_000.0),
                    _ => (row.sd_lbf, 1_000_000_000.0),
                };
                published > 0.0 && (0.0..=max).contains(value)
            }
            _ => false,
        }
    })
}

fn read_saved(path: &Path) -> io::Result<Saved> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Saved {
                version: 1,
                ..Saved::default()
            })
        }
        Err(e) => return Err(e),
    };
    let saved: Saved = serde_json::from_str(&raw).map_err(io::Error::other)?;
    if saved.version != 1 || !valid_edits(&saved.edits) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid saved evidence",
        ));
    }
    Ok(saved)
}

struct Server {
    port: u16,
    token: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(store: PathBuf) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let mut secret = [0u8; 24];
        getrandom::fill(&mut secret).map_err(io::Error::other)?;
        let token: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let route = token.clone();
        let worker = thread::Builder::new()
            .name("thread-evidence".into())
            .spawn(move || {
                for connection in listener.incoming() {
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(stream) = connection {
                        let _ = handle(stream, port, &route, &store);
                    }
                }
            })?;
        Ok(Self {
            port,
            token,
            stop,
            worker: Some(worker),
        })
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/e/{}/", self.port, self.token)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn open() -> Result<(), String> {
    static SERVER: OnceLock<Mutex<Option<Server>>> = OnceLock::new();
    let mut server = SERVER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|e| e.to_string())?;
    if server.is_none() {
        let dir = crate::paths::app_data_dir().ok_or("App-data folder is unavailable")?;
        *server = Some(Server::start(dir.join("thread-evidence.json")).map_err(|e| e.to_string())?);
    }
    open::that(server.as_ref().unwrap().url()).map_err(|e| e.to_string())
}

fn response(stream: &mut TcpStream, code: &str, mime: &str, body: &[u8]) -> io::Result<()> {
    write!(stream, "HTTP/1.1 {code}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'\r\nConnection: close\r\n\r\n", body.len())?;
    stream.write_all(body)
}

fn locked<T>(store: &Path, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let dir = store
        .parent()
        .ok_or_else(|| io::Error::other("no app-data folder"))?;
    std::fs::create_dir_all(dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(dir.join("thread-evidence.lock"))?;
    lock.lock_exclusive()?;
    f()
}

fn handle(mut stream: TcpStream, port: u16, token: &str, store: &Path) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut head = Vec::new();
    loop {
        let mut byte = [0];
        if reader.read_exact(&mut byte).is_err() {
            return Ok(());
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
        if head.len() >= MAX_HEAD {
            return response(
                &mut stream,
                "431 Request Header Fields Too Large",
                "text/plain",
                b"headers too large",
            );
        }
    }
    let head = match std::str::from_utf8(&head) {
        Ok(head) => head,
        Err(_) => {
            return response(
                &mut stream,
                "400 Bad Request",
                "text/plain",
                b"invalid headers",
            )
        }
    };
    let mut lines = head.split("\r\n");
    let mut request = lines.next().unwrap_or("").split_whitespace();
    let (method, target) = (request.next().unwrap_or(""), request.next().unwrap_or(""));
    let (mut host, mut origin, mut length, mut content_type, mut transfer_encoding) =
        ("", "", None, "", false);
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("host") {
                host = value;
            }
            if name.eq_ignore_ascii_case("origin") {
                origin = value;
            }
            if name.eq_ignore_ascii_case("content-length") {
                length = value.parse::<usize>().ok();
            }
            if name.eq_ignore_ascii_case("content-type") {
                content_type = value;
            }
            if name.eq_ignore_ascii_case("transfer-encoding") {
                transfer_encoding = true;
            }
        }
    }
    let base = format!("/e/{token}/");
    let own_origin = format!("http://127.0.0.1:{port}");
    if host != format!("127.0.0.1:{port}") || (!origin.is_empty() && origin != own_origin) {
        return response(&mut stream, "403 Forbidden", "text/plain", b"wrong origin");
    }
    if !target.starts_with(&base) {
        return response(&mut stream, "404 Not Found", "text/plain", b"not found");
    }
    let route = &target[base.len()..];
    match (method, route) {
        ("GET", "") => response(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes(),
        ),
        ("GET", "data") => response(
            &mut stream,
            "200 OK",
            "application/json; charset=utf-8",
            DATA.as_bytes(),
        ),
        ("GET", "tapped") => response(&mut stream, "200 OK", "text/html; charset=utf-8", TAPPED_PAGE.as_bytes()),
        ("GET", "tapped/data") => response(&mut stream, "200 OK", "application/json; charset=utf-8", TAPPED_DATA.as_bytes()),
        ("GET", "tulsa/data") => response(&mut stream, "200 OK", "application/json; charset=utf-8", TULSA_DATA.as_bytes()),
        ("GET", "zhang/data") => response(&mut stream, "200 OK", "application/json; charset=utf-8", ZHANG_DATA.as_bytes()),
        ("GET", "zhang/curves") => response(&mut stream, "200 OK", "application/json; charset=utf-8", ZHANG_CURVES.as_bytes()),
        ("GET", "kraus/data") => response(&mut stream, "200 OK", "application/json; charset=utf-8", KRAUS_DATA.as_bytes()),
        ("GET", "state") => {
            let data = locked(store, || {
                let saved = read_saved(store)?;
                serde_json::to_vec(&serde_json::json!({"revision": crate::persistence::revision(store), "edits": saved.edits})).map_err(io::Error::other)
            });
            match data {
                Ok(body) => response(&mut stream, "200 OK", "application/json", &body),
                Err(_) => response(
                    &mut stream,
                    "500 Internal Server Error",
                    "text/plain",
                    b"cannot read saved evidence",
                ),
            }
        }
        ("PUT", "state") => {
            if transfer_encoding
                || !content_type.starts_with("application/json")
                || length.is_none_or(|n| n > MAX_BODY)
            {
                return response(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"invalid request body",
                );
            }
            let mut body = vec![0; length.unwrap()];
            reader.read_exact(&mut body)?;
            let Ok(update) = serde_json::from_slice::<Update>(&body) else {
                return response(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"invalid JSON",
                );
            };
            if !valid_edits(&update.edits) {
                return response(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"invalid evidence cell",
                );
            }
            let result = locked(store, || {
                read_saved(store)?;
                if crate::persistence::revision(store) != update.revision {
                    return Ok(None);
                }
                let saved = Saved {
                    version: 1,
                    edits: update.edits,
                };
                crate::persistence::write(
                    store,
                    serde_json::to_vec(&saved).map_err(io::Error::other)?,
                )?;
                serde_json::to_vec(
                    &serde_json::json!({"revision": crate::persistence::revision(store)}),
                )
                .map(Some)
                .map_err(io::Error::other)
            });
            match result {
                Ok(Some(body)) => response(&mut stream, "200 OK", "application/json", &body),
                Ok(None) => response(
                    &mut stream,
                    "409 Conflict",
                    "text/plain",
                    b"evidence changed in another window",
                ),
                Err(_) => response(
                    &mut stream,
                    "500 Internal Server Error",
                    "text/plain",
                    b"cannot save evidence",
                ),
            }
        }
        _ => response(&mut stream, "404 Not Found", "text/plain", b"not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(server: &Server, method: &str, route: &str, body: &str, origin: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
        write!(stream, "{method} /e/{}/{route} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", server.token, server.port, body.len()).unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        (
            raw.split_whitespace().nth(1).unwrap().parse().unwrap(),
            raw.split_once("\r\n\r\n").unwrap().1.into(),
        )
    }

    #[test]
    fn edits_survive_restart_and_reject_invalid_or_stale_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("thread-evidence.json");
        let server = Server::start(store.clone()).unwrap();
        let origin = format!("http://127.0.0.1:{}", server.port);
        assert_eq!(http(&server, "GET", "", "", &origin).0, 200);
        assert_eq!(http(&server, "GET", "data", "", &origin).0, 200);
        assert_eq!(http(&server, "GET", "tapped", "", &origin).0, 200);
        assert_eq!(http(&server, "GET", "tapped/data", "", &origin).0, 200);
        assert_eq!(http(&server, "GET", "tulsa/data", "", &origin).0, 200);
        assert_eq!(http(&server, "GET", "zhang/data", "", &origin).0, 200);
        assert_eq!(http(&server, "GET", "zhang/curves", "", &origin).0, 200);
        assert_eq!(http(&server, "GET", "kraus/data", "", &origin).0, 200);
        let (_, state) = http(&server, "GET", "state", "", &origin);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&state).unwrap()["revision"],
            serde_json::Value::Null
        );
        let valid = r#"{"revision":null,"edits":{"load:B1:0":52000.0}}"#;
        assert_eq!(http(&server, "PUT", "state", valid, &origin).0, 200);
        assert_eq!(http(&server, "PUT", "state", valid, &origin).0, 409);
        let (_, current) = http(&server, "GET", "state", "", &origin);
        let current: serde_json::Value = serde_json::from_str(&current).unwrap();
        assert_eq!(current["edits"]["load:B1:0"], 52000.0);
        let tapped = serde_json::json!({"revision": current["revision"], "edits": {"load:B1:0": 52000.0, "pullout:8:0": 7700.0}}).to_string();
        assert_eq!(http(&server, "PUT", "state", &tapped, &origin).0, 200);
        let (_, updated) = http(&server, "GET", "state", "", &origin);
        let updated: serde_json::Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(updated["edits"]["pullout:8:0"], 7700.0);
        let tulsa = serde_json::json!({"revision": updated["revision"], "edits": {"load:B1:0": 52000.0, "tulsa_load:formed-0602:0": 34000.0}}).to_string();
        assert_eq!(http(&server, "PUT", "state", &tulsa, &origin).0, 200);
        let (_, latest) = http(&server, "GET", "state", "", &origin);
        let latest: serde_json::Value = serde_json::from_str(&latest).unwrap();
        let zhang = serde_json::json!({"revision": latest["revision"], "edits": {"load:B1:0": 52000.0, "zhang_fu:MS-D5P0.8:0": 1450.0, "zhang_curve_y:MS-D5P1.1:10": 1500.0, "kraus_mean:M20-S355-17.5:0": 57400.0}}).to_string();
        assert_eq!(http(&server, "PUT", "state", &zhang, &origin).0, 200);
        for key in ["load:P:0", "other:B1:0", "load:B1:9", "zhang_curve_y:MS-D5P0.8:999", "zhang_curve_x:unknown:0", "kraus_mean:M20-S355-17.5:1", "kraus_mean:unknown:0"] {
            let body = serde_json::json!({"revision": updated["revision"], "edits": {(key): 1.0}})
                .to_string();
            assert_eq!(http(&server, "PUT", "state", &body, &origin).0, 400);
        }
        assert_eq!(
            http(&server, "GET", "state", "", "http://bad.example").0,
            403
        );
        drop(server);
        let reopened = Server::start(store).unwrap();
        let new_origin = format!("http://127.0.0.1:{}", reopened.port);
        let (_, saved) = http(&reopened, "GET", "state", "", &new_origin);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved).unwrap()["edits"]["load:B1:0"],
            52000.0
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved).unwrap()["edits"]["zhang_curve_y:MS-D5P1.1:10"],
            1500.0
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved).unwrap()["edits"]["kraus_mean:M20-S355-17.5:0"],
            57400.0
        );
    }
}
