// Suppresses the extra console window Windows opens for a normal
// SUBSYSTEM:CONSOLE binary (the default for `fn main()` with no other
// attribute) - without this, launching app.exe on Windows opens a second,
// blank console alongside the real window, and closing that console
// window kills the whole process (console-owner-process semantics; a GUI
// subsystem process has no such console to begin with). Gated to release
// builds only - `cargo run`/`dx serve` during local development still
// want the console for the tracing::Level::INFO logger output below.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bushing_visualizer;
mod bushing_workbench;
mod command_palette;
mod components;
mod context_menu;
mod drag_drop;
mod fs_watch;
mod net_provider;
mod persistence;
mod preview;
mod pressure_vessel_workbench;
mod state;

use dioxus::html::{Code, Modifiers};
use dioxus::prelude::*;

use command_palette::CommandPalette;
use components::{ResultsPanel, SettingsPanel};
use context_menu::ContextMenu;
use preview::PreviewPane;
use state::AppState;

fn main() {
    // Real diagnostics for the ongoing scroll-perf investigation
    // (docs/epic-ui-performance-and-design.md) - without this, `RUST_LOG`
    // had nothing to report to (verified: no output at any level before
    // this was added).
    let _ = dioxus::logger::init(dioxus::logger::tracing::Level::INFO);

    let window_attributes = dioxus::native::WindowAttributes::default()
        .with_title("GS Engineering - Toolbench")
        .with_window_icon(load_window_icon());
    launch::run(App, window_attributes);
}

/// Decodes the bundled GS Engineering app icon into the raw RGBA buffer
/// `winit::window::Icon::from_rgba` needs. The window previously had no
/// custom icon at all (`docs/rust-rewrite-status.md`'s "not yet done"
/// note) - `WindowAttributes` never called `.with_window_icon(...)`. Loaded
/// from bytes embedded at compile time (`include_bytes!`), not read from
/// disk at runtime, so it stays part of the single self-contained exe (see
/// CLAUDE.md's "no host-machine dependency" target). Returns `None` on any
/// decode failure rather than panicking - a missing/bad icon should never
/// stop the window from opening.
fn load_window_icon() -> Option<winit::window::Icon> {
    const ICON_BYTES: &[u8] =
        include_bytes!("../../GS_Engineering_Brand_Assets/GS_Engineering_AppIcon_64x64.png");
    let img = image::load_from_memory(ICON_BYTES).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), width, height).ok()
}

/// Hand-rolled replacement for `dioxus_native::launch_cfg` - see
/// `drag_drop.rs` for why. This mirrors that function's own body (the
/// public API surface it needs is entirely `pub` on `dioxus-native`/
/// `dioxus-native-dom`/`blitz-shell`/`blitz-html`, verified by reading
/// `dioxus-native-0.7.10/src/lib.rs` directly) with two narrowed-down
/// simplifications specific to this app, not general-purpose omissions:
/// `navigation_provider` is `None` because this app has no `<a href>`
/// navigation at all. `net_provider` is `net_provider::data_uri_only(...)`
/// (not `None`, and not `dioxus-native`'s own full HTTP-capable default) -
/// the Bushing Workbench's pre-rendered LaTeX formula images load via
/// `<img src="data:...">`, which needs *some* provider (see
/// `net_provider.rs`'s doc comment for why `DummyNetProvider`, what
/// `None` resolves to, can't serve that), but this app still has no
/// legitimate use for fetching a real remote URL, so the HTTP-capable
/// `blitz-net::Provider` stays deliberately unwired.
mod launch {
    use std::sync::Arc;

    use blitz_shell::{create_default_event_loop, BlitzShellEvent, WindowConfig};
    use dioxus::core::VirtualDom;
    use dioxus::native::{DioxusDocument, DioxusNativeApplication, DioxusNativeWindowRenderer, DocumentConfig};
    use dioxus::prelude::Element;
    use tokio::sync::mpsc;
    use winit::window::WindowAttributes;

    use crate::drag_drop::{DragDropApplication, DROP_EVENTS};

    pub fn run(app: fn() -> Element, window_attributes: WindowAttributes) {
        let (drop_tx, drop_rx) = mpsc::unbounded_channel();
        // Fine to ignore a `set` failure (can't happen - `run` is only ever
        // called once from `main`) rather than unwrap and panic over it.
        let _ = DROP_EVENTS.set(tokio::sync::Mutex::new(drop_rx));

        let event_loop = create_default_event_loop::<BlitzShellEvent>();

        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("failed to start the async runtime");
        let _guard = rt.enter();

        crate::fs_watch::start();

        let vdom = VirtualDom::new(app);

        let html_parser_provider = Some(Arc::new(blitz_html::HtmlProvider) as _);
        let net_provider = Some(crate::net_provider::data_uri_only(event_loop.create_proxy()));
        let doc = DioxusDocument::new(
            vdom,
            DocumentConfig { net_provider, html_parser_provider, navigation_provider: None, ..Default::default() },
        );

        let renderer = DioxusNativeWindowRenderer::with_features_and_limits(None, None);
        let win_config = WindowConfig::with_attributes(Box::new(doc) as _, renderer, window_attributes);

        let inner = DioxusNativeApplication::new(event_loop.create_proxy(), win_config);
        let mut application = DragDropApplication { inner, drop_tx };

        event_loop.run_app(&mut application).expect("winit event loop exited with an error");
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ResizeTarget {
    Settings,
    Preview,
}

/// Which tool is showing in the dashboard's main stage - "Toolbench" (see
/// `docs/toolbench-status.md`), the multi-tool shell this app is becoming.
/// `Search` is the only one with real functionality behind it; the rest
/// render `PlaceholderTool` until they're actually built. Deliberately a
/// plain runtime `Signal`, not persisted - the mockup this was built from
/// didn't demonstrate remembering the last-open tool across relaunch, and
/// defaulting to `Search` (this app's one real tool) on every launch is
/// the more predictable behavior anyway.
#[derive(Clone, Copy, PartialEq)]
enum ToolId {
    Search,
    Bushing,
    PressureVessel,
    Dupes,
    Rename,
    Logs,
}

/// Inline SVG icons, matching the artifact preview's set exactly (same
/// stroke-based line-icon style as the rest of this app's chrome - see
/// `docs/epic-ui-performance-and-design.md` for why this app avoids an
/// icon-font/sprite-sheet dependency: a handful of hand-written paths
/// costs nothing extra to bundle and stays crisp at any size). Each
/// returns a full `<svg>` `Element` so callers can interpolate it
/// directly (`{icon_search()}`) without any prop-passing ceremony.
fn icon_search() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_linecap: "round", stroke_linejoin: "round",
            circle { cx: "11", cy: "11", r: "7" }
            path { d: "M21 21l-4.3-4.3" }
        }
    }
}
fn icon_dupes() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_linecap: "round", stroke_linejoin: "round",
            rect { x: "9", y: "9", width: "12", height: "12", rx: "2" }
            path { d: "M5 15V5a2 2 0 0 1 2-2h10" }
        }
    }
}
fn icon_rename() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M12 20h9" }
            path { d: "M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" }
        }
    }
}
fn icon_logs() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M4 6h16M4 12h10M4 18h13" }
        }
    }
}
/// A bushing's own cross-section (concentric rings, the two diameters
/// this whole tool centers on) rather than a generic mechanical/gear
/// glyph - the same "icon should mean the specific thing it opens"
/// discipline the other nav icons already follow.
fn icon_bushing() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_linecap: "round", stroke_linejoin: "round",
            circle { cx: "12", cy: "12", r: "9" }
            circle { cx: "12", cy: "12", r: "4" }
        }
    }
}

fn icon_pressure_vessel() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_linecap: "round", stroke_linejoin: "round",
            rect { x: "6", y: "4", width: "12", height: "16", rx: "6" }
            line { x1: "6", y1: "10", x2: "18", y2: "10" }
        }
    }
}
fn icon_plus() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M12 5v14M5 12h14" }
        }
    }
}
fn icon_sun() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_linecap: "round", stroke_linejoin: "round",
            circle { cx: "12", cy: "12", r: "4" }
            path { d: "M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" }
        }
    }
}
fn icon_moon() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8z" }
        }
    }
}
/// The rail's brand mark - a stylized "search beam" glyph, replacing the
/// old plain-text "GS" square (same spot, same size, real vector icon
/// instead of two letters).
fn icon_brand() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "M14 3l7 7-2.5 2.5L14 8l-4 4-3-3 4-4z" }
            path { d: "M3 21l6-6" }
            circle { cx: "18", cy: "6", r: "2" }
        }
    }
}

/// One stub tool card for the dashboard - "Duplicate Finder"/"Batch
/// Rename"/"Log Analyzer" are all this same shape today, differing only
/// in copy and icon. Real content replaces this one tool at a time as
/// each is actually built, same as `Search` already was.
#[component]
fn PlaceholderTool(title: String, description: String, icon: Element) -> Element {
    rsx! {
        div { class: "panel placeholder",
            div { class: "placeholder-icon", {icon} }
            span { class: "soon-pill", "Coming soon" }
            h2 { {title} }
            p { {description} }
        }
    }
}

