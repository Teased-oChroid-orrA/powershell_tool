//! The estimator interface: an immutable context (a solved model), element-wise error fields with a reliability status, and the
//! ZZ energy-norm estimator as the first implementation. Fields are indexed by global element id (blocks in order, elements in
//! order within a block), the numbering of [`crate::meshing::adjacency::Adjacency`].
//!
//! Sums over elements are taken in element order, one thread, so a total never depends on the thread count.

use crate::analysis::Model;
use crate::linear::Dirichlet;
use crate::loads::Loads;
use crate::meshing::adjacency::Adjacency;
use crate::meshing::{Cached, ElementLabels, MeshError};
use std::time::Instant;

/// How far an estimate can be trusted.
#[derive(Debug, Clone, PartialEq)]
pub enum Reliability {
    /// The assumptions of the formulation hold on this problem.
    Reliable,
    /// Computed, but an assumption is shaky (singular solution, thin patches, unconverged dual): use for ranking only.
    Degraded(String),
    /// The computation failed; the field is not meaningful. Never silently dropped by fusion.
    Failed(String),
    /// The formulation does not cover this problem (element family, nonlinear material, contact): no field is produced.
    Unsupported(String),
}

impl Reliability {
    pub fn usable(&self) -> bool {
        matches!(self, Self::Reliable | Self::Degraded(_))
    }
}

/// What the numbers of a field mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indicator {
    /// Non-negative local error contribution (an energy-norm error, `|eta_e|` of a goal-oriented estimate): usable for marking.
    Marking,
    /// Signed local contribution to the error of a quantity of interest (sums to the estimated QoI error): not for marking.
    Signed,
}

/// One estimator's result.
#[derive(Debug, Clone)]
pub struct ErrorField {
    pub name: String,
    pub indicator: Indicator,
    /// Per global element, in the estimator's own (dimensioned) units. Normalisation lives in fusion, never here.
    pub values: Vec<f64>,
    pub reliability: Reliability,
    /// Estimated total error in the units of the field's target (energy norm, or the quantity of interest).
    pub total: f64,
    pub seconds: f64,
    /// Free-form facts for the diagnostics (effectivity bounds, dual residual, patches that fell back).
    pub notes: Vec<String>,
}

impl ErrorField {
    /// A field with every value checked finite; a non-finite value downgrades it to `Failed` instead of poisoning fusion.
    pub fn new(name: &str, indicator: Indicator, values: Vec<f64>, total: f64, reliability: Reliability, seconds: f64) -> Self {
        let bad = values.iter().position(|v| !v.is_finite());
        let reliability = match (bad, reliability) {
            (Some(i), _) => Reliability::Failed(format!("element {i} has a non-finite value")),
            (None, r) => r,
        };
        Self { name: name.to_string(), indicator, values, reliability, total, seconds, notes: Vec::new() }
    }

    /// Unusable placeholder for an estimator that does not cover the problem.
    pub fn unsupported(name: &str, why: &str, n_elems: usize) -> Self {
        Self { name: name.to_string(), indicator: Indicator::Marking, values: vec![0.0; n_elems], reliability: Reliability::Unsupported(why.to_string()), total: 0.0, seconds: 0.0, notes: Vec::new() }
    }

    /// Whether the field may be used to rank elements (reliable or degraded, never failed or unsupported).
    pub fn usable_for_ranking(&self) -> bool {
        self.reliability.usable()
    }

    /// Sum of the values in element order (the estimated QoI error for a signed field).
    pub fn sum(&self) -> f64 {
        self.values.iter().sum()
    }

    /// `sqrt(sum v^2)` in element order.
    pub fn l2(&self) -> f64 {
        self.values.iter().map(|v| v * v).sum::<f64>().sqrt()
    }

    /// Non-negative marking values: `|v|` of a signed field, the field itself otherwise.
    pub fn marking(&self) -> Vec<f64> {
        self.values.iter().map(|v| v.abs()).collect()
    }
}

/// A solved linear static model and everything an estimator may read. Immutable: estimators never change the solved state.
pub struct EstimatorContext<'a> {
    pub model: &'a Model,
    pub loads: &'a Loads,
    pub bc: &'a Dirichlet,
    /// Displacements of the solved model.
    pub u: &'a [f64],
    pub adjacency: &'a Adjacency,
    pub labels: Option<&'a ElementLabels>,
}

impl<'a> EstimatorContext<'a> {
    pub fn new(model: &'a Model, loads: &'a Loads, bc: &'a Dirichlet, u: &'a [f64], adjacency: &'a Adjacency) -> Result<Self, MeshError> {
        if u.len() != model.mesh.n_dofs() {
            return Err(MeshError::Failed(format!("{} displacements for {} dofs", u.len(), model.mesh.n_dofs())));
        }
        if adjacency.n_elems() != model.mesh.n_elems() {
            return Err(MeshError::Failed("the adjacency belongs to another mesh".into()));
        }
        Ok(Self { model, loads, bc, u, adjacency, labels: None })
    }

    pub fn with_labels(mut self, labels: &'a ElementLabels) -> Self {
        self.labels = Some(labels);
        self
    }

    pub fn n_elems(&self) -> usize {
        self.model.mesh.n_elems()
    }
}

/// An a-posteriori error estimator.
pub trait ErrorEstimator {
    fn name(&self) -> &str;
    fn estimate(&self, ctx: &EstimatorContext) -> Result<ErrorField, MeshError>;
}

/// Adjacency cached for a mesh (rebuilt only when the mesh changed).
pub struct AdjacencyCache {
    cached: Option<Cached<Adjacency>>,
    /// How many times the adjacency was built (diagnostics and tests).
    pub builds: usize,
}

impl AdjacencyCache {
    pub fn new() -> Self {
        Self { cached: None, builds: 0 }
    }

