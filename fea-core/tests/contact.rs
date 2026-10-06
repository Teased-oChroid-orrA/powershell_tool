//! Contact against closed-form solutions: Hertz (cylinder and sphere on an elastic half-space),
//! Coulomb friction, a shrink fit (Lame) between non-matching meshes, and a contact patch test.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::contact::{ContactRule, ContactSpec, RigidMaster, RigidShape};
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Loop, Region};
use fea_core::mesh2d::mesh_region;
use fea_core::*;
use std::f64::consts::PI;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;

/// Every slave Gauss point of interface 0 as `(reference x, pressure, gap, active)`, sorted by x. Reconstructed from the faces the
/// way the contact set does (3-point Gauss on each 2- or 3-node edge).
fn slave_points(sol: &NlSolution, model: &Model, faces: &[Vec<usize>], rule: ContactRule) -> Vec<(f64, f64, f64, bool)> {
    let g2 = 1.0 / 3.0f64.sqrt();
    let gl: Vec<f64> = match rule {
        ContactRule::Gauss => vec![-0.774_596_669_241_483_4, 0.0, 0.774_596_669_241_483_4],
        ContactRule::Reduced => vec![-g2, g2],
        ContactRule::Nodal | ContactRule::Auto => vec![-1.0, 0.0, 1.0],
    };
    let mut out = Vec::new();
    let mut k = 0;
    for face in faces {
        for &xi in &gl {
            let n: Vec<f64> = if face.len() == 2 { vec![0.5 * (1.0 - xi), 0.5 * (1.0 + xi)] } else { vec![0.5 * xi * (xi - 1.0), 0.5 * xi * (xi + 1.0), 1.0 - xi * xi] };
            let x: f64 = face.iter().zip(&n).map(|(&nd, nn)| nn * model.mesh.nodes[nd][0]).sum();
            let st = &sol.state.contact[0][k];
            out.push((x, st.p, st.g, st.active));
            k += 1;
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

fn hertz_2d(rule: ContactRule, tol_profile: f64) {
    // Rigid cylinder of radius R pressed into a block (plane strain); symmetry about x = 0.
    let (rad, w, depth) = (1.0f64, 2.0f64, 2.0f64);
    let region = Region::new(Loop::rectangle(0.0, -depth, w, 0.0).unwrap(), vec![], Elastic::new(E, NU)).unwrap();
    // Fine under the contact (a ~ 0.05), coarsening quickly away from it.
    let size = |x: [f64; 2]| (0.004 + 0.25 * x[0].hypot(x[1])).min(0.4);
    let mesh = mesh_region(&region, Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6, &size, MeshOptions::default()).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0); // symmetry
    }
    for &n in model.mesh.node_set("bottom").unwrap() {
        bc.fix_node(n);
    }
    let slave: Vec<Vec<usize>> = model.mesh.surfaces["top"].clone();
    // The cylinder touches the free surface at the origin and is pushed down by `travel` (the load factor scales it).
    let travel = 2.0e-3;
    let master = RigidMaster::new(RigidShape::Circle { c: [0.0, rad], r: rad }).with_travel([0.0, -travel, 0.0]);
    let spec = ContactSpec::rigid("hertz", slave.clone(), master, 1.0e9).with_rule(rule);
    let sol = model.solve_nonlinear_contact(&Loads::default(), &bc, vec![spec], &NlOptions { steps: 4, ..NlOptions::default() }).unwrap();
    assert!(sol.complete(), "{:?}", sol.stop);
    let cs = sol.contact.as_ref().unwrap();
    // Force on the (full, mirrored) cylinder: twice the half-model reaction, per unit thickness.
    let p_total = 2.0 * cs.master_force[0][1];
    let estar = E / (1.0 - NU * NU);
    let a_hertz = (4.0 * p_total * rad / (PI * estar)).sqrt();
    let p0_hertz = 2.0 * p_total / (PI * a_hertz);
    let pts = slave_points(&sol, &model, &slave, rule);
    let p0_fe = pts.iter().map(|p| p.1).fold(0.0f64, f64::max);
    // Contact edge: between the outermost loaded point and the first unloaded one beyond it.
    let last = pts.iter().rposition(|p| p.3).unwrap();
    let a_fe = 0.5 * (pts[last].0 + pts[last + 1].0);
    let spacing = pts[last + 1].0 - pts[last].0;
    eprintln!("{rule:?}: Hertz 2D: P = {p_total:.1}, contact half-width FE {a_fe:.5} (+-{:.5}) vs {a_hertz:.5} ({:+.1}%), peak pressure FE {p0_fe:.1} vs {p0_hertz:.1} ({:+.1}%), {} active, {} factorisations", 0.5 * spacing, 100.0 * (a_fe / a_hertz - 1.0), 100.0 * (p0_fe / p0_hertz - 1.0), cs.n_active, sol.factorisations);
    assert!(p_total > 0.0);
    assert!((a_fe - a_hertz).abs() < spacing.max(0.06 * a_hertz), "contact half-width {a_fe} vs {a_hertz}");
    assert!((p0_fe / p0_hertz - 1.0).abs() < 0.06, "peak pressure {p0_fe} vs {p0_hertz}");
    // Pressure profile: p(x) = p0 sqrt(1 - (x/a)^2) at the loaded points inside 80 % of the patch.
    let mut worst = 0.0f64;
    for &(x, p, _, act) in pts.iter().filter(|p| p.0 < 0.8 * a_hertz) {
        assert!(act, "every point well inside the patch is loaded");
        worst = worst.max((p - p0_hertz * (1.0 - (x / a_hertz).powi(2)).sqrt()).abs() / p0_hertz);
    }
    eprintln!("{rule:?}: worst profile deviation {:.1}% of p0", 100.0 * worst);
    assert!(worst < tol_profile, "{rule:?}: profile deviation {worst}");
    assert!(cs.min_gap > -2e-3 * travel, "the augmented multipliers must remove the penetration: min gap {} (travel {travel})", cs.min_gap);
}

#[test]
fn hertz_2d_with_every_collocation_rule() {
    // Full Gauss collocation on quadratic edges is over-constrained (3 constraints per edge for 2
    // independent displacements) and oscillates; the reduced and nodal rules are smooth.
    hertz_2d(ContactRule::Gauss, 0.15);
    hertz_2d(ContactRule::Reduced, 0.03);
    hertz_2d(ContactRule::Nodal, 0.03);
}

use fea_core::generate::grid;

/// Block `[0, 2] x [0, 1]` of Quad9 elements on a rigid line `y = 0`: the bottom edge is the slave
/// surface, the top edge carries a pressure and is held in `x`, the plane moves by `shift` (times the load factor).
fn block_on_plane(mu: f64, q: f64, shift: f64) -> (Model, Dirichlet, Loads, Vec<ContactSpec>) {
    let mesh = grid(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Quad9, Elastic::new(E, NU), [8, 4, 1], &|p| [2.0 * p[0], p[1], 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    if mu > 0.0 {
        for &n in model.mesh.node_set("v1").unwrap() {
            bc.fix(n, 0, 0.0); // the top is held against the drag
        }
    } else {
        // Frictionless: only the rigid-body slide is removed (one node), so the block expands freely.
        let mid = (0..model.mesh.nodes.len()).find(|&n| (model.mesh.nodes[n][0] - 1.0).abs() < 1e-12 && (model.mesh.nodes[n][1] - 1.0).abs() < 1e-12).unwrap();
        bc.fix(mid, 0, 0.0);
    }
    let loads = Loads { faces: model.mesh.surfaces["v1"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(q))).collect(), ..Loads::default() };
    let plane = RigidMaster::new(RigidShape::Plane { p: [0.0; 3], n: [0.0, 1.0, 0.0] }).with_travel([shift, 0.0, 0.0]);
    // With friction the penalties must stay comparable to the structural stiffness (E / h): the friction
    // force reacts to gap changes with stiffness mu * eps_n, which the symmetric quasi-Newton tangent
    // does not capture; the multiplier passes supply the accuracy instead.
    let mut spec = ContactSpec::rigid("base", model.mesh.surfaces["v0"].clone(), plane, if mu > 0.0 { 3.0e7 } else { 1.0e9 });
    if mu > 0.0 {
        spec = spec.with_friction(mu, 3.0e7);
    }
    (model, bc, loads, vec![spec])
}

#[test]
fn frictionless_block_on_a_rigid_plane_carries_a_uniform_pressure() {
    let q = 1.0e4;
    let (model, bc, loads, specs) = block_on_plane(0.0, q, 0.0);
    let sol = model.solve_nonlinear_contact(&loads, &bc, specs, &NlOptions { steps: 2, ..NlOptions::default() }).unwrap();
    assert!(sol.complete());
    let cs = sol.contact.as_ref().unwrap();
    // Equilibrium: the rigid plane carries q L.
    assert!((cs.master_force[0][1] + q * 2.0).abs() < 1e-4 * q * 2.0, "normal force on the plane {} vs {}", cs.master_force[0][1], -q * 2.0);
    // Uniform pressure at every slave point, equal to the applied one.
    let ps: Vec<f64> = sol.state.contact[0].iter().map(|s| s.p).collect();
    let worst = ps.iter().fold(0.0f64, |m, p| m.max((p - q).abs())) / q;
    eprintln!("uniform pressure: worst deviation {worst:.2e}, {} slave points", ps.len());
    assert!(worst < 5e-3, "pressure deviation {worst}");
    assert!(cs.master_force[0][0].abs() < 1e-6 * q, "no friction force without friction: {}", cs.master_force[0][0]);
}

#[test]
fn coulomb_friction_transmits_exactly_mu_n_when_sliding_and_the_elastic_shear_when_sticking() {
    let (q, mu) = (1.0e4, 0.3);
    // Large drag: the plane slides under the block.
    let (model, bc, loads, specs) = block_on_plane(mu, q, 0.05);
    let sol = model.solve_nonlinear_contact(&loads, &bc, specs, &NlOptions { steps: 5, outer_tol: 1e-6, max_outer: 40, ..NlOptions::default() }).unwrap();
    assert!(sol.complete(), "{:?}", sol.stop);
    let cs = sol.contact.as_ref().unwrap();
    let (n_force, f_t) = (-cs.master_force[0][1], cs.master_force[0][0]);
    eprintln!("sliding: N = {n_force:.2}, friction force on the plane {f_t:.2}, mu N = {:.2}", mu * n_force);
    assert!((n_force - 2.0 * q).abs() < 1e-4 * 2.0 * q);
    assert!((f_t + mu * n_force).abs() < 1e-3 * mu * n_force, "friction {f_t} vs {}", -mu * n_force);
    // `fric` is the traction along the slip direction (the slave slips toward -x relative to the plane); the force on the block, -fric, drags it along +x.
    assert!(sol.state.contact[0].iter().all(|s| s.fric[0] < 0.0 || !s.active), "friction on the block drags it along +x");
    // The same with the plane moving the other way: the force reverses.
    let (model, bc, loads, specs) = block_on_plane(mu, q, -0.05);
    let rev = model.solve_nonlinear_contact(&loads, &bc, specs, &NlOptions { steps: 5, outer_tol: 1e-6, max_outer: 40, ..NlOptions::default() }).unwrap();
    let f_rev = rev.contact.as_ref().unwrap().master_force[0][0];
    assert!((f_rev + f_t).abs() < 1e-3 * f_t.abs(), "{f_rev} vs {f_t}");

    // Tiny drag: the block sticks to the plane, so the response is the elastic shear of the block with
    // its bottom displaced by the same amount (a linear solve with prescribed bottom displacements).
    let delta = 2.0e-5;
    let (model, bc, loads, specs) = block_on_plane(mu, q, delta);
    let stick = model.solve_nonlinear_contact(&loads, &bc, specs, &NlOptions { steps: 4, outer_tol: 1e-6, max_outer: 40, ..NlOptions::default() }).unwrap();
    assert!(stick.complete());
    let f_stick = stick.contact.as_ref().unwrap().master_force[0][0];
    let (tied, mut tbc, _, _) = block_on_plane(mu, q, delta);
    for &n in tied.mesh.node_set("v0").unwrap() {
        tbc.fix(n, 0, delta);
        tbc.fix(n, 1, 0.0);
    }
    // Reference: shear only (the normal load is carried by the fixed bottom in the tied model).
    let lin = tied.solve_static(&Loads::default(), &tbc).unwrap();
    let f_tied: f64 = tied.mesh.node_set("v0").unwrap().iter().map(|&n| lin.reactions[n * 2]).sum();
    eprintln!("sticking: friction force on the plane {f_stick:.4} vs tied elastic shear {:.4} (mu N = {:.1})", -f_tied, mu * 2.0 * q);
    assert!(f_stick.abs() < 0.5 * mu * 2.0 * q, "must be below the friction capacity");
    assert!((f_stick + f_tied).abs() < 0.02 * f_tied.abs(), "stick force {f_stick} vs elastic shear {}", -f_tied);
}

// ---------------------------------------------------------------- deformable-deformable contact

/// Two stacked blocks with different meshes meet at `y = 1`; a pressure `q` loads the top block.
fn stacked_blocks(div_bottom: usize, div_top: usize, slave_is_bottom: bool, q: f64) -> (Model, Dirichlet, Loads, Vec<ContactSpec>) {
    let mat = Elastic::new(E, NU);
    let bottom = grid(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Quad9, mat, [div_bottom, 3, 1], &|p| [2.0 * p[0], p[1], 0.0]).unwrap();
    let top = grid(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Quad9, mat, [div_top, 3, 1], &|p| [2.0 * p[0], 1.0 + p[1], 0.0]).unwrap();
    let mut mesh = bottom;
    mesh.append(&top, "top_").unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("v0").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    bc.fix(model.mesh.node_set("v0").unwrap()[0], 0, 0.0);
    let loads = Loads { faces: model.mesh.surfaces["top_v1"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(q))).collect(), ..Loads::default() };
    let (b_top, t_bot) = (model.mesh.surfaces["v1"].clone(), model.mesh.surfaces["top_v0"].clone());
    // The bottom block's top surface and the top block's bottom surface.
    let specs = if slave_is_bottom { vec![ContactSpec::deformable("interface", b_top, t_bot, 1.0e9)] } else { vec![ContactSpec::deformable("interface", t_bot, b_top, 1.0e9)] };
    (model, bc, loads, specs)
}

#[test]
fn contact_patch_test_across_non_matching_meshes() {
    // Uniform pressure through an interface whose two sides are meshed with 6 and 4 elements: the
    // contact pressure must be q everywhere. Gauss-point-to-segment integration passes only
    // approximately (the classical limitation), so the tolerance is the measured one.
    let q = 1.0e4;
    for slave_is_bottom in [true, false] {
        let (model, bc, loads, specs) = stacked_blocks(6, 4, slave_is_bottom, q);
        let sol = model.solve_nonlinear_contact(&loads, &bc, specs, &NlOptions { steps: 2, ..NlOptions::default() }).unwrap();
        assert!(sol.complete(), "{:?}", sol.stop);
        let ps: Vec<f64> = sol.state.contact[0].iter().map(|s| s.p).collect();
        let (lo, hi) = ps.iter().fold((f64::INFINITY, 0.0f64), |a, &p| (a.0.min(p), a.1.max(p)));
        let mean = ps.iter().sum::<f64>() / ps.len() as f64;
        eprintln!("patch test (slave = {}): pressure {lo:.0}..{hi:.0}, mean {mean:.0} (q = {q}), {} points", if slave_is_bottom { "bottom block, 6 elements" } else { "top block, 4 elements" }, ps.len());
        assert!((mean / q - 1.0).abs() < 2e-2, "mean pressure {mean}");
        assert!(lo > 0.85 * q && hi < 1.15 * q, "pressure {lo}..{hi}");
        let cs = sol.contact.as_ref().unwrap();
        assert!((cs.master_force[0][1].abs() - 2.0 * q).abs() < 1e-3 * 2.0 * q, "transmitted force {} vs {}", cs.master_force[0][1], 2.0 * q);
    }
}

#[test]
fn shrink_fit_between_two_unstructured_meshes_matches_lame() {
    // A shaft (radius a + delta) pressed into a hub (inner radius a, outer radius b), same material,
    // plane strain, one quarter: the interface pressure from Lame's compatibility
    //   delta = (1 + nu) p a / E [ ((1 - 2 nu) a^2 + b^2) / (b^2 - a^2) + (1 - 2 nu) ].
    let (a, b, delta) = (1.0f64, 2.0f64, 0.002f64);
    let p_lame = delta * E / ((1.0 + NU) * a * (((1.0 - 2.0 * NU) * a * a + b * b) / (b * b - a * a) + (1.0 - 2.0 * NU)));
    let mat = Elastic::new(E, NU);
    let seg = |curve, name: &str| fea_core::geometry::Segment { curve, name: name.to_string() };
    use fea_core::geometry::{Curve, Segment};
    let _ = seg(Curve::line([0.0, 0.0], [1.0, 0.0]), "x");
    let rs = a + delta;
    let shaft = Loop::new(vec![
        Segment { curve: Curve::line([0.0, 0.0], [rs, 0.0]), name: "xaxis".into() },
        Segment { curve: Curve::arc([0.0, 0.0], rs, 0.0, std::f64::consts::FRAC_PI_2), name: "iface".into() },
        Segment { curve: Curve::line([0.0, rs], [0.0, 0.0]), name: "yaxis".into() },
    ])
    .unwrap();
    let hub = Loop::new(vec![
        Segment { curve: Curve::line([a, 0.0], [b, 0.0]), name: "xaxis".into() },
        Segment { curve: Curve::arc([0.0, 0.0], b, 0.0, std::f64::consts::FRAC_PI_2), name: "outer".into() },
        Segment { curve: Curve::line([0.0, b], [0.0, a]), name: "yaxis".into() },
        Segment { curve: Curve::arc([0.0, 0.0], a, std::f64::consts::FRAC_PI_2, 0.0), name: "iface".into() },
    ])
    .unwrap();
    let physics = Physics::PlaneStrain { thickness: 1.0 };
    // Different resolutions on the two sides of the interface.
    let m_shaft = mesh_region(&Region::new(shaft, vec![], mat).unwrap(), physics, ElementKind::Tri6, &|_| 0.12, MeshOptions::default()).unwrap();
    let m_hub = mesh_region(&Region::new(hub, vec![], mat).unwrap(), physics, ElementKind::Tri6, &|_| 0.19, MeshOptions::default()).unwrap();
    let mut mesh = m_shaft;
    mesh.append(&m_hub, "hub_").unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for prefix in ["", "hub_"] {
        for &n in model.mesh.node_set(&format!("{prefix}xaxis")).unwrap() {
            bc.fix(n, 1, 0.0);
        }
        for &n in model.mesh.node_set(&format!("{prefix}yaxis")).unwrap() {
            bc.fix(n, 0, 0.0);
        }
    }
    let (slave, master) = (model.mesh.surfaces["iface"].clone(), model.mesh.surfaces["hub_iface"].clone());
    let opt = NlOptions { steps: 1, ..NlOptions::default() };
    // Two-pass: each side is the slave once; the pressures of the two passes add up to Lame's.
    let both = ContactSpec::two_pass("fit", slave.clone(), master.clone(), 1.0e9);
    let two = model.solve_nonlinear_contact(&Loads::default(), &bc, both.iter().map(|s| s.clone().with_margin(0.1)).collect(), &opt).unwrap();
    assert!(two.complete());
    let mean_of = |sol: &NlSolution, k: usize| -> f64 {
        let ps: Vec<f64> = sol.state.contact[k].iter().filter(|s| s.active).map(|s| s.p).collect();
        ps.iter().sum::<f64>() / ps.len() as f64
    };
    let (m0, m1) = (mean_of(&two, 0), mean_of(&two, 1));
    eprintln!("two-pass shrink fit: mean pressures {m0:.1} + {m1:.1} = {:.1} vs Lame {p_lame:.1}", m0 + m1);
    assert!(((m0 + m1) / p_lame - 1.0).abs() < 0.02, "{m0} + {m1} vs {p_lame}");
    // Single pass (shaft as slave).
    let spec = ContactSpec::deformable("fit", slave, master, 1.0e9).with_margin(0.1);
    let sol = model.solve_nonlinear_contact(&Loads::default(), &bc, vec![spec], &opt).unwrap();
    assert!(sol.complete(), "{:?}", sol.stop);
    let cs = sol.contact.as_ref().unwrap();
    let ps: Vec<f64> = sol.state.contact[0].iter().filter(|s| s.active).map(|s| s.p).collect();
    let mean = ps.iter().sum::<f64>() / ps.len() as f64;
    let (lo, hi) = ps.iter().fold((f64::INFINITY, 0.0f64), |m, &p| (m.0.min(p), m.1.max(p)));
    eprintln!("shrink fit: contact pressure mean {mean:.1}, range {lo:.1}..{hi:.1}, Lame {p_lame:.1}; {} active points, min gap {:.2e}", ps.len(), cs.min_gap);
    assert!(ps.len() == sol.state.contact[0].len(), "the whole interface is in contact");
    assert!((mean / p_lame - 1.0).abs() < 0.01, "mean pressure {mean} vs Lame {p_lame}");
    // Pointwise Gauss-point-to-segment pressures scatter on coarse non-matching meshes (about +-25 % here); the mean is what is exact.
    assert!(lo > 0.65 * p_lame && hi < 1.3 * p_lame, "pressure scatter {lo}..{hi}");
    assert!(cs.min_gap > -1e-2 * delta, "penetration must be removed: {}", cs.min_gap);
}

fn sphere_hertz(n: usize, tol_p0: f64, tol_inner: f64) {
    // One quarter of the block (symmetry planes x = 0, y = 0), graded toward the contact point.
    let (rad, side, depth) = (1.0f64, 1.0f64, 1.0f64);
    let mut mesh = grid(Physics::Solid, ElementKind::Hex27, Elastic::new(E, NU), [n, n, n], &move |p| {
        // Nearly uniform 0.008 elements out to the contact radius, strongly graded beyond.
        let g = |t: f64| side * (0.064 * t + 0.936 * t.powi(4));
        [g(p[0]), g(p[1]), -depth * (1.0 - p[2]).powf(2.0)]
    })
    .unwrap();
    let model = {
        mesh.select_nodes("bottom", |x| x[2] < -depth + 1e-12);
        Model::new(mesh).unwrap()
    };
    let mut bc = model.dirichlet();
    for &nd in model.mesh.node_set("u0").unwrap() {
        bc.fix(nd, 0, 0.0);
    }
    for &nd in model.mesh.node_set("v0").unwrap() {
        bc.fix(nd, 1, 0.0);
    }
    for &nd in model.mesh.node_set("bottom").unwrap() {
        bc.fix_node(nd);
    }
    let slave: Vec<Vec<usize>> = model.mesh.surfaces["w1"].clone();
    let travel = 1.6e-3;
    let master = RigidMaster::new(RigidShape::Sphere { c: [0.0, 0.0, rad], r: rad }).with_travel([0.0, 0.0, -travel]);
    let spec = ContactSpec::rigid("sphere", slave.clone(), master, 1.0e9);
    let sol = model.solve_nonlinear_contact(&Loads::default(), &bc, vec![spec], &NlOptions { steps: 4, ..NlOptions::default() }).unwrap();
    assert!(sol.complete(), "{:?}", sol.stop);
    let cs = sol.contact.as_ref().unwrap();
    let p_total = 4.0 * cs.master_force[0][2];
    let estar = E / (1.0 - NU * NU);
    let a_hertz = (3.0 * p_total * rad / (4.0 * estar)).cbrt();
    let p0_hertz = 3.0 * p_total / (2.0 * PI * a_hertz * a_hertz);
    let p0_fe = sol.state.contact[0].iter().map(|s| s.p).fold(0.0f64, f64::max);
    eprintln!("Hertz 3D: P = {p_total:.2}, a_Hertz {a_hertz:.5}, p0 FE {p0_fe:.1} vs {p0_hertz:.1} ({:+.1}%), {} active points, {} factorisations, min gap {:.2e}", 100.0 * (p0_fe / p0_hertz - 1.0), cs.n_active, sol.factorisations, cs.min_gap);
    let mut prof: Vec<(f64, f64)> = sol.contact_points[0].iter().zip(&sol.state.contact[0]).filter(|(_, st)| st.active).map(|(x, st)| (x[0].hypot(x[1]), st.p)).collect();
    prof.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Smoothness: the pressure follows Hertz's curve within a few percent of p0 over the inner 40 % of the patch.
    for lim in [0.4, 0.6, 0.8, 1.0] {
        let w = prof.iter().filter(|(r, _)| *r < lim * a_hertz).map(|(r, p)| (p - p0_hertz * (1.0 - (r / a_hertz).powi(2)).sqrt()).abs() / p0_hertz).fold(0.0f64, f64::max);
        eprintln!("  worst deviation within {lim} a: {:.1}% of p0", 100.0 * w);
    }
    let worst = prof.iter().filter(|(r, _)| *r < 0.4 * a_hertz).map(|(r, p)| (p - p0_hertz * (1.0 - (r / a_hertz).powi(2)).sqrt()).abs() / p0_hertz).fold(0.0f64, f64::max);
    eprintln!("3D profile: worst deviation {:.1}% of p0 over {} active points", 100.0 * worst, prof.len());
    assert!(worst < tol_inner, "3D pressure profile deviates by {worst}");
    // Contact radius from the active slave points: reference positions are recoverable from the active pressure points only
    // through the face geometry; use the force balance instead: p0 against Hertz, and the contact area from sum(p) = P / mean(p).
    assert!(p_total > 0.0);
    assert!((p0_fe / p0_hertz - 1.0).abs() < tol_p0, "peak pressure {p0_fe} vs {p0_hertz}");
    assert!(cs.min_gap > -2e-3 * travel);
}

/// Accurate 3D Hertz (8 x 8 x 8 Hex27, ~28 s): peak pressure +1.7 % from the closed form.
#[test]
#[ignore = "slow (~30 s): run with --ignored"]
fn rigid_sphere_on_an_elastic_block_reproduces_3d_hertz() {
    sphere_hertz(8, 0.04, 0.05);
}

/// A Hex27 block pressed uniformly against a rigid plane: exact uniform pressure and force.
#[test]
fn a_3d_block_on_a_rigid_plane_carries_the_applied_pressure_exactly() {
    let (q, side, h) = (2.0e4f64, 1.5f64, 0.8f64);
    let mesh = grid(Physics::Solid, ElementKind::Hex27, Elastic::new(E, NU), [3, 2, 2], &move |p| [side * p[0], side * p[1], h * p[2]]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    // Remove the rigid-body slides without restraining the lateral expansion: one node fixed in x and y, one more in y.
    let find = |x: f64, y: f64| (0..model.mesh.nodes.len()).find(|&n| (model.mesh.nodes[n][0] - x).abs() < 1e-12 && (model.mesh.nodes[n][1] - y).abs() < 1e-12 && model.mesh.nodes[n][2] < 1e-12).unwrap();
    bc.fix(find(0.0, 0.0), 0, 0.0);
    bc.fix(find(0.0, 0.0), 1, 0.0);
    bc.fix(find(side, 0.0), 1, 0.0);
    let loads = Loads { faces: model.mesh.surfaces["w1"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(q))).collect(), ..Loads::default() };
    let plane = RigidMaster::new(RigidShape::Plane { p: [0.0; 3], n: [0.0, 0.0, 1.0] });
    let spec = ContactSpec::rigid("floor", model.mesh.surfaces["w0"].clone(), plane, 1.0e9);
    let sol = model.solve_nonlinear_contact(&loads, &bc, vec![spec], &NlOptions { steps: 2, ..NlOptions::default() }).unwrap();
    assert!(sol.complete(), "{:?}", sol.stop);
    let cs = sol.contact.as_ref().unwrap();
    let want = q * side * side;
    assert!((cs.master_force[0][2] + want).abs() < 1e-4 * want, "force on the plane {} vs {}", cs.master_force[0][2], -want);
    let worst = sol.state.contact[0].iter().fold(0.0f64, |m, s| m.max((s.p - q).abs())) / q;
    eprintln!("3D uniform pressure: worst deviation {worst:.2e} over {} points", sol.state.contact[0].len());
    assert!(worst < 5e-3, "pressure deviation {worst}");
    assert!(cs.master_force[0][0].abs() < 1e-6 * want && cs.master_force[0][1].abs() < 1e-6 * want);
}

#[test]
fn two_meshed_bodies_reproduce_hertz_contact_and_the_result_is_penalty_independent() {
    use fea_core::geometry::{Curve, Segment};
    // Quarter of an elastic cylinder (radius R, centre (0, R)) pressed by a prescribed top displacement
    // onto an elastic block; both bodies are meshed and both deform.
    let rad = 1.0f64;
    let mat = Elastic::new(E, NU);
    let physics = Physics::PlaneStrain { thickness: 1.0 };
    let disc = Loop::new(vec![
        Segment { curve: Curve::arc([0.0, rad], rad, -std::f64::consts::FRAC_PI_2, 0.0), name: "arc".into() },
        Segment { curve: Curve::line([rad, rad], [0.0, rad]), name: "top".into() },
        Segment { curve: Curve::line([0.0, rad], [0.0, 0.0]), name: "axis".into() },
    ])
    .unwrap();
    let block = Loop::rectangle(0.0, -2.0, 2.0, 0.0).unwrap();
    let dsize = |x: [f64; 2]| (0.005 + 0.25 * x[0].hypot(x[1])).min(0.3);
    let m_disc = mesh_region(&Region::new(disc, vec![], mat).unwrap(), physics, ElementKind::Tri6, &dsize, MeshOptions::default()).unwrap();
    let m_block = mesh_region(&Region::new(block, vec![], mat).unwrap(), physics, ElementKind::Tri6, &|x| (0.006 + 0.25 * x[0].hypot(x[1])).min(0.4), MeshOptions::default()).unwrap();
    let mut mesh = m_disc;
    mesh.append(&m_block, "blk_").unwrap();
    let model = Model::new(mesh).unwrap();
    let delta = 3.0e-3;
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("axis").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    for &n in model.mesh.node_set("top").unwrap() {
        bc.fix(n, 1, -delta); // the load factor scales the displacement
    }
    for &n in model.mesh.node_set("blk_left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    for &n in model.mesh.node_set("blk_bottom").unwrap() {
        bc.fix_node(n);
    }
    let (slave, master) = (model.mesh.surfaces["arc"].clone(), model.mesh.surfaces["blk_top"].clone());
    let estar = 1.0 / (2.0 * (1.0 - NU * NU) / E); // two identical bodies
    let mut results = Vec::new();
    for eps in [3.0e8, 1.0e9, 3.0e9] {
        let spec = ContactSpec::deformable("hertz2", slave.clone(), master.clone(), eps).with_margin(0.08);
        let sol = model.solve_nonlinear_contact(&Loads::default(), &bc, vec![spec], &NlOptions { steps: 6, ..NlOptions::default() }).unwrap();
        assert!(sol.complete(), "eps {eps:e}: {:?}", sol.stop);
        let cs = sol.contact.as_ref().unwrap();
        let p_total = -2.0 * cs.master_force[0][1]; // the quarter-model force on the block (downward), doubled for the full contact
        let a_hertz = (4.0 * p_total * rad / (PI * estar)).sqrt();
        let p0_hertz = 2.0 * p_total / (PI * a_hertz);
        // Contact extent (outermost loaded point plus half the gap to the next one) and the mean pressure over
        // the inner half of the patch against Hertz's `p0 * mean sqrt(1 - (x/a)^2)`.
        let mut pts: Vec<(f64, f64, bool)> = sol.contact_points[0].iter().zip(&sol.state.contact[0]).map(|(x, st)| (x[0], st.p, st.active)).collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let last = pts.iter().rposition(|p| p.2).unwrap();
        let a_fe = 0.5 * (pts[last].0 + pts[(last + 1).min(pts.len() - 1)].0);
        let inner: Vec<f64> = pts.iter().filter(|p| p.0 < 0.5 * a_hertz).map(|p| p.1).collect();
        let mean_inner = inner.iter().sum::<f64>() / inner.len() as f64;
        // mean of sqrt(1 - t^2) over [0, 0.5] = (0.5 sqrt(0.75) + asin(0.5)) / (2 * 0.5)
        let hertz_inner = p0_hertz * (0.5 * 0.75f64.sqrt() + 0.5f64.asin()) / (2.0 * 0.5);
        eprintln!("two bodies, eps {eps:e}: P = {p_total:.1}, a FE {a_fe:.4} vs Hertz {a_hertz:.4}, inner mean pressure {mean_inner:.0} vs {hertz_inner:.0} ({:+.1}%), {} active, min gap {:.1e}", 100.0 * (mean_inner / hertz_inner - 1.0), cs.n_active, cs.min_gap);
        assert!(p_total > 0.0 && cs.n_active > 6);
        assert!((a_fe / a_hertz - 1.0).abs() < 0.12, "eps {eps:e}: contact half-width {a_fe} vs {a_hertz}");
        assert!((mean_inner / hertz_inner - 1.0).abs() < 0.12, "eps {eps:e}: inner pressure {mean_inner} vs {hertz_inner}");
        results.push(p_total);
    }
    // The augmented Lagrangian makes the transmitted force independent of the penalty over a decade.
    let (lo, hi) = results.iter().fold((f64::INFINITY, 0.0f64), |m, &r| (m.0.min(r), m.1.max(r)));
    assert!((hi - lo) / hi < 0.002, "force varies with the penalty: {lo}..{hi}");
}

#[test]
fn contact_with_plasticity_yields_below_the_surface_near_0_78_a() {
    use fea_core::material::J2;
    use mechanics_core::hardening::Hardening;
    // Rigid cylinder indenting an elastic-perfectly-plastic block (plane strain, small strain).
    // The elastic Hertz stress field has its maximum shear below the surface at z ~ 0.78 a, so the
    // first yielding starts there, not at the contact surface.
    let (rad, w, depth) = (1.0f64, 1.5f64, 1.5f64);
    let region = Region::new(Loop::rectangle(0.0, -depth, w, 0.0).unwrap(), vec![], Elastic::new(E, NU)).unwrap();
    let size = |x: [f64; 2]| (0.004 + 0.22 * x[0].hypot(x[1])).min(0.35);
    let mut mesh = mesh_region(&region, Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6, &size, MeshOptions::default()).unwrap();
    mesh.set_plasticity(0, J2::small(Hardening::linear(1.0e5, 0.0, 5.0).unwrap())).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    for &n in model.mesh.node_set("bottom").unwrap() {
        bc.fix_node(n);
    }
    let slave = model.mesh.surfaces["top"].clone();
    let run = |travel: f64| {
        let master = RigidMaster::new(RigidShape::Circle { c: [0.0, rad], r: rad }).with_travel([0.0, -travel, 0.0]);
        model.solve_nonlinear_contact(&Loads::default(), &bc, vec![ContactSpec::rigid("c", slave.clone(), master, 1.0e9)], &NlOptions { steps: 8, ..NlOptions::default() }).unwrap()
    };
    // A small indentation stays elastic; a larger one yields.
    let soft = run(1.0e-3);
    assert!(soft.complete());
    assert_eq!(soft.state.max_ep(), 0.0, "the small indentation must be elastic");
    let sol = run(2.6e-3);
    assert!(sol.complete(), "{:?}", sol.stop);
    let cs = sol.contact.as_ref().unwrap();
    let p_total = 2.0 * cs.master_force[0][1];
    let estar = E / (1.0 - NU * NU);
    let a_hertz = (4.0 * p_total * rad / (PI * estar)).sqrt();
    let yielded: Vec<(f64, f64)> = sol.state.gauss_ep(&model.mesh).iter().filter(|(_, ep)| *ep > 0.0).map(|(x, ep)| (-x[1], *ep)).collect();
    assert!(!yielded.is_empty(), "the large indentation must yield (peak elastic pressure {:.0} vs sigma_y 1e5)", 2.0 * p_total / (PI * a_hertz));
    let depth_of_max = yielded.iter().cloned().fold((0.0, 0.0), |m, v| if v.1 > m.1 { v } else { m }).0;
    eprintln!("contact + plasticity: P {p_total:.0}, a_Hertz {a_hertz:.4}, max ep {:.2e} at depth {depth_of_max:.4} = {:.2} a (Johnson: 0.78 a)", sol.state.max_ep(), depth_of_max / a_hertz);
    assert!((0.5..1.2).contains(&(depth_of_max / a_hertz)), "yielding must start below the surface: {:.2} a", depth_of_max / a_hertz);
    assert!(yielded.iter().all(|(z, _)| *z > 0.0), "no yielding at the contact surface itself");
}

/// A rigid cylinder free to move vertically under an applied force settles where the
/// displacement-controlled run that produces the same contact force puts it.
#[test]
fn a_force_controlled_rigid_cylinder_reproduces_the_prescribed_travel_solution() {
    let (rad, w, depth) = (1.0f64, 2.0f64, 2.0f64);
    let region = Region::new(Loop::rectangle(0.0, -depth, w, 0.0).unwrap(), vec![], Elastic::new(E, NU)).unwrap();
    let size = |x: [f64; 2]| (0.01 + 0.25 * x[0].hypot(x[1])).min(0.4);
    let mesh = mesh_region(&region, Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Tri6, &size, MeshOptions::default()).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    for &n in model.mesh.node_set("bottom").unwrap() {
        bc.fix_node(n);
    }
    let slave = model.mesh.surfaces["top"].clone();
    let travel = 2.0e-3;
    let opt = NlOptions { steps: 4, ..NlOptions::default() };
    let circle = || RigidMaster::new(RigidShape::Circle { c: [0.0, rad], r: rad });
    let disp = model.solve_nonlinear_contact(&Loads::default(), &bc, vec![ContactSpec::rigid("d", slave.clone(), circle().with_travel([0.0, -travel, 0.0]), 1.0e9)], &opt).unwrap();
    assert!(disp.complete(), "{:?}", disp.stop);
    let force = disp.contact.as_ref().unwrap().master_force[0][1]; // force on the (half) cylinder, +y
    assert!(force > 0.0);
    let free = model.solve_nonlinear_contact(&Loads::default(), &bc, vec![ContactSpec::rigid("f", slave, circle().with_free_translation([false, true, false], [0.0, -force, 0.0]), 1.0e9)], &opt).unwrap();
    assert!(free.complete(), "{:?}", free.stop);
    let y = free.rigid_translation[0][1];
    eprintln!("free cylinder: settles at {y:.6e}, prescribed {:.6e}", -travel);
    assert!((y + travel).abs() < 0.01 * travel, "rigid translation {y} vs {}", -travel);
    let f2 = free.contact.as_ref().unwrap().master_force[0][1];
    assert!((f2 / force - 1.0).abs() < 1e-6, "equilibrium: contact force {f2} vs applied {force}");
}

/// A lightly loaded contact (pressures far below `eps_n` times the local finite-difference step of the contact
/// tangent) must still converge in a handful of iterations: the tangent is the derivative of the smooth active
/// branch, not a central difference straddling the pressure kink (which halved the stiffness of every
/// marginal point and made Newton crawl).
#[test]
fn a_lightly_loaded_stiff_penalty_contact_converges_in_few_factorisations() {
    let (rad, w, depth) = (1.0f64, 2.0f64, 2.0f64);
    let region = Region::new(Loop::rectangle(0.0, -depth, w, 0.0).unwrap(), vec![], Elastic::new(E, NU)).unwrap();
    let size = |x: [f64; 2]| (0.01 + 0.25 * x[0].hypot(x[1])).min(0.4);
    let mesh = mesh_region(&region, Physics::PlaneStrain { thickness: 0.25 }, ElementKind::Tri6, &size, MeshOptions::default()).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("left").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    for &n in model.mesh.node_set("bottom").unwrap() {
        bc.fix_node(n);
    }
    let slave = model.mesh.surfaces["top"].clone();
    // About 5 lbf on the half cylinder: contact pressures of order 10 psi against eps_n * h_fd ~ 1e3.
    let master = RigidMaster::new(RigidShape::Circle { c: [0.0, rad], r: rad }).with_free_translation([false, true, false], [0.0, -5.0, 0.0]);
    let spec = ContactSpec::rigid("light", slave, master, 4.0e9).with_rule(ContactRule::Reduced);
    let sol = model.solve_nonlinear_contact(&Loads::default(), &bc, vec![spec], &NlOptions { steps: 2, ..NlOptions::default() }).unwrap();
    assert!(sol.complete(), "{:?}", sol.stop);
    let f = sol.contact.as_ref().unwrap().master_force[0][1];
    assert!((f / 5.0 - 1.0).abs() < 1e-3, "equilibrium: contact force {f} vs 5");
    assert!(sol.factorisations <= 30, "Newton needed {} factorisations", sol.factorisations);
}
