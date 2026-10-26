//! What the Search workbench shows: the page (search bar, progress, results table, matches), the tree, the
//! inspector's option cards, the ribbon, the menus and the value cards. Pure reads of the session.
use super::{act, file_name, SearchAct, SearchSession, SortKey, INPUT_EXCLUDE, INPUT_FILTER, INPUT_FIND, INPUT_FOLDER, INPUT_OUTPUT, SETTING_EXCLUDE, SETTING_EXCLUDE_SCOPE, SETTING_FILTERS, SETTING_OUTPUT, SETTING_PATH};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::{
        authoring::{Card, Relevance},
        session::Tone,
    },
    menu::{Entry, Item, Menu},
    ui::{CardRow, DocBlock, DocColumn, DocDetail, DocHelp, DocInput, DocModal, DocNotice, DocPanel, DocProgress, DocRow, DocTable, Document, HelpRow, HelpSection, DockItem, RibbonTab, TreeNode, ValueCard},
};
use crate::toolboxes::search::{indexing::format_eta, model, preview, run_view, settings_view};
use search_core::models::{ExcludeScope, MatchMode};

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

// ---------------------------------------------------------------------- availability

fn run_rule(s: &SearchSession) -> Result<(), String> {
    let c = &s.st.config;
    if s.st.run.is_running {
        Err("a search is running (Esc stops it)".into())
    } else if c.search_path.trim().is_empty() {
        Err("choose a folder to search first".into())
    } else if c.filters_text.trim().is_empty() {
        Err("type what to find first".into())
    } else if let Some(e) = model::regex_validation_error(c) {
        Err(format!("the pattern is not valid: {e}"))
    } else {
        Ok(())
    }
}
fn selected_rule(s: &SearchSession) -> Result<(), String> {
    if s.current_result().is_some() { Ok(()) } else { Err("select a result first".into()) }
}

fn run_entry(s: &SearchSession) -> Entry {
    e("Search", "Ctrl+Enter", SearchAct::Run).when(run_rule(s))
}
fn stop_entry(s: &SearchSession) -> Entry {
    when(e("Stop", "Esc", SearchAct::Cancel), s.st.run.is_running, "no search is running")
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
    when(e("Open report", "", SearchAct::OpenReport), s.st.run.last_report_path.is_some(), "no report yet: one is written when a search finishes with HTML export on")
}
fn match_mode_entries(s: &SearchSession) -> Vec<Entry> {
    let m = s.st.config.match_mode;
    vec![
        e("Any line", "", SearchAct::Match(MatchMode::AnyLine)).checked(m == MatchMode::AnyLine),
        e("All in file", "", SearchAct::Match(MatchMode::AllInFile)).checked(m == MatchMode::AllInFile),
        Entry::new(format!("Within {} lines", s.st.config.proximity_lines), "", act(SearchAct::Match(MatchMode::Proximity))).checked(m == MatchMode::Proximity),
    ]
}
fn switch(s: &SearchSession, label: &str, idx: usize) -> Entry {
    Entry::new(label, "", act(SearchAct::Step(idx))).checked(settings_view::setting_on(&s.st.config, idx))
}

// ---------------------------------------------------------------------- page

