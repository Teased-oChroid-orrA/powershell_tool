//! OpenCascade modelling through to a finite-element solve. Needs the `occt` feature (compiles OCCT, about 25 minutes the
//! first time): `CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo test -p solid-kernel --features occt`.
#![cfg(feature = "occt")]
use fea_core::{Elastic, Loads, Model, SurfaceLoad};
use solid_kernel::{
    mesh_volume,
    occt::{Bool, Brep, Loop, Profile},
    VolumeParams,
};

fn plate_with_hole() -> Brep {
    let outer = Loop::Polygon(vec![[0.0, 0.0], [12.0, 0.0], [12.0, 4.0], [0.0, 4.0]]);
    Brep::extrude(&Profile { outer, holes: vec![Loop::Circle { centre: [6.0, 2.0], radius: 0.5 }] }, 0.0, 0.25).unwrap()
}

#[test]
fn an_extruded_profile_has_the_expected_faces_and_volume() {
    let b = plate_with_hole();
    assert_eq!(b.face_count(), 7, "top, bottom, four sides and the hole wall");
    let s = b.solid(0.005).unwrap();
    let exact = (48.0 - std::f64::consts::PI * 0.25) * 0.25;
    assert!((s.volume() - exact).abs() / exact < 5e-4, "{} vs {exact}", s.volume());
    assert_eq!(s.face_count(), 7, "every face is tagged in the tessellation");
    // The inside / outside of the solid, from the tessellation.
    assert!(s.signed_distance([1.0, 1.0, 0.125]) < 0.0 && s.signed_distance([6.0, 2.0, 0.125]) > 0.0, "the hole is outside");
}

#[test]
fn booleans_work_for_the_cases_that_broke_truck_and_a_cut_equals_the_extruded_hole() {
    let plate = Brep::box_from_corners([0.0, 0.0, 0.0], [12.0, 4.0, 0.25]);
    // 40 round-number hole positions.
    for k in 0..40 {
        let x = 1.0 + 0.25 * k as f64;
        let cut = plate.boolean(Bool::Cut, &Brep::cylinder([x, 2.0, -1.0], 0.5, [0.0, 0.0, 1.0], 2.0)).unwrap();
        assert_eq!(cut.face_count(), 7, "hole at x = {x}");
    }
    // A boss standing on the face (coplanar contact).
    let boss = plate.boolean(Bool::Union, &Brep::cylinder([3.0, 2.0, 0.25], 0.8, [0.0, 0.0, 1.0], 1.75)).unwrap();
    assert_eq!(boss.face_count(), 8);
    let s = boss.solid(0.005).unwrap();
    let exact = 12.0 * 4.0 * 0.25 + std::f64::consts::PI * 0.64 * 1.75;
    assert!((s.volume() - exact).abs() / exact < 1e-3, "{} vs {exact}", s.volume());
    // A pocket and the fillet of every edge.
    let pocket = plate.boolean(Bool::Cut, &Brep::box_from_corners([4.0, 1.0, 0.1], [6.0, 3.0, 1.0])).unwrap();
    assert_eq!(pocket.face_count(), 11);
    assert_eq!(plate.fillet_all(0.05).unwrap().face_count(), 26);
    // Nothing in common is an error, not an empty solid.
    assert!(plate.boolean(Bool::Intersect, &Brep::box_from_corners([20.0, 20.0, 20.0], [21.0, 21.0, 21.0])).is_err());
}

#[test]
fn step_files_round_trip_the_faces_and_the_volume() {
    let b = plate_with_hole();
    let path = std::env::temp_dir().join(format!("solid-kernel-{}.step", std::process::id()));
    b.write_step(&path).unwrap();
    let back = Brep::read_step(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(back.face_count(), b.face_count());
    let (v1, v2) = (b.solid(0.005).unwrap().volume(), back.solid(0.005).unwrap().volume());
    assert!((v1 - v2).abs() / v1 < 1e-6, "{v1} vs {v2}");
}

#[test]
fn a_model_from_the_kernel_is_meshed_and_solved_end_to_end() {
    // A 1 x 1 x 3 bar from an extruded polygon: held at one end, pulled at the other. The elongation is exactly sigma L / E
    // (patch test), through tessellation, distance queries, the volume mesher and the solver.
    let outer = Loop::Polygon(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    let bar = Brep::extrude(&Profile { outer, holes: vec![] }, 0.0, 3.0).unwrap();
    let solid = bar.solid(0.002).unwrap();
    let normals = solid.face_normals();
    let top = normals.iter().position(|n| n[2] > 0.9).expect("the +z face");
    let bottom = normals.iter().position(|n| n[2] < -0.9).expect("the -z face");
    let (e, nu, sigma) = (200e3, 0.3, 50.0);
    for quadratic in [false, true] {
        let (mesh, rep) = mesh_volume(&solid, Elastic::new(e, nu), VolumeParams { base: 0.5, boundary_size: 0.4, quadratic }, &|_| 1.0).unwrap();
        assert!((rep.volume - 3.0).abs() < 1e-6, "{rep:?}");
        let top_faces = mesh.surfaces.get(&format!("face{}", top + 1)).cloned().expect("the top face is named");
        let bottom_nodes: Vec<usize> = mesh.surfaces.get(&format!("face{}", bottom + 1)).unwrap().iter().flatten().copied().collect();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &n in &bottom_nodes {
            bc.fix(n, 2, 0.0);
        }
        // Remove the rigid motions in the plane: the corner at the origin and one neighbour.
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            if x[0].abs() < 1e-9 && x[2].abs() < 1e-9 {
                bc.fix(n, 0, 0.0);
            }
            if x[1].abs() < 1e-9 && x[2].abs() < 1e-9 {
                bc.fix(n, 1, 0.0);
            }
        }
        let mut loads = Loads::default();
        for f in top_faces {
            loads.faces.push((f, SurfaceLoad::Traction([0.0, 0.0, sigma])));
        }
        let sol = model.solve_static(&loads, &bc).unwrap();
        let expected = sigma * 3.0 / e;
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            if (x[2] - 3.0).abs() < 1e-9 {
                assert!((sol.u[n * 3 + 2] - expected).abs() < 1e-6 * expected, "quadratic {quadratic}: {} vs {expected}", sol.u[n * 3 + 2]);
            }
        }
    }
}

