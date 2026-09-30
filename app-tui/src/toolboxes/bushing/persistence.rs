//! Fixed export-report destination for the Bushing Workbench toolbox's `e`
//! key - uses `crate::paths::app_data_dir` (shared with every other
//! toolbox's persistence module), under its own filename so no toolbox's
//! exports collide. Custom reamer/material/Bushing-ID library persistence
//! lives in this toolbox's own `reamer_persistence.rs`/
//! `material_persistence.rs`/`bushing_id_persistence.rs`, not here.

use std::path::PathBuf;

use crate::paths::app_data_dir;

pub fn report_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("reports").join("bushing-report.txt"))
}
