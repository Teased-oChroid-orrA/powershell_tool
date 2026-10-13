//! The native window: winit events in, egui chrome + scene out. All state machines live in pure modules
//! (`gui`, `controller`, `menus`, editor sessions); this file maps winit input to them, runs the egui frame
//! and draws the scene into the rectangle the chrome leaves free.
use super::{
    bench::Bench,
    controller::{Command, LayoutOp, PanGesture, Pointer, UiAction, ViewState},
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
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowId},
};

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
    /// Interactive model editor, when this window was opened as one.
    editor: Option<Box<dyn Authoring>>,
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

impl Viewer {
    fn scene(&self) -> &Scene {
        &self.project.scenes[self.project.selected]
    }
    fn select_result(&mut self, selected: usize) -> Result<(), String> {
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
    fn switch_mode(&mut self, forward: bool) -> Result<(), String> {
        let next = ViewState::cycle(self.project.selected, self.project.scenes.len(), forward);
        self.select_result(next)
    }
    fn window_size(&self) -> (u32, u32) {
        self.size_override.or_else(|| self.config.as_ref().map(|c| (c.width.max(1), c.height.max(1)))).unwrap_or((1, 1))
    }
    fn viewport(&self) -> PxRect {
        self.gui.viewport_px(self.window_size())
    }
    /// Camera for the viewport (aspect of the region the scene is drawn into).
    fn camera(&self) -> Camera {
        let vp = self.viewport();
        Camera { yaw: self.view.yaw, pitch: self.view.pitch, zoom: self.view.zoom, aspect: vp.w.max(1) as f32 / vp.h.max(1) as f32, pan: self.view.pan }
    }
    /// Carry out what a ribbon button, tree row, menu entry or property row asks for.
    fn dispatch(&mut self, action: UiAction) {
        match action {
            UiAction::Command(c) => self.command(c),
            UiAction::CycleResult { forward } => {
                if let Err(err) = self.switch_mode(forward) {
                    self.error = Some(err);
                }
            }
            UiAction::SelectResult(i) => {
                if let Err(err) = self.select_result(i) {
                    self.error = Some(err);
                }
            }
            UiAction::SwitchBench(b) => self.switch_bench(b),
            UiAction::OpenPalette => self.gui.open_palette(),
            UiAction::Editor(a) => {
                if let Some(ed) = &mut self.editor {
                    ed.act(a);
                }
            }
            UiAction::TogglePanel(p) => self.gui.toggle(p),
            UiAction::Layout(op) => self.layout_op(op),
            UiAction::Quit => self.request_quit(),
        }
    }
    /// Window > Adaptive layout, What has the layout learned?, Reset learned layout, the tree.
    fn layout_op(&mut self, op: LayoutOp) {
        let say = |v: &mut Viewer, text: String| match &mut v.editor {
            Some(ed) => ed.set_status(text),
            None => v.error = Some(text),
        };
        match op {
            LayoutOp::ToggleAdaptive => {
                let on = !self.gui.adapt_enabled();
                self.gui.set_adaptive(on);
                say(self, if on { "Adaptive layout on: tools, cards and the tree follow how you work.".into() } else { "Adaptive layout off: the fixed layout is used.".into() });
            }
            LayoutOp::Reset => {
                self.gui.reset_layout();
                say(self, "Learned layout forgotten: back to the fixed layout.".into());
            }
            LayoutOp::Explain => {
                let text = self.gui.adapt_summary();
                say(self, text);
            }
            LayoutOp::ToggleTree => self.gui.toggle_tree_open(),
        }
    }
    /// Open another bench: solid benches share the session (the model is kept); crossing between the member
    /// and solid models loads the other model's file beside the current one, and is refused while unsaved.
    fn switch_bench(&mut self, bench: Bench) {
        let Some(ed) = &mut self.editor else { return };
        if ed.bench() == bench {
            return;
        }
        if !ed.switch_bench(bench) {
            if ed.dirty() {
                ed.set_status(format!("Save the {} model first (Ctrl/Cmd+S) before opening the {} bench.", ed.bench().label(), bench.label()));
                return;
            }
            let dir = ed.path().parent().map_or_else(std::path::PathBuf::new, std::path::Path::to_path_buf);
            match super::editor::open(bench, &super::editor::bench_file(&dir, bench)) {
                Ok(new) => self.editor = Some(new),
                Err(e) => {
                    self.error = Some(e);
                    return;
                }
            }
        }
        let Some(ed) = &mut self.editor else { return };
        if let Some(v) = ed.initial_view() {
            (self.view.yaw, self.view.pitch) = v.angles();
        }
        self.view.show_mesh = ed.default_mesh_lines();
        ed.set_status(format!("{} bench: {}", bench.label(), bench.summary()));
        match ed.scene() {
            Ok(scene) => {
                if let Some(r) = &mut self.renderer {
                    if let Err(e) = r.set_scene(&scene) {
                        self.error = Some(e);
                        return;
                    }
                }
                self.view.enter_scene(&scene);
                self.project.scenes[0] = scene;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn handle(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Action(a) => self.dispatch(a),
            Outcome::Blocked(reason) => match &mut self.editor {
                Some(ed) => ed.set_status(reason),
                None => self.error = Some(reason),
            },
        }
    }
    fn menu_context(&self) -> MenuContext<'_> {
        MenuContext {
            view: &self.view,
            scene: self.scene(),
            results: if self.editor.is_some() { 1 } else { self.project.scenes.len() },
            editor: self.editor.as_deref(),
            visible: self.gui.visible,
            adaptive: self.gui.adapt_enabled(),
        }
    }
    /// The viewport-relative pixel of a window pixel, if it is inside the viewport and not under a panel or popup.
    fn viewport_point(&self, at: (f64, f64)) -> Option<(f64, f64)> {
        let v = self.viewport();
        (v.contains(at.0, at.1) && !self.gui.wants_pointer()).then_some((at.0 - v.x as f64, at.1 - v.y as f64))
    }
    /// A press released without dragging: in the editor it is a tool click on the model.
    fn model_click(&mut self, at: (f64, f64)) {
        let Some(local) = self.viewport_point(at) else { return };
        let vp = self.viewport();
        let cam = self.camera();
        let additive = self.mods.shift_key();
        if let Some(ed) = &mut self.editor {
            ed.click_with([local.0 as f32, local.1 as f32], [vp.w as f32, vp.h as f32], &cam, additive);
            if let Some(items) = ed.take_popup() {
                self.gui.open_context(at, items);
            }
        }
    }
    /// Left press over the model: does the editor take the gesture (drag a sketch point)?
    fn begin_grab(&mut self, at: (f64, f64)) -> bool {
        let Some(local) = self.viewport_point(at) else { return false };
        let vp = self.viewport();
        let cam = self.camera();
        self.editor.as_mut().is_some_and(|ed| ed.grab([local.0 as f32, local.1 as f32], [vp.w as f32, vp.h as f32], &cam))
    }
    /// Cursor moved: a grabbed entity follows it, otherwise the editor may show a hover preview.
    fn editor_pointer(&mut self, at: (f64, f64)) {
        if self.editor.is_none() || self.gui.floating_open() {
            return;
        }
        let vp = self.viewport();
        let cam = self.camera();
        let grabbing = self.grabbing;
        let Some(local) = self.viewport_point(at).or_else(|| grabbing.then_some((at.0 - vp.x as f64, at.1 - vp.y as f64))) else { return };
        let (px, size) = ([local.0 as f32, local.1 as f32], [vp.w as f32, vp.h as f32]);
        if let Some(ed) = &mut self.editor {
            if grabbing {
                ed.drag(px, size, &cam);
            } else {
                ed.hover(px, size, &cam);
            }
        }
    }
    /// A Shift-drag released over the viewport: box-select in the editor (Ctrl/Cmd adds to the selection).
    fn model_box(&mut self, a: (f64, f64), b: (f64, f64)) {
        let vp = self.viewport();
        let local = |p: (f64, f64)| [(p.0 - vp.x as f64).clamp(0.0, vp.w as f64) as f32, (p.1 - vp.y as f64).clamp(0.0, vp.h as f64) as f32];
        let additive = self.mods.control_key() || self.mods.super_key();
        let cam = self.camera();
        if let Some(ed) = &mut self.editor {
            ed.box_select(local(a), local(b), [vp.w as f32, vp.h as f32], &cam, additive);
        }
    }
    /// Right press in the viewport: select what is under the cursor and open the context menu there.
    fn open_context_menu(&mut self, at: (f64, f64)) {
        let Some(local) = self.viewport_point(at) else { return };
        let vp = self.viewport();
        let cam = self.camera();
        if let Some(ed) = &mut self.editor {
            ed.select_at([local.0 as f32, local.1 as f32], [vp.w as f32, vp.h as f32], &cam);
        }
        let items = menus::context(&self.menu_context());
        self.gui.open_context(at, items);
    }
    /// Keep the GPU scene in step with the editor state (rebuild only when it changed).
    fn sync_editor_scene(&mut self) {
        let Some(ed) = &mut self.editor else { return };
        if let Some(view) = ed.take_view() {
            let scene = &self.project.scenes[self.project.selected];
            self.view.apply(Command::ResetCamera, scene);
            self.view.apply(Command::StandardView(view), scene);
        }
        if !ed.needs_scene() {
            return;
        }
        match ed.scene() {
            Ok(scene) => {
                let had_results = !self.project.scenes[0].fields.is_empty();
                if let Some(r) = &mut self.renderer {
                    if let Err(e) = r.set_scene(&scene) {
                        self.error = Some(e);
                        return;
                    }
                }
                if had_results == scene.fields.is_empty() {
                    self.view.gain = scene.gain;
                    self.view.field = 0;
                }
                self.project.scenes[0] = scene;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
    /// Run one egui frame from the current state and carry out what was clicked; returns what to paint.
    fn run_gui(&mut self, raw: egui::RawInput) -> egui::FullOutput {
        let vp = self.viewport();
        let cam = self.camera();
        if let Some(ed) = &mut self.editor {
            ed.set_view(&cam, [vp.w as f32, vp.h as f32]);
        }
        let overlay = self.editor.as_ref().map(|e| e.overlay(&cam, [vp.w as f32, vp.h as f32])).unwrap_or_default();
        let input = GuiInput {
            view: &self.view,
            scenes: &self.project.scenes,
            selected: self.project.selected,
            editor: self.editor.as_deref(),
            error: self.error.as_deref(),
            box_drag: self.pointer.box_drag(),
            overlay: &overlay,
        };
        let (full, outcomes) = self.gui.run(raw, &input);
        for o in outcomes {
            self.handle(o);
        }
        full
    }
    /// Close the window, unless the editor holds unsaved changes: the first request only says so.
    fn request_quit(&mut self) {
        match &mut self.editor {
            Some(ed) if ed.dirty() && !self.quit_warned => {
                self.quit_warned = true;
                ed.set_status("Unsaved changes: save with Ctrl/Cmd+S, or close again to discard them.".into());
            }
            _ => self.quit = true,
        }
    }
    /// Shift the view by a cursor offset in window pixels.
    fn pan_by_pixels(&mut self, (dx, dy): (f64, f64)) {
        let vp = self.viewport();
        self.command(Command::Pan { dx: (2.0 * dx / vp.w.max(1) as f64) as f32, dy: (-2.0 * dy / vp.h.max(1) as f64) as f32 });
    }
    /// Wheel zoom that keeps the model point under the cursor where it is, so a small feature can be approached.
    fn zoom_at(&mut self, lines: f32, at: (f64, f64)) {
        let vp = self.viewport();
        let (old_zoom, old_pan) = (self.view.zoom, self.view.pan);
        self.command(Command::Zoom(lines));
        let k = self.view.zoom / old_zoom;
        let c = [(2.0 * (at.0 - vp.x as f64) / vp.w.max(1) as f64 - 1.0) as f32, (1.0 - 2.0 * (at.1 - vp.y as f64) / vp.h.max(1) as f64) as f32];
        let pan = [c[0] - (c[0] - old_pan[0]) * k, c[1] - (c[1] - old_pan[1]) * k];
        self.command(Command::Pan { dx: pan[0] - self.view.pan[0], dy: pan[1] - self.view.pan[1] });
    }
    fn command(&mut self, command: Command) {
        let scene = &self.project.scenes[self.project.selected];
        self.view.apply(command, scene);
    }

    /// One unit of window input.
    pub(crate) fn input(&mut self, input: Input) {
        match input {
            Input::Modifiers(m) => self.mods = m,
            Input::Key(k) => self.key_pressed(&k),
            Input::Press(MouseButton::Left) => {
                let Some(at) = self.pointer.cursor else { return };
                // egui takes the press when it is over a panel, a button or a popup; a popup also closes on a click
                // anywhere else, and that click is not a model click.
                let mut consumed = self.gui.wants_pointer();
                if !consumed && self.gui.close_floating() {
                    consumed = true;
                }
                self.egui_press = consumed;
                self.grabbing = !consumed && !self.mods.shift_key() && self.begin_grab(at);
                // Shift-drag in an editor is a selection box; elsewhere a drag orbits.
                self.pointer.press_as(consumed || self.grabbing, self.editor.is_some() && self.mods.shift_key());
            }
            Input::Release(MouseButton::Left) => {
                if std::mem::take(&mut self.egui_press) {
                    self.pointer.release();
                    return;
                }
                if std::mem::take(&mut self.grabbing) {
                    if let Some(ed) = &mut self.editor {
                        ed.release_grab();
                    }
                    self.pointer.release();
                    return;
                }
                let dragged_box = self.pointer.box_drag();
                if let Some(click) = self.pointer.release() {
                    self.model_click(click);
                } else if let Some((a, b)) = dragged_box {
                    self.model_box(a, b);
                }
            }
            // Right: a drag pans, a click (release without travel) opens the context menu. Opening on the
            // release keeps the press egui is still handling from closing the new menu at once.
            Input::Press(button @ (MouseButton::Right | MouseButton::Middle)) => {
                self.pan_gesture = self.pointer.cursor.filter(|&at| self.viewport_point(at).is_some()).map(|at| (button, PanGesture::begin(at, button == MouseButton::Middle)));
            }
            Input::Release(button @ (MouseButton::Right | MouseButton::Middle)) => {
                if let Some((started, gesture)) = self.pan_gesture.take_if(|(b, _)| *b == button) {
                    if started == MouseButton::Right && gesture.is_click() {
                        if let Some(at) = self.pointer.cursor {
                            self.open_context_menu(at);
                        }
                    }
                }
            }
            Input::Press(_) | Input::Release(_) => {}
            Input::Cursor(at) => {
                let from = self.pointer.cursor;
                if let Some(orbit) = self.pointer.moved(at) {
                    self.command(orbit);
                }
                if let (Some(from), Some((_, gesture))) = (from, self.pan_gesture.as_mut()) {
                    if let Some(delta) = gesture.moved(from, at) {
                        self.pan_by_pixels(delta);
                    }
                }
                self.editor_pointer(at);
            }
            Input::Wheel(d) => {
                let cursor = self.pointer.cursor.filter(|&at| self.viewport_point(at).is_some());
                if let Some(at) = cursor {
                    self.zoom_at(d, at);
                }
            }
        }
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let window = Arc::new(event_loop.create_window(Window::default_attributes().with_title("FEA GPU viewport")).map_err(|e| e.to_string())?);
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|e| format!("no compatible GPU adapter: {e}"))?;
        let size = window.inner_size();
        let config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).ok_or("surface has no supported configuration")?;
        let renderer = pollster::block_on(Renderer::new(&adapter, config.format, self.scene(), config.width, config.height))?;
        surface.configure(&renderer.device, &config);
        self.egui_state = Some(egui_winit::State::new(self.gui.ctx().clone(), egui::ViewportId::ROOT, &*window, Some(window.scale_factor() as f32), window.theme(), Some(renderer.device.limits().max_texture_dimension_2d as usize)));
        self.window = Some(window);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.config = Some(config);
        self.last = Instant::now();
        // Companion-process readiness handshake: the controller keeps launch progress visible until here.
        println!("FEA_GPU_READY");
        Ok(())
    }

    fn key_pressed(&mut self, key: &Key) {
        // While a number is being typed every key belongs to the prompt; otherwise Ctrl/Cmd+K opens the palette.
        let typing = self.editor.as_ref().is_some_and(|ed| ed.prompting());
        if !typing && opens_palette(key, self.mods) {
            self.gui.open_palette();
            return;
        }
        if matches!(key, Key::Named(NamedKey::Escape)) && self.gui.close_floating() {
            return;
        }
        // A text field (the palette's query) owns the keyboard.
        if self.gui.wants_keyboard() {
            return;
        }
        if self.editor.is_some() {
            if let Some(k) = editor_key(key) {
                let mods = session::Mods { command: self.mods.control_key() || self.mods.super_key(), shift: self.mods.shift_key() };
                // While a number is being typed every key belongs to the prompt.
                let result = self.editor.as_mut().map(|ed| ed.key(k, mods)).unwrap_or(KeyResult::Unhandled);
                match result {
                    KeyResult::Handled => return,
                    KeyResult::Command(c) => {
                        self.command(c);
                        return;
                    }
                    KeyResult::Unhandled => {}
                }
            }
            // Editor mode: arrows edit properties, so only the remaining viewer keys fall through.
            if matches!(key, Key::Named(NamedKey::Escape | NamedKey::ArrowUp | NamedKey::ArrowDown | NamedKey::Tab)) {
                return;
            }
        }
        match key {
            Key::Named(NamedKey::Escape) => self.request_quit(),
            Key::Named(NamedKey::Space) => self.command(Command::TogglePause),
            Key::Named(NamedKey::Home) => self.command(Command::Rewind),
            Key::Named(NamedKey::Tab) => {
                if let Err(err) = self.switch_mode(true) {
                    self.error = Some(err);
                }
            }
            Key::Character(c) if c == "[" || c == "]" => {
                if let Err(err) = self.switch_mode(c == "]") {
                    self.error = Some(err);
                }
            }
            Key::Character(c) if c.eq_ignore_ascii_case("m") => self.command(Command::ToggleMesh),
            Key::Character(c) if c.eq_ignore_ascii_case("c") => self.command(Command::ToggleContour),
            Key::Character(c) if c.eq_ignore_ascii_case("d") => self.command(Command::ToggleDeformation),
            Key::Character(c) if c.eq_ignore_ascii_case("r") => self.command(Command::ResetCamera),
            Key::Character(c) if c.eq_ignore_ascii_case("f") => self.command(Command::CycleField { forward: c != "F" }),
            Key::Character(c) if c.eq_ignore_ascii_case("h") || c == "?" => self.command(Command::ToggleHelp),
            Key::Named(NamedKey::ArrowRight) => self.command(Command::StepFrame { forward: true }),
            Key::Named(NamedKey::ArrowLeft) => self.command(Command::StepFrame { forward: false }),
            Key::Named(NamedKey::ArrowUp) => self.command(Command::GainUp),
            Key::Named(NamedKey::ArrowDown) => self.command(Command::GainDown),
            Key::Character(c) if c == "+" || c == "=" => self.command(Command::SpeedUp),
            Key::Character(c) if c == "-" => self.command(Command::SpeedDown),
            _ => {}
        }
    }

    /// Draw one frame: advance playback, run the chrome, draw the scene into the free rectangle, paint the chrome.
    fn redraw(&mut self, e: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        let scene = &self.project.scenes[self.project.selected];
        self.view.advance(scene, dt);
        self.sync_editor_scene();
        let (Some(window), Some(state)) = (self.window.clone(), self.egui_state.as_mut()) else { return };
        let raw = state.take_egui_input(&window);
        let full = self.run_gui(raw);
        if let Some(state) = self.egui_state.as_mut() {
            state.handle_platform_output(&window, full.platform_output.clone());
        }
        let vp = self.viewport();
        let size = self.window_size();
        let background = self.gui.background_rgb();
        // An empty model has nothing to draw (the start card says how to begin): clear the viewport only.
        let empty = self.editor.as_ref().is_some_and(|e| !e.start_options().is_empty());
        let scene_rect = if empty { [vp.x, vp.y, 0, 0] } else { [vp.x, vp.y, vp.w, vp.h] };
        let ctx = self.gui.ctx().clone();
        let mut full = Some(full);
        if let (Some(s), Some(r), Some(c)) = (&self.surface, &mut self.renderer, &self.config) {
            match s.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    let view = frame.texture.create_view(&Default::default());
                    r.draw_display(
                        &view,
                        &self.project.scenes[self.project.selected],
                        self.view.time,
                        if self.view.deformed { self.view.gain } else { 0.0 },
                        [self.view.yaw, self.view.pitch, self.view.zoom, vp.w.max(1) as f32 / vp.h.max(1) as f32],
                        DisplayOptions { show_mesh: self.view.show_mesh, show_contour: self.view.show_contour, field: self.view.field, viewport: Some(scene_rect), background: Some(background), pan: self.view.pan },
                    );
                    r.paint_egui(&view, &ctx, full.take().expect("painted once"), [size.0, size.1]);
                    r.queue.present(frame);
                }
                wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => s.configure(&r.device, c),
                // Skip the frame and retry: timeouts and occlusion are transient, and wgpu reports a
                // surface validation error through its uncaptured-error handler.
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Validation => {}
            }
            // A frame that was not shown still has texture changes the renderer must see.
            if let Some(unshown) = full.take() {
                r.discard_egui(unshown);
            }
        }
        if let Some(unused) = full.take() {
            unused.drop_without_applying_deltas();
        }
        if let Some(w) = &self.window {
            w.set_title(&match &self.editor {
                Some(ed) => ed.title(),
                None => format!("FEA GPU viewport | result {}/{}", self.project.selected + 1, self.project.scenes.len()),
            });
        }
        if self.quit {
            e.exit();
        }
    }
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, e: &ActiveEventLoop) {
        if self.window.is_none() {
            if let Err(err) = self.init(e) {
                self.error = Some(err);
                e.exit();
            }
        }
    }
    fn window_event(&mut self, e: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        // egui sees every event first (pointer position, text, scrolling); what it does not use reaches the model.
        if let (Some(state), Some(window)) = (self.egui_state.as_mut(), self.window.as_ref()) {
            let _ = state.on_window_event(window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.request_quit();
                if self.quit {
                    e.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let (Some(c), Some(r), Some(s)) = (&mut self.config, &mut self.renderer, &self.surface) {
                        c.width = size.width;
                        c.height = size.height;
                        s.configure(&r.device, c);
                        r.resize(size.width, size.height);
                    }
                }
            }
            WindowEvent::ModifiersChanged(m) => self.input(Input::Modifiers(m.state())),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                // Alt composes characters on some platforms (Alt+F is a different glyph on macOS): use the plain key.
                let key = if self.mods.alt_key() { winit::platform::modifier_supplement::KeyEventExtModifierSupplement::key_without_modifiers(&event) } else { event.logical_key };
                self.input(Input::Key(key))
            }
            WindowEvent::MouseInput { state, button, .. } => self.input(if state == ElementState::Pressed { Input::Press(button) } else { Input::Release(button) }),
            WindowEvent::CursorMoved { position, .. } => self.input(Input::Cursor((position.x, position.y))),
            WindowEvent::MouseWheel { delta, .. } => self.input(Input::Wheel(match delta {
                MouseScrollDelta::LineDelta(_, y) => y,
                MouseScrollDelta::PixelDelta(p) => p.y as f32 / 100.0,
            })),
            WindowEvent::RedrawRequested => self.redraw(e),
            _ => {}
        }
    }
    fn about_to_wait(&mut self, e: &ActiveEventLoop) {
        if self.quit {
            e.exit();
        }
        self.editor.as_mut().map(|ed| ed.poll());
        if self.layout_flushed.elapsed().as_secs() >= 30 {
            self.layout_flushed = Instant::now();
            self.gui.flush_layout();
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

pub fn run(project: ViewerProject) -> Result<(), String> {
    project.validate()?;
    launch(project, None)
}

/// Open the native member editor on `problem`, saving to `path`.
pub fn run_editor(problem: fea_problem::members::MemberProblem, path: std::path::PathBuf) -> Result<(), String> {
    problem.validate()?;
    let mut ed = super::editor::member_session(problem, path);
    let scene = ed.scene()?;
    launch(ViewerProject::single(scene), Some(Box::new(ed)))
}

/// Open the native solid editor on `problem`, saving to `path`.
pub fn run_solid(problem: fea_problem::Problem, path: std::path::PathBuf) -> Result<(), String> {
    let mut ed = super::editor::solid::SolidSession::new(problem, path);
    let scene = super::editor::authoring::Authoring::scene(&mut ed)?;
    launch(ViewerProject::single(scene), Some(Box::new(ed)))
}

fn new_viewer(project: ViewerProject, editor: Option<Box<dyn Authoring>>) -> Viewer {
    let mut view = ViewState::new(&project.scenes[project.selected]);
    view.show_mesh = editor.as_ref().is_some_and(|e| e.default_mesh_lines());
    if let Some(v) = editor.as_ref().and_then(|e| e.initial_view()) {
        (view.yaw, view.pitch) = v.angles();
    }
    Viewer {
        project,
        view,
        window: None,
        surface: None,
        renderer: None,
        config: None,
        last: Instant::now(),
        pointer: Pointer::default(),
        mods: ModifiersState::default(),
        error: None,
        gui: Gui::new(),
        egui_state: None,
        egui_press: false,
        pan_gesture: None,
        quit: false,
        quit_warned: false,
        size_override: None,
        grabbing: false,
        test_events: Vec::new(),
        test_time: 0.0,
        layout_flushed: Instant::now(),
        editor,
    }
}

fn launch(project: ViewerProject, editor: Option<Box<dyn Authoring>>) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut app = new_viewer(project, editor);
    app.gui.attach_layout_store();
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    if app.editor.as_ref().is_some_and(|ed| ed.dirty()) {
        eprintln!("note: the model had unsaved changes when the window closed");
    }
    app.error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::editor::session::Session;

    #[test]
    fn winit_keys_map_to_editor_keys() {
        assert_eq!(editor_key(&Key::Named(NamedKey::Enter)), Some(session::Key::Enter));
        assert_eq!(editor_key(&Key::Named(NamedKey::Backspace)), Some(session::Key::Backspace));
        assert_eq!(editor_key(&Key::Named(NamedKey::ArrowUp)), Some(session::Key::Up));
        assert_eq!(editor_key(&Key::Named(NamedKey::ArrowLeft)), Some(session::Key::Left));
        assert_eq!(editor_key(&Key::Character("Z".into())), Some(session::Key::Char('Z')));
        assert_eq!(editor_key(&Key::Character("-".into())), Some(session::Key::Char('-')));
        assert_eq!(editor_key(&Key::Character("ab".into())), None, "multi-character input (IME) is not a shortcut");
        assert_eq!(editor_key(&Key::Named(NamedKey::F5)), None);
    }
    #[test]
    fn the_palette_opens_with_ctrl_or_cmd_k_and_f10_only() {
        let none = ModifiersState::default();
        assert!(opens_palette(&Key::Named(NamedKey::F10), none));
        assert!(opens_palette(&Key::Character("k".into()), ModifiersState::CONTROL));
        assert!(opens_palette(&Key::Character("K".into()), ModifiersState::SUPER));
        assert!(!opens_palette(&Key::Character("k".into()), none), "a plain letter belongs to the editor");
        assert!(!opens_palette(&Key::Character("m".into()), ModifiersState::CONTROL));
    }

    // ---- the whole input path without a window ------------------------------------------------

    fn plain_scene() -> Scene {
        Scene {
            label: "t".into(),
            positions: vec![[-0.3, -0.3, 0.0], [0.3, -0.3, 0.0], [0.0, 0.3, 0.0]],
            triangles: vec![0, 1, 2],
            samples: vec![[0.0; 4]; 3],
            times: vec![0.0],
            range: [0.0, 1.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: String::new(),
            fields: Vec::new(),
        }
    }
    fn viewer(editor: Option<Box<dyn Authoring>>) -> Viewer {
        let mut v = new_viewer(ViewerProject::single(plain_scene()), editor);
        v.size_override = Some((1600, 900));
        frame(&mut v);
        v
    }
    fn member_viewer() -> Viewer {
        let ed = Session::new(fea_problem::members::blank_planar(), std::env::temp_dir().join("window-input-test.json"));
        viewer(Some(Box::new(ed)))
    }
    /// One displayed frame: sync the scene and run the chrome twice (the second pass has the measured sizes of the
    /// first, as it does a moment later in the real window), with the queued egui events.
    fn frame(v: &mut Viewer) {
        v.sync_editor_scene();
        for _ in 0..2 {
            let (w, h) = v.window_size();
            v.test_time += 0.016;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))),
                events: std::mem::take(&mut v.test_events),
                time: Some(v.test_time),
                ..Default::default()
            };
            v.run_gui(raw).drop_without_applying_deltas();
        }
    }
    fn pos(at: (f64, f64)) -> egui::Pos2 {
        egui::pos2(at.0 as f32, at.1 as f32)
    }
    fn move_to(v: &mut Viewer, at: (f64, f64)) {
        v.test_events.push(egui::Event::PointerMoved(pos(at)));
        v.input(Input::Cursor(at));
        frame(v);
    }
    fn button(v: &mut Viewer, b: MouseButton, down: bool) {
        let at = v.pointer.cursor.expect("cursor placed");
        let pb = if b == MouseButton::Right { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
        v.test_events.push(egui::Event::PointerButton { pos: pos(at), button: pb, pressed: down, modifiers: Default::default() });
        v.input(if down { Input::Press(b) } else { Input::Release(b) });
    }
    fn click(v: &mut Viewer, at: (f64, f64)) {
        move_to(v, at);
        button(v, MouseButton::Left, true);
        button(v, MouseButton::Left, false);
        frame(v);
    }
    fn key(v: &mut Viewer, k: NamedKey) {
        v.input(Input::Key(Key::Named(k)));
        frame(v);
    }
    /// A key the palette's text field also sees.
    fn egui_key(v: &mut Viewer, k: egui::Key) {
        v.test_events.push(egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
        v.test_events.push(egui::Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: Default::default() });
        frame(v);
    }
    fn type_text(v: &mut Viewer, text: &str) {
        v.test_events.push(egui::Event::Text(text.into()));
        frame(v);
    }
    fn target(v: &Viewer, label: &str) -> (f64, f64) {
        v.gui.find_target(label).unwrap_or_else(|| panic!("no control {label:?} among {:?}", v.gui.target_labels()))
    }
    fn viewport_centre(v: &Viewer) -> (f64, f64) {
        let r = v.viewport();
        ((r.x + r.w / 2) as f64, (r.y + r.h / 2) as f64)
    }
    fn tree_labels(v: &Viewer) -> Vec<String> {
        v.editor.as_ref().unwrap().tree().into_iter().map(|n| n.label).collect()
    }
    fn property_text(v: &Viewer) -> String {
        v.editor.as_ref().unwrap().properties().into_iter().filter_map(|i| i.plain_text()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn a_tool_click_picks_a_tool_and_a_viewport_click_then_uses_it() {
        let mut v = member_viewer();
        assert!(tree_labels(&v).iter().any(|l| l == "Nodes (0)"));
        // Tool pill: Add node. The press is taken by the chrome and never reaches the model.
        let button = target(&v, "Add node");
        click(&mut v, button);
        assert!(property_text(&v).contains("tool Add node"), "{}", property_text(&v));
        assert!(tree_labels(&v).iter().any(|l| l == "Nodes (0)"), "clicking the chrome placed nothing");
        // Viewport: the node tool places a node.
        let at = viewport_centre(&v);
        click(&mut v, at);
        assert!(tree_labels(&v).iter().any(|l| l == "Nodes (1)"), "{:?}", tree_labels(&v));
        // Clicking a dock's empty space does nothing to the model either.
        let vp = v.viewport();
        click(&mut v, (vp.x as f64 - 6.0, (vp.y + vp.h) as f64 - 4.0));
        assert!(tree_labels(&v).iter().any(|l| l == "Nodes (1)"));
    }

    #[test]
    fn right_click_selects_under_the_cursor_and_opens_a_menu_that_closes_on_escape() {
        let mut v = member_viewer();
        let button = target(&v, "Add node");
        click(&mut v, button);
        let at = viewport_centre(&v);
        click(&mut v, at);
        let button = target(&v, "Select");
        click(&mut v, button);
        move_to(&mut v, at);
        v.input(Input::Press(MouseButton::Right));
        v.input(Input::Release(MouseButton::Right));
        frame(&mut v);
        assert!(v.gui.context_open(), "right-click opens the context menu");
        assert!(property_text(&v).contains("x "), "the node under the cursor was selected: {}", property_text(&v));
        // Escape closes the menu and does not quit.
        key(&mut v, NamedKey::Escape);
        assert!(!v.gui.context_open() && !v.quit);
        // A right-click on a dock opens no context menu.
        let dock = target(&v, "Nodes");
        move_to(&mut v, dock);
        v.input(Input::Press(MouseButton::Right));
        v.input(Input::Release(MouseButton::Right));
        assert!(!v.gui.context_open());
    }

    /// The real window gives egui the same secondary press and release the viewer sees; a menu opened on the
    /// press was closed by that press ("pressed outside") when its first frame was not under the cursor.
    #[test]
    fn a_right_click_the_egui_also_sees_leaves_the_menu_open_across_frames_and_positions() {
        for (fx, fy) in [(0.5, 0.5), (0.95, 0.5), (0.5, 0.95), (0.98, 0.98), (0.3, 0.2)] {
            let mut v = member_viewer();
            let vp = v.viewport();
            move_to(&mut v, (vp.x as f64 + fx * vp.w as f64, vp.y as f64 + fy * vp.h as f64));
            button(&mut v, MouseButton::Right, true);
            frame(&mut v);
            button(&mut v, MouseButton::Right, false);
            for _ in 0..3 {
                frame(&mut v);
            }
            assert!(v.gui.context_open(), "menu closed after a right click at {fx},{fy}");
        }
    }

    #[test]
    fn a_middle_or_right_drag_pans_a_right_click_does_not_and_the_wheel_zooms_about_the_cursor() {
        let mut v = member_viewer();
        let vp = v.viewport();
        let start = (vp.x as f64 + 0.5 * vp.w as f64, vp.y as f64 + 0.5 * vp.h as f64);
        // Middle drag: the view follows the cursor (200 px right = 400/width NDC), y is flipped.
        move_to(&mut v, start);
        button(&mut v, MouseButton::Middle, true);
        move_to(&mut v, (start.0 + 100.0, start.1 + 50.0));
        move_to(&mut v, (start.0 + 200.0, start.1 + 100.0));
        button(&mut v, MouseButton::Middle, false);
        let want = [(400.0 / vp.w as f64) as f32, (-200.0 / vp.h as f64) as f32];
        assert!((v.view.pan[0] - want[0]).abs() < 1e-4 && (v.view.pan[1] - want[1]).abs() < 1e-4, "{:?} vs {want:?}", v.view.pan);
        assert!(!v.gui.context_open());
        // Right drag pans from the press point (not from where the 4 px threshold was crossed) and opens no menu.
        v.command(Command::ResetCamera);
        move_to(&mut v, start);
        button(&mut v, MouseButton::Right, true);
        move_to(&mut v, (start.0 + 40.0, start.1));
        button(&mut v, MouseButton::Right, false);
        assert!((v.view.pan[0] - 80.0 / vp.w as f32).abs() < 1e-4, "{:?}", v.view.pan);
        assert!(!v.gui.context_open(), "a drag is not a click");
        // A jitter of 2 px is still a click.
        move_to(&mut v, start);
        button(&mut v, MouseButton::Right, true);
        move_to(&mut v, (start.0 + 2.0, start.1));
        button(&mut v, MouseButton::Right, false);
        frame(&mut v);
        assert!(v.gui.context_open());
        v.gui.close_floating();
        // Wheel zoom keeps the model point under the cursor fixed: ndc = s * zoom + pan.
        v.command(Command::ResetCamera);
        let at = (vp.x as f64 + 0.8 * vp.w as f64, vp.y as f64 + 0.3 * vp.h as f64);
        move_to(&mut v, at);
        let c = [(2.0 * 0.8 - 1.0) as f32, (1.0 - 2.0 * 0.3) as f32];
        let before = [c[0] / v.view.zoom, c[1] / v.view.zoom];
        v.input(Input::Wheel(5.0));
        assert!(v.view.zoom > super::super::controller::DEFAULT_ZOOM);
        let after = [(c[0] - v.view.pan[0]) / v.view.zoom, (c[1] - v.view.pan[1]) / v.view.zoom];
        assert!((before[0] - after[0]).abs() < 1e-4 && (before[1] - after[1]).abs() < 1e-4, "{before:?} {after:?}");
        // R resets zoom and pan.
        v.input(Input::Key(Key::Character("r".into())));
        assert_eq!(v.view.pan, [0.0, 0.0]);
    }

    #[test]
    fn the_palette_finds_a_command_anywhere_runs_it_and_explains_a_disabled_one() {
        let mut v = viewer(None);
        assert!(!v.view.show_mesh);
        v.input(Input::Modifiers(ModifiersState::CONTROL));
        v.input(Input::Key(Key::Character("k".into())));
        v.input(Input::Modifiers(ModifiersState::default()));
        frame(&mut v);
        assert!(v.gui.palette_open());
        // Typed text reaches the query, never the viewer's own shortcuts (M would toggle the mesh).
        type_text(&mut v, "mesh edg");
        assert!(!v.view.show_mesh);
        egui_key(&mut v, egui::Key::Enter);
        assert!(v.view.show_mesh, "Enter ran View: Mesh edges");
        assert!(!v.gui.palette_open(), "choosing a command closes the palette");
        // A disabled command stays findable and says why; the palette stays open.
        v.input(Input::Key(Key::Named(NamedKey::F10)));
        frame(&mut v);
        type_text(&mut v, "next field");
        egui_key(&mut v, egui::Key::Enter);
        assert!(v.gui.palette_open());
        assert_eq!(v.error.as_deref(), Some("this result has a single field"));
    }

    #[test]
    fn escape_closes_the_palette_first_then_quits_a_plain_viewer() {
        let mut v = viewer(None);
        v.input(Input::Key(Key::Named(NamedKey::F10)));
        frame(&mut v);
        assert!(v.gui.palette_open());
        key(&mut v, NamedKey::Escape);
        assert!(!v.gui.palette_open() && !v.quit);
        key(&mut v, NamedKey::Escape);
        assert!(v.quit);
    }

    #[test]
    fn typing_a_number_is_never_hijacked_by_the_palette_or_menu_keys() {
        let mut v = member_viewer();
        let button = target(&v, "Add node");
        click(&mut v, button);
        let at = viewport_centre(&v);
        click(&mut v, at);
        let select = target(&v, "Select");
        click(&mut v, select);
        // Right-click selects the node under the cursor; its first property is then edited with `e`.
        move_to(&mut v, at);
        v.input(Input::Press(MouseButton::Right));
        v.input(Input::Release(MouseButton::Right));
        key(&mut v, NamedKey::Escape);
        v.input(Input::Key(Key::Character("e".into())));
        assert!(v.editor.as_ref().unwrap().prompting());
        // While a number is being typed F10 / Ctrl+K / arrows / Enter belong to the prompt.
        key(&mut v, NamedKey::F10);
        assert!(!v.gui.palette_open());
        v.input(Input::Modifiers(ModifiersState::CONTROL));
        v.input(Input::Key(Key::Character("k".into())));
        v.input(Input::Modifiers(ModifiersState::default()));
        assert!(!v.gui.palette_open());
        v.input(Input::Key(Key::Character("7".into())));
        key(&mut v, NamedKey::Enter);
        assert!(!v.editor.as_ref().unwrap().prompting());
        assert!(property_text(&v).contains("7.000000e0"), "{}", property_text(&v));
    }

    #[test]
    fn the_wheel_zooms_over_the_viewport_and_not_over_a_dock() {
        let mut v = member_viewer();
        let zoom = v.view.zoom;
        let centre = viewport_centre(&v);
        move_to(&mut v, centre);
        v.input(Input::Wheel(2.0));
        assert!(v.view.zoom > zoom);
        let zoom = v.view.zoom;
        let vp = v.viewport();
        move_to(&mut v, (vp.x as f64 - 20.0, (vp.y + 40) as f64)); // the feature tree
        v.input(Input::Wheel(2.0));
        assert_eq!(v.view.zoom, zoom, "a dock consumes the wheel");
    }

    #[test]
    fn dragging_orbits_only_when_the_press_started_in_the_viewport() {
        let mut v = member_viewer();
        let start = viewport_centre(&v);
        move_to(&mut v, start);
        button(&mut v, MouseButton::Left, true);
        move_to(&mut v, (start.0 + 60.0, start.1 + 20.0));
        button(&mut v, MouseButton::Left, false);
        assert!(v.view.yaw != 0.0, "a viewport drag orbits");
        let yaw = v.view.yaw;
        let vp = v.viewport();
        let on_dock = (vp.x as f64 - 20.0, (vp.y + 60) as f64); // the feature tree
        move_to(&mut v, on_dock);
        button(&mut v, MouseButton::Left, true);
        move_to(&mut v, (on_dock.0 - 100.0, on_dock.1 + 40.0));
        button(&mut v, MouseButton::Left, false);
        assert_eq!(v.view.yaw, yaw, "a drag that starts on a dock does not orbit the model");
    }

    #[test]
    fn window_menu_toggles_docks_and_the_viewport_follows() {
        let mut v = viewer(None);
        let before = v.viewport();
        v.dispatch(UiAction::TogglePanel(super::super::controller::Panel::Tree));
        v.dispatch(UiAction::TogglePanel(super::super::controller::Panel::Properties));
        frame(&mut v);
        let after = v.viewport();
        assert!(after.w > before.w);
        assert_eq!(v.camera().aspect, after.w as f32 / after.h as f32, "the camera uses the viewport aspect");
    }

    #[test]
    fn a_solid_model_is_meshed_solved_and_viewed_through_the_chrome() {
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 1.0;
        let ed = crate::gpu_viewer::editor::solid::SolidSession::new(problem, std::env::temp_dir().join("window-solid-test.json"));
        let mut v = viewer(Some(Box::new(ed)));
        let wait = |v: &mut Viewer, done: &dyn Fn(&Viewer) -> bool| {
            let t = Instant::now();
            while !done(v) {
                v.editor.as_mut().unwrap().poll();
                frame(v);
                assert!(t.elapsed().as_secs() < 120, "timed out");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        // Nothing meshes on its own: the viewport says so until the Mesh step's Generate mesh is clicked.
        frame(&mut v);
        std::thread::sleep(std::time::Duration::from_millis(400));
        v.editor.as_mut().unwrap().poll();
        frame(&mut v);
        assert!(v.scene().label.contains("mesh not generated"), "{}", v.scene().label);
        let tab = target(&v, "Mesh tab");
        click(&mut v, tab);
        let generate = target(&v, "Generate mesh");
        click(&mut v, generate);
        // The mesh arrives on a worker and replaces the placeholder scene.
        wait(&mut v, &|v| v.scene().label.contains("geometry"));
        assert!(tree_labels(&v).iter().any(|l| l.starts_with("Mesh: ") && l.contains("nodes")));
        // Simulation step, Solve button.
        let tab = target(&v, "Simulation tab");
        click(&mut v, tab);
        let solve = target(&v, "Solve");
        click(&mut v, solve);
        wait(&mut v, &|v| !v.scene().fields.is_empty());
        assert_eq!(v.scene().fields.len(), 10);
        frame(&mut v);
        assert!(v.gui.target_labels().iter().any(|l| l == "Result card"), "the Simulation step shows the result card: {:?}", v.gui.target_labels());
        // V returns to the editable model.
        v.input(Input::Key(Key::Character("v".into())));
        frame(&mut v);
        assert!(v.scene().fields.is_empty() && v.scene().label.contains("geometry"));
    }

    fn sim_labels(v: &Viewer) -> Vec<String> {
        let ed = v.editor.as_deref().unwrap();
        let m = ed.menus().into_iter().find(|m| m.title == "Simulation").unwrap();
        m.items.into_iter().filter_map(|i| if let crate::gpu_viewer::menu::Item::Entry(e) = i { Some(e.label) } else { None }).collect()
    }

    #[test]
    fn switching_benches_keeps_the_solid_model_and_shows_only_that_benchs_studies() {
        let dir = std::env::temp_dir().join(format!("window-bench-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ed = crate::gpu_viewer::editor::open(Bench::Static, &dir.join("solid-model.json")).unwrap();
        let mut v = viewer(Some(ed));
        assert!(sim_labels(&v).contains(&"Solve".to_string()) && !sim_labels(&v).iter().any(|l| l.contains("mode")));
        v.dispatch(UiAction::SwitchBench(Bench::Modal));
        assert_eq!(v.editor.as_ref().unwrap().bench(), Bench::Modal);
        let labels = sim_labels(&v);
        assert!(labels.contains(&"Run study".to_string()) && !labels.contains(&"Solve".to_string()), "{labels:?}");
        assert!(v.editor.as_ref().unwrap().title().starts_with("Modal"));
        // The Window menu lists every bench with the current one checked.
        let bar = menus::bar(&v.menu_context());
        let window = bar.iter().find(|m| m.title == "Window").unwrap();
        let checked: Vec<_> = window.items.iter().filter_map(|i| if let crate::gpu_viewer::menu::Item::Entry(e) = i { Some((e.label.clone(), e.checked)) } else { None }).filter(|(l, _)| l.starts_with("Bench:")).collect();
        assert_eq!(checked.len(), Bench::ALL.len());
        assert_eq!(checked.iter().filter(|(_, c)| *c == Some(true)).map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Bench: Modal / vibration"]);
        // Crossing to the member model loads its own file; unsaved solid edits block the switch.
        v.editor.as_mut().unwrap().act(session::EditorAction::Solid(crate::gpu_viewer::editor::solid::SolidAction::NextTemplate));
        assert!(v.editor.as_ref().unwrap().dirty());
        v.dispatch(UiAction::SwitchBench(Bench::Members));
        assert_eq!(v.editor.as_ref().unwrap().bench(), Bench::Modal, "refused while unsaved");
        assert!(v.editor.as_ref().unwrap().status().contains("Save"));
        v.editor.as_mut().unwrap().act(session::EditorAction::Solid(crate::gpu_viewer::editor::solid::SolidAction::Save));
        v.dispatch(UiAction::SwitchBench(Bench::Members));
        assert_eq!(v.editor.as_ref().unwrap().bench(), Bench::Members);
        assert!(v.editor.as_ref().unwrap().path().ends_with("member-model.json"));
        // And back: the saved solid model comes back with its template.
        v.dispatch(UiAction::SwitchBench(Bench::Buckling));
        assert_eq!(v.editor.as_ref().unwrap().bench(), Bench::Buckling);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn closing_with_unsaved_changes_warns_once_then_closes() {
        let mut v = member_viewer();
        v.request_quit();
        assert!(v.quit, "a clean model closes at once");
        let mut v = member_viewer();
        let button = target(&v, "Add node");
        click(&mut v, button);
        let at = viewport_centre(&v);
        click(&mut v, at);
        assert!(v.editor.as_ref().unwrap().dirty());
        v.request_quit();
        assert!(!v.quit && v.editor.as_ref().unwrap().status().contains("Unsaved"));
        v.request_quit();
        assert!(v.quit, "asking again discards");
    }

    #[test]
    fn shift_drag_in_the_viewport_box_selects_and_delete_removes_the_group() {
        let mut v = member_viewer();
        let button = target(&v, "Add node");
        click(&mut v, button);
        let (cx, cy) = viewport_centre(&v);
        for dx in [-120.0, 0.0, 120.0] {
            click(&mut v, (cx + dx, cy));
        }
        assert!(tree_labels(&v).iter().any(|l| l == "Nodes (3)"), "{:?}", tree_labels(&v));
        let button = target(&v, "Select");
        click(&mut v, button);
        // Shift-drag a box over the left and middle nodes.
        v.input(Input::Modifiers(ModifiersState::SHIFT));
        move_to(&mut v, (cx - 200.0, cy - 40.0));
        button_press(&mut v);
        move_to(&mut v, (cx + 60.0, cy + 40.0));
        assert!(v.pointer.box_drag().is_some(), "the drag is a box, not an orbit");
        let yaw = v.view.yaw;
        frame(&mut v);
        button_release(&mut v);
        frame(&mut v);
        assert_eq!(v.view.yaw, yaw, "a box drag never orbits");
        assert!(property_text(&v).contains("+1 more selected"), "{}", property_text(&v));
        v.input(Input::Modifiers(ModifiersState::default()));
        key(&mut v, NamedKey::Delete);
        assert!(tree_labels(&v).iter().any(|l| l == "Nodes (1)"), "{:?}", tree_labels(&v));
    }
    fn button_press(v: &mut Viewer) {
        button(v, MouseButton::Left, true);
    }
    fn button_release(v: &mut Viewer) {
        button(v, MouseButton::Left, false);
    }

    #[test]
    fn the_step_strip_follows_the_model_and_a_tab_click_swaps_the_tools() {
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 1.0;
        let ed = crate::gpu_viewer::editor::solid::SolidSession::new(problem, std::env::temp_dir().join("window-steps-test.json"));
        let mut v = viewer(Some(Box::new(ed)));
        let labels = v.gui.target_labels();
        for step in ["Model tab", "Mesh tab", "Simulation tab", "View tab"] {
            assert!(labels.iter().any(|l| l == step), "{step} among {labels:?}");
        }
        assert!(labels.iter().any(|l| l == "New part"), "the Model step's tools are shown first: {labels:?}");
        assert!(!labels.iter().any(|l| l == "Generate mesh"));
        let tab = target(&v, "Mesh tab");
        click(&mut v, tab);
        assert!(v.gui.target_labels().iter().any(|l| l == "Generate mesh"));
        assert!(!v.gui.target_labels().iter().any(|l| l == "New part"), "only the selected step's tools are shown");
        // The Mesh step is still to do; the strip says so through the editor.
        use crate::gpu_viewer::editor::authoring::StepState;
        let ed = v.editor.as_ref().unwrap();
        assert_eq!((ed.step_state("Model"), ed.step_state("Mesh"), ed.step_state("Simulation")), (Some(StepState::Done), Some(StepState::Todo), Some(StepState::Todo)));
        assert_eq!(ed.step_state("View"), None);
    }

    #[test]
    fn hovering_a_tool_shows_what_it_does_in_the_status_bar() {
        let mut v = member_viewer();
        let button = target(&v, "Add node");
        move_to(&mut v, button);
        move_to(&mut v, (button.0 + 1.0, button.1));
        assert!(v.gui.hint().is_some_and(|h| h.starts_with("Add node")), "{:?}", v.gui.hint());
        let centre = viewport_centre(&v);
        move_to(&mut v, centre);
        assert_eq!(v.gui.hint(), None);
    }

    #[test]
    fn an_empty_model_offers_a_way_to_begin_and_a_template_click_starts_one() {
        use crate::gpu_viewer::editor::solid::{SolidAction, SolidSession};
        let (title, problem) = fea_problem::templates::templates().remove(0);
        let mut ed = SolidSession::new(problem, std::env::temp_dir().join("window-empty-test.json"));
        assert!(Authoring::start_options(&ed).is_empty(), "a model with geometry needs no starting card");
        ed.act(session::EditorAction::Solid(SolidAction::ClearModel));
        assert_eq!(Authoring::start_options(&ed).len(), 2 + fea_problem::templates::templates().len(), "new part, every template, open file");
        let mut v = viewer(Some(Box::new(ed)));
        let labels = v.gui.target_labels();
        assert!(labels.iter().any(|l| l.starts_with("Open problem")) && labels.iter().any(|l| l == title), "{labels:?}");
        let card = target(&v, title);
        click(&mut v, card);
        assert!(v.editor.as_ref().unwrap().status().contains("Started from template"), "{}", v.editor.as_ref().unwrap().status());
        assert!(!v.gui.target_labels().iter().any(|l| l.starts_with("Open problem")), "the card goes away once there is a model");
    }

    fn labels(v: &Viewer) -> Vec<String> {
        v.gui.target_labels()
    }

    #[test]
    fn the_inspector_floats_with_what_matters_closes_for_a_selection_and_pins() {
        let mut v = member_viewer();
        // Nothing selected: no selection card, only the cards for the step.
        assert!(!labels(&v).iter().any(|l| l == "Node 1 card"));
        assert!(labels(&v).iter().any(|l| l == "Modelling card"), "{:?}", labels(&v));
        let close = target(&v, "Close the inspector");
        click(&mut v, close);
        assert!(!labels(&v).iter().any(|l| l == "Modelling card"), "closed");
        // A new selection brings it back.
        let add = target(&v, "Add node");
        click(&mut v, add);
        let at = viewport_centre(&v);
        click(&mut v, at);
        let select = target(&v, "Select");
        click(&mut v, select);
        move_to(&mut v, at);
        v.input(Input::Press(MouseButton::Right));
        v.input(Input::Release(MouseButton::Right));
        frame(&mut v);
        key(&mut v, NamedKey::Escape);
        assert!(labels(&v).iter().any(|l| l.starts_with("Node ") && l.ends_with(" card")), "{:?}", labels(&v));
        // Pinned: it stays when closed and the selection changes.
        let pin = target(&v, "Pin the inspector");
        click(&mut v, pin);
        let close = target(&v, "Close the inspector");
        click(&mut v, close);
        assert!(labels(&v).iter().any(|l| l.ends_with(" card")), "a pinned inspector ignores Close");
    }

    #[test]
    fn a_card_folds_and_unfolds_on_a_click() {
        let mut v = member_viewer();
        assert_eq!(v.gui.card_open("mode"), None, "no choice yet: the default (open) applies");
        let card = target(&v, "Modelling card");
        click(&mut v, card);
        assert_eq!(v.gui.card_open("mode"), Some(false));
        let card = target(&v, "Modelling card");
        click(&mut v, card);
        assert_eq!(v.gui.card_open("mode"), Some(true));
    }

    #[test]
    fn the_tree_folds_to_a_chip_and_opens_again_and_the_model_takes_the_space() {
        let mut v = member_viewer();
        let open = v.viewport();
        let fold = target(&v, "Fold the feature tree");
        click(&mut v, fold);
        assert!(!v.gui.tree_is_open());
        assert!(v.viewport().w > open.w, "the viewport widens");
        let chip = target(&v, "Feature tree");
        click(&mut v, chip);
        assert!(v.gui.tree_is_open() && v.viewport().w == open.w);
    }

    #[test]
    fn a_right_click_on_a_tree_row_selects_it_and_opens_the_menu_of_its_entity() {
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 1.0;
        let ed = crate::gpu_viewer::editor::solid::SolidSession::new(problem, std::env::temp_dir().join("window-treemenu-test.json"));
        let mut v = viewer(Some(Box::new(ed)));
        let row = target(&v, "Mesh: ");
        move_to(&mut v, row);
        v.test_events.push(egui::Event::PointerButton { pos: pos(row), button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        v.test_events.push(egui::Event::PointerButton { pos: pos(row), button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        frame(&mut v);
        frame(&mut v);
        assert!(v.gui.context_open(), "the menu opens from the selected state");
        let labels = labels(&v);
        assert!(labels.iter().any(|l| l == "Generate mesh"), "{labels:?}");
        assert!(labels.iter().any(|l| l.starts_with("Element Type")), "{labels:?}");
        // Choosing an entry runs it and closes the menu.
        let generate = v.gui.target_labels().iter().rposition(|l| l == "Generate mesh").map(|_| ()).is_some();
        assert!(generate);
    }

    /// Visit `Model` and `Mesh` alternately so each tool group is offered again, using `tool` each time.
    fn train_tool(v: &mut Viewer, tool: &str, rounds: usize) {
        for _ in 0..rounds {
            let model = target(v, "Model tab");
            click(v, model);
            let button = target(v, tool);
            click(v, button);
            let simulation = target(v, "Simulation tab");
            click(v, simulation);
        }
        let model = target(v, "Model tab");
        click(v, model);
    }

    #[test]
    fn the_tool_pill_learns_which_tools_are_used_first_and_reset_and_off_restore_the_fixed_order() {
        let mut v = member_viewer();
        let x = |v: &Viewer, l: &str| target(v, l).0;
        assert!(x(&v, "Add support") < x(&v, "Add load"), "the fixed order puts Add support first");
        train_tool(&mut v, "Add load", 7);
        assert!(x(&v, "Add load") < x(&v, "Add support"), "the tool that is used moves up: {:?}", labels(&v));
        // Switching the adaptive layout off restores the fixed order; on again brings the learned one back.
        v.dispatch(UiAction::Layout(LayoutOp::ToggleAdaptive));
        frame(&mut v);
        assert!(x(&v, "Add support") < x(&v, "Add load"));
        v.dispatch(UiAction::Layout(LayoutOp::ToggleAdaptive));
        frame(&mut v);
        assert!(x(&v, "Add load") < x(&v, "Add support"));
        // Reset forgets it.
        v.dispatch(UiAction::Layout(LayoutOp::Reset));
        frame(&mut v);
        assert!(x(&v, "Add support") < x(&v, "Add load"));
        assert!(v.editor.as_ref().unwrap().status().contains("forgotten"));
        v.dispatch(UiAction::Layout(LayoutOp::Explain));
        assert!(v.editor.as_ref().unwrap().status().contains("Adaptive layout"));
    }

    #[test]
    fn a_finished_step_rings_the_next_one_and_moves_on_once_that_is_a_habit() {
        fn finish_mesh(v: &mut Viewer) {
            let tab = target(v, "Mesh tab");
            click(v, tab);
            let generate = target(v, "Generate mesh");
            click(v, generate);
            let t = Instant::now();
            while !v.scene().label.contains("geometry") {
                v.editor.as_mut().unwrap().poll();
                frame(v);
                assert!(t.elapsed().as_secs() < 120, "timed out");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            frame(v);
        }
        let make = |name: &str| {
            let mut problem = fea_problem::templates::templates().remove(0).1;
            problem.mesh.size = 1.0;
            viewer(Some(Box::new(crate::gpu_viewer::editor::solid::SolidSession::new(problem, std::env::temp_dir().join(name)))))
        };
        // No habit yet: the next step only gets a ring, the tools stay on Mesh.
        let mut v = make("window-nudge-test.json");
        finish_mesh(&mut v);
        assert_eq!(v.gui.nudge(), Some("Simulation"));
        assert!(labels(&v).iter().any(|l| l == "Generate mesh"), "still on the Mesh step");
        // With a learned habit (Mesh is followed by Simulation) the step moves on by itself.
        let mut v = make("window-advance-test.json");
        for _ in 0..5 {
            v.gui.adapt_mut().step_taken("Mesh", "Simulation");
        }
        finish_mesh(&mut v);
        assert!(labels(&v).iter().any(|l| l == "Solve"), "moved to Simulation: {:?}", labels(&v));
        assert_eq!(v.gui.nudge(), None);
    }

    #[test]
    fn pointing_at_a_fix_highlights_the_surface_it_touches_and_leaving_clears_it() {
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 1.0;
        problem.supports.clear();
        let mut v = viewer(Some(Box::new(crate::gpu_viewer::editor::solid::SolidSession::new(problem, std::env::temp_dir().join("window-fixhover-test.json")))));
        // The mesh first: the preview scene is where surfaces are lit.
        let tab = target(&v, "Mesh tab");
        click(&mut v, tab);
        let generate = target(&v, "Generate mesh");
        click(&mut v, generate);
        let t = Instant::now();
        while !v.scene().label.contains("geometry") {
            v.editor.as_mut().unwrap().poll();
            frame(&mut v);
            assert!(t.elapsed().as_secs() < 120, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        frame(&mut v);
        // Vertices carrying the "selected" state code (4 is the viewer's colour for it; the session marks highlights 3 -> selected).
        let lit = |v: &Viewer| v.scene().samples.iter().filter(|s| s[3] >= 3.0 - 1e-6).count();
        let before = lit(&v);
        let labels = v.gui.target_labels();
        let fix = labels.iter().find(|l| l.contains("Fix 'left'")).unwrap_or_else(|| panic!("no fix row among {labels:?}")).clone();
        let at = target(&v, &fix);
        move_to(&mut v, at);
        frame(&mut v);
        let on = lit(&v);
        assert!(on > before, "pointing at 'Fix left' lights the left edge: {before} -> {on}");
        // Away from every row the preview goes, and nothing was applied.
        let away = viewport_centre(&v);
        move_to(&mut v, away);
        frame(&mut v);
        assert_eq!(lit(&v), before, "leaving the row clears the highlight");
    }

    #[test]
    fn a_real_mouse_click_on_a_3d_face_picks_it_and_its_menu_adds_a_support_there() {
        let mut problem = fea_problem::templates::templates().into_iter().find(|(n, _)| n.contains("3D")).unwrap().1;
        problem.mesh.size = 1.0;
        problem.supports.clear();
        let mut v = viewer(Some(Box::new(crate::gpu_viewer::editor::solid::SolidSession::new(problem, std::env::temp_dir().join("window-facepick-test.json")))));
        let tab = target(&v, "Mesh tab");
        click(&mut v, tab);
        let generate = target(&v, "Generate mesh");
        click(&mut v, generate);
        let t = Instant::now();
        while !v.scene().label.contains("geometry") {
            v.editor.as_mut().unwrap().poll();
            frame(&mut v);
            assert!(t.elapsed().as_secs() < 120, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        frame(&mut v);
        let supports = |v: &Viewer| tree_labels(v).into_iter().find(|l| l.starts_with("Supports (")).unwrap();
        assert_eq!(supports(&v), "Supports (0)");
        // Sweep the viewport with real clicks until one lands on the part and opens the surface menu.
        let r = v.viewport();
        let mut picked = None;
        'scan: for iy in 1..10 {
            for ix in 1..16 {
                let at = ((r.x + r.w * ix / 16) as f64, (r.y + r.h * iy / 10) as f64);
                click(&mut v, at);
                if let Some(item) = labels(&v).into_iter().find(|l| l.starts_with("Fixed support on '")) {
                    picked = Some((at, item));
                    break 'scan;
                }
            }
        }
        let (at, item) = picked.expect("some click in the viewport must land on the part and open the surface menu");
        let face = item.trim_start_matches("Fixed support on '").trim_end_matches('\'').to_string();
        assert!(["left", "right", "top", "bottom", "start", "end", "hole1"].contains(&face.as_str()), "a named face of the extruded plate: {face}");
        // The tree shows the picked surface as selected, and choosing the menu entry adds the support on it.
        let selected: Vec<String> = v.editor.as_ref().unwrap().tree().into_iter().filter(|n| n.selected && n.label == face).map(|n| n.label).collect();
        assert_eq!(selected, [face.clone()], "pointer at {at:?}");
        let entry = target(&v, &item);
        click(&mut v, entry);
        assert_eq!(supports(&v), "Supports (1)", "{}", v.editor.as_ref().unwrap().status());
        assert!(v.editor.as_ref().unwrap().status().contains(&face), "{}", v.editor.as_ref().unwrap().status());
    }

    /// The whole STEP workflow with a real mouse: open a cube, mesh it, click a face, cut a hole in it from the Insert menu's
    /// action, and see the support set on another face follow its face through the edit.
    #[cfg(feature = "occt")]
    #[test]
    fn a_hole_is_cut_in_a_face_picked_with_the_mouse_and_a_support_on_another_face_survives_it() {
        use crate::gpu_viewer::editor::solid::{stepops::PromptOp, SolidAction, SolidSession};
        let dir = std::env::temp_dir().join(format!("window-step-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let step = solid_kernel::occt::Brep::box_from_corners([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]).to_step_text().unwrap();
        let path = dir.join("cube.step");
        std::fs::write(&path, step).unwrap();
        let mut ed = SolidSession::new(fea_problem::templates::templates().remove(0).1, dir.join("model.json"));
        ed.open_path(path.to_str().unwrap());
        let mut v = viewer(Some(Box::new(ed)));
        let tab = target(&v, "Mesh tab");
        click(&mut v, tab);
        let generate = target(&v, "Generate mesh");
        click(&mut v, generate);
        let t = Instant::now();
        while !v.scene().label.contains("geometry") {
            v.editor.as_mut().unwrap().poll();
            frame(&mut v);
            assert!(t.elapsed().as_secs() < 180, "timed out: {}", v.editor.as_ref().unwrap().status());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        frame(&mut v);
        // A real click on the part: the first one that opens the surface menu names a face; give that face a support.
        let r = v.viewport();
        let mut first = None;
        'scan: for iy in 1..10 {
            for ix in 1..16 {
                let at = ((r.x + r.w * ix / 16) as f64, (r.y + r.h * iy / 10) as f64);
                click(&mut v, at);
                if let Some(item) = labels(&v).into_iter().find(|l| l.starts_with("Fixed support on '")) {
                    first = Some(item);
                    break 'scan;
                }
            }
        }
        let item = first.expect("a click lands on the cube");
        let held = item.trim_start_matches("Fixed support on '").trim_end_matches('\'').to_string();
        let entry = target(&v, &item);
        click(&mut v, entry);
        // Pick a different face with the mouse for the hole.
        let mut hole_face = None;
        'scan2: for iy in 1..10 {
            for ix in 1..16 {
                let at = ((r.x + r.w * ix / 16) as f64, (r.y + r.h * iy / 10) as f64);
                click(&mut v, at);
                if let Some(item) = labels(&v).into_iter().find(|l| l.starts_with("Fixed support on '")) {
                    let name = item.trim_start_matches("Fixed support on '").trim_end_matches('\'').to_string();
                    if name != held {
                        hole_face = Some(name);
                        break 'scan2;
                    }
                }
            }
        }
        let hole_face = hole_face.expect("a second face is pickable");
        // Close the surface menu (it stays selected), then run the hole action on the selection.
        key(&mut v, NamedKey::Escape);
        v.dispatch(UiAction::Editor(session::EditorAction::Solid(SolidAction::SolidOp(PromptOp::Hole))));
        frame(&mut v);
        v.input(Input::Key(Key::Character("2".into())));
        key(&mut v, NamedKey::Enter);
        let status = v.editor.as_ref().unwrap().status();
        assert!(status.contains("Cut a hole") && status.contains("7 faces") && status.contains("follow their faces"), "{status} (held {held}, hole in {hole_face})");
        // The support is still on a face of the part: nothing was lost.
        assert!(!status.contains("lost their face"), "{status}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn what_the_layout_learned_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("window-layout-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let x = |v: &Viewer, l: &str| target(v, l).0;
        {
            let mut v = member_viewer();
            v.gui.attach_layout_dir(&dir);
            frame(&mut v);
            train_tool(&mut v, "Add load", 7);
            assert!(x(&v, "Add load") < x(&v, "Add support"));
            v.gui.flush_layout();
        }
        let mut v = member_viewer();
        assert!(x(&v, "Add support") < x(&v, "Add load"), "a fresh window starts from the fixed order");
        v.gui.attach_layout_dir(&dir);
        frame(&mut v);
        assert!(x(&v, "Add load") < x(&v, "Add support"), "the habit came back from the file");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn selecting_the_mesh_row_in_the_tree_brings_the_mesh_tools() {
        let mut problem = fea_problem::templates::templates().remove(0).1;
        problem.mesh.size = 1.0;
        let ed = crate::gpu_viewer::editor::solid::SolidSession::new(problem, std::env::temp_dir().join("window-focusstep-test.json"));
        let mut v = viewer(Some(Box::new(ed)));
        assert!(labels(&v).iter().any(|l| l == "New part"), "starts on the Model step");
        let row = target(&v, "Mesh: ");
        click(&mut v, row);
        frame(&mut v);
        assert!(labels(&v).iter().any(|l| l == "Generate mesh"), "the tools followed the selection: {:?}", labels(&v));
        let row = target(&v, "Material: ");
        click(&mut v, row);
        frame(&mut v);
        assert!(labels(&v).iter().any(|l| l == "New part"));
    }
}
