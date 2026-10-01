//! Always-on diagnostic log, written next to where the app was launched.
//!
//! Exists because the Windows-only failures (fast re-search index above all)
//! cannot be reproduced on the development machines: when something goes
//! wrong the user sends back `toolbench-debug.log` and it contains the
//! environment, every index stage with timings, the full per-file failure
//! list and any panic with a backtrace - the evidence the orchestrator's
//! "reproduce first / last confirmed lifecycle stage" rules ask for.
//!
//! Location: the current working directory (the folder the app was launched
//! from); falls back to the executable's folder, then the OS temp dir, if
//! that is not writable. Set `TOOLBENCH_DEBUG=0` to disable. The file is
//! flushed per line (a crash must not lose the last line - the last line IS
//! the evidence) and rotated to `.old` once it passes [`MAX_BYTES`].
//!
//! Never log file *contents* or secrets - paths, counts, sizes, timings and
//! error strings only.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const LOG_FILE_NAME: &str = "toolbench-debug.log";
const MAX_BYTES: u64 = 5 * 1024 * 1024;

struct Sink {
    file: File,
    path: PathBuf,
    started: Instant,
}

static SINK: OnceLock<Option<Mutex<Sink>>> = OnceLock::new();

/// Opens the log (idempotent), writes the environment banner and installs a
/// panic hook that records the panic before the previous hook runs.
pub fn init() {
    SINK.get_or_init(|| {
        if std::env::var("TOOLBENCH_DEBUG").map(|v| v == "0").unwrap_or(false) {
            return None;
        }
        let (file, path) = open_first_writable()?;
        let sink = Sink { file, path, started: Instant::now() };
        Some(Mutex::new(sink))
    });
    if SINK.get().map(|s| s.is_some()).unwrap_or(false) {
        write_banner();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let bt = std::backtrace::Backtrace::force_capture();
            log("PANIC", format!("{info}\nbacktrace:\n{bt}"));
            previous(info);
        }));
    }
}

/// Where the log is being written, if enabled.
pub fn path() -> Option<PathBuf> {
    SINK.get()?.as_ref().map(|m| lock(m).path.clone())
}

fn lock(m: &Mutex<Sink>) -> std::sync::MutexGuard<'_, Sink> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(d) = std::env::current_dir() {
        dirs.push(d);
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        dirs.push(d);
    }
    dirs.push(std::env::temp_dir());
    dirs
}

fn open_first_writable() -> Option<(File, PathBuf)> {
    for dir in candidate_dirs() {
        let path = dir.join(LOG_FILE_NAME);
        if std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
            let _ = std::fs::rename(&path, dir.join(format!("{LOG_FILE_NAME}.old")));
        }
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
            return Some((file, path));
        }
    }
    None
}

fn write_banner() {
    log("SESSION", "==================== new session ====================");
    log("ENV", format!("app-tui {} | os={} arch={} family={}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, std::env::consts::FAMILY));
    log("ENV", format!("cwd={:?} exe={:?}", std::env::current_dir().ok(), std::env::current_exe().ok()));
    log("ENV", format!("cpus={:?} debug_build={}", std::thread::available_parallelism().map(|n| n.get()).ok(), cfg!(debug_assertions)));
    log("ENV", format!("log_file={:?}", path()));
    if let Ok(v) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        log("ENV", format!("home={v}"));
    }
}

/// Appends one line. A no-op when logging is disabled/unavailable.
pub fn log(component: &str, message: impl AsRef<str>) {
    let Some(Some(sink)) = SINK.get() else { return };
    let mut sink = lock(sink);
    let line = format!(
        "{} +{:>8.3}s [{}] [{}] {}\n",
        timestamp_utc(),
        sink.started.elapsed().as_secs_f64(),
        std::thread::current().name().unwrap_or("worker"),
        component,
        message.as_ref()
    );
    let _ = sink.file.write_all(line.as_bytes());
    let _ = sink.file.flush();
}

fn timestamp_utc() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format_utc(now.as_secs() as i64, now.subsec_millis())
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` from unix seconds (Howard Hinnant's
/// civil-from-days; avoids a date-library dependency for one log line).
fn format_utc(secs: i64, millis: u32) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_utc(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_utc(951_782_400, 5), "2000-02-29T00:00:00.005Z");
        assert_eq!(format_utc(1_790_000_000, 123), "2026-09-21T14:13:20.123Z");
    }

    #[test]
    fn logging_before_init_is_a_silent_no_op() {
        log("TEST", "must not panic");
    }
}
