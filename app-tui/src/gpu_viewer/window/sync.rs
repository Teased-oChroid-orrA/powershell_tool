//! Keeping the editor, the egui frame, quit and the persisted display settings in step with the viewer.
use super::*;

impl Viewer {
    /// Keep the GPU scene in step with the editor state (rebuild only when it changed).
    pub(super) fn sync_editor_scene(&mut self) {
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
    pub(super) fn run_gui(&mut self, raw: egui::RawInput) -> egui::FullOutput {
        if let Some(mut dash) = self.dashboard.take() {
            let (full, events) = self.gui.run_dashboard(raw, &mut dash);
            for crate::gpu_viewer::gui::DashEvent::Open(id, file) in events {
                dash.launch(id, file);
            }
            self.dashboard = Some(dash);
            return full;
        }
        let vp = self.viewport();
        let cam = self.camera();
        if let Some(ed) = &mut self.editor {
            ed.set_view(&cam, [vp.w as f32, vp.h as f32]);
        }
        let overlay = self
            .editor
            .as_ref()
            .map(|e| e.overlay(&cam, [vp.w as f32, vp.h as f32]))
            .unwrap_or_default();
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
    pub(super) fn request_quit(&mut self) {
        match &mut self.editor {
            Some(ed) if ed.dirty() => {
                self.quit_warned = true;
                self.gui.confirm_close = true;
                ed.set_status(
                    "Unsaved changes: choose Save and close, Discard changes, or Keep editing."
                        .into(),
                );
            }
            _ => self.quit = true,
        }
    }
    /// Take the stored display style and background and keep them up to date from now on.
    pub(super) fn attach_display_store(&mut self, path: Option<std::path::PathBuf>) {
        if let Some(p) = &path {
            let s = DisplaySettings::load_from(p);
            (self.view.style, self.view.background, self.view.key_colours) =
                (s.style, s.background, s.key_colours);
            self.view.panel_pos = s
                .panel_pos
                .map(crate::gpu_viewer::display::sanitize_panel_pos);
            self.view.page_layout = s
                .page_layout
                .and_then(crate::gpu_viewer::display::PageLayout::sanitized);
        }
        self.display_file = path;
    }
    pub(super) fn save_display(&mut self) {
        let Some(path) = &self.display_file else {
            return;
        };
        if let Err(e) = self.view.display_settings().save_to(path) {
            self.error = Some(format!("could not save the display settings: {e}"));
        }
    }
}
