//! Nonlinear material points: J2 plasticity with isotropic hardening, in small strain and finite
//! strain.
//!
//! * **Small strain**: radial return on the deviatoric stress with the exact piecewise-linear
//!   hardening increment (`Hardening::plastic_increment`) and the closed-form consistent tangent
//!   `C = K 1(x)1 + 2 G theta (I - 1/3 1(x)1) - 6 G^2 (1/(3G + H') - d/q) n(x)n`.
//! * **Finite strain**: total-Lagrangian logarithmic-strain (Hencky) J2 with multiplicative
//!   plasticity. The state is the inverse plastic right Cauchy-Green tensor `Cp^-1` and the
//!   equivalent plastic strain; the trial elastic left Cauchy-Green `Be = F Cp^-1 F^T` is
//!   decomposed spectrally, the return map acts on the principal logarithmic strains, and the
//!   consistent tangent `dP/dF` is **analytic**: the derivative of the isotropic tensor function
//!   through divided differences of the principal values (no finite differences, no perturbation of
//!   repeated eigenvalues). Finite differences appear only in the tests, as the oracle.
//!
//! Tensors are `3 x 3` arrays and fourth-order tangents `9 x 9` matrices indexed
//! `[3 i + J][3 k + L]` for `d S_iJ / d H_kL` (`S` = Cauchy stress in small strain, first
//! Piola-Kirchhoff in finite strain; `H` = displacement gradient), which is exactly what the
//! element assembly contracts with the shape-function gradients.
//!
//! Plane stress solves the out-of-plane strain (stretch) from `S_zz = 0` by a scalar Newton
//! iteration and condenses the tangent exactly.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use mechanics_core::hardening::Hardening;

pub type M3 = [[f64; 3]; 3];
pub type T4 = [[f64; 9]; 9];
/// `[xx, yy, zz, xy, yz, zx]` of the plastic strain (small strain, tensor shear) or of `Cp^-1`
/// (finite strain), then the equivalent plastic strain.
pub type GpState = [f64; 7];

/// J2 plasticity attached to a mesh block.
#[derive(Debug, Clone, Copy)]
pub struct J2 {
    pub law: Hardening,
    /// Finite-strain (total-Lagrangian Hencky) formulation instead of small strain.
    pub large_strain: bool,
}

impl J2 {
    pub fn small(law: Hardening) -> Self {
        Self { law, large_strain: false }
    }

    pub fn finite(law: Hardening) -> Self {
        Self { law, large_strain: true }
    }

    /// Hyperelastic Hencky material with no yielding (a large-strain elastic body in a finite-strain
    /// model).
    pub fn elastic_finite() -> Self {
        Self { law: Hardening::linear(1.0e30, 0.0, 1.0).expect("valid table"), large_strain: true }
    }

    pub fn initial_state(&self) -> GpState {
        if self.large_strain {
            [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]
        } else {
            [0.0; 7]
        }
    }
}

/// Shear and bulk modulus.
#[derive(Debug, Clone, Copy)]
pub struct Moduli {
    pub g: f64,
    pub k: f64,
}

impl Moduli {
    pub fn new(e: f64, nu: f64) -> Self {
        Self { g: e / (2.0 * (1.0 + nu)), k: e / (3.0 * (1.0 - 2.0 * nu)) }
    }
}

/// Result of a material-point update.
#[derive(Debug, Clone, Copy)]
pub struct Update {
    /// Cauchy stress (small strain) or first Piola-Kirchhoff stress (finite strain).
    pub stress: M3,
    /// `dS / dH`.
    pub tangent: T4,
    pub state: GpState,
}

// ------------------------------------------------------------------ small 3x3 algebra

pub const I3: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn mul(a: &M3, b: &M3) -> M3 {
    let mut c = [[0.0; 3]; 3];
    for i in 0..3 {
        for k in 0..3 {
            for j in 0..3 {
                c[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    c
}

pub fn transpose(a: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[j][i]))
}

pub fn det(a: &M3) -> f64 {
    a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0]) + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
}

