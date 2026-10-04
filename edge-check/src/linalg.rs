//! Small dense/banded linear-algebra kernels. Local on purpose: the crate
//! has no external dependencies, and the two problems here (a banded SPD
//! stiffness system, a small dense least-squares system) are tiny.

/// Symmetric positive-definite banded matrix, lower triangle stored by
/// rows: entry `(i, j)` with `i - bw <= j <= i` lives at
/// `data[i * (bw + 1) + (j + bw - i)]`.
pub struct BandedSpd {
    pub n: usize,
    pub bw: usize,
    data: Vec<f64>,
}

impl BandedSpd {
    pub fn zeros(n: usize, bw: usize) -> Self {
        Self { n, bw, data: vec![0.0; n * (bw + 1)] }
    }

    #[inline]
    fn idx(&self, i: usize, j: usize) -> usize {
        i * (self.bw + 1) + (j + self.bw - i)
    }

    /// Adds `v` to entry `(i, j)`; only the lower triangle (`j <= i`) is
    /// stored, so an upper-triangle request is mirrored.
    #[inline]
    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        let (i, j) = if j > i { (j, i) } else { (i, j) };
        debug_assert!(i - j <= self.bw, "entry outside the band");
        let k = self.idx(i, j);
        self.data[k] += v;
    }

    pub fn diag(&self, i: usize) -> f64 {
        self.data[self.idx(i, i)]
    }

    /// In-place banded Cholesky `A = L Lᵀ`. Returns `false` if the matrix
    /// is not positive definite (a non-positive pivot).
    pub fn factor(&mut self) -> bool {
        let (n, bw) = (self.n, self.bw);
        for i in 0..n {
            let j0 = i.saturating_sub(bw);
            for j in j0..=i {
                let k0 = i.saturating_sub(bw).max(j.saturating_sub(bw));
                let mut s = self.data[self.idx(i, j)];
                for k in k0..j {
                    s -= self.data[self.idx(i, k)] * self.data[self.idx(j, k)];
                }
                if i == j {
                    if s <= 0.0 || !s.is_finite() {
                        return false;
                    }
                    let k = self.idx(i, i);
                    self.data[k] = s.sqrt();
                } else {
                    let d = self.data[self.idx(j, j)];
                    let k = self.idx(i, j);
                    self.data[k] = s / d;
                }
            }
        }
        true
    }

    /// Solves `L Lᵀ x = b` in place (call after [`factor`](Self::factor)).
    pub fn solve_in_place(&self, b: &mut [f64]) {
        let (n, bw) = (self.n, self.bw);
        for i in 0..n {
            let j0 = i.saturating_sub(bw);
            let mut s = b[i];
            for (j, bj) in b.iter().enumerate().take(i).skip(j0) {
                s -= self.data[self.idx(i, j)] * bj;
            }
            b[i] = s / self.data[self.idx(i, i)];
        }
        for i in (0..n).rev() {
            let mut s = b[i];
            let jmax = (i + bw).min(n - 1);
            for (j, bj) in b.iter().enumerate().take(jmax + 1).skip(i + 1) {
                s -= self.data[self.idx(j, i)] * bj;
            }
            b[i] = s / self.data[self.idx(i, i)];
        }
    }
}

