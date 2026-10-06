//! Pin-loaded lug finite-element analysis. See `Cargo.toml`.
pub mod contact;
pub mod fe;
pub mod fea;
mod fea_limit;
mod fea_solve;
pub mod finite;
pub mod geometry;
pub mod mesh;
pub mod pin;
pub mod plastic;
pub mod solve;
pub mod sparse;
pub mod stress;
pub mod thickness;

pub use contact::PointResult;
pub use contact::{IfacePoint, InterfaceSpec};
pub use fe::Material;
pub use geometry::LugGeometry;
pub use mesh::{BushingMesh, MeshSpec, Refinement};
pub use plastic::{Hardening, PlaneMode};
pub use solve::{auto_refinement, auto_refinement_for, solve_once, BushingResult, BushingSpec, CurvePoint, LimitLoad, LimitOptions, BoreStress, LoadCase, LugModel, LugSolution, PinBody, PinSpec, Thermal, Verification};
pub use thickness::{PinBending, Shear, ThicknessResult};
pub use finite::{FiniteLug, FsMaterial, FsOptions, FsPoint, FsResult};
