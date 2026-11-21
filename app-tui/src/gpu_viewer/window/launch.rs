//! Entry points: one `run_*` per window kind, plus the viewer constructor and the event loop launch.
use super::*;

pub fn run(project: ViewerProject) -> Result<(), String> {
    project.validate()?;
    launch(project, None)
}

/// Open the native member editor on `problem`, saving to `path`.
pub fn run_editor(
    problem: fea_problem::members::MemberProblem,
    path: std::path::PathBuf,
) -> Result<(), String> {
    problem.validate()?;
    let mut ed = crate::gpu_viewer::editor::member_session(problem, path);
    let scene = ed.scene()?;
    launch(ViewerProject::single(scene), Some(Box::new(ed)))
}

/// Open the native solid editor on `problem`, saving to `path`.
pub fn run_solid(problem: fea_problem::Problem, path: std::path::PathBuf) -> Result<(), String> {
    let mut ed = crate::gpu_viewer::editor::solid::SolidSession::new(problem, path);
    let scene = crate::gpu_viewer::editor::authoring::Authoring::scene(&mut ed)?;
    launch(ViewerProject::single(scene), Some(Box::new(ed)))
}

/// Open the Toolbench dashboard (`--gpu-dashboard`): the list of workbenches, each opened in its own process.
pub fn run_dashboard() -> Result<(), String> {
    let mut app = new_viewer(ViewerProject::single(blank_scene("dashboard")), None);
    let file = crate::gpu_viewer::dashboard::Recents::path();
    app.dashboard = Some(crate::gpu_viewer::dashboard::Dashboard::new(
        file.as_deref()
            .map(crate::gpu_viewer::dashboard::Recents::load_from)
            .unwrap_or_default(),
        file,
    ));
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    app.gui.attach_layout_store();
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}

/// Open the Bushing workbench on `session`.
pub fn run_bushing(
    mut session: crate::gpu_viewer::editor::bushing::BushingSession,
) -> Result<(), String> {
    let scene = crate::gpu_viewer::editor::authoring::Authoring::scene(&mut session)?;
    launch(ViewerProject::single(scene), Some(Box::new(session)))
}

/// Open the line-reamed bushing stackup as a native page.
pub fn run_stackup(
    session: crate::gpu_viewer::editor::stackup::StackupSession,
) -> Result<(), String> {
    let _lease = crate::gpu_viewer::editor::stackup::WindowLease::acquire(session.path())?;
    launch(
        ViewerProject::single(blank_scene("bushing-stackup")),
        Some(Box::new(session)),
    )
}

/// Open the Fastener Hole workbench on `session`.
pub fn run_fastener(
    mut session: crate::gpu_viewer::editor::fastener::FastenerSession,
) -> Result<(), String> {
    let scene = crate::gpu_viewer::editor::authoring::Authoring::scene(&mut session)?;
    launch(ViewerProject::single(scene), Some(Box::new(session)))
}

/// Open the Assembly Joints window on `session`.
pub fn run_assembly(
    mut session: crate::gpu_viewer::editor::assembly::AssemblySession,
) -> Result<(), String> {
    let scene = crate::gpu_viewer::editor::authoring::Authoring::scene(&mut session)?;
    launch(ViewerProject::single(scene), Some(Box::new(session)))
}

/// Open the Pressure Vessel workbench on `session`.
pub fn run_vessel(
    mut session: crate::gpu_viewer::editor::vessel::VesselSession,
) -> Result<(), String> {
    let scene = crate::gpu_viewer::editor::authoring::Authoring::scene(&mut session)?;
    launch(ViewerProject::single(scene), Some(Box::new(session)))
}

/// Open the Preload Analysis workbench on `session`.
pub fn run_preload(
    mut session: crate::gpu_viewer::editor::preload::PreloadSession,
) -> Result<(), String> {
    let scene = crate::gpu_viewer::editor::authoring::Authoring::scene(&mut session)?;
    launch(ViewerProject::single(scene), Some(Box::new(session)))
}

/// Open the Lug Analysis workbench on `session`.
pub fn run_lug(mut session: crate::gpu_viewer::editor::lug::LugSession) -> Result<(), String> {
    let scene = crate::gpu_viewer::editor::authoring::Authoring::scene(&mut session)?;
    launch(ViewerProject::single(scene), Some(Box::new(session)))
}

/// Open the Search workbench on `session`: a bench without a model, its page drawn where the model would be.
pub fn run_search(session: crate::gpu_viewer::editor::search::SearchSession) -> Result<(), String> {
    launch(
        ViewerProject::single(blank_scene("search")),
        Some(Box::new(session)),
    )
}

/// Open a page bench (no model; its page is drawn where the model would be) on `session`.
pub fn run_page(session: Box<dyn Authoring>, label: &str) -> Result<(), String> {
    launch(ViewerProject::single(blank_scene(label)), Some(session))
}

/// A one-triangle scene for a window that draws no model (the dashboard, a page bench).
pub(super) fn blank_scene(label: &str) -> Scene {
    Scene {
        label: label.into(),
        positions: vec![[0.0; 3]; 3],
        triangles: vec![0, 1, 2],
        samples: vec![[0.0; 4]; 3],
        times: vec![0.0],
        range: [0.0, 1.0],
        gain: 1.0,
        harmonic_period: None,
        field_label: String::new(),
        fields: Vec::new(),
    }
}

/// Open the Eccentric Bushing workbench on `session`.
pub fn run_eccentric(
    mut session: crate::gpu_viewer::editor::eccentric::EccentricSession,
) -> Result<(), String> {
    let scene = crate::gpu_viewer::editor::authoring::Authoring::scene(&mut session)?;
    launch(ViewerProject::single(scene), Some(Box::new(session)))
}

pub(super) fn new_viewer(project: ViewerProject, editor: Option<Box<dyn Authoring>>) -> Viewer {
    let mut view = ViewState::new(&project.scenes[project.selected]);
    view.show_mesh = editor.as_ref().is_some_and(|e| e.default_mesh_lines());
    if let Some(v) = editor.as_ref().and_then(|e| e.initial_view()) {
        (view.yaw, view.pitch) = v.angles();
    }
    Viewer {
        project,
        view,
        window: None,
        surface: None,
        renderer: None,
        config: None,
        last: Instant::now(),
        next_redraw: Instant::now(),
        occluded: false,
        pointer: Pointer::default(),
        mods: ModifiersState::default(),
        error: None,
        gui: Gui::new(),
        egui_state: None,
        egui_press: false,
        pan_gesture: None,
        quit: false,
        quit_warned: false,
        size_override: None,
        grabbing: false,
        test_events: Vec::new(),
        test_time: 0.0,
        layout_flushed: Instant::now(),
        recovery_flushed: Instant::now(),
        display_file: None,
        editor,
        dashboard: None,
    }
}

pub(super) fn launch(
    project: ViewerProject,
    editor: Option<Box<dyn Authoring>>,
) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut app = new_viewer(project, editor);
    app.gui.attach_layout_store();
    app.attach_display_store(DisplaySettings::path());
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    if app.editor.as_ref().is_some_and(|ed| ed.dirty()) {
        eprintln!("note: the model had unsaved changes when the window closed");
    }
    app.error.map_or(Ok(()), Err)
}