pub(super) fn document(s: &SearchSession) -> Document {
    let c = &s.st.config;
    let run = &s.st.run;
    let groups_note = plain(&run_view::filter_group_hint(c, false));
    let inputs = vec![
        DocInput {
            id: INPUT_FOLDER,
            label: "Folder".into(),
            text: c.search_path.clone(),
            hint: "Folder to search, e.g. C:\\Projects".into(),
            error: None,
            note: (!c.search_paths_extra.is_empty()).then(|| format!("Also searching {}", plural(c.search_paths_extra.len(), "more folder", "more folders"))),
            submit: Some(act(SearchAct::Run)),
            grow: 1.0,
            trailing: Some(e("Browse", "", SearchAct::BrowseFolder)),
            help: None,
            choices: Vec::new(),
        },
        DocInput {
            id: INPUT_FIND,
            label: "Find".into(),
            text: c.filters_text.clone(),
            hint: "Words or phrases, comma-separated; ; starts a group".into(),
            error: model::regex_validation_error(c),
            note: (!groups_note.is_empty()).then_some(groups_note),
            submit: Some(act(SearchAct::Run)),
            grow: 1.5,
            trailing: None,
            help: Some(find_help()),
            choices: Vec::new(),
        },
    ];
    let extra_inputs = if s.show_exclude { vec![exclude_input(s)] } else { Vec::new() };
    let buttons = if run.is_running { vec![stop_entry(s)] } else { vec![run_entry(s)] };
    let mut chips = match_mode_chips(s);
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
    chips.push(Entry::new(types, "", act(SearchAct::PickExtensions)).checked(c.selected_extensions.is_some()));

    let notice = if let Some((changed, removed)) = s.stale {
        let mut parts = Vec::new();
        if changed > 0 {
            parts.push(format!("{changed} new or changed"));
        }
        if removed > 0 {
            parts.push(format!("{removed} removed"));
        }
        Some(DocNotice { text: format!("The fast index is out of date ({} files). Update it so the next search is quick?", parts.join(", ")), tone: Tone::Warn, actions: vec![e("Update index", "", SearchAct::UpdateStaleIndex), e("Not now", "", SearchAct::DismissStale)] })
    } else if run.status_text.starts_with("Error") {
        Some(DocNotice { text: run.status_text.clone(), tone: Tone::Warn, actions: Vec::new() })
    } else {
        s.st.index_run.last_error.as_ref().filter(|_| !s.st.index_run.is_building).map(|err| DocNotice { text: format!("Fast index: {err}"), tone: Tone::Warn, actions: vec![e("Build again", "", SearchAct::BuildIndex)] })
    };

    let progress = if run.is_running {
        Some(run_progress(s))
    } else if s.st.index_run.is_building {
        let ix = &s.st.index_run;
        Some(DocProgress {
            fraction: (ix.percent > 0.0).then_some((ix.percent / 100.0) as f32),
            headline: format!("Fast index: {}", plain(&ix.status_text)),
            detail: "The index makes repeat searches of this folder near instant; the search itself stays exact.".into(),
            active: (!ix.current_file.is_empty()).then(|| (ix.current_file.clone(), String::new())).into_iter().collect(),
            cancel: Some(e("Stop", "Esc", SearchAct::CancelIndex)),
        })
    } else {
        None
    };

    let stats = if run.started.is_some() && !run.is_running {
        let hits: usize = run.results.iter().map(|r| r.hits.len()).sum();
        let mut v = vec![("Files with matches".to_string(), group(run.results.len())), ("Matching lines".to_string(), group(hits))];
        if let Some(d) = &run.diagnostics {
            v.push(("Files checked".to_string(), group(d.verified.max(0) as usize)));
        }
        v.push(("Time".to_string(), crate::format::format_elapsed(run.elapsed_secs().unwrap_or(0.0))));
        v
    } else {
        Vec::new()
    };

    let c = cards(s);
    let index_on = settings_view::setting_on(&s.st.config, 29);
    let panels = vec![
        DocPanel { label: "Options".into(), badge: None, inputs: Vec::new(), cards: vec![c.matching, c.files, c.performance] },
        DocPanel { label: "Index".into(), badge: Some(if s.st.index_run.is_building { "building".into() } else if index_on { "on".into() } else { "off".into() }), inputs: Vec::new(), cards: vec![c.index, c.summary] },
        DocPanel { label: "Report".into(), badge: s.st.run.last_report_path.as_ref().map(|_| "ready".to_string()), inputs: vec![output_input(s)], cards: vec![c.output] },
    ];
    let modal = types_card(s).map(|card| DocModal { card, close: act(SearchAct::CloseExtensions) });
    Document { inputs, extra_inputs, buttons, chips, notice, progress, stats, table: table(s), detail: detail(s), panels, modal }
}

