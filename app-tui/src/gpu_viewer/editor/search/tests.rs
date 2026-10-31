use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use search_core::models::{FileSearchStatus, LineHit};
use std::time::Duration;

fn hit(n: i32, line: &str, filter: &str) -> LineHit {
    LineHit { line_number: n, before: Some("above".into()), match_line: line.into(), after: None, matched_filters: vec![filter.into()] }
}
fn result(name: &str, hits: usize, size: i64) -> FileSearchResult {
    FileSearchResult {
        full_name: name.into(),
        status: FileSearchStatus::Hit,
        hits: (0..hits).map(|k| hit(k as i32 + 1, "a bolt torque line", "bolt")).collect(),
        created: chrono::Local::now(),
        modified: chrono::Local::now(),
        file_length: size,
        lines_cache: Vec::new(),
        total_line_count: 0,
        proximity_min_range: None,
        low_confidence_pdf: false,
        error_message: None,
    }
}
fn with_results() -> SearchSession {
    let mut s = SearchSession::in_memory();
    s.st.config.search_path = "/w".into();
    s.st.run.results = vec![result("/w/a/spec.pdf", 3, 900), result("/w/notes.txt", 1, 50), result("/w/b/plan.pdf", 7, 10)];
    s.st.run.started = Some(Instant::now());
    s
}
fn act_on(s: &mut SearchSession, a: SearchAct) {
    s.act(EditorAction::Search(a));
}
/// Poll until the run (and its report) finished, as the window does between frames.
fn wait(s: &mut SearchSession) {
    let t = Instant::now();
    while s.st.run.is_running || s.st.run.last_report_path.is_none() && s.st.config.export_html && t.elapsed() < Duration::from_secs(5) {
        assert!(t.elapsed() < Duration::from_secs(30), "the search did not finish");
        s.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    s.poll();
}

#[test]
fn a_search_runs_on_the_session_runtime_and_its_matches_fill_the_table_and_the_detail() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hit.txt"), "first line\nthe bolt torque is 40\nlast line\n").unwrap();
    std::fs::write(dir.path().join("miss.txt"), "nothing to see\n").unwrap();
    let mut s = SearchSession::in_memory();
    s.st.config.output_folder = dir.path().join("out").to_string_lossy().into_owned();
    s.st.config.export_html = true;
    s.document_field(INPUT_FOLDER, dir.path().to_string_lossy().into_owned());
    s.document_field(INPUT_FIND, "bolt".into());
    act_on(&mut s, SearchAct::Run);
    assert!(s.solving(), "the run is in flight right after Run");
    let live = s.document().unwrap();
    assert!(live.progress.is_some() && live.buttons[0].label == "Stop", "a running search shows its progress and Stop");
    wait(&mut s);
    assert!(!s.solving());
    let doc = s.document().unwrap();
    assert_eq!(doc.table.rows.len(), 1, "only the file with a match is listed");
    assert_eq!(doc.table.rows[0].cells[0], "hit.txt");
    let d = doc.detail.expect("the first result is shown");
    assert_eq!(d.blocks.len(), 1);
    assert_eq!(d.blocks[0].number, 2);
    let (a, b) = d.blocks[0].marks[0];
    assert_eq!(&d.blocks[0].text[a..b], "bolt", "the match is highlighted");
    assert!(doc.stats.iter().any(|(l, v)| l == "Files with matches" && v == "1"), "{:?}", doc.stats);
    assert!(s.status().contains("1 file with matches"), "{}", s.status());
    assert_eq!(s.st.recent_searches.len(), 1, "the run is remembered like in the terminal");
    assert_eq!(s.step_state("Results"), Some(StepState::Done));
    let report = PathBuf::from(s.st.run.last_report_path.as_deref().expect("the report is written"));
    assert_eq!(report.parent(), Some(dir.path().join("out").as_path()), "the report goes to the output folder");
}

#[test]
fn a_test_session_writes_no_report_unless_a_test_asks_for_one() {
    // An empty output folder means the working folder: a test run would leave `search-report.html` in the crate and
    // fail the qualification's unchanged-source check.
    let s = SearchSession::in_memory();
    assert!(!s.st.config.export_html, "the HTML report is off in a test session");
}

