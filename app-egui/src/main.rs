//! Toolbench - egui/eframe port of `app/` (dioxus-native/Blitz).
//!
//! Why this crate exists, staged plan, and what's ported so far: see
//! `docs/issue-11-phase-14.md`. `app/` (the Blitz version) is untouched
//! and stays the shipping build until this reaches full parity and the
//! user signs off on cutover - same "keep the old one working during the
//! transition" pattern this repo already used for its WinUI migration.
//!
//! Stage 1+2 (this file, initial cut): window scaffold, ported color
//! tokens (`theme.rs`), and the app shell chrome - the auto-hiding left
//! rail, topbar, tool switcher. This directly fixes the bug that started
//! the migration: the rail's hover-reveal is driven by egui's own pointer
//! position + `Context::animate_bool`, not a CSS `:hover`/`transform`
//! combo, and `SidePanel::exact_width` is real layout (not an overlay),
//! so sibling content reflows automatically as it opens/closes - both
//! properties Blitz could never deliver after three attempts.

mod bushing;
mod command_palette;
mod components;
mod persistence;
mod pressure_vessel;
mod search;
mod theme;

use std::sync::Arc;

use bushing::BushingTool;
use command_palette::{Command, CommandPalette};
use eframe::egui;
use pressure_vessel::PressureVesselTool;
use search::SearchTool;
use theme::Tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolId {
    Search,
    Bushing,
    PressureVessel,
    Dupes,
    Rename,
    Logs,
}

impl ToolId {
    fn title(self) -> &'static str {
        match self {
            ToolId::Search => "Search Files",
            ToolId::Bushing => "Bushing Workbench",
            ToolId::PressureVessel => "Pressure Vessel Analyzer",
            ToolId::Dupes => "Duplicate Finder",
            ToolId::Rename => "Batch Rename",
            ToolId::Logs => "Log Analyzer",
        }
    }
    fn desc(self) -> &'static str {
        match self {
            ToolId::Search => "Keyword & regex search",
            ToolId::Bushing => "Interference-fit stress & margins",
            ToolId::PressureVessel => "Lam\u{e9} stress, failure modes & min thickness",
            ToolId::Dupes => "Find identical files",
            ToolId::Rename => "Pattern-based renaming",
            ToolId::Logs => "Parse & chart log files",
        }
    }
    fn enabled(self) -> bool {
        matches!(self, ToolId::Search | ToolId::Bushing | ToolId::PressureVessel)
    }
}

const NAV_ITEMS: [ToolId; 6] = [
    ToolId::Search,
    ToolId::Bushing,
    ToolId::PressureVessel,
    ToolId::Dupes,
    ToolId::Rename,
    ToolId::Logs,
];

struct ToolbenchApp {
    dark: bool,
    rail_pinned: bool,
    active_tool: ToolId,
    search: SearchTool,
    pv: PressureVesselTool,
    bushing: BushingTool,
    palette: CommandPalette,
}

