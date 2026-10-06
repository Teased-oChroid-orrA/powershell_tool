//! Rigid analytic pin against the lug bore.
//!
//! The pin is a circle of radius `r_p` whose centre `c` is the single
//! reference point. It is never meshed. Contact is enforced at the Gauss
//! points of every bore element edge (not at the nodes): quadratic edges
//! carry a smooth contact pressure only if the penalty is integrated along
//! the edge - node-to-surface on Q9 elements gives the classic 1:4:1
//! oscillation. The master surface being a perfect circle, the gap at a
//! point `x` is exactly `|x - c| - r_p`.
//!
//! Normal contact is a penalty, tangential contact Coulomb friction with a
//! penalty-regularised stick. Forces act on the lug only; the pin
//! reaction is their sum.
//!
//! Everything is solved in the condensed bore space: with `u_b` the bore
//! displacements and `S` the Schur complement from `fe.rs`, the lug
//! equilibrium is `S u_b = f_c(u_b, c)`.

use crate::fe::Condensed;
use edge_check::fem::{shape1, GAUSS3};
use edge_check::linalg::BandedSpd;

#[derive(Debug, Clone, Copy)]
pub struct ContactParams {
    /// Normal penalty `kn` as a multiple of `E / a` (a = bore radius). The
    /// augmented-Lagrangian multipliers remove the penetration this would
    /// otherwise leave, so the result does not depend on it; it only sets how
    /// fast Newton converges.
    pub penalty_factor: f64,
    /// Coulomb friction coefficient (0 = frictionless).
    pub friction: f64,
}

impl Default for ContactParams {
    fn default() -> Self {
        Self { penalty_factor: 100.0, friction: 0.0 }
    }
}

/// One contact integration point on a bore element edge.
#[derive(Debug, Clone)]
pub struct GaussPoint {
    /// The three edge nodes (a bore edge of the bushing or lug).
    nodes: [usize; 3],
    n: [f64; 3],
    /// Quadrature weight x edge Jacobian (length).
    w: f64,
    /// Reference position.
    x0: [f64; 2],
}

/// The bushing-to-lug interface: a unilateral contact with an initial overlap (the
/// interference fit) and Coulomb friction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterfaceSpec {
    /// Radial overlap of the bushing's outer surface into the hole (half the diametral
    /// interference); negative is a clearance.
    pub interference: f64,
    /// Coulomb friction between bushing and lug.
    pub friction: f64,
}

/// One interface integration point: a pair of conforming edges (bushing outer, lug bore).
#[derive(Debug, Clone)]
struct IfaceGp {
    nb: [usize; 3],
    nl: [usize; 3],
    n: [f64; 3],
    w: f64,
    nrm: [f64; 2],
    tau: [f64; 2],
    x0: [f64; 2],
}

/// Interface state at one integration point.
#[derive(Debug, Clone, Copy)]
pub struct IfacePoint {
    /// Polar angle about the hole centre, degrees in `[0, 360)`.
    pub angle_deg: f64,
    /// Contact pressure between bushing and lug (psi); 0 where the fit has been lost.
    pub pressure: f64,
    /// Friction traction on the lug along increasing angle (psi).
    pub shear: f64,
    pub gap: f64,
    pub slipping: bool,
    pub w: f64,
}

/// Path-dependent state carried between load steps.
#[derive(Debug, Clone)]
pub struct ContactState {
    /// Bore displacements (condensed free dofs).
    pub u: Vec<f64>,
    /// Tangential friction traction on the lug per Gauss point (along +tau).
    pub t: Vec<f64>,
    /// Polar angle of each Gauss point about the pin centre at the last converged step.
    pub phi: Vec<f64>,
    /// Augmented-Lagrangian contact pressure multipliers (warm start between steps): the
    /// pin Gauss points, then the interface ones.
    pub lambda: Vec<f64>,
    /// Friction traction at each interface point at the last converged step (along +tau).
    pub ti: Vec<f64>,
    /// Relative tangential displacement of each interface point at the last converged step: the
    /// friction increment of the next step is measured from it.
    pub st_prev: Vec<f64>,
    /// Bore displacement the lug would have from plastic strain alone (`K^-1 F_p` at the
    /// bore). Zero for an elastic lug; the equilibrium is `S (u - offset) = f_c`.
    pub offset: Vec<f64>,
    /// Pin centre.
    pub centre: [f64; 2],
}

/// What a converged state looks like at one Gauss point.
#[derive(Debug, Clone, Copy)]
pub struct PointResult {
    /// Polar angle about the hole centre, degrees in `[0, 360)`.
    pub angle_deg: f64,
    /// Normal contact pressure (psi).
    pub pressure: f64,
    /// Friction traction on the lug along increasing angle (psi).
    pub shear: f64,
    pub gap: f64,
    pub slipping: bool,
    pub x: [f64; 2],
    pub w: f64,
}

#[derive(Debug, Clone)]
pub struct StepResult {
    /// Total contact force on the lug per unit thickness (lbf/in): equals the pin load.
    pub force: [f64; 2],
    pub iterations: usize,
    pub residual: f64,
    pub points: Vec<PointResult>,
    /// Bushing-to-lug interface points (empty without a bushing).
    pub iface: Vec<IfacePoint>,
}


/// Tangential penalty as a fraction of the normal one. Stick is enforced by the augmented-Lagrangian
/// tangential multiplier (the traction is carried between passes), so this only sets the convergence
/// rate of that iteration: small keeps Newton easy, large converges in fewer passes. Measured on the
/// bushed full-ring model, 0.005-0.03 all converge to the same peak hoop stress (within 0.3 %) while
/// a pure penalty stick (no multiplier) was 4-8 % low at 45-90 degrees.
const STICK_RATIO: f64 = 0.005;

