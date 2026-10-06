//! Smoothed-aggregation algebraic multigrid for elasticity, and preconditioned conjugate gradients.
//!
//! The coarse spaces reproduce a user-supplied near-null space (the rigid-body modes of the
//! mechanical problem): aggregates of nodes, a per-aggregate QR of the near-null-space rows gives
//! the tentative prolongator and the next level's near-null space, and one damped-Jacobi smoothing
//! of the prolongator gives the smoothed-aggregation interpolation. Levels use symmetric
//! Gauss-Seidel (forward before, backward after the coarse correction, so the V-cycle is a
//! symmetric positive definite preconditioner) and a direct sparse Cholesky on the coarsest level.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::linear::{CscCholesky, SolveError};
use crate::sparse::Csr;

const NONE: u32 = u32::MAX;

struct Level {
    a: Csr,
    /// Prolongator to the next (coarser) level and its transpose.
    p: Csr,
    r: Csr,
    diag: Vec<f64>,
}

pub struct Amg {
    sweeps: usize,
    levels: Vec<Level>,
    coarse_a: Csr,
    coarse: CscCholesky,
}

#[derive(Debug, Clone, Copy)]
pub struct AmgOptions {
    /// Stop coarsening at this many unknowns and factor directly.
    pub coarse_size: usize,
    pub max_levels: usize,
    /// Node connections with `|A_ij|_F / sqrt(|A_ii|_F |A_jj|_F)` below this are ignored when
    /// aggregating.
    pub strength: f64,
    /// Gauss-Seidel sweeps before and after each coarse correction (two sweeps roughly halve the
    /// iteration count of quadratic elements for a small per-iteration cost).
    pub sweeps: usize,
}

impl Default for AmgOptions {
    fn default() -> Self {
        Self { coarse_size: 1500, max_levels: 12, strength: 0.0, sweeps: 2 }
    }
}

impl Amg {
    /// Build the hierarchy for SPD `a` whose unknowns are grouped into nodes (`node_ptr`, length
    /// `n_nodes + 1`) with near-null space `b` (`n x nb`, row major).
    pub fn new(a: Csr, node_ptr: Vec<usize>, b: Vec<f64>, nb: usize, opt: AmgOptions) -> Result<Self, SolveError> {
        let (mut a, mut node_ptr, mut b) = (a, node_ptr, b);
        let mut levels = Vec::new();
        loop {
            let n = a.n_rows;
            if n <= opt.coarse_size || levels.len() + 1 >= opt.max_levels {
                break;
            }
            let (p, node_ptr_c, b_c) = tentative_and_smooth(&a, &node_ptr, &b, nb, opt.strength);
            let nc = p.n_cols;
            if nc == 0 || nc as f64 > 0.75 * n as f64 {
                break; // stalled coarsening: finish with a direct solve here
            }
            let r = p.transpose();
            let ac = r.matmul(&a.matmul(&p));
            let diag = a.diagonal();
            levels.push(Level { a, p, r, diag });
            a = ac;
            node_ptr = node_ptr_c;
            b = b_c;
        }
        let (cp, ci, cv) = a.to_lower_csc();
        let coarse = CscCholesky::new(a.n_rows, &cp, &ci, &cv)?;
        Ok(Self { sweeps: opt.sweeps.max(1), levels, coarse_a: a, coarse })
    }

    pub fn n_levels(&self) -> usize {
        self.levels.len() + 1
    }

    /// Unknowns per level, finest first.
    pub fn sizes(&self) -> Vec<usize> {
        self.levels.iter().map(|l| l.a.n_rows).chain(std::iter::once(self.coarse_a.n_rows)).collect()
    }

    /// Operator complexity: total nonzeros over the fine-level nonzeros.
    pub fn complexity(&self) -> f64 {
        let fine = self.levels.first().map_or(self.coarse_a.nnz(), |l| l.a.nnz()) as f64;
        (self.levels.iter().map(|l| l.a.nnz()).sum::<usize>() + self.coarse_a.nnz()) as f64 / fine
    }

    /// Apply one V-cycle to `rhs`: `x = M^-1 rhs` (a symmetric positive definite operator).
    pub fn apply(&self, rhs: &[f64], x: &mut [f64]) {
        self.cycle(0, rhs, x);
    }

    fn cycle(&self, l: usize, b: &[f64], x: &mut [f64]) {
        if l == self.levels.len() {
            x.copy_from_slice(b);
            self.coarse.solve_in_place(x);
            return;
        }
        let lv = &self.levels[l];
        let n = lv.a.n_rows;
        x.iter_mut().for_each(|v| *v = 0.0);
        for _ in 0..self.sweeps {
            gauss_seidel(&lv.a, &lv.diag, b, x, true);
        }
        let mut res = vec![0.0; n];
        lv.a.spmv(x, &mut res);
        for i in 0..n {
            res[i] = b[i] - res[i];
        }
        let nc = lv.p.n_cols;
        let mut rc = vec![0.0; nc];
        lv.r.spmv(&res, &mut rc);
        let mut xc = vec![0.0; nc];
        self.cycle(l + 1, &rc, &mut xc);
        let mut corr = vec![0.0; n];
        lv.p.spmv(&xc, &mut corr);
        for i in 0..n {
            x[i] += corr[i];
        }
        for _ in 0..self.sweeps {
            gauss_seidel(&lv.a, &lv.diag, b, x, false);
        }
    }
}

