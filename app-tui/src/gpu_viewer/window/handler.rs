//! The winit `ApplicationHandler`: window events, resume and the redraw clock.
use super::*;

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
        if !matches!(event, WindowEvent::RedrawRequested) {
            self.next_redraw = Instant::now();
        }
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
            WindowEvent::ModifiersChanged(m) => self.input(Input::Modifiers(m.state())),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                // Alt composes characters on some platforms (Alt+F is a different glyph on macOS): use the plain key.
                let key = if self.mods.alt_key() {
                    winit::platform::modifier_supplement::KeyEventExtModifierSupplement::key_without_modifiers(&event)
                } else {
                    event.logical_key
                };
                self.input(Input::Key(key))
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.input(if state == ElementState::Pressed {
                    Input::Press(button)
                } else {
                    Input::Release(button)
                })
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.input(Input::Cursor((position.x, position.y)))
            }
            WindowEvent::MouseWheel { delta, .. } => self.input(Input::Wheel(match delta {
                MouseScrollDelta::LineDelta(_, y) => y,
                MouseScrollDelta::PixelDelta(p) => p.y as f32 / 100.0,
            })),
            WindowEvent::Occluded(hidden) => {
                self.occluded = hidden;
                if !hidden {
                    self.next_redraw = Instant::now();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(e),
            _ => {}
        }
    }
    fn about_to_wait(&mut self, e: &ActiveEventLoop) {
        if self.quit {
            e.exit();
        }
        let changed = self.editor.as_mut().is_some_and(|ed| ed.poll()) | self.poll_parked() | self.serve_asked();
        let now = Instant::now();
        let moving = !self.view.paused
            && (self.scene().duration() > 0.0 || self.view.swing(self.scene()).is_some());
        if moving && !self.occluded {
            self.next_redraw = self
                .next_redraw
                .min(self.last + std::time::Duration::from_millis(16));
        }
        if self.any_tab_busy()
            || self
                .editor
                .as_ref()
                .is_some_and(|ed| ed.solving_for().is_some())
        {
            self.next_redraw = self
                .next_redraw
                .min(self.last + std::time::Duration::from_millis(100));
        }
        if changed {
            self.next_redraw = now;
        }
        // Poll worker completion and linked files without repainting unchanged chrome.
        e.set_control_flow(ControlFlow::WaitUntil(if self.occluded {
            now + std::time::Duration::from_millis(250)
        } else {
            self.next_redraw
                .min(now + std::time::Duration::from_millis(250))
        }));
        if self.recovery_flushed.elapsed().as_secs() >= 30 {
            self.recovery_flushed = Instant::now();
            for ed in self.editors_mut() {
                if ed.dirty() {
                    if let Ok(json) = ed.recovery_json() {
                        if let Err(e) = crate::persistence::write(
                            ed.path().with_extension("recovery.json"),
                            json,
                        ) {
                            ed.set_status(format!("Recovery snapshot failed: {e}"));
                        }
                    }
                }
            }
        }
        if self.layout_flushed.elapsed().as_secs() >= 30 {
            self.layout_flushed = Instant::now();
            self.gui.flush_layout();
        }
        if !self.occluded && now >= self.next_redraw {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }
}