#[test]
fn a_search_that_cannot_run_says_why_and_starts_nothing() {
    let mut s = SearchSession::in_memory();
    act_on(&mut s, SearchAct::Run);
    assert!(!s.solving());
    assert!(s.status().contains("search path") || s.status().contains("folder"), "{}", s.status());
    let search = s.ribbon().remove(0).groups.remove(0).buttons.remove(0);
    assert!(!search.enabled && search.reason.as_deref().unwrap().contains("folder"));
    s.st.config.search_path = "/w".into();
    s.st.config.filters_text = "(".into();
    s.st.config.use_regex = true;
    act_on(&mut s, SearchAct::Run);
    assert!(!s.solving());
    assert!(s.status().starts_with("Fix the pattern"), "{}", s.status());
    assert!(s.document().unwrap().inputs[1].error.is_some(), "the Find input shows the pattern error");
}

#[test]
fn the_table_filters_sorts_and_follows_the_keyboard() {
    let mut s = with_results();
    assert_eq!(s.view(), [0, 1, 2], "found order by default");
    act_on(&mut s, SearchAct::SortBy(SortKey::Hits));
    assert_eq!(s.view(), [2, 0, 1], "most matches first");
    act_on(&mut s, SearchAct::SortBy(SortKey::Hits));
    assert_eq!(s.view(), [1, 0, 2], "a second click reverses");
    s.document_field(INPUT_FILTER, "PDF".into());
    assert_eq!(s.view(), [0, 2], "the filter is case-insensitive over the path");
    assert_eq!(s.document().unwrap().table.title, "Results  ·  2 of 3 files");
    assert_eq!(s.current(), Some(0), "the first shown row is current without a click");
    s.key(Key::Down, Mods::default());
    assert_eq!(s.current(), Some(2));
    s.key(Key::Down, Mods::default());
    assert_eq!(s.current(), Some(2), "the selection stops at the last row");
    // The tree's file types filter the table.
    let types = s.types();
    assert_eq!(types[0], (".pdf".to_string(), 2));
    act_on(&mut s, SearchAct::OnlyType(None));
    assert_eq!(s.view().len(), 3);
    act_on(&mut s, SearchAct::OnlyType(Some(1)));
    assert_eq!(s.view(), [1]);
    assert!(s.tree().iter().any(|n| n.label == ".txt  1" && n.selected));
}

#[test]
fn rows_show_the_name_the_folder_below_the_root_and_the_type() {
    let s = with_results();
    let rows = s.document().unwrap().table.rows;
    assert_eq!(rows[0].cells[..3], ["spec.pdf", "3", "900 B"]);
    assert_eq!(rows[0].sub, "a");
    assert_eq!(rows[1].sub, "(top folder)");
    assert_eq!(rows[0].badge.as_deref(), Some("PDF"));
}

#[test]
fn options_are_the_terminal_forms_settings_typed_flipped_and_validated() {
    let mut s = SearchSession::in_memory();
    let size = settings_view::settings().position(|x| x.2 == "Max file size (MB)").unwrap();
    act_on(&mut s, SearchAct::Edit(size));
    let card = s.value_card().expect("a typed setting opens a value card");
    assert_eq!(card.title, "Max file size (MB)");
    s.card_field(0, "12.5".into());
    act_on(&mut s, SearchAct::ApplyEdit);
    assert_eq!(s.st.config.max_file_size_mb, 12.5);
    act_on(&mut s, SearchAct::Edit(size));
    s.card_field(0, "lots".into());
    act_on(&mut s, SearchAct::ApplyEdit);
    assert_eq!(s.st.config.max_file_size_mb, 12.5, "an invalid number keeps the value");
    assert!(s.status().contains("not a valid value"), "{}", s.status());
    let regex = settings_view::settings().position(|x| x.2 == "Use regex").unwrap();
    act_on(&mut s, SearchAct::Edit(regex));
    assert!(s.st.config.use_regex && s.value_card().is_none(), "a switch flips without a card");
    act_on(&mut s, SearchAct::Match(MatchModeArg::Proximity));
    assert_eq!(s.st.config.match_mode, MatchModeArg::Proximity);
    // The bar's inputs are not repeated as inspector rows.
    let rows: Vec<String> = s.inspector().into_iter().flat_map(|c| c.items).filter_map(|i| i.plain_text()).collect();
    assert!(!rows.iter().any(|r| r.starts_with("Search path") || r.starts_with("Filters ")), "{rows:?}");
    assert!(rows.iter().any(|r| r.starts_with("Whole word")));
}

