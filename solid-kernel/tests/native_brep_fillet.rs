//! Native fillet of every edge of a convex polyhedron, against the Steiner formula (volume of an inner parallel body plus a ball:
//! `V + A r + M r^2 + 4/3 pi r^3`) and the rounded-box closed form. `docs/issue-46-phase-122.md`.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::cad::{Loop, Profile};
use solid_kernel::native_brep::NativeBrep;
use solid_kernel::native_surface::{Limits as SurfaceLimits, SampledSurface};
use solid_kernel::step::{inventory, Limits};
use std::f64::consts::PI;

fn sample(b: &NativeBrep, chord: f64) -> (usize, SampledSurface) {
    let text = b.to_step_text();
    let data = inventory(&text, Limits::default()).unwrap_or_else(|e| panic!("inventory: {e}"));
    let model = AnalyticModel::read(&data, Options { chord_mm: chord, ..Options::default() }).unwrap_or_else(|e| panic!("read: {e}"));
    let faces = model.body().faces.len();
    (faces, model.sampled_surface(SurfaceLimits::default(), |_| true).unwrap_or_else(|e| panic!("sample: {e}")))
}

fn rounded_box(a: f64, b: f64, c: f64, r: f64) -> f64 {
    let (x, y, z) = (a - 2. * r, b - 2. * r, c - 2. * r);
    x * y * z + 2. * r * (x * y + x * z + y * z) + PI * r * r * (x + y + z) + 4. / 3. * PI * r * r * r
}

