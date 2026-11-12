use super::*;
use crate::gpu_viewer::editor::authoring::Authoring;
use search_core::models::{FileSearchStatus, LineHit, MatchMode};
use std::time::Duration;

/// The Fast index card of the Options panel.
fn index_card(doc: &crate::gpu_viewer::ui::Document) -> &Card {
    doc.panels[0].cards.iter().find(|card| card.title == "Fast index").expect("an index card")
}
/// The items of the "Passages" card between the bar and the results (empty when it is not shown).
#[cfg(feature = "semantic-search")]
fn insight_items(doc: &crate::gpu_viewer::ui::Document) -> &[DockItem] {
    doc.insight.as_ref().map_or(&[], |insight| insight.items.as_slice())
}
/// Type `text` into Find and press Run, then wait for the searches, as the window does.
#[cfg(feature = "semantic-search")]
fn find_and_wait(session: &mut SearchSession, text: &str) {
    session.document_field(INPUT_FIND, text.into());
    act_on(session, SearchAct::Run);
    let started = Instant::now();
    while session.semantic_running || session.st.run.is_running {
        assert!(started.elapsed() < Duration::from_secs(120));
        session.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    session.poll();
}
fn hit(n: i32, line: &str, filter: &str) -> LineHit {
    LineHit {
        line_number: n,
        before: Some("above".into()),
        match_line: line.into(),
        after: None,
        matched_filters: vec![filter.into()],
    }
}
fn result(name: &str, hits: usize, size: i64) -> FileSearchResult {
    FileSearchResult {
        full_name: name.into(),
        status: FileSearchStatus::Hit,
        hits: (0..hits)
            .map(|k| hit(k as i32 + 1, "a bolt torque line", "bolt"))
            .collect(),
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
    s.st.run.results = vec![
        result("/w/a/spec.pdf", 3, 900),
        result("/w/notes.txt", 1, 50),
        result("/w/b/plan.pdf", 7, 10),
    ];
    s.st.run.started = Some(Instant::now());
    s
}

#[test]
fn full_document_text_has_no_line_numbers_and_only_matching_text_is_marked() {
    let mut s = with_results();
    s.selected = Some(0);
    let r = s.st.run.results.get_mut(0).unwrap();
    r.lines_cache = vec!["above".into(), "a bolt torque line".into(), "below".into()];
    r.total_line_count = 3;
    r.hits = vec![hit(2, "a bolt torque line", "bolt")];
    s.poll();
    let detail = s.document().unwrap().detail.unwrap();
    let text = detail.text.unwrap();
    assert_eq!(&*text.content, "above\na bolt torque line\nbelow");
    assert_eq!(&text.content[text.marks[0].0..text.marks[0].1], "bolt");
    assert!(detail.actions.iter().all(|a| a.label != "Copy content"));
}

#[test]
fn a_document_larger_than_the_embedded_report_limit_loads_every_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.txt");
    let mut lines = vec!["ordinary line".to_string(); 4_005];
    lines[4_004] = "last bolt".into();
    std::fs::write(&path, lines.join("\n")).unwrap();
    let mut s = SearchSession::in_memory();
    let mut r = result(&path.to_string_lossy(), 0, 0);
    r.total_line_count = lines.len() as i32;
    r.lines_cache = lines[..4_000].to_vec();
    r.hits = vec![hit(4_005, "last bolt", "bolt")];
    s.st.run.results.push(r);
    let start = Instant::now();
    loop {
        s.poll();
        if let Some(text) = s.document().unwrap().detail.unwrap().text {
            assert_eq!(text.content.lines().count(), 4_005);
            assert!(text.content.ends_with("last bolt"));
            assert_eq!(&text.content[text.marks[0].0..text.marks[0].1], "bolt");
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "full preview did not load"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn an_old_preview_cannot_replace_a_new_run_of_the_same_file() {
    let mut s = SearchSession::in_memory();
    let mut r = result("/w/same.txt", 1, 0);
    r.lines_cache = vec!["new bolt".into()];
    r.total_line_count = 1;
    r.hits = vec![hit(1, "new bolt", "bolt")];
    s.st.run.results.push(r);
    s.poll();
    let current = s.preview.as_ref().unwrap().generation;
    s.preview_tx
        .send(PreviewEvent::Done(
            current.wrapping_sub(1),
            Ok(vec!["old bolt".into()]),
        ))
        .unwrap();
    s.poll();
    assert_eq!(
        &*s.document().unwrap().detail.unwrap().text.unwrap().content,
        "new bolt"
    );
}

#[test]
fn report_auto_open_control_is_absent_from_the_ribbon() {
    let s = SearchSession::in_memory();
    assert!(view::ribbon(&s)
        .iter()
        .flat_map(|tab| &tab.groups)
        .flat_map(|group| &group.buttons)
        .all(|button| button.label != "Open when done"));
}

#[test]
fn cached_search_rows_follow_filter_sort_selection_and_new_runs() {
    let mut s = with_results();
    let original = view::document(&s).table.rows;
    assert_eq!(view::document(&s).table.rows, original);
    s.filter = "pdf".into();
    assert_eq!(view::document(&s).table.rows.len(), 2);
    s.sort = (SortKey::Hits, false);
    s.selected = Some(2);
    let rows = view::document(&s).table.rows;
    assert!(rows[0].selected && rows[0].cells[0] == "plan.pdf");
    s.st.run.started = Some(Instant::now());
    s.st.run.results = vec![
        result("/w/new.pdf", 1, 1),
        result("/w/other.pdf", 2, 2),
        result("/w/third.txt", 3, 3),
    ];
    assert_eq!(view::document(&s).table.rows[0].cells[0], "other.pdf");
}
#[test]
fn detail_highlights_accent_folded_and_typo_hits() {
    let mut s = with_results();
    s.st.config.filters_text = "cafe".into();
    s.st.config.fold_accents = true;
    let lines = vec!["un café noir".to_string()];
    let mut r = result("/w/a.txt", 0, 1);
    r.hits = vec![hit(1, "un café noir", "cafe")];
    let text = view::full_text(&r, &lines, s.line_explainer().as_ref());
    let (a, b) = text.marks[0];
    assert_eq!(&text.content[a..b], "café");

    s.st.config.fold_accents = false;
    s.st.config.typo_tolerance = 1;
    s.st.config.filters_text = "cafe".into();
    let text = view::full_text(&r, &lines, s.line_explainer().as_ref());
    assert!(text.marks.iter().any(|&(a, b)| &text.content[a..b] == "café"), "{:?}", text.marks);
}

#[test]
fn options_matching_card_offers_accent_and_typo_settings() {
    let s = with_results();
    let text = format!("{:?}", view::document(&s).panels);
    assert!(text.contains("Fold accents") && text.contains("Typo tolerance"), "{text}");
}

#[test]
fn best_match_sort_follows_relevance_order_and_flips_with_direction() {
    let mut s = with_results();
    s.st.config.filters_text = "bolt".into();
    let expected = search_core::ordering::relevance_order(
        &s.st.run.results,
        &crate::toolboxes::search::model::build_settings(&s.st.config),
    );
    s.sort = (SortKey::BestMatch, false);
    assert_eq!(s.view(), expected, "descending shows the best match first");
    s.sort = (SortKey::BestMatch, true);
    let mut reversed = expected.clone();
    reversed.reverse();
    assert_eq!(s.view(), reversed, "ascending shows the worst match first");
}

fn act_on(s: &mut SearchSession, a: SearchAct) {
    s.act(EditorAction::Search(a));
}

#[cfg(feature = "neural-search")]
#[test]
fn entity_preview_keeps_neural_scores_separate_from_ambiguous_pattern_hints() {
    use search_core::entities::{EntityCandidate, EntityKind, EntityScore};

    let candidate = EntityCandidate {
        bytes: 0..7,
        line_number: 3,
        shape_hints: vec![EntityKind::PartNumber, EntityKind::Standard],
        category_scores: vec![
            EntityScore {
                kind: EntityKind::PartNumber,
                similarity: 0.61,
            },
            EntityScore {
                kind: EntityKind::Standard,
                similarity: 0.57,
            },
        ],
    };

    assert_eq!(
        view::entity_signal_label(&candidate),
        "PartNumber or Standard: likely PartNumber 61%"
    );
    assert_eq!(candidate.category_scores[0].kind, EntityKind::PartNumber);
    assert_eq!(candidate.category_scores[0].similarity, 0.61);
}

#[cfg(feature = "neural-search")]
#[test]
fn saved_alert_runs_exact_search_then_updates_bounded_baseline_asynchronously() {
    use crate::toolboxes::search::{model::SearchToolConfig, persistence::PersistedSearchSettings};

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("first.txt"),
        "fatigue crack at the fastener hole\n",
    )
    .unwrap();
    let config = SearchToolConfig {
        search_path: dir.path().to_string_lossy().into_owned(),
        filters_text: "fatigue".into(),
        output_folder: dir.path().to_string_lossy().into_owned(),
        selected_extensions: Some(vec![".txt".into()]),
        ..Default::default()
    };
    let alert = search_core::alerts::SavedSearchAlert::create(
        "fatigue".into(),
        model::build_settings(&config),
    )
    .unwrap();
    let record = alerts::Record::new(alert, PersistedSearchSettings::from(&config));
    let mut s = SearchSession::in_memory();
    s.st.config = config;
    s.alerts.push(record);

    let wait_for_alert = |s: &mut SearchSession| {
        let deadline = Instant::now() + Duration::from_secs(120);
        while !s.alert_jobs.is_empty() {
            assert!(Instant::now() < deadline, "saved alert did not finish");
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        s.poll();
    };

    s.run_saved_alert(0, true);
    wait_for_alert(&mut s);
    assert_eq!(s.alerts[0].alert.baseline.as_ref().unwrap().len(), 1);

    std::fs::write(
        dir.path().join("first.txt"),
        "fatigue crack moved to the lower fastener hole\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("second.txt"), "fatigue limit changed\n").unwrap();
    s.run_saved_alert(0, false);
    wait_for_alert(&mut s);
    assert_eq!(s.alerts[0].alert.baseline.as_ref().unwrap().len(), 2);
    assert!(s.status.contains("2 new or changed exact-hit files"));
    assert!(
        !s.st.run.is_running,
        "alert search must not replace interactive run state"
    );
    let rows: Vec<_> = index_card(&view::document(&s))
        .items
        .iter()
        .filter_map(DockItem::plain_text)
        .collect();
    assert!(rows
        .iter()
        .any(|row| row.contains("New ·") && row.contains("second.txt")));
    assert!(rows
        .iter()
        .any(|row| row.contains("Updated ·") && row.contains("first.txt")));
    assert!(rows.iter().any(|row| row.contains("% different")));
    s.delete_alert(0);
    assert!(s.last_alert_changes.is_none());
}

#[test]
fn recent_searches_can_be_removed_individually_or_cleared() {
    use crate::toolboxes::search::persistence::RecentSearch;

    let mut s = SearchSession::in_memory();
    s.st.recent_searches = vec![
        RecentSearch {
            search_path: "/a".into(),
            filters_text: "bolt".into(),
        },
        RecentSearch {
            search_path: "/b".into(),
            filters_text: "bearing".into(),
        },
    ];

    let sidebar = view::document(&s).sidebar.unwrap();
    assert_eq!(
        sidebar.recent_searches[0].remove,
        act(SearchAct::RemoveRecent(0))
    );
    assert_eq!(sidebar.clear_recents, Some(act(SearchAct::ClearRecents)));
    act_on(&mut s, SearchAct::RemoveRecent(0));
    assert_eq!(s.st.recent_searches.len(), 1);
    assert_eq!(s.st.recent_searches[0].search_path, "/b");

    act_on(&mut s, SearchAct::ClearRecents);
    assert!(s.st.recent_searches.is_empty());
}

#[test]
fn completed_search_keeps_heading_and_progress_summary_visible() {
    let mut s = with_results();
    s.st.run.started = Some(Instant::now());
    s.st.run.total_files = 12;
    s.st.run.files_completed = 12;
    s.st.run.matching_lines = 11;

    let doc = s.document().unwrap();
    assert_eq!(doc.header.as_ref().unwrap().title, "Search files");
    let progress = doc.progress.unwrap();
    assert!(progress.headline.starts_with("Search complete"));
    assert_eq!(
        progress.metrics[0],
        ("Files with matches".into(), "3".into())
    );
    assert!(progress.cancel.is_none());
}

#[cfg(feature = "neural-search")]
#[test]
fn neural_autocomplete_ranks_corpus_phrases_without_changing_exact_results() {
    let mut s = SearchSession::in_memory();
    s.st.config.search_path = "/w".into();
    s.st.config.filters_text = "dog".into();
    let mut r = result("/w/garden.txt", 1, 1);
    r.hits[0] = hit(
        1,
        "dog playing outdoors in a garden with notes about tax policy",
        "dog",
    );
    s.st.run.results.push(r);
    let exact_paths: Vec<_> =
        s.st.run
            .results
            .iter()
            .map(|r| r.full_name.clone())
            .collect();

    s.poll();
    let started = Instant::now();
    while s.neural_order.is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "neural autocomplete did not return"
        );
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }

    let candidates = s.corpus_term_suggestions();
    let (context, inputs) = view::neural_suggestion_inputs(&s);
    let expected =
        search_core::neural_search::rank_suggestions(&context, &inputs).unwrap()[0].index;
    let choices = &view::document(&s).inputs[1].choices;
    assert_eq!(
        choices[0].label,
        format!("Related from hits: {}", candidates[expected])
    );
    assert_eq!(
        s.st.run
            .results
            .iter()
            .map(|r| r.full_name.clone())
            .collect::<Vec<_>>(),
        exact_paths
    );
}

#[cfg(feature = "neural-search")]
#[test]
fn neural_entity_suggestions_arrive_for_the_current_preview_without_changing_exact_results() {
    let mut s = with_results();
    s.selected = Some(0);
    s.preview_generation = 7;
    s.preview = Some(Preview {
        generation: 7,
        path: "/w/a/spec.pdf".into(),
        text: Some(crate::gpu_viewer::ui::DocText {
            content: "P/N AX-42-7".into(),
            marks: Default::default(),
        }),
        status: String::new(),
        related: Vec::new(),
        entities: None,
    });
    let paths =
        s.st.run
            .results
            .iter()
            .map(|r| r.full_name.clone())
            .collect::<Vec<_>>();
    s.start_entity_analysis(7, "P/N AX-42-7".into());

    let started = Instant::now();
    while s
        .preview
        .as_ref()
        .unwrap()
        .entities
        .as_ref()
        .is_none_or(Result::is_err)
    {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "entity analysis did not finish"
        );
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    s.poll();

    let detail = view::document(&s).detail.unwrap();
    assert!(detail.notes.iter().any(|note| note.hint.contains("AX-42-7")));
    assert!(detail.notes.iter().any(|note| note.hint.contains("not confirmed")));
    assert_eq!(
        s.st.run
            .results
            .iter()
            .map(|r| r.full_name.clone())
            .collect::<Vec<_>>(),
        paths
    );
}

#[cfg(feature = "neural-search")]
#[test]
fn extractive_summary_is_cited_in_index_panel_and_bound_to_selected_file() {
    let mut s = with_results();
    s.selected = Some(0);
    s.preview_generation = 7;
    s.preview = Some(Preview {
        generation: 7,
        path: "/w/a/spec.pdf".into(),
        text: Some(crate::gpu_viewer::ui::DocText {
            content: "Torque loads the shaft.".into(),
            marks: Default::default(),
        }),
        status: String::new(),
        related: Vec::new(),
        entities: None,
    });
    let key = SummaryKey {
        path: "/w/a/spec.pdf".into(),
        preview_generation: 7,
    };
    s.summary_request = Some(key.clone());
    s.summary_running = true;
    s.summary_tx
        .send(SummaryEvent {
            key,
            result: Ok(search_core::neural_search::ExtractiveSummary {
                lines: vec![search_core::neural_search::CitedSummaryLine {
                    line_number: 1,
                    text: "Torque loads the shaft.".into(),
                }],
                source_lines: 1,
                sampled_lines: 1,
                skipped_long_lines: 0,
            }),
        })
        .unwrap();
    s.poll();

    assert_eq!(s.current_summary().unwrap().lines[0].line_number, 1);
    let doc = view::document(&s);
    let index_items = &index_card(&doc).items;
    assert!(index_items
        .iter()
        .filter_map(DockItem::plain_text)
        .any(|row| row == "L1  Torque loads the shaft."));
    assert_eq!(
        s.st.run.results.len(),
        3,
        "summaries preserve exact results"
    );

    s.selected = Some(1);
    assert!(
        s.current_summary().is_none(),
        "old file summary stays hidden"
    );
}

#[cfg(feature = "neural-search")]
#[test]
fn cancelled_summary_keeps_session_busy_until_worker_completes() {
    let mut s = with_results();
    s.selected = Some(0);
    s.preview_generation = 7;
    s.preview = Some(Preview {
        generation: 7,
        path: "/w/a/spec.pdf".into(),
        text: Some(crate::gpu_viewer::ui::DocText {
            content: "Torque loads the shaft.".into(),
            marks: Default::default(),
        }),
        status: String::new(),
        related: Vec::new(),
        entities: None,
    });
    let key = SummaryKey {
        path: "/w/a/spec.pdf".into(),
        preview_generation: 7,
    };
    let cancel = CancellationToken::new();
    s.summary_request = Some(key.clone());
    s.summary_running = true;
    s.summary_cancel = Some(cancel.clone());

    s.selected = Some(1);
    s.refresh_preview();
    assert!(cancel.is_cancelled());
    assert!(s.summary_running, "cancel waits for inference worker exit");
    s.summarize_selected();
    assert!(s.summary_running, "a second inference cannot overlap");

    s.summary_tx
        .send(SummaryEvent {
            key,
            result: Err("Extractive summary cancelled.".into()),
        })
        .unwrap();
    s.poll();
    assert!(!s.summary_running, "worker completion releases the session");
}

#[cfg(feature = "neural-search")]
#[test]
fn relevance_sort_only_reorders_exact_results_and_explains_each_score() {
    let mut s = with_results();
    s.st.config.filters_text = "dog playing outdoors in a garden".into();
    let mut tax = result("/w/tax.txt", 1, 10);
    tax.hits[0] = hit(
        1,
        "The annual tax policy changed after the election.",
        "dog",
    );
    let mut garden = result("/w/garden-notes.txt", 3, 10);
    garden.hits[0] = hit(1, "A dog plays outside among plants.", "dog");
    garden.proximity_min_range = Some(1);
    s.st.run.results = vec![tax, garden];
    let exact_paths =
        s.st.run
            .results
            .iter()
            .map(|result| result.full_name.clone())
            .collect::<Vec<_>>();

    act_on(&mut s, SearchAct::SortBy(SortKey::Relevance));
    let started = Instant::now();
    while s.relevance_scores.is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "local relevance ranker did not return"
        );
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(s.view(), [1, 0]);
    assert_eq!(
        s.st.run
            .results
            .iter()
            .map(|result| result.full_name.clone())
            .collect::<Vec<_>>(),
        exact_paths
    );
    assert!(view::document(&s)
        .table
        .columns
        .iter()
        .any(|column| column.title == "Relevance"));
    let detail = view::document(&s).detail.unwrap();
    let labels: Vec<_> = detail.signals.iter().map(|signal| signal.label.as_str()).collect();
    assert_eq!(labels.first(), Some(&"Relevance"));
    for label in ["Text", "Name", "Exact", "Recent", "Nearby"] {
        assert!(labels.contains(&label), "missing meter {label}: {labels:?}");
    }
    assert!(detail.signals.iter().all(|signal| (0.0..=1.0).contains(&signal.fraction) && !signal.hint.is_empty()));
}

#[cfg(feature = "neural-search")]
#[test]
fn local_feedback_labels_reorder_scores_without_changing_exact_result_membership() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = with_results();
    s.settings_file = dir.path().join("settings-tui.json");
    s.st.config.filters_text = "bolt torque".into();
    let exact_paths =
        s.st.run
            .results
            .iter()
            .map(|result| result.full_name.clone())
            .collect::<Vec<_>>();

    act_on(&mut s, SearchAct::ToggleRelevanceFeedback);
    assert!(s.feedback_enabled());
    act_on(&mut s, SearchAct::SortBy(SortKey::Relevance));
    wait_for_relevance(&mut s);
    assert!(view::document(&s)
        .detail
        .unwrap()
        .actions
        .iter()
        .any(|entry| entry.label == "Relevant"));

    act_on(
        &mut s,
        SearchAct::VoteRelevance(1, search_core::relevance::FeedbackLabel::Relevant),
    );
    wait_for_relevance(&mut s);
    act_on(
        &mut s,
        SearchAct::VoteRelevance(0, search_core::relevance::FeedbackLabel::NotRelevant),
    );
    wait_for_relevance(&mut s);

    assert!(s.relevance_score(0).unwrap().feedback_applied);
    assert_eq!(feedback::snapshot(&s.feedback_path()).unwrap().1.len(), 2);
    assert_eq!(
        s.st.run
            .results
            .iter()
            .map(|result| result.full_name.clone())
            .collect::<Vec<_>>(),
        exact_paths
    );

    act_on(&mut s, SearchAct::ResetRelevanceFeedback);
    wait_for_relevance(&mut s);
    assert!(!s.feedback_enabled());
    assert!(!s.relevance_score(0).unwrap().feedback_applied);
}

#[cfg(feature = "neural-search")]
fn wait_for_relevance(s: &mut SearchSession) {
    let started = Instant::now();
    while s.relevance_running {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "relevance ranking did not finish"
        );
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    s.poll();
}

#[cfg(feature = "neural-search")]
#[test]
fn relevance_sort_keeps_found_order_until_current_scores_arrive() {
    let mut s = with_results();
    s.sort = (SortKey::Relevance, false);
    assert_eq!(s.view(), [0, 1, 2]);

    s.relevance_scores = Some(("stale query".into(), std::collections::HashMap::new()));
    assert_eq!(s.view(), [0, 1, 2]);
}

#[cfg(feature = "neural-search")]
#[test]
fn relevance_rows_refresh_when_the_query_changes() {
    let mut s = with_results();
    s.st.config.filters_text = "first query".into();
    s.sort = (SortKey::Relevance, false);
    let key = s.relevance_key();
    s.relevance_scores = Some((
        key,
        [(
            0,
            search_core::relevance::RelevanceExplanation {
                index: 0,
                score: 0.9,
                base_score: 0.9,
                feedback_applied: false,
                match_text_similarity: 0.9,
                filename_similarity: 0.9,
                exact_evidence: 0.9,
                proximity: None,
                recency: 0.9,
            },
        )]
        .into_iter()
        .collect(),
    ));
    assert_eq!(view::document(&s).table.rows[0].cells[4], "90%");

    s.st.config.filters_text = "different query".into();
    let rows = view::document(&s).table.rows;
    assert!(rows.iter().all(|row| row.cells[4].is_empty()));
}

#[test]
fn typo_correction_is_offered_only_after_zero_results_and_requires_explicit_action() {
    let mut s = SearchSession::in_memory();
    s.st.config.filters_text = "ultimate tensile strenght".into();
    assert!(!view::document(&s).inputs.iter().any(|input| input
        .choices
        .iter()
        .any(|choice| choice.label.contains("instead of"))));

    s.st.run.started = Some(Instant::now());
    s.st.run.is_running = false;
    s.completed_search_query = Some(s.st.config.filters_text.clone());
    assert!(view::document(&s).inputs.iter().any(|input| input
        .choices
        .iter()
        .any(|choice| choice.label.contains("instead of"))));

    s.st.config.filters_text.push_str(" changed");
    assert!(!view::document(&s).inputs.iter().any(|input| input
        .choices
        .iter()
        .any(|choice| choice.label.contains("instead of"))));
    act_on(&mut s, SearchAct::ApplyCorrection(0));
    assert_eq!(
        s.st.config.filters_text,
        "ultimate tensile strenght changed"
    );
    s.st.config.filters_text = "ultimate tensile strenght".into();

    act_on(&mut s, SearchAct::ApplyCorrection(0));
    assert_eq!(s.st.config.filters_text, "ultimate tensile strength");
    assert!(
        s.st.run.results.is_empty(),
        "applying suggestion must not run or change exact results"
    );
}

#[test]
fn proximity_suggestion_only_appears_for_current_zero_result_proximity_run() {
    let mut s = SearchSession::in_memory();
    s.st.config.filters_text = "alpha,beta".into();
    s.st.config.match_mode = MatchMode::Proximity;
    s.st.run.started = Some(Instant::now());
    s.completed_search_query = Some(s.st.config.filters_text.clone());
    s.st.run.diagnostics = Some(crate::toolboxes::search::diagnostics::SearchDiagnostics {
        skipped_by_mode: 2,
        ..Default::default()
    });
    s.st.run.is_running = true;
    assert!(view::document(&s).notice.is_none());
    s.st.run.is_running = false;
    assert!(view::document(&s)
        .notice
        .as_ref()
        .is_some_and(|notice| notice.text.contains("within")));

    s.st.config.filters_text.push_str(" changed");
    assert!(!s.can_widen_proximity());
    s.st.config.filters_text = "alpha,beta".into();
    s.st.run.results.push(result("/tmp/hit.txt", 1, 10));
    assert!(!s.can_widen_proximity());
}

#[test]
fn widening_proximity_reruns_the_same_exact_filters_with_user_selected_range() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("near.txt"), "alpha\nx\nx\nx\nbeta\n").unwrap();
    let mut s = SearchSession::in_memory();
    s.st.config.search_path = dir.path().to_string_lossy().into_owned();
    s.st.config.filters_text = "alpha,beta".into();
    s.st.config.match_mode = MatchMode::Proximity;
    s.st.config.proximity_lines = 1;
    s.st.run.started = Some(Instant::now());
    s.st.run.diagnostics = Some(crate::toolboxes::search::diagnostics::SearchDiagnostics {
        skipped_by_mode: 1,
        ..Default::default()
    });
    s.completed_search_query = Some(s.st.config.filters_text.clone());

    act_on(&mut s, SearchAct::WidenProximity);
    assert_eq!(s.st.config.proximity_lines, 4);
    let started = Instant::now();
    loop {
        s.poll();
        if !s.st.run.is_running && !s.st.run.results.is_empty() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "widened exact search did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(s.st.run.results.len(), 1);
    let matched_filters = s.st.run.results[0]
        .hits
        .iter()
        .flat_map(|hit| hit.matched_filters.iter().map(String::as_str))
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(matched_filters, ["alpha", "beta"].into_iter().collect());
}

