//! Linear static analysis against closed-form solutions: patch tests on every element type,
//! the Lame thick cylinder in plane strain, axisymmetric and 3D, thermal and body loads, and
//! equilibrium of the reactions.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::element::ALL_KINDS;
use fea_core::generate::grid;
use fea_core::*;
use std::f64::consts::PI;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;

fn mat() -> Elastic {
    Elastic::new(E, NU)
}

fn physics_for(kind: ElementKind) -> Physics {
    if kind.dim() == 3 {
        Physics::Solid
    } else {
        Physics::PlaneStress { thickness: 0.5 }
    }
}

/// An affine map with skew, so no cell is a rectangle.
fn skew(p: [f64; 3]) -> [f64; 3] {
    [2.0 * p[0] + 0.3 * p[1] + 0.1 * p[2], 0.2 * p[0] + 1.5 * p[1] + 0.2 * p[2], 0.1 * p[0] - 0.1 * p[1] + 1.2 * p[2]]
}

#[test]
fn patch_test_every_element_reproduces_a_linear_displacement_field() {
    // Prescribe u = H x + c on the whole boundary; the interior must follow exactly.
    let h = [[1.0e-3, 4.0e-4, -2.0e-4], [-3.0e-4, 2.0e-3, 5.0e-4], [2.0e-4, -1.0e-4, 8.0e-4]];
    for kind in ALL_KINDS {
        let d = kind.dim();
        let div = if d == 2 { [3, 3, 1] } else { [2, 2, 2] };
        let mesh = grid(physics_for(kind), kind, mat(), div, &skew).unwrap();
        let model = Model::new(mesh).unwrap();
        let exact = |x: &[f64; 3], i: usize| -> f64 { (0..d).map(|k| h[i][k] * x[k]).sum::<f64>() + 1e-3 * (i as f64 + 1.0) };
        let mut bc = model.dirichlet();
        let mut interior = vec![true; model.mesh.nodes.len()];
        for name in model.mesh.node_sets.keys() {
            for &n in &model.mesh.node_sets[name] {
                interior[n] = false;
            }
        }
        for n in 0..model.mesh.nodes.len() {
            if !interior[n] {
                for i in 0..d {
                    bc.fix(n, i, exact(&model.mesh.nodes[n], i));
                }
            }
        }
        assert!(interior.iter().any(|b| *b), "{kind:?}: the patch needs interior nodes");
        let sol = model.solve_static(&Loads::default(), &bc).unwrap();
        let scale = 2e-3 * 4.0;
        for n in 0..model.mesh.nodes.len() {
            for i in 0..d {
                let err = (sol.u[n * d + i] - exact(&model.mesh.nodes[n], i)).abs();
                assert!(err < 1e-9 * scale, "{kind:?} node {n} comp {i}: error {err:e}");
            }
        }
    }
}

/// Exact Lame solution of a thick cylinder with internal pressure, `eps_zz = 0`.
struct Lame {
    a: f64,
    b: f64,
    p: f64,
}

impl Lame {
    fn k(&self) -> f64 {
        self.p * self.a * self.a / (self.b * self.b - self.a * self.a)
    }

    fn ur(&self, r: f64) -> f64 {
        (1.0 + NU) / E * self.k() * ((1.0 - 2.0 * NU) * r + self.b * self.b / r)
    }

    fn hoop(&self, r: f64) -> f64 {
        self.k() * (1.0 + self.b * self.b / (r * r))
    }

    fn radial(&self, r: f64) -> f64 {
        self.k() * (1.0 - self.b * self.b / (r * r))
    }
}

/// Quarter annulus map: `u` radial, `v` angle in `[0, pi/2]`, `w` axial.
fn annulus(a: f64, b: f64, h: f64) -> impl Fn([f64; 3]) -> [f64; 3] {
    move |p| {
        let (r, th) = (a + (b - a) * p[0], 0.5 * PI * p[1]);
        [r * th.cos(), r * th.sin(), h * p[2]]
    }
}

