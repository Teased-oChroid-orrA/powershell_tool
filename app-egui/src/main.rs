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

// Suppresses the extra console window Windows opens for a normal
// SUBSYSTEM:CONSOLE binary (the default for `fn main()` with no other
// attribute) - without this, launching app-egui.exe on Windows opens a
// second, blank console alongside the real window, and closing that
// console window kills the whole process (console-owner-process
// semantics; a GUI subsystem process has no such console to begin with).
// `app/src/main.rs` (the dioxus-native predecessor) already carries this
// exact fix - it never got ported over when `app-egui` was scaffolded as
// a new binary target, a real regression a user hit on a real Windows
// machine, not a hypothetical. Gated to release builds only - `cargo
// run` during local development still wants the console for stdout/
// stderr.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bushing;
mod command_palette;
mod components;
mod design;
mod graph;
mod persistence;
mod pressure_vessel;
mod search;
mod sketches;
mod theme;
mod widgets;

use std::sync::Arc;

use bushing::BushingTool;
use bushing_solver::countersink::CsMode;
use bushing_solver::geometry::{BushingType, IdType};
use command_palette::{Command, CommandPalette};
use eframe::egui;

/// `BushingType` has no `Serialize`/`Deserialize` (that's a UI-persistence
/// concern `bushing_solver` shouldn't carry) - persisted as a plain string
/// instead, same pattern `persistence::IndexLocation` already established.
fn head_type_to_str(t: BushingType) -> &'static str {
    match t {
        BushingType::Straight => "slug",
        BushingType::Countersink => "countersunk",
        BushingType::Flanged => "flanged",
    }
}
fn head_type_from_str(s: &str) -> Option<BushingType> {
    match s {
        "slug" => Some(BushingType::Straight),
        "countersunk" => Some(BushingType::Countersink),
        "flanged" => Some(BushingType::Flanged),
        _ => None,
    }
}

/// Same string-persistence reasoning as `head_type_to_str`.
fn id_type_to_str(t: IdType) -> &'static str {
    match t {
        IdType::Straight => "straight",
        IdType::Countersink => "countersunk",
    }
}
fn id_type_from_str(s: &str) -> Option<IdType> {
    match s {
        "straight" => Some(IdType::Straight),
        "countersunk" => Some(IdType::Countersink),
        _ => None,
    }
}

