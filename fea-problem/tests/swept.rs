//! The Sweep feature: a profile sketch carried along a path sketch (lines and tangent arcs).
use fea_problem::history::{Body, FeatureKind, History, Plane};
use fea_problem::sketch::{Constraint as C, Sketch};
use fea_problem::{build, solve, templates, Load, Support};
use std::f64::consts::FRAC_PI_2;

/// The profile: a rectangle x in [0, 1], y in [0, 1] on the Front plane (normal +z).
fn profile() -> Sketch {
    Sketch::from_shapes(&fea_problem::Shape::Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, &[])
}

/// A path in the Right plane (sketch x = world y, sketch y = world z): up 2 from the origin, then a quarter turn of radius 3
/// towards +y. Drawn arc-first so the walk has to turn it around to start on the profile's plane.
fn bent_path() -> Sketch {
    let mut s = Sketch::default();
    let (c, end, join, origin) = (s.add_point(3.0, 2.0), s.add_point(3.0, 5.0), s.add_point(0.0, 2.0), s.add_point(0.0, 0.0));
    let arc = s.add_arc(c, end, join); // counter-clockwise from (3, 5) to (0, 2) about (3, 2)
    let line = s.add_line(join, origin);
    s.add_constraint(C::Tangent { a: line, b: arc }).unwrap();
    s
}

fn swept(path: Sketch, path_plane: Plane) -> Result<(History, fea_problem::history::Built), String> {
    let mut h = History::default();
    let pr = h.add("Profile", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: profile() });
    let pa = h.add("Path", FeatureKind::Sketch { plane: path_plane, sketch: path });
    h.add("Sweep1", FeatureKind::Sweep { profile: pr, path: pa, layers: 8 });
    let b = h.rebuild()?;
    Ok((h, b))
}

fn volume(m: &fea_core::Mesh) -> f64 {
    let loads = fea_core::Loads { body: Some([0.0, 0.0, 1.0]), ..Default::default() };
    fea_core::loads::assemble(m, &loads).unwrap().chunks(3).map(|f| f[2]).sum()
}

fn problem_of(h: History) -> fea_problem::Problem {
    let mut p = templates::blank();
    p.history = Some(h);
    p.apply_history().unwrap();
    p.mesh.size = 0.25;
    p.mesh.element = fea_problem::ElementChoice::Quad9;
    p
}

#[test]
fn a_bent_sweep_has_the_pappus_volume_and_lands_on_the_path() {
    let (h, built) = swept(bent_path(), Plane::Right { offset: 0.0 }).unwrap();
    let Body::Sweep { path, origin, .. } = &built.body else { panic!("{:?}", built.body) };
    assert_eq!(*origin, [0.0, 0.0]);
    assert!((path.length() - (2.0 + 3.0 * FRAC_PI_2)).abs() < 1e-12);
    let mesh = build::sketch_mesh(&problem_of(h), None).unwrap();
    // Profile area 1: a straight part of length 2, then a quarter turn whose centroid (1 - 0.5 from the centre side) runs at radius 2.5.
    let want = 2.0 + FRAC_PI_2 * 2.5;
    let v = volume(&mesh);
    assert!((v - want).abs() < 2e-3 * want, "{v} vs {want}");
    // The far end face lies in the plane y = 3 + 2... the path ends at world (0, 3, 5) heading +y: end-face nodes have y = 3.
    let end = &mesh.node_sets["end"];
    assert!(!end.is_empty());
    for &n in end {
        let q = mesh.nodes[n];
        assert!((q[1] - 3.0).abs() < 1e-9, "end node {q:?} is not in the plane y = 3");
    }
    assert!(mesh.surfaces.contains_key("start") && mesh.surfaces.contains_key("end") && mesh.surfaces.contains_key("bottom"));
}

#[test]
fn a_swept_bar_carries_the_axial_stress_of_a_straight_one() {
    // The same sweep, pulled at the end and held at the start: sigma = F / A in the straight part.
    let (h, _) = swept(bent_path(), Plane::Right { offset: 0.0 }).unwrap();
    let mut p = problem_of(h);
    p.supports = vec![Support::fixed("start")];
    p.loads = vec![Load::Traction { edge: "end".into(), tx: 0.0, ty: 10.0, tz: 0.0 }];
    let s = solve(&p, None).unwrap();
    assert!(s.summary.equilibrium_error < 1e-6, "{}", s.summary.equilibrium_error);
    // 10 per unit area on a unit area: the reaction balances 10 along y.
    assert!((s.summary.reaction[1] + 10.0).abs() < 1e-6, "{:?}", s.summary.reaction);
}

#[test]
fn a_sweep_reports_what_is_wrong_with_the_path() {
    // Away from the profile's plane.
    let mut far = Sketch::default();
    let (a, b) = (far.add_point(0.0, 1.0), far.add_point(0.0, 4.0));
    far.add_line(a, b);
    let e = swept(far, Plane::Right { offset: 0.0 }).unwrap_err();
    assert!(e.contains("does not start on the plane"), "{e}");
    // A corner between two lines.
    let mut corner = Sketch::default();
    let (a, b, c) = (corner.add_point(0.0, 0.0), corner.add_point(0.0, 2.0), corner.add_point(2.0, 2.0));
    corner.add_line(a, b);
    corner.add_line(b, c);
    let e = swept(corner, Plane::Right { offset: 0.0 }).unwrap_err();
    assert!(e.contains("corner") && e.contains("tangent arc"), "{e}");
    // Leaving the plane the wrong way (path along -z).
    let mut down = Sketch::default();
    let (a, b) = (down.add_point(0.0, 0.0), down.add_point(0.0, -2.0));
    down.add_line(a, b);
    let e = swept(down, Plane::Right { offset: 0.0 }).unwrap_err();
    assert!(e.contains("along the normal"), "{e}");
    // A closed loop and a circle are not paths.
    let mut circle = Sketch::default();
    circle.add_circle(0.0, 0.0, 1.0);
    assert!(swept(circle, Plane::Right { offset: 0.0 }).unwrap_err().contains("circle"));
    // Deleting the path sketch deletes the sweep that used it.
    let (mut h, _) = swept(bent_path(), Plane::Right { offset: 0.0 }).unwrap();
    let path_id = h.features[1].id;
    h.remove(path_id);
    assert_eq!(h.features.len(), 1);
}

#[test]
fn a_sweep_along_a_straight_path_is_the_extrusion() {
    let mut line = Sketch::default();
    let (a, b) = (line.add_point(0.0, 0.0), line.add_point(0.0, 3.0));
    line.add_line(a, b);
    let (h, _) = swept(line, Plane::Right { offset: 0.0 }).unwrap();
    let swept_mesh = build::sketch_mesh(&problem_of(h), None).unwrap();
    let mut ext = History::default();
    let sk = ext.add("S", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: profile() });
    ext.add("E", FeatureKind::Extrude { profile: sk, depth: 3.0, layers: 8 });
    let ext_mesh = build::sketch_mesh(&problem_of(ext), None).unwrap();
    assert!((volume(&swept_mesh) - 3.0).abs() < 1e-9 && (volume(&ext_mesh) - 3.0).abs() < 1e-9);
}
