//! Sparse symmetric positive-definite Cholesky for the finite-strain solver.
//!
//! The banded Cholesky of `edge-check` costs `n * bw^2`, which is 150 ms for the full lug (the
//! ring-shaped mesh has a wide band). A mesh is a planar graph, so a fill-reducing ordering makes
//! the factor far cheaper: a geometric nested-dissection ordering (recursive coordinate
//! bisection, separators last) plus an up-looking left-to-right Cholesky over the elimination
//! tree (the algorithm of Davis, "Direct Methods for Sparse Linear Systems", `cs_chol`).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use std::collections::HashSet;

const LEAF: usize = 8;

/// Geometric nested dissection of a node graph. `adj[i]` lists the neighbours of node `i`; returns
/// the elimination order (`order[k]` = node eliminated `k`-th).
pub fn nested_dissection(coords: &[[f64; 2]], adj: &[Vec<usize>]) -> Vec<usize> {
    let mut order = Vec::with_capacity(coords.len());
    let all: Vec<usize> = (0..coords.len()).collect();
    dissect(coords, adj, all, &mut order);
    order
}

fn dissect(coords: &[[f64; 2]], adj: &[Vec<usize>], mut nodes: Vec<usize>, order: &mut Vec<usize>) {
    if nodes.len() <= LEAF {
        order.extend(nodes);
        return;
    }
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for &n in &nodes {
        for c in 0..2 {
            lo[c] = lo[c].min(coords[n][c]);
            hi[c] = hi[c].max(coords[n][c]);
        }
    }
    let axis = if hi[0] - lo[0] >= hi[1] - lo[1] { 0 } else { 1 };
    nodes.sort_by(|&a, &b| coords[a][axis].total_cmp(&coords[b][axis]).then(a.cmp(&b)));
    let half = nodes.len() / 2;
    let (a, b) = nodes.split_at(half);
    let in_a: HashSet<usize> = a.iter().copied().collect();
    // Separator: the thinner of "nodes of B touching A" and "nodes of A touching B" (so no A-B
    // edge remains once it is removed).
    let in_b: HashSet<usize> = b.iter().copied().collect();
    let sep_b: Vec<usize> = b.iter().copied().filter(|&n| adj[n].iter().any(|m| in_a.contains(m))).collect();
    let sep_a: Vec<usize> = a.iter().copied().filter(|&n| adj[n].iter().any(|m| in_b.contains(m))).collect();
    let (left, right, sep) = if sep_b.len() <= sep_a.len() {
        let s: HashSet<usize> = sep_b.iter().copied().collect();
        (a.to_vec(), b.iter().copied().filter(|n| !s.contains(n)).collect::<Vec<_>>(), sep_b)
    } else {
        let s: HashSet<usize> = sep_a.iter().copied().collect();
        (a.iter().copied().filter(|n| !s.contains(n)).collect::<Vec<_>>(), b.to_vec(), sep_a)
    };
    dissect(coords, adj, left, order);
    dissect(coords, adj, right, order);
    order.extend(sep);
}

/// The fixed sparsity pattern (upper triangle in CSC, permuted order), shared by every matrix
/// assembled on one mesh.
pub struct Pattern {
    pub n: usize,
    /// `perm[old] = new`.
    perm: Vec<usize>,
    colptr: Vec<usize>,
    rowidx: Vec<usize>,
}

/// Values of a symmetric matrix on a [`Pattern`].
pub struct SparseSym {
    pat: std::sync::Arc<Pattern>,
    pub vals: Vec<f64>,
}

