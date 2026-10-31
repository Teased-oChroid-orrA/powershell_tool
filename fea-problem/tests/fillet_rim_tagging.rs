//! Boundary-face tagging at a tangent fillet rim and the default size of a filleted import (`docs/issue-46-phase-125.md`).
//!
//! The 12 x 14 x 16 box with every edge rounded by 2 mm has a flat top face of exactly (12 - 4) x (14 - 4) = 80 mm2. A traction
//! on that face must act on 80 mm2: the force is `traction x area` and nothing else. Before phase 125 the mesh's top-face
//! boundary carried 103 mm2 at a 1 mm element (+29 %): vertices of the rim were snapped onto the intersection of nearly parallel
//! planes and landed 1.4 mm outside the solid, and a tolerance of a tenth of the size gave the flat face a strip of fillet.
use fea_problem::solid::{flat_face, smallest_curvature_radius, solid_mesh, SolidCache};
use fea_problem::{solve, Geometry, Load, MeshSpec, Problem, Support};
use solid_kernel::native_brep::NativeBrep;

const TOP_AREA: f64 = 80.0;

fn tri_area(p: &[[f64; 3]], t: &[usize]) -> f64 {
    let (a, b, c) = (p[t[0]], p[t[1]], p[t[2]]);
    let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
}

fn cache_of(p: &Problem) -> &SolidCache {
    match &p.geometry {
        Geometry::Solid { cache: Some(c), .. } => c,
        _ => panic!("not a solid"),
    }
}

fn rounded_box() -> Problem {
    let b = NativeBrep::box_from_corners([0.; 3], [12., 14., 16.]).unwrap().fillet_all_convex(2.).unwrap();
    Problem::from_step("rounded box", &b.to_step_text()).unwrap()
}

fn face_with_z(p: &Problem, sign: f64) -> usize {
    let c = cache_of(p);
    (1..=c.face_names().len()).find(|f| flat_face(c, *f).map(|(m, n)| n[2] * sign > 0.999 && (m[2] - if sign > 0.0 { 16.0 } else { 0.0 }).abs() < 1e-6).unwrap_or(false)).expect("a flat z face")
}

#[test]
fn the_flat_face_next_to_tangent_fillets_is_tagged_with_its_own_area_and_no_vertex_leaves_the_solid() {
    let mut p = rounded_box();
    let top = face_with_z(&p, 1.0);
    for (h, tolerance) in [(1.356, 0.025), (1.0, 0.005), (0.85, 0.005)] {
        p.mesh = MeshSpec { size: h, ..p.mesh.clone() };
        let (mesh, _) = solid_mesh(&p).unwrap();
        // No boundary vertex outside the box [0,12] x [0,14] x [0,16] (the old spike: z = 17.4 at h = 1).
        // Snapping accepts a boundary vertex within 0.02 of its local size of the surface (it is not always moved the last bit).
        let slack = 0.03 * h;
        for v in &mesh.nodes {
            assert!((-slack..=12.0 + slack).contains(&v[0]) && (-slack..=14.0 + slack).contains(&v[1]) && (-slack..=16.0 + slack).contains(&v[2]), "h {h}: node {v:?} is outside the solid");
        }
        let area: f64 = mesh.surfaces[&format!("face{top}")].iter().map(|f| tri_area(&mesh.nodes, f)).sum();
        // Measured -1.75 % (h 1.356, one and a half elements per radius), 0.00 % (1.0), -0.28 % (0.85): the remainder is the
        // facets of the boundary mesh at the rim, never a strip of fillet.
        assert!((area / TOP_AREA - 1.0).abs() < tolerance, "h {h}: tagged area {area} against the exact {TOP_AREA}");
    }
}

#[test]
fn the_fillet_rim_at_085_stays_inside_the_solid() {
    let mut p = rounded_box();
    p.mesh = MeshSpec { size: 0.85, ..p.mesh };
    let (mesh, _) = solid_mesh(&p).unwrap();
    let slack = 0.03 * 0.85;
    let outside = mesh.nodes.iter().find(|v| !(-slack..=12.0 + slack).contains(&v[0]) || !(-slack..=14.0 + slack).contains(&v[1]) || !(-slack..=16.0 + slack).contains(&v[2]));
    assert!(outside.is_none(), "node {outside:?} is outside the solid");
}