#[component]
fn App() -> Element {
    // Settings/recent-search persistence (epic §22) - loaded once at
    // startup, applied onto the freshly-defaulted AppState/theme signal
    // before the first render.
    let persisted = use_hook(persistence::load);
    let mut dark = use_signal({
        let initial_dark = persisted.as_ref().map(|p| p.dark_theme).unwrap_or(true);
        move || initial_dark
    });
    let mut state = AppState::new();
    use_hook(move || {
        if let Some(p) = persisted {
            persistence::apply(&mut state, p);
        }
    });
    // Re-saves whenever any persisted field changes - `use_effect`'s
    // dependency tracking is based on which signals actually get `.read()`
    // during the closure call, which happens transitively inside
    // `persistence::save` here just as it would if the reads were written
    // out inline.
    use_effect(move || {
        persistence::save(&state, dark());
    });

    let mut is_drag_hovering = use_signal(|| false);
    let mut command_palette_open = use_signal(|| false);
    let mut active_tool = use_signal(|| ToolId::Search);

    // Resizable three-pane layout (epic §12). Plain mouse events, not
    // scroll/drag/keyboard - no verified platform gap applies here, so
    // this is ordinary drag-to-resize logic: a resize handle's
    // `onmousedown` records the starting cursor X and starting pane
    // width, `.app-shell`'s `onmousemove` (added below, alongside the
    // existing keyboard listener) applies the delta while a drag is in
    // progress, and `onmouseup` ends it. Column widths are clamped to
    // stay usable rather than lettable shrink to zero or grow to
    // swallow the whole window.
    let mut settings_width = use_signal(|| 380.0_f64);
    let mut preview_width = use_signal(|| 340.0_f64);
    let mut resizing: Signal<Option<ResizeTarget>> = use_signal(|| None);
    let mut resize_start_x = use_signal(|| 0.0_f64);
    let mut resize_start_width = use_signal(|| 0.0_f64);

    // Filesystem watching (epic §21) - retarget the watcher thread
    // whenever the search folder changes, and drain its change
    // notifications into a signal ResultsPanel surfaces as a hint.
    use_effect(move || {
        fs_watch::set_path(&state.search_path.read());
    });
    use_hook(move || {
        spawn(async move {
            loop {
                let Some(mutex) = fs_watch::CHANGE_EVENTS.get() else { break };
                let event = {
                    let mut rx = mutex.lock().await;
                    rx.recv().await
                };
                match event {
                    Some(paths) => {
                        if !*state.is_running.read() {
                            state.folder_changed_since_search.set(true);
                        }
                        // Incremental reindex (issue #6 Phase 1) - only
                        // worth queuing while fast-search indexing is
                        // actually on; the periodic flush task
                        // (state::run_incremental_reindex_flusher, spawned
                        // below) re-checks this same flag before doing any
                        // real work too, but there's no reason to even
                        // accumulate paths for an index that isn't being
                        // kept current.
                        if *state.index_for_fast_search.read() {
                            let mut pending = state.pending_reindex_paths.write();
                            for p in paths {
                                pending.insert(p.to_string_lossy().into_owned());
                            }
                        }
                    }
                    None => break,
                }
            }
        });
    });
    use_hook(move || {
        spawn(state::run_incremental_reindex_flusher(state));
    });

    // Drains the drag_drop::DROP_EVENTS channel `main.rs`'s hand-rolled
    // launch sequence feeds (see `launch::run`/`drag_drop.rs`) - the
    // application-facing side of the drag-and-drop workaround. Dropping a
    // folder sets it as the search folder directly; dropping a single
    // file uses its containing folder, since "drop a file to search a
    // folder" is a reasonable reading of the gesture and there's no
    // sensible alternative action for a bare file drop in this app.
    use_hook(|| {
        spawn(async move {
            loop {
                let Some(mutex) = crate::drag_drop::DROP_EVENTS.get() else { break };
                let event = {
                    let mut rx = mutex.lock().await;
                    rx.recv().await
                };
                match event {
                    Some(crate::drag_drop::DropEvent::Hovering) => is_drag_hovering.set(true),
                    Some(crate::drag_drop::DropEvent::HoverCancelled) => is_drag_hovering.set(false),
                    Some(crate::drag_drop::DropEvent::Dropped(path)) => {
                        is_drag_hovering.set(false);
                        let folder = if path.is_dir() { Some(path) } else { path.parent().map(|p| p.to_path_buf()) };
                        if let Some(folder) = folder {
                            state.search_path.set(folder.to_string_lossy().into_owned());
                        }
                    }
                    None => break,
                }
            }
        });
    });

    rsx! {
        style { {APP_CSS} }
        div {
            class: "app-shell",
            "data-theme": if dark() { "dark" } else { "light" },
            tabindex: "-1",
            // Global Ctrl/Cmd+K (command palette), Escape (close it), and
            // Up/Down/Enter (results-list selection/open) - verified this
            // renderer dispatches keydown regardless of modifier state or
            // focus target (see command_palette.rs's doc comment for the
            // exact evidence). `tabindex: "-1"` makes this div a valid
            // keyboard-event target even though it's never meant to be
            // tab-stopped into. Up/Down/Enter are gated on the command
            // palette being closed so they don't fight its own (separate)
            // list; row selection was previously mouse-only.
            onkeydown: move |e| {
                let mods = e.modifiers();
                if e.code() == Code::KeyK && (mods.contains(Modifiers::CONTROL) || mods.contains(Modifiers::META)) {
                    command_palette_open.set(!command_palette_open());
                } else if e.code() == Code::Escape && command_palette_open() {
                    command_palette_open.set(false);
                } else if !command_palette_open() {
                    match e.code() {
                        Code::ArrowDown => state.select_relative(1),
                        Code::ArrowUp => state.select_relative(-1),
                        Code::Enter => state.open_selected_result(),
                        _ => {}
                    }
                }
            },
            onmousemove: move |e| {
                let Some(target) = resizing() else { return };
                let dx = e.client_coordinates().x - resize_start_x();
                let new_width = match target {
                    ResizeTarget::Settings => (resize_start_width() + dx).clamp(260.0, 640.0),
                    ResizeTarget::Preview => (resize_start_width() - dx).clamp(260.0, 640.0),
                };
                match target {
                    ResizeTarget::Settings => settings_width.set(new_width),
                    ResizeTarget::Preview => preview_width.set(new_width),
                }
            },
            onmouseup: move |_| resizing.set(None),
            // Ambient-glow blobs approximating profile_capabilities' glassmorphic
            // look. That project uses `filter: blur(80px)` on these - verified
            // unsupported here (Stylo parses `filter` as a CSS value but
            // blitz-paint/src/render.rs never reads it off get_effects(), only
            // `.opacity`; same for `backdrop-filter`, zero references anywhere
            // in blitz-paint/blitz-dom). Radial gradients with a transparent
            // outer stop are confirmed-working and fade softly on their own
            // without needing real blur, so that's the substitute technique -
            // not a pixel-match, but the same "colored soft light behind glass
            // chrome" effect the source design uses blur for.
            div { class: "ambient-glow ambient-glow-a" }
            div { class: "ambient-glow ambient-glow-b" }
            div { class: "ambient-glow ambient-glow-c" }
            div { class: "shell",
                nav { class: "rail",
                    div { class: "brand",
                        div { class: "brand-mark", {icon_brand()} }
                        div { class: "brand-text",
                            span { class: "brand-name", "Toolbench" }
                            span { class: "brand-sub", "GS Engineering" }
                        }
                    }
                    div { class: "nav-label", "Tools" }
                    div { class: "nav-list",
                        button {
                            class: if active_tool() == ToolId::Search { "nav-item active" } else { "nav-item" },
                            onclick: move |_| active_tool.set(ToolId::Search),
                            span { class: "nav-icon", {icon_search()} }
                            span { class: "nav-item-body",
                                span { class: "nav-item-title", "Search Files" }
                                span { class: "nav-item-desc", "Keyword & regex search" }
                            }
                        }
                        button {
                            class: if active_tool() == ToolId::Bushing { "nav-item active" } else { "nav-item" },
                            onclick: move |_| active_tool.set(ToolId::Bushing),
                            span { class: "nav-icon", {icon_bushing()} }
                            span { class: "nav-item-body",
                                span { class: "nav-item-title", "Bushing Workbench" }
                                span { class: "nav-item-desc", "Interference-fit stress & margins" }
                            }
                        }
                        button {
                            class: if active_tool() == ToolId::PressureVessel { "nav-item active" } else { "nav-item" },
                            onclick: move |_| active_tool.set(ToolId::PressureVessel),
                            span { class: "nav-icon", {icon_pressure_vessel()} }
                            span { class: "nav-item-body",
                                span { class: "nav-item-title", "Pressure Vessel Analyzer" }
                                span { class: "nav-item-desc", "Lam\u{e9} stress, failure modes & min thickness" }
                            }
                        }
                        button {
                            class: if active_tool() == ToolId::Dupes { "nav-item active" } else { "nav-item" },
                            onclick: move |_| active_tool.set(ToolId::Dupes),
                            span { class: "nav-icon", {icon_dupes()} }
                            span { class: "nav-item-body",
                                span { class: "nav-item-title", "Duplicate Finder" }
                                span { class: "nav-item-desc", "Find identical files" }
                            }
                            span { class: "soon-pill", "Soon" }
                        }
                        button {
                            class: if active_tool() == ToolId::Rename { "nav-item active" } else { "nav-item" },
                            onclick: move |_| active_tool.set(ToolId::Rename),
                            span { class: "nav-icon", {icon_rename()} }
                            span { class: "nav-item-body",
                                span { class: "nav-item-title", "Batch Rename" }
                                span { class: "nav-item-desc", "Pattern-based renaming" }
                            }
                            span { class: "soon-pill", "Soon" }
                        }
                        button {
                            class: if active_tool() == ToolId::Logs { "nav-item active" } else { "nav-item" },
                            onclick: move |_| active_tool.set(ToolId::Logs),
                            span { class: "nav-icon", {icon_logs()} }
                            span { class: "nav-item-body",
                                span { class: "nav-item-title", "Log Analyzer" }
                                span { class: "nav-item-desc", "Parse & chart log files" }
                            }
                            span { class: "soon-pill", "Soon" }
                        }
                    }
                    div { class: "rail-footer",
                        button { class: "add-tool-btn", {icon_plus()} "Add tool" }
                        button {
                            class: "theme-toggle",
                            onclick: move |_| dark.set(!dark()),
                            if dark() { {icon_moon()} } else { {icon_sun()} }
                            span { if dark() { "Dark mode" } else { "Light mode" } }
                        }
                    }
                }
                div { class: "main",
                    div { class: "topbar",
                        div { class: "topbar-title",
                            h1 {
                                match active_tool() {
                                    ToolId::Search => "Search Files",
                                    ToolId::Bushing => "Bushing Workbench",
                                    ToolId::PressureVessel => "Pressure Vessel Analyzer",
                                    ToolId::Dupes => "Duplicate Finder",
                                    ToolId::Rename => "Batch Rename",
                                    ToolId::Logs => "Log Analyzer",
                                }
                            }
                            p {
                                match active_tool() {
                                    ToolId::Search => "Recursively search a folder for keyword filters",
                                    ToolId::Bushing => "Straight-bushing interference-fit stress & margins",
                                    ToolId::PressureVessel => "Full Lam\u{e9} stress, failure-mode margins & minimum wall thickness",
                                    ToolId::Dupes => "Find and clean up redundant files",
                                    ToolId::Rename => "Pattern-based bulk renaming",
                                    ToolId::Logs => "Parse and chart log files",
                                }
                            }
                        }
                        div { class: "topbar-actions",
                            button {
                                class: "palette-trigger",
                                title: "Command palette (Ctrl/Cmd+K)",
                                onclick: move |_| command_palette_open.set(true),
                                "\u{2318}K"
                            }
                        }
                    }
                    div { class: "stage",
                        if active_tool() == ToolId::Search {
                            div { class: "main-grid",
                                div { class: "settings-column", style: "width: {settings_width()}px;", SettingsPanel { state } }
                                div {
                                    class: "resize-handle",
                                    onmousedown: move |e| {
                                        resizing.set(Some(ResizeTarget::Settings));
                                        resize_start_x.set(e.client_coordinates().x);
                                        resize_start_width.set(settings_width());
                                    },
                                }
                                div { class: "results-column", ResultsPanel { state } }
                                div {
                                    class: "resize-handle",
                                    onmousedown: move |e| {
                                        resizing.set(Some(ResizeTarget::Preview));
                                        resize_start_x.set(e.client_coordinates().x);
                                        resize_start_width.set(preview_width());
                                    },
                                }
                                div { class: "preview-column", style: "width: {preview_width()}px;", PreviewPane { state } }
                            }
                        } else if active_tool() == ToolId::Bushing {
                            bushing_workbench::BushingWorkbench { dark }
                        } else if active_tool() == ToolId::PressureVessel {
                            pressure_vessel_workbench::PressureVesselWorkbench { dark }
                        } else if active_tool() == ToolId::Dupes {
                            PlaceholderTool {
                                title: "Duplicate Finder",
                                description: "Scan a folder tree for byte-identical or near-identical files and clear out redundant copies safely.",
                                icon: icon_dupes(),
                            }
                        } else if active_tool() == ToolId::Rename {
                            PlaceholderTool {
                                title: "Batch Rename",
                                description: "Rename hundreds of files at once with pattern matching, numbering, and a live preview before committing.",
                                icon: icon_rename(),
                            }
                        } else {
                            PlaceholderTool {
                                title: "Log Analyzer",
                                description: "Parse structured and unstructured log files, surface error spikes, and chart activity over time.",
                                icon: icon_logs(),
                            }
                        }
                    }
                }
            }
            if command_palette_open() {
                CommandPalette { state, dark, open: command_palette_open }
            }
            if state.context_menu.read().is_some() {
                ContextMenu { state }
            }
            if is_drag_hovering() {
                div { class: "drop-overlay",
                    div { class: "drop-overlay-card",
                        p { class: "drop-overlay-title", "\u{2193} Drop to search" }
                        p { class: "caption", "Drop a folder (or a file - its folder will be used) to set it as the search folder." }
                    }
                }
            }
        }
    }
}

