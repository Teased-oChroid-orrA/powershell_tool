//! A deformable (elastic) pin.
//!
//! The rigid analytic pin of `contact.rs` presses on the lug with a prescribed shape. A real
//! pin ovalises under the bearing load, spreading it. Here the pin is an elastic disc (a ring
//! with a small core hole, plane stress or plane strain like the lug) whose surface nodes sit at
//! the same angles as the lug's pin-contact nodes. Its compliance `A` on those nodes is added to
//! the lug's bore Green's matrix, `G~ = G + A`, and the contact is solved exactly as for a rigid
//! pin of the same radius against the combined bore displacement `u~ = u_lug + A f`
//! (`u_lug - w` with `w = -A f` the pin's own surface displacement): the contact code only sees
//! a different `G`/`S`. Nothing else changes.
//!
//! The pin is a free body: its rigid motion (translation, and rotation in a full model) is the
//! pin position the contact solves for. The contact forces on it are not self-equilibrated (their
//! sum is the pin load), so they are balanced by a body force proportional to the nodal mass
//! (the pin loaded through its length) and the displacement is taken orthogonal to the rigid
//! modes in the mass inner product.

use crate::fe::{invert_spd, Condensed, Material};
use edge_check::fem::{element_stiffness, shape_q9, GAUSS3};
use edge_check::linalg::BandedSpd;

/// Ratio of the core hole to the pin radius: the disc is not meshed to its centre.
const CORE_RATIO: f64 = 0.1;
/// Regularisation of the free body, as a fraction of `E` (the load is projected so it only
/// pins the rigid modes numerically).
const GROUND: f64 = 1e-9;

pub struct PinDisc {
    nodes: Vec<[f64; 2]>,
    elems: Vec<[usize; 9]>,
    n_cols: usize,
    n_rows: usize,
    rank: Vec<usize>,
    factor: BandedSpd,
    constrained: Vec<bool>,
    /// Lumped (row-sum) mass of every node: `int N_a dA`.
    mass: Vec<f64>,
    periodic: bool,
}

