//! Adaptivity support for small-strain plasticity: the tangent adjoint against a finite difference of the nonlinear solve, and the
//! transfer of the plastic history to a new mesh.

use fea_core::adaptivity::*;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::loads;
use fea_core::material::J2;
use fea_core::mesh2d::mesh_region;
use fea_core::nonlinear::Stop;
use fea_core::*;
use mechanics_core::hardening::Hardening;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;
const SIGMA: f64 = 21_000.0; // nominal stress: the hole edge (Kt ~ 3) yields at 40 ksi-equivalent 40000

fn law() -> Hardening {
    Hardening::linear(40_000.0, 2.0e5, 5.0).unwrap()
}

fn plate_with_hole(h: f64) -> (Model, Dirichlet, Loads) {
    let region = Region::new(Loop::rectangle(0.0, 0.0, 6.0, 2.0).unwrap(), vec![Loop::circle([3.0, 1.0], 0.4, "hole").unwrap()], Elastic::new(E, NU)).unwrap();
    let size = move |p: [f64; 2]| (h * 0.35 + 0.3 * ((p[0] - 3.0).hypot(p[1] - 1.0) - 0.4).max(0.0)).min(h);
    let mut mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, &size, MeshOptions::default()).unwrap();
    mesh.set_plasticity(0, J2::small(law())).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 1.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 1.0).abs())).unwrap();
    bc.fix(anchor, 1, 0.0);
    let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([SIGMA, 0.0, 0.0]))).collect(), ..Loads::default() };
    (model, bc, loads)
}

fn scaled(loads: &Loads, s: f64) -> Loads {
    Loads { faces: loads.faces.iter().map(|(f, l)| (f.clone(), if let SurfaceLoad::Traction(t) = l { SurfaceLoad::Traction([t[0] * s, t[1] * s, t[2] * s]) } else { *l })).collect(), ..Loads::default() }
}

fn goal(model: &Model) -> LinearFunctional {
    LinearFunctional::mean_displacement(model, model.mesh.node_set("right").unwrap(), 0).unwrap()
}

#[test]
fn the_tangent_adjoint_gives_the_load_sensitivity_of_a_plastic_solution() {
    let (model, bc, loads) = plate_with_hole(0.5);
    let opt = NlOptions { steps: 4, tol: 1e-12, ..NlOptions::default() };
    let sol = model.solve_nonlinear(&loads, &bc, &opt).unwrap();
    assert_eq!(sol.stop, Stop::Completed);
    assert!(sol.state.plastic_fraction() > 0.0, "the problem must yield");
    assert!(sol.prev_state.is_some());
    let g = goal(&model);
    let adj = tangent_adjoint(&model, &bc, &sol, &g).unwrap();
    assert!(adj.residual < 1e-10, "{:e}", adj.residual);
    // dJ/ds = z . f_ext
    let f = loads::assemble(&model.mesh, &loads).unwrap();
    let analytic: f64 = adj.z.iter().zip(&f).map(|(a, b)| a * b).sum();
    let delta = 1e-4;
    let j = |s: f64| {
        let sol = model.solve_nonlinear(&scaled(&loads, s), &bc, &opt).unwrap();
        assert_eq!(sol.stop, Stop::Completed);
        g.value(&sol.u)
    };
    let fd = (j(1.0 + delta) - j(1.0 - delta)) / (2.0 * delta);
    eprintln!("dJ/ds: adjoint {analytic:.6e}, finite difference {fd:.6e} (relative difference {:.2e}); plastic fraction {:.2}", (analytic - fd).abs() / fd.abs(), sol.state.plastic_fraction());
    assert!((analytic - fd).abs() < 2e-4 * fd.abs(), "adjoint {analytic} vs finite difference {fd}");
}