impl Pattern {
    /// `pairs` lists every `(i, j)` position that can become non-zero (any order, duplicates fine);
    /// the diagonal is added. `order[k]` is the old index eliminated `k`-th.
    pub fn new(n: usize, order: &[usize], pairs: impl Iterator<Item = (usize, usize)>) -> Self {
        let mut perm = vec![0usize; n];
        for (k, &old) in order.iter().enumerate() {
            perm[old] = k;
        }
        let mut cols: Vec<Vec<usize>> = (0..n).map(|c| vec![c]).collect();
        for (i, j) in pairs {
            let (a, b) = (perm[i], perm[j]);
            let (r, c) = if a <= b { (a, b) } else { (b, a) };
            cols[c].push(r);
        }
        let mut colptr = vec![0usize; n + 1];
        let mut rowidx = Vec::new();
        for (c, col) in cols.iter_mut().enumerate() {
            col.sort_unstable();
            col.dedup();
            rowidx.extend_from_slice(col);
            colptr[c + 1] = rowidx.len();
        }
        Self { n, perm, colptr, rowidx }
    }

    pub fn nnz(&self) -> usize {
        self.rowidx.len()
    }
}

impl SparseSym {
    /// An all-zero matrix on `pat`.
    pub fn zeros(pat: &std::sync::Arc<Pattern>) -> Self {
        Self { pat: pat.clone(), vals: vec![0.0; pat.nnz()] }
    }

    /// Add `v` at old indices `(i, j)` (either triangle).
    #[inline]
    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        let pat = &*self.pat;
        let (a, b) = (pat.perm[i], pat.perm[j]);
        let (r, c) = if a <= b { (a, b) } else { (b, a) };
        let col = &pat.rowidx[pat.colptr[c]..pat.colptr[c + 1]];
        if let Ok(p) = col.binary_search(&r) {
            self.vals[pat.colptr[c] + p] += v;
        } else {
            debug_assert!(false, "entry outside the pattern");
        }
    }

    pub fn diag(&self, i: usize) -> f64 {
        let c = self.pat.perm[i];
        self.vals[self.pat.colptr[c + 1] - 1]
    }
}

/// Elimination tree, supernodes and the (symbolic) structure of the factor.
///
/// A supernode is a run of consecutive columns of `L` with the same row structure below the
/// diagonal block, so it is stored as one dense `rows x width` block and factored and updated with
/// contiguous dense kernels instead of one scattered column at a time (about three times the flop
/// rate of the column-by-column algorithm on these meshes).
pub struct Symbolic {
    n: usize,
    /// Supernode `s` has columns `sn_start[s] .. sn_start[s + 1]`.
    sn_start: Vec<usize>,
    /// Row structure of every supernode (sorted, the first `width` rows are its own columns).
    row_ptr: Vec<usize>,
    rows: Vec<usize>,
    /// Offset of the dense block of every supernode in the value array (`rows x width`, row-major).
    val_ptr: Vec<usize>,
    col2sn: Vec<usize>,
    /// Lower-triangle entries of `A` by column: `(row, index into the upper-CSC values)`.
    low_ptr: Vec<usize>,
    low_row: Vec<usize>,
    low_src: Vec<usize>,
    /// Index of every column's diagonal in the upper-CSC values.
    diag_src: Vec<usize>,
    flops: f64,
}

const NONE: usize = usize::MAX;
/// Supernodes are cut at this width so a dense diagonal block stays cache sized.
const MAX_WIDTH: usize = 96;

