//! HUD, context menu, result legend and dashboard snapshots.
use super::*;

/// Renders a session file with the real chrome into a raw RGBA dump for visual inspection:
/// `FEA_SNAPSHOT_SCENE=scene.json FEA_SNAPSHOT_OUT=out.rgba cargo test ... -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_SCENE / FEA_SNAPSHOT_OUT"]
fn hud_snapshot() {
    let (Ok(scene_path), Ok(out)) = (
        std::env::var("FEA_SNAPSHOT_SCENE"),
        std::env::var("FEA_SNAPSHOT_OUT"),
    ) else {
        return;
    };
    let _guard = headless_guard();
    let field: usize = std::env::var("FEA_SNAPSHOT_FIELD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let project = transport::read_project(std::fs::File::open(scene_path).unwrap()).unwrap();
    let scene = &project.scenes[project.selected];
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1280u32, 800u32);
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, scene, w, h)
            .await
            .unwrap();
        let mut view = ViewState::new(scene);
        view.field = field;
        view.show_help = std::env::var("FEA_SNAPSHOT_HELP").is_ok();
        let texture = target(&r, w, h);
        let mut gui = Gui::new();
        let input = GuiInput {
            view: &view,
            scenes: &project.scenes,
            selected: project.selected,
            editor: None,
            error: None,
            box_drag: None,
            overlay: &[],
        };
        draw(&mut r, &mut gui, &texture, &input, [0.35, 0.4], true);
        std::fs::write(out, read_back(&r, &texture)).unwrap();
    });
}

/// The right-click menu of a selected face and the background panel over the editor window, as one raw RGBA dump:
/// `FEA_SNAPSHOT_MENU_OUT=/dir cargo test ... context_menu_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_MENU_OUT"]
fn context_menu_snapshot() {
    use crate::gpu_viewer::editor::solid::{SolidAction, SolidSession};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_MENU_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut problem = fea_problem::templates::templates()
        .into_iter()
        .map(|(_, p)| p)
        .find(|p| p.analysis == fea_problem::Analysis::Solid)
        .unwrap();
    problem.mesh.size = 0.3;
    let mut session = SolidSession::new(problem, "demo-menu.json".into());
    let t = std::time::Instant::now();
    while session
        .scene()
        .map_or(true, |s| !s.label.contains("body (not meshed)"))
    {
        session.poll();
        assert!(t.elapsed().as_secs() < 60, "body did not finish");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    session.do_action(SolidAction::SelectSurface(1));
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1280u32, 800u32);
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
            editor: Some(&session as &dyn Authoring),
            error: None,
            box_drag: None,
            overlay: &[],
        };
        let ctx = crate::gpu_viewer::menus::MenuContext {
            view: &view,
            scene: input.scene(),
            results: 1,
            editor: input.editor,
            visible: Default::default(),
            adaptive: true,
        };
        gui.open_context((640.0, 300.0), crate::gpu_viewer::menus::context(&ctx));
        gui.toggle_background_picker();
        draw(&mut r, &mut gui, &texture, &input, [-0.6, -0.5], false);
        std::fs::write(format!("{dir}/context.rgba"), read_back(&r, &texture)).unwrap();
        // The same face with the Pressure card open beside where the menu was.
        gui.toggle_background_picker();
        gui.close_floating();
        let mut session = session;
        session.do_action(SolidAction::OpenCard(
            crate::gpu_viewer::editor::solid::CardForm::Pressure,
        ));
        let input = GuiInput {
            view: &view,
            scenes: &scenes,
            selected: 0,
            editor: Some(&session as &dyn Authoring),
            error: None,
            box_drag: None,
            overlay: &[],
        };
        draw(&mut r, &mut gui, &texture, &input, [-0.6, -0.5], false);
        std::fs::write(format!("{dir}/card.rgba"), read_back(&r, &texture)).unwrap();
        // The material library, and the Simulation tab with its Analysis group.
        session.do_action(SolidAction::CardCancel);
        session.do_action(SolidAction::OpenCard(
            crate::gpu_viewer::editor::solid::CardForm::Materials,
        ));
        session.do_action(SolidAction::CardMaterialGroup);
        let input = GuiInput {
            view: &view,
            scenes: &scenes,
            selected: 0,
            editor: Some(&session as &dyn Authoring),
            error: None,
            box_drag: None,
            overlay: &[],
        };
        draw(&mut r, &mut gui, &texture, &input, [-0.6, -0.5], false);
        std::fs::write(format!("{dir}/library.rgba"), read_back(&r, &texture)).unwrap();
        session.do_action(SolidAction::CardCancel);
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
        draw(&mut r, &mut gui, &texture, &input, [-0.6, -0.5], false);
        std::fs::write(format!("{dir}/simulation.rgba"), read_back(&r, &texture)).unwrap();
    });
}

