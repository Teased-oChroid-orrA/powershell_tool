//! What the Search workbench shows: the page (search bar, progress, results table, matches), the tree, the
//! inspector's option cards, the ribbon, the menus and the value cards. Pure reads of the session.
use super::{
    act, file_name, SearchAct, SearchSession, SortKey, INPUT_EXCLUDE, INPUT_FILTER, INPUT_FIND,
    INPUT_FOLDER, INPUT_OUTPUT, SETTING_EXCLUDE, SETTING_EXCLUDE_SCOPE, SETTING_FILTERS,
    SETTING_OUTPUT, SETTING_PATH,
};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        session::Tone,
    },
    menu::{Entry, Item, Menu},
    ui::{
        CardRow, DocColumn, DocDetail, DocHelp, DocNote, DocSignal, DocInput, DocMatchNavigation, DocModal, DocNotice,
        DocInsight, DocPanel, DocProgress, DocRibbon, DocRow, DocSidebar, DocSidebarRecent, DocTable, DocText, DockItem,
        Document, HelpRow, HelpSection, RibbonTab, TreeNode, ValueCard,
    },
};
use crate::toolboxes::search::{indexing::format_eta, model, preview, run_view, settings_view};
use search_core::models::{ExcludeScope, MatchMode};

pub(super) struct RowsCache {
    started: Option<std::time::Instant>,
    result_count: usize,
    sort: (SortKey, bool),
    filter: String,
    selected: Option<usize>,
    root: String,
    #[cfg(feature = "neural-search")]
    relevance_key: String,
    rows: std::sync::Arc<Vec<DocRow>>,
}

impl RowsCache {
    fn current(&self, s: &SearchSession) -> bool {
        self.started == s.st.run.started
            && self.result_count == s.st.run.results.len()
            && self.sort == s.sort
            && self.filter == s.filter
            && self.selected == s.selected
            && self.root == s.st.config.search_path
            && {
                #[cfg(feature = "neural-search")]
                {
                    self.relevance_key == s.relevance_key()
                }
                #[cfg(not(feature = "neural-search"))]
                {
                    true
                }
            }
    }

    fn can_extend(&self, s: &SearchSession) -> bool {
        self.started == s.st.run.started
            && self.result_count < s.st.run.results.len()
            && self.sort == (SortKey::Found, false)
            && s.sort == (SortKey::Found, false)
            && self.filter == s.filter
            && self.selected == s.selected
            && self.root == s.st.config.search_path
    }
}

fn e(label: &str, shortcut: &str, a: SearchAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
fn when(entry: Entry, ok: bool, why: &str) -> Entry {
    entry.when(if ok { Ok(()) } else { Err(why.to_string()) })
}
/// "1,204".
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", group(n), if n == 1 { one } else { many })
}
/// The window font lacks a few symbols the terminal text uses.
fn plain(s: &str) -> String {
    s.replace('⚠', "!").replace('…', "...").replace('→', "->")
}

#[cfg(feature = "semantic-search")]
pub(super) fn semantic_citation(hit: &search_core::semantic_index::SemanticHit) -> String {
    let lines = if hit.first_line == hit.last_line {
        format!("line {}", hit.first_line)
    } else {
        format!("lines {}–{}", hit.first_line, hit.last_line)
    };
    let range = hit.page.map_or(lines.clone(), |page| {
        format!("page {} · {lines}", page.get())
    });
    format!("{} · {range}", file_name(&hit.path))
}

// ---------------------------------------------------------------------- availability

fn run_rule(s: &SearchSession) -> Result<(), String> {
    let c = &s.st.config;
    if s.st.run.is_running {
        Err("a search is running (Esc stops it)".into())
    } else if c.filters_text.trim().is_empty() {
        Err("type what to find first".into())
    } else if super::find::classify(&c.filters_text, s.find_as) == super::find::Intent::Question {
        // A question about the app needs no folder; one about files asks for it when it is answered.
        Ok(())
    } else if c.search_path.trim().is_empty() {
        Err("choose a folder to search first".into())
    } else if let Some(e) = model::regex_validation_error(c) {
        Err(format!("the pattern is not valid: {e}"))
    } else {
        Ok(())
    }
}
fn selected_rule(s: &SearchSession) -> Result<(), String> {
    if s.current_result().is_some() {
        Ok(())
    } else {
        Err("select a result first".into())
    }
}

fn run_entry(s: &SearchSession) -> Entry {
    e("Search", "Ctrl+Enter", SearchAct::Run).when(run_rule(s))
}
fn stop_entry(s: &SearchSession) -> Entry {
    when(
        e("Stop", "Esc", SearchAct::Cancel),
        s.st.run.is_running,
        "no search is running",
    )
}
fn result_entries(s: &SearchSession) -> Vec<Entry> {
    let ok = selected_rule(s);
    vec![
        e("Open", "Enter", SearchAct::OpenSelected).when(ok.clone()),
        e("Show in folder", "", SearchAct::Reveal).when(ok.clone()),
        e("Copy path", "Ctrl+C", SearchAct::CopyPath).when(ok.clone()),
        e("Export matches", "Ctrl+E", SearchAct::ExportHits).when(ok),
    ]
}
fn report_entry(s: &SearchSession) -> Entry {
    when(
        e("Open report", "", SearchAct::OpenReport),
        s.st.run.last_report_path.is_some(),
        "no report yet: one is written when a search finishes with HTML export on",
    )
}
fn match_mode_entries(s: &SearchSession) -> Vec<Entry> {
    let m = s.st.config.match_mode;
    vec![
        e("Any line", "", SearchAct::Match(MatchMode::AnyLine)).checked(m == MatchMode::AnyLine),
        e("All in file", "", SearchAct::Match(MatchMode::AllInFile))
            .checked(m == MatchMode::AllInFile),
        Entry::new(
            format!("Within {} lines", s.st.config.proximity_lines),
            "",
            act(SearchAct::Match(MatchMode::Proximity)),
        )
        .checked(m == MatchMode::Proximity),
    ]
}
fn switch(s: &SearchSession, label: &str, idx: usize) -> Entry {
    Entry::new(label, "", act(SearchAct::Step(idx)))
        .checked(settings_view::setting_on(&s.st.config, idx))
}

// ---------------------------------------------------------------------- page

fn recent_filter_suggestions(s: &SearchSession) -> Vec<Entry> {
    let typed = s.st.config.filters_text.trim();
    if typed.is_empty() {
        return Vec::new();
    }
    let needle = typed.to_lowercase();
    s.st.recent_searches
        .iter()
        .enumerate()
        .filter(|(_, recent)| {
            recent.search_path == s.st.config.search_path
                && recent
                    .filters_text
                    .trim()
                    .to_lowercase()
                    .starts_with(&needle)
                && recent.filters_text.trim() != typed
        })
        .take(3)
        .map(|(i, recent)| {
            let label: String = recent.filters_text.chars().take(32).collect();
            let label = if recent.filters_text.chars().count() > 32 {
                format!("{label}…")
            } else {
                label
            };
            e(&label, "", SearchAct::ApplyRecentFilters(i))
        })
        .collect()
}

fn recent_folder_suggestions(s: &SearchSession) -> Vec<Entry> {
    let typed = s.st.config.search_path.trim();
    if typed.is_empty() {
        return Vec::new();
    }
    let needle = typed.to_lowercase();
    let mut seen = std::collections::HashSet::new();
    s.st.recent_searches
        .iter()
        .enumerate()
        .filter_map(|(i, recent)| {
            let path = recent.search_path.trim();
            let folder = file_name(path);
            if path.eq_ignore_ascii_case(typed)
                || !(path.to_lowercase().starts_with(&needle)
                    || folder.to_lowercase().starts_with(&needle))
                || !seen.insert(path.to_lowercase())
            {
                return None;
            }
            let label = if path.chars().count() > 36 {
                let tail: String = path
                    .chars()
                    .rev()
                    .take(35)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                format!("…{tail}")
            } else {
                path.to_string()
            };
            Some((i, label))
        })
        .take(3)
        .map(|(i, label)| e(&label, "", SearchAct::ApplyRecentFolder(i)))
        .collect()
}

#[derive(Debug, Clone)]
struct QueryExpansion {
    group_index: usize,
    source: &'static str,
    term: &'static str,
    relation: search_core::synonyms::Relation,
}

fn synonym_suggestions(s: &SearchSession) -> Vec<QueryExpansion> {
    model::parse_filter_groups(&s.st.config.filters_text)
        .into_iter()
        .enumerate()
        .filter(|(_, group)| {
            matches!(
                group
                    .mode
                    .unwrap_or(model::GroupMode::Match(s.st.config.match_mode, None)),
                model::GroupMode::Match(MatchMode::AnyLine, _)
            )
        })
        .flat_map(|(group_index, group)| {
            group.filters.into_iter().flat_map(move |filter| {
                search_core::synonyms::suggestions(&filter)
                    .into_iter()
                    .map(move |suggestion| QueryExpansion {
                        group_index,
                        source: suggestion.source,
                        term: suggestion.term,
                        relation: suggestion.relation,
                    })
            })
        })
        .fold(Vec::new(), |mut out: Vec<QueryExpansion>, suggestion| {
            if !out.iter().any(|prior| {
                prior.group_index == suggestion.group_index
                    && prior.term.eq_ignore_ascii_case(suggestion.term)
            }) {
                out.push(suggestion);
            }
            out
        })
}

