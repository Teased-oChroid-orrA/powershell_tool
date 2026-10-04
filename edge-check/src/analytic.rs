//! Elastic stress field of a bored plate near a free edge from Muskhelishvili
//! complex potentials, no mesh.
//!
//! The plate is the half plane `x > 0` minus the disc `|z - c| < a`
//! (`c = edge`). Stresses follow from two analytic functions:
//!
//! ```text
//! sxx + syy        = 4 Re Phi(z)                 Phi = phi'
//! syy - sxx + 2i sxy = 2 [ conj(z) Phi'(z) + Psi(z) ]   Psi = psi'
//! ```
//!
//! `phi`/`psi` are sums of *fundamental solutions*: poles of every order at
//! the bore centre `z = c` (the bore's own multipole field, with the
//! `ln(z - c)` Kelvin terms tied by `psi ~ -kappa conj(alpha) ln` so the
//! displacement is single valued around the bore) and the same at the
//! mirror point `z = -c` plus a log there - analytic in the plate, so they
//! carry exactly the reflection of the bore field in the free edge. The
//! coefficients are fixed by least squares so that the bore carries the
//! prescribed contact tractions and the edge `x = 0` is traction free; the
//! problem is symmetric about the load line, so every coefficient is real.
//!
//! Nothing here is meshed or empirical. The result is a smooth, closed-form
//! field that can be evaluated anywhere, and is cross-checked against the
//! FE model (an independent method) in the tests.

use crate::fem::UnitCase;
use crate::field::UnitFieldSource;
use crate::linalg::least_squares_multi;
use crate::types::Stress;
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug)]
struct Cx {
    re: f64,
    im: f64,
}

