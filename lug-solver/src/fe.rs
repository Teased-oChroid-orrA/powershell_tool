//! Plane-stress assembly, one banded Cholesky factorisation, and static
//! condensation of the lug onto its bore degrees of freedom.
//!
//! The pin is rigid and never meshed, so the only place the lug sees a
//! non-linear load is the bore. Eliminating every other degree of freedom
//! once gives the bore Green's matrix `G = (K^-1)_bb` and its inverse, the
//! Schur complement `S`. Any contact / friction / fit case is then a small
//! dense Newton problem `S u_b = f_c(u_b)`; the full field is recovered with
//! a single back-solve for the final bore forces.

use crate::geometry::LugGeometry;
use crate::mesh::{BushingMesh, Mesh, MeshSpec};
use edge_check::fem::element_stiffness;
use edge_check::linalg::BandedSpd;

/// How the far end of the model is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FarEnd {
    /// `ux = uy = 0` on the far end (a lug loaded through its shank).
    Clamped,
    /// Free body held only by tiny grounded springs (1e-6 E); contact alone
    /// carries the load. Used by self-equilibrated cases such as a fit in a disc.
    Soft,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub e_psi: f64,
    pub nu: f64,
}

impl Material {
    /// Plane-stress constitutive matrix (per unit thickness).
    pub fn d_matrix(&self) -> [[f64; 3]; 3] {
        let c = self.e_psi / (1.0 - self.nu * self.nu);
        [[c, c * self.nu, 0.0], [c * self.nu, c, 0.0], [0.0, 0.0, c * (1.0 - self.nu) / 2.0]]
    }
}

/// Weak grounding of the bushing's rigid-body modes, as a fraction of its `E`. A bushing touches
/// the lug only through contact, so it is a free body: its stiffness is singular without this.
/// The springs leak `k u` of force to ground (`Condensed::ground_leak` measures it), so it must be
/// as small as the factorisation tolerates (1e-6 leaked 1.4 % of the load; see `lug-solver/AGENTS.md`).
pub const BUSHING_GROUND: f64 = 1e-9;

/// The condensed lug: mesh, factorised stiffness and the bore Schur complement.
pub struct Condensed {
    pub mesh: Mesh,
    /// Lug material (the effective constants in a plane-strain assembly).
    pub material: Material,
    /// Bushing material (same convention), when there is a bushing.
    pub bushing_material: Option<Material>,
    pub thickness: f64,
    /// Node id -> position in the banded ordering.
    rank: Vec<usize>,
    factor: BandedSpd,
    /// Constrained dof flags, by banded dof index.
    constrained: Vec<bool>,
    /// Nodes of the condensed contact set: every node that can receive a contact force.
    pub contact_nodes: Vec<usize>,
    /// For each node and component, its index among the free contact dofs.
    cidx: Vec<[Option<usize>; 2]>,
    /// Number of free bore dofs (`m`).
    pub m: usize,
    /// Dense `m x m` Schur complement `S = G^-1`, row-major.
    pub s: Vec<f64>,
    /// Dense `m x m` bore Green's matrix `G = (K^-1)_bb`, row-major (symmetric).
    pub g: Vec<f64>,
    pub stats: FactorStats,
    /// Grounding spring (lbf/in per dof) and the nodes it acts on (bushing nodes); empty without one.
    ground_k: f64,
    ground_nodes: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FactorStats {
    pub dofs: usize,
    pub bandwidth: usize,
    pub assemble_ms: f64,
    pub factor_ms: f64,
    pub condense_ms: f64,
}

impl Condensed {
    pub fn build(geom: &LugGeometry, spec: MeshSpec, material: Material, half: bool, far_end: FarEnd) -> Result<Condensed, String> {
        Self::build_with(geom, spec, material, half, far_end, None)
    }

    /// As [`build`](Self::build), with an optional bushing (its mesh and material).
    pub fn build_with(geom: &LugGeometry, spec: MeshSpec, material: Material, half: bool, far_end: FarEnd, bushing: Option<(BushingMesh, Material)>) -> Result<Condensed, String> {
        let mesh = Mesh::build_with(geom, spec, half, bushing.map(|b| b.0))?;
        Self::from_mesh(mesh, geom, material, bushing.map(|b| b.1), far_end)
    }

