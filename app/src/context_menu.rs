//! Ports the epic's §35 custom context menu. `dioxus-html` defines
//! `oncontextmenu` as a framework-level event type, but this renderer
//! never actually dispatches one - right-click only ever arrives as an
//! ordinary mouse-button event (`blitz-shell-0.2.3/src/window.rs`'s
//! `MouseButton::Right => MouseEventButton::Secondary` conversion is the
//! only right-click-specific code anywhere in the stack; there is no
//! synthesized "contextmenu" DOM event, and no native OS context-menu
//! creation API either - see docs/epic-ui-performance-and-design.md's
//! "Verified platform constraints" table). Rows trigger this via a plain
//! `onmousedown` checking `trigger_button() == Some(MouseButton::Secondary)`
//! instead (see `components.rs`'s `hit-row` handlers).

use dioxus::prelude::*;

use crate::components::copy_to_clipboard;
use crate::state::{AppState, ContextMenuState};

#[component]
pub fn ContextMenu(mut state: AppState) -> Element {
    let Some(menu) = state.context_menu.read().clone() else {
        return rsx! {};
    };

    let position_style = format!("left: {}px; top: {}px;", menu.x, menu.y);
    let file_name = std::path::Path::new(&menu.full_name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| menu.full_name.clone());

    rsx! {
        div {
            class: "ctx-overlay",
            onmousedown: move |_| state.context_menu.set(None),
            div {
                class: "ctx-menu",
                style: "{position_style}",
                onmousedown: move |e| e.stop_propagation(),
                div { class: "ctx-menu-title caption", "{file_name}" }
                button {
                    class: "ctx-item",
                    onclick: {
                        let path = menu.full_name.clone();
                        move |_| {
                            let _ = open::that(&path);
                            state.context_menu.set(None);
                        }
                    },
                    "Open"
                }
                button {
                    class: "ctx-item",
                    onclick: {
                        let path = menu.full_name.clone();
                        move |_| {
                            copy_to_clipboard(&path);
                            state.context_menu.set(None);
                        }
                    },
                    "Copy full path"
                }
                button {
                    class: "ctx-item",
                    onclick: {
                        let name = file_name.clone();
                        move |_| {
                            copy_to_clipboard(&name);
                            state.context_menu.set(None);
                        }
                    },
                    "Copy file name"
                }
                button {
                    class: "ctx-item",
                    onclick: {
                        let path = menu.full_name.clone();
                        move |_| {
                            if let Some(parent) = std::path::Path::new(&path).parent() {
                                let _ = open::that(parent);
                            }
                            state.context_menu.set(None);
                        }
                    },
                    "Open containing folder"
                }
            }
        }
    }
}

/// Call from a row's `onmousedown` - stores the click position + target
/// file so `ContextMenu` (rendered once, at the app root) knows what to
/// show. Ignores anything but a right-click (`MouseButton::Secondary`).
pub fn maybe_open_context_menu(mut state: AppState, evt: &Event<MouseData>, full_name: &str) {
    if evt.trigger_button() != Some(dioxus::html::input_data::MouseButton::Secondary) {
        return;
    }
    let coords = evt.client_coordinates();
    state.context_menu.set(Some(ContextMenuState { full_name: full_name.to_string(), x: coords.x, y: coords.y }));
}
