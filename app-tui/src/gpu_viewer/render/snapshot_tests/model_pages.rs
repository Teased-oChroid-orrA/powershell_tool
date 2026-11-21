//! Materials, Eccentric Bushing, body styles and editor snapshots.
use super::*;

/// The Material Lookup workbench (one material's properties, then a comparison; dark, light, a narrow window), as
/// raw RGBA dumps: `FEA_SNAPSHOT_MATERIALS_OUT=/dir cargo test ... materials_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_MATERIALS_OUT"]
fn materials_snapshot() {
    use crate::gpu_viewer::editor::materials::{
        MaterialsAct, MaterialsSession, INPUT_FLOOR0, INPUT_SEARCH,
    };
    use crate::gpu_viewer::editor::session::EditorAction;
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_MATERIALS_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut session = MaterialsSession::with_report(None);
    session.document_field(INPUT_SEARCH, "7075".into());
    let shoot = |session: &MaterialsSession, name: &str, dark: bool, (w, h): (u32, u32)| {
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let s = session as &dyn Authoring;
            let scene = Scene {
                label: "m".into(),
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
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    };
    shoot(&session, "materials-properties-dark", true, (1440, 900));
    shoot(&session, "materials-properties-light", false, (1440, 900));
    // The Refine rail and the basket: aluminum plate with a Fty floor, three marks.
    session.document_field(INPUT_SEARCH, String::new());
    let model = crate::toolboxes::material_lookup::model::groups();
    let alu = model.iter().position(|g| g.contains("Aluminum")).unwrap();
    session.act(EditorAction::Materials(MaterialsAct::Group(alu)));
    let plate = crate::toolboxes::material_lookup::model::FORMS
        .iter()
        .position(|f| *f == "Plate")
        .unwrap();
    session.act(EditorAction::Materials(MaterialsAct::Form(plate)));
    session.document_field(INPUT_FLOOR0, "60".into());
    let marks: Vec<usize> = session.state().hits.iter().take(3).copied().collect();
    for i in marks {
        session.act(EditorAction::Materials(MaterialsAct::ToggleMark(i)));
    }
    shoot(&session, "materials-facets-dark", true, (1440, 900));
    shoot(&session, "materials-facets-light", false, (1440, 900));
    session.act(EditorAction::Materials(MaterialsAct::Compare));
    shoot(&session, "materials-compare-dark", true, (1440, 900));
    shoot(&session, "materials-compare-light", false, (1440, 900));
    shoot(&session, "materials-narrow", true, (860, 760));
}

/// The Eccentric Bushing workbench before and after an analysis, as raw RGBA dumps:
/// `FEA_SNAPSHOT_ECC_OUT=/dir cargo test ... eccentric_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_ECC_OUT"]
fn eccentric_snapshot() {
    use crate::gpu_viewer::editor::eccentric::{
        file::EccentricFile, EccentricAct, EccentricSession,
    };
    use crate::gpu_viewer::editor::session::EditorAction;
    use crate::toolboxes::eccentric_bushing::model::EccentricUi;
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_ECC_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let ui = EccentricUi {
        offset: 0.04,
        ..EccentricUi::default()
    };
    let mut session = EccentricSession::new(
        &EccentricFile::new("Wing fitting offset bore", "none.json", &ui),
        std::env::temp_dir().join("ecc-snapshot.json"),
    );
    let t = std::time::Instant::now();
    let wait = |s: &mut EccentricSession, done: &dyn Fn(&EccentricSession) -> bool| {
        while !done(s) {
            s.poll();
            assert!(t.elapsed().as_secs() < 240);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    wait(&mut session, &|s| s.fea().preview_summary().is_ok());
    let shoot = |session: &mut EccentricSession, step: &str, name: &str| {
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1360u32, 860u32);
            let scene = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
                .await
                .unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(egui::Theme::Dark);
            let view = ViewState::new(&scene);
            let scenes = [scene];
            let input = GuiInput {
                view: &view,
                scenes: &scenes,
                selected: 0,
                editor: Some(session as &dyn Authoring),
                error: None,
                box_drag: None,
                overlay: &[],
            };
            gui.select_tab_named(&input, step);
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    };
    shoot(&mut session, "Inputs", "ecc-inputs");
    session.act(EditorAction::Eccentric(EccentricAct::Analyse));
    session.act(EditorAction::Eccentric(EccentricAct::Step("Analysis")));
    wait(&mut session, &|s| {
        s.state().analysis.is_some() && !s.solving()
    });
    shoot(&mut session, "Analysis", "ecc-result");
    session.show_bracket_for_snapshot();
    shoot(&mut session, "Analysis", "ecc-offset-bracket");
}

/// The un-meshed 3D body in every display style and the same body meshed, as raw RGBA dumps:
/// `FEA_SNAPSHOT_BODY_OUT=/dir cargo test ... body_styles_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_BODY_OUT"]
fn body_styles_snapshot() {
    use crate::gpu_viewer::editor::solid::{SolidAction, SolidSession};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_BODY_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut problem = fea_problem::templates::templates()
        .into_iter()
        .map(|(_, p)| p)
        .find(|p| p.analysis == fea_problem::Analysis::Solid)
        .unwrap();
    problem.mesh.size = 0.3;
    let mut session = SolidSession::new(problem, "demo-body.json".into());
    let t = std::time::Instant::now();
    while session
        .scene()
        .map_or(true, |s| !s.label.contains("body (not meshed)"))
    {
        session.poll();
        assert!(t.elapsed().as_secs() < 60, "body did not finish");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let body = session.scene().unwrap();
    session.do_action(SolidAction::GenerateMesh);
    while session.preview_summary().is_err() {
        session.poll();
        assert!(t.elapsed().as_secs() < 120, "mesh did not finish");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let meshed = session.scene().unwrap();
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (640u32, 420u32);
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &body, w, h)
            .await
            .unwrap();
        let texture = target(&r, w, h);
        let shot = |r: &mut Renderer,
                    scene: &Scene,
                    name: &str,
                    style: DisplayStyle,
                    mesh: bool,
                    bg: ([u8; 3], [u8; 3])| {
            r.set_scene(scene).unwrap();
            r.draw_display(
                &texture.create_view(&Default::default()),
                scene,
                0.0,
                1.0,
                [-0.6, -0.5, 1.5, w as f32 / h as f32],
                DisplayOptions {
                    show_mesh: mesh,
                    style,
                    shade: true,
                    background: Some(bg),
                    ..Default::default()
                },
            );
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(r, &texture)).unwrap();
        };
        let grey_white = ([112u8; 3], [255u8; 3]);
        for (k, style) in DisplayStyle::ALL.into_iter().enumerate() {
            shot(
                &mut r,
                &body,
                &format!("body-{k}"),
                style,
                false,
                grey_white,
            );
        }
        shot(
            &mut r,
            &body,
            "hl-wire",
            DisplayStyle::Wireframe,
            false,
            grey_white,
        );
        shot(
            &mut r,
            &body,
            "hl-hlr",
            DisplayStyle::HiddenLinesRemoved,
            false,
            grey_white,
        );
        shot(
            &mut r,
            &meshed,
            "meshed-wire",
            DisplayStyle::Wireframe,
            true,
            ([0; 3], [112; 3]),
        );
        shot(
            &mut r,
            &meshed,
            "meshed-shaded-edges",
            DisplayStyle::ShadedEdges,
            true,
            grey_white,
        );
    });
}