impl Symbolic {
    pub fn new(a: &Pattern) -> Self {
        let n = a.n;
        // Elimination tree.
        let mut parent = vec![NONE; n];
        let mut ancestor = vec![NONE; n];
        for k in 0..n {
            for &r in &a.rowidx[a.colptr[k]..a.colptr[k + 1]] {
                let mut i = r;
                while i != NONE && i < k {
                    let next = ancestor[i];
                    ancestor[i] = k;
                    if next == NONE {
                        parent[i] = k;
                    }
                    i = next;
                }
            }
        }
        // Column counts of L by walking the row patterns once.
        let mut counts = vec![1usize; n];
        let mut mark = vec![NONE; n];
        for k in 0..n {
            mark[k] = k;
            for &r in &a.rowidx[a.colptr[k]..a.colptr[k + 1]] {
                let mut i = r;
                if i >= k {
                    continue;
                }
                while mark[i] != k {
                    counts[i] += 1;
                    mark[i] = k;
                    i = parent[i];
                    if i == NONE {
                        break;
                    }
                }
            }
        }
        // Lower-triangle access to A (column j: the entries A(k, j), k > j) and the diagonal.
        let mut low_cnt = vec![0usize; n];
        let mut diag_src = vec![0usize; n];
        for k in 0..n {
            for p in a.colptr[k]..a.colptr[k + 1] {
                let r = a.rowidx[p];
                if r < k {
                    low_cnt[r] += 1;
                } else {
                    diag_src[k] = p;
                }
            }
        }
        let mut low_ptr = vec![0usize; n + 1];
        for j in 0..n {
            low_ptr[j + 1] = low_ptr[j] + low_cnt[j];
        }
        let mut fill = low_ptr[..n].to_vec();
        let mut low_row = vec![0usize; low_ptr[n]];
        let mut low_src = vec![0usize; low_ptr[n]];
        for k in 0..n {
            for p in a.colptr[k]..a.colptr[k + 1] {
                let r = a.rowidx[p];
                if r < k {
                    low_row[fill[r]] = k;
                    low_src[fill[r]] = p;
                    fill[r] += 1;
                }
            }
        }
        // Fundamental supernodes: column j joins j-1 when j is its parent and the structures nest.
        let mut sn_start = vec![0usize];
        for j in 1..n {
            let width = j - sn_start[sn_start.len() - 1];
            if !(parent[j - 1] == j && counts[j - 1] == counts[j] + 1 && width < MAX_WIDTH) {
                sn_start.push(j);
            }
        }
        sn_start.push(n);
        let ns = sn_start.len() - 1;
        let mut col2sn = vec![0usize; n];
        for s in 0..ns {
            for j in sn_start[s]..sn_start[s + 1] {
                col2sn[j] = s;
            }
        }
        // Row structure: own columns, the lower entries of A, and the children's structures.
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); ns];
        for s in 0..ns {
            let last = sn_start[s + 1] - 1;
            if parent[last] != NONE {
                children[col2sn[parent[last]]].push(s);
            }
        }
        let mut row_ptr = vec![0usize];
        let mut rows: Vec<usize> = Vec::new();
        let mut val_ptr = vec![0usize];
        let mut seen = vec![NONE; n];
        let mut flops = 0.0;
        for s in 0..ns {
            let (f, l) = (sn_start[s], sn_start[s + 1]);
            let begin = rows.len();
            for j in f..l {
                seen[j] = s;
                rows.push(j);
            }
            let mut below: Vec<usize> = Vec::new();
            for j in f..l {
                for p in low_ptr[j]..low_ptr[j + 1] {
                    let r = low_row[p];
                    if r >= l && seen[r] != s {
                        seen[r] = s;
                        below.push(r);
                    }
                }
            }
            for &c in &children[s] {
                for &r in &rows[row_ptr[c] + (sn_start[c + 1] - sn_start[c])..row_ptr[c + 1]] {
                    if r >= l && seen[r] != s {
                        seen[r] = s;
                        below.push(r);
                    }
                }
            }
            below.sort_unstable();
            rows.extend(below);
            row_ptr.push(rows.len());
            let (m, w) = (rows.len() - begin, l - f);
            val_ptr.push(val_ptr[s] + m * w);
            flops += (w * w * w) as f64 / 3.0 + (m - w) as f64 * (w * w) as f64 + ((m - w) * (m - w) * w) as f64;
        }
        Self { n, sn_start, row_ptr, rows, val_ptr, col2sn, low_ptr, low_row, low_src, diag_src, flops }
    }

    pub fn nnz(&self) -> usize {
        *self.val_ptr.last().unwrap()
    }

    /// Multiply-adds of one factorisation (dense supernodal blocks, including their explicit zeros).
    pub fn flops(&self) -> f64 {
        self.flops
    }

    /// Number of supernodes.
    pub fn supernodes(&self) -> usize {
        self.sn_start.len() - 1
    }
}

