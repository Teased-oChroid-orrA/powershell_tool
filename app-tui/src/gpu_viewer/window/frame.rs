//! Window creation (`init`) and the per-frame `redraw`.
use super::*;

impl Viewer {
    pub(super) fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let mut attributes = Window::default_attributes().with_title("FEA GPU viewport");
        if self.dashboard.is_some()
            || self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.workbench().is_some())
        {
            let (w, h) = nav::WORKBENCH_SIZE;
            attributes = attributes.with_inner_size(winit::dpi::LogicalSize::new(w, h));
        }
        let window = Arc::new(
            event_loop
                .create_window(attributes)
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
            ..Default::default()
        }))
        .map_err(|e| format!("no compatible GPU adapter: {e}"))?;
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
        self.egui_state = Some(egui_winit::State::new(
            self.gui.ctx().clone(),
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(renderer.device.limits().max_texture_dimension_2d as usize),
        ));
        self.window = Some(window);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.config = Some(config);
        self.last = Instant::now();
        // Companion-process readiness handshake: the controller keeps launch progress visible until here.
        println!("FEA_GPU_READY");
        Ok(())
    }
    /// Draw one frame: advance playback, run the chrome, draw the scene into the free rectangle, paint the chrome.
    pub(super) fn redraw(&mut self, e: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        let scene = &self.project.scenes[self.project.selected];
        self.view.advance(scene, dt);
        self.sync_editor_scene();
        let (Some(window), Some(state)) = (self.window.clone(), self.egui_state.as_mut()) else {
            return;
        };
        let raw = state.take_egui_input(&window);
        let full = self.run_gui(raw);
        let delay = full
            .viewport_output
            .values()
            .map(|v| v.repaint_delay)
            .min()
            .unwrap_or(std::time::Duration::from_secs(30));
        self.next_redraw = now + delay.min(std::time::Duration::from_secs(30));
        if let Some(state) = self.egui_state.as_mut() {
            state.handle_platform_output(&window, full.platform_output.clone());
        }
        let vp = self.viewport();
        let size = self.window_size();
        let background = self.view.background.colors(self.gui.background_rgb());
        // An empty model has nothing to draw (the start card says how to begin): clear the viewport only.
        let empty = self.dashboard.is_some()
            || self.page_mode()
            || self
                .editor
                .as_ref()
                .is_some_and(|e| !e.start_options().is_empty());
        let geometry = self.editor.as_ref().is_some_and(|e| e.geometry_view());
        let scene_rect = if empty {
            [vp.x, vp.y, 0, 0]
        } else {
            [vp.x, vp.y, vp.w, vp.h]
        };
        let ctx = self.gui.ctx().clone();
        let mut full = Some(full);
        if let (Some(s), Some(r), Some(c)) = (&self.surface, &mut self.renderer, &self.config) {
            match s.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    let view = frame.texture.create_view(&Default::default());
                    r.draw_display(
                        &view,
                        &self.project.scenes[self.project.selected],
                        if self
                            .view
                            .swing(&self.project.scenes[self.project.selected])
                            .is_some()
                        {
                            self.view.clock
                        } else {
                            self.view.time
                        },
                        if self.view.deformed {
                            self.view.gain
                        } else {
                            0.0
                        },
                        [
                            self.view.yaw,
                            self.view.pitch,
                            self.view.zoom,
                            vp.w.max(1) as f32 / vp.h.max(1) as f32,
                        ],
                        DisplayOptions {
                            show_mesh: self.view.show_mesh
                                && !self.editor.as_ref().is_some_and(|e| e.shows_body_only()),
                            show_contour: self.view.show_contour,
                            field: self.view.field,
                            viewport: Some(scene_rect),
                            background: Some(background),
                            style: if self.editor.as_ref().is_some_and(|e| e.adjustable_shading()) && !self.view.style.paints_surface() {
                                crate::gpu_viewer::display::DisplayStyle::Shaded
                            } else {
                                self.view.style_for(&self.project.scenes[self.project.selected], geometry)
                            },
                            swing: self.view.swing(&self.project.scenes[self.project.selected]),
                            shade: geometry,
                            shade_intensity: if self.editor.as_ref().is_some_and(|e| e.adjustable_shading()) { self.gui.shade_intensity() } else { 1.0 },
                            key_colours: geometry.then_some(self.view.key_colours),
                            pan: self.view.pan,
                            translucent: self.editor.as_ref().and_then(|e| e.translucent()),
                        },
                    );
                    r.paint_egui(
                        &view,
                        &ctx,
                        full.take().expect("painted once"),
                        [size.0, size.1],
                    );
                    r.queue.present(frame);
                }
                wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                    s.configure(&r.device, c)
                }
                // Skip the frame and retry: timeouts and occlusion are transient, and wgpu reports a
                // surface validation error through its uncaptured-error handler.
                wgpu::CurrentSurfaceTexture::Timeout
                | wgpu::CurrentSurfaceTexture::Occluded
                | wgpu::CurrentSurfaceTexture::Validation => {}
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
                _ if self.dashboard.is_some() => "GS Engineering Toolbench".to_string(),
                Some(ed) => ed.title(),
                None => format!(
                    "FEA GPU viewport | result {}/{}",
                    self.project.selected + 1,
                    self.project.scenes.len()
                ),
            });
        }
        if self.quit {
            e.exit();
        }
    }
}
