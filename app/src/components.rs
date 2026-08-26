//! Ports `src/TextInFilesSearch/Views/MainWindow.xaml`: the settings panel
//! (Required / Matching / Scope and output / Performance and robustness /
//! Fast re-search sections, mirrored via `<details>` - a native HTML
//! equivalent of WinUI's `Expander`) and the progress/results panel.

use dioxus::prelude::*;
use search_core::models::{ExcludeScope, GroupByMode, MatchMode};

use crate::state::{filtered_extensions, selected_extensions_summary, AppState, FileResultView};

/// Stands in for `<select>`, which `blitz-dom` does not yet implement as a
/// real dropdown widget - it renders every `<option>`'s text flattened
/// together with no popup at all (see `docs/epic-ui-performance-and-design.md`'s
/// "Verified platform constraints" table; `blitz-dom-0.2.4/src/form.rs`'s
/// only select-specific code is a bare `// TODO` for form submission).
/// Renders in normal document flow (not `position: absolute/fixed`) when
/// open, rather than a floating popover - deliberately, since this app's
/// *other* now-fixed rendering bug (overlapping list rows) means
/// positioning/stacking behavior on this renderer shouldn't be trusted
/// without its own separate verification first.
#[component]
fn Dropdown(field_label: String, selected_label: String, options: Vec<(&'static str, &'static str)>, on_select: EventHandler<String>) -> Element {
    let mut open = use_signal(|| false);

    rsx! {
        div { class: "field",
            span { "{field_label}" }
            div { class: "select-box",
                button {
                    class: "select-trigger",
                    r#type: "button",
                    onclick: move |_| open.set(!open()),
                    span { "{selected_label}" }
                    span { class: "select-caret", if open() { "▴" } else { "▾" } }
                }
                if open() {
                    div { class: "select-menu",
                        for (value, display) in options.iter().copied() {
                            div {
                                key: "{value}",
                                class: if display == selected_label.as_str() { "select-option selected" } else { "select-option" },
                                onclick: move |_| {
                                    on_select.call(value.to_string());
                                    open.set(false);
                                },
                                "{display}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn SettingsPanel(mut state: AppState) -> Element {
    let can_run = state.can_run();
    let is_running = *state.is_running.read();
    let can_native_search = state.can_native_search();
    let is_native_searching = *state.is_native_searching.read();
    let has_report = state.last_report_path.read().is_some();

    // Memoized (not recomputed inline) so typing anywhere else in this
    // panel - e.g. "Search folder" - doesn't re-filter/re-clone the whole
    // ~50-entry extension catalog on every keystroke. See
    // docs/epic-ui-performance-and-design.md's sluggishness investigation.
    let filtered = use_memo(move || filtered_extensions(&state.extension_catalog.read(), &state.extension_filter_text.read()));
    let summary = use_memo(move || selected_extensions_summary(&state.extension_catalog.read()));
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

            // Multi-root search - additional folders searched alongside
            // "Search folder" above in the same run (`run_search` loops
            // over all of them and merges results into one report). Only
            // shown once there's at least one extra root, or via the
            // always-visible "+ Add another folder" affordance to add the
            // first one.
            if !state.search_paths_extra.read().is_empty() {
                div { class: "field",
                    span { "Additional folders" }
                    div { class: "chip-row",
                        for path in state.search_paths_extra.read().iter().cloned() {
                            button {
                                key: "{path}",
                                class: "chip",
                                title: "Remove {path}",
                                disabled: is_running,
                                onclick: move |_| state.remove_extra_search_path(&path),
                                "{path} \u{2715}"
                            }
                        }
                    }
                }
            }
            div { class: "row",
                button {
                    disabled: is_running,
                    onclick: move |_| { spawn(state.browse_add_search_folder()); },
                    "+ Add another folder"
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

            if !state.recent_searches.read().is_empty() {
                div { class: "field",
                    span { "Recent" }
                    div { class: "chip-row",
                        for recent in state.recent_searches.read().iter().cloned() {
                            button {
                                key: "{recent.label()}",
                                class: "chip",
                                title: "{recent.search_path}",
                                onclick: move |_| state.apply_recent_search(&recent),
                                "{recent.label()}"
                            }
                        }
                    }
                }
            }

            // Named presets - a full settings snapshot saved under a
            // user-given name, distinct from the automatic "Recent" MRU
            // above (which only remembers path+filters and can't be
            // deliberately kept). Click a chip to apply it; its own × button
            // deletes it.
            {
                let mut preset_name = use_signal(String::new);
                rsx! {
                    div { class: "field",
                        span { "Presets" }
                        div { class: "row",
                            input {
                                r#type: "text",
                                placeholder: "Preset name...",
                                value: "{preset_name}",
                                oninput: move |e| preset_name.set(e.value()),
                            }
                            button {
                                disabled: preset_name.read().trim().is_empty(),
                                onclick: move |_| {
                                    let name = preset_name.read().trim().to_string();
                                    if !name.is_empty() {
                                        state.save_current_as_preset(name);
                                        preset_name.set(String::new());
                                    }
                                },
                                "Save current as preset"
                            }
                        }
                        if !state.saved_presets.read().is_empty() {
                            div { class: "chip-row",
                                for preset in state.saved_presets.read().iter().cloned() {
                                    {
                                        let preset_name = preset.name.clone();
                                        rsx! {
                                            div { key: "{preset.name}", class: "chip-removable",
                                                button {
                                                    class: "chip",
                                                    title: "Apply this preset",
                                                    onclick: move |_| state.apply_preset(&preset),
                                                    "{preset_name}"
                                                }
                                                button {
                                                    class: "chip-remove",
                                                    title: "Delete this preset",
                                                    onclick: move |_| state.delete_preset(&preset_name),
                                                    "\u{2715}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            details {
                summary { "Matching" }
                div { class: "expander-body",
                    Dropdown {
                        field_label: "Match mode",
                        selected_label: match_mode_display(*state.match_mode.read()).to_string(),
                        options: vec![("AnyLine", "Any line"), ("AllInFile", "All in file"), ("Proximity", "Proximity")],
                        on_select: move |v: String| state.match_mode.set(parse_match_mode(&v)),
                    }
                    // Proximity lines only means anything in Proximity mode
                    // (AnyLine/AllInFile both ignore it entirely - see
                    // matching.rs's `apply_line_matching`) - shown only once
                    // its prerequisite (match mode) is actually set.
                    if *state.match_mode.read() == MatchMode::Proximity {
                        label { class: "field",
                            span { "Proximity lines" }
                            input {
                                r#type: "number", min: "0",
                                value: "{state.proximity_lines}",
                                oninput: move |e| { if let Ok(v) = e.value().parse::<i32>() { state.proximity_lines.set(v.max(0)); } },
                            }
                        }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.use_regex.read(),
                            oninput: move |e| state.use_regex.set(e.checked()),
                        }
                        span { "Use regex" }
                    }
                    // Catches an invalid regex filter before a run starts
                    // instead of only after (`OrchestratorError::
                    // InvalidFilterRegex`). `use_memo` re-derives only when
                    // filters_text/exclude_filters_text/use_regex actually
                    // change, not on every unrelated keystroke elsewhere in
                    // this panel.
                    {
                        let regex_error = use_memo(move || {
                            let _ = state.filters_text.read();
                            let _ = state.exclude_filters_text.read();
                            let _ = state.use_regex.read();
                            state.regex_validation_error()
                        });
                        rsx! {
                            if let Some(err) = regex_error() {
                                p { class: "field-error", "{err}" }
                            }
                        }
                    }
                    // Whole-word mode requires regex mode to be OFF - `is_hit`
                    // in matching.rs checks `use_regex` first and never even
                    // looks at `whole_word` when it's on, so a checked-but-
                    // regex-mode-active whole-word box would silently do
                    // nothing. Hidden rather than just disabled so there's no
                    // dead control sitting in the panel with no visible effect.
                    if !*state.use_regex.read() {
                        label { class: "field-inline",
                            input {
                                r#type: "checkbox",
                                checked: *state.whole_word.read(),
                                oninput: move |e| state.whole_word.set(e.checked()),
                            }
                            span { "Whole word matching" }
                        }
                    }
                    label { class: "field",
                        span { "Exclude filters (comma-separated)" }
                        input {
                            r#type: "text",
                            value: "{state.exclude_filters_text}",
                            oninput: move |e| state.exclude_filters_text.set(e.value()),
                        }
                    }
                    // Exclude scope (line vs. whole file) only matters once
                    // there's at least one exclude filter to apply it to.
                    if !state.exclude_filters_text.read().trim().is_empty() {
                        Dropdown {
                            field_label: "Exclude scope",
                            selected_label: exclude_scope_str(*state.exclude_scope.read()).to_string(),
                            options: vec![("Line", "Line"), ("File", "File")],
                            on_select: move |v: String| state.exclude_scope.set(parse_exclude_scope(&v)),
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
                        for opt in filtered() {
                            {
                                let ext_key = opt.extension.clone();
                                rsx! {
                                    label { key: "{ext_key}", class: "field-inline",
                                        input {
                                            r#type: "checkbox",
                                            checked: opt.is_selected,
                                            oninput: move |e| {
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
                    p { class: "caption", "{summary()}" }

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
                            oninput: move |e| state.include_hidden.set(e.checked()),
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
                    Dropdown {
                        field_label: "Group by",
                        selected_label: group_by_str(*state.group_by.read()).to_string(),
                        options: vec![("Created", "Created"), ("Modified", "Modified"), ("None", "None")],
                        on_select: move |v: String| state.group_by.set(parse_group_by(&v)),
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.open_report_when_done.read(),
                            oninput: move |e| state.open_report_when_done.set(e.checked()),
                        }
                        span { "Open report when done" }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.export_csv.read(),
                            oninput: move |e| state.export_csv.set(e.checked()),
                        }
                        span { "Export CSV" }
                    }
                    label { class: "field-inline",
                        input {
                            r#type: "checkbox",
                            checked: *state.export_json.read(),
                            oninput: move |e| state.export_json.set(e.checked()),
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
                            oninput: move |e| state.parallel.set(e.checked()),
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
                            oninput: move |e| state.dry_run.set(e.checked()),
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
                            oninput: move |e| state.index_for_fast_search.set(e.checked()),
                        }
                        span { "Index this folder for fast re-search" }
                    }
                    // The query box/Search/Cancel/Build controls are
                    // meaningless until indexing has been turned on - shown
                    // only once that prerequisite is met, with an
                    // explanatory hint in its place otherwise instead of a
                    // box of controls that all just silently no-op.
                    if *state.index_for_fast_search.read() {
                        // Decoupled from Run Search entirely (issue #6
                        // Phase 1) - indexes the whole corpus proactively,
                        // not just this run's hits. Run Search also keeps
                        // the index current automatically after every
                        // completed search when this checkbox is on; this
                        // button is for building/refreshing it without
                        // running a full text-scan search first.
                        div { class: "row",
                            button {
                                disabled: state.search_path.read().trim().is_empty() || *state.is_building_index.read(),
                                onclick: move |_| { spawn(state.build_corpus_index()); },
                                if *state.is_building_index.read() { "Indexing..." } else { "Build/update index" }
                            }
                        }
                        if !state.index_build_status_text.read().is_empty() {
                            p { class: "caption", "{state.index_build_status_text}" }
                        }
                        div { class: "row",
                            label { class: "field",
                                span { "Search the fast index directly" }
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
                        p { class: "caption",
                            "Run Search (left) also uses this index automatically now for non-regex filters, narrowing to candidate files first."
                        }
                    } else {
                        p { class: "caption", "Enable indexing above, then click \"Build/update index\" or run a search - either keeps this folder's index current." }
                    }
                    div { class: "extension-list",
                        for hit in state.native_search_results.read().iter().cloned() {
                            div {
                                key: "{hit.id}",
                                class: "hit-row",
                                onmousedown: {
                                    let full_name = hit.path.clone();
                                    move |e| crate::context_menu::maybe_open_context_menu(state, &e, &full_name)
                                },
                                div { class: "hit-row-top",
                                    div { class: "hit-name", "{hit.filename}" }
                                    div { class: "hit-value", "{hit.score}" }
                                }
                                div { class: "caption", title: "{hit.path}", "{hit.path}" }
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

/// Real substitute for scroll-based list virtualization (epic §5/§31),
/// not just a defensive cap. Scroll-based virtualization is not merely
/// unimplemented but architecturally incoherent to attempt here: (1)
/// scroll position is never forwarded from `blitz-shell` to application
/// code (confirmed - see docs/epic-ui-performance-and-design.md's
/// "Verified platform constraints" table), so there's no way to compute
/// "which slice is visible"; and (2) `WindowEvent::MouseWheel` is
/// consumed entirely inside `blitz-shell` to drive the *native* CSS
/// scroll (`Document::scroll_node_by_has_changed`) - it never reaches a
/// dioxus `onwheel` handler either (checked the same handler - it calls
/// `request_redraw()` directly, never `handle_ui_event`). Even
/// intercepting the raw window-level wheel delta (the same channel
/// pattern `drag_drop.rs` uses) couldn't drive a *virtual* slice without
/// fighting the native scroll already happening underneath it, since
/// there is no way to suppress or query that native scroll from
/// application code either. Pagination sidesteps the whole problem: it
/// bounds the live DOM node count exactly like virtualization would,
/// without needing scroll position at all.
pub(crate) const RESULTS_PAGE_SIZE: usize = 50;

/// Ports the epic's §24 "search statistics" micro-breakdown - top 6
/// extensions among the current hits, most-common first. Pure
/// client-side aggregation over data already in `AppState.results`, no
/// new backend plumbing needed.
fn extension_breakdown(results: &[FileResultView]) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in results {
        let ext = std::path::Path::new(&r.file_name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_else(|| "(no extension)".to_string());
        *counts.entry(ext).or_insert(0) += 1;
    }
    let mut breakdown: Vec<(String, usize)> = counts.into_iter().collect();
    breakdown.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    breakdown.truncate(6);
    breakdown
}

pub(crate) fn copy_to_clipboard(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text);
    }
}

#[component]
pub fn ResultsPanel(state: AppState) -> Element {
    let is_running = *state.is_running.read();
    let has_started = state.status_text.read().as_str() != "Ready.";
    let in_flight = state.in_flight_files.read().clone();
    let results = state.results.read().clone();
    let total_results = results.len();

    rsx! {
        div { class: "results-panel",
            div { class: "progress-block",
                progress { class: "progress-bar", max: "100", value: "{state.progress_percent}" }
                p { "{state.status_text}" }
            }

            if *state.folder_changed_since_search.read() && !is_running {
                div { class: "folder-changed-hint",
                    span { "Files changed in this folder since your last search - run it again to see what's new." }
                    // Re-running IS how re-indexing happens in this app -
                    // indexing is a side effect of a normal search run
                    // (state.rs's `finish_successful_run`), not a separate
                    // pathway - so "Reindex now" is just a convenience
                    // alias for Run Search, shown only when there's
                    // actually an index to keep current.
                    if *state.index_for_fast_search.read() {
                        button {
                            class: "hit-action",
                            disabled: !state.can_run(),
                            onclick: move |_| { spawn(state.run_search()); },
                            "Reindex now"
                        }
                    }
                }
            }

            if is_running && !in_flight.is_empty() {
                div { class: "in-flight-list",
                    for f in in_flight {
                        div { key: "{f.file_name}", class: "hit-row",
                            div { class: "hit-row-top",
                                div { class: "hit-name", "{f.file_name}" }
                                div { class: "hit-value caption", {format!("{:.1}s", f.elapsed_seconds)} }
                            }
                            div { class: "caption", "{f.status_text}" }
                        }
                    }
                }
            }

            if total_results == 0 {
                div { class: "empty-state",
                    if has_started {
                        p { class: "empty-state-title", "No matches" }
                        p { class: "caption", "Nothing matched your current filters. Try a broader term, or remove an exclude filter." }
                    } else {
                        p { class: "empty-state-title", "Search anything" }
                        p { class: "caption", "Choose a search folder and at least one filter on the left, then Run Search." }
                    }
                }
            } else {
                h3 { "{state.results_summary_text}" }
                {
                    let breakdown = extension_breakdown(&results);
                    let max_count = breakdown.first().map(|(_, c)| *c).unwrap_or(1).max(1);
                    rsx! {
                        if breakdown.len() > 1 {
                            div { class: "stat-bars",
                                for (ext, count) in breakdown {
                                    div { key: "{ext}", class: "stat-bar-row",
                                        span { class: "stat-bar-label", "{ext}" }
                                        span { class: "stat-bar-track",
                                            span { class: "stat-bar-fill", style: "width: {100 * count / max_count}%" }
                                        }
                                        span { class: "stat-bar-count", "{count}" }
                                    }
                                }
                            }
                        }
                    }
                }
                {
                    let total_pages = total_results.div_ceil(RESULTS_PAGE_SIZE).max(1);
                    let page = (*state.results_page.read()).min(total_pages - 1);
                    let page_start = page * RESULTS_PAGE_SIZE;
                    let page_results: Vec<FileResultView> = results.iter().skip(page_start).take(RESULTS_PAGE_SIZE).cloned().collect();
                    rsx! {
                        if total_pages > 1 {
                            div { class: "pagination",
                                button {
                                    class: "hit-action",
                                    disabled: page == 0,
                                    onclick: move |_| { let mut s = state; let p = *s.results_page.read(); s.results_page.set(p.saturating_sub(1)); },
                                    "\u{2190} Previous"
                                }
                                span { class: "caption", "Page {page + 1} of {total_pages} ({total_results} total)" }
                                button {
                                    class: "hit-action",
                                    disabled: page + 1 >= total_pages,
                                    onclick: move |_| { let mut s = state; let p = *s.results_page.read(); s.results_page.set(p + 1); },
                                    "Next \u{2192}"
                                }
                            }
                        }
                        div { class: "results-list",
                            for r in page_results {
                        div {
                            key: "{r.full_name}",
                            class: if state.selected_result.read().as_ref().map(|s| &s.full_name) == Some(&r.full_name) { "hit-row selected" } else { "hit-row" },
                            onmousedown: {
                                let full_name = r.full_name.clone();
                                move |e| crate::context_menu::maybe_open_context_menu(state, &e, &full_name)
                            },
                            onclick: {
                                let mut state = state;
                                let r = r.clone();
                                move |_| state.selected_result.set(Some(r.clone()))
                            },
                            div { class: "hit-row-top",
                                div { class: "hit-name", "{r.file_name}" }
                                div { class: "hit-value", "{r.hit_count}" }
                            }
                            div { class: "hit-row-bottom",
                                div { class: "caption", title: "{r.full_name}", "{r.full_name}" }
                                div { class: "hit-actions",
                                    button {
                                        class: "hit-action",
                                        title: "Open this file",
                                        onclick: {
                                            let path = r.full_name.clone();
                                            move |_| { let _ = open::that(&path); }
                                        },
                                        "Open"
                                    }
                                    button {
                                        class: "hit-action",
                                        title: "Copy full path",
                                        onclick: {
                                            let path = r.full_name.clone();
                                            move |_| copy_to_clipboard(&path)
                                        },
                                        "Copy"
                                    }
                                    button {
                                        class: "hit-action",
                                        title: "Open the containing folder",
                                        onclick: {
                                            let path = r.full_name.clone();
                                            move |_| {
                                                if let Some(parent) = std::path::Path::new(&path).parent() {
                                                    let _ = open::that(parent);
                                                }
                                            }
                                        },
                                        "Folder"
                                    }
                                    button {
                                        class: "hit-action",
                                        title: "Export just this file's hits as a text file",
                                        onclick: {
                                            let mut state = state;
                                            let r = r.clone();
                                            move |_| {
                                                let output_folder = state.output_folder.read().trim().to_string();
                                                if output_folder.is_empty() {
                                                    return;
                                                }
                                                let file_stem = std::path::Path::new(&r.file_name)
                                                    .file_stem()
                                                    .map(|s| s.to_string_lossy().into_owned())
                                                    .unwrap_or_else(|| r.file_name.clone());
                                                let out_path = std::path::Path::new(&output_folder)
                                                    .join(format!("{}_hits.txt", crate::state::sanitize_file_name(&file_stem)));
                                                if std::fs::write(&out_path, r.hits_as_text()).is_ok() {
                                                    let _ = open::that(&out_path);
                                                    state.status_text.set(format!("Exported hits to {}", out_path.display()));
                                                } else {
                                                    state.status_text.set("Error exporting hits: could not write file.".to_string());
                                                }
                                            }
                                        },
                                        "Export"
                                    }
                                }
                            }
                        }
                    }
                        }
                    }
                }
            }
        }
    }
}

fn match_mode_display(m: MatchMode) -> &'static str {
    match m {
        MatchMode::AnyLine => "Any line",
        MatchMode::AllInFile => "All in file",
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
