//! 1-D Euler-Bernoulli beam and axial bar solvers on a node list - the member-force engine of
//! `line_run.rs`. Hermite-cubic beam elements and linear bar elements give the *exact* nodal values for
//! point loads at nodes and element-wise uniform loads (the 1-D bar and beam FE are nodally exact for
//! these loads), and the in-element moment, shear and axial force follow from statics, so they are exact too.
//!
//! Sign convention (per bending plane `c`): loads and deflection `w` are positive along `+c`,
//! `M = EI w''`, `V = dM/dx = EI w'''`, so `V' = q` and `M(s) = M_L + V_L s + q s^2 / 2`. A moment load
//! is positive when it increases the slope `w'`. Axial: tension positive, `N' = -q`.
//!
//! Linear algebra: symmetric positive-definite banded Cholesky on the free DOFs (half bandwidth 3 for
//! the beam, 1 for the bar). A pivot below `1e-10` of its diagonal means a mechanism (not enough
//! supports) and is refused, never solved through.

/// Lower-band symmetric matrix: row `i` stores `A(i, i-k)` for `k = 0..=bw`.
struct Band {
    n: usize,
    bw: usize,
    a: Vec<f64>,
}

impl Band {
    fn new(n: usize, bw: usize) -> Self {
        Self { n, bw, a: vec![0.0; n * (bw + 1)] }
    }
    fn idx(&self, i: usize, j: usize) -> usize {
        i * (self.bw + 1) + (i - j)
    }
    fn add(&mut self, i: usize, j: usize, v: f64) {
        let (i, j) = if i >= j { (i, j) } else { (j, i) };
        debug_assert!(i - j <= self.bw);
        let k = self.idx(i, j);
        self.a[k] += v;
    }
    fn get(&self, i: usize, j: usize) -> f64 {
        self.a[self.idx(i, j)]
    }
    /// `A x`.
    fn mul(&self, x: &[f64]) -> Vec<f64> {
        let mut y = vec![0.0; self.n];
        for i in 0..self.n {
            for j in i.saturating_sub(self.bw)..=i {
                let v = self.get(i, j);
                y[i] += v * x[j];
                if j != i {
                    y[j] += v * x[i];
                }
            }
        }
        y
    }
    /// Solves `A x = b`; `None` when a pivot is not safely positive (a mechanism).
    fn solve(&self, b: &[f64]) -> Option<Vec<f64>> {
        let (n, bw) = (self.n, self.bw);
        let mut l = Band { n, bw, a: self.a.clone() };
        for i in 0..n {
            for j in i.saturating_sub(bw)..=i {
                let mut s = self.get(i, j);
                for k in i.saturating_sub(bw)..j {
                    s -= l.get(i, k) * l.get(j, k);
                }
                if i == j {
                    if s.is_nan() || s <= 1e-10 * self.get(i, i) {
                        return None;
                    }
                    let k = l.idx(i, i);
                    l.a[k] = s.sqrt();
                } else {
                    let k = l.idx(i, j);
                    l.a[k] = s / l.get(j, j);
                }
            }
        }
        let mut y = vec![0.0; n];
        for i in 0..n {
            let mut s = b[i];
            for k in i.saturating_sub(bw)..i {
                s -= l.get(i, k) * y[k];
            }
            y[i] = s / l.get(i, i);
        }
        let mut x = vec![0.0; n];
        for i in (0..n).rev() {
            let mut s = y[i];
            for k in i + 1..(i + bw + 1).min(n) {
                s -= l.get(k, i) * x[k];
            }
            x[i] = s / l.get(i, i);
        }
        Some(x)
    }
}

