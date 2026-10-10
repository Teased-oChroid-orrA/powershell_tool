//! Mass-dependent and stress-stiffened analyses: natural frequencies (`Model::modal`), linear buckling
//! (`Model::buckling`) and linear transient response (`Model::transient`, Newmark / HHT).
//!
//! The eigenproblems `K phi = lambda B phi` are solved by block shift-and-invert subspace iteration with Rayleigh-Ritz
//! projection on the existing sparse factorisation (`docs/adr/ADR-013-eigen-and-transient-methods.md` for the decision
//! and the measured alternatives). The result is never accepted on iteration behaviour alone: every returned pair carries
//! its backward-error residual `|K phi - lambda B phi| / (|K phi| + |lambda| |B phi|)` (less a floating-point allowance of `100 eps max(K_ii) |phi|`, which is all a rigid-body mode of a free structure can reach), and a pair whose residual is above the tolerance is an error.

use crate::analysis::Model;
use crate::assembly::{assemble_scalar_identity, BlockMatrix};
use crate::dense::sym_gen_eigen;
use crate::kernel;
use crate::linear::{Dirichlet, Ordering, Reduced};
use crate::loads::{self, Loads};
use rayon::prelude::*;

/// How the mass matrix is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MassKind {
    /// `rho integral N_a N_b`: the consistent mass (frequencies converge from above).
    Consistent,
    /// HRZ lumping: the diagonal of the consistent mass scaled to preserve each element's mass (positive for every
    /// element type, including the serendipity ones where row-sum lumping produces negative masses).
    Lumped,
}

/// Eigensolver. Both work on the shift-and-invert operator with the same factorisation and the same verified residual.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EigMethod {
    /// Whatever the measurements of `docs/adr/ADR-013-eigen-and-transient-methods.md` favour.
    #[default]
    Auto,
    /// Block subspace iteration with Rayleigh-Ritz (robust, many solves).
    Subspace,
    /// Lanczos with full reorthogonalisation in the `B` (modal) or `K` (buckling) inner product (few solves).
    Lanczos,
}

#[derive(Debug, Clone)]
pub struct ModalOptions {
    pub n_modes: usize,
    /// Frequencies are sought nearest to this eigenvalue `omega^2` (`0`: the lowest modes of a supported structure).
    /// A structure with rigid-body modes needs a negative shift (they appear as `lambda ~ 0`).
    pub shift: f64,
    /// Accepted backward-error residual of every returned mode (see [`Mode::residual`]).
    pub tol: f64,
    pub max_iter: usize,
    pub mass: MassKind,
    pub method: EigMethod,
}

impl Default for ModalOptions {
    fn default() -> Self {
        Self { n_modes: 6, shift: 0.0, tol: 1e-9, max_iter: 200, mass: MassKind::Consistent, method: EigMethod::Auto }
    }
}

#[derive(Debug, Clone)]
pub struct Mode {
    /// `omega^2` (rad^2/s^2 in consistent units).
    pub omega2: f64,
    pub frequency_hz: f64,
    /// Mass-normalised shape over all dofs (`phi^T M phi = 1`; zero at constrained dofs).
    pub shape: Vec<f64>,
    /// Backward-error residual `|K phi - lambda M phi| / (|K phi| + |lambda| |M phi|)`.
    pub residual: f64,
    /// Effective modal mass per global axis, `(phi^T M r_axis)^2` (mass units): sums to the total mass over a complete set.
    pub effective_mass: [f64; 3],
}

#[derive(Debug, Clone)]
pub struct Modal {
    /// Why the cheaper eigensolver was replaced (`None`: it was not).
    pub fallback: Option<String>,
    pub modes: Vec<Mode>,
    pub total_mass: f64,
    pub iterations: usize,
    pub subspace: usize,
    pub factor_nnz: usize,
}

#[derive(Debug, Clone)]
pub struct BucklingOptions {
    pub n_modes: usize,
    pub tol: f64,
    pub max_iter: usize,
    pub method: EigMethod,
}

impl Default for BucklingOptions {
    fn default() -> Self {
        Self { n_modes: 3, tol: 1e-9, max_iter: 200, method: EigMethod::Auto }
    }
}

#[derive(Debug, Clone)]
pub struct BucklingMode {
    /// Multiplier of the reference load that makes the structure singular (`K + lambda K_G` singular).
    pub load_factor: f64,
    pub shape: Vec<f64>,
    pub residual: f64,
}

#[derive(Debug, Clone)]
pub struct Buckling {
    pub fallback: Option<String>,
    pub modes: Vec<BucklingMode>,
    pub iterations: usize,
}

/// One eigenpair of the reduced pencil.
struct Pair {
    lambda: f64,
    x: Vec<f64>,
    residual: f64,
}

struct Pairs {
    fallback: Option<String>,
    pairs: Vec<Pair>,
    iterations: usize,
    subspace: usize,
    factor_nnz: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// `K phi = lambda M phi`, `M` positive definite; the modes nearest `sigma`.
    Modal,
    /// `K phi = lambda (-K_G) phi`; the smallest positive `lambda`.
    Buckling,
}

