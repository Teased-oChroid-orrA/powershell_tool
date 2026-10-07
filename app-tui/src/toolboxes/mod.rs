//! Toolbox registry. Search Files, Fastener Holes, Bushing Workbench, and
//! Pressure Vessel Analyzer are real toolboxes; Dupes/Rename/Logs exist
//! only as `ToolId` rail placeholders (see `nav.rs`) until they're migrated
//! in a future phase.

pub mod bushing;
pub mod fea_workbench;
pub mod fastener_hole;
pub mod eccentric_bushing;
pub mod lug_analysis;
pub mod material_lookup;
pub mod preload_analysis;
pub mod pressure_vessel;
pub mod search;
