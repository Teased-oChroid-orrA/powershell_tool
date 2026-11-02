//! The solid editor on the native pipeline (no OpenCascade): a STEP text written by `solid-kernel`'s `NativeBrep` becomes a problem,
//! gets a hole, a boss and fillets with the closed-form volumes, keeps its supports and loads on their faces, and solves.
//! `docs/issue-46-phase-123.md`.
use fea_problem::solid::{flat_face, solid_surface, SolidCache, SolidOp};
use fea_problem::{solve, Geometry, Load, MeshSpec, Problem, Support};
use solid_kernel::native_brep::NativeBrep;
use std::f64::consts::PI;

fn cube_text(a: f64) -> String {
    NativeBrep::box_from_corners([0., 0., 0.], [a, a, a]).unwrap().to_step_text()
}
fn cube() -> Problem {
    Problem::from_step("Cube", &cube_text(10.0)).unwrap()
}
fn cache_of(p: &Problem) -> &SolidCache {
    match &p.geometry {
        Geometry::Solid { cache: Some(c), .. } => c,
        _ => panic!("not a solid"),
    }
}
fn face_with_normal(p: &Problem, n: [f64; 3]) -> usize {
    let c = cache_of(p);
    (1..=c.face_names().len()).find(|f| flat_face(c, *f).map(|(_, m)| (0..3).all(|k| (m[k] - n[k]).abs() < 1e-6)).unwrap_or(false)).expect("a face with that normal")
}

#[test]
fn a_native_cube_becomes_a_problem_and_gets_a_boss_a_hole_and_fillets_with_the_exact_volume() {
    let mut p = cube();
    assert_eq!(p.edge_names().len(), 6);
    assert!((solid_surface(&p).unwrap().volume() - 1000.0).abs() < 1e-6);
    // A boss on the bottom (whole face): + pi * 1.5^2 * 3.
    let bottom = face_with_normal(&p, [0., 0., -1.]);
    p.apply_solid_op(&SolidOp::Boss { face: bottom, diameter: 3.0, height: 3.0 }).unwrap();
    let boss = PI * 1.5 * 1.5 * 3.0;
    let v = solid_surface(&p).unwrap().volume();
    assert!((v - (1000.0 + boss)).abs() / (1000.0 + boss) < 5e-3, "{v}");
    assert_eq!(p.edge_names().len(), 8);
    // A hole of diameter 2 through the top of the cube and the boss's whole height: 1000 + boss - pi * 1^2 * 13.
    let top = face_with_normal(&p, [0., 0., 1.]);
    p.apply_solid_op(&SolidOp::Hole { face: top, diameter: 2.0 }).unwrap();
    assert_eq!(p.edge_names().len(), 9);
    let want = 1000.0 + boss - PI * 13.0;
    let v = solid_surface(&p).unwrap().volume();
    assert!((v - want).abs() / want < 2e-3, "{v} vs {want}");
    // Fillet every edge of a plain cube: 26 faces and the rounded-box volume.
    let mut q = cube();
    q.apply_solid_op(&SolidOp::Fillet { radius: 0.5 }).unwrap();
    assert_eq!(q.edge_names().len(), 26);
    let (a, r) = (10.0f64 - 1.0, 0.5f64);
    let exact = a * a * a + 6.0 * r * a * a + 3.0 * PI * r * r * a + 4.0 / 3.0 * PI * r * r * r;
    let v = solid_surface(&q).unwrap().volume();
    assert!((v / exact - 1.0).abs() < 2e-3, "{v} vs {exact}");
    // Refusals say why.
    assert!(q.apply_solid_op(&SolidOp::Hole { face: 99, diameter: 1.0 }).unwrap_err().contains("no face99"));
    let curved = (1..=26).find(|f| flat_face(cache_of(&q), *f).is_err()).unwrap();
    assert!(q.apply_solid_op(&SolidOp::Hole { face: curved, diameter: 1.0 }).unwrap_err().contains("flat"));
    assert!(cube().apply_solid_op(&SolidOp::Fillet { radius: -1.0 }).unwrap_err().contains("positive"));
    assert!(cube().apply_solid_op(&SolidOp::Hole { face: top, diameter: 20.0 }).is_err(), "wider than the face");
}

