//! Problem definition layer over `fea-core`. See `Cargo.toml`.
pub mod build;
pub mod joint;
pub mod problem;
pub mod raster;
pub mod report;
pub mod solve;
pub mod templates;

pub use problem::{Analysis, Bushing, ElementChoice, Geometry, Load, MaterialSpec, MeshSpec, Problem, Shape, Support};
pub use solve::{solve, Field, Solved};