fn stick_ratio() -> f64 {
    STICK_RATIO
}

/// Interface contact by nodal collocation (see `with_interface`); `false` = Gauss points.
const NODAL_INTERFACE: bool = true;

/// Passes of the multiplier update per load step.
const MAX_OUTER: usize = 3;
/// Extra passes when friction is active.
const FRICTION_EXTRA_PASSES: usize = 10;


#[derive(Clone)]
pub struct ContactModel<'a> {
    pub cond: &'a Condensed,
    pub gps: Vec<GaussPoint>,
    igps: Vec<IfaceGp>,
    iface: Option<InterfaceSpec>,
    kn_i: f64,
    pub pin_radius: f64,
    pub kn: f64,
    pub kt: f64,
    pub mu: f64,
    /// Pin centre can move perpendicular to `dir` (full model).
    pub free_perp: bool,
    pub dir: [f64; 2],
    /// Bore compliance with an elastic pin's added (`pin.rs`); the lug's own `G`/`S` when `None`.
    pin: Option<std::sync::Arc<crate::pin::CombinedCompliance>>,
}

impl<'a> ContactModel<'a> {
    pub fn new(cond: &'a Condensed, pin_radius: f64, params: ContactParams, dir: [f64; 2]) -> Self {
        Self::with_interface(cond, pin_radius, params, dir, None)
    }

    /// As [`new`](Self::new); with a bushing in the mesh, `iface` gives its fit.
    pub fn with_interface(cond: &'a Condensed, pin_radius: f64, params: ContactParams, dir: [f64; 2], iface: Option<InterfaceSpec>) -> Self {
        let mesh = &cond.mesh;
        let mut igps = Vec::new();
        if iface.is_some() {
            if NODAL_INTERFACE {
                // Node-to-node collocation on the conforming interface with row-sum (lumped) edge
                // weights `int N_k ds` = (1/6, 4/6, 1/6) of the edge length: one pressure per node,
                // all weights positive. Gauss-point constraints on quadratic edges leave a 3-point
                // sawtooth in the pressure (+-60 % on the bushed lug) that nodal collocation does not.
                let mut by_node: std::collections::BTreeMap<usize, (usize, f64)> = std::collections::BTreeMap::new();
                for (eb, el) in &mesh.interface_edges {
                    let p = [mesh.nodes[el[0]], mesh.nodes[el[1]], mesh.nodes[el[2]]];
                    for &(eta, wq) in &GAUSS3 {
                        let (n, dn) = shape1(eta);
                        let mut dx = [0.0; 2];
                        for k in 0..3 {
                            for c in 0..2 {
                                dx[c] += dn[k] * p[k][c];
                            }
                        }
                        let len = wq * dx[0].hypot(dx[1]);
                        for k in 0..3 {
                            let e = by_node.entry(el[k]).or_insert((eb[k], 0.0));
                            e.1 += n[k] * len;
                        }
                    }
                }
                for (nl, (nb, w)) in by_node {
                    let x0 = mesh.nodes[nl];
                    let r = x0[0].hypot(x0[1]).max(1e-300);
                    let nrm = [x0[0] / r, x0[1] / r];
                    igps.push(IfaceGp { nb: [nb; 3], nl: [nl; 3], n: [1.0, 0.0, 0.0], w, nrm, tau: [-nrm[1], nrm[0]], x0 });
                }
            } else {
                for (eb, el) in &mesh.interface_edges {
                    let p = [mesh.nodes[el[0]], mesh.nodes[el[1]], mesh.nodes[el[2]]];
                    for &(eta, wq) in &GAUSS3 {
                        let (n, dn) = shape1(eta);
                        let (mut x0, mut dx) = ([0.0; 2], [0.0; 2]);
                        for k in 0..3 {
                            for c in 0..2 {
                                x0[c] += n[k] * p[k][c];
                                dx[c] += dn[k] * p[k][c];
                            }
                        }
                        let r = x0[0].hypot(x0[1]).max(1e-300);
                        let nrm = [x0[0] / r, x0[1] / r];
                        igps.push(IfaceGp { nb: *eb, nl: *el, n, w: wq * dx[0].hypot(dx[1]), nrm, tau: [-nrm[1], nrm[0]], x0 });
                    }
                }
            }
        }
        let mut gps = Vec::with_capacity(mesh.bore_edges.len() * 3);
        for edge in &mesh.bore_edges {
            let nodes = *edge;
            let p = [mesh.nodes[edge[0]], mesh.nodes[edge[1]], mesh.nodes[edge[2]]];
            for &(eta, wq) in &GAUSS3 {
                let (n, dn) = shape1(eta);
                let mut x0 = [0.0; 2];
                let mut dx = [0.0; 2];
                for k in 0..3 {
                    for c in 0..2 {
                        x0[c] += n[k] * p[k][c];
                        dx[c] += dn[k] * p[k][c];
                    }
                }
                gps.push(GaussPoint { nodes, n, w: wq * dx[0].hypot(dx[1]), x0 });
            }
        }
        // The pin presses on the bushing when there is one.
        let e_pin = cond.bushing_material.map_or(cond.material.e_psi, |m| m.e_psi);
        let kn = params.penalty_factor * e_pin / mesh.bore_radius;
        let e_if = cond.material.e_psi.max(cond.bushing_material.map_or(0.0, |m| m.e_psi));
        let kn_i = params.penalty_factor * e_if / mesh.hole_radius;
        Self { cond, gps, igps, iface, kn_i, pin_radius, kn, kt: kn * stick_ratio(), mu: params.friction, free_perp: cond.mesh.periodic, dir, pin: None }
    }

