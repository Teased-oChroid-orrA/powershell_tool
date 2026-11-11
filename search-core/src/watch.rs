//! Opt-in folder watcher (#240 P2-2): remembers which files under one root changed, so the fast index
//! can be brought up to date for just those paths (`native_index::update_corpus_index_paths_send`)
//! instead of walking and freshness-checking every file.
//!
//! In-process and user-mode only. Windows uses `ReadDirectoryChangesW` on a handle opened with
//! ordinary read access: no administrator rights, no service, driver, scheduled task, registry value
//! or system file is created or changed, and nothing outlives the process (dropping the watcher
//! closes the handle). Nothing is written to the watched folder.
//!
//! The watcher only collects paths. It never reads file contents and never touches the index; the
//! caller decides when to apply a batch. An event it cannot trust (the OS dropped events, the batch
//! grew too large, the backend reported an error) sets `overflow`: the caller must then do a normal
//! full update. Watching is best effort (network shares, some synced folders report nothing), so a
//! full update remains the way to be sure.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::file_reader::path_in_scope;

/// More distinct changed paths than this and a full update is cheaper and safer than a path list.
const MAX_PENDING_PATHS: usize = 50_000;

#[derive(Default)]
struct Pending {
    paths: BTreeSet<String>,
    overflow: bool,
    last_event: Option<Instant>,
}

/// A drained batch of changes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Changes {
    /// Changed paths in scope (files, or directories created, renamed or removed).
    pub paths: Vec<String>,
    /// Events may have been lost: do a full update instead of trusting `paths`.
    pub overflow: bool,
}

pub struct FolderWatcher {
    _watcher: RecommendedWatcher,
    pending: Arc<Mutex<Pending>>,
}

impl FolderWatcher {
    /// Starts watching `root` recursively. `include_hidden` and `exclude_folders` are the search
    /// settings' own, so the watcher reports exactly what the index walk would visit.
    pub fn start(root: &str, include_hidden: bool, exclude_folders: Vec<String>) -> Result<Self, String> {
        let pending = Arc::new(Mutex::new(Pending::default()));
        let sink = Arc::clone(&pending);
        let root_owned = root.to_string();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let mut state = match sink.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            state.last_event = Some(Instant::now());
            let event = match event {
                Ok(event) => event,
                Err(_) => {
                    state.overflow = true;
                    return;
                }
            };
            if event.need_rescan() {
                state.overflow = true;
            }
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            for path in &event.paths {
                if !path_in_scope(&root_owned, path, include_hidden, &exclude_folders) {
                    continue;
                }
                if state.paths.len() >= MAX_PENDING_PATHS {
                    state.overflow = true;
                    state.paths.clear();
                    return;
                }
                state.paths.insert(path.to_string_lossy().into_owned());
            }
        })
        .map_err(|e| e.to_string())?;
        watcher
            .watch(Path::new(root), RecursiveMode::Recursive)
            .map_err(|e| e.to_string())?;
        Ok(Self { _watcher: watcher, pending })
    }

    /// The pending batch if there is one and no event has arrived for `quiet` (so a burst of saves or
    /// a copy in progress is applied once, after it settles). Draining resets the batch.
    pub fn take_quiet(&self, quiet: Duration) -> Option<Changes> {
        let mut state = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        if state.paths.is_empty() && !state.overflow {
            return None;
        }
        if state.last_event.is_some_and(|t| t.elapsed() < quiet) {
            return None;
        }
        let changes = Changes { paths: std::mem::take(&mut state.paths).into_iter().collect(), overflow: state.overflow };
        state.overflow = false;
        Some(changes)
    }

    /// Puts a batch back (an update could not run, e.g. a search was in progress) without losing it.
    pub fn restore(&self, changes: Changes) {
        let mut state = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        state.overflow |= changes.overflow;
        state.paths.extend(changes.paths);
        if state.paths.len() > MAX_PENDING_PATHS {
            state.paths.clear();
            state.overflow = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(watcher: &FolderWatcher, want: impl Fn(&Changes) -> bool) -> Changes {
        let mut seen = Changes::default();
        for _ in 0..100 {
            if let Some(batch) = watcher.take_quiet(Duration::from_millis(150)) {
                seen.overflow |= batch.overflow;
                seen.paths.extend(batch.paths);
                if want(&seen) {
                    return seen;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        seen
    }

    #[test]
    fn reports_created_changed_and_removed_files_but_not_hidden_excluded_or_index_paths() {
        let dir = tempfile::tempdir().unwrap();
        // macOS reports /private/var paths for /var temp dirs; watch the canonical root.
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("skip")).unwrap();
        std::fs::create_dir_all(root.join(crate::native_index::INDEX_FOLDER_NAME)).unwrap();
        std::fs::write(root.join("old.txt"), "x").unwrap();
        let watcher = FolderWatcher::start(&root.to_string_lossy(), false, vec!["skip".into()]).unwrap();
        std::thread::sleep(Duration::from_millis(300));

        std::fs::write(root.join("new.txt"), "hello").unwrap();
        std::fs::remove_file(root.join("old.txt")).unwrap();
        std::fs::write(root.join(".hidden.txt"), "h").unwrap();
        std::fs::write(root.join("skip/s.txt"), "s").unwrap();
        std::fs::write(root.join(crate::native_index::INDEX_FOLDER_NAME).join("seg"), "i").unwrap();

        let new = root.join("new.txt").to_string_lossy().into_owned();
        let old = root.join("old.txt").to_string_lossy().into_owned();
        let seen = wait_for(&watcher, |c| c.paths.contains(&new) && c.paths.contains(&old));
        assert!(seen.paths.contains(&new), "{:?}", seen.paths);
        assert!(seen.paths.contains(&old), "{:?}", seen.paths);
        assert!(!seen.paths.iter().any(|p| p.contains(".hidden") || p.contains("skip") || p.contains(crate::native_index::INDEX_FOLDER_NAME)), "{:?}", seen.paths);
        assert!(watcher.take_quiet(Duration::ZERO).is_none(), "draining empties the batch");
    }

    #[test]
    fn a_burst_is_held_until_quiet_and_restore_keeps_a_batch() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let watcher = FolderWatcher::start(&root.to_string_lossy(), false, Vec::new()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(root.join("a.txt"), "a").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(watcher.take_quiet(Duration::from_secs(60)).is_none(), "not quiet for a minute yet");
        let batch = wait_for(&watcher, |c| !c.paths.is_empty());
        assert!(!batch.paths.is_empty());
        watcher.restore(batch.clone());
        let again = watcher.take_quiet(Duration::ZERO).unwrap();
        assert_eq!(again.paths, batch.paths);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_panic() {
        assert!(FolderWatcher::start("/definitely/not/a/folder/xyz", false, Vec::new()).is_err());
    }
}
