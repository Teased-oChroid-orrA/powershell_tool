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
        (1..=c.face_names().len()).find(|f| {
                flat_face(c, *f).map(|(_, m)| {
                        (m[0] - n[0]).abs() + (m[1] - n[1]).abs() + (m[2] - n[2]).abs() < 1e-6
                    }).unwrap_or(false)
            }).expect("a face with that normal")
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

/// Faces are matched by what they are, not by their number: a permuted copy maps back through the permutation, a face the
/// new solid lacks maps to nothing.
#[test]
fn remap_faces_follows_a_renumbering_and_reports_a_removed_face() {
    use fea_problem::solid::remap_faces;
    let old = SolidCache::new(STEP, box_mesh(1.0, 2.0, 3.0), 0.01);
    let perm = [3usize, 0, 5, 1, 2, 4]; // new index of old face k
    let mut m = box_mesh(1.0, 2.0, 3.0);
    m.face.iter_mut().for_each(|f| *f = perm[*f]);
    let renumbered = SolidCache::new(STEP, m, 0.01);
    assert_eq!(remap_faces(&old, &renumbered), perm.iter().map(|p| Some(*p)).collect::<Vec<_>>());
    // Drop face 2 (y-): its triangles vanish, and the survivors keep their identity.
    let mut m = box_mesh(1.0, 2.0, 3.0);
    let keep: Vec<usize> = (0..m.face.len()).filter(|t| m.face[*t] != 2).collect();
    let (tri, face): (Vec<_>, Vec<_>) = keep.iter().map(|t| (m.triangles[*t], m.face[*t])).unzip();
    m.triangles = tri;
    m.face = face.iter().map(|f| if *f > 2 { f - 1 } else { *f }).collect();
    let gone = remap_faces(&old, &SolidCache::new(STEP, m, 0.01));
    assert_eq!(gone, vec![Some(0), Some(1), None, Some(2), Some(3), Some(4)]);
    // A scaled box is the same faces moved too far to be the same surface.
    let far = remap_faces(&old, &SolidCache::new(STEP, box_mesh(1.0, 2.0, 30.0), 0.01));
    assert_eq!(far[4], Some(4), "z- stays put when only z+ moves");
    assert_eq!(far[5], None, "z+ moved out of its plane");
}

/// A support and a load on untouched faces keep working through a boss and a hole; the one on the face a hole pierces
/// follows that face; a name the edit cannot carry is renamed to something `diagnose` reports.
#[cfg(feature = "occt")]
mod naming {
    use super::*;
    use fea_problem::diagnose::{diagnose, FailureClass};
    use fea_problem::solid::{flat_face, SolidOp};
    use solid_kernel::occt::Brep;

    fn cube() -> Problem {
        let step = Brep::box_from_corners([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]).to_step_text().unwrap();
        Problem::from_step("Cube", &step).unwrap()
    }
    fn cache(p: &Problem) -> &SolidCache {
        match &p.geometry {
            Geometry::Solid { cache: Some(c), .. } => c,
            _ => panic!(),
        }
    }
    fn name_of(p: &Problem, n: [f64; 3]) -> String {
        let c = cache(p);
        let f = (1..=c.face_names().len()).find(|f| {
                flat_face(c, *f).map(|(_, m)| (0..3).all(|k| (m[k] - n[k]).abs() < 1e-6)).unwrap_or(false)
            }).unwrap();
        format!("face{f}")
    }
    fn centre(p: &Problem, name: &str) -> [f64; 3] {
        flat_face(cache(p), name[4..].parse().unwrap()).unwrap().0
    }
    fn support_face(p: &Problem) -> &str {
        match &p.supports[0] {
            Support::Edge { edge, .. } => edge,
            _ => panic!(),
        }
    }
    fn load_face(p: &Problem) -> &str {
        match &p.loads[0] {
            Load::Traction { edge, .. } => edge,
            _ => panic!(),
        }
    }

    fn held_and_pulled() -> Problem {
        let mut p = cube();
        let (bottom, top) = (name_of(&p, [0.0, 0.0, -1.0]), name_of(&p, [0.0, 0.0, 1.0]));
        p.supports = vec![Support::fixed(&bottom)];
        p.loads = vec![Load::Traction { edge: top, tx: 0.0, ty: 0.0, tz: 5.0 }];
        p
    }

    #[test]
    fn boundary_conditions_follow_their_faces_through_a_hole_a_boss_and_fillets() {
        for boss in [false, true] {
            let mut p = held_and_pulled();
            // The hole / boss goes in a side face, so both the held bottom and the loaded top are untouched by it.
            let face: usize = name_of(&p, [1.0, 0.0, 0.0])[4..].parse().unwrap();
            let op = if boss { SolidOp::Boss { face, diameter: 2.0, height: 2.0 } } else { SolidOp::Hole { face, diameter: 2.0 } };
            p.apply_solid_op(&op).unwrap();
            let (s, l) = (support_face(&p).to_string(), load_face(&p).to_string());
            assert_eq!(s, name_of(&p, [0.0, 0.0, -1.0]), "{op:?}: the held face is still the bottom");
            assert_eq!(l, name_of(&p, [0.0, 0.0, 1.0]), "{op:?}: the loaded face is still the top");
            assert!(diagnose(&p, None).iter().all(|d| d.class != FailureClass::UnknownSurface));
        }
        // A hole through the loaded face itself: the load follows the pierced face (centre unchanged).
        let mut p = held_and_pulled();
        let top: usize = load_face(&p)[4..].parse().unwrap();
        p.apply_solid_op(&SolidOp::Hole { face: top, diameter: 3.0 }).unwrap();
        let l = load_face(&p).to_string();
        assert_eq!(l, name_of(&p, [0.0, 0.0, 1.0]));
        assert!((centre(&p, &l)[2] - 10.0).abs() < 1e-9);
        assert_eq!(support_face(&p), name_of(&p, [0.0, 0.0, -1.0]), "a through hole also pierces the held face and it still matches");
        // Fillets trim every flat face's rim but keep their planes.
        let mut q = held_and_pulled();
        q.apply_solid_op(&SolidOp::Fillet { radius: 0.5 }).unwrap();
        assert_eq!(support_face(&q), name_of(&q, [0.0, 0.0, -1.0]));
        assert_eq!(load_face(&q), name_of(&q, [0.0, 0.0, 1.0]));
        solve(&q, None).expect("and the model still solves");
    }