#[test]
fn synonym_expansion_is_explained_and_keeps_any_mode_semantics() {
    let mut s = SearchSession::in_memory();
    s.st.config.filters_text = "OD, bolt".into();
    let choices = view::document(&s).inputs[1].choices.clone();
    assert!(choices
        .iter()
        .any(|choice| choice.label == "Expand OD with outer diameter · abbreviation"));

    act_on(&mut s, SearchAct::AddSynonym(0));
    assert_eq!(s.st.config.filters_text, "OD, bolt, outer diameter");
    assert_eq!(
        model::parse_filter_groups(&s.st.config.filters_text)[0].filters,
        ["OD", "bolt", "outer diameter"]
    );
    assert!(s.status().contains("any-line alternative"));

    s.st.config.match_mode = MatchMode::AllInFile;
    s.st.config.filters_text = "OD".into();
    assert!(view::document(&s).inputs[1]
        .choices
        .iter()
        .all(|choice| !choice.label.starts_with("Expand ")));
}

#[test]
fn synonym_expansion_keeps_explicit_any_group_tags() {
    let mut s = SearchSession::in_memory();
    s.st.config.match_mode = MatchMode::AllInFile;
    s.st.config.filters_text = "OD [any]; load [all]".into();
    let choices = view::document(&s).inputs[1].choices.clone();
    assert!(choices
        .iter()
        .any(|choice| choice.label.starts_with("Expand OD with ")));
    assert!(choices.iter().all(|choice| !choice.label.contains("load")));
    act_on(&mut s, SearchAct::AddSynonym(0));
    assert_eq!(
        s.st.config.filters_text,
        "OD, outer diameter [any]; load [all]"
    );
}

