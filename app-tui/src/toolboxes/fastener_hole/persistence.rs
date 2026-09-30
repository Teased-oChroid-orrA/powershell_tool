//! Fixed export-report destination for the Fastener Holes toolbox's `e`
//! key - uses `crate::paths::app_data_dir` (shared with every other
//! toolbox's persistence module), under its own filename so this toolbox's
//! exports never collide with any other toolbox's. No settings/materials
//! persistence exists for this toolbox - every input is a plain toleranced
//! dimension, no catalog/custom-entry concept to persist.

use std::path::PathBuf;

use crate::paths::app_data_dir;

pub fn report_path() -> Option<PathBuf> {
    app_data_dir().map(|d| d.join("reports").join("fastener-hole-report.txt"))
}
