//! Ports the epic's §10 command palette *and* §11 Quick Open, merged into
//! one overlay: this app has no file-tree/workspace concept for a
//! separate Quick Open to distinguish itself against, so its real
//! purpose - jump straight to a recent search - is folded in here as a
//! second, filterable group instead of a whole second modal with its own
//! keyboard shortcut and focus-management surface to get right.
//!
//! Global keyboard capture is verified working in this renderer, unlike
//! scroll/drag-drop: blitz-shell's `WindowEvent::KeyboardInput` handler
//! always calls `self.doc.handle_ui_event(UiEvent::KeyDown(..))`
//! regardless of which modifier keys are held (checked its source
//! directly - the Ctrl/Super branch only handles zoom shortcuts and falls
//! through to the normal dispatch for anything else, including
//! Ctrl/Cmd+K), so a real `onkeydown` listener at the app root reliably
//! sees the shortcut. Opened via `Ctrl`/`Cmd`+`K` (checked in `main.rs`'s
//! `App` component) or the palette button in the title bar.

use dioxus::html::Code;
use dioxus::prelude::*;

use crate::state::{AppState, RecentSearch};

#[derive(Clone, Copy, PartialEq)]
pub enum Command {
    RunSearch,
    CancelSearch,
    OpenReport,
    ToggleTheme,
    BrowseSearchFolder,
    BrowseOutputFolder,
    ClearRecentSearches,
    ToggleIndexForFastSearch,
}

impl Command {
    const ALL: [Command; 8] = [
        Command::RunSearch,
        Command::CancelSearch,
        Command::OpenReport,
        Command::ToggleTheme,
        Command::BrowseSearchFolder,
        Command::BrowseOutputFolder,
        Command::ClearRecentSearches,
        Command::ToggleIndexForFastSearch,
    ];

    fn label(self) -> &'static str {
        match self {
            Command::RunSearch => "Run Search",
            Command::CancelSearch => "Cancel Search",
            Command::OpenReport => "Open Report",
            Command::ToggleTheme => "Toggle theme (dark/light)",
            Command::BrowseSearchFolder => "Browse search folder...",
            Command::BrowseOutputFolder => "Browse output folder...",
            Command::ClearRecentSearches => "Clear recent searches",
            Command::ToggleIndexForFastSearch => "Toggle \"Index for fast re-search\"",
        }
    }
}

#[component]
pub fn CommandPalette(mut state: AppState, mut dark: Signal<bool>, mut open: Signal<bool>) -> Element {
    let mut query = use_signal(String::new);
    let needle = query.read().trim().to_lowercase();
    let filtered: Vec<Command> =
        Command::ALL.into_iter().filter(|c| needle.is_empty() || c.label().to_lowercase().contains(&needle)).collect();
    let filtered_recent: Vec<RecentSearch> = state
        .recent_searches
        .read()
        .iter()
        .filter(|r| needle.is_empty() || r.label().to_lowercase().contains(&needle))
        .cloned()
        .collect();

    let mut apply_recent = move |recent: RecentSearch| {
        state.apply_recent_search(&recent);
        open.set(false);
        query.set(String::new());
    };

    let mut execute = move |cmd: Command| {
        match cmd {
            Command::RunSearch => {
                spawn(state.run_search());
            }
            Command::CancelSearch => state.cancel_search(),
            Command::OpenReport => {
                spawn(state.open_report());
            }
            Command::ToggleTheme => dark.set(!dark()),
            Command::BrowseSearchFolder => {
                spawn(state.browse_search_folder());
            }
            Command::BrowseOutputFolder => {
                spawn(state.browse_output_folder());
            }
            Command::ClearRecentSearches => state.recent_searches.write().clear(),
            Command::ToggleIndexForFastSearch => {
                let current = *state.index_for_fast_search.read();
                state.index_for_fast_search.set(!current);
            }
        }
        open.set(false);
        query.set(String::new());
    };

    rsx! {
        div {
            class: "palette-overlay",
            onclick: move |_| open.set(false),
            div {
                class: "palette-card",
                onclick: move |e| e.stop_propagation(),
                input {
                    class: "palette-input",
                    r#type: "text",
                    placeholder: "Type a command...",
                    value: "{query}",
                    oninput: move |e| query.set(e.value()),
                    onkeydown: move |e| {
                        if e.code() == Code::Escape {
                            open.set(false);
                        }
                    },
                }
                div { class: "palette-list",
                    if filtered.is_empty() && filtered_recent.is_empty() {
                        div { class: "palette-empty caption", "No matches" }
                    }
                    if !filtered_recent.is_empty() {
                        div { class: "palette-group-label caption", "Recent searches" }
                        for recent in filtered_recent {
                            div {
                                key: "recent:{recent.label()}",
                                class: "palette-item",
                                onclick: move |_| apply_recent(recent.clone()),
                                "{recent.label()}"
                            }
                        }
                    }
                    if !filtered.is_empty() {
                        div { class: "palette-group-label caption", "Commands" }
                        for cmd in filtered {
                            div {
                                key: "cmd:{cmd.label()}",
                                class: "palette-item",
                                onclick: move |_| execute(cmd),
                                "{cmd.label()}"
                            }
                        }
                    }
                }
            }
        }
    }
}
