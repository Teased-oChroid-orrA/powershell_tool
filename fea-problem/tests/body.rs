//! `solve::body_preview`: the coarse surface of an extruded body, shown before the analysis mesh exists.
use fea_problem::{solve, templates, Analysis, Geometry, Problem};

fn plate() -> Problem {
    templates::templates().into_iter().map(|(_, p)| p).find(|p| p.analysis == Analysis::Solid && matches!(p.geometry, Geometry::Sketch { .. })).expect("a 3D sketch template")
}

fn extent(mesh: &fea_core::Mesh) -> [f64; 3] {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for n in &mesh.nodes {
        for k in 0..3 {
            lo[k] = lo[k].min(n[k]);
            hi[k] = hi[k].max(n[k]);
        }
    }
    std::array::from_fn(|k| hi[k] - lo[k])
}

#[test]
fn the_body_of_an_extrusion_has_its_size_and_its_surface_names() {
    let p = plate();
    let body = solve::body_preview(&p).unwrap();
    let full = solve::preview(&p, None).unwrap();
    // The 4 x 2 x 0.5 plate, with its named sides.
    let e = extent(&body);
    assert!((e[0] - 4.0).abs() < 1e-9 && (e[1] - 2.0).abs() < 1e-9 && (e[2] - 0.5).abs() < 1e-9, "{e:?}");
    assert_eq!(extent(&full).map(|v| (v * 1e6).round()), e.map(|v| (v * 1e6).round()), "the same body");
    for name in ["left", "right", "top", "bottom", "hole1"] {
        assert!(body.surfaces.contains_key(name) || body.node_sets.contains_key(name), "{name} is missing from the body");
    }
    assert!(!body.boundary_faces().is_empty());
    // Loads and supports are not part of the body, and it never refines.
    let mut loaded = p.clone();
    loaded.supports.clear();
    loaded.loads.clear();
    assert_eq!(solve::body_preview(&loaded).unwrap().nodes.len(), body.nodes.len());
}

#[test]
fn a_plane_model_and_an_imported_mesh_have_no_body() {
    let mut p = plate();
    p.analysis = Analysis::PlaneStress;
    assert!(solve::body_preview(&p).is_err());
    assert!(solve::body_preview(&templates::imported("x.msh")).is_err());
}
