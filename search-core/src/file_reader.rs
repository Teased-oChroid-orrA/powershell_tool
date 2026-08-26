//! Ports `TextInFilesSearch.Core/Services/FileReaderService.cs`: read-only
//! file access helpers - a byte reader with retry-with-backoff (for files
//! transiently locked by another program) and a hard timeout (for a
//! stalled network share), plus a recursive directory walker that tracks
//! resolved real paths to guard against a symlink/junction cycle.
//!
//! This code never deletes, moves, or modifies anything - every function
//! here only ever opens files for reading.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

/// One file found by the directory walk, with metadata captured at walk
/// time - mirrors the C# side's `FileInfo`, which snapshots `Length`/
/// `CreationTimeUtc`/`LastWriteTimeUtc` at construction rather than
/// re-stat'ing the file later (avoiding both a second syscall per file and
/// a TOCTOU gap between enumeration and use).
#[derive(Debug, Clone)]
pub struct EnumeratedFile {
    pub path: PathBuf,
    pub length: i64,
    pub created: DateTime<Local>,
    pub modified: DateTime<Local>,
}

pub(crate) fn system_time_to_local(t: SystemTime) -> DateTime<Local> {
    DateTime::<chrono::Utc>::from(t).with_timezone(&Local)
}

/// Reported before each retry attempt so the caller can show "locked,
/// retrying...".
#[derive(Debug, Clone)]
pub struct RetryStatus {
    pub attempt: i32,
    pub max_retries: i32,
    pub path: String,
}

#[derive(Debug)]
pub enum ReadFileError {
    Cancelled,
    Timeout { path: String, timeout_seconds: u64 },
    Io { path: String, source: std::io::Error },
    /// The stream ended before delivering the byte count it reported at
    /// open time - the file was very likely truncated by another process
    /// mid-read.
    Truncated { path: String, expected: u64, got: u64 },
}

impl std::fmt::Display for ReadFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadFileError::Cancelled => write!(f, "read cancelled"),
            ReadFileError::Timeout { path, timeout_seconds } => {
                write!(f, "Timed out reading '{path}' after {timeout_seconds} second(s).")
            }
            ReadFileError::Io { path, source } => write!(f, "'{path}': {source}"),
            ReadFileError::Truncated { path, expected, got } => write!(
                f,
                "'{path}' was truncated during read (expected {expected} byte(s), got {got}) - likely modified concurrently by another process."
            ),
        }
    }
}

impl std::error::Error for ReadFileError {}

/// Reads a whole file as bytes with retry-with-backoff for transient
/// sharing-violation errors and a hard timeout, so a stalled network share
/// can't block the whole run. `on_retry` is invoked before each retry
/// attempt so the UI can show "locked, retrying...".
///
/// Retry policy: every I/O error is retried up to `max_retries` times
/// EXCEPT `PermissionDenied` - this mirrors the real .NET exception
/// hierarchy the C# original relies on (`catch (IOException)`), where
/// `FileNotFoundException`/`DirectoryNotFoundException` are themselves
/// `IOException` subclasses and so *are* retried, while
/// `UnauthorizedAccessException` is not an `IOException` and propagates
/// immediately. (The C# file's doc comment claims not-found errors
/// propagate immediately too - that comment is stale relative to the
/// actual .NET type hierarchy; this port follows the real behavior, not
/// the comment.)
pub async fn read_file_bytes_robust(
    path: &str,
    timeout_seconds: u64,
    max_retries: i32,
    retry_delay_ms: u64,
    mut on_retry: Option<&mut (dyn FnMut(RetryStatus) + Send)>,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ReadFileError> {
    let mut attempt = 0;

    loop {
        attempt += 1;
        if cancellation.is_cancelled() {
            return Err(ReadFileError::Cancelled);
        }

        let attempt_result = tokio::select! {
            _ = cancellation.cancelled() => Err(ReadFileError::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(timeout_seconds), read_once(path)) => {
                match result {
                    Ok(inner) => inner,
                    Err(_) => Err(ReadFileError::Timeout {
                        path: path.to_string(),
                        timeout_seconds,
                    }),
                }
            }
        };

        match attempt_result {
            Ok(bytes) => return Ok(bytes),
            Err(ReadFileError::Cancelled) => return Err(ReadFileError::Cancelled),
            Err(ReadFileError::Io { source, .. }) if source.kind() != std::io::ErrorKind::PermissionDenied && attempt <= max_retries => {
                if let Some(cb) = on_retry.as_deref_mut() {
                    cb(RetryStatus {
                        attempt,
                        max_retries,
                        path: path.to_string(),
                    });
                }
                tokio::select! {
                    _ = cancellation.cancelled() => return Err(ReadFileError::Cancelled),
                    _ = tokio::time::sleep(Duration::from_millis(retry_delay_ms * attempt as u64)) => {}
                }
            }
            Err(ReadFileError::Truncated { path: p, .. }) if attempt <= max_retries => {
                if let Some(cb) = on_retry.as_deref_mut() {
                    cb(RetryStatus {
                        attempt,
                        max_retries,
                        path: p,
                    });
                }
                tokio::select! {
                    _ = cancellation.cancelled() => return Err(ReadFileError::Cancelled),
                    _ = tokio::time::sleep(Duration::from_millis(retry_delay_ms * attempt as u64)) => {}
                }
            }
            Err(other) => return Err(other),
        }
    }
}

async fn read_once(path: &str) -> Result<Vec<u8>, ReadFileError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| ReadFileError::Io { path: path.to_string(), source: e })?;

    let expected_len = file
        .metadata()
        .await
        .map_err(|e| ReadFileError::Io { path: path.to_string(), source: e })?
        .len();

    let mut buffer = vec![0u8; expected_len as usize];
    let mut total_read: usize = 0;

    while total_read < buffer.len() {
        let read = file
            .read(&mut buffer[total_read..])
            .await
            .map_err(|e| ReadFileError::Io { path: path.to_string(), source: e })?;
        if read == 0 {
            break;
        }
        total_read += read;
    }

    if total_read < buffer.len() {
        return Err(ReadFileError::Truncated {
            path: path.to_string(),
            expected: buffer.len() as u64,
            got: total_read as u64,
        });
    }

    Ok(buffer)
}