    /// Press an elastic pin instead of a rigid one: the bore compliance becomes `G + A`.
    pub fn with_pin_compliance(mut self, pin: std::sync::Arc<crate::pin::CombinedCompliance>) -> Self {
        self.pin = Some(pin);
        self
    }

    /// Bore Green's matrix the contact solves with (`m x m`, row-major).
    fn g_mat(&self) -> &[f64] {
        self.pin.as_ref().map_or(&self.cond.g, |p| &p.g)
    }

    /// Its inverse, the Schur complement.
    fn s_mat(&self) -> &[f64] {
        self.pin.as_ref().map_or(&self.cond.s, |p| &p.s)
    }

    pub fn fresh_state(&self) -> ContactState {
        let centre = [0.0, 0.0];
        let phi = self.gps.iter().map(|g| g.x0[1].atan2(g.x0[0])).collect();
        ContactState { u: vec![0.0; self.cond.m], t: vec![0.0; self.gps.len()], phi, lambda: vec![0.0; self.gps.len() + self.igps.len()], ti: vec![0.0; self.igps.len()], st_prev: vec![0.0; self.igps.len()], offset: vec![0.0; self.cond.m], centre }
    }

    fn disp(&self, u: &[f64], g: &GaussPoint) -> [f64; 2] {
        let mut d = [0.0; 2];
        for k in 0..3 {
            for (c, dc) in d.iter_mut().enumerate() {
                if let Some(i) = self.cond.cidx(g.nodes[k], c) {
                    *dc += g.n[k] * u[i];
                }
            }
        }
        d
    }

    /// Advance the pin so its centre sits at `s * dir` (+ a free perpendicular
    /// offset in the full model), starting from `state`. The state is updated
    /// in place on success.
    pub fn advance(&self, state: &mut ContactState, s: f64) -> Result<StepResult, String> {
        // Friction needs a few more passes than the pressure alone: the tangential multiplier
        // converges at the rate of the stick penalty against the lug's own tangential compliance.
        let friction = self.mu > 0.0 || self.iface.is_some_and(|i| i.friction > 0.0);
        self.advance_depth(state, s, 0, if friction { MAX_OUTER + FRICTION_EXTRA_PASSES } else { MAX_OUTER })
    }

    /// One multiplier pass only: the penalised solution, accurate to the penalty error (a few
    /// percent of the load). Good enough to bracket a target load; follow with [`advance`](Self::advance).
    pub fn advance_coarse(&self, state: &mut ContactState, s: f64) -> Result<StepResult, String> {
        self.advance_depth(state, s, 0, 1)
    }

    /// `advance_once`, halving the step (and so the friction increment) when Newton fails.
    fn advance_depth(&self, state: &mut ContactState, s: f64, depth: u32, outer: usize) -> Result<StepResult, String> {
        let mut trial = state.clone();
        match self.advance_once(&mut trial, s, outer) {
            Ok(r) => {
                *state = trial;
                Ok(r)
            }
            // The bordered Newton failed. A free sideways coordinate is first tried by halving the
            // step (cheap: each half starts close), and only if that keeps failing by an outer root
            // search on the coordinate around the robust fixed-centre problem (16 full solves).
            Err(_) if self.free_perp && depth >= 3 => {
                let mut nested = state.clone();
                match self.advance_nested(&mut nested, s, outer) {
                    Ok(r) => {
                        *state = nested;
                        Ok(r)
                    }
                    Err(e) if depth >= 8 => Err(e),
                    Err(_) => self.substep(state, s, depth, outer),
                }
            }
            Err(e) if depth >= 8 => Err(e),
            Err(_) => self.substep(state, s, depth, outer),
        }
    }

    fn substep(&self, state: &mut ContactState, s: f64, depth: u32, outer: usize) -> Result<StepResult, String> {
        let s0 = state.centre[0] * self.dir[0] + state.centre[1] * self.dir[1];
        let mid = 0.5 * (s0 + s);
        self.advance_depth(state, mid, depth + 1, outer)?;
        self.advance_depth(state, s, depth + 1, outer)
    }

    /// Fixed-centre contact problems (symmetric, robust) inside a secant search on the sideways
    /// pin coordinate `q`: the net force on the pin perpendicular to the load must vanish.
    fn advance_nested(&self, state: &mut ContactState, s: f64, outer: usize) -> Result<StepResult, String> {
        let perp = [-self.dir[1], self.dir[0]];
        let mut fixed = self.clone();
        fixed.free_perp = false;
        let a = self.cond.mesh.bore_radius;
        let q0 = state.centre[0] * perp[0] + state.centre[1] * perp[1];
        let at = |q: f64| -> Result<(ContactState, StepResult, f64), String> {
            let mut t = state.clone();
            t.centre = [q * perp[0], q * perp[1]];
            let r = fixed.advance_once(&mut t, s, outer)?;
            let g = r.force[0] * perp[0] + r.force[1] * perp[1];
            Ok((t, r, g))
        };
        let scale = |r: &StepResult| r.force[0].hypot(r.force[1]).max(1e-300);
        let (mut a0, mut r0, mut g0) = at(q0)?;
        if g0.abs() <= 2e-4 * scale(&r0) {
            *state = a0;
            return Ok(r0);
        }
        let max_dq = 0.02 * a;
        let mut q_prev = q0;
        let mut q = q0 - (g0.signum() * 0.002 * a).min(max_dq);
        let (mut a1, mut r1, mut g1) = at(q)?;
        for _ in 0..16 {
            if g1.abs() <= 2e-4 * scale(&r1) {
                *state = a1;
                return Ok(r1);
            }
            let slope = (g1 - g0) / (q - q_prev);
            let dq = if slope.abs() > 1e-300 && slope.is_finite() { -g1 / slope } else { -g1.signum() * 0.002 * a };
            let q_next = q + dq.clamp(-max_dq, max_dq);
            q_prev = q;
            (a0, r0, g0) = (a1, r1, g1);
            q = q_next;
            (a1, r1, g1) = at(q)?;
        }
        let _ = (a0, r0);
        Err("the sideways pin position did not settle".into())
    }

