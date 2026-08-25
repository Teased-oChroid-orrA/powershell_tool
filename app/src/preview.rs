//! Ports the epic's §14 preview pane. Shows the *matches* for the
//! selected result (line number, before/match/after context, the actual
//! matched text highlighted) rather than full source-file syntax
//! highlighting - that data already exists in full
//! (`search_core::models::LineHit`, carried through into
//! `FileResultView` for exactly this) from the search that already ran,
//! so no extra file re-read is needed to populate this. Full
//! multi-language syntax highlighting (as opposed to match highlighting)
//! is a substantially larger, separate feature - a real lexer/highlighter
//! library integration - not attempted here; scoped out deliberately, not
//! silently dropped (see docs/epic-ui-performance-and-design.md).

use dioxus::prelude::*;

use crate::components::copy_to_clipboard;
use crate::state::AppState;

#[component]
pub fn PreviewPane(state: AppState) -> Element {
    let Some(selected) = state.selected_result.read().clone() else {
        return rsx! {
            div { class: "preview-pane preview-pane-empty",
                p { class: "caption", "Select a result on the left to preview its matches here." }
            }
        };
    };

    rsx! {
        div { class: "preview-pane",
            div { class: "preview-header",
                div { class: "preview-title", title: "{selected.full_name}", "{selected.file_name}" }
                div { class: "hit-actions preview-actions-visible",
                    button {
                        class: "hit-action",
                        title: "Open this file",
                        onclick: {
                            let path = selected.full_name.clone();
                            move |_| { let _ = open::that(&path); }
                        },
                        "Open"
                    }
                    button {
                        class: "hit-action",
                        title: "Copy full path",
                        onclick: {
                            let path = selected.full_name.clone();
                            move |_| copy_to_clipboard(&path)
                        },
                        "Copy"
                    }
                }
            }
            p { class: "caption preview-path", "{selected.full_name}" }
            if selected.low_confidence_pdf {
                p { class: "folder-changed-hint",
                    "This PDF's extracted text looks unreliable (often a sign of embedded/subsetted fonts) - open the file directly if you expected more matches."
                }
            }
            {
                // Before/after context lines previously rendered plain,
                // even in Proximity mode where a *different* filter
                // matching on a nearby line is exactly what makes that
                // context relevant - only the single match line ever got
                // `<mark>` spans. Highlighted here against the full
                // current filter list (not just `hit.matched_filters`,
                // which is only ever this one line's own matches) so a
                // filter that hit on the line just above/below shows up
                // too. Best-effort against the *current* Filters field
                // rather than a filter list captured at search time (this
                // view has no such snapshot to read) - if filters were
                // edited after the run without re-searching, highlighting
                // may drift from what actually matched; the match line
                // itself (`hit.matched_filters`) stays exact regardless.
                let context_filters = crate::state::parse_list(&state.filters_text.read());
                rsx! {
                    div { class: "preview-matches",
                        for (i, hit) in selected.hits.iter().enumerate() {
                            div { key: "{i}", class: "preview-match",
                                div { class: "preview-lineno caption", "Line {hit.line_number}" }
                                if let Some(before) = &hit.before {
                                    pre { class: "preview-context", {highlighted_line(before, &context_filters)} }
                                }
                                pre { class: "preview-context preview-matchline", {highlighted_line(&hit.match_line, &hit.matched_filters)} }
                                if let Some(after) = &hit.after {
                                    pre { class: "preview-context", {highlighted_line(after, &context_filters)} }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Wraps each occurrence of any matched filter in `<mark>`. A plain
/// case-insensitive substring search over `matched_filters` (the literal
/// filter text search-core already determined matched this line) rather
/// than re-running whole-word/regex matching here - close enough for a
/// preview highlight, not required to be pixel-identical to the HTML
/// report's own highlighter (`search_core::report`'s highlight logic,
/// which *is* whole-word/regex-precise, stays the source of truth for the
/// saved report).
fn highlighted_line(line: &str, matched_filters: &[String]) -> Element {
    let lower_line = line.to_lowercase();
    let mut ranges: Vec<(usize, usize)> = Vec::new();

    for f in matched_filters {
        if f.is_empty() {
            continue;
        }
        let lower_f = f.to_lowercase();
        let mut search_from = 0usize;
        while search_from <= lower_line.len() {
            let Some(rel_pos) = lower_line[search_from..].find(&lower_f) else { break };
            let start = search_from + rel_pos;
            let end = start + lower_f.len();
            ranges.push((start, end));
            search_from = end.max(start + 1);
        }
    }

    ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in ranges {
        if let Some(last) = merged.last_mut() {
            if r.0 <= last.1 {
                if r.1 > last.1 {
                    last.1 = r.1;
                }
                continue;
            }
        }
        merged.push(r);
    }

    let mut pos = 0usize;
    let mut pieces: Vec<Element> = Vec::new();
    for (start, end) in merged {
        if start > pos && line.is_char_boundary(pos) && line.is_char_boundary(start) {
            let plain = line[pos..start].to_string();
            pieces.push(rsx! {
                "{plain}"
            });
        }
        if line.is_char_boundary(start) && line.is_char_boundary(end) {
            let marked = line[start..end].to_string();
            pieces.push(rsx! {
                mark { "{marked}" }
            });
            pos = end;
        }
    }
    if pos < line.len() && line.is_char_boundary(pos) {
        let rest = line[pos..].to_string();
        pieces.push(rsx! {
            "{rest}"
        });
    }

    rsx! {
        for piece in pieces {
            {piece}
        }
    }
}