/// Symmetry constraints of the quarter model: `u_y = 0` on the `x` axis (`v0`), `u_x = 0` on the
/// `y` axis (`v1`).
fn quarter_symmetry(model: &Model, bc: &mut Dirichlet) {
    for &n in model.mesh.node_set("v0").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    for &n in model.mesh.node_set("v1").unwrap() {
        bc.fix(n, 0, 0.0);
    }
}

fn lame_check(mesh: Mesh, lame: &Lame, d: usize, ez_fixed: bool, tol_u: f64, tol_s: f64, label: &str) {
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    quarter_symmetry(&model, &mut bc);
    if ez_fixed {
        for name in ["w0", "w1"] {
            for &n in model.mesh.node_set(name).unwrap() {
                bc.fix(n, 2, 0.0);
            }
        }
    }
    let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(lame.p))).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let nodal = model.nodal_stresses(&sol.u, 0.0).unwrap();
    let (mut eu, mut eh) = (0.0f64, 0.0f64);
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        let r = x[0].hypot(x[1]);
        let ur = sol.u[n * d] * x[0] / r + sol.u[n * d + 1] * x[1] / r;
        eu = eu.max((ur - lame.ur(r)).abs() / lame.ur(lame.b));
        // Hoop stress from the Cartesian components.
        let (c, s) = (x[0] / r, x[1] / r);
        let st = nodal[n];
        let hoop = st[0] * s * s + st[1] * c * c - 2.0 * st[3] * s * c;
        // Skip the centre-of-element averaging noise at the very ends only through the tolerance.
        eh = eh.max((hoop - lame.hoop(r)).abs() / lame.hoop(lame.a));
    }
    eprintln!("{label}: max u_r error {eu:.2e}, max hoop error {eh:.2e}");
    assert!(eu < tol_u, "{label}: displacement error {eu:e}");
    assert!(eh < tol_s, "{label}: hoop stress error {eh:e}");
}

#[test]
fn lame_cylinder_plane_strain_quadratic_elements() {
    let lame = Lame { a: 1.0, b: 2.0, p: 1000.0 };
    for (kind, n, tu, ts) in [(ElementKind::Quad9, 6, 2e-4, 2e-2), (ElementKind::Quad8, 6, 2e-4, 2e-2), (ElementKind::Tri6, 6, 1e-3, 4e-2), (ElementKind::Quad4, 16, 3e-3, 8e-2), (ElementKind::Tri3, 16, 3e-2, 2.5e-1)] {
        let mesh = grid(Physics::PlaneStrain { thickness: 1.0 }, kind, mat(), [n, n, 1], &annulus(1.0, 2.0, 1.0)).unwrap();
        lame_check(mesh, &lame, 2, false, tu, ts, &format!("plane strain {kind:?} {n}x{n}"));
    }
}

#[test]
fn lame_cylinder_converges_at_the_theoretical_rate() {
    // Quadratic elements: nodal displacement error falls at least ~ h^3 (the L2 rate) as the mesh
    // is refined; linear ones at h^2.
    let lame = Lame { a: 1.0, b: 2.0, p: 1000.0 };
    let err = |kind: ElementKind, n: usize| -> f64 {
        let mesh = grid(Physics::PlaneStrain { thickness: 1.0 }, kind, mat(), [n, n, 1], &annulus(1.0, 2.0, 1.0)).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        quarter_symmetry(&model, &mut bc);
        let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(lame.p))).collect(), ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        model.mesh.nodes.iter().enumerate().map(|(i, x)| {
            let r = x[0].hypot(x[1]);
            ((sol.u[2 * i] * x[0] + sol.u[2 * i + 1] * x[1]) / r - lame.ur(r)).abs()
        }).fold(0.0, f64::max)
    };
    for (kind, ratio) in [(ElementKind::Quad9, 6.0), (ElementKind::Quad8, 6.0), (ElementKind::Tri6, 6.0), (ElementKind::Quad4, 3.0), (ElementKind::Tri3, 2.0)] {
        let (e1, e2, e3) = (err(kind, 2), err(kind, 4), err(kind, 8));
        eprintln!("{kind:?} nodal u_r error 2/4/8 per side: {e1:.2e} {e2:.2e} {e3:.2e}");
        assert!(e2 < e1 / ratio && e3 < e2 / ratio, "{kind:?}: convergence too slow: {e1:e} {e2:e} {e3:e}");
    }
}

