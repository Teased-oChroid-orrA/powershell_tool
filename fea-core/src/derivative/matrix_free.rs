//! Matrix-free tangent of an elastic-plastic model (Story F): the action `K_T v` of the consistent tangent computed element by
//! element from the constitutive Jacobian-vector products, with no global matrix assembled.
//!
//! Two modes: **cached** stores one `6 x 6` constitutive tangent per Gauss point (assembled-free but memory proportional to the number
//! of Gauss points, the usual matrix-free finite-element trade), **recompute** stores nothing and evaluates the constitutive JVP
//! at every product (memory independent of the mesh, more arithmetic). Element contributions are computed in parallel into an ordered
//! buffer and scattered in element order, so the product is bit-identical for any thread count.
//!
//! It is *not* assumed to be faster than the assembled sparse product: [`MatrixFreeTangent::apply`] and the assembled matvec are
//! benchmarked against each other in `tests/derivative_bench.rs`, and [`super::select_sensitivity`] prices both.

use super::constitutive::*;
use super::linalg::LinearOperator;
use super::path::{Gp, Law, Setup, W};
use crate::analysis::Model;
use crate::linear::Dirichlet;
use crate::nonlinear::NlState;
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixFreeMode {
    Cached,
    Recompute,
}

pub struct MatrixFreeTangent<'m> {
    setup: Setup<'m>,
    u: Vec<f64>,
    old: Vec<[f64; STATE]>,
    mode: MatrixFreeMode,
    /// Per Gauss point `C` (6 x 6 row-major) in `Cached` mode.
    c: Vec<[f64; 36]>,
    symmetric: bool,
}

impl<'m> MatrixFreeTangent<'m> {
    /// The tangent at displacements `u` from the committed state `old` (the state at the start of the step `u` belongs to).
    pub fn new(model: &'m Model, bc: &Dirichlet, u: &[f64], old: &NlState, mode: MatrixFreeMode) -> Result<Self, String> {
        let free: Vec<usize> = (0..model.mesh.n_dofs()).filter(|&d| !bc.fixed[d]).collect();
        let setup = Setup::new(model, Vec::new(), free)?;
        let old_gp = setup.gp_states(old);
        let mut c = Vec::new();
        let mut symmetric = true;
        {
            let all: Result<Vec<[f64; 36]>, String> = setup.gps.par_iter().zip(old_gp.par_iter()).map(|(gp, o)| setup.sens(gp, u, o).map(|s| std::array::from_fn(|i| s.d_stress_d_strain[i]))).collect();
            let all = all?;
            // the tangent is symmetric in the work-conjugate pairing when W C is
            for t in &all {
                let scale = (0..36).fold(0.0f64, |m, k| m.max((W[k / 6] * t[k]).abs())).max(1e-300);
                for i in 0..VOIGT {
                    for j in 0..VOIGT {
                        if (W[i] * t[i * 6 + j] - W[j] * t[j * 6 + i]).abs() > 1e-9 * scale {
                            symmetric = false;
                        }
                    }
                }
            }
            if mode == MatrixFreeMode::Cached {
                c = all;
            }
        }
        Ok(Self { setup, u: u.to_vec(), old: old_gp, mode, c, symmetric })
    }

    pub fn mode(&self) -> MatrixFreeMode {
        self.mode
    }

    /// Bytes held between products.
    pub fn stored_bytes(&self) -> usize {
        self.c.len() * 36 * 8 + self.old.len() * STATE * 8
    }

    fn element_ranges(&self) -> Vec<(usize, usize)> {
        // Gauss points of one element are contiguous
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.setup.gps.len() {
            let (b, e) = (self.setup.gps[i].block, self.setup.gps[i].elem);
            let mut j = i;
            while j < self.setup.gps.len() && self.setup.gps[j].block == b && self.setup.gps[j].elem == e {
                j += 1;
            }
            out.push((i, j));
            i = j;
        }
        out
    }

    fn gp_dstress(&self, gi: usize, gp: &Gp, v: &[f64]) -> [f64; VOIGT] {
        let dstrain = self.setup.strain(gp, v);
        match self.mode {
            MatrixFreeMode::Cached => {
                let c = &self.c[gi];
                std::array::from_fn(|i| (0..VOIGT).map(|j| c[i * 6 + j] * dstrain[j]).sum())
            }
            MatrixFreeMode::Recompute => {
                let strain = self.setup.strain(gp, &self.u);
                self.jvp(gp, &strain, &self.old[gi], &dstrain).expect("a converged state differentiates")
            }
        }
    }

