//! Problem definition layer over `fea-core`. See `Cargo.toml`.
pub mod advice;
pub mod benchmark;
pub mod build;
pub mod cases;
pub mod diagnose;
pub mod dynamics;
pub mod history;
pub mod joint;
pub mod members;
pub mod path;
pub mod problem;
pub mod raster;
pub mod report;
pub mod sketch;
pub mod solid;
pub mod solve;
pub mod templates;
pub mod thermal;

pub use cases::LoadCase;
pub use problem::{AdaptMode, MeshStrategy, Analysis, Bushing, ElementChoice, Geometry, Load, MaterialSpec, MeshSpec, Problem, RefineRegion, Seg, Shape, SketchRegion, Support};
pub use solve::{solve, Field, Solved};
pub use thermal::{HeatLoad, HeatSummary, HeatSupport, Study, ThermalSpec};