#[test]
fn lame_cylinder_axisymmetric_matches_plane_strain_solution() {
    // Axisymmetric (r, z) strip with u_z = 0 on both ends has eps_zz = 0: the plane-strain solution.
    let lame = Lame { a: 1.0, b: 2.0, p: 1000.0 };
    for (kind, n, tu, ts) in [(ElementKind::Quad9, 8, 2e-4, 2e-2), (ElementKind::Quad8, 8, 2e-4, 2e-2), (ElementKind::Tri6, 8, 5e-4, 4e-2), (ElementKind::Quad4, 16, 3e-3, 8e-2)] {
        // Strip map: u radial, v axial.
        let map = |p: [f64; 3]| [1.0 + p[0], 0.5 * p[1], 0.0];
        let mesh = grid(Physics::Axisymmetric, kind, mat(), [n, 2, 1], &map).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for name in ["v0", "v1"] {
            for &nd in model.mesh.node_set(name).unwrap() {
                bc.fix(nd, 1, 0.0);
            }
        }
        // Inner face (u0): pressure acts radially outward on the body; total force over 360 deg is
        // handled by the 2 pi r weight in the face integral.
        let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(lame.p))).collect(), ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let nodal = model.nodal_stresses(&sol.u, 0.0).unwrap();
        let (mut eu, mut eh) = (0.0f64, 0.0f64);
        for (nd, x) in model.mesh.nodes.iter().enumerate() {
            eu = eu.max((sol.u[nd * 2] - lame.ur(x[0])).abs() / lame.ur(lame.b));
            eh = eh.max((nodal[nd][2] - lame.hoop(x[0])).abs() / lame.hoop(lame.a));
        }
        eprintln!("axisymmetric {kind:?} {n}: u_r error {eu:.2e}, hoop error {eh:.2e}");
        assert!(eu < tu, "{kind:?}: {eu:e}");
        assert!(eh < ts, "{kind:?}: {eh:e}");
        let _ = lame.radial(1.5);
    }
}

#[test]
fn lame_cylinder_3d_extruded() {
    let lame = Lame { a: 1.0, b: 2.0, p: 1000.0 };
    for (kind, tu, ts) in [(ElementKind::Hex27, 3e-4, 2e-2), (ElementKind::Hex20, 3e-4, 2e-2), (ElementKind::Hex8, 6e-3, 1.2e-1)] {
        let n = if kind == ElementKind::Hex8 { 12 } else { 5 };
        let mesh = grid(Physics::Solid, kind, mat(), [n, n, 1], &annulus(1.0, 2.0, 0.4)).unwrap();
        lame_check(mesh, &lame, 3, true, tu, ts, &format!("3D {kind:?} {n}x{n}x1"));
    }
}

#[test]
fn lame_cylinder_with_tetrahedra_driven_by_the_exact_boundary_displacement() {
    // Both cylinder surfaces carry the exact Lame displacement; the interior must follow it.
    let lame = Lame { a: 1.0, b: 2.0, p: 1000.0 };
    for (kind, n, tol) in [(ElementKind::Tet10, 4, 3e-3), (ElementKind::Tet4, 10, 1e-2)] {
        let mesh = grid(Physics::Solid, kind, mat(), [n, n, 1], &annulus(1.0, 2.0, 0.4)).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        quarter_symmetry(&model, &mut bc);
        for name in ["w0", "w1"] {
            for &nd in model.mesh.node_set(name).unwrap() {
                bc.fix(nd, 2, 0.0);
            }
        }
        for name in ["u0", "u1"] {
            for &nd in model.mesh.node_set(name).unwrap() {
                let x = model.mesh.nodes[nd];
                let r = x[0].hypot(x[1]);
                bc.fix(nd, 0, lame.ur(r) * x[0] / r);
                bc.fix(nd, 1, lame.ur(r) * x[1] / r);
            }
        }
        let sol = model.solve_static(&Loads::default(), &bc).unwrap();
        let mut eu = 0.0f64;
        for (nd, x) in model.mesh.nodes.iter().enumerate() {
            let r = x[0].hypot(x[1]);
            let ur = (sol.u[nd * 3] * x[0] + sol.u[nd * 3 + 1] * x[1]) / r;
            eu = eu.max((ur - lame.ur(r)).abs() / lame.ur(lame.b));
        }
        eprintln!("{kind:?} {n}x{n}x1 tets: max u_r error {eu:.2e}");
        assert!(eu < tol, "{kind:?}: {eu:e}");
    }
}