impl PinDisc {
    /// `theta` are the lug's column angles (`mesh.theta`); `material` the constants the 2D
    /// stiffness is assembled with (effective ones in plane strain).
    pub fn build(theta: &[f64], periodic: bool, radius: f64, material: Material, layers: usize) -> Result<Self, String> {
        if radius.is_nan() || radius <= 0.0 || layers == 0 {
            return Err("the pin needs a positive radius and at least one layer".into());
        }
        let n_cols = theta.len();
        let n_rows = 2 * layers + 1;
        let r_core = CORE_RATIO * radius;
        let mut nodes = vec![[0.0; 2]; n_cols * n_rows];
        for (c, &t) in theta.iter().enumerate() {
            for i in 0..n_rows {
                let r = r_core + (radius - r_core) * i as f64 / (n_rows - 1) as f64;
                nodes[c * n_rows + i] = [r * t.cos(), r * t.sin()];
            }
        }
        let n_ang = if periodic { n_cols / 2 } else { (n_cols - 1) / 2 };
        let col = |c: usize| if periodic { c % n_cols } else { c };
        let mut elems = Vec::new();
        for jj in 0..n_ang {
            let cols = [col(2 * jj), col(2 * jj + 1), col(2 * jj + 2)];
            for k in 0..layers {
                let mut e = [0usize; 9];
                for (b, &c) in cols.iter().enumerate() {
                    for ia in 0..3 {
                        e[3 * b + ia] = c * n_rows + 2 * k + ia;
                    }
                }
                elems.push(e);
            }
        }
        // Banded ordering, folded for a closed ring (as in the lug).
        let col_rank = |c: usize| -> usize {
            if !periodic {
                c
            } else if c < n_cols.div_ceil(2) {
                2 * c
            } else {
                2 * (n_cols - 1 - c) + 1
            }
        };
        let mut rank = vec![0usize; nodes.len()];
        for c in 0..n_cols {
            for i in 0..n_rows {
                rank[c * n_rows + i] = col_rank(c) * n_rows + i;
            }
        }
        let bw_nodes = elems.iter().map(|e| {
            let (lo, hi) = e.iter().fold((usize::MAX, 0usize), |(lo, hi), &n| (lo.min(rank[n]), hi.max(rank[n])));
            hi - lo
        }).max().unwrap_or(0);
        let bw = 2 * bw_nodes + 1;
        let n_dof = 2 * nodes.len();
        let dof = |node: usize, comp: usize| 2 * rank[node] + comp;
        // Symmetry plane of a half model: no motion normal to it.
        let mut constrained = vec![false; n_dof];
        if !periodic {
            for (id, p) in nodes.iter().enumerate() {
                if p[1].abs() < 1e-12 {
                    constrained[dof(id, 1)] = true;
                }
            }
        }
        let d = material.d_matrix();
        let mut k = BandedSpd::zeros(n_dof, bw);
        let mut mass = vec![0.0; nodes.len()];
        for e in &elems {
            let xy: Vec<[f64; 2]> = e.iter().map(|&n| nodes[n]).collect();
            let ke = element_stiffness(&xy, &d)?;
            for (la, &na) in e.iter().enumerate() {
                for (lb, &nb) in e.iter().enumerate() {
                    for ca in 0..2 {
                        let i = dof(na, ca);
                        if constrained[i] {
                            continue;
                        }
                        for cb in 0..2 {
                            let j = dof(nb, cb);
                            if j > i || constrained[j] {
                                continue;
                            }
                            k.add(i, j, ke[2 * la + ca][2 * lb + cb]);
                        }
                    }
                }
            }
            for &(gx, wx) in &GAUSS3 {
                for &(gy, wy) in &GAUSS3 {
                    let (n, dxi, deta) = shape_q9(gx, gy);
                    let (mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0);
                    for a in 0..9 {
                        j11 += dxi[a] * xy[a][0];
                        j12 += dxi[a] * xy[a][1];
                        j21 += deta[a] * xy[a][0];
                        j22 += deta[a] * xy[a][1];
                    }
                    let det = j11 * j22 - j12 * j21;
                    if det <= 0.0 {
                        return Err("inverted pin element".into());
                    }
                    for (a, &na) in e.iter().enumerate() {
                        mass[na] += n[a] * det * wx * wy;
                    }
                }
            }
        }
        let scale = (0..n_dof).map(|i| k.diag(i)).fold(0.0, f64::max).max(1.0);
        for (i, &fixed) in constrained.iter().enumerate() {
            k.add(i, i, if fixed { scale } else { GROUND * material.e_psi });
        }
        if !k.factor() {
            return Err("the pin stiffness is not positive definite".into());
        }
        Ok(Self { nodes, elems, n_cols, n_rows, rank, factor: k, constrained, mass, periodic })
    }

    pub fn n_cols(&self) -> usize {
        self.n_cols
    }

    /// Node id of the pin surface at lattice column `c`.
    pub fn surface_node(&self, c: usize) -> usize {
        c * self.n_rows + self.n_rows - 1
    }

    fn dof(&self, node: usize, comp: usize) -> usize {
        2 * self.rank[node] + comp
    }

    /// Rigid modes as nodal vectors (`2 * node + comp`): translations, and rotation in a ring.
    fn rigid_modes(&self) -> Vec<Vec<f64>> {
        let n = self.nodes.len();
        let mut modes = vec![vec![0.0; 2 * n]];
        for i in 0..n {
            modes[0][2 * i] = 1.0;
        }
        if self.periodic {
            let mut ty = vec![0.0; 2 * n];
            let mut rot = vec![0.0; 2 * n];
            for (i, p) in self.nodes.iter().enumerate() {
                ty[2 * i + 1] = 1.0;
                rot[2 * i] = -p[1];
                rot[2 * i + 1] = p[0];
            }
            modes.push(ty);
            modes.push(rot);
        }
        modes
    }

    /// `c = (R^T M R)^-1 R^T v` for the mass-weighted rigid components of the nodal vector `v`.
    fn rigid_components(&self, modes: &[Vec<f64>], v: &[f64]) -> Vec<f64> {
        let q = modes.len();
        let mv = |a: &[f64], b: &[f64]| -> f64 { (0..self.nodes.len()).map(|i| self.mass[i] * (a[2 * i] * b[2 * i] + a[2 * i + 1] * b[2 * i + 1])).sum() };
        let mut gram = vec![0.0; q * q];
        let mut rhs = vec![0.0; q];
        for i in 0..q {
            for j in 0..q {
                gram[i * q + j] = mv(&modes[i], &modes[j]);
            }
            rhs[i] = mv(&modes[i], v);
        }
        // q <= 3: solve by Gaussian elimination.
        solve_small(&mut gram, &mut rhs, q);
        rhs
    }