/// Same string-persistence reasoning as `head_type_to_str`.
fn cs_mode_to_str(m: CsMode) -> &'static str {
    match m {
        CsMode::DepthAngle => "depth_angle",
        CsMode::DiaAngle => "dia_angle",
        CsMode::DiaDepth => "dia_depth",
    }
}
fn cs_mode_from_str(s: &str) -> Option<CsMode> {
    match s {
        "depth_angle" => Some(CsMode::DepthAngle),
        "dia_angle" => Some(CsMode::DiaAngle),
        "dia_depth" => Some(CsMode::DiaDepth),
        _ => None,
    }
}
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
    fn icon(self) -> &'static str {
        match self {
            ToolId::Search => "\u{1F50D}",
            ToolId::Bushing => "\u{2699}",
            ToolId::PressureVessel => "\u{25a0}",
            ToolId::Dupes => "\u{1F4D1}",
            ToolId::Rename => "\u{1f58a}",
            ToolId::Logs => "\u{1F4CA}",
        }
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
    last_rail_width: f32,
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
            last_rail_width: RAIL_COLLAPSED_W,
        };
        if let Some(p) = persistence::load() {
            app.dark = p.dark.unwrap_or(app.dark);
            app.rail_pinned = p.rail_pinned.unwrap_or(app.rail_pinned);
            app.search.apply_snapshot(p.search);
            app.search.set_recent_and_presets(p.recent_searches, p.saved_presets);
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
            if let Some(v) = p.bu_min_wall_neck {
                app.bushing.min_wall_neck = v;
            }
            if let Some(t) = head_type_from_str(&p.bu_head_type) {
                app.bushing.head_type = t;
            }
            if let Some(v) = p.bu_bushing_length {
                app.bushing.bushing_length = v;
            }
            if let Some(v) = p.bu_ext_cs_dia {
                app.bushing.ext_cs_dia = v;
            }
            if let Some(v) = p.bu_ext_cs_depth {
                app.bushing.ext_cs_depth = v;
            }
            if let Some(v) = p.bu_ext_cs_angle {
                app.bushing.ext_cs_angle = v;
            }
            if let Some(v) = p.bu_lower_chamfer_min {
                app.bushing.lower_chamfer_min = v;
            }
            if let Some(v) = p.bu_lower_chamfer_max {
                app.bushing.lower_chamfer_max = v;
            }
            if let Some(v) = p.bu_lower_chamfer_angle_deg {
                app.bushing.lower_chamfer_angle_deg = v;
            }
            if let Some(v) = p.bu_head_chamfer_min {
                app.bushing.head_chamfer_min = v;
            }
            if let Some(v) = p.bu_head_chamfer_max {
                app.bushing.head_chamfer_max = v;
            }
            if let Some(v) = p.bu_head_chamfer_angle_deg {
                app.bushing.head_chamfer_angle_deg = v;
            }
            if let Some(v) = p.bu_bore_tol_plus {
                app.bushing.bore_tol_plus = v;
            }
            if let Some(v) = p.bu_bore_tol_minus {
                app.bushing.bore_tol_minus = v;
            }
            if let Some(v) = p.bu_interference_tol_plus {
                app.bushing.interference_tol_plus = v;
            }
            if let Some(v) = p.bu_interference_tol_minus {
                app.bushing.interference_tol_minus = v;
            }
            if let Some(v) = p.bu_enforcement_enabled {
                app.bushing.enforcement_enabled = v;
            }
            if let Some(t) = id_type_from_str(&p.bu_id_type) {
                app.bushing.id_type = t;
            }
            if let Some(m) = cs_mode_from_str(&p.bu_cs_mode) {
                app.bushing.cs_mode = m;
            }
            if let Some(v) = p.bu_cs_dia {
                app.bushing.cs_dia = v;
            }
            if let Some(v) = p.bu_cs_depth {
                app.bushing.cs_depth = v;
            }
            if let Some(v) = p.bu_cs_angle {
                app.bushing.cs_angle = v;
            }
            if let Some(v) = p.bu_cs_dia_tol_plus {
                app.bushing.cs_dia_tol_plus = v;
            }
            if let Some(v) = p.bu_cs_dia_tol_minus {
                app.bushing.cs_dia_tol_minus = v;
            }
            if let Some(v) = p.bu_cs_depth_tol_plus {
                app.bushing.cs_depth_tol_plus = v;
            }
            if let Some(v) = p.bu_cs_depth_tol_minus {
                app.bushing.cs_depth_tol_minus = v;
            }
            if let Some(v) = p.bu_cs_angle_tol_plus {
                app.bushing.cs_angle_tol_plus = v;
            }
            if let Some(v) = p.bu_cs_angle_tol_minus {
                app.bushing.cs_angle_tol_minus = v;
            }
            if let Some(m) = cs_mode_from_str(&p.bu_ext_cs_mode) {
                app.bushing.ext_cs_mode = m;
            }
            if let Some(v) = p.bu_ext_cs_dia_tol_plus {
                app.bushing.ext_cs_dia_tol_plus = v;
            }
            if let Some(v) = p.bu_ext_cs_dia_tol_minus {
                app.bushing.ext_cs_dia_tol_minus = v;
            }
            if let Some(v) = p.bu_ext_cs_depth_tol_plus {
                app.bushing.ext_cs_depth_tol_plus = v;
            }
            if let Some(v) = p.bu_ext_cs_depth_tol_minus {
                app.bushing.ext_cs_depth_tol_minus = v;
            }
            if let Some(v) = p.bu_ext_cs_angle_tol_plus {
                app.bushing.ext_cs_angle_tol_plus = v;
            }
            if let Some(v) = p.bu_ext_cs_angle_tol_minus {
                app.bushing.ext_cs_angle_tol_minus = v;
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
            search: self.search.to_snapshot(),
            recent_searches: self.search.recent_searches().to_vec(),
            saved_presets: self.search.saved_presets().to_vec(),
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
            bu_min_wall_neck: Some(self.bushing.min_wall_neck),
            bu_head_type: head_type_to_str(self.bushing.head_type).to_string(),
            bu_bushing_length: Some(self.bushing.bushing_length),
            bu_ext_cs_dia: Some(self.bushing.ext_cs_dia),
            bu_ext_cs_depth: Some(self.bushing.ext_cs_depth),
            bu_ext_cs_angle: Some(self.bushing.ext_cs_angle),
            bu_lower_chamfer_min: Some(self.bushing.lower_chamfer_min),
            bu_lower_chamfer_max: Some(self.bushing.lower_chamfer_max),
            bu_lower_chamfer_angle_deg: Some(self.bushing.lower_chamfer_angle_deg),
            bu_head_chamfer_min: Some(self.bushing.head_chamfer_min),
            bu_head_chamfer_max: Some(self.bushing.head_chamfer_max),
            bu_head_chamfer_angle_deg: Some(self.bushing.head_chamfer_angle_deg),
            bu_bore_tol_plus: Some(self.bushing.bore_tol_plus),
            bu_bore_tol_minus: Some(self.bushing.bore_tol_minus),
            bu_interference_tol_plus: Some(self.bushing.interference_tol_plus),
            bu_interference_tol_minus: Some(self.bushing.interference_tol_minus),
            bu_enforcement_enabled: Some(self.bushing.enforcement_enabled),
            bu_id_type: id_type_to_str(self.bushing.id_type).to_string(),
            bu_cs_mode: cs_mode_to_str(self.bushing.cs_mode).to_string(),
            bu_cs_dia: Some(self.bushing.cs_dia),
            bu_cs_depth: Some(self.bushing.cs_depth),
            bu_cs_angle: Some(self.bushing.cs_angle),
            bu_cs_dia_tol_plus: Some(self.bushing.cs_dia_tol_plus),
            bu_cs_dia_tol_minus: Some(self.bushing.cs_dia_tol_minus),
            bu_cs_depth_tol_plus: Some(self.bushing.cs_depth_tol_plus),
            bu_cs_depth_tol_minus: Some(self.bushing.cs_depth_tol_minus),
            bu_cs_angle_tol_plus: Some(self.bushing.cs_angle_tol_plus),
            bu_cs_angle_tol_minus: Some(self.bushing.cs_angle_tol_minus),
            bu_ext_cs_mode: cs_mode_to_str(self.bushing.ext_cs_mode).to_string(),
            bu_ext_cs_dia_tol_plus: Some(self.bushing.ext_cs_dia_tol_plus),
            bu_ext_cs_dia_tol_minus: Some(self.bushing.ext_cs_dia_tol_minus),
            bu_ext_cs_depth_tol_plus: Some(self.bushing.ext_cs_depth_tol_plus),
            bu_ext_cs_depth_tol_minus: Some(self.bushing.ext_cs_depth_tol_minus),
            bu_ext_cs_angle_tol_plus: Some(self.bushing.ext_cs_angle_tol_plus),
            bu_ext_cs_angle_tol_minus: Some(self.bushing.ext_cs_angle_tol_minus),
        }
    }
}