#[cfg(feature = "semantic-search")]
#[test]
fn semantic_citations_show_page_only_when_verified() {
    let hit = search_core::semantic_index::SemanticHit {
        path: "manual.pdf".into(),
        first_line: 9,
        last_line: 10,
        excerpt: "source".into(),
        similarity: 0.8,
        page: Some(std::num::NonZeroU32::new(4).unwrap()),
    };
    assert_eq!(
        view::semantic_citation(&hit),
        "manual.pdf · page 4 · lines 9–10"
    );

    let mut unknown = hit;
    unknown.page = None;
    assert_eq!(view::semantic_citation(&unknown), "manual.pdf · lines 9–10");
}

#[cfg(feature = "semantic-search")]
#[test]
fn semantic_search_shows_cited_hits_separately_from_exact_results() {
    use crate::toolboxes::search::indexing::{index_directory, IndexLocation};
    use search_core::semantic_index::SemanticIndex;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shaft.txt");
    let related_file = dir.path().join("shaft-revision.txt");
    std::fs::write(&file, "The shaft rotates under torque and torsional load.").unwrap();
    std::fs::write(
        &related_file,
        "The rotating shaft experiences torsional stress under applied torque.",
    )
    .unwrap();
    let metadata = std::fs::metadata(&file).unwrap();
    let related_metadata = std::fs::metadata(&related_file).unwrap();
    let modified = search_core::cache::ticks_from_modified(
        chrono::DateTime::<chrono::Local>::from(metadata.modified().unwrap()),
    );
    let index_dir = index_directory(
        IndexLocation::SearchFolder,
        &dir.path().to_string_lossy(),
        "",
    );
    std::fs::create_dir_all(&index_dir).unwrap();
    let semantic = SemanticIndex::open(&index_dir.join("semantic.sqlite")).unwrap();
    semantic
        .index_document(
            &file.to_string_lossy(),
            modified,
            metadata.len() as i64,
            &["The shaft rotates under torque and torsional load.".into()],
        )
        .unwrap();

    let mut s = SearchSession::in_memory();
    s.st.config.search_path = dir.path().to_string_lossy().into_owned();
    s.st.config.filters_text = "rotational force".into();
    s.st.config.index.semantic = true;
    act_on(&mut s, SearchAct::SemanticSearch);
    let started = Instant::now();
    while s.semantic_running {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "semantic search did not return"
        );
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    s.poll();

    let hits = s.semantic_hits.as_ref().unwrap_or_else(|| {
        panic!(
            "semantic results missing: {}; error: {:?}",
            s.status, s.semantic_error
        )
    });
    assert!(
        !hits.is_empty(),
        "semantic search returned no cited matches: {}",
        s.status
    );
    let hit = &hits[0];
    assert_eq!(hit.path, file.to_string_lossy());
    assert_eq!((hit.first_line, hit.last_line), (1, 1));
    assert_eq!(hit.page, None);
    assert!(
        s.st.run.results.is_empty(),
        "semantic search must not modify exact results"
    );
    let doc = view::document(&s);
    let index_items = insight_items(&doc);
    assert!(index_items
        .iter()
        .any(|item| matches!(item, DockItem::Heading(label) if label == "Possible evidence for ‘rotational force’")));
    assert!(index_items
        .iter()
        .filter_map(DockItem::plain_text)
        .any(|row| row.contains("shaft.txt · line 1")));
    assert!(index_items
        .iter()
        .filter_map(DockItem::plain_text)
        .any(|row| row.contains("quoted from your files")));


    semantic
        .index_document(
            &related_file.to_string_lossy(),
            search_core::cache::ticks_from_modified(chrono::DateTime::<chrono::Local>::from(
                related_metadata.modified().unwrap(),
            )),
            related_metadata.len() as i64,
            &["The rotating shaft experiences torsional stress under applied torque.".into()],
        )
        .unwrap();

    s.st.run.results = vec![result(&file.to_string_lossy(), 1, metadata.len() as i64)];
    s.selected = Some(0);
    s.preview_generation = 7;
    s.preview = Some(Preview {
        generation: 7,
        path: file.to_string_lossy().into_owned(),
        text: Some(crate::gpu_viewer::ui::DocText {
            content: std::sync::Arc::from("The shaft rotates under torque and torsional load."),
            marks: std::sync::Arc::from(Vec::<(usize, usize)>::new()),
        }),
        status: String::new(),
        related: Vec::new(),
        #[cfg(feature = "neural-search")]
        entities: None,
    });
    act_on(&mut s, SearchAct::FindRelated);
    let started = Instant::now();
    while s.related_running {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "related-document search did not return"
        );
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    s.poll();
    let related = s.current_related_hits().expect("related candidates");
    assert_eq!(related[0].path, related_file.to_string_lossy());
    assert_eq!((related[0].first_line, related[0].last_line), (1, 1));
    assert!(!related.iter().any(|hit| hit.path == file.to_string_lossy()));
    assert_eq!(s.st.run.results[0].full_name, file.to_string_lossy());
    act_on(&mut s, SearchAct::CompareRelated(0));
    let started = Instant::now();
    while s.diff_running {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "exact related-document comparison did not return"
        );
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    s.poll();
    let (_, diff) = s.diff_result.as_ref().expect("exact extracted-line diff");
    assert_eq!((diff.added, diff.removed), (1, 1));
    assert_eq!(diff.rows[0].side, diff::Side::Source);
    assert_eq!(diff.rows[1].side, diff::Side::Candidate);
    let index_panel = view::document(&s)
        .panels
        .into_iter()
        .find(|panel| panel.label == "Options")
        .unwrap();
    assert!(index_panel
        .cards
        .iter()
        .flat_map(|card| &card.items)
        .any(|item| {
            matches!(item, DockItem::Heading(title) if title == "Exact extracted-line changes")
        }));
    assert!(s.current_related_diff().is_some());
    let (stale_key, stale_diff) = s.diff_result.take().unwrap();
    let stale_generation = s.diff_generation;
    s.diff_request = Some(stale_key.clone());
    s.diff_running = true;
    s.diff_tx
        .send(RelatedDiffEvent {
            generation: stale_generation,
            key: stale_key,
            result: Ok(stale_diff.clone()),
        })
        .unwrap();
    s.st.config.search_path = "/different-scope".into();
    s.poll();
    assert!(s.diff_result.is_none());
    assert!(s.current_related_hits().is_none());
    assert!(s.current_related_diff().is_none());
}