use search_core::models::MatchMode as MatchModeArg;

#[test]
fn presets_save_apply_and_delete() {
    let mut s = SearchSession::in_memory();
    s.st.config.filters_text = "bolt".into();
    act_on(&mut s, SearchAct::SavePreset);
    s.card_field(0, "Bolts".into());
    act_on(&mut s, SearchAct::ApplyEdit);
    assert_eq!(s.st.saved_presets.len(), 1);
    s.st.config.filters_text = "other".into();
    act_on(&mut s, SearchAct::ApplyPreset(0));
    assert_eq!(s.st.config.filters_text, "bolt");
    assert!(s.tree().iter().any(|n| n.label == "Bolts"));
    act_on(&mut s, SearchAct::DeletePreset(0));
    assert!(s.st.saved_presets.is_empty());
}

#[test]
fn browse_uses_the_system_folder_picker_and_a_cancel_changes_nothing() {
    let mut s = SearchSession::in_memory();
    s.st.config.search_path = "/keep".into();
    act_on(&mut s, SearchAct::BrowseFolder);
    assert_eq!(s.st.config.search_path, "/keep", "a cancelled dialog keeps the folder");
    assert_eq!(s.status(), "No folder chosen.");
    s.set_folder_picker(|_| Some(PathBuf::from("/picked/Drawings")));
    act_on(&mut s, SearchAct::BrowseFolder);
    assert_eq!(s.st.config.search_path, PathBuf::from("/picked/Drawings").to_string_lossy());
    assert!(s.document().unwrap().inputs[0].trailing.as_ref().is_some_and(|b| b.label == "Browse"));
}

#[test]
fn the_output_folder_is_an_input_with_browse_in_the_report_popover() {
    let mut s = SearchSession::in_memory();
    let doc = s.document().unwrap();
    let report = doc.panels.iter().find(|p| p.label == "Report").unwrap();
    let input = &report.inputs[0];
    assert_eq!((input.id, input.label.as_str()), (INPUT_OUTPUT, "Output folder"));
    assert_eq!(input.trailing.as_ref().map(|b| b.action), Some(act(SearchAct::BrowseOutput)));
    let rows: Vec<String> = report.cards.iter().flat_map(|c| c.items.iter()).filter_map(|i| i.plain_text()).collect();
    assert!(!rows.iter().any(|r| r.starts_with("Output folder")), "not repeated as a row: {rows:?}");
    // Typed, as the folder to search is.
    s.document_field(INPUT_OUTPUT, "/typed/out".into());
    assert_eq!(s.st.config.output_folder, "/typed/out");
    // The picker starts in the folder being searched when the output folder is not one; a cancel keeps the value.
    let dir = tempfile::tempdir().unwrap();
    s.st.config.search_path = dir.path().to_string_lossy().into_owned();
    act_on(&mut s, SearchAct::BrowseOutput);
    assert_eq!(s.st.config.output_folder, "/typed/out");
    s.set_folder_picker(|start| Some(start.join("reports")));
    act_on(&mut s, SearchAct::BrowseOutput);
    assert_eq!(PathBuf::from(&s.st.config.output_folder), dir.path().join("reports"));
    assert!(s.st.config.search_path == dir.path().to_string_lossy(), "the folder to search is untouched");
    // The ribbon and a typed-setting edit of the output folder open the picker too, never a value card.
    let out = settings_view::settings().position(|x| x.2 == "Output folder").unwrap();
    act_on(&mut s, SearchAct::Edit(out));
    assert!(s.value_card().is_none());
}

