//! Through-thickness load distribution: the pin as a beam on the lug's own load-travel curve.
//!
//! The 2D lug model assumes every slice of the thickness carries the same bearing load. In a real
//! joint the pin bends between the clevis (or the single-shear tang) and the lug, so the lug faces
//! nearest the load path see more bearing load than the middle. This module couples the 2D model
//! to a Timoshenko pin beam: the lug is `n` slices, each a nonlinear spring `P'(s)` (load per
//! unit thickness against pin travel, measured on the 2D model itself, contact and clearance
//! included), the pin is a beam on those springs, and the clevis reactions are the applied
//! load. The result is the slice load distribution, the pin's bending moment and stresses, and the
//! peak slice load for a hoop-stress run of the 2D model at that load.

use crate::solve::{LoadCase, LugModel, PinSpec};

/// How the pin is carried outside the lug. Single shear is deliberately absent: the load and its
/// reaction then act on different planes, so the pin carries a net couple that only the members'
/// own eccentric bending can balance, which a pin-and-slices model cannot give.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shear {
    /// Lug between two clevis lugs: reactions `P/2` at `+-(t/2 + offset)`, `offset` being the
    /// distance from the lug face to the clevis lug's load centroid (about half its thickness).
    Double { offset: f64 },

}

#[derive(Debug, Clone, Copy)]
pub struct PinBending {
    /// Pin material (steel is typical).
    pub e_psi: f64,
    pub nu: f64,
    pub shear: Shear,
    /// Slices across the lug thickness (at least 3).
    pub slices: usize,
}

#[derive(Debug, Clone)]
pub struct ThicknessResult {
    /// Slice centres, `-t/2 .. t/2` across the lug thickness (in), the load side at `z > 0`.
    pub z: Vec<f64>,
    /// Bearing load per unit thickness in every slice (lbf/in).
    pub slice_load: Vec<f64>,
    /// Bearing stress `P' / D` of every slice (psi).
    pub bearing_stress: Vec<f64>,
    /// Largest slice load over the mean `P / t`: the through-thickness peaking factor (>= 1).
    pub peaking: f64,
    /// Largest pin bending moment (lbf-in) and its section stress `M D / 2 I` (psi).
    pub max_moment: f64,
    pub pin_bending_stress: f64,
    /// Largest shear force and the pin's mean shear stress `V / A` (psi) and peak `4 V / 3 A`.
    pub max_shear_force: f64,
    pub pin_shear_stress: f64,
    /// Pin deflection at the slice centres (in), the travel each slice sees.
    pub travel: Vec<f64>,
    /// Newton iterations of the beam solve.
    pub iterations: usize,
}

/// Monotone piecewise-cubic (Fritsch-Carlson) interpolation of `P'(s)`.
struct Law {
    s: Vec<f64>,
    p: Vec<f64>,
    d: Vec<f64>,
}

impl Law {
    fn new(s: Vec<f64>, p: Vec<f64>) -> Self {
        let n = s.len();
        let delta: Vec<f64> = (0..n - 1).map(|i| (p[i + 1] - p[i]) / (s[i + 1] - s[i])).collect();
        let mut d = vec![0.0; n];
        d[0] = delta[0];
        d[n - 1] = delta[n - 2];
        for i in 1..n - 1 {
            d[i] = if delta[i - 1] * delta[i] <= 0.0 { 0.0 } else { 0.5 * (delta[i - 1] + delta[i]) };
        }
        for i in 0..n - 1 {
            if delta[i].abs() < 1e-300 {
                d[i] = 0.0;
                d[i + 1] = 0.0;
                continue;
            }
            let (a, b) = (d[i] / delta[i], d[i + 1] / delta[i]);
            let r = a * a + b * b;
            if r > 9.0 {
                let tau = 3.0 / r.sqrt();
                d[i] = tau * a * delta[i];
                d[i + 1] = tau * b * delta[i];
            }
        }
        Self { s, p, d }
    }

