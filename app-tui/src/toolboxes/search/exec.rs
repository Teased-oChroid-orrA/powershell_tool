//! Carries out the Search side effects (`Effect::StartSearch`, report writing, index builds, the extension scan,
//! open / copy / export, saving settings) on the current tokio runtime. The terminal's `main.rs` and the native
//! window's Search bench both call [`execute`], so a run, its report and its index behave the same in either head.
//! Must be called inside a runtime context (`Runtime::enter` or a runtime thread).

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app::{AppEvent, Effect};

use super::{extension_picker, indexing, persistence, runner, SearchToolState};

/// Execute `effect` when it is a Search effect; any other effect is handed back for the caller to carry out.
pub fn execute(tx: &mpsc::UnboundedSender<AppEvent>, search: &mut SearchToolState, effect: Effect) -> Option<Effect> {
    match effect {
        Effect::StartSearch { roots, settings, index } => {
            let cancellation = CancellationToken::new();
            search.cancel_token = Some(cancellation.clone());
            tokio::spawn(runner::run_search(tx.clone(), cancellation, roots, settings, index));
        }
        Effect::WriteReport { settings, run_result, write_html } => {
            let tx = tx.clone();
            tokio::task::spawn_blocking(move || runner::write_report(&tx, settings, run_result, write_html));
        }
        Effect::OpenPath(path) => {
            tokio::task::spawn_blocking(move || {
                let _ = open::that(path);
            });
        }
        Effect::CopyToClipboard(text) => {
            tokio::task::spawn_blocking(move || {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(text);
                }
            });
        }
        Effect::WriteTextFileAndOpen { path, contents } => {
            tokio::task::spawn_blocking(move || {
                if let Some(parent) = std::path::Path::new(&path).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if std::fs::write(&path, contents).is_ok() {
                    let _ = open::that(&path);
                }
            });
        }
        Effect::PersistSearchSettings => {
            let persisted = search.to_persisted_file();
            tokio::task::spawn_blocking(move || persistence::save(&persisted));
        }
        Effect::BuildIndex { settings, index_dir, cancel } => {
            tokio::spawn(indexing::build_or_rebuild_index(tx.clone(), settings, index_dir, cancel));
        }
        Effect::ScanExtensions { root, exclude_folders, include_hidden } => {
            let tx = tx.clone();
            tokio::task::spawn_blocking(move || {
                let path = std::path::Path::new(&root);
                if !path.is_dir() {
                    let _ = tx.send(AppEvent::ExtensionsScanned(Err(format!("'{root}' is not a directory"))));
                    return;
                }
                let found = extension_picker::scan_extensions(path, &exclude_folders, include_hidden);
                let _ = tx.send(AppEvent::ExtensionsScanned(Ok(found)));
            });
        }
        other => return Some(other),
    }
    None
}
