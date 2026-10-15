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
    let e = s.profile().unwrap_err();
    assert!(e.contains("cross"), "{e}");
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
fn arc_in_outline_stays_an_exact_arc_and_remove_entity_cascades() {
    // D-shape: line from (0,-1) to (0,1) closed by a half circle arc about the origin.
    let mut s = Sketch::default();
    let c = s.add_point(0.0, 0.0);
    let a = s.add_point(0.0, -1.0);
    let b = s.add_point(0.0, 1.0);
    let line = s.add_line(b, a);
    let arc = s.add_arc(c, a, b);
    let pr = s.profile().unwrap();
    let Shape::Outline { segs } = &pr.outer else { panic!("expected an outline, got {:?}", pr.outer) };
    assert_eq!(segs.len(), 2, "one edge per entity, not 48 per turn");
    let (lo, hi) = pr.outer.bounds();
    assert!((lo[0]).abs() < 1e-12 && (hi[0] - 1.0).abs() < 1e-12 && (lo[1] + 1.0).abs() < 1e-12 && (hi[1] - 1.0).abs() < 1e-12, "{lo:?} {hi:?}: the arc's extreme x = 1 is not a vertex");
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
        p.geometry = Geometry::Sketch { outer: Shape::Slot { cx: 3.0, cy: 1.0, length: 6.0, width: 2.0, angle: 20.0 }, holes: vec![], depth: 1.0, layers: 2, regions: Vec::new() };
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
    fn an_arc_is_one_named_edge_and_the_mesh_has_it_on_the_exact_circle() {
        let mut s = Sketch::default();
        let c = s.add_point(0.0, 0.0);
        let a = s.add_point(0.0, -1.0);
        let b = s.add_point(0.0, 1.0);
        let line = s.add_line(b, a);
        let arc = s.add_arc(c, a, b);
        s.add_constraint(C::Vertical { line });
        assert_eq!(names_of(&s, line), ["edge1"]);
        assert_eq!(names_of(&s, arc), ["edge2"], "an arc is one surface");
        let mut prob = fea_problem::templates::templates()[0].1.clone();
        let mut h = History::default();
        h.add("S", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: s.clone() });
        prob.history = Some(h);
        prob.apply_history().unwrap();
        prob.mesh.size = 0.2;
        assert_eq!(prob.edge_names()[..2], ["edge1", "edge2"]);
        let mesh = build::sketch_mesh(&prob, None).unwrap();
        let on_arc = &mesh.surfaces["edge2"];
        assert!(!on_arc.is_empty());
        let nodes: std::collections::BTreeSet<usize> = on_arc.iter().flat_map(|f| f.iter().copied()).collect();
        for n in nodes {
            let p = mesh.nodes[n];
            assert!((p[0].hypot(p[1]) - 1.0).abs() < 1e-9, "node {n} at {p:?} is off the unit circle");
        }
    }
}

mod arcs {
    use fea_problem::sketch::Sketch;
    use fea_problem::Shape;

    /// The outline of a profile that contains an arc is continuous through BOTH arc end points in either walk direction.
    #[test]
    fn arc_outline_is_continuous_in_both_directions() {
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
            let Shape::Outline { segs } = s.profile().unwrap().outer else { panic!("outline") };
            assert_eq!(segs.len(), 3);
            // Walk-continuous in both directions: every edge ends where the next begins, whichever way it is stored.
            for i in 0..3 {
                let (e, n) = (segs[i].end(), segs[(i + 1) % 3].start());
                assert!((e[0] - n[0]).hypot(e[1] - n[1]) < 1e-9, "reversed={reversed}: edge {i} ends at {e:?}, next starts at {n:?}");
            }
            let pts: Vec<[f64; 2]> = segs.iter().flat_map(|sg| sg.sample(480.0)).collect();
            let has = |p: [f64; 2]| pts.iter().any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) < 1e-9);
            assert!(has([1.0, 0.0]) && has([0.0, 1.0]) && has([0.0, 0.0]), "reversed={reversed}");
            let area = 0.5 * (0..pts.len()).map(|i| pts[i][0] * pts[(i + 1) % pts.len()][1] - pts[i][1] * pts[(i + 1) % pts.len()][0]).sum::<f64>().abs();
            assert!((area - std::f64::consts::FRAC_PI_4).abs() < 1e-4, "area {area}");
        }
    }
}