    fn advance_once(&self, state: &mut ContactState, s: f64, outer: usize) -> Result<StepResult, String> {
        let perp = [-self.dir[1], self.dir[0]];
        let mut u = state.u.clone();
        // The sideways pin coordinate starts where the state left it (constant when not free).
        let mut q = state.centre[0] * perp[0] + state.centre[1] * perp[1];
        // Multipliers: normal pressures (pin points, then interface points), then the friction
        // tractions in the same order. The friction increment is measured from the step start, so
        // the tangential multiplier is what makes a sticking point stick exactly (Alart-Curnier).
        let nn = state.lambda.len();
        let mut lam = state.lambda.clone();
        lam.extend(state.t.iter().chain(state.ti.iter()));
        let centre_of = |q: f64| [s * self.dir[0] + q * perp[0], s * self.dir[1] + q * perp[1]];

        // Augmented Lagrangian: Newton on the penalised problem, then move the
        // multipliers to the pressures it produced. Two or three passes remove the
        // penalty's penetration error (hoop stress within 0.005 % of the converged
        // value); more only chase point-to-point pressure oscillations that the
        // stresses cannot see.
        let mut iterations = 0usize;
        let mut residual = 0.0;
        let mut fin = None;
        for _outer in 0..outer {
            let (it, res) = self.newton(state, &lam, &mut u, &mut q, &centre_of)?;
            iterations += it;
            residual = res;
            let ev = self.evaluate(&u, centre_of(q), state, &lam, true);
            let pressures: Vec<f64> = ev.points.iter().map(|p| p.pressure).chain(ev.ipoints.iter().map(|p| p.pressure)).collect();
            let shears: Vec<f64> = ev.points.iter().map(|p| p.shear).chain(ev.ipoints.iter().map(|p| p.shear)).collect();
            let peak = pressures.iter().fold(0.0f64, |m, p| m.max(*p));
            let change = pressures.iter().zip(&lam[..nn]).fold(0.0f64, |m, (p, l)| m.max((p - l).abs()));
            let change_t = shears.iter().zip(&lam[nn..]).fold(0.0f64, |m, (t, l)| m.max((t - l).abs()));
            lam[..nn].copy_from_slice(&pressures);
            lam[nn..].copy_from_slice(&shears);
            let done = change <= 1e-2 * peak.max(1e-12) && change_t <= 2e-3 * (self.mu.max(self.iface.map_or(0.0, |i| i.friction)) * peak).max(1e-12);
            fin = Some(ev);
            if done {
                break;
            }
        }
        let fin = fin.expect("at least one outer iteration");
        // Commit: friction state and multipliers follow the converged configuration.
        state.u = u;
        state.centre = centre_of(q);
        state.t = fin.t_new.clone();
        state.phi = fin.phi_new.clone();
        state.ti = fin.ti_new.clone();
        state.st_prev = fin.st_new.clone();
        lam.truncate(nn);
        state.lambda = lam;
        Ok(StepResult { force: fin.total, iterations, residual, points: fin.points, iface: fin.ipoints })
    }

    /// Damped Newton on the condensed bore equations for fixed multipliers.
    fn newton(&self, state: &ContactState, lam: &[f64], u: &mut Vec<f64>, q: &mut f64, centre_of: &dyn Fn(f64) -> [f64; 2]) -> Result<(usize, f64), String> {
        let m = self.cond.m;
        let f_scale = (self.cond.material.e_psi * self.cond.mesh.bore_radius * 1e-6).max(1e-12);
        let mut residual = f64::INFINITY;
        let mut iterations = 0usize;
        let mut converged = false;
        let mut factor: Option<TangentFactor> = None;
        let mut prev_residual = f64::INFINITY;
        let mut eval = self.evaluate(u, centre_of(*q), state, lam, true);
        for it in 0..80 {
            iterations = it + 1;
            let r_norm = norm_inf(&eval.r_u).max(eval.r_q.abs());
            let scale = f_scale.max(norm_inf(&eval.f));
            residual = r_norm / scale;
            if residual < 1e-7 {
                converged = true;
                break;
            }
            // Solve (S + Kc) x = b through the bore Green's matrix (Woodbury).
            let rhs_u: Vec<f64> = eval.r_u.iter().map(|v| -v).collect();
            let mut rhs_all: Vec<&[f64]> = vec![&rhs_u];
            if self.free_perp {
                rhs_all.push(&eval.h);
            }
            // Refactor when the contact set changed or the last step did not at least halve the
            // residual; otherwise keep the factorisation (chord iteration).
            let refresh = it == 0 || residual > 0.5 * prev_residual;
            prev_residual = residual;
            let sols = self.solve_tangent(&eval.rows, &rhs_all, &mut factor, refresh)?;
            let y1 = &sols[0];
            let mut dq = 0.0;
            let mut du = y1.clone();
            if self.free_perp {
                let y2 = &sols[1];
                let ht_y1: f64 = eval.h.iter().zip(y1).map(|(a, b)| a * b).sum();
                let ht_y2: f64 = eval.h.iter().zip(y2).map(|(a, b)| a * b).sum();
                // The pin's sideways coordinate is nearly neutral at first touch (one or two
                // contact points: it can slide along the bore). A tiny pivot regularisation in the
                // iteration matrix only keeps that update bounded; the residual is untouched.
                let den = eval.c_qq - ht_y2 + 1e-5 * self.cond.material.e_psi;
                if den.abs() > 1e-12 * (eval.c_qq.abs() + 1.0) {
                    dq = (-eval.r_q - ht_y1) / den;
                    for i in 0..m {
                        du[i] = y1[i] - y2[i] * dq;
                    }
                }
            }
            // Backtracking on the 2-norm of the residual (Armijo).
            let r2 = norm2(&eval.r_u, eval.r_q);
            let sdu: Vec<f64> = (0..m).map(|i| self.s_mat()[i * m..(i + 1) * m].iter().zip(&du).map(|(a, b)| a * b).sum()).collect();
            let mut alpha = 1.0;
            let mut accepted = false;
            for _ in 0..30 {
                let un: Vec<f64> = u.iter().zip(&du).map(|(a, b)| a + alpha * b).collect();
                let qn = *q + alpha * dq;
                let su_n: Vec<f64> = eval.su.iter().zip(&sdu).map(|(a, b)| a + alpha * b).collect();
                let trial = self.evaluate_su(&un, centre_of(qn), state, lam, true, Some(su_n));
                if norm2(&trial.r_u, trial.r_q) < (1.0 - 1e-4 * alpha) * r2 {
                    *u = un;
                    *q = qn;
                    eval = trial;
                    accepted = true;
                    break;
                }
                alpha *= 0.5;
            }
            if !accepted {
                // No decrease possible: at the round-off floor this is convergence.
                if residual < 1e-4 {
                    converged = true;
                }
                break;
            }
        }
        if !converged {
            return Err(format!("contact did not converge (residual {residual:.2e} after {iterations} iterations)"));
        }
        Ok((iterations, residual))
    }