fn gauss_seidel(a: &Csr, diag: &[f64], b: &[f64], x: &mut [f64], forward: bool) {
    let row = |i: usize, x: &mut [f64]| {
        let mut s = b[i];
        for k in a.ptr[i]..a.ptr[i + 1] {
            let j = a.idx[k] as usize;
            if j != i {
                s -= a.val[k] * x[j];
            }
        }
        x[i] = s / diag[i];
    };
    if forward {
        for i in 0..a.n_rows {
            row(i, x);
        }
    } else {
        for i in (0..a.n_rows).rev() {
            row(i, x);
        }
    }
}

/// Greedy aggregation of nodes on the node graph of `a`.
fn aggregate(nodes: usize, nbrs: &[Vec<u32>]) -> (Vec<u32>, usize) {
    let mut agg = vec![NONE; nodes];
    let mut count = 0u32;
    for i in 0..nodes {
        if agg[i] == NONE && nbrs[i].iter().all(|&j| agg[j as usize] == NONE) {
            agg[i] = count;
            for &j in &nbrs[i] {
                agg[j as usize] = count;
            }
            count += 1;
        }
    }
    // Attach leftovers to a neighbouring aggregate (decided from the pass-1 state).
    let snapshot = agg.clone();
    for i in 0..nodes {
        if agg[i] == NONE {
            if let Some(&j) = nbrs[i].iter().find(|&&j| snapshot[j as usize] != NONE) {
                agg[i] = snapshot[j as usize];
            }
        }
    }
    for i in 0..nodes {
        if agg[i] == NONE {
            agg[i] = count;
            for &j in &nbrs[i] {
                if agg[j as usize] == NONE {
                    agg[j as usize] = count;
                }
            }
            count += 1;
        }
    }
    (agg, count as usize)
}

/// Spectral radius estimate of `D^-1 A` by power iteration.
fn spectral_radius(a: &Csr, diag: &[f64]) -> f64 {
    let n = a.n_rows;
    let mut v: Vec<f64> = (0..n).map(|i| 1.0 + ((i * 2654435761) % 1000) as f64 * 1e-3).collect();
    let mut w = vec![0.0; n];
    let mut rho = 1.0;
    for _ in 0..20 {
        let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        v.iter_mut().for_each(|x| *x /= norm);
        a.spmv(&v, &mut w);
        for i in 0..n {
            w[i] /= diag[i];
        }
        rho = w.iter().zip(&v).map(|(a, b)| a * b).sum::<f64>();
        std::mem::swap(&mut v, &mut w);
    }
    rho.abs() * 1.05
}

