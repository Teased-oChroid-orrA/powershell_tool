use fea_problem::sketch::solve::Status;
use fea_problem::sketch::{Constraint as C, Sketch};
use fea_problem::Shape;

/// A rough rectangle drawn by hand: 4 points, 4 lines.
fn rough_rect() -> (Sketch, [u32; 4], [u32; 4]) {
    let mut s = Sketch::default();
    let p = [s.add_point(0.1, -0.1), s.add_point(10.3, 0.2), s.add_point(9.8, 5.1), s.add_point(-0.2, 4.7)];
    let l = [s.add_line(p[0], p[1]), s.add_line(p[1], p[2]), s.add_line(p[2], p[3]), s.add_line(p[3], p[0])];
    (s, p, l)
}

#[test]
fn rectangle_constraints_and_dimensions_fully_define_it() {
    let (mut s, p, l) = rough_rect();
    for (i, id) in l.iter().enumerate() {
        s.add_constraint(if i % 2 == 0 { C::Horizontal { line: *id } } else { C::Vertical { line: *id } }).unwrap();
    }
    let r = s.solve();
    assert_eq!(r.status, Status::UnderConstrained);
    assert_eq!(r.dof, 4, "a rectangle has position (2), width, height");
    s.add_constraint(C::Fixed { p: p[0], x: 0.0, y: 0.0 }).unwrap();
    s.add_constraint(C::Length { line: l[0], d: 12.0 }).unwrap();
    s.add_constraint(C::Length { line: l[1], d: 4.0 }).unwrap();
    let r = s.solve();
    assert_eq!(r.status, Status::FullyConstrained, "{r:?}");
    assert!(r.max_residual < 1e-9);
    let pr = s.profile().unwrap();
    assert_eq!(pr.outer, Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 4.0 });
    assert!(pr.holes.is_empty());
}

#[test]
fn redundant_agreeing_constraint_is_over_defined_and_contradiction_is_conflicting() {
    let (mut s, _p, l) = rough_rect();
    for (i, id) in l.iter().enumerate() {
        s.add_constraint(if i % 2 == 0 { C::Horizontal { line: *id } } else { C::Vertical { line: *id } }).unwrap();
    }
    s.add_constraint(C::Perpendicular { a: l[0], b: l[1] }).unwrap();
    assert_eq!(s.solve().status, Status::OverConstrained);
    // Contradiction: top and bottom length differ.
    s.add_constraint(C::Length { line: l[0], d: 5.0 }).unwrap();
    let c = s.add_constraint(C::Length { line: l[2], d: 7.0 }).unwrap();
    let r = s.solve();
    assert_eq!(r.status, Status::Conflicting);
    assert!(!r.violated.is_empty());
    assert!(s.remove_constraint(c));
    assert_ne!(s.solve().status, Status::Conflicting);
}

#[test]
fn invalid_constraints_are_refused() {
    let (mut s, p, l) = rough_rect();
    assert!(s.add_constraint(C::Horizontal { line: p[0] }).is_none(), "a point is not a line");
    assert!(s.add_constraint(C::Parallel { a: l[0], b: l[0] }).is_none());
    assert!(s.add_constraint(C::Coincident { a: p[0], b: 999 }).is_none());
    assert!(s.constraints.is_empty());
}

#[test]
fn triangle_with_angle_and_lengths_hits_the_exact_value() {
    let mut s = Sketch::default();
    let p = [s.add_point(0.0, 0.0), s.add_point(4.0, 0.3), s.add_point(1.0, 3.0)];
    let l = [s.add_line(p[0], p[1]), s.add_line(p[1], p[2]), s.add_line(p[2], p[0])];
    s.add_constraint(C::Fixed { p: p[0], x: 0.0, y: 0.0 }).unwrap();
    s.add_constraint(C::Horizontal { line: l[0] }).unwrap();
    s.add_constraint(C::Length { line: l[0], d: 4.0 }).unwrap();
    s.add_constraint(C::Length { line: l[2], d: 3.0 }).unwrap();
    s.add_constraint(C::Angle { a: l[0], b: l[2], deg: 180.0 - 90.0 }).unwrap(); // l2 runs p2->p0
    let r = s.solve();
    assert_eq!(r.status, Status::FullyConstrained, "{r:?}");
    let q = s.point(p[2]).unwrap();
    assert!((q[0].abs() < 1e-8) && ((q[1] - 3.0).abs() < 1e-8) || (q[0].abs() < 1e-8) && ((q[1] + 3.0).abs() < 1e-8), "{q:?}");
}

