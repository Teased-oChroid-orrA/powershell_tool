//! The adaptive solve-estimate-mark-remesh-resolve loop.
//!
//! One cycle: solve; estimate the error of every goal (dual-weighted residual) and the physics error (equilibrated stress, else
//! SPR+/ZZ); resolve the zones; fuse the criteria into a ranking plus mandatory elements; mark by the bulk criterion within the
//! budget; turn the marked set into a size field (the marked elements shrink by `refine_ratio`, no element coarsens); ask the
//! caller's mesher for the next mesh; verify the candidate (quality, conformity, preservation of the applied load, the material
//! set and the area); solve it. A candidate that fails any check is rolled back: the run stops and returns the last good state.
//!
//! The mesher is a callback `size field -> meshed problem` that rebuilds supports and loads on its mesh by name, exactly as
//! [`crate::strategy::solve_refined`] requires; it may be the plain Delaunay mesher or the hybrid structured/unstructured one,
//! and the controller does not know which: refinement therefore reaches structured and unstructured regions alike, wherever the
//! indicators say resolution is missing. Stopping conditions for the goals and for the physics are separate and both recorded.

use super::dwr::DwrEstimator;
use super::enrich::EnrichedDwrEstimator;
use super::equilibrated::EquilibratedEstimator;
use super::estimator::{ErrorEstimator, ErrorField, EstimatorContext, Reliability, ZzEstimator};
use super::fusion::{fuse, FusionConfig, FusionInput, Fused};
use super::goal::LinearFunctional;
use super::marking::{dorfler, trim_to_budget, Budget};
use super::snapshot::SolverSnapshot;
use super::transfer::{NodalInterpolation, StateTransferOperator};
use super::zones::{DefaultZoneResolver, PhysicsEstimator, PhysicsZoneResolver, Zone, ZoneMap, ZoneSpec};
use crate::adapt::{element_sizes_3d, size_field_at_nodes, size_field_at_nodes_3d, AdaptOptions, NodalSize, SizeField, SizeField3};
use crate::analysis::Model;
use crate::linear::Dirichlet;
use crate::loads::{self, Loads};
use crate::mesh::{Elastic, Mesh};
use crate::meshing::cost::MeshCost;
use crate::meshing::quality::QualityThresholds;
use crate::meshing::{ElementLabels, MeshError, RegionKind};
use std::sync::Arc;
use std::time::Instant;

/// Size field handed to the mesher.
pub enum AdaptSize {
    D2(SizeField),
    D3(SizeField3),
}

impl AdaptSize {
    /// Target element size at `x` (`z` ignored in 2D).
    pub fn at(&self, x: [f64; 3]) -> f64 {
        match self {
            Self::D2(f) => f.at([x[0], x[1]]),
            Self::D3(f) => f.at(x),
        }
    }
}

/// A meshed analysis: everything the controller needs for one cycle.
pub struct MeshedProblem {
    pub model: Model,
    pub loads: Loads,
    pub bc: Dirichlet,
    /// Structured / unstructured / transition labels when the mesher is the hybrid one.
    pub labels: Option<ElementLabels>,
    /// Edge of a structured quadrilateral relative to the size it was requested at (`HybridOptions2d::structured_scale`);
    /// `1` for a mesh without structured blocks.
    pub quad_scale: f64,
}

type GoalFn = dyn Fn(&Model, &Loads) -> Result<LinearFunctional, MeshError> + Send + Sync;
type GoalBuilder = Arc<GoalFn>;

/// A quantity of interest rebuilt on every mesh.
#[derive(Clone)]
pub struct GoalSpec {
    pub name: String,
    pub build: GoalBuilder,
    /// Allowed estimated error in the units of the goal.
    pub tolerance: Option<f64>,
    pub weight: f64,
}

impl GoalSpec {
    pub fn new(name: &str, tolerance: Option<f64>, build: impl Fn(&Model, &Loads) -> Result<LinearFunctional, MeshError> + Send + Sync + 'static) -> Self {
        Self { name: name.to_string(), build: Arc::new(build), tolerance, weight: 1.0 }
    }

