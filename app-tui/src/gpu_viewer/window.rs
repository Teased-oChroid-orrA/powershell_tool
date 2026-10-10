use super::{render::Renderer, scene::ViewerProject, Scene};
use std::{sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};
struct Viewer {
    project: ViewerProject,
    show_mesh: bool,
    show_contour: bool,
    deformed: bool,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    renderer: Option<Renderer>,
    config: Option<wgpu::SurfaceConfiguration>,
    last: Instant,
    time: f32,
    paused: bool,
    speed: f32,
    gain: f32,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    drag: bool,
    cursor: Option<(f64, f64)>,
    error: Option<String>,
}
impl Viewer {
    fn scene(&self) -> &Scene {
        &self.project.scenes[self.project.selected]
    }
    fn switch_mode(&mut self, forward: bool) -> Result<(), String> {
        let n = self.project.scenes.len();
        let selected = if forward {
            (self.project.selected + 1) % n
        } else {
            (self.project.selected + n - 1) % n
        };
        let scene = &self.project.scenes[selected];
        if let Some(renderer) = &mut self.renderer {
            renderer.set_scene(scene)?;
        }
        self.project.selected = selected;
        self.gain = self.scene().gain;
        self.time = 0.0;
        self.speed = if self.scene().duration() > 0.0 {
            self.scene().duration() / 4.0
        } else {
            1.0
        };
        self.error = None;
        Ok(())
    }

    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("FEA GPU viewport"))
                .map_err(|e| e.to_string())?,
        );
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or("no compatible GPU adapter")?;
        let size = window.inner_size();
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface has no supported configuration")?;
        let renderer = pollster::block_on(Renderer::new(
            &adapter,
            config.format,
            self.scene(),
            config.width,
            config.height,
        ))?;
        surface.configure(&renderer.device, &config);
        self.window = Some(window);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.config = Some(config);
        self.last = Instant::now();
        // Companion-process readiness handshake: the controller keeps launch progress visible until here.
        println!("FEA_GPU_READY");
        Ok(())
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
        match event {
            WindowEvent::CloseRequested => e.exit(),
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let (Some(c), Some(r), Some(s)) =
                        (&mut self.config, &mut self.renderer, &self.surface)
                    {
                        c.width = size.width;
                        c.height = size.height;
                        s.configure(&r.device, c);
                        r.resize(size.width, size.height);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.logical_key {
                    Key::Named(NamedKey::Escape) => e.exit(),
                    Key::Named(NamedKey::Space) => self.paused = !self.paused,
                    Key::Named(NamedKey::Home) => self.time = 0.0,
                    Key::Named(NamedKey::Tab) => {
                        if let Err(err) = self.switch_mode(true) {
                            self.error = Some(err);
                        }
                    }
                    Key::Character(ref c) if c == "[" || c == "]" => {
                        if let Err(err) = self.switch_mode(c == "]") {
                            self.error = Some(err);
                        }
                    }
                    Key::Character(ref c) if c.eq_ignore_ascii_case("m") => {
                        self.show_mesh = !self.show_mesh
                    }
                    Key::Character(ref c) if c.eq_ignore_ascii_case("c") => {
                        self.show_contour = !self.show_contour
                    }
                    Key::Character(ref c) if c.eq_ignore_ascii_case("d") => {
                        self.deformed = !self.deformed
                    }
                    Key::Character(ref c) if c.eq_ignore_ascii_case("r") => {
                        self.yaw = 0.0;
                        self.pitch = 0.0;
                        self.zoom = 1.6;
                    }
                    Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::ArrowLeft) => {
                        self.paused = true;
                        if let Some(period) = self.scene().harmonic_period {
                            let delta = if event.logical_key == Key::Named(NamedKey::ArrowRight) {
                                period / 120.0
                            } else {
                                -period / 120.0
                            };
                            self.time = (self.time + delta).rem_euclid(period);
                            return;
                        }
                        let i = self
                            .scene()
                            .times
                            .partition_point(|t| *t <= self.time)
                            .saturating_sub(1);
                        let j = if event.logical_key == Key::Named(NamedKey::ArrowRight) {
                            (i + 1).min(self.scene().times.len() - 1)
                        } else {
                            i.saturating_sub(1)
                        };
                        self.time = self.scene().times[j];
                    }
                    Key::Named(NamedKey::ArrowUp) => self.gain = (self.gain * 1.25).min(1e12),
                    Key::Named(NamedKey::ArrowDown) => self.gain /= 1.25,
                    Key::Character(c) if c == "+" || c == "=" => {
                        self.speed = (self.speed * 2.0).min(1024.0)
                    }
                    Key::Character(c) if c == "-" => {
                        self.speed = (self.speed / 2.0).max(1.0 / 1024.0)
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => self.drag = state == ElementState::Pressed,
            WindowEvent::CursorMoved { position, .. } => {
                if self.drag {
                    if let Some((x, y)) = self.cursor {
                        self.yaw += (position.x - x) as f32 * 0.01;
                        self.pitch = (self.pitch + (position.y - y) as f32 * 0.01).clamp(-1.5, 1.5);
                    }
                }
                self.cursor = Some((position.x, position.y));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 100.0,
                };
                self.zoom = (self.zoom * (1.0 + d * 0.1)).clamp(0.1, 10.0);
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last).as_secs_f32().min(0.1);
                self.last = now;
                let end = self.scene().duration();
                if !self.paused && end > 0.0 {
                    self.time = (self.time + dt * self.speed) % end;
                }
                if let (Some(s), Some(r), Some(c)) = (&self.surface, &self.renderer, &self.config) {
                    match s.get_current_texture() {
                        Ok(frame) => {
                            r.draw_display(
                                &frame.texture.create_view(&Default::default()),
                                self.scene(),
                                self.time,
                                if self.deformed { self.gain } else { 0.0 },
                                [
                                    self.yaw,
                                    self.pitch,
                                    self.zoom,
                                    c.width as f32 / c.height as f32,
                                ],
                                self.show_mesh,
                                self.show_contour,
                            );
                            frame.present();
                        }
                        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                            s.configure(&r.device, c)
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => {
                            self.error = Some("GPU out of memory".into());
                            e.exit();
                        }
                        Err(wgpu::SurfaceError::Timeout) => {}
                    }
                }
                if let Some(w) = &self.window {
                    w.set_title(&format!("{} | result {}/{} | t={:.4} | gain ×{:.2} | speed ×{:.2} | mesh {} / contour {} / deformed {} | Tab/[ ] modes, M mesh, C contour, D deform, Space play, arrows frame/gain, +/- speed, R camera{}",self.scene().label,self.project.selected+1,self.project.scenes.len(),self.time,self.gain,self.speed,self.show_mesh,self.show_contour,self.deformed,self.error.as_ref().map(|e|format!(" | {e}")).unwrap_or_default()));
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
pub fn run(project: ViewerProject) -> Result<(), String> {
    project.validate()?;
    let scene = &project.scenes[project.selected];
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let gain = scene.gain;
    let end = scene.duration();
    let mut app = Viewer {
        project,
        show_mesh: false,
        show_contour: true,
        deformed: true,
        window: None,
        surface: None,
        renderer: None,
        config: None,
        last: Instant::now(),
        time: 0.0,
        paused: false,
        speed: if end > 0.0 { end / 4.0 } else { 1.0 },
        gain,
        yaw: 0.0,
        pitch: 0.0,
        zoom: 1.6,
        drag: false,
        cursor: None,
        error: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}