    /// Displacement of the pin for nodal forces `f` on it (`2 * node + comp`): the response to
    /// `f` plus the mass-proportional body force that balances its resultant, with the rigid
    /// motion removed (mass-orthogonal).
    pub fn displacements(&self, f: &[f64]) -> Vec<f64> {
        let n = self.nodes.len();
        let modes = self.rigid_modes();
        // Body force b = -M R c with c such that R^T (f + b) = 0, i.e. R^T M R c = R^T f.
        let c = self.rigid_components_force(&modes, f);
        let mut rhs = vec![0.0; self.factor.n];
        for i in 0..n {
            for comp in 0..2 {
                let mut v = f[2 * i + comp];
                for (q, mode) in modes.iter().enumerate() {
                    v -= self.mass[i] * mode[2 * i + comp] * c[q];
                }
                let d = self.dof(i, comp);
                if !self.constrained[d] {
                    rhs[d] = v;
                }
            }
        }
        self.factor.solve_in_place(&mut rhs);
        let mut w = vec![0.0; 2 * n];
        for i in 0..n {
            for comp in 0..2 {
                w[2 * i + comp] = rhs[self.dof(i, comp)];
            }
        }
        let rc = self.rigid_components(&modes, &w);
        for (q, mode) in modes.iter().enumerate() {
            for (wi, mi) in w.iter_mut().zip(mode) {
                *wi -= rc[q] * mi;
            }
        }
        w
    }

    /// `(R^T M R)^-1 R^T f` for a *force* vector (no mass weight on `f`).
    fn rigid_components_force(&self, modes: &[Vec<f64>], f: &[f64]) -> Vec<f64> {
        let q = modes.len();
        let n = self.nodes.len();
        let mut gram = vec![0.0; q * q];
        let mut rhs = vec![0.0; q];
        for i in 0..q {
            for j in 0..q {
                gram[i * q + j] = (0..n).map(|a| self.mass[a] * (modes[i][2 * a] * modes[j][2 * a] + modes[i][2 * a + 1] * modes[j][2 * a + 1])).sum();
            }
            rhs[i] = (0..n).map(|a| modes[i][2 * a] * f[2 * a] + modes[i][2 * a + 1] * f[2 * a + 1]).sum();
        }
        solve_small(&mut gram, &mut rhs, q);
        rhs
    }

    /// Elements (for stress recovery by the caller).
    pub fn elems(&self) -> &[[usize; 9]] {
        &self.elems
    }

    pub fn nodes(&self) -> &[[f64; 2]] {
        &self.nodes
    }
}

fn solve_small(a: &mut [f64], b: &mut [f64], n: usize) {
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| a[i * n + col].abs().total_cmp(&a[j * n + col].abs())).unwrap();
        for k in 0..n {
            a.swap(col * n + k, piv * n + k);
        }
        b.swap(col, piv);
        let d = a[col * n + col];
        if d.abs() < 1e-300 {
            continue;
        }
        for r in col + 1..n {
            let f = a[r * n + col] / d;
            for k in col..n {
                a[r * n + k] -= f * a[col * n + k];
            }
            b[r] -= f * b[col];
        }
    }
    for r in (0..n).rev() {
        let mut v = b[r];
        for k in r + 1..n {
            v -= a[r * n + k] * b[k];
        }
        let d = a[r * n + r];
        b[r] = if d.abs() < 1e-300 { 0.0 } else { v / d };
    }
}

/// The lug's bore compliance with the pin's added, and its inverse: what the contact solves with.
pub struct CombinedCompliance {
    /// `G + A`, dense `m x m` row-major.
    pub g: Vec<f64>,
    /// `(G + A)^-1`.
    pub s: Vec<f64>,
}