#[test]
fn every_example_of_the_find_guide_parses_the_way_the_guide_says() {
    use crate::toolboxes::search::model::{parse_filter_groups, GroupMode};
    use search_core::models::ExcludeScope;
    let help = view::find_help();
    let examples: Vec<(String, String)> = help.sections.iter().flat_map(|s| s.rows.iter().filter_map(|r| r.example.clone().map(|e| (r.code.clone(), e)))).collect();
    assert!(examples.len() >= 10, "the guide offers examples to try");
    for (code, ex) in &examples {
        let groups = parse_filter_groups(ex);
        assert!(!groups.is_empty() && groups.iter().all(|g| !g.filters.is_empty()), "{code}: {ex}");
        assert!(groups.iter().any(|g| !matches!(g.mode, Some(GroupMode::Exclude(_)))), "{code}: an example must search for something");
        let mut c = model::SearchToolConfig { filters_text: ex.clone(), ..Default::default() };
        c.use_regex = code.contains('\\');
        assert_eq!(model::regex_validation_error(&c), None, "{code}: {ex}");
    }
    let mode = |ex: &str, k: usize| parse_filter_groups(ex)[k].mode;
    assert_eq!(mode("bolt, torque [near 3]", 0), Some(GroupMode::Match(MatchModeArg::Proximity, Some(3))));
    assert_eq!(mode("hole, edge [all]", 0), Some(GroupMode::Match(MatchModeArg::AllInFile, None)));
    assert_eq!(mode("torque ; draft, obsolete [not]", 1), Some(GroupMode::Exclude(ExcludeScope::File)));
    assert_eq!(mode("torque ; todo [not line]", 1), Some(GroupMode::Exclude(ExcludeScope::Line)));
    assert_eq!(parse_filter_groups("bolt torque")[0].filters, ["bolt torque"], "a phrase is one term");
    assert_eq!(parse_filter_groups("bolt, torque")[0].filters, ["bolt", "torque"]);
    let mut c = model::SearchToolConfig { filters_text: "torque\\s+\\d+".into(), use_regex: true, ..Default::default() };
    assert_eq!(model::regex_validation_error(&c), None);
    c.filters_text = "torque(".into();
    assert!(model::regex_validation_error(&c).is_some(), "the check the guide relies on catches a broken pattern");
}

#[test]
fn the_page_puts_its_options_in_bar_popovers_and_the_file_types_in_a_modal() {
    let mut s = SearchSession::in_memory();
    let doc = s.document().unwrap();
    let labels: Vec<&str> = doc.panels.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["Options", "Index", "Report"]);
    assert_eq!(doc.panels[1].badge.as_deref(), Some("off"));
    let options: Vec<String> = doc.panels[0].cards.iter().flat_map(|c| c.items.iter()).filter_map(|i| i.plain_text()).collect();
    assert!(options.iter().any(|r| r.starts_with("Use regex")) && options.iter().any(|r| r.starts_with("Exclude folders")), "{options:?}");
    assert!(doc.inputs[1].help.is_some(), "Find has its guide");
    assert!(doc.modal.is_none());
    s.st.extension_picker = crate::toolboxes::search::extension_picker::ExtensionPicker::open_with(vec![".pdf".into()], None);
    let modal = s.document().unwrap().modal.expect("an open picker is a modal");
    assert_eq!(modal.close, act(SearchAct::CloseExtensions));
    act_on(&mut s, SearchAct::CloseExtensions);
    assert!(s.document().unwrap().modal.is_none());
}

#[test]
fn the_extension_picker_reads_the_folder_and_its_choice_filters_the_run() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "x").unwrap();
    std::fs::write(dir.path().join("b.md"), "x").unwrap();
    let mut s = SearchSession::in_memory();
    s.st.config.search_path = dir.path().to_string_lossy().into_owned();
    act_on(&mut s, SearchAct::PickExtensions);
    let t = Instant::now();
    while !s.st.extension_picker.open {
        assert!(t.elapsed() < Duration::from_secs(10), "the scan did not come back");
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    let md = s.st.extension_picker.available.iter().position(|e| e == ".md").unwrap();
    act_on(&mut s, SearchAct::ToggleExtension(md));
    act_on(&mut s, SearchAct::CloseExtensions);
    assert_eq!(s.st.config.selected_extensions, Some(vec![".md".to_string()]));
    assert!(s.document().unwrap().chips.iter().any(|c| c.label == "Types: .md"));
}

#[test]
fn a_page_bench_has_no_view_tab_or_menu_and_no_benches() {
    let s = SearchSession::in_memory();
    assert!(s.has_document() && s.benches().is_empty());
    let titles: Vec<String> = s.ribbon().into_iter().map(|t| t.title).collect();
    assert_eq!(titles, ["Search", "Results", "Index", "Output"]);
}