#[test]
fn supports_and_loads_follow_their_faces_and_the_edited_solid_solves() {
    let mut p = cube();
    let (bottom, top, side) = (face_with_normal(&p, [0., 0., -1.]), face_with_normal(&p, [0., 0., 1.]), face_with_normal(&p, [1., 0., 0.]));
    p.supports = vec![Support::fixed(&format!("face{bottom}"))];
    p.loads = vec![Load::Traction { edge: format!("face{top}"), tx: 0.0, ty: 0.0, tz: 5.0 }];
    // A hole in a side face leaves the held bottom and the pulled top where they are (by geometry, whatever their numbers).
    p.apply_solid_op(&SolidOp::Hole { face: side, diameter: 2.0 }).unwrap();
    let (b2, t2) = (face_with_normal(&p, [0., 0., -1.]), face_with_normal(&p, [0., 0., 1.]));
    let Support::Edge { edge, .. } = &p.supports[0] else { panic!() };
    assert_eq!(edge, &format!("face{b2}"));
    let Load::Traction { edge, .. } = &p.loads[0] else { panic!() };
    assert_eq!(edge, &format!("face{t2}"));
    p.mesh = MeshSpec { size: 2.5, ..MeshSpec::default() };
    let s = solve(&p, None).unwrap();
    assert!(s.summary.equilibrium_error < 1e-3, "{}", s.summary.equilibrium_error);
    assert!(s.summary.max_von_mises.value > 4.0, "the load is 5 MPa tension: {}", s.summary.max_von_mises.value);
}

#[test]
fn the_native_tessellation_agrees_with_the_cache_it_replaces() {
    // Tessellating the same text twice gives the same cache (hash, faces, triangles): a stored cache stays current.
    let text = cube_text(4.0);
    let a = fea_problem::solid::tessellate_step(&text, 0.01).unwrap();
    let b = fea_problem::solid::tessellate_step(&text, 0.01).unwrap();
    assert_eq!(a, b);
    assert!(a.is_current(&text) && !a.is_current(&cube_text(5.0)));
    assert_eq!(a.face_names().len(), 6);
    // A text that is not a solid we can read says why instead of producing a cache.
    assert!(fea_problem::solid::tessellate_step("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n", 0.01).is_err());
}

