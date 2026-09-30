//! Shared per-OS app-data directory resolution - one implementation for
//! every toolbox's persistence module (`toolboxes/*/persistence.rs`) rather
//! than each hand-copying the same `cfg!(target_os = ...)` branch. Windows
//! is the only real shipping target; the macOS/Linux branches only help
//! local development (see root `CLAUDE.md`'s target-environment invariant).

use std::path::PathBuf;

pub fn app_data_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("GSEngineeringToolbench"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_dir_ends_in_the_shared_app_folder_name() {
        if let Some(dir) = app_data_dir() {
            assert_eq!(dir.file_name().unwrap(), "GSEngineeringToolbench");
        }
    }
}