impl Cx {
    fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    fn add(self, o: Cx) -> Cx {
        Cx::new(self.re + o.re, self.im + o.im)
    }
    fn sub(self, o: Cx) -> Cx {
        Cx::new(self.re - o.re, self.im - o.im)
    }
    fn mul(self, o: Cx) -> Cx {
        Cx::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
    fn scale(self, k: f64) -> Cx {
        Cx::new(self.re * k, self.im * k)
    }
    fn conj(self) -> Cx {
        Cx::new(self.re, -self.im)
    }
    fn inv(self) -> Cx {
        let d = self.re * self.re + self.im * self.im;
        Cx::new(self.re / d, -self.im / d)
    }
}

/// Basis functions' contributions `(Phi, Phi', Psi)` at a point.
#[derive(Clone, Copy)]
struct Pot {
    phi: Cx,
    dphi: Cx,
    psi: Cx,
}

const ZERO: Cx = Cx { re: 0.0, im: 0.0 };
const ZERO_POT: Pot = Pot { phi: ZERO, dphi: ZERO, psi: ZERO };

/// Discretisation of the fit. The bore-centre pole family converges
/// geometrically in `a / (distance to the nearest field point)`, the mirror
/// family in `a / (2 c)`; the collocation counts only need to resolve the
/// load's slope kink and the edge's far decay.
#[derive(Debug, Clone, Copy)]
pub struct FitSpec {
    pub n_bore: usize,
    pub n_mirror: usize,
    pub k_hole: usize,
    pub k_edge: usize,
}

impl Default for FitSpec {
    fn default() -> Self {
        Self { n_bore: 24, n_mirror: 20, k_hole: 97, k_edge: 100 }
    }
}

impl FitSpec {
    /// Unknown layout: a_1..a_N | b_1..b_N | beta | gamma | c_1..c_M | d_1..d_M.
    fn unknowns(&self) -> usize {
        2 * self.n_bore + 2 + 2 * self.n_mirror
    }
}

/// `(Phi, Phi', Psi)` of every unknown at `z`, computed together: the pole
/// powers are built by recurrence (`w^-k = w^-(k-1) / w`) once per point
/// instead of re-multiplied for every basis function.
fn basis_all(z: Cx, c: f64, sp: &FitSpec) -> Vec<Pot> {
    let (nb, nm) = (sp.n_bore, sp.n_mirror);
    let w_inv = z.sub(Cx::new(c, 0.0)).inv();
    let m_inv = z.add(Cx::new(c, 0.0)).inv();
    let mut out = vec![ZERO_POT; sp.unknowns()];
    // w^{-n} for n = 0.. count.
    let powers = |inv: Cx, count: usize| -> Vec<Cx> {
        let mut v = Vec::with_capacity(count + 1);
        v.push(Cx::new(1.0, 0.0));
        for n in 1..=count {
            let prev = v[n - 1];
            v.push(prev.mul(inv));
        }
        v
    };
    let wp = powers(w_inv, nb + 2);
    let mp = powers(m_inv, nm + 2);
    for k in 1..=nb {
        // phi = w^-k: Phi = -k w^{-k-1}, Phi' = k(k+1) w^{-k-2};  psi = w^-k: Psi = -k w^{-k-1}.
        out[k - 1] = Pot { phi: wp[k + 1].scale(-(k as f64)), dphi: wp[k + 2].scale((k * (k + 1)) as f64), psi: ZERO };
        out[nb + k - 1] = Pot { phi: ZERO, dphi: ZERO, psi: wp[k + 1].scale(-(k as f64)) };
    }
    // beta: phi = ln(z + c); gamma: psi = ln(z + c).
    out[2 * nb] = Pot { phi: m_inv, dphi: m_inv.mul(m_inv).scale(-1.0), psi: ZERO };
    out[2 * nb + 1] = Pot { phi: ZERO, dphi: ZERO, psi: m_inv };
    let base = 2 * nb + 2;
    for k in 1..=nm {
        out[base + k - 1] = Pot { phi: mp[k + 1].scale(-(k as f64)), dphi: mp[k + 2].scale((k * (k + 1)) as f64), psi: ZERO };
        out[base + nm + k - 1] = Pot { phi: ZERO, dphi: ZERO, psi: mp[k + 1].scale(-(k as f64)) };
    }
    out
}

/// The known Kelvin log pair at the bore centre for a resultant force
/// `x_force` (per unit thickness, applied to the plate at the bore):
/// `phi = alpha ln w`, `psi = -kappa alpha ln w`, `alpha = -X / (2 pi (1 + kappa))`.
fn known(z: Cx, c: f64, alpha: f64, kappa: f64) -> Pot {
    let w = z.sub(Cx::new(c, 0.0));
    let iw = w.inv();
    Pot { phi: iw.scale(alpha), dphi: iw.mul(iw).scale(-alpha), psi: iw.scale(-kappa * alpha) }
}

fn stress_from(p: Pot, z: Cx) -> Stress {
    let s = 4.0 * p.phi.re;
    let q = z.conj().mul(p.dphi).add(p.psi).scale(2.0); // syy - sxx + 2i sxy
    Stress { xx: 0.5 * (s - q.re), yy: 0.5 * (s + q.re), xy: 0.5 * q.im }
}

pub struct AnalyticField {
    c: f64,
    spec: FitSpec,
    kappa: f64,
    alphas: [f64; 3],
    coeffs: [Vec<f64>; 3],
    /// Max relative residual of the boundary conditions per unit case, as a
    /// self-check of the fit (traction scale = the case's own peak load).
    pub residual: [f64; 3],
}

impl AnalyticField {
    /// Builds the three unit-case fields. `nu` is Poisson's ratio (plane
    /// stress: `kappa = (3 - nu) / (1 + nu)`); the stresses depend on it only
    /// through the force-resultant (Kelvin) terms.
    pub fn build(bore_radius: f64, edge: f64, thickness: f64, nu: f64) -> Result<Self, String> {
        Self::build_with(bore_radius, edge, thickness, nu, FitSpec::default())
    }