impl PinDisc {
    /// `A`: the pin's surface compliance on the lug's free pin-contact dofs, added to `G`.
    pub fn combine(&self, lug: &Condensed) -> Result<CombinedCompliance, String> {
        let m = lug.m;
        let nc = self.n_cols;
        if lug.mesh.n_cols != nc {
            return Err("the pin and lug meshes do not share their angular stations".into());
        }
        // (lug contact dof index, pin node, comp) of every coupled dof.
        let mut pairs: Vec<(usize, usize, usize)> = Vec::new();
        for c in 0..nc {
            let lug_node = lug.contact_nodes[c];
            for comp in 0..2 {
                if let Some(i) = lug.cidx(lug_node, comp) {
                    pairs.push((i, self.surface_node(c), comp));
                }
            }
        }
        let mut g = lug.g.clone();
        let nn = self.nodes.len();
        for &(ij, node_j, comp_j) in &pairs {
            let mut f = vec![0.0; 2 * nn];
            f[2 * node_j + comp_j] = 1.0;
            let w = self.displacements(&f);
            for &(ii, node_i, comp_i) in &pairs {
                g[ii * m + ij] += w[2 * node_i + comp_i];
            }
        }
        // Remove round-off asymmetry.
        for i in 0..m {
            for j in 0..i {
                let v = 0.5 * (g[i * m + j] + g[j * m + i]);
                g[i * m + j] = v;
                g[j * m + i] = v;
            }
        }
        let s = invert_spd(&g, m)?;
        Ok(CombinedCompliance { g, s })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEEL: Material = Material { e_psi: 29.0e6, nu: 0.30 };

    fn ring_theta(n: usize) -> Vec<f64> {
        (0..n).map(|c| 2.0 * std::f64::consts::PI * c as f64 / n as f64).collect()
    }

    /// Nodal forces of a uniform pressure `p` pushing inward on the pin surface (forces ON the pin).
    fn pressure_forces(pin: &PinDisc, radius: f64, p: f64) -> Vec<f64> {
        let nc = pin.n_cols();
        let mut f = vec![0.0; 2 * pin.nodes().len()];
        let theta = ring_theta(nc);
        for e in 0..nc / 2 {
            let (c0, c1, c2) = (2 * e, 2 * e + 1, (2 * e + 2) % nc);
            let span = if e + 1 == nc / 2 { 2.0 * std::f64::consts::PI - theta[c0] } else { theta[c2] - theta[c0] };
            let len = radius * span;
            for (c, w) in [(c0, 1.0 / 6.0), (c1, 4.0 / 6.0), (c2, 1.0 / 6.0)] {
                let node = pin.surface_node(c);
                f[2 * node] -= p * len * w * theta[c].cos();
                f[2 * node + 1] -= p * len * w * theta[c].sin();
            }
        }
        f
    }

    #[test]
    fn a_uniform_external_pressure_compresses_the_disc_by_the_lame_displacement() {
        let (b, p) = (0.25, 10_000.0);
        let a = CORE_RATIO * b;
        let pin = PinDisc::build(&ring_theta(96), true, b, STEEL, 6).unwrap();
        let w = pin.displacements(&pressure_forces(&pin, b, p));
        let (aa, bb) = (-p * b * b / (b * b - a * a), -p * a * a * b * b / (b * b - a * a));
        let exact = ((1.0 - STEEL.nu) * aa * b + (1.0 + STEEL.nu) * bb / b) / STEEL.e_psi;
        for c in [0usize, 17, 40, 70] {
            let node = pin.surface_node(c);
            let th = ring_theta(96)[c];
            let ur = w[2 * node] * th.cos() + w[2 * node + 1] * th.sin();
            assert!((ur / exact - 1.0).abs() < 0.01, "column {c}: {ur:.3e} vs Lame {exact:.3e}");
        }
    }

    #[test]
    fn the_response_is_free_of_rigid_motion_and_a_net_force_does_not_translate_the_pin() {
        let b = 0.25;
        let pin = PinDisc::build(&ring_theta(48), true, b, STEEL, 4).unwrap();
        // A net force on one node: balanced by the body force, so the pin deforms, not flies off.
        let mut f = vec![0.0; 2 * pin.nodes().len()];
        let node = pin.surface_node(0);
        f[2 * node] = 100.0;
        let w = pin.displacements(&f);
        let modes = pin.rigid_modes();
        let rc = pin.rigid_components(&modes, &w);
        assert!(rc.iter().all(|c| c.abs() < 1e-9), "rigid components {rc:?}");
        assert!(w.iter().any(|v| v.abs() > 1e-9));
        // Symmetric positive semi-definite response: u.f >= 0 for any f.
        let work: f64 = w.iter().zip(&f).map(|(a, b)| a * b).sum();
        assert!(work > 0.0);
    }
}