/// A solved static result with its legend, the feature tree folded to its chip, and the glyphs of the supports and
/// loads, as raw RGBA dumps: `FEA_SNAPSHOT_RESULT_OUT=/dir cargo test ... result_legend_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_RESULT_OUT"]
fn result_legend_snapshot() {
    use crate::gpu_viewer::editor::solid::{SolidAction, SolidSession};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_RESULT_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut problem = fea_problem::templates::templates().remove(0).1;
    problem.mesh.size = 0.6;
    let mut session = SolidSession::new(problem, "demo-result.json".into());
    session.do_action(SolidAction::GenerateMesh);
    let t = std::time::Instant::now();
    while session.preview_summary().is_err() {
        session.poll();
        assert!(t.elapsed().as_secs() < 60);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    session.solve();
    while !session.poll() || session.solving() {
        assert!(t.elapsed().as_secs() < 120);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1280u32, 800u32);
        let scene = session.scene().unwrap();
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
            .await
            .unwrap();
        let texture = target(&r, w, h);
        let mut gui = Gui::new();
        gui.ctx().set_theme(egui::Theme::Dark);
        gui.toggle_tree_open();
        let mut view = ViewState::new(&scene);
        view.field = 0;
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
        std::fs::write(format!("{dir}/result.rgba"), read_back(&r, &texture)).unwrap();
    });
}

/// The Toolbench dashboard in both themes, as raw RGBA dumps: `FEA_SNAPSHOT_DASH_OUT=/dir cargo test ... dashboard_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_DASH_OUT"]
fn dashboard_snapshot() {
    use crate::gpu_viewer::dashboard::{unix_now, Dashboard, Recents};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_DASH_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut recents = Recents::default();
    let now = unix_now();
    recents.record("fea", Some("/work/bracket.json".into()), now - 7_200);
    recents.record(
        "members",
        Some("/work/wing-truss.json".into()),
        now - 90_000,
    );
    let mut dash = Dashboard::new(recents, None);
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        // FEA_SNAPSHOT_DASH_W=<px> shoots a narrower window (the recent files move below the tiles).
        let w: u32 = std::env::var("FEA_SNAPSHOT_DASH_W").ok().and_then(|v| v.parse().ok()).unwrap_or(1280);
        let h = 800u32;
        let scene = Scene {
            label: "d".into(),
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
        // The circadian palette across a day: the hour sets colour temperature and light or dark.
        // Then each fixed library theme (`FEA_SNAPSHOT_DASH_OUT/dashboard-t-<id>.rgba`).
        let mut shots: Vec<(String, Option<usize>, f32)> =
            [3.0f32, 6.5, 8.0, 12.0, 17.5, 19.0, 21.5]
                .iter()
                .map(|h| {
                    (
                        format!("h{:02}{:02}", *h as u32, (h.fract() * 60.0) as u32),
                        None,
                        *h,
                    )
                })
                .collect();
        shots.extend(
            crate::gpu_viewer::gui::theme_library::LIBRARY
                .iter()
                .enumerate()
                .map(|(i, t)| (format!("t-{}", t.id), Some(i), 12.0)),
        );
        for (name, choice, hour) in shots {
            crate::gpu_viewer::gui::theme::choose(choice);
            crate::gpu_viewer::gui::theme::set_hour(Some(hour));
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            let view = texture.create_view(&Default::default());
            let ctx = gui.ctx().clone();
            for _ in 0..3 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(w as f32, h as f32),
                    )),
                    ..Default::default()
                };
                let (out, _) = gui.run_dashboard(raw, &mut dash);
                r.paint_egui(&view, &ctx, out, [w, h]);
            }
            std::fs::write(
                format!("{dir}/dashboard-{name}.rgba"),
                read_back(&r, &texture),
            )
            .unwrap();
        }
    });
}

