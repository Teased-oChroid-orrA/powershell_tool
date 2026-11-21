//! Pressure Vessel, Preload and Lug snapshots.
use super::*;

/// The Pressure Vessel workbench (the inputs with the hoop contour, the checks, the material picker; dark, light, a
/// narrow window), as raw RGBA dumps: `FEA_SNAPSHOT_VESSEL_OUT=/dir cargo test ... vessel_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_VESSEL_OUT"]
fn vessel_snapshot() {
    use crate::gpu_viewer::editor::session::EditorAction;
    use crate::gpu_viewer::editor::vessel::{file::VesselFile, VesselAct, VesselSession};
    use crate::toolboxes::pressure_vessel::model::{FieldRow, PressureVesselModel};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_VESSEL_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut session = VesselSession::new(
        &VesselFile::capture(&PressureVesselModel::default(), "Accumulator"),
        std::env::temp_dir().join("vessel-snapshot.json"),
    );
    let shoot =
        |session: &mut VesselSession, step: &str, name: &str, dark: bool, (w, h): (u32, u32)| {
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
                let view = ViewState::new(&scene);
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
                    &mut r,
                    &mut gui,
                    &texture,
                    &input,
                    [yaw, pitch],
                    session.default_mesh_lines(),
                );
                std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
            });
        };
    shoot(
        &mut session,
        "Inputs",
        "vessel-inputs-dark",
        true,
        (1440, 900),
    );
    shoot(
        &mut session,
        "Inputs",
        "vessel-inputs-light",
        false,
        (1440, 900),
    );
    shoot(&mut session, "Inputs", "vessel-narrow", true, (860, 760));
    session.act(EditorAction::Vessel(VesselAct::Step("Results")));
    shoot(
        &mut session,
        "Results",
        "vessel-results-dark",
        true,
        (1440, 900),
    );
    shoot(
        &mut session,
        "Results",
        "vessel-results-light",
        false,
        (1440, 900),
    );
    use crate::gpu_viewer::editor::session::{Key, Mods};
    use crate::toolboxes::pressure_vessel::model::NumberTarget;
    for (target, text) in [
        (NumberTarget::CorrosionAllowance, "0.125"),
        (NumberTarget::MillUndertolerance, "12.5"),
    ] {
        session.act(EditorAction::Vessel(VesselAct::Row(FieldRow::Number(
            target,
        ))));
        for c in text.chars() {
            session.key(Key::Char(c), Mods::default());
        }
        session.key(Key::Enter, Mods::default());
    }
    shoot(
        &mut session,
        "Results",
        "vessel-allowances-results",
        true,
        (1440, 900),
    );
    session.act(EditorAction::Vessel(VesselAct::Step("Inputs")));
    shoot(
        &mut session,
        "Inputs",
        "vessel-allowances-inputs",
        true,
        (1440, 900),
    );
    session.act(EditorAction::Vessel(VesselAct::Step("Inputs")));
    session.act(EditorAction::Vessel(VesselAct::Row(
        FieldRow::OpenMaterialPicker,
    )));
    shoot(
        &mut session,
        "Inputs",
        "vessel-picker-dark",
        true,
        (1440, 900),
    );
    // The line run: its page over the viewport (a valve on the run), then the cylinder's table card.
    session.act(EditorAction::Vessel(VesselAct::ClosePicker));
    session.act(EditorAction::Vessel(VesselAct::ToggleLineRun));
    session.act(EditorAction::Vessel(VesselAct::AddLoad));
    for text in ["150"] {
        use crate::toolboxes::pressure_vessel::edit::{Edit, LineField};
        session.act(EditorAction::Vessel(VesselAct::Edit(Edit::Line(
            LineField::LoadWeight(0),
        ))));
        for c in text.chars() {
            session.key(Key::Char(c), Mods::default());
        }
        session.key(Key::Enter, Mods::default());
    }
    let t = std::time::Instant::now();
    while session.model().walls.is_none() {
        session.poll();
        assert!(t.elapsed().as_secs() < 60, "the wall solve did not finish");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    session.act(EditorAction::Vessel(VesselAct::Step("Results")));
    shoot(
        &mut session,
        "Results",
        "vessel-line-dark",
        true,
        (1440, 900),
    );
    shoot(
        &mut session,
        "Results",
        "vessel-line-light",
        false,
        (1440, 900),
    );
    shoot(
        &mut session,
        "Results",
        "vessel-line-narrow",
        true,
        (860, 760),
    );
    session.act(EditorAction::Vessel(VesselAct::Step("Inputs")));
    shoot(
        &mut session,
        "Inputs",
        "vessel-line-inputs",
        true,
        (1440, 900),
    );
    // Segment mode with the bound: the page with both results, then the Segment card.
    session.act(EditorAction::Vessel(VesselAct::ToggleSegment));
    session.act(EditorAction::Vessel(VesselAct::ToggleBound));
    let t = std::time::Instant::now();
    loop {
        session.poll();
        if session.model().walls.is_some() {
            break;
        }
        assert!(
            t.elapsed().as_secs() < 60,
            "the segment wall solve did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    session.act(EditorAction::Vessel(VesselAct::Step("Results")));
    shoot(
        &mut session,
        "Results",
        "vessel-segment-dark",
        true,
        (1440, 900),
    );
    shoot(
        &mut session,
        "Results",
        "vessel-segment-light",
        false,
        (1440, 900),
    );
    shoot(
        &mut session,
        "Results",
        "vessel-segment-narrow",
        true,
        (860, 760),
    );
    session.act(EditorAction::Vessel(VesselAct::Step("Inputs")));
    shoot(
        &mut session,
        "Inputs",
        "vessel-segment-inputs",
        true,
        (1440, 900),
    );
}

/// The Preload Analysis workbench (inputs dark, light and in a narrow window; the parts picture before the member FE
/// answers; the FE stress after; the bolt picker), as raw RGBA dumps:
/// `FEA_SNAPSHOT_PRELOAD_OUT=/dir cargo test ... preload_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_PRELOAD_OUT"]
fn preload_snapshot() {
    use crate::gpu_viewer::editor::preload::{file::PreloadFile, PreloadAct, PreloadSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    use crate::toolboxes::preload_analysis::model::{FieldRow, PreloadModel};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_PRELOAD_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut session = PreloadSession::new(
        &PreloadFile::capture(&PreloadModel::default(), "Lap joint"),
        std::env::temp_dir().join("preload-snapshot.json"),
    );
    let shoot =
        |session: &mut PreloadSession, step: &str, name: &str, dark: bool, (w, h): (u32, u32)| {
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
                let view = ViewState::new(&scene);
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
                    &mut r,
                    &mut gui,
                    &texture,
                    &input,
                    [yaw, pitch],
                    session.default_mesh_lines(),
                );
                std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
            });
        };
    // Before the member FE answers the picture is the parts in the colour key.
    if session.fe_current().is_none() {
        shoot(
            &mut session,
            "Inputs",
            "preload-parts-dark",
            true,
            (1440, 900),
        );
    }
    let start = std::time::Instant::now();
    while session.fe_current().is_none() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(120),
            "the member FE never answered"
        );
        session.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    shoot(
        &mut session,
        "Inputs",
        "preload-inputs-dark",
        true,
        (1440, 900),
    );
    shoot(
        &mut session,
        "Inputs",
        "preload-inputs-light",
        false,
        (1440, 900),
    );
    shoot(&mut session, "Inputs", "preload-narrow", true, (860, 760));
    session.act(EditorAction::Preload(PreloadAct::Step("Results")));
    shoot(
        &mut session,
        "Results",
        "preload-results-dark",
        true,
        (1440, 900),
    );
    shoot(
        &mut session,
        "Results",
        "preload-results-light",
        false,
        (1440, 900),
    );
    session.act(EditorAction::Preload(PreloadAct::Step("Inputs")));
    session.act(EditorAction::Preload(PreloadAct::Row(
        FieldRow::OpenBoltPicker,
    )));
    shoot(
        &mut session,
        "Inputs",
        "preload-picker-dark",
        true,
        (1440, 900),
    );
}

