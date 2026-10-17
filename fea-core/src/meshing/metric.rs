//! Validated spatial metric tensors for size-controlled and anisotropic meshing.
//!
//! A metric `M` is a symmetric positive-definite 3x3 matrix; a vector `e` has length
//! `sqrt(e^T M e)`, and a metric-unit mesh has every edge of length one. Provided here:
//! construction from sizes and directions, validation against size and anisotropy
//! bounds, eigen-decomposition, metric-space edge length, log-Euclidean
//! interpolation, intersection (the smaller size in every direction), gradation
//! (bounded size growth along mesh edges), curvature- and error-driven size
//! metrics, and the expected element count (complexity) with budget rescaling.
//! Nothing here meshes: it only builds and checks the field a mesher is driven by.
#![allow(clippy::needless_range_loop)] // index-parallel 3x3 kernels read clearer as loops
use crate::adaptivity::estimator::{ErrorField, Indicator};
use std::fmt;
pub type V3 = [f64; 3];
type M3 = [[f64; 3]; 3];
/// Why a metric or field was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum MetricError {
    NotFinite,
    NotSymmetric {
        asymmetry: f64,
    },
    NotPositiveDefinite {
        smallest_eigenvalue: f64,
    },
    SizeOutOfBounds {
        size: f64,
        min: f64,
        max: f64,
    },
    TooAnisotropic {
        ratio: f64,
        max: f64,
    },
    InvalidInput(String),
    /// A field entry failed validation (the index is the vertex or element).
    Entry {
        index: usize,
        cause: Box<MetricError>,
    },
    /// The error field cannot drive sizes (failed, unsupported or signed).
    UnusableErrorField(String),
}
impl fmt::Display for MetricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for MetricError {}
/// Allowed element sizes and anisotropy. `min_size <= h <= max_size` in every
/// principal direction and `h_max/h_min <= max_anisotropy` at every point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricBounds {
    pub min_size: f64,
    pub max_size: f64,
    pub max_anisotropy: f64,
}
impl MetricBounds {
    pub fn new(min_size: f64, max_size: f64, max_anisotropy: f64) -> Result<Self, MetricError> {
        if !(min_size.is_finite() && max_size.is_finite() && max_anisotropy.is_finite())
            || min_size <= 0.
            || max_size < min_size
            || max_anisotropy < 1.
        {
            return Err(MetricError::InvalidInput(
                "need 0 < min_size <= max_size and max_anisotropy >= 1".into(),
            ));
        }
        Ok(Self {
            min_size,
            max_size,
            max_anisotropy,
        })
    }
}
fn mul(a: M3, b: M3) -> M3 {
    let mut c = [[0.; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                c[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    c
}
/// Cyclic Jacobi: eigenvalues (ascending) and orthonormal eigenvectors as columns.
fn jacobi(mut a: M3) -> ([f64; 3], M3) {
    let mut v = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for _ in 0..64 {
        let off = a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2];
        let scale: f64 = a[0][0] * a[0][0] + a[1][1] * a[1][1] + a[2][2] * a[2][2] + 2. * off;
        if off <= 1e-34 * scale.max(f64::MIN_POSITIVE) {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q] == 0. {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2. * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.).sqrt());
            let (c, s) = (1. / (t * t + 1.).sqrt(), t / (t * t + 1.).sqrt());
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = c * akp - s * akq;
                a[k][q] = s * akp + c * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = c * apk - s * aqk;
                a[q][k] = s * apk + c * aqk;
            }
            for k in 0..3 {
                let (vkp, vkq) = (v[k][p], v[k][q]);
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    (
        order.map(|i| a[i][i]),
        std::array::from_fn(|r| std::array::from_fn(|c| v[r][order[c]])),
    )
}
/// Symmetric positive-definite metric. Construct through the checked builders.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metric(M3);
impl Metric {
    /// Unchecked wrapper for a matrix that is known SPD (results of metric algebra).
    fn raw(m: M3) -> Self {
        Self(m)
    }
    /// Check symmetry (relative 1e-10), finiteness and positive definiteness.
    pub fn new(m: [[f64; 3]; 3]) -> Result<Self, MetricError> {
        if m.iter().flatten().any(|x| !x.is_finite()) {
            return Err(MetricError::NotFinite);
        }
        let scale = m.iter().flatten().fold(0f64, |s, x| s.max(x.abs()));
        let asym = (0..3)
            .flat_map(|i| (0..3).map(move |j| (i, j)))
            .map(|(i, j)| (m[i][j] - m[j][i]).abs())
            .fold(0., f64::max);
        if asym > 1e-10 * scale.max(f64::MIN_POSITIVE) {
            return Err(MetricError::NotSymmetric { asymmetry: asym });
        }
        let sym: M3 = std::array::from_fn(|i| std::array::from_fn(|j| 0.5 * (m[i][j] + m[j][i])));
        let (values, _) = jacobi(sym);
        if values[0] <= 0. || values[0] < 1e-14 * values[2] {
            return Err(MetricError::NotPositiveDefinite {
                smallest_eigenvalue: values[0],
            });
        }
        Ok(Self(sym))
    }
    /// Isotropic metric of size `h` (`M = I / h^2`).
    pub fn isotropic(h: f64) -> Result<Self, MetricError> {
        if !(h.is_finite() && h > 0.) {
            return Err(MetricError::InvalidInput(
                "size must be positive and finite".into(),
            ));
        }
        let d = 1. / (h * h);
        Self::new([[d, 0., 0.], [0., d, 0.], [0., 0., d]])
    }
    /// `M = sum_i v_i v_i^T / h_i^2` for orthonormal directions `v_i` (checked to 1e-9).
    pub fn from_sizes(directions: [V3; 3], sizes: [f64; 3]) -> Result<Self, MetricError> {
        if sizes.iter().any(|h| !(h.is_finite() && *h > 0.)) {
            return Err(MetricError::InvalidInput(
                "sizes must be positive and finite".into(),
            ));
        }
        for i in 0..3 {
            for j in 0..3 {
                let d: f64 = (0..3).map(|k| directions[i][k] * directions[j][k]).sum();
                if (d - if i == j { 1. } else { 0. }).abs() > 1e-9 {
                    return Err(MetricError::InvalidInput(
                        "directions must be orthonormal".into(),
                    ));
                }
            }
        }
        let mut m = [[0.; 3]; 3];
        for (v, h) in directions.iter().zip(sizes) {
            for i in 0..3 {
                for j in 0..3 {
                    m[i][j] += v[i] * v[j] / (h * h);
                }
            }
        }
        Self::new(m)
    }
    pub fn matrix(&self) -> [[f64; 3]; 3] {
        self.0
    }
    /// Principal sizes (ascending: the largest eigenvalue gives the smallest size)
    /// and the matching unit directions.
    pub fn principal(&self) -> ([f64; 3], [V3; 3]) {
        let (values, v) = jacobi(self.0);
        let column = |c: usize| -> V3 { std::array::from_fn(|r| v[r][c]) };
        (
            [
                1. / values[2].sqrt(),
                1. / values[1].sqrt(),
                1. / values[0].sqrt(),
            ],
            [column(2), column(1), column(0)],
        )
    }
    /// Check the principal sizes and anisotropy against `bounds` (relative slack 1e-9).
    pub fn validate(&self, bounds: &MetricBounds) -> Result<(), MetricError> {
        let (sizes, _) = self.principal();
        let (small, large) = (sizes[0], sizes[2]);
        let slack = 1e-9;
        for h in [small, large] {
            if h < bounds.min_size * (1. - slack) || h > bounds.max_size * (1. + slack) {
                return Err(MetricError::SizeOutOfBounds {
                    size: h,
                    min: bounds.min_size,
                    max: bounds.max_size,
                });
            }
        }
        if large / small > bounds.max_anisotropy * (1. + slack) {
            return Err(MetricError::TooAnisotropic {
                ratio: large / small,
                max: bounds.max_anisotropy,
            });
        }
        Ok(())
    }
    /// `M^{1/2} v`: maps a vector into the space where the metric is the identity,
    /// so an element of unit metric edges becomes a unit-edge element.
    pub fn transform(&self, v: V3) -> V3 {
        let r = self.power(0.5);
        std::array::from_fn(|i| (0..3).map(|j| r[i][j] * v[j]).sum())
    }
    /// Arithmetic mean of metrics (SPD, cheap); `None` for an empty slice.
    pub fn mean(metrics: &[Metric]) -> Option<Metric> {
        if metrics.is_empty() {
            return None;
        }
        let n = metrics.len() as f64;
        let mut m = [[0.; 3]; 3];
        for x in metrics {
            for i in 0..3 {
                for j in 0..3 {
                    m[i][j] += x.0[i][j] / n;
                }
            }
        }
        Some(Metric::raw(m))
    }
    /// `sqrt(e^T M e)`.
    pub fn length(&self, e: V3) -> f64 {
        let mut s = 0.;
        for i in 0..3 {
            for j in 0..3 {
                s += e[i] * self.0[i][j] * e[j];
            }
        }
        s.max(0.).sqrt()
    }
    /// Density `sqrt(det M)`: metric-unit volume per unit volume.
    pub fn density(&self) -> f64 {
        let m = self.0;
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        det.max(0.).sqrt()
    }
    /// Sizes scaled by `factor` (`M / factor^2`).
    pub fn scaled_sizes(&self, factor: f64) -> Result<Self, MetricError> {
        if !(factor.is_finite() && factor > 0.) {
            return Err(MetricError::InvalidInput(
                "size factor must be positive".into(),
            ));
        }
        Ok(Self::raw(self.0.map(|r| r.map(|x| x / (factor * factor)))))
    }
    fn apply_to_eigenvalues(&self, f: impl Fn(f64) -> f64) -> M3 {
        let (values, v) = jacobi(self.0);
        let mut out = [[0.; 3]; 3];
        for k in 0..3 {
            let l = f(values[k]);
            for i in 0..3 {
                for j in 0..3 {
                    out[i][j] += l * v[i][k] * v[j][k];
                }
            }
        }
        out
    }
    fn power(&self, p: f64) -> M3 {
        self.apply_to_eigenvalues(|l| l.powf(p))
    }
    fn log(&self) -> M3 {
        self.apply_to_eigenvalues(f64::ln)
    }
    /// Log-Euclidean interpolation `exp((1-t) log A + t log B)`: stays SPD, is
    /// symmetric in its arguments and gives the geometric mean of commuting sizes.
    pub fn interpolate(&self, other: &Metric, t: f64) -> Result<Metric, MetricError> {
        if !(0. ..=1.).contains(&t) {
            return Err(MetricError::InvalidInput("t must lie in [0, 1]".into()));
        }
        let (la, lb) = (self.log(), other.log());
        let blend: M3 =
            std::array::from_fn(|i| std::array::from_fn(|j| (1. - t) * la[i][j] + t * lb[i][j]));
        let (values, v) = jacobi(blend);
        let mut out = [[0.; 3]; 3];
        for k in 0..3 {
            let l = values[k].exp();
            for i in 0..3 {
                for j in 0..3 {
                    out[i][j] += l * v[i][k] * v[j][k];
                }
            }
        }
        Metric::new(symmetrise(out))
    }
    /// Intersection by simultaneous reduction: the metric whose unit ball is the
    /// largest ellipsoid inside both (the smaller size in every reduction direction).
    pub fn intersect(&self, other: &Metric) -> Result<Metric, MetricError> {
        let a_half = self.power(0.5);
        let a_inv_half = self.power(-0.5);
        // S = A^{-1/2} B A^{-1/2} is symmetric with the reduction eigenvalues.
        let s = symmetrise(mul(mul(a_inv_half, other.0), a_inv_half));
        let (d, q) = jacobi(s);
        let mut mid = [[0.; 3]; 3];
        for k in 0..3 {
            let l = d[k].max(1.);
            for i in 0..3 {
                for j in 0..3 {
                    mid[i][j] += l * q[i][k] * q[j][k];
                }
            }
        }
        Metric::new(symmetrise(mul(mul(a_half, mid), a_half)))
    }
}
fn symmetrise(m: M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| 0.5 * (m[i][j] + m[j][i])))
}
/// Length of the straight edge `p -> q` when the metric varies between `at_p` and
/// `at_q` (logarithmic mean of the two end lengths, the exact integral for sizes
/// that vary geometrically along the edge).
pub fn edge_length(p: V3, q: V3, at_p: &Metric, at_q: &Metric) -> f64 {
    let e: V3 = std::array::from_fn(|i| q[i] - p[i]);
    let (lp, lq) = (at_p.length(e), at_q.length(e));
    if (lp - lq).abs() <= 1e-12 * lp.max(lq) {
        0.5 * (lp + lq)
    } else {
        (lp - lq) / (lp / lq).ln()
    }
}
/// Check every metric of a field; the failure names the vertex.
pub fn validate_field(metrics: &[Metric], bounds: &MetricBounds) -> Result<(), MetricError> {
    for (index, m) in metrics.iter().enumerate() {
        m.validate(bounds).map_err(|cause| MetricError::Entry {
            index,
            cause: Box::new(cause),
        })?;
    }
    Ok(())
}
/// Result of [`gradate`].
#[derive(Debug, Clone, PartialEq)]
pub struct Gradation {
    pub sweeps: usize,
    pub updates: usize,
    pub converged: bool,
}
/// Bound the size growth along every mesh edge: after convergence the metric at
/// each end of an edge of length `l` (measured in the other end's metric) is no
/// larger than the other's scaled by `1 + beta * l` (Alauzet's gradation). Only
/// ever shrinks sizes, so the field stays within its lower bound.
pub fn gradate(
    vertices: &[V3],
    edges: &[[usize; 2]],
    metrics: &mut [Metric],
    beta: f64,
    max_sweeps: usize,
) -> Result<Gradation, MetricError> {
    if vertices.len() != metrics.len() || !(beta.is_finite() && beta > 0.) {
        return Err(MetricError::InvalidInput(
            "need one metric per vertex and beta > 0".into(),
        ));
    }
    if let Some(bad) = edges
        .iter()
        .find(|e| e.iter().any(|&v| v >= vertices.len()))
    {
        return Err(MetricError::InvalidInput(format!(
            "edge {bad:?} out of range"
        )));
    }
    let mut report = Gradation {
        sweeps: 0,
        updates: 0,
        converged: false,
    };
    for _ in 0..max_sweeps {
        report.sweeps += 1;
        let mut changed = 0;
        for &[i, j] in edges {
            for (from, to) in [(i, j), (j, i)] {
                let e: V3 = std::array::from_fn(|k| vertices[to][k] - vertices[from][k]);
                let l = metrics[from].length(e);
                let grown = metrics[from].scaled_sizes(1. + beta * l)?;
                let merged = metrics[to].intersect(&grown)?;
                let moved = (0..3)
                    .flat_map(|r| (0..3).map(move |c| (r, c)))
                    .map(|(r, c)| (merged.0[r][c] - metrics[to].0[r][c]).abs())
                    .fold(0., f64::max);
                let scale = metrics[to]
                    .0
                    .iter()
                    .flatten()
                    .fold(0f64, |s, x| s.max(x.abs()));
                if moved > 1e-9 * scale {
                    metrics[to] = merged;
                    changed += 1;
                }
            }
        }
        report.updates += changed;
        if changed == 0 {
            report.converged = true;
            break;
        }
    }
    Ok(report)
}
/// Metric from the local surface curvature: a chord deviation of at most `deviation`
/// gives size `sqrt(8 deviation / kappa)` along each principal direction (the chord
/// of a circle of radius `1/kappa`); flat directions and the normal take the
/// largest allowed size. `normal` and the two tangent `directions` must be orthonormal.
pub fn curvature_metric(
    curvatures: [f64; 2],
    directions: [V3; 2],
    normal: V3,
    deviation: f64,
    bounds: &MetricBounds,
) -> Result<Metric, MetricError> {
    if !(deviation.is_finite() && deviation > 0.) || curvatures.iter().any(|k| !k.is_finite()) {
        return Err(MetricError::InvalidInput(
            "deviation must be positive and curvatures finite".into(),
        ));
    }
    let size = |k: f64| {
        let k = k.abs();
        if k * bounds.max_size * bounds.max_size <= 8. * deviation {
            bounds.max_size
        } else {
            (8. * deviation / k)
                .sqrt()
                .clamp(bounds.min_size, bounds.max_size)
        }
    };
    let mut sizes = [size(curvatures[0]), size(curvatures[1]), bounds.max_size];
    let smallest = sizes.iter().copied().fold(f64::INFINITY, f64::min);
    for h in &mut sizes {
        *h = h.min(smallest * bounds.max_anisotropy);
    }
    let m = Metric::from_sizes([directions[0], directions[1], normal], sizes)?;
    m.validate(bounds)?;
    Ok(m)
}
/// Target element size from an error field: `h_new = h (target / error)^(1/order)`,
/// clamped to the bounds. Only usable, non-negative (marking) fields drive sizes;
/// failed, unsupported and signed fields are refused, never silently skipped.
pub fn sizes_from_error(
    field: &ErrorField,
    element_sizes: &[f64],
    target_error: f64,
    convergence_order: f64,
    bounds: &MetricBounds,
) -> Result<Vec<f64>, MetricError> {
    if !field.reliability.usable() || field.indicator != Indicator::Marking {
        return Err(MetricError::UnusableErrorField(format!(
            "{}: {:?} / {:?}",
            field.name, field.reliability, field.indicator
        )));
    }
    if field.values.len() != element_sizes.len() {
        return Err(MetricError::InvalidInput(
            "error field and element sizes differ in length".into(),
        ));
    }
    if !(target_error.is_finite()
        && target_error > 0.
        && convergence_order.is_finite()
        && convergence_order > 0.)
    {
        return Err(MetricError::InvalidInput(
            "target error and order must be positive".into(),
        ));
    }
    field
        .values
        .iter()
        .zip(element_sizes)
        .enumerate()
        .map(|(index, (&e, &h))| {
            if !(e.is_finite() && e >= 0. && h.is_finite() && h > 0.) {
                return Err(MetricError::Entry {
                    index,
                    cause: Box::new(MetricError::InvalidInput(
                        "error must be finite and non-negative, size positive".into(),
                    )),
                });
            }
            // A vanishing error asks for the largest allowed size, not infinity.
            let new = if e <= f64::MIN_POSITIVE {
                bounds.max_size
            } else {
                h * (target_error / e).powf(1. / convergence_order)
            };
            Ok(new.clamp(bounds.min_size, bounds.max_size))
        })
        .collect()
}
/// Isotropic vertex metrics from element sizes: the geometric mean of the sizes of
/// the elements around each vertex.
pub fn vertex_metrics_from_element_sizes(
    elements: &[[usize; 4]],
    vertex_count: usize,
    sizes: &[f64],
) -> Result<Vec<Metric>, MetricError> {
    if elements.len() != sizes.len() {
        return Err(MetricError::InvalidInput(
            "one size per element required".into(),
        ));
    }
    let mut sum = vec![0.; vertex_count];
    let mut count = vec![0usize; vertex_count];
    for (e, h) in elements.iter().zip(sizes) {
        for &v in e {
            if v >= vertex_count || !(h.is_finite() && *h > 0.) {
                return Err(MetricError::InvalidInput(
                    "element refers to a missing vertex or has a bad size".into(),
                ));
            }
            sum[v] += h.ln();
            count[v] += 1;
        }
    }
    (0..vertex_count)
        .map(|v| {
            if count[v] == 0 {
                Err(MetricError::Entry {
                    index: v,
                    cause: Box::new(MetricError::InvalidInput(
                        "vertex belongs to no element".into(),
                    )),
                })
            } else {
                Metric::isotropic((sum[v] / count[v] as f64).exp())
            }
        })
        .collect()
}
/// Volume of the regular unit-edge tetrahedron.
pub const UNIT_TET_VOLUME: f64 = 0.117_851_130_197_757_92; // sqrt(2) / 12
/// Expected number of tetrahedra of a metric-unit mesh of `elements`: each element's
/// volume times the mean vertex density, over the unit-tetrahedron volume.
pub fn tet_complexity(
    vertices: &[V3],
    elements: &[[usize; 4]],
    metrics: &[Metric],
) -> Result<f64, MetricError> {
    if vertices.len() != metrics.len() {
        return Err(MetricError::InvalidInput(
            "one metric per vertex required".into(),
        ));
    }
    let mut total = 0.;
    for (index, t) in elements.iter().enumerate() {
        if t.iter().any(|&v| v >= vertices.len()) {
            return Err(MetricError::Entry {
                index,
                cause: Box::new(MetricError::InvalidInput("vertex out of range".into())),
            });
        }
        let [a, b, c, d] = t.map(|v| vertices[v]);
        let u: V3 = std::array::from_fn(|i| b[i] - a[i]);
        let v: V3 = std::array::from_fn(|i| c[i] - a[i]);
        let w: V3 = std::array::from_fn(|i| d[i] - a[i]);
        let volume = (u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
            + u[2] * (v[0] * w[1] - v[1] * w[0]))
            .abs()
            / 6.;
        let density: f64 = t.iter().map(|&v| metrics[v].density()).sum::<f64>() / 4.;
        total += volume * density / UNIT_TET_VOLUME;
    }
    Ok(total)
}
/// Result of [`rescale_to_budget`].
#[derive(Debug, Clone, PartialEq)]
pub struct Budget {
    pub complexity: f64,
    pub iterations: usize,
    /// Whether the complexity is within `tolerance` of the target (it cannot be when
    /// the size bounds clamp the field).
    pub met: bool,
}
/// Scale all sizes by one factor, re-clamped to `bounds`, until the expected
/// element count is within `tolerance` (relative) of `target`. Complexity scales as
/// `factor^-3`, so the iteration converges quickly unless the bounds bind.
pub fn rescale_to_budget(
    metrics: &mut [Metric],
    bounds: &MetricBounds,
    target: f64,
    tolerance: f64,
    complexity: impl Fn(&[Metric]) -> Result<f64, MetricError>,
) -> Result<Budget, MetricError> {
    if !(target.is_finite() && target > 0. && tolerance > 0.) {
        return Err(MetricError::InvalidInput(
            "target and tolerance must be positive".into(),
        ));
    }
    let mut now = complexity(metrics)?;
    for iteration in 0..40 {
        if (now - target).abs() <= tolerance * target {
            return Ok(Budget {
                complexity: now,
                iterations: iteration,
                met: true,
            });
        }
        let factor = (now / target).powf(1. / 3.);
        let before = metrics.to_vec();
        for m in metrics.iter_mut() {
            *m = clamp_sizes(&m.scaled_sizes(factor)?, bounds)?;
        }
        let next = complexity(metrics)?;
        if before.iter().zip(metrics.iter()).all(|(a, b)| a == b) {
            return Ok(Budget {
                complexity: next,
                iterations: iteration + 1,
                met: (next - target).abs() <= tolerance * target,
            });
        }
        now = next;
    }
    Ok(Budget {
        complexity: now,
        iterations: 40,
        met: (now - target).abs() <= tolerance * target,
    })
}
/// Clamp principal sizes into the bounds (and anisotropy by raising the smallest sizes).
fn clamp_sizes(m: &Metric, bounds: &MetricBounds) -> Result<Metric, MetricError> {
    let (sizes, dirs) = m.principal();
    let mut h = sizes.map(|s| s.clamp(bounds.min_size, bounds.max_size));
    let smallest = h.iter().copied().fold(f64::INFINITY, f64::min);
    for s in &mut h {
        *s = s.min(smallest * bounds.max_anisotropy);
    }
    Metric::from_sizes(dirs, h)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptivity::estimator::Reliability;
    const AX: [V3; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    /// Reproducible SPD metric with rotated, anisotropic axes.
    fn random_metric(seed: u64) -> Metric {
        let mut s = seed;
        let mut next = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / (1u64 << 31) as f64
        };
        let (a, b, c) = (next() * 3., next() * 3., next() * 3.);
        let rot = |t: f64, i: usize, j: usize| {
            let mut m = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
            m[i][i] = t.cos();
            m[j][j] = t.cos();
            m[i][j] = -t.sin();
            m[j][i] = t.sin();
            m
        };
        let q = mul(mul(rot(a, 0, 1), rot(b, 1, 2)), rot(c, 0, 2));
        let dirs = [
            [q[0][0], q[1][0], q[2][0]],
            [q[0][1], q[1][1], q[2][1]],
            [q[0][2], q[1][2], q[2][2]],
        ];
        Metric::from_sizes(dirs, [0.05 + next(), 0.2 + 3. * next(), 1. + 10. * next()]).unwrap()
    }
    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * (1. + a.abs().max(b.abs()))
    }
    #[test]
    fn invalid_metrics_are_refused_with_a_reason() {
        assert_eq!(
            Metric::new([[f64::NAN, 0., 0.], [0., 1., 0.], [0., 0., 1.]]),
            Err(MetricError::NotFinite)
        );
        assert!(matches!(
            Metric::new([[1., 0.5, 0.], [0., 1., 0.], [0., 0., 1.]]),
            Err(MetricError::NotSymmetric { .. })
        ));
        assert!(matches!(
            Metric::new([[1., 0., 0.], [0., -1., 0.], [0., 0., 1.]]),
            Err(MetricError::NotPositiveDefinite { .. })
        ));
        assert!(matches!(
            Metric::new([[1., 0., 0.], [0., 1., 0.], [0., 0., 0.]]),
            Err(MetricError::NotPositiveDefinite { .. })
        ));
        assert!(Metric::isotropic(0.).is_err() && Metric::isotropic(f64::INFINITY).is_err());
        assert!(Metric::from_sizes([AX[0], AX[0], AX[2]], [1., 1., 1.]).is_err());
        assert!(MetricBounds::new(0., 1., 2.).is_err() && MetricBounds::new(2., 1., 2.).is_err());
    }
    #[test]
    fn bounds_on_size_and_anisotropy_are_enforced_per_entry() {
        let b = MetricBounds::new(0.1, 10., 20.).unwrap();
        let ok = Metric::from_sizes(AX, [0.5, 2., 10.]).unwrap();
        assert!(ok.validate(&b).is_ok());
        let tiny = Metric::from_sizes(AX, [0.05, 1., 1.]).unwrap();
        assert!(matches!(
            tiny.validate(&b),
            Err(MetricError::SizeOutOfBounds { .. })
        ));
        let huge = Metric::from_sizes(AX, [1., 1., 20.]).unwrap();
        assert!(matches!(
            huge.validate(&b),
            Err(MetricError::SizeOutOfBounds { .. })
        ));
        let skew = Metric::from_sizes(AX, [0.2, 5., 5.]).unwrap();
        assert!(matches!(
            skew.validate(&b),
            Err(MetricError::TooAnisotropic { .. })
        ));
        let field = [ok, ok, tiny, ok];
        assert!(matches!(
            validate_field(&field, &b),
            Err(MetricError::Entry { index: 2, .. })
        ));
    }
    #[test]
    fn eigen_sizes_directions_and_lengths_match_the_construction() {
        for seed in 1..20 {
            let m = random_metric(seed);
            let (sizes, dirs) = m.principal();
            assert!(sizes[0] <= sizes[1] && sizes[1] <= sizes[2]);
            let rebuilt = Metric::from_sizes(dirs, sizes).unwrap();
            for i in 0..3 {
                for j in 0..3 {
                    assert!(close(rebuilt.matrix()[i][j], m.matrix()[i][j], 1e-11));
                }
            }
            // The length along a principal direction is 1 / size.
            for k in 0..3 {
                assert!(close(m.length(dirs[k]), 1. / sizes[k], 1e-11));
            }
        }
        let m = Metric::from_sizes(AX, [2., 3., 1.]).unwrap();
        assert!(close(m.length([2., 0., 0.]), 1., 1e-14));
        assert!(close(m.length([0., 3., 0.]), 1., 1e-14));
        assert!(close(m.density(), 1. / 6., 1e-14));
    }
    #[test]
    fn interpolation_is_symmetric_exact_at_the_ends_and_geometric_for_commuting_sizes() {
        let a = Metric::from_sizes(AX, [1., 2., 4.]).unwrap();
        let b = Metric::from_sizes(AX, [4., 2., 1.]).unwrap();
        assert_eq!(
            a.interpolate(&b, 0.).unwrap().principal().0,
            a.principal().0
        );
        let mid = a.interpolate(&b, 0.5).unwrap();
        for h in mid.principal().0 {
            assert!(close(h, 2., 1e-12));
        }
        let (p, q) = (random_metric(3), random_metric(8));
        let (x, y) = (
            p.interpolate(&q, 0.3).unwrap(),
            q.interpolate(&p, 0.7).unwrap(),
        );
        for i in 0..3 {
            for j in 0..3 {
                assert!(close(x.matrix()[i][j], y.matrix()[i][j], 1e-10));
            }
        }
        assert!(p.interpolate(&q, 1.5).is_err());
    }
    #[test]
    fn intersection_takes_the_smaller_size_in_every_direction() {
        let a = Metric::from_sizes(AX, [1., 0.5, 1.]).unwrap();
        let b = Metric::from_sizes(AX, [0.5, 1., 1.]).unwrap();
        let m = a.intersect(&b).unwrap();
        for h in m.principal().0 {
            assert!(close(h, if h < 0.75 { 0.5 } else { 1. }, 1e-12));
        }
        for seed in 1..15 {
            let (p, q) = (random_metric(seed), random_metric(seed + 40));
            let m = p.intersect(&q).unwrap();
            // M dominates both: M - A and M - B are positive semi-definite, so no
            // vector is longer in A or B than in M.
            for k in 0..200 {
                let t = k as f64 * 0.37;
                let e = [t.sin(), (1.7 * t).cos(), (0.9 * t + 0.4).sin()];
                assert!(m.length(e) + 1e-12 >= p.length(e).max(q.length(e)) * (1. - 1e-9));
            }
            // And it is tight: along the reduction directions M equals the larger of the two.
            let a_inv_half = p.power(-0.5);
            let s = symmetrise(mul(mul(a_inv_half, q.matrix()), a_inv_half));
            let (_, qv) = jacobi(s);
            for c in 0..3 {
                let v = mul(
                    a_inv_half,
                    [[qv[0][c], 0., 0.], [qv[1][c], 0., 0.], [qv[2][c], 0., 0.]],
                );
                let e = [v[0][0], v[1][0], v[2][0]];
                assert!(close(m.length(e), p.length(e).max(q.length(e)), 1e-9));
            }
            // Idempotent and commutative.
            let n = q.intersect(&p).unwrap();
            for i in 0..3 {
                for j in 0..3 {
                    assert!(close(m.matrix()[i][j], n.matrix()[i][j], 1e-8));
                }
            }
            let again = m.intersect(&p).unwrap();
            for i in 0..3 {
                for j in 0..3 {
                    assert!(close(m.matrix()[i][j], again.matrix()[i][j], 1e-8));
                }
            }
        }
    }
    #[test]
    fn edge_length_is_the_logarithmic_mean_of_the_end_lengths() {
        let a = Metric::isotropic(1.).unwrap();
        let b = Metric::isotropic(4.).unwrap();
        // Sizes 1 -> 4 along an edge of length 6: lengths 6 and 1.5 at the ends.
        let l = edge_length([0., 0., 0.], [6., 0., 0.], &a, &b);
        assert!(close(l, (6. - 1.5) / (6f64 / 1.5).ln(), 1e-12));
        // Equal ends reduce to the plain length; order of the ends does not matter.
        assert!(close(
            edge_length([0., 0., 0.], [3., 0., 0.], &a, &a),
            3.,
            1e-14
        ));
        assert!(close(
            edge_length([0., 0., 0.], [6., 0., 0.], &b, &a),
            l,
            1e-12
        ));
        // Check against numerical integration of geometrically varying size h(s) = 4^s.
        let n = 20000;
        let numeric: f64 = (0..n)
            .map(|i| {
                let s = (i as f64 + 0.5) / n as f64;
                6. / 4f64.powf(s)
            })
            .sum::<f64>()
            / n as f64;
        assert!(close(l, numeric, 1e-6));
    }
    #[test]
    fn gradation_bounds_the_size_growth_along_edges() {
        // Sizes 1 and 100 at distance 10 along a chain of three vertices.
        let v = [[0., 0., 0.], [10., 0., 0.], [20., 0., 0.]];
        let mut m = vec![
            Metric::isotropic(1.).unwrap(),
            Metric::isotropic(100.).unwrap(),
            Metric::isotropic(100.).unwrap(),
        ];
        let r = gradate(&v, &[[0, 1], [1, 2]], &mut m, 0.3, 50).unwrap();
        assert!(r.converged && r.updates > 0);
        let h: Vec<f64> = m.iter().map(|x| x.principal().0[0]).collect();
        assert!(close(h[0], 1., 1e-12), "the small end never grows");
        // Isotropic closed form: h_next <= h + beta * distance.
        assert!(close(h[1], 1. + 0.3 * 10., 1e-9), "{h:?}");
        assert!(close(h[2], h[1] + 0.3 * 10., 1e-9), "{h:?}");
        // Already-graded fields are untouched.
        let mut fine = vec![Metric::isotropic(1.).unwrap(); 3];
        let r = gradate(&v, &[[0, 1], [1, 2]], &mut fine, 0.3, 5).unwrap();
        assert_eq!(r.updates, 0);
        assert!(gradate(&v, &[[0, 5]], &mut fine, 0.3, 5).is_err());
        assert!(gradate(&v, &[[0, 1]], &mut fine, 0., 5).is_err());
    }
    #[test]
    fn curvature_metric_follows_the_chord_error_of_a_sphere() {
        let b = MetricBounds::new(1e-3, 10., 1e3).unwrap();
        // Sphere of radius 5, deviation 0.01: h = sqrt(8 * 0.01 * 5) = 0.632...
        let m = curvature_metric([0.2, 0.2], [AX[0], AX[1]], AX[2], 0.01, &b).unwrap();
        let (sizes, _) = m.principal();
        assert!(close(sizes[0], (8. * 0.01 * 5f64).sqrt(), 1e-12));
        assert!(
            close(sizes[1], sizes[0], 1e-12) && close(sizes[2], 10f64.min(sizes[0] * 1e3), 1e-12)
        );
        // A cylinder: size along the axis stays at the maximum, across it follows the radius.
        let c = curvature_metric([0.2, 0.], [AX[0], AX[1]], AX[2], 0.01, &b).unwrap();
        let (cs, dirs) = c.principal();
        assert!(close(cs[0], (8. * 0.01 * 5f64).sqrt(), 1e-12));
        assert!(close(dirs[0][0].abs(), 1., 1e-12));
        // Nearly flat surfaces reach the maximum size, very curved ones the minimum.
        let flat = curvature_metric([1e-9, 1e-9], [AX[0], AX[1]], AX[2], 0.01, &b).unwrap();
        assert!(close(flat.principal().0[0], 10., 1e-12));
        let sharp = curvature_metric([1e9, 1e9], [AX[0], AX[1]], AX[2], 0.01, &b).unwrap();
        assert!(close(sharp.principal().0[0], 1e-3, 1e-12));
        assert!(curvature_metric([0.2, 0.2], [AX[0], AX[1]], AX[2], 0., &b).is_err());
        assert!(curvature_metric([f64::NAN, 0.2], [AX[0], AX[1]], AX[2], 0.01, &b).is_err());
    }
    #[test]
    fn error_fields_drive_sizes_only_when_usable_and_non_negative() {
        let b = MetricBounds::new(0.01, 5., 10.).unwrap();
        let field = ErrorField::new(
            "zz",
            Indicator::Marking,
            vec![4., 1., 0.25, 0.],
            5.,
            Reliability::Reliable,
            0.,
        );
        let h = sizes_from_error(&field, &[1., 1., 1., 1.], 1., 2., &b).unwrap();
        // order 2: h_new = h * sqrt(target / e)
        assert!(close(h[0], 0.5, 1e-12) && close(h[1], 1., 1e-12) && close(h[2], 2., 1e-12));
        assert!(
            close(h[3], 5., 1e-12),
            "zero error asks for the largest size"
        );
        let degraded = ErrorField::new(
            "zz",
            Indicator::Marking,
            vec![1.],
            1.,
            Reliability::Degraded("thin".into()),
            0.,
        );
        assert!(sizes_from_error(&degraded, &[1.], 1., 1., &b).is_ok());
        let failed = ErrorField::new(
            "zz",
            Indicator::Marking,
            vec![1.],
            1.,
            Reliability::Failed("x".into()),
            0.,
        );
        assert!(matches!(
            sizes_from_error(&failed, &[1.], 1., 1., &b),
            Err(MetricError::UnusableErrorField(_))
        ));
        let signed = ErrorField::new(
            "goal",
            Indicator::Signed,
            vec![1.],
            1.,
            Reliability::Reliable,
            0.,
        );
        assert!(matches!(
            sizes_from_error(&signed, &[1.], 1., 1., &b),
            Err(MetricError::UnusableErrorField(_))
        ));
        let nan = ErrorField::new(
            "zz",
            Indicator::Marking,
            vec![f64::NAN],
            1.,
            Reliability::Reliable,
            0.,
        );
        assert!(
            sizes_from_error(&nan, &[1.], 1., 1., &b).is_err(),
            "NaN downgrades the field"
        );
        let neg = ErrorField::new(
            "zz",
            Indicator::Marking,
            vec![-1.],
            1.,
            Reliability::Reliable,
            0.,
        );
        assert!(matches!(
            sizes_from_error(&neg, &[1.], 1., 1., &b),
            Err(MetricError::Entry { index: 0, .. })
        ));
        assert!(sizes_from_error(&field, &[1.], 1., 1., &b).is_err());
    }
    /// The unit cube as five tetrahedra.
    fn cube() -> (Vec<V3>, Vec<[usize; 4]>) {
        let v: Vec<V3> = (0..8)
            .map(|i| [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64])
            .collect();
        (
            v,
            vec![
                [0, 1, 2, 4],
                [1, 2, 3, 7],
                [1, 4, 5, 7],
                [2, 4, 6, 7],
                [1, 2, 4, 7],
            ],
        )
    }
    #[test]
    fn complexity_matches_the_closed_form_for_uniform_metrics() {
        let (v, e) = cube();
        for h in [1., 0.5, 0.25] {
            let m = vec![Metric::isotropic(h).unwrap(); 8];
            let n = tet_complexity(&v, &e, &m).unwrap();
            // N = V / (h^3 * V_unit)
            assert!(
                close(n, 1. / (h * h * h * UNIT_TET_VOLUME), 1e-12),
                "h={h}: {n}"
            );
        }
        // Anisotropic: density multiplies the three sizes.
        let m = vec![Metric::from_sizes(AX, [0.5, 1., 2.]).unwrap(); 8];
        assert!(close(
            tet_complexity(&v, &e, &m).unwrap(),
            1. / UNIT_TET_VOLUME,
            1e-12
        ));
        assert!(tet_complexity(&v, &e, &m[..3]).is_err());
        assert!(tet_complexity(&v, &[[0, 1, 2, 99]], &vec![m[0]; 8]).is_err());
    }
    #[test]
    fn budget_rescaling_hits_the_target_and_reports_binding_bounds() {
        let (v, e) = cube();
        let b = MetricBounds::new(0.01, 100., 100.).unwrap();
        let mut m = vec![Metric::isotropic(1.).unwrap(); 8];
        let target = 5000.;
        let r = rescale_to_budget(&mut m, &b, target, 1e-6, |f| tet_complexity(&v, &e, f)).unwrap();
        assert!(r.met, "{r:?}");
        assert!(close(r.complexity, target, 1e-6));
        assert!(m.iter().all(|x| x.validate(&b).is_ok()));
        // An impossible budget stops at the size bound and says so.
        let tight = MetricBounds::new(0.5, 100., 100.).unwrap();
        let mut m = vec![Metric::isotropic(1.).unwrap(); 8];
        let r =
            rescale_to_budget(&mut m, &tight, 1e9, 1e-3, |f| tet_complexity(&v, &e, f)).unwrap();
        assert!(!r.met);
        assert!(close(m[0].principal().0[0], 0.5, 1e-12));
        assert!(rescale_to_budget(&mut m, &b, 0., 1e-3, |f| tet_complexity(&v, &e, f)).is_err());
    }
    #[test]
    fn element_sizes_become_vertex_metrics_by_geometric_mean() {
        let (_, e) = cube();
        let sizes = [1., 1., 1., 1., 4.];
        let m = vertex_metrics_from_element_sizes(&e, 8, &sizes).unwrap();
        // Vertex 0 belongs only to element 0.
        assert!(close(m[0].principal().0[0], 1., 1e-12));
        // Vertex 3 belongs only to element 1.
        assert!(close(m[3].principal().0[0], 1., 1e-12));
        // Vertex 1 is in elements 0, 1, 2 and 4: geometric mean of (1, 1, 1, 4).
        assert!(close(m[1].principal().0[0], 4f64.powf(0.25), 1e-12));
        assert!(vertex_metrics_from_element_sizes(&e, 8, &sizes[..2]).is_err());
        assert!(
            vertex_metrics_from_element_sizes(&e, 9, &sizes).is_err(),
            "vertex 8 has no element"
        );
    }
}