#[cfg(feature = "semantic-search")]
#[test]
fn semantic_results_are_discarded_when_scope_changes_before_poll() {
    let mut s = SearchSession::in_memory();
    s.st.config.search_path = "/w".into();
    s.st.config.filters_text = "shaft".into();
    let roots = semantic_search_roots(&s.st.config);
    let key = semantic_search_key(&s.st.config, "shaft".into(), &roots);
    s.semantic_request = Some(key.clone());
    s.semantic_running = true;
    s.semantic_tx
        .send(SemanticSearchEvent {
            key,
            result: Ok(Vec::new()),
        })
        .unwrap();

    s.st.config.include_hidden = !s.st.config.include_hidden;
    s.poll();

    assert!(
        !s.semantic_running,
        "matching stale request stops its progress state"
    );
    assert!(
        s.semantic_hits.is_none(),
        "results for old scope are discarded"
    );
}

/// Poll until the run (and its report) finished, as the window does between frames.
fn wait(s: &mut SearchSession) {
    let t = Instant::now();
    while s.st.run.is_running
        || s.st.run.last_report_path.is_none()
            && s.st.config.export_html
            && t.elapsed() < Duration::from_secs(5)
    {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "the search did not finish"
        );
        s.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    s.poll();
}

#[test]
fn a_search_runs_on_the_session_runtime_and_its_matches_fill_the_table_and_the_detail() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("hit.txt"),
        "first line\nthe bolt torque is 40\nlast line\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("miss.txt"), "nothing to see\n").unwrap();
    let mut s = SearchSession::in_memory();
    s.st.config.output_folder = dir.path().join("out").to_string_lossy().into_owned();
    s.st.config.export_html = true;
    s.document_field(INPUT_FOLDER, dir.path().to_string_lossy().into_owned());
    s.document_field(INPUT_FIND, "bolt".into());
    act_on(&mut s, SearchAct::Run);
    assert!(s.solving(), "the run is in flight right after Run");
    let live = s.document().unwrap();
    assert!(
        live.progress.is_some() && live.buttons[0].label == "Stop",
        "a running search shows its progress and Stop"
    );
    wait(&mut s);
    assert!(!s.solving());
    let doc = s.document().unwrap();
    assert_eq!(
        doc.table.rows.len(),
        1,
        "only the file with a match is listed"
    );
    assert_eq!(doc.table.rows[0].cells[0], "hit.txt");
    let d = doc.detail.expect("the first result is shown");
    let text = d.text.expect("the entire document is shown");
    assert_eq!(
        &*text.content,
        "first line\nthe bolt torque is 40\nlast line\n"
    );
    let (a, b) = text.marks[0];
    assert_eq!(&text.content[a..b], "bolt", "the match is highlighted");
    assert!(
        doc.stats
            .iter()
            .any(|(l, v)| l == "Files with matches" && v == "1"),
        "{:?}",
        doc.stats
    );
    assert!(s.status().contains("1 file with matches"), "{}", s.status());
    assert_eq!(
        s.st.recent_searches.len(),
        1,
        "the run is remembered like in the terminal"
    );
    assert_eq!(s.step_state("Results"), Some(StepState::Done));
    let report = PathBuf::from(
        s.st.run
            .last_report_path
            .as_deref()
            .expect("the report is written"),
    );
    assert_eq!(
        report.parent(),
        Some(dir.path().join("out").as_path()),
        "the report goes to the output folder"
    );
    assert!(doc
        .buttons
        .iter()
        .any(|button| button.label == "Open report"));
}

