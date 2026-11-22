//! Window input: the `Input` entry point (also driven by tests) and key presses.
use super::*;

impl Viewer {
    pub(crate) fn input(&mut self, input: Input) {
        if self.page_mode() {
            match input {
                Input::Modifiers(m) => self.mods = m,
                Input::Key(k) => self.key_pressed(&k),
                Input::Cursor(at) => self.pointer.cursor = Some(at),
                // A click away from an open popup closes it; everything else is the page's (egui saw it first).
                Input::Press(MouseButton::Left) if !self.gui.wants_pointer() => {
                    self.gui.close_floating();
                }
                _ => {}
            }
            return;
        }
        match input {
            Input::Modifiers(m) => self.mods = m,
            Input::Key(k) => self.key_pressed(&k),
            Input::Press(MouseButton::Left) => {
                let Some(at) = self.pointer.cursor else {
                    return;
                };
                // egui takes the press when it is over a panel, a button or a popup; a popup also closes on a click
                // anywhere else, and that click is not a model click.
                let mut consumed = self.gui.wants_pointer();
                if !consumed && self.gui.close_floating() {
                    consumed = true;
                }
                self.egui_press = consumed;
                self.grabbing = !consumed && !self.mods.shift_key() && self.begin_grab(at);
                // Shift-drag in an editor is a selection box; elsewhere a drag orbits.
                self.pointer.press_as(
                    consumed || self.grabbing,
                    self.editor.is_some() && self.mods.shift_key(),
                );
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
                self.pan_gesture = self
                    .pointer
                    .cursor
                    .filter(|&at| self.viewport_point(at).is_some())
                    .map(|at| (button, PanGesture::begin(at, button == MouseButton::Middle)));
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
                let cursor = self
                    .pointer
                    .cursor
                    .filter(|&at| self.viewport_point(at).is_some());
                if let Some(at) = cursor {
                    self.zoom_at(d, at);
                }
            }
        }
    }
    pub(super) fn key_pressed(&mut self, key: &Key) {
        if self.gui.confirm_close {
            match key {
                Key::Named(NamedKey::Escape) => self.dispatch(UiAction::CancelClose),
                Key::Named(NamedKey::Enter) => self.dispatch(UiAction::SaveAndClose),
                _ => {}
            }
            return;
        }
        // While a number is being typed every key belongs to the prompt; otherwise Ctrl/Cmd+K opens the palette.
        let typing = self.editor.as_ref().is_some_and(|ed| ed.prompting());
        if !typing && self.dashboard.is_none() && opens_palette(key, self.mods) {
            self.gui.open_palette();
            return;
        }
        if !typing && self.nav.is_some() && opens_switcher(key, self.mods) {
            self.gui.open_switcher();
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
                let mods = session::Mods {
                    command: self.mods.control_key() || self.mods.super_key(),
                    shift: self.mods.shift_key(),
                };
                // While a number is being typed every key belongs to the prompt.
                let result = self
                    .editor
                    .as_mut()
                    .map(|ed| ed.key(k, mods))
                    .unwrap_or(KeyResult::Unhandled);
                match result {
                    KeyResult::Handled => return,
                    KeyResult::Command(c) => {
                        self.command(c);
                        return;
                    }
                    KeyResult::Unhandled => {}
                }
            }
            // Editor mode: arrows edit properties, so only the remaining viewer keys fall through; a page has no viewer keys.
            if self.page_mode()
                || matches!(
                    key,
                    Key::Named(
                        NamedKey::Escape | NamedKey::ArrowUp | NamedKey::ArrowDown | NamedKey::Tab
                    )
                )
            {
                return;
            }
        }
        match key {
            // The dashboard with workbenches parked behind it keeps them: closing is the window's button.
            Key::Named(NamedKey::Escape) if self.has_open_workbench() => {}
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
            Key::Character(c) if c.eq_ignore_ascii_case("c") => {
                self.command(Command::ToggleContour)
            }
            Key::Character(c) if c.eq_ignore_ascii_case("d") => {
                self.command(Command::ToggleDeformation)
            }
            Key::Character(c) if c.eq_ignore_ascii_case("r") => self.command(Command::ResetCamera),
            Key::Character(c) if c.eq_ignore_ascii_case("f") => {
                self.command(Command::CycleField { forward: c != "F" })
            }
            Key::Character(c) if c.eq_ignore_ascii_case("h") || c == "?" => {
                self.command(Command::ToggleHelp)
            }
            Key::Named(NamedKey::ArrowRight) => self.command(Command::StepFrame { forward: true }),
            Key::Named(NamedKey::ArrowLeft) => self.command(Command::StepFrame { forward: false }),
            Key::Named(NamedKey::ArrowUp) => self.command(Command::GainUp),
            Key::Named(NamedKey::ArrowDown) => self.command(Command::GainDown),
            Key::Character(c) if c == "+" || c == "=" => self.command(Command::SpeedUp),
            Key::Character(c) if c == "-" => self.command(Command::SpeedDown),
            _ => {}
        }
    }
}