    pub fn compliance(tolerance: Option<f64>) -> Self {
        Self::new("compliance", tolerance, LinearFunctional::compliance)
    }

    /// Mean of a stress component over the elements whose centroid satisfies `inside`.
    pub fn average_stress(name: &str, comp: usize, inside: impl Fn(&[f64; 3]) -> bool + Send + Sync + 'static, tolerance: Option<f64>) -> Self {
        Self::new(name, tolerance, move |m, _| LinearFunctional::average_stress(m, comp, &inside))
    }

    /// Mean of a displacement component over the nodes of a named node set.
    pub fn mean_displacement(name: &str, node_set: &str, comp: usize, tolerance: Option<f64>) -> Self {
        let set = node_set.to_string();
        Self::new(name, tolerance, move |m, _| {
            let nodes = m.mesh.node_sets.get(&set).ok_or_else(|| MeshError::Failed(format!("no node set '{set}'")))?;
            LinearFunctional::mean_displacement(m, nodes, comp)
        })
    }

    /// Sum of the reactions of a component over a named node set.
    pub fn reaction(name: &str, node_set: &str, comp: usize, tolerance: Option<f64>) -> Self {
        let set = node_set.to_string();
        Self::new(name, tolerance, move |m, l| {
            let nodes = m.mesh.node_sets.get(&set).ok_or_else(|| MeshError::Failed(format!("no node set '{set}'")))?;
            LinearFunctional::reaction(m, l, nodes, comp)
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StagnationRule {
    /// Cycles over which progress is judged.
    pub window: usize,
    /// The progress metric must fall below `gain` times its value `window` cycles earlier.
    pub gain: f64,
}

pub struct AmrConfig {
    pub fusion: FusionConfig,
    /// Dorfler fraction.
    pub theta: f64,
    pub squared: bool,
    /// Adaptation steps (the run solves up to `max_cycles + 1` meshes).
    pub max_cycles: usize,
    /// Relative energy-norm error of the stress (physics criterion).
    pub physics_tolerance: Option<f64>,
    pub budget: Budget,
    /// Marked elements are requested at this fraction of their present size.
    pub refine_ratio: f64,
    /// Largest factor by which one cycle may grow the element count (mandatory elements beyond it wait for the next cycle,
    /// highest ranked first): without it an unreachable physics requirement makes every element mandatory and the mesh
    /// explodes by `(1 / refine_ratio)^dim` per cycle.
    pub max_growth: f64,
    /// Grading and size limits (`h_min`/`h_max`) of the size field.
    pub adapt: AdaptOptions,
    pub zones: ZoneSpec,
    /// Elements of a singular zone at or below this size are not refined further.
    pub singular_h_min: Option<f64>,
    pub stagnation: StagnationRule,
    pub quality: QualityThresholds,
    /// Relative change of the load resultant / area allowed between meshes (curved boundaries change them slightly).
    pub state_tolerance: f64,
    /// Which cycles keep their element-wise fields (`AmrResult::fields`).
    pub keep_fields: FieldRetention,
}

/// Which cycles of an adaptive run keep their element-wise diagnostic fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRetention {
    None,
    /// The cycle the run ended on.
    Last,
    /// Every cycle (memory grows with the history).
    All,
}

/// Element-wise diagnostics of one cycle (global element order of that cycle's mesh).
#[derive(Debug, Clone)]
pub struct CycleFields {
    pub cycle: usize,
    /// Signed goal-oriented contributions per goal.
    pub goal_indicators: Vec<(String, Vec<f64>)>,
    pub physics_indicator: Vec<f64>,
    /// Combined refinement ranking score and the mandatory flags.
    pub score: Vec<f64>,
    pub mandatory: Vec<bool>,
    /// Elements marked for refinement after this cycle (all false for the final cycle).
    pub marked: Vec<bool>,
    pub zones: Vec<Zone>,
    /// Structured / unstructured / transition label, when the mesh came from the hybrid mesher.
    pub labels: Option<Vec<RegionKind>>,
    pub scaled_jacobian: Vec<f64>,
    /// Size of each element in the size-function units of the mesher.
    pub size: Vec<f64>,
}

impl Default for AmrConfig {
    fn default() -> Self {
        Self {
            fusion: FusionConfig::default(),
            theta: 0.5,
            squared: false,
            max_cycles: 8,
            physics_tolerance: None,
            budget: Budget::default(),
            refine_ratio: 0.5,
            max_growth: 3.0,
            adapt: AdaptOptions { target_rel_error: 0.0, order: 2.0, min_factor: 0.0, max_factor: 1.0, grading: 0.4, h_min: 0.0, h_max: f64::INFINITY },
            zones: ZoneSpec::default(),
            singular_h_min: None,
            stagnation: StagnationRule { window: 2, gain: 0.9 },
            quality: QualityThresholds::default(),
            state_tolerance: 0.02,
            keep_fields: FieldRetention::Last,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StageSeconds {
    pub solve: f64,
    pub goals: f64,
    pub physics: f64,
    pub fusion: f64,
    pub marking_and_sizing: f64,
    pub meshing: f64,
    pub checks: f64,
}

#[derive(Debug, Clone)]
pub struct GoalOutcome {
    pub name: String,
    pub value: f64,
    pub estimated_error: f64,
    pub corrected: f64,
    pub tolerance: Option<f64>,
    pub reliability: Reliability,
}

/// One cycle of the run.
#[derive(Debug, Clone)]
pub struct CycleRecord {
    pub cycle: usize,
    pub elements: usize,
    pub dofs: usize,
    pub factor_nnz: usize,
    /// Bytes of the factor (8 per entry): the dominant memory of a direct solve.
    pub factor_bytes: usize,
    pub goals: Vec<GoalOutcome>,
    /// Relative energy-norm error from the physics estimator that ran.
    pub physics_error: f64,
    pub physics_estimator: String,
    pub goal_estimator: String,
    pub qoi_converged: bool,
    pub physics_converged: bool,
    pub marked: usize,
    pub mandatory: usize,
    pub capped: usize,
    pub min_scaled_jacobian: f64,
    pub structured_fraction: f64,
    pub transition_elements: usize,
    /// `||u - I(u_previous)|| / ||u||`: how far the new solution is from the previous one carried over.
    pub solution_change: Option<f64>,
    pub reliability: Vec<String>,
    pub seconds: StageSeconds,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stop {
    /// Every stated tolerance (goals and physics) is met.
    Converged,
    MaxCycles,
    BudgetExhausted,
    Stagnated(String),
    /// Nothing is left to refine (all candidates capped or no error).
    NothingToRefine,
    /// The candidate mesh failed a check; the previous state is returned.
    RolledBack(String),
    /// No usable estimator.
    EstimatorUnavailable(String),
}

pub struct AmrResult {
    pub problem: MeshedProblem,
    pub u: Vec<f64>,
    pub stop: Stop,
    pub history: Vec<CycleRecord>,
    /// Element-wise fields of the cycles `AmrConfig::keep_fields` selects, in cycle order.
    pub fields: Vec<CycleFields>,
}

struct Evaluation {
    u: Vec<f64>,
    /// Signed goal-oriented element fields and the physics indicator that ran (for the diagnostics).
    goal_fields: Vec<(String, Vec<f64>)>,
    physics_values: Vec<f64>,
    zones: ZoneMap,
    goals: Vec<GoalOutcome>,
    physics_rel: f64,
    physics_name: String,
    fused: Fused,
    capped: Vec<bool>,
    cost: MeshCost,
    goal_estimator: &'static str,
    seconds: StageSeconds,
    h_elem: Vec<f64>,
}

pub(crate) fn element_sizes(problem: &MeshedProblem) -> Result<Vec<f64>, MeshError> {
    let mesh = &problem.model.mesh;
    if mesh.dim() == 3 && mesh.blocks.iter().all(|b| matches!(b.kind, crate::ElementKind::Tet4 | crate::ElementKind::Tet10)) {
        return element_sizes_3d(mesh).map_err(MeshError::Failed);
    }
    // size-function units: a simplex edge is about 0.87 of the size it was asked for, a structured quadrilateral about
    // `quad_scale` of it
    let q = mesh.quality();
    let mut out = Vec::with_capacity(mesh.n_elems());
    let mut g = 0;
    for blk in &mesh.blocks {
        let unit = if blk.kind.is_simplex() { 0.87 } else { problem.quad_scale };
        for _ in 0..blk.n_elems() {
            out.push(q.elements[g].size / unit);
            g += 1;
        }
    }
    Ok(out)
}

fn evaluate(problem: &MeshedProblem, goals: &[GoalSpec], cfg: &AmrConfig, adj: &crate::meshing::adjacency::Adjacency) -> Result<Evaluation, MeshError> {
    let mut seconds = StageSeconds::default();
    let t = Instant::now();
    let model = &problem.model;
    let sol = model.solve_static(&problem.loads, &problem.bc).map_err(MeshError::Failed)?;
    if !sol.rel_residual.is_finite() || sol.rel_residual >= 1e-6 {
        return Err(MeshError::Failed(format!("the solve left a relative residual of {:.2e}", sol.rel_residual)));
    }
    seconds.solve = t.elapsed().as_secs_f64();
    let mut ctx = EstimatorContext::new(model, &problem.loads, &problem.bc, &sol.u, adj)?;
    if let Some(l) = &problem.labels {
        ctx = ctx.with_labels(l);
    }

    let t = Instant::now();
    let functionals: Vec<LinearFunctional> = goals.iter().map(|g| (g.build)(model, &problem.loads)).collect::<Result<_, _>>()?;
    let refs: Vec<&LinearFunctional> = functionals.iter().collect();
    // h-enriched dual where the mesh can be refined (2D triangles and quadrilaterals), else the recovery-based estimate
    let builders: Vec<&GoalFn> = goals.iter().map(|g| &*g.build).collect();
    let enriched = EnrichedDwrEstimator.estimate_goals(&ctx, &refs, &builders)?;
    let (dwr, goal_estimator) = if enriched.iter().any(|r| matches!(r.field.reliability, Reliability::Unsupported(_))) {
        (DwrEstimator.estimate_goals(&ctx, &refs)?, "recovery-dwr")
    } else {
        (enriched, "enriched-dwr")
    };
    seconds.goals = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let zz = model.zz_error_field(&sol.u, problem.loads.delta_t, None).map_err(MeshError::Failed);
    let zz_field = ZzEstimator.estimate(&ctx)?;
    let eq_field = EquilibratedEstimator.estimate(&ctx)?;
    seconds.physics = t.elapsed().as_secs_f64();
    let zz_norm = zz.as_ref().map(|z| z.norm).unwrap_or(0.0);

    let zones = DefaultZoneResolver { spec: cfg.zones.clone() }.resolve(&model.mesh, adj)?;

    let t = Instant::now();
    let mut physics: Vec<(PhysicsEstimator, &ErrorField)> = Vec::new();
    let (physics_name, physics_total) = if eq_field.reliability.usable() {
        physics.push((PhysicsEstimator::Equilibrated, &eq_field));
        ("equilibrated", eq_field.total)
    } else {
        ("zz", zz_field.total)
    };
    if zz_field.reliability.usable() {
        physics.push((PhysicsEstimator::SprPlus, &zz_field));
    }
    let adjoint: Vec<(f64, &ErrorField)> = goals.iter().zip(&dwr).map(|(g, r)| (g.weight, &r.field)).collect();

    // sizes, caps and the physics allowance
    let h_elem = element_sizes(problem)?;
    let n = model.mesh.n_elems();
    let mut capped = vec![false; n];
    for e in 0..n {
        if cfg.adapt.h_min > 0.0 && h_elem[e] * cfg.refine_ratio < cfg.adapt.h_min {
            capped[e] = true;
        }
        if let Some(hs) = cfg.singular_h_min {
            if zones.zones[e] == Zone::Singular && h_elem[e] <= hs {
                capped[e] = true;
            }
        }
    }
    let mut fcfg = cfg.fusion.clone();
    if let (Some(tol), None) = (cfg.physics_tolerance, fcfg.physics_allowance) {
        // equidistribution of the allowed energy-norm error over the elements
        let allowed_total = tol * zz_norm.hypot(physics_total);
        fcfg.physics_allowance = Some(allowed_total / (n as f64).sqrt());
    }
    let fused = fuse(&FusionInput { adjoint: &adjoint, physics: &physics, zones: Some(&zones), capped: Some(&capped), protected: None }, &fcfg)?;
    seconds.fusion = t.elapsed().as_secs_f64();

    let goal_fields: Vec<(String, Vec<f64>)> = dwr.iter().map(|r| (r.goal.clone(), r.field.values.clone())).collect();
    let physics_values: Vec<f64> = if eq_field.reliability.usable() { eq_field.values.clone() } else { zz_field.values.clone() };
    let outcomes: Vec<GoalOutcome> = goals
        .iter()
        .zip(&dwr)
        .map(|(g, r)| GoalOutcome { name: g.name.clone(), value: r.value, estimated_error: r.estimated_error, corrected: r.corrected, tolerance: g.tolerance, reliability: r.field.reliability.clone() })
        .collect();
    let physics_rel = if physics_total.is_finite() { physics_total / zz_norm.hypot(physics_total).max(1e-300) } else { f64::NAN };
    let cost = MeshCost::of(&model.mesh)?;
    Ok(Evaluation { u: sol.u, goal_fields, physics_values, zones, goals: outcomes, physics_rel, physics_name: physics_name.to_string(), fused, capped, cost, goal_estimator, seconds, h_elem })
}

fn converged(goals: &[GoalOutcome], physics_rel: f64, cfg: &AmrConfig) -> (bool, bool, bool) {
    let any_tol = goals.iter().any(|g| g.tolerance.is_some()) || cfg.physics_tolerance.is_some();
    let qoi = goals.iter().all(|g| g.tolerance.is_none_or(|t| g.estimated_error.abs() <= t));
    let phys = cfg.physics_tolerance.is_none_or(|t| physics_rel <= t);
    (qoi, phys, any_tol && qoi && phys)
}

/// Progress metric: the worst ratio of an estimate to what is allowed (or to the value / the energy when nothing is stated).
fn progress(goals: &[GoalOutcome], physics_rel: f64, cfg: &AmrConfig) -> f64 {
    let mut m = 0.0f64;
    for g in goals {
        let denom = g.tolerance.unwrap_or_else(|| g.value.abs().max(1e-300));
        m = m.max(g.estimated_error.abs() / denom);
    }
    m.max(physics_rel / cfg.physics_tolerance.unwrap_or(1.0))
}

fn measure(mesh: &Mesh) -> f64 {
    // 2D: area from the corner polygons; 3D: volume of the corner tetrahedra / hexahedra via the scaled-Jacobian free formula
    let mut total = 0.0;
    for blk in &mesh.blocks {
        let nc = blk.kind.n_corners();
        for e in 0..blk.n_elems() {
            let c = &blk.elem(e)[..nc];
            if mesh.dim() == 2 {
                let mut s = 0.0;
                for k in 0..nc {
                    let (p, q) = (mesh.nodes[c[k]], mesh.nodes[c[(k + 1) % nc]]);
                    s += p[0] * q[1] - p[1] * q[0];
                }
                total += 0.5 * s.abs();
            } else if nc == 4 {
                let p = |i: usize| mesh.nodes[c[i]];
                let (a, b, cc, d) = (p(0), p(1), p(2), p(3));
                let (u, v, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [cc[0] - a[0], cc[1] - a[1], cc[2] - a[2]], [d[0] - a[0], d[1] - a[1], d[2] - a[2]]);
                total += ((u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0]) + u[2] * (v[0] * w[1] - v[1] * w[0])) / 6.0).abs();
            }
        }
    }
    total
}

fn materials(mesh: &Mesh) -> Vec<Elastic> {
    let mut m: Vec<Elastic> = Vec::new();
    for b in &mesh.blocks {
        if !m.contains(&b.material) {
            m.push(b.material);
        }
    }
    m
}

/// Everything the adaptation must preserve from `old` to `new`: the applied load resultant, the material set, the measure of the
/// domain and the presence of restraints. A silent loss of a load or a restraint would otherwise look like a good solve.
pub(crate) fn state_check(old: &MeshedProblem, new: &MeshedProblem, tol: f64) -> Result<(), String> {
    state_check_parts((&old.model, &old.loads, &old.bc), (&new.model, &new.loads, &new.bc), tol)
}

/// [`state_check`] on the parts of a problem (the nonlinear loop has contact specifications besides).
pub(crate) fn state_check_parts(old: (&Model, &Loads, &Dirichlet), new: (&Model, &Loads, &Dirichlet), tol: f64) -> Result<(), String> {
    let force = |p: (&Model, &Loads, &Dirichlet)| -> Result<[f64; 3], MeshError> {
        let f = loads::assemble(&p.0.mesh, p.1).map_err(MeshError::Failed)?;
        let d = p.0.mesh.dim();
        let mut out = [0.0; 3];
        for (i, v) in f.iter().enumerate() {
            out[i % d] += v;
        }
        Ok(out)
    };
    let (fo, fn_) = (force(old).map_err(|e| e.to_string())?, force(new).map_err(|e| e.to_string())?);
    let scale = fo.iter().map(|v| v.abs()).fold(0.0f64, f64::max);
    for c in 0..3 {
        if (fo[c] - fn_[c]).abs() > tol * scale.max(1e-300) {
            return Err(format!("the applied load resultant changed in component {c}: {} -> {}", fo[c], fn_[c]));
        }
    }
    let (mo, mn) = (measure(&old.0.mesh), measure(&new.0.mesh));
    if mo > 0.0 && (mo - mn).abs() > tol * mo {
        return Err(format!("the measure of the domain changed from {mo} to {mn}"));
    }
    for m in materials(&old.0.mesh) {
        if !materials(&new.0.mesh).contains(&m) {
            return Err("a material of the previous mesh is missing from the new one".into());
        }
    }
    if new.2.fixed.iter().filter(|&&f| f).count() == 0 && old.2.fixed.iter().any(|&f| f) {
        return Err("the new mesh has no restraints".into());
    }
    Ok(())
}

fn record(cycle: usize, p: &MeshedProblem, ev: &Evaluation, cfg: &AmrConfig, marked: usize, solution_change: Option<f64>, q_min: f64) -> CycleRecord {
    let (qoi, phys, _) = converged(&ev.goals, ev.physics_rel, cfg);
    let labels = p.labels.as_ref();
    let mut rel = Vec::new();
    for g in &ev.goals {
        if let Reliability::Degraded(w) | Reliability::Failed(w) | Reliability::Unsupported(w) = &g.reliability {
            rel.push(format!("{}: {w}", g.name));
        }
    }
    if let Reliability::Degraded(w) = &ev.fused.reliability {
        rel.push(format!("fusion: {w}"));
    }
    CycleRecord {
        cycle,
        elements: p.model.mesh.n_elems(),
        dofs: p.model.mesh.n_dofs(),
        factor_nnz: ev.cost.factor_nnz,
        factor_bytes: 8 * ev.cost.factor_nnz,
        goals: ev.goals.clone(),
        physics_error: ev.physics_rel,
        physics_estimator: ev.physics_name.clone(),
        goal_estimator: ev.goal_estimator.to_string(),
        qoi_converged: qoi,
        physics_converged: phys,
        marked,
        mandatory: ev.fused.mandatory.iter().filter(|&&m| m).count(),
        capped: ev.capped.iter().filter(|&&c| c).count(),
        min_scaled_jacobian: q_min,
        structured_fraction: labels.map_or(0.0, |l| l.fraction(RegionKind::Structured)),
        transition_elements: labels.map_or(0, |l| l.count(RegionKind::Transition)),
        solution_change,
        reliability: rel,
        seconds: ev.seconds,
    }
}

fn cycle_fields(cycle: usize, p: &MeshedProblem, ev: &Evaluation, marked: &[bool]) -> CycleFields {
    let q = p.model.mesh.quality();
    CycleFields {
        cycle,
        goal_indicators: ev.goal_fields.clone(),
        physics_indicator: ev.physics_values.clone(),
        score: ev.fused.score.clone(),
        mandatory: ev.fused.mandatory.clone(),
        marked: marked.to_vec(),
        zones: ev.zones.zones.clone(),
        labels: p.labels.as_ref().map(|l| l.0.clone()),
        scaled_jacobian: q.elements.iter().map(|e| e.scaled_jacobian).collect(),
        size: ev.h_elem.clone(),
    }
}

/// Run the adaptive loop from `initial`. `mesher` turns a size field into the next meshed problem.
pub fn run_amr(initial: MeshedProblem, goals: &[GoalSpec], mesher: &mut dyn FnMut(&AdaptSize) -> Result<MeshedProblem, String>, cfg: &AmrConfig) -> Result<AmrResult, MeshError> {
    if goals.is_empty() {
        return Err(MeshError::Failed("at least one goal is required".into()));
    }
    if !(cfg.refine_ratio > 0.0 && cfg.refine_ratio < 1.0) {
        return Err(MeshError::Failed(format!("the refinement ratio {} must be in (0, 1)", cfg.refine_ratio)));
    }
    let dim = initial.model.mesh.dim();
    let children = (1.0 / cfg.refine_ratio).powi(dim as i32);
    let mut history: Vec<CycleRecord> = Vec::new();
    let mut fields_acc: Vec<CycleFields> = Vec::new();
    let mut metrics: Vec<f64> = Vec::new();
    let mut current = initial;
    let adj = current.model.mesh.adjacency();
    let mut ev = evaluate(&current, goals, cfg, &adj)?;
    let mut change: Option<f64> = None;
    for cycle in 0..=cfg.max_cycles {
        let q_min = current.model.mesh.quality().min_scaled_jacobian;
        let (_, _, done) = converged(&ev.goals, ev.physics_rel, cfg);
        metrics.push(progress(&ev.goals, ev.physics_rel, cfg));
        let mut rec = record(cycle, &current, &ev, cfg, 0, change, q_min);
        let mut marked_flags = vec![false; current.model.mesh.n_elems()];
        let finish = |history: &mut Vec<CycleRecord>, acc: &mut Vec<CycleFields>, marked: &[bool], rec: CycleRecord, stop: Stop, current: MeshedProblem, ev: Evaluation| -> Result<AmrResult, MeshError> {
            if cfg.keep_fields != FieldRetention::None {
                if cfg.keep_fields == FieldRetention::Last {
                    acc.clear();
                }
                acc.push(cycle_fields(rec.cycle, &current, &ev, marked));
            }
            history.push(rec);
            Ok(AmrResult { problem: current, u: ev.u, stop, history: std::mem::take(history), fields: std::mem::take(acc) })
        };
        if done {
            return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::Converged, current, ev);
        }
        if cycle == cfg.max_cycles {
            return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::MaxCycles, current, ev);
        }
        // stagnation: the progress metric has not fallen enough over the window
        let w = cfg.stagnation.window;
        if metrics.len() > w && metrics[metrics.len() - 1] > cfg.stagnation.gain * metrics[metrics.len() - 1 - w] {
            let why = format!("the progress metric went {:.3e} -> {:.3e} over {w} cycles (qoi converged: {}, physics converged: {})", metrics[metrics.len() - 1 - w], metrics[metrics.len() - 1], rec.qoi_converged, rec.physics_converged);
            return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::Stagnated(why), current, ev);
        }

        // ---- mark, within the budget
        let t = Instant::now();
        let n = current.model.mesh.n_elems();
        let mut marked = match dorfler(&ev.fused.score, cfg.theta, &ev.fused.mandatory, cfg.squared) {
            Ok(m) => m,
            Err(e) => return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::EstimatorUnavailable(e.to_string()), current, ev),
        };
        let cap = cfg.budget.element_cap(ev.cost.dofs, n);
        if !trim_to_budget(&mut marked.marked, &ev.fused.score, &ev.fused.mandatory, n, cap, children) {
            return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::BudgetExhausted, current, ev);
        }
        // growth per cycle: even mandatory elements are limited, the highest ranked first
        let growth_cap = (cfg.max_growth.max(1.0) * n as f64).floor() as usize;
        trim_to_budget(&mut marked.marked, &ev.fused.score, &vec![false; n], n, growth_cap, children);
        marked.n_marked = marked.marked.iter().filter(|&&m| m).count();
        marked_flags = marked.marked.clone();
        rec.marked = marked.n_marked;
        if marked.n_marked == 0 {
            return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::NothingToRefine, current, ev);
        }
        let h_new: Vec<f64> = (0..n)
            .map(|e| {
                let h = if marked.marked[e] { ev.h_elem[e] * cfg.refine_ratio } else { ev.h_elem[e] };
                h.clamp(cfg.adapt.h_min.max(f64::MIN_POSITIVE), cfg.adapt.h_max)
            })
            .collect();
        let size = if dim == 2 {
            size_field_at_nodes(&current.model.mesh, &h_new, &cfg.adapt, NodalSize::Min).map(AdaptSize::D2)
        } else {
            size_field_at_nodes_3d(&current.model.mesh, &h_new, &cfg.adapt, NodalSize::Min).map(AdaptSize::D3)
        };
        let size = match size {
            Ok(s) => s,
            Err(e) => return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::RolledBack(format!("size field: {e}")), current, ev),
        };
        rec.seconds.marking_and_sizing = t.elapsed().as_secs_f64();

        // ---- mesh, verify, solve (a failure anywhere rolls back to `current`)
        let t = Instant::now();
        let candidate = mesher(&size);
        let meshing = t.elapsed().as_secs_f64();
        let next = match candidate {
            Ok(p) => p,
            Err(e) => {
                rec.seconds.meshing = meshing;
                return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::RolledBack(format!("the mesher failed: {e}")), current, ev);
            }
        };
        let t = Instant::now();
        let checks = (|| -> Result<(), String> {
            next.model.mesh.quality().require(&cfg.quality).map_err(|e| e.to_string())?;
            next.model.mesh.conformity().require_conforming().map_err(|e| e.to_string())?;
            state_check(&current, &next, cfg.state_tolerance)
        })();
        let checks_s = t.elapsed().as_secs_f64();
        rec.seconds.meshing = meshing;
        rec.seconds.checks = checks_s;
        if let Err(why) = checks {
            return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::RolledBack(why), current, ev);
        }
        let next_adj = next.model.mesh.adjacency();
        let next_ev = match evaluate(&next, goals, cfg, &next_adj) {
            Ok(e) => e,
            Err(e) => return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::RolledBack(format!("the new mesh could not be solved or estimated: {e}")), current, ev),
        };
        if !cfg.budget.allows(next.model.mesh.n_dofs(), next.model.mesh.n_elems(), 8 * next_ev.cost.factor_nnz) {
            return finish(&mut history, &mut fields_acc, &marked_flags, rec, Stop::BudgetExhausted, current, ev);
        }
        // how far did the solution move: previous displacement carried onto the new mesh
        change = (|| {
            let snap = SolverSnapshot::new(&current.model.mesh, ev.u.clone(), current.loads.delta_t, "previous").ok()?;
            let t = NodalInterpolation { max_unlocated_fraction: 0.2 }.transfer(&current.model, &snap, &next.model.mesh).ok()?;
            let num: f64 = t.snapshot.u.iter().zip(&next_ev.u).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
            let den: f64 = next_ev.u.iter().map(|b| b * b).sum::<f64>().sqrt();
            (den > 0.0).then_some(num / den)
        })();
        if cfg.keep_fields == FieldRetention::All {
            fields_acc.push(cycle_fields(rec.cycle, &current, &ev, &marked_flags));
        }
        history.push(rec);
        current = next;
        ev = next_ev;
    }
    unreachable!("the loop returns at max_cycles")
}

impl ZoneMap {
    /// Elements of the zone.
    pub fn elements_of(&self, z: Zone) -> Vec<usize> {
        (0..self.zones.len()).filter(|&e| self.zones[e] == z).collect()
    }
}
