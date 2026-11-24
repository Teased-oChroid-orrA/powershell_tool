//! Stackup, Bushing, Fastener Hole and Assembly snapshots.
use super::*;

#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_STACKUP_OUT"]
fn stackup_snapshot() {
    use crate::gpu_viewer::editor::stackup::StackupSession;
    let dir = std::env::var("FEA_SNAPSHOT_STACKUP_OUT").expect("snapshot folder");
    std::fs::create_dir_all(&dir).unwrap();
    let _guard = headless_guard();
    let mut session =
        StackupSession::open(std::path::Path::new("/tmp/stackup-snapshot-unused-v2.json")).unwrap();
    session
        .inputs
        .members
        .push(bushing_solver::stackup::Member::new(3));
    session.inputs.members[0].name = "Outer lug".into();
    session.inputs.members[1].name = "Spacer".into();
    session.inputs.members[1].thickness = 0.25;
    session.inputs.members[2].name = "Inner lug".into();
    session.inputs.bushing_enabled = true;
    session.inputs.common_wall = Some(0.06);
    session.act(crate::gpu_viewer::editor::session::EditorAction::Stackup(
        crate::gpu_viewer::editor::stackup::StackupAct::Recalculate,
    ));
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("graphics adapter required");
        let scene = session.scene().unwrap();
        for (w, h, name, dark) in [
            (1360, 860, "wide-dark", true),
            (1360, 860, "wide-light", false),
            (780, 900, "narrow-dark", true),
        ] {
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
            let scenes = [scene.clone()];
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
            std::fs::write(
                format!("{dir}/{name}-{w}x{h}.rgba"),
                read_back(&r, &texture),
            )
            .unwrap();
            let (x, y) = gui.find_target("Drag member 1").unwrap();
            let start = egui::pos2(x as f32, y as f32);
            let finish = start - egui::vec2(0.0, 90.0);
            for events in [
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                vec![egui::Event::PointerMoved(finish)],
                vec![egui::Event::PointerButton {
                    pos: finish,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            ] {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(w as f32, h as f32),
                    )),
                    events,
                    ..Default::default()
                };
                let (output, _) = gui.run(raw, &input);
                r.discard_egui(output);
            }
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            let offsets = gui.stackup_component_offsets();
            assert!(offsets[0].1[0] < 0.0, "member drag did not move the member");
            assert_eq!(offsets[0].1[1], 0.0, "member drag moved its bushing");
            assert!(
                offsets[1..].iter().all(|(_, offset)| *offset == [0.0, 0.0]),
                "member drag moved neighbors"
            );
            std::fs::write(
                format!("{dir}/{name}-individual-{w}x{h}.rgba"),
                read_back(&r, &texture),
            )
            .unwrap();
            for (control, pose) in [("Explode all", "exploded"), ("Stack setup", "settings")] {
                let (_, y) = gui.find_target(control).unwrap();
                if y > h as f64 - 40.0 {
                    let raw = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(w as f32, h as f32),
                        )),
                        events: vec![
                            egui::Event::PointerMoved(egui::pos2(w as f32 - 24.0, h as f32 / 2.0)),
                            egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta: egui::vec2(0.0, h as f32 / 2.0 - y as f32),
                                phase: egui::TouchPhase::Move,
                                modifiers: Default::default(),
                            },
                        ],
                        ..Default::default()
                    };
                    let (output, _) = gui.run(raw, &input);
                    r.discard_egui(output);
                    for _ in 0..12 {
                        draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
                    }
                }
                let (x, y) = gui.find_target(control).unwrap();
                let pos = egui::pos2(x as f32, y as f32);
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(w as f32, h as f32),
                    )),
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                };
                let (output, _) = gui.run(raw, &input);
                r.discard_egui(output);
                draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
                if pose == "settings" {
                    assert!(
                        gui.target_labels()
                            .iter()
                            .any(|label| label == "Pilot position from stack entrance (in) input"),
                        "snapshot did not enter settings"
                    );
                } else {
                    assert!(
                        gui.stackup_component_offsets()
                            .iter()
                            .any(|(_, offset)| offset[0] > 0.0),
                        "snapshot did not explode all"
                    );
                }
                std::fs::write(
                    format!("{dir}/{name}-{pose}-{w}x{h}.rgba"),
                    read_back(&r, &texture),
                )
                .unwrap();
            }
        }
        session.act(crate::gpu_viewer::editor::session::EditorAction::Stackup(
            crate::gpu_viewer::editor::stackup::StackupAct::OpenMaterial(26),
        ));
        let (w, h) = (1360, 860);
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
            .await
            .unwrap();
        let texture = target(&r, w, h);
        let mut gui = Gui::new();
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
        std::fs::write(
            format!("{dir}/material-picker.rgba"),
            read_back(&r, &texture),
        )
        .unwrap();
    });
}

