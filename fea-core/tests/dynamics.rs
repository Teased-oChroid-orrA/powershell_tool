//! Natural frequencies, buckling and transient response against closed forms (unit-free: E = rho = 1 unless noted).

use fea_core::generate::grid;
use fea_core::loads::SurfaceLoad;
use fea_core::*;
use std::f64::consts::PI;

fn beam2d(kind: ElementKind, div: [usize; 2], l: f64, h: f64, rho: f64) -> Model {
    let mut mesh = grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(1.0, 0.0), [div[0], div[1], 1], &move |p| [l * p[0], h * p[1], 0.0]).unwrap();
    mesh.set_density_all(rho).unwrap();
    Model::new(mesh).unwrap()
}

fn fix_x0(model: &Model, bc: &mut Dirichlet) {
    for n in 0..model.mesh.nodes.len() {
        if model.mesh.nodes[n][0] < 1e-12 {
            bc.fix_node(n);
        }
    }
}

#[test]
fn axial_bar_frequencies_match_the_closed_form() {
    // Fixed-free bar, only axial motion allowed: omega_n = (2n - 1) pi / (2 L) sqrt(E / rho).
    let model = beam2d(ElementKind::Quad9, [12, 1], 1.0, 0.1, 1.0);
    let mut bc = model.dirichlet();
    for n in 0..model.mesh.nodes.len() {
        bc.fix(n, 1, 0.0);
        if model.mesh.nodes[n][0] < 1e-12 {
            bc.fix(n, 0, 0.0);
        }
    }
    let modal = model.modal(&bc, &ModalOptions { n_modes: 4, ..ModalOptions::default() }).unwrap();
    for (i, m) in modal.modes.iter().enumerate() {
        let exact = (2.0 * i as f64 + 1.0) * PI / 2.0;
        // (higher modes are the discretisation's: 12 elements resolve mode 4 to ~5e-4)
        let tol = if i == 0 { 2e-6 } else { 1e-3 };
        assert!((m.omega2.sqrt() / exact - 1.0).abs() < tol, "mode {i}: {} vs {exact}", m.omega2.sqrt());
        assert!(m.residual < 1e-9);
    }
    // The modes are mass-orthonormal and carry the mass: the first axial mode of a bar has 8/pi^2 = 81 % effective mass.
    assert!((modal.modes[0].effective_mass[0] / modal.total_mass - 8.0 / (PI * PI)).abs() < 1e-3);
    assert!(modal.modes.iter().map(|m| m.effective_mass[0]).sum::<f64>() <= modal.total_mass * (1.0 + 1e-9));
}

#[test]
fn total_mass_is_density_times_volume() {
    let model = beam2d(ElementKind::Quad8, [5, 2], 2.0, 0.3, 3.0);
    assert!((model.total_mass().unwrap() - 3.0 * 2.0 * 0.3 * 1.0).abs() < 1e-12);
    let mut mesh = grid(Physics::Solid, ElementKind::Hex20, Elastic::new(1.0, 0.3), [2, 2, 2], &|p| [2.0 * p[0], 3.0 * p[1], 0.5 * p[2]]).unwrap();
    mesh.set_density_all(7.0).unwrap();
    let m3 = Model::new(mesh).unwrap();
    assert!((m3.total_mass().unwrap() - 7.0 * 3.0).abs() < 1e-10);
}

#[test]
fn lumped_mass_brackets_the_consistent_frequency_from_below() {
    let model = beam2d(ElementKind::Quad4, [10, 1], 1.0, 0.1, 1.0);
    let mut bc = model.dirichlet();
    for n in 0..model.mesh.nodes.len() {
        bc.fix(n, 1, 0.0);
        if model.mesh.nodes[n][0] < 1e-12 {
            bc.fix(n, 0, 0.0);
        }
    }
    let exact = PI / 2.0;
    let w = |mass| model.modal(&bc, &ModalOptions { n_modes: 1, mass, ..ModalOptions::default() }).unwrap().modes[0].omega2.sqrt();
    let (wc, wl) = (w(MassKind::Consistent), w(MassKind::Lumped));
    assert!(wl < exact && exact < wc, "lumped {wl} < exact {exact} < consistent {wc}");
    assert!((wc - exact) / exact < 1e-2 && (exact - wl) / exact < 1e-2);
}