// Design tokens + layout. Two rendering-correctness rules baked in here,
// not incidental style choices - see docs/epic-ui-performance-and-design.md's
// "Verified platform constraints" table before "simplifying" either:
//   1. Every scrollable list (.extension-list/.in-flight-list/.results-list)
//      is `display: block` with `overflow-y: auto` directly on it, and each
//      row inside is `display: block` with margin-bottom for spacing
//      (never `gap`, which only applies inside flex/grid) - NOT
//      `display: flex; flex-direction: column`. That combination is what
//      produced the original overlapping-rows bug; a plain block list is
//      the safer, verified-to-render-correctly pattern for a still-young
//      rendering engine like Blitz.
//   2. No `<select>` anywhere - see the `Dropdown` component in
//      components.rs.
// Design tokens: graphite/instrument-panel direction, adapted from
// profile_capabilities' theme.rs "Instrument" palette (a sibling Dioxus
// desktop app whose visual language this project was asked to follow),
// re-keyed to GS Engineering's own accent blue for brand continuity with
// the HTML report (search-core::report's CSS_BLOCK uses the same blue
// family - #0b5fa5/#6cb2f2). Explicit `data-theme` attribute on
// `.app-shell` (not a `prefers-color-scheme` media query) - stamped from
// a real signal in `App`, so the in-app toggle always wins outright
// rather than fighting the OS setting.
//
// Two rendering-correctness rules baked in here, not incidental style
// choices - see docs/epic-ui-performance-and-design.md's "Verified
// platform constraints" table before "simplifying" either:
//   1. Every scrollable list (.extension-list/.in-flight-list/.results-list)
//      is `display: block` with `overflow-y: auto` directly on it, and each
//      row inside is `display: block` with margin/padding for spacing
//      (never `gap`, which only applies inside flex/grid) - NOT
//      `display: flex; flex-direction: column`. That combination is what
//      produced the original overlapping-rows bug.
//   2. No `<select>` anywhere - see the `Dropdown` component in
//      components.rs. And no `backdrop-filter` anywhere - confirmed
//      absent from blitz-paint's render pipeline (grepped the crate
//      source directly), unlike box-shadow/gradients/border-radius/
//      color-mix/transitions, which are all confirmed present and used
//      freely below.
// Also: every flex child that can hold long/user-typed text gets an
// explicit `min-width: 0` (flex items default to `min-width: auto`,
// i.e. "never shrink below content size" - the root cause of the
// horizontal-scroll bug this replaced: a long path/filename in a flex
// row forced the whole row, and everything alongside it, wider than the
// window instead of wrapping/eliding).
const APP_CSS: &str = r#"
:root { color-scheme: dark; }

/* "Instrument" palette, adopted literally from profile_capabilities'
   theme.rs (the sibling Dioxus app whose look this was asked to match),
   not a re-keyed variant. --glass-* tokens carry the same alpha values as
   that project's `--glass`/`--glass-strong`/`--glass-border` too, minus
   `--glass-blur` - blur is dropped, not approximated with a fake value,
   because it's verified unrenderable here (see the ambient-glow div
   comment in App() for the source check). The glass look instead comes
   from translucent color-mix() surfaces + layered shadow + the glow blobs
   showing through the transparency - all three confirmed-working
   primitives. */