impl Default for ToolbenchApp {
    fn default() -> Self {
        // One persistent multi-thread runtime for the whole app's async
        // work (search runs, future Bushing/PV background tasks) - created
        // once, kept alive for the process lifetime, spawned into from
        // `update()` via `Runtime::spawn` (which schedules onto the
        // runtime's own worker threads; it does not need to be inside a
        // `block_on` to work). Mirrors how `dioxus-native` gave every
        // component an ambient tokio context - egui has no such thing
        // built in, so this is the one piece of "framework glue" every
        // future async-backed tool will share.
        let runtime = Arc::new(tokio::runtime::Runtime::new().expect("failed to start tokio runtime"));
        let mut app = Self {
            dark: true,
            rail_pinned: false,
            active_tool: ToolId::Search,
            search: SearchTool::new(runtime),
            pv: PressureVesselTool::default(),
            bushing: BushingTool::default(),
            palette: CommandPalette::default(),
        };
        if let Some(p) = persistence::load() {
            app.dark = p.dark.unwrap_or(app.dark);
            app.rail_pinned = p.rail_pinned.unwrap_or(app.rail_pinned);
            app.search.restore(p.search_path, p.filters_text, p.parallel.unwrap_or(true));
            if let Some(v) = p.pv_outer_diameter {
                app.pv.outer_diameter = v;
            }
            if let Some(v) = p.pv_wall_thickness {
                app.pv.wall_thickness = v;
            }
            if let Some(v) = p.pv_internal_pressure {
                app.pv.internal_pressure = v;
            }
            if let Some(v) = p.pv_external_pressure {
                app.pv.external_pressure = v;
            }
            if let Some(v) = p.pv_closed_ends {
                app.pv.closed_ends = v;
            }
            if !p.pv_material_id.is_empty() {
                app.pv.material_id = p.pv_material_id;
            }
            if let Some(v) = p.pv_required_ms {
                app.pv.required_ms = v;
            }
            if let Some(v) = p.pv_unsupported_length {
                app.pv.unsupported_length = v;
            }
            if let Some(v) = p.bu_bore_dia {
                app.bushing.bore_dia = v;
            }
            if let Some(v) = p.bu_id_bushing {
                app.bushing.id_bushing = v;
            }
            if let Some(v) = p.bu_housing_len {
                app.bushing.housing_len = v;
            }
            if let Some(v) = p.bu_housing_width {
                app.bushing.housing_width = v;
            }
            if let Some(v) = p.bu_edge_dist {
                app.bushing.edge_dist = v;
            }
            if let Some(v) = p.bu_interference {
                app.bushing.interference = v;
            }
            if !p.bu_mat_housing.is_empty() {
                app.bushing.mat_housing = p.bu_mat_housing;
            }
            if !p.bu_mat_bushing.is_empty() {
                app.bushing.mat_bushing = p.bu_mat_bushing;
            }
            if let Some(v) = p.bu_d_t {
                app.bushing.d_t = v;
            }
            if let Some(v) = p.bu_min_wall_straight {
                app.bushing.min_wall_straight = v;
            }
        }
        app
    }
}

impl ToolbenchApp {
    fn snapshot(&self) -> persistence::PersistedState {
        persistence::PersistedState {
            dark: Some(self.dark),
            rail_pinned: Some(self.rail_pinned),
            search_path: self.search.search_path().to_string(),
            filters_text: self.search.filters_text().to_string(),
            parallel: Some(self.search.parallel()),
            pv_outer_diameter: Some(self.pv.outer_diameter),
            pv_wall_thickness: Some(self.pv.wall_thickness),
            pv_internal_pressure: Some(self.pv.internal_pressure),
            pv_external_pressure: Some(self.pv.external_pressure),
            pv_closed_ends: Some(self.pv.closed_ends),
            pv_material_id: self.pv.material_id.clone(),
            pv_required_ms: Some(self.pv.required_ms),
            pv_unsupported_length: Some(self.pv.unsupported_length),
            bu_bore_dia: Some(self.bushing.bore_dia),
            bu_id_bushing: Some(self.bushing.id_bushing),
            bu_housing_len: Some(self.bushing.housing_len),
            bu_housing_width: Some(self.bushing.housing_width),
            bu_edge_dist: Some(self.bushing.edge_dist),
            bu_interference: Some(self.bushing.interference),
            bu_mat_housing: self.bushing.mat_housing.clone(),
            bu_mat_bushing: self.bushing.mat_bushing.clone(),
            bu_d_t: Some(self.bushing.d_t),
            bu_min_wall_straight: Some(self.bushing.min_wall_straight),
        }
    }
}

const RAIL_COLLAPSED_W: f32 = 10.0;
const RAIL_EXPANDED_W: f32 = 232.0;

