//! Linear multi-point constraints (`u_slave = sum c_j u_j + g`) and reference-point rigid
//! coupling, solved by elimination: `u = T u_r + u_g`, `K_r = T^T K T`, `f_r = T^T (f - K u_g)`.
//!
//! Extra degrees of freedom (indices `>= mesh.n_dofs()`) carry the translations and rotations of
//! reference points; they appear only as masters of rigid couplings and have no stiffness of
//! their own. Rotations are small (linearised): `u_s = u_ref + theta x (x_s - x_ref)`.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use crate::analysis::Model;
use crate::linear::{solve_csc_lower, Dirichlet, SolveError};
use crate::loads::{self, Loads};
use crate::mesh::Mesh;
use std::collections::HashMap;

/// One constraint equation `u[slave] = sum coef * u[dof] + rhs`.
#[derive(Debug, Clone, PartialEq)]
pub struct Equation {
    pub slave: usize,
    pub terms: Vec<(usize, f64)>,
    pub rhs: f64,
}

/// A set of constraint equations plus the extra (reference-point) degrees of freedom they use.
#[derive(Debug, Clone, Default)]
pub struct Constraints {
    /// Number of extra dofs after the node dofs.
    pub n_extra: usize,
    pub equations: Vec<Equation>,
}

/// A rigid-body reference point: its extra dofs are `first .. first + n` (translations `d`, then
/// rotations: 1 in 2D, 3 in 3D).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefPoint {
    pub first: usize,
    pub d: usize,
    pub x: [f64; 3],
}

impl RefPoint {
    /// Global dof of translation component `c`.
    pub fn trans(&self, c: usize) -> usize {
        self.first + c
    }

    /// Global dof of rotation component `c` (2D: `c = 0` is the rotation about `z`).
    pub fn rot(&self, c: usize) -> usize {
        self.first + self.d + c
    }
}

impl Constraints {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `u[slave] = sum terms + rhs`.
    pub fn add_equation(&mut self, slave: usize, terms: Vec<(usize, f64)>, rhs: f64) {
        self.equations.push(Equation { slave, terms, rhs });
    }

    /// Tie component `comp` of node `slave` to node `master` (equal displacement).
    pub fn tie(&mut self, mesh: &Mesh, slave: usize, master: usize, comp: usize) {
        let d = mesh.dim();
        self.add_equation(slave * d + comp, vec![(master * d + comp, 1.0)], 0.0);
    }

    /// New reference point at `x` with its translation and rotation dofs.
    pub fn add_ref_point(&mut self, mesh: &Mesh, x: [f64; 3]) -> RefPoint {
        let d = mesh.dim();
        let rp = RefPoint { first: mesh.n_dofs() + self.n_extra, d, x };
        self.n_extra += d + if d == 2 { 1 } else { 3 };
        rp
    }

    /// Rigidly couple the nodes to the reference point (RBE2-like): `u = u_ref + theta x r`.
    pub fn rigid(&mut self, mesh: &Mesh, rp: &RefPoint, nodes: &[usize]) {
        let d = mesh.dim();
        for &n in nodes {
            let r: [f64; 3] = std::array::from_fn(|i| mesh.nodes[n][i] - rp.x[i]);
            if d == 2 {
                self.add_equation(n * 2, vec![(rp.trans(0), 1.0), (rp.rot(0), -r[1])], 0.0);
                self.add_equation(n * 2 + 1, vec![(rp.trans(1), 1.0), (rp.rot(0), r[0])], 0.0);
            } else {
                // theta x r = (ty rz - tz ry, tz rx - tx rz, tx ry - ty rx)
                self.add_equation(n * 3, vec![(rp.trans(0), 1.0), (rp.rot(1), r[2]), (rp.rot(2), -r[1])], 0.0);
                self.add_equation(n * 3 + 1, vec![(rp.trans(1), 1.0), (rp.rot(2), r[0]), (rp.rot(0), -r[2])], 0.0);
                self.add_equation(n * 3 + 2, vec![(rp.trans(2), 1.0), (rp.rot(0), r[1]), (rp.rot(1), -r[0])], 0.0);
            }
        }
    }
}

/// Result of a constrained solve.
#[derive(Debug, Clone)]
pub struct ConstrainedSolution {
    /// Node displacements `node * d + comp`.
    pub u: Vec<f64>,
    /// Extra (reference-point) dof values.
    pub extra: Vec<f64>,
    /// `K u - f` at Dirichlet-fixed dofs (zero elsewhere).
    pub reactions: Vec<f64>,
    pub n_independent: usize,
}

