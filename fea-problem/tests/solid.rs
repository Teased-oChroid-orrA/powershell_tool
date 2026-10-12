//! `Geometry::Solid`: STEP text plus a cached surface mesh, solved without the OpenCascade kernel.
use fea_problem::solid::{text_hash, SolidCache};
use fea_problem::{solve, templates, Analysis, Geometry, Load, MeshSpec, Problem, Support};
use solid_kernel::{SurfaceMesh, V3};

const STEP: &str = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";

/// A [0,w] x [0,h] x [0,d] box, faces 0..5 = x-, x+, y-, y+, z-, z+ (counter-clockwise seen from outside).
fn box_mesh(w: f64, h: f64, d: f64) -> SurfaceMesh {
    let p = |x: f64, y: f64, z: f64| [x * w, y * h, z * d];
    let mut m = SurfaceMesh::default();
    let faces: [([V3; 4], usize); 6] = [
        ([p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0), p(0.0, 1.0, 1.0), p(0.0, 1.0, 0.0)], 0),
        ([p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 1.0, 1.0), p(1.0, 0.0, 1.0)], 1),
        ([p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(1.0, 0.0, 1.0), p(0.0, 0.0, 1.0)], 2),
        ([p(0.0, 1.0, 0.0), p(0.0, 1.0, 1.0), p(1.0, 1.0, 1.0), p(1.0, 1.0, 0.0)], 3),
        ([p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 0.0, 0.0)], 4),
        ([p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0), p(1.0, 1.0, 1.0), p(0.0, 1.0, 1.0)], 5),
    ];
    for (q, f) in faces {
        m.push_triangle(q[0], q[1], q[2], f);
        m.push_triangle(q[0], q[2], q[3], f);
    }
    m
}

fn bar() -> Problem {
    let mut p = templates::blank();
    p.name = "Solid bar".into();
    p.set_solid(STEP, SolidCache::new(STEP, box_mesh(1.0, 1.0, 3.0), 0.01));
    p.mesh = MeshSpec { size: 0.5, ..MeshSpec::default() };
    // Held at z = 0 (face5), pulled at z = 3 (face6).
    p.supports = vec![Support::fixed("face5")];
    p.loads = vec![Load::Traction { edge: "face6".into(), tx: 0.0, ty: 0.0, tz: 50.0 }];
    p
}

#[test]
fn a_stored_solid_is_meshed_solved_and_balances_without_the_kernel() {
    let p = bar();
    assert_eq!(p.edge_names(), (1..=6).map(|i| format!("face{i}")).collect::<Vec<_>>());
    let s = solve(&p, None).unwrap();
    // sigma L / E for the free bar; the fixed end restrains the Poisson contraction a little.
    let free = 50.0 * 3.0 / p.material.e;
    let uz = s.summary.max_displacement.value;
    assert!(uz > 0.9 * free && uz < 1.02 * free, "{uz} vs {free}");
    assert!((s.summary.reaction[2] + 50.0).abs() < 0.5, "the base carries the 50 pulling on a unit area: {:?}", s.summary.reaction);
    assert!(s.summary.equilibrium_error < 1e-3, "{}", s.summary.equilibrium_error);
    assert!(fea_problem::report::report(&s).contains("B-rep solid (STEP), 6 faces"));
}

#[test]
fn the_cache_is_tied_to_the_step_text() {
    let mut p = bar();
    assert!(p.validate_geometry().is_ok());
    let Geometry::Solid { step, .. } = &mut p.geometry else { unreachable!() };
    step.push_str("/* edited */");
    let err = p.validate_geometry().unwrap_err();
    assert!(err.contains("out of date"), "{err}");
    assert!(solve(&p, None).is_err(), "a stale cache is never solved");
    // The hash is stable: this value is stored in files.
    assert_eq!(text_hash(""), "cbf29ce484222325");
    assert_ne!(text_hash("a"), text_hash("b"));
}

#[test]
fn a_solid_without_a_cache_or_in_a_plane_analysis_is_refused_with_the_reason() {
    let mut p = bar();
    p.geometry = Geometry::Solid { step: STEP.into(), cache: None };
    assert!(p.validate_geometry().unwrap_err().contains("not been tessellated"));
    let mut q = bar();
    q.analysis = Analysis::PlaneStress;
    assert!(q.validate_geometry().unwrap_err().contains("3D solid"));
    #[cfg(not(feature = "occt"))]
    assert!(Geometry::solid_from_step(STEP, 0.01).unwrap_err().contains("OpenCascade"));
}

#[test]
fn a_solid_problem_round_trips_through_json_and_old_files_still_load() {
    let p = bar();
    let back = Problem::from_json(&p.to_json().unwrap()).unwrap();
    assert_eq!(back, p);
    assert!(solve(&back, None).is_ok());
    // A file written before the cache existed (no `cache` key) loads and says what is missing.
    let mut v: serde_json::Value = serde_json::from_str(&p.to_json().unwrap()).unwrap();
    v["geometry"].as_object_mut().unwrap().remove("cache");
    let old = Problem::from_json(&v.to_string()).unwrap();
    assert!(matches!(old.geometry, Geometry::Solid { cache: None, .. }));
    assert!(old.validate_geometry().is_err());
    // Sketch and imported geometry are unchanged by the new variant.
    for (_, t) in templates::templates() {
        assert_eq!(Problem::from_json(&t.to_json().unwrap()).unwrap(), t);
    }
}