/// The native modeller numbers faces from 0 in its own order; a refusal is shown with the editor's `faceN` of the cache (from 1,
/// in whichever kernel made the cache: with `occt` the cache is OpenCascade's), so the face it names is the one meant. Phase 166.
#[test]
fn a_native_refusal_names_the_face_as_the_editor_does() {
    use fea_problem::native_solid::apply_op_native;
    let parts = |p: &Problem| match &p.geometry {
        Geometry::Solid { step, cache: Some(c) } => (step.clone(), c.clone()),
        _ => panic!("not a solid"),
    };
    let named = |e: &str| -> usize {
        assert!(!e.match_indices("face ").any(|(i, _)| e[i + 5..].starts_with(|c: char| c.is_ascii_digit())), "a kernel index is left: {e}");
        let at = e.find("face").expect("a face is named") + 4;
        e[at..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().expect("faceN")
    };
    let mut p = cube();
    let top = face_with_normal(&p, [0., 0., 1.]);
    let (step, cache) = parts(&p);
    let (step, cache) = apply_op_native(&step, &cache, &SolidOp::Hole { face: top, diameter: 2.0 }).unwrap();
    p.geometry = Geometry::Solid { step: step.clone(), cache: Some(cache.clone()) };
    // Every face with the through hole's rim is flat and normal to z: the fillet names one of them.
    let e = apply_op_native(&step, &cache, &SolidOp::Fillet { radius: 0.5 }).unwrap_err();
    assert!(e.starts_with("without OpenCASCADE, the edges cannot be filleted: face") && e.contains("has a hole"), "{e}");
    let (_, n) = flat_face(&cache, named(&e)).unwrap();
    assert!((n[2].abs() - 1.0).abs() < 1e-9, "{e}: {n:?}");
    // A hole across the first one's axis may touch its wall, the only curved face.
    let side = face_with_normal(&p, [1., 0., 0.]);
    let e = apply_op_native(&step, &cache, &SolidOp::Hole { face: side, diameter: 1.0 }).unwrap_err();
    assert!(e.starts_with("without OpenCASCADE, this hole cannot be cut:") && e.contains("may touch"), "{e}");
    assert!(flat_face(&cache, named(&e)).unwrap_err().contains("curved"), "{e}");
}

/// A STEP solid the native importer reads (B-spline edges) but the native modeller cannot edit: every edit says so, and in the
/// build without OpenCascade the problem is left as it was. Phase 166.
#[test]
fn a_solid_the_native_modeller_cannot_edit_is_refused_for_every_edit() {
    use fea_problem::native_solid::apply_op_native;
    let text = include_str!("../../solid-kernel/tests/data/wavy-prism.step");
    let p = Problem::from_step("wavy", text).unwrap();
    let c = cache_of(&p).clone();
    let flat = (1..=c.face_names().len()).find(|f| flat_face(&c, *f).is_ok()).expect("a flat cap");
    for op in [SolidOp::Fillet { radius: 0.2 }, SolidOp::Hole { face: flat, diameter: 0.5 }, SolidOp::Boss { face: flat, diameter: 0.5, height: 0.5 }] {
        let e = apply_op_native(text, &c, &op).unwrap_err();
        assert!(e.starts_with("without OpenCASCADE, this solid cannot be edited:") && e.contains("not supported by the native modeller"), "{op:?}: {e}");
        #[cfg(not(feature = "occt"))]
        {
            let mut q = p.clone();
            assert_eq!(q.apply_solid_op(&op).unwrap_err(), e);
            assert_eq!(q, p, "{op:?}: a refused edit changes nothing");
        }
    }
}

/// Every OpenCascade-written fixture of `solid-kernel/tests/data` (30 files: analytic and spline solids, holes, fillets, chamfers,
/// assemblies) through every native edit: an edit either gives a closed solid with more faces, or is refused with its reason (a
/// face check shared with OpenCascade, or the native modeller's own refusal starting "without OpenCASCADE"). Nothing panics,
/// nothing is approximated silently. The editor-operation capability matrix rests on this sweep (`docs/issue-46-phase-166.md`).
#[test]
fn every_native_edit_of_every_fixture_succeeds_or_says_why() {
    use fea_problem::native_solid::apply_op_native;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../solid-kernel/tests/data");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "step")).collect();
    files.sort();
    assert!(files.len() >= 30, "{}", files.len());
    let (mut edited, mut refused) = (0, 0);
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let p = match std::panic::catch_unwind(|| Problem::from_step(&name, &text)) {
            Err(_) => panic!("{name}: the import panicked"),
            Ok(Err(e)) => {
                assert!(!e.is_empty(), "{name}");
                continue;
            }
            Ok(Ok(p)) => p,
        };
        let c = cache_of(&p).clone();
        let faces = c.face_names().len();
        let flat = (1..=faces).find(|f| flat_face(&c, *f).is_ok());
        let curved = (1..=faces).find(|f| flat_face(&c, *f).is_err());
        let mut ops = vec![SolidOp::Fillet { radius: 0.2 }];
        ops.extend(flat.into_iter().flat_map(|face| [SolidOp::Hole { face, diameter: 0.5 }, SolidOp::Boss { face, diameter: 0.5, height: 0.5 }]));
        ops.extend(curved.map(|face| SolidOp::Hole { face, diameter: 0.5 }));
        for op in ops {
            match std::panic::catch_unwind(|| apply_op_native(&text, &c, &op)) {
                Err(_) => panic!("{name} {op:?}: panicked"),
                Ok(Ok((step, cache))) => {
                    let mut q = p.clone();
                    q.geometry = Geometry::Solid { step, cache: Some(cache) };
                    assert!(q.edge_names().len() > faces && solid_surface(&q).is_ok(), "{name} {op:?}");
                    edited += 1;
                }
                Ok(Err(e)) => {
                    assert!(e.starts_with("without OpenCASCADE, ") || e.contains("pick a flat face"), "{name} {op:?}: {e}");
                    refused += 1;
                }
            }
        }
    }
    assert!(edited >= 10 && refused >= 50, "{edited} edited, {refused} refused");
}