/// The editor window in each of its states (light and dark), as raw RGBA dumps:
/// `FEA_SNAPSHOT_EDITOR_OUT=/dir cargo test ... solid_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
fn solid_snapshot() {
    use crate::gpu_viewer::editor::solid::SolidSession;
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 0.4;
    let mut session = SolidSession::new(problem, "demo-solid.json".into());
    session.do_action(crate::gpu_viewer::editor::solid::SolidAction::GenerateMesh);
    let t = std::time::Instant::now();
    while session.preview_summary().is_err() {
        session.poll();
        assert!(t.elapsed().as_secs() < 60, "mesh did not finish");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    session.do_action(crate::gpu_viewer::editor::solid::SolidAction::SelectSurface(1));
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1600u32, 900u32);
        let first = session.scene().unwrap();
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &first, w, h)
            .await
            .unwrap();
        let texture = target(&r, w, h);
        let mut gui = Gui::new();
        let mut frame = |r: &mut Renderer,
                         session: &mut SolidSession,
                         name: &str,
                         mesh_lines: bool,
                         tab: Option<&str>,
                         dark: bool| {
            gui.ctx().set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let scene = session.scene().unwrap();
            r.set_scene(&scene).unwrap();
            let mut view = ViewState::new(&scene);
            view.show_mesh = mesh_lines;
            let scenes = [scene];
            let input = GuiInput {
                view: &view,
                scenes: &scenes,
                selected: 0,
                editor: Some(&*session as &dyn Authoring),
                error: None,
                box_drag: None,
                overlay: &[],
            };
            if let Some(tab) = tab {
                gui.select_tab_named(&input, tab);
            }
            draw(r, &mut gui, &texture, &input, [0.0, 0.0], mesh_lines);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(r, &texture)).unwrap();
        };
        frame(
            &mut r,
            &mut session,
            "solid-before",
            true,
            Some("Mesh"),
            true,
        );
        frame(
            &mut r,
            &mut session,
            "solid-before-light",
            true,
            Some("Model"),
            false,
        );
        session.solve();
        let t = std::time::Instant::now();
        while !session.poll() || session.solving() {
            assert!(t.elapsed().as_secs() < 120);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        frame(
            &mut r,
            &mut session,
            "solid-after",
            false,
            Some("Simulation"),
            true,
        );
        frame(
            &mut r,
            &mut session,
            "solid-after-light",
            false,
            Some("Simulation"),
            false,
        );
        session.do_action(crate::gpu_viewer::editor::solid::SolidAction::ClearModel);
        frame(
            &mut r,
            &mut session,
            "solid-empty",
            false,
            Some("Model"),
            true,
        );
        frame(
            &mut r,
            &mut session,
            "solid-empty-light",
            false,
            Some("Model"),
            false,
        );
    });
}

