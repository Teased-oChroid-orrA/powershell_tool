//! Audit: the assembled contact tangent (matrix, extra-unknown couplings and frictional defect) against central
//! differences of the assembled contact residual.

#![allow(clippy::needless_range_loop)]

use fea_core::assembly::{BlockMatrix, Pattern};
use fea_core::contact::{ContactSet, ContactSpec, ExtraOut, RigidMaster, RigidShape};
use fea_core::generate::grid;
use fea_core::*;

/// Dense `n x n` copy of the (lower-block-stored, symmetric) block matrix.
fn dense(k: &BlockMatrix, pat: &Pattern, n: usize) -> Vec<f64> {
    let mut out = vec![0.0; n * n];
    let mut e = vec![0.0; n];
    let mut col = vec![0.0; n];
    for j in 0..n {
        e[j] = 1.0;
        col.fill(0.0);
        k.matvec_add(pat, &e, &mut col);
        e[j] = 0.0;
        for i in 0..n {
            out[i * n + j] = col[i];
        }
    }
    out
}

struct Eval {
    f: Vec<f64>,
    k: Vec<f64>,
    rq: Vec<f64>,
    kuq: Vec<f64>,
    kqq: Vec<f64>,
    defect: Vec<(Vec<usize>, Vec<f64>)>,
    n_active: usize,
}

fn eval(cs: &ContactSet, mesh: &Mesh, pat: &Pattern, u: &[f64], q: &[f64], old: &[Vec<fea_core::contact::CpState>], want_k: bool) -> Eval {
    let n = mesh.n_dofs();
    let m = cs.n_extra();
    let mut new = old.to_vec();
    let mut f = vec![0.0; n];
    let mut k = BlockMatrix::zeros(pat);
    let mut ex = ExtraOut::new(n, m);
    let st = cs.eval(mesh, pat, u, q, 0.0, old, &mut new, &mut f, if want_k { Some(&mut k) } else { None }, if m > 0 { Some(&mut ex) } else { None }).unwrap();
    assert!(st.missing_pairs.is_empty(), "missing pattern pairs {:?}", st.missing_pairs);
    let kd = if want_k { dense(&k, pat, n) } else { Vec::new() };
    Eval { f, k: kd, rq: ex.rq, kuq: ex.kuq, kqq: ex.kqq, defect: st.defect, n_active: st.n_active }
}

/// Compare `K (+ defect)` with central differences of `f` over every dof of the contact nodes (and every extra unknown).
fn check(name: &str, mesh: &Mesh, specs: Vec<ContactSpec>, u: &[f64], q: &[f64], with_defect: bool) {
    let cs = ContactSet::new(mesh, specs).unwrap();
    let pat = Pattern::new_with_extra(mesh, &cs.pattern_pairs(mesh)).unwrap();
    let old = cs.initial_state(mesh, &pat);
    let n = mesh.n_dofs();
    let m = cs.n_extra();
    let e0 = eval(&cs, mesh, &pat, u, q, &old, true);
    assert!(e0.n_active > 0, "{name}: nothing in contact");
    let mut k = e0.k.clone();
    if with_defect {
        assert!(!e0.defect.is_empty(), "{name}: a frictional contact must report its defect");
        for (dofs, dm) in &e0.defect {
            let nd = dofs.len();
            for a in 0..nd {
                for b in 0..nd {
                    if dofs[a] < n && dofs[b] < n {
                        k[dofs[a] * n + dofs[b]] += dm[a * nd + b];
                    }
                }
            }
        }
    }
    let scale = k.iter().fold(0.0f64, |s, v| s.max(v.abs()));
    let touched: Vec<usize> = (0..n).filter(|&j| (0..n).any(|i| k[i * n + j] != 0.0)).collect();
    assert!(!touched.is_empty());
    let mut worst = (0.0f64, 0, 0);
    for &j in &touched {
        let h = 1e-7;
        let (mut up, mut um) = (u.to_vec(), u.to_vec());
        up[j] += h;
        um[j] -= h;
        let (ep, em) = (eval(&cs, mesh, &pat, &up, q, &old, false), eval(&cs, mesh, &pat, &um, q, &old, false));
        assert_eq!((ep.n_active, em.n_active), (e0.n_active, e0.n_active), "{name}: the active set changed under the probe");
        for i in 0..n {
            let fd = (ep.f[i] - em.f[i]) / (2.0 * h);
            let err = (fd - k[i * n + j]).abs();
            if err > worst.0 {
                worst = (err, i, j);
            }
        }
        for a in 0..m {
            // d rq_a / d u_j is the coupling block (stored once, symmetric).
            let fd = (ep.rq[a] - em.rq[a]) / (2.0 * h);
            let err = (fd - e0.kuq[a * n + j]).abs();
            assert!(err <= 1e-5 * scale, "{name}: d rq[{a}] / d u[{j}]: fd {fd} vs kuq {}", e0.kuq[a * n + j]);
        }
    }
    let (i, j) = (worst.1, worst.2);
    assert!(worst.0 <= 1e-5 * scale, "{name}: K[{i},{j}] = {} vs fd error {} (scale {scale})", k[i * n + j], worst.0);
    for b in 0..m {
        let h = 1e-7;
        let (mut qp, mut qm) = (q.to_vec(), q.to_vec());
        qp[b] += h;
        qm[b] -= h;
        let (ep, em) = (eval(&cs, mesh, &pat, u, &qp, &old, false), eval(&cs, mesh, &pat, u, &qm, &old, false));
        for a in 0..m {
            let fd = (ep.rq[a] - em.rq[a]) / (2.0 * h);
            assert!((fd - e0.kqq[a * m + b]).abs() <= 1e-5 * scale, "{name}: kqq[{a},{b}] = {} vs fd {fd}", e0.kqq[a * m + b]);
        }
        for i in 0..n {
            let fd = (ep.f[i] - em.f[i]) / (2.0 * h);
            assert!((fd - e0.kuq[b * n + i]).abs() <= 1e-5 * scale, "{name}: d f[{i}] / d q[{b}]: fd {fd} vs kuq {}", e0.kuq[b * n + i]);
        }
    }
    eprintln!("{name}: {} active points, worst |K - fd| = {:.3e} of {scale:.3e}", e0.n_active, worst.0);
}