    /// Solve `(S + sum_k w_k r_k r_k^T) x = b` for each `b`, with `S = G^-1`:
    /// `x = G b - G R^T (W^-1 + R G R^T)^-1 R G b`. The inner system has one
    /// row per active contact term, far fewer than the bore dofs.
    fn solve_tangent(&self, rows: &[Row], rhs: &[&[f64]], cache: &mut Option<TangentFactor>, refresh: bool) -> Result<Vec<Vec<f64>>, String> {
        let m = self.cond.m;
        let g = self.g_mat();
        let gmul = |b: &[f64]| -> Vec<f64> {
            (0..m).map(|i| g[i * m..(i + 1) * m].iter().zip(b).map(|(a, x)| a * x).sum()).collect()
        };
        let r = rows.len();
        if r == 0 {
            *cache = None;
            return Ok(rhs.iter().map(|b| gmul(b)).collect());
        }
        let reusable = !refresh && cache.as_ref().is_some_and(|c| c.ids.len() == r && c.ids.iter().zip(rows).all(|(i, row)| *i == row.id));
        if !reusable {
            // Row j of `gbt` is (G R^T)_{.j}: a sum of rows of G (G is symmetric, so a column of G
            // is a contiguous row), kept transposed so every loop below runs over contiguous memory.
            let mut gbt = vec![0.0; r * m];
            for (j, row) in rows.iter().enumerate() {
                let out = &mut gbt[j * m..(j + 1) * m];
                for &(dof, coef) in &row.nz {
                    let col = &g[dof * m..(dof + 1) * m];
                    for (o, c) in out.iter_mut().zip(col) {
                        *o += coef * c;
                    }
                }
            }
            let mut mm = BandedSpd::zeros(r, r - 1);
            for (i, row) in rows.iter().enumerate() {
                for j in 0..=i {
                    let src = &gbt[j * m..(j + 1) * m];
                    let mut v = 0.0;
                    for &(dof, coef) in &row.nz {
                        v += coef * src[dof];
                    }
                    if i == j {
                        v += 1.0 / row.weight;
                    }
                    mm.add(i, j, v);
                }
            }
            if !mm.factor() {
                *cache = None;
                return Err("contact tangent is not positive definite".into());
            }
            *cache = Some(TangentFactor { ids: rows.iter().map(|row| row.id).collect(), nz: rows.iter().map(|row| row.nz.clone()).collect(), gbt, mm });
        }
        let c = cache.as_ref().expect("factor just built or reused");
        let mut out = Vec::with_capacity(rhs.len());
        for b in rhs {
            let mut x = gmul(b);
            let mut y: Vec<f64> = c.nz.iter().map(|nz| nz.iter().map(|&(dof, coef)| coef * x[dof]).sum()).collect();
            c.mm.solve_in_place(&mut y);
            for (j, yj) in y.iter().enumerate() {
                for (xi, gi) in x.iter_mut().zip(&c.gbt[j * m..(j + 1) * m]) {
                    *xi -= yj * gi;
                }
            }
            out.push(x);
        }
        Ok(out)
    }

    /// Residual, tangent contributions and per-point results at `(u, centre)`.
    fn evaluate(&self, u: &[f64], centre: [f64; 2], state: &ContactState, lambda: &[f64], with_tangent: bool) -> Eval {
        self.evaluate_su(u, centre, state, lambda, with_tangent, None)
    }

    /// [`evaluate`](Self::evaluate) with `S (u - offset)` supplied (a line search gets it for
    /// free from `S du`); computed here otherwise.
    fn evaluate_su(&self, u: &[f64], centre: [f64; 2], state: &ContactState, lambda: &[f64], with_tangent: bool, su: Option<Vec<f64>>) -> Eval {
        self.evaluate_i(u, centre, state, lambda, with_tangent, su)
    }