pub(super) fn apply_synonym(s: &mut SearchSession, index: usize) -> bool {
    let Some(expansion) = synonym_suggestions(s).get(index).cloned() else {
        return false;
    };
    let Some(query) = model::append_filter_to_group(
        &s.st.config.filters_text,
        expansion.group_index,
        expansion.term,
    ) else {
        return false;
    };
    s.st.config.filters_text = query;
    s.status = format!(
        "Added {} as an any-line alternative for {}; review the query, then search.",
        expansion.term, expansion.source
    );
    true
}

struct QuerySuggestion {
    term: String,
    label: String,
    action: SearchAct,
}

fn query_suggestions(s: &SearchSession) -> Vec<QuerySuggestion> {
    let mut suggestions = synonym_suggestions(s)
        .into_iter()
        .enumerate()
        .map(|(i, suggestion)| QuerySuggestion {
            term: suggestion.term.to_string(),
            label: format!(
                "Expand {} with {} · {}",
                suggestion.source,
                suggestion.term,
                suggestion.relation.label()
            ),
            action: SearchAct::AddSynonym(i),
        })
        .collect::<Vec<_>>();
    let corpus_terms = s.corpus_term_suggestions();
    for (i, term) in corpus_terms.into_iter().enumerate() {
        if !suggestions
            .iter()
            .any(|suggestion| suggestion.term.eq_ignore_ascii_case(&term))
        {
            suggestions.push(QuerySuggestion {
                term: term.clone(),
                label: format!("Related from hits: {term}"),
                action: SearchAct::AddCorpusTerm(i),
            });
        }
    }
    if let Some(query) = s.completed_search_query.as_deref().filter(|query| {
        !s.st.run.is_running
            && s.st.run.started.is_some()
            && s.st.run.results.is_empty()
            && *query == s.st.config.filters_text
    }) {
        for (i, correction) in search_core::synonyms::typo_corrections(query)
            .into_iter()
            .enumerate()
        {
            suggestions.push(QuerySuggestion {
                term: correction.query,
                label: format!(
                    "Try '{}' instead of '{}'",
                    correction.replacement, correction.original
                ),
                action: SearchAct::ApplyCorrection(i),
            });
        }
    }
    suggestions
}

#[cfg(feature = "neural-search")]
pub(super) fn neural_suggestion_inputs(s: &SearchSession) -> (String, Vec<String>) {
    let context = format!(
        "Search query: {}. Folder: {}. File types: {}.",
        s.st.config.filters_text,
        s.st.config.search_path,
        s.st.config
            .selected_extensions
            .as_ref()
            .map_or_else(|| "all supported".to_string(), |v| v.join(" ")),
    );
    let candidates = query_suggestions(s)
        .into_iter()
        .map(|suggestion| suggestion.term)
        .collect();
    (context, candidates)
}

fn find_suggestions(s: &SearchSession) -> Vec<Entry> {
    let mut suggestions: Vec<Entry> = recent_filter_suggestions(s).into_iter().take(2).collect();
    let candidates = query_suggestions(s);
    #[cfg(feature = "neural-search")]
    {
        if !candidates.is_empty() {
            let (context, embedding_candidates) = neural_suggestion_inputs(s);
            let key = format!("{context}\0{}", embedding_candidates.join("\0"));
            if let Some((ranked_key, order)) = &s.neural_order {
                if ranked_key == &key && order.len() == embedding_candidates.len() {
                    suggestions.extend(order.iter().take(3).filter_map(|&i| {
                        query_suggestions(s)
                            .get(i)
                            .map(|candidate| e(&candidate.label, "", candidate.action))
                    }));
                    return suggestions;
                }
            }
            suggestions.extend(
                candidates
                    .iter()
                    .take(3)
                    .map(|candidate| e(&candidate.label, "", candidate.action)),
            );
        }
        suggestions
    }
    #[cfg(not(feature = "neural-search"))]
    {
        suggestions.extend(
            candidates
                .into_iter()
                .take(3)
                .map(|candidate| e(&candidate.label, "", candidate.action)),
        );
        suggestions
    }
}

pub(super) fn document(s: &SearchSession) -> Document {
    let c = &s.st.config;
    let run = &s.st.run;
    let groups_note = plain(&query_preview(c, s.show_exclude));
    let inputs = vec![
        DocInput {
            id: INPUT_FOLDER,
            label: "Folder".into(),
            text: c.search_path.clone(),
            hint: "Folder to search, e.g. C:\\Projects".into(),
            error: None,
            note: (!c.search_paths_extra.is_empty()).then(|| {
                format!(
                    "Also searching {}",
                    plural(c.search_paths_extra.len(), "more folder", "more folders")
                )
            }),
            submit: Some(act(SearchAct::Run)),
            commit_on_blur: false,
            grow: 1.0,
            trailing: Some(e("Browse", "", SearchAct::BrowseFolder)),
            help: None,
            choices: recent_folder_suggestions(s),
        },
        DocInput {
            id: INPUT_FIND,
            label: "Find".into(),
            text: c.filters_text.clone(),
            hint: "Words or phrases, a sentence, or a question".into(),
            error: model::regex_validation_error(c),
            note: (!groups_note.is_empty()).then_some(groups_note),
            submit: Some(act(SearchAct::Run)),
            commit_on_blur: false,
            grow: 1.5,
            trailing: None,
            help: Some(find_help()),
            choices: find_suggestions(s),
        },
    ];
    let extra_inputs = if s.show_exclude {
        vec![exclude_input(s)]
    } else {
        Vec::new()
    };
    let mut buttons = if run.is_running {
        vec![stop_entry(s)]
    } else {
        vec![run_entry(s)]
    };
    if run.last_report_path.is_some() {
        buttons.push(report_entry(s));
    }
    let mut chips = vec![Entry::new(
        s.find_as.label(),
        "How Find reads what you typed: auto picks words, a sentence or a question; click to pin one",
        act(SearchAct::FindAs),
    )
    .checked(s.find_as != super::find::FindAs::Auto)];
    chips.extend(match_mode_chips(s));
    chips.push(exclude_chip(s));
    chips.push(switch(s, "Regex", 6));
    chips.push(switch(s, "Whole word", 7));
    chips.push(switch(s, "Hidden files", 11));
    chips.push(switch(s, "Fast index", 29));
    let types = match &c.selected_extensions {
        Some(list) if list.len() <= 3 => format!("Types: {}", list.join(" ")),
        Some(list) => format!("Types: {} selected", list.len()),
        None => "Types: all".into(),
    };
    chips.push(
        Entry::new(types, "", act(SearchAct::PickExtensions))
            .checked(c.selected_extensions.is_some()),
    );

    let notice = if let Some((changed, removed)) = s.stale {
        let mut parts = Vec::new();
        if changed > 0 {
            parts.push(format!("{changed} new or changed"));
        }
        if removed > 0 {
            parts.push(format!("{removed} removed"));
        }
        Some(DocNotice {
            text: format!(
                "The fast index is out of date ({} files). Update it so the next search is quick?",
                parts.join(", ")
            ),
            tone: Tone::Warn,
            actions: vec![
                e("Update index", "", SearchAct::UpdateStaleIndex),
                e("Not now", "", SearchAct::DismissStale),
            ],
        })
    } else if proximity_widening_available(s) {
        Some(DocNotice {
            text: format!(
                "Matching terms were found, but none fell within {} lines. Try a wider proximity window?",
                c.proximity_lines
            ),
            tone: Tone::Normal,
            actions: vec![e("Widen by 3 lines", "", SearchAct::WidenProximity)],
        })
    } else if run.status_text.starts_with("Error") {
        Some(DocNotice {
            text: run.status_text.clone(),
            tone: Tone::Warn,
            actions: Vec::new(),
        })
    } else {
        s.st.index_run
            .last_error
            .as_ref()
            .filter(|_| !s.st.index_run.is_building)
            .map(|err| DocNotice {
                text: format!("Fast index: {err}"),
                tone: Tone::Warn,
                actions: vec![e("Build again", "", SearchAct::BuildIndex)],
            })
    };

    let progress = if run.started.is_some() {
        Some(run_progress(s))
    } else if s.st.index_run.is_building {
        let ix = &s.st.index_run;
        Some(DocProgress {
            fraction: (ix.percent > 0.0).then_some((ix.percent / 100.0) as f32),
            headline: format!("Fast index: {}", plain(&ix.status_text)),
            detail: "The index makes repeat searches of this folder near instant; the search itself stays exact.".into(),
            active: (!ix.current_file.is_empty()).then(|| (ix.current_file.clone(), String::new())).into_iter().collect(),
            metrics: Vec::new(),
            cancel: Some(e("Stop", "Esc", SearchAct::CancelIndex)),
        })
    } else {
        None
    };

    let stats = if run.started.is_some() && !run.is_running {
        let hits: usize = run.results.iter().map(|r| r.hits.len()).sum();
        let mut v = vec![
            ("Files with matches".to_string(), group(run.results.len())),
            ("Matching lines".to_string(), group(hits)),
        ];
        if let Some(d) = &run.diagnostics {
            v.push((
                "Files checked".to_string(),
                group(d.verified.max(0) as usize),
            ));
        }
        v.push((
            "Time".to_string(),
            crate::format::format_elapsed(run.elapsed_secs().unwrap_or(0.0)),
        ));
        v
    } else {
        Vec::new()
    };

    let c = cards(s);
    let index_on = settings_view::setting_on(&s.st.config, 29);
    let panels = vec![
        DocPanel {
            label: "Options".into(),
            badge: Some(if s.st.index_run.is_building {
                "index building".into()
            } else if index_on {
                "index on".into()
            } else {
                "index off".into()
            }),
            progress: s
                .st
                .index_run
                .is_building
                .then(|| (s.st.index_run.percent / 100.0).clamp(0.02, 1.0) as f32),
            inputs: Vec::new(),
            chips: Vec::new(),
            cards: vec![c.matching, c.files, c.performance, c.index, c.summary],
        },
        DocPanel {
            label: "Report".into(),
            progress: None,
            badge: s
                .st
                .run
                .last_report_path
                .as_ref()
                .map(|_| "ready".to_string()),
            inputs: vec![output_input(s)],
            chips: Vec::new(),
            cards: vec![c.output],
        },
    ];
    let modal = types_card(s).map(|card| DocModal {
        card,
        close: act(SearchAct::CloseExtensions),
    });
    let ix = &s.st.index_run;
    let index_detail = ix.docs.map_or_else(
        || "Build to accelerate repeat searches".into(),
        |n| format!("{} indexed files", group(n as usize)),
    );
    let sidebar = DocSidebar {
        recent_searches: s
            .st
            .recent_searches
            .iter()
            .enumerate()
            .map(|(i, r)| DocSidebarRecent {
                label: r.label(),
                action: act(SearchAct::ApplyRecent(i)),
                remove: act(SearchAct::RemoveRecent(i)),
            })
            .collect(),
        clear_recents: (!s.st.recent_searches.is_empty()).then(|| act(SearchAct::ClearRecents)),
        index_state: if ix.is_building {
            "Building"
        } else if ix.docs.is_some() {
            "Ready"
        } else {
            "Not built"
        }
        .into(),
        index_name: file_name(s.st.config.search_path.trim()).to_string(),
        index_detail,
        index_action: (!ix.is_building && !s.st.config.search_path.trim().is_empty()).then(|| {
            e(
                if ix.docs.is_some() {
                    "Build / update index"
                } else {
                    "Build index"
                },
                "",
                SearchAct::BuildIndex,
            )
        }),
    };
    Document {
        precision: None,
        policy: None,
        sidebar: Some(sidebar),
        header: Some(crate::gpu_viewer::ui::DocHeader {
            eyebrow: "ENGINEERING ARCHIVE".into(),
            title: "Search files".into(),
            subtitle: "Find the details you need across project documents and technical records."
                .into(),
        }),
        inputs,
        extra_inputs,
        buttons,
        chips,
        notice,
        progress,
        stats,
        table: table(s),
        detail: detail(s),
        panels,
        modal,
        ribbon: Some(status_strip(s)),
        insight: insight(s),
    }
}