#[test]
fn tangent_line_to_circle_and_circle_circle() {
    let mut s = Sketch::default();
    let (c, cp) = s.add_circle(0.0, 0.0, 2.0);
    let a = s.add_point(-3.0, 2.5);
    let b = s.add_point(3.0, 2.2);
    let l = s.add_line(a, b);
    s.add_constraint(C::Fixed { p: cp, x: 0.0, y: 0.0 }).unwrap();
    s.add_constraint(C::Radius { e: c, d: 2.0 }).unwrap();
    s.add_constraint(C::Horizontal { line: l }).unwrap();
    s.add_constraint(C::Tangent { a: l, b: c }).unwrap();
    let r = s.solve();
    assert!(r.max_residual < 1e-9, "{r:?}");
    assert!((s.point(a).unwrap()[1].abs() - 2.0).abs() < 1e-8);
    // Second circle tangent to the first, radius 1, centre held on the x axis.
    let (c2, cp2) = s.add_circle(3.6, 0.2, 1.0);
    s.add_constraint(C::Tangent { a: c, b: c2 }).unwrap();
    s.add_constraint(C::Radius { e: c2, d: 1.0 }).unwrap();
    s.add_constraint(C::Fixed { p: cp2, x: 3.0, y: 0.0 }).unwrap();
    let r = s.solve();
    assert!(r.max_residual < 1e-9, "{r:?}");
}

#[test]
fn drag_keeps_constraints_and_follows_the_cursor() {
    let (mut s, p, l) = rough_rect();
    for (i, id) in l.iter().enumerate() {
        s.add_constraint(if i % 2 == 0 { C::Horizontal { line: *id } } else { C::Vertical { line: *id } }).unwrap();
    }
    s.solve();
    let r = s.drag(&[(p[2], [14.0, 8.0])]);
    assert!(r.violated.is_empty());
    let q = s.point(p[2]).unwrap();
    assert!((q[0] - 14.0).abs() < 0.05 && (q[1] - 8.0).abs() < 0.05, "{q:?}");
    // The rectangle stayed a rectangle.
    assert!((s.point(p[1]).unwrap()[0] - q[0]).abs() < 1e-8 && (s.point(p[3]).unwrap()[1] - q[1]).abs() < 1e-8);
}

#[test]
fn profile_with_circle_hole_and_arc_slot_outline() {
    let (mut s, p, l) = rough_rect();
    for (i, id) in l.iter().enumerate() {
        s.add_constraint(if i % 2 == 0 { C::Horizontal { line: *id } } else { C::Vertical { line: *id } }).unwrap();
    }
    s.add_constraint(C::Fixed { p: p[0], x: 0.0, y: 0.0 }).unwrap();
    s.add_constraint(C::Length { line: l[0], d: 12.0 }).unwrap();
    s.add_constraint(C::Length { line: l[1], d: 4.0 }).unwrap();
    s.add_circle(6.0, 2.0, 0.5);
    s.solve();
    let pr = s.profile().unwrap();
    assert_eq!(pr.holes, vec![Shape::Circle { cx: 6.0, cy: 2.0, r: 0.5 }]);
    // A circle sticking out of the outline is a separate region -> error.
    s.add_circle(11.9, 2.0, 0.5);
    assert!(s.profile().unwrap_err().contains("separate"));
}

#[test]
fn open_profile_and_branch_are_reported() {
    let mut s = Sketch::default();
    let p = [s.add_point(0.0, 0.0), s.add_point(1.0, 0.0), s.add_point(1.0, 1.0)];
    s.add_line(p[0], p[1]);
    s.add_line(p[1], p[2]);
    assert!(s.profile().unwrap_err().contains("open"));
}