pub fn inverse(a: &M3) -> Option<M3> {
    let d = det(a);
    if d == 0.0 || !d.is_finite() {
        return None;
    }
    let r = 1.0 / d;
    Some([
        [(a[1][1] * a[2][2] - a[1][2] * a[2][1]) * r, (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * r, (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * r],
        [(a[1][2] * a[2][0] - a[1][0] * a[2][2]) * r, (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * r, (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * r],
        [(a[1][0] * a[2][1] - a[1][1] * a[2][0]) * r, (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * r, (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * r],
    ])
}

fn sym_from_voigt(v: &[f64]) -> M3 {
    [[v[0], v[3], v[5]], [v[3], v[1], v[4]], [v[5], v[4], v[2]]]
}

fn voigt_from_sym(m: &M3) -> [f64; 6] {
    [m[0][0], m[1][1], m[2][2], 0.5 * (m[0][1] + m[1][0]), 0.5 * (m[1][2] + m[2][1]), 0.5 * (m[2][0] + m[0][2])]
}

/// Eigen-decomposition of a symmetric matrix by cyclic Jacobi: eigenvalues and eigenvectors
/// (`vec[k]` is the unit eigenvector of `val[k]`).
pub fn sym_eig(a: &M3) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut m = *a;
    let mut v = I3; // columns are the eigenvectors
    for _ in 0..40 {
        let off = m[0][1] * m[0][1] + m[0][2] * m[0][2] + m[1][2] * m[1][2];
        let scale = m[0][0].abs() + m[1][1].abs() + m[2][2].abs();
        if off <= 1e-32 * (scale * scale).max(1e-300) {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if m[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (m[q][q] - m[p][p]) / (2.0 * m[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            // m <- J^T m J with the rotation in the (p, q) plane.
            for k in 0..3 {
                let (mkp, mkq) = (m[k][p], m[k][q]);
                m[k][p] = c * mkp - s * mkq;
                m[k][q] = s * mkp + c * mkq;
            }
            for k in 0..3 {
                let (mpk, mqk) = (m[p][k], m[q][k]);
                m[p][k] = c * mpk - s * mqk;
                m[q][k] = s * mpk + c * mqk;
            }
            for k in 0..3 {
                let (vkp, vkq) = (v[k][p], v[k][q]);
                v[k][p] = c * vkp - s * vkq;
                v[k][q] = s * vkp + c * vkq;
            }
        }
    }
    ([m[0][0], m[1][1], m[2][2]], [[v[0][0], v[1][0], v[2][0]], [v[0][1], v[1][1], v[2][1]], [v[0][2], v[1][2], v[2][2]]])
}

/// `(ln a - ln b) / (a - b)` for positive `a`, `b`, stable as `a -> b` (atanh series).
fn dlog(a: f64, b: f64) -> f64 {
    let t = (a - b) / (a + b);
    if t.abs() < 1e-3 {
        let t2 = t * t;
        (2.0 / (a + b)) * (1.0 + t2 / 3.0 + t2 * t2 / 5.0 + t2 * t2 * t2 / 7.0)
    } else {
        (a.ln() - b.ln()) / (a - b)
    }
}

// ------------------------------------------------------------------ the deviatoric radial return

/// Principal-space radial return: for trial principal (log-)strains `eps` returns the principal
/// stresses, the principal tangent `d tau_a / d eps_b`, the elastic strains after return and the
/// plastic increment `d` (0 if elastic).
struct Principal {
    tau: [f64; 3],
    a: [[f64; 3]; 3],
    eps_e: [f64; 3],
    d: f64,
}

fn return_principal(mo: Moduli, law: &Hardening, eps: [f64; 3], ep: f64) -> Principal {
    let tr = eps[0] + eps[1] + eps[2];
    let dev = [eps[0] - tr / 3.0, eps[1] - tr / 3.0, eps[2] - tr / 3.0];
    let dn = (dev[0] * dev[0] + dev[1] * dev[1] + dev[2] * dev[2]).sqrt();
    let q = 1.5f64.sqrt() * 2.0 * mo.g * dn;
    let (mut theta, mut c, mut d) = (1.0, 0.0, 0.0);
    if q > law.stress(ep) && dn > 0.0 {
        d = law.plastic_increment(ep, q, mo.g);
        theta = 1.0 - 3.0 * mo.g * d / q;
        let hp = law.slope(ep + d);
        c = 6.0 * mo.g * mo.g * (1.0 / (3.0 * mo.g + hp) - d / q);
    }
    let n: [f64; 3] = if dn > 0.0 { std::array::from_fn(|i| dev[i] / dn) } else { [0.0; 3] };
    let tau = std::array::from_fn(|i| 2.0 * mo.g * theta * dev[i] + mo.k * tr);
    let a = std::array::from_fn(|i| std::array::from_fn(|j| mo.k + 2.0 * mo.g * theta * (if i == j { 1.0 } else { 0.0 } - 1.0 / 3.0) - c * n[i] * n[j]));
    let eps_e = std::array::from_fn(|i| theta * dev[i] + tr / 3.0);
    Principal { tau, a, eps_e, d }
}

// ------------------------------------------------------------------ small strain

/// Small-strain J2 update for the symmetric strain tensor `eps` (tensor shear components).
pub fn small_strain_update(mo: Moduli, law: &Hardening, eps: &M3, old: &GpState) -> Update {
    let ep_t = sym_from_voigt(&old[..6]);
    let ep = old[6];
    let mut e = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            e[i][j] = eps[i][j] - ep_t[i][j];
        }
    }
    let tr = e[0][0] + e[1][1] + e[2][2];
    let mut dev = e;
    for i in 0..3 {
        dev[i][i] -= tr / 3.0;
    }
    let dn = dev.iter().flatten().map(|v| v * v).sum::<f64>().sqrt();
    let q = 1.5f64.sqrt() * 2.0 * mo.g * dn;
    let (mut theta, mut c, mut d) = (1.0, 0.0, 0.0);
    let plastic = q > law.stress(ep) && dn > 0.0;
    if plastic {
        d = law.plastic_increment(ep, q, mo.g);
        theta = 1.0 - 3.0 * mo.g * d / q;
        c = 6.0 * mo.g * mo.g * (1.0 / (3.0 * mo.g + law.slope(ep + d)) - d / q);
    }
    let n: M3 = if dn > 0.0 { std::array::from_fn(|i| std::array::from_fn(|j| dev[i][j] / dn)) } else { [[0.0; 3]; 3] };
    let mut stress = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            stress[i][j] = 2.0 * mo.g * theta * dev[i][j] + if i == j { mo.k * tr } else { 0.0 };
        }
    }
    let mut state = *old;
    if plastic {
        let f = d * 1.5f64.sqrt();
        let np = [n[0][0], n[1][1], n[2][2], n[0][1], n[1][2], n[2][0]];
        for k in 0..6 {
            state[k] += f * np[k];
        }
        state[6] += d;
    }
    let mut t = [[0.0; 9]; 9];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                for l in 0..3 {
                    let id_s = 0.5 * (if i == k && j == l { 1.0 } else { 0.0 } + if i == l && j == k { 1.0 } else { 0.0 });
                    let (dij, dkl) = (if i == j { 1.0 } else { 0.0 }, if k == l { 1.0 } else { 0.0 });
                    t[3 * i + j][3 * k + l] = mo.k * dij * dkl + 2.0 * mo.g * theta * (id_s - dij * dkl / 3.0) - c * n[i][j] * n[k][l];
                }
            }
        }
    }
    Update { stress, tangent: t, state }
}

// ------------------------------------------------------------------ small strain, anisotropic elasticity

/// Tensor index pair of every entry of the library's stress / strain order `(xx, yy, zz, xy, yz, zx)`.
const PAIRS: [(usize, usize); 6] = [(0, 0), (1, 1), (2, 2), (0, 1), (1, 2), (2, 0)];

/// Solve `A X = B` (`N x N`, `N x M`) by Gaussian elimination with partial pivoting; `None` if singular.
fn lu_solve<const N: usize, const M: usize>(mut a: [[f64; N]; N], mut b: [[f64; M]; N]) -> Option<[[f64; M]; N]> {
    let scale = a.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-300);
    for c in 0..N {
        let p = (c..N).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-14 * scale {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for i in c + 1..N {
            let f = a[i][c] / a[c][c];
            for k in c..N {
                a[i][k] -= f * a[c][k];
            }
            for k in 0..M {
                b[i][k] -= f * b[c][k];
            }
        }
    }
    let mut x = [[0.0; M]; N];
    for i in (0..N).rev() {
        for k in 0..M {
            x[i][k] = (b[i][k] - (i + 1..N).map(|j| a[i][j] * x[j][k]).sum::<f64>()) / a[i][i];
        }
    }
    Some(x)
}

/// Von Mises stress, the flow direction `n_v = d sigma_vm / d sigma` as a strain-like vector (engineering shear: the
/// plastic strain increment is `dgamma n_v`) and `M = d n_v / d sigma_v` (6 x 6) of a stress in Voigt order.
fn flow_direction(sig: &[f64; 6]) -> (f64, [f64; 6], [[f64; 6]; 6]) {
    let tr = (sig[0] + sig[1] + sig[2]) / 3.0;
    let s = [sig[0] - tr, sig[1] - tr, sig[2] - tr, sig[3], sig[4], sig[5]]; // tensor components s_ij (shear once)
    let ss = s[0] * s[0] + s[1] * s[1] + s[2] * s[2] + 2.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5]);
    let vm = (1.5 * ss).sqrt().max(1e-300);
    let n: [f64; 6] = std::array::from_fn(|a| 1.5 * s[a] / vm); // tensor components n_ij
    let g: [f64; 6] = std::array::from_fn(|a| if a < 3 { n[a] } else { 2.0 * n[a] }); // d vm / d sigma_v
    let mut m = [[0.0; 6]; 6];
    for a in 0..6 {
        let (i, j) = PAIRS[a];
        let ca = if a < 3 { 1.0 } else { 2.0 };
        for b in 0..6 {
            let (k, l) = PAIRS[b];
            let ds = if b < 3 { f64::from(u8::from(i == k && j == k)) - if i == j { 1.0 / 3.0 } else { 0.0 } } else { f64::from(u8::from(i == k && j == l)) + f64::from(u8::from(i == l && j == k)) };
            m[a][b] = ca * (1.5 / vm * ds - n[a] * g[b] / vm);
        }
    }
    (vm, g, m)
}

