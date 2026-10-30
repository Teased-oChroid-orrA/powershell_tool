//! Direct wgpu companion viewport. Scene construction is pure; process/window I/O belongs to runners.
pub mod bench;
pub mod controller;
pub mod dashboard;
pub mod display;
pub mod editor;
#[cfg(feature = "gpu-viewer")]
pub mod gui;
pub mod menu;
pub mod menus;
pub mod progress;
pub mod results;
#[cfg(feature = "gpu-viewer")]
pub mod render;
pub mod scene;
pub mod text;
pub mod transient;
pub mod transport;
pub mod ui;
#[cfg(feature = "gpu-viewer")]
pub mod window;
pub use scene::Scene;

pub fn run(path: &std::path::Path) -> Result<(), String> {
    #[cfg(feature = "gpu-viewer")]
    {
        if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 256 * 1024 * 1024 {
            return Err("GPU scene exceeds the 256 MiB file budget".into());
        }
        let project =
            transport::read_project(std::fs::File::open(path).map_err(|e| e.to_string())?)?;
        window::run(project)
    }
    #[cfg(not(feature = "gpu-viewer"))]
    {
        let _ = path;
        Err("rebuild with the gpu-viewer feature to open a native viewport".into())
    }
}

/// Which model the native editor window edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKind {
    /// Frame / truss members (`fea_problem::members`).
    Members,
    /// A meshed solid (`fea_problem::Problem`, the Workbench's file format).
    Solid,
    /// The bushing press-fit workbench (`editor::bushing::file::BushingFile`).
    Bushing,
    BushingStackup,
    /// The eccentric-bushing spin-capacity workbench (`editor::eccentric::file::EccentricFile`).
    Eccentric,
    /// The Fastener Hole workbench (`editor::fastener::file::FastenerFile`).
    FastenerHole,
    /// The Search workbench (`editor::search`); its argument is an optional folder to search, its settings are the
    /// Search toolbox's own (`settings-tui.json`).
    Search,
}
impl EditorKind {
    /// Command-line flag of the editor process.
    pub fn flag(self) -> &'static str {
        match self {
            EditorKind::Members => "--gpu-editor",
            EditorKind::Solid => "--gpu-solid",
            EditorKind::Bushing => "--gpu-bushing",
            EditorKind::BushingStackup => "--gpu-bushing-stackup",
            EditorKind::Eccentric => "--gpu-eccentric",
            EditorKind::FastenerHole => "--gpu-fastener",
            EditorKind::Search => "--gpu-search",
        }
    }
    pub fn from_flag(flag: &str) -> Option<Self> {
        [EditorKind::Members, EditorKind::Solid, EditorKind::Bushing, EditorKind::BushingStackup, EditorKind::Eccentric, EditorKind::FastenerHole, EditorKind::Search].into_iter().find(|k| k.flag() == flag)
    }
    pub fn title(self) -> &'static str {
        match self {
            EditorKind::Members => "Member editor",
            EditorKind::Solid => "Solid editor",
            EditorKind::Bushing => "Bushing workbench",
            EditorKind::BushingStackup => "Bushing stackup",
            EditorKind::Eccentric => "Eccentric bushing workbench",
            EditorKind::FastenerHole => "Fastener hole workbench",
            EditorKind::Search => "Search workbench",
        }
    }
    /// File used when none is given (Search: none, the saved folder is kept).
    pub fn default_file(self) -> &'static str {
        match self {
            EditorKind::Members => "member-model.json",
            EditorKind::Solid => "solid-model.json",
            EditorKind::Bushing => "bushing-case.json",
            EditorKind::BushingStackup => "bushing-stackup.json",
            EditorKind::Eccentric => "eccentric-case.json",
            EditorKind::FastenerHole => "fastener-hole-case.json",
            EditorKind::Search => "",
        }
    }
}

/// Open the native editor window (`--gpu-editor` / `--gpu-solid [file]`): loads `path` when it exists,
/// otherwise starts from a blank planar member model or the default solid template, which Ctrl/Cmd+S
/// writes to `path`.
pub fn run_native_editor(kind: EditorKind, path: &std::path::Path) -> Result<(), String> {
    #[cfg(feature = "gpu-viewer")]
    {
        match kind {
            EditorKind::Members => {
                let problem = if path.exists() { editor::state::read_problem(path)? } else { fea_problem::members::blank_planar() };
                window::run_editor(problem, path.to_path_buf())
            }
            EditorKind::Solid => {
                let (problem, destination) = editor::solid::open_path(path)?;
                window::run_solid(problem, destination)
            }
            EditorKind::Bushing => window::run_bushing(editor::bushing::BushingSession::open(path)?),
            EditorKind::BushingStackup => window::run_stackup(editor::stackup::StackupSession::open(path)?),
            EditorKind::Eccentric => window::run_eccentric(editor::eccentric::EccentricSession::open(path)?),
            EditorKind::FastenerHole => window::run_fastener(editor::fastener::FastenerSession::open(path)?),
            EditorKind::Search => window::run_search(editor::search::SearchSession::open(Some(path))?),
        }
    }
    #[cfg(not(feature = "gpu-viewer"))]
    {
        let _ = (kind, path);
        Err("rebuild with the gpu-viewer feature to open the native editor".into())
    }
}

/// Open the Toolbench dashboard window (`--gpu-dashboard`).
pub fn run_dashboard() -> Result<(), String> {
    #[cfg(feature = "gpu-viewer")]
    {
        window::run_dashboard()
    }
    #[cfg(not(feature = "gpu-viewer"))]
    {
        Err("rebuild with the gpu-viewer feature to open the dashboard".into())
    }
}

/// Open the native member editor (`--gpu-editor [file]`).
pub fn run_editor(path: &std::path::Path) -> Result<(), String> {
    run_native_editor(EditorKind::Members, path)
}

#[cfg(test)]
mod tests {
    use super::EditorKind;

    #[test]
    fn editor_flags_round_trip_and_unknown_flags_are_not_editors() {
        for k in [EditorKind::Members, EditorKind::Solid, EditorKind::Bushing, EditorKind::BushingStackup, EditorKind::Eccentric, EditorKind::FastenerHole, EditorKind::Search] {
            assert_eq!(EditorKind::from_flag(k.flag()), Some(k));
        }
        assert_eq!(EditorKind::from_flag("--gpu-viewer"), None);
        assert_ne!(EditorKind::Members.default_file(), EditorKind::Solid.default_file());
    }
}
