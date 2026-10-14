//! Level 5 of the derivative suite, nonlinear: direct and discrete-adjoint sensitivities of a path-dependent elastic-plastic solution
//! (several load steps, history-dependent) against central differences of the full solve, against each other, and against the
//! single-step adjoint of the adaptivity code.

use fea_core::derivative::constitutive::STATE;
use fea_core::derivative::path::*;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::material::J2;
use fea_core::mesh2d::mesh_region;
use fea_core::*;
use mechanics_core::hardening::Hardening;

const SIGMA: f64 = 21_000.0;

#[derive(Clone, Copy)]
struct Knobs {
    e_scale: f64,
    nu_shift: f64,
    yield_scale: f64,
    hard_scale: f64,
    load_scale: f64,
}

const NOMINAL: Knobs = Knobs { e_scale: 1.0, nu_shift: 0.0, yield_scale: 1.0, hard_scale: 1.0, load_scale: 1.0 };

fn law(k: &Knobs) -> Hardening {
    // nominal: sigma_y 40000, tangent 2e5 up to ep 5; the knobs are the multipliers the sensitivity parameters refer to
    let (s0, rise) = (40_000.0, 2.0e5 * 5.0);
    Hardening::table(&[(0.0, k.yield_scale * s0), (0.002, k.yield_scale * (s0 + k.hard_scale * 0.002 * 2.0e5)), (5.0, k.yield_scale * (s0 + k.hard_scale * rise))]).unwrap()
}

fn build(kind: ElementKind, physics: Physics, k: &Knobs) -> (Model, Loads, Dirichlet) {
    let region = Region::new(Loop::rectangle(0.0, 0.0, 6.0, 2.0).unwrap(), vec![Loop::circle([3.0, 1.0], 0.45, "hole").unwrap()], Elastic::new(10.0e6, 0.3)).unwrap();
    let size = |p: [f64; 2]| (0.2 + 0.3 * ((p[0] - 3.0).hypot(p[1] - 1.0) - 0.45).max(0.0)).min(0.55);
    let mut mesh = mesh_region(&region, physics, kind, &size, MeshOptions::default()).unwrap();
    mesh.blocks[0].material = Elastic::new(10.0e6 * k.e_scale, 0.3 + k.nu_shift);
    mesh.set_plasticity(0, J2::small(law(k))).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 1.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 1.0).abs())).unwrap();
    bc.fix(anchor, 1, 0.0);
    let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([SIGMA, 0.0, 0.0]))).collect(), ..Loads::default() };
    (model, loads, bc)
}

fn qois(model: &Model) -> Vec<Qoi> {
    let n = model.mesh.n_dofs();
    // mean x displacement of the loaded edge
    let right = model.mesh.node_set("right").unwrap();
    let mut g1 = vec![0.0; n];
    for &nd in right {
        g1[nd * 2] = 1.0 / right.len() as f64;
    }
    // y displacement of the top-left node region (a transverse quantity that depends on Poisson's ratio)
    let top = *model.mesh.node_set("top").unwrap().iter().max_by(|&&a, &&b| model.mesh.nodes[a][0].total_cmp(&model.mesh.nodes[b][0])).unwrap();
    let mut g2 = vec![0.0; n];
    g2[top * 2 + 1] = 1.0;
    // the area integral of the equivalent plastic strain: a function of the final state only
    let mut weights: Vec<Vec<[f64; STATE]>> = Vec::new();
    let mut work = kernel::Work::new();
    for blk in &model.mesh.blocks {
        let t = blk.kind.table();
        let mut w = Vec::new();
        for e in 0..blk.n_elems() {
            let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| model.mesh.nodes[n]).collect();
            kernel::geometry(blk.kind, model.mesh.physics, &xyz, &mut work).unwrap();
            for g in 0..t.ngp {
                let mut c = [0.0; STATE];
                c[6] = work.wdet[g];
                w.push(c);
            }
        }
        weights.push(w);
    }
    vec![Qoi::displacement(g1), Qoi::displacement(g2), Qoi { grad_u: vec![0.0; n], state_weights: Some(weights) }]
}

fn response(kind: ElementKind, physics: Physics, k: &Knobs, opt: &PathOptions) -> Vec<f64> {
    let (model, loads, bc) = build(kind, physics, k);
    // the load scale multiplies the whole load: scale the traction instead (the model is rebuilt)
    let loads = Loads { faces: loads.faces.iter().map(|(f, l)| (f.clone(), if let SurfaceLoad::Traction(t) = l { SurfaceLoad::Traction([t[0] * k.load_scale, t[1], t[2]]) } else { *l })).collect(), ..Loads::default() };
    let sol = solve_path(&model, &loads, &bc, opt).unwrap();
    let u = sol.u.last().unwrap();
    let q = qois(&model);
    q.iter()
        .map(|qq| {
            let mut v: f64 = qq.grad_u.iter().zip(u).map(|(a, b)| a * b).sum();
            if let Some(w) = &qq.state_weights {
                let st = sol.states.last().unwrap();
                for (b, wb) in w.iter().enumerate() {
                    for (i, c) in wb.iter().enumerate() {
                        v += c.iter().zip(&st.gp[b][i]).map(|(a, bb)| a * bb).sum::<f64>();
                    }
                }
            }
            v
        })
        .collect()
}

fn perturb(base: &Knobs, which: usize, h: f64) -> Knobs {
    let mut k = *base;
    match which {
        0 => k.e_scale += h,
        1 => k.nu_shift += h * 0.3, // d/d(nu) in absolute units: the parameter is nu itself
        2 => k.yield_scale += h,
        3 => k.hard_scale += h,
        _ => k.load_scale += h,
    }
    k
}