/// Deterministic pseudo-random start vectors (a fixed generator: runs are reproducible and cover every symmetry class).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    }
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl Model {
    fn require_mass(&self) -> Result<(), String> {
        if self.mesh.blocks.iter().any(|b| !(b.density.is_finite() && b.density > 0.0)) {
            return Err("every block needs a mass density (Mesh::set_density); a block without one is massless".into());
        }
        Ok(())
    }

    /// Mass matrix in the stiffness pattern.
    pub fn assemble_mass(&self, kind: MassKind) -> Result<BlockMatrix, String> {
        self.require_mass()?;
        let mesh = &self.mesh;
        assemble_scalar_identity(mesh, &self.pattern, |bi, _e, xyz, work, me| {
            let blk = &mesh.blocks[bi];
            kernel::mass(blk.kind, mesh.physics, blk.density, xyz, work, me)?;
            if kind == MassKind::Lumped {
                let nn = blk.kind.n_nodes();
                let total: f64 = me[..nn * nn].iter().sum();
                let diag: f64 = (0..nn).map(|a| me[a * nn + a]).sum();
                let s = total / diag;
                for a in 0..nn {
                    for b in 0..nn {
                        me[a * nn + b] = if a == b { me[a * nn + a] * s } else { 0.0 };
                    }
                }
            }
            Ok(())
        })
    }

    /// Mass of the whole mesh (`sum rho V`; plane problems include the thickness, axisymmetric the full revolution).
    pub fn total_mass(&self) -> Result<f64, String> {
        let m = self.assemble_mass(MassKind::Consistent)?;
        let d = self.mesh.dim();
        // Sum of all entries of one component's block rows = mass: the full symmetric matrix applied to a unit translation.
        let mut ones = vec![0.0; self.mesh.n_dofs()];
        for n in 0..self.mesh.nodes.len() {
            ones[n * d] = 1.0;
        }
        let mut y = vec![0.0; ones.len()];
        m.matvec_add(&self.pattern, &ones, &mut y);
        Ok(dot(&ones, &y))
    }

    /// Geometric (initial-stress) stiffness for the stress state of displacement `u` (a linear static solution of the
    /// reference load): plane stress / strain and 3D solids.
    pub fn assemble_geometric_stiffness(&self, u: &[f64], delta_t: f64) -> Result<BlockMatrix, String> {
        if matches!(self.mesh.physics, crate::mesh::Physics::Axisymmetric) {
            return Err("buckling is not available for axisymmetric models (the geometric stiffness has hoop terms that are not implemented)".into());
        }
        let stress = self.gauss_stresses(u, delta_t)?;
        let mesh = &self.mesh;
        assemble_scalar_identity(mesh, &self.pattern, |bi, e, xyz, work, kg| {
            let blk = &mesh.blocks[bi];
            let ngp = blk.kind.table().ngp;
            let s: Vec<[f64; 6]> = stress[bi][e * ngp..(e + 1) * ngp].iter().map(|(_, s)| *s).collect();
            kernel::geometric_stiffness(blk.kind, mesh.physics, &s, xyz, work, kg)
        })
    }

    /// Block shift-and-invert subspace iteration with Rayleigh-Ritz on the free dofs.
    #[allow(clippy::too_many_arguments)]
    fn eig_pencil(&self, bc: &Dirichlet, k: &BlockMatrix, b: &BlockMatrix, sigma: f64, kind: Kind, nev: usize, tol: f64, max_iter: usize, method: EigMethod) -> Result<Pairs, String> {
        let red = Reduced::with_ordering(&self.pattern, bc, Some(&self.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let m = red.n_free();
        if nev == 0 || nev > m {
            return Err(format!("{nev} modes requested of a system with {m} free dofs"));
        }
        let p = (2 * nev).max(nev + 8).min(m);
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let n = self.mesh.n_dofs();
        let mul = |mat: &BlockMatrix, x: &[f64]| -> Vec<f64> {
            let mut full = vec![0.0; n];
            for (&d, v) in free.iter().zip(x) {
                full[d] = *v;
            }
            let mut y = vec![0.0; n];
            mat.matvec_add(&self.pattern, &full, &mut y);
            free.iter().map(|&d| y[d]).collect()
        };
        // The shifted operator. Modal: the caller's shift. Buckling: the pencil `K phi = lambda B phi` has `B = -K_G` indefinite
        // whenever the stress state has tension and compression (bending), so the factors come in both signs and the positive end
        // converges no faster than the negative one. A shift just below the smallest positive factor (a Cholesky factorisation of
        // `K - sigma B` that succeeds *proves* `sigma` is below it and above every negative one) puts the wanted factors nearest
        // to the shift.
        let (sigma, a_mat, fac) = if kind == Kind::Buckling {
            let fac0 = red.factor(k).map_err(|e| e.to_string())?;
            let theta_hat = largest_positive_theta(m, nev, &mul, k, b, &fac0).ok_or_else(|| "no positive buckling factor: the reference load does not compress the structure".to_string())?;
            let mut sg = 0.5 / theta_hat;
            let mut tries = 0;
            loop {
                let a = k.axpy(-sg, b);
                match red.factor(&a) {
                    Ok(f) => break (sg, a, crate::linear::AnyFactor::Llt(f)),
                    Err(_) if tries < 60 => {
                        sg *= 0.5;
                        tries += 1;
                    }
                    Err(e) => return Err(format!("no admissible buckling shift was found: {e}")),
                }
            }
        } else {
            let a = if sigma == 0.0 { k.clone() } else { k.axpy(-sigma, b) };
            let f = red.factor_any(&a).map_err(|e| format!("{e} (for a structure with rigid-body modes use a negative shift)"))?;
            (sigma, a, f)
        };
        // Floating-point allowance of a residual: `K x` of an exact null vector (a rigid-body mode of a free structure) is
        // round-off, not zero, so a backward error measured against `|K x| + |lambda| |B x|` alone is meaningless there.
        let kscale = k.diagonal(&self.pattern).iter().fold(0.0f64, |m, v| m.max(v.abs()));
        let noise = 100.0 * f64::EPSILON * kscale;
        let mut fallback: Option<String> = None;
        let pair_of = |lam: f64, v: &[f64]| -> Pair {
            let kv = mul(k, v);
            let bv = mul(b, v);
            let r: f64 = kv.iter().zip(&bv).map(|(a, bb)| (a - lam * bb).powi(2)).sum::<f64>().sqrt();
            Pair { lambda: lam, x: v.to_vec(), residual: (r - noise * dot(v, v).sqrt()).max(0.0) / (dot(&kv, &kv).sqrt() + lam.abs() * dot(&bv, &bv).sqrt()).max(1e-300) }
        };
        if method == EigMethod::Lanczos || (method == EigMethod::Auto && AUTO_METHOD == EigMethod::Lanczos) {
            let attempt = lanczos(m, nev, tol, max_iter, kind, sigma, &mul, &a_mat, b, &fac, &pair_of, red.factor_nnz());
            match (attempt, method) {
                // The cheap method failed its verification: `Auto` goes on with the robust one and says so; an explicit request is an error.
                (Err(why), EigMethod::Auto) => fallback = Some(format!("Lanczos was rejected ({why}); subspace iteration used")),
                (other, _) => return other,
            }
        }
        let mut rng = Lcg(0x9E37_79B9_7F4A_7C15);
        let mut fallback_note = fallback;
        let mut x: Vec<Vec<f64>> = (0..p).map(|_| (0..m).map(|_| rng.next()).collect()).collect();
        let mut best: Vec<Pair> = Vec::new();
        let mut converged = false;
        let mut iterations = 0;
        for it in 0..max_iter {
            iterations = it + 1;
            // Y = (K - sigma B)^-1 B X
            let y: Vec<Vec<f64>> = x
                .par_iter()
                .map(|col| {
                    let mut r = mul(b, col);
                    fac.solve_reduced(&mut r);
                    r
                })
                .collect();
            let ky: Vec<Vec<f64>> = y.par_iter().map(|c| mul(k, c)).collect();
            let by: Vec<Vec<f64>> = y.par_iter().map(|c| mul(b, c)).collect();
            let proj = |v: &[Vec<f64>]| -> Vec<f64> {
                let mut out = vec![0.0; p * p];
                for i in 0..p {
                    for j in 0..=i {
                        let s = dot(&y[i], &v[j]);
                        out[i * p + j] = s;
                        out[j * p + i] = s;
                    }
                }
                out
            };
            let kt = proj(&ky);
            let bt = proj(&by);
            // Ritz problem; order the pairs by interest.
            let (order, lambdas, z): (Vec<usize>, Vec<f64>, Vec<f64>) = match kind {
                Kind::Modal => {
                    let (val, vec) = sym_gen_eigen(&kt, &bt, p)?;
                    let mut ord: Vec<usize> = (0..p).collect();
                    ord.sort_by(|&i, &j| (val[i] - sigma).abs().total_cmp(&(val[j] - sigma).abs()));
                    (ord, val, vec)
                }
                Kind::Buckling => {
                    // `B~ z = theta (K~ - sigma B~) z` with `theta = 1 / (lambda - sigma)`; the factors nearest the shift first.
                    let at: Vec<f64> = kt.iter().zip(&bt).map(|(a, bb)| a - sigma * bb).collect();
                    let (theta, vec) = sym_gen_eigen(&bt, &at, p)?;
                    let mut ord: Vec<usize> = (0..p).filter(|&i| theta[i] != 0.0).collect();
                    ord.sort_by(|&i, &j| theta[j].abs().total_cmp(&theta[i].abs()));
                    let lam: Vec<f64> = theta.iter().map(|t| if *t != 0.0 { sigma + 1.0 / t } else { f64::INFINITY }).collect();
                    (ord, lam, vec)
                }
            };
            // New subspace: all Ritz vectors in order (the wanted ones first), then the rest.
            let rest: Vec<usize> = (0..p).filter(|i| !order.contains(i)).collect();
            let all: Vec<usize> = order.iter().copied().chain(rest).collect();
            x = all
                .iter()
                .map(|&kz| {
                    let mut v = vec![0.0; m];
                    for (i, yi) in y.iter().enumerate() {
                        let c = z[kz * p + i];
                        for (vv, yy) in v.iter_mut().zip(yi) {
                            *vv += c * yy;
                        }
                    }
                    v
                })
                .collect();
            let wanted = order.len().min(nev);
            best = (0..wanted)
                .map(|w| {
                    let lam = lambdas[order[w]];
                    let v = &x[w];
                    let kv = mul(k, v);
                    let bv = mul(b, v);
                    let r: f64 = kv.iter().zip(&bv).map(|(a, bb)| (a - lam * bb).powi(2)).sum::<f64>().sqrt();
                    Pair { lambda: lam, x: v.clone(), residual: (r - noise * dot(v, v).sqrt()).max(0.0) / (dot(&kv, &kv).sqrt() + lam.abs() * dot(&bv, &bv).sqrt()).max(1e-300) }
                })
                .collect();
            if std::env::var("EIG_TRACE").is_ok() {
                eprintln!("eig it {it}: lambda {:?} residual {:?}", best.iter().map(|q| q.lambda).collect::<Vec<_>>(), best.iter().map(|q| format!("{:.1e}", q.residual)).collect::<Vec<_>>());
            }
            if best.len() == nev && best.iter().all(|q| q.lambda.is_finite() && q.residual.is_finite() && q.residual <= tol && q.x.iter().all(|x| x.is_finite())) {
                converged = true;
                break;
            }
        }
        if best.len() < nev {
            return Err(format!("only {} of the {nev} requested modes were found in a subspace of {p} (positive buckling factors; a reference load that does not compress the structure has none)", best.len()));
        }
        if !converged {
            let worst = best.iter().map(|q| q.residual).fold(0.0f64, f64::max);
            return Err(format!("the eigensolver did not reach residual {tol:e} in {max_iter} iterations (worst residual {worst:.2e}, subspace {p})"));
        }
        Ok(Pairs { pairs: best, iterations, subspace: p, factor_nnz: red.factor_nnz(), fallback: fallback_note.take() })
    }

    fn expand(&self, bc: &Dirichlet, x: &[f64]) -> Vec<f64> {
        let red = Reduced::structure_only(&self.pattern, bc).expect("a constrained system");
        let mut u = vec![0.0; self.mesh.n_dofs()];
        red.scatter(x, &mut u);
        u
    }

    fn validate_dynamic_constraints(&self, bc: &Dirichlet) -> Result<(), String> {
        let n = self.mesh.n_dofs();
        if bc.d != self.mesh.dim() || bc.fixed.len() != n || bc.value.len() != n || bc.value.iter().any(|v| !v.is_finite()) {
            return Err("dynamics: invalid constraint dimensions or nonfinite values".into());
        }
        Ok(())
    }

    /// Natural frequencies and mode shapes of the constrained structure (homogeneous Dirichlet conditions).
    pub fn modal(&self, bc: &Dirichlet, opt: &ModalOptions) -> Result<Modal, String> {
        self.validate_dynamic_constraints(bc)?;
        if !opt.tol.is_finite() || opt.tol <= 0.0 || !opt.shift.is_finite() || opt.max_iter == 0 || opt.n_modes == 0 {
            return Err("modal: finite positive tolerance, finite shift, and nonzero modes/iterations are required".into());
        }
        self.require_mass()?;
        if bc.fixed.iter().zip(&bc.value).any(|(f, v)| *f && *v != 0.0) {
            return Err("a modal analysis needs homogeneous constraints (prescribed displacements are not part of the free vibration)".into());
        }
        if opt.shift >= 0.0 {
            self.check_constrained(bc)?;
        }
        let k = self.assemble()?;
        let mass = self.assemble_mass(opt.mass)?;
        let r = self.eig_pencil(bc, &k, &mass, opt.shift, Kind::Modal, opt.n_modes, opt.tol, opt.max_iter, opt.method)?;
        let d = self.mesh.dim();
        let total_mass = self.total_mass()?;
        let mut modes = Vec::new();
        for q in &r.pairs {
            let mut shape = self.expand(bc, &q.x);
            let mut mphi = vec![0.0; shape.len()];
            mass.matvec_add(&self.pattern, &shape, &mut mphi);
            let norm = dot(&shape, &mphi).sqrt();
            if !norm.is_finite() || norm <= 0.0 || !q.lambda.is_finite() || !q.residual.is_finite() || q.residual > opt.tol {
                return Err("modal: eigenpair failed finite mass-norm/residual acceptance".into());
            }
            shape.iter_mut().for_each(|v| *v /= norm);
            mphi.iter_mut().for_each(|v| *v /= norm);
            let mut eff = [0.0; 3];
            for (axis, e) in eff.iter_mut().enumerate().take(d) {
                let gamma: f64 = (0..self.mesh.nodes.len()).map(|nd| mphi[nd * d + axis]).sum();
                *e = gamma * gamma;
            }
            modes.push(Mode { omega2: q.lambda, frequency_hz: q.lambda.max(0.0).sqrt() / (2.0 * std::f64::consts::PI), shape, residual: q.residual, effective_mass: eff });
        }
        Ok(Modal { modes, total_mass, iterations: r.iterations, subspace: r.subspace, factor_nnz: r.factor_nnz, fallback: r.fallback })
    }

    /// Linear (eigenvalue) buckling: the smallest positive multipliers `lambda` of `loads` (and its prescribed displacements, taken
    /// as part of the reference state) for which `K + lambda K_G(sigma_ref)` is singular. The prestress is the linear
    /// static solution of the reference load; supports are those of `bc`. The reference load must put the structure in
    /// compression somewhere or no positive factor exists.
    pub fn buckling(&self, loads: &Loads, bc: &Dirichlet, opt: &BucklingOptions) -> Result<Buckling, String> {
        self.validate_dynamic_constraints(bc)?;
        if !opt.tol.is_finite() || opt.tol <= 0.0 || opt.max_iter == 0 || opt.n_modes == 0 || opt.n_modes > self.mesh.n_dofs() {
            return Err("buckling: finite positive tolerance and valid nonzero modes/iterations are required".into());
        }
        self.check_constrained(bc)?;
        let sol = self.solve_static(loads, bc)?;
        let k = self.assemble()?;
        let kg = self.assemble_geometric_stiffness(&sol.u, loads.delta_t)?;
        let neg_kg = BlockMatrix { d: kg.d, vals: kg.vals.iter().map(|v| -v).collect() };
        // The eigensolvers find the factors of largest 1/|lambda| (both signs: the reversed load buckles too); the positive ones
        // are kept. Ask for more until `n_modes` of them are in hand.
        let mut total = 2 * opt.n_modes + 2;
        let r = loop {
            let found = self.eig_pencil(bc, &k, &neg_kg, 0.0, Kind::Buckling, total, opt.tol, opt.max_iter, opt.method)?;
            let positive = found.pairs.iter().filter(|q| q.lambda > 0.0 && q.lambda.is_finite()).count();
            if positive >= opt.n_modes {
                break found;
            }
            if total >= 8 * opt.n_modes + 16 || total >= self.mesh.n_dofs() / 2 {
                return Err(format!("only {positive} of the {} requested positive buckling factors were found among the {total} modes of smallest |factor| (a reference load that does not compress the structure has none)", opt.n_modes));
            }
            total *= 2;
        };
        let mut pairs: Vec<&Pair> = r.pairs.iter().filter(|q| q.lambda > 0.0 && q.lambda.is_finite()).collect();
        pairs.sort_by(|a, b| a.lambda.total_cmp(&b.lambda));
        pairs.truncate(opt.n_modes);
        let modes = pairs
            .iter()
            .map(|q| {
                let mut shape = self.expand(bc, &q.x);
                let peak = shape.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-300);
                shape.iter_mut().for_each(|v| *v /= peak);
                BucklingMode { load_factor: q.lambda, shape, residual: q.residual }
            })
            .collect();
        Ok(Buckling { modes, iterations: r.iterations, fallback: r.fallback })
    }
}

/// Largest positive eigenvalue estimate (a Ritz value, so a lower bound) of `K^-1 B` from a short Lanczos process in the
/// `K` inner product; `None` when the process finds none.
fn largest_positive_theta(m: usize, nev: usize, mul: &(dyn Fn(&BlockMatrix, &[f64]) -> Vec<f64> + Sync), k: &BlockMatrix, b: &BlockMatrix, fac: &crate::linear::Factor<'_>) -> Option<f64> {
    let steps = (4 * nev + 24).min(m);
    let mut rng = Lcg(0x2545_F491_4F6C_DD1D);
    let mut q: Vec<Vec<f64>> = Vec::new();
    let mut kq: Vec<Vec<f64>> = Vec::new();
    let (mut alpha, mut beta): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    let mut w: Vec<f64> = (0..m).map(|_| rng.next()).collect();
    for _ in 0..steps {
        let kw = mul(k, &w);
        let nrm = dot(&w, &kw).sqrt();
        if nrm.is_nan() || nrm <= 0.0 {
            break;
        }
        w.iter_mut().for_each(|v| *v /= nrm);
        q.push(w.clone());
        kq.push(kw.iter().map(|v| v / nrm).collect());
        let j = q.len() - 1;
        let mut r = mul(b, &q[j]);
        fac.solve_reduced(&mut r);
        let mut a_j = 0.0;
        for pass in 0..2 {
            for i in 0..q.len() {
                let c = dot(&r, &kq[i]);
                if i == j && pass == 0 {
                    a_j = c;
                }
                for (rv, qv) in r.iter_mut().zip(&q[i]) {
                    *rv -= c * qv;
                }
            }
        }
        alpha.push(a_j);
        let kr = mul(k, &r);
        let b_j = dot(&r, &kr).abs().sqrt();
        if b_j < 1e-12 * a_j.abs().max(1.0) {
            break;
        }
        beta.push(b_j);
        w = r;
    }
    let dim = alpha.len();
    let mut t = vec![0.0; dim * dim];
    for i in 0..dim {
        t[i * dim + i] = alpha[i];
        if i + 1 < dim {
            t[i * dim + i + 1] = beta[i];
            t[(i + 1) * dim + i] = beta[i];
        }
    }
    let (theta, _) = crate::dense::sym_eigen(&t, dim);
    theta.into_iter().filter(|v| *v > 0.0).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |x| x.max(v))))
}