/// Numeric Cholesky factor in supernodal form.
pub struct Cholesky {
    sym: std::sync::Arc<Symbolic>,
    perm: Vec<usize>,
    vals: Vec<f64>,
}

impl Cholesky {
    /// Factor `m` (values filled in); `None` if it is not positive definite.
    pub fn factor(m: &SparseSym, shared: &std::sync::Arc<Symbolic>) -> Option<Self> {
        let sym = &**shared;
        let n = sym.n;
        let ns = sym.supernodes();
        let mut vals = vec![0.0; sym.nnz()];
        let mut relmap = vec![0usize; n];
        // Left-looking: descendants waiting to update each supernode, and where in their rows they are.
        let mut head = vec![NONE; ns];
        let mut next = vec![NONE; ns];
        let mut dptr = vec![0usize; ns];
        let mut update: Vec<f64> = Vec::new();
        for s in 0..ns {
            let (f, l) = (sym.sn_start[s], sym.sn_start[s + 1]);
            let w = l - f;
            let rows_s = &sym.rows[sym.row_ptr[s]..sym.row_ptr[s + 1]];
            let m_s = rows_s.len();
            for (i, &r) in rows_s.iter().enumerate() {
                relmap[r] = i;
            }
            let base = sym.val_ptr[s];
            // Assemble the entries of A.
            for j in f..l {
                vals[base + (j - f) * w + (j - f)] = m.vals[sym.diag_src[j]];
                for p in sym.low_ptr[j]..sym.low_ptr[j + 1] {
                    vals[base + relmap[sym.low_row[p]] * w + (j - f)] += m.vals[sym.low_src[p]];
                }
            }
            // Updates from the descendants whose next target is this supernode.
            let mut d = head[s];
            head[s] = NONE;
            while d != NONE {
                let nd = next[d];
                let (fd, ld) = (sym.sn_start[d], sym.sn_start[d + 1]);
                let wd = ld - fd;
                let rows_d = &sym.rows[sym.row_ptr[d]..sym.row_ptr[d + 1]];
                let md = rows_d.len();
                let p0 = dptr[d];
                // Rows of d that fall in the columns of s (nq of them) and all rows from p0 on.
                let nq = rows_d[p0..].iter().take_while(|&&r| r < l).count();
                let np = md - p0;
                update.clear();
                update.resize(nq * np, 0.0);
                let bd = sym.val_ptr[d];
                let dense = &vals[bd..bd + md * wd];
                // `update[q * np + pp]` = row p0+pp of d dotted with row p0+q of d (pp >= q): rows of
                // `update` are independent, so a large product is split across threads.
                let fill = |q0: usize, out: &mut [f64]| {
                    for (k, row) in out.chunks_mut(np).enumerate() {
                        let q = q0 + k;
                        let rq = &dense[(p0 + q) * wd..(p0 + q + 1) * wd];
                        for pp in q..np {
                            row[pp] = dot(&dense[(p0 + pp) * wd..(p0 + pp + 1) * wd], rq);
                        }
                    }
                };
                parallel_rows(&mut update, nq, np, np * nq * wd / 2, &fill);
                for q in 0..nq {
                    for pp in q..np {
                        vals[base + relmap[rows_d[p0 + pp]] * w + (rows_d[p0 + q] - f)] -= update[q * np + pp];
                    }
                }
                dptr[d] = p0 + nq;
                if dptr[d] < md {
                    let t = sym.col2sn[rows_d[dptr[d]]];
                    next[d] = head[t];
                    head[t] = d;
                }
                d = nd;
            }
            // Dense Cholesky of the diagonal block, then the rows below it.
            for i in 0..w {
                for j in 0..i {
                    let v = vals[base + i * w + j] - dot(&vals[base + i * w..base + i * w + j], &vals[base + j * w..base + j * w + j]);
                    vals[base + i * w + j] = v / vals[base + j * w + j];
                }
                let dd = vals[base + i * w + i] - dot(&vals[base + i * w..base + i * w + i], &vals[base + i * w..base + i * w + i]);
                if dd <= 0.0 || !dd.is_finite() {
                    return None;
                }
                vals[base + i * w + i] = dd.sqrt();
            }
            if m_s > w {
                let (diag, below) = vals[base..base + m_s * w].split_at_mut(w * w);
                let solve = |i0: usize, out: &mut [f64]| {
                    let _ = i0;
                    for row in out.chunks_mut(w) {
                        for j in 0..w {
                            let v = row[j] - dot(&row[..j], &diag[j * w..j * w + j]);
                            row[j] = v / diag[j * w + j];
                        }
                    }
                };
                parallel_rows(below, m_s - w, w, (m_s - w) * w * w / 2, &solve);
            }
            if m_s > w {
                let t = sym.col2sn[rows_s[w]];
                dptr[s] = w;
                next[s] = head[t];
                head[t] = s;
            }
        }
        Some(Self { sym: shared.clone(), perm: m.pat.perm.clone(), vals })
    }