#[test]
fn the_nonlinear_stress_read_out_matches_the_internal_force() {
    // virtual work: for the converged state, sum over Gauss points of w sigma : grad(v) equals v . f_int for any v; with f_int =
    // f_ext at the free dofs this reproduces the applied load resultant
    let (model, bc, loads) = plate_with_hole(0.5);
    let sol = model.solve_nonlinear(&loads, &bc, &NlOptions { steps: 4, tol: 1e-12, ..NlOptions::default() }).unwrap();
    let virgin = NlState::new(&model.mesh);
    let old = sol.prev_state.as_ref().unwrap_or(&virgin);
    let gauss = model.nl_gauss_stresses(&sol.u, old).unwrap();
    // the section-force check: integrate sigma_xx over a vertical cut is the load on the plate. Use the cut at x = 1.5 (away from the hole).
    // Approximate the line integral by the area integral over the strip 1.4 < x < 1.6 divided by its width.
    let mut work = fea_core::kernel::Work::new();
    let (mut force, mut width_area) = (0.0, 0.0);
    for (bi, blk) in model.mesh.blocks.iter().enumerate() {
        let t = blk.kind.table();
        for e in 0..blk.n_elems() {
            let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| model.mesh.nodes[n]).collect();
            fea_core::kernel::geometry(blk.kind, model.mesh.physics, &xyz, &mut work).unwrap();
            for g in 0..t.ngp {
                let (x, s) = gauss[bi][e * t.ngp + g];
                if x[0] > 1.0 && x[0] < 2.0 {
                    force += work.wdet[g] * s[0];
                    width_area += work.wdet[g];
                }
            }
        }
    }
    let _ = width_area;
    let resultant = force / 1.0; // strip of width 1: mean section force
    let applied = SIGMA * 2.0; // stress times the plate height (thickness 1)
    eprintln!("section force {resultant:.1} vs applied {applied:.1}");
    assert!((resultant - applied).abs() < 0.02 * applied, "{resultant} vs {applied}");
}

#[test]
fn the_plastic_history_transfer_is_approximate_but_re_equilibrates_and_a_displacement_warm_start_serves_proportional_loading() {
    let (coarse, bc_c, loads_c) = plate_with_hole(0.6);
    let (fine, bc_f, loads_f) = plate_with_hole(0.3);
    let opt = NlOptions { steps: 4, tol: 1e-10, ..NlOptions::default() };
    let sc = coarse.solve_nonlinear(&loads_c, &bc_c, &opt).unwrap();
    let sf = fine.solve_nonlinear(&loads_f, &bc_f, &opt).unwrap();
    assert_eq!((sc.stop.clone(), sf.stop.clone()), (Stop::Completed, Stop::Completed));
    let scratch_iterations: usize = sf.steps.iter().map(|s| s.iterations).sum();
    let g = goal(&fine);
    let j_scratch = g.value(&sf.u);

    // --- the history transfer: admissible by construction, approximate in value, re-equilibrated by the driver
    let t = transfer_history(&coarse, &sc.u, &sc.state, &fine, 4).unwrap();
    assert_eq!(t.unlocated_nodes, 0);
    assert!(t.state.gp[0].iter().all(|s| s[6] >= 0.0), "equivalent plastic strain must stay non-negative");
    assert!(t.max_ep.1 <= t.max_ep.0 * (1.0 + 1e-12), "a convex combination cannot exceed the old maximum: {:?}", t.max_ep);
    let f = loads::assemble(&fine.mesh, &loads_f).unwrap();
    let resid = |u: &[f64], st: &NlState| -> f64 {
        let mut new = NlState::new(&fine.mesh);
        let (fi, _) = fea_core::nonlinear::assemble_nl_public(&fine.mesh, &fine.pattern, u, st, &mut new, false).unwrap();
        (0..f.len()).filter(|&i| !bc_f.fixed[i]).map(|i| (fi[i] - f[i]).powi(2)).sum::<f64>().sqrt()
    };
    let transfer_error = resid(&sf.u, &t.state);
    let virgin_error = resid(&sf.u, &NlState::new(&fine.mesh));
    eprintln!("residual at the exact fine displacements: transferred history {transfer_error:.3e}, virgin history {virgin_error:.3e}, load norm {:.3e}", f.iter().map(|v| v * v).sum::<f64>().sqrt());
    let r = reequilibrate(&fine, &loads_f, &bc_f, &t, 1.0, &opt).unwrap();
    eprintln!("re-equilibration: {} iterations (from scratch {scratch_iterations}); J {:.6e} against {j_scratch:.6e}", r.iterations, g.value(&r.solution.u));
    assert_eq!(r.solution.stop, Stop::Completed);
    assert!((g.value(&r.solution.u) - j_scratch).abs() < 5e-3 * j_scratch.abs(), "{} vs {j_scratch}", g.value(&r.solution.u));

    // --- for monotone proportional loading the plastic state is a function of the final strain: carry the displacements only
    let warm = warm_start_displacement(&coarse, &sc.u, &fine, &loads_f, &bc_f, &opt).unwrap();
    let warm_iterations: usize = warm.steps.iter().map(|s| s.iterations).sum();
    eprintln!("displacement warm start: {warm_iterations} iterations in one step; J {:.6e}", g.value(&warm.u));
    assert_eq!(warm.stop, Stop::Completed);
    assert!(warm_iterations < scratch_iterations, "{warm_iterations} against {scratch_iterations}");
    assert!((g.value(&warm.u) - j_scratch).abs() < 5e-3 * j_scratch.abs());
    // contact states are refused rather than silently dropped
    let mut with_contact = sc.state.clone();
    with_contact.contact = vec![Vec::new()];
    assert!(matches!(transfer_history(&coarse, &sc.u, &with_contact, &fine, 4), Err(fea_core::meshing::MeshError::Unsupported(_))));
}

