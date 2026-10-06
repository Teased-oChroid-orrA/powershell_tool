//! Extrusion and revolution against the 2D solutions they must reproduce.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::generate::grid;
use fea_core::sweep::{extrude, revolve};
use fea_core::*;

const E: f64 = 10.0e6;
const NU: f64 = 0.3;

#[test]
fn extruded_plane_strain_ring_reproduces_the_2d_solution_exactly() {
    let (a, b, p, h) = (1.0f64, 2.0f64, 1000.0, 1.0);
    let mesh2 = grid(Physics::PlaneStrain { thickness: h }, ElementKind::Quad9, Elastic::new(E, NU), [4, 6, 1], &move |q| {
        let (r, th) = (a + (b - a) * q[0], 0.5 * std::f64::consts::PI * q[1]);
        [r * th.cos(), r * th.sin(), 0.0]
    })
    .unwrap();
    let solve2 = |mesh: &Mesh| -> Vec<f64> {
        let model = Model::new(mesh.clone()).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("v0").unwrap() {
            bc.fix(n, 1, 0.0);
        }
        for &n in model.mesh.node_set("v1").unwrap() {
            bc.fix(n, 0, 0.0);
        }
        let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(p))).collect(), ..Loads::default() };
        model.solve_static(&loads, &bc).unwrap().u
    };
    let u2 = solve2(&mesh2);
    // The same ring, 3D, two layers of unit total height, ends held at u_z = 0 (plane strain).
    let mesh3 = extrude(&mesh2, &[0.0, 0.4, 1.0]).unwrap();
    let model = Model::new(mesh3).unwrap();
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("v0").unwrap() {
        bc.fix(n, 1, 0.0);
    }
    for &n in model.mesh.node_set("v1").unwrap() {
        bc.fix(n, 0, 0.0);
    }
    for name in ["start", "end"] {
        for &n in model.mesh.node_set(name).unwrap() {
            bc.fix(n, 2, 0.0);
        }
    }
    let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(p))).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    // Match 3D nodes to 2D nodes by (x, y).
    let scale = u2.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let mut worst = 0.0f64;
    for (n3, x3) in model.mesh.nodes.iter().enumerate() {
        let n2 = mesh2.nodes.iter().position(|x2| (x2[0] - x3[0]).abs() < 1e-12 && (x2[1] - x3[1]).abs() < 1e-12).expect("matching 2D node");
        worst = worst.max((sol.u[n3 * 3] - u2[n2 * 2]).abs()).max((sol.u[n3 * 3 + 1] - u2[n2 * 2 + 1]).abs()).max(sol.u[n3 * 3 + 2].abs());
    }
    eprintln!("extruded Hex27 vs plane strain Quad9: max difference {:.2e} of {scale:.2e}", worst);
    assert!(worst < 1e-9 * scale, "3D extrusion differs from the 2D solution by {worst:e}");
}

/// Revolved Lame cylinder sector against the axisymmetric solution of the same 2D mesh.
fn revolved_cylinder_error(layers: usize, sector: f64) -> f64 {
    let (a, b, p, hgt) = (1.0f64, 2.0f64, 1000.0, 0.6);
    let m2 = |physics| grid(physics, ElementKind::Quad9, Elastic::new(E, NU), [4, 1, 1], &move |q| [a + (b - a) * q[0], hgt * q[1], 0.0]).unwrap();
    // Axisymmetric reference (plane strain: u_z = 0 on both ends).
    let ax = m2(Physics::Axisymmetric);
    let model = Model::new(ax.clone()).unwrap();
    let mut bc = model.dirichlet();
    for name in ["v0", "v1"] {
        for &n in model.mesh.node_set(name).unwrap() {
            bc.fix(n, 1, 0.0);
        }
    }
    // Pressure on the inner radius (u0 edge), full 360 degrees.
    let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(p))).collect(), ..Loads::default() };
    let u_ax = model.solve_static(&loads, &bc).unwrap().u;
    // 3D sector by revolution: (r, z) -> (r cos phi, z, r sin phi).
    let angles: Vec<f64> = (0..=layers).map(|k| sector * k as f64 / layers as f64).collect();
    let m3 = revolve(&m2(Physics::PlaneStrain { thickness: 1.0 }), &angles).unwrap();
    let model = Model::new(m3).unwrap();
    let mut bc = model.dirichlet();
    for name in ["v0", "v1"] {
        for &n in model.mesh.node_set(name).unwrap() {
            bc.fix(n, 1, 0.0); // axial ends (the 2D v0/v1 edges are the z = const faces)
        }
    }
    // Symmetry plane phi = 0 (z' = 0): u_z' = 0.
    for &n in model.mesh.node_set("start").unwrap() {
        bc.fix(n, 2, 0.0);
    }
    // Symmetry plane phi = sector: no displacement along its normal n = (-sin, 0, cos): slave u_z' to u_x'.
    let (s, c) = sector.sin_cos();
    let mut cons = Constraints::new();
    for &n in model.mesh.node_set("end").unwrap() {
        let (ux, uz) = (n * 3, n * 3 + 2);
        if !bc.fixed[ux] && !bc.fixed[uz] {
            // -sin * u_x + cos * u_z = 0  =>  u_z = tan(sector) u_x
            cons.add_equation(uz, vec![(ux, s / c)], 0.0);
        }
    }
    let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(p))).collect(), ..Loads::default() };
    let sol = model.solve_static_constrained(&loads, &bc, &cons, &[]).unwrap();
    // Radial displacement against the axisymmetric field at the same (r, y).
    let scale = u_ax.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let mut worst = 0.0f64;
    for (n3, x3) in model.mesh.nodes.iter().enumerate() {
        let r = x3[0].hypot(x3[2]);
        let n2 = ax.nodes.iter().position(|x2| (x2[0] - r).abs() < 1e-12 && (x2[1] - x3[1]).abs() < 1e-12).expect("matching axisymmetric node");
        let ur = (sol.u[n3 * 3] * x3[0] + sol.u[n3 * 3 + 2] * x3[2]) / r;
        worst = worst.max((ur - u_ax[n2 * 2]).abs());
    }
    worst / scale
}

