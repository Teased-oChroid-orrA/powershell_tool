//! How the volume mesh names its boundary facets next to a concave edge (`docs/issue-46-phase-172.md`), on the native pipeline
//! (no OpenCascade needed). Run: `cargo test -p fea-problem --release --test facet_naming -- --ignored --nocapture`.
use fea_problem::solid::{flat_face, solid_mesh, solid_surface};
use fea_problem::{Geometry, Problem};
use solid_kernel::native_brep::NativeBrep;
use std::collections::BTreeMap;

/// Unit normal and area of a triangle.
fn normal_area(v: [[f64; 3]; 3]) -> ([f64; 3], f64) {
    let (u, w) = ([v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]], [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]]);
    let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    (n.map(|x| x / l), 0.5 * l)
}

/// A 10 x 10 x 4 box with a R2 x 3 boss on its top: the top face meets the boss along a concave circle. For each flat face, the
/// area of the facets named after it against its surface area, and the facets more than 45 degrees off its plane grouped by the
/// model faces their corners lie on (within a hundredth of the size, the namer's tolerance).
#[test]
#[ignore = "diagnostic table"]
fn facets_named_after_a_flat_face_at_a_concave_edge() {
    let text = NativeBrep::box_from_corners([0.; 3], [10., 10., 4.]).unwrap().boss(5, [5., 5., 4.], 2., 3.).unwrap().to_step_text();
    let p = Problem::from_step("box + boss", &text).unwrap();
    let Geometry::Solid { cache: Some(cache), .. } = &p.geometry else { panic!() };
    let h = p.mesh.size;
    let solid = solid_surface(&p).unwrap();
    let (mesh, _) = solid_mesh(&p).unwrap();
    let on = |x: [f64; 3]| -> Vec<usize> { solid.faces_near(x, 1.2 * h).into_iter().filter(|f| f.dist.abs() <= 0.01 * h).map(|f| f.face + 1).collect() };
    eprintln!("size {h:.4}");
    for (f, surface) in solid.face_areas().iter().enumerate().map(|(f, a)| (f + 1, *a)) {
        let Ok((_, n)) = flat_face(cache, f) else { continue };
        let (mut area, mut steep) = (0.0, BTreeMap::<String, (usize, f64)>::new());
        for facet in mesh.surfaces.get(&format!("face{f}")).map_or(&[][..], Vec::as_slice) {
            let v = [0, 1, 2].map(|k| mesh.nodes[facet[k]]);
            let (m, a) = normal_area(v);
            area += a;
            if m[0] * n[0] + m[1] * n[1] + m[2] * n[2] < 45f64.to_radians().cos() {
                let mut corners: Vec<String> = v.iter().map(|x| format!("{:?}", on(*x))).collect();
                corners.sort();
                let e = steep.entry(corners.join(" ")).or_default();
                e.0 += 1;
                e.1 += a;
            }
        }
        eprintln!("face{f:<3} surface {surface:>9.5} facets {area:>9.5} ({:+.3} %)", 100.0 * (area / surface - 1.0));
        for (corners, (count, a)) in steep {
            eprintln!("  over 45 deg, corners on {corners:<24} facets {count:>3} area {a:.5}");
        }
    }
}
