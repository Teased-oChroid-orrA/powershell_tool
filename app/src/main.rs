mod command_palette;
mod components;
mod context_menu;
mod drag_drop;
mod fs_watch;
mod persistence;
mod preview;
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

    let window_attributes = dioxus::native::WindowAttributes::default().with_title("GS Engineering - Text Search");
    launch::run(App, window_attributes);
}

/// Hand-rolled replacement for `dioxus_native::launch_cfg` - see
/// `drag_drop.rs` for why. This mirrors that function's own body (the
/// public API surface it needs is entirely `pub` on `dioxus-native`/
/// `dioxus-native-dom`/`blitz-shell`/`blitz-html`, verified by reading
/// `dioxus-native-0.7.10/src/lib.rs` directly) with two narrowed-down
/// simplifications specific to this app, not general-purpose omissions:
/// `net_provider`/`navigation_provider` are both `None` because this app
/// has no `<img src="http://...">`/remote-font/`<a href>` usage at all -
/// everything is inline `style{}` and local file I/O - so the (crate-
/// private, unreachable from outside `dioxus-native`) providers that
/// would normally supply those aren't needed here.
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
        let doc = DioxusDocument::new(
            vdom,
            DocumentConfig { net_provider: None, html_parser_provider, navigation_provider: None, ..Default::default() },
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
                    Some(()) => {
                        if !*state.is_running.read() {
                            state.folder_changed_since_search.set(true);
                        }
                    }
                    None => break,
                }
            }
        });
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
            // Global Ctrl/Cmd+K (command palette) and Escape (close it) -
            // verified this renderer dispatches keydown regardless of
            // modifier state or focus target (see command_palette.rs's
            // doc comment for the exact evidence). `tabindex: "-1"` makes
            // this div a valid keyboard-event target even though it's
            // never meant to be tab-stopped into.
            onkeydown: move |e| {
                let mods = e.modifiers();
                if e.code() == Code::KeyK && (mods.contains(Modifiers::CONTROL) || mods.contains(Modifiers::META)) {
                    command_palette_open.set(!command_palette_open());
                } else if e.code() == Code::Escape && command_palette_open() {
                    command_palette_open.set(false);
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
            div { class: "title-bar",
                div { class: "title-bar-brand",
                    span { class: "brand-mark", "GS" }
                    h1 { "Text Search" }
                }
                div { class: "title-bar-actions",
                    button {
                        class: "palette-trigger",
                        title: "Command palette (Ctrl/Cmd+K)",
                        onclick: move |_| command_palette_open.set(true),
                        "\u{2318}K"
                    }
                    button {
                        class: "theme-toggle",
                        title: if dark() { "Switch to light theme" } else { "Switch to dark theme" },
                        onclick: move |_| dark.set(!dark()),
                        if dark() { "\u{2600}" } else { "\u{263D}" }
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

.app-shell[data-theme="dark"] {
    --fg: #eef0f4;
    --fg-muted: #8d96a3;
    --fg-subtle: #626a76;
    --bg: #14161b;
    --bg-sunken: #0e1013;
    --panel-bg: #1b1e25;
    --panel-hover: #262b34;
    --border: #2b303a;
    --border-strong: #3c424e;
    --accent: #4fa8e8;
    --accent-strong: #7cc0f5;
    --accent-fg: #06202e;
    --active: #e0a05f;
    --danger: #e2657a;
    --good: #52c98a;
    --shadow-sm: 0 1px 2px rgba(0, 0, 0, 0.3);
    --shadow-md: 0 6px 20px rgba(0, 0, 0, 0.35);
}
.app-shell[data-theme="light"] {
    --fg: #171a1f;
    --fg-muted: #5b6472;
    --fg-subtle: #8992a1;
    --bg: #f2f4f7;
    --bg-sunken: #e7eaee;
    --panel-bg: #ffffff;
    --panel-hover: #eef1f5;
    --border: #d9dfe6;
    --border-strong: #c2cad4;
    --accent: #1c6fae;
    --accent-strong: #0b5fa5;
    --accent-fg: #ffffff;
    --active: #a8632c;
    --danger: #c2394f;
    --good: #29875a;
    --shadow-sm: 0 1px 2px rgba(20, 25, 35, 0.08);
    --shadow-md: 0 6px 20px rgba(20, 25, 35, 0.12);
}

:root {
    --space-1: 4px; --space-2: 8px; --space-3: 12px; --space-4: 16px; --space-5: 20px; --space-6: 28px;
    --radius-sm: 5px; --radius-md: 8px; --radius-pill: 999px;
    --ease: cubic-bezier(0.4, 0, 0.2, 1);
    --mono: ui-monospace, "SF Mono", Consolas, monospace;
}

* { box-sizing: border-box; min-width: 0; }
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
    /* Positioning context for .drop-overlay - see that rule's comment. */
    position: relative;
}
.title-bar {
    flex: none;
    display: flex; align-items: center; justify-content: space-between;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
    background: var(--panel-bg);
    box-shadow: var(--shadow-sm);
}
.title-bar-brand { display: flex; align-items: center; gap: var(--space-3); min-width: 0; }
.brand-mark {
    flex: none;
    display: flex; align-items: center; justify-content: center;
    width: 26px; height: 26px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--accent) 22%, var(--panel-bg));
    color: var(--accent-strong);
    font-size: 11px; font-weight: 700; letter-spacing: 0.02em;
}
.title-bar h1 { margin: 0; font-size: 1.02em; font-weight: 650; letter-spacing: -0.005em; }
.theme-toggle {
    flex: none; width: 30px; height: 30px; padding: 0;
    display: flex; align-items: center; justify-content: center;
    border-radius: 50%; font-size: 13px;
    background: var(--panel-bg); border: 1px solid var(--border); color: var(--fg-muted);
}
.theme-toggle:hover { background: var(--panel-hover); color: var(--fg); border-color: var(--border-strong); }

.main-grid { display: flex; flex: 1; min-height: 0; gap: var(--space-2); padding: var(--space-4); overflow: hidden; }
.settings-column { flex: none; overflow-y: auto; overflow-x: hidden; padding-right: var(--space-1); }
.results-column { flex: 1; min-width: 0; overflow-y: auto; overflow-x: hidden; }
.preview-column { flex: none; overflow-y: auto; overflow-x: hidden; border-left: 1px solid var(--border); padding-left: var(--space-4); }
.resize-handle { flex: none; width: 6px; margin: 0 calc(-1 * var(--space-1)); cursor: ew-resize; position: relative; }
.resize-handle::after {
    content: ""; position: absolute; left: 2px; top: 0; bottom: 0; width: 2px;
    background: var(--border); border-radius: var(--radius-pill);
}
.resize-handle:hover::after { background: var(--accent); }
.settings-panel, .results-panel { display: flex; flex-direction: column; gap: var(--space-3); min-width: 0; }

h3 {
    margin: var(--space-2) 0 0;
    font-size: 0.76em; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--fg-muted);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
details {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 0 var(--space-3);
    background: var(--panel-bg);
    box-shadow: var(--shadow-sm);
    transition: border-color 0.15s var(--ease);
}
details[open] { border-color: var(--border-strong); }
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
    flex: none; width: 16px; height: 16px; margin: 0;
    accent-color: var(--accent);
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
    display: block; margin-top: var(--space-1); border: 1px solid var(--border);
    border-radius: var(--radius-sm); background: var(--panel-bg); overflow: hidden; box-shadow: var(--shadow-md);
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
    padding: var(--space-6); border-radius: var(--radius-md);
    background: var(--panel-bg); border: 2px dashed var(--accent);
    box-shadow: var(--shadow-md);
    text-align: center; max-width: 320px;
}
.drop-overlay-title { margin: 0; font-size: 1.1em; font-weight: 700; color: var(--fg); }

.title-bar-actions { flex: none; display: flex; align-items: center; gap: var(--space-2); }
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
    background: var(--panel-bg); border: 1px solid var(--border-strong);
    border-radius: var(--radius-md); box-shadow: var(--shadow-md);
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
    background: var(--panel-bg); border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm); box-shadow: var(--shadow-md);
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
    display: block; padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm); font-size: 0.85em; font-weight: 500;
    background: color-mix(in srgb, var(--active) 18%, var(--panel-bg));
    color: var(--active);
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
"#;