/// The Bushing workbench's three panes (inputs | 3D over the two dimensioned sections | results, install force first):
/// a failing case with the checks and fixes open, dark and light, a narrow window, the Send dialog, and the quick FE
/// check solved, as raw RGBA dumps: `FEA_SNAPSHOT_BUSHING_OUT=/dir cargo test ... bushing_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_BUSHING_OUT"]
fn bushing_snapshot() {
    use crate::gpu_viewer::editor::bushing::{file::BushingFile, BushingAct, BushingSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    use crate::toolboxes::bushing::model::{BushingModel, NumberTarget};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_BUSHING_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut model = BushingModel::default();
    // A case with a visible failure to show the verdict and a fix.
    model.commit_number(NumberTarget::EdgeDist, 0.42);
    let mut session = BushingSession::new(
        &BushingFile::capture(&model, "Wing fitting rev C"),
        std::env::temp_dir().join("bushing-snapshot.json"),
    );
    let t = std::time::Instant::now();
    let wait = |s: &mut BushingSession, done: &dyn Fn(&BushingSession) -> bool| {
        while !done(s) {
            s.poll();
            assert!(t.elapsed().as_secs() < 180);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    session.act(EditorAction::Bushing(BushingAct::EdgeCheck(false)));
    wait(&mut session, &|s| {
        s.model().recommendations.is_empty() || !s.status().starts_with("Edge check running")
    });
    let shoot = |session: &mut BushingSession,
                 name: &str,
                 dark: bool,
                 (w, h): (u32, u32),
                 drawer: Option<bool>| {
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let scene = session.scene().unwrap();
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
            if let Some(results) = drawer {
                gui.open_drawer(results);
            }
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
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    };
    shoot(&mut session, "bushing-workbench", true, (1360, 860), None);
    session.act(EditorAction::Bushing(BushingAct::Why));
    session.act(EditorAction::Bushing(BushingAct::AllFixes));
    shoot(&mut session, "bushing-checks", true, (1360, 860), None);
    shoot(
        &mut session,
        "bushing-checks-light",
        false,
        (1360, 860),
        None,
    );
    shoot(&mut session, "bushing-narrow", true, (980, 720), None);
    shoot(
        &mut session,
        "bushing-drawer-results",
        true,
        (980, 720),
        Some(true),
    );
    shoot(&mut session, "bushing-drawer-small", true, (600, 720), None);
    shoot(
        &mut session,
        "bushing-drawer-inputs",
        true,
        (600, 720),
        Some(false),
    );
    session.act(EditorAction::Bushing(BushingAct::SendDialog));
    shoot(&mut session, "bushing-send", true, (1360, 860), None);
    session.act(EditorAction::Bushing(BushingAct::CloseDialog));
    session.act(EditorAction::Bushing(BushingAct::QuickFe));
    wait(&mut session, &|s| {
        s.fea().current_solution().is_some() && !s.solving()
    });
    shoot(&mut session, "bushing-quick-fe", true, (1360, 860), None);
}

/// The Bushing edge-distance matrix (contact FE run, best estimate + governing legacy row), one tall dark shot:
/// `FEA_SNAPSHOT_BUSHING_OUT=/dir cargo test ... bushing_edge_matrix_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_BUSHING_OUT"]
fn bushing_edge_matrix_snapshot() {
    use crate::gpu_viewer::editor::bushing::{file::BushingFile, BushingAct, BushingSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    use crate::toolboxes::bushing::model::BushingModel;
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_BUSHING_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut session = BushingSession::new(&BushingFile::capture(&BushingModel::default(), "Edge matrix"), std::env::temp_dir().join("bushing-edge-matrix.json"));
    let t = std::time::Instant::now();
    session.act(EditorAction::Bushing(BushingAct::EdgeCheck(true)));
    while session.status().starts_with("Edge check running") || session.model().recommendations.is_empty() && t.elapsed().as_millis() < 300 {
        session.poll();
        assert!(t.elapsed().as_secs() < 180);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    for (name, dark, (w, h)) in [("bushing-edge-matrix", true, (1360u32, 2600u32)), ("bushing-edge-matrix-light", false, (1360, 2600))] {
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let scene = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
            let view = ViewState::new(&scene);
            let scenes = [scene];
            let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    }
}

/// The Fastener Hole edge-strength card after a contact FE run (advisory matrix): one tall dark shot,
/// `FEA_SNAPSHOT_FASTENER_OUT=/dir cargo test ... fastener_edge_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_FASTENER_OUT"]
fn fastener_edge_snapshot() {
    use crate::gpu_viewer::editor::fastener::{file::FastenerFile, FastenerAct, FastenerSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    use crate::toolboxes::fastener_hole::model::{FastenerHoleModel, NumberPart, NumberTarget};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_FASTENER_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut model = FastenerHoleModel::default();
    model.commit_number(NumberTarget::EdgeDistance, NumberPart::Nominal, 0.5);
    let pin = FastenerFile::capture(&model, "Edge estimate");
    // The plate under an inclined end load (30 degrees).
    let mut plate = FastenerFile::capture(&model, "Edge estimate");
    plate.edge.case = plate.edge.case.next();
    plate.edge.angle_deg = 30.0;
    let t = std::time::Instant::now();
    for (name, file, deep) in [("fastener-edge", pin, true), ("fastener-edge-plate", plate, false)] {
        let mut session = FastenerSession::new(&file, std::env::temp_dir().join(format!("{name}-snapshot.json")));
        session.act(EditorAction::Fastener(FastenerAct::EdgeRun(deep)));
        while session.solving() {
            session.poll();
            assert!(t.elapsed().as_secs() < 180);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1360u32, 2600u32);
            let scene = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(egui::Theme::Dark);
            let view = ViewState::new(&scene);
            let scenes = [scene];
            let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    }
}

/// The FEA Workbench's edge check card for a selected hole (contact FE run), one tall dark shot:
/// `FEA_SNAPSHOT_FEA_EDGE_OUT=/dir cargo test ... fea_edge_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_FEA_EDGE_OUT"]
fn fea_edge_snapshot() {
    use crate::gpu_viewer::editor::solid::SolidSession;
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_FEA_EDGE_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut p = fea_problem::templates::templates().remove(0).1;
    p.supports.clear();
    p.loads.clear();
    crate::toolboxes::fea_workbench::model::apply_material(&mut p, mechanics_core::materials::get_material("al7075"));
    p.loads.push(fea_problem::Load::Bearing { hole: 1, fx: -800.0, fy: 0.0 });
    let mut session = SolidSession::new(p, std::env::temp_dir().join("fea-edge-snapshot.json"));
    session.selected = Some("hole1".into());
    session.start_edge_check(true);
    let t = std::time::Instant::now();
    while session.edge_checking() {
        session.poll();
        assert!(t.elapsed().as_secs() < 180);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1360u32, 2600u32);
        let scene = session.scene().unwrap();
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
        let texture = target(&r, w, h);
        let mut gui = Gui::new();
        gui.ctx().set_theme(egui::Theme::Dark);
        let view = ViewState::new(&scene);
        let scenes = [scene];
        let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
        draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
        draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
        std::fs::write(format!("{dir}/fea-edge.rgba"), read_back(&r, &texture)).unwrap();
    });
}

/// The Bushing fit section when the bore band is wider than the interference band: Advanced settings open, the
/// tolerance bands entered, and the fit routes card showing: `FEA_SNAPSHOT_BUSHING_OUT=/dir cargo test ... bushing_fit_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_BUSHING_OUT"]
fn bushing_fit_snapshot() {
    use crate::gpu_viewer::editor::bushing::{file::BushingFile, BushingAct, BushingSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    use crate::toolboxes::bushing::advice::Keep;
    use crate::toolboxes::bushing::model::{BushingModel, NumberTarget};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_BUSHING_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut model = BushingModel::default();
    model.commit_number(NumberTarget::BoreTolPlus, 0.0010);
    model.commit_number(NumberTarget::BoreTolMinus, 0.0);
    model.commit_number(NumberTarget::InterferenceTolPlus, 0.0003);
    model.commit_number(NumberTarget::InterferenceTolMinus, 0.0003);
    let mut session = BushingSession::new(
        &BushingFile::capture(&model, "Fit routes"),
        std::env::temp_dir().join("bushing-fit-snapshot.json"),
    );
    let shoot = |session: &mut BushingSession, name: &str| {
        pollster::block_on(async {
            let (w, h) = (1360u32, 860u32);
            let adapter = test_adapter().await.expect("adapter required");
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
                editor: Some(&*session as &dyn Authoring),
                error: None,
                box_drag: None,
                overlay: &[],
            };
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    };
    shoot(&mut session, "bushing-fit-open");
    session.act(EditorAction::Bushing(BushingAct::ToggleKeep(Keep::Bore)));
    shoot(&mut session, "bushing-fit-keep-bore");
}

/// The Fastener Hole workbench (regular fit, countersink dark and light, the Report step), as raw RGBA dumps:
/// `FEA_SNAPSHOT_FASTENER_OUT=/dir cargo test ... fastener_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_FASTENER_OUT"]
fn fastener_snapshot() {
    use crate::gpu_viewer::editor::fastener::{file::FastenerFile, FastenerAct, FastenerSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    use crate::toolboxes::fastener_hole::model::{
        FastenerHoleModel, FieldRow, NumberPart, NumberTarget,
    };
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_FASTENER_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut model = FastenerHoleModel::default();
    model.commit_number(NumberTarget::RegularHole2, NumberPart::TolPlus, 0.003);
    let mut session = FastenerSession::new(
        &FastenerFile::capture(&model, "Skin splice"),
        std::env::temp_dir().join("fastener-snapshot.json"),
    );
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1360u32, 860u32);
        let scene = session.scene().unwrap();
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
            .await
            .unwrap();
        let shoot = |r: &mut Renderer,
                     session: &mut FastenerSession,
                     step: &str,
                     name: &str,
                     dark: bool| {
            let scene = session.scene().unwrap();
            r.set_scene(&scene).unwrap();
            let texture = target(r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let mut view = ViewState::new(&scene);
            // The light shot keeps the default style (wireframe: only the coloured faces are painted).
            if dark {
                view.style = crate::gpu_viewer::display::DisplayStyle::ShadedEdges;
            }
            // The window opens on the editor's view, without mesh lines.
            let (yaw, pitch) = session.initial_view().map_or((0.0, 0.0), |v| v.angles());
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
            draw(
                r,
                &mut gui,
                &texture,
                &input,
                [yaw, pitch],
                session.default_mesh_lines(),
            );
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(r, &texture)).unwrap();
        };
        shoot(&mut r, &mut session, "Inputs", "fastener-regular", true);
        session.act(EditorAction::Fastener(FastenerAct::Row(
            FieldRow::ToggleHoleType,
        )));
        shoot(&mut r, &mut session, "Inputs", "fastener-countersink", true);
        shoot(
            &mut r,
            &mut session,
            "Inputs",
            "fastener-countersink-light",
            false,
        );
        shoot(&mut r, &mut session, "Report", "fastener-report", true);
    });
}

/// The Assembly Joints window on `solid-kernel/tests/data/assembly-stacked-plates.step` (exploded with the fastener site
/// selected, joined with the interface selected, light; the upper plate slid sideways with an unmated hole selected), as raw RGBA dumps:
/// `FEA_SNAPSHOT_ASSEMBLY_OUT=/dir cargo test ... assembly_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_ASSEMBLY_OUT"]
fn assembly_snapshot() {
    use crate::gpu_viewer::editor::assembly::{AssemblyAct, AssemblySession, Sel};
    use crate::gpu_viewer::editor::session::EditorAction;
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_ASSEMBLY_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../solid-kernel/tests/data/assembly-stacked-plates.step");
    let mut session = AssemblySession::open(&file).unwrap();
    pollster::block_on(async {
        let adapter = test_adapter().await.expect("adapter required");
        let (w, h) = (1360u32, 860u32);
        let scene = session.scene().unwrap();
        let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h)
            .await
            .unwrap();
        let shoot = |r: &mut Renderer, session: &mut AssemblySession, name: &str, dark: bool| {
            let scene = session.scene().unwrap();
            r.set_scene(&scene).unwrap();
            let texture = target(r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let mut view = ViewState::new(&scene);
            if dark {
                view.style = crate::gpu_viewer::display::DisplayStyle::ShadedEdges;
            }
            let (yaw, pitch) = session.initial_view().map_or((0.0, 0.0), |v| v.angles());
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
            gui.select_tab_named(&input, "Assembly");
            draw(
                r,
                &mut gui,
                &texture,
                &input,
                [yaw, pitch],
                session.default_mesh_lines(),
            );
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(r, &texture)).unwrap();
        };
        shoot(&mut r, &mut session, "assembly-none", true);
        session.act(EditorAction::Assembly(AssemblyAct::Select(Sel::Item(1))));
        shoot(&mut r, &mut session, "assembly-fastener", true);
        session.act(EditorAction::Assembly(AssemblyAct::Select(Sel::Item(0))));
        shoot(&mut r, &mut session, "assembly-interface", true);
        session.act(EditorAction::Assembly(AssemblyAct::ToggleExploded));
        shoot(&mut r, &mut session, "assembly-interface-joined", true);
        shoot(
            &mut r,
            &mut session,
            "assembly-interface-joined-light",
            false,
        );
        // the upper plate slid 15 mm along x: no shared axis, so both holes are "Holes without a mate"
        let text = std::fs::read_to_string(&file).unwrap();
        let slid = std::path::Path::new(&dir).join("assembly-slid-plates.step");
        std::fs::write(
            &slid,
            text.replace(
                "#944 = CARTESIAN_POINT('',(0.,0.,10.));",
                "#944 = CARTESIAN_POINT('',(15.,0.,10.));",
            ),
        )
        .unwrap();
        let mut session = AssemblySession::open(&slid).unwrap();
        let first = session
            .loaded()
            .unwrap()
            .advice
            .iter()
            .position(|a| {
                matches!(
                    a,
                    fea_problem::assembly_advice::AssemblyAdvice::Unmated { .. }
                )
            })
            .unwrap();
        session.act(EditorAction::Assembly(AssemblyAct::Select(Sel::Item(
            first,
        ))));
        shoot(&mut r, &mut session, "assembly-unmated", true);
    });
}

/// The Plate Edge window after a 2D run (clearance fit): the inspector with the allowables and the solved model by
/// plastic strain, wide and dark, plus the graded plate before any run in light:
/// `FEA_SNAPSHOT_PLATE_EDGE_OUT=/dir cargo test ... plate_edge_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_PLATE_EDGE_OUT"]
fn plate_edge_snapshot() {
    use crate::gpu_viewer::editor::plate_edge::{inputs::{FitKind, PlateEdgeInputs}, run::Kind, PlateEdgeAct, PlateEdgeSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_PLATE_EDGE_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let wait = |s: &mut PlateEdgeSession| {
        let t = std::time::Instant::now();
        loop {
            s.poll();
            if !s.solving() && s.status() != "Computing the allowables..." {
                break;
            }
            assert!(t.elapsed().as_secs() < 300);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    let mut inputs = PlateEdgeInputs::default();
    inputs.fit = FitKind::Clearance;
    inputs.fit_value = 0.002;
    inputs.edge = 0.5;
    for (name, run, dark) in [("plate-edge-run", true, true), ("plate-edge-light", false, false)] {
        let mut session = PlateEdgeSession::new(inputs.clone(), "Plate edge".into(), std::env::temp_dir().join(format!("{name}-snapshot.json")));
        wait(&mut session);
        if run {
            session.act(EditorAction::PlateEdge(PlateEdgeAct::Run(Kind::TwoD)));
            wait(&mut session);
        }
        pollster::block_on(async {
            let adapter = test_adapter().await.expect("adapter required");
            let (w, h) = (1500u32, 1500u32);
            let scene = session.scene().unwrap();
            let mut r = Renderer::new(&adapter, wgpu::TextureFormat::Rgba8Unorm, &scene, w, h).await.unwrap();
            let texture = target(&r, w, h);
            let mut gui = Gui::new();
            gui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
            let mut view = ViewState::new(&scene);
            view.style = crate::gpu_viewer::display::DisplayStyle::ShadedEdges;
            view.show_mesh = true;
            let scenes = [scene];
            let input = GuiInput { view: &view, scenes: &scenes, selected: 0, editor: Some(&session as &dyn Authoring), error: None, box_drag: None, overlay: &[] };
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            draw(&mut r, &mut gui, &texture, &input, [0.0, 0.0], false);
            std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
        });
    }
}