    fn jvp(&self, gp: &Gp, strain: &[f64; VOIGT], old: &[f64; STATE], d: &[f64; VOIGT]) -> Result<[f64; VOIGT], ConstitutiveError> {
        let blk = &self.setup.model.mesh.blocks[gp.block];
        let (e, nu) = (blk.material.e, blk.material.nu);
        match &self.setup.laws[gp.block] {
            Law::Elastic { plane_stress: false } => stress_jvp(&Hooke { e, nu }, &[e, nu], strain, old, d),
            Law::Elastic { plane_stress: true } => stress_jvp(&PlaneStress { inner: Hooke { e, nu } }, &[e, nu], strain, old, d),
            Law::Plastic { plane_stress: false, table } => stress_jvp(table, &table.parameters(), strain, old, d),
            Law::Plastic { plane_stress: true, table } => stress_jvp(&PlaneStress { inner: table.clone() }, &table.parameters(), strain, old, d),
        }
    }

    /// `y = K_T v` over all dofs (`v` zero at constrained dofs on entry is not required: constrained entries of `v` are ignored).
    pub fn apply_full(&self, v: &[f64]) -> Vec<f64> {
        let n = self.setup.model.mesh.n_dofs();
        let ranges = self.element_ranges();
        // per element: (dofs, force contribution), computed in parallel, scattered in element order
        let parts: Vec<(&[usize], Vec<f64>)> = ranges
            .par_iter()
            .map(|&(a, b)| {
                let dofs = &self.setup.gps[a].dofs[..];
                let mut fe = vec![0.0; dofs.len()];
                for gi in a..b {
                    let gp = &self.setup.gps[gi];
                    let ds = self.gp_dstress(gi, gp, v);
                    for (k, col) in gp.b.iter().enumerate() {
                        let mut s = 0.0;
                        for c in 0..VOIGT {
                            s += col[c] * W[c] * ds[c];
                        }
                        fe[k] += gp.w * s;
                    }
                }
                (dofs, fe)
            })
            .collect();
        let mut y = vec![0.0; n];
        for (dofs, fe) in parts {
            for (d, f) in dofs.iter().zip(fe) {
                y[*d] += f;
            }
        }
        y
    }

    /// The diagonal of the tangent over the free dofs (a Jacobi preconditioner), from the cached tangents.
    pub fn diagonal(&self) -> Result<Vec<f64>, String> {
        if self.mode != MatrixFreeMode::Cached {
            return Err("the diagonal is built from the cached tangents".into());
        }
        let mut diag = vec![0.0; self.setup.model.mesh.n_dofs()];
        for (gi, gp) in self.setup.gps.iter().enumerate() {
            let c = &self.c[gi];
            for (k, col) in gp.b.iter().enumerate() {
                let mut s = 0.0;
                for i in 0..VOIGT {
                    let mut ci = 0.0;
                    for j in 0..VOIGT {
                        ci += c[i * 6 + j] * col[j];
                    }
                    s += col[i] * W[i] * ci;
                }
                diag[gp.dofs[k]] += gp.w * s;
            }
        }
        Ok(self.setup.free.iter().map(|&d| diag[d]).collect())
    }

    pub fn free_dofs(&self) -> &[usize] {
        &self.setup.free
    }
}

impl LinearOperator for MatrixFreeTangent<'_> {
    fn dim(&self) -> usize {
        self.setup.free.len()
    }
    fn apply(&self, x: &[f64], y: &mut [f64]) {
        let mut v = vec![0.0; self.setup.model.mesh.n_dofs()];
        for (j, &d) in self.setup.free.iter().enumerate() {
            v[d] = x[j];
        }
        let full = self.apply_full(&v);
        for (j, &d) in self.setup.free.iter().enumerate() {
            y[j] = full[d];
        }
    }
    /// The transposed product. For a symmetric tangent (associative plasticity, hyperelasticity) it is the product itself; for an
    /// unsymmetric one it uses the transposed cached tangents (`Cached` mode only).
    fn apply_transpose(&self, x: &[f64], y: &mut [f64]) {
        if self.symmetric {
            return self.apply(x, y);
        }
        assert!(self.mode == MatrixFreeMode::Cached, "the transposed product of an unsymmetric tangent needs the cached tangents");
        let mut v = vec![0.0; self.setup.model.mesh.n_dofs()];
        for (j, &d) in self.setup.free.iter().enumerate() {
            v[d] = x[j];
        }
        let n = v.len();
        let mut out = vec![0.0; n];
        for (gi, gp) in self.setup.gps.iter().enumerate() {
            let e = self.setup.strain(gp, &v);
            let c = &self.c[gi];
            // (B^T W C B)^T = B^T C^T W B
            let we: Vec<f64> = (0..VOIGT).map(|i| W[i] * e[i]).collect();
            let ds: [f64; VOIGT] = std::array::from_fn(|j| (0..VOIGT).map(|i| c[i * 6 + j] * we[i]).sum());
            for (k, col) in gp.b.iter().enumerate() {
                let mut s = 0.0;
                for cidx in 0..VOIGT {
                    s += col[cidx] * ds[cidx];
                }
                out[gp.dofs[k]] += gp.w * s;
            }
        }
        for (j, &d) in self.setup.free.iter().enumerate() {
            y[j] = out[d];
        }
    }
    fn is_symmetric(&self) -> bool {
        self.symmetric
    }
}