#[test]
fn arc_in_outline_becomes_polygon_and_remove_entity_cascades() {
    // D-shape: line from (0,-1) to (0,1) closed by a half circle arc about the origin.
    let mut s = Sketch::default();
    let c = s.add_point(0.0, 0.0);
    let a = s.add_point(0.0, -1.0);
    let b = s.add_point(0.0, 1.0);
    let line = s.add_line(b, a);
    let arc = s.add_arc(c, a, b);
    let pr = s.profile().unwrap();
    let Shape::Polygon { pts } = pr.outer else { panic!("expected polygon") };
    let area = 0.5 * (0..pts.len()).map(|i| pts[i][0] * pts[(i + 1) % pts.len()][1] - pts[i][1] * pts[(i + 1) % pts.len()][0]).sum::<f64>();
    assert!((area - std::f64::consts::FRAC_PI_2).abs() < 0.01, "area {area}");
    s.add_constraint(C::Vertical { line }).unwrap();
    s.add_constraint(C::Radius { e: arc, d: 1.0 }).unwrap();
    s.remove_entity(a);
    assert!(s.entity(line).is_none() && s.entity(arc).is_none());
    assert!(s.constraints.is_empty());
}

#[test]
fn json_round_trip_keeps_ids_and_constraints() {
    let (mut s, _p, l) = rough_rect();
    s.add_constraint(C::Length { line: l[0], d: 3.0 }).unwrap();
    let text = serde_json::to_string(&s).unwrap();
    let back: Sketch = serde_json::from_str(&text).unwrap();
    assert_eq!(s, back);
}

mod infer {
    use fea_problem::sketch::infer::{Rule, Tolerance};
    use fea_problem::sketch::{Constraint as C, Sketch};

    fn rules(s: &Sketch, id: u32) -> Vec<Rule> {
        s.suggest(id, &Tolerance::for_extent(10.0)).into_iter().map(|x| x.rule).collect()
    }

    #[test]
    fn nearly_horizontal_line_and_nearby_endpoint() {
        let mut s = Sketch::default();
        let a = s.add_point(0.0, 0.0);
        let b = s.add_point(5.0, 0.05);
        let l = s.add_line(a, b);
        let c = s.add_point(5.05, 0.0);
        let r = rules(&s, l);
        assert!(r.contains(&Rule::Horizontal) && !r.contains(&Rule::Vertical), "{r:?}");
        assert!(r.contains(&Rule::Coincident), "endpoint b is near the loose point");
        let _ = c;
    }

    #[test]
    fn parallel_perpendicular_equal_between_lines() {
        let mut s = Sketch::default();
        let p = [s.add_point(0.0, 0.0), s.add_point(4.0, 1.0), s.add_point(0.0, 3.0), s.add_point(4.0, 4.05), s.add_point(0.0, 8.0), s.add_point(-1.0, 12.0)];
        let l1 = s.add_line(p[0], p[1]);
        s.add_line(p[2], p[3]);
        let perp = s.add_line(p[4], p[5]);
        let sug = s.suggest(l1, &Tolerance::for_extent(10.0));
        assert!(sug.iter().any(|x| x.rule == Rule::Parallel));
        assert!(sug.iter().any(|x| x.rule == Rule::Perpendicular && x.constraint == C::Perpendicular { a: l1, b: perp }));
        // Best suggestion is the one satisfied most exactly.
        assert!(sug.windows(2).all(|w| w[0].score >= w[1].score));
    }

    #[test]
    fn existing_constraints_are_not_suggested_again_and_apply_enforces() {
        let mut s = Sketch::default();
        let a = s.add_point(0.0, 0.0);
        let b = s.add_point(5.0, 0.1);
        let l = s.add_line(a, b);
        let sug = s.suggest(l, &Tolerance::for_extent(10.0));
        let h = sug.iter().find(|x| x.rule == Rule::Horizontal).unwrap().clone();
        s.apply(&h).unwrap();
        assert!((s.point(b).unwrap()[1] - s.point(a).unwrap()[1]).abs() < 1e-9);
        assert!(!rules(&s, l).contains(&Rule::Horizontal));
    }