/// Slave expressed through independent dofs (reduced indices) plus a constant.
struct Resolved {
    terms: Vec<(usize, f64)>,
    g: f64,
}

const NONE: u32 = u32::MAX;

/// Memoised substitution of slave-of-slave chains down to independent dofs.
struct Resolver<'a> {
    cons: &'a Constraints,
    slave_eq: &'a HashMap<usize, usize>,
    ind_of: &'a [u32],
    bc: &'a Dirichlet,
    nd: usize,
    memo: HashMap<usize, Resolved>,
    stack: Vec<usize>,
}

impl Resolver<'_> {
    fn resolve(&mut self, i: usize) -> Result<(), String> {
        if self.memo.contains_key(&i) {
            return Ok(());
        }
        if self.stack.contains(&i) {
            return Err(format!("circular constraint chain through dof {i}"));
        }
        self.stack.push(i);
        let eq = &self.cons.equations[self.slave_eq[&i]];
        let mut acc: HashMap<usize, f64> = HashMap::new();
        let mut g = eq.rhs;
        for &(m, c) in &eq.terms {
            if m < self.nd && self.bc.fixed[m] {
                g += c * self.bc.value[m];
            } else if self.slave_eq.contains_key(&m) {
                self.resolve(m)?;
                let r = &self.memo[&m];
                g += c * r.g;
                for &(j, cj) in &r.terms {
                    *acc.entry(j).or_insert(0.0) += c * cj;
                }
            } else {
                *acc.entry(self.ind_of[m] as usize).or_insert(0.0) += c;
            }
        }
        self.stack.pop();
        let mut terms: Vec<(usize, f64)> = acc.into_iter().filter(|(_, c)| *c != 0.0).collect();
        terms.sort_by_key(|t| t.0);
        self.memo.insert(i, Resolved { terms, g });
        Ok(())
    }
}

