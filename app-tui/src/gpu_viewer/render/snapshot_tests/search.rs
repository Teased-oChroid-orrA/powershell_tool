//! Search window snapshots.
use super::*;

/// The Search workbench after a real search (dark, light, a narrow window) and while one runs, as raw RGBA dumps:
/// `FEA_SNAPSHOT_SEARCH_OUT=/dir cargo test ... search_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_SEARCH_OUT"]
fn search_snapshot() {
    use crate::gpu_viewer::editor::search::{SearchAct, SearchSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_SEARCH_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let data = tempfile::tempdir().unwrap();
    let root = data.path().join("Wing Project");
    std::fs::create_dir_all(root.join("specs/fasteners")).unwrap();
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(root.join("specs/fasteners/hi-lok.txt"), "Fastener schedule\nHi-Lok HL18 installed with torque 40 in-lb\nsealant applied\nCheck torque after cure\nend\n").unwrap();
    std::fs::write(
        root.join("specs/wing-box.md"),
        "# Wing box\nSpar cap bolt torque per drawing\nrib spacing 20 in\n",
    )
    .unwrap();
    std::fs::write(
        root.join("notes/review.txt"),
        "Design review\nopen item: verify bolt torque on fitting\nclosed\n",
    )
    .unwrap();
    std::fs::write(root.join("notes/todo.txt"), "nothing relevant here\n").unwrap();
    let mut session = SearchSession::in_memory();
    session.document_field(0, root.to_string_lossy().into_owned());
    session.document_field(1, "torque, bolt".into());
    session.act(EditorAction::Search(SearchAct::Run));
    let t = std::time::Instant::now();
    while session.solving() || session.state().run.results.len() < 3 {
        session.poll();
        assert!(t.elapsed().as_secs() < 60, "the search did not finish");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    session.poll();
    session.act(EditorAction::Search(SearchAct::Select(0)));
    let shoot_with = |session: &SearchSession,
                      name: &str,
                      dark: bool,
                      (w, h): (u32, u32),
                      popover: Option<&str>,
                      layout: Option<crate::gpu_viewer::display::PageLayout>| {
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let s = session as &dyn Authoring;
            let scene = Scene {
                label: "s".into(),
                positions: vec![[0.0; 3]; 3],
                triangles: vec![0, 1, 2],
                samples: vec![[0.0; 4]; 3],
                times: vec![0.0],
                range: [0.0, 1.0],
                gain: 1.0,
                harmonic_period: None,
                field_label: String::new(),
                fields: Vec::new(),
            };
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
                .await
                .unwrap();
            let texture = target(&r, w, h);
            // FEA_SNAPSHOT_HOUR=0..24 shoots the circadian palette at that local hour (light or dark follows it).
            if let Some(h) = std::env::var("FEA_SNAPSHOT_HOUR")
                .ok()
                .and_then(|v| v.parse().ok())
            {
                crate::gpu_viewer::gui::theme::set_hour(Some(h));
            }
            // FEA_SNAPSHOT_THEME=<library id> shoots a fixed library theme instead.
            if let Ok(id) = std::env::var("FEA_SNAPSHOT_THEME") {
                crate::gpu_viewer::gui::theme::choose(
                    crate::gpu_viewer::gui::theme_library::LIBRARY
                        .iter()
                        .position(|t| t.id == id),
                );
            }
            let mut gui = Gui::new();
            gui.ctx().set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let mut view = ViewState::new(&scene);
            view.page_layout = layout;
            let scenes = [scene];
            let input = GuiInput {
                view: &view,
                scenes: &scenes,
                selected: 0,
                editor: Some(s),
                error: None,
                box_drag: None,
                overlay: &[],
            };
            gui.select_tab_named(&input, "Results");
            if let Some(k) = popover {
                gui.open_doc_popover(k);
            }
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    };
    let shoot = |session: &SearchSession, name: &str, dark: bool, size: (u32, u32)| {
        shoot_with(session, name, dark, size, None, None)
    };
    shoot_with(
        &session,
        "search-guide",
        true,
        (1440, 900),
        Some("help:1"),
        None,
    );
    shoot_with(
        &session,
        "search-options",
        false,
        (1440, 900),
        Some("Options"),
        None,
    );
    shoot_with(
        &session,
        "search-report",
        true,
        (1440, 900),
        Some("Report"),
        None,
    );
    {
        let mut projects = SearchSession::in_memory();
        projects.gaps_for_snapshot(|| {
            use search_core::catalog_gaps::{Gap, GapKind};
            Ok(vec![
                Gap {
                    kind: GapKind::MissingDocument,
                    subject: "AMS 5643".into(),
                    detail: "cited in 3 files; no file name contains it".into(),
                    files: vec!["/Work/Aircraft/Lug-Study/plan0.txt".into()],
                    support: 3,
                },
                Gap {
                    kind: GapKind::DroppedIdentifier,
                    subject: "P/N 4410-102".into(),
                    detail: "in all 3 earlier versions, not in the newest, Bracket_v4.docx".into(),
                    files: vec!["/Work/Aircraft/Bracket-4410/Bracket_v4.docx".into()],
                    support: 3,
                },
            ])
        });
        projects.projects_for_snapshot(|| {
            let p = |folder: &str, files, linked, evidence: &[&str]| {
                search_core::catalog_projects::ProjectSuggestion {
                    folder: folder.into(),
                    files,
                    linked,
                    cohesion: linked as f64 / files as f64,
                    evidence: evidence.iter().map(|e| e.to_string()).collect(),
                }
            };
            Ok(vec![
                p(
                    "/Work/Aircraft/Bracket-4410",
                    23,
                    21,
                    &["P/N 4410-102", "ASTM A36", "5 versioned files"],
                ),
                p(
                    "/Work/Aircraft/Lug-Study",
                    12,
                    9,
                    &["AN3-5A", "MIL-HDBK-5J"],
                ),
                p(
                    "/Archive/2024/Fixture-Redesign",
                    8,
                    5,
                    &["3 versioned files"],
                ),
            ])
        });
        shoot_with(
            &projects,
            "search-projects",
            false,
            (1440, 900),
            Some("Projects"),
            None,
        );
    }
    shoot(&session, "search-results-dark", true, (1440, 900));
    shoot(&session, "search-results-light", false, (1440, 900));
    {
        // The sidebar's narrowing: one type chosen, with its chip over the table.
        let mut narrowed = SearchSession::in_memory();
        narrowed.document_field(0, root.to_string_lossy().into_owned());
        narrowed.document_field(1, "torque, bolt".into());
        narrowed.act(EditorAction::Search(SearchAct::Run));
        let t = std::time::Instant::now();
        while narrowed.solving() || narrowed.state().run.results.len() < 3 {
            narrowed.poll();
            assert!(t.elapsed().as_secs() < 60, "the search did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        narrowed.poll();
        let txt = narrowed
            .facets()
            .types
            .iter()
            .position(|v| v.key == ".txt")
            .expect("a .txt result");
        narrowed.act(EditorAction::Search(SearchAct::NarrowType(txt)));
        narrowed.act(EditorAction::Search(SearchAct::NarrowModified(
            crate::gpu_viewer::editor::search::narrow::Modified::Month,
        )));
        shoot(&narrowed, "search-narrowed-dark", true, (1440, 900));
        shoot(&narrowed, "search-narrowed-light", false, (1440, 900));
    }
    shoot(&session, "search-narrow", true, (860, 760));
    shoot(&session, "search-short", true, (1100, 560));
    // The Exclude row, and the cards moved: matches on the left, then stacked under the list.
    session.act(EditorAction::Search(SearchAct::ToggleExclude));
    session.document_field(3, "draft, obsolete".into());
    session.act(EditorAction::Search(SearchAct::ExcludeScope(
        search_core::models::ExcludeScope::File,
    )));
    use crate::gpu_viewer::display::PageLayout;
    shoot_with(
        &session,
        "search-exclude-swapped",
        true,
        (1440, 900),
        None,
        Some(PageLayout {
            detail_first: true,
            stacked: false,
            split: 0.45,
        }),
    );
    shoot_with(
        &session,
        "search-exclude-guide",
        false,
        (1440, 900),
        Some("help:3"),
        Some(PageLayout {
            detail_first: false,
            stacked: true,
            split: 0.5,
        }),
    );
    shoot_with(
        &session,
        "search-exclude-narrow",
        true,
        (860, 760),
        None,
        None,
    );
    session.state_mut_for_snapshot();
    shoot(&session, "search-running", true, (1440, 900));
    shoot(&session, "search-running-narrow", true, (860, 760));
}

/// The Search window with the Find bar's "Related from hits" chips (terms mined from the hits, related ones folded), dark and
/// light, as raw RGBA: `FEA_SNAPSHOT_CLUSTERS_OUT=/dir cargo test --release -p app-tui --lib search_term_clusters_snapshot -- --ignored`.
#[cfg(feature = "neural-search")]
#[test]
#[ignore]
fn search_term_clusters_snapshot() {
    use crate::gpu_viewer::editor::search::{SearchAct, SearchSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_CLUSTERS_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let data = tempfile::tempdir().unwrap();
    let root = data.path().join("Lug Study");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("lug.txt"),
        "Lug outer diameter shall not exceed 1.500 in\nLug outside diameter is checked after the press fit\nLug bolt diameter 0.250 in per drawing\nLug yield strength of 7075-T6 governs the net section\nLug yield stress is taken from MMPDS\nLug fatigue life is limited by bearing stress\nLug fatigue limit from the S-N curve\n",
    )
    .unwrap();
    std::fs::write(
        root.join("notes.txt"),
        "Lug landing gear fitting sealant cure time 24 hours\nLug ultimate tensile strength verified by coupon test\n",
    )
    .unwrap();
    let mut session = SearchSession::in_memory();
    session.document_field(0, root.to_string_lossy().into_owned());
    session.document_field(1, "lug".into());
    session.act(EditorAction::Search(SearchAct::Run));
    let t = std::time::Instant::now();
    let chips = |session: &SearchSession| -> Vec<String> {
        (session as &dyn Authoring)
            .document()
            .map(|d| {
                d.inputs
                    .iter()
                    .filter(|i| i.label == "Find")
                    .flat_map(|i| i.choices.iter().map(|c| c.label.clone()))
                    .collect()
            })
            .unwrap_or_default()
    };
    // The chips are ordered in the background by the bundled model once the search has produced hits.
    while session.solving() || !session.neural_order_ready_for_snapshot() {
        session.poll();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(
            t.elapsed().as_secs() < 90,
            "the chips never arrived: {:?}",
            chips(&session)
        );
    }
    session.poll();
    eprintln!("CHIPS: {:#?}", chips(&session));
    session.act(EditorAction::Search(SearchAct::Select(0)));
    for (name, dark, (w, h)) in [
        ("search-clusters-dark", true, (1440, 900)),
        ("search-clusters-light", false, (1440, 900)),
        ("search-clusters-narrow", true, (580, 760)),
    ] {
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let scene = Scene {
                label: "s".into(),
                positions: vec![[0.0; 3]; 3],
                triangles: vec![0, 1, 2],
                samples: vec![[0.0; 4]; 3],
                times: vec![0.0],
                range: [0.0, 1.0],
                gain: 1.0,
                harmonic_period: None,
                field_label: String::new(),
                fields: Vec::new(),
            };
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
                .await
                .unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let view = ViewState::new(&scene);
            let scenes = [scene];
            let input = GuiInput {
                view: &view,
                scenes: &scenes,
                selected: 0,
                editor: Some(&session as &dyn Authoring),
                error: None,
                box_drag: None,
                overlay: &[],
            };
            gui.select_tab_named(&input, "Results");
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            // Out of focus the pills are not drawn: the Find box says how many there are.
            let idle = gui.target_labels();
            assert!(
                chips(&session).iter().all(|chip| !idle.contains(chip)),
                "the pills are drawn without focus"
            );
            std::fs::write(format!("{dir}/{name}-idle.rgba"), read_back(&r, &texture)).unwrap();
            // Focus in Find opens them under it, over the page.
            gui.ctx()
                .memory_mut(|m| m.request_focus(egui::Id::new(("doc-input", 1u8))));
            for _ in 0..2 {
                draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            }
            let drawn = gui.target_labels();
            for chip in chips(&session) {
                assert!(drawn.contains(&chip), "chip {chip:?} was not drawn");
            }
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    }
}

/// The Search window before a search, and after a words search with cited semantic passages (dark and light), as raw RGBA:
/// `FEA_SNAPSHOT_PASSAGES_OUT=/dir cargo test ... --features neural-search,semantic-search passages_snapshot -- --ignored`.
#[cfg(feature = "semantic-search")]
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_PASSAGES_OUT"]
fn passages_snapshot() {
    use crate::gpu_viewer::editor::search::{semantic_fixture, SearchSession};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_PASSAGES_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("design")).unwrap();
    std::fs::create_dir_all(data.path().join("reports")).unwrap();
    semantic_fixture(
        data.path(),
        &[
            (
                "design/torque.txt",
                "Applied torque causes torsional stress in the shaft.",
            ),
            (
                "design/limits.txt",
                "Shaft torque must stay under the rated torsional limit.",
            ),
            (
                "reports/temperature.txt",
                "The shaft temperature limit is 120 C.",
            ),
            (
                "reports/test-17.txt",
                "Torque on the shaft was raised until torsional shear stress appeared.",
            ),
        ],
    );
    let shoot = |session: &SearchSession, name: &str, dark: bool, open: Option<&str>| {
        pollster::block_on(async {
            let (w, h) = (1440, 900);
            let adapter = test_adapter().await.expect("adapter required");
            let s = session as &dyn Authoring;
            let scene = Scene {
                label: "s".into(),
                positions: vec![[0.0; 3]; 3],
                triangles: vec![0, 1, 2],
                samples: vec![[0.0; 4]; 3],
                times: vec![0.0],
                range: [0.0, 1.0],
                gain: 1.0,
                harmonic_period: None,
                field_label: String::new(),
                fields: Vec::new(),
            };
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
                .await
                .unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let view = ViewState::new(&scene);
            let scenes = [scene];
            let input = GuiInput {
                view: &view,
                scenes: &scenes,
                selected: 0,
                editor: Some(s),
                error: None,
                box_drag: None,
                overlay: &[],
            };
            gui.select_tab_named(&input, "Results");
            if let Some(panel) = open {
                gui.open_doc_popover(panel);
            }
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    };
    let mut session = SearchSession::in_memory();
    shoot(&session, "passages-before-dark", true, None);
    session.words_for_snapshot(data.path(), "what causes torsional stress");
    shoot(&session, "passages-dark", true, None);
    shoot(&session, "passages-light", false, None);
    session.building_for_snapshot(60.0);
    shoot(&session, "index-building-button-dark", true, None);
    shoot(&session, "index-building-popup-dark", true, Some("Options"));
}

/// The Search window's Index popup with natural-language Find: the chips to review, then the cited results (dark and light), as raw RGBA:
/// `FEA_SNAPSHOT_NL_OUT=/dir cargo test ... --features nl-find nl_find_snapshot -- --ignored`.
#[cfg(feature = "nl-find")]
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_NL_OUT"]
fn nl_find_snapshot() {
    use crate::gpu_viewer::editor::search::{semantic_fixture, SearchSession};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_NL_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let data = tempfile::tempdir().unwrap();
    semantic_fixture(
        data.path(),
        &[
            (
                "blend.txt",
                "Corrosion was blended out and the surface was filled and faired.",
            ),
            ("paint.txt", "The panel was painted and the torque checked."),
        ],
    );
    let shoot = |session: &SearchSession, name: &str, dark: bool| {
        pollster::block_on(async {
            let (w, h) = (1440, 900);
            let adapter = test_adapter().await.expect("adapter required");
            let s = session as &dyn Authoring;
            let scene = Scene {
                label: "s".into(),
                positions: vec![[0.0; 3]; 3],
                triangles: vec![0, 1, 2],
                samples: vec![[0.0; 4]; 3],
                times: vec![0.0],
                range: [0.0, 1.0],
                gain: 1.0,
                harmonic_period: None,
                field_label: String::new(),
                fields: Vec::new(),
            };
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
                .await
                .unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let view = ViewState::new(&scene);
            let scenes = [scene];
            let input = GuiInput {
                view: &view,
                scenes: &scenes,
                selected: 0,
                editor: Some(s),
                error: None,
                box_drag: None,
                overlay: &[],
            };
            gui.select_tab_named(&input, "Results");
            gui.open_doc_popover("Index");
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    };
    let mut session = SearchSession::in_memory();
    shoot(&session, "nl-empty-dark", true);
    session.nl_for_snapshot(
        data.path(),
        "corrosion blended and surface filled and faired",
        false,
    );
    shoot(&session, "nl-chips-dark", true);
    session.nl_for_snapshot(
        data.path(),
        "corrosion blended and surface filled and faired",
        true,
    );
    shoot(&session, "nl-results-dark", true);
    shoot(&session, "nl-results-light", false);
}