.app-shell[data-theme="dark"] {
    --fg: #eef0f4;
    --fg-muted: #8d96a3;
    --fg-subtle: #626a76;
    --bg: #14161b;
    --bg-raised: #1b1e25;
    --bg-sunken: #0e1013;
    --panel-bg: #1b1e25;
    --panel-hover: #282d37;
    --border: #2b303a;
    --border-strong: #3c424e;
    --accent: #3fbfe8;
    --accent-strong: #6ad2f2;
    --accent-fg: #05222c;
    --active: #e6a05f;
    --active-fg: #2c1608;
    --constraint: #a78bfa;
    --danger: #e2657a;
    --danger-bg: #3a1f24;
    --good: #52c98a;
    --good-bg: #173a2a;
    --warning: #e0b355;
    --shadow-sm: 0 1px 2px rgba(0, 0, 0, 0.35);
    --shadow-md: 0 8px 28px rgba(0, 0, 0, 0.4);
    --shadow-lg: 0 18px 48px rgba(0, 0, 0, 0.5);
    --glass-bg: color-mix(in srgb, #08090c 55%, transparent);
    --glass: rgba(255, 255, 255, 0.055);
    --glass-strong: rgba(255, 255, 255, 0.09);
    --glass-border: rgba(255, 255, 255, 0.11);
    --glass-border-strong: rgba(255, 255, 255, 0.2);
    --glow-a: #b18cf7; --glow-a-op: 0.28;
    --glow-b: #4fd2f0; --glow-b-op: 0.22;
    --glow-c: #f0a860; --glow-c-op: 0.14;
}
.app-shell[data-theme="light"] {
    --fg: #171a1f;
    --fg-muted: #626c78;
    --fg-subtle: #8992a1;
    --bg: #eef1f5;
    --bg-raised: #ffffff;
    --bg-sunken: #e4e8ed;
    --panel-bg: #ffffff;
    --panel-hover: #eef1f5;
    --border: #d9dfe6;
    --border-strong: #c2cad4;
    --accent: #1c7fae;
    --accent-strong: #0b6a97;
    --accent-fg: #ffffff;
    --active: #a8632c;
    --active-fg: #ffffff;
    --constraint: #7c5cd6;
    --danger: #c2394f;
    --danger-bg: #fbe9ec;
    --good: #29875a;
    --good-bg: #e6f5ec;
    --warning: #a1751f;
    --shadow-sm: 0 1px 2px rgba(20, 25, 35, 0.08);
    --shadow-md: 0 8px 24px rgba(20, 25, 35, 0.1);
    --shadow-lg: 0 18px 40px rgba(20, 25, 35, 0.14);
    --glass-bg: color-mix(in srgb, #ffffff 65%, transparent);
    --glass: rgba(20, 25, 35, 0.035);
    --glass-strong: rgba(20, 25, 35, 0.06);
    --glass-border: rgba(20, 25, 35, 0.09);
    --glass-border-strong: rgba(20, 25, 35, 0.16);
    --glow-a: #b18cf7; --glow-a-op: 0.16;
    --glow-b: #4fd2f0; --glow-b-op: 0.14;
    --glow-c: #f0a860; --glow-c-op: 0.1;
}

:root {
    --space-1: 4px; --space-2: 8px; --space-3: 12px; --space-4: 16px; --space-5: 20px; --space-6: 28px;
    --radius-sm: 6px; --radius-md: 9px; --radius-lg: 14px; --radius-pill: 999px;
    --ease: cubic-bezier(0.4, 0, 0.2, 1);
    --ease-spring: cubic-bezier(0.16, 1, 0.3, 1);
    --mono: ui-monospace, "SF Mono", Consolas, monospace;
}

* { box-sizing: border-box; min-width: 0; appearance: none; -webkit-appearance: none; }
html, body { height: 100%; overflow: hidden; }
body {
    margin: 0;
    font-family: -apple-system, "Segoe UI", Inter, Arial, sans-serif;
    font-size: 13px;
    line-height: 1.5;
    background: var(--bg);
    color: var(--fg);
}
button:focus-visible, input:focus-visible, [tabindex]:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
    border-radius: var(--radius-sm);
}
@media (prefers-reduced-motion: reduce) {
    *, *::before, *::after { transition-duration: 0.001ms !important; animation-duration: 0.001ms !important; }
}

.app-shell {
    display: flex; flex-direction: column; height: 100vh; width: 100vw;
    background: var(--bg); color: var(--fg); overflow: hidden;
    /* Positioning context for .ambient-glow / .drop-overlay - see those rules' comments. */
    position: relative;
}
/* Soft-edged color blobs standing in for profile_capabilities'
   `filter: blur(80px)` glow (see App()'s comment on the divs using these
   classes for why blur itself isn't used). A radial-gradient's own
   transparent outer stop fades softly without any blur operator, so the
   "colored ambient light" read survives even though the edge is a true
   gradient boundary rather than a Gaussian blur. Same positions/sizes/hues
   as the source design's .a/.b/.c blobs. */
.ambient-glow {
    position: absolute; z-index: 0; border-radius: 50%; pointer-events: none;
}
.ambient-glow-a {
    top: -120px; left: -80px; width: 480px; height: 480px;
    background: radial-gradient(circle, color-mix(in srgb, var(--glow-a) calc(var(--glow-a-op) * 100%), transparent) 0%, transparent 70%);
}
.ambient-glow-b {
    top: -140px; right: -100px; width: 420px; height: 420px;
    background: radial-gradient(circle, color-mix(in srgb, var(--glow-b) calc(var(--glow-b-op) * 100%), transparent) 0%, transparent 70%);
}
.ambient-glow-c {
    bottom: -160px; left: 12%; width: 380px; height: 380px;
    background: radial-gradient(circle, color-mix(in srgb, var(--glow-c) calc(var(--glow-c-op) * 100%), transparent) 0%, transparent 70%);
}
/* ---- Toolbench shell: a left tool-switcher rail + a right main column
   (topbar + stage). `.shell` is the single positioned/z-index-1 layer now
   (the old .title-bar/.main-grid each set their own `position: relative;
   z-index: 1` individually - one wrapper doing it once is equivalent,
   since the ambient-glow blobs are `.app-shell`'s direct children at
   z-index 0 either way). The command palette / context menu / drop
   overlay stay direct children of `.app-shell` (siblings of `.shell`,
   not nested inside it) specifically so `.drop-overlay`'s `position:
   absolute; inset: 0` keeps covering the whole `.app-shell` area, not
   just `.shell` - see that rule's own comment. ---- */
.shell { position: relative; z-index: 1; display: flex; flex: 1; min-height: 0; overflow: hidden; }

.rail {
    position: relative; z-index: 2; flex: none; width: 232px;
    display: flex; flex-direction: column;
    background: var(--glass-bg);
    border-right: 1px solid var(--glass-border);
    padding: var(--space-4) var(--space-3);
    overflow-y: auto;
}
.brand {
    display: flex; align-items: center; gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-5);
    margin-bottom: var(--space-2);
    border-bottom: 1px solid var(--border);
}
.brand-mark {
    flex: none;
    display: flex; align-items: center; justify-content: center;
    width: 30px; height: 30px;
    border-radius: var(--radius-md);
    background: linear-gradient(135deg, var(--accent), var(--constraint));
    color: var(--accent-fg);
    box-shadow: var(--shadow-sm);
}
.brand-mark svg { width: 17px; height: 17px; }
.brand-text { display: flex; flex-direction: column; line-height: 1.2; min-width: 0; }
.brand-name { font-weight: 700; font-size: 1.05em; letter-spacing: -0.01em; }
.brand-sub { font-size: 0.76em; color: var(--fg-subtle); text-transform: uppercase; letter-spacing: 0.07em; }