/// The failure-guidance card and the status-bar fix after a failed solve (ADR-020).
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
fn guidance_snapshot() {
    use crate::gpu_viewer::editor::solid::{SolidAction, SolidSession};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 0.4;
    problem.supports = vec![fea_problem::Support::roller("left", 0)];
    let mut session = SolidSession::new(problem, "demo-guidance.json".into());
    session.do_action(SolidAction::GenerateMesh);
    let t = std::time::Instant::now();
    while session.preview_summary().is_err() {
        session.poll();
        assert!(t.elapsed().as_secs() < 60, "mesh did not finish");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    session.solve();
    let t = std::time::Instant::now();
    while !session.poll() || session.solving() {
        assert!(t.elapsed().as_secs() < 120);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1600u32, 900u32);
        let first = session.scene().unwrap();
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &first, w, h)
            .await
            .unwrap();
        let texture = target(&r, w, h);
        let mut gui = Gui::new();
        for (name, dark) in [("guidance-failed", true), ("guidance-failed-light", false)] {
            gui.ctx().set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let scene = session.scene().unwrap();
            r.set_scene(&scene).unwrap();
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
            gui.select_tab_named(&input, "Simulation");
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        }
    });
}

/// The viewport while the pointer is on a guidance fix: the surface it would touch is lit, nothing is applied.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
fn fix_preview_snapshot() {
    use crate::gpu_viewer::editor::solid::{FixRef, SolidAction, SolidSession};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 0.4;
    problem.supports.clear();
    let mut session = SolidSession::new(problem, "demo-fixpreview.json".into());
    session.do_action(SolidAction::GenerateMesh);
    let t = std::time::Instant::now();
    while session.preview_summary().is_err() {
        session.poll();
        assert!(t.elapsed().as_secs() < 60, "mesh did not finish");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1600u32, 900u32);
        let first = session.scene().unwrap();
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &first, w, h)
            .await
            .unwrap();
        let texture = target(&r, w, h);
        let mut gui = Gui::new();
        gui.ctx().set_theme(egui::Theme::Dark);
        for (name, fix) in [
            ("fix-preview-off", None),
            ("fix-preview-on", Some(FixRef::Remedy(0, 0))),
        ] {
            session.do_action(SolidAction::PreviewFix(fix));
            let scene = session.scene().unwrap();
            r.set_scene(&scene).unwrap();
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
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        }
    });
}