#[test]
fn cantilever_beam_bending_frequency_2d() {
    // Euler-Bernoulli: omega_1 = 1.875104^2 sqrt(E I / (rho A L^4)); shear deformation lowers it by ~(h / L)^2.
    let (l, h) = (1.0, 0.05);
    let model = beam2d(ElementKind::Quad9, [40, 2], l, h, 1.0);
    let mut bc = model.dirichlet();
    fix_x0(&model, &mut bc);
    let modal = model.modal(&bc, &ModalOptions { n_modes: 3, ..ModalOptions::default() }).unwrap();
    let exact = |c: f64| c * c * (h * h / 12.0).sqrt() / (l * l);
    assert!((modal.modes[0].omega2.sqrt() / exact(1.875_104_07) - 1.0).abs() < 4e-3, "{} vs {}", modal.modes[0].omega2.sqrt(), exact(1.875_104_07));
    assert!((modal.modes[1].omega2.sqrt() / exact(4.694_091_13) - 1.0).abs() < 1.2e-2);
}

#[test]
fn cantilever_beam_bending_frequency_3d_has_a_degenerate_pair() {
    let (l, w) = (10.0, 1.0);
    let mut mesh = grid(Physics::Solid, ElementKind::Hex20, Elastic::new(1.0, 0.0), [20, 2, 2], &move |p| [l * p[0], w * p[1], w * p[2]]).unwrap();
    mesh.set_density_all(1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    fix_x0(&model, &mut bc);
    let modal = model.modal(&bc, &ModalOptions { n_modes: 2, ..ModalOptions::default() }).unwrap();
    let exact = 1.875_104_07f64.powi(2) * (w * w / 12.0).sqrt() / (l * l);
    for m in &modal.modes {
        assert!((m.omega2.sqrt() / exact - 1.0).abs() < 1e-2, "{} vs {exact}", m.omega2.sqrt());
    }
    assert!((modal.modes[0].omega2 / modal.modes[1].omega2 - 1.0).abs() < 1e-6, "the square section's two bending planes are degenerate");
}

#[test]
fn a_free_structure_needs_a_negative_shift_and_then_finds_its_rigid_modes() {
    // (slender, so that shear deformation and rotary inertia, ~ (h / L)^2 times a mode constant, stay under 1 %)
    let (l, h) = (1.0, 0.02);
    let model = beam2d(ElementKind::Quad9, [40, 2], l, h, 1.0);
    let bc = model.dirichlet();
    let err = model.modal(&bc, &ModalOptions::default()).unwrap_err();
    assert!(err.contains("rigid") || err.contains("unsupported") || err.contains("positive definite"), "{err}");
    let modal = model.modal(&bc, &ModalOptions { n_modes: 5, shift: -1.0, ..ModalOptions::default() }).unwrap();
    // Three rigid-body modes (two translations, one rotation) at zero, then the free-free bending frequency 22.3733 sqrt(EI/rho A)/L^2.
    assert!(modal.modes.iter().filter(|m| m.omega2.abs() < 1e-8).count() == 3, "{:?}", modal.modes.iter().map(|m| m.omega2).collect::<Vec<_>>());
    let bending = modal.modes.iter().find(|m| m.omega2 > 1e-3).unwrap().omega2.sqrt();
    let exact = 22.3733 * (h * h / 12.0).sqrt() / (l * l);
    assert!((bending / exact - 1.0).abs() < 1e-2, "{bending} vs {exact}");
}

#[test]
fn modal_refuses_a_massless_mesh_and_prescribed_displacements() {
    let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0, 0.3), [2, 2, 1], &|p| p).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    fix_x0(&model, &mut bc);
    assert!(model.modal(&bc, &ModalOptions::default()).unwrap_err().contains("density"));
    let mut mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad4, Elastic::new(1.0, 0.3), [2, 2, 1], &|p| p).unwrap();
    mesh.set_density_all(1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    fix_x0(&model, &mut bc);
    bc.fix(2, 0, 0.1);
    assert!(model.modal(&bc, &ModalOptions::default()).unwrap_err().contains("homogeneous"));
}