impl eframe::App for ToolbenchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let tokens = if self.dark { &Tokens::DARK } else { &Tokens::LIGHT };
        ctx.set_visuals(tokens.visuals());

        if let Some(cmd) = self.palette.update(ctx) {
            match cmd {
                Command::SwitchToSearch => self.active_tool = ToolId::Search,
                Command::SwitchToBushing => self.active_tool = ToolId::Bushing,
                Command::SwitchToPressureVessel => self.active_tool = ToolId::PressureVessel,
                Command::RunSearch => self.search.trigger_run(),
                Command::CancelSearch => self.search.trigger_cancel(),
                Command::ToggleTheme => self.dark = !self.dark,
                Command::PinRail => self.rail_pinned = !self.rail_pinned,
            }
        }

        // Hover test is a plain rect check against the pointer position -
        // real layout, real input, no renderer-specific pseudo-class.
        let pointer_over_rail = ctx
            .pointer_hover_pos()
            .is_some_and(|p| p.x <= RAIL_EXPANDED_W);
        let rail_open = self.rail_pinned || pointer_over_rail;
        let anim = ctx.animate_bool(egui::Id::new("rail_open"), rail_open);
        let rail_width = RAIL_COLLAPSED_W + (RAIL_EXPANDED_W - RAIL_COLLAPSED_W) * anim;

        egui::SidePanel::left("rail")
            .resizable(false)
            .exact_width(rail_width)
            .frame(egui::Frame::default().fill(tokens.bg_raised).inner_margin(egui::Margin::symmetric(if anim > 0.5 { 12.0 } else { 0.0 }, 16.0)))
            .show(ctx, |ui| {
                if anim > 0.5 {
                    self.rail_contents(ui, tokens);
                }
            });

        egui::TopBottomPanel::top("topbar")
            .frame(egui::Frame::default().fill(tokens.bg).inner_margin(egui::Margin { left: 28.0, right: 28.0, top: 16.0, bottom: 12.0 }))
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.heading(self.active_tool.title());
                ui.colored_label(tokens.fg_muted, self.active_tool.desc());
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(tokens.bg).inner_margin(egui::Margin::symmetric(28.0, 18.0)))
            .show(ctx, |ui| match self.active_tool {
                ToolId::Search => self.search.ui(ui, tokens),
                ToolId::Bushing => self.bushing.ui(ui, tokens),
                ToolId::PressureVessel => self.pv.ui(ui, tokens),
                _ => {
                    ui.label("Coming soon.");
                }
            });

        // Background search progress doesn't originate from egui input, so
        // it wouldn't otherwise trigger a repaint - poll while a search is
        // running (cheap: a mutex lock, not real work) rather than a tight
        // unconditional repaint loop when idle.
        if self.search.is_running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Best-effort, same as every other persistence write in this
        // app (see persistence.rs's own doc comment) - save once on
        // exit rather than every frame, since these fields only change
        // on user input, not continuously.
        persistence::save(&self.snapshot());
    }
}

impl ToolbenchApp {
    fn rail_contents(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 7.0, tokens.accent);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "T", egui::FontId::proportional(13.0), tokens.accent_fg);
            ui.vertical(|ui| {
                ui.strong("Toolbench");
                ui.small("GS Engineering");
            });
        });
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(6.0);
        ui.small("TOOLS");
        for tool in NAV_ITEMS {
            let selected = self.active_tool == tool;
            ui.add_enabled_ui(tool.enabled(), |ui| {
                let label = if tool.enabled() { tool.title().to_string() } else { format!("{}  \u{2022} soon", tool.title()) };
                if ui.selectable_label(selected, label).clicked() && tool.enabled() {
                    self.active_tool = tool;
                }
            });
        }
        ui.add_space(ui.available_height() - 70.0);
        ui.separator();
        if ui.button(if self.rail_pinned { "\u{1F4CC} Unpin rail" } else { "\u{1F4CC} Pin rail open" }).clicked() {
            self.rail_pinned = !self.rail_pinned;
        }
        if ui.button(if self.dark { "\u{263D} Light mode" } else { "\u{2600} Dark mode" }).clicked() {
            self.dark = !self.dark;
        }
    }
}

fn load_icon() -> egui::IconData {
    let bytes = include_bytes!("../../GS_Engineering_Brand_Assets/GS_Engineering_AppIcon_64x64.png");
    let img = image::load_from_memory(bytes).expect("bundled app icon PNG must decode").into_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData { rgba: img.into_raw(), width, height }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("GS Engineering - Toolbench")
            .with_icon(load_icon())
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "GS Engineering - Toolbench",
        options,
        Box::new(|_cc| Ok(Box::new(ToolbenchApp::default()))),
    )
}
