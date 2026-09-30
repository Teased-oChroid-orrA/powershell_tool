//! Fixed export-report destination for the Preload Analysis toolbox's `e`
//! key - uses `crate::paths::app_data_dir` (shared with every other
//! toolbox's persistence module), under its own filename.

use std::path::PathBuf;

use crate::paths::app_data_dir;

pub fn report_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("reports").join("preload-analysis-report.txt"))
}
