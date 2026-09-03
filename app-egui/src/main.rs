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
mod components;
mod pressure_vessel;
mod search;
mod theme;

use std::sync::Arc;

use bushing::BushingTool;
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
        Self {
            dark: true,
            rail_pinned: false,
            active_tool: ToolId::Search,
            search: SearchTool::new(runtime),
            pv: PressureVesselTool::default(),
            bushing: BushingTool::default(),
        }
    }
}

const RAIL_COLLAPSED_W: f32 = 10.0;
const RAIL_EXPANDED_W: f32 = 232.0;

impl eframe::App for ToolbenchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let tokens = if self.dark { &Tokens::DARK } else { &Tokens::LIGHT };
        ctx.set_visuals(tokens.visuals());

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
