//! Toolbox registry. Search Files, Fastener Holes, Bushing Workbench, and
//! Pressure Vessel Analyzer are real toolboxes; Dupes/Rename/Logs exist
//! only as `ToolId` rail placeholders (see `nav.rs`) until they're migrated
//! in a future phase.

pub mod bushing;
pub mod fastener_hole;
pub mod preload_analysis;
pub mod pressure_vessel;
pub mod search;