#[test]
fn construction_geometry_drives_the_solve_but_is_not_in_the_profile() {
    let (mut s, p, l) = rough_rect();
    for (i, id) in l.iter().enumerate() {
        s.add_constraint(if i % 2 == 0 { C::Horizontal { line: *id } } else { C::Vertical { line: *id } }).unwrap();
    }
    s.add_constraint(C::Fixed { p: p[0], x: 0.0, y: 0.0 }).unwrap();
    s.add_constraint(C::Length { line: l[0], d: 12.0 }).unwrap();
    // A diagonal through two corners branches the outline until it is construction.
    let diag = s.add_line(p[0], p[2]);
    assert!(s.profile().unwrap_err().contains("more than two edges"));
    s.set_construction(diag, true).unwrap();
    s.add_constraint(C::Length { line: diag, d: 13.0 }).unwrap(); // 12 x 5 -> diagonal 13
    s.solve();
    let pr = s.profile().unwrap();
    assert_eq!(pr.outer, Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 5.0 }, "the diagonal dimension sized the rectangle");
    // A reference circle is no hole; unmarking it makes it one.
    let (c, _) = s.add_circle(6.0, 2.5, 1.0);
    s.set_construction(c, true).unwrap();
    assert!(s.profile().unwrap().holes.is_empty());
    s.set_construction(c, false).unwrap();
    assert_eq!(s.profile().unwrap().holes.len(), 1);
    // Points cannot be construction; deleting the entity forgets the flag; the flag survives JSON.
    assert!(s.set_construction(p[0], true).is_err() && s.set_construction(999, true).is_err());
    let back: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
    assert!(back.is_construction(diag));
    let mut t = back;
    t.remove_entity(diag);
    assert!(t.construction.is_empty());
}

#[test]
fn an_outline_with_arcs_round_trips_through_a_sketch_and_solves_on_its_arc_edge() {
    use fea_problem::{solve, templates, Geometry, Load, Seg, Support};
    use std::f64::consts::FRAC_PI_2;
    // A D-shape: the straight edge x = 0 and a half circle of radius 2 bulging to +x (clockwise walk, then counter-clockwise).
    let outline = Shape::Outline {
        segs: vec![Seg::Line { a: [0.0, -2.0], b: [0.0, 2.0] }, Seg::Arc { c: [0.0, 0.0], r: 2.0, a0: FRAC_PI_2, a1: -FRAC_PI_2 }],
    };
    let sk = Sketch::from_shapes(&outline, &[]);
    let back = sk.profile().unwrap().outer;
    let Shape::Outline { segs } = &back else { panic!("{back:?}") };
    assert_eq!(segs.len(), 2);
    let (lo, hi) = back.bounds();
    assert!((hi[0] - 2.0).abs() < 1e-12 && lo[0].abs() < 1e-12 && (hi[1] - 2.0).abs() < 1e-12, "{lo:?} {hi:?}");
    // Mesh and solve: held on the straight edge, pulled along +x on the arc edge. Reaction balances the load.
    let mut p = templates::blank();
    p.history = None;
    p.geometry = Geometry::Sketch { outer: outline, holes: vec![], depth: 1.0, layers: 1, regions: Vec::new() };
    p.mesh.size = 0.25;
    p.thickness = 1.0;
    let names = p.edge_names();
    assert_eq!(names[..2], ["edge1", "edge2"]);
    p.supports = vec![Support::fixed("edge1")];
    p.loads = vec![Load::Force { edge: "edge2".into(), fx: 100.0, fy: 0.0, fz: 0.0 }];
    let s = solve(&p, None).unwrap();
    assert!((s.summary.reaction[0] + 100.0).abs() < 0.5, "{:?}", s.summary.reaction);
    assert!(s.summary.equilibrium_error < 1e-6, "{}", s.summary.equilibrium_error);
}

mod faces {
    use fea_problem::history::{Body, FeatureKind, History, Plane};
    use fea_problem::sketch::Sketch;
    use fea_problem::{Shape, Seg};