/// Maps global DOFs to reduced (free) indices.
fn reduce(fixed: &[bool]) -> (Vec<Option<usize>>, usize) {
    let mut map = vec![None; fixed.len()];
    let mut n = 0;
    for (i, f) in fixed.iter().enumerate() {
        if !f {
            map[i] = Some(n);
            n += 1;
        }
    }
    (map, n)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NodeBc {
    pub fix_w: bool,
    pub fix_theta: bool,
    /// Rotational spring to ground (moment per radian) on a free slope; ignored where `fix_theta` holds it.
    pub k_theta: f64,
}

/// Actions at the left end of one beam element and its load.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BeamElem {
    pub l: f64,
    pub q: f64,
    pub m_left: f64,
    pub v_left: f64,
}
impl BeamElem {
    pub fn moment(&self, s: f64) -> f64 {
        self.m_left + self.v_left * s + 0.5 * self.q * s * s
    }
    pub fn shear(&self, s: f64) -> f64 {
        self.v_left + self.q * s
    }
}

pub(crate) struct BeamSolution {
    pub elems: Vec<BeamElem>,
    /// Support force on the structure (`+c`) per node (0 where `w` is free).
    pub reaction_w: Vec<f64>,
    /// Support moment (conjugate to the slope) per node.
    pub reaction_m: Vec<f64>,
    /// Backward error of the solve on the free DOFs.
    pub residual: f64,
}

/// Solves one bending plane. `q[e]` is the uniform load of element `e`, `p`/`m` the nodal force and
/// slope-conjugate moment.
pub(crate) fn solve_beam(x: &[f64], ei: f64, bc: &[NodeBc], q: &[f64], p: &[f64], m: &[f64]) -> Result<BeamSolution, String> {
    let nn = x.len();
    let ne = nn - 1;
    let mut fixed = vec![false; 2 * nn];
    for (i, b) in bc.iter().enumerate() {
        fixed[2 * i] = b.fix_w;
        fixed[2 * i + 1] = b.fix_theta;
    }
    let (map, nf) = reduce(&fixed);
    let mut k = Band::new(nf, 3);
    let mut f = vec![0.0; 2 * nn];
    for i in 0..nn {
        f[2 * i] += p[i];
        f[2 * i + 1] += m[i];
    }
    let ke = |l: f64| {
        let c = ei / l.powi(3);
        [
            [12.0 * c, 6.0 * l * c, -12.0 * c, 6.0 * l * c],
            [6.0 * l * c, 4.0 * l * l * c, -6.0 * l * c, 2.0 * l * l * c],
            [-12.0 * c, -6.0 * l * c, 12.0 * c, -6.0 * l * c],
            [6.0 * l * c, 2.0 * l * l * c, -6.0 * l * c, 4.0 * l * l * c],
        ]
    };
    for (i, b) in bc.iter().enumerate() {
        if let (false, Some(r)) = (b.fix_theta, map[2 * i + 1]) {
            if b.k_theta > 0.0 {
                k.add(r, r, b.k_theta);
            }
        }
    }
    let feq = |l: f64, q: f64| [q * l / 2.0, q * l * l / 12.0, q * l / 2.0, -q * l * l / 12.0];
    for e in 0..ne {
        let l = x[e + 1] - x[e];
        let ke = ke(l);
        let fe = feq(l, q[e]);
        let dofs = [2 * e, 2 * e + 1, 2 * e + 2, 2 * e + 3];
        for a in 0..4 {
            f[dofs[a]] += fe[a];
            let Some(ra) = map[dofs[a]] else { continue };
            for b in 0..4 {
                if let Some(rb) = map[dofs[b]] {
                    if ra >= rb {
                        k.add(ra, rb, ke[a][b]);
                    }
                }
            }
        }
    }
    let mut rhs = vec![0.0; nf];
    for (g, r) in map.iter().enumerate() {
        if let Some(r) = r {
            rhs[*r] = f[g];
        }
    }
    let sol = k.solve(&rhs).ok_or_else(|| "beam is not stably supported (mechanism): add supports or a rotation restraint".to_string())?;
    let mut d = vec![0.0; 2 * nn];
    for (g, r) in map.iter().enumerate() {
        if let Some(r) = r {
            d[g] = sol[*r];
        }
    }
    let kd = k.mul(&sol);
    let scale = rhs.iter().fold(0.0_f64, |a, v| a.max(v.abs())).max(1e-300);
    let residual = kd.iter().zip(&rhs).fold(0.0_f64, |a, (u, v)| a.max((u - v).abs())) / scale;
    // Element end actions and reactions.
    let mut elems = Vec::with_capacity(ne);
    let mut internal = vec![0.0; 2 * nn];
    for e in 0..ne {
        let l = x[e + 1] - x[e];
        let (ke, fe) = (ke(l), feq(l, q[e]));
        let de = [d[2 * e], d[2 * e + 1], d[2 * e + 2], d[2 * e + 3]];
        let mut fn_ = [0.0; 4];
        for a in 0..4 {
            fn_[a] = (0..4).map(|b| ke[a][b] * de[b]).sum::<f64>() - fe[a];
            internal[2 * e + a] += fn_[a];
        }
        elems.push(BeamElem { l, q: q[e], m_left: -fn_[1], v_left: fn_[0] });
    }
    let mut reaction_w = vec![0.0; nn];
    let mut reaction_m = vec![0.0; nn];
    for i in 0..nn {
        if bc[i].fix_w {
            reaction_w[i] = internal[2 * i] - p[i];
        }
        if bc[i].fix_theta || bc[i].k_theta > 0.0 {
            reaction_m[i] = internal[2 * i + 1] - m[i];
        }
    }
    Ok(BeamSolution { elems, reaction_w, reaction_m, residual })
}