.nav-label {
    font-size: 0.76em; font-weight: 650; color: var(--fg-subtle);
    text-transform: uppercase; letter-spacing: 0.08em;
    padding: var(--space-3) var(--space-2) var(--space-1);
}
.nav-list { display: flex; flex-direction: column; gap: 2px; }
.nav-item {
    display: flex; align-items: center; gap: var(--space-3);
    padding: 9px var(--space-2); border-radius: var(--radius-md);
    border: 1px solid transparent; background: transparent; color: inherit; cursor: pointer;
    text-align: left; width: 100%;
    transition: background-color 0.12s var(--ease), border-color 0.12s var(--ease);
}
.nav-item:hover { background: var(--panel-hover); }
.nav-item.active { background: color-mix(in srgb, var(--accent) 14%, var(--panel-bg)); border-color: color-mix(in srgb, var(--accent) 30%, transparent); }
.nav-item.active .nav-icon, .nav-item.active .nav-item-title { color: var(--accent-strong); }
.nav-icon { width: 17px; height: 17px; flex: none; color: var(--fg-muted); stroke-width: 1.8; }
.nav-icon svg { width: 100%; height: 100%; }
.nav-item-body { display: flex; flex-direction: column; min-width: 0; gap: 1px; }
.nav-item-title { font-size: 0.97em; font-weight: 600; color: var(--fg); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.nav-item-desc { font-size: 0.83em; color: var(--fg-subtle); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.soon-pill {
    margin-left: auto; flex: none; font-size: 0.7em; font-weight: 700; letter-spacing: 0.04em;
    text-transform: uppercase; padding: 2px 6px; border-radius: var(--radius-pill);
    background: var(--bg-sunken); color: var(--fg-subtle); border: 1px solid var(--border);
}

.rail-footer { margin-top: auto; display: flex; flex-direction: column; gap: var(--space-2); padding-top: var(--space-3); border-top: 1px solid var(--border); }
.add-tool-btn {
    display: flex; align-items: center; gap: var(--space-2);
    padding: 8px var(--space-2); border-radius: var(--radius-md);
    background: transparent; border: 1px dashed var(--border-strong); color: var(--fg-subtle); cursor: pointer;
    font-size: 0.89em;
}
.add-tool-btn:hover { color: var(--accent-strong); border-color: var(--accent); }
.add-tool-btn svg { width: 14px; height: 14px; stroke-width: 2; }
.theme-toggle {
    display: flex; align-items: center; gap: var(--space-2);
    padding: 8px var(--space-2); border-radius: var(--radius-md);
    background: transparent; border: 1px solid var(--border); color: var(--fg-muted); cursor: pointer;
    font-size: 0.89em;
    transition: background-color 0.15s var(--ease), color 0.15s var(--ease);
}
.theme-toggle:hover { background: var(--panel-hover); color: var(--fg); }
.theme-toggle svg { width: 15px; height: 15px; stroke-width: 1.8; }

.main { flex: 1; min-width: 0; display: flex; flex-direction: column; overflow: hidden; }
.topbar {
    flex: none; display: flex; align-items: center; justify-content: space-between;
    padding: var(--space-4) var(--space-6) var(--space-3);
}
.topbar-title { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.topbar-title h1 { margin: 0; font-size: 1.28em; font-weight: 700; letter-spacing: -0.015em; }
.topbar-title p { margin: 0; font-size: 0.86em; color: var(--fg-muted); }
.topbar-actions { flex: none; display: flex; align-items: center; gap: var(--space-2); }

.stage { flex: 1; min-height: 0; padding: 0 var(--space-6) var(--space-6); overflow: auto; }
.panel {
    background: var(--glass-bg);
    border: 1px solid var(--glass-border); border-radius: var(--radius-lg);
    box-shadow: var(--shadow-md);
}

.main-grid { display: flex; height: 100%; gap: var(--space-2); overflow: hidden; }
.settings-column { flex: none; overflow-y: auto; overflow-x: hidden; padding-right: var(--space-1); }
.results-column { flex: 1; min-width: 0; overflow-y: auto; overflow-x: hidden; }
.preview-column { flex: none; overflow-y: auto; overflow-x: hidden; border-left: 1px solid var(--border); padding-left: var(--space-4); }
.resize-handle { flex: none; width: 6px; margin: 0 calc(-1 * var(--space-1)); cursor: ew-resize; position: relative; }
.resize-handle::after {
    content: ""; position: absolute; left: 2px; top: 0; bottom: 0; width: 2px;
    background: var(--border); border-radius: var(--radius-pill);
}
.resize-handle:hover::after { background: var(--accent); }

/* ---- Placeholder ("Coming soon") tool stage ---- */
.placeholder {
    height: 100%; display: flex; align-items: center; justify-content: center; flex-direction: column;
    gap: var(--space-3); text-align: center; padding: var(--space-6);
}
.placeholder-icon {
    width: 56px; height: 56px; border-radius: var(--radius-lg);
    background: var(--bg-sunken); color: var(--fg-subtle);
    display: flex; align-items: center; justify-content: center;
    margin-bottom: var(--space-2);
}
.placeholder-icon svg { width: 26px; height: 26px; stroke-width: 1.5; }
.placeholder h2 { margin: 0; font-size: 1.25em; font-weight: 700; }
.placeholder p { margin: 0; max-width: 360px; font-size: 0.96em; color: var(--fg-muted); }
.placeholder .soon-pill { margin: 0; }
.settings-panel, .results-panel { display: flex; flex-direction: column; gap: var(--space-3); min-width: 0; }

h3 {
    margin: var(--space-2) 0 0;
    font-size: 0.76em; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--fg-muted);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
details {
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-md);
    padding: 0 var(--space-3);
    background: var(--glass);
    box-shadow: var(--shadow-sm);
    transition: border-color 0.15s var(--ease), background-color 0.15s var(--ease);
}
details[open] { border-color: var(--glass-border-strong); background: var(--glass-strong); }
summary {
    cursor: pointer;
    font-weight: 650;
    font-size: 0.92em;
    padding: var(--space-3) 0;
    list-style: none;
    display: flex; align-items: center; gap: var(--space-2);
    color: var(--fg);
}
summary::-webkit-details-marker { display: none; }
summary::before {
    content: "\25B8";
    display: inline-block;
    color: var(--fg-subtle);
    font-size: 0.85em;
    transition: transform 0.15s var(--ease);
}
details[open] summary::before { transform: rotate(90deg); }
.expander-body {
    display: flex; flex-direction: column; gap: var(--space-3);
    padding: var(--space-1) 0 var(--space-4);
    border-top: 1px solid var(--border);
    margin-top: -1px;
}

.field { display: flex; flex-direction: column; gap: var(--space-1); font-size: 0.94em; flex: 1; min-width: 0; }
.field > span:first-child {
    font-weight: 600; font-size: 0.78em; color: var(--fg-muted);
    text-transform: uppercase; letter-spacing: 0.03em;
}
.field-inline { display: flex; align-items: center; gap: var(--space-2); font-size: 0.94em; padding: var(--space-1) 0; min-width: 0; }
.field-inline span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

input[type="text"], input[type="number"] {
    display: block;
    width: 100%;
    height: 34px;
    padding: 0 var(--space-3);
    line-height: 34px;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-sunken); color: var(--fg); font-size: 0.94em; font-family: inherit;
    transition: border-color 0.15s var(--ease), box-shadow 0.15s var(--ease);
}
input[type="text"]:focus, input[type="number"]:focus {
    outline: none; border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
}
input[type="checkbox"] {
    /* blitz-paint's draw_checkbox (form_controls.rs) does NOT read the CSS
       `accent-color` property at all - its own source comment admits this
       ("TODO this should be coming from css accent-color, but I couldn't
       find how to retrieve it") and falls back to painting the checked
       fill using the element's computed `color` (text-color) property
       instead. With `--fg` a near-white #eef0f4 in dark mode and no
       explicit `color` set here, a checked box painted near-white plus a
       white tick on top was visually indistinguishable from an unchecked
       box (also painted white by blitz's UA default) - "can't tell if
       it's ticked." Setting `color` (not `accent-color`) to the real
       accent is the actual fix for this renderer; `accent-color` is kept
       below in case a future blitz-paint version starts honoring it. */
    color: var(--accent);
    accent-color: var(--accent);
    flex: none; width: 16px; height: 16px; margin: 0;
}

.row { display: flex; gap: var(--space-2); align-items: flex-end; min-width: 0; }
.row .field { min-width: 0; }
.action-row { margin-top: var(--space-2); padding-bottom: var(--space-2); flex-wrap: wrap; }

button {
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--panel-bg); color: var(--fg);
    cursor: pointer; font-size: 0.9em; font-weight: 600; font-family: inherit;
    transition: background-color 0.15s var(--ease), border-color 0.15s var(--ease), color 0.15s var(--ease);
}
button:hover:not(:disabled) { background: var(--panel-hover); border-color: var(--border-strong); }
button:disabled { opacity: 0.4; cursor: default; }
button.primary {
    background: var(--accent); color: var(--accent-fg); border-color: var(--accent);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 45%, transparent), 0 2px 10px color-mix(in srgb, var(--accent) 25%, transparent);
}
button.primary:hover:not(:disabled) { background: var(--accent-strong); border-color: var(--accent-strong); }
.caption { color: var(--fg-muted); font-size: 0.8em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.field-error {
    margin: 0; padding: var(--space-1) var(--space-2);
    font-size: 0.8em; font-weight: 500; color: var(--danger);
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    border-radius: var(--radius-sm); word-break: break-word;
}

/* Custom dropdown - stands in for <select> (see the comment on `Dropdown` in components.rs) */
.select-box { position: relative; min-width: 0; }
.select-trigger {
    width: 100%; height: 34px; display: flex; justify-content: space-between; align-items: center;
    padding: 0 var(--space-3); border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-sunken); color: var(--fg); font-size: 0.94em; font-weight: 500; text-align: left;
    transition: border-color 0.15s var(--ease);
}
.select-trigger:hover { background: var(--panel-hover); }
.select-caret { flex: none; color: var(--fg-subtle); font-size: 0.75em; margin-left: var(--space-2); }
.select-menu {
    display: block; margin-top: var(--space-1); border: 1px solid var(--glass-border-strong);
    border-radius: var(--radius-sm); background: var(--bg-raised); overflow: hidden; box-shadow: var(--shadow-md);
}
.select-option { display: block; padding: var(--space-2) var(--space-3); font-size: 0.9em; cursor: pointer; }
.select-option:hover { background: var(--panel-hover); }
.select-option.selected { background: color-mix(in srgb, var(--accent) 18%, var(--panel-bg)); color: var(--accent-strong); font-weight: 650; }

/* Scrollable lists - plain block layout throughout, see the top-of-file note */
.extension-list, .in-flight-list, .results-list {
    display: block;
    overflow-y: auto;
    overflow-x: hidden;
}
.extension-list {
    max-height: 190px; border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: var(--space-1) var(--space-2); background: var(--bg-sunken);
}
.in-flight-list, .results-list { max-height: 260px; }
.extension-list .field-inline { display: block; padding: var(--space-1) 0; }

.progress-block { display: flex; flex-direction: column; gap: var(--space-2); }
.progress-bar { width: 100%; height: 6px; accent-color: var(--accent); }