    pub fn build_with(bore_radius: f64, edge: f64, thickness: f64, nu: f64, spec: FitSpec) -> Result<Self, String> {
        let n_unk = spec.unknowns();
        let (a, c) = (bore_radius, edge);
        if !(a > 0.0 && c > a && thickness > 0.0) {
            return Err("degenerate plate geometry".to_string());
        }
        let kappa = (3.0 - nu) / (1.0 + nu);

        // Unit-case bore tractions: sigma_rr = -p(phi), sigma_r,phi = 0.
        let per_t = 1.0 / thickness;
        let pressure = |case: usize, phi: f64| -> f64 {
            let cos_load = -phi.cos();
            match case {
                0 => 1.0,
                1 => if cos_load > 0.0 { 2.0 / (PI * a) * cos_load * per_t } else { 0.0 },
                _ => 1.0 / (PI * a) * cos_load * per_t,
            }
        };
        // Resultant force on the plate along x (per unit thickness): the pin
        // pushes the plate toward -x, X = -1/t per unit pin load.
        let alphas = [0.0, -(-per_t) / (2.0 * PI * (1.0 + kappa)), -(-per_t) / (2.0 * PI * (1.0 + kappa))];

        // Collocation points. Bore: phi in [0, pi] (symmetric half). The
        // half-cosine load has a slope kink at phi = pi/2: include it.
        let k_hole = spec.k_hole;
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut rhs: [Vec<f64>; 3] = [vec![], vec![], vec![]];
        let mut scale_rows: Vec<f64> = Vec::new();
        for k in 0..k_hole {
            let phi = PI * k as f64 / (k_hole - 1) as f64;
            let z = Cx::new(c + a * phi.cos(), a * phi.sin());
            let (cp, sp) = (phi.cos(), phi.sin());
            // Rows of the linear map unknown -> (sigma_rr, sigma_rphi).
            let mut row_rr = vec![0.0; n_unk];
            let mut row_rp = vec![0.0; n_unk];
            let pots = basis_all(z, c, &spec);
            for j in 0..n_unk {
                let st = stress_from(pots[j], z);
                row_rr[j] = st.xx * cp * cp + st.yy * sp * sp + 2.0 * st.xy * sp * cp;
                row_rp[j] = (st.yy - st.xx) * sp * cp + st.xy * (cp * cp - sp * sp);
            }
            let kn: [Stress; 3] = std::array::from_fn(|i| stress_from(known(z, c, alphas[i], kappa), z));
            for (r_idx, row) in [row_rr, row_rp].into_iter().enumerate() {
                rows.push(row);
                for case in 0..3 {
                    let st = kn[case];
                    let known_val = if r_idx == 0 {
                        st.xx * cp * cp + st.yy * sp * sp + 2.0 * st.xy * sp * cp
                    } else {
                        (st.yy - st.xx) * sp * cp + st.xy * (cp * cp - sp * sp)
                    };
                    let target = if r_idx == 0 { -pressure(case, phi) } else { 0.0 };
                    rhs[case].push(target - known_val);
                }
                scale_rows.push(1.0);
            }
        }
        // Free edge x = 0, y in [0, Y]: sigma_xx = 0, sigma_xy = 0. Geometric
        // spacing, dense near the load line, reaching 80 bore radii.
        let k_edge = spec.k_edge;
        let y_max = 80.0 * a.max(c);
        for k in 0..k_edge {
            let y = y_max * (k as f64 / (k_edge - 1) as f64).powi(3);
            let z = Cx::new(0.0, y);
            let mut row_xx = vec![0.0; n_unk];
            let mut row_xy = vec![0.0; n_unk];
            let pots = basis_all(z, c, &spec);
            for j in 0..n_unk {
                let st = stress_from(pots[j], z);
                row_xx[j] = st.xx;
                row_xy[j] = st.xy;
            }
            let kn: [Stress; 3] = std::array::from_fn(|i| stress_from(known(z, c, alphas[i], kappa), z));
            // Weight the edge rows so far points do not dominate but all are met.
            let wgt = 1.0;
            for (r_idx, row) in [row_xx, row_xy].into_iter().enumerate() {
                rows.push(row.into_iter().map(|v| v * wgt).collect());
                for case in 0..3 {
                    let known_val = if r_idx == 0 { kn[case].xx } else { kn[case].xy };
                    rhs[case].push(-known_val * wgt);
                }
            }
        }
        let m = rows.len();
        let mut flat = Vec::with_capacity(m * n_unk);
        for r in &rows {
            flat.extend_from_slice(r);
        }
        let sols = least_squares_multi(&flat, m, n_unk, &rhs, 1e-13);

        // Residual self-check on the collocation equations.
        let mut residual = [0.0; 3];
        for case in 0..3 {
            let sol = &sols[case];
            let mut worst: f64 = 0.0;
            let mut peak: f64 = 0.0;
            for i in 0..m {
                let pred: f64 = (0..n_unk).map(|j| rows[i][j] * sol[j]).sum();
                worst = worst.max((pred - rhs[case][i]).abs());
                peak = peak.max(rhs[case][i].abs());
            }
            residual[case] = if peak > 0.0 { worst / peak } else { 0.0 };
        }
        let _ = scale_rows;
        let [s0, s1, s2] = sols.try_into().map_err(|_| "solve failed".to_string())?;
        Ok(Self { c, spec, kappa, alphas, coeffs: [s0, s1, s2], residual })
    }
}

impl AnalyticField {
    fn eval_cases(&self, x: f64, y: f64) -> [Stress; 3] {
        let (y, flip) = if y < 0.0 { (-y, -1.0) } else { (y, 1.0) };
        let z = Cx::new(x, y);
        let pots = basis_all(z, self.c, &self.spec);
        std::array::from_fn(|idx| {
            let mut p = known(z, self.c, self.alphas[idx], self.kappa);
            for (j, &cj) in self.coeffs[idx].iter().enumerate() {
                if cj != 0.0 {
                    p.phi = p.phi.add(pots[j].phi.scale(cj));
                    p.dphi = p.dphi.add(pots[j].dphi.scale(cj));
                    p.psi = p.psi.add(pots[j].psi.scale(cj));
                }
            }
            let st = stress_from(p, z);
            Stress { xx: st.xx, yy: st.yy, xy: st.xy * flip }
        })
    }
}

impl UnitFieldSource for AnalyticField {
    fn stress(&self, case: UnitCase, x: f64, y: f64) -> Option<Stress> {
        Some(self.eval_cases(x, y)[case as usize])
    }

