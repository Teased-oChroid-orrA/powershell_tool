//! Ports `src/TextInFilesSearch/Views/MainWindow.xaml`: the settings panel
//! (Required / Matching / Scope and output / Performance and robustness /
//! Fast re-search sections, mirrored via `<details>` - a native HTML
//! equivalent of WinUI's `Expander`) and the progress/results panel.

use dioxus::prelude::*;
use search_core::models::{ExcludeScope, GroupByMode, MatchMode};

use crate::state::{filtered_extensions, selected_extensions_summary, AppState};

#[component]
pub fn SettingsPanel(mut state: AppState) -> Element {
    let can_run = state.can_run();
    let is_running = *state.is_running.read();
    let can_native_search = state.can_native_search();
    let is_native_searching = *state.is_native_searching.read();
    let has_report = state.last_report_path.read().is_some();

    let filtered = filtered_extensions(&state.extension_catalog.read(), &state.extension_filter_text.read());
    let summary = selected_extensions_summary(&state.extension_catalog.read());
    let can_add_custom = !state.extension_filter_text.read().trim().is_empty();

    rsx! {
        div { class: "settings-panel",
            h3 { "Required" }

            div { class: "row",
                label { class: "field",
                    span { "Search folder" }
                    input {
                        r#type: "text",
                        value: "{state.search_path}",
                        oninput: move |e| state.search_path.set(e.value()),
                    }
                }
                button {
                    disabled: is_running,
                    onclick: move |_| { spawn(state.browse_search_folder()); },
                    "Browse..."
                }
            }

            div { class: "row",
                label { class: "field",
                    span { "Output folder" }
                    input {
                        r#type: "text",
                        value: "{state.output_folder}",
                        oninput: move |e| state.output_folder.set(e.value()),
                    }
                }
                button {
                    disabled: is_running,
                    onclick: move |_| { spawn(state.browse_output_folder()); },
                    "Browse..."
                }
            }

            label { class: "field",
                span { "Filters (comma-separated)" }
                input {
                    r#type: "text",
                    placeholder: "e.g. invoice, overdue",
                    value: "{state.filters_text}",
                    oninput: move |e| state.filters_text.set(e.value()),
                }
            }

            details {
                summary { "Matching" }
                div { class: "expander-body",
                    label { class: "field",
                        span { "Match mode" }
                        select {
                            value: "{match_mode_str(*state.match_mode.read())}",
                            onchange: move |e| state.match_mode.set(parse_match_mode(&e.value())),
                            option { value: "AnyLine", "AnyLine" }
                            option { value: "AllInFile", "AllInFile" }
                            option { value: "Proximity", "Proximity" }
                        }
                    }
                    label { class: "field",
                        span { "Proximity lines" }
                        input {
                            r#type: "number", min: "0",
                            value: "{state.proximity_lines}",
                            oninput: move |e| { if let Ok(v) = e.value().parse::<i32>() { state.proximity_lines.set(v.max(0)); } },
                        }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.use_regex.read(),
                            onchange: move |e| state.use_regex.set(e.checked()),
                        }
                        span { "Use regex" }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.whole_word.read(),
                            onchange: move |e| state.whole_word.set(e.checked()),
                        }
                        span { "Whole word matching" }
                    }
                    label { class: "field",
                        span { "Exclude filters (comma-separated)" }
                        input {
                            r#type: "text",
                            value: "{state.exclude_filters_text}",
                            oninput: move |e| state.exclude_filters_text.set(e.value()),
                        }
                    }
                    label { class: "field",
                        span { "Exclude scope" }
                        select {
                            value: "{exclude_scope_str(*state.exclude_scope.read())}",
                            onchange: move |e| state.exclude_scope.set(parse_exclude_scope(&e.value())),
                            option { value: "Line", "Line" }
                            option { value: "File", "File" }
                        }
                    }
                }
            }

            details {
                summary { "Scope and output" }
                div { class: "expander-body",
                    label { class: "field",
                        span { "File extensions - type to filter, tick to select" }
                        input {
                            r#type: "text",
                            placeholder: "e.g. doc, py, log...",
                            value: "{state.extension_filter_text}",
                            oninput: move |e| state.extension_filter_text.set(e.value()),
                        }
                    }
                    div { class: "extension-list",
                        for opt in filtered {
                            {
                                let ext_key = opt.extension.clone();
                                rsx! {
                                    label { key: "{ext_key}", class: "field-inline",
                                        input {
                                            r#type: "checkbox",
                                            checked: opt.is_selected,
                                            onchange: move |e| {
                                                let checked = e.checked();
                                                if let Some(entry) = state
                                                    .extension_catalog
                                                    .write()
                                                    .iter_mut()
                                                    .find(|o| o.extension == ext_key)
                                                {
                                                    entry.is_selected = checked;
                                                }
                                            },
                                        }
                                        span { "{opt.extension} ({opt.category})" }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "row",
                        button {
                            disabled: !can_add_custom,
                            onclick: move |_| state.add_custom_extension(),
                            "Add as custom extension"
                        }
                        button { onclick: move |_| state.clear_selected_extensions(), "Clear selection" }
                    }
                    p { class: "caption", "{summary}" }

                    label { class: "field",
                        span { "Exclude folders (comma-separated)" }
                        input {
                            r#type: "text",
                            value: "{state.exclude_folders_text}",
                            oninput: move |e| state.exclude_folders_text.set(e.value()),
                        }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.include_hidden.read(),
                            onchange: move |e| state.include_hidden.set(e.checked()),
                        }
                        span { "Include hidden files" }
                    }
                    label { class: "field",
                        span { "Max file size (MB)" }
                        input {
                            r#type: "number", step: "0.01",
                            value: "{state.max_file_size_mb}",
                            oninput: move |e| { if let Ok(v) = e.value().parse::<f64>() { state.max_file_size_mb.set(v.max(0.01)); } },
                        }
                    }
                    label { class: "field",
                        span { "Group by" }
                        select {
                            value: "{group_by_str(*state.group_by.read())}",
                            onchange: move |e| state.group_by.set(parse_group_by(&e.value())),
                            option { value: "Created", "Created" }
                            option { value: "Modified", "Modified" }
                            option { value: "None", "None" }
                        }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.open_report_when_done.read(),
                            onchange: move |e| state.open_report_when_done.set(e.checked()),
                        }
                        span { "Open report when done" }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.export_csv.read(),
                            onchange: move |e| state.export_csv.set(e.checked()),
                        }
                        span { "Export CSV" }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.export_json.read(),
                            onchange: move |e| state.export_json.set(e.checked()),
                        }
                        span { "Export JSON" }
                    }
                }
            }

            details {
                summary { "Performance and robustness" }
                div { class: "expander-body",
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.parallel.read(),
                            onchange: move |e| state.parallel.set(e.checked()),
                        }
                        span { "Parallel processing" }
                    }
                    label { class: "field",
                        span { "Parallel throttle limit" }
                        input {
                            r#type: "number", min: "1",
                            value: "{state.throttle_limit}",
                            oninput: move |e| { if let Ok(v) = e.value().parse::<i32>() { state.throttle_limit.set(v.max(1)); } },
                        }
                    }
                    label { class: "field",
                        span { "Cache file (blank = disabled)" }
                        input {
                            r#type: "text",
                            value: "{state.cache_file_path}",
                            oninput: move |e| state.cache_file_path.set(e.value()),
                        }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.dry_run.read(),
                            onchange: move |e| state.dry_run.set(e.checked()),
                        }
                        span { "Dry run (list files only)" }
                    }
                    label { class: "field",
                        span { "PDF extraction timeout (seconds)" }
                        input {
                            r#type: "number", min: "1",
                            value: "{state.pdf_timeout_seconds}",
                            oninput: move |e| { if let Ok(v) = e.value().parse::<i32>() { state.pdf_timeout_seconds.set(v.max(1)); } },
                        }
                    }
                    label { class: "field",
                        span { "Per-file read timeout (seconds)" }
                        input {
                            r#type: "number", min: "1",
                            value: "{state.file_timeout_seconds}",
                            oninput: move |e| { if let Ok(v) = e.value().parse::<i32>() { state.file_timeout_seconds.set(v.max(1)); } },
                        }
                    }
                    label { class: "field",
                        span { "Max retries (locked files)" }
                        input {
                            r#type: "number", min: "0",
                            value: "{state.max_retries}",
                            oninput: move |e| { if let Ok(v) = e.value().parse::<i32>() { state.max_retries.set(v.max(0)); } },
                        }
                    }
                }
            }

            details {
                summary { "Fast re-search (experimental)" }
                div { class: "expander-body",
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.index_for_fast_search.read(),
                            onchange: move |e| state.index_for_fast_search.set(e.checked()),
                        }
                        span { "Index results for fast re-search" }
                    }
                    div { class: "row",
                        label { class: "field",
                            span { "Search the fast index" }
                            input {
                                r#type: "text",
                                placeholder: "e.g. torque OR extension:.pdf",
                                value: "{state.native_search_query}",
                                oninput: move |e| state.native_search_query.set(e.value()),
                            }
                        }
                        button {
                            disabled: !can_native_search,
                            onclick: move |_| { spawn(state.run_native_search()); },
                            "Search"
                        }
                        button {
                            disabled: !is_native_searching,
                            onclick: move |_| state.cancel_native_search(),
                            "Cancel"
                        }
                    }
                    p { class: "caption", "{state.native_search_status_text}" }
                    div { class: "extension-list",
                        for hit in state.native_search_results.read().iter().cloned() {
                            div { key: "{hit.id}", class: "hit-row",
                                div {
                                    div { class: "hit-name", "{hit.filename}" }
                                    div { class: "caption", "{hit.path}" }
                                }
                                div { "{hit.score}" }
                            }
                        }
                    }
                }
            }

            div { class: "row action-row",
                button {
                    class: "primary",
                    disabled: !can_run,
                    onclick: move |_| { spawn(state.run_search()); },
                    "Run Search"
                }
                button { disabled: !is_running, onclick: move |_| state.cancel_search(), "Cancel" }
                button { disabled: !has_report, onclick: move |_| state.open_report(), "Open Report" }
            }
        }
    }
}

