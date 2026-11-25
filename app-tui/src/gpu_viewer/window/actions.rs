//! Reducing UI actions and menu outcomes into viewer state: dispatch, layout ops, bench switching.
use super::*;

impl Viewer {
    /// Carry out what a ribbon button, tree row, menu entry or property row asks for.
    pub(super) fn dispatch(&mut self, action: UiAction) {
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
            UiAction::BackgroundPicker => self.gui.toggle_background_picker(),
            UiAction::Editor(a) => {
                if let Some(ed) = &mut self.editor {
                    ed.act(a);
                }
            }
            UiAction::TogglePanel(p) => self.gui.toggle(p),
            UiAction::Layout(op) => self.layout_op(op),
            UiAction::Quit => self.request_quit(),
            UiAction::CancelClose => {
                self.gui.confirm_close = false;
                self.quit_warned = false;
            }
            UiAction::DiscardAndClose => self.discard_and_close(),
            UiAction::Nav(act) => self.nav_act(act),
            UiAction::RestoreRecovery => {
                if let Some(ed) = &mut self.editor {
                    let message = match ed.restore_recovery() {
                        Ok(()) => "Recovered model as an unsaved edit; review then Save.".into(),
                        Err(e) => format!("Recovery failed: {e}"),
                    };
                    ed.set_status(message);
                }
            }
            UiAction::SaveAndClose => {
                if let Some(ed) = &mut self.editor {
                    ed.key(
                        session::Key::Char('s'),
                        session::Mods {
                            command: true,
                            shift: false,
                        },
                    );
                }
                if !self.editor.as_ref().is_some_and(|ed| ed.dirty()) {
                    self.gui.confirm_close = false;
                    // Another workbench may still hold unsaved edits: that asks next.
                    self.request_quit();
                }
            }
        }
    }
    /// Window > Adaptive layout, What has the layout learned?, Reset learned layout, the tree.
    pub(super) fn layout_op(&mut self, op: LayoutOp) {
        let say = |v: &mut Viewer, text: String| match &mut v.editor {
            Some(ed) => ed.set_status(text),
            None => v.error = Some(text),
        };
        match op {
            LayoutOp::ToggleAdaptive => {
                let on = !self.gui.adapt_enabled();
                self.gui.set_adaptive(on);
                say(
                    self,
                    if on {
                        "Adaptive layout on: tools, cards and the tree follow how you work.".into()
                    } else {
                        "Adaptive layout off: the fixed layout is used.".into()
                    },
                );
            }
            LayoutOp::Reset => {
                self.gui.reset_layout();
                say(
                    self,
                    "Learned layout forgotten: back to the fixed layout.".into(),
                );
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
    pub(super) fn switch_bench(&mut self, bench: Bench) {
        let Some(ed) = &mut self.editor else { return };
        if ed.bench() == bench {
            return;
        }
        if !ed.switch_bench(bench) {
            if ed.dirty() {
                ed.set_status(format!(
                    "Save the {} model first (Ctrl/Cmd+S) before opening the {} bench.",
                    ed.bench().label(),
                    bench.label()
                ));
                return;
            }
            let dir = ed
                .path()
                .parent()
                .map_or_else(std::path::PathBuf::new, std::path::Path::to_path_buf);
            match crate::gpu_viewer::editor::open(
                bench,
                &crate::gpu_viewer::editor::bench_file(&dir, bench),
            ) {
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
        super::launch::apply_initial_display(&mut self.view, &**ed);
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
    pub(super) fn handle(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Action(a) => self.dispatch(a),
            Outcome::Blocked(reason) => match &mut self.editor {
                Some(ed) => ed.set_status(reason),
                None => self.error = Some(reason),
            },
            Outcome::CardField { index, text } => {
                if let Some(ed) = &mut self.editor {
                    ed.card_field(index, text);
                }
            }
            Outcome::DocField { id, text } => {
                if let Some(ed) = &mut self.editor {
                    ed.document_field(id, text);
                }
            }
        }
    }
    pub(super) fn menu_context(&self) -> MenuContext<'_> {
        MenuContext {
            view: &self.view,
            scene: self.scene(),
            results: if self.editor.is_some() {
                1
            } else {
                self.project.scenes.len()
            },
            editor: self.editor.as_deref(),
            visible: self.gui.visible,
            adaptive: self.gui.adapt_enabled(),
        }
    }
    pub(super) fn command(&mut self, command: Command) {
        let scene = &self.project.scenes[self.project.selected];
        let before = self.view.display_settings();
        self.view.apply(command, scene);
        if before != self.view.display_settings() {
            self.save_display();
        }
    }
}