#[test]
fn revolved_sector_converges_to_the_axisymmetric_solution() {
    let coarse = revolved_cylinder_error(1, std::f64::consts::PI / 6.0);
    let fine = revolved_cylinder_error(1, std::f64::consts::PI / 12.0);
    let layered = revolved_cylinder_error(3, std::f64::consts::PI / 6.0);
    eprintln!("revolved sector u_r error vs axisymmetric: 30 deg x1 layer {coarse:.2e}, 15 deg x1 {fine:.2e}, 30 deg x3 layers {layered:.2e}");
    assert!(coarse < 5e-3, "30 degree sector: {coarse:e}");
    assert!(fine < coarse / 4.0, "halving the sector angle must cut the hoop-representation error: {coarse:e} -> {fine:e}");
    assert!(layered < coarse / 4.0, "more layers across the same sector: {coarse:e} -> {layered:e}");
}

#[test]
fn a_closed_revolution_shares_the_seam_nodes() {
    let m2 = grid(Physics::PlaneStrain { thickness: 1.0 }, ElementKind::Quad9, Elastic::new(E, NU), [2, 1, 1], &|q| [1.0 + q[0], q[1], 0.0]).unwrap();
    let layers = 6;
    let angles: Vec<f64> = (0..=layers).map(|k| std::f64::consts::TAU * k as f64 / layers as f64).collect();
    let m3 = revolve(&m2, &angles).unwrap();
    // 2D has 5 x 3 = 15 nodes; the ring has 2*layers distinct half-levels.
    assert_eq!(m3.nodes.len(), 15 * 2 * layers);
    assert_eq!(m3.node_sets["start"], m3.node_sets["end"]);
    assert!(m3.surfaces["start"].is_empty() && m3.surfaces["end"].is_empty());
    // Volume of the ring equals plane area x 2 pi x centroid radius (Pappus): 2D area 1, centroid r = 1.5.
    // Quadratic elements represent the circle to O(angle^4), so doubling the layers cuts the error ~16x.
    let volume = |m3: &Mesh| -> f64 {
        let mut vol = 0.0;
        let mut work = fea_core::kernel::Work::new();
        for blk in &m3.blocks {
            let t = blk.kind.table();
            for e in 0..blk.n_elems() {
                let xyz: Vec<[f64; 3]> = blk.elem(e).iter().map(|&n| m3.nodes[n]).collect();
                fea_core::kernel::geometry(blk.kind, m3.physics, &xyz, &mut work).unwrap();
                vol += (0..t.ngp).map(|g| work.wdet[g]).sum::<f64>();
            }
        }
        vol
    };
    let exact = 1.0 * std::f64::consts::TAU * 1.5;
    let e6 = (volume(&m3) / exact - 1.0).abs();
    let angles12: Vec<f64> = (0..=12).map(|k| std::f64::consts::TAU * k as f64 / 12.0).collect();
    let e12 = (volume(&revolve(&m2, &angles12).unwrap()) / exact - 1.0).abs();
    eprintln!("ring volume error vs Pappus: 6 layers {e6:.2e}, 12 layers {e12:.2e}");
    assert!(e6 < 5e-3 && e12 < e6 / 10.0, "{e6:e} -> {e12:e}");
}
