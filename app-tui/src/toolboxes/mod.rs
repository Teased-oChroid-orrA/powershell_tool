//! Toolbox registry. Search Files, Fastener Holes, Bushing Workbench,
//! Eccentric Bushing and the FEA Workbench are terminal toolboxes (`nav.rs`);
//! Dupes/Rename/Logs exist only as `ToolId` rail placeholders.
//! `lug_analysis`, `material_lookup`, `preload_analysis` and `pressure_vessel`
//! have no terminal toolbox: they are the models their native windows
//! (`gpu_viewer/editor/`) wrap (`material_lookup` is also the FEA Workbench's
//! material picker).

pub mod bushing;
pub mod fea_workbench;
pub mod fastener_hole;
pub mod eccentric_bushing;
pub mod lug_analysis;
pub mod material_lookup;
pub mod preload_analysis;
pub mod pressure_vessel;
pub mod search;