fn run(kind: ElementKind, physics: Physics, steps: usize) {
    let opt = PathOptions { steps, ..PathOptions::default() };
    let (model, loads, bc) = build(kind, physics, &NOMINAL);
    let sol = solve_path(&model, &loads, &bc, &opt).unwrap();
    let yielded = sol.states.last().unwrap().plastic_fraction();
    assert!(yielded > 0.01, "the problem must yield ({yielded})");
    let params = [NlParam::ModulusScale(0), NlParam::PoissonRatio(0), NlParam::YieldScale(0), NlParam::HardeningScale(0), NlParam::LoadScale];
    let q = qois(&model);
    let engine = PathSensitivityEngine::new(&model, &loads, &bc).unwrap();
    let (direct, rd) = engine.direct(&sol, &params, &q).unwrap();
    let (adjoint, ra) = engine.adjoint(&sol, &params, &q, JacobianStrategy::Recompute).unwrap();
    let (stored, rs) = engine.adjoint(&sol, &params, &q, JacobianStrategy::Store).unwrap();
    // direct and adjoint agree to rounding (they are the same linearisation propagated in opposite directions)
    for qi in 0..q.len() {
        for pi in 0..params.len() {
            let (d, a, s) = (direct[qi][pi], adjoint[qi][pi], stored[qi][pi]);
            let scale = d.abs().max(a.abs()).max(1e-12);
            assert!((d - a).abs() < 1e-8 * scale + 1e-14, "{kind:?} {physics:?} q{qi} p{pi}: direct {d:e} vs adjoint {a:e}");
            assert!((a - s).abs() <= 1e-12 * scale, "store and recompute are the same arithmetic: {a} vs {s}");
        }
    }
    // against central differences of the full nonlinear solve
    let mut worst = 0.0f64;
    for pi in 0..params.len() {
        let h = 1e-5;
        let (fp, fm) = (response(kind, physics, &perturb(&NOMINAL, pi, h), &opt), response(kind, physics, &perturb(&NOMINAL, pi, -h), &opt));
        for qi in 0..q.len() {
            // PoissonRatio is a derivative per unit nu: the perturbation moved nu by 0.3 h
            let step = if pi == 1 { 0.3 * h } else { h };
            let fd = (fp[qi] - fm[qi]) / (2.0 * step);
            let ad = adjoint[qi][pi];
            let scale = fd.abs().max(ad.abs()).max(1e-12);
            worst = worst.max((fd - ad).abs() / scale);
            assert!((fd - ad).abs() < 2e-5 * scale + 1e-12, "{kind:?} {physics:?} q{qi} param {pi}: adjoint {ad:e} vs finite difference {fd:e}");
        }
    }
    eprintln!("{kind:?} {physics:?} {steps} steps: {} dofs, worst relative gap to finite differences {worst:.2e}; direct {} solves / {} factorisations, adjoint {} solves / {} factorisations; jacobian bytes recompute {} store {}", model.mesh.n_dofs(), rd.tangent_solves, rd.tangent_factorizations, ra.tangent_solves, ra.tangent_factorizations, ra.jacobian_bytes, rs.jacobian_bytes);
    assert!(rd.tangent_solves == steps * params.len() && ra.tangent_solves == steps * q.len(), "one solve per parameter and step (direct), per quantity and step (adjoint)");
    assert!(rs.jacobian_bytes > ra.jacobian_bytes, "storing every step keeps more than recomputing");
}

#[test]
fn history_dependent_sensitivities_match_finite_differences_in_plane_strain() {
    run(ElementKind::Tri6, Physics::PlaneStrain { thickness: 1.0 }, 4);
}

#[test]
fn history_dependent_sensitivities_match_finite_differences_in_plane_stress_on_quadrilaterals() {
    run(ElementKind::Quad8, Physics::PlaneStress { thickness: 1.0 }, 3);
}

#[test]
fn for_one_step_the_path_adjoint_is_the_tangent_adjoint_the_adaptivity_code_uses() {
    // one load step has no history: the discrete path adjoint must reduce to the single-tangent adjoint of the goal-oriented code,
    // which is the same linear algebra (K_T^T z = g) the sensitivity engine shares with it
    use fea_core::adaptivity::{tangent_adjoint, LinearFunctional};
    let opt = PathOptions { steps: 1, ..PathOptions::default() };
    let (model, loads, bc) = build(ElementKind::Tri6, Physics::PlaneStrain { thickness: 1.0 }, &NOMINAL);
    let sol = solve_path(&model, &loads, &bc, &opt).unwrap();
    let q = qois(&model);
    let engine = PathSensitivityEngine::new(&model, &loads, &bc).unwrap();
    let (exact, _) = engine.adjoint(&sol, &[NlParam::LoadScale], &q[..1], JacobianStrategy::Recompute).unwrap();
    let one = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 1, tol: 1e-12, ..NlOptions::default() }).unwrap();
    let g = LinearFunctional::mean_displacement(&model, model.mesh.node_set("right").unwrap(), 0).unwrap();
    let adj = tangent_adjoint(&model, &bc, &one, &g).unwrap();
    let f = loads::assemble(&model.mesh, &loads).unwrap();
    let single: f64 = adj.z.iter().zip(&f).map(|(a, b)| a * b).sum();
    assert!((exact[0][0] - single).abs() < 1e-8 * single.abs(), "path adjoint {:e} vs tangent adjoint {:e}", exact[0][0], single);
}
