//! Filesystem watching (epic §21): flags the search folder as changed
//! since the last run, so the user knows a re-run might find something
//! new. `notify`'s watcher API is blocking/callback-based (native OS
//! watch APIs under the hood - FSEvents/ReadDirectoryChangesW/inotify,
//! not polling), so it runs on its own dedicated OS thread, bridged into
//! the app via two channels - the same "background thread/loop +
//! channel" shape `drag_drop.rs` uses for the same underlying reason
//! (this app's UI-facing state only lives inside the Dioxus/tokio side).

use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::OnceLock;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{mpsc as tokio_mpsc, Mutex};

enum WatchCommand {
    SetPath(PathBuf),
}

static WATCH_COMMANDS: OnceLock<std_mpsc::Sender<WatchCommand>> = OnceLock::new();
/// Drained by a spawned task in `main.rs`'s `App` - one message per
/// detected change, already filtered to exclude the native_search index
/// folder (see below). Carries the actual changed path(s) (not just a
/// bare "something changed" signal) - state.rs's incremental-reindex task
/// (issue #6 Phase 1) needs to know *which* file to re-extract/re-index or
/// remove, not just that the folder is now possibly stale.
pub static CHANGE_EVENTS: OnceLock<Mutex<tokio_mpsc::UnboundedReceiver<Vec<PathBuf>>>> = OnceLock::new();

/// Starts the watcher thread. Call once, before the first `set_path`.
#[allow(unused_assignments, unused_variables)]
pub fn start() {
    let (cmd_tx, cmd_rx) = std_mpsc::channel::<WatchCommand>();
    let (change_tx, change_rx) = tokio_mpsc::unbounded_channel::<Vec<PathBuf>>();
    let _ = WATCH_COMMANDS.set(cmd_tx);
    let _ = CHANGE_EVENTS.set(Mutex::new(change_rx));

    std::thread::spawn(move || {
        let (notify_tx, notify_rx) = std_mpsc::channel::<notify::Result<notify::Event>>();
        // Never read directly - held only so dropping/replacing it (on
        // path change) actually stops the old OS-level watch via
        // `RecommendedWatcher`'s `Drop` impl.
        let mut current_watcher: Option<RecommendedWatcher> = None;
        let mut current_path: Option<PathBuf> = None;

        loop {
            match cmd_rx.try_recv() {
                Ok(WatchCommand::SetPath(path)) => {
                    if current_path.as_ref() != Some(&path) {
                        current_watcher = None; // drop the old watcher first - stops the old watch
                        let tx = notify_tx.clone();
                        if let Ok(mut watcher) = notify::recommended_watcher(move |res| {
                            let _ = tx.send(res);
                        }) {
                            if watcher.watch(&path, RecursiveMode::Recursive).is_ok() {
                                current_watcher = Some(watcher);
                            }
                        }
                        current_path = Some(path);
                    }
                }
                Err(std_mpsc::TryRecvError::Disconnected) => break,
                Err(std_mpsc::TryRecvError::Empty) => {}
            }

            if let Ok(Ok(event)) = notify_rx.recv_timeout(Duration::from_millis(250)) {
                // The native_search "Fast re-search" index this app itself
                // writes into `<search_path>/.native-search-index/` (see
                // search_core::native_index) lives INSIDE the watched
                // folder - without this filter, every search run that also
                // indexed would immediately flag its own folder as
                // "changed", a false positive on every single run.
                let is_own_index_write = event
                    .paths
                    .iter()
                    .any(|p| p.components().any(|c| c.as_os_str() == search_core::native_index::INDEX_FOLDER_NAME));
                if !is_own_index_write {
                    let _ = change_tx.send(event.paths.clone());
                }
            }
        }
    });
}

/// Re-points the watcher at a new folder (or starts watching for the
/// first time). Safe to call on every keystroke while the user is typing
/// a path - a path that doesn't exist yet just fails to watch silently,
/// exactly like every other best-effort operation in this app.
pub fn set_path(path: &str) {
    if let Some(tx) = WATCH_COMMANDS.get() {
        let _ = tx.send(WatchCommand::SetPath(PathBuf::from(path)));
    }
}