.hit-row {
    display: block;
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
    transition: background-color 0.12s var(--ease);
}
.hit-row:hover { background: var(--panel-hover); cursor: pointer; }
.hit-row.selected { background: color-mix(in srgb, var(--accent) 12%, var(--panel-bg)); border-left: 2px solid var(--accent); }
.hit-row:last-child { border-bottom: none; }
.hit-row-top { display: flex; justify-content: space-between; align-items: baseline; gap: var(--space-2); min-width: 0; }
.hit-name {
    font-weight: 600; min-width: 0; flex: 1 1 auto;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
/* The trailing number (hit count / score / elapsed seconds) on each row -
   flex items default to flex-shrink:1, so without this it competes for
   space with .hit-name and can itself get clipped under space pressure
   ("content that does not fit, especially numbers" - the name should be
   the thing that shrinks/ellipsizes, never the number). */
.hit-value {
    flex: none; min-width: 0;
    font-family: var(--mono); font-variant-numeric: tabular-nums;
}
.hit-row .caption { font-family: var(--mono); font-size: 0.76em; }
.hit-row-bottom {
    display: flex; justify-content: space-between; align-items: center; gap: var(--space-2);
    min-width: 0; margin-top: 2px;
}
.hit-row-bottom .caption { flex: 1 1 auto; min-width: 0; }
.hit-actions { flex: none; display: flex; gap: var(--space-1); opacity: 0; transition: opacity 0.12s var(--ease); }
.hit-row:hover .hit-actions { opacity: 1; }
.hit-action {
    padding: 2px var(--space-2); font-size: 0.76em; font-weight: 600;
    border-radius: var(--radius-sm); background: transparent;
}
.hit-action:hover { background: var(--panel-bg); border-color: var(--border-strong); }

.empty-state {
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: var(--space-2); text-align: center;
    padding: var(--space-6) var(--space-4);
    color: var(--fg-muted);
}
.empty-state-title { margin: 0; font-size: 1em; font-weight: 650; color: var(--fg); }

.chip-row { display: flex; flex-wrap: wrap; gap: var(--space-1); }
.chip {
    max-width: 100%;
    padding: 3px var(--space-2);
    font-size: 0.78em; font-weight: 600;
    border-radius: var(--radius-pill);
    background: var(--bg-sunken); color: var(--fg-muted);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.chip:hover { color: var(--fg); border-color: var(--border-strong); background: var(--panel-hover); }
/* A preset chip paired with its own small delete button - two adjacent
   buttons that read as one pill (the round radius only on the outer
   edges), unlike a plain `.chip` (recent searches), which has no per-item
   delete affordance at all. */
.chip-removable { display: inline-flex; max-width: 100%; }
.chip-removable .chip { border-radius: var(--radius-pill) 0 0 var(--radius-pill); border-right: none; }
.chip-remove {
    flex: none; padding: 3px 6px; font-size: 0.72em;
    border-radius: 0 var(--radius-pill) var(--radius-pill) 0;
    background: var(--bg-sunken); color: var(--fg-muted);
}
.chip-remove:hover { color: var(--danger); background: var(--panel-hover); border-color: var(--danger); }

.stat-bars { display: flex; flex-direction: column; gap: 3px; margin: -4px 0 4px; }
.stat-bar-row { display: flex; align-items: center; gap: var(--space-2); font-size: 0.78em; min-width: 0; }
.stat-bar-label {
    flex: none; width: 64px; color: var(--fg-muted); font-family: var(--mono);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.stat-bar-track { flex: 1 1 auto; min-width: 0; height: 6px; border-radius: var(--radius-pill); background: var(--bg-sunken); overflow: hidden; }
.stat-bar-fill { display: block; height: 100%; background: var(--accent); border-radius: var(--radius-pill); }
.stat-bar-count { flex: none; width: 28px; text-align: right; color: var(--fg-muted); font-family: var(--mono); font-variant-numeric: tabular-nums; }

/* `position: fixed` isn't implemented as true viewport-relative
   positioning in this renderer - it's silently treated identically to
   `position: absolute` (confirmed by reading stylo_taffy's own position()
   conversion function, which has a literal `// TODO: support
   position:fixed and sticky` next to the line mapping both to the same
   Taffy::Position::Absolute). `.app-shell` already fills the viewport
   (100vw/100vh) and is the overlay's direct positioned ancestor, so
   `position: absolute; inset: 0;` on `.drop-overlay` covers the same area
   a real `fixed` overlay would here - deliberately not using `fixed`. */
.drop-overlay {
    position: absolute; inset: 0; z-index: 50;
    display: flex; align-items: center; justify-content: center;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
}
.drop-overlay-card {
    display: flex; flex-direction: column; align-items: center; gap: var(--space-2);
    padding: var(--space-6); border-radius: var(--radius-lg);
    background: var(--glass-bg); border: 2px dashed var(--accent);
    box-shadow: var(--shadow-lg);
    text-align: center; max-width: 320px;
}
.drop-overlay-title { margin: 0; font-size: 1.1em; font-weight: 700; color: var(--fg); }

.palette-trigger {
    height: 30px; padding: 0 var(--space-3);
    font-family: var(--mono); font-size: 0.8em; font-weight: 600;
    border-radius: var(--radius-sm); color: var(--fg-muted);
}
.palette-trigger:hover { color: var(--fg); }

/* Same position:absolute-not-fixed reasoning as .drop-overlay above. */
.palette-overlay {
    position: absolute; inset: 0; z-index: 60;
    display: flex; align-items: flex-start; justify-content: center;
    padding-top: 12vh;
    background: color-mix(in srgb, var(--bg) 55%, transparent);
}
.palette-card {
    width: 420px; max-width: 90vw; max-height: 60vh;
    display: flex; flex-direction: column;
    background: var(--glass-bg); border: 1px solid var(--glass-border-strong);
    border-radius: var(--radius-lg); box-shadow: var(--shadow-lg);
    overflow: hidden;
}
.palette-input {
    height: 42px; padding: 0 var(--space-4); border: none; border-bottom: 1px solid var(--border);
    background: transparent; color: var(--fg); font-size: 1em; font-family: inherit;
}
.palette-input:focus { outline: none; }
.palette-list { display: block; overflow-y: auto; overflow-x: hidden; padding: var(--space-1); }
.palette-item { display: block; padding: var(--space-2) var(--space-3); border-radius: var(--radius-sm); cursor: pointer; font-size: 0.92em; }
.palette-item:hover { background: var(--panel-hover); color: var(--accent-strong); }
.palette-empty { display: block; padding: var(--space-3); }
.palette-group-label {
    display: block; padding: var(--space-2) var(--space-3) var(--space-1);
    text-transform: uppercase; letter-spacing: 0.04em; font-weight: 700;
}

.ctx-overlay { position: absolute; inset: 0; z-index: 70; }
.ctx-menu {
    position: absolute;
    display: flex; flex-direction: column;
    min-width: 190px; padding: var(--space-1);
    background: var(--glass-bg); border: 1px solid var(--glass-border-strong);
    border-radius: var(--radius-md); box-shadow: var(--shadow-lg);
}
.ctx-menu-title {
    display: block; padding: var(--space-2) var(--space-3) var(--space-1);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.ctx-item {
    display: block; width: 100%; text-align: left;
    padding: var(--space-2) var(--space-3); border: none;
    background: transparent; border-radius: var(--radius-sm); font-weight: 500;
}
.ctx-item:hover { background: var(--panel-hover); }

.folder-changed-hint {
    display: flex; align-items: center; justify-content: space-between; gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm); font-size: 0.85em; font-weight: 500;
    background: color-mix(in srgb, var(--active) 18%, var(--panel-bg));
    color: var(--active);
}
.folder-changed-hint button {
    flex: none; color: var(--active); border-color: color-mix(in srgb, var(--active) 45%, var(--border));
}

.pagination { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); padding: var(--space-1) 0; }

.preview-pane { display: flex; flex-direction: column; gap: var(--space-2); height: 100%; }
.preview-pane-empty { align-items: center; justify-content: center; text-align: center; }
.preview-header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); min-width: 0; }
.preview-title { font-weight: 700; font-size: 1em; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.preview-actions-visible { opacity: 1; }
.preview-path { font-family: var(--mono); word-break: break-all; margin: 0; }
.preview-matches { display: block; overflow-y: auto; overflow-x: hidden; flex: 1; }
.preview-match { display: block; padding: var(--space-2) 0; border-bottom: 1px solid var(--border); }
.preview-match:last-child { border-bottom: none; }
.preview-lineno { display: block; margin-bottom: 2px; }
.preview-context {
    display: block; margin: 0 0 2px; padding: var(--space-1) var(--space-2);
    font-family: var(--mono); font-size: 0.82em; white-space: pre-wrap; word-break: break-word;
    color: var(--fg-muted); background: transparent; border-radius: var(--radius-sm);
}
.preview-matchline { color: var(--fg); background: var(--bg-sunken); }
.preview-context mark { background: color-mix(in srgb, var(--accent) 55%, transparent); color: var(--fg); border-radius: 2px; padding: 0 1px; }

/* ---- Bushing Workbench (bushing_workbench.rs) ----
   Single flowing column, matching the approved mockup shape exactly - no
   fixed-width sidebar, no independent inner scroll regions. `.stage`
   (`overflow:auto`) already provides one page-level scrollbar for every
   other tool in this app; this used to opt out of that with its own
   `height:100%; overflow:hidden` two-pane shell, which was both a mockup
   mismatch and a worse scrolling experience (two scrollbars instead of
   one). `.bushing-page` intentionally sets no height/overflow at all. */
.bushing-page { display: flex; flex-direction: column; gap: var(--space-4); padding: var(--space-4) 0; }
.bushing-card {
    background: var(--glass); border: 1px solid var(--glass-border); border-radius: var(--radius-md);
    padding: var(--space-4); display: flex; flex-direction: column; gap: var(--space-3);
}
.bushing-card-head { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); flex-wrap: wrap; }
.bushing-card-title { margin: 0; font-size: 1em; font-weight: 700; }
.bushing-card-sub { margin: -6px 0 0; font-size: 0.82em; color: var(--fg-subtle); }

/* Every plain number/text input inside the workbench sized to fit its
   own content instead of the app-wide `input[type="number"] { width:
   100% }` rule (main.rs's generic form-field styling, shared with every
   other tool's settings panel - not touched here, scoped to
   `.bushing-card` only). Numbers: ~7 characters ("-0.0500", "1000.00")
   is enough for every value this tool actually shows - matches the
   `.spec-input` convention the Fit step's tables already use (excluded
   below so this rule can't win the specificity tie and un-narrow them).
   Material dropdowns: sized to the longest real material name
   (`mechanics-core::materials::MATERIALS`, "Al 7075-T6 (typical)" /
   "Al 2024-T3 (typical)" are the longest at 21 characters) rather than
   the panel width, and rather than resizing per-selection (`width:
   max-content` would do that, and would visibly jump/jitter as the
   selection changes - a fixed width sized to the worst case doesn't). */
.bushing-card input[type="number"]:not(.spec-input),
.bushing-card input[type="text"]:not(.spec-input) { width: 90px; flex: none; }
.bushing-card .select-trigger { width: 230px; flex: none; }
/* `.field` is `flex:1` app-wide (SettingsPanel etc. rely on it to fill a
   row) - inside a `.bushing-card` field-row specifically, narrowing the
   input above without this just moves the wasted space from "inside a
   too-wide input" to "an empty gap after a now-narrow one," spread
   across however many fields share the row. `flex:none` makes each
   field hug its own label/input width instead, so a row of several
   fields sits together instead of spreading out to fill it. */
.bushing-card .field-row .field { flex: none; }

/* ---- Workflow shell: top stepper + step-gated workspace + persistent
   design-status rail (Phase 1 of the workflow redesign - see the
   session's approved plan). Only `current_step`'s cards render in
   `.bushing-workspace`; `.bushing-status-rail` stays visible regardless
   of which step is open. */