fn column_load(model: &Model, p_total: f64, h: f64) -> Loads {
    // Compression on the free end face x = l (thickness 1).
    let faces = model.mesh.surfaces["u1"].clone();
    Loads { faces: faces.into_iter().map(|f| (f, SurfaceLoad::Traction([-p_total / h, 0.0, 0.0]))).collect(), ..Loads::default() }
}

#[test]
fn euler_buckling_of_a_cantilever_column_2d() {
    // Pcr = pi^2 E I / (4 L^2), I = h^3 / 12 (plane stress, thickness 1); shear and Poisson effects are ~(h / L)^2.
    let (l, h) = (2.0, 0.1);
    let model = beam2d(ElementKind::Quad9, [40, 2], l, h, 1.0);
    let mut bc = model.dirichlet();
    fix_x0(&model, &mut bc);
    let loads = column_load(&model, 1.0, h);
    let b = model.buckling(&loads, &bc, &BucklingOptions { n_modes: 2, ..BucklingOptions::default() }).unwrap();
    fea_core::report::AcceptanceReport::buckling(&b, 1e-9).require().unwrap();
    let pcr = PI * PI * (h * h * h / 12.0) / (4.0 * l * l);
    assert!((b.modes[0].load_factor / pcr - 1.0).abs() < 1.5e-2, "{} vs {pcr}", b.modes[0].load_factor);
    // The second cantilever mode: (3 pi / 2)^2 / (pi / 2)^2 = 9 times the first.
    assert!((b.modes[1].load_factor / b.modes[0].load_factor / 9.0 - 1.0).abs() < 3e-2);
    assert!(b.modes.iter().all(|m| m.residual < 1e-9));
}

