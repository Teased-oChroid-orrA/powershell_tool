//! Compressed-row sparse matrices for the iterative solvers: symmetric expansion from the lower
//! triangle, parallel matrix-vector product, transpose and Gustavson matrix product.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use rayon::prelude::*;

#[derive(Debug, Clone)]
pub struct Csr {
    pub n_rows: usize,
    pub n_cols: usize,
    pub ptr: Vec<usize>,
    pub idx: Vec<u32>,
    pub val: Vec<f64>,
}

impl Csr {
    /// Full symmetric matrix from its lower triangle in compressed-column form.
    pub fn from_lower_csc(m: usize, col_ptr: &[u32], row_idx: &[u32], vals: &[f64]) -> Self {
        let mut count = vec![0usize; m + 1];
        for j in 0..m {
            for k in col_ptr[j] as usize..col_ptr[j + 1] as usize {
                let i = row_idx[k] as usize;
                count[i + 1] += 1;
                if i != j {
                    count[j + 1] += 1;
                }
            }
        }
        for i in 0..m {
            count[i + 1] += count[i];
        }
        let nnz = count[m];
        let mut fill = count.clone();
        let (mut idx, mut val) = (vec![0u32; nnz], vec![0.0; nnz]);
        // Row i receives its lower entries (columns j <= i) as columns j scan upward and its upper
        // entries (columns > i) from the mirrored writes, so each row ends up sorted by column.
        for j in 0..m {
            for k in col_ptr[j] as usize..col_ptr[j + 1] as usize {
                let i = row_idx[k] as usize;
                idx[fill[i]] = j as u32;
                val[fill[i]] = vals[k];
                fill[i] += 1;
                if i != j {
                    idx[fill[j]] = i as u32;
                    val[fill[j]] = vals[k];
                    fill[j] += 1;
                }
            }
        }
        let mut a = Self { n_rows: m, n_cols: m, ptr: count, idx, val };
        a.sort_rows();
        a
    }

    fn sort_rows(&mut self) {
        let (ptr, idx, val) = (&self.ptr, &mut self.idx, &mut self.val);
        let mut tmp: Vec<(u32, f64)> = Vec::new();
        for i in 0..self.n_rows {
            let r = ptr[i]..ptr[i + 1];
            if idx[r.clone()].windows(2).all(|w| w[0] < w[1]) {
                continue;
            }
            tmp.clear();
            tmp.extend(idx[r.clone()].iter().copied().zip(val[r.clone()].iter().copied()));
            tmp.sort_by_key(|t| t.0);
            for (k, (c, v)) in tmp.iter().enumerate() {
                idx[r.start + k] = *c;
                val[r.start + k] = *v;
            }
        }
    }

    /// Lower triangle in compressed-column form of a symmetric matrix (the upper triangle of each
    /// CSR row is the corresponding column of the lower triangle).
    pub fn to_lower_csc(&self) -> (Vec<u32>, Vec<u32>, Vec<f64>) {
        let (mut col_ptr, mut row_idx, mut vals) = (vec![0u32], Vec::new(), Vec::new());
        for i in 0..self.n_rows {
            for k in self.ptr[i]..self.ptr[i + 1] {
                if self.idx[k] as usize >= i {
                    row_idx.push(self.idx[k]);
                    vals.push(self.val[k]);
                }
            }
            col_ptr.push(row_idx.len() as u32);
        }
        (col_ptr, row_idx, vals)
    }

    pub fn nnz(&self) -> usize {
        self.val.len()
    }

    /// `y = A x`, parallel over rows.
    pub fn spmv(&self, x: &[f64], y: &mut [f64]) {
        debug_assert!(x.len() >= self.n_cols && y.len() >= self.n_rows);
        let (ptr, idx, val) = (&self.ptr, &self.idx, &self.val);
        let row = |i: usize| -> f64 { (ptr[i]..ptr[i + 1]).map(|k| val[k] * x[idx[k] as usize]).sum() };
        if self.n_rows < 4096 {
            for (i, yi) in y[..self.n_rows].iter_mut().enumerate() {
                *yi = row(i);
            }
        } else {
            y[..self.n_rows].par_iter_mut().enumerate().for_each(|(i, yi)| *yi = row(i));
        }
    }

    pub fn diagonal(&self) -> Vec<f64> {
        (0..self.n_rows)
            .map(|i| {
                let r = self.ptr[i]..self.ptr[i + 1];
                match self.idx[r.clone()].binary_search(&(i as u32)) {
                    Ok(k) => self.val[r.start + k],
                    Err(_) => 0.0,
                }
            })
            .collect()
    }