/// Manual recursive directory walk (instead of a naive recursive
/// enumeration) that tracks visited real (resolved) directory paths to
/// guard against a symlink/junction cycle. Inaccessible folders are
/// counted and skipped, never fatal. Excluded directories are pruned from
/// the walk itself (not filtered out of the result afterward) so a huge
/// excluded tree (`node_modules`, `.git`) is never actually descended
/// into. Cancellable and reports periodic progress so a large or slow
/// (network-share) tree never looks hung with no feedback.
pub fn enumerate_files_safely(
    root_path: &str,
    include_hidden: bool,
    exclude_folders: &[String],
    cancellation: &CancellationToken,
    mut on_progress: Option<&mut dyn FnMut(i32)>,
) -> Result<(Vec<EnumeratedFile>, i32), ()> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut results: Vec<EnumeratedFile> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(root_path)];
    let mut errors = 0i32;

    let start = Instant::now();
    let mut last_progress_report = Duration::ZERO;

    while let Some(dir) = stack.pop() {
        if cancellation.is_cancelled() {
            return Err(());
        }

        let resolved_dir = match std::fs::canonicalize(&dir) {
            Ok(p) => p,
            Err(_) => {
                errors += 1;
                continue;
            }
        };
        // Case-insensitive, matching the C# side's OrdinalIgnoreCase visited
        // set - the shipped target is Windows, where paths are
        // case-insensitive.
        let resolved_key = resolved_dir.to_string_lossy().to_lowercase();
        if !visited.insert(resolved_key) {
            continue; // already visited this real directory - breaks any cycle
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => {
                errors += 1;
                continue;
            }
        };

        let mut child_dirs: Vec<PathBuf> = Vec::new();
        let mut child_files: Vec<EnumeratedFile> = Vec::new();
        for entry in entries {
            match entry {
                Ok(e) => {
                    let path = e.path();
                    match e.file_type() {
                        Ok(ft) if ft.is_dir() => child_dirs.push(path),
                        Ok(ft) if ft.is_file() => match e.metadata() {
                            Ok(meta) => {
                                let modified = meta.modified().map(system_time_to_local).unwrap_or_else(|_| Local::now());
                                let created = meta.created().map(system_time_to_local).unwrap_or(modified);
                                child_files.push(EnumeratedFile {
                                    path,
                                    length: meta.len() as i64,
                                    created,
                                    modified,
                                });
                            }
                            Err(_) => errors += 1,
                        },
                        Ok(_) => {} // symlink-to-nothing, device file, etc. - skip
                        Err(_) => errors += 1,
                    }
                }
                Err(_) => errors += 1,
            }
        }

        for f in child_files {
            if !include_hidden && is_hidden(&f.path) {
                continue;
            }
            results.push(f);
        }

        if let Some(cb) = on_progress.as_deref_mut() {
            if (start.elapsed() - last_progress_report).as_millis() >= 200 {
                cb(results.len() as i32);
                last_progress_report = start.elapsed();
            }
        }

        for d in child_dirs {
            if !include_hidden && is_hidden(&d) {
                continue;
            }
            let dir_name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let full_path = d.to_string_lossy().to_string();
            if !exclude_folders.is_empty() && is_excluded_directory(&dir_name, &full_path, exclude_folders) {
                continue;
            }
            stack.push(d);
        }
    }

    if let Some(cb) = on_progress.as_deref_mut() {
        cb(results.len() as i32);
    }

    Ok((results, errors))
}

