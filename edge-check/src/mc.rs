//! Monte-Carlo sampling: fixed-seed (reproducible) Latin-hypercube samples
//! of independent uniform variables, plus the summary statistics the report
//! needs. No external RNG: SplitMix64 is a few lines and statistically
//! ample for stratified sampling of two variables.

pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// `n` Latin-hypercube samples in `dims` dimensions: every column is a
/// random permutation of the `n` equal strata, jittered within its stratum,
/// so each marginal is covered uniformly even for small `n`.
pub fn latin_hypercube(n: usize, dims: usize, seed: u64) -> Vec<Vec<f64>> {
    let mut rng = SplitMix64::new(seed);
    let mut cols: Vec<Vec<f64>> = Vec::with_capacity(dims);
    for _ in 0..dims {
        let mut perm: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = (rng.next_u64() % (i as u64 + 1)) as usize;
            perm.swap(i, j);
        }
        cols.push(perm.into_iter().map(|k| (k as f64 + rng.next_f64()) / n as f64).collect());
    }
    (0..n).map(|i| (0..dims).map(|d| cols[d][i]).collect()).collect()
}

#[allow(clippy::excessive_precision)] // published coefficients, kept verbatim
/// Inverse standard-normal CDF (Acklam's rational approximation, relative
/// error < 1.2e-9 over the open unit interval).
pub fn inverse_normal_cdf(p: f64) -> f64 {
    const A: [f64; 6] = [-3.969683028665376e+01, 2.209460984245205e+02, -2.759285104469687e+02, 1.383577518672690e+02, -3.066479806614716e+01, 2.506628277459239e+00];
    const B: [f64; 5] = [-5.447609879822406e+01, 1.615858368580409e+02, -1.556989798598866e+02, 6.680131188771972e+01, -1.328068155288572e+01];
    const C: [f64; 6] = [-7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e+00, -2.549732539343734e+00, 4.374664141464968e+00, 2.938163982698783e+00];
    const D: [f64; 4] = [7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e+00, 3.754408661907416e+00];
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    let plow = 0.02425;
    if p < plow {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - plow {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        -inverse_normal_cdf(1.0 - p)
    }
}

/// Distribution summary of a margin sample (non-finite margins - "nothing
/// applied" - are treated as passing and excluded from the mean/percentile).
#[derive(Debug, Clone, PartialEq)]
pub struct MarginStats {
    pub samples: usize,
    /// Fraction of samples with margin < 0.
    pub p_fail: f64,
    pub mean: f64,
    pub p05: f64,
    pub min: f64,
}

pub fn summarize(margins: &[f64]) -> MarginStats {
    let n = margins.len();
    let fails = margins.iter().filter(|m| **m < 0.0).count();
    let mut finite: Vec<f64> = margins.iter().copied().filter(|m| m.is_finite()).collect();
    finite.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (mean, p05, min) = if finite.is_empty() {
        (f64::INFINITY, f64::INFINITY, f64::INFINITY)
    } else {
        let mean = finite.iter().sum::<f64>() / finite.len() as f64;
        let idx = ((finite.len() as f64 * 0.05).floor() as usize).min(finite.len() - 1);
        (mean, finite[idx], finite[0])
    };
    MarginStats { samples: n, p_fail: if n > 0 { fails as f64 / n as f64 } else { 0.0 }, mean, p05, min }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_hypercube_covers_every_stratum_exactly_once_per_dimension() {
        let n = 64;
        let s = latin_hypercube(n, 3, 7);
        for d in 0..3 {
            let mut hit = vec![false; n];
            for row in &s {
                let k = (row[d] * n as f64).floor() as usize;
                assert!(!hit[k], "stratum {k} hit twice");
                hit[k] = true;
            }
        }
    }

    #[test]
    fn sampling_is_reproducible_for_a_fixed_seed() {
        assert_eq!(latin_hypercube(16, 2, 42), latin_hypercube(16, 2, 42));
        assert_ne!(latin_hypercube(16, 2, 42), latin_hypercube(16, 2, 43));
    }

    #[test]
    fn inverse_normal_matches_known_quantiles() {
        assert!(inverse_normal_cdf(0.5).abs() < 1e-9);
        assert!((inverse_normal_cdf(0.975) - 1.959964).abs() < 1e-5);
        assert!((inverse_normal_cdf(0.001) + 3.090232).abs() < 1e-5);
    }

    #[test]
    fn summarize_counts_failures_and_percentiles() {
        let m: Vec<f64> = (0..100).map(|i| (i as f64 - 9.5) / 10.0).collect(); // 10 negatives
        let s = summarize(&m);
        assert!((s.p_fail - 0.10).abs() < 1e-12);
        assert!(s.p05 < 0.0 && s.min < s.p05);
    }
}