/// STEP text from the kernel -> tessellation cache -> mesh -> solve, with a plate-with-hole whose faces are named.
#[cfg(feature = "occt")]
#[test]
fn a_kernel_solid_goes_from_step_text_to_a_solve() {
    use solid_kernel::occt::{Brep, Loop, Profile};
    let outer = Loop::Polygon(vec![[0.0, 0.0], [12.0, 0.0], [12.0, 4.0], [0.0, 4.0]]);
    let step = Brep::extrude(&Profile { outer, holes: vec![Loop::Circle { centre: [6.0, 2.0], radius: 0.5 }] }, 0.0, 0.25).unwrap().to_step_text().unwrap();
    let mut p = templates::blank();
    p.set_solid(&step, fea_problem::solid::tessellate_step(&step, 0.004).unwrap());
    assert_eq!(p.edge_names().len(), 7, "top, bottom, four sides, hole wall");
    p.mesh = MeshSpec { size: 0.5, element: fea_problem::ElementChoice::Tri6, ..MeshSpec::default() };
    // Face order is the kernel's; find the x- and x+ faces from the solid's own normals.
    let normals = fea_problem::solid::solid_surface(&p).unwrap().face_normals();
    let (right, left) = (normals.iter().position(|n| n[0] > 0.9).unwrap(), normals.iter().position(|n| n[0] < -0.9).unwrap());
    p.supports = vec![Support::roller(&format!("face{}", left + 1), 0), Support::Point { x: 0.0, y: 0.0, z: 0.0, ux: None, uy: Some(0.0), uz: Some(0.0) }, Support::Point { x: 0.0, y: 4.0, z: 0.0, ux: None, uy: None, uz: Some(0.0) }];
    p.loads = vec![Load::Traction { edge: format!("face{}", right + 1), tx: 100.0, ty: 0.0, tz: 0.0 }];
    let s = solve(&p, None).unwrap();
    assert!(s.summary.equilibrium_error < 1e-3, "{}", s.summary.equilibrium_error);
    assert!(s.summary.max_von_mises.value > 100.0, "a hole concentrates the stress");
    // Editing the text invalidates the cache.
    let Geometry::Solid { step, .. } = &mut p.geometry else { unreachable!() };
    step.push(' ');
    assert!(p.validate_geometry().is_err());
}

#[cfg(feature = "occt")]
mod editing {
    use super::*;
    use fea_problem::solid::{flat_face, solid_surface, SolidOp};
    use solid_kernel::occt::Brep;

    fn cache_of(p: &Problem) -> &SolidCache {
        match &p.geometry {
            Geometry::Solid { cache: Some(c), .. } => c,
            _ => panic!("not a solid"),
        }
    }
    fn cube() -> Problem {
        let step = Brep::box_from_corners([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]).to_step_text().unwrap();
        Problem::from_step("Cube", &step).unwrap()
    }
    fn face_with_normal(p: &Problem, n: [f64; 3]) -> usize {
        let c = cache_of(p);
        (1..=c.face_names().len()).find(|f| flat_face(c, *f).map(|(_, m)| (m[0] - n[0]).abs() + (m[1] - n[1]).abs() + (m[2] - n[2]).abs() < 1e-6).unwrap_or(false)).expect("a face with that normal")
    }

    #[test]
    fn a_step_cube_becomes_a_problem_and_gets_a_hole_a_boss_and_fillets_with_the_exact_volume() {
        let mut p = cube();
        assert_eq!(p.edge_names().len(), 6);
        assert!((solid_surface(&p).unwrap().volume() - 1000.0).abs() < 1e-6);
        let top = face_with_normal(&p, [0.0, 0.0, 1.0]);
        let (c, _) = flat_face(cache_of(&p), top).unwrap();
        assert!((c[2] - 10.0).abs() < 1e-9 && (c[0] - 5.0).abs() < 1e-9, "{c:?}");
        // A hole of diameter 2 through the cube: 1000 - pi * 1^2 * 10.
        p.apply_solid_op(&SolidOp::Hole { face: top, diameter: 2.0 }).unwrap();
        assert_eq!(p.edge_names().len(), 7);
        let want = 1000.0 - std::f64::consts::PI * 10.0;
        let v = solid_surface(&p).unwrap().volume();
        assert!((v - want).abs() / want < 2e-3, "{v} vs {want}");
        // A boss on the bottom (whole face): + pi * 1.5^2 * 3.
        let bottom = face_with_normal(&p, [0.0, 0.0, -1.0]);
        p.apply_solid_op(&SolidOp::Boss { face: bottom, diameter: 3.0, height: 3.0 }).unwrap();
        let grown = solid_surface(&p).unwrap().volume() - v;
        let boss = std::f64::consts::PI * 1.5 * 1.5 * 3.0;
        assert!((grown - boss).abs() / boss < 5e-3, "{grown} vs {boss}");
        // Fillet every edge of a plain cube.
        let mut q = cube();
        q.apply_solid_op(&SolidOp::Fillet { radius: 0.5 }).unwrap();
        assert_eq!(q.edge_names().len(), 26);
        assert!(solid_surface(&q).unwrap().volume() < 1000.0);
        // Refusals say why.
        assert!(q.apply_solid_op(&SolidOp::Hole { face: 99, diameter: 1.0 }).unwrap_err().contains("no face99"));
        let curved = (1..=26).find(|f| flat_face(cache_of(&q), *f).is_err()).unwrap();
        assert!(q.apply_solid_op(&SolidOp::Hole { face: curved, diameter: 1.0 }).unwrap_err().contains("flat"));
        assert!(cube().apply_solid_op(&SolidOp::Fillet { radius: -1.0 }).unwrap_err().contains("positive"));
    }
}