/// The bottom strip: the semantic index state, the size of the result, and a PDF warning for the selected file.
fn status_strip(s: &SearchSession) -> DocRibbon {
    let ix = &s.st.index_run;
    let run = &s.st.run;
    let (state, tone) = if ix.is_building {
        ("Index building", Tone::Warn)
    } else if ix.docs.is_some() {
        ("Index ready", Tone::On)
    } else {
        ("No index", Tone::Dim)
    };
    let mut facts = Vec::new();
    if run.started.is_some() {
        let hits: usize = run.results.iter().map(|r| r.hits.len()).sum();
        facts.push(format!(
            "{} · {}",
            plural(run.results.len(), "file", "files"),
            plural(hits, "hit", "hits")
        ));
    }
    if let Some(n) = ix.docs {
        facts.push(format!("{} indexed", group(n as usize)));
    }
    DocRibbon {
        state: state.into(),
        tone,
        facts,
        progress: ix
            .is_building
            .then(|| ("Indexing".to_string(), (ix.percent / 100.0).clamp(0.0, 1.0) as f32)),
        warning: s
            .current_result()
            .filter(|r| r.low_confidence_pdf)
            .map(|_| "PDF text may be misread".to_string()),
    }
}

fn proximity_widening_available(s: &SearchSession) -> bool {
    s.can_widen_proximity()
}

/// The switch that shows the Exclude row. While the row is hidden and terms are still set, the switch names them: an
/// exclusion that applies is never out of sight.
fn exclude_chip(s: &SearchSession) -> Entry {
    let terms = model::parse_list(&s.st.config.exclude_filters_text);
    let label = if s.show_exclude || terms.is_empty() {
        "Exclude".to_string()
    } else {
        let shown = terms.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
        if terms.len() > 2 {
            format!("Exclude: {shown} +{}", terms.len() - 2)
        } else {
            format!("Exclude: {shown}")
        }
    };
    Entry::new(label, "", act(SearchAct::ToggleExclude))
        .checked(s.show_exclude || !terms.is_empty())
}

/// The Exclude row: terms that always leave things out, with the choice of what they leave out.
fn exclude_input(s: &SearchSession) -> DocInput {
    let c = &s.st.config;
    let terms = model::parse_list(&c.exclude_filters_text);
    let scope = c.exclude_scope;
    let note = (!terms.is_empty()).then(|| {
        let list = terms.join(", ");
        match scope {
            ExcludeScope::File => format!("Leaves out every file that contains {list}."),
            ExcludeScope::Line => format!(
                "Ignores the lines that contain {list}; the rest of each file still counts."
            ),
        }
    });
    DocInput {
        id: INPUT_EXCLUDE,
        label: "Exclude".into(),
        text: c.exclude_filters_text.clone(),
        hint: "Words or phrases to leave out, comma-separated".into(),
        error: None,
        note,
        submit: Some(act(SearchAct::Run)),
        commit_on_blur: false,
        grow: 1.0,
        trailing: None,
        help: Some(exclude_help()),
        choices: vec![
            e(
                "Whole file",
                "",
                SearchAct::ExcludeScope(ExcludeScope::File),
            )
            .checked(scope == ExcludeScope::File),
            e(
                "Matching lines",
                "",
                SearchAct::ExcludeScope(ExcludeScope::Line),
            )
            .checked(scope == ExcludeScope::Line),
        ],
    }
}

fn query_preview(config: &model::SearchToolConfig, exclude_visible: bool) -> String {
    let groups = model::parse_filter_groups(&config.filters_text);
    let mut parts = Vec::new();
    if !groups.is_empty() {
        let summary = if groups.len() > 1 {
            run_view::filter_group_hint(config, false)
        } else {
            let group = &groups[0];
            format!(
                "{} [{}]",
                group.filters.join(", "),
                model::mode_label(model::effective_mode(group, config), config.proximity_lines)
            )
        };
        parts.push(format!("Find: {summary}"));
    }
    let excluded = model::parse_list(&config.exclude_filters_text);
    if !exclude_visible && !excluded.is_empty() {
        let scope = match config.exclude_scope {
            ExcludeScope::File => "whole files",
            ExcludeScope::Line => "matching lines",
        };
        parts.push(format!("Exclude {} from {scope}", excluded.join(", ")));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("Preview: {}", parts.join(" · "))
    }
}

/// The conversation: where Ask looks, then each question with the lines that answer it, quoted unchanged.
#[cfg(feature = "semantic-search")]
fn ask_items(s: &SearchSession) -> Vec<DockItem> {
    let files_ready = s.st.config.index.semantic && !s.st.config.search_path.trim().is_empty();
    let mut items = vec![DockItem::Chips {
        title: "Look in".into(),
        chips: vec![
            e("Search help", "the Search app's own help", SearchAct::AskHelp).checked(s.ask_help),
            e("My files", "the indexed folder", SearchAct::AskFiles).checked(s.ask_files),
        ],
    }];
    let Some(parts) = s.current_answers() else {
        if !s.answer_running {
            items.push(DockItem::Chips {
                title: "Try asking".into(),
                chips: super::help::EXAMPLES
                    .iter()
                    .enumerate()
                    .map(|(i, question)| e(question, "", SearchAct::AskExample(i)))
                    .collect(),
            });
        }
        if s.ask_files && !files_ready {
            items.push(DockItem::text(
                "Questions about your files need a folder and the Semantic index (Options).",
                Tone::Dim,
            ));
        }
        return items;
    };
    for (part_index, part) in parts.iter().enumerate() {
        items.push(DockItem::Asked(plain(&part.question)));
        items.extend(part.facts.iter().map(|fact| DockItem::Reply {
            text: plain(&fact.text),
            source: fact.source.clone(),
            tone: Tone::Title,
            action: None,
        }));
        if let Some(help) = &part.help {
            match help {
                Ok(lines) if lines.is_empty() => items.push(DockItem::text(
                    "The Search help does not cover this.",
                    Tone::Dim,
                )),
                Ok(lines) => items.extend(lines.iter().map(|line| DockItem::Reply {
                    text: plain(&line.line),
                    source: format!("Search help · {}", line.section),
                    tone: Tone::Title,
                    action: None,
                })),
                Err(error) => items.push(DockItem::text(plain(error), Tone::Warn)),
            }
        }
        match (&part.evidence, part.files_asked) {
            (Ok(lines), true) if lines.is_empty() => items.push(DockItem::text(
                "No lines of your files matched.",
                Tone::Dim,
            )),
            (Ok(lines), true) => {
                items.extend(lines.iter().enumerate().map(|(i, line)| DockItem::Reply {
                    text: plain(&line.answer),
                    source: semantic_citation(&line.hit),
                    tone: Tone::Normal,
                    action: Some(act(SearchAct::OpenAnswer(part_index, i))),
                }))
            }
            (Err(error), true) => items.push(DockItem::text(plain(error), Tone::Warn)),
            _ => {}
        }
    }
    if parts.iter().any(|part| part.files_asked) || parts.iter().any(|part| part.help.is_some()) {
        items.push(DockItem::text(
            "Lines are quoted from their source, unchanged. They may not answer the question; check each in context.",
            Tone::Dim,
        ));
    }
    if s.ask_files && !files_ready {
        items.push(DockItem::text(
            "Your files were not searched: choose a folder and turn on the Semantic index.",
            Tone::Dim,
        ));
    }
    items
}