#[test]
fn a_plate_with_a_hole_from_the_kernel_gives_the_textbook_stress_concentration() {
    // 12 x 4 x 0.25 plate, hole of diameter 1 (d/W = 0.25), pulled by sigma at the ends. Peterson's finite-width plate:
    // Kt(net) = 3.00 - 3.14 (d/W) + 3.667 (d/W)^2 - 1.527 (d/W)^3 = 2.42 on the net-section stress sigma W / (W - d), so
    // the peak stress is about 3.23 sigma. The whole pipeline is in the answer: extrusion and boolean, tessellation,
    // distance queries, the volume mesh refined at the hole (a size field), Tet10, the solve and nodal stress recovery.
    let b = plate_with_hole();
    let solid = b.solid(0.004).unwrap();
    let normals = solid.face_normals();
    let (right, left) = (normals.iter().position(|n| n[0] > 0.9).expect("x+ face"), normals.iter().position(|n| n[0] < -0.9).expect("x- face"));
    let sigma = 100.0;
    let size = |x: [f64; 3]| {
        let d = ((x[0] - 6.0).powi(2) + (x[1] - 2.0).powi(2)).sqrt() - 0.5;
        (0.12 + 0.35 * d.max(0.0)).min(0.8)
    };
    let (mesh, rep) = mesh_volume(&solid, Elastic::new(200e3, 0.3), VolumeParams { base: 0.8, boundary_size: 0.3, quadratic: true }, &size).unwrap();
    eprintln!("{} tets, worst quality {:.3}", rep.tets, rep.worst_quality);
    let right_faces = mesh.surfaces.get(&format!("face{}", right + 1)).cloned().unwrap();
    let left_nodes: Vec<usize> = mesh.surfaces.get(&format!("face{}", left + 1)).unwrap().iter().flatten().copied().collect();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for &n in &left_nodes {
        bc.fix(n, 0, 0.0);
    }
    // One in-plane point fixed against rigid motion in y and z (the corner at the origin).
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if x[0].abs() < 1e-9 && x[1].abs() < 1e-9 {
            bc.fix(n, 1, 0.0);
            bc.fix(n, 2, 0.0);
        }
        if x[0].abs() < 1e-9 && (x[1] - 4.0).abs() < 1e-9 && x[2].abs() < 1e-9 {
            bc.fix(n, 2, 0.0);
        }
    }
    let mut loads = Loads::default();
    for f in right_faces {
        loads.faces.push((f, SurfaceLoad::Traction([sigma, 0.0, 0.0])));
    }
    let sol = model.solve_static(&loads, &bc).unwrap();
    let nodal = model.nodal_stresses(&sol.u, 0.0).unwrap();
    // The peak sigma_xx among the nodes on the hole wall at the top and bottom of the hole (x = 6, y = 2 +- 0.5).
    let peak = model.mesh.nodes.iter().enumerate().filter(|(_, x)| ((x[0] - 6.0).hypot(x[1] - 2.0) - 0.5).abs() < 0.05 && (x[0] - 6.0).abs() < 0.2).map(|(n, _)| nodal[n][0]).fold(0.0f64, f64::max);
    let expected = 2.42 * (4.0 / 3.0) * sigma;
    eprintln!("peak sigma_xx at the hole {peak:.1} vs Peterson {expected:.1} ({:.1}%)", 100.0 * (peak / expected - 1.0));
    assert!((peak / expected - 1.0).abs() < 0.1, "within 10% of the textbook value: {peak} vs {expected}");
}