#[test]
fn euler_buckling_of_a_cantilever_column_3d() {
    let (l, w) = (10.0, 1.0);
    let mut mesh = grid(Physics::Solid, ElementKind::Hex20, Elastic::new(1.0, 0.0), [20, 2, 2], &move |p| [l * p[0], w * p[1], w * p[2]]).unwrap();
    mesh.set_density_all(1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    fix_x0(&model, &mut bc);
    let faces = model.mesh.surfaces["u1"].clone();
    let loads = Loads { faces: faces.into_iter().map(|f| (f, SurfaceLoad::Traction([-1.0 / (w * w), 0.0, 0.0]))).collect(), ..Loads::default() };
    let b = model.buckling(&loads, &bc, &BucklingOptions { n_modes: 2, ..BucklingOptions::default() }).unwrap();
    fea_core::report::AcceptanceReport::buckling(&b, 1e-9).require().unwrap();
    let pcr = PI * PI * (w.powi(4) / 12.0) / (4.0 * l * l);
    for m in &b.modes {
        assert!((m.load_factor / pcr - 1.0).abs() < 2e-2, "{} vs {pcr}", m.load_factor);
    }
}

#[test]
fn a_tensile_reference_load_has_no_positive_buckling_factor() {
    let (l, h) = (2.0, 0.1);
    let model = beam2d(ElementKind::Quad9, [10, 1], l, h, 1.0);
    let mut bc = model.dirichlet();
    fix_x0(&model, &mut bc);
    let loads = column_load(&model, -1.0, h);
    assert!(model.buckling(&loads, &bc, &BucklingOptions { n_modes: 1, ..BucklingOptions::default() }).is_err());
}

/// A small axial bar with every mode computed: the exact semi-discrete response by modal superposition is the oracle
/// of the time integrator.
fn bar_for_transient() -> (Model, Dirichlet, usize, Loads) {
    let model = beam2d(ElementKind::Quad4, [6, 1], 1.0, 0.2, 1.0);
    let mut bc = model.dirichlet();
    for n in 0..model.mesh.nodes.len() {
        bc.fix(n, 1, 0.0);
        if model.mesh.nodes[n][0] < 1e-12 {
            bc.fix(n, 0, 0.0);
        }
    }
    let tip = (0..model.mesh.nodes.len()).find(|&n| (model.mesh.nodes[n][0] - 1.0).abs() < 1e-12 && model.mesh.nodes[n][1] < 1e-12).unwrap();
    let loads = Loads { nodal: vec![(tip, [1.0, 0.0, 0.0])], ..Loads::default() };
    (model, bc, tip, loads)
}

fn exact_step_response(model: &Model, bc: &Dirichlet, tip: usize, t: f64) -> f64 {
    let n_free = model.mesh.n_dofs() - bc.n_fixed();
    let modal = model.modal(bc, &ModalOptions { n_modes: n_free, ..ModalOptions::default() }).unwrap();
    modal.modes.iter().map(|m| m.shape[tip * 2] * m.shape[tip * 2] * (1.0 - (m.omega2.sqrt() * t).cos()) / m.omega2).sum()
}

#[test]
fn newmark_follows_the_modal_superposition_at_second_order() {
    let (model, bc, tip, loads) = bar_for_transient();
    let t_end = 3.0;
    let exact = exact_step_response(&model, &bc, tip, t_end);
    let mut errs = Vec::new();
    for steps in [600usize, 1200, 2400] {
        let opt = TransientOptions { dt: t_end / steps as f64, steps, record: vec![tip * 2], ..TransientOptions::default() };
        let r = model.transient(&loads, &|_| 1.0, &bc, None, &opt).unwrap();
        errs.push((r.u[tip * 2] - exact).abs());
    }
    let (r1, r2) = (errs[0] / errs[1], errs[1] / errs[2]);
    assert!((3.5..4.5).contains(&r1) && (3.5..4.5).contains(&r2), "error ratios {r1}, {r2} (errors {errs:?})");
    assert!(errs[2] < 2e-3 * exact.abs());
}

#[test]
fn undamped_newmark_conserves_energy_and_hht_dissipates_it() {
    let (model, bc, tip, loads) = bar_for_transient();
    // Release from the static deflection under the tip load: free vibration, no load afterwards.
    let u0 = model.solve_static(&loads, &bc).unwrap().u;
    let v0 = vec![0.0; u0.len()];
    let zero = |_t: f64| 0.0;
    let run = |alpha: f64, rayleigh: (f64, f64)| {
        let opt = TransientOptions { dt: 0.05, steps: 400, alpha, rayleigh, record: vec![tip * 2], ..TransientOptions::default() };
        model.transient(&loads, &zero, &bc, Some((&u0, &v0)), &opt).unwrap()
    };
    let n = run(0.0, (0.0, 0.0));
    let e0 = n.energy[0];
    assert!(n.energy.iter().all(|e| (e / e0 - 1.0).abs() < 1e-10), "energy drift {:?}", n.energy.iter().map(|e| e / e0 - 1.0).fold(0.0f64, |m, v| m.max(v.abs())));
    let hht = run(-0.2, (0.0, 0.0));
    assert!(hht.energy.iter().all(|e| *e <= e0 * (1.0 + 1e-9)), "HHT must not gain energy beyond the initial");
    assert!(*hht.energy.last().unwrap() < 0.999 * e0, "HHT should dissipate");
    let damped = run(0.0, (0.05, 0.0));
    assert!(damped.energy.windows(2).all(|w| w[1] <= w[0] * (1.0 + 1e-12)) && *damped.energy.last().unwrap() < 0.5 * e0);
}

#[test]
fn lanczos_and_subspace_iteration_agree_including_multiple_eigenvalues() {
    // A square 3D column has every bending frequency twice (and buckles identically in both planes): a single-vector
    // Lanczos process can miss the second copy, which the deflated restarts must find.
    let (l, w) = (10.0, 1.0);
    let mut mesh = grid(Physics::Solid, ElementKind::Hex20, Elastic::new(1.0, 0.0), [10, 2, 2], &move |p| [l * p[0], w * p[1], w * p[2]]).unwrap();
    mesh.set_density_all(1.0).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    fix_x0(&model, &mut bc);
    let run = |method| model.modal(&bc, &ModalOptions { n_modes: 8, method, ..ModalOptions::default() }).unwrap();
    let (a, b) = (run(EigMethod::Subspace), run(EigMethod::Lanczos));
    for (x, y) in a.modes.iter().zip(&b.modes) {
        assert!((x.omega2 / y.omega2 - 1.0).abs() < 1e-8, "{} vs {}", x.omega2, y.omega2);
    }
    // Pairs: modes (0,1), (2,3), ... are degenerate bending pairs among the first 4.
    assert!((b.modes[0].omega2 / b.modes[1].omega2 - 1.0).abs() < 1e-6 && (b.modes[2].omega2 / b.modes[3].omega2 - 1.0).abs() < 1e-6);
    let faces = model.mesh.surfaces["u1"].clone();
    let loads = Loads { faces: faces.into_iter().map(|f| (f, SurfaceLoad::Traction([-1.0, 0.0, 0.0]))).collect(), ..Loads::default() };
    let bk = |method| model.buckling(&loads, &bc, &BucklingOptions { n_modes: 4, method, ..BucklingOptions::default() }).unwrap();
    let (a, b) = (bk(EigMethod::Subspace), bk(EigMethod::Lanczos));
    for (x, y) in a.modes.iter().zip(&b.modes) {
        assert!((x.load_factor / y.load_factor - 1.0).abs() < 1e-8, "{} vs {}", x.load_factor, y.load_factor);
    }
    assert!((b.modes[0].load_factor / b.modes[1].load_factor - 1.0).abs() < 1e-6, "buckling in both planes");
}

#[test]
fn dynamic_inputs_fail_before_indexing_or_factorization() {
    let (model, bc, _, loads) = bar_for_transient();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
        assert!(model.modal(&bc, &ModalOptions { tol: bad, ..Default::default() }).is_err());
        assert!(model.buckling(&loads, &bc, &BucklingOptions { tol: bad, ..Default::default() }).is_err());
        assert!(model.transient(&loads, &|_| 1.0, &bc, None, &TransientOptions { dt: bad, ..Default::default() }).is_err());
    }
    let opt = TransientOptions { steps: 2, ..Default::default() };
    assert!(model.transient(&loads, &|_| 1.0, &bc, Some((&[], &[])), &opt).is_err());
    let mut malformed = bc.clone();
    malformed.fixed.pop();
    assert!(model.transient(&loads, &|_| 1.0, &malformed, None, &opt).is_err());
    assert!(model.modal(&malformed, &Default::default()).is_err());
    assert!(model.transient(&loads, &|_| 1.0, &bc, None, &TransientOptions { record: vec![model.mesh.n_dofs()], ..opt.clone() }).is_err());
    assert!(model.transient(&loads, &|_| 1.0, &bc, None, &TransientOptions { rayleigh: (-1.0, 0.0), ..opt.clone() }).is_err());
    assert!(model.transient(&loads, &|t| if t == 0.0 { 0.0 } else { f64::NAN }, &bc, None, &opt).is_err());
}