    /// Solve `A x = b` in place (old indexing).
    pub fn solve_in_place(&self, b: &mut [f64]) {
        let sym = &*self.sym;
        let n = sym.n;
        let ns = sym.supernodes();
        let mut y = vec![0.0; n];
        for old in 0..n {
            y[self.perm[old]] = b[old];
        }
        // L z = y.
        for s in 0..ns {
            let (f, l) = (sym.sn_start[s], sym.sn_start[s + 1]);
            let w = l - f;
            let base = sym.val_ptr[s];
            let rows = &sym.rows[sym.row_ptr[s]..sym.row_ptr[s + 1]];
            for i in 0..w {
                let v = (y[f + i] - dot(&self.vals[base + i * w..base + i * w + i], &y[f..f + i])) / self.vals[base + i * w + i];
                y[f + i] = v;
            }
            for (i, &r) in rows.iter().enumerate().skip(w) {
                y[r] -= dot(&self.vals[base + i * w..base + (i + 1) * w], &y[f..l]);
            }
        }
        // L^T x = z.
        for s in (0..ns).rev() {
            let (f, l) = (sym.sn_start[s], sym.sn_start[s + 1]);
            let w = l - f;
            let base = sym.val_ptr[s];
            let rows = &sym.rows[sym.row_ptr[s]..sym.row_ptr[s + 1]];
            for j in (0..w).rev() {
                let mut acc = y[f + j];
                for i in j + 1..w {
                    acc -= self.vals[base + i * w + j] * y[f + i];
                }
                for (i, &r) in rows.iter().enumerate().skip(w) {
                    acc -= self.vals[base + i * w + j] * y[r];
                }
                y[f + j] = acc / self.vals[base + j * w + j];
            }
        }
        for old in 0..n {
            b[old] = y[self.perm[old]];
        }
    }

    pub fn nnz(&self) -> usize {
        self.vals.len()
    }
}

