mod components;
mod state;

use dioxus::prelude::*;

use components::{ResultsPanel, SettingsPanel};
use state::AppState;

fn main() {
    let window_attributes = dioxus::native::WindowAttributes::default().with_title("GS Engineering - Text Search");
    let config = dioxus::native::Config::default().with_window_attributes(window_attributes);
    dioxus::native::launch_cfg(App, vec![], vec![Box::new(config)]);
}

#[component]
fn App() -> Element {
    let state = AppState::new();

    rsx! {
        style { {APP_CSS} }
        div { class: "app-shell",
            div { class: "title-bar", h1 { "GS Engineering - Text Search" } }
            div { class: "main-grid",
                div { class: "settings-column", SettingsPanel { state } }
                div { class: "results-column", ResultsPanel { state } }
            }
        }
    }
}

const APP_CSS: &str = r#"
:root {
    color-scheme: light dark;
    --fg: #1a1a1a;
    --bg: #fafafa;
    --panel-bg: #ffffff;
    --border: #ddd;
    --muted: #666;
    --accent: #0b5fa5;
}
@media (prefers-color-scheme: dark) {
    :root {
        --fg: #e6e6e6;
        --bg: #1b1d21;
        --panel-bg: #20232a;
        --border: #3a3f4a;
        --muted: #9aa4b2;
        --accent: #6cb2f2;
    }
}
* { box-sizing: border-box; }
body { margin: 0; font-family: -apple-system, "Segoe UI", Arial, sans-serif; background: var(--bg); color: var(--fg); }
.app-shell { display: flex; flex-direction: column; height: 100vh; }
.title-bar { padding: 12px 16px; border-bottom: 1px solid var(--border); background: var(--panel-bg); }
.title-bar h1 { margin: 0; font-size: 1.1em; }
.main-grid { display: flex; flex: 1; min-height: 0; gap: 16px; padding: 16px; }
.settings-column { width: 400px; overflow-y: auto; }
.results-column { flex: 1; overflow-y: auto; }
.settings-panel, .results-panel { display: flex; flex-direction: column; gap: 12px; }
h3 { margin: 0.4em 0 0.2em 0; font-size: 1em; }
details { border: 1px solid var(--border); border-radius: 6px; padding: 6px 10px; background: var(--panel-bg); }
summary { cursor: pointer; font-weight: 600; padding: 4px 0; }
.expander-body { display: flex; flex-direction: column; gap: 10px; padding: 8px 0 4px 0; }
.field { display: flex; flex-direction: column; gap: 4px; font-size: 0.9em; flex: 1; }
.field span { font-weight: 600; font-size: 0.85em; }
.field-inline { display: flex; align-items: center; gap: 8px; font-size: 0.9em; }
input[type="text"], input[type="number"], select {
    padding: 6px 8px; border: 1px solid var(--border); border-radius: 4px;
    background: var(--panel-bg); color: var(--fg); font-size: 0.9em;
}
.row { display: flex; gap: 8px; align-items: flex-end; }
.action-row { margin-top: 8px; }
button {
    padding: 7px 14px; border: 1px solid var(--border); border-radius: 4px;
    background: var(--panel-bg); color: var(--fg); cursor: pointer; font-size: 0.9em;
}
button:disabled { opacity: 0.5; cursor: default; }
button.primary { background: var(--accent); color: #fff; border-color: var(--accent); font-weight: 600; }
.caption { color: var(--muted); font-size: 0.8em; }
.extension-list { max-height: 200px; overflow-y: auto; border: 1px solid var(--border); border-radius: 4px; padding: 6px; display: flex; flex-direction: column; gap: 4px; }
.progress-block { display: flex; flex-direction: column; gap: 6px; }
.progress-bar { width: 100%; height: 10px; }
.in-flight-list, .results-list { display: flex; flex-direction: column; gap: 2px; max-height: 220px; overflow-y: auto; }
.hit-row { display: flex; justify-content: space-between; align-items: center; padding: 6px 8px; border-bottom: 1px solid var(--border); gap: 8px; }
.hit-name { font-weight: 600; }
"#;