/// Area, perimeter and exterior angles of a convex counter-clockwise polygon, and its inward offset by `r` (line intersections).
fn offset_polygon(p: &[[f64; 2]], r: f64) -> Vec<[f64; 2]> {
    let n = p.len();
    let line = |i: usize| {
        let (a, b) = (p[i], p[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = (dx * dx + dy * dy).sqrt();
        // Inward (left) unit normal and the shifted line n.x = c.
        let nx = [-dy / l, dx / l];
        (nx, nx[0] * a[0] + nx[1] * a[1] + r)
    };
    (0..n)
        .map(|i| {
            let (n1, c1) = line((i + n - 1) % n);
            let (n2, c2) = line(i);
            let d = n1[0] * n2[1] - n1[1] * n2[0];
            [(c1 * n2[1] - c2 * n1[1]) / d, (n1[0] * c2 - n2[0] * c1) / d]
        })
        .collect()
}
fn area(p: &[[f64; 2]]) -> f64 {
    (0..p.len()).map(|i| p[i][0] * p[(i + 1) % p.len()][1] - p[(i + 1) % p.len()][0] * p[i][1]).sum::<f64>() / 2.
}
fn perimeter(p: &[[f64; 2]]) -> f64 {
    (0..p.len()).map(|i| ((p[i][0] - p[(i + 1) % p.len()][0]).powi(2) + (p[i][1] - p[(i + 1) % p.len()][1]).powi(2)).sqrt()).sum()
}

/// Steiner volume of the prism `p` (height `h`, convex, counter-clockwise) with every edge rounded by `r`.
fn rounded_prism(p: &[[f64; 2]], h: f64, r: f64) -> f64 {
    let q = offset_polygon(p, r);
    let hh = h - 2. * r;
    let (a, l) = (area(&q), perimeter(&q));
    let volume = a * hh;
    let surface = 2. * a + l * hh;
    // Integral of the mean curvature: vertical edges carry the exterior angles (summing to 2 pi), horizontal ones pi/2.
    let m = 0.5 * (2. * PI * hh + 2. * l * PI / 2.);
    volume + surface * r + m * r * r + 4. / 3. * PI * r * r * r
}

#[test]
fn a_box_with_every_edge_rounded_matches_the_closed_form() {
    let b = NativeBrep::box_from_corners([0., 0., 0.], [2., 3., 4.]).unwrap();
    let f = b.fillet_all_convex(0.2).unwrap();
    assert_eq!((f.faces.len(), f.vertices.len(), f.edges.len()), (6 + 12 + 8, 24, 48), "6 planes, 12 cylinders, 8 sphere patches");
    let (faces, s) = sample(&f, 0.002);
    assert_eq!(faces, 26);
    let exact = rounded_box(2., 3., 4., 0.2);
    assert!((s.signed_volume_mm3 / exact - 1.).abs() < 1e-3, "{} vs {exact}", s.signed_volume_mm3);
    // An inscribed sampling is a little small, never large.
    assert!(s.signed_volume_mm3 <= exact * (1. + 1e-9));
    // Rounding removes volume: less than the box, more than the box with a smaller radius.
    let small = sample(&b.fillet_all_convex(0.05).unwrap(), 0.002).1.signed_volume_mm3;
    assert!(small < 24. && small > s.signed_volume_mm3);
    // The largest radius a 2 mm side allows is 1 (a ball): 1.0 degenerates, 0.99 builds.
    assert!(b.fillet_all_convex(1.0).is_err());
    assert!(b.fillet_all_convex(0.99).is_ok());
}

#[test]
fn rounded_prisms_match_the_steiner_formula() {
    let tri = vec![[0., 0.], [6., 0.], [0., 4.]];
    let hex: Vec<[f64; 2]> = (0..6).map(|i| [3. * (PI / 3. * i as f64).cos(), 3. * (PI / 3. * i as f64).sin()]).collect();
    let trap = vec![[0., 0.], [8., 0.], [6., 3.], [2., 3.]];
    for (name, p, h, r) in [("triangle", &tri, 5., 0.3), ("hexagon", &hex, 4., 0.5), ("trapezoid", &trap, 2., 0.25)] {
        assert!(area(p) > 0.0, "{name} must be counter-clockwise");
        let b = NativeBrep::extrude(&Profile { outer: Loop::Polygon(p.clone()), holes: vec![] }, 0., h).unwrap();
        let f = b.fillet_all_convex(r).unwrap();
        let n = p.len();
        assert_eq!(f.faces.len(), (n + 2) + 3 * n + 2 * n, "{name}: planes + edge cylinders + vertex spheres");
        let (_, s) = sample(&f, 0.002);
        let exact = rounded_prism(p, h, r);
        assert!((s.signed_volume_mm3 / exact - 1.).abs() < 2e-3, "{name}: {} vs {exact}", s.signed_volume_mm3);
        assert!(s.signed_volume_mm3 <= exact * (1. + 1e-9), "{name}");
    }
}

#[test]
fn a_filleted_shape_goes_through_the_step_text_and_back() {
    let b = NativeBrep::box_from_corners([0., 0., 0.], [5., 4., 3.]).unwrap().fillet_all_convex(0.5).unwrap();
    let back = NativeBrep::from_step_text(&b.to_step_text()).unwrap();
    assert_eq!(back.faces.len(), 26);
    // The planar faces keep their original planes: a hole can still be drilled in the top of the rounded box.
    let top = back.faces.iter().position(|f| matches!(&f.surface, solid_kernel::native_brep::Surface::Plane { origin, .. } if (origin[2] - 3.).abs() < 1e-9)).unwrap();
    let drilled = back.drill(top, [2.5, 2., 3.], 0.8, None).unwrap();
    let (_, s) = sample(&drilled, 0.002);
    let exact = rounded_box(5., 4., 3., 0.5) - PI * 0.64 * 3.;
    assert!((s.signed_volume_mm3 / exact - 1.).abs() < 2e-3, "{} vs {exact}", s.signed_volume_mm3);
}

#[test]
fn what_is_not_a_convex_polyhedron_is_refused() {
    let l = Profile { outer: Loop::Polygon(vec![[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 3.], [0., 3.]]), holes: vec![] };
    assert!(NativeBrep::extrude(&l, 0., 2.).unwrap().fillet_all_convex(0.1).unwrap_err().contains("not convex"));
    let holed = Profile { outer: Loop::Polygon(vec![[0., 0.], [10., 0.], [10., 10.], [0., 10.]]), holes: vec![Loop::Circle { centre: [5., 5.], radius: 1. }] };
    assert!(NativeBrep::extrude(&holed, 0., 2.).unwrap().fillet_all_convex(0.1).is_err());
    assert!(NativeBrep::cylinder_z([0.; 3], 2., 3.).unwrap().fillet_all_convex(0.1).is_err());
    let b = NativeBrep::box_from_corners([0., 0., 0.], [2., 3., 4.]).unwrap();
    assert!(b.fillet_all_convex(0.0).is_err() && b.fillet_all_convex(-1.0).is_err() && b.fillet_all_convex(f64::NAN).is_err());
}
