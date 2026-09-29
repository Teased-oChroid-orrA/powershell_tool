//! Entry point for the ratatui terminal head of the GS Engineering
//! Toolbench. `ratatui::init()`/`restore()` already install and tear down
//! a panic hook that restores the terminal before any panic propagates -
//! no hand-rolled hook needed. The event loop follows the standard async
//! ratatui shape: one task forwards crossterm input + a periodic tick into
//! a single `AppEvent` channel, and one consumer loop drains it, calling
//! the pure `handle_event` reducer, executing whatever `Effect`s it
//! returns, and redrawing (see the migration plan's Event and State Model
//! section for the rationale). The actual background search/report work
//! lives in `app_tui::toolboxes::search::runner` (library code, so it's
//! covered by real `#[tokio::test]` integration tests) - this file only
//! wires it up.

use std::io;
use std::io::stdout;
use std::time::Duration;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream};
use crossterm::execute;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use app_tui::app::{handle_event, handle_mouse, AppEvent, AppState, Effect};
use app_tui::mouse::MouseRegions;
use app_tui::toolboxes::search::{extension_picker, indexing, persistence, runner};
use app_tui::widgets::shell;

fn main() -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let mut terminal = ratatui::init();
    enable_mouse_capture_failsafe();
    let result = runtime.block_on(run(&mut terminal));
    disable_mouse_capture_best_effort();
    ratatui::restore();
    result
}

/// Enables mouse input and makes sure it can never survive the process -
/// `ratatui::init()`/`restore()` already chain a panic hook that restores
/// raw mode/the alternate screen, but they know nothing about mouse capture
/// since it's never been enabled before now. A terminal left in mouse
/// capture mode after this process dies prints raw escape sequences into
/// the user's shell on every subsequent click - this hook (and the
/// matching explicit disable on normal exit, in `main()`) is what prevents
/// that, on both the panic path and the ordinary-exit path.
fn enable_mouse_capture_failsafe() {
    let _ = execute!(stdout(), EnableMouseCapture);
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture);
        previous_hook(info);
    }));
}

fn disable_mouse_capture_best_effort() {
    let _ = execute!(stdout(), DisableMouseCapture);
}

async fn run(terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();

    let forward_tx = tx.clone();
    tokio::spawn(async move {
        let mut events = EventStream::new();
        let mut ticker = tokio::time::interval(Duration::from_millis(200));
        loop {
            tokio::select! {
                maybe_event = events.next() => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            if forward_tx.send(AppEvent::Terminal(event)).is_err() {
                                break;
                            }
                        }
                        // A read error or a closed stream both mean input
                        // is no longer available - stop forwarding rather
                        // than spin.
                        Some(Err(_)) | None => break,
                    }
                }
                _ = ticker.tick() => {
                    if forward_tx.send(AppEvent::Tick).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut state = AppState::default();
    let mut tick_count: u64 = 0;
    // Render-derived hit-test scratch, rebuilt every frame by
    // `shell::draw` - deliberately not part of `AppState` (see
    // `app_tui::mouse`'s module doc).
    let mut mouse_regions = MouseRegions::default();

    terminal.draw(|frame| shell::draw(frame, &state, tick_count, &mut mouse_regions))?;

    while let Some(event) = rx.recv().await {
        if matches!(event, AppEvent::Tick) {
            tick_count = tick_count.wrapping_add(1);
        }

        // Mouse events are dispatched directly against the region set the
        // most recent render just published, bypassing `handle_event`
        // (whose `AppEvent::Terminal(_) => Vec::new()` catch-all stays the
        // safe default for anyone else still handing it a raw Mouse event).
        let effects = if let AppEvent::Terminal(Event::Mouse(mouse_event)) = event {
            handle_mouse(&mut state, &mouse_regions, mouse_event)
        } else {
            handle_event(&mut state, event)
        };
        for effect in effects {
            execute_effect(&tx, &mut state, effect);
        }

        if state.should_quit {
            break;
        }

        terminal.draw(|frame| shell::draw(frame, &state, tick_count, &mut mouse_regions))?;
    }

    // Best-effort - a failed settings save is never a reason to interrupt
    // shutdown, matching both existing GUI heads' own persistence
    // philosophy.
    let persisted = state.search.to_persisted_file();
    let _ = tokio::task::spawn_blocking(move || persistence::save(&persisted)).await;

    Ok(())
}

/// Runs everything `handle_event` asked for. This is the ONLY place in the
/// crate that touches the async runtime, the filesystem, or the OS
/// directly for toolbox effects - `handle_event` and everything it calls
/// stays pure/synchronous (see the plan's Event and State Model section).
fn execute_effect(tx: &mpsc::UnboundedSender<AppEvent>, state: &mut AppState, effect: Effect) {
    match effect {
        Effect::StartSearch { roots, settings, index } => {
            let cancellation = CancellationToken::new();
            state.search.cancel_token = Some(cancellation.clone());
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
            let persisted = state.search.to_persisted_file();
            tokio::task::spawn_blocking(move || persistence::save(&persisted));
        }
        Effect::BuildIndex { settings, index_dir, force_rebuild } => {
            tokio::spawn(indexing::build_or_rebuild_index(tx.clone(), settings, index_dir, force_rebuild));
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
    }
}