    #[test]
    fn tangent_line_to_circle_is_found() {
        let mut s = Sketch::default();
        let (c, _) = s.add_circle(0.0, 0.0, 2.0);
        let a = s.add_point(-4.0, 2.01);
        let b = s.add_point(4.0, 2.0);
        s.add_line(a, b);
        let r = rules(&s, c);
        assert!(r.contains(&Rule::Tangent), "{r:?}");
    }

    #[test]
    fn far_geometry_suggests_nothing() {
        let mut s = Sketch::default();
        let p = [s.add_point(0.0, 0.0), s.add_point(3.0, 2.0), s.add_point(10.0, 10.0), s.add_point(14.0, 17.0)];
        let l = s.add_line(p[0], p[1]);
        s.add_line(p[2], p[3]);
        assert!(rules(&s, l).is_empty());
    }
}

mod history {
    use fea_problem::history::{FeatureKind, History, Plane};
    use fea_problem::sketch::{Constraint as C, Sketch};
    use fea_problem::{build, templates, Analysis, Geometry};

    /// A constrained w x h rectangle with a Ø hole in its middle.
    fn plate(w: f64, h: f64) -> Sketch {
        let mut s = Sketch::default();
        let p = [s.add_point(0.1, -0.1), s.add_point(w + 0.3, 0.2), s.add_point(w - 0.2, h + 0.1), s.add_point(-0.2, h - 0.3)];
        let l = [s.add_line(p[0], p[1]), s.add_line(p[1], p[2]), s.add_line(p[2], p[3]), s.add_line(p[3], p[0])];
        for (i, id) in l.iter().enumerate() {
            s.add_constraint(if i % 2 == 0 { C::Horizontal { line: *id } } else { C::Vertical { line: *id } }).unwrap();
        }
        s.add_constraint(C::Fixed { p: p[0], x: 0.0, y: 0.0 }).unwrap();
        s.add_constraint(C::Length { line: l[0], d: w }).unwrap();
        s.add_constraint(C::Length { line: l[1], d: h }).unwrap();
        s.add_circle(0.5 * w, 0.5 * h, 0.25);
        s
    }

    fn problem(plane: Plane, make: impl Fn(u32) -> FeatureKind) -> fea_problem::Problem {
        let mut p = templates::templates()[0].1.clone();
        let mut h = History::default();
        let sk = h.add("Sketch1", FeatureKind::Sketch { plane, sketch: plate(6.0, 2.0) });
        h.add("Body", make(sk));
        p.history = Some(h);
        p.apply_history().unwrap();
        p
    }

    #[test]
    fn extrude_rebuilds_geometry_and_meshes_in_place() {
        let p = problem(Plane::Front { offset: 0.0 }, |sk| FeatureKind::Extrude { profile: sk, depth: 1.5, layers: 3 });
        assert_eq!(p.analysis, Analysis::Solid);
        let Geometry::Sketch { depth, layers, holes, .. } = &p.geometry else { panic!() };
        assert_eq!((*depth, *layers, holes.len()), (1.5, 3, 1));
        let mut q = p.clone();
        q.mesh.size = 0.6;
        let m = build::sketch_mesh(&q, None).unwrap();
        let zmax = m.nodes.iter().map(|n| n[2]).fold(f64::MIN, f64::max);
        assert!((zmax - 1.5).abs() < 1e-9);
        assert!(m.surfaces.contains_key("hole1") && m.surfaces.contains_key("bottom") && m.surfaces.contains_key("start"));
    }

    #[test]
    fn top_plane_places_the_part_along_plus_y_and_names_survive() {
        let mut p = problem(Plane::Top { offset: 1.0 }, |sk| FeatureKind::Extrude { profile: sk, depth: 2.0, layers: 2 });
        p.mesh.size = 0.7;
        let m = build::sketch_mesh(&p, None).unwrap();
        let (lo, hi) = m.nodes.iter().fold(([f64::MAX; 3], [f64::MIN; 3]), |(mut lo, mut hi), n| {
            for a in 0..3 {
                lo[a] = lo[a].min(n[a]);
                hi[a] = hi[a].max(n[a]);
            }
            (lo, hi)
        });
        // Sketch x -> X; sketch y (0..2) -> -Z; normal -> +Y from offset 1 to 3.
        assert!((lo[1] - 1.0).abs() < 1e-9 && (hi[1] - 3.0).abs() < 1e-9, "{lo:?} {hi:?}");
        assert!((hi[0] - 6.0).abs() < 1e-9 && (lo[2] + 2.0).abs() < 1e-9 && hi[2].abs() < 1e-9, "{lo:?} {hi:?}");
        assert!(m.surfaces.contains_key("hole1"));
    }