#[cfg(windows)]
fn is_hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    std::fs::metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
        .unwrap_or(false)
}

/// Non-Windows fallback (this app's shipped target is win-x64 only - see
/// CLAUDE.md - so this path only matters for local development/testing on
/// macOS/Linux, where there's no real "hidden" file attribute bit).
#[cfg(not(windows))]
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

/// Matches a directory against `exclude_folders` by whole path segment, not
/// raw substring - a raw `full_path.contains(ex)` check would let excluding
/// "bin" also exclude any path merely containing "bin" as a substring
/// elsewhere, e.g. `C:\Users\robin\Documents`. A plain folder name (no
/// separator) must match a whole segment exactly; a path-like exclude term
/// (contains a separator) must match a contiguous run of segments, so
/// excluding "src/bin" still works as a sub-path exclusion without falling
/// back to substring matching.
fn is_excluded_directory(directory_name: &str, full_path: &str, exclude_folders: &[String]) -> bool {
    let is_sep = |c: char| c == '/' || c == '\\';

    for raw in exclude_folders {
        let trimmed = raw.trim().trim_end_matches(is_sep);
        if trimmed.is_empty() {
            continue;
        }

        if !trimmed.contains(is_sep) {
            if directory_name.eq_ignore_ascii_case(trimmed) {
                return true;
            }
            continue;
        }

        let ex_segments: Vec<&str> = trimmed.split(is_sep).filter(|s| !s.is_empty()).collect();
        let path_segments: Vec<&str> = full_path.split(is_sep).filter(|s| !s.is_empty()).collect();

        if ex_segments.is_empty() || path_segments.len() < ex_segments.len() {
            continue;
        }

        for i in 0..=(path_segments.len() - ex_segments.len()) {
            let all_match = ex_segments
                .iter()
                .enumerate()
                .all(|(j, seg)| path_segments[i + j].eq_ignore_ascii_case(seg));
            if all_match {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_file_bytes_robust_reads_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello world").unwrap();

        let cancellation = CancellationToken::new();
        let bytes = read_file_bytes_robust(
            path.to_str().unwrap(),
            30,
            3,
            10,
            None,
            &cancellation,
        )
        .await
        .unwrap();

        assert_eq!(bytes, b"hello world");
    }

    #[tokio::test]
    async fn read_file_bytes_robust_retries_a_missing_file_then_fails() {
        // Pins the documented, non-obvious behavior: a missing file DOES
        // get retried (matches real .NET IOException hierarchy), contrary
        // to the C# source's own stale doc comment.
        let mut retry_count = 0;
        let mut on_retry = |_status: RetryStatus| retry_count += 1;

        let cancellation = CancellationToken::new();
        let result = read_file_bytes_robust(
            "/definitely/does/not/exist/anywhere.txt",
            5,
            2,
            1,
            Some(&mut on_retry),
            &cancellation,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(retry_count, 2);
    }

    #[tokio::test]
    async fn read_file_bytes_robust_respects_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").unwrap();

        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let result = read_file_bytes_robust(path.to_str().unwrap(), 30, 3, 10, None, &cancellation).await;
        assert!(matches!(result, Err(ReadFileError::Cancelled)));
    }

    #[test]
    fn enumerate_files_safely_finds_files_and_prunes_excluded_folders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("bin").join("b.txt"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("robin")).unwrap();
        std::fs::write(dir.path().join("robin").join("c.txt"), b"x").unwrap();

        let cancellation = CancellationToken::new();
        let (files, errors) = enumerate_files_safely(
            dir.path().to_str().unwrap(),
            true,
            &["bin".to_string()],
            &cancellation,
            None,
        )
        .unwrap();

        assert_eq!(errors, 0);
        let names: Vec<String> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"a.txt".to_string()));
        assert!(names.contains(&"c.txt".to_string()), "excluding 'bin' must not exclude 'robin'");
        assert!(!names.contains(&"b.txt".to_string()), "'bin' folder must be pruned");
    }

    #[test]
    fn enumerate_files_safely_respects_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();

        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let result = enumerate_files_safely(dir.path().to_str().unwrap(), true, &[], &cancellation, None);
        assert!(result.is_err());
    }

    #[test]
    fn is_excluded_directory_matches_whole_segment_not_substring() {
        assert!(is_excluded_directory("bin", "/Users/x/project/bin", &["bin".to_string()]));
        assert!(!is_excluded_directory("robin", "/Users/robin/Documents", &["bin".to_string()]));
    }

    #[test]
    fn is_excluded_directory_matches_multi_segment_subpath() {
        assert!(is_excluded_directory(
            "bin",
            "/Users/x/project/src/bin",
            &["src/bin".to_string()]
        ));
        assert!(!is_excluded_directory(
            "bin",
            "/Users/x/project/other/bin",
            &["src/bin".to_string()]
        ));
    }
}