/// Sketch mode: a constrained plate outline with a hole, a half-drawn line and a selected edge.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
fn sketch_snapshot() {
    use crate::gpu_viewer::editor::{
        sketcher::Tool,
        solid::{SketchAction, SolidAction, SolidSession},
    };
    use fea_problem::history::{FeatureKind, History, Plane};
    use fea_problem::sketch::{Constraint as C, Sketch};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut problem = fea_problem::templates::templates().remove(0).1;
    let mut sk = Sketch::default();
    let p = [
        sk.add_point(0.0, 0.0),
        sk.add_point(12.0, 0.0),
        sk.add_point(12.0, 4.0),
        sk.add_point(0.0, 4.0),
    ];
    let l = [
        sk.add_line(p[0], p[1]),
        sk.add_line(p[1], p[2]),
        sk.add_line(p[2], p[3]),
        sk.add_line(p[3], p[0]),
    ];
    for (i, id) in l.iter().enumerate() {
        sk.add_constraint(if i % 2 == 0 {
            C::Horizontal { line: *id }
        } else {
            C::Vertical { line: *id }
        });
    }
    sk.add_constraint(C::Fixed {
        p: p[0],
        x: 0.0,
        y: 0.0,
    });
    sk.add_constraint(C::Length {
        line: l[0],
        d: 12.0,
    });
    sk.add_constraint(C::Length { line: l[1], d: 4.0 });
    sk.add_circle(6.0, 2.0, 0.8);
    let mut h = History::default();
    let id = h.add(
        "Sketch1",
        FeatureKind::Sketch {
            plane: Plane::Front { offset: 0.0 },
            sketch: sk,
        },
    );
    problem.history = Some(h);
    let mut session = SolidSession::new(problem, "demo-sketch.json".into());
    session.do_action(SolidAction::Sketch(SketchAction::Edit(id)));
    let finished = std::env::var("FEA_SNAPSHOT_FINISHED").is_ok();
    if finished {
        // The model after Finish, before any mesh exists: the sketch outline stays visible.
        session.do_action(SolidAction::Sketch(SketchAction::Finish));
    } else {
        session.do_action(SolidAction::Sketch(SketchAction::Tool(Tool::Line)));
    }
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1600u32, 900u32);
        let scene = session.scene().unwrap();
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
            .await
            .unwrap();
        let texture = target(&r, w, h);
        r.set_scene(&scene).unwrap();
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
        for (dark, suffix) in [(true, ""), (false, "-light")] {
            let mut gui = Gui::new();
            gui.ctx().set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(
                format!(
                    "{dir}/{}{suffix}.rgba",
                    if finished {
                        "sketch-finished"
                    } else {
                        "sketch"
                    }
                ),
                read_back(&r, &texture),
            )
            .unwrap();
        }
    });
}

#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_EDITOR_OUT"]
fn editor_snapshot() {
    use crate::gpu_viewer::editor::{camera::Hit, caps::Operation, session::Session};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_EDITOR_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let template: usize = std::env::var("FEA_SNAPSHOT_TEMPLATE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut session = Session::new(
        fea_problem::members::templates().remove(template),
        "demo.members.json".into(),
    );
    session
        .state
        .select(Some(Hit::Node(session.state.problem.nodes.len() as u32)));
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1600u32, 900u32);
        let first = session.scene().unwrap();
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &first, w, h)
            .await
            .unwrap();
        let texture = target(&r, w, h);
        let mut gui = Gui::new();
        let mut frame = |r: &mut Renderer, session: &mut Session, name: &str| {
            let scene = session.scene().unwrap();
            r.set_scene(&scene).unwrap();
            let view = ViewState::new(&scene);
            let scenes = [scene];
            let input = GuiInput {
                view: &view,
                scenes: &scenes,
                selected: 0,
                editor: Some(&*session as &dyn Authoring),
                error: None,
                box_drag: None,
                overlay: &[],
            };
            draw(r, &mut gui, &texture, &input, [0.55, 0.35], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(r, &texture)).unwrap();
        };
        frame(&mut r, &mut session, "editor-before");
        session.run(Operation::Solve);
        let t = std::time::Instant::now();
        while !session.poll() {
            assert!(t.elapsed().as_secs() < 30);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        frame(&mut r, &mut session, "editor-after");
    });
}