/// The switch that shows the Exclude row. While the row is hidden and terms are still set, the switch names them: an
/// exclusion that applies is never out of sight.
fn exclude_chip(s: &SearchSession) -> Entry {
    let terms = model::parse_list(&s.st.config.exclude_filters_text);
    let label = if s.show_exclude || terms.is_empty() {
        "Exclude".to_string()
    } else {
        let shown = terms.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
        if terms.len() > 2 { format!("Exclude: {shown} +{}", terms.len() - 2) } else { format!("Exclude: {shown}") }
    };
    Entry::new(label, "", act(SearchAct::ToggleExclude)).checked(s.show_exclude || !terms.is_empty())
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
            ExcludeScope::Line => format!("Ignores the lines that contain {list}; the rest of each file still counts."),
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
        grow: 1.0,
        trailing: None,
        help: Some(exclude_help()),
        choices: vec![
            e("Whole file", "", SearchAct::ExcludeScope(ExcludeScope::File)).checked(scope == ExcludeScope::File),
            e("Matching lines", "", SearchAct::ExcludeScope(ExcludeScope::Line)).checked(scope == ExcludeScope::Line),
        ],
    }
}

/// Where reports and exports go: typed, or chosen with the system's folder picker, as the folder to search is.
fn output_input(s: &SearchSession) -> DocInput {
    let folder = s.st.config.output_folder.trim();
    DocInput {
        id: INPUT_OUTPUT,
        label: "Output folder".into(),
        text: s.st.config.output_folder.clone(),
        hint: "Folder for the report and exports".into(),
        error: None,
        note: Some(if folder.is_empty() { "Empty: the report goes to the folder the app was started from; Export matches writes beside each file.".into() } else { "The HTML / CSV / JSON report and Export matches are written here.".into() }),
        submit: None,
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
    vec![Entry::new(label, "", act(SearchAct::Step(4))).checked(s.st.config.match_mode != MatchMode::AnyLine)]
}

fn run_progress(s: &SearchSession) -> DocProgress {
    let run = &s.st.run;
    let elapsed = run.elapsed_secs().unwrap_or(0.0);
    let fraction = (run.total_files > 0).then(|| (run.files_completed as f32 / run.total_files as f32).clamp(0.0, 1.0));
    let mut detail = Vec::new();
    if run.total_files > 0 && elapsed >= 1.0 && run.files_completed > 0 {
        let rate = f64::from(run.files_completed) / elapsed;
        detail.push(format!("{rate:.1} files/s"));
        let left = f64::from((run.total_files - run.files_completed).max(0)) / rate.max(1e-9);
        detail.push(format!("about {} left", format_eta(left)));
    }
    detail.push(format!("{} elapsed", crate::format::format_elapsed(elapsed)));
    if !run.results.is_empty() {
        detail.push(format!("{} so far", plural(run.results.len(), "file with matches", "files with matches")));
    }
    let root = s.st.config.search_path.trim().trim_end_matches(['/', '\\']).to_string();
    DocProgress {
        fraction,
        headline: if run.total_files > 0 {
            let lines: usize = run.results.iter().map(|r| r.hits.len()).sum();
            format!("Searching  ·  {} of {}  ·  {}", group(run.files_completed.max(0) as usize), plural(run.total_files as usize, "file", "files"), plural(lines, "matching line", "matching lines"))
        } else if run.status_text.is_empty() {
            "Starting...".into()
        } else {
            plain(&run.status_text)
        },
        detail: detail.join("  ·  "),
        // Every file still being read, with how long it has taken (a slow PDF shows itself here).
        active: run.in_flight_files.iter().map(|f| {
            let shown = f.file_name.strip_prefix(&root).map(|r| r.trim_start_matches(['/', '\\']).to_string()).filter(|r| !r.is_empty()).unwrap_or_else(|| f.file_name.clone());
            (shown, format!("{} · {}", plain(&f.status_text), crate::format::format_elapsed(f.elapsed_seconds)))
        }).collect(),
        cancel: Some(stop_entry(s)),
    }
}

fn table(s: &SearchSession) -> DocTable {
    let run = &s.st.run;
    let view = s.view();
    let current = s.current();
    let col = |title: &str, width: f32, right: bool, key: SortKey| DocColumn { title: title.into(), width, right, sort: Some(act(SearchAct::SortBy(key))), sorted: (s.sort.0 == key).then_some(s.sort.1) };
    let root = s.st.config.search_path.trim().trim_end_matches(['/', '\\']).to_string();
    let rows = view
        .iter()
        .map(|&i| {
            let r = &run.results[i];
            let folder = std::path::Path::new(&r.full_name).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let rel = folder.strip_prefix(&root).map(|f| f.trim_start_matches(['/', '\\']).to_string());
            let sub = match rel {
                Some(f) if f.is_empty() => "(top folder)".to_string(),
                Some(f) => f,
                None => folder,
            };
            DocRow {
                cells: vec![file_name(&r.full_name).to_string(), group(r.hits.len()), crate::format::format_bytes(r.file_length), r.modified.format("%Y-%m-%d %H:%M").to_string()],
                sub,
                selected: current == Some(i),
                select: act(SearchAct::Select(i)),
                open: Some(act(SearchAct::Open(i))),
                badge: std::path::Path::new(&r.full_name).extension().map(|x| x.to_string_lossy().to_uppercase()),
            }
        })
        .collect();
    let total = run.results.len();
    let title = if total == 0 { "Results".to_string() } else if view.len() == total { format!("Results  ·  {}", plural(total, "file", "files")) } else { format!("Results  ·  {} of {}", group(view.len()), plural(total, "file", "files")) };
    let (empty_title, empty_body) = if total > 0 {
        ("Nothing matches the filter".to_string(), "Clear the filter to see every file with matches.".to_string())
    } else if run.is_running {
        ("Searching...".to_string(), "Files with matches appear here as soon as they are found.".to_string())
    } else if run.started.is_some() {
        ("No matches".to_string(), "Nothing in this folder matched. Try fewer words, another match mode, or more file types.".to_string())
    } else {
        ("Search a folder".to_string(), "Choose a folder, type what to find and press Enter. Matches appear here as they are found, with every matching line highlighted.".to_string())
    };
    DocTable {
        title,
        filter: (total > 0).then(|| DocInput { id: INPUT_FILTER, label: "Filter".into(), text: s.filter.clone(), hint: "Filter by name or folder".into(), error: None, note: None, submit: None, grow: 1.0, trailing: None, help: None, choices: Vec::new() }),
        columns: vec![col("File", 0.0, false, SortKey::Name), col("Matches", 76.0, true, SortKey::Hits), col("Size", 84.0, true, SortKey::Size), col("Modified", 138.0, false, SortKey::Modified)],
        rows,
        empty_title,
        empty_body,
    }
}

/// Matching lines shown for one file (a file can have thousands; the export has them all).
const MAX_BLOCKS: usize = 400;

fn detail(s: &SearchSession) -> Option<DocDetail> {
    let r = s.current_result()?;
    let mut blocks: Vec<DocBlock> = r.hits.iter().take(MAX_BLOCKS).map(|h| DocBlock { number: h.line_number, before: h.before.clone(), text: h.match_line.clone(), after: h.after.clone(), marks: preview::highlight_ranges(&h.match_line, &h.matched_filters), tags: h.matched_filters.clone() }).collect();
    let mut subtitle = format!("{}  ·  {}", r.full_name, plain(&preview::metadata_summary(r)));
    if r.low_confidence_pdf {
        subtitle.push_str("  ·  scanned PDF: text may be incomplete");
    }
    if r.hits.len() > MAX_BLOCKS {
        subtitle.push_str(&format!("  ·  first {MAX_BLOCKS} lines shown (Export matches has all)"));
    }
    blocks.shrink_to_fit();
    Some(DocDetail { title: file_name(&r.full_name).to_string(), subtitle, actions: result_entries(s), blocks, empty: "The file matched as a whole (all-in-file mode), with no single line to show.".into() })
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
    t.push(TreeNode::new(10, 1, 'o', if folder.is_empty() { "Folder: not chosen".to_string() } else { format!("Folder: {}", file_name(folder)) }).tone(if folder.is_empty() { Tone::Warn } else { Tone::Normal }).on_click(act(SearchAct::BrowseFolder)));
    let find = c.filters_text.trim();
    t.push(TreeNode::new(11, 1, 'o', if find.is_empty() { "Find: nothing yet".to_string() } else { format!("Find: {find}") }).tone(if find.is_empty() { Tone::Warn } else { Tone::Normal }).on_click(act(SearchAct::StepTab("Search"))));
    let types = s.types();
    t.push(TreeNode::new(NODE_TYPES, 0, '+', format!("Results ({})", group(s.st.run.results.len()))).folder());
    if !types.is_empty() {
        t.push(TreeNode::new(NODE_TYPE0 - 1, 1, 'R', "All types").selected(s.filter.is_empty()).on_click(act(SearchAct::OnlyType(None))));
    }
    for (k, (ext, n)) in types.iter().enumerate() {
        t.push(TreeNode::new(NODE_TYPE0 + k as u32, 1, 'f', format!("{ext}  {n}")).selected(s.filter.eq_ignore_ascii_case(ext)).on_click(act(SearchAct::OnlyType(Some(k)))));
    }
    if !s.st.recent_searches.is_empty() {
        t.push(TreeNode::new(NODE_RECENT, 0, '+', "Recent searches").folder());
        for (k, r) in s.st.recent_searches.iter().enumerate() {
            t.push(TreeNode::new(NODE_RECENT0 + k as u32, 1, '#', r.label()).on_click(act(SearchAct::ApplyRecent(k))));
        }
    }
    if !s.st.saved_presets.is_empty() {
        t.push(TreeNode::new(NODE_PRESETS, 0, '+', "Presets").folder());
        for (k, p) in s.st.saved_presets.iter().enumerate() {
            t.push(TreeNode::new(NODE_PRESET0 + k as u32, 1, 'M', p.name.clone()).on_click(act(SearchAct::ApplyPreset(k))));
        }
    }
    t
}

pub(super) fn tree_menu(s: &SearchSession, node: u32) -> Vec<Item> {
    let item = |x: Entry| Item::Entry(x);
    match node {
        n if (NODE_PRESET0..NODE_PRESET0 + 1_000).contains(&n) => {
            let k = (n - NODE_PRESET0) as usize;
            vec![item(e("Apply preset", "", SearchAct::ApplyPreset(k))), item(e("Delete preset", "", SearchAct::DeletePreset(k)))]
        }
        n if (NODE_RECENT0..NODE_PRESET0).contains(&n) => vec![item(e("Load this search", "", SearchAct::ApplyRecent((n - NODE_RECENT0) as usize))), Item::Separator, item(e("Clear recent searches", "", SearchAct::ClearRecents))],
        NODE_PRESETS => vec![item(e("Save settings as preset...", "", SearchAct::SavePreset))],
        NODE_RECENT => vec![item(e("Clear recent searches", "", SearchAct::ClearRecents))],
        10 => vec![item(e("Choose a folder...", "", SearchAct::BrowseFolder))],
        _ if s.current_result().is_some() => result_entries(s).into_iter().map(Item::Entry).collect(),
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
        .filter(|(i, g, ..)| groups.contains(g) && ![SETTING_PATH, SETTING_FILTERS, SETTING_EXCLUDE, SETTING_EXCLUDE_SCOPE, SETTING_OUTPUT].contains(i))
        .map(|(i, _, label, _, kind)| {
            let value = settings_view::setting_display(&s.st.config, i);
            let tone = match kind {
                settings_view::SettingKind::Switch if settings_view::setting_on(&s.st.config, i) => Tone::On,
                _ => Tone::Normal,
            };
            let value = if value.trim().is_empty() { "-".to_string() } else { value };
            DockItem::field(label, value, tone, Some(act(SearchAct::Edit(i))), s.edit.as_ref().and_then(|e| e.setting) == Some(i))
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
        index.push(DockItem::row("Stop the build", Tone::Warn, Some(act(SearchAct::CancelIndex)), false));
    } else if !s.st.run.is_running && !s.st.config.search_path.trim().is_empty() {
        index.push(DockItem::row(if ix.docs.is_some() { "Update the index now" } else { "Build the index now" }, Tone::Title, Some(act(SearchAct::BuildIndex)), false));
    }
    if !ix.status_text.is_empty() {
        index.push(DockItem::text(plain(&ix.status_text), Tone::Dim));
    }
    if let Some(n) = ix.docs {
        index.push(DockItem::field("Documents", group(n as usize), Tone::Normal, None, false));
    }
    if let Some(n) = &ix.last_narrow {
        index.push(DockItem::text(plain(n), Tone::Dim));
    }
    index.push(DockItem::text("The index only narrows which files are read; every match is still checked in the file itself.", Tone::Dim));
    let mut summary = Vec::new();
    if let Some(d) = &s.st.run.diagnostics {
        summary.extend(d.lines(ix.last_build_failed).into_iter().map(|l| DockItem::text(plain(&l), Tone::Dim)));
    }
    if !s.st.run.results_summary_text.is_empty() {
        summary.push(DockItem::text(s.st.run.results_summary_text.clone(), Tone::Dim));
    }
    let mut output = setting_rows(s, &["Output"]);
    if let Some(p) = &s.st.run.last_report_path {
        output.push(DockItem::row(format!("Open report: {}", file_name(p)), Tone::Title, Some(act(SearchAct::OpenReport)), false));
    }
    Cards {
        matching: Card::new("search-matching", "Matching", Relevance::Context, setting_rows(s, &["Filters"])).for_steps(&["Search"]),
        files: Card::new("search-files", "Files and folders", Relevance::Context, setting_rows(s, &["Search", "Extensions & Exclusions"])).for_steps(&["Search"]),
        performance: Card::new("search-performance", "Performance", Relevance::Info, setting_rows(s, &["Concurrency & Diagnostics"])).for_steps(&["Search"]),
        output: Card::new("search-output", "Report and exports", Relevance::Context, output).for_steps(&["Output"]),
        index: Card::new("search-index", "Fast index", Relevance::Primary, index).for_steps(&["Index"]),
        summary: Card::new("search-summary", "Last run", Relevance::Info, summary).for_steps(&["Index"]),
    }
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
    items.push(DockItem::text(if picker.selected.is_empty() { "None ticked: every supported type is searched.".to_string() } else { format!("{} ticked.", picker.selected.len()) }, Tone::Dim));
    items.push(DockItem::row("Search every type", Tone::Normal, Some(act(SearchAct::AllExtensions)), false));
    items.extend(picker.available.iter().enumerate().map(|(i, ext)| {
        let on = picker.selected.contains(ext);
        DockItem::field(ext.clone(), if on { "On" } else { "Off" }, if on { Tone::On } else { Tone::Normal }, Some(act(SearchAct::ToggleExtension(i))), false)
    }));
    items.push(DockItem::row("Done", Tone::Title, Some(act(SearchAct::CloseExtensions)), false));
    Some(Card::new("search-types", "File types in this folder", Relevance::Primary, items))
}

/// Every card, for the chrome's property list (the page shows them in its popovers, not in a dock).
pub(super) fn inspector(s: &SearchSession) -> Vec<Card> {
    let c = cards(s);
    let mut all: Vec<Card> = types_card(s).into_iter().collect();
    all.extend([c.matching, c.files, c.performance, c.output, c.index, c.summary]);
    all
}

/// How to write a search: words, phrases, either / all / near, groups, exclusions, patterns. Each example fills Find.
pub(super) fn find_help() -> DocHelp {
    let row = |code: &str, text: &str, example: Option<&str>| HelpRow { code: code.into(), text: text.into(), example: example.map(str::to_string) };
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
    let row = |code: &str, text: &str, example: Option<&str>| HelpRow { code: code.into(), text: text.into(), example: example.map(str::to_string) };
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
            let (_, _, label, hint, _) = settings_view::settings().nth(i)?;
            (label.to_string(), label.to_string(), hint.to_string())
        }
        None => ("Save settings as preset".to_string(), "Name".to_string(), "Every setting of this search is kept under this name; the Presets folder of the tree applies it.".to_string()),
    };
    Some(ValueCard { title, width: 360.0, tag: String::new(), rows: vec![CardRow::Field { label, text: edit.text.clone(), index: 0, unit: String::new() }, CardRow::Text(hint)], apply_label: "Apply".into(), apply: act(SearchAct::ApplyEdit), cancel: act(SearchAct::CancelEdit) })
}

// ---------------------------------------------------------------------- ribbon and menus

pub(super) fn ribbon(s: &SearchSession) -> Vec<RibbonTab> {
    let c = &s.st.config;
    let ix = &s.st.index_run;
    vec![
        RibbonTab::new("Search")
            .group("Run", vec![run_entry(s), stop_entry(s)])
            .group("Folder", vec![e("Browse...", "", SearchAct::BrowseFolder), Entry::new("File types...", "", act(SearchAct::PickExtensions)).checked(c.selected_extensions.is_some())])
            .group("Match", match_mode_entries(s))
            .group("Options", vec![switch(s, "Regex", 6), switch(s, "Whole word", 7), switch(s, "Hidden files", 11)]),
        RibbonTab::new("Results")
            .group("File", result_entries(s))
            .group("Sort", [("Found", SortKey::Found), ("Name", SortKey::Name), ("Matches", SortKey::Hits), ("Size", SortKey::Size), ("Modified", SortKey::Modified)].into_iter().map(|(l, k)| Entry::new(l, "", act(SearchAct::SortBy(k))).checked(s.sort.0 == k)).collect())
            .group("Report", vec![report_entry(s), when(e("Show all", "", SearchAct::OnlyType(None)), !s.filter.is_empty(), "the results are not filtered")]),
        RibbonTab::new("Index").group("Fast index", vec![
            switch(s, "Use fast index", 29),
            when(e("Build / update", "", SearchAct::BuildIndex), !ix.is_building && !s.st.run.is_running && !c.search_path.trim().is_empty(), if ix.is_building { "the index is building" } else { "choose a folder first, and wait for a running search" }),
            when(e("Stop build", "Esc", SearchAct::CancelIndex), ix.is_building, "no index is building"),
            Entry::new(format!("Location: {}", settings_view::setting_display(c, 30)), "", act(SearchAct::Step(30))),
        ]),
        RibbonTab::new("Output").group("Write", vec![switch(s, "HTML report", 16), switch(s, "CSV", 17), switch(s, "JSON", 18), switch(s, "Open when done", 19)]).group("Where", vec![e("Output folder...", "", SearchAct::BrowseOutput), report_entry(s)]),
    ]
}

pub(super) fn menus(s: &SearchSession) -> Vec<Menu> {
    let item = Item::Entry;
    let mut file = vec![item(run_entry(s)), item(stop_entry(s)), Item::Separator, item(e("Choose a folder...", "", SearchAct::BrowseFolder)), item(report_entry(s)), Item::Separator, item(e("Save settings as preset...", "", SearchAct::SavePreset))];
    if !s.st.recent_searches.is_empty() {
        file.push(item(e("Clear recent searches", "", SearchAct::ClearRecents)));
    }
    file.push(Item::Separator);
    file.push(item(Entry::new("Close", "", UiAction::Quit)));
    let edit: Vec<Item> = result_entries(s).into_iter().map(Item::Entry).collect();
    let mut tools = vec![item(e("Choose file types...", "", SearchAct::PickExtensions)), Item::Separator];
    tools.extend(ribbon(s).into_iter().find(|t| t.title == "Index").into_iter().flat_map(|t| t.groups).flat_map(|g| g.buttons).map(Item::Entry));
    vec![Menu { title: "File".into(), items: file }, Menu { title: "Edit".into(), items: edit }, Menu { title: "Tools".into(), items: tools }]
}