#[test]
fn a_test_session_writes_no_report_unless_a_test_asks_for_one() {
    // An empty output folder means the working folder: a test run would leave `search-report.html` in the crate and
    // fail the qualification's unchanged-source check.
    let s = SearchSession::in_memory();
    assert!(
        !s.st.config.export_html,
        "the HTML report is off in a test session"
    );
}

#[test]
fn a_search_that_cannot_run_says_why_and_starts_nothing() {
    let mut s = SearchSession::in_memory();
    // Nothing typed: the button says what is missing.
    let search = s.ribbon().remove(0).groups.remove(0).buttons.remove(0);
    assert!(!search.enabled && search.reason.as_deref().unwrap().contains("find"));
    // Words but no folder.
    s.st.config.filters_text = "torque, bolt".into();
    act_on(&mut s, SearchAct::Run);
    assert!(!s.solving());
    assert!(
        s.status().contains("search path") || s.status().contains("folder"),
        "{}",
        s.status()
    );
    let search = s.ribbon().remove(0).groups.remove(0).buttons.remove(0);
    assert!(!search.enabled && search.reason.as_deref().unwrap().contains("folder"));
    s.st.config.search_path = "/w".into();
    s.st.config.filters_text = "(".into();
    s.st.config.use_regex = true;
    act_on(&mut s, SearchAct::Run);
    assert!(!s.solving());
    assert!(s.status().starts_with("Fix the pattern"), "{}", s.status());
    assert!(
        s.document().unwrap().inputs[1].error.is_some(),
        "the Find input shows the pattern error"
    );
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
    assert_eq!(
        s.view(),
        [0, 2],
        "the filter is case-insensitive over the path"
    );
    assert_eq!(
        s.document().unwrap().table.title,
        "Results  ·  2 of 3 files"
    );
    assert_eq!(
        s.current(),
        Some(0),
        "the first shown row is current without a click"
    );
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
    let size = settings_view::settings()
        .position(|x| x.2 == "Max file size (MB)")
        .unwrap();
    act_on(&mut s, SearchAct::Edit(size));
    let card = s.value_card().expect("a typed setting opens a value card");
    assert_eq!(card.title, "Max file size (MB)");
    s.card_field(0, "12.5".into());
    act_on(&mut s, SearchAct::ApplyEdit);
    assert_eq!(s.st.config.max_file_size_mb, 12.5);
    act_on(&mut s, SearchAct::Edit(size));
    s.card_field(0, "lots".into());
    act_on(&mut s, SearchAct::ApplyEdit);
    assert_eq!(
        s.st.config.max_file_size_mb, 12.5,
        "an invalid number keeps the value"
    );
    assert!(s.status().contains("not a valid value"), "{}", s.status());
    let regex = settings_view::settings()
        .position(|x| x.2 == "Use regex")
        .unwrap();
    act_on(&mut s, SearchAct::Edit(regex));
    assert!(
        s.st.config.use_regex && s.value_card().is_none(),
        "a switch flips without a card"
    );
    act_on(&mut s, SearchAct::Match(MatchModeArg::Proximity));
    assert_eq!(s.st.config.match_mode, MatchModeArg::Proximity);
    // The bar's inputs are not repeated as inspector rows.
    let rows: Vec<String> = s
        .inspector()
        .into_iter()
        .flat_map(|c| c.items)
        .filter_map(|i| i.plain_text())
        .collect();
    assert!(
        !rows
            .iter()
            .any(|r| r.starts_with("Search path") || r.starts_with("Filters ")),
        "{rows:?}"
    );
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
fn also_indexed_folders_reports_what_it_did_and_never_adds_the_main_folder() {
    let mut s = SearchSession::in_memory();
    s.st.config.search_path = "/main".into();
    act_on(&mut s, SearchAct::AlsoIndexedFolders);
    // The catalog is the machine's real one: assert the contract, not its contents.
    assert!(
        s.status().starts_with("No other indexed folders") || s.status().starts_with("Also searching"),
        "{}",
        s.status()
    );
    assert!(!s.st.config.search_paths_extra.iter().any(|p| p == "/main"));
}

#[test]
fn browse_uses_the_system_folder_picker_and_a_cancel_changes_nothing() {
    let mut s = SearchSession::in_memory();
    s.st.config.search_path = "/keep".into();
    act_on(&mut s, SearchAct::BrowseFolder);
    assert_eq!(
        s.st.config.search_path, "/keep",
        "a cancelled dialog keeps the folder"
    );
    assert_eq!(s.status(), "No folder chosen.");
    s.set_folder_picker(|_| Some(PathBuf::from("/picked/Drawings")));
    act_on(&mut s, SearchAct::BrowseFolder);
    assert_eq!(
        s.st.config.search_path,
        PathBuf::from("/picked/Drawings").to_string_lossy()
    );
    assert!(s.document().unwrap().inputs[0]
        .trailing
        .as_ref()
        .is_some_and(|b| b.label == "Browse"));
}

#[test]
fn the_output_folder_is_an_input_with_browse_in_the_report_popover() {
    let mut s = SearchSession::in_memory();
    let doc = s.document().unwrap();
    let report = doc.panels.iter().find(|p| p.label == "Report").unwrap();
    let input = &report.inputs[0];
    assert_eq!(
        (input.id, input.label.as_str()),
        (INPUT_OUTPUT, "Output folder")
    );
    assert_eq!(
        input.trailing.as_ref().map(|b| b.action),
        Some(act(SearchAct::BrowseOutput))
    );
    let rows: Vec<String> = report
        .cards
        .iter()
        .flat_map(|c| c.items.iter())
        .filter_map(|i| i.plain_text())
        .collect();
    assert!(
        !rows.iter().any(|r| r.starts_with("Output folder")),
        "not repeated as a row: {rows:?}"
    );
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
    assert_eq!(
        PathBuf::from(&s.st.config.output_folder),
        dir.path().join("reports")
    );
    assert!(
        s.st.config.search_path == dir.path().to_string_lossy(),
        "the folder to search is untouched"
    );
    // The ribbon and a typed-setting edit of the output folder open the picker too, never a value card.
    let out = settings_view::settings()
        .position(|x| x.2 == "Output folder")
        .unwrap();
    act_on(&mut s, SearchAct::Edit(out));
    assert!(s.value_card().is_none());
}

#[test]
fn every_example_of_the_find_guide_parses_the_way_the_guide_says() {
    use crate::toolboxes::search::model::{parse_filter_groups, GroupMode};
    use search_core::models::ExcludeScope;
    let help = view::find_help();
    let examples: Vec<(String, String)> = help
        .sections
        .iter()
        .flat_map(|s| {
            s.rows
                .iter()
                .filter_map(|r| r.example.clone().map(|e| (r.code.clone(), e)))
        })
        .collect();
    assert!(examples.len() >= 10, "the guide offers examples to try");
    for (code, ex) in &examples {
        let groups = parse_filter_groups(ex);
        assert!(
            !groups.is_empty() && groups.iter().all(|g| !g.filters.is_empty()),
            "{code}: {ex}"
        );
        assert!(
            groups
                .iter()
                .any(|g| !matches!(g.mode, Some(GroupMode::Exclude(_)))),
            "{code}: an example must search for something"
        );
        let mut c = model::SearchToolConfig {
            filters_text: ex.clone(),
            ..Default::default()
        };
        c.use_regex = code.contains('\\');
        assert_eq!(model::regex_validation_error(&c), None, "{code}: {ex}");
    }
    let mode = |ex: &str, k: usize| parse_filter_groups(ex)[k].mode;
    assert_eq!(
        mode("bolt, torque [near 3]", 0),
        Some(GroupMode::Match(MatchModeArg::Proximity, Some(3)))
    );
    assert_eq!(
        mode("hole, edge [all]", 0),
        Some(GroupMode::Match(MatchModeArg::AllInFile, None))
    );
    assert_eq!(
        mode("torque ; draft, obsolete [not]", 1),
        Some(GroupMode::Exclude(ExcludeScope::File))
    );
    assert_eq!(
        mode("torque ; todo [not line]", 1),
        Some(GroupMode::Exclude(ExcludeScope::Line))
    );
    assert_eq!(
        parse_filter_groups("bolt torque")[0].filters,
        ["bolt torque"],
        "a phrase is one term"
    );
    assert_eq!(
        parse_filter_groups("bolt, torque")[0].filters,
        ["bolt", "torque"]
    );
    let mut c = model::SearchToolConfig {
        filters_text: "torque\\s+\\d+".into(),
        use_regex: true,
        ..Default::default()
    };
    assert_eq!(model::regex_validation_error(&c), None);
    c.filters_text = "torque(".into();
    assert!(
        model::regex_validation_error(&c).is_some(),
        "the check the guide relies on catches a broken pattern"
    );
}

#[test]
fn the_page_puts_its_options_in_bar_popovers_and_the_file_types_in_a_modal() {
    let mut s = SearchSession::in_memory();
    let doc = s.document().unwrap();
    let labels: Vec<&str> = doc.panels.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["Options", "Report", "Projects"]);
    assert_eq!(doc.panels[0].badge.as_deref(), Some("index off"));
    let options: Vec<String> = doc.panels[0]
        .cards
        .iter()
        .flat_map(|c| c.items.iter())
        .filter_map(|i| i.plain_text())
        .collect();
    assert!(
        options.iter().any(|r| r.starts_with("Use regex"))
            && options.iter().any(|r| r.starts_with("Exclude folders")),
        "{options:?}"
    );
    assert!(doc.inputs[1].help.is_some(), "Find has its guide");
    assert!(doc.modal.is_none());
    s.st.extension_picker = crate::toolboxes::search::extension_picker::ExtensionPicker::open_with(
        vec![".pdf".into()],
        None,
    );
    let modal = s
        .document()
        .unwrap()
        .modal
        .expect("an open picker is a modal");
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
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "the scan did not come back"
        );
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    let md =
        s.st.extension_picker
            .available
            .iter()
            .position(|e| e == ".md")
            .unwrap();
    act_on(&mut s, SearchAct::ToggleExtension(md));
    act_on(&mut s, SearchAct::CloseExtensions);
    assert_eq!(
        s.st.config.selected_extensions,
        Some(vec![".md".to_string()])
    );
    assert!(s
        .document()
        .unwrap()
        .chips
        .iter()
        .any(|c| c.label == "Types: .md"));
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
    let chip = doc
        .chips
        .iter()
        .find(|c| c.label == "Exclude")
        .expect("the Exclude switch is under the bar");
    assert_eq!(chip.checked, Some(false));
    let options: Vec<String> = doc.panels[0]
        .cards
        .iter()
        .flat_map(|c| c.items.iter())
        .filter_map(|i| i.plain_text())
        .collect();
    assert!(
        !options
            .iter()
            .any(|r| r.starts_with("Exclude filters") || r.starts_with("Exclude scope")),
        "not behind Options: {options:?}"
    );
    act_on(&mut s, SearchAct::ToggleExclude);
    let row = s.document().unwrap().extra_inputs.remove(0);
    assert_eq!(row.id, INPUT_EXCLUDE);
    assert!(row.help.is_some(), "Exclude has its own guide");
    let labels: Vec<(&str, Option<bool>)> = row
        .choices
        .iter()
        .map(|c| (c.label.as_str(), c.checked))
        .collect();
    assert_eq!(
        labels,
        [("Whole file", Some(false)), ("Matching lines", Some(true))]
    );
    s.document_field(INPUT_EXCLUDE, "draft".into());
    assert!(s.document().unwrap().extra_inputs[0]
        .note
        .as_deref()
        .unwrap()
        .starts_with("Ignores the lines"));
    // Lines only: a.txt still matches on its other line.
    s.document_field(INPUT_FOLDER, dir.path().to_string_lossy().into_owned());
    s.document_field(INPUT_FIND, "bolt".into());
    act_on(&mut s, SearchAct::Run);
    wait(&mut s);
    let mut names: Vec<&str> =
        s.st.run
            .results
            .iter()
            .map(|r| file_name(&r.full_name))
            .collect();
    names.sort();
    assert_eq!(names, ["a.txt", "b.txt"]);
    assert!(s
        .st
        .run
        .results
        .iter()
        .find(|r| r.full_name.ends_with("a.txt"))
        .unwrap()
        .hits
        .iter()
        .all(|h| !h.match_line.contains("draft")));
    // Whole file: a.txt is left out.
    act_on(&mut s, SearchAct::ExcludeScope(ExcludeScope::File));
    assert_eq!(s.st.config.exclude_scope, ExcludeScope::File);
    assert!(s.document().unwrap().extra_inputs[0]
        .note
        .as_deref()
        .unwrap()
        .starts_with("Leaves out every file"));
    act_on(&mut s, SearchAct::Run);
    wait(&mut s);
    let names: Vec<&str> =
        s.st.run
            .results
            .iter()
            .map(|r| file_name(&r.full_name))
            .collect();
    assert_eq!(names, ["b.txt"]);
    // Hidden while it still applies: the switch names what is left out.
    act_on(&mut s, SearchAct::ToggleExclude);
    let doc = s.document().unwrap();
    assert!(doc.extra_inputs.is_empty());
    let chip = doc
        .chips
        .iter()
        .find(|c| c.label.starts_with("Exclude"))
        .unwrap();
    assert_eq!(
        (chip.label.as_str(), chip.checked),
        ("Exclude: draft", Some(true))
    );
    assert!(
        s.status().contains("still leaving out draft"),
        "{}",
        s.status()
    );
    // A session whose settings already exclude something opens with the row shown.
    let mut st = SearchToolState::default();
    st.config.exclude_filters_text = "obsolete".into();
    let s = SearchSession::with_state(st, false).unwrap();
    assert_eq!(s.document().unwrap().extra_inputs.len(), 1);
}