    fn evaluate_i(&self, u: &[f64], centre: [f64; 2], state: &ContactState, lambda: &[f64], with_tangent: bool, su_given: Option<Vec<f64>>) -> Eval {
        let m = self.cond.m;
        // `lambda` = normal multipliers then tangential ones (see `advance_once`).
        let nn = self.gps.len() + self.igps.len();
        let perp = [-self.dir[1], self.dir[0]];
        let mut f = vec![0.0; m];
        let mut rows: Vec<Row> = Vec::new();
        let mut h = vec![0.0; m];
        let (mut c_qq, mut r_q) = (0.0, 0.0);
        let mut c_geo = 0.0;
        let mut h_geo = vec![0.0; m];
        let mut total = [0.0; 2];
        let mut points = Vec::with_capacity(self.gps.len());
        let mut t_new = vec![0.0; self.gps.len()];
        let mut phi_new = vec![0.0; self.gps.len()];

        for (gi, g) in self.gps.iter().enumerate() {
            let d = self.disp(u, g);
            let x = [g.x0[0] + d[0], g.x0[1] + d[1]];
            let rel = [x[0] - centre[0], x[1] - centre[1]];
            let dist = rel[0].hypot(rel[1]).max(1e-300);
            let nrm = [rel[0] / dist, rel[1] / dist];
            let tau = [-nrm[1], nrm[0]];
            let gap = dist - self.pin_radius;
            let p = (lambda[gi] + self.kn * (-gap)).max(0.0);
            let phi = rel[1].atan2(rel[0]);
            phi_new[gi] = phi;

            let mut t = 0.0;
            let mut slipping = false;
            let mut stick = false;
            if self.mu > 0.0 && p > 0.0 {
                let mut dphi = phi - state.phi[gi];
                if dphi > std::f64::consts::PI {
                    dphi -= 2.0 * std::f64::consts::PI;
                } else if dphi < -std::f64::consts::PI {
                    dphi += 2.0 * std::f64::consts::PI;
                }
                let trial = lambda[nn + gi] - self.kt * dist * dphi;
                let cap = self.mu * p;
                if trial.abs() > cap {
                    t = cap * trial.signum();
                    slipping = true;
                } else {
                    t = trial;
                    stick = true;
                }
            }
            t_new[gi] = t;

            let fx = g.w * (p * nrm[0] + t * tau[0]);
            let fy = g.w * (p * nrm[1] + t * tau[1]);
            total[0] += fx;
            total[1] += fy;
            r_q += fx * perp[0] + fy * perp[1];
            for k in 0..3 {
                for (c, fc) in [fx, fy].iter().enumerate() {
                    if let Some(i) = self.cond.cidx(g.nodes[k], c) {
                        f[i] += g.n[k] * fc;
                    }
                }
            }
            if with_tangent && p > 0.0 && self.free_perp {
                // Geometric part of the pin-coordinate tangent: the contact force turns with the
                // normal, d(f)/dc = w [kn n n^T - (p / dist)(I - n n^T)]. The normal part is in the
                // rows below; this is the rest, projected on the sideways direction. It matters
                // when the sliding stiffness is small (light load, oblique pin).
                let np = nrm[0] * perp[0] + nrm[1] * perp[1];
                let w_geo = g.w * p / dist;
                c_geo -= w_geo * (1.0 - np * np);
                let t = [perp[0] - nrm[0] * np, perp[1] - nrm[1] * np];
                for k in 0..3 {
                    for (c, tc) in t.iter().enumerate() {
                        if let Some(i) = self.cond.cidx(g.nodes[k], c) {
                            h_geo[i] += w_geo * tc * g.n[k];
                        }
                    }
                }
            }
            if with_tangent && p > 0.0 {
                // Normal stiffness: w kn n n^T; stick friction: w kt tau tau^T.
                let mut blocks: Vec<([f64; 2], f64)> = vec![(nrm, self.kn)];
                if stick {
                    blocks.push((tau, self.kt));
                }
                for (bi, (dirv, kk)) in blocks.into_iter().enumerate() {
                    let wk = g.w * kk;
                    let mut nz: Vec<(usize, f64)> = Vec::with_capacity(6);
                    for a in 0..3 {
                        for (ca, dc) in dirv.iter().enumerate() {
                            if let Some(i) = self.cond.cidx(g.nodes[a], ca) {
                                nz.push((i, g.n[a] * dc));
                            }
                        }
                    }
                    // Coupling to the free perpendicular pin coordinate.
                    let dp = dirv[0] * perp[0] + dirv[1] * perp[1];
                    c_qq += wk * dp * dp;
                    for &(i, coef) in &nz {
                        h[i] -= wk * dp * coef;
                    }
                    rows.push(Row { id: ((bi as u32) << 24) | gi as u32, nz, weight: wk });
                }
            }

            let ang = g.x0[1].atan2(g.x0[0]).to_degrees().rem_euclid(360.0);
            points.push(PointResult { angle_deg: ang, pressure: p, shear: t, gap, slipping, x, w: g.w });
        }

        // Bushing-to-lug interface.
        let ng = self.gps.len();
        let mut ipoints = Vec::with_capacity(self.igps.len());
        let mut ti_new = vec![0.0; self.igps.len()];
        let mut st_new = vec![0.0; self.igps.len()];
        if let Some(spec) = self.iface {
            for (ii, g) in self.igps.iter().enumerate() {
                let (mut dl, mut db) = ([0.0; 2], [0.0; 2]);
                for k in 0..3 {
                    for c in 0..2 {
                        if let Some(i) = self.cond.cidx(g.nl[k], c) {
                            dl[c] += g.n[k] * u[i];
                        }
                        if let Some(i) = self.cond.cidx(g.nb[k], c) {
                            db[c] += g.n[k] * u[i];
                        }
                    }
                }
                let rel = [dl[0] - db[0], dl[1] - db[1]];
                let gap = rel[0] * g.nrm[0] + rel[1] * g.nrm[1] - spec.interference;
                let st = rel[0] * g.tau[0] + rel[1] * g.tau[1];
                let p = (lambda[ng + ii] + self.kn_i * (-gap)).max(0.0);
                let (mut t, mut stick, mut slipping) = (0.0, false, false);
                if spec.friction > 0.0 && p > 0.0 {
                    let trial = lambda[nn + ng + ii] - self.kn_i * stick_ratio() * (st - state.st_prev[ii]);
                    let cap = spec.friction * p;
                    if trial.abs() > cap {
                        t = cap * trial.signum();
                        slipping = true;
                    } else {
                        t = trial;
                        stick = true;
                    }
                }
                st_new[ii] = st;
                ti_new[ii] = t;
                // Force on the lug F = w (p n + T tau); the bushing receives -F.
                let fl = [g.w * (p * g.nrm[0] + t * g.tau[0]), g.w * (p * g.nrm[1] + t * g.tau[1])];
                for k in 0..3 {
                    for (c, fc) in fl.iter().enumerate() {
                        if let Some(i) = self.cond.cidx(g.nl[k], c) {
                            f[i] += g.n[k] * fc;
                        }
                        if let Some(i) = self.cond.cidx(g.nb[k], c) {
                            f[i] -= g.n[k] * fc;
                        }
                    }
                }
                if with_tangent && p > 0.0 {
                    let mut blocks: Vec<([f64; 2], f64)> = vec![(g.nrm, self.kn_i)];
                    if stick {
                        blocks.push((g.tau, self.kn_i * stick_ratio()));
                    }
                    for (bi, (dirv, kk)) in blocks.into_iter().enumerate() {
                        let mut nz: Vec<(usize, f64)> = Vec::with_capacity(12);
                        for k in 0..3 {
                            for (c, dc) in dirv.iter().enumerate() {
                                if let Some(i) = self.cond.cidx(g.nl[k], c) {
                                    nz.push((i, g.n[k] * dc));
                                }
                                if let Some(i) = self.cond.cidx(g.nb[k], c) {
                                    nz.push((i, -g.n[k] * dc));
                                }
                            }
                        }
                        rows.push(Row { id: ((2 + bi as u32) << 24) | ii as u32, nz, weight: g.w * kk });
                    }
                }
                let ang = g.x0[1].atan2(g.x0[0]).to_degrees().rem_euclid(360.0);
                ipoints.push(IfacePoint { angle_deg: ang, pressure: p, shear: t, gap, slipping, w: g.w });
            }
        }

        // R_u = S u - f.
        let su = su_given.unwrap_or_else(|| {
            (0..m)
                .map(|i| {
                    let row = &self.s_mat()[i * m..(i + 1) * m];
                    row.iter().zip(u).zip(&state.offset).map(|((a, b), o)| a * (b - o)).sum::<f64>()
                })
                .collect()
        });
        let r_u: Vec<f64> = su.iter().zip(&f).map(|(a, b)| a - b).collect();
        // Keep the pivot positive: the geometric term is negative and is only used while the
        // total stays a proper (stable) stiffness.
        let (c_qq, h) = if self.free_perp && c_qq + c_geo > 0.0 { (c_qq + c_geo, h.iter().zip(&h_geo).map(|(a, b)| a + b).collect()) } else { (c_qq, h) };
        Eval { su, r_u, f, rows, h, c_qq, r_q: if self.free_perp { r_q } else { 0.0 }, total, points, ipoints, t_new, phi_new, ti_new, st_new }
    }