fn output_input(s: &SearchSession) -> DocInput {
    let folder = s.st.config.output_folder.trim();
    DocInput {
        id: INPUT_OUTPUT,
        label: "Output folder".into(),
        text: s.st.config.output_folder.clone(),
        hint: "Folder for the report and exports".into(),
        error: None,
        note: Some(if folder.is_empty() {
            "Empty: the report goes to the folder the app was started from; Export matches writes beside each file.".into()
        } else {
            "The HTML / CSV / JSON report and Export matches are written here.".into()
        }),
        submit: None,
        commit_on_blur: false,
        grow: 1.0,
        trailing: Some(e("Browse", "", SearchAct::BrowseOutput)),
        help: None,
        choices: Vec::new(),
    }
}

fn match_mode_chips(s: &SearchSession) -> Vec<Entry> {
    // One chip that names the mode; a click moves to the next one.
    let label = match s.st.config.match_mode {
        MatchMode::AnyLine => "Match: any line".to_string(),
        MatchMode::AllInFile => "Match: all in file".to_string(),
        MatchMode::Proximity => format!("Match: within {} lines", s.st.config.proximity_lines),
    };
    vec![Entry::new(label, "", act(SearchAct::Step(4)))
        .checked(s.st.config.match_mode != MatchMode::AnyLine)]
}

fn run_progress(s: &SearchSession) -> DocProgress {
    let run = &s.st.run;
    let elapsed = run.elapsed_secs().unwrap_or(0.0);
    let fraction = if run.started.is_some() && !run.is_running {
        Some(1.0)
    } else {
        (run.total_files > 0)
            .then(|| (run.files_completed as f32 / run.total_files as f32).clamp(0.0, 1.0))
    };
    let mut detail = Vec::new();
    if run.is_running && run.total_files > 0 && elapsed >= 1.0 && run.files_completed > 0 {
        let rate = f64::from(run.files_completed) / elapsed;
        detail.push(format!("{rate:.1} files/s"));
        let left = f64::from((run.total_files - run.files_completed).max(0)) / rate.max(1e-9);
        detail.push(format!("about {} left", format_eta(left)));
    }
    if run.is_running {
        detail.push(format!(
            "{} elapsed",
            crate::format::format_elapsed(elapsed)
        ));
    } else if run.started.is_some() {
        detail.push(format!(
            "Finished in {}",
            crate::format::format_elapsed(elapsed)
        ));
    }
    let root =
        s.st.config
            .search_path
            .trim()
            .trim_end_matches(['/', '\\'])
            .to_string();
    DocProgress {
        fraction,
        headline: if run.started.is_some() && !run.is_running {
            format!(
                "Search complete  ·  {}",
                plural(run.results.len(), "file with matches", "files with matches")
            )
        } else if run.total_files > 0 {
            format!(
                "Searching  ·  {} of {}  ·  {}",
                group(run.files_completed.max(0) as usize),
                plural(run.total_files as usize, "file", "files"),
                plural(run.matching_lines, "matching line", "matching lines")
            )
        } else if run.status_text.is_empty() {
            "Starting...".into()
        } else {
            plain(&run.status_text)
        },
        detail: detail.join("  ·  "),
        // Every file still being read, with how long it has taken (a slow PDF shows itself here).
        active: run
            .in_flight_files
            .iter()
            .map(|f| {
                let shown = f
                    .file_name
                    .strip_prefix(&root)
                    .map(|r| r.trim_start_matches(['/', '\\']).to_string())
                    .filter(|r| !r.is_empty())
                    .unwrap_or_else(|| f.file_name.clone());
                (
                    shown,
                    format!(
                        "{} · {}",
                        plain(&f.status_text),
                        crate::format::format_elapsed(f.elapsed_seconds)
                    ),
                )
            })
            .collect(),
        metrics: {
            let mut values = vec![
                ("Files with matches".into(), group(run.results.len())),
                ("Matching lines".into(), group(run.matching_lines)),
                (
                    "Files checked".into(),
                    group(run.files_completed.max(0) as usize),
                ),
            ];
            if !run.is_running {
                if let Some(d) = &run.diagnostics {
                    values[2].1 = group(d.verified.max(0) as usize);
                }
            }
            values
        },
        cancel: run.is_running.then(|| stop_entry(s)),
    }
}

fn table(s: &SearchSession) -> DocTable {
    let run = &s.st.run;
    let col = |title: &str, width: f32, right: bool, key: SortKey| DocColumn {
        title: title.into(),
        width,
        right,
        sort: Some(act(SearchAct::SortBy(key))),
        sorted: (s.sort.0 == key).then_some(s.sort.1),
    };
    let cached = {
        let mut cache = s.rows_cache.borrow_mut();
        if let Some(c) = cache.as_mut() {
            if c.can_extend(s) {
                let first_implicit = c.rows.is_empty() && s.selected.is_none();
                let needle = s.filter.trim().to_lowercase();
                let root =
                    s.st.config
                        .search_path
                        .trim()
                        .trim_end_matches(['/', '\\'])
                        .to_string();
                for i in c.result_count..run.results.len() {
                    if needle.is_empty()
                        || run.results[i].full_name.to_lowercase().contains(&needle)
                    {
                        let selected =
                            s.selected == Some(i) || (first_implicit && c.rows.is_empty());
                        std::sync::Arc::make_mut(&mut c.rows)
                            .push(result_row(s, i, selected, &root));
                    }
                }
                c.result_count = run.results.len();
            }
            if c.current(s) {
                Some(c.rows.clone())
            } else {
                None
            }
        } else {
            None
        }
    };
    let rows: std::sync::Arc<Vec<DocRow>> = cached.unwrap_or_else(|| {
        let view = s.view();
        let current = s.current();
        let root =
            s.st.config
                .search_path
                .trim()
                .trim_end_matches(['/', '\\'])
                .to_string();
        let rows: Vec<DocRow> = view
            .iter()
            .map(|&i| result_row(s, i, current == Some(i), &root))
            .collect();
        let rows = std::sync::Arc::new(rows);
        *s.rows_cache.borrow_mut() = Some(RowsCache {
            started: run.started,
            result_count: run.results.len(),
            sort: s.sort,
            filter: s.filter.clone(),
            selected: s.selected,
            root: s.st.config.search_path.clone(),
            #[cfg(feature = "neural-search")]
            relevance_key: s.relevance_key(),
            rows: rows.clone(),
        });
        rows
    });
    let total = run.results.len();
    let title = if total == 0 {
        "Results".to_string()
    } else if rows.len() == total {
        format!("Results  ·  {}", plural(total, "file", "files"))
    } else {
        format!(
            "Results  ·  {} of {}",
            group(rows.len()),
            plural(total, "file", "files")
        )
    };
    let (empty_title, empty_body) = if total > 0 {
        (
            "Nothing matches the filter".to_string(),
            "Clear the filter to see every file with matches.".to_string(),
        )
    } else if run.is_running {
        (
            "Searching...".to_string(),
            "Files with matches appear here as soon as they are found.".to_string(),
        )
    } else if run.started.is_some() {
        ("No matches".to_string(), "Nothing in this folder matched. Try fewer words, another match mode, or more file types.".to_string())
    } else {
        ("Search a folder".to_string(), "Choose a folder, type what to find and press Enter. Matches appear here as they are found, with every matching line highlighted.".to_string())
    };
    DocTable {
        title,
        filter: (total > 0).then(|| DocInput {
            id: INPUT_FILTER,
            label: "Filter".into(),
            text: s.filter.clone(),
            hint: "Filter by name or folder".into(),
            error: None,
            note: None,
            submit: None,
            commit_on_blur: false,
            grow: 1.0,
            trailing: None,
            help: None,
            choices: Vec::new(),
        }),
        columns: vec![
            col("File", 0.0, false, SortKey::Name),
            col("Matches", 76.0, true, SortKey::Hits),
            col("Size", 84.0, true, SortKey::Size),
            col("Modified", 138.0, false, SortKey::Modified),
            #[cfg(feature = "neural-search")]
            col("Relevance", 122.0, true, SortKey::Relevance),
        ],
        rows,
        empty_title,
        empty_body,
    }
}

fn result_row(s: &SearchSession, i: usize, selected: bool, root: &str) -> DocRow {
    let r = &s.st.run.results[i];
    let folder = std::path::Path::new(&r.full_name)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let rel = folder
        .strip_prefix(root)
        .map(|f| f.trim_start_matches(['/', '\\']).to_string());
    let sub = match rel {
        Some(f) if f.is_empty() => "(top folder)".to_string(),
        Some(f) => f,
        None => folder,
    };
    DocRow {
        cells: vec![
            file_name(&r.full_name).to_string(),
            group(r.hits.len()),
            crate::format::format_bytes(r.file_length),
            r.modified.format("%Y-%m-%d %H:%M").to_string(),
            #[cfg(feature = "neural-search")]
            s.relevance_score(i)
                .map_or_else(String::new, |score| format!("{:.0}%", score.score * 100.0)),
        ],
        sub,
        selected,
        select: act(SearchAct::Select(i)),
        open: Some(act(SearchAct::Open(i))),
        badge: std::path::Path::new(&r.full_name)
            .extension()
            .map(|x| x.to_string_lossy().to_uppercase()),
    }
}

