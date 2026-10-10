//! Direct wgpu companion viewport. Scene construction is pure; process/window I/O belongs to runners.
pub mod progress;
#[cfg(feature = "gpu-viewer")]
pub mod render;
pub mod scene;
pub mod transient;
pub mod transport;
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
