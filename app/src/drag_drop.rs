//! Workaround for a verified `dioxus-native`/`blitz-shell` gap: dropped
//! files are never forwarded from the winit event loop to application
//! code at all (`blitz-shell-0.2.3/src/window.rs` has empty match arms for
//! `WindowEvent::DroppedFile`/`HoveredFile`/`HoveredFileCancelled` - see
//! docs/epic-ui-performance-and-design.md's "Verified platform
//! constraints" table). Rather than patching/forking `blitz-shell`, this
//! wraps `DioxusNativeApplication` in a thin `winit::application::
//! ApplicationHandler` that intercepts those three events *before*
//! delegating everything else to the real application unchanged - see
//! `launch::run` in `main.rs` for the hand-rolled launch sequence this
//! requires (dioxus_native::launch_cfg doesn't expose a hook for this).

use std::path::PathBuf;

use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone)]
pub enum DropEvent {
    /// A file/folder is being dragged over the window (not yet dropped).
    Hovering,
    HoverCancelled,
    Dropped(PathBuf),
}

/// Set once in `main()` before the event loop starts; drained by a
/// spawned Dioxus task in `App` (see `main.rs`). A plain global rather
/// than threaded through `AppState` because it has to exist before the
/// component tree (and therefore `AppState`) does - `main()` builds the
/// window/event loop first, the same as the code it replaces.
pub static DROP_EVENTS: std::sync::OnceLock<Mutex<mpsc::UnboundedReceiver<DropEvent>>> = std::sync::OnceLock::new();

pub struct DragDropApplication {
    pub inner: dioxus::native::DioxusNativeApplication,
    pub drop_tx: mpsc::UnboundedSender<DropEvent>,
}

impl winit::application::ApplicationHandler<blitz_shell::BlitzShellEvent> for DragDropApplication {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.resumed(event_loop);
    }

    fn suspended(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }

    fn new_events(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, cause: winit::event::StartCause) {
        self.inner.new_events(event_loop, cause);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        match &event {
            winit::event::WindowEvent::DroppedFile(path) => {
                let _ = self.drop_tx.send(DropEvent::Dropped(path.clone()));
            }
            winit::event::WindowEvent::HoveredFile(_) => {
                let _ = self.drop_tx.send(DropEvent::Hovering);
            }
            winit::event::WindowEvent::HoveredFileCancelled => {
                let _ = self.drop_tx.send(DropEvent::HoverCancelled);
            }
            _ => {}
        }
        // Always still forward to the real application - this is purely an
        // additional tap on the event stream, never a replacement for
        // normal rendering/input handling.
        self.inner.window_event(event_loop, window_id, event);
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: blitz_shell::BlitzShellEvent) {
        self.inner.user_event(event_loop, event);
    }
}
