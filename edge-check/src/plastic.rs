//! Elastic-perfectly-plastic limit load of the same plate the elastic FE
//! models: J2 (von Mises) plane-stress plasticity with flow stress
//! `sigma0`, no hardening, small strain.
//!
//! The fit pressure is applied first as a dead load; the pin load (half
//! cosine, contact lost on the back side - the conservative distribution)
//! is then increased in adaptive steps until the equilibrium iteration no
//! longer converges: that load is the collapse load. For a perfectly
//! plastic body this is exactly the limit load (up to the step tolerance),
//! with no assumption about *where* or *how* the ligament fails - the
//! plastic zone finds its own path. That is what the mean-shear limit in
//! the other models assumes instead of computing.
//!
//! Solver: modified Newton, i.e. initial-stress iteration with the elastic
//! stiffness factored once in [`FemSolution::solve`] (a back-substitution
//! per iteration, no refactorisation). Stresses come from a closed-form
//! plane-stress return map (the mean and deviatoric modes decouple).

use crate::fem::{shape_q9, FemSolution, GAUSS3};

#[derive(Debug, Clone, PartialEq)]
pub enum Collapse {
    /// Collapse pin load (lbf), to within the step tolerance.
    Limit(f64),
    /// No collapse found up to this pin load (lbf): the kinematic upper
    /// bound was exceeded several times over, so something is wrong or the
    /// load path is not a mechanism (reported, not guessed).
    Unbounded(f64),
    /// The fit pressure alone does not equilibrate.
    FitOnlyFails,
}

struct Gp {
    dnx: [f64; 9],
    dny: [f64; 9],
    w: f64,
}

/// Plastic strain state `(exx, eyy, ezz, gxy)`; `ezz` is only used under
/// plane strain.
type Ep = [f64; 4];

#[derive(Clone, Copy)]
struct Elastic {
    e: f64,
    nu: f64,
    plane_strain: bool,
}

/// J2 return map. `eps` is the total in-plane strain `(exx, eyy, gxy)`
/// (`ezz` is zero under plane strain and free under plane stress);
/// returns the in-plane stress `(sxx, syy, txy)` and the new plastic strain.
#[inline]
fn return_map(eps: [f64; 3], ep: Ep, c: Elastic, sigma0: f64) -> ([f64; 3], Ep) {
    if c.plane_strain {
        return return_map_plane_strain(eps, ep, c, sigma0);
    }
    let (e, nu) = (c.e, c.nu);
    let f = e / (1.0 - nu * nu);
    let g = e / (2.0 * (1.0 + nu));
    let (ex, ey, gx) = (eps[0] - ep[0], eps[1] - ep[1], eps[2] - ep[3]);
    let sxx = f * (ex + nu * ey);
    let syy = f * (ey + nu * ex);
    let txy = g * gx;
    let m = 0.5 * (sxx + syy);
    let dx = 0.5 * (sxx - syy);
    let q2 = dx * dx + txy * txy; // |q|^2
    let vm2 = m * m + 3.0 * q2;
    if vm2 <= sigma0 * sigma0 {
        return ([sxx, syy, txy], ep);
    }
    // Mean mode stiffness K' = E/(1-nu), deviatoric 2G = E/(1+nu); solve
    // g(d) = (m/(1+d K'))^2 + 3 q2/(1+6 G d)^2 - sigma0^2 = 0, convex and
    // decreasing, so Newton from d = 0 converges monotonically.
    let kp = e / (1.0 - nu);
    let g6 = 6.0 * g;
    let s2 = sigma0 * sigma0;
    let mut d = 0.0;
    for _ in 0..40 {
        let (a, b) = (1.0 + d * kp, 1.0 + d * g6);
        let val = m * m / (a * a) + 3.0 * q2 / (b * b) - s2;
        let dval = -2.0 * m * m * kp / (a * a * a) - 6.0 * q2 * g6 / (b * b * b);
        if dval == 0.0 {
            break;
        }
        let step = val / dval;
        d -= step;
        if step.abs() <= 1e-14 * (1.0 + d.abs()) {
            break;
        }
    }
    let (a, b) = (1.0 + d * kp, 1.0 + d * g6);
    let (m2, dx2, t2) = (m / a, dx / b, txy / b);
    let (dm, dd, ds) = ((m - m2) / kp, (dx - dx2) / (2.0 * g), (txy - t2) / (2.0 * g));
    ([m2 + dx2, m2 - dx2, t2], [ep[0] + dm + dd, ep[1] + dm - dd, 0.0, ep[3] + 2.0 * ds])
}

