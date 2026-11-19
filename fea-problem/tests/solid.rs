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
    // A STEP text without a solid is refused by whichever kernel reads it (the native one in the default build), with a reason.
    assert!(!Geometry::solid_from_step(STEP, 0.01).unwrap_err().is_empty());
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
