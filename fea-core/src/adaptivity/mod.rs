//! Goal-oriented adaptive refinement (epic #25, System A). The pieces are independent of every mesh generator: an estimator
//! reads a solved [`crate::Model`] and returns an element-wise error field; fusion and marking turn fields into a marked set;
//! the controller asks a caller-supplied mesher for the next mesh (through a size field, as `adapt.rs` does) and keeps or
//! rolls back the step. Estimator code never calls a mesher and a mesher never sees an estimator.

pub mod controller;
pub mod dwr;
pub mod eigen;
pub mod enrich;
pub mod equilibrated;
pub mod estimator;
pub mod fusion;
pub mod goal;
pub mod patch;
pub mod marking;
pub mod nonlinear;
pub mod report;
pub mod snapshot;
pub mod transaction;
pub mod transfer;
pub(crate) mod trimesh;
pub mod zones;

pub use equilibrated::{reconstruct, EquilibratedEstimator, EquilibrationReport, Reconstruction};
pub use eigen::{clusters, estimate_modes, run_eigen_amr, track, EigenConfig, EigenResult, EigenStop, ModeEstimate};
pub use enrich::EnrichedDwrEstimator;
pub use estimator::{AdjacencyCache, ErrorEstimator, ErrorField, EstimatorContext, Indicator, Reliability, ZzEstimator};
pub use dwr::{DwrEstimator, DwrGoalEstimator, DwrResult};
pub use fusion::{fuse, normalise, FusionConfig, FusionInput, FusionStrategy, Fused, GoalCombine, Normalization};
pub use goal::LinearFunctional;
pub use patch::PatchTopology;
pub use snapshot::SolverSnapshot;
pub use transaction::{attempt, Attempt};
pub use zones::{DefaultZoneResolver, PhysicsEstimator, PhysicsZoneResolver, Zone, ZoneMap, ZoneSpec};
pub use transfer::{NodalInterpolation, StateTransferOperator, Transferred};
pub use controller::{run_amr, AdaptSize, AmrConfig, AmrResult, CycleFields, CycleRecord, FieldRetention, GoalOutcome, GoalSpec, MeshedProblem, StagnationRule, Stop};
pub use nonlinear::{run_amr_nonlinear, NlAmrConfig, NlCycle, NlGoal, NlProblem, NlResult, NlStop, reequilibrate, warm_start_displacement, tangent_adjoint, transfer_history, HistoryTransfer, ReEquilibration, TangentAdjoint};
pub use marking::{dorfler, trim_to_budget, Budget, Marked};