#[test]
fn every_example_of_the_exclude_guide_is_a_valid_exclusion() {
    let help = view::exclude_help();
    let examples: Vec<(String, String)> = help
        .sections
        .iter()
        .flat_map(|s| {
            s.rows
                .iter()
                .filter_map(|r| r.example.clone().map(|e| (r.code.clone(), e)))
        })
        .collect();
    assert!(examples.len() >= 4, "the guide offers examples to try");
    for (code, ex) in &examples {
        assert!(!model::parse_list(ex).is_empty(), "{code}: {ex}");
        let c = model::SearchToolConfig {
            filters_text: "bolt".into(),
            exclude_filters_text: ex.clone(),
            use_regex: code.contains('\\'),
            ..Default::default()
        };
        assert_eq!(model::regex_validation_error(&c), None, "{code}: {ex}");
    }
    assert_eq!(
        model::parse_list("draft, obsolete"),
        ["draft", "obsolete"],
        "commas separate the terms the guide says"
    );
    assert_eq!(
        model::parse_list("do not use"),
        ["do not use"],
        "a phrase is one term"
    );
}

#[cfg(feature = "semantic-search")]
#[test]
fn the_page_has_only_options_and_report_panels_and_one_find_input() {
    let session = SearchSession::in_memory();
    let doc = view::document(&session);
    let labels: Vec<&str> = doc.panels.iter().map(|panel| panel.label.as_str()).collect();
    assert_eq!(labels, ["Options", "Report", "Projects"]);
    assert!(doc.panels.iter().all(|panel| panel.inputs.iter().all(|input| input.id <= INPUT_OUTPUT)));
    assert!(doc.insight.is_none(), "nothing found yet");
}

#[cfg(all(feature = "semantic-search", not(feature = "nl-find")))]
#[test]
fn a_build_without_natural_language_find_says_so_in_the_index_panel() {
    let session = SearchSession::in_memory();
    let doc = view::document(&session);
    let texts: Vec<String> = doc.panels[1]
        .cards
        .iter()
        .flat_map(|card| card.items.iter().filter_map(DockItem::plain_text))
        .collect();
    assert!(texts.iter().any(|t| t.contains("not in this build")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("--features nl-find")), "{texts:?}");
}

#[test]
fn the_options_panel_reports_build_progress_for_its_border() {
    let mut session = SearchSession::in_memory();
    let progress = |session: &SearchSession| view::document(session).panels[0].progress;
    assert_eq!(progress(&session), None);
    session.building_for_snapshot(60.0);
    assert_eq!(progress(&session), Some(0.6));
    session.building_for_snapshot(0.0);
    assert_eq!(
        progress(&session),
        Some(0.02),
        "a started build always shows a sliver"
    );
}

#[test]
fn the_find_as_chip_cycles_the_pinned_reading() {
    let mut session = SearchSession::in_memory();
    let label = |session: &SearchSession| view::document(session).chips[0].label.clone();
    assert_eq!(label(&session), "Find as: auto");
    for expected in ["Find as: words", "Find as: sentence", "Find as: auto"] {
        act_on(&mut session, SearchAct::FindAs);
        assert_eq!(label(&session), expected);
    }
    act_on(&mut session, SearchAct::FindAs);
    assert_eq!(view::document(&session).chips[0].checked, Some(true), "a pinned reading is lit");
}

#[cfg(feature = "semantic-search")]
#[test]
fn a_words_find_with_a_built_semantic_index_also_shows_cited_passages() {
    let dir = tempfile::tempdir().unwrap();
    semantic_fixture(dir.path(), &[("shaft.txt", "The shaft rotates under torque and torsional load.")]);
    let mut session = SearchSession::in_memory();
    session.st.config.search_path = dir.path().to_string_lossy().into_owned();
    session.st.config.index.semantic = true;
    session.st.index_run.docs = Some(1);
    act_on(&mut session, SearchAct::FindAs); // pin "words": the text is searched as typed
    find_and_wait(&mut session, "shaft rotates under torque");
    assert_eq!(session.st.run.results.len(), 1, "the exact search still runs");
    let doc = view::document(&session);
    assert!(
        insight_items(&doc).iter().any(|item| matches!(item, DockItem::Heading(h) if h.starts_with("Possible evidence for"))),
        "{:?}",
        insight_items(&doc).iter().filter_map(DockItem::plain_text).collect::<Vec<_>>()
    );
    assert!(insight_items(&doc).iter().filter_map(DockItem::plain_text).any(|t| t.contains("shaft.txt")));
}