/// The method `EigMethod::Auto` resolves to (set from the benchmark of `tests/bench.rs::modal_analysis_cost`).
const AUTO_METHOD: EigMethod = EigMethod::Lanczos;

/// Lanczos on `T = (K - sigma B)^-1 B`, self-adjoint in the `G` inner product (`B` for modal, `K` for buckling), with
/// full reorthogonalisation. The wanted Ritz values are those of largest `|theta|` (modal; `lambda = sigma + 1/theta`)
/// or largest positive `theta` (buckling; `lambda = 1/theta`). The subspace grows until the Ritz error estimates
/// `beta_j |s_ji|` of the wanted pairs fall below `tol |theta|`.
///
/// A single-vector Lanczos process can miss part of a multiple eigenvalue (symmetric structures have many: a square
/// column buckles identically in both planes). Completeness is therefore checked, not assumed: after the pairs
/// converge, a second process runs with the found vectors deflated; if it finds a value that belongs among the wanted
/// ones, that pair replaces the least wanted one and the check repeats. Every pair still goes through the true residual.
#[allow(clippy::too_many_arguments)]
fn lanczos(m: usize, nev: usize, tol: f64, max_iter: usize, kind: Kind, sigma: f64, mul: &(dyn Fn(&BlockMatrix, &[f64]) -> Vec<f64> + Sync), k: &BlockMatrix, b: &BlockMatrix, fac: &crate::linear::AnyFactor<'_>, pair_of: &dyn Fn(f64, &[f64]) -> Pair, factor_nnz: usize) -> Result<Pairs, String> {
    let g_mat = if kind == Kind::Modal { b } else { k }; // (for buckling `k` is the shifted SPD matrix `K - sigma B`)
    let theta_of = |lam: f64| 1.0 / (lam - sigma);
    // `interest`: larger is more wanted (|theta| for both problems; buckling keeps the positive factors afterwards).
    let interest = |lam: f64| theta_of(lam).abs();
    let ctx = LanczosCtx { m, tol, max_iter, kind, sigma, mul, g_mat, k, b, fac, pair_of };
    let mut dim_total = 0;
    let (mut found, d) = ctx.run(nev, &[])?;
    dim_total += d;
    // Completeness check by deflated restarts.
    for _ in 0..nev + 4 {
        let locked: Vec<(Vec<f64>, Vec<f64>)> = found.iter().map(|(_, x)| {
            let gx = mul(g_mat, x);
            let nrm = dot(x, &gx).sqrt();
            (x.iter().map(|v| v / nrm).collect(), gx.iter().map(|v| v / nrm).collect())
        }).collect();
        let Ok((extra, d)) = ctx.run(1, &locked) else { break };
        dim_total += d;
        let Some((lam, x)) = extra.into_iter().next() else { break };
        let (worst_i, worst) = found.iter().enumerate().map(|(i, (l, _))| (i, interest(*l))).fold((0, f64::INFINITY), |a, c| if c.1 < a.1 { c } else { a });
        if interest(lam) > worst * (1.0 + 1e-8) {
            found[worst_i] = (lam, x);
        } else {
            break;
        }
    }
    found.sort_by(|a, b| interest(b.0).total_cmp(&interest(a.0)));
    let pairs: Vec<Pair> = found.iter().map(|(l, x)| pair_of(*l, x)).collect();
    let worst = pairs.iter().map(|p| p.residual).fold(0.0f64, f64::max);
    if !worst.is_finite() || worst > tol || pairs.iter().any(|p| !p.lambda.is_finite() || !p.residual.is_finite() || p.x.iter().any(|x| !x.is_finite())) {
        return Err(format!("Lanczos did not reach residual {tol:e} (worst residual {worst:.2e}); try EigMethod::Subspace or a larger max_iter"));
    }
    Ok(Pairs { pairs, iterations: dim_total, subspace: dim_total, factor_nnz, fallback: None })
}

