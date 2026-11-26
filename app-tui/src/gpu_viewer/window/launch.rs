//! Entry points: the workbench factory (`build_workbench`), the window runners, the viewer constructor and the event loop launch.
use super::*;
use crate::gpu_viewer::{editor::stackup::WindowLease, EditorKind};

pub fn run(project: ViewerProject) -> Result<(), String> {
    project.validate()?;
    launch(project, None)
}

/// Open the Toolbench dashboard (`--gpu-dashboard`): the list of workbenches. Each opens in this window (`nav`);
/// Shift-click and "New window" start a process of its own.
pub fn run_dashboard() -> Result<(), String> {
    let mut app = new_viewer(ViewerProject::single(blank_scene("dashboard")), None);
    app.nav = Some(Nav::default());
    crate::gpu_viewer::editor::handoff::serve_shell();
    let file = crate::gpu_viewer::dashboard::Recents::path();
    app.dashboard = Some(crate::gpu_viewer::dashboard::Dashboard::new(
        file.as_deref()
            .map(crate::gpu_viewer::dashboard::Recents::load_from)
            .unwrap_or_default(),
        file,
    ));
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    app.gui.attach_layout_store();
    app.attach_display_store(DisplaySettings::path());
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.map_or(Ok(()), Err)
}

/// A workbench session ready to show: the editor, the scene its first frame draws and, for a case that one window
/// may hold at a time, the lease on it.
pub(super) struct Built {
    pub editor: Box<dyn Authoring>,
    pub scene: Scene,
    pub lease: Option<WindowLease>,
}

/// Build the session of `kind` on `path` (loads it when it exists, otherwise starts the default). The one place
/// that knows how each workbench opens: the `--gpu-*` processes and the dashboard window both come here.
pub(super) fn build_workbench(kind: EditorKind, path: &std::path::Path) -> Result<Built, String> {
    use crate::gpu_viewer::editor as ed;
    fn model(mut s: impl Authoring + 'static) -> Result<Built, String> {
        let scene = Authoring::scene(&mut s)?;
        Ok(Built { editor: Box::new(s), scene, lease: None })
    }
    // A bench without a model: its page is drawn where the model would be.
    fn page(s: impl Authoring + 'static, label: &str) -> Built {
        Built { editor: Box::new(s), scene: blank_scene(label), lease: None }
    }
    match kind {
        EditorKind::Members => {
            let problem = if path.exists() { ed::state::read_problem(path)? } else { fea_problem::members::blank_planar() };
            problem.validate()?;
            model(ed::member_session(problem, path.to_path_buf()))
        }
        EditorKind::Solid => {
            let (problem, destination) = ed::solid::open_path(path)?;
            model(ed::solid::SolidSession::new(problem, destination))
        }
        EditorKind::Bushing => model(ed::bushing::BushingSession::open(path)?),
        EditorKind::BushingStackup => {
            let session = ed::stackup::StackupSession::open(path)?;
            let lease = ed::stackup::WindowLease::acquire(session.path())?;
            Ok(Built { lease: Some(lease), ..page(session, "bushing-stackup") })
        }
        EditorKind::Eccentric => model(ed::eccentric::EccentricSession::open(path)?),
        EditorKind::FastenerHole => model(ed::fastener::FastenerSession::open(path)?),
        EditorKind::PlateEdge => model(ed::plate_edge::PlateEdgeSession::open(path)?),
        EditorKind::Thread => model(ed::thread_workbench::ThreadSession::open(path)?),
        EditorKind::Assembly => model(ed::assembly::AssemblySession::open(path)?),
        EditorKind::Search => {
            // The search and index stages are logged like the terminal toolbox's (no paths or terms, see debug_log).
            crate::debug_log::init();
            Ok(page(ed::search::SearchSession::open(Some(path))?, "search"))
        }
        EditorKind::Materials => Ok(page(ed::materials::MaterialsSession::open(Some(path))?, "materials")),
        EditorKind::PressureVessel => model(ed::vessel::VesselSession::open(path)?),
        EditorKind::Preload => model(ed::preload::PreloadSession::open(path)?),
        EditorKind::Lug => model(ed::lug::LugSession::open(path)?),
    }
}

/// Open `kind` on `path` in a window of its own (`--gpu-solid [file]`, ...).
pub fn run_workbench(kind: EditorKind, path: &std::path::Path) -> Result<(), String> {
    let built = build_workbench(kind, path)?;
    let _lease = built.lease;
    launch(ViewerProject::single(built.scene), Some(built.editor))
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

/// The view a window starts with on `project` and `editor`.
pub(super) fn fresh_view(project: &ViewerProject, editor: Option<&dyn Authoring>) -> ViewState {
    let mut view = ViewState::new(&project.scenes[project.selected]);
    view.show_mesh = editor.is_some_and(|e| e.default_mesh_lines());
    if let Some(editor) = editor {
        apply_initial_display(&mut view, editor);
    }
    if let Some(v) = editor.and_then(|e| e.initial_view()) {
        (view.yaw, view.pitch) = v.angles();
    }
    view
}

pub(super) fn apply_initial_display(view: &mut ViewState, editor: &dyn Authoring) {
    if let Some(style) = editor.initial_style() {
        view.style = style;
    }
    if let Some(colours) = editor.initial_key_colours() {
        view.key_colours = colours;
    }
    if editor.adjustable_shading() {
        view.panel_pos[0] = None;
    }
}

pub(super) fn new_viewer(project: ViewerProject, editor: Option<Box<dyn Authoring>>) -> Viewer {
    let view = fresh_view(&project, editor.as_deref());
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
        nav: None,
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