pub(super) fn full_text(r: &search_core::models::FileSearchResult, lines: &[String]) -> DocText {
    let mut hits = std::collections::HashMap::new();
    for h in &r.hits {
        hits.entry(h.line_number).or_insert_with(Vec::new).push(h);
    }
    let mut content = String::new();
    let mut marks = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            content.push('\n');
        }
        let offset = content.len();
        content.push_str(line);
        if let Some(found) = hits.get(&((i + 1) as i32)) {
            let filters: Vec<String> = found
                .iter()
                .filter(|h| h.match_line == *line)
                .flat_map(|h| h.matched_filters.iter().cloned())
                .collect();
            marks.extend(
                preview::highlight_ranges(line, &filters)
                    .into_iter()
                    .map(|(start, end)| (offset + start, offset + end)),
            );
        }
    }
    DocText {
        content: content.into(),
        marks: marks.into(),
    }
}

fn detail(s: &SearchSession) -> Option<DocDetail> {
    #[cfg(feature = "neural-search")]
    let i = s.current()?;
    let r = s.current_result()?;
    let subtitle = format!(
        "{}  ·  {}",
        r.full_name,
        plain(&preview::metadata_summary(r))
    );
    #[cfg_attr(not(feature = "neural-search"), allow(unused_mut))]
    let mut signals: Vec<DocSignal> = Vec::new();
    #[cfg_attr(not(feature = "neural-search"), allow(unused_mut))]
    let mut notes: Vec<DocNote> = Vec::new();
    #[cfg(feature = "neural-search")]
    if let Some(score) = s.current().and_then(|i| s.relevance_score(i)) {
        let mut signal = |label: &str, fraction: f32, hint: &str, primary: bool| {
            signals.push(DocSignal {
                label: label.into(),
                fraction,
                hint: hint.into(),
                primary,
            })
        };
        signal(
            "Relevance",
            score.score,
            "How well this file fits your search, combining the signals beside it. A ranking aid, not a confidence level.",
            true,
        );
        signal(
            "Text",
            score.match_text_similarity,
            "How closely the matching lines read like your query.",
            false,
        );
        signal(
            "Name",
            score.filename_similarity,
            "How closely the file name resembles your query.",
            false,
        );
        signal(
            "Exact",
            score.exact_evidence,
            "Share of your exact search terms found in the file.",
            false,
        );
        signal(
            "Recent",
            score.recency,
            "Newer files score higher; this fades as the file ages.",
            false,
        );
        if let Some(proximity) = score.proximity {
            signal(
                "Nearby",
                proximity,
                "Matches that sit close together in the file score higher.",
                false,
            );
        }
        if score.feedback_applied {
            notes.push(DocNote::new(
                "Adjusted by feedback",
                format!("Your relevant / not relevant votes moved this file; it scored {:.0}% before.", score.base_score * 100.0),
            ));
        }
    }
    if r.low_confidence_pdf {
        notes.push(DocNote::new("PDF: check source", "Text was read from a PDF and may be misread. Verify against the original."));
    }
    let preview = s.preview.as_ref().filter(|p| p.path == r.full_name);
    let text = preview.and_then(|p| p.text.clone());
    let empty = preview.map_or("Loading the full document...".to_string(), |p| {
        p.status.clone()
    });
    #[cfg(feature = "neural-search")]
    if let Some(preview) = preview {
        match &preview.entities {
            Some(Ok(analysis)) if analysis.candidates.is_empty() => {
                notes.push(DocNote::new("No entities", "No part numbers, standards or fastener callouts were found."));
            }
            Some(Ok(analysis)) => {
                let mut hint = String::from("First suggestions (ranking aids, not confirmed):\n");
                for candidate in analysis.candidates.iter().take(8) {
                    let value = text
                        .as_ref()
                        .and_then(|text| text.content.get(candidate.bytes.clone()))
                        .unwrap_or("?");
                    hint.push_str(&format!(
                        "L{}  {value}  ({})\n",
                        candidate.line_number,
                        entity_signal_label(candidate)
                    ));
                }
                if analysis.coverage.candidate_limit_reached {
                    hint.push_str("The list is capped; more exist.\n");
                }
                if analysis.coverage.skipped_long_lines > 0 {
                    hint.push_str(&format!("{} very long lines were not scanned.", analysis.coverage.skipped_long_lines));
                }
                let count = analysis.candidates.len();
                notes.push(DocNote::new(
                    format!(
                        "{count}{} engineering entit{}",
                        if analysis.coverage.candidate_limit_reached { "+" } else { "" },
                        if count == 1 { "y" } else { "ies" }
                    ),
                    hint.trim_end(),
                ));
            }
            Some(Err(status)) => notes.push(DocNote::new(status.to_string(), "")),
            None => {}
        }
    }
    let subtitle = if text.is_some() {
        format!("{subtitle}  ·  Select text, then right-click to copy")
    } else {
        subtitle
    };
    Some(DocDetail {
        drawing: None,
        grid: None,
        title: file_name(&r.full_name).to_string(),
        subtitle,
        signals,
        notes,
        #[cfg(feature = "neural-search")]
        actions: if s.feedback_enabled() && s.relevance_score(i).is_some() {
            vec![
                Entry::new(
                    "Relevant",
                    "",
                    act(SearchAct::VoteRelevance(
                        i,
                        search_core::relevance::FeedbackLabel::Relevant,
                    )),
                ),
                Entry::new(
                    "Not relevant",
                    "",
                    act(SearchAct::VoteRelevance(
                        i,
                        search_core::relevance::FeedbackLabel::NotRelevant,
                    )),
                ),
            ]
        } else {
            Vec::new()
        },
        #[cfg(not(feature = "neural-search"))]
        actions: Vec::new(),
        match_navigation: (r.hits.len() > 1).then(|| DocMatchNavigation {
            current: s.match_index + 1,
            total: r.hits.len(),
            previous: act(SearchAct::PreviousMatch),
            next: act(SearchAct::NextMatch),
        }),
        scroll_to_line: r.hits.get(s.match_index).map(|hit| hit.line_number),
        scroll_request: Some(s.match_scroll_request),
        blocks: Vec::new(),
        text,
        empty,
    })
}

#[cfg(feature = "neural-search")]
pub(super) fn entity_signal_label(candidate: &search_core::entities::EntityCandidate) -> String {
    let neural = candidate.category_scores.first();
    let supported = candidate.neural_suggestion();
    let likely = supported.map_or_else(
        || "unranked".to_string(),
        |score| format!("likely {:?} {:.0}%", score.kind, score.similarity * 100.0),
    );
    let global_disagrees = neural
        .zip(supported)
        .is_some_and(|(global, supported)| global.kind != supported.kind);
    if candidate.shape_hints.len() > 1 || global_disagrees {
        let hints = candidate
            .shape_hints
            .iter()
            .map(|kind| format!("{kind:?}"))
            .collect::<Vec<_>>()
            .join(" or ");
        if !hints.is_empty() {
            let global = neural.map_or_else(String::new, |score| {
                if global_disagrees {
                    format!(", best overall {:?}", score.kind)
                } else {
                    String::new()
                }
            });
            return format!("{hints}: {likely}{global}");
        }
    }
    likely
}

// ---------------------------------------------------------------------- tree

const NODE_SEARCH: u32 = 1;
const NODE_TYPES: u32 = 2;
const NODE_RECENT: u32 = 3;
const NODE_PRESETS: u32 = 4;
const NODE_TYPE0: u32 = 100;
const NODE_RECENT0: u32 = 1_000;
const NODE_PRESET0: u32 = 2_000;

pub(super) fn tree(s: &SearchSession) -> Vec<TreeNode> {
    let c = &s.st.config;
    let mut t = vec![TreeNode::new(NODE_SEARCH, 0, '+', "Search").folder()];
    let folder = c.search_path.trim();
    t.push(
        TreeNode::new(
            10,
            1,
            'o',
            if folder.is_empty() {
                "Folder: not chosen".to_string()
            } else {
                format!("Folder: {}", file_name(folder))
            },
        )
        .tone(if folder.is_empty() {
            Tone::Warn
        } else {
            Tone::Normal
        })
        .on_click(act(SearchAct::BrowseFolder)),
    );
    let find = c.filters_text.trim();
    t.push(
        TreeNode::new(
            11,
            1,
            'o',
            if find.is_empty() {
                "Find: nothing yet".to_string()
            } else {
                format!("Find: {find}")
            },
        )
        .tone(if find.is_empty() {
            Tone::Warn
        } else {
            Tone::Normal
        })
        .on_click(act(SearchAct::StepTab("Search"))),
    );
    let types = s.types();
    t.push(
        TreeNode::new(
            NODE_TYPES,
            0,
            '+',
            format!("Results ({})", group(s.st.run.results.len())),
        )
        .folder(),
    );
    if !types.is_empty() {
        t.push(
            TreeNode::new(NODE_TYPE0 - 1, 1, 'R', "All types")
                .selected(s.filter.is_empty())
                .on_click(act(SearchAct::OnlyType(None))),
        );
    }
    for (k, (ext, n)) in types.iter().enumerate() {
        t.push(
            TreeNode::new(NODE_TYPE0 + k as u32, 1, 'f', format!("{ext}  {n}"))
                .selected(s.filter.eq_ignore_ascii_case(ext))
                .on_click(act(SearchAct::OnlyType(Some(k)))),
        );
    }
    if !s.st.recent_searches.is_empty() {
        t.push(TreeNode::new(NODE_RECENT, 0, '+', "Recent searches").folder());
        for (k, r) in s.st.recent_searches.iter().enumerate() {
            t.push(
                TreeNode::new(NODE_RECENT0 + k as u32, 1, '#', r.label())
                    .on_click(act(SearchAct::ApplyRecent(k))),
            );
        }
    }
    if !s.st.saved_presets.is_empty() {
        t.push(TreeNode::new(NODE_PRESETS, 0, '+', "Presets").folder());
        for (k, p) in s.st.saved_presets.iter().enumerate() {
            t.push(
                TreeNode::new(NODE_PRESET0 + k as u32, 1, 'M', p.name.clone())
                    .on_click(act(SearchAct::ApplyPreset(k))),
            );
        }
    }
    t
}