    #[test]
    fn revolve_makes_a_solid_of_revolution() {
        // Profile: the 6x2 rectangle moved to x >= 2 would be needed; use x in 0..6 with the hole removed.
        let mut p = templates::templates()[0].1.clone();
        let mut h = History::default();
        let mut s = plate(6.0, 2.0);
        let circle = s.entities.iter().find(|e| matches!(e.v, fea_problem::sketch::Entity::Circle { .. })).unwrap().id;
        s.remove_entity(circle);
        let sk = h.add("Sketch1", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: s });
        h.add("Revolve", FeatureKind::Revolve { profile: sk, angle_deg: 180.0, segments: 6 });
        p.history = Some(h);
        p.apply_history().unwrap();
        p.mesh.size = 1.0;
        let m = build::sketch_mesh(&p, None).unwrap();
        let (zmin, zmax) = m.nodes.iter().fold((f64::MAX, f64::MIN), |(a, b), n| (a.min(n[2]), b.max(n[2])));
        assert!(zmin.abs() < 1e-9 && (zmax - 6.0).abs() < 1e-6, "half turn about y: z in [0, r_max], got {zmin} {zmax}");
    }

    #[test]
    fn rollback_suppress_and_errors() {
        let mut p = problem(Plane::Front { offset: 0.0 }, |sk| FeatureKind::Extrude { profile: sk, depth: 1.0, layers: 2 });
        let h = p.history.as_mut().unwrap();
        // Roll back to just the sketch: it is a plane region again.
        h.rollback = Some(1);
        let b = h.rebuild().unwrap();
        assert_eq!(b.body, fea_problem::history::Body::Region);
        h.rollback = None;
        // An extrude with no depth is refused with a message, not a panic.
        let FeatureKind::Extrude { depth, .. } = &mut h.features[1].kind else { panic!() };
        *depth = 0.0;
        assert!(h.rebuild().unwrap_err().contains("positive depth"));
        // Suppressing the sketch the extrude needs is reported.
        *h = History { features: h.features.clone(), ..Default::default() };
        h.features[0].suppressed = true;
        assert!(h.rebuild().is_err());
    }

    #[test]
    fn plane_problem_on_the_top_plane_is_refused_and_old_files_still_load() {
        let mut p = templates::templates()[0].1.clone();
        let mut h = History::default();
        h.add("S", FeatureKind::Sketch { plane: Plane::Top { offset: 0.0 }, sketch: plate(4.0, 2.0) });
        p.history = Some(h);
        assert!(p.apply_history().unwrap_err().contains("Front plane"));
        // A file without the history field (written before it existed) loads with None; with none, JSON omits it.
        let plain = templates::templates()[0].1.to_json().unwrap();
        assert!(!plain.contains("history"));
        assert!(fea_problem::Problem::from_json(&plain).unwrap().history.is_none());
        // And a history round-trips.
        let mut q = problem(Plane::Right { offset: 0.0 }, |sk| FeatureKind::Extrude { profile: sk, depth: 1.0, layers: 2 });
        q.name = "x".into();
        let back = fea_problem::Problem::from_json(&q.to_json().unwrap()).unwrap();
        assert_eq!(back, q);
    }

    #[test]
    fn datum_plane_frame_is_orthonormal_and_rejects_degenerate_input() {
        let d = Plane::Datum { origin: [1.0, 2.0, 3.0], normal: [0.0, 1.0, 1.0], x_dir: [1.0, 0.0, 0.0] }.frame().unwrap();
        let dotp = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        assert!(dotp(d.u, d.v).abs() < 1e-12 && dotp(d.u, d.w()).abs() < 1e-12 && (dotp(d.w(), d.w()) - 1.0).abs() < 1e-12);
        assert!(Plane::Datum { origin: [0.0; 3], normal: [0.0, 0.0, 1.0], x_dir: [0.0, 0.0, 2.0] }.frame().is_err());
        assert!(Plane::Datum { origin: [0.0; 3], normal: [0.0; 3], x_dir: [1.0, 0.0, 0.0] }.frame().is_err());
    }
}

