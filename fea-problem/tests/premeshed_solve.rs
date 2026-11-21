//! A solid solve takes the analysis mesh the editor already made for its preview instead of meshing the same problem again
//! (`solve::solve_observed_with`). The mesh depends on the geometry, mesh settings and material, not on the supports and loads;
//! the one check that does depend on them (a condition on a face the mesh does not resolve) still runs against the real ones.
use std::cell::RefCell;

use fea_problem::solid::{flat_face, solid_mesh, SolidCache};
use fea_problem::solve::{preview, solve, solve_observed_with};
use fea_problem::{Geometry, Load, MeshSpec, Problem, Support};
use solid_kernel::native_brep::NativeBrep;

fn cache_of(p: &Problem) -> &SolidCache {
    match &p.geometry {
        Geometry::Solid { cache: Some(c), .. } => c,
        _ => panic!("not a solid"),
    }
}

/// The face of the 10 mm cube at `z` (0 or 10).
fn z_face(p: &Problem, z: f64) -> usize {
    let c = cache_of(p);
    (1..=c.face_names().len())
        .find(|f| flat_face(c, *f).map(|(m, n)| n[2].abs() > 0.999 && (m[2] - z).abs() < 1e-6).unwrap_or(false))
        .expect("a flat z face")
}

fn cube(size: f64) -> Problem {
    let b = NativeBrep::box_from_corners([0.; 3], [10., 10., 10.]).unwrap();
    let mut p = Problem::from_step("cube", &b.to_step_text()).unwrap();
    p.mesh = MeshSpec { size, ..p.mesh };
    let (bottom, top) = (z_face(&p, 0.0), z_face(&p, 10.0));
    p.supports = vec![Support::fixed(&format!("face{bottom}"))];
    p.loads = vec![Load::Traction { edge: format!("face{top}"), tx: 0.0, ty: 0.0, tz: 10.0 }];
    p
}

/// What the editor holds: the mesh of the problem without supports and loads (its `preview_key`).
fn editor_preview(p: &Problem) -> fea_core::Mesh {
    preview(&Problem { supports: Vec::new(), loads: Vec::new(), ..p.clone() }, None).unwrap()
}

#[test]
fn a_solve_on_the_preview_mesh_equals_a_fresh_solve_bit_for_bit_and_does_not_mesh_again() {
    let p = cube(2.5);
    let mesh = editor_preview(&p);
    let fresh = solve(&p, None).unwrap();

    let stages = RefCell::new(Vec::new());
    let reused = solve_observed_with(&p, None, Some(&mesh), &|stage| {
        stages.borrow_mut().push(stage);
        Ok(())
    })
    .unwrap();

    let stages = stages.into_inner();
    assert!(stages.contains(&"Reusing the analysis mesh"), "stages: {stages:?}");
    assert!(!stages.contains(&"Generating analysis mesh"), "the mesh was made again: {stages:?}");
    assert_eq!(fresh.u.len(), reused.u.len());
    assert!(fresh.u.iter().zip(&reused.u).all(|(a, b)| a.to_bits() == b.to_bits()), "displacements differ");
    assert_eq!(fresh.summary.max_displacement.value.to_bits(), reused.summary.max_displacement.value.to_bits());
}

#[test]
fn a_solve_without_a_preview_mesh_still_meshes() {
    let p = cube(2.5);
    let stages = RefCell::new(Vec::new());
    solve_observed_with(&p, None, None, &|stage| {
        stages.borrow_mut().push(stage);
        Ok(())
    })
    .unwrap();
    let stages = stages.into_inner();
    assert!(stages.contains(&"Generating analysis mesh") && !stages.contains(&"Reusing the analysis mesh"), "stages: {stages:?}");
}

#[test]
fn a_solve_on_the_preview_mesh_still_refuses_a_condition_on_a_face_the_mesh_does_not_resolve() {
    // A 0.2 mm fillet is far below a 4 mm element: its faces own no boundary element.
    let b = NativeBrep::box_from_corners([0.; 3], [10., 10., 10.]).unwrap().fillet_all_convex(0.2).unwrap();
    let mut p = Problem::from_step("fine fillets", &b.to_step_text()).unwrap();
    p.mesh = MeshSpec { size: 4.0, ..p.mesh };
    let (bottom, top) = (z_face(&p, 0.0), z_face(&p, 10.0));
    p.supports = vec![Support::fixed(&format!("face{bottom}"))];
    let unresolved = (1..=cache_of(&p).face_names().len())
        .filter(|f| *f != bottom && *f != top)
        .find(|f| {
            let mut q = p.clone();
            q.loads = vec![Load::Traction { edge: format!("face{f}"), tx: 0.0, ty: 0.0, tz: 1.0 }];
            solid_mesh(&q).err().is_some_and(|e| e.contains("does not resolve STEP surfaces"))
        })
        .expect("a fillet face that the 4 mm mesh does not resolve");
    p.loads = vec![Load::Traction { edge: format!("face{unresolved}"), tx: 0.0, ty: 0.0, tz: 1.0 }];

    let mesh = editor_preview(&p);
    let fresh = solve(&p, None).err().expect("a fresh solve refuses");
    let reused = solve_observed_with(&p, None, Some(&mesh), &|_| Ok(())).err().expect("the reused-mesh solve refuses too");
    assert_eq!(fresh, reused);
    assert!(reused.contains("does not resolve STEP surfaces"), "{reused}");
}