#[component]
pub fn ResultsPanel(state: AppState) -> Element {
    rsx! {
        div { class: "results-panel",
            div { class: "progress-block",
                progress { class: "progress-bar", max: "100", value: "{state.progress_percent}" }
                p { "{state.status_text}" }
            }

            div { class: "in-flight-list",
                for f in state.in_flight_files.read().iter().cloned() {
                    div { key: "{f.file_name}", class: "hit-row",
                        div {
                            div { class: "hit-name", "{f.file_name}" }
                            div { class: "caption", "{f.status_text}" }
                        }
                        div { class: "caption", {format!("{:.1}s", f.elapsed_seconds)} }
                    }
                }
            }

            h3 { "{state.results_summary_text}" }

            div { class: "results-list",
                for r in state.results.read().iter().cloned() {
                    div { key: "{r.full_name}", class: "hit-row",
                        div {
                            div { class: "hit-name", "{r.file_name}" }
                            div { class: "caption", "{r.full_name}" }
                        }
                        div { "{r.hit_count}" }
                    }
                }
            }
        }
    }
}

fn match_mode_str(m: MatchMode) -> &'static str {
    match m {
        MatchMode::AnyLine => "AnyLine",
        MatchMode::AllInFile => "AllInFile",
        MatchMode::Proximity => "Proximity",
    }
}

fn parse_match_mode(s: &str) -> MatchMode {
    match s {
        "AllInFile" => MatchMode::AllInFile,
        "Proximity" => MatchMode::Proximity,
        _ => MatchMode::AnyLine,
    }
}

fn exclude_scope_str(s: ExcludeScope) -> &'static str {
    match s {
        ExcludeScope::Line => "Line",
        ExcludeScope::File => "File",
    }
}

fn parse_exclude_scope(s: &str) -> ExcludeScope {
    match s {
        "File" => ExcludeScope::File,
        _ => ExcludeScope::Line,
    }
}

fn group_by_str(g: GroupByMode) -> &'static str {
    match g {
        GroupByMode::Created => "Created",
        GroupByMode::Modified => "Modified",
        GroupByMode::None => "None",
    }
}

fn parse_group_by(s: &str) -> GroupByMode {
    match s {
        "Modified" => GroupByMode::Modified,
        "None" => GroupByMode::None,
        _ => GroupByMode::Created,
    }
}