    fn stress_all(&self, x: f64, y: f64) -> Option<[Stress; 3]> {
        Some(self.eval_cases(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_bore_pressure_far_from_the_edge_is_lame() {
        let a = 0.25;
        let f = AnalyticField::build(a, 40.0 * a, 0.5, 0.33).unwrap();
        // top of bore: hoop = sxx, radial = syy
        let s = f.stress(UnitCase::Fit, 40.0 * a, a).unwrap();
        assert!((s.xx - 1.0).abs() < 3e-3 && (s.yy + 1.0).abs() < 3e-3, "{s:?} residual {:?}", f.residual);
    }

    #[test]
    fn residuals_are_small_for_every_unit_case() {
        let a = 0.25;
        for ed in [1.0, 1.5, 2.0, 4.0] {
            let f = AnalyticField::build(a, ed * 2.0 * a, 0.5, 0.33).unwrap();
            println!("e/D={ed} residual {:?}", f.residual);
            assert!(f.residual.iter().all(|r| *r < 0.02), "e/D={ed}: {:?}", f.residual);
        }
    }
}

#[cfg(test)]
mod convergence {
    use super::*;

    #[test]
    fn discretisation_study() {
        let a = 0.25;
        let refspec = FitSpec { n_bore: 44, n_mirror: 44, k_hole: 241, k_edge: 280 };
        let cands = [
            FitSpec { n_bore: 24, n_mirror: 20, k_hole: 97, k_edge: 100 },
            FitSpec { n_bore: 20, n_mirror: 16, k_hole: 81, k_edge: 90 },
            FitSpec { n_bore: 16, n_mirror: 12, k_hole: 65, k_edge: 70 },
            FitSpec { n_bore: 12, n_mirror: 10, k_hole: 49, k_edge: 56 },
        ];
        for ed in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let e = ed * 2.0 * a;
            let t0 = std::time::Instant::now();
            let rf = AnalyticField::build_with(a, e, 0.5, 0.33, refspec).unwrap();
            let t_ref = t0.elapsed();
            let mut pts: Vec<(f64, f64)> = Vec::new();
            for k in 0..37 {
                let phi = std::f64::consts::PI * k as f64 / 36.0;
                pts.push((e + a * phi.cos(), a * phi.sin()));
            }
            for k in 0..20 {
                pts.push((0.0, 4.0 * a * (k as f64 / 19.0).powi(2)));
                pts.push(((e - a) * k as f64 / 19.0, 0.0));
                pts.push((e * k as f64 / 19.0, a));
            }
            for sp in cands {
                let t0 = std::time::Instant::now();
                let f = AnalyticField::build_with(a, e, 0.5, 0.33, sp).unwrap();
                let dt = t0.elapsed();
                let mut worst = [0.0f64; 3];
                for (case, w) in worst.iter_mut().enumerate() {
                    let mut peak: f64 = 0.0;
                    let mut err: f64 = 0.0;
                    for &(x, y) in &pts {
                        let (s1, s2) = (rf.eval_cases(x, y)[case], f.eval_cases(x, y)[case]);
                        peak = peak.max(s1.xx.abs().max(s1.yy.abs()).max(s1.xy.abs()));
                        err = err.max((s1.xx - s2.xx).abs().max((s1.yy - s2.yy).abs()).max((s1.xy - s2.xy).abs()));
                    }
                    *w = err / peak;
                }
                println!("e/D={ed} spec {:?} build {:?} (ref {:?}) rel.err fit/half/full = {:.2e} {:.2e} {:.2e}", (sp.n_bore, sp.n_mirror, sp.k_hole, sp.k_edge), dt, t_ref, worst[0], worst[1], worst[2]);
            }
        }
    }
}
