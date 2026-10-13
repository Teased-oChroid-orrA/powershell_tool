//! Constraint solver: Levenberg-Marquardt on the constraint residuals (forward-difference Jacobian; sketches
//! are tens of variables, so dense algebra is ample), with degrees-of-freedom and conflict reporting.
use super::*;

/// A constraint whose residual (model units, or radians for an angle) is at least this is reported as violated. The
/// iteration itself runs to numerical precision (it stops when no step lowers the cost, or after 200 iterations).
pub const VIOLATION_TOL: f64 = 1e-7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    UnderConstrained,
    FullyConstrained,
    /// Redundant constraints that agree (rank-deficient but satisfied).
    OverConstrained,
    /// No solution satisfies every constraint.
    Conflicting,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub status: Status,
    /// Remaining degrees of freedom (variables minus rank of the constraint Jacobian).
    pub dof: usize,
    pub max_residual: f64,
    /// Constraints whose residual is still above tolerance (empty unless `Conflicting`).
    pub violated: Vec<Id>,
    pub iterations: usize,
}

struct Layout {
    /// Variable index of point `id` (x; y is +1) and of circle `id` (radius).
    pt: Vec<(Id, usize)>,
    rad: Vec<(Id, usize)>,
    n: usize,
}

impl Layout {
    fn new(s: &Sketch) -> Self {
        let (mut pt, mut rad, mut n) = (Vec::new(), Vec::new(), 0);
        for e in &s.entities {
            match e.v {
                Entity::Point { .. } => {
                    pt.push((e.id, n));
                    n += 2;
                }
                Entity::Circle { .. } => {
                    rad.push((e.id, n));
                    n += 1;
                }
                _ => {}
            }
        }
        Self { pt, rad, n }
    }
    fn p(&self, x: &[f64], id: Id) -> [f64; 2] {
        let i = self.pt.iter().find(|p| p.0 == id).map(|p| p.1).expect("point in layout");
        [x[i], x[i + 1]]
    }
    /// Centre and radius of a circle or arc at `x`.
    fn round(&self, s: &Sketch, x: &[f64], id: Id) -> ([f64; 2], f64) {
        match s.entity(id) {
            Some(Entity::Circle { c, .. }) => (self.p(x, *c), x[self.rad.iter().find(|r| r.0 == id).expect("circle in layout").1]),
            Some(Entity::Arc { c, s: st, .. }) => {
                let (c, st) = (self.p(x, *c), self.p(x, *st));
                (c, (st[0] - c[0]).hypot(st[1] - c[1]))
            }
            _ => unreachable!("validated constraint"),
        }
    }
    fn line(&self, s: &Sketch, x: &[f64], id: Id) -> ([f64; 2], [f64; 2]) {
        match s.entity(id) {
            Some(Entity::Line { a, b }) => (self.p(x, *a), self.p(x, *b)),
            _ => unreachable!("validated constraint"),
        }
    }
    fn start(&self, s: &Sketch) -> Vec<f64> {
        let mut x = vec![0.0; self.n];
        for &(id, i) in &self.pt {
            let p = s.point(id).unwrap_or([0.0; 2]);
            x[i] = p[0];
            x[i + 1] = p[1];
        }
        for &(id, i) in &self.rad {
            x[i] = s.round(id).map_or(1.0, |r| r.1);
        }
        x
    }
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn norm(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}
fn unit_guard(v: f64) -> f64 {
    if v.abs() < 1e-12 { 1e-12 } else { v }
}
fn wrap(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    let a = a.rem_euclid(t);
    if a > std::f64::consts::PI { a - t } else { a }
}

/// Residuals of one constraint, appended to `out`.
fn residual(s: &Sketch, l: &Layout, x: &[f64], c: &Constraint, out: &mut Vec<f64>) {
    match *c {
        Constraint::Coincident { a, b } => {
            let d = sub(l.p(x, a), l.p(x, b));
            out.extend(d);
        }
        Constraint::Horizontal { line } => {
            let (a, b) = l.line(s, x, line);
            out.push(a[1] - b[1]);
        }
        Constraint::Vertical { line } => {
            let (a, b) = l.line(s, x, line);
            out.push(a[0] - b[0]);
        }
        Constraint::Parallel { a, b } | Constraint::Perpendicular { a, b } => {
            let ((a0, a1), (b0, b1)) = (l.line(s, x, a), l.line(s, x, b));
            let (d1, d2) = (sub(a1, a0), sub(b1, b0));
            let k = unit_guard(norm(d1) * norm(d2));
            out.push(if matches!(c, Constraint::Parallel { .. }) { cross(d1, d2) / k } else { dot(d1, d2) / k });
        }
        Constraint::Equal { a, b } => {
            if matches!(s.entity(a), Some(Entity::Line { .. })) {
                let ((a0, a1), (b0, b1)) = (l.line(s, x, a), l.line(s, x, b));
                out.push(norm(sub(a1, a0)) - norm(sub(b1, b0)));
            } else {
                out.push(l.round(s, x, a).1 - l.round(s, x, b).1);
            }
        }
        Constraint::Tangent { a, b } => {
            let is_line = |i| matches!(s.entity(i), Some(Entity::Line { .. }));
            if is_line(a) || is_line(b) {
                let (ln, rd) = if is_line(a) { (a, b) } else { (b, a) };
                let (p0, p1) = l.line(s, x, ln);
                let (c, r) = l.round(s, x, rd);
                let d = sub(p1, p0);
                out.push(cross(d, sub(c, p0)).abs() / unit_guard(norm(d)) - r);
            } else {
                let ((c1, r1), (c2, r2)) = (l.round(s, x, a), l.round(s, x, b));
                let d = norm(sub(c1, c2));
                let (ext, int) = (d - (r1 + r2), d - (r1 - r2).abs());
                out.push(if ext.abs() <= int.abs() { ext } else { int });
            }
        }
        Constraint::PointOn { p, on } => {
            let q = l.p(x, p);
            if matches!(s.entity(on), Some(Entity::Line { .. })) {
                let (p0, p1) = l.line(s, x, on);
                let d = sub(p1, p0);
                out.push(cross(d, sub(q, p0)) / unit_guard(norm(d)));
            } else {
                let (c, r) = l.round(s, x, on);
                out.push(norm(sub(q, c)) - r);
            }
        }
        Constraint::Midpoint { p, line } => {
            let (a, b) = l.line(s, x, line);
            let q = l.p(x, p);
            out.push(q[0] - 0.5 * (a[0] + b[0]));
            out.push(q[1] - 0.5 * (a[1] + b[1]));
        }
        Constraint::Fixed { p, x: fx, y: fy } => {
            let q = l.p(x, p);
            out.push(q[0] - fx);
            out.push(q[1] - fy);
        }
        Constraint::Length { line, d } => {
            let (a, b) = l.line(s, x, line);
            out.push(norm(sub(b, a)) - d);
        }
        Constraint::Distance { a, b, d } => out.push(norm(sub(l.p(x, b), l.p(x, a))) - d),
        Constraint::HDist { a, b, d } => out.push(l.p(x, b)[0] - l.p(x, a)[0] - d),
        Constraint::VDist { a, b, d } => out.push(l.p(x, b)[1] - l.p(x, a)[1] - d),
        Constraint::Radius { e, d } => out.push(l.round(s, x, e).1 - d),
        Constraint::Angle { a, b, deg } => {
            let ((a0, a1), (b0, b1)) = (l.line(s, x, a), l.line(s, x, b));
            let (d1, d2) = (sub(a1, a0), sub(b1, b0));
            out.push(wrap(cross(d1, d2).atan2(dot(d1, d2)) - deg.to_radians()));
        }
    }
}

/// All residuals: constraints, then the implicit arc condition `|cs| = |ce|`, then optional drag targets
/// (weak pulls so the least-surprising solution is chosen among those the constraints allow).
fn residuals(s: &Sketch, l: &Layout, x: &[f64], pulls: &[(Id, [f64; 2])]) -> Vec<f64> {
    let mut r = Vec::new();
    for c in &s.constraints {
        residual(s, l, x, &c.v, &mut r);
    }
    for e in &s.entities {
        if let Entity::Arc { c, s: st, e: en } = e.v {
            let c = l.p(x, c);
            r.push(norm(sub(l.p(x, st), c)) - norm(sub(l.p(x, en), c)));
        }
    }
    for &(id, t) in pulls {
        let q = l.p(x, id);
        r.push(PULL * (q[0] - t[0]));
        r.push(PULL * (q[1] - t[1]));
    }
    r
}

const PULL: f64 = 1e-3;

/// Solve `A d = b` (A symmetric positive definite after damping) by Gaussian elimination with partial pivoting.
fn lin_solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for k in 0..n {
        let p = (k..n).max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))?;
        if a[p][k].abs() < 1e-300 {
            return None;
        }
        a.swap(k, p);
        b.swap(k, p);
        for i in k + 1..n {
            let f = a[i][k] / a[k][k];
            if f != 0.0 {
                let pivot_row = a[k].clone();
                for (x, y) in a[i][k..].iter_mut().zip(&pivot_row[k..]) {
                    *x -= f * y;
                }
                b[i] -= f * b[k];
            }
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let s: f64 = (i + 1..n).map(|j| a[i][j] * x[j]).sum();
        x[i] = (b[i] - s) / a[i][i];
    }
    Some(x)
}

