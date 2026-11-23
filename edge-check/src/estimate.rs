//! The shared edge-distance estimate: one input and one output for every workbench that has a hole near a free edge
//! (the Bushing window first; Fastener Hole, the FEA Workbench and Assembly Joints later). It builds the
//! [`EdgeInput`] / [`EdgeConfig`] the runner needs, runs the models and returns the report with its
//! [`EdgeMatrix`] (per-model minimum e/D, the best-estimate and the governing markers).
//!
//! What counts as governing is the caller's: a workbench that has its own legacy check hands it in
//! ([`EstimateInput::legacy`]) and that row is governing; with none, every row is advisory
//! ([`EdgeMatrix::is_advisory`]). Nothing here decides a verdict.

use crate::matrix::{EdgeMatrix, LegacyEdge};
use crate::runner::{run, EdgeConfig, EdgeInput, EdgeReport};
use crate::types::{BushingSpec, Geometry, Strengths};

/// Elastic constants of the steel pin the contact FE uses for a bare hole (what the NACA TN 1503 validation used:
/// `tests/validation_naca_tn1503.rs`).
pub const PIN_E: f64 = 29.0e6;
pub const PIN_NU: f64 = 0.30;
/// The pin's loaded core, as a fraction of the bore radius (same source).
pub const PIN_CORE: f64 = 0.4;

/// What sits in the bore.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoreFit {
    /// A pin or fastener in a bare (reamed) hole: no bushing, no interference. This is the configuration the contact FE
    /// is validated on. `friction` is the pin-hole coefficient; the tests agree best with `0` (friction raised the
    /// prediction 5 to 8 %).
    Pin { friction: f64 },
    /// A bushing pressed into the bore (the contact FE meshes it; its fit is validated analytically only).
    Bushing(BushingSpec),
    /// Only the fit pressure is known (the bushing itself is not described): the stress models run, the contact FE cannot,
    /// and a deep run falls back to the dead-load plastic plate model.
    PressureOnly,
}

/// How the hole is loaded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoadCase {
    /// A pin or fastener bearing on the bore, pulling toward the nearest free edge (the worst case), `load` lbf.
    PinTowardEdge { load: f64 },
}

impl LoadCase {
    fn pin_load(self) -> f64 {
        match self {
            LoadCase::PinTowardEdge { load } => load,
        }
    }
}

/// Everything an edge estimate needs, in inches, lbf and psi.
#[derive(Debug, Clone, PartialEq)]
pub struct EstimateInput {
    pub bore_radius: f64,
    /// Bore centre to the free edge.
    pub edge: f64,
    pub thickness: f64,
    /// Inclination of the classical shear-out planes (the allowables model), degrees.
    pub plane_angle_deg: f64,
    pub material: Strengths,
    pub fit: BoreFit,
    pub load: LoadCase,
    /// Interference contact pressure and its band over the tolerances (psi). Ignored (zero) for a bare pin.
    pub fit_pressure: f64,
    pub fit_pressure_min: f64,
    pub fit_pressure_max: f64,
    /// The workbench's own (legacy) edge check, if it has one: that row is governing.
    pub legacy: Option<LegacyEdge>,
}

/// Run options.
#[derive(Debug, Clone, Default)]
pub struct EstimateOptions {
    /// Also run the contact FE (about 3 s): the best estimate.
    pub deep: bool,
    /// Material-strength scatter and per-model error overrides; `None` keeps the defaults.
    pub strength_cv: Option<f64>,
    pub model_cvs: Vec<(&'static str, f64)>,
}

/// A finished estimate.
#[derive(Debug, Clone)]
pub struct EdgeEstimate {
    pub report: EdgeReport,
    pub matrix: EdgeMatrix,
}

/// The pin as the contact FE meshes it: a thick steel cylinder loaded through a small rigid core, no interference.
pub fn pin_spec(bore_radius: f64, friction: f64) -> BushingSpec {
    BushingSpec { inner_radius: PIN_CORE * bore_radius, interference: 0.0, e: PIN_E, nu: PIN_NU, friction: friction.max(0.0) }
}

/// Validates the input and builds what the runner needs. Cheap; the heavy part is [`execute`].
pub fn prepare(input: &EstimateInput, opts: &EstimateOptions) -> Result<(EdgeInput, EdgeConfig), String> {
    let (a, e, t) = (input.bore_radius, input.edge, input.thickness);
    let load = input.load.pin_load();
    if a.is_nan() || a <= 0.0 || !input.fit_pressure.is_finite() || !load.is_finite() {
        return Err("bore diameter, contact pressure and load must be finite and positive".to_string());
    }
    if e.is_nan() || e <= a {
        return Err("edge distance must exceed the bore radius".to_string());
    }
    if t.is_nan() || t <= 0.0 {
        return Err("housing length must be positive".to_string());
    }
    // The stress models assume a plate wide enough that its other edges do not matter (the half-plane analytic model has
    // none at all); the FE plate is sized to match: 3 e or 10 bore radii, whichever is larger.
    let reach = (3.0 * e).max(10.0 * a);
    let (fit, lo, hi) = match input.fit {
        BoreFit::Pin { .. } => (0.0, 0.0, 0.0),
        BoreFit::Bushing(_) | BoreFit::PressureOnly => (input.fit_pressure.max(0.0), input.fit_pressure_min.max(0.0), input.fit_pressure_max.max(0.0)),
    };
    let edge_input = EdgeInput {
        geom: Geometry { bore_radius: a, edge: e, thickness: t, plate_far: reach, plate_half_height: reach, plane_angle_deg: input.plane_angle_deg },
        strengths: input.material,
        applied_load: load,
        fit_pressure: fit,
        fit_pressure_min: lo,
        fit_pressure_max: hi,
    };
    let bushing = match input.fit {
        BoreFit::Pin { friction } => Some(pin_spec(a, friction)),
        BoreFit::Bushing(spec) => Some(spec),
        BoreFit::PressureOnly => None,
    };
    let mut cfg = EdgeConfig { include_plastic: opts.deep, bushing, model_cvs: opts.model_cvs.clone(), ..EdgeConfig::default() };
    if let Some(cv) = opts.strength_cv {
        cfg.strength_cv = cv;
    }
    Ok((edge_input, cfg))
}

/// Runs the models: about 0.3 s without the contact FE, about 3 s with it. Blocking; call from a worker.
pub fn execute(input: &EdgeInput, cfg: &EdgeConfig, legacy: Option<LegacyEdge>) -> EdgeEstimate {
    let report = run(&crate::models::default_models(cfg), input, cfg);
    let matrix = EdgeMatrix::build(&report, legacy);
    EdgeEstimate { report, matrix }
}

/// [`prepare`] then [`execute`].
pub fn estimate(input: &EstimateInput, opts: &EstimateOptions) -> Result<EdgeEstimate, String> {
    let (edge_input, cfg) = prepare(input, opts)?;
    Ok(execute(&edge_input, &cfg, input.legacy))
}