.bushing-stepper { display: flex; align-items: center; gap: 2px; overflow-x: auto; padding-bottom: 2px; }
.bushing-step-pill {
    flex: none; display: flex; align-items: center; gap: var(--space-2); padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-pill); cursor: pointer; font-weight: 650; font-size: 0.86em; color: var(--fg-muted);
    transition: background-color 0.15s var(--ease), color 0.15s var(--ease);
}
.bushing-step-pill:not(:last-child)::after { content: "\2014"; margin-left: var(--space-2); color: var(--border-strong); font-weight: 400; }
.bushing-step-pill:hover { background: var(--panel-hover); }
.bushing-step-pill.bushing-step-current { background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent-strong); }
.bushing-step-num {
    flex: none; width: 18px; height: 18px; border-radius: 50%; display: flex; align-items: center; justify-content: center;
    font-size: 0.72em; font-family: var(--mono); border: 1.5px solid var(--border-strong); color: var(--fg-subtle);
}
.bushing-step-current .bushing-step-num { background: var(--accent); border-color: var(--accent); color: var(--accent-fg); }

.bushing-workspace-split { display: flex; align-items: flex-start; gap: var(--space-4); }
.bushing-workspace { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--space-4); }

.bushing-status-rail { flex: none; width: 240px; border: 1px solid var(--glass-border); border-radius: var(--radius-md); background: var(--glass); overflow: hidden; }
.bushing-status-head { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-3); border-bottom: 1px solid var(--glass-border); }
.bushing-status-dot { flex: none; width: 11px; height: 11px; border-radius: 50%; }
.bushing-status-pass .bushing-status-dot { background: var(--good); box-shadow: 0 0 0 3px var(--good-bg); }
.bushing-status-review .bushing-status-dot { background: var(--warning); box-shadow: 0 0 0 3px color-mix(in srgb, var(--warning) 18%, transparent); }
.bushing-status-text { font-weight: 700; font-size: 0.92em; }
.bushing-status-pass .bushing-status-text { color: var(--good); }
.bushing-status-review .bushing-status-text { color: var(--warning); }
.bushing-status-sub { font-size: 0.76em; color: var(--fg-subtle); margin-top: 1px; }
.bushing-checklist { display: flex; flex-direction: column; gap: 1px; padding: var(--space-2); }
.bushing-check-row { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-2); border-radius: var(--radius-sm); cursor: pointer; font-size: 0.82em; }
.bushing-check-row:hover { background: var(--panel-hover); }
.bushing-check-row.bushing-check-attn .bushing-check-name { color: var(--fg); font-weight: 650; }
.bushing-check-dot { flex: none; width: 8px; height: 8px; border-radius: 50%; }
.bushing-check-dot.ok { background: var(--good); }
.bushing-check-dot.warn { background: var(--warning); }
.bushing-check-dot.crit { background: var(--danger); }
.bushing-check-dot.neutral { background: var(--border-strong); }
.bushing-check-name { flex: 1; min-width: 0; color: var(--fg-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.bushing-check-val { flex: none; font-size: 0.92em; color: var(--fg-subtle); }
.field-label { font-weight: 600; font-size: 0.78em; color: var(--fg-muted); text-transform: uppercase; letter-spacing: 0.03em; }
.field-row { display: flex; gap: var(--space-2); }
.field-row .field { min-width: 0; }
.field-inline-row { display: flex; align-items: center; gap: var(--space-2); }

.field-group {
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3) var(--space-3);
    display: flex; flex-direction: column; gap: var(--space-2);
}
.field-group-label { font-weight: 650; font-size: 0.76em; color: var(--fg-subtle); text-transform: uppercase; letter-spacing: 0.03em; }
.field-group-body { display: flex; flex-direction: column; gap: var(--space-2); }
.field-group-body .field-row { margin: 0; }

.bushing-card .chip { cursor: pointer; transition: background-color 0.15s var(--ease), color 0.15s var(--ease), border-color 0.15s var(--ease); border: 1px solid transparent; }
.bushing-card .chip.selected { background: var(--accent); color: var(--accent-fg); }

.reamer-picker-trigger { flex: none; font-size: 0.82em; padding: var(--space-1) var(--space-3); }
.reamer-picker {
    position: relative; width: 100%; margin-top: var(--space-2);
    background: var(--bg-raised); border: 1px solid var(--border-strong); border-radius: var(--radius-md);
    box-shadow: var(--shadow-md); overflow: hidden;
}
.reamer-picker-header {
    display: flex; align-items: center; justify-content: space-between; gap: var(--space-2);
    padding: var(--space-2) var(--space-3); font-size: 0.82em; font-weight: 650; color: var(--fg-muted);
    border-bottom: 1px solid var(--border); background: var(--bg-sunken);
}
.reamer-picker-close { flex: none; padding: 2px 8px; font-size: 0.8em; }
.reamer-picker-list { max-height: 240px; overflow-y: auto; display: flex; flex-direction: column; }
.reamer-picker-row {
    display: flex; align-items: center; gap: var(--space-2); width: 100%;
    padding: var(--space-2) var(--space-3); border: none; border-radius: 0; border-bottom: 1px solid var(--border);
    background: transparent; text-align: left; font-weight: 500;
}
.reamer-picker-row:last-child { border-bottom: none; }
.reamer-picker-row:hover { background: var(--panel-hover); }
.reamer-picker-size { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.reamer-picker-nominal { flex: none; font-family: var(--mono); color: var(--fg-muted); font-size: 0.9em; }
.reamer-picker-tier { flex: none; font-size: 0.72em; color: var(--fg-subtle); }

/* Sized for their real home: the Results step's own "cross-section" card
   (a full card of width to itself, not squeezed next to 5 stat cards at
   the top of every step like the earlier always-visible version was -
   "small... not useful" was the direct complaint that removed that). */
.bushing-viz-panes { display: flex; gap: var(--space-3); flex: none; }
.bushing-viz-pane {
    position: relative; border: 1px solid var(--glass-border); border-radius: var(--radius-md);
    background: var(--glass); padding: var(--space-2); overflow: hidden;
}
.bushing-viz-overview { width: 160px; }
.bushing-viz-detail { width: 380px; }
.bushing-viz-img { display: block; width: 100%; height: auto; }
.bushing-viz-tag {
    position: absolute; top: var(--space-1); left: var(--space-1); z-index: 1;
    font-size: 0.68em; font-weight: 650; text-transform: uppercase; letter-spacing: 0.02em;
    color: var(--fg-muted); background: var(--bg-raised); border: 1px solid var(--border);
    border-radius: var(--radius-sm); padding: 1px 6px;
}
.bushing-viz-expand {
    position: absolute; top: var(--space-1); right: var(--space-1); z-index: 1;
    width: 22px; height: 22px; padding: 4px; line-height: 1;
    border-radius: var(--radius-sm); background: var(--bg-raised); border: 1px solid var(--border-strong);
    color: var(--fg-muted);
}
.bushing-viz-expand:hover { color: var(--fg); border-color: var(--accent); }
.bushing-viz-expand svg { width: 100%; height: 100%; }

/* User report: the close button became unreachable once the rendered
   drawing was taller than the viewport - `max-height`/`overflow-y:auto`
   on an inner wrapper (the previous version of this rule) wasn't
   reliably clipping/scrolling in this renderer (the same class of
   layout surprise as the width-circularity bug two rounds earlier), so
   the card just grew to its full content height and, centered by
   `align-items:center`, extended above and below the viewport with no
   way back to the top where the button (position:absolute on the card)
   lived. Fixed by moving the scroll responsibility to the BACKDROP
   itself (`overflow-y:auto` on the `position:fixed` element - proven
   elsewhere in this app, `.stage` does exactly this) and pinning the
   close button to the VIEWPORT (`position:fixed`, not `absolute` on the
   card) so it is reachable regardless of how tall the card's content
   grows or how far the backdrop is scrolled. */
.bushing-viz-lightbox-backdrop {
    position: fixed; inset: 0; background: rgba(10, 12, 14, 0.6);
    display: flex; justify-content: center; align-items: flex-start;
    overflow-y: auto; padding: var(--space-6);
    z-index: 50;
}
.bushing-viz-lightbox-card {
    /* A definite `width` (not `max-width`), on purpose: this card is a
       flex item, i.e. shrink-to-fit-sized by its own content. The `img`
       inside is `width:100%; height:auto` - a percentage width resolved
       against a shrink-to-fit parent that in turn is sized BY that same
       img is a circular case that (whether in Blitz's layout engine or
       otherwise) has no reliable single-pass answer, and was the actual
       cause of the lightbox rendering as an empty box: the dock's
       overview/detail panes work precisely because their parent
       (`.bushing-viz-pane`) has a definite pixel `width`, breaking the
       same cycle. `max-width` alone (an earlier version of this rule)
       keeps the container shrink-to-fit and reintroduces it. */
    position: relative; width: 720px; max-width: 90vw; margin: 0 0 var(--space-6);
    background: var(--bg-raised); border: 1px solid var(--border); border-radius: var(--radius-lg);
    box-shadow: var(--shadow-md); padding: var(--space-4);
}
.bushing-viz-lightbox-close {
    position: fixed; top: var(--space-4); right: var(--space-4); z-index: 51;
    width: 34px; height: 34px; padding: 7px; line-height: 1; font-size: 1em;
    border-radius: var(--radius-sm); background: var(--bg-sunken); border: 1px solid var(--border-strong);
    color: var(--fg-muted); box-shadow: var(--shadow-md);
}
.bushing-viz-lightbox-close:hover { color: var(--fg); }
.bushing-viz-lightbox-close svg { width: 100%; height: 100%; }
.bushing-viz-lightbox-img {
    display: block; width: 100%; height: auto; min-height: 200px;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
}

.spec-table-wrap { overflow-x: auto; }
/* Results table (7 columns, the widest spec-table in this app): must
   only ever scroll vertically, never horizontally. `table-layout: fixed`
   plus letting the Quantity column wrap onto a second line (instead of
   forcing every column to stay on one row, which is what pushes a wide
   table past its container and triggers the wrap's default
   `overflow-x: auto`) keeps it within the workspace width regardless of
   how long a check's name or how wide its numbers get. */
.spec-table-wrap.no-hscroll { overflow-x: hidden; }
.spec-table-wrap.no-hscroll .spec-table { table-layout: fixed; }
.spec-table-wrap.no-hscroll .spec-table td:first-child,
.spec-table-wrap.no-hscroll .spec-table th:first-child { white-space: normal; word-break: break-word; width: 30%; }
.spec-table { width: 100%; border-collapse: collapse; font-size: 0.86em; }
.spec-table th {
    text-align: left; font-weight: 650; color: var(--fg-muted); font-size: 0.78em;
    text-transform: uppercase; letter-spacing: 0.03em; padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border-strong);
}
.spec-table th.num, .spec-table td.num { text-align: right; }
.spec-table td { padding: var(--space-1) var(--space-2); border-bottom: 1px solid var(--border); vertical-align: middle; }
.spec-table tbody tr:last-child td { border-bottom: none; }
.spec-table .spec-row-derived td { background: color-mix(in srgb, var(--fg-subtle) 12%, transparent); }
.spec-input {
    width: 72px; border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised); color: var(--fg); padding: 3px 6px; font-size: 0.95em; text-align: right;
}
.spec-row-derived .spec-input { background: transparent; }
.range-cell { font-weight: 650; }