fn jacobian(s: &Sketch, l: &Layout, x: &[f64], r0: &[f64], pulls: &[(Id, [f64; 2])]) -> Vec<Vec<f64>> {
    let mut j = vec![vec![0.0; l.n]; r0.len()];
    let mut xp = x.to_vec();
    for k in 0..l.n {
        let h = 1e-7 * x[k].abs().max(1.0);
        xp[k] = x[k] + h;
        let r1 = residuals(s, l, &xp, pulls);
        xp[k] = x[k];
        for (i, row) in j.iter_mut().enumerate() {
            row[k] = (r1[i] - r0[i]) / h;
        }
    }
    j
}

/// Rank of `m` by elimination with a relative pivot threshold.
fn rank(mut m: Vec<Vec<f64>>, cols: usize) -> usize {
    let scale = m.iter().flatten().fold(0.0f64, |a, v| a.max(v.abs())).max(1e-300);
    let (mut rank, rows) = (0, m.len());
    for c in 0..cols {
        let Some(p) = (rank..rows).max_by(|&i, &j| m[i][c].abs().total_cmp(&m[j][c].abs())) else { break };
        if m[p][c].abs() < 1e-6 * scale {
            continue;
        }
        m.swap(rank, p);
        for i in rank + 1..rows {
            let f = m[i][c] / m[rank][c];
            if f != 0.0 {
                let pivot_row = m[rank].clone();
                for (x, y) in m[i][c..].iter_mut().zip(&pivot_row[c..]) {
                    *x -= f * y;
                }
            }
        }
        rank += 1;
    }
    rank
}

