//! General finite-element kernel. See `Cargo.toml`.
pub mod adapt;
pub mod adaptivity;
pub mod amg;
pub mod analysis;
pub mod assembly;
pub mod contact;
pub mod coupling;
pub mod delaunay;
pub mod dense;
pub mod derivative;
pub mod dynamics;
pub mod element;
pub mod fields;
pub mod fit;
pub mod generate;
pub mod geometry;
pub mod import;
pub mod kernel;
pub mod linear;
pub mod loads;
pub mod locate;
pub mod material;
pub mod mesh;
pub mod mesh2d;
pub mod meshing;
pub mod mpc;
pub mod nonlinear;
pub mod optimize;
pub mod ordering;
pub mod plate;
pub mod recover;
pub mod report;
pub mod sensitivity;
pub mod sparse;
pub mod strategy;
pub mod structural;
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
pub use dynamics::{
    Buckling, BucklingMode, BucklingOptions, EigMethod, MassKind, Modal, ModalOptions, Mode,
    Transient, TransientOptions,
};
pub use element::ElementKind;
pub use linear::Dirichlet;
pub use loads::{FaceField, Loads, SurfaceLoad};
pub use material::J2;
pub use mesh::{Elastic, Mesh, Physics};
pub use mpc::{Constraints, RefPoint};
pub use nonlinear::{
    Control, Interrupt, NlOptions, NlSolution, NlState, Start, StepEvent, StepObserver,
};
pub use optimize::{OptOptions, OptResult};
pub use report::{Condition, EventKind, Profile, SolveEvent};
pub use sensitivity::Param;
pub use strategy::{Adaptive, Ladder, Requirements, Rung, Verification};
pub use thermal::{Conductivity, HeatLoads, HeatSolution, ThermalProps};
pub use validate::{Issue, Severity};