/// Deterministic small perturbation field.
fn wiggle(n: usize, amp: f64) -> Vec<f64> {
    (0..n).map(|i| amp * ((i as f64 * 1.618_034).sin() + 0.3 * (i as f64 * 0.7).cos())).collect()
}

fn stacked(kind: ElementKind, three_d: bool) -> (Mesh, Vec<usize>) {
    let mat = Elastic::new(1.0e4, 0.3);
    let (phys, db, dt) = if three_d { (Physics::Solid, [2, 1, 2], [3, 1, 1]) } else { (Physics::PlaneStrain { thickness: 1.0 }, [3, 1, 1], [2, 1, 1]) };
    let bottom = grid(phys, kind, mat, db, &|p| [2.0 * p[0], p[1], p[2]]).unwrap();
    let top = grid(phys, kind, mat, dt, &|p| [2.0 * p[0], 1.0 + p[1], p[2]]).unwrap();
    let mut mesh = bottom;
    let first_top = mesh.append(&top, "top_").unwrap();
    let top_nodes: Vec<usize> = (first_top..mesh.nodes.len()).collect();
    (mesh, top_nodes)
}

#[test]
fn the_assembled_contact_tangent_matches_finite_differences_of_the_residual() {
    for (kind, three_d) in [(ElementKind::Quad9, false), (ElementKind::Quad4, false), (ElementKind::Hex8, true), (ElementKind::Hex20, true)] {
        for mu in [0.0, 0.3] {
            let (mesh, top) = stacked(kind, three_d);
            let d = mesh.dim();
            let mut u = wiggle(mesh.n_dofs(), 1e-4);
            for &nd in &top {
                u[nd * d + 1] -= 0.01; // uniform interference, far from the pressure kink
                u[nd * d] += 0.002; // and a slip (sticking: eps_t * slip far below mu p)
            }
            let mut spec = ContactSpec::deformable("i", mesh.surfaces["top_v0"].clone(), mesh.surfaces["v1"].clone(), 1.0e5);
            if mu > 0.0 {
                spec = spec.with_friction(mu, 1.0e2);
            }
            check(&format!("{kind:?} mu {mu}"), &mesh, vec![spec], &u, &[], mu > 0.0);
        }
    }
}

#[test]
fn the_rigid_master_couplings_match_finite_differences() {
    let mesh = grid(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Quad9, Elastic::new(1.0e4, 0.3), [4, 2, 1], &|p| [2.0 * p[0] - 1.0, p[1] - 1.0, 0.0]).unwrap();
    // A circle of radius 0.5 centred just above the top edge (y = 0), pushed in by its free translation.
    let circle = RigidMaster::new(RigidShape::Circle { c: [0.05, 0.45], r: 0.5 }).with_free_translation([true, true, false], [0.0, -1.0, 0.0]);
    let spec = ContactSpec::rigid("pin", mesh.surfaces["v1"].clone(), circle, 1.0e5);
    let u = wiggle(mesh.n_dofs(), 1e-4);
    check("rigid circle", &mesh, vec![spec], &u, &[0.01, -0.03], false);
}