impl Sketch {
    /// Solve the constraints in place.
    pub fn solve(&mut self) -> Report {
        self.solve_pulled(&[])
    }

    /// Solve while pulling the given points toward targets (dragging): constraints stay satisfied, the points
    /// follow as far as the constraints allow.
    pub fn drag(&mut self, pulls: &[(Id, [f64; 2])]) -> Report {
        self.solve_pulled(pulls)
    }

    fn solve_pulled(&mut self, pulls: &[(Id, [f64; 2])]) -> Report {
        if self.check().is_err() {
            // Nothing can be solved; name the constraints that cannot be evaluated.
            let violated = self.constraints.iter().filter(|c| !self.accepts(&c.v)).map(|c| c.id).collect();
            return Report { status: Status::Conflicting, dof: 0, max_residual: f64::INFINITY, violated, iterations: 0 };
        }
        let l = Layout::new(self);
        let mut x = l.start(self);
        let (mut lambda, mut iterations) = (1e-3, 0);
        let mut r = residuals(self, &l, &x, pulls);
        let cost = |r: &[f64]| r.iter().map(|v| v * v).sum::<f64>();
        let mut c = cost(&r);
        while iterations < 200 && l.n > 0 && !r.is_empty() {
            iterations += 1;
            let j = jacobian(self, &l, &x, &r, pulls);
            let mut g = vec![0.0; l.n];
            let mut h = vec![vec![0.0; l.n]; l.n];
            for (row, ri) in j.iter().zip(&r) {
                for a in 0..l.n {
                    if row[a] != 0.0 {
                        g[a] += row[a] * ri;
                        for b in 0..l.n {
                            h[a][b] += row[a] * row[b];
                        }
                    }
                }
            }
            if g.iter().all(|v| v.abs() < 1e-14) {
                break;
            }
            let mut improved = false;
            for _ in 0..12 {
                let mut hd = h.clone();
                for (a, row) in hd.iter_mut().enumerate() {
                    row[a] += lambda * h[a][a].max(1e-9);
                }
                let Some(d) = lin_solve(hd, g.iter().map(|v| -v).collect()) else {
                    lambda *= 10.0;
                    continue;
                };
                let xn: Vec<f64> = x.iter().zip(&d).map(|(a, b)| a + b).collect();
                let rn = residuals(self, &l, &xn, pulls);
                let cn = cost(&rn);
                if cn.is_finite() && cn < c {
                    let step = d.iter().fold(0.0f64, |a, v| a.max(v.abs()));
                    x = xn;
                    r = rn;
                    c = cn;
                    lambda = (lambda * 0.3).max(1e-12);
                    improved = true;
                    if step < 1e-13 {
                        iterations = 1000;
                    }
                    break;
                }
                lambda *= 10.0;
            }
            if !improved || c.sqrt() < 1e-13 {
                break;
            }
        }
        // Write back.
        for &(id, i) in &l.pt {
            if let Some(Entity::Point { x: px, y: py }) = self.entity_mut(id) {
                *px = x[i];
                *py = x[i + 1];
            }
        }
        for &(id, i) in &l.rad {
            if let Some(Entity::Circle { r, .. }) = self.entity_mut(id) {
                *r = x[i];
            }
        }
        self.report(iterations.min(999))
    }