#[cfg(feature = "semantic-search")]
#[test]
fn a_new_find_clears_the_previous_insight() {
    let dir = tempfile::tempdir().unwrap();
    semantic_fixture(dir.path(), &[("shaft.txt", "The shaft rotates under torque and torsional load.")]);
    let mut session = SearchSession::in_memory();
    session.st.config.search_path = dir.path().to_string_lossy().into_owned();
    session.st.config.index.semantic = true;
    session.st.index_run.docs = Some(1);
    act_on(&mut session, SearchAct::FindAs); // pin "words"
    find_and_wait(&mut session, "shaft rotates under torque");
    assert!(view::document(&session).insight.is_some(), "the passages card is shown");
    session.st.index_run.docs = None;
    find_and_wait(&mut session, "torque, bolt");
    assert!(session.current_semantic_hits().is_none(), "the old passages are gone");
    assert!(view::document(&session).insight.is_none(), "words without an index leave no card");
}

#[test]
fn a_typed_question_is_an_ordinary_find() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("q.txt"), "what is torque?\nbolt\n").unwrap();
    let mut session = SearchSession::in_memory();
    session.st.config.search_path = dir.path().to_string_lossy().into_owned();
    session.document_field(INPUT_FIND, "what is torque?".into());
    assert!(view::document(&session).buttons[0].enabled, "a question needs a folder like any Find");
    act_on(&mut session, SearchAct::Run);
    wait(&mut session);
    assert_eq!(session.st.run.results.len(), 1, "the words are searched as typed");
    assert!(view::document(&session).insight.is_none(), "no answer card");
    // Without a folder it says what is missing, as any Find does.
    session.st.config.search_path.clear();
    assert!(!view::document(&session).buttons[0].enabled);
}

#[cfg(feature = "nl-find")]
mod natural_language_find {
    use super::*;

    fn wait(session: &mut SearchSession) {
        let started = Instant::now();
        while session.nl.stage != nl::Stage::Idle {
            assert!(started.elapsed() < Duration::from_secs(60));
            session.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        session.poll();
    }

    fn ready_session(dir: &Path) -> SearchSession {
        let mut session = SearchSession::in_memory();
        session.st.config.search_path = dir.to_string_lossy().into_owned();
        session.st.config.index.semantic = true;
        session
    }

    fn texts(session: &SearchSession) -> Vec<String> {
        nl::items(session).iter().filter_map(|i| i.plain_text()).collect()
    }

    #[test]
    fn understanding_needs_the_semantic_index_and_a_sentence() {
        let mut session = SearchSession::in_memory();
        session.document_field(INPUT_NL, "corrosion is blended".into());
        act_on(&mut session, SearchAct::NlUnderstand);
        assert!(session.nl.conditions.is_none());
        assert!(session.status.contains("Semantic index"), "{}", session.status);
        session.st.config.index.semantic = true;
        act_on(&mut session, SearchAct::NlUnderstand);
        assert_eq!(session.nl.conditions.as_ref().unwrap()[0].text, "corrosion blended");
        session.document_field(INPUT_NL, "find the files where it is".into());
        act_on(&mut session, SearchAct::NlUnderstand);
        assert!(session.nl.conditions.is_none() && session.nl.error.is_some());
    }

    #[test]
    fn a_sentence_in_the_search_box_is_read_as_conditions_not_searched_as_one_phrase() {
        assert!(nl::looks_like_sentence("surfaces that were blended and filled and fair"));
        assert!(!nl::looks_like_sentence("blended, filled, fair"));
        assert!(!nl::looks_like_sentence("torque and bolt"));
        assert!(!nl::looks_like_sentence("\"blended and filled and fair surfaces\" extra"));
        let dir = tempfile::tempdir().unwrap();
        let mut session = ready_session(dir.path());
        session.st.config.filters_text = "surfaces that were blended and filled and fair".into();
        act_on(&mut session, SearchAct::Run);
        assert!(!session.st.run.is_running, "no literal phrase search starts");
        assert_eq!(session.nl.conditions.as_ref().unwrap().len(), 3);
        // Without the semantic index the entry stays a keyword search.
        let mut plain = SearchSession::in_memory();
        plain.st.config.filters_text = "surfaces that were blended and filled and fair".into();
        assert!(!plain.nl_ready());
    }

    #[test]
    fn unconfirmed_closest_passages_are_labelled_as_such() {
        use search_core::nl_find::{Evidence, Fit, FileMatch};
        use search_core::semantic_index::SemanticHit;
        let mut session = SearchSession::in_memory();
        let hit = SemanticHit {
            path: "/docs/manual.pdf".into(),
            first_line: 3,
            last_line: 4,
            excerpt: "Bolts are identified by head markings.".into(),
            similarity: 0.5,
            page: None,
        };
        session.nl.conditions = Some(vec![search_core::nl_find::Condition {
            text: "fasteners size specified".into(),
            terms: vec!["fastener".into()],
            negated: false,
        }]);
        session.nl.files = vec![FileMatch {
            path: hit.path.clone(),
            fit: Fit::Unconfirmed,
            evidence: vec![Some(Evidence { hit, quote: "Bolts are identified by head markings.".into() })],
        }];
        let shown = texts(&session);
        assert!(shown.iter().any(|t| t.contains("No passage states every condition")), "{shown:?}");
        assert!(shown.iter().any(|t| t.contains("manual.pdf") && t.contains("not confirmed")), "{shown:?}");
        assert!(shown.iter().any(|t| t.contains("?  fasteners size specified")), "{shown:?}");
        assert!(!shown.iter().any(|t| t.contains("Each quote is copied")), "{shown:?}");
    }

    #[test]
    fn chips_can_be_removed_and_merged_then_find_cites_confirmed_files() {
        let dir = tempfile::tempdir().unwrap();
        semantic_fixture(
            dir.path(),
            &[
                ("blend.txt", "Corrosion was blended out and the surface filled and faired."),
                ("paint.txt", "The panel was painted and the torque checked."),
            ],
        );
        let mut session = ready_session(dir.path());
        session.document_field(INPUT_NL, "corrosion blended and surface filled and bogus extra".into());
        act_on(&mut session, SearchAct::NlUnderstand);
        assert_eq!(session.nl.conditions.as_ref().unwrap().len(), 3);

        act_on(&mut session, SearchAct::NlRemove(2));
        act_on(&mut session, SearchAct::NlMerge(0));
        let conditions = session.nl.conditions.as_ref().unwrap();
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].text, "corrosion blended surface filled");

        act_on(&mut session, SearchAct::NlFind);
        wait(&mut session);
        assert_eq!(session.nl.files.len(), 1);
        assert!(session.nl.files[0].path.ends_with("blend.txt"));
        assert!(texts(&session).iter().any(|t| t.contains("blend.txt · all matched")));
    }

    #[test]
    fn editing_the_sentence_discards_stale_conditions_and_results() {
        let dir = tempfile::tempdir().unwrap();
        semantic_fixture(dir.path(), &[("a.txt", "Corrosion was blended.")]);
        let mut session = ready_session(dir.path());
        session.document_field(INPUT_NL, "corrosion blended".into());
        act_on(&mut session, SearchAct::NlUnderstand);
        assert!(session.nl.conditions.is_some());
        session.document_field(INPUT_NL, "something else".into());
        assert!(session.nl.conditions.is_none() && session.nl.files.is_empty());
    }
}

/// Probe over a folder: `FEA_CORPUS=<folder> FEA_SENTENCE=<sentence>
/// cargo test -p app-tui --features nl-find corpus_probe -- --ignored --nocapture`.
#[cfg(feature = "nl-find")]
#[test]
#[ignore]
fn corpus_probe() {
    let folder = std::env::var("FEA_CORPUS").expect("FEA_CORPUS");
    let mut session = SearchSession::in_memory();
    session.st.config.search_path = folder;
    session.st.config.index.semantic = true;
    let spin = |session: &mut SearchSession, busy: &dyn Fn(&SearchSession) -> bool| {
        while busy(session) {
            session.poll();
            std::thread::sleep(Duration::from_millis(50));
        }
        session.poll();
    };
    let t = Instant::now();
    act_on(&mut session, SearchAct::BuildIndex);
    spin(&mut session, &|s| s.st.index_run.is_building);
    eprintln!("index built in {:?}: {}", t.elapsed(), session.status);

    if let Ok(queries) = std::env::var("FEA_QUERIES") {
        let settings = crate::toolboxes::search::model::build_settings(&session.st.config);
        let index = crate::toolboxes::search::indexing::index_directory(
            session.st.config.index.location,
            &session.st.config.search_path,
            &session.st.config.output_folder,
        );
        for query in queries.split('|') {
            eprintln!("RAW {query:?}");
            let hits = search_core::semantic_index::search_directory_with_settings(&index, query, 8, &settings);
            for hit in hits.unwrap_or_default() {
                eprintln!("  [{:.2}] {}:{} {}", hit.similarity, hit.path.rsplit('/').next().unwrap_or(""), hit.first_line, hit.excerpt.replace('\n', " ").chars().take(110).collect::<String>());
            }
        }
        return;
    }
    let sentence = std::env::var("FEA_SENTENCE").unwrap_or_default();
    if !sentence.is_empty() {
        session.st.config.filters_text = sentence.clone();
        let t = Instant::now();
        act_on(&mut session, SearchAct::Run);
        eprintln!("parsed in {:?}: {:?}", t.elapsed(), session.nl.conditions);
        for condition in session.nl.conditions.clone().unwrap_or_default() {
            let settings = crate::toolboxes::search::model::build_settings(&session.st.config);
            let index = crate::toolboxes::search::indexing::index_directory(
                session.st.config.index.location,
                &session.st.config.search_path,
                &session.st.config.output_folder,
            );
            let hits = search_core::semantic_index::search_directory_with_settings(&index, &condition.text, 6, &settings);
            eprintln!("RETRIEVE {:?}", condition.text);
            for hit in hits.unwrap_or_default() {
                eprintln!("  [{:.2}] {}:{} {}", hit.similarity, hit.path.rsplit('/').next().unwrap_or(""), hit.first_line, hit.excerpt.replace('\n', " ").chars().take(150).collect::<String>());
            }
        }
        if std::env::var("FEA_RETRIEVE_ONLY").is_ok() {
            return;
        }
        let t = Instant::now();
        act_on(&mut session, SearchAct::NlFind);
        spin(&mut session, &|s| s.nl.stage != nl::Stage::Idle);
        eprintln!("find in {:?}, error {:?}", t.elapsed(), session.nl.error);
        for file in &session.nl.files {
            eprintln!("{} fit {:?}", file.path, file.fit);
            for e in file.evidence.iter().flatten() {
                eprintln!("    [{:.2}] {}", e.hit.similarity, e.quote);
            }
        }
    }
}