    fn history(outer: &Shape, depth: f64, plane: Plane) -> History {
        let mut h = History::default();
        let sk = h.add("S", FeatureKind::Sketch { plane, sketch: Sketch::from_shapes(outer, &[]) });
        h.add("E", FeatureKind::Extrude { profile: sk, depth, layers: 2 });
        h
    }
    fn normal(p: &Plane) -> [f64; 3] {
        p.frame().unwrap().w()
    }
    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }

    #[test]
    fn a_box_has_six_outward_planes_through_its_faces() {
        let h = history(&Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 4.0 }, 3.0, Plane::Front { offset: 0.0 });
        let faces = h.planar_faces().unwrap();
        let by = |n: &str| faces.iter().find(|f| f.name == n).unwrap_or_else(|| panic!("{n}")).plane.clone();
        assert_eq!(faces.len(), 6);
        for (name, n, through) in [
            ("start", [0.0, 0.0, -1.0], [6.0, 2.0, 0.0]),
            ("end", [0.0, 0.0, 1.0], [6.0, 2.0, 3.0]),
            ("bottom", [0.0, -1.0, 0.0], [6.0, 0.0, 1.5]),
            ("top", [0.0, 1.0, 0.0], [6.0, 4.0, 1.5]),
            ("left", [-1.0, 0.0, 0.0], [0.0, 2.0, 1.5]),
            ("right", [1.0, 0.0, 0.0], [12.0, 2.0, 1.5]),
        ] {
            let pl = by(name);
            assert!(close(normal(&pl), n), "{name}: normal {:?}", normal(&pl));
            // The face centre lies in the plane: (centre - origin) . normal = 0.
            let fr = pl.frame().unwrap();
            let d: f64 = (0..3).map(|i| (through[i] - fr.origin[i]) * n[i]).sum();
            assert!(d.abs() < 1e-9, "{name}: the face is {d} off its plane");
        }
    }

    #[test]
    fn a_clockwise_outline_and_a_plane_off_the_origin_still_face_outward() {
        // The same outline drawn clockwise, on the Top plane offset 2: every side normal must still point away from the body.
        let outline = Shape::Polygon { pts: vec![[0.0, 0.0], [0.0, 3.0], [5.0, 3.0], [5.0, 0.0]] };
        let h = history(&outline, 1.0, Plane::Top { offset: 2.0 });
        let built = h.rebuild().unwrap();
        assert!(matches!(built.body, Body::Extrude { .. }));
        let centre = built.frame.apply([2.5, 1.5, 0.5]);
        for f in h.planar_faces().unwrap() {
            let fr = f.plane.frame().unwrap();
            let n = fr.w();
            let out: f64 = (0..3).map(|i| (fr.origin[i] - centre[i]) * n[i]).sum();
            assert!(out > 0.0, "{}: normal {n:?} points into the body", f.name);
        }
    }

    #[test]
    fn arcs_are_not_flat_and_a_revolve_has_no_planar_faces() {
        let d = Shape::Outline { segs: vec![Seg::Line { a: [0.0, -2.0], b: [0.0, 2.0] }, Seg::Arc { c: [0.0, 0.0], r: 2.0, a0: std::f64::consts::FRAC_PI_2, a1: -std::f64::consts::FRAC_PI_2 }] };
        let names: Vec<String> = history(&d, 1.0, Plane::Front { offset: 0.0 }).planar_faces().unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["start", "end", "edge1"], "the arc edge2 is a curved face");
        let mut h = History::default();
        let sk = h.add("S", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: Sketch::from_shapes(&Shape::Rect { x0: 1.0, y0: 0.0, x1: 2.0, y1: 1.0 }, &[]) });
        h.add("R", FeatureKind::Revolve { profile: sk, angle_deg: 360.0, segments: 8 });
        assert!(h.planar_faces().unwrap().is_empty());
    }
}

mod regions {
    use fea_problem::history::{FeatureKind, History, Plane};
    use fea_problem::sketch::{Constraint as C, Sketch};
    use fea_problem::{build, solve, templates, Load, Shape, Support};

    fn rect(s: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) -> [u32; 4] {
        let p = [s.add_point(x0, y0), s.add_point(x1, y0), s.add_point(x1, y1), s.add_point(x0, y1)];
        let l = [s.add_line(p[0], p[1]), s.add_line(p[1], p[2]), s.add_line(p[2], p[3]), s.add_line(p[3], p[0])];
        for (i, id) in l.iter().enumerate() {
            s.add_constraint(if i % 2 == 0 { C::Horizontal { line: *id } } else { C::Vertical { line: *id } }).unwrap();
        }
        l
    }