pub(crate) struct BarSolution {
    /// Axial force at the left end of each element; `N(s) = n_left - q s`.
    pub n_left: Vec<f64>,
    pub reaction: Vec<f64>,
    pub residual: f64,
}

/// Solves the axial bar: `eps_th = alpha * dT` free thermal strain, `q[e]` uniform axial load, `p[i]` nodal force.
pub(crate) fn solve_bar(x: &[f64], ea: f64, fix_u: &[bool], eps_th: f64, q: &[f64], p: &[f64]) -> Result<BarSolution, String> {
    let nn = x.len();
    let (map, nf) = reduce(fix_u);
    let mut k = Band::new(nf, 1);
    let mut f = p.to_vec();
    for e in 0..nn - 1 {
        let l = x[e + 1] - x[e];
        let c = ea / l;
        let fe = [-ea * eps_th + q[e] * l / 2.0, ea * eps_th + q[e] * l / 2.0];
        let ke = [[c, -c], [-c, c]];
        for a in 0..2 {
            f[e + a] += fe[a];
            let Some(ra) = map[e + a] else { continue };
            for b in 0..2 {
                if let Some(rb) = map[e + b] {
                    if ra >= rb {
                        k.add(ra, rb, ke[a][b]);
                    }
                }
            }
        }
    }
    let mut rhs = vec![0.0; nf];
    for (g, r) in map.iter().enumerate() {
        if let Some(r) = r {
            rhs[*r] = f[g];
        }
    }
    let sol = k.solve(&rhs).ok_or_else(|| "line has no axial anchor (rigid-body axial motion)".to_string())?;
    let mut u = vec![0.0; nn];
    for (g, r) in map.iter().enumerate() {
        if let Some(r) = r {
            u[g] = sol[*r];
        }
    }
    let kd = k.mul(&sol);
    let scale = rhs.iter().fold(0.0_f64, |a, v| a.max(v.abs())).max(1e-300);
    let residual = kd.iter().zip(&rhs).fold(0.0_f64, |a, (s, v)| a.max((s - v).abs())) / scale;
    let mut n_left = Vec::with_capacity(nn - 1);
    let mut internal = vec![0.0; nn];
    for e in 0..nn - 1 {
        let l = x[e + 1] - x[e];
        let n_avg = ea * ((u[e + 1] - u[e]) / l - eps_th);
        let nl = n_avg + q[e] * l / 2.0;
        n_left.push(nl);
        internal[e] += -nl;
        internal[e + 1] += n_avg - q[e] * l / 2.0;
    }
    let reaction = (0..nn).map(|i| if fix_u[i] { internal[i] - p[i] } else { 0.0 }).collect();
    Ok(BarSolution { n_left, reaction, residual })
}
