//! Goal-oriented adaptive refinement (epic #25, System A). The pieces are independent of every mesh generator: an estimator
//! reads a solved [`crate::Model`] and returns an element-wise error field; fusion and marking turn fields into a marked set;
//! the controller asks a caller-supplied mesher for the next mesh (through a size field, as `adapt.rs` does) and keeps or
//! rolls back the step. Estimator code never calls a mesher and a mesher never sees an estimator.

pub mod dwr;
pub mod equilibrated;
pub mod estimator;
pub mod goal;
pub mod patch;
pub mod snapshot;
pub mod transaction;
pub mod transfer;
pub mod zones;

pub use equilibrated::{reconstruct, EquilibratedEstimator, EquilibrationReport, Reconstruction};
pub use estimator::{AdjacencyCache, ErrorEstimator, ErrorField, EstimatorContext, Indicator, Reliability, ZzEstimator};
pub use dwr::{DwrEstimator, DwrGoalEstimator, DwrResult};
pub use goal::LinearFunctional;
pub use patch::PatchTopology;
pub use snapshot::SolverSnapshot;
pub use transaction::{attempt, Attempt};
pub use zones::{DefaultZoneResolver, PhysicsEstimator, PhysicsZoneResolver, Zone, ZoneMap, ZoneSpec};
pub use transfer::{NodalInterpolation, StateTransferOperator, Transferred};