    /// Nodal bore forces (node id, force per unit thickness) for the converged state.
    pub fn bore_forces(&self, state: &ContactState) -> Vec<(usize, [f64; 2])> {
        let mut lam = state.lambda.clone();
        lam.extend(state.t.iter().chain(state.ti.iter()));
        let ev = self.evaluate(&state.u, state.centre, state, &lam, false);
        let mut by_node = vec![[0.0f64; 2]; self.cond.mesh.nodes.len()];
        for (gi, g) in self.gps.iter().enumerate() {
            let pt = &ev.points[gi];
            let rel = [pt.x[0] - state.centre[0], pt.x[1] - state.centre[1]];
            let dist = rel[0].hypot(rel[1]).max(1e-300);
            let nrm = [rel[0] / dist, rel[1] / dist];
            let tau = [-nrm[1], nrm[0]];
            let f = [g.w * (pt.pressure * nrm[0] + pt.shear * tau[0]), g.w * (pt.pressure * nrm[1] + pt.shear * tau[1])];
            for k in 0..3 {
                for c in 0..2 {
                    by_node[g.nodes[k]][c] += g.n[k] * f[c];
                }
            }
        }
        for (g, pt) in self.igps.iter().zip(&ev.ipoints) {
            let fl = [g.w * (pt.pressure * g.nrm[0] + pt.shear * g.tau[0]), g.w * (pt.pressure * g.nrm[1] + pt.shear * g.tau[1])];
            for k in 0..3 {
                for c in 0..2 {
                    by_node[g.nl[k]][c] += g.n[k] * fl[c];
                    by_node[g.nb[k]][c] -= g.n[k] * fl[c];
                }
            }
        }
        self.cond.contact_nodes.iter().map(|&n| (n, by_node[n])).collect()
    }
}

/// One rank-one term `weight * (row . du)^2` of the contact tangent: a
/// Gauss point's normal (or stick-tangent) stiffness, as a sparse row over the
/// free bore dofs.
struct Row {
    /// Which contact term this is: kind (0 pin normal, 1 pin stick, 2 interface normal, 3
    /// interface stick) in the top byte, the Gauss point below. Equal id lists mean the same
    /// active / stick set, so a factorisation can be reused.
    id: u32,
    nz: Vec<(usize, f64)>,
    weight: f64,
}

