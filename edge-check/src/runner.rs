//! Runs the registered models on one [`EdgeInput`]: margins at the actual
//! edge distance, the smallest edge distance that satisfies each check, the
//! effect of the interference fit, and a Monte-Carlo pass over the fit-
//! pressure (and optional strength) scatter. Models are independent; a
//! model that cannot evaluate the case reports why and the rest still run.

use crate::mc::{inverse_normal_cdf, latin_hypercube, summarize, MarginStats};
use crate::model::{EdgeModel, Response};
use crate::types::{Geometry, Loads, Mode, Strengths};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeInput {
    pub geom: Geometry,
    pub strengths: Strengths,
    /// Applied (design) pin load toward the edge, lbf.
    pub applied_load: f64,
    /// Nominal interference contact pressure, psi, and the band it sweeps
    /// over the bore/interference tolerances.
    pub fit_pressure: f64,
    pub fit_pressure_min: f64,
    pub fit_pressure_max: f64,
}

#[derive(Debug, Clone)]
pub struct EdgeConfig {
    /// Monte-Carlo samples (0 disables the pass).
    pub mc_samples: usize,
    pub seed: u64,
    /// Coefficient of variation of the material strengths in the Monte-Carlo
    /// pass and the recommended-edge-distance search (0 = none). The default
    /// 5 % is an assumption for typical (not A-basis) values, not user data.
    pub strength_cv: f64,
    /// Include each model's own prediction error (`EdgeModel::model_cv`) in the
    /// Monte-Carlo pass and the recommended edge distances.
    pub model_error: bool,
    /// `Fbru` at `e/D = 1.5` (psi) for the tabulated-allowable check.
    pub fbru_e15: Option<f64>,
    /// Edge-distance search bracket, in multiples of the bore diameter.
    pub search_lo: f64,
    pub search_hi: f64,
    /// Also run the elastic-plastic FE limit-load model (~1-3 s): the
    /// bushing-and-housing contact model when `bushing` is known, else the
    /// dead-load plate model.
    pub include_plastic: bool,
    /// The bushing pressed into the bore (needed by the contact model).
    pub bushing: Option<crate::types::BushingSpec>,
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self { mc_samples: 2000, seed: 0x5EED_ED6E, strength_cv: 0.05, model_error: true, fbru_e15: None, search_lo: 0.75, search_hi: 8.0, include_plastic: false, bushing: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// The user's applied load.
    Applied,
    /// The load the joint can deliver in bearing, `Fbru * D * t` - the edge
    /// must not fail before the bearing does (the "Bearing" target).
    BearingLimit,
}

/// One pass/fail question asked of every model.
#[derive(Debug, Clone, Copy)]
pub struct Target {
    pub label: &'static str,
    pub case: Case,
    pub modes: &'static [Mode],
}

pub const TARGETS: [Target; 3] = [
    Target { label: "Strength (applied load)", case: Case::Applied, modes: &[Mode::ShearOut, Mode::Splitting, Mode::Bearing, Mode::Collapse] },
    Target { label: "Bearing (bearing-limit load)", case: Case::BearingLimit, modes: &[Mode::ShearOut, Mode::Splitting, Mode::Collapse] },
    Target { label: "First yield (applied load)", case: Case::Applied, modes: &[Mode::FirstYield] },
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgeMin {
    /// Smallest edge distance (in) with margin >= 0.
    Value(f64),
    /// Already satisfied at the lower end of the search range (in).
    AtMost(f64),
    /// Not satisfied up to the upper end of the search range (in).
    Exceeds(f64),
    NotSearched,
}

impl EdgeMin {
    /// Ordering for "which requirement is stricter": exceeding the search
    /// range > a found value > already satisfied at the lower end.
    pub fn rank(self) -> (i8, f64) {
        match self {
            EdgeMin::Exceeds(v) => (2, v),
            EdgeMin::Value(v) => (1, v),
            EdgeMin::AtMost(v) => (0, v),
            EdgeMin::NotSearched => (-1, 0.0),
        }
    }

    /// The stricter of two requirements.
    pub fn larger(self, other: EdgeMin) -> EdgeMin {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

#[derive(Debug, Clone)]
pub struct TargetResult {
    pub target: Target,
    /// Lowest margin among the target's modes at the actual edge distance.
    pub margin: f64,
    pub governing: Mode,
    /// The same with the interference fit removed (isolates its effect).
    pub margin_no_fit: f64,
    /// Pin load (lbf) at which the target's lowest margin reaches zero at the
    /// actual edge distance and nominal fit: what the joint can carry for
    /// this check. `0` = fails with no pin load at all; infinite = never
    /// fails in the searched range. Compare with the target's load.
    pub capacity_lbf: f64,
    /// The same with the interference fit removed; the shortfall against
    /// `capacity_lbf` is what the fit consumes.
    pub capacity_no_fit_lbf: f64,
    pub e_min: EdgeMin,
    pub mc: Option<MarginStats>,
    /// Smallest edge distance whose failure probability, over the fit-pressure
    /// band, material scatter and the model's own error, is at most
    /// 10 % / 5 % / 1 % (see [`CONFIDENCE`]). `NotSearched` for targets or
    /// models that do not support it.
    pub e_levels: [EdgeMin; 3],
}

/// Survival levels of the recommended edge distance (P90, P95, P99).
pub const CONFIDENCE: [f64; 3] = [0.90, 0.95, 0.99]; 

#[derive(Debug, Clone)]
pub struct ModelReport {
    pub id: &'static str,
    pub label: &'static str,
    /// Solves the same stress problem as the other field models (see
    /// [`EdgeModel::is_field_model`]).
    pub field_model: bool,
    pub error: Option<String>,
    pub targets: Vec<Option<TargetResult>>,
    pub notes: Vec<String>,
    pub elapsed: Duration,
    /// The model's own prediction-error CV used in its Monte-Carlo pass (0 if switched off).
    pub model_cv: f64,
}

impl ModelReport {
    /// This model's edge distance at survival level `level`: the larger of
    /// its strength and bearing targets. `None` if it has none.
    pub fn level(&self, level: usize) -> Option<EdgeMin> {
        self.targets.iter().flatten().filter(|t| recommends(&t.target)).map(|t| t.e_levels[level]).filter(|e| *e != EdgeMin::NotSearched).reduce(EdgeMin::larger)
    }
}

#[derive(Debug, Clone)]
pub struct EdgeReport {
    pub bore_diameter: f64,
    pub edge: f64,
    pub applied_load: f64,
    pub bearing_limit_load: f64,
    pub fit_pressure: f64,
    /// Material-strength CV assumed in the Monte-Carlo pass.
    pub strength_cv: f64,
    pub models: Vec<ModelReport>,
}

impl EdgeReport {
    /// The recommended (conservative) edge distance at survival level
    /// `level` (index into [`CONFIDENCE`]): the largest over the models and
    /// the strength / bearing targets, with the model that sets it. `None`
    /// if no model produced one.
    pub fn recommended(&self, level: usize) -> Option<(EdgeMin, &'static str)> {
        let mut best: Option<(EdgeMin, &'static str)> = None;
        for m in self.models.iter().filter(|m| m.error.is_none()) {
            if let Some(e) = m.level(level) {
                if best.is_none_or(|(b, _)| e.rank() > b.rank()) {
                    best = Some((e, m.id));
                }
            }
        }
        best
    }

    /// Whether the models that found a smallest edge distance for target
    /// `i` agree to within `rel` (fraction of the larger value) - `None`
    /// when fewer than two did.
    pub fn e_min_agree(&self, i: usize, rel: f64) -> Option<bool> {
        self.e_min_spread(i).map(|(lo, hi)| hi <= 0.0 || (hi - lo) / hi <= rel)
    }

    /// `(min, max)` of the finite smallest-edge-distance values across the
    /// field models for target `i` - how far they disagree.
    pub fn e_min_spread(&self, i: usize) -> Option<(f64, f64)> {
        let vals: Vec<f64> = self
            .models
            .iter()
            .filter(|m| m.field_model)
            .filter_map(|m| m.targets.get(i).and_then(|t| t.as_ref()))
            .filter_map(|t| match t.e_min {
                EdgeMin::Value(v) | EdgeMin::AtMost(v) => Some(v),
                _ => None,
            })
            .collect();
        let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (vals.len() >= 2).then_some((lo, hi))
    }
}

/// Pin load at which the target's lowest margin reaches zero for a fixed fit
/// pressure: bracket by doubling, then bisect (margins fall as the load
/// rises; where contact retention switches off they step down, which the
/// bisection still brackets).
fn capacity(resp: &dyn Response, target: &Target, fit: f64, scale: f64, start: f64) -> f64 {
    let m = |pin: f64| target_margin(resp, target, &Loads { fit_pressure: fit, pin_load: pin }, scale).map_or(f64::INFINITY, |x| x.0);
    let tiny = 1e-6;
    if m(tiny) < 0.0 {
        return 0.0;
    }
    let mut lo = tiny;
    let mut hi = start.max(1.0);
    let mut grew = 0;
    while m(hi) >= 0.0 {
        lo = hi;
        hi *= 2.0;
        grew += 1;
        if grew > 60 {
            return f64::INFINITY;
        }
    }
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if m(mid) >= 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo <= 1e-9 * hi {
            break;
        }
    }
    lo
}

fn loads_for(input: &EdgeInput, case: Case, fit: f64) -> Loads {
    let pin = match case {
        Case::Applied => input.applied_load,
        Case::BearingLimit => bearing_limit_load(input),
    };
    Loads { fit_pressure: fit, pin_load: pin }
}

pub fn bearing_limit_load(input: &EdgeInput) -> f64 {
    input.strengths.fbru * 2.0 * input.geom.bore_radius * input.geom.thickness
}

/// Lowest margin among `target.modes` the response reports, and which mode.
fn target_margin(resp: &dyn Response, target: &Target, loads: &Loads, scale: f64) -> Option<(f64, Mode)> {
    resp.margins(loads, scale).into_iter().filter(|m| target.modes.contains(&m.mode)).map(|m| (m.margin, m.mode)).min_by(|a, b| a.0.total_cmp(&b.0))
}

/// Illinois-method root of `eval` (a margin that grows with the edge
/// distance) on `[lo_in, hi_in]`: the smallest edge distance with
/// `eval >= 0`, or why there is none.
fn root_search(lo_in: f64, hi_in: f64, tol: f64, eval: &mut dyn FnMut(f64) -> Option<f64>) -> EdgeMin {
    let (Some(mut flo), Some(mut fhi)) = (eval(lo_in), eval(hi_in)) else {
        return EdgeMin::NotSearched;
    };
    if flo >= 0.0 {
        return EdgeMin::AtMost(lo_in);
    }
    if fhi < 0.0 {
        return EdgeMin::Exceeds(hi_in);
    }
    let (mut lo, mut hi) = (lo_in, hi_in);
    // Infinite margins (nothing applied) would break the secant step: clamp.
    let fin = |v: f64| if v.is_finite() { v } else { 1e6 };
    let mut side = 0i32;
    for _ in 0..30 {
        if hi - lo < tol {
            break;
        }
        let (a, b) = (fin(flo), fin(fhi));
        let mut e = if b - a > 0.0 { lo - a * (hi - lo) / (b - a) } else { 0.5 * (lo + hi) };
        if !(e > lo && e < hi) {
            e = 0.5 * (lo + hi);
        }
        let Some(fe) = eval(e) else {
            return EdgeMin::NotSearched;
        };
        if fe >= 0.0 {
            hi = e;
            fhi = fe;
            if side == 1 {
                flo *= 0.5;
            }
            side = 1;
        } else {
            lo = e;
            flo = fe;
            if side == -1 {
                fhi *= 0.5;
            }
            side = -1;
        }
    }
    EdgeMin::Value(hi)
}

/// Smallest edge distance for which `target`'s nominal margin is >= 0.
fn search_edge(model: &dyn EdgeModel, input: &EdgeInput, target: &Target, lo_in: f64, hi_in: f64, cache: &mut Vec<(f64, Vec<Option<f64>>)>) -> EdgeMin {
    let d = 2.0 * input.geom.bore_radius;
    let ti = TARGETS.iter().position(|t| t.label == target.label).unwrap();
    let mut eval = |e: f64| -> Option<f64> {
        if let Some((_, v)) = cache.iter().find(|(ce, _)| (*ce - e).abs() < 1e-12) {
            return v[ti];
        }
        let mut geom = input.geom;
        geom.edge = e;
        let resp = model.respond_for_search(&geom, &input.strengths, input.fit_pressure).ok()?;
        let vals: Vec<Option<f64>> = TARGETS.iter().map(|t| target_margin(&*resp, t, &loads_for(input, t.case, input.fit_pressure), 1.0).map(|x| x.0)).collect();
        let out = vals[ti];
        cache.push((e, vals));
        out
    };
    root_search(lo_in, hi_in, 0.004 * d, &mut eval)
}

/// Targets that get a recommended edge distance (everything but the
/// informational first-yield one).
fn recommends(target: &Target) -> bool {
    !target.modes.contains(&Mode::FirstYield)
}

/// One Monte-Carlo draw of `target`'s margin: fit pressure uniform over its
/// band; the edge strength and the model's own error as independent
/// normal factors on the capacity; for the bearing-limit load, the bearing
/// strength as a third independent factor on the load (independent, so the
/// recommendation is the more conservative of correlated and uncorrelated).
fn margin_sample(resp: &dyn Response, target: &Target, input: &EdgeInput, cfg: &EdgeConfig, model_cv: f64, u: &[f64]) -> f64 {
    let z = |k: usize| inverse_normal_cdf(u[k]).clamp(-3.0, 3.0);
    let fit = input.fit_pressure_min + (input.fit_pressure_max - input.fit_pressure_min) * u[0];
    let cap_scale = ((1.0 + cfg.strength_cv * z(1)) * (1.0 + model_cv * z(2))).max(0.05);
    let mut loads = loads_for(input, target.case, fit);
    if target.case == Case::BearingLimit {
        loads.pin_load *= (1.0 + cfg.strength_cv * z(3)).max(0.05);
    }
    target_margin(resp, target, &loads, cap_scale).map_or(f64::INFINITY, |x| x.0)
}

/// Samples used for the recommended-edge-distance search (fewer than the
/// reported Monte-Carlo pass: it is evaluated at every edge distance tried).
const LEVEL_SAMPLES: usize = 800;

/// For each target that has one, the edge distances at which the failure
/// probability under the full variability falls to 10 / 5 / 1 %: the
/// `(1 - confidence)` quantile of the sampled margin crosses zero.
fn search_levels(model: &dyn EdgeModel, input: &EdgeInput, cfg: &EdgeConfig, model_cv: f64, lo: f64, hi: f64) -> Vec<[EdgeMin; 3]> {
    let samples = latin_hypercube(LEVEL_SAMPLES, 4, cfg.seed ^ 0xA5A5);
    let d = 2.0 * input.geom.bore_radius;
    // edge -> per target, the margin quantile at each level
    let mut cache: Vec<(f64, Vec<[f64; 3]>)> = Vec::new();
    let mut out = vec![[EdgeMin::NotSearched; 3]; TARGETS.len()];
    for ti in (0..TARGETS.len()).filter(|&i| recommends(&TARGETS[i])) {
        for (li, conf) in CONFIDENCE.iter().enumerate() {
            let alpha = 1.0 - conf;
            let mut eval = |e: f64| -> Option<f64> {
                if let Some((_, v)) = cache.iter().find(|(ce, _)| (*ce - e).abs() < 1e-12) {
                    return Some(v[ti][li]);
                }
                let mut geom = input.geom;
                geom.edge = e;
                let resp = model.respond_for_search(&geom, &input.strengths, input.fit_pressure).ok()?;
                let per_target: Vec<[f64; 3]> = TARGETS
                    .iter()
                    .map(|t| {
                        if !recommends(t) {
                            return [f64::NAN; 3];
                        }
                        let mut ms: Vec<f64> = samples.iter().map(|u| margin_sample(&*resp, t, input, cfg, model_cv, u)).collect();
                        ms.sort_by(|a, b| a.total_cmp(b));
                        let q = |a: f64| ms[((a * ms.len() as f64) as usize).min(ms.len() - 1)];
                        [q(1.0 - CONFIDENCE[0]), q(1.0 - CONFIDENCE[1]), q(1.0 - CONFIDENCE[2])]
                    })
                    .collect();
                let v = per_target[ti][li];
                cache.push((e, per_target));
                Some(v)
            };
            let _ = alpha;
            out[ti][li] = root_search(lo, hi, 0.004 * d, &mut eval);
        }
    }
    out
}

pub fn run(models: &[Box<dyn EdgeModel>], input: &EdgeInput, cfg: &EdgeConfig) -> EdgeReport {
    let d = 2.0 * input.geom.bore_radius;
    let samples = if cfg.mc_samples > 0 { latin_hypercube(cfg.mc_samples, 4, cfg.seed) } else { Vec::new() };
    let mut reports = Vec::with_capacity(models.len());
    for model in models {
        let t0 = Instant::now();
        let model_cv = if cfg.model_error { model.model_cv() } else { 0.0 };
        let mut rep = ModelReport { id: model.id(), label: model.label(), field_model: model.is_field_model(), error: None, targets: vec![None; TARGETS.len()], notes: Vec::new(), elapsed: Duration::ZERO, model_cv };
        match model.respond(&input.geom, &input.strengths, input.fit_pressure) {
            Err(e) => rep.error = Some(e),
            Ok(resp) => {
                rep.notes.extend(resp.notes());
                if let Some(retained) = resp.contact_retained(&loads_for(input, Case::Applied, input.fit_pressure)) {
                    rep.notes.push(if retained {
                        "interference keeps the bushing in contact all round at the applied load (load shared with the back side)".to_string()
                    } else {
                        "applied load exceeds the fit's retention: contact is lost on the back side (load carried on the loaded half only)".to_string()
                    });
                }
                for (i, target) in TARGETS.iter().enumerate() {
                    let loads = loads_for(input, target.case, input.fit_pressure);
                    let Some((margin, governing)) = target_margin(&*resp, target, &loads, 1.0) else { continue };
                    let no_fit = target_margin(&*resp, target, &loads_for(input, target.case, 0.0), 1.0).map_or(f64::NAN, |x| x.0);
                    let mc = (!samples.is_empty()).then(|| {
                        let ms: Vec<f64> = samples.iter().map(|u| margin_sample(&*resp, target, input, cfg, model_cv, u)).collect();
                        summarize(&ms)
                    });
                    let start = loads.pin_load;
                    let cap = capacity(&*resp, target, input.fit_pressure, 1.0, start);
                    let cap_no_fit = capacity(&*resp, target, 0.0, 1.0, start);
                    rep.targets[i] = Some(TargetResult { target: *target, margin, governing, margin_no_fit: no_fit, capacity_lbf: cap, capacity_no_fit_lbf: cap_no_fit, e_min: EdgeMin::NotSearched, mc, e_levels: [EdgeMin::NotSearched; 3] });
                }
                if model.supports_edge_search() {
                    let mut cache = Vec::new();
                    for (i, target) in TARGETS.iter().enumerate() {
                        if let Some(tr) = rep.targets[i].as_mut() {
                            tr.e_min = search_edge(&**model, input, target, cfg.search_lo * d, cfg.search_hi * d, &mut cache);
                        }
                    }
                    if cfg.mc_samples > 0 {
                        let levels = search_levels(&**model, input, cfg, model_cv, cfg.search_lo * d, cfg.search_hi * d);
                        for (i, l) in levels.into_iter().enumerate() {
                            if let Some(tr) = rep.targets[i].as_mut() {
                                tr.e_levels = l;
                            }
                        }
                    }
                }
            }
        }
        rep.elapsed = t0.elapsed();
        reports.push(rep);
    }
    EdgeReport { bore_diameter: d, edge: input.geom.edge, applied_load: input.applied_load, bearing_limit_load: bearing_limit_load(input), fit_pressure: input.fit_pressure, strength_cv: cfg.strength_cv, models: reports }
}