#[test]
fn a_uniform_traction_on_that_face_carries_the_closed_form_force() {
    let mut p = rounded_box();
    let (top, bottom) = (face_with_z(&p, 1.0), face_with_z(&p, -1.0));
    p.mesh = MeshSpec { size: 1.0, ..p.mesh.clone() };
    p.supports = vec![Support::fixed(&format!("face{bottom}"))];
    p.loads = vec![Load::Traction { edge: format!("face{top}"), tx: 0.0, ty: 0.0, tz: 10.0 }];
    let s = solve(&p, None).unwrap();
    let want = 10.0 * TOP_AREA;
    let got = s.summary.reaction[2].abs();
    // Before: +26 % (native) / +16 % (OpenCascade tessellation) at a 1 mm element.
    assert!((got / want - 1.0).abs() < 0.01, "reaction {got} against traction x area = {want}");
}

#[test]
fn the_default_size_resolves_a_fillet_so_the_part_meshes_without_the_user_lowering_it() {
    let p = rounded_box();
    let r = smallest_curvature_radius(cache_of(&p)).expect("the box has fillets");
    // The estimate may read low on a skewed fillet triangulation (1.41 measured; finer is the safe side), never high.
    assert!((1.2..=2.1).contains(&r), "estimated fillet radius {r} against 2");
    // Two elements across the true radius (1.0), never coarser than the bounding-box rule (diagonal / 18 = 1.356).
    assert!(p.mesh.size <= 1.0 + 1e-9 && p.mesh.size < 1.356, "default size {}", p.mesh.size);
    let (mesh, rep) = solid_mesh(&p).expect("the default size must mesh the filleted box");
    assert!(rep.tets > 1000 && rep.tets < 100_000, "{} tets", rep.tets);
    assert!(mesh.surfaces.len() >= 26, "every one of the 26 faces owns boundary elements: {}", mesh.surfaces.len());
}

#[test]
fn the_default_size_never_asks_for_more_than_the_default_element_cap() {
    // 100 mm cube with 0.5 mm fillets: resolving the fillet would take about 10^9 elements; the default keeps to 2.5 x 10^4.
    let b = NativeBrep::box_from_corners([0.; 3], [100., 100., 100.]).unwrap().fillet_all_convex(0.5).unwrap();
    let p = Problem::from_step("big", &b.to_step_text()).unwrap();
    let est = 1.0e6 / (0.1179 * p.mesh.size.powi(3));
    assert!(est <= 2.55e4, "default size {} asks for about {est:.0} elements", p.mesh.size);
    assert!(p.mesh.size > 0.25 && p.mesh.size < 9.0, "the cap, not the fillet, set it: {}", p.mesh.size);
}

#[test]
#[ignore = "diagnostic"]
fn diag_default() {
    let mut p = rounded_box();
    let top = face_with_z(&p, 1.0);
    for h in [1.356, 1.2, 1.0, 0.85, 0.7, 0.6] {
    p.mesh = MeshSpec { size: h, ..p.mesh.clone() };
    let (mesh, _) = solid_mesh(&p).unwrap();
    let list = &mesh.surfaces[&format!("face{top}")];
    let (mut n, mut out, mut tot) = (0usize, 0.0f64, 0.0f64);
    for f in list {
        let v: Vec<[f64; 3]> = f.iter().take(3).map(|&i| mesh.nodes[i]).collect();
        let a = tri_area(&mesh.nodes, f);
        tot += a;
        let inside = v.iter().all(|x| x[0] >= 2.0 - 1e-9 && x[0] <= 10.0 + 1e-9 && x[1] >= 2.0 - 1e-9 && x[1] <= 12.0 + 1e-9);
        if !inside {
            n += 1;
            out += a;
        }
    }
    eprintln!("size {h} total {tot:.3} ({:+.2} %) partly-outside tris {n} area {out:.3}", 100.0 * (tot / TOP_AREA - 1.0));
    }
}

#[test]
fn the_curvature_radius_of_a_cylinder_is_its_radius_and_an_all_flat_solid_has_none() {
    let cyl = Problem::from_step("cyl", &NativeBrep::cylinder_z([0.; 3], 10., 30.).unwrap().to_step_text()).unwrap();
    let r = smallest_curvature_radius(cache_of(&cyl)).expect("a cylinder is curved");
    assert!((r / 10.0 - 1.0).abs() < 0.05, "{r}");
    let cube = Problem::from_step("cube", &NativeBrep::box_from_corners([0.; 3], [10., 20., 30.]).unwrap().to_step_text()).unwrap();
    assert_eq!(smallest_curvature_radius(cache_of(&cube)), None);
}