#[test]
fn transient_load_function_is_evaluated_once_per_time() {
    use std::cell::Cell;
    let (model, bc, _, loads) = bar_for_transient();
    let calls = Cell::new(0);
    let opt = TransientOptions { steps: 4, ..Default::default() };
    model.transient(&loads, &|_| { calls.set(calls.get() + 1); 1.0 }, &bc, None, &opt).unwrap();
    assert_eq!(calls.get(), opt.steps + 1);
}

#[test]
fn freely_vibrating_axial_mode_converges_to_the_harmonic_solution() {
    // Independent temporal reference: an undamped eigenmode obeys q'' + omega² q = 0.
    // Average acceleration is second order; halving dt must reduce displacement error by ~4.
    let (model, bc, tip, loads) = bar_for_transient();
    let mode = model.modal(&bc, &ModalOptions { n_modes: 1, ..Default::default() }).unwrap().modes.remove(0);
    let omega = mode.omega2.sqrt();
    let v0 = vec![0.0; mode.shape.len()];
    let end = 1.3 / omega;
    let mut errors = Vec::new();
    for steps in [20, 40, 80] {
        let out = model.transient(&loads, &|_| 0.0, &bc, Some((&mode.shape, &v0)), &TransientOptions { dt: end / steps as f64, steps, record: vec![tip * 2], ..Default::default() }).unwrap();
        let exact = mode.shape[tip * 2] * (omega * end).cos();
        errors.push((out.u[tip * 2] - exact).abs());
        fea_core::report::AcceptanceReport::transient(&out, 1e-8,
            Some(fea_core::report::TransientEnergy::Conserved { tol: 1e-10 })).require().unwrap();
        assert_eq!(out.residuals.len(), steps + 1);
        assert!(out.residuals.iter().all(|r| r.is_finite() && *r <= 1e-8));
        let e0 = out.energy[0];
        assert!(out.energy.iter().all(|e| (e / e0 - 1.0).abs() < 1e-10));
    }
    assert!(errors[0] / errors[1] > 3.9 && errors[1] / errors[2] > 3.9, "{errors:?}");
}

