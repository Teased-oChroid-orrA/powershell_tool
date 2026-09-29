//! Fixed export-report destination for the Preload Analysis toolbox's `e`
//! key - same per-OS path resolution as
//! `toolboxes/pressure_vessel/persistence.rs::report_path` /
//! `toolboxes/bushing/persistence.rs::report_path`, under its own
//! filename.

use std::path::PathBuf;

fn app_data_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("GSEngineeringToolbench"))
}

pub fn report_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("reports").join("preload-analysis-report.txt"))
}