.src-chip {
    font-size: 0.68em; font-weight: 700; letter-spacing: 0.03em; text-transform: uppercase;
    padding: 1px 7px; border-radius: var(--radius-sm); white-space: nowrap;
}
.src-direct { background: color-mix(in srgb, var(--active) 20%, transparent); color: var(--active); }
.src-derived { background: var(--bg-sunken); color: var(--fg-subtle); }
.src-calculated { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent-strong); }

.bushing-headline {
    display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap;
    gap: var(--space-4); padding: var(--space-3) var(--space-4); border-radius: var(--radius-md);
    border: 1px solid var(--glass-border); background: var(--glass);
}
.bushing-headline-status { display: flex; align-items: center; gap: var(--space-3); flex: none; }
.bushing-headline-dot { flex: none; width: 12px; height: 12px; border-radius: 50%; }
.bushing-headline.pass .bushing-headline-dot { background: var(--good); box-shadow: 0 0 0 4px var(--good-bg); }
.bushing-headline.review .bushing-headline-dot { background: var(--warning); box-shadow: 0 0 0 4px color-mix(in srgb, var(--warning) 18%, transparent); }
.bushing-headline-text { display: block; font-weight: 700; font-size: 1.05em; letter-spacing: 0.02em; }
.bushing-headline.pass .bushing-headline-text { color: var(--good); }
.bushing-headline.review .bushing-headline-text { color: var(--warning); }
.bushing-headline-sub { display: block; font-size: 0.78em; color: var(--fg-subtle); margin-top: 1px; }
.bushing-mini-stats { display: flex; gap: var(--space-5); flex-wrap: wrap; }
.bushing-mini-stat { display: flex; flex-direction: column; gap: 2px; min-width: 110px; }
.bushing-mini-label { font-size: 0.72em; color: var(--fg-muted); text-transform: uppercase; letter-spacing: 0.03em; }
.bushing-mini-val { font-weight: 650; font-variant-numeric: tabular-nums; }

.fab-card .bushing-card-head { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); }
.fab-badge {
    flex: none; font-size: 0.72em; font-weight: 700; letter-spacing: 0.02em; text-transform: uppercase;
    padding: 3px var(--space-2); border-radius: var(--radius-pill);
}
.fab-badge.ready { background: var(--good-bg); color: var(--good); }
.fab-badge.review { background: color-mix(in srgb, var(--warning) 18%, transparent); color: var(--warning); }
.fab-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: var(--space-3) var(--space-4); margin-top: var(--space-3); }
.fab-item.wide { grid-column: 1 / -1; }
.fab-note { margin-top: var(--space-3); font-size: 0.78em; color: var(--fg-subtle); }

.checks-list { display: flex; flex-direction: column; gap: var(--space-1); margin-top: var(--space-2); }
.check-item { display: flex; align-items: center; gap: var(--space-3); padding: var(--space-3) var(--space-1); border-bottom: 1px solid var(--border); }
.check-item:last-child { border-bottom: none; }
.check-dot { flex: none; width: 8px; height: 8px; border-radius: 50%; }
.check-dot.ok { background: var(--good); }
.check-dot.fail { background: var(--danger); }
.check-dot.neutral { background: var(--border-strong); }
.check-name { flex: none; width: 168px; font-size: 0.84em; color: var(--fg-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.value-track { position: relative; flex: 1; height: 16px; margin: 0 var(--space-2); }
.value-track .rail { position: absolute; top: 50%; left: 0; right: 0; height: 3px; transform: translateY(-50%); background: var(--bg-sunken); border-radius: var(--radius-pill); }
.value-track .allow-line { position: absolute; top: -3px; bottom: -3px; width: 2px; background: var(--fg-subtle); }
.value-track .allow-tag {
    position: absolute; top: -16px; transform: translateX(-50%); font-size: 0.68em; color: var(--fg-subtle);
    white-space: nowrap; font-variant-numeric: tabular-nums;
}
.value-track .whisker { position: absolute; top: 50%; height: 5px; transform: translateY(-50%); border-radius: var(--radius-pill); }
.value-track .whisker.ok { background: var(--good); }
.value-track .whisker.fail { background: var(--danger); }
.value-track .whisker-point { position: absolute; top: 50%; width: 9px; height: 9px; margin-left: -4.5px; transform: translateY(-50%); border-radius: 50%; }
.value-track .whisker-point.ok { background: var(--good); }
.value-track .whisker-point.fail { background: var(--danger); }
.value-track .end-label {
    position: absolute; font-size: 0.68em; color: var(--fg-subtle); white-space: nowrap; font-variant-numeric: tabular-nums;
}
.value-track .end-label.lo { bottom: -15px; transform: translateX(-50%); }
.value-track .end-label.hi { bottom: -15px; transform: translateX(-50%); }
.value-track .end-label.point { top: 14px; transform: translateX(-50%); }
.check-item .ms-pill { flex: none; margin-left: auto; }

.bushing-alert-action { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); flex-wrap: wrap; cursor: default; }
.bushing-alert-msg { flex: 1; min-width: 200px; }
.bushing-alert-action-btn {
    flex: none; font-size: 0.86em; font-weight: 650; padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-pill); border: 1px solid currentColor; background: transparent; color: inherit;
}
.bushing-alert-action-btn:hover { background: color-mix(in srgb, currentColor 14%, transparent); }

.ms-pill {
    align-self: flex-start; font-size: 0.76em; font-weight: 700; padding: 2px var(--space-2);
    border-radius: var(--radius-pill); font-variant-numeric: tabular-nums;
}
.ms-pass { background: var(--good-bg); color: var(--good); }
.ms-marginal { background: color-mix(in srgb, var(--warning) 18%, transparent); color: var(--warning); }
.ms-fail { background: var(--danger-bg); color: var(--danger); }
.ms-neutral { background: var(--bg-sunken); color: var(--fg-subtle); }

.bushing-alert {
    padding: var(--space-3); border-radius: var(--radius-md); font-size: 0.9em; font-weight: 550;
    border: 1px solid transparent;
}
.bushing-alert-fail { background: var(--danger-bg); color: var(--danger); border-color: color-mix(in srgb, var(--danger) 35%, transparent); }
.bushing-alert-warn { background: color-mix(in srgb, var(--warning) 14%, transparent); color: var(--warning); border-color: color-mix(in srgb, var(--warning) 35%, transparent); }

.bushing-detail-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: var(--space-2) var(--space-4); }
.detail-field { display: flex; flex-direction: column; gap: 2px; }
.detail-field-label { font-size: 0.76em; color: var(--fg-muted); }
.detail-field-value { font-weight: 650; font-variant-numeric: tabular-nums; }

.bushing-derivation-toggle { display: flex; }
.link-button {
    padding: 0; border: none; background: none; color: var(--accent); font-weight: 650;
    font-size: 0.88em; text-decoration: underline; text-underline-offset: 2px;
}
.link-button:hover:not(:disabled) { background: none; color: var(--accent-strong); }

.derivation-block { display: flex; flex-direction: column; gap: var(--space-3); padding-top: var(--space-1); }
.derivation-note { font-size: 0.82em; color: var(--fg-subtle); line-height: 1.5; margin: 0 0 var(--space-2); }
.derivation-note strong { color: var(--fg-muted); font-weight: 650; }
.derivation-row {
    display: flex; align-items: center; gap: var(--space-4); padding: var(--space-2) 0;
    border-bottom: 1px solid var(--border);
}
.derivation-row:last-child { border-bottom: none; }
.derivation-formula { flex: none; max-height: 42px; width: auto; }
.derivation-value { flex: 1; min-width: 0; font-family: var(--mono); font-size: 0.86em; color: var(--fg-muted); }
"#;