/// Small-strain J2 update with a general anisotropic elastic stiffness `d` (`6 x 6`, library order, engineering shear):
/// the associative von Mises flow `d eps_p = dgamma n(sigma)` with `n = d sigma_vm / d sigma` and the stress `sigma =
/// D (eps - eps_p)`. Unlike the isotropic law the flow direction changes during the return (`D n` is not parallel to
/// `n`), so the return solves `sigma + dgamma D n(sigma) = sigma_trial`, `sigma_vm(sigma) = sigma_y(p + dgamma)` for the
/// stress and the multiplier together by Newton's method; the tangent is the consistent one of that linearisation.
pub fn small_strain_update_aniso(d: &[[f64; 6]; 6], law: &Hardening, eps: &M3, old: &GpState) -> Update {
    let ep_v = [old[0], old[1], old[2], 2.0 * old[3], 2.0 * old[4], 2.0 * old[5]];
    let e_v = [eps[0][0], eps[1][1], eps[2][2], 2.0 * eps[0][1], 2.0 * eps[1][2], 2.0 * eps[2][0]];
    let ee: [f64; 6] = std::array::from_fn(|i| e_v[i] - ep_v[i]);
    let tr: [f64; 6] = std::array::from_fn(|i| (0..6).map(|j| d[i][j] * ee[j]).sum());
    let p0 = old[6];
    let (vm_tr, _, _) = flow_direction(&tr);
    let sy0 = law.stress(p0);
    let to_t4 = |tv: &[[f64; 6]; 6]| -> T4 {
        // d sigma_ij / d H_kl = sum_b T[a(ij)][b] d eps_v[b] / d H_kl.
        let mut t = [[0.0; 9]; 9];
        for i in 0..3 {
            for j in 0..3 {
                let a = PAIRS.iter().position(|&(p, q)| (p == i && q == j) || (p == j && q == i)).expect("pair");
                for k in 0..3 {
                    for l in 0..3 {
                        let mut v = 0.0;
                        for b in 0..6 {
                            let (p, q) = PAIRS[b];
                            let de = if b < 3 { f64::from(u8::from(k == p && l == p)) } else { f64::from(u8::from(k == p && l == q)) + f64::from(u8::from(k == q && l == p)) };
                            v += tv[a][b] * de;
                        }
                        t[3 * i + j][3 * k + l] = v;
                    }
                }
            }
        }
        t
    };
    let tensor = |s: &[f64; 6]| -> M3 { [[s[0], s[3], s[5]], [s[3], s[1], s[4]], [s[5], s[4], s[2]]] };
    if vm_tr <= sy0 {
        return Update { stress: tensor(&tr), tangent: to_t4(d), state: *old };
    }
    // Plastic: Newton on (sigma, dgamma).
    let gbar = (d[3][3] + d[4][4] + d[5][5]) / 3.0;
    let mut dg = ((vm_tr - sy0) / (3.0 * gbar + law.slope(p0).max(0.0))).max(0.0);
    let mut sig: [f64; 6] = std::array::from_fn(|i| tr[i]);
    let resid = |sig: &[f64; 6], dg: f64| -> ([f64; 6], f64) {
        let (vm, _, _) = flow_direction(sig);
        let (_, g, _) = flow_direction(sig);
        let dn: [f64; 6] = std::array::from_fn(|i| (0..6).map(|j| d[i][j] * g[j]).sum());
        (std::array::from_fn(|i| sig[i] - tr[i] + dg * dn[i]), vm - law.stress(p0 + dg))
    };
    let norm = |r: &[f64; 6], f: f64| (r.iter().map(|v| v * v).sum::<f64>() + f * f).sqrt();
    let (mut r, mut f) = resid(&sig, dg);
    let mut rn = norm(&r, f);
    for _ in 0..60 {
        if rn <= 1e-12 * sy0.max(1e-300) {
            break;
        }
        let (_, g, m) = flow_direction(&sig);
        let hp = law.slope(p0 + dg);
        // J = [I + dg D M, D g; g^T, -H] over (sigma, dgamma).
        let mut jac = [[0.0; 7]; 7];
        for i in 0..6 {
            for j in 0..6 {
                jac[i][j] = f64::from(u8::from(i == j)) + dg * (0..6).map(|k| d[i][k] * m[k][j]).sum::<f64>();
            }
            jac[i][6] = (0..6).map(|k| d[i][k] * g[k]).sum();
            jac[6][i] = g[i];
        }
        jac[6][6] = -hp;
        let mut rhs = [[0.0; 1]; 7];
        for i in 0..6 {
            rhs[i][0] = -r[i];
        }
        rhs[6][0] = -f;
        let Some(dx) = lu_solve(jac, rhs) else { break };
        // Backtrack if the step does not reduce the residual (the multiplier stays non-negative).
        let mut alpha = 1.0;
        let mut accepted = false;
        for _ in 0..12 {
            let st: [f64; 6] = std::array::from_fn(|i| sig[i] + alpha * dx[i][0]);
            let dgt = (dg + alpha * dx[6][0]).max(0.0);
            let (rt, ft) = resid(&st, dgt);
            let n = norm(&rt, ft);
            if n < rn || n <= 1e-12 * sy0 {
                (sig, dg, r, f, rn) = (st, dgt, rt, ft, n);
                accepted = true;
                break;
            }
            alpha *= 0.5;
        }
        if !accepted {
            break;
        }
    }
    let (_, g, m) = flow_direction(&sig);
    let hp = law.slope(p0 + dg);
    // Tangent: A = I + dg D M; T = A^-1 D - (A^-1 D g)(g^T A^-1 D) / (g^T A^-1 D g + H).
    let mut a = [[0.0; 6]; 6];
    for i in 0..6 {
        for j in 0..6 {
            a[i][j] = f64::from(u8::from(i == j)) + dg * (0..6).map(|k| d[i][k] * m[k][j]).sum::<f64>();
        }
    }
    let mut rhs = [[0.0; 7]; 6];
    for i in 0..6 {
        rhs[i][..6].copy_from_slice(&d[i]);
        rhs[i][6] = (0..6).map(|k| d[i][k] * g[k]).sum();
    }
    let x = lu_solve(a, rhs).expect("the return-mapping linearisation is regular");
    // x[.][0..6] = A^-1 D, x[.][6] = A^-1 D g.
    let adg: [f64; 6] = std::array::from_fn(|i| x[i][6]);
    let gad: [f64; 6] = std::array::from_fn(|j| (0..6).map(|i| g[i] * x[i][j]).sum());
    let denom = (0..6).map(|i| g[i] * adg[i]).sum::<f64>() + hp;
    let mut tv = [[0.0; 6]; 6];
    for i in 0..6 {
        for j in 0..6 {
            tv[i][j] = x[i][j] - adg[i] * gad[j] / denom;
        }
    }
    let mut state = *old;
    for k in 0..3 {
        state[k] += dg * g[k];
    }
    for k in 3..6 {
        state[k] += dg * 0.5 * g[k]; // tensor shear = engineering / 2; g[k] holds the engineering component
    }
    state[6] += dg;
    Update { stress: tensor(&sig), tangent: to_t4(&tv), state }
}

