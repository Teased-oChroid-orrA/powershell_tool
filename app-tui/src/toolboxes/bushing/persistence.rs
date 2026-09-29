//! Fixed export-report destination for the Bushing Workbench toolbox's `e`
//! key - mirrors `toolboxes/pressure_vessel/persistence.rs::report_path`'s
//! per-OS path resolution exactly, under its own filename so the two
//! toolboxes' exports never collide. No settings/materials persistence
//! exists for this toolbox (see `material_picker.rs`'s own doc comment for
//! why there's no custom-material concept to persist).

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
    app_data_dir().map(|d| d.join("reports").join("bushing-report.txt"))
}