    /// Constraint status at the current geometry (also used after loading a file).
    pub fn report(&self, iterations: usize) -> Report {
        let l = Layout::new(self);
        let x = l.start(self);
        let all = residuals(self, &l, &x, &[]);
        let max_residual = all.iter().fold(0.0f64, |a, v| a.max(v.abs()));
        let mut violated = Vec::new();
        for c in &self.constraints {
            let mut rr = Vec::new();
            residual(self, &l, &x, &c.v, &mut rr);
            if rr.iter().any(|v| v.abs() >= VIOLATION_TOL) {
                violated.push(c.id);
            }
        }
        let arc_bad = self.entities.iter().any(|e| matches!(e.v, Entity::Arc { .. })) && max_residual >= VIOLATION_TOL;
        let j = jacobian(self, &l, &x, &all, &[]);
        let rk = rank(j, l.n);
        let dof = l.n - rk;
        let n_res = all.len();
        let ok = violated.is_empty() && !arc_bad;
        let status = if !ok {
            Status::Conflicting
        } else if rk < n_res {
            // Fewer independent directions than residual rows: some constraints are redundant (they agree).
            Status::OverConstrained
        } else if dof == 0 {
            Status::FullyConstrained
        } else {
            Status::UnderConstrained
        };
        Report { status, dof, max_residual, violated, iterations }
    }
}