/// Plane strain (`eps_zz = 0`): 3D J2 radial return on the deviator
/// (the mean stress is unchanged by plastic flow).
fn return_map_plane_strain(eps: [f64; 3], ep: Ep, c: Elastic, sigma0: f64) -> ([f64; 3], Ep) {
    let g = c.e / (2.0 * (1.0 + c.nu));
    let lam = c.e * c.nu / ((1.0 + c.nu) * (1.0 - 2.0 * c.nu));
    let (ex, ey, ez, gx) = (eps[0] - ep[0], eps[1] - ep[1], -ep[2], eps[2] - ep[3]);
    let tr = ex + ey + ez;
    let (sxx, syy, szz, txy) = (lam * tr + 2.0 * g * ex, lam * tr + 2.0 * g * ey, lam * tr + 2.0 * g * ez, g * gx);
    let p = (sxx + syy + szz) / 3.0;
    let (dx, dy, dz) = (sxx - p, syy - p, szz - p);
    let vm = (1.5 * (dx * dx + dy * dy + dz * dz + 2.0 * txy * txy)).sqrt();
    if vm <= sigma0 {
        return ([sxx, syy, txy], ep);
    }
    let k = sigma0 / vm; // deviator scale
    let (dx2, dy2, t2) = (dx * k, dy * k, txy * k);
    let r = (1.0 - k) / (2.0 * g);
    ([p + dx2, p + dy2, t2], [ep[0] + dx * r, ep[1] + dy * r, ep[2] + dz * r, ep[3] + 2.0 * txy * r])
}