/// Plane-stress version of [`small_strain_update_aniso`]: `eps_zz` is solved so that `sigma_zz = 0`.
pub fn small_strain_plane_stress_aniso(d: &[[f64; 6]; 6], law: &Hardening, eps: &M3, old: &GpState) -> (Update, f64) {
    let mut e = *eps;
    let mut ezz = -0.3 * (e[0][0] + e[1][1]);
    let mut up = small_strain_update_aniso(d, law, &e, old);
    for _ in 0..40 {
        e[2][2] = ezz;
        up = small_strain_update_aniso(d, law, &e, old);
        let szz = up.stress[2][2];
        let scale = d[2][2].abs().max(1e-300) * (e[0][0].abs() + e[1][1].abs() + e[0][1].abs() + ezz.abs());
        if szz.abs() <= 1e-13 * scale {
            break;
        }
        ezz -= szz / up.tangent[8][8];
    }
    up.tangent = condense_zz(&up.tangent);
    (up, ezz)
}

// ------------------------------------------------------------------ finite strain

/// Finite-strain Hencky J2 update for the deformation gradient `f` (3 x 3, `det f > 0`).
/// Returns `None` for a non-positive determinant.
pub fn finite_strain_update(mo: Moduli, law: &Hardening, f: &M3, old: &GpState) -> Option<Update> {
    if det(f) <= 0.0 || !det(f).is_finite() {
        return None;
    }
    let finv = inverse(f)?;
    let cinv = sym_from_voigt(&old[..6]);
    let be = mul(&mul(f, &cinv), &transpose(f));
    let (lam, vec) = sym_eig(&be);
    if lam.iter().any(|&l| l <= 0.0 || !l.is_finite()) {
        return None;
    }
    let eps_tr: [f64; 3] = std::array::from_fn(|a| 0.5 * lam[a].ln());
    let pr = return_principal(mo, law, eps_tr, old[6]);
    // Elastic left Cauchy-Green after the return, Kirchhoff stress and first Piola-Kirchhoff stress.
    let (mut be_new, mut tau) = ([[0.0; 3]; 3], [[0.0; 3]; 3]);
    for a in 0..3 {
        let l = (2.0 * pr.eps_e[a]).exp();
        for i in 0..3 {
            for j in 0..3 {
                be_new[i][j] += l * vec[a][i] * vec[a][j];
                tau[i][j] += pr.tau[a] * vec[a][i] * vec[a][j];
            }
        }
    }
    let finv_t = transpose(&finv);
    let cinv_new = mul(&mul(&finv, &be_new), &finv_t);
    let p = mul(&tau, &finv_t);
    let cv = voigt_from_sym(&cinv_new);
    let state = [cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], old[6] + pr.d];
    // Tangent dP/dF, one unit perturbation dF = e_k (x) e_L at a time.
    let mut tangent = [[0.0; 9]; 9];
    let hs: [[f64; 3]; 3] = std::array::from_fn(|a| std::array::from_fn(|b| if a == b { 0.5 / lam[a] } else { 0.5 * dlog(lam[a], lam[b]) }));
    let ms: [[f64; 3]; 3] = std::array::from_fn(|a| {
        std::array::from_fn(|b| {
            if a == b {
                0.0
            } else if (eps_tr[a] - eps_tr[b]).abs() > 1e-9 * (1.0 + eps_tr[a].abs() + eps_tr[b].abs()) {
                (pr.tau[a] - pr.tau[b]) / (eps_tr[a] - eps_tr[b])
            } else {
                // Isotropic function with equal arguments: dtau_a/de_a - dtau_a/de_b.
                0.5 * (pr.a[a][a] + pr.a[b][b]) - 0.5 * (pr.a[a][b] + pr.a[b][a])
            }
        })
    });
    for k in 0..3 {
        for l in 0..3 {
            // dBe = dF Cp^-1 F^T + F Cp^-1 dF^T with dF = e_k e_l^T.
            let fcinv = mul(f, &cinv);
            let mut dbe = [[0.0; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    // (dF Cinv F^T)_ij = delta_ik (Cinv F^T)_lj ; (F Cinv dF^T)_ij = (F Cinv)_il delta_jk.
                    let cf_lj: f64 = (0..3).map(|m| cinv[l][m] * f[j][m]).sum();
                    if i == k {
                        dbe[i][j] += cf_lj;
                    }
                    if j == k {
                        dbe[i][j] += fcinv[i][l];
                    }
                }
            }
            // Principal-frame components of dBe, then of d eps_tr and d tau.
            let mut db = [[0.0; 3]; 3];
            for a in 0..3 {
                for b in 0..3 {
                    for i in 0..3 {
                        for j in 0..3 {
                            db[a][b] += vec[a][i] * dbe[i][j] * vec[b][j];
                        }
                    }
                }
            }
            let de: [f64; 3] = std::array::from_fn(|a| hs[a][a] * db[a][a]);
            let mut dtau = [[0.0; 3]; 3]; // principal frame
            for a in 0..3 {
                dtau[a][a] = (0..3).map(|b| pr.a[a][b] * de[b]).sum();
                for b in 0..3 {
                    if a != b {
                        dtau[a][b] = ms[a][b] * (hs[a][b] * db[a][b]);
                    }
                }
            }
            let mut dt = [[0.0; 3]; 3]; // global
            for a in 0..3 {
                for b in 0..3 {
                    for i in 0..3 {
                        for j in 0..3 {
                            dt[i][j] += vec[a][i] * dtau[a][b] * vec[b][j];
                        }
                    }
                }
            }
            // dP = dtau F^-T + tau d(F^-T), d(F^-T) = -F^-T dF^T F^-T.
            let mut dfit = [[0.0; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    // (F^-T dF^T F^-T)_ij = sum_m F^-T_im (dF^T)_mn F^-T_nj = F^-1_mi delta_mk ... (dF^T)_mn = dF_nm = delta_nk delta_ml
                    dfit[i][j] = -finv_t[i][l] * finv_t[k][j];
                }
            }
            let dp1 = mul(&dt, &finv_t);
            let dp2 = mul(&tau, &dfit);
            for i in 0..3 {
                for j in 0..3 {
                    tangent[3 * i + j][3 * k + l] = dp1[i][j] + dp2[i][j];
                }
            }
        }
    }
    Some(Update { stress: p, tangent, state })
}

