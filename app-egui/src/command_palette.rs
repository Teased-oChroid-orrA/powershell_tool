//! Command palette - egui port of `app/src/command_palette.rs`'s core
//! idea (a filterable `Ctrl`/`Cmd`+`K` overlay), scoped to commands this
//! crate's tools actually support today (see `docs/issue-11-phase-14.md`'s
//! scope-cut list) - the original's `OpenReport`/`BrowseOutputFolder`/
//! `ClearRecentSearches`/`ToggleIndexForFastSearch` reference Search
//! settings not yet ported here. Extend `Command::ALL` as each of those
//! lands, the same incremental pattern every other tool in this crate
//! has followed.
//!
//! Global keyboard capture just works here - `ctx.input(|i| ...)` sees
//! every key event regardless of which widget has focus, no
//! renderer-specific verification needed (unlike Blitz, where this
//! module's own original doc comment had to cite `blitz-shell` source to
//! confirm Ctrl/Cmd+K would actually reach the app).

use eframe::egui;

#[derive(Clone, Copy, PartialEq)]
pub enum Command {
    SwitchToSearch,
    SwitchToBushing,
    SwitchToPressureVessel,
    RunSearch,
    CancelSearch,
    ToggleTheme,
    PinRail,
}

impl Command {
    const ALL: [Command; 7] = [
        Command::SwitchToSearch,
        Command::SwitchToBushing,
        Command::SwitchToPressureVessel,
        Command::RunSearch,
        Command::CancelSearch,
        Command::ToggleTheme,
        Command::PinRail,
    ];

    fn label(self) -> &'static str {
        match self {
            Command::SwitchToSearch => "Go to: Search Files",
            Command::SwitchToBushing => "Go to: Bushing Workbench",
            Command::SwitchToPressureVessel => "Go to: Pressure Vessel Analyzer",
            Command::RunSearch => "Run Search",
            Command::CancelSearch => "Cancel Search",
            Command::ToggleTheme => "Toggle theme (dark/light)",
            Command::PinRail => "Toggle rail pin",
        }
    }
}

#[derive(Default)]
pub struct CommandPalette {
    open: bool,
    query: String,
}

impl CommandPalette {
    /// Returns `Some(command)` the frame the user picks one (Enter or
    /// click) - the caller executes it, since only `main.rs` has access
    /// to every field a command might touch.
    pub fn update(&mut self, ctx: &egui::Context) -> Option<Command> {
        let toggle = ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::K));
        let just_opened = toggle && !self.open;
        if toggle {
            self.open = !self.open;
            self.query.clear();
        }
        if !self.open {
            return None;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.open = false;
            return None;
        }

        let mut picked = None;
        let query = self.query.to_lowercase();
        let matches: Vec<Command> = Command::ALL.into_iter().filter(|c| c.label().to_lowercase().contains(&query)).collect();

        egui::Window::new("Command Palette")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .fixed_size(egui::vec2(420.0, 260.0))
            .show(ctx, |ui| {
                let resp = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Type a command\u{2026}").desired_width(f32::INFINITY));
                if just_opened {
                    resp.request_focus();
                }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for c in &matches {
                        if ui.selectable_label(false, c.label()).clicked() {
                            picked = Some(*c);
                        }
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    picked = matches.first().copied();
                }
            });

        if picked.is_some() {
            self.open = false;
        }
        picked
    }
}
