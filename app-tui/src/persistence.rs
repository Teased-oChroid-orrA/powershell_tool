//! Crash-safe replacement and bounded event journals shared by native workbenches.
use std::{
    collections::VecDeque,
    io::{self, BufRead, Write},
    path::Path,
};

/// Stage beside the destination so the final rename stays on the same filesystem.
/// Keep the previous complete file; never truncate the live file on a failed write.
pub fn write(path: impl AsRef<Path>, data: impl AsRef<[u8]>) -> io::Result<()> {
    let path = path.as_ref();
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut staged = tempfile::NamedTempFile::new_in(dir)?;
    staged.write_all(data.as_ref())?;
    staged.as_file().sync_all()?;
    if path.exists() {
        let old = std::fs::read(path)?;
        let mut backup = tempfile::NamedTempFile::new_in(dir)?;
        backup.write_all(&old)?;
        backup.as_file().sync_all()?;
        backup
            .persist(path.with_extension("previous.json"))
            .map_err(|e| e.error)?;
    }
    staged.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// Stream a journal with bounded line size and retained records. Torn/oversized records are ignored.
pub fn read_recent<T: serde::de::DeserializeOwned>(
    path: &Path,
    limit: usize,
) -> io::Result<Vec<T>> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut reader = io::BufReader::new(file);
    let mut records = VecDeque::new();
    let mut line = Vec::new();
    let mut retained_bytes = 0usize;
    let mut oversized = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            break;
        }
        let end = buf.iter().position(|b| *b == b'\n');
        let n = end.map_or(buf.len(), |i| i + 1);
        if line.len() + n <= 64 * 1024 && !oversized {
            line.extend_from_slice(&buf[..n]);
        } else {
            oversized = true;
            line.clear();
        }
        reader.consume(n);
        if end.is_some() {
            if !oversized {
                if let Ok(record) = serde_json::from_slice(&line) {
                    if limit > 0 {
                        retained_bytes += line.len();
                        records.push_back((line.len(), record));
                    }
                    while records.len() > limit || retained_bytes > 8 * 1024 * 1024 {
                        if let Some((n, _)) = records.pop_front() {
                            retained_bytes -= n;
                        } else {
                            break;
                        }
                    }
                }
            }
            line.clear();
            oversized = false;
        }
    }
    Ok(records.into_iter().map(|(_, record)| record).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_preserves_previous_complete_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("case.json");
        write(&p, "first").unwrap();
        write(&p, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "second");
        assert_eq!(
            std::fs::read_to_string(p.with_extension("previous.json")).unwrap(),
            "first"
        );
    }
    #[test]
    fn journal_retention_ignores_torn_and_oversized_records() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("events");
        std::fs::write(&p, format!("1\n{}\n2\n3\n4", "9".repeat(100_000))).unwrap();
        assert_eq!(read_recent::<u32>(&p, 2).unwrap(), [2, 3]);
    }
}

/// Rotate large local journals at a complete-record boundary before appending.
pub fn append_json(path: &Path, record: &impl serde::Serialize) -> io::Result<()> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > 16 * 1024 * 1024) {
        let records = read_recent::<serde_json::Value>(path, 50_000)?;
        let mut data = Vec::new();
        for r in records {
            serde_json::to_writer(&mut data, &r)?;
            data.push(b'\n');
        }
        write(path, data)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    serde_json::to_writer(&mut f, record)?;
    f.write_all(b"\n")
}

/// Content identity, rather than mtime alone (editors and network drives can preserve timestamps).
pub fn revision(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| fea_problem::solid::text_hash(&s))
}