// ------------------------------------------------------------------ plane stress

/// Condense the out-of-plane (`zz`, index 8) row and column out of a tangent.
pub fn condense_zz(t: &T4) -> T4 {
    let mut out = *t;
    for a in 0..9 {
        for b in 0..9 {
            out[a][b] = t[a][b] - t[a][8] * t[8][b] / t[8][8];
        }
    }
    out
}

/// Plane-stress small strain: in-plane strain tensor `eps` (the `zz` entry is solved so that
/// `sigma_zz = 0`); returns the update with the converged `eps_zz` and the condensed tangent.
pub fn small_strain_plane_stress(mo: Moduli, law: &Hardening, eps: &M3, old: &GpState) -> (Update, f64) {
    let mut e = *eps;
    let mut ezz = -0.3 * (e[0][0] + e[1][1]); // a typical Poisson-like start
    let mut up = small_strain_update(mo, law, &e, old);
    for _ in 0..30 {
        e[2][2] = ezz;
        up = small_strain_update(mo, law, &e, old);
        let szz = up.stress[2][2];
        // Converged when sigma_zz is negligible against the elastic stress scale of this strain state
        // (never against the yield stress: an elastic law has an enormous one).
        let scale = (mo.k + 4.0 * mo.g / 3.0) * (e[0][0].abs() + e[1][1].abs() + e[0][1].abs() + ezz.abs());
        if szz.abs() <= 1e-13 * scale {
            break;
        }
        ezz -= szz / up.tangent[8][8];
    }
    up.tangent = condense_zz(&up.tangent);
    (up, ezz)
}