    /// `(P'(s), dP'/ds)`; zero below the first point, linear beyond the last.
    fn eval(&self, s: f64) -> (f64, f64) {
        let n = self.s.len();
        if s <= self.s[0] {
            return (0.0, 0.0);
        }
        if s >= self.s[n - 1] {
            let k = self.d[n - 1].max(0.0);
            return (self.p[n - 1] + k * (s - self.s[n - 1]), k);
        }
        let i = self.s.partition_point(|x| *x <= s) - 1;
        let h = self.s[i + 1] - self.s[i];
        let t = (s - self.s[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        let (h00, h10, h01, h11) = (2.0 * t3 - 3.0 * t2 + 1.0, t3 - 2.0 * t2 + t, -2.0 * t3 + 3.0 * t2, t3 - t2);
        let p = h00 * self.p[i] + h10 * h * self.d[i] + h01 * self.p[i + 1] + h11 * h * self.d[i + 1];
        let dp = ((6.0 * t2 - 6.0 * t) * self.p[i] + (3.0 * t2 - 4.0 * t + 1.0) * h * self.d[i] + (-6.0 * t2 + 6.0 * t) * self.p[i + 1] + (3.0 * t2 - 2.0 * t) * h * self.d[i + 1]) / h;
        (p, dp.max(0.0))
    }
}

impl LugModel {
    /// Slice the lug thickness and let the pin bend. `case.load_lbf` is the whole pin load.
    pub fn through_thickness(&self, pin: PinSpec, case: LoadCase, bending: PinBending) -> Result<ThicknessResult, String> {
        let n = bending.slices.max(3);
        let t = self.geometry().thickness;
        let d = pin.diameter;
        if !(bending.e_psi > 0.0 && (0.0..0.5).contains(&bending.nu) && case.load_lbf > 0.0) {
            return Err("the pin material and a positive load are needed".into());
        }
        let mean = case.load_lbf / t;
        // The 2D model's own load-travel law, from just above zero to a few times the mean load.
        let fractions = [0.02, 0.1, 0.25, 0.5, 0.8, 1.0, 1.3, 1.7, 2.2, 2.8, 3.5];
        let mut s = vec![0.0];
        let mut p = vec![0.0];
        for f in fractions {
            let sol = self.solve(pin, LoadCase { load_lbf: case.load_lbf * f, angle_deg: case.angle_deg })?;
            let travel = sol.bearing_deflection.max(*s.last().unwrap() + 1e-9);
            s.push(travel);
            p.push(mean * f);
        }
        let law = Law::new(s, p);

        // Beam nodes: slice centres, then the support node(s) outside the lug on the load side
        // (and the other side for double shear).
        let w = t / n as f64;
        let z_slice: Vec<f64> = (0..n).map(|i| -0.5 * t + (i as f64 + 0.5) * w).collect();
        let Shear::Double { offset } = bending.shear;
        let (supports, shares) = (vec![-(0.5 * t + offset), 0.5 * t + offset], vec![0.5, 0.5]);
        let mut z_nodes = z_slice.clone();
        z_nodes.extend(&supports);
        let mut order: Vec<usize> = (0..z_nodes.len()).collect();
        order.sort_by(|&a, &b| z_nodes[a].total_cmp(&z_nodes[b]));
        let nn = z_nodes.len();
        // Position in the sorted beam of every original node.
        let mut at = vec![0usize; nn];
        for (pos, &orig) in order.iter().enumerate() {
            at[orig] = pos;
        }
        let zs: Vec<f64> = order.iter().map(|&o| z_nodes[o]).collect();

        let ndof = 2 * nn;
        let ei = bending.e_psi * std::f64::consts::PI * d.powi(4) / 64.0;
        let area = std::f64::consts::PI * d * d / 4.0;
        let g = bending.e_psi / (2.0 * (1.0 + bending.nu));
        let kappa = 6.0 * (1.0 + bending.nu) / (7.0 + 6.0 * bending.nu);
        let mut kb = vec![0.0; ndof * ndof];
        for e in 0..nn - 1 {
            let l = zs[e + 1] - zs[e];
            let phi = 12.0 * ei / (kappa * g * area * l * l);
            let c = ei / ((1.0 + phi) * l * l * l);
            let ke = [
                [12.0 * c, 6.0 * l * c, -12.0 * c, 6.0 * l * c],
                [6.0 * l * c, (4.0 + phi) * l * l * c, -6.0 * l * c, (2.0 - phi) * l * l * c],
                [-12.0 * c, -6.0 * l * c, 12.0 * c, -6.0 * l * c],
                [6.0 * l * c, (2.0 - phi) * l * l * c, -6.0 * l * c, (4.0 + phi) * l * l * c],
            ];
            let dofs = [2 * e, 2 * e + 1, 2 * e + 2, 2 * e + 3];
            for a in 0..4 {
                for b in 0..4 {
                    kb[dofs[a] * ndof + dofs[b]] += ke[a][b];
                }
            }
        }
        // External support forces.
        let mut rext = vec![0.0; ndof];
        for (k, &sh) in shares.iter().enumerate() {
            rext[2 * at[n + k]] += sh * case.load_lbf;
        }
        // Levenberg-Marquardt on K d + F(d) = R from the uniform guess.
        let slice_pos: Vec<usize> = (0..n).map(|i| at[i]).collect();
        let nu = ndof;
        let guess = {
            // Travel at which one slice carries the mean load.
            let mut lo = 0.0;
            let mut hi = law.s[law.s.len() - 1];
            for _ in 0..60 {
                let mid = 0.5 * (lo + hi);
                if law.eval(mid).0 < mean {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            0.5 * (lo + hi)
        };
        let mut x = vec![0.0; nu];
        for i in 0..nn {
            x[2 * i] = guess;
        }
        let floor = 1e-6 * bending.e_psi.min(1e8);
        // Residual and the diagonal contact tangent (and, in single shear, the tang row/column).
        let force = |xv: &[f64]| -> (Vec<f64>, Vec<f64>) {
            let mut res = vec![0.0; nu];
            for i in 0..ndof {
                let mut acc = -rext[i];
                for j in 0..ndof {
                    acc += kb[i * ndof + j] * xv[j];
                }
                res[i] = acc;
            }
            let mut tang = vec![0.0; nu];
            for &pos in &slice_pos {
                let (pl, dp) = law.eval(xv[2 * pos]);
                res[2 * pos] += pl * w; // the lug pushes the pin back
                tang[2 * pos] += dp * w;
            }
            (res, tang)
        };
        let norm = |v: &[f64]| v.iter().map(|q| q * q).sum::<f64>().sqrt();
        let (mut res, mut tang) = force(&x);
        let mut iterations = 0;
        let scale = case.load_lbf;
        // Levenberg-Marquardt: a lifted-off slice has no stiffness, so a plain Newton step can
        // throw the pin far away; the damping is tied to the beam's own stiffness.
        let diag_scale = (0..ndof).map(|i| kb[i * ndof + i]).fold(0.0f64, f64::max).max(1.0);
        let mut lambda = 1e-6;
        for it in 0..400 {
            iterations = it;
            let r0 = norm(&res);
            if r0 < 1e-9 * scale {
                break;
            }
            let mut accepted = false;
            for _ in 0..40 {
                let mut a = vec![0.0; nu * nu];
                for i in 0..ndof {
                    for j in 0..ndof {
                        a[i * nu + j] = kb[i * ndof + j];
                    }
                    a[i * nu + i] += tang[i] + if i % 2 == 0 { floor * w } else { 0.0 } + lambda * (kb[i * ndof + i] + diag_scale * 1e-3);
                }
                let mut rhs: Vec<f64> = res.iter().map(|v| -v).collect();
                if solve_dense(&mut a, &mut rhs, nu).is_err() {
                    lambda *= 10.0;
                    continue;
                }
                let trial: Vec<f64> = x.iter().zip(&rhs).map(|(q, dx)| q + dx).collect();
                let (r1, t1) = force(&trial);
                if norm(&r1) < r0 {
                    x = trial;
                    res = r1;
                    tang = t1;
                    lambda = (lambda / 4.0).max(1e-9);
                    accepted = true;
                    break;
                }
                lambda *= 5.0;
            }
            if !accepted {
                break;
            }
        }
        if norm(&res) > 1e-6 * scale {
            return Err("the pin bending solve did not converge".into());
        }
        let dvec = x[..ndof].to_vec();
        let travel: Vec<f64> = slice_pos.iter().map(|&pos| dvec[2 * pos]).collect();
        let slice_load: Vec<f64> = travel.iter().map(|&s| law.eval(s).0).collect();
        // Beam end actions per element from the converged displacements.
        let (mut max_m, mut max_v) = (0.0f64, 0.0f64);
        for e in 0..nn - 1 {
            let l = zs[e + 1] - zs[e];
            let phi = 12.0 * ei / (kappa * g * area * l * l);
            let c = ei / ((1.0 + phi) * l * l * l);
            let dd = [dvec[2 * e], dvec[2 * e + 1], dvec[2 * e + 2], dvec[2 * e + 3]];
            let v1 = c * (12.0 * dd[0] + 6.0 * l * dd[1] - 12.0 * dd[2] + 6.0 * l * dd[3]);
            let m1 = c * (6.0 * l * dd[0] + (4.0 + phi) * l * l * dd[1] - 6.0 * l * dd[2] + (2.0 - phi) * l * l * dd[3]);
            let m2 = c * (6.0 * l * dd[0] + (2.0 - phi) * l * l * dd[1] - 6.0 * l * dd[2] + (4.0 + phi) * l * l * dd[3]);
            max_v = max_v.max(v1.abs());
            max_m = max_m.max(m1.abs()).max(m2.abs());
        }
        let peak = slice_load.iter().cloned().fold(0.0, f64::max);
        Ok(ThicknessResult {
            z: z_slice,
            bearing_stress: slice_load.iter().map(|q| q / d).collect(),
            peaking: peak / mean,
            max_moment: max_m,
            pin_bending_stress: max_m * d / 2.0 / (ei / bending.e_psi),
            max_shear_force: max_v,
            pin_shear_stress: max_v / area,
            slice_load,
            travel,
            iterations,
        })
    }
}

/// Gaussian elimination with partial pivoting.
fn solve_dense(a: &mut [f64], b: &mut [f64], n: usize) -> Result<(), String> {
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| a[i * n + col].abs().total_cmp(&a[j * n + col].abs())).unwrap();
        if a[piv * n + col].abs() < 1e-300 {
            return Err("singular pin beam system".into());
        }
        for k in 0..n {
            a.swap(col * n + k, piv * n + k);
        }
        b.swap(col, piv);
        for r in col + 1..n {
            let f = a[r * n + col] / a[col * n + col];
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
        b[r] = v / a[r * n + r];
    }
    Ok(())
}