mod hertz {
    use super::*;
    use fea_core::contact::{ContactRule, ContactSpec, RigidMaster, RigidShape};
    use std::f64::consts::PI;

    pub const RAD: f64 = 1.0;
    const TRAVEL: f64 = 2.0e-3;

    pub fn problem(size: &dyn Fn([f64; 2]) -> f64) -> Result<NlProblem, String> {
        let region = Region::new(Loop::rectangle(0.0, -2.0, 2.0, 0.0).unwrap(), vec![], Elastic::new(E, NU))?;
        let mesh = mesh_region(&region, Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6, size, MeshOptions::default())?;
        let model = Model::new(mesh)?;
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("left").unwrap() {
            bc.fix(n, 0, 0.0);
        }
        for &n in model.mesh.node_set("bottom").unwrap() {
            bc.fix_node(n);
        }
        let slave = model.mesh.surfaces["top"].clone();
        let master = RigidMaster::new(RigidShape::Circle { c: [0.0, RAD], r: RAD }).with_travel([0.0, -TRAVEL, 0.0]);
        let spec = ContactSpec::rigid("hertz", slave, master, 1.0e9).with_rule(ContactRule::Nodal);
        Ok(NlProblem { model, loads: Loads::default(), bc, contacts: vec![spec], labels: None, quad_scale: 1.0 })
    }

    /// FE half-width and peak pressure against the Hertz formulas for the FE force.
    pub fn against_hertz(p: &NlProblem, sol: &NlSolution) -> (f64, f64, f64, f64) {
        let tr = sol.contact_tractions(0, 0.0);
        let force: f64 = 2.0 * tr.iter().map(|t| t.pressure * t.weight).sum::<f64>(); // the mirrored full contact
        let estar = E / (1.0 - NU * NU);
        let a_h = (4.0 * force * RAD / (PI * estar)).sqrt();
        let p0_h = 2.0 * force / (PI * a_h);
        let a_fe = tr.iter().filter(|t| t.active && t.pressure > 0.0).map(|t| t.x[0].abs()).fold(0.0f64, f64::max);
        let p0_fe = tr.iter().map(|t| t.pressure).fold(0.0f64, f64::max);
        let _ = p;
        (a_fe, a_h, p0_fe, p0_h)
    }
}

#[test]
fn contact_protection_resolves_the_hertz_patch_from_a_mesh_that_cannot_see_it() {
    let mut mesher = |s: &AdaptSize| hertz::problem(&|p| s.at([p[0], p[1], 0.0]));
    let goals = [NlGoal::contact_force(0, 0.03), NlGoal::peak_pressure(0, 0.05)];
    let cfg = NlAmrConfig { nl: NlOptions { steps: 4, ..NlOptions::default() }, max_cycles: 10, refine_ratio: 0.4, contact_cells_across: 5.0, adapt: fea_core::adapt::AdaptOptions { grading: 0.5, ..NlAmrConfig::default().adapt }, ..NlAmrConfig::default() };
    let first = hertz::problem(&|_| 0.3).unwrap();
    let r = run_amr_nonlinear(first, &goals, &mut mesher, &cfg).unwrap();
    for c in &r.history {
        eprintln!("cycle {} dofs {:>6}: active {:>3}, half-width {:.4}, force {:.2}, peak {:.0}, change {:?}, protected {:>4}, marked {:>4}, {} newton iterations in {} steps ({} factorisations)", c.cycle, c.dofs, c.contact_active, c.contact_halfwidth, c.goals[0].1, c.goals[1].1, c.goal_change.iter().map(|x| (x * 1000.0).round() / 1000.0).collect::<Vec<_>>(), c.contact_protected, c.marked, c.newton_iterations, c.load_steps, c.factorisations);
    }
    eprintln!("stop {:?}", r.stop);
    assert!(r.history.iter().all(|c| c.contact_balance < 1e-9), "contact traction balance: {:?}", r.history.iter().map(|c| c.contact_balance).collect::<Vec<_>>());
    let (a_fe, a_h, p0_fe, p0_h) = hertz::against_hertz(&r.problem, &r.solution);
    eprintln!("Hertz for the FE force: half-width {a_fe:.5} vs {a_h:.5} ({:+.1} %), peak pressure {p0_fe:.1} vs {p0_h:.1} ({:+.1} %)", 100.0 * (a_fe / a_h - 1.0), 100.0 * (p0_fe / p0_h - 1.0));
    assert!(matches!(r.stop, NlStop::Converged), "{:?}", r.stop);
    assert!((a_fe / a_h - 1.0).abs() < 0.12 && (p0_fe / p0_h - 1.0).abs() < 0.08, "a {a_fe} vs {a_h}, p0 {p0_fe} vs {p0_h}");
}