/// An eigenvalue and its vector.
type RitzPair = (f64, Vec<f64>);

struct LanczosCtx<'a> {
    m: usize,
    tol: f64,
    max_iter: usize,
    kind: Kind,
    sigma: f64,
    mul: &'a (dyn Fn(&BlockMatrix, &[f64]) -> Vec<f64> + Sync),
    g_mat: &'a BlockMatrix,
    k: &'a BlockMatrix,
    b: &'a BlockMatrix,
    fac: &'a crate::linear::AnyFactor<'a>,
    pair_of: &'a dyn Fn(f64, &[f64]) -> Pair,
}

impl LanczosCtx<'_> {
    /// One Lanczos process for the `nev` most wanted pairs orthogonal (in `G`) to `locked = (x, G x)`. Returns the pairs
    /// (eigenvalue, vector) and the Krylov dimension used.
    fn run(&self, nev: usize, locked: &[(Vec<f64>, Vec<f64>)]) -> Result<(Vec<RitzPair>, usize), String> {
        let Self { m, tol, max_iter, sigma, mul, g_mat, b, fac, pair_of, .. } = *self;
        let _ = self.k;
        let max_dim = if locked.is_empty() { (nev * 3 + 30).max(max_iter.min(nev * 3 + 30)) } else { 40 }.min(m.saturating_sub(locked.len())).max(1);
        let deflate = |r: &mut Vec<f64>| {
            for (x, gx) in locked {
                let c = dot(r, gx);
                for (rv, xv) in r.iter_mut().zip(x) {
                    *rv -= c * xv;
                }
            }
        };
        let mut q: Vec<Vec<f64>> = Vec::new();
        let mut gq: Vec<Vec<f64>> = Vec::new();
        let (mut alpha, mut beta): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
        let mut rng = Lcg(0x9E37_79B9_7F4A_7C15 ^ (locked.len() as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
        let mut w: Vec<f64> = (0..m).map(|_| rng.next()).collect();
        deflate(&mut w);
        let mut result: Option<Vec<(f64, Vec<f64>)>> = None;
        let mut dim_done = 0;
        loop {
            let gw = mul(g_mat, &w);
            let nrm = dot(&w, &gw).sqrt();
            if nrm.is_nan() || nrm <= 0.0 || !nrm.is_finite() {
                break;
            }
            w.iter_mut().for_each(|v| *v /= nrm);
            let gw: Vec<f64> = gw.iter().map(|v| v / nrm).collect();
            q.push(w.clone());
            gq.push(gw);
            let j = q.len() - 1;
            let mut r = mul(b, &q[j]);
            fac.solve_reduced(&mut r);
            deflate(&mut r);
            let mut a_j = 0.0;
            for pass in 0..2 {
                for i in 0..q.len() {
                    let c = dot(&r, &gq[i]);
                    if i == j && pass == 0 {
                        a_j = c;
                    }
                    for (rv, qv) in r.iter_mut().zip(&q[i]) {
                        *rv -= c * qv;
                    }
                }
                deflate(&mut r);
            }
            alpha.push(a_j);
            let gr = mul(g_mat, &r);
            let b_j = dot(&r, &gr).abs().sqrt();
            let dim = q.len();
            let at_limit = dim >= max_dim || dim + locked.len() >= m;
            let exhausted = b_j < 1e-12 * a_j.abs().max(1.0);
            #[allow(clippy::manual_is_multiple_of)] // is_multiple_of needs a newer toolchain than the CI pin
            let check_now = dim % 4 == 0;
            if dim >= nev && (check_now || at_limit || exhausted) {
                let mut t = vec![0.0; dim * dim];
                for i in 0..dim {
                    t[i * dim + i] = alpha[i];
                    if i + 1 < dim {
                        t[i * dim + i + 1] = beta[i];
                        t[(i + 1) * dim + i] = beta[i];
                    }
                }
                let (theta, s) = crate::dense::sym_eigen(&t, dim);
                let mut order: Vec<usize> = (0..dim).filter(|&i| theta[i] != 0.0).collect();
                order.sort_by(|&i, &jx| theta[jx].abs().total_cmp(&theta[i].abs()));
                let wanted: Vec<usize> = order.into_iter().take(nev).collect();
                let est_ok = wanted.len() == nev && wanted.iter().all(|&i| b_j * s[i * dim + dim - 1].abs() <= 0.1 * tol * theta[i].abs().max(1e-300));
                dim_done = dim;
                if est_ok || at_limit || exhausted {
                    let pairs: Vec<(f64, Vec<f64>)> = wanted
                        .iter()
                        .map(|&i| {
                            let mut x = vec![0.0; m];
                            for (c, qv) in s[i * dim..(i + 1) * dim].iter().zip(&q) {
                                for (xv, v) in x.iter_mut().zip(qv) {
                                    *xv += c * v;
                                }
                            }
                            let lam = sigma + 1.0 / theta[i];
                            (lam, x)
                        })
                        .collect();
                    // The estimate only says when to look; the true residual decides.
                    let verified = pairs.len() == nev && pairs.iter().all(|(l, x)| pair_of(*l, x).residual <= tol);
                    result = Some(pairs);
                    if (est_ok && verified) || at_limit || exhausted {
                        break;
                    }
                }
            }
            beta.push(b_j);
            w = r;
        }
        let found = result.ok_or("Lanczos found no Ritz pair")?;
        if found.len() < nev {
            return Err(format!("only {} of the {nev} requested modes were found (positive buckling factors; a reference load that does not compress the structure has none)", found.len()));
        }
        Ok((found, dim_done))
    }
}

/// Time integration parameters: Newmark (`alpha = 0`) or HHT-alpha (`alpha` in `[-1/3, 0]`, numerical dissipation of the
/// high frequencies at second-order accuracy).
#[derive(Debug, Clone)]
pub struct TransientOptions {
    pub dt: f64,
    pub steps: usize,
    pub alpha: f64,
    /// Rayleigh damping `C = a0 M + a1 K`.
    pub rayleigh: (f64, f64),
    pub mass: MassKind,
    /// Dofs whose displacement history is recorded at every step.
    pub record: Vec<usize>,
}

impl Default for TransientOptions {
    fn default() -> Self {
        Self { dt: 1e-3, steps: 100, alpha: 0.0, rayleigh: (0.0, 0.0), mass: MassKind::Consistent, record: Vec::new() }
    }
}

#[derive(Debug, Clone)]
pub struct Transient {
    pub times: Vec<f64>,
    /// `history[i][s]`: displacement of `record[i]` at `times[s]`.
    pub history: Vec<Vec<f64>>,
    /// Kinetic plus strain energy at each time (conserved by an undamped, unloaded Newmark average-acceleration run).
    pub energy: Vec<f64>,
    /// Normwise backward error of each effective acceleration solve, including the initial mass solve.
    /// Every value is finite and at most `1e-8`; physical energy remains a separate diagnostic.
    pub residuals: Vec<f64>,
    pub u: Vec<f64>,
    pub v: Vec<f64>,
}

impl Model {
    /// Linear transient response of `K u + C v + M a = g(t) f` from the state `(u0, v0)` (zero when `None`), `f` the
    /// assembly of `loads`, homogeneous constraints. Constant step, one factorisation of the effective matrix.
    pub fn transient(&self, loads: &Loads, g: &dyn Fn(f64) -> f64, bc: &Dirichlet, init: Option<(&[f64], &[f64])>, opt: &TransientOptions) -> Result<Transient, String> {
        if !opt.dt.is_finite() || opt.dt <= 0.0 || !(-1.0 / 3.0 - 1e-12..=1e-12).contains(&opt.alpha) {
            return Err("transient: dt must be positive and alpha within [-1/3, 0]".into());
        }
        let n = self.mesh.n_dofs();
        if bc.d != self.mesh.physics.dim() || bc.fixed.len() != n || bc.value.len() != n
            || bc.value.iter().any(|v| !v.is_finite()) {
            return Err("transient: invalid constraint dimensions or nonfinite values".into());
        }
        if !opt.rayleigh.0.is_finite() || !opt.rayleigh.1.is_finite()
            || opt.rayleigh.0 < 0.0 || opt.rayleigh.1 < 0.0 {
            return Err("transient: Rayleigh coefficients must be finite and nonnegative".into());
        }
        if !(opt.steps as f64 * opt.dt).is_finite() || opt.record.iter().any(|&d| d >= n) {
            return Err("transient: nonfinite final time or history DOF out of range".into());
        }
        if let Some((u, v)) = init {
            if u.len() != n || v.len() != n || u.iter().chain(v).any(|x| !x.is_finite()) {
                return Err("transient: initial state must have the correct length and finite values".into());
            }
        }
        if bc.fixed.iter().zip(&bc.value).any(|(f, v)| *f && *v != 0.0) {
            return Err("transient: prescribed non-zero displacements are not supported (use an equivalent load)".into());
        }
        self.check_constrained(bc)?;
        let k = self.assemble()?;
        let mass = self.assemble_mass(opt.mass)?;
        let (a0, a1) = opt.rayleigh;
        let c = BlockMatrix { d: mass.d, vals: mass.vals.iter().zip(&k.vals).map(|(m, kk)| a0 * m + a1 * kk).collect() };
        let alpha = opt.alpha;
        let beta = (1.0 - alpha) * (1.0 - alpha) / 4.0;
        let gamma = 0.5 - alpha;
        let dt = opt.dt;
        let eff = BlockMatrix { d: k.d, vals: (0..k.vals.len()).map(|i| mass.vals[i] + (1.0 + alpha) * gamma * dt * c.vals[i] + (1.0 + alpha) * beta * dt * dt * k.vals[i]).collect() };
        let red = Reduced::with_ordering(&self.pattern, bc, Some(&self.mesh.nodes), Ordering::Auto).map_err(|e| e.to_string())?;
        let fac_eff = red.factor(&eff).map_err(|e| e.to_string())?;
        let fac_m = red.factor(&mass).map_err(|e| e.to_string())?;
        let n = self.mesh.n_dofs();
        let free: Vec<usize> = red.free_dofs().iter().map(|&d| d as usize).collect();
        let f_ref = loads::assemble(&self.mesh, loads)?;
        let mul = |mat: &BlockMatrix, x: &[f64]| -> Vec<f64> {
            let mut y = vec![0.0; n];
            mat.matvec_add(&self.pattern, x, &mut y);
            y
        };
        let (mut u, mut v) = match init {
            Some((u0, v0)) => (u0.to_vec(), v0.to_vec()),
            None => (vec![0.0; n], vec![0.0; n]),
        };
        for (i, fx) in bc.fixed.iter().enumerate() {
            if *fx {
                u[i] = 0.0;
                v[i] = 0.0;
            }
        }
        let backward_error = |mat: &BlockMatrix, x: &[f64], rhs: &[f64]| -> Result<f64, String> {
            let ax = mul(mat, x);
            let abs_mat = BlockMatrix { d: mat.d, vals: mat.vals.iter().map(|v| v.abs()).collect() };
            let scale = mul(&abs_mat, &x.iter().map(|v| v.abs()).collect::<Vec<_>>());
            let mut rn = 0.0_f64;
            let mut sn = 0.0_f64;
            for &i in &free {
                rn = rn.hypot(ax[i] - rhs[i]);
                sn = sn.hypot(scale[i] + rhs[i].abs());
            }
            let r = rn / sn.max(1e-300);
            if !rn.is_finite() || !sn.is_finite() || !r.is_finite() || r > 1e-8 {
                return Err(format!("transient: effective-system residual {r:e} exceeds 1e-8 or is nonfinite"));
            }
            Ok(r)
        };
        let load_at = |t: f64| -> Result<Vec<f64>, String> {
            let scale = g(t);
            let f: Vec<f64> = f_ref.iter().map(|x| x * scale).collect();
            if !scale.is_finite() || f.iter().any(|x| !x.is_finite()) {
                return Err(format!("transient: nonfinite load at time {t}"));
            }
            Ok(f)
        };
        let mut f_prev = load_at(0.0)?;
        // a0 = M^-1 (f0 - C v0 - K u0)
        let (cv0, ku0) = (mul(&c, &v), mul(&k, &u));
        let initial_rhs: Vec<f64> = (0..n).map(|d| f_prev[d] - cv0[d] - ku0[d]).collect();
        let mut a = {
            let mut r: Vec<f64> = free.iter().map(|&d| initial_rhs[d]).collect();
            fac_m.solve_reduced(&mut r);
            let mut a = vec![0.0; n];
            for (&d, x) in free.iter().zip(&r) {
                a[d] = *x;
            }
            a
        };
        let energy = |u: &[f64], v: &[f64]| -> f64 { 0.5 * dot(v, &mul(&mass, v)) + 0.5 * dot(u, &mul(&k, u)) };
        let mut out = Transient { times: vec![0.0], history: opt.record.iter().map(|&d| vec![u[d]]).collect(), energy: vec![energy(&u, &v)], residuals: vec![backward_error(&mass, &a, &initial_rhs)?], u: Vec::new(), v: Vec::new() };
        if !out.energy[0].is_finite() || a.iter().any(|x| !x.is_finite()) {
            return Err("transient: nonfinite initial acceleration or energy".into());
        }
        for s in 1..=opt.steps {
            let t = s as f64 * dt;
            let f_new = load_at(t)?;
            let u_pred: Vec<f64> = (0..n).map(|i| u[i] + dt * v[i] + dt * dt * (0.5 - beta) * a[i]).collect();
            let v_pred: Vec<f64> = (0..n).map(|i| v[i] + dt * (1.0 - gamma) * a[i]).collect();
            let (c_vp, c_v, k_up, k_u) = (mul(&c, &v_pred), mul(&c, &v), mul(&k, &u_pred), mul(&k, &u));
            let mut rhs: Vec<f64> = free.iter().map(|&d| (1.0 + alpha) * f_new[d] - alpha * f_prev[d] - (1.0 + alpha) * c_vp[d] + alpha * c_v[d] - (1.0 + alpha) * k_up[d] + alpha * k_u[d]).collect();
            let mut full_rhs = vec![0.0; n];
            for (&d, &r) in free.iter().zip(&rhs) { full_rhs[d] = r; }
            fac_eff.solve_reduced(&mut rhs);
            let mut a_new = vec![0.0; n];
            for (&d, x) in free.iter().zip(&rhs) {
                a_new[d] = *x;
            }
            out.residuals.push(backward_error(&eff, &a_new, &full_rhs)?);
            for i in 0..n {
                u[i] = u_pred[i] + beta * dt * dt * a_new[i];
                v[i] = v_pred[i] + gamma * dt * a_new[i];
            }
            a = a_new;
            f_prev = f_new;
            out.times.push(t);
            for (h, &d) in out.history.iter_mut().zip(&opt.record) {
                h.push(u[d]);
            }
            let e = energy(&u, &v);
            if !e.is_finite() || u.iter().chain(&v).chain(&a).any(|x| !x.is_finite()) {
                return Err(format!("transient: nonfinite state or energy at step {s}"));
            }
            out.energy.push(e);
        }
        out.u = u;
        out.v = v;
        Ok(out)
    }
}