    fn problem_of(s: Sketch) -> fea_problem::Problem {
        let mut p = templates::blank();
        let mut h = History::default();
        h.add("S", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: s });
        p.history = Some(h);
        p.apply_history().unwrap();
        p.mesh.size = 0.4;
        p
    }

    #[test]
    fn an_island_in_a_hole_is_its_own_region_with_prefixed_names() {
        let mut s = Sketch::default();
        rect(&mut s, 0.0, 0.0, 10.0, 10.0);
        let (hole, _) = s.add_circle(5.0, 5.0, 3.0);
        let (island, _) = s.add_circle(5.0, 5.0, 1.0);
        let pr = s.profile().unwrap();
        assert_eq!(pr.outer, Shape::Rect { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 });
        assert_eq!(pr.holes, vec![Shape::Circle { cx: 5.0, cy: 5.0, r: 3.0 }]);
        assert_eq!(pr.regions.len(), 1);
        assert_eq!(pr.regions[0].outer, Shape::Circle { cx: 5.0, cy: 5.0, r: 1.0 });
        let names = s.surface_names().unwrap();
        let name_of = |id| names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone());
        assert_eq!(name_of(hole).as_deref(), Some("hole1"));
        assert_eq!(name_of(island).as_deref(), Some("r2/outer"));
        let mut p = problem_of(s);
        assert_eq!(p.edge_names(), ["bottom", "right", "top", "left", "hole1", "r2/outer"]);
        // The mesh has the island as a separate body with its own surface (one block per body: the all-unstructured strategy; a
        // hybrid mesh splits each body into a quadrilateral and a triangle block).
        p.mesh.strategy = fea_problem::MeshStrategy::Unstructured;
        let mesh = build::sketch_mesh(&p, None).unwrap();
        assert!(mesh.surfaces.contains_key("r2/outer") && mesh.surfaces.contains_key("hole1"));
        assert_eq!(mesh.blocks.len(), 2, "plate and island are separate blocks");
        // Four concentric circles: a disc with a hole (first region) holding a disc with a hole (second region).
        let mut t = Sketch::default();
        t.add_circle(0.0, 0.0, 4.0);
        t.add_circle(0.0, 0.0, 3.0);
        t.add_circle(0.0, 0.0, 2.0);
        t.add_circle(0.0, 0.0, 1.0);
        let pr = t.profile().unwrap();
        assert_eq!((pr.holes.len(), pr.regions.len(), pr.regions[0].holes.len()), (1, 1, 1));
    }

    #[test]
    fn two_separate_outlines_are_solved_as_independent_bodies() {
        let mut s = Sketch::default();
        let a = rect(&mut s, 0.0, 0.0, 4.0, 2.0);
        let b = rect(&mut s, 6.0, 0.0, 10.0, 2.0);
        let pr = s.profile().unwrap();
        assert_eq!(pr.regions.len(), 1, "{pr:?}");
        let names = s.surface_names().unwrap();
        let name_of = |id| names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone()).unwrap();
        assert_eq!(name_of(a[3]), "left");
        assert_eq!(name_of(b[3]), "r2/left");
        assert_eq!(name_of(b[1]), "r2/right");
        let mut p = problem_of(s);
        p.analysis = fea_problem::Analysis::PlaneStress;
        p.thickness = 1.0;
        p.supports = vec![Support::fixed("left"), Support::fixed("r2/left")];
        // 100 on the first body's end, 300 on the second's (per unit area).
        p.loads = vec![Load::Traction { edge: "right".into(), tx: 100.0, ty: 0.0, tz: 0.0 }, Load::Traction { edge: "r2/right".into(), tx: 300.0, ty: 0.0, tz: 0.0 }];
        let sol = solve(&p, None).unwrap();
        assert!(sol.summary.equilibrium_error < 1e-6, "{}", sol.summary.equilibrium_error);
        // Total force = (100 + 300) x the 2-high end: reaction balances it.
        assert!((sol.summary.reaction[0] + 800.0).abs() < 1e-6, "{:?}", sol.summary.reaction);
        // Each body stretches by stress x length / E, independently (the free end of each, away from the Poisson-restrained root).
        let mesh = &sol.model.mesh;
        let tip = |x: f64| -> f64 { (0..mesh.nodes.len()).filter(|&n| (mesh.nodes[n][0] - x).abs() < 1e-9).map(|n| sol.u[n * 2]).sum::<f64>() / (0..mesh.nodes.len()).filter(|&n| (mesh.nodes[n][0] - x).abs() < 1e-9).count() as f64 };
        let e = p.material.e;
        let (ua, ub) = (tip(4.0), tip(10.0) - 0.0);
        assert!((ua - 100.0 * 4.0 / e).abs() < 0.03 * 100.0 * 4.0 / e, "{ua} vs {}", 100.0 * 4.0 / e);
        assert!((ub - 300.0 * 4.0 / e).abs() < 0.03 * 300.0 * 4.0 / e, "{ub} vs {}", 300.0 * 4.0 / e);
    }
}