#[test]
fn pinned_column_buckles_at_euler_load_and_scales_with_reference_force() {
    let (l, h) = (2.0, 0.05);
    let model = beam2d(ElementKind::Quad9, [32, 2], l, h, 1.0);
    let mut bc = model.dirichlet();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        // Roller supports at both section centroids permit end rotation and right-end shortening.
        if (x[1] - h / 2.0).abs() < 1e-12 && (x[0] < 1e-12 || (x[0] - l).abs() < 1e-12) {
            bc.fix(n, 1, 0.0);
            if x[0] < 1e-12 { bc.fix(n, 0, 0.0); }
        }
    }
    let euler = PI * PI * h.powi(3) / (12.0 * l * l);
    let opt = BucklingOptions { n_modes: 1, ..Default::default() };
    let first = model.buckling(&column_load(&model, 1.0, h), &bc, &opt).unwrap().modes[0].load_factor;
    let doubled = model.buckling(&column_load(&model, 2.0, h), &bc, &opt).unwrap().modes[0].load_factor;
    assert!((first / euler - 1.0).abs() < 0.01, "{first} vs {euler}");
    assert!((doubled * 2.0 / first - 1.0).abs() < 1e-8);
}

#[test]
fn fixed_fixed_axial_modes_match_the_independent_spectrum_and_mass_orthogonality() {
    let model = beam2d(ElementKind::Quad9, [16, 1], 2.0, 0.1, 3.0);
    let mut bc = model.dirichlet();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        bc.fix(n, 1, 0.0);
        if x[0] < 1e-12 || (x[0] - 2.0).abs() < 1e-12 { bc.fix(n, 0, 0.0); }
    }
    let result = model.modal(&bc, &ModalOptions { n_modes: 3, ..Default::default() }).unwrap();
    fea_core::report::AcceptanceReport::modal(&result, 1e-9).require().unwrap();
    let mass = model.assemble_mass(MassKind::Consistent).unwrap();
    for (i, a) in result.modes.iter().enumerate() {
        let exact = (i + 1) as f64 * PI / (2.0 * 3.0_f64.sqrt());
        assert!((a.omega2.sqrt() / exact - 1.0).abs() < 1e-4);
        let mut ma = vec![0.0; a.shape.len()];
        mass.matvec_add(&model.pattern, &a.shape, &mut ma);
        for (j, b) in result.modes.iter().enumerate() {
            let product: f64 = b.shape.iter().zip(&ma).map(|(x, y)| x * y).sum();
            assert!((product - if i == j { 1.0 } else { 0.0 }).abs() < 1e-8);
        }
    }
}
