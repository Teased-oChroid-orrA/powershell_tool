//! Keeping the editor, the egui frame, quit and the persisted display settings in step with the viewer.
use super::*;
use crate::gpu_viewer::gui::DashEvent;

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
        self.gui.set_nav(self.nav_strip());
        if let Some(mut dash) = self.dashboard.take() {
            let (full, events) = self.gui.run_dashboard(raw, &mut dash);
            self.dashboard = Some(dash);
            for event in events {
                match event {
                    DashEvent::Open(id, file) => self.open_workbench(id, file),
                    DashEvent::NewWindow(id, file) => {
                        if let Some(d) = &mut self.dashboard {
                            d.launch(id, file);
                        }
                    }
                    DashEvent::Nav(act) => self.nav_act(act),
                }
            }
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
    /// Take the stored display style and background and keep them up to date from now on.
    pub(super) fn attach_display_store(&mut self, path: Option<std::path::PathBuf>) {
        if let Some(p) = &path {
            let mut s = DisplaySettings::load_from(p);
            s.panel_pos = s.panel_pos.map(crate::gpu_viewer::display::sanitize_panel_pos);
            s.page_layout = s.page_layout.and_then(crate::gpu_viewer::display::PageLayout::sanitized);
            self.view.set_display(s);
            if let Some(editor) = self.editor.as_deref() {
                super::launch::apply_initial_display(&mut self.view, editor);
            }
        }
        self.display_file = path;
    }
    pub(super) fn save_display(&mut self) {
        let Some(path) = &self.display_file else {
            return;
        };
        if let Err(e) = self.shared_display().save_to(path) {
            self.error = Some(format!("could not save the display settings: {e}"));
        }
    }
}
