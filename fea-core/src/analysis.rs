//! Linear static analysis driver and stress recovery.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::amg::{pcg, Amg, AmgOptions};
use crate::assembly::{assemble_stiffness, BlockMatrix, Pattern};
use crate::kernel::{self, Work};
use crate::linear::{residual, Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use crate::mesh::{Mesh, Physics};
use crate::sparse::Csr;
use std::sync::Arc;
use std::time::Instant;

/// Physical position and stress (component order of [`kernel::strain_at`]) at one Gauss point.
pub type GaussStress = ([f64; 3], [f64; 6]);

/// A mesh with its (reusable) sparsity pattern.
pub struct Model {
    pub mesh: Mesh,
    pub pattern: Arc<Pattern>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Timings {
    pub assemble_ms: f64,
    pub symbolic_ms: f64,
    pub factor_ms: f64,
    pub solve_ms: f64,
}

/// How the reduced linear system is solved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SolveMethod {
    /// Sparse Cholesky (faer).
    Direct,
    /// Conjugate gradients preconditioned by smoothed-aggregation multigrid with rigid-body modes.
    Iterative { tol: f64, max_iter: usize },
    /// Direct, except for 3D systems above [`AUTO_ITERATIVE_DOFS`] free dofs.
    Auto,
}

/// Free dofs above which a 3D model is solved iteratively under [`SolveMethod::Auto`] (the
/// factorization of a 3D system grows like `n^2` in flops and `n^{4/3}` in memory).
pub const AUTO_ITERATIVE_DOFS: usize = 150_000;

impl SolveMethod {
    /// The default iterative settings: relative residual `1e-10`.
    pub fn iterative() -> Self {
        SolveMethod::Iterative { tol: 1e-10, max_iter: 200 }
    }

    /// `Auto` replaced by the concrete method for a `dim`-dimensional system of `n_free` unknowns.
    pub fn resolve(self, dim: usize, n_free: usize) -> Self {
        match self {
            SolveMethod::Auto if dim == 3 && n_free > AUTO_ITERATIVE_DOFS => SolveMethod::iterative(),
            SolveMethod::Auto => SolveMethod::Direct,
            m => m,
        }
    }
}

pub struct Solution {
    /// Displacements `node * d + comp`.
    pub u: Vec<f64>,
    /// `K u - f`: the support reactions at constrained dofs (zero elsewhere to solver precision).
    pub reactions: Vec<f64>,
    pub n_free: usize,
    pub timings: Timings,
    /// Conjugate-gradient iterations (iterative solves only).
    pub iterations: Option<usize>,
}

impl Model {
    pub fn new(mesh: Mesh) -> Result<Self, String> {
        let pattern = Arc::new(Pattern::new(&mesh)?);
        Ok(Self { mesh, pattern })
    }

    pub fn assemble(&self) -> Result<BlockMatrix, String> {
        assemble_stiffness(&self.mesh, &self.pattern)
    }

    pub fn dirichlet(&self) -> Dirichlet {
        Dirichlet::new(self.mesh.nodes.len(), self.mesh.dim())
    }

    /// Linear static solve (method chosen by size: see [`SolveMethod::Auto`]).
    pub fn solve_static(&self, loads: &Loads, bc: &Dirichlet) -> Result<Solution, String> {
        self.solve_static_with(loads, bc, SolveMethod::Auto)
    }

    /// Linear static solve with an explicit method.
    pub fn solve_static_with(&self, loads: &Loads, bc: &Dirichlet, method: SolveMethod) -> Result<Solution, String> {
        match method.resolve(self.mesh.dim(), self.mesh.n_dofs() - bc.n_fixed()) {
            SolveMethod::Iterative { tol, max_iter } => self.solve_iterative(loads, bc, tol, max_iter),
            _ => self.solve_direct(loads, bc),
        }
    }

    fn solve_iterative(&self, loads: &Loads, bc: &Dirichlet, tol: f64, max_iter: usize) -> Result<Solution, String> {
        let mut t = Timings::default();
        let clock = Instant::now();
        let k = self.assemble()?;
        t.assemble_ms = clock.elapsed().as_secs_f64() * 1e3;
        let clock = Instant::now();
        let red = Reduced::structure_only(&self.pattern, bc).map_err(|e| e.to_string())?;
        let (cp, ri, vals) = red.csc(&k);
        let a = Csr::from_lower_csc(red.n_free(), cp, ri, &vals);
        let (node_ptr, b, nb) = self.near_null_space(&red);
        let amg = Amg::new(a.clone(), node_ptr, b, nb, AmgOptions::default()).map_err(|e| e.to_string())?;
        t.symbolic_ms = clock.elapsed().as_secs_f64() * 1e3;
        let clock = Instant::now();
        let f = loads::assemble(&self.mesh, loads)?;
        let (rf, mut u) = red.reduced_rhs(&k, &f, bc);
        let mut x = vec![0.0; rf.len()];
        let rep = pcg(&a, &rf, &mut x, &|r, z| amg.apply(r, z), tol, max_iter);
        if !rep.converged {
            return Err(format!("conjugate gradients did not converge: relative residual {:.2e} after {} iterations", rep.rel_residual, rep.iterations));
        }
        red.scatter(&x, &mut u);
        t.solve_ms = clock.elapsed().as_secs_f64() * 1e3;
        let mut reactions = residual(&k, &self.pattern, &u, &f);
        for (r, fx) in reactions.iter_mut().zip(&bc.fixed) {
            if !fx {
                *r = 0.0;
            }
        }
        Ok(Solution { u, reactions, n_free: red.n_free(), timings: t, iterations: Some(rep.iterations) })
    }

    /// Node grouping of the reduced dofs and the rigid-body near-null space (row major, `nb`
    /// columns) used to seed the multigrid coarse spaces.
    fn near_null_space(&self, red: &Reduced) -> (Vec<usize>, Vec<f64>, usize) {
        let d = self.mesh.dim();
        let free = red.free_dofs();
        let mut node_ptr = vec![0usize];
        for k in 1..free.len() {
            if free[k] as usize / d != free[k - 1] as usize / d {
                node_ptr.push(k);
            }
        }
        node_ptr.push(free.len());
        let n = self.mesh.nodes.len() as f64;
        let mut c = [0.0; 3];
        for x in &self.mesh.nodes {
            for i in 0..3 {
                c[i] += x[i] / n;
            }
        }
        let nb = match (self.mesh.physics, d) {
            (Physics::Axisymmetric, _) => 2,
            (_, 2) => 3,
            _ => 6,
        };
        let mut b = vec![0.0; free.len() * nb];
        for (row, &dof) in free.iter().enumerate() {
            let (node, comp) = (dof as usize / d, dof as usize % d);
            let r: [f64; 3] = std::array::from_fn(|i| self.mesh.nodes[node][i] - c[i]);
            let out = &mut b[row * nb..(row + 1) * nb];
            match (self.mesh.physics, d) {
                // (r, z): the two constant displacements (only the axial one is a true rigid-body mode).
                (Physics::Axisymmetric, _) => {
                    out[comp] = 1.0;
                }
                (_, 2) => {
                    out[comp] = 1.0;
                    out[2] = if comp == 0 { -r[1] } else { r[0] };
                }
                _ => {
                    out[comp] = 1.0;
                    // rotations about x, y, z: (0,-rz,ry), (rz,0,-rx), (-ry,rx,0)
                    let rot = [[0.0, -r[2], r[1]], [r[2], 0.0, -r[0]], [-r[1], r[0], 0.0]];
                    for q in 0..3 {
                        out[3 + q] = rot[q][comp];
                    }
                }
            }
        }
        (node_ptr, b, nb)
    }

    fn solve_direct(&self, loads: &Loads, bc: &Dirichlet) -> Result<Solution, String> {
        let mut t = Timings::default();
        let clock = Instant::now();
        let k = self.assemble()?;
        t.assemble_ms = clock.elapsed().as_secs_f64() * 1e3;
        let clock = Instant::now();
        let red = Reduced::with_ordering(&self.pattern, bc, Some(&self.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        t.symbolic_ms = clock.elapsed().as_secs_f64() * 1e3;
        let clock = Instant::now();
        let fac = red.factor(&k).map_err(|e| e.to_string())?;
        t.factor_ms = clock.elapsed().as_secs_f64() * 1e3;
        let clock = Instant::now();
        let f = loads::assemble(&self.mesh, loads)?;
        let u = fac.solve(&k, &f, bc);
        t.solve_ms = clock.elapsed().as_secs_f64() * 1e3;
        let mut reactions = residual(&k, &self.pattern, &u, &f);
        for (r, fx) in reactions.iter_mut().zip(&bc.fixed) {
            if !fx {
                *r = 0.0;
            }
        }
        Ok(Solution { u, reactions, n_free: red.n_free(), timings: t, iterations: None })
    }

    /// Strain energy `1/2 u^T K u`.
    pub fn strain_energy(&self, u: &[f64]) -> Result<f64, String> {
        let k = self.assemble()?;
        let mut ku = vec![0.0; u.len()];
        k.matvec_add(&self.pattern, u, &mut ku);
        Ok(0.5 * u.iter().zip(&ku).map(|(a, b)| a * b).sum::<f64>())
    }

    /// Stress at every Gauss point of every element, per mesh block:
    /// `[block][element * ngp + g] -> (physical position, stress)`; stress components in the order
    /// of [`kernel::strain_at`].
    pub fn gauss_stresses(&self, u: &[f64], delta_t: f64) -> Result<Vec<Vec<GaussStress>>, String> {
        let d = self.mesh.dim();
        let mut work = Work::new();
        let mut out = Vec::new();
        for blk in &self.mesh.blocks {
            let t = blk.kind.table();
            let nn = t.nn;
            let mut v = Vec::with_capacity(blk.n_elems() * t.ngp);
            let mut xyz = vec![[0.0; 3]; nn];
            let mut ue = vec![0.0; nn * d];
            for conn in blk.conn.chunks_exact(nn) {
                for (a, &nd) in conn.iter().enumerate() {
                    xyz[a] = self.mesh.nodes[nd];
                    for i in 0..d {
                        ue[a * d + i] = u[nd * d + i];
                    }
                }
                kernel::geometry(blk.kind, self.mesh.physics, &xyz, &mut work).map_err(|e| e.to_string())?;
                for g in 0..t.ngp {
                    let n = t.n_at(g);
                    let mut pos = [0.0; 3];
                    for a in 0..nn {
                        for i in 0..3 {
                            pos[i] += n[a] * xyz[a][i];
                        }
                    }
                    let strain = kernel::strain_at(blk.kind, self.mesh.physics, &work, g, &ue);
                    v.push((pos, kernel::stress_from_strain(self.mesh.physics, &blk.material, strain, delta_t)));
                }
            }
            out.push(v);
        }
        Ok(out)
    }

    /// Nodal stress: the stress of each element evaluated at its nodes, averaged over the
    /// elements that share the node.
    pub fn nodal_stresses(&self, u: &[f64], delta_t: f64) -> Result<Vec<[f64; 6]>, String> {
        let d = self.mesh.dim();
        let n = self.mesh.nodes.len();
        let mut sum = vec![[0.0f64; 6]; n];
        let mut count = vec![0u32; n];
        for blk in &self.mesh.blocks {
            let kind = blk.kind;
            let nn = kind.n_nodes();
            let mut xyz = vec![[0.0; 3]; nn];
            let mut ue = vec![0.0; nn * d];
            for conn in blk.conn.chunks_exact(nn) {
                for (a, &nd) in conn.iter().enumerate() {
                    xyz[a] = self.mesh.nodes[nd];
                    for i in 0..d {
                        ue[a * d + i] = u[nd * d + i];
                    }
                }
                for (a, &nd) in conn.iter().enumerate() {
                    // Gradients at node `a` itself: build a one-point table on the fly.
                    let s = node_stress(kind, self.mesh.physics, &blk.material, &xyz, &ue, a, delta_t)?;
                    for c in 0..6 {
                        sum[nd][c] += s[c];
                    }
                    count[nd] += 1;
                }
            }
        }
        for i in 0..n {
            if count[i] > 0 {
                for c in 0..6 {
                    sum[i][c] /= count[i] as f64;
                }
            }
        }
        Ok(sum)
    }
}

/// Stress at node `a` of one element (strain from the shape-function derivatives there).
fn node_stress(kind: crate::element::ElementKind, physics: crate::mesh::Physics, mat: &crate::mesh::Elastic, xyz: &[[f64; 3]], ue: &[f64], a: usize, delta_t: f64) -> Result<[f64; 6], String> {
    let d = kind.dim();
    let nn = kind.n_nodes();
    let xi = kind.node_coords()[a];
    let (n, dn) = kind.shape(xi);
    let mut jac = [[0.0f64; 3]; 3];
    for b in 0..nn {
        for i in 0..d {
            for k in 0..d {
                jac[i][k] += xyz[b][i] * dn[b][k];
            }
        }
    }
    let inv = invert(&jac, d).ok_or("degenerate element at a node")?;
    let mut h = [[0.0f64; 3]; 3];
    for b in 0..nn {
        let mut g = [0.0; 3];
        for i in 0..d {
            for k in 0..d {
                g[i] += inv[k][i] * dn[b][k];
            }
        }
        for i in 0..d {
            for k in 0..d {
                h[i][k] += ue[b * d + i] * g[k];
            }
        }
    }
    let strain = match physics {
        crate::mesh::Physics::Solid => [h[0][0], h[1][1], h[2][2], h[0][1] + h[1][0], h[1][2] + h[2][1], h[2][0] + h[0][2]],
        crate::mesh::Physics::Axisymmetric => {
            let r: f64 = (0..nn).map(|b| n[b] * xyz[b][0]).sum();
            let ur: f64 = (0..nn).map(|b| n[b] * ue[b * 2]).sum();
            // On the axis (r = 0) hoop strain equals radial strain by symmetry.
            let tt = if r > 1e-12 * (1.0 + xyz[0][0].abs()) { ur / r } else { h[0][0] };
            [h[0][0], h[1][1], tt, h[0][1] + h[1][0], 0.0, 0.0]
        }
        _ => [h[0][0], h[1][1], 0.0, h[0][1] + h[1][0], 0.0, 0.0],
    };
    Ok(kernel::stress_from_strain(physics, mat, strain, delta_t))
}

fn invert(j: &[[f64; 3]; 3], d: usize) -> Option<[[f64; 3]; 3]> {
    if d == 2 {
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        Some([[j[1][1] / det, -j[0][1] / det, 0.0], [-j[1][0] / det, j[0][0] / det, 0.0], [0.0; 3]])
    } else {
        let c00 = j[1][1] * j[2][2] - j[1][2] * j[2][1];
        let c01 = j[1][2] * j[2][0] - j[1][0] * j[2][2];
        let c02 = j[1][0] * j[2][1] - j[1][1] * j[2][0];
        let det = j[0][0] * c00 + j[0][1] * c01 + j[0][2] * c02;
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        Some([
            [c00 / det, (j[0][2] * j[2][1] - j[0][1] * j[2][2]) / det, (j[0][1] * j[1][2] - j[0][2] * j[1][1]) / det],
            [c01 / det, (j[0][0] * j[2][2] - j[0][2] * j[2][0]) / det, (j[0][2] * j[1][0] - j[0][0] * j[1][2]) / det],
            [c02 / det, (j[0][1] * j[2][0] - j[0][0] * j[2][1]) / det, (j[0][0] * j[1][1] - j[0][1] * j[1][0]) / det],
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_goes_iterative_only_for_large_3d_systems() {
        assert_eq!(SolveMethod::Auto.resolve(3, AUTO_ITERATIVE_DOFS), SolveMethod::Direct);
        assert_eq!(SolveMethod::Auto.resolve(3, AUTO_ITERATIVE_DOFS + 1), SolveMethod::iterative());
        assert_eq!(SolveMethod::Auto.resolve(2, 10 * AUTO_ITERATIVE_DOFS), SolveMethod::Direct, "2D factorizations stay cheap");
        assert_eq!(SolveMethod::Direct.resolve(3, 10 * AUTO_ITERATIVE_DOFS), SolveMethod::Direct);
    }
}