/// Tentative prolongator from per-aggregate QR of the near-null space, smoothed by one damped
/// Jacobi step. Returns `(P, coarse node_ptr, coarse near-null space)`.
fn tentative_and_smooth(a: &Csr, node_ptr: &[usize], b: &[f64], nb: usize, strength: f64) -> (Csr, Vec<usize>, Vec<f64>) {
    let n = a.n_rows;
    let nodes = node_ptr.len() - 1;
    let mut node_of = vec![0u32; n];
    for i in 0..nodes {
        for d in node_ptr[i]..node_ptr[i + 1] {
            node_of[d] = i as u32;
        }
    }
    // Node graph: connections whose block norm is at least `strength` of the geometric mean of the
    // two diagonal block norms.
    let mut nbrs: Vec<Vec<u32>> = vec![Vec::new(); nodes];
    let mut block_norm: Vec<std::collections::HashMap<u32, f64>> = vec![std::collections::HashMap::new(); nodes];
    for i in 0..n {
        let ni = node_of[i] as usize;
        for k in a.ptr[i]..a.ptr[i + 1] {
            *block_norm[ni].entry(node_of[a.idx[k] as usize]).or_insert(0.0) += a.val[k] * a.val[k];
        }
    }
    let diag_norm: Vec<f64> = (0..nodes).map(|i| block_norm[i].get(&(i as u32)).copied().unwrap_or(0.0).sqrt()).collect();
    for i in 0..nodes {
        for (&j, &sq) in &block_norm[i] {
            if j as usize != i && sq.sqrt() >= strength * (diag_norm[i] * diag_norm[j as usize]).sqrt() {
                nbrs[i].push(j);
            }
        }
        nbrs[i].sort_unstable();
    }
    // Keep the graph symmetric (a connection is strong if it is strong from either side).
    let strong = nbrs.clone();
    for (i, list) in strong.iter().enumerate() {
        for &j in list {
            if !nbrs[j as usize].contains(&(i as u32)) {
                nbrs[j as usize].push(i as u32);
            }
        }
    }
    for v in nbrs.iter_mut() {
        v.sort_unstable();
        v.dedup();
    }
    let (agg, n_agg) = aggregate(nodes, &nbrs);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); n_agg];
    for i in 0..nodes {
        members[agg[i] as usize].push(i);
    }
    // Per-aggregate orthonormalisation of the near-null-space rows (Gram-Schmidt, twice).
    let mut p_ptr = vec![0usize];
    let (mut p_idx, mut p_val) = (Vec::<u32>::new(), Vec::<f64>::new());
    // Rows of P are fine dofs; build per-row entry lists then flatten in row order.
    let mut rows: Vec<Vec<(u32, f64)>> = vec![Vec::new(); n];
    let mut coarse_node_ptr = vec![0usize];
    let mut b_c: Vec<f64> = Vec::new();
    let mut n_coarse = 0usize;
    for mem in &members {
        let dofs: Vec<usize> = mem.iter().flat_map(|&nd| node_ptr[nd]..node_ptr[nd + 1]).collect();
        let m = dofs.len();
        let col_norm: Vec<f64> = (0..nb).map(|c| dofs.iter().map(|&d| b[d * nb + c].powi(2)).sum::<f64>().sqrt()).collect();
        // Orthonormal basis of the column space (dependent columns, e.g. a rotation about the axis of
        // collinear nodes, are dropped), then the coarse near-null space is R = Q^T B.
        let mut q: Vec<Vec<f64>> = Vec::new();
        for c in 0..nb {
            let mut v: Vec<f64> = dofs.iter().map(|&d| b[d * nb + c]).collect();
            for _pass in 0..2 {
                for qk in &q {
                    let dot: f64 = qk.iter().zip(&v).map(|(a, b)| a * b).sum();
                    for i in 0..m {
                        v[i] -= dot * qk[i];
                    }
                }
            }
            let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            if col_norm[c] > 0.0 && norm > 1e-8 * col_norm[c] {
                v.iter_mut().for_each(|x| *x /= norm);
                q.push(v);
            }
        }
        let rank = q.len();
        let r_rows: Vec<Vec<f64>> = q.iter().map(|qk| (0..nb).map(|c| dofs.iter().zip(qk).map(|(&d, qv)| qv * b[d * nb + c]).sum()).collect()).collect();
        for k in 0..rank {
            for (i, &d) in dofs.iter().enumerate() {
                rows[d].push(((n_coarse + k) as u32, q[k][i]));
            }
            b_c.extend_from_slice(&r_rows[k]);
        }
        n_coarse += rank;
        coarse_node_ptr.push(n_coarse);
    }
    for r in &rows {
        p_idx.extend(r.iter().map(|e| e.0));
        p_val.extend(r.iter().map(|e| e.1));
        p_ptr.push(p_idx.len());
    }
    let p_tent = Csr { n_rows: n, n_cols: n_coarse, ptr: p_ptr, idx: p_idx, val: p_val };
    // P = (I - omega D^-1 A) P_tent, built as S * P_tent with S = I - omega D^-1 A.
    let diag = a.diagonal();
    let omega = 4.0 / (3.0 * spectral_radius(a, &diag));
    let mut s = a.clone();
    for i in 0..n {
        for k in s.ptr[i]..s.ptr[i + 1] {
            let j = s.idx[k] as usize;
            s.val[k] = -omega * a.val[k] / diag[i] + if i == j { 1.0 } else { 0.0 };
        }
    }
    (s.matmul(&p_tent), coarse_node_ptr, b_c)
}

#[derive(Debug, Clone, Copy)]
pub struct PcgReport {
    pub iterations: usize,
    pub rel_residual: f64,
    pub converged: bool,
}

/// Preconditioned conjugate gradients for SPD `a`, zero initial guess.
pub fn pcg(a: &Csr, b: &[f64], x: &mut [f64], precond: &dyn Fn(&[f64], &mut [f64]), tol: f64, max_iter: usize) -> PcgReport {
    let n = a.n_rows;
    x.iter_mut().for_each(|v| *v = 0.0);
    let bnorm = b.iter().map(|v| v * v).sum::<f64>().sqrt();
    if bnorm == 0.0 {
        return PcgReport { iterations: 0, rel_residual: 0.0, converged: true };
    }
    let mut r = b.to_vec();
    let mut z = vec![0.0; n];
    precond(&r, &mut z);
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let mut ap = vec![0.0; n];
    let mut rel = 1.0;
    for it in 1..=max_iter {
        a.spmv(&p, &mut ap);
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap <= 0.0 || !pap.is_finite() {
            return PcgReport { iterations: it, rel_residual: rel, converged: false };
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        rel = r.iter().map(|v| v * v).sum::<f64>().sqrt() / bnorm;
        if rel < tol {
            return PcgReport { iterations: it, rel_residual: rel, converged: true };
        }
        precond(&r, &mut z);
        let rz_new: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let beta = rz_new / rz;
        rz = rz_new;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    PcgReport { iterations: max_iter, rel_residual: rel, converged: false }
}