/// Run `f(first_row, rows)` over `n_rows` rows of `row_len` values, on scoped threads when the
/// work (`madds`) is large enough to pay for them, else inline.
fn parallel_rows(data: &mut [f64], n_rows: usize, row_len: usize, madds: usize, f: &(dyn Fn(usize, &mut [f64]) + Sync)) {
    const PARALLEL_MIN: usize = 400_000;
    let threads = if madds >= PARALLEL_MIN { std::thread::available_parallelism().map_or(1, |n| n.get()).min(8).min(n_rows) } else { 1 };
    if threads <= 1 || n_rows == 0 {
        f(0, &mut data[..n_rows * row_len]);
        return;
    }
    // Interleave the chunks so early (short) and late (long) rows of a triangular product balance.
    let chunk = n_rows.div_ceil(threads * 4).max(1);
    let jobs: Vec<(usize, &mut [f64])> = data[..n_rows * row_len].chunks_mut(chunk * row_len).enumerate().map(|(i, c)| (i * chunk, c)).collect();
    let next = std::sync::Mutex::new(jobs.into_iter());
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let job = next.lock().unwrap().next();
                match job {
                    Some((first, rows)) => f(first, rows),
                    None => break,
                }
            });
        }
    });
}

#[inline]
fn dot(a: &[f64], b: &[f64]) -> f64 {
    edge_check::linalg::dot(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5-point Laplacian plus a mass term on an `m x m` grid (SPD), two dofs per node coupled.
    fn grid(m: usize) -> (Vec<[f64; 2]>, Vec<Vec<usize>>) {
        let id = |i: usize, j: usize| i * m + j;
        let mut coords = vec![[0.0; 2]; m * m];
        let mut adj = vec![Vec::new(); m * m];
        for i in 0..m {
            for j in 0..m {
                coords[id(i, j)] = [i as f64, j as f64];
                for (di, dj) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                    let (a, b) = (i as i64 + di, j as i64 + dj);
                    if a >= 0 && b >= 0 && (a as usize) < m && (b as usize) < m {
                        adj[id(i, j)].push(id(a as usize, b as usize));
                    }
                }
            }
        }
        (coords, adj)
    }

    #[test]
    fn the_sparse_factor_solves_a_grid_system_exactly() {
        let m = 24;
        let (coords, adj) = grid(m);
        let n = m * m;
        let order = nested_dissection(&coords, &adj);
        let mut seen = vec![false; n];
        for &o in &order {
            assert!(!seen[o], "an ordering is a permutation");
            seen[o] = true;
        }
        assert_eq!(order.len(), n);
        let pairs: Vec<(usize, usize)> = (0..n).flat_map(|i| adj[i].iter().map(move |&j| (i, j))).collect();
        let pat = std::sync::Arc::new(Pattern::new(n, &order, pairs.iter().copied()));
        let mut a = SparseSym::zeros(&pat);
        for i in 0..n {
            a.add(i, i, 4.5);
            for &j in &adj[i] {
                if j > i {
                    a.add(i, j, -1.0);
                }
            }
        }
        let sym = std::sync::Arc::new(Symbolic::new(&pat));
        let ch = Cholesky::factor(&a, &sym).expect("positive definite");
        let x_true: Vec<f64> = (0..n).map(|i| ((i * 7 % 13) as f64) - 6.0).collect();
        let mut b = vec![0.0; n];
        for i in 0..n {
            b[i] += 4.5 * x_true[i];
            for &j in &adj[i] {
                b[i] -= x_true[j];
            }
        }
        ch.solve_in_place(&mut b);
        for i in 0..n {
            assert!((b[i] - x_true[i]).abs() < 1e-9, "{i}: {} vs {}", b[i], x_true[i]);
        }
        // Nested dissection keeps the fill near n log n, far below the dense n^2 / 2.
        assert!(ch.nnz() < 40 * n, "fill {}", ch.nnz());
    }

    #[test]
    fn an_indefinite_matrix_is_refused() {
        let order = vec![0, 1];
        let pat = std::sync::Arc::new(Pattern::new(2, &order, [(0, 1)].into_iter()));
        let mut a = SparseSym::zeros(&pat);
        a.add(0, 0, 1.0);
        a.add(1, 1, 1.0);
        a.add(0, 1, 2.0);
        let sym = std::sync::Arc::new(Symbolic::new(&pat));
        assert!(Cholesky::factor(&a, &sym).is_none());
    }
}