#[test]
fn thermal_expansion_fully_constrained_gives_the_exact_stress() {
    // Cube with symmetry planes on three faces and the opposite faces also held: eps = 0, so
    // sigma = -E alpha dT / (1 - 2 nu) in all three directions.
    let alpha = 12e-6;
    let dt = 100.0;
    let m = Elastic::new(E, NU).with_alpha(alpha);
    for kind in [ElementKind::Hex8, ElementKind::Hex20, ElementKind::Hex27, ElementKind::Tet4, ElementKind::Tet10] {
        let mesh = grid(Physics::Solid, kind, m, [2, 2, 2], &|p| p).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for (axis, names) in [["u0", "u1"], ["v0", "v1"], ["w0", "w1"]].iter().enumerate() {
            for name in names {
                for &n in model.mesh.node_set(name).unwrap() {
                    bc.fix(n, axis, 0.0);
                }
            }
        }
        let loads = Loads { delta_t: dt, ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let want = -E * alpha * dt / (1.0 - 2.0 * NU);
        let gs = model.gauss_stresses(&sol.u, dt).unwrap();
        for (_, s) in &gs[0] {
            for c in 0..3 {
                assert!((s[c] - want).abs() < 1e-9 * want.abs(), "{kind:?} sigma {c}: {} vs {want}", s[c]);
            }
        }
    }
}

#[test]
fn free_thermal_expansion_is_stress_free() {
    let alpha = 12e-6;
    let dt = 80.0;
    let m = Elastic::new(E, NU).with_alpha(alpha);
    // In plane strain the out-of-plane constraint makes the in-plane free expansion (1 + nu) alpha dT.
    for (kind, physics, factor) in [
        (ElementKind::Quad9, Physics::PlaneStress { thickness: 1.0 }, 1.0),
        (ElementKind::Quad9, Physics::PlaneStrain { thickness: 1.0 }, 1.0 + NU),
        (ElementKind::Tri6, Physics::PlaneStrain { thickness: 1.0 }, 1.0 + NU),
        (ElementKind::Hex20, Physics::Solid, 1.0),
        (ElementKind::Tet10, Physics::Solid, 1.0),
    ] {
        let d = kind.dim();
        let mesh = grid(physics, kind, m, [2, 2, 2], &|p| p).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        // Just enough supports: symmetry planes through the origin.
        for (axis, name) in ["u0", "v0", "w0"].iter().enumerate().take(d) {
            for &n in model.mesh.node_set(name).unwrap() {
                bc.fix(n, axis, 0.0);
            }
        }
        let sol = model.solve_static(&Loads { delta_t: dt, ..Loads::default() }, &bc).unwrap();
        let gs = model.gauss_stresses(&sol.u, dt).unwrap();
        let scale = E * alpha * dt;
        for (_, s) in &gs[0] {
            for c in 0..d.max(2) {
                // sigma_zz of plane strain is not zero; the in-plane components and all solid ones are.
                let skip = matches!(physics, Physics::PlaneStrain { .. }) && c == 2;
                if !skip {
                    assert!(s[c].abs() < 1e-9 * scale, "{kind:?} {physics:?} stress {c}: {}", s[c]);
                }
            }
        }
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            for i in 0..d {
                let want = factor * alpha * dt * x[i];
                assert!((sol.u[n * d + i] - want).abs() < 1e-9 * alpha * dt, "{kind:?} {physics:?} u[{n}][{i}]: {} vs {want}", sol.u[n * d + i]);
            }
        }
    }
}