impl Model {
    /// Linear static solve with Dirichlet constraints, multi-point constraints and loads on the
    /// extra dofs `(extra dof index (>= n_dofs), force)`.
    pub fn solve_static_constrained(&self, loads: &Loads, bc: &Dirichlet, cons: &Constraints, extra_loads: &[(usize, f64)]) -> Result<ConstrainedSolution, String> {
        let nd = self.mesh.n_dofs();
        let n = nd + cons.n_extra;
        // ---- classify dofs
        let mut slave_eq: HashMap<usize, usize> = HashMap::new();
        for (k, eq) in cons.equations.iter().enumerate() {
            if eq.slave >= nd {
                return Err("an extra dof cannot be a slave".into());
            }
            if eq.slave < nd && bc.fixed[eq.slave] {
                return Err(format!("dof {} is both prescribed and a slave", eq.slave));
            }
            if slave_eq.insert(eq.slave, k).is_some() {
                return Err(format!("dof {} is the slave of two equations", eq.slave));
            }
            if eq.terms.iter().any(|&(m, _)| m >= n) {
                return Err("constraint refers to a dof that does not exist".into());
            }
        }
        let is_fixed = |i: usize| i < nd && bc.fixed[i];
        let mut ind_of = vec![NONE; n];
        let mut ind_dofs: Vec<usize> = Vec::new();
        for i in 0..n {
            if !is_fixed(i) && !slave_eq.contains_key(&i) {
                ind_of[i] = ind_dofs.len() as u32;
                ind_dofs.push(i);
            }
        }
        // ---- resolve slave chains by memoised substitution
        let mut resolver = Resolver { cons, slave_eq: &slave_eq, ind_of: &ind_of, bc, nd, memo: HashMap::new(), stack: Vec::new() };
        for &s in slave_eq.keys() {
            resolver.resolve(s)?;
        }
        let resolved = resolver.memo;
        let m = ind_dofs.len();
        if m == 0 {
            return Err(SolveError::NothingToSolve.to_string());
        }
        // rev[a]: slaves depending on reduced dof a, with their coefficient.
        let mut rev: Vec<Vec<(usize, f64)>> = vec![Vec::new(); m];
        for (&s, r) in &resolved {
            for &(j, c) in &r.terms {
                rev[j].push((s, c));
            }
        }
        for v in rev.iter_mut() {
            v.sort_by_key(|t| t.0);
        }
        // ---- full symmetric K by dof column
        let k = self.assemble()?;
        let pat = &self.pattern;
        let d = self.mesh.dim();
        let mut kcol: Vec<Vec<(u32, f64)>> = vec![Vec::new(); nd];
        for j in 0..pat.n_nodes {
            for blk in pat.col_ptr[j]..pat.col_ptr[j + 1] {
                let i = pat.row_idx[blk] as usize;
                let b = &k.vals[blk * d * d..(blk + 1) * d * d];
                for p in 0..d {
                    for q in 0..d {
                        let v = b[p * d + q];
                        if v == 0.0 {
                            continue;
                        }
                        kcol[j * d + q].push(((i * d + p) as u32, v));
                        if i != j {
                            kcol[i * d + p].push(((j * d + q) as u32, v));
                        }
                    }
                }
            }
        }
        // ---- load and prescribed-value vectors over all dofs
        let mut f = loads::assemble(&self.mesh, loads)?;
        f.resize(n, 0.0);
        for &(dof, v) in extra_loads {
            if dof < nd || dof >= n {
                return Err(format!("extra load on dof {dof}, which is not an extra dof"));
            }
            f[dof] += v;
        }
        let mut ug = vec![0.0; n];
        for i in 0..nd {
            if bc.fixed[i] {
                ug[i] = bc.value[i];
            }
        }
        for (&s, r) in &resolved {
            ug[s] = r.g;
        }
        let mut res = f.clone();
        for j in 0..nd {
            if ug[j] != 0.0 {
                for &(i, v) in &kcol[j] {
                    res[i as usize] -= v * ug[j];
                }
            }
        }
        let mut fr = vec![0.0; m];
        for (a, &dof) in ind_dofs.iter().enumerate() {
            fr[a] = res[dof];
            for &(s, c) in &rev[a] {
                fr[a] += c * res[s];
            }
        }
        // ---- reduced matrix, lower triangle by column
        let mut col_ptr = vec![0u32];
        let mut row_idx: Vec<u32> = Vec::new();
        let mut vals: Vec<f64> = Vec::new();
        let mut acc: HashMap<usize, f64> = HashMap::new();
        let mut out: Vec<(usize, f64)> = Vec::new();
        let mut v: HashMap<usize, f64> = HashMap::new();
        for (b, &bdof) in ind_dofs.iter().enumerate() {
            v.clear();
            let add_col = |dof: usize, c: f64, v: &mut HashMap<usize, f64>| {
                if dof < nd {
                    for &(i, val) in &kcol[dof] {
                        *v.entry(i as usize).or_insert(0.0) += c * val;
                    }
                }
            };
            add_col(bdof, 1.0, &mut v);
            for &(s, c) in &rev[b] {
                add_col(s, c, &mut v);
            }
            acc.clear();
            for (&i, &val) in &v {
                if is_fixed(i) {
                    continue;
                }
                if let Some(r) = resolved.get(&i) {
                    for &(j, c) in &r.terms {
                        *acc.entry(j).or_insert(0.0) += c * val;
                    }
                } else {
                    *acc.entry(ind_of[i] as usize).or_insert(0.0) += val;
                }
            }
            out.clear();
            out.extend(acc.iter().filter(|(&a, _)| a >= b).map(|(&a, &x)| (a, x)));
            if !out.iter().any(|t| t.0 == b) {
                out.push((b, 0.0));
            }
            out.sort_by_key(|t| t.0);
            for &(a, x) in &out {
                row_idx.push(a as u32);
                vals.push(x);
            }
            col_ptr.push(row_idx.len() as u32);
        }
        let mut ur = fr;
        solve_csc_lower(m, &col_ptr, &row_idx, &vals, &mut ur).map_err(|e| e.to_string())?;
        // ---- expand
        let mut u_full = ug;
        for (a, &dof) in ind_dofs.iter().enumerate() {
            u_full[dof] = ur[a];
        }
        for (&s, r) in &resolved {
            u_full[s] = r.g + r.terms.iter().map(|&(j, c)| c * ur[j]).sum::<f64>();
        }
        let mut ku = vec![0.0; nd];
        for j in 0..nd {
            if u_full[j] != 0.0 {
                for &(i, val) in &kcol[j] {
                    ku[i as usize] += val * u_full[j];
                }
            }
        }
        let mut reactions = vec![0.0; nd];
        for i in 0..nd {
            if bc.fixed[i] {
                reactions[i] = ku[i] - f[i];
            }
        }
        Ok(ConstrainedSolution { u: u_full[..nd].to_vec(), extra: u_full[nd..].to_vec(), reactions, n_independent: m })
    }
}