/// A factored low-rank tangent kept between Newton iterations while the contact set is unchanged
/// (a chord iteration: the coefficients go slightly stale, the residual stays exact).
struct TangentFactor {
    ids: Vec<u32>,
    nz: Vec<Vec<(usize, f64)>>,
    gbt: Vec<f64>,
    mm: BandedSpd,
}

struct Eval {
    /// `S (u - offset)`.
    su: Vec<f64>,
    r_u: Vec<f64>,
    f: Vec<f64>,
    rows: Vec<Row>,
    h: Vec<f64>,
    c_qq: f64,
    r_q: f64,
    total: [f64; 2],
    points: Vec<PointResult>,
    ipoints: Vec<IfacePoint>,
    t_new: Vec<f64>,
    phi_new: Vec<f64>,
    ti_new: Vec<f64>,
    st_new: Vec<f64>,
}

fn norm2(r: &[f64], rq: f64) -> f64 {
    (r.iter().map(|v| v * v).sum::<f64>() + rq * rq).sqrt()
}

fn norm_inf(v: &[f64]) -> f64 {
    v.iter().fold(0.0, |a, b| a.max(b.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fe::{FarEnd, Material};
    use crate::geometry::LugGeometry;
    use crate::mesh::MeshSpec;

    const AL: Material = Material { e_psi: 10.0e6, nu: 0.33 };

    /// A plain disc: the full-round head with the far end rounded to the same radius.
    fn disc(hole: f64, radius: f64) -> LugGeometry {
        LugGeometry { hole_dia: hole, width: 2.0 * radius, edge: radius, length: radius, thickness: 1.0, head_corner_radius: radius, far_corner_radius: radius }
    }

    /// Stick is enforced by the tangential multiplier, not by the penalty: a sticking point
    /// moves over the step by a small fraction of `t / kt` (a pure penalty stick moves by all of it).
    #[test]
    fn sticking_points_do_not_creep_by_the_penalty_slip() {
        let g = LugGeometry::round_head(0.5, 1.5, 0.25, 3.75);
        let cond = Condensed::build(&g, MeshSpec::default(), AL, true, FarEnd::Clamped).unwrap();
        let params = ContactParams { friction: 0.3, ..ContactParams::default() };
        let cm = ContactModel::new(&cond, 0.2495, params, [-1.0, 0.0]);
        let mut st = cm.fresh_state();
        let res = cm.advance(&mut st, 0.004).unwrap();
        let start = cm.fresh_state();
        let mut checked = 0;
        for (gi, p) in res.points.iter().enumerate() {
            if p.pressure > 0.0 && !p.slipping && p.shear.abs() > 0.02 * 0.3 * p.pressure {
                let dist = (p.x[0] - st.centre[0]).hypot(p.x[1] - st.centre[1]);
                let creep = (st.phi[gi] - start.phi[gi]).abs() * dist * cm.kt;
                assert!(creep < 0.1 * p.shear.abs(), "point {gi}: penalty slip carries {creep} of traction {}", p.shear);
                checked += 1;
            }
        }
        assert!(checked > 0, "the case must have sticking points");
    }

    #[test]
    fn interference_fit_in_a_disc_matches_the_lame_pressure() {
        let (a, r_out, delta) = (0.25, 1.0, 0.0005);
        let g = disc(2.0 * a, r_out);
        let cond = Condensed::build(&g, MeshSpec::default(), AL, false, FarEnd::Soft).unwrap();
        let mut cm = ContactModel::new(&cond, a + delta, ContactParams::default(), [-1.0, 0.0]);
        cm.free_perp = false; // a free disc: pin translation is a rigid-body mode
        let mut st = cm.fresh_state();
        let res = cm.advance(&mut st, 0.0).unwrap();
        // Lame, plane stress, rigid inner surface displaced by delta.
        let c = (r_out * r_out + a * a) / (r_out * r_out - a * a) + AL.nu;
        let p_exact = AL.e_psi * delta / (a * c);
        let n = res.points.len() as f64;
        let mean: f64 = res.points.iter().map(|p| p.pressure).sum::<f64>() / n;
        assert!((mean - p_exact).abs() / p_exact < 3e-3, "mean {mean} vs Lame {p_exact}");
        let (lo, hi) = res.points.iter().fold((f64::MAX, 0.0f64), |(l, h), p| (l.min(p.pressure), h.max(p.pressure)));
        assert!((hi - lo) / p_exact < 1e-2, "pressure must be uniform: {lo}..{hi}");
        // Self-equilibrated: no net force on the pin.
        assert!(res.force[0].hypot(res.force[1]) < 1e-6 * p_exact * a, "net {:?}", res.force);
    }

    #[test]
    fn a_clearance_pin_does_not_touch_until_the_gap_closes() {
        let g = LugGeometry::round_head(0.5, 1.5, 0.25, 3.0);
        let cond = Condensed::build(&g, MeshSpec { elements_around: 36, ..Default::default() }, AL, true, FarEnd::Clamped).unwrap();
        let cm = ContactModel::new(&cond, 0.25 - 0.001, ContactParams::default(), [-1.0, 0.0]);
        let mut st = cm.fresh_state();
        let r0 = cm.advance(&mut st, 0.0005).unwrap();
        assert!(r0.points.iter().all(|p| p.pressure == 0.0), "still in clearance");
        let r1 = cm.advance(&mut st, 0.0012).unwrap();
        assert!(r1.points.iter().any(|p| p.pressure > 0.0));
        assert!(r1.force[0] < 0.0, "pin pulls the lug toward -x: {:?}", r1.force);
    }
}