mod convert {
    use fea_problem::history::History;
    use fea_problem::{build, templates, Analysis, Geometry, Shape};

    /// Every template round-trips: history -> geometry reproduces the same outline, holes and names.
    #[test]
    fn templates_convert_to_a_history_that_rebuilds_the_same_geometry() {
        for (title, p) in templates::templates() {
            let Geometry::Sketch { outer, holes, .. } = &p.geometry else { continue };
            if matches!(outer, Shape::Slot { .. }) || holes.iter().any(|h| matches!(h, Shape::Slot { .. })) {
                continue; // slots come back as polygons (see below)
            }
            let mut q = p.clone();
            q.history = Some(History::from_problem(&p).unwrap());
            q.apply_history().unwrap_or_else(|e| panic!("{title}: {e}"));
            assert_eq!(q.geometry, p.geometry, "{title}");
            assert_eq!(q.analysis, p.analysis, "{title}");
            assert_eq!(q.edge_names(), p.edge_names(), "{title}: supports and loads keep their names");
        }
    }

    #[test]
    fn slot_converts_to_a_polygon_outline_of_the_same_area() {
        let mut p = templates::templates()[0].1.clone();
        p.geometry = Geometry::Sketch { outer: Shape::Slot { cx: 3.0, cy: 1.0, length: 6.0, width: 2.0, angle: 20.0 }, holes: vec![], depth: 1.0, layers: 2 };
        let area_before = build::region_of(&p).unwrap().area();
        p.history = Some(History::from_problem(&p).unwrap());
        p.apply_history().unwrap();
        let area_after = build::region_of(&p).unwrap().area();
        assert!((area_after / area_before - 1.0).abs() < 5e-3, "{area_before} vs {area_after}");
    }

    #[test]
    fn imported_mesh_has_no_history() {
        let mut p = templates::templates()[0].1.clone();
        p.geometry = Geometry::Imported { path: "x.msh".into() };
        assert!(History::from_problem(&p).is_err());
        p.analysis = Analysis::PlaneStress;
    }
}

mod names {
    use fea_problem::build;
    use fea_problem::history::{FeatureKind, History, Plane};
    use fea_problem::sketch::{Constraint as C, Sketch};

    fn names_of(s: &Sketch, id: u32) -> Vec<String> {
        s.surface_names().unwrap().into_iter().filter(|(e, _)| *e == id).map(|(_, n)| n).collect()
    }

    #[test]
    fn rectangle_lines_and_circle_get_the_mesh_surface_names() {
        let mut s = Sketch::default();
        let p = [s.add_point(0.0, 0.0), s.add_point(6.0, 0.0), s.add_point(6.0, 2.0), s.add_point(0.0, 2.0)];
        let l = [s.add_line(p[0], p[1]), s.add_line(p[1], p[2]), s.add_line(p[2], p[3]), s.add_line(p[3], p[0])];
        let (c, _) = s.add_circle(3.0, 1.0, 0.4);
        assert_eq!(names_of(&s, l[0]), ["bottom"]);
        assert_eq!(names_of(&s, l[1]), ["right"]);
        assert_eq!(names_of(&s, l[2]), ["top"]);
        assert_eq!(names_of(&s, l[3]), ["left"]);
        assert_eq!(names_of(&s, c), ["hole1"]);
    }