pub(super) fn tree_menu(s: &SearchSession, node: u32) -> Vec<Item> {
    let item = |x: Entry| Item::Entry(x);
    match node {
        n if (NODE_PRESET0..NODE_PRESET0 + 1_000).contains(&n) => {
            let k = (n - NODE_PRESET0) as usize;
            vec![
                item(e("Apply preset", "", SearchAct::ApplyPreset(k))),
                item(e("Delete preset", "", SearchAct::DeletePreset(k))),
            ]
        }
        n if (NODE_RECENT0..NODE_PRESET0).contains(&n) => vec![
            item(e(
                "Load this search",
                "",
                SearchAct::ApplyRecent((n - NODE_RECENT0) as usize),
            )),
            item(e(
                "Remove this search",
                "",
                SearchAct::RemoveRecent((n - NODE_RECENT0) as usize),
            )),
        ],
        NODE_PRESETS => vec![item(e(
            "Save settings as preset...",
            "",
            SearchAct::SavePreset,
        ))],
        NODE_RECENT => vec![item(e(
            "Clear recent searches",
            "",
            SearchAct::ClearRecents,
        ))],
        10 => vec![item(e("Choose a folder...", "", SearchAct::BrowseFolder))],
        _ if s.current_result().is_some() => {
            result_entries(s).into_iter().map(Item::Entry).collect()
        }
        _ => Vec::new(),
    }
}

pub(super) fn context_menu(s: &SearchSession) -> Vec<Item> {
    let mut items: Vec<Item> = result_entries(s).into_iter().map(Item::Entry).collect();
    items.push(Item::Separator);
    items.push(Item::Entry(report_entry(s)));
    items
}

// ---------------------------------------------------------------------- inspector

/// Rows for the settings of `groups`, skipping the ones the search bar already shows.
fn setting_rows(s: &SearchSession, groups: &[&str]) -> Vec<DockItem> {
    settings_view::settings()
        .filter(|(i, g, ..)| {
            groups.contains(g)
                && ![
                    SETTING_PATH,
                    SETTING_FILTERS,
                    SETTING_EXCLUDE,
                    SETTING_EXCLUDE_SCOPE,
                    SETTING_OUTPUT,
                ]
                .contains(i)
        })
        .map(|(i, _, label, _, kind)| {
            let value = settings_view::setting_display(&s.st.config, i);
            let tone = match kind {
                settings_view::SettingKind::Switch
                    if settings_view::setting_on(&s.st.config, i) =>
                {
                    Tone::On
                }
                _ => Tone::Normal,
            };
            let value = if value.trim().is_empty() {
                "-".to_string()
            } else {
                value
            };
            DockItem::field(
                label,
                value,
                tone,
                Some(act(SearchAct::Edit(i))),
                s.edit.as_ref().and_then(|e| e.setting) == Some(i),
            )
        })
        .collect()
}

/// The option cards, grouped as the page's popovers show them (Options, Index, Report).
pub(super) struct Cards {
    matching: Card,
    files: Card,
    performance: Card,
    output: Card,
    index: Card,
    summary: Card,
}

pub(super) fn cards(s: &SearchSession) -> Cards {
    let ix = &s.st.index_run;
    let mut index = setting_rows(s, &["Fast Re-search Index"]);
    if ix.is_building {
        index.push(DockItem::row(
            "Stop the build",
            Tone::Warn,
            Some(act(SearchAct::CancelIndex)),
            false,
        ));
    } else if !s.st.run.is_running && !s.st.config.search_path.trim().is_empty() {
        index.push(DockItem::row(
            if ix.docs.is_some() {
                "Update the index now"
            } else {
                "Build the index now"
            },
            Tone::Title,
            Some(act(SearchAct::BuildIndex)),
            false,
        ));
    }
    if !ix.status_text.is_empty() {
        index.push(DockItem::text(plain(&ix.status_text), Tone::Dim));
    }
    if let Some(n) = ix.docs {
        index.push(DockItem::field(
            "Documents",
            group(n as usize),
            Tone::Normal,
            None,
            false,
        ));
    }
    if let Some(n) = &ix.last_narrow {
        index.push(DockItem::text(plain(n), Tone::Dim));
    }
    index.push(DockItem::text("The index only narrows which files are read; every match is still checked in the file itself.", Tone::Dim));
    #[cfg(feature = "neural-search")]
    {
        let has_preview = s
            .preview
            .as_ref()
            .is_some_and(|preview| preview.text.is_some());
        index.push(DockItem::row(
            if s.summary_running {
                "Summarizing selected document…"
            } else {
                "Summarize selected document"
            },
            Tone::Title,
            has_preview.then_some(act(SearchAct::SummarizeSelected)),
            false,
        ));
        if let Some(summary) = s.current_summary() {
            index.push(DockItem::Heading(
                "Extractive summary · selected source lines".into(),
            ));
            index.push(DockItem::text(
                format!(
                    "{} source lines; sampled {}; {} long lines skipped. Text is copied from the source, not generated.",
                    summary.source_lines, summary.sampled_lines, summary.skipped_long_lines
                ),
                Tone::Dim,
            ));
            for line in &summary.lines {
                index.push(DockItem::text(
                    format!("L{}  {}", line.line_number, plain(&line.text)),
                    Tone::Normal,
                ));
            }
            if summary.lines.is_empty() {
                index.push(DockItem::text(
                    "No eligible source lines were available to summarize.",
                    Tone::Dim,
                ));
            }
        }
        if let Some(error) = &s.summary_error {
            index.push(DockItem::text(plain(error), Tone::Warn));
        }
    }
    #[cfg(feature = "semantic-search")]
    {
        index.push(DockItem::row(
            if s.related_running {
                "Comparing selected document…"
            } else {
                "Find possible related documents"
            },
            Tone::Title,
            Some(act(SearchAct::FindRelated)),
            false,
        ));
        if let Some(hits) = s.current_related_hits() {
            index.push(DockItem::Heading("Possible related documents".into()));
            index.push(DockItem::text(
                "Grouped by file. Similarity ranks candidates; it does not prove a duplicate or a revision, so read the excerpts.",
                Tone::Dim,
            ));
            for (i, hit) in hits.iter().enumerate() {
                index.push(DockItem::row(
                    format!(
                        "{} · {:.0}% similar",
                        semantic_citation(hit),
                        hit.similarity * 100.0
                    ),
                    Tone::Normal,
                    Some(act(SearchAct::OpenRelated(i))),
                    false,
                ));
                index.push(DockItem::text(plain(&hit.excerpt), Tone::Dim));
                index.push(DockItem::row(
                    if s.diff_running
                        && s.diff_request
                            .as_ref()
                            .is_some_and(|key| key.candidate_path == hit.path)
                    {
                        "Comparing exact lines…"
                    } else {
                        "Compare exact lines"
                    },
                    Tone::Title,
                    Some(act(SearchAct::CompareRelated(i))),
                    false,
                ));
            }
            if hits.is_empty() {
                index.push(DockItem::text(
                    "No related indexed documents found.",
                    Tone::Dim,
                ));
            }
        }
        if let Some(diff) = s.current_related_diff() {
            let identical = diff.added == 0 && diff.removed == 0;
            index.push(DockItem::Heading(if identical {
                "Identical extracted text".into()
            } else {
                "Exact extracted-line changes".into()
            }));
            index.push(DockItem::text(if identical {
                "The extracted lines match exactly. This confirms text equality, not binary-file identity."
            } else {
                "Line numbers refer to extracted text. PDF pages are flattened. Similarity does not classify revisions."
            }, Tone::Dim));
            for row in &diff.rows {
                let prefix = match row.side {
                    super::diff::Side::Source => "−",
                    super::diff::Side::Candidate => "+",
                };
                let suffix = if row.clipped { " …" } else { "" };
                index.push(DockItem::text(
                    format!("{prefix} {:>5}  {}{suffix}", row.line, row.text),
                    Tone::Normal,
                ));
            }
            if diff.omitted > 0 {
                index.push(DockItem::text(
                    format!("{} additional changed lines omitted.", diff.omitted),
                    Tone::Dim,
                ));
            }
        }
        if let Some(error) = &s.diff_error {
            index.push(DockItem::text(plain(error), Tone::Warn));
        }
        if let Some(error) = &s.related_error {
            index.push(DockItem::text(plain(error), Tone::Warn));
        }
    }
    #[cfg(feature = "neural-search")]
    {
        index.push(DockItem::Heading("Saved search alerts".into()));
        index.push(DockItem::text(
            "Alerts match files by exact search. Similarity scores only help you review changes; they never decide what is important.",
            Tone::Dim,
        ));
        if s.alerts.len() < search_core::alerts::MAX_SAVED_ALERTS {
            index.push(DockItem::row(
                "Save current search as alert",
                Tone::Title,
                Some(act(SearchAct::SaveAlert)),
                false,
            ));
        } else {
            index.push(DockItem::text(
                "Saved alert limit reached (32).",
                Tone::Warn,
            ));
        }
        if let Some(status) = &s.alert_status {
            index.push(DockItem::text(plain(status), Tone::Dim));
        }
        for (i, record) in s.alerts.iter().enumerate() {
            let id = &record.alert.id;
            index.push(DockItem::Heading(plain(&record.alert.name)));
            index.push(DockItem::text(
                if record.alert.baseline.is_some() {
                    format!(
                        "Baseline: {} files",
                        record.alert.baseline.as_ref().map_or(0, Vec::len)
                    )
                } else {
                    "Baseline not captured".into()
                },
                Tone::Dim,
            ));
            let busy = s.alert_jobs.contains_key(id);
            if !busy {
                if record.alert.baseline.is_some() {
                    index.push(DockItem::row(
                        "Check now",
                        Tone::Title,
                        Some(act(SearchAct::CheckAlert(i))),
                        false,
                    ));
                    index.push(DockItem::row(
                        "Replace baseline",
                        Tone::Normal,
                        Some(act(SearchAct::CaptureAlert(i))),
                        false,
                    ));
                } else {
                    index.push(DockItem::row(
                        "Capture baseline",
                        Tone::Title,
                        Some(act(SearchAct::CaptureAlert(i))),
                        false,
                    ));
                }
            } else {
                index.push(DockItem::text("Checking…", Tone::Dim));
            }
            index.push(DockItem::row(
                "Delete alert",
                Tone::Warn,
                Some(act(SearchAct::DeleteAlert(i))),
                false,
            ));
        }
        if let Some((name, changes)) = &s.last_alert_changes {
            index.push(DockItem::Heading(format!("Latest check: {}", plain(name))));
            index.push(DockItem::text(
                "Exact-hit file changes. Cosine distance is an uncalibrated review signal.",
                Tone::Dim,
            ));
            for change in changes.iter().take(10) {
                let detail = match change {
                    search_core::alerts::AlertChange::New { path } => {
                        format!("New · {}", plain(path))
                    }
                    search_core::alerts::AlertChange::Updated {
                        path,
                        cosine_distance,
                    } => format!(
                        "Updated · {} · {:.0}% different",
                        plain(path),
                        f64::from(*cosine_distance) * 100.0
                    ),
                };
                index.push(DockItem::text(detail, Tone::Normal));
            }
            if changes.len() > 10 {
                index.push(DockItem::text(
                    format!("{} more changes not shown.", changes.len() - 10),
                    Tone::Dim,
                ));
            }
        }
    }
    let mut summary = Vec::new();
    if let Some(d) = &s.st.run.diagnostics {
        summary.extend(
            d.lines(ix.last_build_failed)
                .into_iter()
                .map(|l| DockItem::text(plain(&l), Tone::Dim)),
        );
    }
    if !s.st.run.results_summary_text.is_empty() {
        summary.push(DockItem::text(
            s.st.run.results_summary_text.clone(),
            Tone::Dim,
        ));
    }
    let mut output = setting_rows(s, &["Output"]);
    if let Some(p) = &s.st.run.last_report_path {
        output.push(DockItem::row(
            format!("Open report: {}", file_name(p)),
            Tone::Title,
            Some(act(SearchAct::OpenReport)),
            false,
        ));
    }
    Cards {
        matching: Card::new(
            "search-matching",
            "Matching",
            Relevance::Context,
            setting_rows(s, &["Filters"]),
        )
        .for_steps(&["Search"]),
        files: Card::new(
            "search-files",
            "Files and folders",
            Relevance::Context,
            setting_rows(s, &["Search", "Extensions & Exclusions"]),
        )
        .for_steps(&["Search"]),
        performance: Card::new(
            "search-performance",
            "Performance",
            Relevance::Info,
            setting_rows(s, &["Concurrency & Diagnostics"]),
        )
        .for_steps(&["Search"]),
        output: Card::new(
            "search-output",
            "Report and exports",
            Relevance::Context,
            output,
        )
        .for_steps(&["Output"]),
        index: Card::new("search-index", "Fast index", Relevance::Primary, index)
            .for_steps(&["Index"]),
        summary: Card::new("search-summary", "Last run", Relevance::Info, summary)
            .for_steps(&["Index"]),
    }
}

