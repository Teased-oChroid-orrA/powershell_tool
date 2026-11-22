//! The native window: winit events in, egui chrome + scene out. All state machines live in pure modules
//! (`gui`, `controller`, `menus`, editor sessions); this file maps winit input to them, runs the egui frame
//! and draws the scene into the rectangle the chrome leaves free.
use super::{
    bench::Bench,
    controller::{Command, LayoutOp, PanGesture, Pointer, UiAction, ViewState},
    display::DisplaySettings,
    editor::{
        authoring::Authoring,
        camera::Camera,
        session::{self, KeyResult},
    },
    gui::{Gui, PxRect},
    menu::Outcome,
    menus::{self, MenuContext},
    render::{DisplayOptions, Renderer},
    results::GuiInput,
    scene::ViewerProject,
    Scene,
};
use std::{sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowId},
};

mod actions;
mod frame;
mod handler;
mod input;
mod launch;
mod nav;
mod pointer;
mod sync;
#[cfg(test)]
mod tests;
pub use launch::*;
use nav::Nav;

/// Window input reduced to what the viewer acts on, so the whole path (chrome, editor, camera) can be driven
/// without a window: `window_event` converts winit events, tests feed these directly.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Input {
    Cursor((f64, f64)),
    Press(MouseButton),
    Release(MouseButton),
    /// Wheel lines (positive zooms in).
    Wheel(f32),
    Key(Key),
    Modifiers(ModifiersState),
}

struct Viewer {
    project: ViewerProject,
    view: ViewState,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    renderer: Option<Renderer>,
    config: Option<wgpu::SurfaceConfiguration>,
    last: Instant,
    next_redraw: Instant,
    occluded: bool,
    pointer: Pointer,
    mods: ModifiersState,
    error: Option<String>,
    gui: Gui,
    /// Feeds winit events to egui (absent in tests, which queue `test_events` instead).
    egui_state: Option<egui_winit::State>,
    /// A left press that egui took (a button, a dock): its release is not a model click either.
    egui_press: bool,
    quit: bool,
    /// Unsaved changes were reported once; asking to close again discards them.
    quit_warned: bool,
    /// Window size used when there is no surface (tests).
    size_override: Option<(u32, u32)>,
    /// A middle or right press in the viewport that may become a pan.
    pan_gesture: Option<(MouseButton, PanGesture)>,
    /// A left-button gesture started on something the editor moves (a sketch point), so it does not orbit.
    grabbing: bool,
    /// Events for the next egui frame when there is no window (tests).
    #[cfg_attr(not(test), allow(dead_code))]
    test_events: Vec<egui::Event>,
    #[cfg_attr(not(test), allow(dead_code))]
    test_time: f64,
    /// When the learned layout was last written.
    layout_flushed: Instant,
    recovery_flushed: Instant,
    /// Where the display style and background persist (`DisplaySettings::path`); `None` in tests.
    display_file: Option<std::path::PathBuf>,
    /// Interactive model editor, when this window was opened as one.
    editor: Option<Box<dyn Authoring>>,
    /// This window is the Toolbench dashboard: no model, just the list of workbenches.
    dashboard: Option<super::dashboard::Dashboard>,
    /// The dashboard window as a shell: workbenches opened and parked in it (ADR-024). `None` in a workbench's own window.
    nav: Option<Nav>,
}

/// winit key to editor key (letters keep their case; unsupported keys are `None`).
fn editor_key(key: &Key) -> Option<session::Key> {
    match key {
        Key::Named(NamedKey::Enter) => Some(session::Key::Enter),
        Key::Named(NamedKey::Escape) => Some(session::Key::Escape),
        Key::Named(NamedKey::Backspace) => Some(session::Key::Backspace),
        Key::Named(NamedKey::Delete) => Some(session::Key::Delete),
        Key::Named(NamedKey::ArrowUp) => Some(session::Key::Up),
        Key::Named(NamedKey::ArrowDown) => Some(session::Key::Down),
        Key::Named(NamedKey::ArrowLeft) => Some(session::Key::Left),
        Key::Named(NamedKey::ArrowRight) => Some(session::Key::Right),
        Key::Named(NamedKey::Tab) => Some(session::Key::Tab),
        Key::Named(NamedKey::Space) => Some(session::Key::Char(' ')),
        Key::Character(c) if c.chars().count() == 1 => c.chars().next().map(session::Key::Char),
        _ => None,
    }
}

/// Ctrl/Cmd+K, Ctrl/Cmd+Shift+P and F10 open the command palette.
fn opens_palette(key: &Key, mods: ModifiersState) -> bool {
    let command = mods.control_key() || mods.super_key();
    match key {
        Key::Named(NamedKey::F10) => true,
        Key::Character(c) if command && c.eq_ignore_ascii_case("k") => true,
        Key::Character(c) if command && mods.shift_key() && c.eq_ignore_ascii_case("p") => true,
        _ => false,
    }
}

/// Ctrl/Cmd+J opens the workbench switcher of the dashboard window.
fn opens_switcher(key: &Key, mods: ModifiersState) -> bool {
    matches!(key, Key::Character(c) if (mods.control_key() || mods.super_key()) && c.eq_ignore_ascii_case("j"))
}

impl Viewer {
    pub(super) fn scene(&self) -> &Scene {
        &self.project.scenes[self.project.selected]
    }
    pub(super) fn select_result(&mut self, selected: usize) -> Result<(), String> {
        let selected = selected.min(self.project.scenes.len() - 1);
        let scene = &self.project.scenes[selected];
        if let Some(renderer) = &mut self.renderer {
            renderer.set_scene(scene)?;
        }
        self.project.selected = selected;
        self.view.enter_scene(&self.project.scenes[selected]);
        self.error = None;
        Ok(())
    }
    pub(super) fn switch_mode(&mut self, forward: bool) -> Result<(), String> {
        let next = ViewState::cycle(self.project.selected, self.project.scenes.len(), forward);
        self.select_result(next)
    }
    pub(super) fn window_size(&self) -> (u32, u32) {
        self.size_override
            .or_else(|| {
                self.config
                    .as_ref()
                    .map(|c| (c.width.max(1), c.height.max(1)))
            })
            .unwrap_or((1, 1))
    }
    pub(super) fn viewport(&self) -> PxRect {
        self.gui.viewport_px(self.window_size())
    }
    /// Camera for the viewport (aspect of the region the scene is drawn into).
    pub(super) fn camera(&self) -> Camera {
        let vp = self.viewport();
        Camera {
            yaw: self.view.yaw,
            pitch: self.view.pitch,
            zoom: self.view.zoom,
            aspect: vp.w.max(1) as f32 / vp.h.max(1) as f32,
            pan: self.view.pan,
        }
    }
    /// One unit of window input.
    /// The editor is a bench without a model (Search): its page is all egui, there is no scene to orbit or pick.
    pub(super) fn page_mode(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.has_document())
    }
}