mod splines {
    use fea_problem::history::{FeatureKind, History, Plane};
    use fea_problem::sketch::{Entity, Sketch};
    use fea_problem::{build, templates, Seg, Shape};

    /// A hump over the x axis: a spline through (0,0), (1,1.2), (3,1.5), (4,0), closed by the straight base.
    fn hump() -> (Sketch, u32, u32) {
        let mut s = Sketch::default();
        let p: Vec<u32> = [[0.0, 0.0], [1.0, 1.2], [3.0, 1.5], [4.0, 0.0]].iter().map(|q| s.add_point(q[0], q[1])).collect();
        let spline = s.add_spline(p.clone()).unwrap();
        let base = s.add_line(p[3], p[0]);
        (s, spline, base)
    }

    #[test]
    fn a_spline_interpolates_its_points_and_closes_a_profile_with_one_named_edge() {
        let (s, spline, base) = hump();
        let pr = s.profile().unwrap();
        let Shape::Outline { segs } = &pr.outer else { panic!("{:?}", pr.outer) };
        assert_eq!(segs.len(), 2);
        let Seg::Spline { pts } = &segs[0] else { panic!("{:?}", segs[0]) };
        // Through every given point, exactly, at the span ends.
        for (k, want) in [[0.0, 0.0], [1.0, 1.2], [3.0, 1.5], [4.0, 0.0]].iter().enumerate() {
            let got = segs[0].point(k as f64 / 3.0);
            assert!((got[0] - want[0]).hypot(got[1] - want[1]) < 1e-12, "point {k}: {got:?} vs {want:?}");
        }
        assert_eq!(pts.len(), 4);
        let names = s.surface_names().unwrap();
        assert_eq!(names.iter().find(|(i, _)| *i == spline).unwrap().1, "edge1");
        assert_eq!(names.iter().find(|(i, _)| *i == base).unwrap().1, "edge2");
        // Smooth: no kink at an interior through point (the one-sided tangents agree).
        let h = 1e-6;
        let tangent = |t: f64, dt: f64| {
            let (a, b) = (segs[0].point(t), segs[0].point(t + dt));
            let l = (b[0] - a[0]).hypot(b[1] - a[1]);
            [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
        };
        for k in [1.0, 2.0] {
            let (before, after) = (tangent(k / 3.0 - h, h), tangent(k / 3.0, h));
            assert!((before[0] * after[0] + before[1] * after[1]) > 1.0 - 1e-9, "kink at point {k}");
        }
        // Its bounding box includes the overshoot between the points, not just the through points.
        let (lo, hi) = pr.outer.bounds();
        assert!(hi[1] >= 1.5 && lo[1] <= 0.0 && (hi[0] - 4.0).abs() < 1e-9, "{lo:?} {hi:?}");
    }

    #[test]
    fn a_spline_edge_meshes_on_the_curve_and_has_the_curves_area() {
        let (s, _, _) = hump();
        let mut p = templates::blank();
        let mut h = History::default();
        h.add("S", FeatureKind::Sketch { plane: Plane::Front { offset: 0.0 }, sketch: s.clone() });
        p.history = Some(h);
        p.apply_history().unwrap();
        p.mesh.size = 0.2;
        p.analysis = fea_problem::Analysis::PlaneStress;
        p.thickness = 1.0;
        let mesh = build::sketch_mesh(&p, None).unwrap();
        let on = &mesh.surfaces["edge1"];
        assert!(on.len() > 20, "the curved edge carries many boundary elements");
        // Area of the mesh against the area under the spline (sampled finely).
        let Shape::Outline { segs } = s.profile().unwrap().outer else { panic!() };
        let fine: Vec<[f64; 2]> = (0..=4000).map(|i| segs[0].point(i as f64 / 4000.0)).collect();
        let area = 0.5 * (0..fine.len() - 1).map(|i| fine[i][0] * fine[i + 1][1] - fine[i + 1][0] * fine[i][1]).sum::<f64>().abs();
        let loads = fea_core::Loads { body: Some([1.0, 0.0, 0.0]), ..Default::default() };
        // Plane stress, thickness 1: the total of a unit body force is the area.
        let total: f64 = fea_core::loads::assemble(&mesh, &loads).unwrap().chunks(2).map(|c| c[0]).sum();
        assert!((total - area).abs() < 2e-3 * area, "mesh area {total} vs the area under the spline {area}");
        // Every node of the spline edge lies on the curve to within the piece sag (nodes are on the chain's vertices or chord midpoints).
        let curve: Vec<[f64; 2]> = (0..=2000).map(|i| segs[0].point(i as f64 / 2000.0)).collect();
        let worst = on.iter().flatten().map(|&n| curve.iter().map(|c| (c[0] - mesh.nodes[n][0]).hypot(c[1] - mesh.nodes[n][1])).fold(f64::INFINITY, f64::min)).fold(0.0f64, f64::max);
        assert!(worst < 2e-3, "{worst}");
    }

    #[test]
    fn splines_follow_their_points_and_are_not_sweep_paths() {
        let (mut s, spline, _) = hump();
        // JSON round trip keeps the entity.
        let back: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert!(matches!(back.entity(spline), Some(Entity::Spline { pts }) if pts.len() == 4));
        // A path with a spline is refused with the reason.
        let mut path = Sketch::default();
        let (a, b, c) = (path.add_point(0.0, 0.0), path.add_point(0.0, 1.0), path.add_point(0.5, 2.0));
        path.add_spline(vec![a, b, c]);
        assert!(path.open_path().unwrap_err().contains("spline"));
        // Fewer than two points, or a non-point, make no spline.
        assert!(s.add_spline(vec![0]).is_none() && s.add_spline(vec![0, spline]).is_none());
        // Deleting a through point deletes the spline (and the profile it closed).
        s.remove_entity(1);
        assert!(s.entity(spline).is_none());
    }
}

/// A sketch read from a hand-edited or damaged file names entities that are missing or of the wrong kind. Every
/// entry point must report that, never panic (a panic would take the window down while loading a model).
#[test]
fn a_damaged_sketch_file_is_reported_not_panicked_on() {
    let bad: Vec<(&str, &str)> = vec![
        ("line ending at a missing point", r#"{"entities":[{"id":0,"type":"Point","x":0,"y":0},{"id":1,"type":"Line","a":0,"b":9}]}"#),
        ("line whose end is a line", r#"{"entities":[{"id":0,"type":"Point","x":0,"y":0},{"id":1,"type":"Point","x":1,"y":0},{"id":2,"type":"Line","a":0,"b":1},{"id":3,"type":"Line","a":0,"b":2}]}"#),
        ("radius of a line", r#"{"entities":[{"id":0,"type":"Point","x":0,"y":0},{"id":1,"type":"Point","x":1,"y":0},{"id":2,"type":"Line","a":0,"b":1}],"constraints":[{"id":3,"type":"Radius","e":2,"d":1}]}"#),
        ("constraint on a missing entity", r#"{"entities":[{"id":0,"type":"Point","x":0,"y":0}],"constraints":[{"id":1,"type":"Horizontal","line":7}]}"#),
        ("circle about a line", r#"{"entities":[{"id":0,"type":"Point","x":0,"y":0},{"id":1,"type":"Point","x":1,"y":0},{"id":2,"type":"Line","a":0,"b":1},{"id":3,"type":"Circle","c":2,"r":1}]}"#),
        ("arc with a missing end", r#"{"entities":[{"id":0,"type":"Point","x":0,"y":0},{"id":1,"type":"Point","x":1,"y":0},{"id":2,"type":"Arc","c":0,"s":1,"e":8}]}"#),
        ("spline through a missing point", r#"{"entities":[{"id":0,"type":"Point","x":0,"y":0},{"id":1,"type":"Spline","pts":[0,5]}]}"#),
    ];
    for (what, json) in bad {
        let mut s: Sketch = serde_json::from_str(json).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(s.check().is_err(), "{what}: check must object");
        let r = s.solve();
        assert_eq!(r.status, Status::Conflicting, "{what}: {r:?}");
        let err = s.profile().expect_err(what);
        assert!(err.contains("damaged"), "{what}: {err}");
        for e in s.entities.iter().map(|e| e.id).collect::<Vec<_>>() {
            assert!(s.suggest(e, &fea_problem::sketch::infer::Tolerance::for_extent(1.0)).is_empty(), "{what}: no suggestions from a damaged sketch");
        }
    }
    // The checked sketch of an ordinary drawing is fine.
    let (s, _, _) = rough_rect();
    assert!(s.check().is_ok());
}