/// Least squares `min |A x - b|` for a dense row-major `m x n` matrix
/// (`m >= n`) and one or more right-hand sides (`rhs` is `n_rhs` vectors
/// of length `m`), by Householder QR with column scaling. Columns are
/// normalised to unit length first (the pole bases used by the analytic
/// model span many orders of magnitude), and any column whose reduced norm
/// collapses below `rank_tol` times the largest is dropped (coefficient 0)
/// instead of blowing the solution up. Returns one solution per RHS.
pub fn least_squares_multi(a: &[f64], m: usize, n: usize, rhs: &[Vec<f64>], rank_tol: f64) -> Vec<Vec<f64>> {
    assert_eq!(a.len(), m * n);
    let mut r = a.to_vec();
    let mut bs: Vec<Vec<f64>> = rhs.to_vec();
    for b in &bs {
        assert_eq!(b.len(), m);
    }
    let mut scale = vec![1.0; n];
    for j in 0..n {
        let norm = (0..m).map(|i| r[i * n + j] * r[i * n + j]).sum::<f64>().sqrt();
        if norm > 0.0 {
            scale[j] = norm;
            for i in 0..m {
                r[i * n + j] /= norm;
            }
        }
    }
    let mut keep = vec![true; n];
    let mut max_diag: f64 = 0.0;
    let steps = n.min(m);
    for k in 0..steps {
        let norm = (k..m).map(|i| r[i * n + k] * r[i * n + k]).sum::<f64>().sqrt();
        max_diag = max_diag.max(norm);
        if norm <= rank_tol * max_diag.max(f64::MIN_POSITIVE) {
            keep[k] = false;
            continue;
        }
        let alpha = if r[k * n + k] > 0.0 { -norm } else { norm };
        let mut v: Vec<f64> = (k..m).map(|i| r[i * n + k]).collect();
        v[0] -= alpha;
        let vnorm2: f64 = v.iter().map(|x| x * x).sum();
        if vnorm2 > 0.0 {
            for j in k..n {
                let dot: f64 = (k..m).map(|i| v[i - k] * r[i * n + j]).sum();
                let f = 2.0 * dot / vnorm2;
                for i in k..m {
                    r[i * n + j] -= f * v[i - k];
                }
            }
            for b in bs.iter_mut() {
                let dot: f64 = (k..m).map(|i| v[i - k] * b[i]).sum();
                let f = 2.0 * dot / vnorm2;
                for i in k..m {
                    b[i] -= f * v[i - k];
                }
            }
        }
    }
    bs.iter()
        .map(|b| {
            let mut x = vec![0.0; n];
            for k in (0..steps).rev() {
                if !keep[k] {
                    continue;
                }
                let mut s = b[k];
                for j in (k + 1)..n {
                    s -= r[k * n + j] * x[j];
                }
                x[k] = s / r[k * n + k];
            }
            for j in 0..n {
                x[j] /= scale[j];
            }
            x
        })
        .collect()
}

/// Single-RHS convenience wrapper over [`least_squares_multi`].
pub fn least_squares(a: &[f64], m: usize, n: usize, b: &[f64], rank_tol: f64) -> Vec<f64> {
    least_squares_multi(a, m, n, &[b.to_vec()], rank_tol).pop().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banded_cholesky_matches_a_known_tridiagonal_solution() {
        // 1D Poisson-like SPD tridiagonal: diag 2, off -1; A x = b with x = [1,2,3,4].
        let n = 4;
        let mut a = BandedSpd::zeros(n, 1);
        for i in 0..n {
            a.add(i, i, 2.0);
            if i > 0 {
                a.add(i, i - 1, -1.0);
            }
        }
        let x = [1.0, 2.0, 3.0, 4.0];
        let mut b = vec![0.0; n];
        for i in 0..n {
            b[i] = 2.0 * x[i] - if i > 0 { x[i - 1] } else { 0.0 } - if i + 1 < n { x[i + 1] } else { 0.0 };
        }
        assert!(a.factor());
        a.solve_in_place(&mut b);
        for i in 0..n {
            assert!((b[i] - x[i]).abs() < 1e-12, "{b:?}");
        }
    }

    #[test]
    fn banded_cholesky_rejects_an_indefinite_matrix() {
        let mut a = BandedSpd::zeros(2, 1);
        a.add(0, 0, 1.0);
        a.add(1, 1, 1.0);
        a.add(1, 0, 2.0);
        assert!(!a.factor());
    }

    #[test]
    fn least_squares_recovers_an_exact_overdetermined_fit() {
        // y = 2 + 3 t sampled at 5 points.
        let ts = [0.0, 1.0, 2.0, 3.0, 4.0];
        let mut a = Vec::new();
        let mut b = Vec::new();
        for t in ts {
            a.extend_from_slice(&[1.0, t]);
            b.push(2.0 + 3.0 * t);
        }
        let x = least_squares(&a, 5, 2, &b, 1e-12);
        assert!((x[0] - 2.0).abs() < 1e-12 && (x[1] - 3.0).abs() < 1e-12, "{x:?}");
    }

    #[test]
    fn least_squares_drops_a_dependent_column_instead_of_exploding() {
        // Second column duplicates the first.
        let a = [1.0, 1.0, 2.0, 2.0, 3.0, 3.0];
        let b = [1.0, 2.0, 3.0];
        let x = least_squares(&a, 3, 2, &b, 1e-10);
        let fit: Vec<f64> = (0..3).map(|i| a[i * 2] * x[0] + a[i * 2 + 1] * x[1]).collect();
        for i in 0..3 {
            assert!((fit[i] - b[i]).abs() < 1e-10, "{fit:?}");
        }
        assert!(x.iter().all(|v| v.abs() < 10.0));
    }
}