/// What Find found besides files, in one card on the page: cited answers to a question, a sentence's conditions with
/// their passages, and the passages the Semantic index finds for the words. `None` when there is nothing to show.
pub(super) fn insight(s: &SearchSession) -> Option<DocInsight> {
    #[cfg_attr(not(feature = "semantic-search"), allow(unused_mut))]
    let mut items: Vec<DockItem> = Vec::new();
    #[cfg_attr(not(feature = "nl-find"), allow(unused_mut))]
    let mut chips: Vec<Entry> = Vec::new();
    #[cfg(feature = "semantic-search")]
    {
        if s.answer_running {
            items.push(DockItem::text("Looking for cited lines…", Tone::Dim));
        } else if s.current_answers().is_some() || s.answer_error.is_some() {
            items.extend(ask_items(s));
            let files_ready = s.st.config.index.semantic && !s.st.config.search_path.trim().is_empty();
            if s.ask_files && !files_ready && s.current_answers().is_some() {
                items.push(DockItem::text(
                    "Your files were not searched: choose a folder and turn on the Semantic index (Options).",
                    Tone::Dim,
                ));
            }
        }
        if let Some(error) = &s.answer_error {
            items.push(DockItem::text(plain(error), Tone::Warn));
        }
    }
    #[cfg(feature = "nl-find")]
    {
        let nl = &s.nl;
        if nl.conditions.is_some() || nl.stage != super::nl::Stage::Idle || nl.error.is_some() || !nl.files.is_empty() {
            chips = super::nl::chips(s);
            items.extend(super::nl::items(s));
        }
    }
    #[cfg(feature = "semantic-search")]
    {
        if s.semantic_running {
            items.push(DockItem::text("Searching the semantic index for passages…", Tone::Dim));
        }
        if let Some(hits) = s.current_semantic_hits() {
            items.push(DockItem::Heading(format!(
                "Possible evidence for ‘{}’",
                plain(
                    s.semantic_request
                        .as_ref()
                        .map_or("", |key| key.query.as_str())
                )
            )));
            items.push(DockItem::text(
                "Extractive excerpts, not generated answers. Check each source in context.",
                Tone::Dim,
            ));
            items.push(DockItem::row(
                if s.refine.running {
                    "Refining…"
                } else if s.refined() {
                    "Refined: order uses a second model"
                } else {
                    "Refine results"
                },
                Tone::Title,
                Some(act(SearchAct::RefineResults)),
                false,
            ));
            for hit in s.evidence_hits(hits) {
                let i = hits
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, hit))
                    .unwrap_or(0);
                items.push(DockItem::row(
                    format!(
                        "{} · {:.0}% similar",
                        semantic_citation(hit),
                        hit.similarity * 100.0
                    ),
                    Tone::Normal,
                    Some(act(SearchAct::OpenSemantic(i))),
                    false,
                ));
                items.push(DockItem::text(plain(&hit.excerpt), Tone::Dim));
            }
            if hits.is_empty() {
                items.push(DockItem::text(
                    "No current indexed evidence found.",
                    Tone::Dim,
                ));
            }
        }
        if let Some(error) = &s.semantic_error {
            items.push(DockItem::text(plain(error), Tone::Warn));
        }
    }
    (!items.is_empty() || !chips.is_empty()).then(|| DocInsight {
        title: "Answers and passages".into(),
        chips,
        items,
    })
}

/// The file types read from the folder, to tick; shown as a modal while the picker is open.
fn types_card(s: &SearchSession) -> Option<Card> {
    let picker = &s.st.extension_picker;
    if !picker.open {
        return None;
    }
    let mut items = Vec::new();
    if let Some(err) = &picker.scan_error {
        items.push(DockItem::text(err.clone(), Tone::Warn));
    }
    items.push(DockItem::text(
        if picker.selected.is_empty() {
            "None ticked: every supported type is searched.".to_string()
        } else {
            format!("{} ticked.", picker.selected.len())
        },
        Tone::Dim,
    ));
    items.push(DockItem::row(
        "Search every type",
        Tone::Normal,
        Some(act(SearchAct::AllExtensions)),
        false,
    ));
    items.extend(picker.available.iter().enumerate().map(|(i, ext)| {
        let on = picker.selected.contains(ext);
        DockItem::field(
            ext.clone(),
            if on { "On" } else { "Off" },
            if on { Tone::On } else { Tone::Normal },
            Some(act(SearchAct::ToggleExtension(i))),
            false,
        )
    }));
    items.push(DockItem::row(
        "Done",
        Tone::Title,
        Some(act(SearchAct::CloseExtensions)),
        false,
    ));
    Some(Card::new(
        "search-types",
        "File types in this folder",
        Relevance::Primary,
        items,
    ))
}

/// Every card, for the chrome's property list (the page shows them in its popovers, not in a dock).
pub(super) fn inspector(s: &SearchSession) -> Vec<Card> {
    let c = cards(s);
    let mut all: Vec<Card> = types_card(s).into_iter().collect();
    all.extend([
        c.matching,
        c.files,
        c.performance,
        c.output,
        c.index,
        c.summary,
    ]);
    all
}