/// The nav strip of the dashboard window (ADR-024) over the dashboard and over a viewer page, with a dirty and a busy tab,
/// as raw RGBA dumps: `FEA_SNAPSHOT_NAV_OUT=/dir cargo test ... nav_strip_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_NAV_OUT"]
fn nav_strip_snapshot() {
    use crate::gpu_viewer::dashboard::{Dashboard, Recents};
    use crate::gpu_viewer::nav::{NavStrip, NavTab};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_NAV_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let strip = |active: Option<&'static str>, ask_close: Option<&'static str>| NavStrip {
        tabs: vec![
            NavTab { id: "fea", name: "FEA Workbench", dirty: true, busy: false },
            NavTab { id: "lug", name: "Lug Analysis", dirty: false, busy: true },
            NavTab { id: "bushing", name: "Bushing", dirty: false, busy: false },
        ],
        active,
        ask_close,
    };
    let mut dash = Dashboard::new(Recents::default(), None);
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1280u32, 800u32);
        let scene = Scene {
            label: "n".into(),
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
        crate::gpu_viewer::gui::theme::choose(None);
        crate::gpu_viewer::gui::theme::set_hour(Some(12.0));
        // The dashboard with the strip (a tab is open, none shown) and the close question.
        for (name, ask) in [("dashboard", None), ("close-question", Some("fea"))] {
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.set_nav(Some(strip(None, ask)));
            let view = texture.create_view(&Default::default());
            let ctx = gui.ctx().clone();
            for _ in 0..3 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))),
                    ..Default::default()
                };
                let (out, _) = gui.run_dashboard(raw, &mut dash);
                r.paint_egui(&view, &ctx, out, [w, h]);
            }
            std::fs::write(format!("{dir}/nav-{name}.rgba"), read_back(&r, &texture)).unwrap();
        }
        // The switcher open over the dashboard.
        {
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.set_nav(Some(strip(None, None)));
            gui.open_switcher();
            let view = texture.create_view(&Default::default());
            let ctx = gui.ctx().clone();
            for _ in 0..3 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))),
                    ..Default::default()
                };
                let (out, _) = gui.run_dashboard(raw, &mut dash);
                r.paint_egui(&view, &ctx, out, [w, h]);
            }
            std::fs::write(format!("{dir}/nav-switcher.rgba"), read_back(&r, &texture)).unwrap();
        }
        // A workbench page under the strip.
        let texture = target(&r, w, h);
        let mut gui = Gui::new();
        gui.set_nav(Some(strip(Some("fea"), None)));
        let view = ViewState::new(&scene);
        let scenes = [scene];
        let input = GuiInput {
            view: &view,
            scenes: &scenes,
            selected: 0,
            editor: None,
            error: None,
            box_drag: None,
            overlay: &[],
        };
        draw(&mut r, &mut gui, &texture, &input, [0.35, 0.4], true);
        std::fs::write(format!("{dir}/nav-viewer.rgba"), read_back(&r, &texture)).unwrap();
    });
}