    #[test]
    fn rectangle_drawn_from_the_other_corner_keeps_geometric_names() {
        // Lowest-id edge is the TOP one here: names follow the geometry, not the walk start.
        let mut s = Sketch::default();
        let p = [s.add_point(6.0, 2.0), s.add_point(0.0, 2.0), s.add_point(0.0, 0.0), s.add_point(6.0, 0.0)];
        let l = [s.add_line(p[0], p[1]), s.add_line(p[1], p[2]), s.add_line(p[2], p[3]), s.add_line(p[3], p[0])];
        assert_eq!(names_of(&s, l[0]), ["top"]);
        assert_eq!(names_of(&s, l[2]), ["bottom"]);
    }

    #[test]
    fn polygon_edges_are_numbered_in_walk_order_and_match_the_mesher() {
        let mut s = Sketch::default();
        let p = [s.add_point(0.0, 0.0), s.add_point(5.0, 0.0), s.add_point(5.0, 1.0), s.add_point(2.0, 1.0), s.add_point(2.0, 3.0), s.add_point(0.0, 3.0)];
        let l: Vec<u32> = (0..6).map(|i| s.add_line(p[i], p[(i + 1) % 6])).collect();
        for (i, id) in l.iter().enumerate() {
            assert_eq!(names_of(&s, *id), [format!("edge{}", i + 1)]);
        }
        // The names are real surfaces of the mesh built from this sketch.
        let mut prob = fea_problem::templates::templates()[0].1.clone();
        let mut h = History::default();
        h.add("S", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: s.clone() });
        prob.history = Some(h);
        prob.apply_history().unwrap();
        prob.mesh.size = 0.5;
        let mesh = build::sketch_mesh(&prob, None).unwrap();
        for (_, n) in s.surface_names().unwrap() {
            assert!(mesh.surfaces.contains_key(&n), "{n} missing from {:?}", mesh.surfaces.keys().collect::<Vec<_>>());
        }
    }

    #[test]
    fn arc_contributes_several_polygon_edges() {
        let mut s = Sketch::default();
        let c = s.add_point(0.0, 0.0);
        let a = s.add_point(0.0, -1.0);
        let b = s.add_point(0.0, 1.0);
        let line = s.add_line(b, a);
        let arc = s.add_arc(c, a, b);
        s.add_constraint(C::Vertical { line });
        assert_eq!(names_of(&s, line).len(), 1);
        assert!(names_of(&s, arc).len() > 10);
    }
}

mod arcs {
    use fea_problem::sketch::Sketch;
    use fea_problem::Shape;

    /// The polygon of a profile that contains an arc passes through BOTH arc end points (each edge contributes its start
    /// point and the next edge starts where this one ends), in either walk direction.
    #[test]
    fn arc_end_points_are_polygon_vertices_in_both_directions() {
        for reversed in [false, true] {
            let mut s = Sketch::default();
            let c = s.add_point(0.0, 0.0);
            let a = s.add_point(1.0, 0.0);
            let b = s.add_point(0.0, 1.0);
            // Quarter-disc: arc a -> b (ccw) closed by two radii. Create the arc first or last to flip the walk.
            let (arc, l1, l2);
            if reversed {
                l1 = s.add_line(b, c);
                l2 = s.add_line(c, a);
                arc = s.add_arc(c, a, b);
            } else {
                arc = s.add_arc(c, a, b);
                l1 = s.add_line(b, c);
                l2 = s.add_line(c, a);
            }
            let _ = (arc, l1, l2);
            let Shape::Polygon { pts } = s.profile().unwrap().outer else { panic!("polygon") };
            let has = |p: [f64; 2]| pts.iter().any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) < 1e-9);
            assert!(has([1.0, 0.0]) && has([0.0, 1.0]) && has([0.0, 0.0]), "reversed={reversed}: {pts:?}");
            let area = 0.5 * (0..pts.len()).map(|i| pts[i][0] * pts[(i + 1) % pts.len()][1] - pts[i][1] * pts[(i + 1) % pts.len()][0]).sum::<f64>().abs();
            assert!((area - std::f64::consts::FRAC_PI_4).abs() < 5e-3, "area {area}");
            // No duplicate consecutive vertices (a duplicate would make a zero-length mesh edge).
            assert!(pts.windows(2).all(|w| (w[0][0] - w[1][0]).hypot(w[0][1] - w[1][1]) > 1e-9), "{pts:?}");
        }
    }
}