/// How to write a search: words, phrases, either / all / near, groups, exclusions, patterns. Each example fills Find.
pub(super) fn find_help() -> DocHelp {
    let row = |code: &str, text: &str, example: Option<&str>| HelpRow {
        code: code.into(),
        text: text.into(),
        example: example.map(str::to_string),
    };
    DocHelp {
        link: "How to write a search".into(),
        title: "Writing a search".into(),
        intro: "Upper and lower case do not matter. Click an example to put it in Find, then press Enter.".into(),
        sections: vec![
            HelpSection { title: "Words and phrases".into(), rows: vec![
                row("torque", "Lines that contain the word, also inside longer words (torqued). Turn on Whole word for the word alone.", Some("torque")),
                row("bolt torque", "A phrase: the words together, in this order, with the spaces between them.", Some("bolt torque")),
                row("bolt, torque", "Commas separate search terms. How they combine is the Match switch under the bar:", Some("bolt, torque")),
                row("Match: any line", "A file is listed when any line holds any of the terms.", None),
                row("Match: all in file", "Every term must appear somewhere in the file.", None),
                row("Match: within N lines", "Every term must appear within N lines of the others (set N in Options > Proximity lines).", None),
            ] },
            HelpSection { title: "Groups: several conditions at once".into(), rows: vec![
                row("bolt ; torque", "A semicolon starts another group. A file must satisfy every group.", Some("bolt ; torque")),
                row("fastener, rivet [any]", "A tag at the end of a group sets how that group matches: [any] any term on any line.", Some("fastener, rivet [any] ; torque")),
                row("hole, edge [all]", "[all]: every term of the group somewhere in the file.", Some("hole, edge [all]")),
                row("bolt, torque [near 3]", "[near N]: the terms within N lines of each other. [near] alone uses Proximity lines.", Some("bolt, torque [near 3]")),
                row("house ; floor, two [near 3]", "Groups mix: house anywhere, and floor and two within 3 lines.", Some("house ; floor, two [near 3]")),
            ] },
            HelpSection { title: "Leaving things out".into(), rows: vec![
                row("draft [not]", "Drops every file that contains any term of the group.", Some("torque ; draft, obsolete [not]")),
                row("todo [not line]", "Ignores only the lines that contain the term; the rest of the file still counts.", Some("torque ; todo [not line]")),
                row("Exclude switch", "Opens a row under Find for terms that always leave things out, without typing a group.", None),
            ] },
            HelpSection { title: "Patterns".into(), rows: vec![
                row("torque\\s+\\d+", "With Regex on, each term is a regular expression (here: torque, spaces, a number).", Some("torque\\s+\\d+")),
                row("HL\\d{2}", "Part numbers and codes: HL then two digits.", Some("HL\\d{2}")),
                row("Limits", "Commas and semicolons always split terms, and a trailing [tag] is always a tag: they cannot be escaped inside a pattern.", None),
            ] },
            HelpSection { title: "Where it looks".into(), rows: vec![
                row("Types", "The Types switch picks the file types read from the folder; none ticked searches every supported type (text, Office, PDF, zip, rtf and many code formats).", None),
                row("Hidden files", "Includes hidden files and folders.", None),
                row("Options", "Extra folders, excluded folders, size limit and time-outs.", None),
            ] },
        ],
    }
}

/// How the Exclude row works. Each example fills Exclude.
pub(super) fn exclude_help() -> DocHelp {
    let row = |code: &str, text: &str, example: Option<&str>| HelpRow {
        code: code.into(),
        text: text.into(),
        example: example.map(str::to_string),
    };
    DocHelp {
        link: "How exclusion works".into(),
        title: "Leaving things out".into(),
        intro: "Every term here applies to the whole search, whatever Find holds. Click an example to put it in Exclude.".into(),
        sections: vec![
            HelpSection { title: "Terms".into(), rows: vec![
                row("draft", "A word: also inside longer words (drafts), or alone with Whole word on.", Some("draft")),
                row("draft, obsolete", "Commas separate terms; any one of them excludes.", Some("draft, obsolete")),
                row("do not use", "A phrase: the words together, in this order.", Some("do not use")),
            ] },
            HelpSection { title: "What is left out".into(), rows: vec![
                row("Whole file", "A file that contains any term is dropped from the results.", None),
                row("Matching lines", "Only the lines with a term are ignored; the rest of the file can still match.", None),
            ] },
            HelpSection { title: "Patterns".into(), rows: vec![
                row("rev\\s*[A-C]", "With Regex on, each term is a regular expression (here: rev, then A, B or C).", Some("rev\\s*[A-C]")),
                row("Whole word", "The Whole word switch applies here as it does to Find.", None),
            ] },
            HelpSection { title: "Only for one group".into(), rows: vec![
                row("torque ; draft [not]", "To exclude inside one search only, add a [not] or [not line] group in Find instead (see How to write a search).", None),
            ] },
        ],
    }
}

pub(super) fn value_card(s: &SearchSession) -> Option<ValueCard> {
    let edit = s.edit.as_ref()?;
    let (title, label, hint) = match edit.setting {
        Some(i) => {
            let (_, _, label, hint, _) = settings_view::setting(i)?;
            (label.to_string(), label.to_string(), hint.to_string())
        }
        None => ("Save settings as preset".to_string(), "Name".to_string(), "Every setting of this search is kept under this name; the Presets folder of the tree applies it.".to_string()),
    };
    Some(ValueCard {
        title,
        width: 360.0,
        tag: String::new(),
        rows: vec![
            CardRow::Field {
                label,
                text: edit.text.clone(),
                index: 0,
                unit: String::new(),
            },
            CardRow::Text(hint),
        ],
        apply_label: "Apply".into(),
        apply: act(SearchAct::ApplyEdit),
        cancel: act(SearchAct::CancelEdit),
    })
}

// ---------------------------------------------------------------------- ribbon and menus

pub(super) fn ribbon(s: &SearchSession) -> Vec<RibbonTab> {
    let c = &s.st.config;
    let ix = &s.st.index_run;
    vec![
        RibbonTab::new("Search")
            .group("Run", vec![run_entry(s), stop_entry(s)])
            .group(
                "Folder",
                vec![
                    e("Browse...", "", SearchAct::BrowseFolder),
                    Entry::new("File types...", "", act(SearchAct::PickExtensions))
                        .checked(c.selected_extensions.is_some()),
                ],
            )
            .group("Match", match_mode_entries(s))
            .group(
                "Options",
                vec![
                    switch(s, "Regex", 6),
                    switch(s, "Whole word", 7),
                    switch(s, "Hidden files", 11),
                ],
            ),
        RibbonTab::new("Results")
            .group("File", result_entries(s))
            .group(
                "Sort",
                [
                    ("Found", SortKey::Found),
                    ("Name", SortKey::Name),
                    ("Matches", SortKey::Hits),
                    ("Size", SortKey::Size),
                    ("Modified", SortKey::Modified),
                ]
                .into_iter()
                .map(|(l, k)| Entry::new(l, "", act(SearchAct::SortBy(k))).checked(s.sort.0 == k))
                .collect(),
            )
            .group(
                "Report",
                vec![
                    report_entry(s),
                    when(
                        e("Show all", "", SearchAct::OnlyType(None)),
                        !s.filter.is_empty(),
                        "the results are not filtered",
                    ),
                ],
            ),
        RibbonTab::new("Index").group(
            "Fast index",
            vec![
                switch(s, "Use fast index", 29),
                when(
                    e("Build / update", "", SearchAct::BuildIndex),
                    !ix.is_building && !s.st.run.is_running && !c.search_path.trim().is_empty(),
                    if ix.is_building {
                        "the index is building"
                    } else {
                        "choose a folder first, and wait for a running search"
                    },
                ),
                when(
                    e("Stop build", "Esc", SearchAct::CancelIndex),
                    ix.is_building,
                    "no index is building",
                ),
                Entry::new(
                    format!("Location: {}", settings_view::setting_display(c, 30)),
                    "",
                    act(SearchAct::Step(30)),
                ),
            ],
        ),
        RibbonTab::new("Output")
            .group(
                "Write",
                vec![
                    switch(s, "HTML report", 16),
                    switch(s, "CSV", 17),
                    switch(s, "JSON", 18),
                ],
            )
            .group(
                "Where",
                vec![
                    e("Output folder...", "", SearchAct::BrowseOutput),
                    report_entry(s),
                ],
            ),
    ]
}

pub(super) fn menus(s: &SearchSession) -> Vec<Menu> {
    let item = Item::Entry;
    let mut file = vec![
        item(run_entry(s)),
        item(stop_entry(s)),
        Item::Separator,
        item(e("Choose a folder...", "", SearchAct::BrowseFolder)),
        item(report_entry(s)),
        Item::Separator,
        item(e("Save settings as preset...", "", SearchAct::SavePreset)),
    ];
    if !s.st.recent_searches.is_empty() {
        file.push(item(e(
            "Clear recent searches",
            "",
            SearchAct::ClearRecents,
        )));
    }
    file.push(Item::Separator);
    file.push(item(Entry::new("Close", "", UiAction::Quit)));
    let edit: Vec<Item> = result_entries(s).into_iter().map(Item::Entry).collect();
    let mut tools = vec![
        item(e("Choose file types...", "", SearchAct::PickExtensions)),
        Item::Separator,
    ];
    tools.extend(
        ribbon(s)
            .into_iter()
            .find(|t| t.title == "Index")
            .into_iter()
            .flat_map(|t| t.groups)
            .flat_map(|g| g.buttons)
            .map(Item::Entry),
    );
    #[cfg(feature = "neural-search")]
    {
        tools.push(Item::Separator);
        tools.push(Item::Heading(
            "Stores score factors and labels locally".into(),
        ));
        tools.push(item(
            Entry::new(
                if s.feedback_enabled() {
                    "Local relevance feedback"
                } else {
                    "Enable local relevance feedback"
                },
                "",
                act(SearchAct::ToggleRelevanceFeedback),
            )
            .checked(s.feedback_enabled()),
        ));
        tools.push(item(e(
            "Reset local feedback",
            "",
            SearchAct::ResetRelevanceFeedback,
        )));
        tools.push(item(e(
            "Export local feedback…",
            "",
            SearchAct::ExportRelevanceFeedback,
        )));
    }
    vec![
        Menu {
            title: "File".into(),
            items: file,
        },
        Menu {
            title: "Edit".into(),
            items: edit,
        },
        Menu {
            title: "Tools".into(),
            items: tools,
        },
    ]
}
