//! General finite-element kernel. See `Cargo.toml`.
pub mod adapt;
pub mod amg;
pub mod adaptivity;
pub mod analysis;
pub mod assembly;
pub mod contact;
pub mod dense;
pub mod dynamics;
pub mod delaunay;
pub mod derivative;
pub mod element;
pub mod fit;
pub mod fields;
pub mod coupling;
pub mod generate;
pub mod geometry;
pub mod import;
pub mod kernel;
pub mod linear;
pub mod locate;
pub mod material;
pub mod loads;
pub mod mesh;
pub mod meshing;
pub mod mesh2d;
pub mod mpc;
pub mod nonlinear;
pub mod optimize;
pub mod ordering;
pub mod recover;
pub mod report;
pub mod sensitivity;
pub mod sparse;
pub mod strategy;
pub mod structural;
pub mod plate;
pub mod sweep;
pub mod tetmesh;
pub mod thermal;
pub mod topology;
pub mod validate;
pub mod vtu;

pub use analysis::{Model, Solution, SolveMethod};

/// Run `f` on a one-thread pool: every parallel loop inside it (the colour-parallel assembly, sparse products, the
/// parallel factorisation) then runs in sequence. For recomputing a result whose verification failed, in case the failure
/// came from the parallel schedule.
pub fn sequentially<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    match rayon::ThreadPoolBuilder::new().num_threads(1).build() {
        Ok(pool) => pool.install(f),
        Err(_) => f(),
    }
}
pub use element::ElementKind;
pub use linear::Dirichlet;
pub use loads::{FaceField, Loads, SurfaceLoad};
pub use mesh::{Elastic, Mesh, Physics};
pub use material::J2;
pub use nonlinear::{Control, Interrupt, NlOptions, StepEvent, StepObserver, NlSolution, NlState, Start};
pub use dynamics::{EigMethod, Buckling, BucklingMode, BucklingOptions, MassKind, Modal, ModalOptions, Mode, Transient, TransientOptions};
pub use strategy::{Adaptive, Ladder, Requirements, Rung, Verification};
pub use validate::{Issue, Severity};
pub use thermal::{Conductivity, HeatLoads, HeatSolution, ThermalProps};
pub use optimize::{OptOptions, OptResult};
pub use sensitivity::Param;
pub use report::{Condition, EventKind, Profile, SolveEvent};
pub use mpc::{Constraints, RefPoint};