    pub fn transpose(&self) -> Self {
        let mut count = vec![0usize; self.n_cols + 1];
        for &c in &self.idx {
            count[c as usize + 1] += 1;
        }
        for j in 0..self.n_cols {
            count[j + 1] += count[j];
        }
        let mut fill = count.clone();
        let (mut idx, mut val) = (vec![0u32; self.nnz()], vec![0.0; self.nnz()]);
        for i in 0..self.n_rows {
            for k in self.ptr[i]..self.ptr[i + 1] {
                let c = self.idx[k] as usize;
                idx[fill[c]] = i as u32;
                val[fill[c]] = self.val[k];
                fill[c] += 1;
            }
        }
        Self { n_rows: self.n_cols, n_cols: self.n_rows, ptr: count, idx, val }
    }

    /// `A * B` (Gustavson), parallel over rows of `A`.
    pub fn matmul(&self, b: &Csr) -> Csr {
        assert_eq!(self.n_cols, b.n_rows);
        let rows: Vec<(Vec<u32>, Vec<f64>)> = (0..self.n_rows)
            .into_par_iter()
            .map_init(
                || (vec![usize::MAX; b.n_cols], vec![0.0f64; b.n_cols]),
                |(mark, acc), i| {
                    let mut cols: Vec<u32> = Vec::new();
                    for ka in self.ptr[i]..self.ptr[i + 1] {
                        let (j, va) = (self.idx[ka] as usize, self.val[ka]);
                        for kb in b.ptr[j]..b.ptr[j + 1] {
                            let c = b.idx[kb] as usize;
                            if mark[c] != i {
                                mark[c] = i;
                                acc[c] = 0.0;
                                cols.push(c as u32);
                            }
                            acc[c] += va * b.val[kb];
                        }
                    }
                    cols.sort_unstable();
                    let vals = cols.iter().map(|&c| acc[c as usize]).collect();
                    (cols, vals)
                },
            )
            .collect();
        let mut ptr = Vec::with_capacity(self.n_rows + 1);
        ptr.push(0);
        let (mut idx, mut val) = (Vec::new(), Vec::new());
        for (c, v) in rows {
            idx.extend(c);
            val.extend(v);
            ptr.push(idx.len());
        }
        Csr { n_rows: self.n_rows, n_cols: b.n_cols, ptr, idx, val }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lower triangle of `[[4,1,0],[1,3,2],[0,2,5]]`.
    fn small() -> Csr {
        Csr::from_lower_csc(3, &[0, 2, 4, 5], &[0, 1, 1, 2, 2], &[4.0, 1.0, 3.0, 2.0, 5.0])
    }

    #[test]
    fn symmetric_expansion_matvec_and_diagonal() {
        let a = small();
        assert_eq!(a.nnz(), 7);
        let mut y = [0.0; 3];
        a.spmv(&[1.0, 2.0, 3.0], &mut y);
        assert_eq!(y, [6.0, 1.0 + 6.0 + 6.0, 4.0 + 15.0]);
        assert_eq!(a.diagonal(), vec![4.0, 3.0, 5.0]);
        assert!(a.ptr.windows(2).all(|w| w[0] <= w[1]));
        for i in 0..3 {
            assert!(a.idx[a.ptr[i]..a.ptr[i + 1]].windows(2).all(|w| w[0] < w[1]));
        }
    }

    #[test]
    fn transpose_and_product_match_dense() {
        let a = small();
        let t = a.transpose();
        assert_eq!(t.val, a.val, "symmetric matrix equals its transpose");
        let sq = a.matmul(&a);
        // A^2 row 1 = [4*1+1*3, 1+9+4, 6+10] = [7, 14, 16]
        let r = sq.ptr[1]..sq.ptr[2];
        assert_eq!(&sq.val[r], &[7.0, 14.0, 16.0]);
        // Rectangular product: (3x3) * (3x2)
        let b = Csr { n_rows: 3, n_cols: 2, ptr: vec![0, 1, 3, 4], idx: vec![0, 0, 1, 1], val: vec![1.0, 2.0, 3.0, 4.0] };
        let ab = a.matmul(&b);
        let dense: Vec<f64> = (0..3).flat_map(|i| (0..2).map(move |j| (i, j))).map(|(i, j)| (ab.ptr[i]..ab.ptr[i + 1]).find(|&k| ab.idx[k] as usize == j).map_or(0.0, |k| ab.val[k])).collect();
        // rows of A times columns of B: [4+2, 3] / [1+6, 9+8] / [4+... computed by hand below
        assert_eq!(dense, vec![4.0 * 1.0 + 1.0 * 2.0, 1.0 * 3.0, 1.0 * 1.0 + 3.0 * 2.0, 3.0 * 3.0 + 2.0 * 4.0, 2.0 * 2.0 + 5.0 * 0.0, 2.0 * 3.0 + 5.0 * 4.0]);
    }
}