    #[test]
    fn a_boss_nearly_as_wide_as_the_loaded_face_leaves_the_load_on_the_remaining_ring() {
        let mut p = held_and_pulled();
        let top: usize = load_face(&p)[4..].parse().unwrap();
        p.apply_solid_op(&SolidOp::Boss { face: top, diameter: 9.99, height: 2.0 }).unwrap();
        let l = load_face(&p).to_string();
        assert_eq!(l, name_of(&p, [0.0, 0.0, 1.0]), "the ring left of the face is still that face (flat, same plane, same centroid)");
        assert!(diagnose(&p, None).iter().all(|d| d.class != FailureClass::UnknownSurface));
    }
}

#[test]
fn step_surface_preview_exists_before_volume_meshing_and_preserves_every_face() {
    let p = bar();
    let mesh = fea_problem::solve::body_preview(&p).unwrap();
    assert!(
        mesh.blocks.is_empty(),
        "preview must not run the volume mesher"
    );
    assert_eq!(mesh.dim(), 3);
    assert_eq!(mesh.surfaces.len(), 6);
    assert_eq!(mesh.surfaces.values().map(Vec::len).sum::<usize>(), 12);
    assert!(mesh.nodes.iter().all(|p| p.iter().all(|x| x.is_finite())));
}

#[test]
fn converted_step_cache_keeps_physical_geometry_material_and_loads() {
    use fea_problem::UnitSystem;
    let p = bar();
    for target in [UnitSystem::MmMpa, UnitSystem::MeterPa] {
        let mut q = p.clone();
        q.convert_units(target).unwrap();
        let before = fea_problem::solve::body_preview(&p).unwrap();
        let after = fea_problem::solve::body_preview(&q).unwrap();
        for (a, b) in before.nodes.iter().zip(&after.nodes) {
            for d in 0..3 {
                assert!((a[d] * p.units.length_m() - b[d] * target.length_m()).abs() < 1e-12);
            }
        }
        assert!((q.modulus_pa() / p.modulus_pa() - 1.0).abs() < 1e-12);
        let solved = solve(&q, None).unwrap();
        let expected = 50.0 * 3.0 / p.material.e * p.units.length_m();
        let actual = solved.summary.max_displacement.value * target.length_m();
        assert!(
            (actual / expected - 1.0).abs() < 0.12,
            "{target:?}: {actual} vs {expected}"
        );
        q.convert_units(UnitSystem::InchPsi).unwrap();
        assert!((q.mesh.size - p.mesh.size).abs() < 1e-12);
    }
}

#[cfg(feature = "occt")]
#[test]
fn nema17_step_import_has_a_complete_preview_and_consistent_units() {
    use fea_problem::UnitSystem;
    let step=include_str!("../../nema17-motor-bracket.step");
    let mut p=Problem::from_step("NEMA17 bracket",step).unwrap();
    assert_eq!(p.units,UnitSystem::MmMpa);
    assert!((p.material.e-UnitSystem::MmMpa.from_ksi(29_000.0)).abs()<1e-8);
    let surface=fea_problem::solid::solid_surface(&p).unwrap();
    let body=fea_problem::solve::body_preview(&p).unwrap();
    assert!(body.surfaces.len()>6 && !body.nodes.is_empty());
    assert_eq!(body.surfaces.len(),p.edge_names().len());
    let volume=surface.volume()*p.units.length_m().powi(3);
    // Independent OCCT BRepGProp integration of the supplied STEP, in mm^3.
    assert!((surface.volume()/34_631.087_277_812_58-1.0).abs()<0.001);
    p.convert_units(UnitSystem::InchPsi).unwrap();
    assert!((p.material.e/29.0e6-1.0).abs()<1e-12);
    let converted=fea_problem::solid::solid_surface(&p).unwrap();
    assert!((converted.volume()*p.units.length_m().powi(3)/volume-1.0).abs()<1e-8);
    eprintln!("NEMA17: {} faces, {} triangles, suggested h={}; volume={} m^3",converted.face_count(),converted.triangle_count(),p.mesh.size,volume);
    match fea_problem::solid::solid_mesh(&p) {
        Ok((mesh,report)) => {
            assert!(mesh.surfaces.len() >= 30, "the bracket's major surfaces must be represented");
            let unresolved=fea_problem::solid::unresolved_faces(&p,&mesh);
            assert!(unresolved.iter().all(|name| ["face31","face32","face35","face37"].contains(&name.as_str())), "{unresolved:?}");
            assert!(fea_problem::solid::resolution_note(&p,&mesh).is_some());
            assert!((report.volume/converted.volume()-1.0).abs()<=0.05);
            eprintln!("NEMA17 mesh: {} nodes, {} tets, volume error {:.3}%",report.nodes,report.tets,100.0*(report.volume/converted.volume()-1.0).abs());
        }
        Err(e) => panic!("NEMA17 default mesh must resolve the actual bracket: {e}"),
    }
}