    pub fn get(&mut self, mesh: &crate::mesh::Mesh) -> &Adjacency {
        if self.cached.as_ref().and_then(|c| c.get(mesh)).is_none() {
            self.cached = Some(Cached::new(mesh, Adjacency::new(mesh)));
            self.builds += 1;
        }
        self.cached.as_ref().and_then(|c| c.get(mesh)).expect("just filled")
    }
}

impl Default for AdjacencyCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Zienkiewicz-Zhu energy-norm estimator on SPR-recovered stress (`Model::zz_error_field`).
pub struct ZzEstimator;

impl ErrorEstimator for ZzEstimator {
    fn name(&self) -> &str {
        "zz"
    }

    fn estimate(&self, ctx: &EstimatorContext) -> Result<ErrorField, MeshError> {
        let t = Instant::now();
        if ctx.loads.temperature.is_some() {
            return Ok(ErrorField::unsupported("zz", "nodal temperature fields are not supported by the ZZ recovery", ctx.n_elems()));
        }
        let zz = ctx.model.zz_error_field(ctx.u, ctx.loads.delta_t, None).map_err(MeshError::Failed)?;
        let total = zz.total;
        let mut f = ErrorField::new("zz", Indicator::Marking, zz.eta, total, Reliability::Reliable, t.elapsed().as_secs_f64());
        if !zz.norm.is_finite() {
            f.reliability = Reliability::Failed("the recovered-stress energy norm is not finite".into());
        }
        f.notes.push(format!("relative error {:.3e}", total / zz.norm.hypot(total).max(1e-300)));
        Ok(f)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::generate::grid;
    use crate::mesh::{Elastic, Physics};
    use crate::loads::SurfaceLoad;

    /// A solved cantilever strip used by the estimator tests.
    pub(crate) fn solved_strip(n: usize) -> (Model, Loads, Dirichlet, Vec<f64>) {
        let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1e7, 0.3), [n * 4, n, 1], &|p| [4.0 * p[0], p[1], 0.0]).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &nd in model.mesh.node_set("u0").unwrap() {
            bc.fix(nd, 0, 0.0);
            bc.fix(nd, 1, 0.0);
        }
        let loads = Loads { faces: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([0.0, -100.0, 0.0]))).collect(), ..Default::default() };
        let u = model.solve_static(&loads, &bc).unwrap().u;
        (model, loads, bc, u)
    }

    struct Mock(Vec<f64>);
    impl ErrorEstimator for Mock {
        fn name(&self) -> &str {
            "mock"
        }
        fn estimate(&self, ctx: &EstimatorContext) -> Result<ErrorField, MeshError> {
            assert_eq!(self.0.len(), ctx.n_elems());
            Ok(ErrorField::new("mock", Indicator::Marking, self.0.clone(), self.0.iter().map(|v| v * v).sum::<f64>().sqrt(), Reliability::Reliable, 0.0))
        }
    }

    #[test]
    fn a_mock_estimator_runs_through_the_context_and_nonfinite_values_downgrade_the_field() {
        let (model, loads, bc, u) = solved_strip(2);
        let adj = Adjacency::new(&model.mesh);
        let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
        let ok = Mock(vec![1.0; model.mesh.n_elems()]).estimate(&ctx).unwrap();
        assert_eq!(ok.reliability, Reliability::Reliable);
        assert!((ok.l2() - (model.mesh.n_elems() as f64).sqrt()).abs() < 1e-12);
        let mut bad = vec![1.0; model.mesh.n_elems()];
        bad[3] = f64::NAN;
        let f = Mock(bad).estimate(&ctx).unwrap();
        assert!(matches!(f.reliability, Reliability::Failed(_)) && !f.reliability.usable());
        assert!(EstimatorContext::new(&model, &loads, &bc, &u[..4], &adj).is_err());
        let other = solved_strip(1).0;
        assert!(EstimatorContext::new(&other, &loads, &bc, &u, &adj).is_err());
    }

    #[test]
    fn the_zz_estimator_is_deterministic_and_matches_the_kernel_estimate() {
        let (model, loads, bc, u) = solved_strip(3);
        let adj = Adjacency::new(&model.mesh);
        let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
        let a = ZzEstimator.estimate(&ctx).unwrap();
        let b = ZzEstimator.estimate(&ctx).unwrap();
        assert_eq!(a.values, b.values);
        assert_eq!(a.reliability, Reliability::Reliable);
        let direct = model.zz_error(&u, 0.0).unwrap();
        assert_eq!(a.values, direct.eta);
        assert!((a.total - direct.total).abs() < 1e-15);
        assert_eq!(a.values.len(), model.mesh.n_elems());
    }

    #[test]
    fn temperature_fields_are_reported_unsupported_not_ignored() {
        let (model, mut loads, bc, u) = solved_strip(2);
        loads.temperature = Some(vec![0.0; model.mesh.nodes.len()]);
        let adj = Adjacency::new(&model.mesh);
        let ctx = EstimatorContext::new(&model, &loads, &bc, &u, &adj).unwrap();
        let f = ZzEstimator.estimate(&ctx).unwrap();
        assert!(matches!(f.reliability, Reliability::Unsupported(_)));
    }

    #[test]
    fn the_adjacency_cache_rebuilds_only_for_a_changed_mesh() {
        let (model, ..) = solved_strip(2);
        let mut cache = AdjacencyCache::new();
        cache.get(&model.mesh);
        cache.get(&model.mesh);
        assert_eq!(cache.builds, 1);
        let mut moved = model.mesh.clone();
        moved.nodes[7][0] += 1e-9;
        cache.get(&moved);
        assert_eq!(cache.builds, 2);
        cache.get(&moved);
        assert_eq!(cache.builds, 2);
    }
}