#[test]
fn reactions_balance_the_applied_load_and_body_force() {
    // Bar hanging under its own weight plus an end load: reaction = total weight + end load.
    let (len, area_w, area_h) = (4.0, 0.5, 0.5);
    let rho_g = 0.1; // body force per unit volume (lbf/in^3)
    let map = move |p: [f64; 3]| [area_w * p[0], area_h * p[1], len * p[2]];
    for kind in [ElementKind::Hex8, ElementKind::Hex20, ElementKind::Hex27] {
        let mesh = grid(Physics::Solid, kind, mat(), [1, 1, 4], &map).unwrap();
        let model = Model::new(mesh).unwrap();
        // Symmetry-plane supports: the bar stretches freely (no restrained Poisson contraction),
        // so the 1D formula is exact.
        let mut bc = model.dirichlet();
        for (axis, name) in ["u0", "v0", "w0"].iter().enumerate() {
            for &n in model.mesh.node_set(name).unwrap() {
                bc.fix(n, axis, 0.0);
            }
        }
        let end = 250.0;
        // End load as a uniform traction on the free face (consistent nodal forces are not equal
        // for serendipity / quadratic faces, so lumping it equally would be a different load).
        let top = model.mesh.surfaces["w1"].clone();
        let loads = Loads { body: Some([0.0, 0.0, -rho_g]), faces: top.into_iter().map(|f| (f, SurfaceLoad::Traction([0.0, 0.0, end / (area_w * area_h)]))).collect(), ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let rz: f64 = (0..model.mesh.nodes.len()).map(|n| sol.reactions[n * 3 + 2]).sum();
        // Only the w0 face reacts in z (the other supports carry no axial load).
        let weight = rho_g * area_w * area_h * len;
        // Reaction K u - f at the support: it carries the whole load downward-negated.
        assert!((rz + (end - weight)).abs() < 1e-8 * (end + weight), "{kind:?}: reaction sum {rz} vs {}", -(end - weight));
        // Free-end deflection is exact in 1D: PL/EA minus the self-weight shortening.
        let tip = model.mesh.node_set("w1").unwrap()[0];
        let uz = sol.u[tip * 3 + 2];
        // Gravity acts toward the support (-z), shortening the bar.
        let exact = end * len / (E * area_w * area_h) - rho_g * len * len / (2.0 * E);
        assert!((uz / exact - 1.0).abs() < 1e-5, "{kind:?}: tip {uz} vs {exact}");
    }
}

#[test]
fn a_block_body_force_loads_only_its_own_block() {
    // Two separate unit squares (blocks 0 and 1); gravity-like force on block 1 only: the total force is
    // b * area * thickness there and nothing lands on block 0's nodes.
    let m0 = generate::grid(Physics::PlaneStress { thickness: 2.0 }, ElementKind::Quad4, Elastic::new(1.0, 0.3), [2, 2, 1], &|p| p).unwrap();
    let mut mesh = m0.clone();
    let off = mesh.append(&m0, "b/").unwrap();
    let f = loads::assemble(&mesh, &Loads { block_body: vec![(1, [0.0, -3.0, 0.0])], ..Default::default() }).unwrap();
    let on_block0: f64 = (0..off).map(|n| f[n * 2 + 1]).sum();
    let on_block1: f64 = (off..mesh.nodes.len()).map(|n| f[n * 2 + 1]).sum();
    assert!(on_block0.abs() < 1e-14, "block 0 got {on_block0}");
    assert!((on_block1 + 3.0 * 1.0 * 2.0).abs() < 1e-12, "block 1 got {on_block1}");
}

/// A support set that leaves a rigid-body motion free used to factorise anyway (rounding leaves
/// tiny positive pivots) and return the true answer plus an arbitrary rigid motion.
#[test]
fn unconstrained_rigid_body_motion_is_an_error() {
    let id = |p: [f64; 3]| p;
    for (kind, name) in [(ElementKind::Quad8, "2D"), (ElementKind::Hex8, "3D")] {
        let d = kind.dim();
        let div = if d == 2 { [3, 2, 1] } else { [2, 2, 2] };
        let mesh = grid(physics_for(kind), kind, mat(), div, &id).unwrap();
        let model = Model::new(mesh).unwrap();
        let n = model.mesh.nodes.len();
        let loads = Loads { nodal: vec![(n - 1, [1.0, 0.0, 0.0])], ..Loads::default() };
        let corner = (0..n).min_by(|&a, &b| model.mesh.nodes[a].iter().sum::<f64>().total_cmp(&model.mesh.nodes[b].iter().sum::<f64>())).unwrap();
        // Nothing fixed, one node fixed (rotations free), one 2D roller line (translation + rotation free).
        let mut one = model.dirichlet();
        one.fix_node(corner);
        for bc in [model.dirichlet(), one] {
            let e = model.solve_static(&loads, &bc).err().unwrap_or_else(|| panic!("{name}: a free rigid motion went unnoticed"));
            assert!(e.contains("not fully constrained"), "{name}: {e}");
        }
    }
    // Fully constrained (two nodes in 2D fix all three motions) is accepted.
    let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, mat(), [3, 2, 1], &id).unwrap();
    let model = Model::new(mesh).unwrap();
    let n = model.mesh.nodes.len();
    let mut bc = model.dirichlet();
    bc.fix_node(0);
    bc.fix(n - 1, 1, 0.0);
    let loads = Loads { nodal: vec![(n / 2, [1.0, 0.0, 0.0])], ..Loads::default() };
    model.solve_static(&loads, &bc).expect("pinned at one node and rolled at another is statically determinate");
}