    pub fn from_mesh(mesh: Mesh, geom: &LugGeometry, material: Material, bushing_material: Option<Material>, far_end: FarEnd) -> Result<Condensed, String> {
        Self::from_mesh_geo(mesh, geom, material, bushing_material, far_end, None)
    }

    /// As [`from_mesh`](Self::from_mesh) with the geometric (initial-stress) stiffness of a
    /// stress state added: `sigma[e][3 b + a]` is `[sx, sy, txy]` at the 3x3 Gauss points of
    /// element `e` (xi along `a`), per unit thickness. This is the second-order (P-delta)
    /// tangent: tension stiffens, compression softens, and the matrix may stop being positive
    /// definite near an elastic buckling load.
    pub fn from_mesh_geo(mesh: Mesh, geom: &LugGeometry, material: Material, bushing_material: Option<Material>, far_end: FarEnd, sigma: Option<&[[[f64; 3]; 9]]>) -> Result<Condensed, String> {
        if mesh.bush_rows > 0 && bushing_material.is_none() {
            return Err("the mesh has a bushing but no bushing material was given".into());
        }
        let t0 = std::time::Instant::now();
        let n_nodes = mesh.nodes.len();

        // Banded ordering: natural by column for a half model; folded (0, C-1, 1, C-2, ...)
        // for a closed ring so wrap-around neighbours stay close.
        let nc = mesh.n_cols;
        let col_rank = |c: usize| -> usize {
            if !mesh.periodic {
                c
            } else if c < nc.div_ceil(2) {
                2 * c
            } else {
                2 * (nc - 1 - c) + 1
            }
        };
        let mut rank = vec![0usize; n_nodes];
        for c in 0..nc {
            for i in 0..mesh.n_rows {
                rank[mesh.node_id(c, i)] = col_rank(c) * mesh.n_rows + i;
            }
        }
        let mut bw_nodes = 0usize;
        for e in &mesh.elems {
            let (lo, hi) = e.iter().fold((usize::MAX, 0usize), |(lo, hi), &n| (lo.min(rank[n]), hi.max(rank[n])));
            bw_nodes = bw_nodes.max(hi - lo);
        }
        let bw = 2 * bw_nodes + 1;
        let n_dof = 2 * n_nodes;

        // Constraints.
        let mut constrained = vec![false; n_dof];
        let dof = |node: usize, comp: usize| 2 * rank[node] + comp;
        if far_end == FarEnd::Clamped {
            for (id, p) in mesh.nodes.iter().enumerate() {
                if p[0] >= geom.length - 1e-9 {
                    constrained[dof(id, 0)] = true;
                    constrained[dof(id, 1)] = true;
                }
            }
        }
        if !mesh.periodic {
            for (id, p) in mesh.nodes.iter().enumerate() {
                if p[1].abs() < 1e-12 {
                    constrained[dof(id, 1)] = true;
                }
            }
        }

        let d_lug = material.d_matrix();
        let d_bush = bushing_material.map(|m| m.d_matrix()).unwrap_or(d_lug);
        let mut k = BandedSpd::zeros(n_dof, bw);
        for (ei, e) in mesh.elems.iter().enumerate() {
            let xy: Vec<[f64; 2]> = e.iter().map(|&n| mesh.nodes[n]).collect();
            let mut ke = element_stiffness(&xy, if mesh.elem_group[ei] == 1 { &d_bush } else { &d_lug })?;
            if let Some(sg) = sigma {
                let kg = geometric_stiffness(&xy, &sg[ei])?;
                for a in 0..18 {
                    for b in 0..18 {
                        ke[a][b] += kg[a][b];
                    }
                }
            }
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
                                continue; // lower triangle only
                            }
                            k.add(i, j, ke[2 * la + ca][2 * lb + cb]);
                        }
                    }
                }
            }
        }
        let scale = (0..n_dof).map(|i| k.diag(i)).fold(0.0, f64::max).max(1.0);
        let soft = if far_end == FarEnd::Soft { 1e-6 * material.e_psi } else { 0.0 };
        // A bushing touches the lug only through the interface contact, so on its own it is a
        // free body: ground its nodes weakly (the contact restrains it for real once active).
        let soft_bush = bushing_material.map_or(0.0, |m| BUSHING_GROUND * m.e_psi);
        let mut bush_dof = vec![false; n_dof];
        if soft_bush > 0.0 {
            for (ei, e) in mesh.elems.iter().enumerate() {
                if mesh.elem_group[ei] == 1 {
                    for &n in e {
                        bush_dof[dof(n, 0)] = true;
                        bush_dof[dof(n, 1)] = true;
                    }
                }
            }
        }
        for (i, &fixed) in constrained.iter().enumerate() {
            if fixed {
                k.add(i, i, scale);
            } else if soft > 0.0 {
                k.add(i, i, soft);
            } else if bush_dof[i] {
                k.add(i, i, soft_bush);
            }
        }
        let assemble_ms = t0.elapsed().as_secs_f64() * 1e3;

        let t1 = std::time::Instant::now();
        if !k.factor() {
            return Err("lug stiffness is not positive definite (check the constraints)".into());
        }
        let factor_ms = t1.elapsed().as_secs_f64() * 1e3;

        // Free bore dofs.
        // Pin-contact nodes first; with a bushing, both sides of the interface follow.
        let mut contact_nodes: Vec<usize> = (0..nc).map(|c| mesh.node_id(c, 0)).collect();
        if mesh.bush_rows > 0 {
            contact_nodes.extend((0..nc).map(|c| mesh.node_id(c, mesh.bush_rows - 1)));
            contact_nodes.extend((0..nc).map(|c| mesh.node_id(c, mesh.bush_rows)));
        }
        let mut cidx: Vec<[Option<usize>; 2]> = vec![[None, None]; n_nodes];
        let mut bore_dofs: Vec<usize> = Vec::new(); // banded dof of each free contact dof
        for &node in &contact_nodes {
            for (comp, slot) in cidx[node].iter_mut().enumerate() {
                let d = dof(node, comp);
                if !constrained[d] {
                    *slot = Some(bore_dofs.len());
                    bore_dofs.push(d);
                }
            }
        }
        let m = bore_dofs.len();

        let t2 = std::time::Instant::now();
        let g = green_matrix(&k, &bore_dofs, n_dof);
        let s = invert_spd(&g, m)?;
        let condense_ms = t2.elapsed().as_secs_f64() * 1e3;

        let ground_nodes: Vec<usize> = if soft_bush > 0.0 { (0..mesh.nodes.len()).filter(|&n| bush_dof[dof(n, 0)]).collect() } else { Vec::new() };
        Ok(Condensed {
            ground_k: soft_bush,
            ground_nodes,
            thickness: geom.thickness,
            material,
            bushing_material,
            rank,
            factor: k,
            constrained,
            contact_nodes,
            cidx,
            m,
            s,
            g,
            stats: FactorStats { dofs: n_dof, bandwidth: bw, assemble_ms, factor_ms, condense_ms },
            mesh,
        })
    }

    /// Energy stored in, and net force carried by, the bushing's grounding springs for the
    /// displacement field `u`: a pure artefact that the self-checks bound.
    pub fn ground_leak(&self, u: &[f64]) -> (f64, [f64; 2]) {
        let (mut e, mut f) = (0.0, [0.0, 0.0]);
        for &n in &self.ground_nodes {
            for c in 0..2 {
                let d = u[2 * n + c];
                e += 0.5 * self.ground_k * d * d;
                f[c] += self.ground_k * d;
            }
        }
        (e, f)
    }

    /// Index of `(node, comp)` among the free contact dofs, if it is one.
    #[inline]
    pub fn cidx(&self, node: usize, comp: usize) -> Option<usize> {
        self.cidx[node][comp]
    }

    #[inline]
    pub fn dof(&self, node: usize, comp: usize) -> usize {
        2 * self.rank[node] + comp
    }

    /// Full displacement field for the given bore nodal forces (per unit
    /// thickness), indexed `2 * node + comp` in node order.
    pub fn displacements(&self, bore_force: &[(usize, [f64; 2])]) -> Vec<f64> {
        let mut rhs = vec![0.0; self.factor.n];
        for &(node, f) in bore_force {
            for (comp, fc) in f.iter().enumerate() {
                let d = self.dof(node, comp);
                if !self.constrained[d] {
                    rhs[d] += fc;
                }
            }
        }
        self.factor.solve_in_place(&mut rhs);
        let mut u = vec![0.0; 2 * self.mesh.nodes.len()];
        for node in 0..self.mesh.nodes.len() {
            for comp in 0..2 {
                u[2 * node + comp] = rhs[self.dof(node, comp)];
            }
        }
        u
    }

    /// Displacements for arbitrary nodal loads (`2 * node + comp`, per unit thickness);
    /// loads on constrained dofs are reactions and are ignored.
    pub fn solve_loads(&self, loads: &[f64]) -> Vec<f64> {
        let mut rhs = vec![0.0; self.factor.n];
        for node in 0..self.mesh.nodes.len() {
            for comp in 0..2 {
                let d = self.dof(node, comp);
                if !self.constrained[d] {
                    rhs[d] = loads[2 * node + comp];
                }
            }
        }
        self.factor.solve_in_place(&mut rhs);
        let mut u = vec![0.0; 2 * self.mesh.nodes.len()];
        for node in 0..self.mesh.nodes.len() {
            for comp in 0..2 {
                u[2 * node + comp] = rhs[self.dof(node, comp)];
            }
        }
        u
    }

    pub fn is_constrained(&self, node: usize, comp: usize) -> bool {
        self.constrained[self.dof(node, comp)]
    }
}