impl FemSolution {
    /// Collapse pin load for dead fit pressure `p_fit` (psi) and flow
    /// stress `sigma0` (psi).
    #[allow(clippy::needless_range_loop)] // dof-indexed vector updates read clearer than zips
    pub fn collapse_load(&self, p_fit: f64, sigma0: f64) -> Collapse {
        let ndof = 2 * self.nodes.len();
        let gps: Vec<Gp> = self
            .elems
            .iter()
            .flat_map(|conn| {
                let xy: Vec<[f64; 2]> = conn.iter().map(|&n| self.nodes[n]).collect();
                let mut out = Vec::with_capacity(9);
                for &(gx, wx) in &GAUSS3 {
                    for &(gy, wy) in &GAUSS3 {
                        let (_, dxi, deta) = shape_q9(gx, gy);
                        let (mut j11, mut j12, mut j21, mut j22) = (0.0, 0.0, 0.0, 0.0);
                        for n in 0..9 {
                            j11 += dxi[n] * xy[n][0];
                            j12 += dxi[n] * xy[n][1];
                            j21 += deta[n] * xy[n][0];
                            j22 += deta[n] * xy[n][1];
                        }
                        let det = j11 * j22 - j12 * j21;
                        let inv = [[j22 / det, -j12 / det], [-j21 / det, j11 / det]];
                        let (mut dnx, mut dny) = ([0.0; 9], [0.0; 9]);
                        for n in 0..9 {
                            dnx[n] = inv[0][0] * dxi[n] + inv[0][1] * deta[n];
                            dny[n] = inv[1][0] * dxi[n] + inv[1][1] * deta[n];
                        }
                        out.push(Gp { dnx, dny, w: det * wx * wy });
                    }
                }
                out
            })
            .collect();

        let (e, nu) = (self.e_psi, self.nu);
        let elastic = Elastic { e, nu, plane_strain: self.plane_strain };
        let mut u = vec![0.0; ndof];
        let mut ep: Vec<Ep> = vec![[0.0f64; 4]; gps.len()];
        let mut ep_trial = ep.clone();
        let mut f_int = vec![0.0; ndof];
        let mut r = vec![0.0; ndof];

        // Internal force and trial plastic strains for displacement `u`.
        let internal = |u: &[f64], ep: &[Ep], ep_trial: &mut [Ep], f_int: &mut [f64]| {
            f_int.iter_mut().for_each(|v| *v = 0.0);
            for (ie, conn) in self.elems.iter().enumerate() {
                for g in 0..9 {
                    let gp = &gps[ie * 9 + g];
                    let (mut exx, mut eyy, mut gxy) = (0.0, 0.0, 0.0);
                    for k in 0..9 {
                        let (ux, uy) = (u[2 * conn[k]], u[2 * conn[k] + 1]);
                        exx += gp.dnx[k] * ux;
                        eyy += gp.dny[k] * uy;
                        gxy += gp.dny[k] * ux + gp.dnx[k] * uy;
                    }
                    let (s, epn) = return_map([exx, eyy, gxy], ep[ie * 9 + g], elastic, sigma0);
                    ep_trial[ie * 9 + g] = epn;
                    for k in 0..9 {
                        f_int[2 * conn[k]] += (gp.dnx[k] * s[0] + gp.dny[k] * s[2]) * gp.w;
                        f_int[2 * conn[k] + 1] += (gp.dny[k] * s[1] + gp.dnx[k] * s[2]) * gp.w;
                    }
                }
            }
        };

        // Modified-Newton equilibrium at external load `f_ext`; on success
        // `ep` holds the committed plastic strains.
        let mut equilibrate = |f_ext: &[f64], u: &mut Vec<f64>, ep: &mut Vec<Ep>| -> bool {
            let fnorm = f_ext.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
            for _ in 0..60 {
                internal(u, ep, &mut ep_trial, &mut f_int);
                for i in 0..ndof {
                    r[i] = if self.constrained[i] { 0.0 } else { f_ext[i] - f_int[i] };
                }
                let rn = r.iter().map(|v| v * v).sum::<f64>().sqrt();
                if rn <= 5e-4 * fnorm {
                    ep.copy_from_slice(&ep_trial);
                    return true;
                }
                if !rn.is_finite() {
                    return false;
                }
                self.k_factor.solve_in_place(&mut r);
                for i in 0..ndof {
                    u[i] += r[i];
                }
            }
            false
        };

        let mut f_ext = vec![0.0; ndof];

        // Everything below first yield is linear elastic and already solved
        // (unit fit + unit half-pin fields): jump straight to 90 % of the
        // load at which the first Gauss point reaches the flow stress.
        // `vm^2(lambda) = A lambda^2 + B lambda + C` per Gauss point.
        let strain_of = |ue: &[f64], conn: &[usize; 9], gp: &Gp| {
            let (mut exx, mut eyy, mut gxy) = (0.0, 0.0, 0.0);
            for k in 0..9 {
                let (ux, uy) = (ue[2 * conn[k]], ue[2 * conn[k] + 1]);
                exx += gp.dnx[k] * ux;
                eyy += gp.dny[k] * uy;
                gxy += gp.dny[k] * ux + gp.dnx[k] * uy;
            }
            // In-plane constants of the FE model (primed under plane strain).
            let (ee, nn) = if self.plane_strain { (e / (1.0 - nu * nu), nu / (1.0 - nu)) } else { (e, nu) };
            let f = ee / (1.0 - nn * nn);
            let (sxx, syy) = (f * (exx + nn * eyy), f * (eyy + nn * exx));
            let szz = if self.plane_strain { nu * (sxx + syy) } else { 0.0 };
            [sxx, syy, ee / (2.0 * (1.0 + nn)) * gxy, szz]
        };
        // Bilinear form of the von Mises equivalent stress squared.
        let vmf = |a: [f64; 4], b: [f64; 4]| {
            0.5 * ((a[0] - a[1]) * (b[0] - b[1]) + (a[1] - a[3]) * (b[1] - b[3]) + (a[3] - a[0]) * (b[3] - b[0])) + 3.0 * a[2] * b[2]
        };
        let mut lambda_y = f64::INFINITY;
        let mut fit_alone_yields = false;
        for (ie, conn) in self.elems.iter().enumerate() {
            for g in 0..9 {
                let gp = &gps[ie * 9 + g];
                let sf = strain_of(self.u_unit(0), conn, gp).map(|v| v * p_fit);
                let sp = strain_of(self.u_unit(1), conn, gp);
                let (a, b, c) = (vmf(sp, sp), 2.0 * vmf(sf, sp), vmf(sf, sf) - sigma0 * sigma0);
                if c >= 0.0 {
                    fit_alone_yields = true;
                    continue;
                }
                let disc = b * b - 4.0 * a * c;
                if a > 0.0 && disc >= 0.0 {
                    let root = (-b + disc.sqrt()) / (2.0 * a);
                    if root > 0.0 {
                        lambda_y = lambda_y.min(root);
                    }
                }
            }
        }
        let (mut lambda, jumped) = if !fit_alone_yields && lambda_y.is_finite() {
            let l0 = 0.9 * lambda_y;
            for k in 0..ndof {
                u[k] = p_fit * self.u_unit(0)[k] + l0 * self.u_unit(1)[k];
            }
            (l0, true)
        } else {
            (0.0, false)
        };
        if !jumped {
            // Fit pressure as a dead load, in a few steps.
            for i in 1..=4 {
                let scale = p_fit * i as f64 / 4.0;
                for k in 0..ndof {
                    f_ext[k] = scale * self.loads[0][k];
                }
                if !equilibrate(&f_ext, &mut u, &mut ep) {
                    return Collapse::FitOnlyFails;
                }
            }
        }

        // Displacement control: prescribe the x displacement of the bore's
        // loaded point and solve for the pin load. Under load control a
        // perfectly plastic body has no equilibrium past collapse, so "the
        // iteration stopped converging" is ambiguous (slow or impossible);
        // under displacement control equilibrium always exists and the load
        // simply plateaus at the limit load.
        let c_dof = 2 * self.loaded_point_node();
        let u1c = self.u_unit(1)[c_dof];
        if u1c == 0.0 {
            return Collapse::FitOnlyFails;
        }
        let mut f_pin_part = vec![0.0; ndof];
        let mut step_ratio = 1.6f64;
        let mut delta = u[c_dof];
        let mut prev_lambda = lambda;
        let mut plateau_hits = 0;
        let mut plateau_seen = false;
        for _ in 0..80 {
            // Next target: grow the displacement beyond the first-yield level.
            delta = if delta.abs() < 1e-30 { u1c } else { delta * step_ratio };
            let (u_save, ep_save, lam_save) = (u.clone(), ep.clone(), lambda);
            let mut ok = false;
            // Anderson-accelerated initial-stress iteration on x = (u, lambda*|u1c|).
            // The elastic back-substitution is the fixed-point map; Anderson
            // mixing cuts the iteration count several-fold near collapse
            // where the plain map converges linearly with ratio -> 1.
            const MEM: usize = 8;
            let mut hist_dx: Vec<Vec<f64>> = Vec::new();
            let mut hist_dg: Vec<Vec<f64>> = Vec::new();
            let mut prev: Option<(Vec<f64>, Vec<f64>)> = None;
            let lam_scale = u1c.abs();
            for _it in 0..80 {
                for k in 0..ndof {
                    f_pin_part[k] = self.loads[0][k] * p_fit + lambda * self.loads[1][k];
                }
                internal(&u, &ep, &mut ep_trial, &mut f_int);
                for i in 0..ndof {
                    r[i] = if self.constrained[i] { 0.0 } else { f_pin_part[i] - f_int[i] };
                }
                let fnorm = f_pin_part.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
                let rn = r.iter().map(|v| v * v).sum::<f64>().sqrt();
                if !rn.is_finite() {
                    break;
                }
                let disp_err = (u[c_dof] - delta).abs() / delta.abs();
                if rn <= 2e-3 * fnorm && disp_err < 1e-3 {
                    ep.copy_from_slice(&ep_trial);
                    ok = true;
                    break;
                }
                self.k_factor.solve_in_place(&mut r);
                let dlam = (delta - u[c_dof] - r[c_dof]) / u1c;
                // g = G(x) - x
                let mut g = Vec::with_capacity(ndof + 1);
                for i in 0..ndof {
                    g.push(r[i] + dlam * self.u_unit(1)[i]);
                }
                g.push(dlam * lam_scale);
                let mut x: Vec<f64> = u.clone();
                x.push(lambda * lam_scale);
                if let Some((px, pg)) = &prev {
                    hist_dx.push(x.iter().zip(px).map(|(a, b)| a - b).collect());
                    hist_dg.push(g.iter().zip(pg).map(|(a, b)| a - b).collect());
                    if hist_dx.len() > MEM {
                        hist_dx.remove(0);
                        hist_dg.remove(0);
                    }
                }
                prev = Some((x.clone(), g.clone()));
                let mut x_next: Vec<f64> = x.iter().zip(&g).map(|(a, b)| a + b).collect();
                let m = hist_dg.len();
                if m > 0 {
                    // min_gamma |g - sum gamma_j dg_j|
                    let n = ndof + 1;
                    let mut a_mat = vec![0.0; n * m];
                    for (j, dgj) in hist_dg.iter().enumerate() {
                        for i in 0..n {
                            a_mat[i * m + j] = dgj[i];
                        }
                    }
                    let gamma = crate::linalg::least_squares(&a_mat, n, m, &g, 1e-10);
                    for (j, gj) in gamma.iter().enumerate() {
                        for i in 0..n {
                            x_next[i] -= gj * (hist_dx[j][i] + hist_dg[j][i]);
                        }
                    }
                }
                u.copy_from_slice(&x_next[..ndof]);
                lambda = x_next[ndof] / lam_scale;
            }
            if !ok {
                // Slow or impossible to equilibrate at this displacement. If the
                // load the iteration was heading for is already within 2 % of the
                // last converged load, the load has plateaued: that is the limit.
                if lam_save > 0.0 && (lambda - lam_save).abs() < 0.02 * lam_save && plateau_seen {
                    return Collapse::Limit(lam_save);
                }
                u = u_save;
                ep = ep_save;
                lambda = lam_save;
                step_ratio = 1.0 + (step_ratio - 1.0) * 0.5;
                delta /= 1.0 + (step_ratio - 1.0) * 2.0; // retry nearer
                if step_ratio < 1.01 {
                    break;
                }
                continue;
            }
            plateau_seen = prev_lambda > 0.0 && (lambda - prev_lambda).abs() < 0.03 * lambda;
            if lambda > 0.0 && (lambda - prev_lambda).abs() < 2.5e-3 * lambda {
                plateau_hits += 1;
                if plateau_hits >= 2 {
                    return Collapse::Limit(lambda);
                }
            } else {
                plateau_hits = 0;
            }
            prev_lambda = lambda;
        }
        Collapse::Limit(lambda)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::fem::MeshSpec;
    use crate::types::Geometry;

    const E: f64 = 10.3e6;
    const NU: f64 = 0.33;

    const PS: Elastic = Elastic { e: E, nu: NU, plane_strain: false };
    const PE: Elastic = Elastic { e: E, nu: NU, plane_strain: true };

    fn vm_plane_stress(s: [f64; 3]) -> f64 {
        (s[0] * s[0] - s[0] * s[1] + s[1] * s[1] + 3.0 * s[2] * s[2]).sqrt()
    }

    #[test]
    fn return_map_is_elastic_below_yield_and_lands_on_the_yield_surface_above_it() {
        let s0 = 50_000.0;
        let (s, ep) = return_map([1e-3, 0.0, 0.0], [0.0; 4], PS, s0);
        assert_eq!(ep, [0.0; 4]);
        assert!(s[0] > 0.0 && vm_plane_stress(s) < s0);
        for eps in [[0.01, 0.0, 0.0], [0.004, -0.003, 0.02], [0.0, 0.0, 0.05], [0.01, 0.01, 0.0]] {
            let (s, ep) = return_map(eps, [0.0; 4], PS, s0);
            assert!((vm_plane_stress(s) - s0).abs() < 1e-6 * s0, "{eps:?}: vm {}", vm_plane_stress(s));
            assert!(ep.iter().any(|v| *v != 0.0));
        }
    }

    #[test]
    fn pure_shear_flows_at_sigma0_over_root_three_in_both_states() {
        let s0 = 60_000.0;
        for c in [PS, PE] {
            let (s, _) = return_map([0.0, 0.0, 0.05], [0.0; 4], c, s0);
            assert!((s[2] - s0 / 3f64.sqrt()).abs() < 1e-6 * s0, "{s:?}");
        }
    }

    #[test]
    fn plane_strain_return_map_is_elastic_below_yield_and_has_zero_volumetric_plastic_strain() {
        let s0 = 50_000.0;
        let (_, ep) = return_map([1e-3, 0.0, 0.0], [0.0; 4], PE, s0);
        assert_eq!(ep, [0.0; 4]);
        for eps in [[0.01, 0.0, 0.0], [0.004, -0.003, 0.02], [0.02, 0.02, 0.0]] {
            let (_, ep) = return_map(eps, [0.0; 4], PE, s0);
            assert!((ep[0] + ep[1] + ep[2]).abs() < 1e-12, "plastic flow must be incompressible: {ep:?}");
            assert!(ep.iter().any(|v| *v != 0.0));
        }
    }

    #[test]
    fn plane_strain_equibiaxial_state_yields_on_the_deviator_not_the_mean() {
        // Equibiaxial in-plane strain with eps_zz = 0: sigma_zz = nu-related, the
        // deviator is what saturates; the in-plane mean stress stays elastic.
        let s0 = 50_000.0;
        let (s, ep) = return_map([0.02, 0.02, 0.0], [0.0; 4], PE, s0);
        assert!(ep[0] == ep[1] && ep[0].abs() > 0.0);
        assert!(s[0] > s0, "plane strain carries more than sigma0 in-plane under confinement: {s:?}");
    }

    fn geom(ed: f64) -> Geometry {
        let a = 0.25;
        let e = ed * 2.0 * a;
        let reach = (3.0 * e).max(10.0 * a);
        Geometry { bore_radius: a, edge: e, thickness: 0.5, plate_far: reach, plate_half_height: reach, plane_angle_deg: 40.0 }
    }

    fn limit(ed: f64, p_fit: f64, s0: f64, mesh: MeshSpec, plane_strain: bool) -> f64 {
        let sol = FemSolution::solve_with(&geom(ed), E, NU, mesh, plane_strain).unwrap();
        match sol.collapse_load(p_fit, s0) {
            Collapse::Limit(l) => l,
            other => panic!("e/D={ed}: {other:?}"),
        }
    }

    const COARSE: MeshSpec = MeshSpec { n_radial: 10, n_arc: [3, 3, 10], grade: 2.0 };

    #[test]
    fn collapse_load_scales_with_the_flow_stress_for_perfect_plasticity() {
        for ps in [false, true] {
            let (a, b) = (limit(2.0, 0.0, 40_000.0, COARSE, ps), limit(2.0, 0.0, 80_000.0, COARSE, ps));
            assert!((b / a - 2.0).abs() < 0.03, "plane_strain={ps}: {a} {b}");
        }
    }

    #[test]
    fn collapse_lies_between_first_yield_and_the_parallel_plane_upper_bound() {
        let m = MeshSpec { n_radial: 12, n_arc: [3, 4, 12], grade: 2.0 };
        let s0 = 77_000.0;
        for ed in [1.25, 1.5, 2.0, 3.0] {
            let g = geom(ed);
            let ub = 2.0 * (s0 / 3f64.sqrt()) * g.edge * g.thickness;
            // Plane stress: elastic first-yield load from the elastic field.
            let sol = FemSolution::solve(&g, E, NU, m).unwrap();
            let mut vm_peak: f64 = 0.0;
            for k in 0..=90 {
                let phi = std::f64::consts::PI * k as f64 / 90.0;
                let s = sol.stress_at(crate::fem::UnitCase::PinHalf, g.edge + g.bore_radius * phi.cos(), g.bore_radius * phi.sin()).unwrap();
                vm_peak = vm_peak.max(s.von_mises());
            }
            let first_yield = s0 / vm_peak;
            for ps in [false, true] {
                let pc = limit(ed, 0.0, s0, m, ps);
                assert!(pc <= ub * 1.005, "e/D={ed} plane_strain={ps}: collapse {pc} above the kinematic bound {ub}");
                assert!(pc >= first_yield * 0.98, "e/D={ed} plane_strain={ps}: collapse {pc} below first yield {first_yield}");
            }
        }
    }

    #[test]
    fn collapse_load_grows_with_edge_distance_and_falls_with_fit_pressure() {
        let s0 = 77_000.0;
        let mut prev = 0.0;
        for ed in [1.0, 1.5, 2.0, 3.0] {
            let pc = limit(ed, 0.0, s0, COARSE, true);
            assert!(pc > prev, "e/D={ed}: {pc} <= {prev}");
            prev = pc;
        }
        assert!(limit(2.0, 8000.0, s0, COARSE, true) < limit(2.0, 0.0, s0, COARSE, true));
    }

    #[test]
    fn collapse_load_is_mesh_converged_to_a_few_percent() {
        let s0 = 77_000.0;
        let coarse = limit(1.5, 0.0, s0, COARSE, true);
        let fine = limit(1.5, 0.0, s0, MeshSpec { n_radial: 16, n_arc: [4, 6, 18], grade: 2.0 }, true);
        assert!((coarse / fine - 1.0).abs() < 0.05, "{coarse} vs {fine}");
    }
}