#[test]
fn exclude_is_a_row_of_the_page_shown_by_its_switch_and_never_hidden_while_it_applies() {
    use search_core::models::ExcludeScope;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "bolt draft\nbolt ok\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "bolt\n").unwrap();
    let mut s = SearchSession::in_memory();
    s.st.config.export_html = false;
    let doc = s.document().unwrap();
    assert!(doc.extra_inputs.is_empty(), "hidden until switched on");
    let chip = doc.chips.iter().find(|c| c.label == "Exclude").expect("the Exclude switch is under the bar");
    assert_eq!(chip.checked, Some(false));
    let options: Vec<String> = doc.panels[0].cards.iter().flat_map(|c| c.items.iter()).filter_map(|i| i.plain_text()).collect();
    assert!(!options.iter().any(|r| r.starts_with("Exclude filters") || r.starts_with("Exclude scope")), "not behind Options: {options:?}");
    act_on(&mut s, SearchAct::ToggleExclude);
    let row = s.document().unwrap().extra_inputs.remove(0);
    assert_eq!(row.id, INPUT_EXCLUDE);
    assert!(row.help.is_some(), "Exclude has its own guide");
    let labels: Vec<(&str, Option<bool>)> = row.choices.iter().map(|c| (c.label.as_str(), c.checked)).collect();
    assert_eq!(labels, [("Whole file", Some(false)), ("Matching lines", Some(true))]);
    s.document_field(INPUT_EXCLUDE, "draft".into());
    assert!(s.document().unwrap().extra_inputs[0].note.as_deref().unwrap().starts_with("Ignores the lines"));
    // Lines only: a.txt still matches on its other line.
    s.document_field(INPUT_FOLDER, dir.path().to_string_lossy().into_owned());
    s.document_field(INPUT_FIND, "bolt".into());
    act_on(&mut s, SearchAct::Run);
    wait(&mut s);
    let mut names: Vec<&str> = s.st.run.results.iter().map(|r| file_name(&r.full_name)).collect();
    names.sort();
    assert_eq!(names, ["a.txt", "b.txt"]);
    assert!(s.st.run.results.iter().find(|r| r.full_name.ends_with("a.txt")).unwrap().hits.iter().all(|h| !h.match_line.contains("draft")));
    // Whole file: a.txt is left out.
    act_on(&mut s, SearchAct::ExcludeScope(ExcludeScope::File));
    assert_eq!(s.st.config.exclude_scope, ExcludeScope::File);
    assert!(s.document().unwrap().extra_inputs[0].note.as_deref().unwrap().starts_with("Leaves out every file"));
    act_on(&mut s, SearchAct::Run);
    wait(&mut s);
    let names: Vec<&str> = s.st.run.results.iter().map(|r| file_name(&r.full_name)).collect();
    assert_eq!(names, ["b.txt"]);
    // Hidden while it still applies: the switch names what is left out.
    act_on(&mut s, SearchAct::ToggleExclude);
    let doc = s.document().unwrap();
    assert!(doc.extra_inputs.is_empty());
    let chip = doc.chips.iter().find(|c| c.label.starts_with("Exclude")).unwrap();
    assert_eq!((chip.label.as_str(), chip.checked), ("Exclude: draft", Some(true)));
    assert!(s.status().contains("still leaving out draft"), "{}", s.status());
    // A session whose settings already exclude something opens with the row shown.
    let mut st = SearchToolState::default();
    st.config.exclude_filters_text = "obsolete".into();
    let s = SearchSession::with_state(st, false).unwrap();
    assert_eq!(s.document().unwrap().extra_inputs.len(), 1);
}

#[test]
fn every_example_of_the_exclude_guide_is_a_valid_exclusion() {
    let help = view::exclude_help();
    let examples: Vec<(String, String)> = help.sections.iter().flat_map(|s| s.rows.iter().filter_map(|r| r.example.clone().map(|e| (r.code.clone(), e)))).collect();
    assert!(examples.len() >= 4, "the guide offers examples to try");
    for (code, ex) in &examples {
        assert!(!model::parse_list(ex).is_empty(), "{code}: {ex}");
        let c = model::SearchToolConfig { filters_text: "bolt".into(), exclude_filters_text: ex.clone(), use_regex: code.contains('\\'), ..Default::default() };
        assert_eq!(model::regex_validation_error(&c), None, "{code}: {ex}");
    }
    assert_eq!(model::parse_list("draft, obsolete"), ["draft", "obsolete"], "commas separate the terms the guide says");
    assert_eq!(model::parse_list("do not use"), ["do not use"], "a phrase is one term");
}