#[test]
fn several_load_cases_on_one_factorisation_match_separate_solves() {
    let mesh = grid(Physics::PlaneStress { thickness: 0.5 }, ElementKind::Quad8, mat(), [4, 3, 1], &|p| [3.0 * p[0], 2.0 * p[1], p[2]]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(n);
    }
    let n = model.mesh.nodes.len();
    let cases: Vec<Loads> = vec![
        Loads { nodal: vec![(n - 1, [100.0, 0.0, 0.0])], ..Loads::default() },
        Loads { nodal: vec![(n - 1, [0.0, -50.0, 0.0])], ..Loads::default() },
        Loads { body: Some([0.0, -1.0, 0.0]), ..Loads::default() },
    ];
    let many = model.solve_static_many(&cases, &bc).unwrap();
    assert_eq!(many.len(), 3);
    for (loads, m) in cases.iter().zip(&many) {
        let one = model.solve_static_with(loads, &bc, SolveMethod::Direct).unwrap();
        let scale = one.u.iter().fold(0.0f64, |a, v| a.max(v.abs()));
        for (a, b) in one.u.iter().zip(&m.u) {
            assert!((a - b).abs() < 1e-12 * scale, "{a} vs {b}");
        }
    }
    assert!(model.solve_static_many(&cases, &model.dirichlet()).is_err(), "an unsupported model is refused here too");
}

/// A tiny lever arm is still a constraint: a micro hole fixed in a large body holds the rotation only
/// through `r / span`, which the check must not mistake for a free mode (found while normalising it).
#[test]
fn a_small_fixed_hole_in_a_large_body_is_accepted_but_one_pinned_node_is_not() {
    use fea_core::delaunay::MeshOptions;
    use fea_core::geometry::{Loop, Region};
    use fea_core::mesh2d::mesh_region;
    let region = Region::new(Loop::rectangle(0.0, 0.0, 1000.0, 1000.0).unwrap(), vec![Loop::circle([500.0, 500.0], 0.01, "hole").unwrap()], mat()).unwrap();
    let size = |x: [f64; 2]| (0.003 + 0.4 * ((x[0] - 500.0).hypot(x[1] - 500.0) - 0.01).max(0.0)).min(250.0);
    let mesh = mesh_region(&region, Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad8, &size, MeshOptions::default()).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("hole").unwrap() {
        bc.fix_node(n);
    }
    model.check_constrained(&bc).expect("a fixed micro hole holds all three rigid motions");
    let mut pinned = model.dirichlet();
    pinned.fix_node(model.mesh.node_set("hole").unwrap()[0]);
    assert!(model.check_constrained(&pinned).unwrap_err().contains("2 of 3"), "one node leaves the rotation free");
}