/// The Lug Analysis workbench (the parts picture before the analysis answers; inputs dark, light and in a narrow
/// window over the von Mises contour; the results; the material picker), as raw RGBA dumps:
/// `FEA_SNAPSHOT_LUG_OUT=/dir cargo test ... lug_snapshot -- --ignored`.
#[test]
#[ignore = "needs a graphics adapter and FEA_SNAPSHOT_LUG_OUT"]
fn lug_snapshot() {
    use crate::gpu_viewer::editor::lug::{file::LugFile, LugAct, LugSession};
    use crate::gpu_viewer::editor::session::EditorAction;
    use crate::toolboxes::lug_analysis::model::{FieldRow, LugUiModel};
    let Ok(dir) = std::env::var("FEA_SNAPSHOT_LUG_OUT") else {
        return;
    };
    let _guard = headless_guard();
    let mut session = LugSession::new(
        &LugFile::capture(&LugUiModel::default(), "Clevis lug"),
        std::env::temp_dir().join("lug-snapshot.json"),
    );
    let shoot =
        |session: &mut LugSession, step: &str, name: &str, dark: bool, (w, h): (u32, u32)| {
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
                let view = ViewState::new(&scene);
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
                    &mut r,
                    &mut gui,
                    &texture,
                    &input,
                    [yaw, pitch],
                    session.default_mesh_lines(),
                );
                std::fs::write(format!("{dir}/{name}.rgba"), read_back(&r, &texture)).unwrap();
            });
        };
    // Before the analysis answers the picture is the parts in the colour key.
    if session.current_run().is_none() {
        shoot(&mut session, "Inputs", "lug-parts-dark", true, (1440, 900));
    }
    let start = std::time::Instant::now();
    while session.current_run().is_none() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(300),
            "the lug analysis never answered"
        );
        session.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    shoot(&mut session, "Inputs", "lug-inputs-dark", true, (1440, 900));
    shoot(
        &mut session,
        "Inputs",
        "lug-inputs-light",
        false,
        (1440, 900),
    );
    shoot(&mut session, "Inputs", "lug-narrow", true, (860, 760));
    session.act(EditorAction::Lug(LugAct::Step("Results")));
    shoot(
        &mut session,
        "Results",
        "lug-results-dark",
        true,
        (1440, 900),
    );
    shoot(
        &mut session,
        "Results",
        "lug-results-light",
        false,
        (1440, 900),
    );
    session.act(EditorAction::Lug(LugAct::Step("Inputs")));
    session.act(EditorAction::Lug(LugAct::Row(FieldRow::OpenMaterialPicker)));
    shoot(&mut session, "Inputs", "lug-picker-dark", true, (1440, 900));
}