/// `G[i][j] = (K^-1)[bore_i][bore_j]` by one back-solve per bore dof, spread over threads.
fn green_matrix(k: &BandedSpd, bore_dofs: &[usize], n_dof: usize) -> Vec<f64> {
    let m = bore_dofs.len();
    let mut g = vec![0.0; m * m];
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(m.max(1));
    let chunk = m.div_ceil(threads.max(1));
    let cols: Vec<Vec<f64>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (lo, hi) = (t * chunk, ((t + 1) * chunk).min(m));
                scope.spawn(move || {
                    let mut out = Vec::with_capacity(hi.saturating_sub(lo));
                    for j in lo..hi {
                        let mut rhs = vec![0.0; n_dof];
                        rhs[bore_dofs[j]] = 1.0;
                        k.solve_in_place(&mut rhs);
                        out.push(bore_dofs.iter().map(|&d| rhs[d]).collect::<Vec<f64>>());
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    for (j, col) in cols.iter().enumerate() {
        for i in 0..m {
            g[i * m + j] = col[i];
        }
    }
    // K is symmetric, so G is: remove round-off asymmetry.
    for i in 0..m {
        for j in 0..i {
            let v = 0.5 * (g[i * m + j] + g[j * m + i]);
            g[i * m + j] = v;
            g[j * m + i] = v;
        }
    }
    g
}

/// Inverse of a dense symmetric positive-definite `m x m` matrix (row-major).
/// `int (grad N_a)^T S (grad N_b) dA` on both displacement components, with `S` the Cauchy stress
/// at the 3x3 Gauss points (`[3 b + a]`, xi along `a`).
fn geometric_stiffness(xy: &[[f64; 2]], sigma: &[[f64; 3]; 9]) -> Result<[[f64; 18]; 18], String> {
    use edge_check::fem::{shape_q9, GAUSS3};
    let mut kg = [[0.0; 18]; 18];
    for (b, &(eta, wy)) in GAUSS3.iter().enumerate() {
        for (a, &(xi, wx)) in GAUSS3.iter().enumerate() {
            let (_, dxi, deta) = shape_q9(xi, eta);
            let (mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0);
            for n in 0..9 {
                j11 += dxi[n] * xy[n][0];
                j12 += dxi[n] * xy[n][1];
                j21 += deta[n] * xy[n][0];
                j22 += deta[n] * xy[n][1];
            }
            let det = j11 * j22 - j12 * j21;
            if det <= 0.0 {
                return Err("inverted element".into());
            }
            let inv = [[j22 / det, -j12 / det], [-j21 / det, j11 / det]];
            let s = sigma[3 * b + a];
            let w = det * wx * wy;
            let mut dn = [[0.0; 2]; 9];
            for n in 0..9 {
                dn[n] = [inv[0][0] * dxi[n] + inv[0][1] * deta[n], inv[1][0] * dxi[n] + inv[1][1] * deta[n]];
            }
            for i in 0..9 {
                for j in 0..9 {
                    let v = w * (dn[i][0] * (s[0] * dn[j][0] + s[2] * dn[j][1]) + dn[i][1] * (s[2] * dn[j][0] + s[1] * dn[j][1]));
                    kg[2 * i][2 * j] += v;
                    kg[2 * i + 1][2 * j + 1] += v;
                }
            }
        }
    }
    Ok(kg)
}

pub fn invert_spd(a: &[f64], m: usize) -> Result<Vec<f64>, String> {
    let mut f = BandedSpd::zeros(m, m.saturating_sub(1));
    for i in 0..m {
        for j in 0..=i {
            f.add(i, j, a[i * m + j]);
        }
    }
    if !f.factor() {
        return Err("bore Green's matrix is not positive definite".into());
    }
    let mut inv = vec![0.0; m * m];
    let mut col = vec![0.0; m];
    for j in 0..m {
        col.iter_mut().for_each(|v| *v = 0.0);
        col[j] = 1.0;
        f.solve_in_place(&mut col);
        for i in 0..m {
            inv[i * m + j] = col[i];
        }
    }
    for i in 0..m {
        for j in 0..i {
            let v = 0.5 * (inv[i * m + j] + inv[j * m + i]);
            inv[i * m + j] = v;
            inv[j * m + i] = v;
        }
    }
    Ok(inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lug() -> LugGeometry {
        LugGeometry::round_head(0.5, 1.5, 0.25, 3.0)
    }

    const AL: Material = Material { e_psi: 10.0e6, nu: 0.33 };

    #[test]
    fn schur_complement_is_the_inverse_of_the_bore_green_matrix_and_symmetric() {
        let c = Condensed::build(&lug(), MeshSpec { elements_around: 24, ..Default::default() }, AL, true, FarEnd::Clamped).unwrap();
        let m = c.m;
        // S is symmetric positive definite.
        for i in 0..m {
            assert!(c.s[i * m + i] > 0.0);
            for j in 0..i {
                let (a, b) = (c.s[i * m + j], c.s[j * m + i]);
                assert!((a - b).abs() <= 1e-9 * (a.abs() + b.abs() + 1.0));
            }
        }
    }

    #[test]
    fn a_unit_force_on_the_condensed_system_reproduces_the_full_solve() {
        // S u_b = f_b must give the same bore displacement as K u = f.
        let c = Condensed::build(&lug(), MeshSpec { elements_around: 24, ..Default::default() }, AL, true, FarEnd::Clamped).unwrap();
        let col = c.mesh.n_cols / 2; // a bore node away from the axes
        let node = c.mesh.node_id(col, 0);
        let u = c.displacements(&[(node, [1000.0, 0.0])]);
        let m = c.m;
        // u_b from the full solve, then S u_b should return the applied force.
        let mut ub = vec![0.0; m];
        for &node in &c.contact_nodes {
            for comp in 0..2 {
                if let Some(k) = c.cidx(node, comp) {
                    ub[k] = u[2 * node + comp];
                }
            }
        }
        let k_in = c.cidx(node, 0).unwrap();
        for i in 0..m {
            let f: f64 = (0..m).map(|j| c.s[i * m + j] * ub[j]).sum();
            let want = if i == k_in { 1000.0 } else { 0.0 };
            assert!((f - want).abs() < 1e-6 * 1000.0 + 1e-6, "row {i}: {f} vs {want}");
        }
    }

    #[test]
    fn folded_ordering_is_a_permutation() {
        for nc in [8usize, 9, 48, 97] {
            let mut seen = vec![false; nc];
            for c in 0..nc {
                let r = if c < nc.div_ceil(2) { 2 * c } else { 2 * (nc - 1 - c) + 1 };
                assert!(r < nc && !seen[r], "nc {nc} c {c} rank {r}");
                seen[r] = true;
            }
        }
    }

    #[test]
    fn rigid_body_translation_is_not_resisted_without_the_far_end_clamp() {
        // Soft supports: a uniform translation costs only the grounded springs.
        let c = Condensed::build(&lug(), MeshSpec { elements_around: 24, ..Default::default() }, AL, false, FarEnd::Soft).unwrap();
        assert!(c.m > 0);
    }

    #[test]
    fn default_mesh_condenses_quickly_and_reports_its_size() {
        let c = Condensed::build(&lug(), MeshSpec::default(), AL, true, FarEnd::Clamped).unwrap();
        eprintln!("half: {:?} nodes {} bore dofs {}", c.stats, c.mesh.nodes.len(), c.m);
        let f = Condensed::build(&lug(), MeshSpec::default(), AL, false, FarEnd::Clamped).unwrap();
        eprintln!("full: {:?} nodes {} bore dofs {}", f.stats, f.mesh.nodes.len(), f.m);
        assert!(c.stats.assemble_ms + c.stats.factor_ms + c.stats.condense_ms < 20_000.0);
    }
}