/// Plane-stress finite strain: in-plane deformation gradient (the `zz` stretch is solved so that
/// `P_zz = 0`). Returns the update and the converged stretch.
pub fn finite_strain_plane_stress(mo: Moduli, law: &Hardening, f: &M3, old: &GpState, lambda_guess: f64) -> Option<(Update, f64)> {
    let mut ff = *f;
    let mut lz = lambda_guess.max(1e-3);
    let mut up = None;
    for _ in 0..40 {
        ff[2][2] = lz;
        let u = finite_strain_update(mo, law, &ff, old)?;
        let pzz = u.stress[2][2];
        let strain_size = (ff[0][0] - 1.0).abs() + (ff[1][1] - 1.0).abs() + ff[0][1].abs() + ff[1][0].abs() + (lz - 1.0).abs();
        let converged = pzz.abs() <= 1e-13 * (mo.k + mo.g) * strain_size;
        let slope = u.tangent[8][8];
        up = Some(u);
        if converged {
            break;
        }
        // A damped Newton step keeps the stretch positive.
        let step = -pzz / slope;
        lz = (lz + step).max(0.3 * lz);
    }
    let mut u = up?;
    u.tangent = condense_zz(&u.tangent);
    Some((u, lz))
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: f64 = 10.0e6;
    const NU: f64 = 0.3;

    fn mo() -> Moduli {
        Moduli::new(E, NU)
    }

    fn vm(s: &M3) -> f64 {
        let tr = (s[0][0] + s[1][1] + s[2][2]) / 3.0;
        let mut dev = *s;
        for i in 0..3 {
            dev[i][i] -= tr;
        }
        (1.5 * dev.iter().flatten().map(|v| v * v).sum::<f64>()).sqrt()
    }

    fn law() -> Hardening {
        Hardening::table(&[(0.0, 40_000.0), (0.01, 50_000.0), (0.05, 60_000.0), (0.5, 70_000.0)]).unwrap()
    }

    #[test]
    fn symmetric_eigen_decomposition_reconstructs_the_matrix() {
        for m in [[[2.0, 0.3, -0.1], [0.3, 1.0, 0.5], [-0.1, 0.5, 3.0]], [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], [[5.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0]], [[1.0, 1e-9, 0.0], [1e-9, 1.0, 0.0], [0.0, 0.0, 1.0]]] {
            let (l, v) = sym_eig(&m);
            for i in 0..3 {
                for j in 0..3 {
                    let r: f64 = (0..3).map(|a| l[a] * v[a][i] * v[a][j]).sum();
                    assert!((r - m[i][j]).abs() < 1e-12, "({i},{j}): {r} vs {}", m[i][j]);
                }
            }
            for a in 0..3 {
                for b in 0..3 {
                    let dot: f64 = (0..3).map(|i| v[a][i] * v[b][i]).sum();
                    assert!((dot - if a == b { 1.0 } else { 0.0 }).abs() < 1e-12, "orthonormality ({a},{b})");
                }
            }
        }
    }

    #[test]
    fn elastic_response_is_hooke_and_small_strain_simple_shear_yields_at_sigma_y_over_root3() {
        // Elastic: uniaxial strain-controlled stress state.
        let eps = [[1e-4, 0.0, 0.0], [0.0, -NU * 1e-4, 0.0], [0.0, 0.0, -NU * 1e-4]];
        let up = small_strain_update(mo(), &law(), &eps, &[0.0; 7]);
        assert!((up.stress[0][0] - E * 1e-4).abs() < 1e-9 * E * 1e-4 && up.stress[1][1].abs() < 1e-8 && up.stress[0][1] == 0.0);
        assert_eq!(up.state, [0.0; 7]);
        // Perfectly plastic simple shear: sigma_xy = sigma_y / sqrt 3 whatever the (large) shear.
        let flat = Hardening::linear(40_000.0, 0.0, 1.0).unwrap();
        for gamma in [0.02, 0.1, 0.5] {
            let eps = [[0.0, gamma / 2.0, 0.0], [gamma / 2.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
            let up = small_strain_update(mo(), &flat, &eps, &[0.0; 7]);
            assert!((up.stress[0][1] - 40_000.0 / 3.0f64.sqrt()).abs() < 1e-8 * 40_000.0, "gamma {gamma}: {}", up.stress[0][1]);
            assert!((vm(&up.stress) - 40_000.0).abs() < 1e-8 * 40_000.0);
            // Equivalent plastic strain: (gamma - gamma_e) / sqrt 3 with gamma_e = tau / G.
            let g = mo().g;
            let want = (gamma - 40_000.0 / 3.0f64.sqrt() / g) / 3.0f64.sqrt();
            assert!((up.state[6] - want).abs() < 1e-10, "ep {} vs {want}", up.state[6]);
        }
    }

    #[test]
    fn plane_stress_elastic_response_is_the_reduced_hooke_law_for_any_yield_stress() {
        // sigma_x = E/(1-nu^2) (e_x + nu e_y), tau_xy = G gamma: with a normal and with an enormous yield stress.
        let (ex, ey, exy) = (1.2e-3, -3.0e-4, 4.0e-4);
        let h = [[ex, exy, 0.0], [exy, ey, 0.0], [0.0; 3]];
        for law in [law(), Hardening::linear(1.0e30, 0.0, 1.0).unwrap()] {
            let (up, ezz) = small_strain_plane_stress(mo(), &law, &h, &[0.0; 7]);
            let c = E / (1.0 - NU * NU);
            assert!((up.stress[0][0] - c * (ex + NU * ey)).abs() < 1e-9 * c * ex, "sigma_x {} vs {}", up.stress[0][0], c * (ex + NU * ey));
            assert!((up.stress[1][1] - c * (ey + NU * ex)).abs() < 1e-9 * c * ex);
            assert!((up.stress[0][1] - mo().g * 2.0 * exy).abs() < 1e-9 * c * ex);
            assert!((ezz + NU / (1.0 - NU) * (ex + ey)).abs() < 1e-12, "eps_zz {ezz}");
            assert!(up.stress[2][2].abs() < 1e-9 * c * ex);
        }
        // Finite strain with a huge yield stress: hyperelastic, P_zz = 0 and a small-deformation limit equal to the same law.
        let f = [[1.0 + ex, exy, 0.0], [exy, 1.0 + ey, 0.0], [0.0, 0.0, 1.0]];
        let flat = Hardening::linear(1.0e30, 0.0, 1.0).unwrap();
        let (up, lz) = finite_strain_plane_stress(mo(), &flat, &f, &[1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0], 1.0).unwrap();
        assert!(up.stress[2][2].abs() < 1e-6 && (lz - 1.0 + NU / (1.0 - NU) * (ex + ey)).abs() < 1e-5, "stretch {lz}");
        let c = E / (1.0 - NU * NU);
        assert!((up.stress[0][0] - c * (ex + NU * ey)).abs() < 5e-3 * c * ex, "P_xx {}", up.stress[0][0]);
    }

    #[test]
    fn hardening_uniaxial_stress_path_follows_the_curve() {
        // Strain-controlled uniaxial *stress*: given eps_x find the lateral strains (equal) so that
        // sigma_y = sigma_z = 0; then sigma_x must lie on the yield curve at the accumulated ep.
        let l = law();
        for ex in [0.004, 0.008, 0.02, 0.06] {
            let mut ey = -NU * ex;
            for _ in 0..60 {
                let eps = [[ex, 0.0, 0.0], [0.0, ey, 0.0], [0.0, 0.0, ey]];
                let up = small_strain_update(mo(), &l, &eps, &[0.0; 7]);
                let sy = up.stress[1][1];
                if sy.abs() < 1e-9 {
                    assert!((up.stress[0][0] - l.stress(up.state[6])).abs() < 1e-6 * up.stress[0][0] || up.state[6] == 0.0, "ex {ex}");
                    break;
                }
                ey -= sy / up.tangent[4][4];
            }
        }
    }

    /// Central finite difference of `S` with respect to every `H_kl`.
    fn fd_tangent(f: &dyn Fn(&M3) -> M3, h0: &M3, step: f64) -> T4 {
        let mut t = [[0.0; 9]; 9];
        for k in 0..3 {
            for l in 0..3 {
                let (mut hp, mut hm) = (*h0, *h0);
                hp[k][l] += step;
                hm[k][l] -= step;
                let (sp, sm) = (f(&hp), f(&hm));
                for i in 0..3 {
                    for j in 0..3 {
                        t[3 * i + j][3 * k + l] = (sp[i][j] - sm[i][j]) / (2.0 * step);
                    }
                }
            }
        }
        t
    }

    fn max_rel(a: &T4, b: &T4) -> f64 {
        let scale = a.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()));
        a.iter().flatten().zip(b.iter().flatten()).fold(0.0f64, |m, (x, y)| m.max((x - y).abs())) / scale
    }

    #[test]
    fn small_strain_consistent_tangent_matches_finite_differences_elastic_and_plastic() {
        let l = law();
        let old_plastic: GpState = small_strain_update(mo(), &l, &[[0.012, 0.0, 0.0], [0.0, -0.004, 0.003], [0.0, 0.003, -0.004]], &[0.0; 7]).state;
        assert!(old_plastic[6] > 0.0);
        for (old, h) in [
            ([0.0; 7], [[1e-3, 2e-4, 0.0], [-1e-4, 3e-4, 1e-4], [0.0, 2e-4, -5e-4]]),
            ([0.0; 7], [[0.02, 0.004, 0.001], [0.002, -0.006, 0.003], [0.001, 0.001, 0.004]]),
            ([0.0; 7], [[0.2, 0.05, 0.0], [0.05, -0.1, 0.02], [0.0, 0.02, 0.01]]),
            (old_plastic, [[0.014, 0.001, 0.0], [0.001, -0.005, 0.002], [0.0, 0.002, -0.003]]),
        ] {
            let sym = |h: &M3| -> M3 { std::array::from_fn(|i| std::array::from_fn(|j| 0.5 * (h[i][j] + h[j][i]))) };
            let up = small_strain_update(mo(), &l, &sym(&h), &old);
            let fd = fd_tangent(&|hh: &M3| small_strain_update(mo(), &l, &sym(hh), &old).stress, &h, 1e-7);
            let err = max_rel(&up.tangent, &fd);
            assert!(err < 1e-8, "tangent error {err:e} (ep = {})", up.state[6]);
        }
    }

    fn deformation(g: &M3) -> M3 {
        *g
    }

    #[test]
    fn finite_strain_tangent_is_analytic_and_matches_finite_differences_everywhere() {
        let l = law();
        let rot = |a: f64| -> M3 { [[a.cos(), -a.sin(), 0.0], [a.sin(), a.cos(), 0.0], [0.0, 0.0, 1.0]] };
        let stretched = mul(&rot(0.7), &[[1.01, 0.3, 0.05], [0.0, 0.98, 0.1], [0.0, 0.0, 1.02]]);
        // A plastic prehistory so the state is non-trivial.
        let hist = finite_strain_update(mo(), &l, &deformation(&[[1.03, 0.2, 0.0], [0.0, 0.97, 0.0], [0.0, 0.0, 1.0]]), &[1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]).unwrap().state;
        assert!(hist[6] > 0.0, "the history must have yielded");
        let init = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        for (old, f) in [
            (init, [[1.0001, 0.0002, 0.0], [0.0, 0.9999, 0.0001], [0.0, 0.0, 1.0001]]), // elastic, nearly isotropic Be
            (init, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),             // exactly repeated eigenvalues
            (init, [[1.002, 0.001, 0.0], [0.0, 1.001, 0.0], [0.0, 0.0, 1.0015]]),   // elastic
            (init, [[1.04, 0.25, 0.0], [0.0, 0.96, 0.1], [0.0, 0.0, 1.0]]),         // plastic
            (init, stretched),                                                       // plastic, rotated
            (hist, [[1.05, 0.22, 0.01], [0.01, 0.95, 0.12], [0.0, 0.02, 1.03]]),   // plastic with history
            (init, [[1.2, 0.0, 0.0], [0.0, 0.9, 0.0], [0.0, 0.0, 0.9]]),            // large uniaxial-ish
        ] {
            let up = finite_strain_update(mo(), &l, &f, &old).unwrap();
            let fd = fd_tangent(&|ff: &M3| finite_strain_update(mo(), &l, ff, &old).unwrap().stress, &f, 1e-6);
            let err = max_rel(&up.tangent, &fd);
            assert!(err < 1e-8, "dP/dF error {err:e} for F = {f:?} (ep {})", up.state[6]);
        }
    }

    #[test]
    fn finite_strain_is_objective_isochoric_in_plasticity_and_matches_small_strain_for_small_deformation() {
        let l = law();
        let init = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let f = [[1.05, 0.25, 0.0], [0.0, 0.96, 0.1], [0.0, 0.0, 1.0]];
        let a = finite_strain_update(mo(), &l, &f, &init).unwrap();
        assert!(a.state[6] > 0.0);
        // Objectivity: P(QF) = Q P(F) with the same history, same ep.
        let q: M3 = [[0.6, -0.8, 0.0], [0.8, 0.6, 0.0], [0.0, 0.0, 1.0]];
        let b = finite_strain_update(mo(), &l, &mul(&q, &f), &init).unwrap();
        let qp = mul(&q, &a.stress);
        for i in 0..3 {
            for j in 0..3 {
                assert!((b.stress[i][j] - qp[i][j]).abs() < 1e-8 * 40_000.0, "objectivity ({i},{j})");
            }
        }
        assert!((a.state[6] - b.state[6]).abs() < 1e-12 && (0..6).all(|k| (a.state[k] - b.state[k]).abs() < 1e-12), "state is frame independent");
        // Plastic flow preserves the volume: det Cp^-1 = 1.
        let c = sym_from_voigt(&a.state[..6]);
        assert!((det(&c) - 1.0).abs() < 1e-12, "det Cp^-1 = {}", det(&c));
        // Kirchhoff von Mises on the yield curve.
        let tau = mul(&a.stress, &transpose(&f));
        assert!((vm(&tau) - l.stress(a.state[6])).abs() < 1e-7 * 40_000.0, "tau vm {} vs {}", vm(&tau), l.stress(a.state[6]));
        // Small deformations: finite equals small strain (stress and ep) to O(eps^2).
        let h = [[2e-3, 1.5e-3, 0.0], [-0.5e-3, -1e-3, 4e-4], [0.0, 1e-3, 5e-4]];
        let ff: M3 = std::array::from_fn(|i| std::array::from_fn(|j| h[i][j] + if i == j { 1.0 } else { 0.0 }));
        let big = finite_strain_update(mo(), &l, &ff, &init).unwrap();
        let eps: M3 = std::array::from_fn(|i| std::array::from_fn(|j| 0.5 * (h[i][j] + h[j][i])));
        let small = small_strain_update(mo(), &l, &eps, &[0.0; 7]);
        let scale = E * 2e-3;
        for i in 0..3 {
            for j in 0..3 {
                assert!((big.stress[i][j] - small.stress[i][j]).abs() < 5e-3 * scale, "({i},{j}): {} vs {}", big.stress[i][j], small.stress[i][j]);
            }
        }
        // Plasticity at larger but still small strain agrees on ep.
        let h = [[0.008, 0.004, 0.0], [0.0, -0.004, 0.0], [0.0, 0.0, -0.002]];
        let ff: M3 = std::array::from_fn(|i| std::array::from_fn(|j| h[i][j] + if i == j { 1.0 } else { 0.0 }));
        let big = finite_strain_update(mo(), &l, &ff, &init).unwrap();
        let eps: M3 = std::array::from_fn(|i| std::array::from_fn(|j| 0.5 * (h[i][j] + h[j][i])));
        let small = small_strain_update(mo(), &l, &eps, &[0.0; 7]);
        assert!(small.state[6] > 0.0 && (big.state[6] / small.state[6] - 1.0).abs() < 0.05, "ep {} vs {}", big.state[6], small.state[6]);
    }

    #[test]
    fn finite_strain_simple_shear_of_a_perfectly_plastic_body_reaches_the_yield_surface() {
        let flat = Hardening::linear(40_000.0, 0.0, 1.0).unwrap();
        let init = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let mut prev_ep = 0.0;
        for gamma in [0.05, 0.2, 1.0, 3.0] {
            let f = [[1.0, gamma, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            let up = finite_strain_update(mo(), &flat, &f, &init).unwrap();
            let tau = mul(&up.stress, &transpose(&f));
            assert!((vm(&tau) - 40_000.0).abs() < 1e-6 * 40_000.0, "gamma {gamma}: vm(tau) {}", vm(&tau));
            assert!(up.state[6] > prev_ep, "ep grows with the shear");
            prev_ep = up.state[6];
            assert!((det(&sym_from_voigt(&up.state[..6])) - 1.0).abs() < 1e-10);
        }
        assert!(finite_strain_update(mo(), &flat, &[[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], &init).is_none(), "an inverted gradient is rejected");
    }

    #[test]
    fn plane_stress_returns_zero_out_of_plane_stress_and_a_condensed_tangent_matching_finite_differences() {
        let l = law();
        let old = [0.0; 7];
        for h in [[[1e-3, 3e-4, 0.0], [3e-4, -2e-4, 0.0], [0.0; 3]], [[0.012, 0.004, 0.0], [0.004, -0.003, 0.0], [0.0; 3]]] {
            let (up, ezz) = small_strain_plane_stress(mo(), &l, &h, &old);
            assert!(up.stress[2][2].abs() < 1e-6, "sigma_zz {}", up.stress[2][2]);
            assert!(ezz != 0.0);
            let fd = fd_tangent(
                &|hh: &M3| {
                    // The tangent is dS/dH for the symmetric strain the gradient produces.
                    let mut hh: M3 = std::array::from_fn(|i| std::array::from_fn(|j| 0.5 * (hh[i][j] + hh[j][i])));
                    for i in 0..3 {
                        hh[i][2] = 0.0;
                        hh[2][i] = 0.0;
                    }
                    small_strain_plane_stress(mo(), &l, &hh, &old).0.stress
                },
                &h,
                1e-7,
            );
            // Compare the in-plane (xx, xy, yx, yy) entries (the sym map gives xy and yx the same column derivative).
            let idx = [0usize, 1, 3, 4];
            let mut worst = 0.0f64;
            let scale = up.tangent[0][0].abs();
            for &a in &idx {
                for &b in &idx {
                    worst = worst.max((up.tangent[a][b] - fd[a][b]).abs() / scale);
                }
            }
            assert!(worst < 1e-7, "plane stress tangent error {worst:e}");
        }
        // Finite strain plane stress: P_zz = 0 and a tangent equal to finite differences.
        let f = [[1.04, 0.15, 0.0], [0.0, 0.97, 0.0], [0.0, 0.0, 1.0]];
        let init = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        let (up, lz) = finite_strain_plane_stress(mo(), &l, &f, &init, 1.0).unwrap();
        assert!(up.stress[2][2].abs() < 1e-5 && (lz - 1.0).abs() > 1e-6);
        let fd = fd_tangent(
            &|ff: &M3| {
                let mut g = *ff;
                g[2][2] = 1.0;
                finite_strain_plane_stress(mo(), &l, &g, &init, lz).unwrap().0.stress
            },
            &f,
            1e-6,
        );
        let idx = [0usize, 1, 3, 4];
        let scale = up.tangent[0][0].abs();
        let worst = idx.iter().flat_map(|&a| idx.iter().map(move |&b| (a, b))).fold(0.0f64, |m, (a, b)| m.max((up.tangent[a][b] - fd[a][b]).abs() / scale));
        assert!(worst < 1e-6, "finite plane stress tangent error {worst:e}");
    }
}