/// "Keep index current": a real watcher on a temp folder updates the index for just the changed files, without
/// a walk, and leaves the settings file alone (the persist effect is never produced on this path).
#[test]
fn keep_index_current_updates_the_index_for_changed_files_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("a.txt"), "alpha first").unwrap();
    std::fs::write(root.join("b.txt"), "bravo stays").unwrap();
    let mut s = SearchSession::in_memory();
    s.st.config.search_path = root.to_string_lossy().into_owned();
    s.st.config.index.enabled = true;
    s.st.config.index.watch = true;
    let spin = |s: &mut SearchSession, busy: &dyn Fn(&SearchSession) -> bool| {
        let started = Instant::now();
        while busy(s) {
            assert!(started.elapsed() < Duration::from_secs(60), "timed out: {}", s.status);
            s.poll();
            std::thread::sleep(Duration::from_millis(50));
        }
        s.poll();
    };
    // No watcher before a first build, and none while the session is a test session (persist off).
    s.poll();
    assert!(s.watcher.is_none());
    act_on(&mut s, SearchAct::BuildIndex);
    spin(&mut s, &|s| s.st.index_run.is_building);
    assert_eq!(s.st.index_run.docs, Some(2));

    s.persist = true; // lets the watcher start; nothing below saves settings
    s.poll();
    assert!(s.watcher.is_some(), "{}", s.status);
    std::thread::sleep(Duration::from_millis(400));
    std::fs::write(root.join("a.txt"), "alpha second edit, longer").unwrap();
    std::fs::write(root.join("c.txt"), "charlie is new").unwrap();
    std::fs::remove_file(root.join("b.txt")).unwrap();

    let started = Instant::now();
    while s.st.index_run.docs != Some(2) || !s.status.contains("Updating") && started.elapsed() < Duration::from_secs(30) {
        if started.elapsed() > Duration::from_secs(30) {
            break;
        }
        s.poll();
        std::thread::sleep(Duration::from_millis(100));
        if s.status.contains("Updating the index") {
            break;
        }
    }
    assert!(s.status.contains("Updating the index"), "watcher never fired: {}", s.status);
    spin(&mut s, &|s| s.st.index_run.is_building);
    s.persist = false;

    let index_dir = crate::toolboxes::search::indexing::index_directory(s.st.config.index.location, &s.st.config.search_path, "");
    let engine = native_search::engine::NativeSearchEngine::open_or_create(&index_dir).unwrap();
    let mut ids: Vec<String> = engine.all_document_ids().unwrap().into_iter().map(|p| p.rsplit('/').next().unwrap().to_string()).collect();
    ids.sort();
    assert_eq!(ids, ["a.txt", "c.txt"], "edit kept, new added, removed gone");
    let hits = search_core::native_index::search(&engine, "charlie", 5).unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn related_files_note_lists_reasons_and_marks_the_inferred_ones() {
    use search_core::catalog_edges::Reason;
    let mut s = with_results();
    s.selected = Some(0);
    let path = s.st.run.results[0].full_name.clone();
    s.preview = Some(Preview {
        generation: s.preview_generation,
        path,
        text: None,
        status: String::new(),
        related: vec![
            Reason { other: "/w/a/copy.pdf".into(), text: "Identical text to copy.pdf".into(), inferred: false },
            Reason { other: "/w/b/x.pdf".into(), text: "Shares 2 entities with x.pdf, e.g. AN3-5A".into(), inferred: true },
        ],
        #[cfg(feature = "neural-search")]
        entities: None,
    });
    let detail = view::document(&s).detail.unwrap();
    let note = detail.notes.iter().find(|n| n.text == "Related files").expect("related note");
    assert_eq!(note.hint, "Identical text to copy.pdf\nShares 2 entities with x.pdf, e.g. AN3-5A (inferred)");
    let actions = &detail.actions;
    let labels: Vec<&str> = actions.iter().map(|a| a.label.as_str()).filter(|l| l.starts_with("Open ")).collect();
    assert_eq!(labels, ["Open copy.pdf", "Open x.pdf"]);
    s.preview.as_mut().unwrap().related.clear();
    let detail = view::document(&s).detail.unwrap();
    assert!(detail.notes.iter().all(|n| n.text != "Related files"));
}

#[test]
fn opening_a_related_file_that_is_not_listed_only_reports_it() {
    let mut s = with_results();
    s.act_search(SearchAct::OpenRelatedFile(3));
    assert_eq!(s.status, "That related file is no longer listed.");
}

#[cfg(feature = "semantic-search")]
#[test]
fn passages_from_several_files_are_briefed_by_folder() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("a")).unwrap();
    std::fs::create_dir_all(dir.path().join("b")).unwrap();
    semantic_fixture(
        dir.path(),
        &[
            ("a/x.txt", "The shaft rotates under torque and torsional load."),
            ("a/y.txt", "Torque on the rotating shaft causes torsional shear stress."),
            ("b/z.txt", "A rotating shaft transmits torque through its cross section."),
        ],
    );
    let mut session = SearchSession::in_memory();
    session.st.config.search_path = dir.path().to_string_lossy().into_owned();
    session.st.config.index.semantic = true;
    session.st.index_run.docs = Some(3);
    act_on(&mut session, SearchAct::FindAs);
    find_and_wait(&mut session, "shaft torque");
    let doc = view::document(&session);
    let texts: Vec<String> = insight_items(&doc).iter().filter_map(DockItem::plain_text).map(|t| t.to_string()).collect();
    assert!(texts.iter().any(|t| t == "3 passages from 3 files in 2 folders"), "{texts:?}");
    let folder_a = dir.path().join("a").to_string_lossy().into_owned();
    let folder_b = dir.path().join("b").to_string_lossy().into_owned();
    assert!(texts.iter().any(|t| *t == folder_a) && texts.iter().any(|t| *t == folder_b), "{texts:?}");
}

fn two_projects() -> Result<Vec<search_core::catalog_projects::ProjectSuggestion>, String> {
    let p = |folder: &str, files, linked, evidence: &[&str]| search_core::catalog_projects::ProjectSuggestion {
        folder: folder.into(),
        files,
        linked,
        cohesion: linked as f64 / files as f64,
        evidence: evidence.iter().map(|e| e.to_string()).collect(),
    };
    Ok(vec![p("/w/work/alpha", 7, 7, &["QX-11-100", "3 versioned files"]), p("/w/old/beta", 5, 4, &[])])
}

#[test]
fn project_suggestions_wait_for_a_click_and_only_a_click_sets_the_folder() {
    let mut s = SearchSession::in_memory();
    s.projects_source = two_projects;
    s.st.config.search_path = "/w/other".into();
    let lines = |s: &SearchSession| -> Vec<String> {
        let doc = view::document(s);
        let panel = doc.panels.iter().find(|p| p.label == "Projects").expect("Projects panel");
        panel.cards[0]
            .items
            .iter()
            .filter_map(|i| match i {
                DockItem::Line { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    };
    // Nothing is looked up, and nothing offered, before the click.
    assert!(lines(&s).iter().all(|l| !l.contains("alpha")));
    s.act_search(SearchAct::FindProjects);
    let started = Instant::now();
    while s.projects.is_none() {
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        s.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let shown = lines(&s);
    assert!(shown.contains(&"alpha · in work · 7 of 7 files linked".to_string()), "{shown:?}");
    assert!(shown.contains(&"Shared: QX-11-100, 3 versioned files".to_string()));
    assert!(shown.contains(&"beta · in old · 4 of 5 files linked".to_string()));
    assert_eq!(s.st.config.search_path, "/w/other", "suggestions never change the folder by themselves");
    s.act_search(SearchAct::UseProject(1));
    assert_eq!(s.st.config.search_path, "/w/old/beta");
    s.act_search(SearchAct::UseProject(9));
    assert_eq!(s.st.config.search_path, "/w/old/beta");
    assert_eq!(s.status, "That project suggestion is no longer listed.");
}

#[test]
fn project_suggestions_report_an_empty_catalog_and_errors_plainly() {
    let mut s = SearchSession::in_memory();
    s.projects_source = || Ok(Vec::new());
    s.act_search(SearchAct::FindProjects);
    while s.projects.is_none() {
        s.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(s.status, "No project stands out in the indexed folders yet.");
    s.projects = None;
    s.projects_source = || Err("catalog locked".into());
    s.act_search(SearchAct::FindProjects);
    while s.projects.is_none() {
        s.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(s.status, "Project suggestions unavailable: catalog locked");
}