const RAIL_COLLAPSED_W: f32 = 10.0;
const RAIL_EXPANDED_W: f32 = 232.0;

impl eframe::App for ToolbenchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let tokens = if self.dark { &Tokens::DARK } else { &Tokens::LIGHT };
        ctx.set_visuals(tokens.visuals());
        // `.field input, .field select { padding: 7px 10px }` in the
        // approved mockup CSS - egui's own default `button_padding`
        // (~4x1px) made every input/button in the app read as a cramped,
        // unstyled box next to the 14-16px padding everywhere else
        // (cards, the stepper). Set once here (cheap, called every frame
        // already alongside `set_visuals`) so it applies app-wide rather
        // than needing a per-widget override at each call site.
        ctx.style_mut(|s| s.spacing.button_padding = egui::vec2(10.0, 7.0));

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

        // Hover test against the rail's *actual last-rendered width*, not
        // a fixed "anywhere in the left third of the window" threshold -
        // the mockup's own hover target was a persistent thin handle
        // strip, not the full rail's eventual footprint. Using a fixed
        // `x <= RAIL_EXPANDED_W` threshold here would mean hovering
        // anywhere up to 232px from the left edge re-opens the rail even
        // while it's fully collapsed to 10px - not what was approved, and
        // uncomfortably close to the exact class of bug (a hoverable
        // region bigger than what's visually there) this migration
        // started specifically to get away from. One frame of lag between
        // the rendered width and the hover test against it is
        // imperceptible (well under 16ms) and still real-layout-based,
        // not a hardcoded guess at the expanded width.
        let pointer_over_rail = ctx.pointer_hover_pos().is_some_and(|p| p.x <= self.last_rail_width);
        let rail_open = self.rail_pinned || pointer_over_rail;
        let anim = ctx.animate_bool(egui::Id::new("rail_open"), rail_open);
        let rail_width = RAIL_COLLAPSED_W + (RAIL_EXPANDED_W - RAIL_COLLAPSED_W) * anim;
        self.last_rail_width = rail_width;

        // Mockup's `.rail-handle`: the collapsed sliver isn't blank - it
        // tints toward the accent color and shows a chevron affordance,
        // the same "this is hoverable" signal `.rail-handle:hover`/
        // `.rail-zone.open .rail-handle` gave in the mockup.
        let handle_bg = if pointer_over_rail { tokens.accent.gamma_multiply(0.14) } else { tokens.bg_raised };
        egui::SidePanel::left("rail")
            .resizable(false)
            .exact_width(rail_width)
            .frame(egui::Frame::default().fill(if anim < 0.5 { handle_bg } else { tokens.bg_raised }).inner_margin(egui::Margin::symmetric(if anim > 0.5 { 12.0 } else { 0.0 }, 16.0)))
            .show(ctx, |ui| {
                if anim > 0.5 {
                    self.rail_contents(ui, tokens);
                } else {
                    let chevron_color = if pointer_over_rail { tokens.accent_strong } else { tokens.fg_subtle };
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 2.0 - 6.0);
                        ui.colored_label(chevron_color, "\u{203A}");
                    });
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
        ui.colored_label(tokens.fg_subtle, egui::RichText::new("TOOLS").size(10.5));
        for tool in NAV_ITEMS {
            if crate::widgets::nav_item(ui, tokens, tool.icon(), tool.title(), tool.desc(), self.active_tool == tool, tool.enabled()) {
                self.active_tool = tool;
            }
        }
        ui.add_space((ui.available_height() - 70.0).max(0.0));
        ui.separator();
        if ui.button(if self.rail_pinned { "\u{1F4CC} Unpin rail" } else { "\u{1F4CC} Pin rail open" }).clicked() {
            self.rail_pinned = !self.rail_pinned;
        }
        if ui.button(if self.dark { "\u{1f319} Light mode" } else { "\u{2600} Dark mode" }).clicked() {
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
        Box::new(|cc| {
            // Design System Epic Phase 1: bundled Inter/JetBrains Mono
            // fonts, installed once here (not per-frame in `update()` -
            // ~1.6MB of font data is too expensive to rebuild every
            // frame). See `design::typography` for why egui's bundled
            // default fonts stay installed as fallbacks rather than
            // being replaced.
            cc.egui_ctx.set_fonts(design::typography::font_definitions());
            Ok(Box::new(ToolbenchApp::default()))
        }),
    )
}