#[test]
fn the_plastic_zone_is_refined_and_the_displacement_warm_start_saves_iterations() {
    fn problem(size: &dyn Fn([f64; 2]) -> f64) -> Result<NlProblem, String> {
        let region = Region::new(Loop::rectangle(0.0, 0.0, 6.0, 2.0).unwrap(), vec![Loop::circle([3.0, 1.0], 0.4, "hole").unwrap()], Elastic::new(E, NU))?;
        let mut mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Tri6, size, MeshOptions::default())?;
        mesh.set_plasticity(0, J2::small(law()))?;
        let model = Model::new(mesh)?;
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("left").unwrap() {
            bc.fix(n, 0, 0.0);
        }
        let anchor = *model.mesh.node_set("left").unwrap().iter().min_by(|&&a, &&b| (model.mesh.nodes[a][1] - 1.0).abs().total_cmp(&(model.mesh.nodes[b][1] - 1.0).abs())).unwrap();
        bc.fix(anchor, 1, 0.0);
        let loads = Loads { faces: model.mesh.surfaces["right"].iter().map(|f| (f.clone(), SurfaceLoad::Traction([SIGMA, 0.0, 0.0]))).collect(), ..Loads::default() };
        Ok(NlProblem { model, loads, bc, contacts: Vec::new(), labels: None, quad_scale: 1.0 })
    }
    let goals = [NlGoal::mean_displacement("end displacement", "right", 0, 2e-3), NlGoal::max_plastic_strain(2e-2)];
    let run = |warm: bool| {
        let mut mesher = |s: &AdaptSize| problem(&|p| s.at([p[0], p[1], 0.0]));
        let cfg = NlAmrConfig { nl: NlOptions { steps: 4, tol: 1e-9, ..NlOptions::default() }, max_cycles: 6, warm_start: warm, theta: 0.6, ..NlAmrConfig::default() };
        run_amr_nonlinear(problem(&|_| 1.0).unwrap(), &goals, &mut mesher, &cfg).unwrap()
    };
    let (cold, warm) = (run(false), run(true));
    for (label, r) in [("cold", &cold), ("warm", &warm)] {
        for c in &r.history {
            eprintln!("{label} cycle {} dofs {:>6}: u {:.5e} max ep {:.3e} (plastic {:.1}%) change {:?} newton {:>3} warm {}", c.cycle, c.dofs, c.goals[0].1, c.max_ep, 100.0 * c.plastic_fraction, c.goal_change.iter().map(|x| (x * 1e4).round() / 1e4).collect::<Vec<_>>(), c.newton_iterations, c.warm_started);
        }
        eprintln!("{label} stop {:?}", r.stop);
        assert!(matches!(r.stop, NlStop::Converged | NlStop::MaxCycles), "{label}: {:?}", r.stop);
    }
    let last = |r: &NlResult| r.history.last().unwrap().clone();
    let (c, w) = (last(&cold), last(&warm));
    // both runs describe the same physics
    assert!((c.goals[0].1 - w.goals[0].1).abs() < 5e-3 * c.goals[0].1.abs(), "{} vs {}", c.goals[0].1, w.goals[0].1);
    assert!(c.max_ep > 0.0 && c.plastic_fraction > 0.0);
    // the warm start is used on the new meshes and costs fewer Newton iterations than the load stepping
    assert!(warm.history.iter().skip(1).all(|h| h.warm_started), "every adapted mesh should have warm started");
    let sum = |r: &NlResult| -> usize { r.history.iter().skip(1).map(|h| h.newton_iterations).sum() };
    eprintln!("Newton iterations on the adapted meshes: cold {} warm {}", sum(&cold), sum(&warm));
    assert!(sum(&warm) < sum(&cold));
}
