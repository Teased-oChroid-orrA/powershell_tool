//! Optional learned solve-time prediction with a confidence interval and a deterministic fallback (epic #61).
//!
//! The deterministic [`CostModel`] stays the authority. This module fits a log-linear model of each solver stage on measured
//! samples (assembly, factorisation and back-substitution seconds against the mesh's dofs, factor size and assembly work,
//! separately for 2D and 3D) by ridge regression, and answers with the prediction, an interval calibrated on held-out
//! (leave-one-out) residuals, and which source produced it. It falls back to the deterministic model, saying why, when the
//! training set is too small, the mesh is out of the training distribution (feature range or leverage), the solve would be
//! iterative (not modelled) or the model is not finite. A learned number is never used silently: [`Prediction::source`] is
//! part of the answer. Nothing here trains on its own output or reads telemetry; the caller supplies verified samples.

use super::cost::{CostModel, MeshCost, Workload};

/// One measured solve: the mesh's cost features and the seconds each stage took.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub cost: MeshCost,
    pub dim: usize,
    pub assemble_s: f64,
    pub factor_s: f64,
    pub solve_s: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    Learned,
    /// The deterministic model answered, for this reason.
    Fallback(Reason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Fewer training samples than the model needs for its features.
    TooFewSamples,
    /// A feature lies outside the range seen in training (extrapolation).
    OutsideTrainingRange,
    /// The mesh is far from the training points in the model's own geometry (high leverage).
    HighLeverage,
    /// `SolveMethod::Auto` would solve this system iteratively; no learned model covers it.
    IterativeRegime,
    /// The learned value was not finite and positive.
    NotFinite,
    /// The mesh dimension was not in the training data.
    UnseenDimension,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prediction {
    /// Seconds for the whole workload.
    pub seconds: f64,
    /// A conservative interval for the solver part (`lo <= seconds <= hi` for the learned source; both equal `seconds` for the
    /// deterministic one, which carries no interval).
    pub low: f64,
    pub high: f64,
    pub source: Source,
}

/// Minimum samples per dimension to fit.
pub const MIN_SAMPLES: usize = 8;
const FEATURES: usize = 4;

#[derive(Debug, Clone)]
struct StageFit {
    /// Coefficients on `[1, ln dofs, ln factor_nnz, ln assembly_units]`.
    w: [f64; FEATURES],
    /// Half-width of the interval in log space: the empirical 90th percentile of the leave-one-out absolute residuals.
    log_half_width: f64,
}

#[derive(Debug, Clone)]
struct DimFit {
    dim: usize,
    stages: [StageFit; 3],
    /// Inverse of `X'X + ridge I` for the leverage of a new point, and the largest training leverage.
    inverse: [[f64; FEATURES]; FEATURES],
    max_leverage: f64,
    /// Observed range of each non-constant feature.
    low: [f64; FEATURES],
    high: [f64; FEATURES],
    samples: usize,
}

/// A fitted learned model (one fit per mesh dimension seen in the training data).
#[derive(Debug, Clone)]
pub struct LearnedCost {
    fits: Vec<DimFit>,
}

fn features(c: &MeshCost) -> [f64; FEATURES] {
    [1.0, (c.dofs as f64).max(1.0).ln(), (c.factor_nnz as f64).max(1.0).ln(), c.assembly_units.max(1.0).ln()]
}

/// Solve `a x = b` (small, symmetric positive definite) by Gaussian elimination with partial pivoting.
fn solve(mut a: [[f64; FEATURES]; FEATURES], mut b: [f64; FEATURES]) -> Option<[f64; FEATURES]> {
    for i in 0..FEATURES {
        let p = (i..FEATURES).max_by(|&r, &s| a[r][i].abs().total_cmp(&a[s][i].abs()))?;
        if a[p][i].abs() < 1e-300 {
            return None;
        }
        a.swap(i, p);
        b.swap(i, p);
        for r in i + 1..FEATURES {
            let f = a[r][i] / a[i][i];
            let pivot_row = a[i];
            for (c, v) in a[r].iter_mut().enumerate().skip(i) {
                *v -= f * pivot_row[c];
            }
            b[r] -= f * b[i];
        }
    }
    let mut x = [0.0; FEATURES];
    for i in (0..FEATURES).rev() {
        let s: f64 = (i + 1..FEATURES).map(|c| a[i][c] * x[c]).sum();
        x[i] = (b[i] - s) / a[i][i];
    }
    Some(x)
}

fn invert(a: [[f64; FEATURES]; FEATURES]) -> Option<[[f64; FEATURES]; FEATURES]> {
    let mut inv = [[0.0; FEATURES]; FEATURES];
    for c in 0..FEATURES {
        let mut e = [0.0; FEATURES];
        e[c] = 1.0;
        let col = solve(a, e)?;
        for r in 0..FEATURES {
            inv[r][c] = col[r];
        }
    }
    Some(inv)
}

fn normal_matrix(rows: &[[f64; FEATURES]], ridge: f64) -> [[f64; FEATURES]; FEATURES] {
    let mut a = [[0.0; FEATURES]; FEATURES];
    for x in rows {
        for i in 0..FEATURES {
            for j in 0..FEATURES {
                a[i][j] += x[i] * x[j];
            }
        }
    }
    // The intercept is not penalised.
    for (i, row) in a.iter_mut().enumerate().skip(1) {
        row[i] += ridge;
    }
    a
}

fn fit_weights(rows: &[[f64; FEATURES]], y: &[f64], ridge: f64) -> Option<[f64; FEATURES]> {
    let a = normal_matrix(rows, ridge);
    let mut b = [0.0; FEATURES];
    for (x, t) in rows.iter().zip(y) {
        for i in 0..FEATURES {
            b[i] += x[i] * t;
        }
    }
    solve(a, b)
}

impl LearnedCost {
    /// Fit on `samples` (times must be finite and positive). Returns `None` when no dimension has [`MIN_SAMPLES`] usable samples.
    pub fn fit(samples: &[Sample], ridge: f64) -> Option<Self> {
        let mut fits = Vec::new();
        for dim in [2usize, 3] {
            let rows_of: Vec<&Sample> = samples
                .iter()
                .filter(|s| s.dim == dim && !s.cost.iterative && [s.assemble_s, s.factor_s, s.solve_s].iter().all(|t| t.is_finite() && *t > 0.0))
                .collect();
            if rows_of.len() < MIN_SAMPLES {
                continue;
            }
            let rows: Vec<[f64; FEATURES]> = rows_of.iter().map(|s| features(&s.cost)).collect();
            let a = normal_matrix(&rows, ridge);
            let inverse = invert(a)?;
            let leverage = |x: &[f64; FEATURES]| -> f64 { (0..FEATURES).map(|i| (0..FEATURES).map(|j| x[i] * inverse[i][j] * x[j]).sum::<f64>()).sum() };
            let max_leverage = rows.iter().map(leverage).fold(0.0, f64::max);
            let mut low = [f64::INFINITY; FEATURES];
            let mut high = [f64::NEG_INFINITY; FEATURES];
            for x in &rows {
                for i in 1..FEATURES {
                    low[i] = low[i].min(x[i]);
                    high[i] = high[i].max(x[i]);
                }
            }
            let mut stages: Vec<StageFit> = Vec::new();
            for stage in 0..3 {
                let y: Vec<f64> = rows_of.iter().map(|s| [s.assemble_s, s.factor_s, s.solve_s][stage].ln()).collect();
                let w = fit_weights(&rows, &y, ridge)?;
                // Leave-one-out residuals: refit without each sample.
                let mut residuals = Vec::new();
                for i in 0..rows.len() {
                    let sub_rows: Vec<[f64; FEATURES]> = rows.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| *r).collect();
                    let sub_y: Vec<f64> = y.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, v)| *v).collect();
                    if let Some(wi) = fit_weights(&sub_rows, &sub_y, ridge) {
                        let p: f64 = (0..FEATURES).map(|k| wi[k] * rows[i][k]).sum();
                        residuals.push((y[i] - p).abs());
                    }
                }
                residuals.sort_by(f64::total_cmp);
                let q = residuals.get(((residuals.len() as f64 * 0.9).ceil() as usize).saturating_sub(1)).copied().unwrap_or(f64::INFINITY);
                stages.push(StageFit { w, log_half_width: q });
            }
            fits.push(DimFit { dim, stages: stages.try_into().ok()?, inverse, max_leverage, low, high, samples: rows.len() });
        }
        (!fits.is_empty()).then_some(Self { fits })
    }

    /// Predicted seconds for `workload` on a mesh of cost `cost` and dimension `dim`. The deterministic `fallback` answers (with
    /// the reason) whenever the learned model should not be trusted.
    pub fn predict(&self, cost: &MeshCost, dim: usize, workload: &Workload, generation_seconds: f64, fallback: &CostModel) -> Prediction {
        let deterministic = |reason: Reason| {
            let s = fallback.predict(cost, workload, generation_seconds);
            Prediction { seconds: s, low: s, high: s, source: Source::Fallback(reason) }
        };
        if cost.iterative {
            return deterministic(Reason::IterativeRegime);
        }
        let Some(fit) = self.fits.iter().find(|f| f.dim == dim) else {
            return deterministic(Reason::UnseenDimension);
        };
        let x = features(cost);
        if (1..FEATURES).any(|i| x[i] < fit.low[i] - 0.5 || x[i] > fit.high[i] + 0.5) {
            return deterministic(Reason::OutsideTrainingRange);
        }
        let leverage: f64 = (0..FEATURES).map(|i| (0..FEATURES).map(|j| x[i] * fit.inverse[i][j] * x[j]).sum::<f64>()).sum();
        if leverage > 2.0 * fit.max_leverage.max(FEATURES as f64 / fit.samples as f64) {
            return deterministic(Reason::HighLeverage);
        }
        let counts = [workload.assemblies, workload.factorisations, workload.solves];
        let (mut mid, mut lo, mut hi) = (0.0, 0.0, 0.0);
        for (stage, n) in fit.stages.iter().zip(counts) {
            let log_t: f64 = (0..FEATURES).map(|k| stage.w[k] * x[k]).sum();
            mid += n * log_t.exp();
            lo += n * (log_t - stage.log_half_width).exp();
            hi += n * (log_t + stage.log_half_width).exp();
        }
        let once = fallback.one_time_seconds(workload, generation_seconds);
        let (mid, lo, hi) = (once + workload.analyses * mid, once + workload.analyses * lo, once + workload.analyses * hi);
        if !(mid.is_finite() && mid > 0.0 && lo.is_finite() && hi.is_finite()) {
            return deterministic(Reason::NotFinite);
        }
        Prediction { seconds: mid, low: lo, high: hi, source: Source::Learned }
    }

    /// Samples the fit for `dim` was trained on.
    pub fn samples(&self, dim: usize) -> usize {
        self.fits.iter().find(|f| f.dim == dim).map_or(0, |f| f.samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(dofs: usize, nnz: usize, units: f64) -> MeshCost {
        MeshCost { dofs, elements: dofs / 3, matrix_nnz: dofs * 20, factor_nnz: nnz, assembly_units: units, factor_units: (nnz as f64).powf(1.5), iterative: false }
    }

    /// Exact power laws in the three features, so a log-linear fit must recover them.
    fn synthetic(dim: usize, n: usize) -> Vec<Sample> {
        (0..n)
            .map(|i| {
                let dofs = 1000 * (1 + i * 3);
                let nnz = dofs * (30 + 5 * i);
                let units = dofs as f64 * (100.0 + 7.0 * i as f64);
                Sample {
                    cost: cost(dofs, nnz, units),
                    dim,
                    assemble_s: 1e-9 * units.powf(0.9),
                    factor_s: 2e-10 * (nnz as f64).powf(1.2),
                    solve_s: 3e-9 * (nnz as f64).powf(0.95),
                }
            })
            .collect()
    }

    fn model() -> CostModel {
        CostModel { assembly_s_per_unit: 1e-9, factor_s_per_unit: 1e-9, solve_s_per_factor_nnz: 1e-9 }
    }

    #[test]
    fn exact_power_laws_are_recovered_and_the_interval_is_tight() {
        let train = synthetic(3, 16);
        let m = LearnedCost::fit(&train, 1e-9).expect("enough samples");
        // A held-out size inside the training range.
        let probe = synthetic(3, 16)[7].cost;
        let w = Workload::linear_static();
        let p = m.predict(&probe, 3, &w, 0.0, &model());
        assert_eq!(p.source, Source::Learned);
        let truth = {
            let s = &synthetic(3, 16)[7];
            s.assemble_s + s.factor_s + s.solve_s
        };
        assert!((p.seconds / truth - 1.0).abs() < 1e-3, "{} vs {truth}", p.seconds);
        assert!(p.low <= p.seconds && p.seconds <= p.high);
        assert!(p.high / p.low < 1.05, "noise-free data gives a tight interval: {p:?}");
    }

    #[test]
    fn untrustworthy_cases_fall_back_to_the_deterministic_model_and_say_why() {
        let m = LearnedCost::fit(&synthetic(3, 16), 1e-6).unwrap();
        let w = Workload::linear_static();
        let det = model();
        // Too few samples: no fit at all.
        assert!(LearnedCost::fit(&synthetic(3, MIN_SAMPLES - 1), 1e-6).is_none());
        // Far beyond the training range.
        let big = cost(5_000_000, 5_000_000 * 60, 1e10);
        assert_eq!(m.predict(&big, 3, &w, 0.0, &det).source, Source::Fallback(Reason::OutsideTrainingRange));
        // Unseen dimension.
        assert_eq!(m.predict(&synthetic(3, 16)[4].cost, 2, &w, 0.0, &det).source, Source::Fallback(Reason::UnseenDimension));
        // Iterative regime.
        let mut it = synthetic(3, 16)[4].cost;
        it.iterative = true;
        assert_eq!(m.predict(&it, 3, &w, 0.0, &det).source, Source::Fallback(Reason::IterativeRegime));
        // A fallback answer is exactly the deterministic one, with no interval.
        let p = m.predict(&it, 3, &w, 2.0, &det);
        assert_eq!(p.seconds, det.predict(&it, &w, 2.0));
        assert_eq!((p.low, p.high), (p.seconds, p.seconds));
        // Non-finite or non-positive times are not training data.
        let mut bad = synthetic(3, 16);
        bad[0].solve_s = f64::NAN;
        bad[1].factor_s = 0.0;
        let m2 = LearnedCost::fit(&bad, 1e-6).unwrap();
        assert_eq!(m2.samples(3), 14);
        // Iterative samples are excluded from training.
        let mut it_samples = synthetic(3, 16);
        for s in it_samples.iter_mut().take(9) {
            s.cost.iterative = true;
        }
        assert!(LearnedCost::fit(&it_samples, 1e-6).is_none());
    }

    #[test]
    fn one_time_terms_and_analysis_counts_pass_through_unchanged() {
        let m = LearnedCost::fit(&synthetic(2, 12), 1e-9).unwrap();
        let probe = synthetic(2, 12)[5].cost;
        let one = Workload::linear_static();
        let many = one.repeated(10.0);
        let det = model();
        let (a, b) = (m.predict(&probe, 2, &one, 0.0, &det), m.predict(&probe, 2, &many, 0.0, &det));
        assert!((b.seconds / a.seconds - 10.0).abs() < 1e-9);
        let with_cad = Workload { cad_seconds: 5.0, ..one };
        let c = m.predict(&probe, 2, &with_cad, 1.0, &det);
        assert!((c.seconds - a.seconds - 6.0).abs() < 1e-9);
    }
}
