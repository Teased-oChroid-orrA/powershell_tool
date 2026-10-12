//! Mesh-density advice and the refinement zones it applies.
use fea_problem::advice::{apply_region, level_order, refinement_regions, RegionKind, Resolution};
use fea_problem::{build, templates, Geometry, Load, Problem, RefineRegion, Shape};

fn plate() -> Problem {
    let mut p = templates::templates()[0].1.clone();
    p.supports.clear();
    p.loads.clear();
    p.mesh.size = 0.5;
    p
}
fn l_bracket() -> Problem {
    let mut p = plate();
    p.geometry = Geometry::Sketch { outer: Shape::Polygon { pts: vec![[0.0, 0.0], [6.0, 0.0], [6.0, 2.0], [2.0, 2.0], [2.0, 6.0], [0.0, 6.0]] }, holes: vec![], depth: 1.0, layers: 4, regions: Vec::new() };
    p
}
fn nodes_within(p: &Problem, c: [f64; 2], r: f64) -> usize {
    build::sketch_mesh(p, None).unwrap().nodes.iter().filter(|n| (n[0] - c[0]).hypot(n[1] - c[1]) <= r).count()
}

#[test]
fn a_hole_is_a_region_and_a_loaded_hole_is_recommended_a_finer_level() {
    let mut p = plate();
    let r = refinement_regions(&p);
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].kind, r[0].level, r[0].loaded, r[0].applied), (RegionKind::Hole, 2, false, false));
    assert_eq!(r[0].centre, [6.0, 2.0]);
    p.loads.push(Load::Bearing { hole: 1, fx: 100.0, fy: 0.0 });
    let r = refinement_regions(&p);
    let hole = r.iter().find(|x| x.kind == RegionKind::Hole).unwrap();
    assert_eq!((hole.level, hole.loaded), (3, true));
    assert!(hole.reason.contains("pin or bushing"));
}

#[test]
fn only_a_concave_corner_is_re_entrant_and_a_rectangle_has_none() {
    assert!(!refinement_regions(&plate()).iter().any(|r| r.kind == RegionKind::ReentrantCorner));
    let r: Vec<_> = refinement_regions(&l_bracket()).into_iter().filter(|r| r.kind == RegionKind::ReentrantCorner).collect();
    assert_eq!(r.len(), 1, "an L has one inside corner");
    assert_eq!(r[0].centre, [2.0, 2.0]);
    assert_eq!(r[0].level, 3);
    // The same corner either orientation of the outline.
    let mut q = l_bracket();
    if let Geometry::Sketch { outer: Shape::Polygon { pts }, .. } = &mut q.geometry {
        pts.reverse();
    }
    assert_eq!(refinement_regions(&q).iter().filter(|r| r.kind == RegionKind::ReentrantCorner).count(), 1);
}

#[test]
fn supports_and_loads_get_regions_at_their_surfaces_and_resolution_follows_the_mesh_size() {
    let mut p = plate();
    p.supports.push(fea_problem::Support::fixed("left"));
    p.loads.push(Load::Pressure { edge: "right".into(), p: 10.0 });
    let r = refinement_regions(&p);
    let kinds: Vec<RegionKind> = r.iter().map(|x| x.kind).collect();
    assert!(kinds.contains(&RegionKind::Support) && kinds.contains(&RegionKind::Load), "{kinds:?}");
    let support = r.iter().find(|x| x.kind == RegionKind::Support).unwrap();
    assert_eq!((support.level, support.centre, support.to), (1, [0.0, 4.0], Some([0.0, 0.0])), "a band along the left edge");
    // Hole radius 0.5: against element size 0.5 (1 element per radius) it is barely resolved, at 0.1 (5) medium, at 0.05 (10) fine.
    let res = |p: &Problem| refinement_regions(p).iter().find(|x| x.kind == RegionKind::Hole).unwrap().resolution;
    assert_eq!(res(&p), Resolution::Coarse);
    p.mesh.size = 0.1;
    assert_eq!(res(&p), Resolution::Medium);
    p.mesh.size = 0.05;
    assert_eq!(res(&p), Resolution::Fine);
    assert_eq!(level_order(RegionKind::Hole, false), [2, 3, 1], "the order is advice; any level may be applied");
}

#[test]
fn applying_a_region_refines_the_mesh_there_and_only_there() {
    let mut p = l_bracket();
    p.mesh.hole_factor = 1.0;
    let corner = refinement_regions(&p).into_iter().find(|r| r.kind == RegionKind::ReentrantCorner).unwrap();
    let (near, far) = (corner.centre, [5.5, 0.4]);
    let (before_near, before_far) = (nodes_within(&p, near, corner.radius), nodes_within(&p, far, 0.5));
    apply_region(&mut p, &corner, 3).unwrap();
    assert_eq!(p.mesh.refine, vec![RefineRegion { centre: corner.centre, radius: corner.radius, level: 3, to: None, blend: Some(corner.blend) }]);
    let (after_near, after_far) = (nodes_within(&p, near, corner.radius), nodes_within(&p, far, 0.5));
    assert!(after_near > 4 * before_near, "level 3 is 8x finer in length: {before_near} -> {after_near}");
    assert!(after_far <= before_far + before_far / 4 + 2, "far away the mesh is unchanged: {before_far} -> {after_far}");
    assert!(refinement_regions(&p).iter().any(|r| r.kind == RegionKind::ReentrantCorner && r.applied), "applied zones are marked");
    // Applying again at a lower level lowers the zone instead of stacking another one.
    apply_region(&mut p, &corner, 1).unwrap();
    assert_eq!(p.mesh.refine.len(), 1);
    assert_eq!(p.mesh.refine[0].level, 1);
}

#[test]
fn zones_survive_adaptive_passes() {
    let mut p = l_bracket();
    p.supports.push(fea_problem::Support::fixed("edge6"));
    p.loads.push(Load::Pressure { edge: "edge2".into(), p: 100.0 });
    p.mesh.size = 0.6;
    p.mesh.adapt_passes = 1;
    let plain = fea_problem::solve(&p, None).unwrap();
    let corner = refinement_regions(&p).into_iter().find(|r| r.kind == RegionKind::ReentrantCorner).unwrap();
    apply_region(&mut p, &corner, 3).unwrap();
    let zoned = fea_problem::solve(&p, None).unwrap();
    let count = |s: &fea_problem::Solved| s.model.mesh.nodes.iter().filter(|n| (n[0] - 2.0).hypot(n[1] - 2.0) <= corner.radius).count();
    assert!(count(&zoned) > count(&plain), "{} vs {}", count(&zoned), count(&plain));
    assert!(zoned.summary.equilibrium_error < 1e-6);
}

#[test]
fn zones_serialize_only_when_present_old_files_load_and_bad_zones_are_refused() {
    let mut p = plate();
    let json = p.to_json().unwrap();
    assert!(!json.contains("refine"), "an empty list adds nothing to saved files");
    assert_eq!(Problem::from_json(&json).unwrap(), p);
    p.mesh.refine.push(RefineRegion::circle([1.0, 1.0], 1.0, 2));
    assert_eq!(Problem::from_json(&p.to_json().unwrap()).unwrap(), p);
    for bad in [RefineRegion::circle([1.0, 1.0], 0.0, 2), RefineRegion::circle([f64::NAN, 1.0], 1.0, 2), RefineRegion::circle([1.0, 1.0], 1.0, 4), RefineRegion::circle([1.0, 1.0], 1.0, 0)] {
        let mut q = plate();
        q.mesh.refine.push(bad);
        assert!(q.validate_geometry().is_err());
    }
    let mut q = plate();
    q.mesh.refine = vec![RefineRegion::circle([1.0, 1.0], 1.0, 1); 17];
    assert!(q.validate_geometry().unwrap_err().contains("at most"));
}

#[test]
fn an_empty_or_unbuilt_model_has_no_regions() {
    assert!(refinement_regions(&templates::blank()).is_empty());
}

fn plate_with_bcs() -> Problem {
    let mut p = plate();
    p.supports.push(fea_problem::Support::fixed("left"));
    p.loads.push(Load::Pressure { edge: "right".into(), p: 10.0 });
    p
}

#[test]
fn zones_are_sized_to_their_feature_a_hole_tight_and_supports_and_loads_as_edge_bands() {
    let p = plate_with_bcs();
    let (hole_r, Geometry::Sketch { outer, .. }) = (0.5, &p.geometry) else { panic!() };
    let (lo, hi) = outer.bounds();
    let diag = (hi[0] - lo[0]).hypot(hi[1] - lo[1]);
    let r = refinement_regions(&p);
    let hole = r.iter().find(|x| x.kind == RegionKind::Hole).unwrap();
    assert!((hole.radius - 1.25 * hole_r).abs() < 1e-9 && (hole.blend - 0.75 * hole_r).abs() < 1e-9, "{hole:?}");
    for kind in [RegionKind::Support, RegionKind::Load] {
        let z = r.iter().find(|x| x.kind == kind).unwrap();
        let to = z.to.expect("an edge zone is a band along the edge");
        assert!((z.radius - 0.03 * diag).abs() < 1e-9);
        assert!((z.centre[0] - to[0]).abs() < 1e-9, "a vertical edge: both ends share x");
        assert!(((z.centre[1] - to[1]).abs() - (hi[1] - lo[1])).abs() < 1e-9, "the band spans the whole edge");
    }
}

#[test]
fn a_band_refines_along_its_whole_edge_and_nowhere_else() {
    let mut p = plate_with_bcs();
    p.mesh.hole_factor = 1.0;
    let support = refinement_regions(&p).into_iter().find(|x| x.kind == RegionKind::Support).unwrap();
    let edge = |p: &Problem, y: f64| nodes_within(p, [0.0, y], 0.4);
    let before: Vec<usize> = [0.3, 2.0, 3.7].iter().map(|y| edge(&p, *y)).collect();
    apply_region(&mut p, &support, 2).unwrap();
    for (y, b) in [0.3, 2.0, 3.7].iter().zip(before) {
        assert!(edge(&p, *y) > 2 * b, "the edge is finer at y = {y}: {b} -> {}", edge(&p, *y));
    }
    let near = nodes_within(&p, [6.0, 2.0], 0.4);
    let q = plate_with_bcs();
    assert!(near <= nodes_within(&q, [6.0, 2.0], 0.4) + 3, "the far side of the plate is untouched by the left band");
}

#[test]
fn recommended_zones_touch_a_small_share_of_the_plate_and_every_zone_has_an_element_count() {
    let mut p = plate_with_bcs();
    let none = build::estimate_mesh(&p).unwrap();
    assert!(none.per_zone.is_empty() && none.core_share == 0.0);
    for r in refinement_regions(&p) {
        apply_region(&mut p, &r, r.level).unwrap();
    }
    let e = build::estimate_mesh(&p).unwrap();
    assert_eq!(e.per_zone.len(), 3, "hole, support and load");
    assert!(e.per_zone.iter().all(|n| *n > 0.0));
    assert!(e.elements > none.elements && e.per_zone.iter().sum::<f64>() <= e.elements + 1e-6);
    assert!(e.core_share < 0.25 && e.touched_share < 0.5, "core {:.2}, touched {:.2}", e.core_share, e.touched_share);
    assert!(e.core_share <= e.touched_share);
}

#[test]
fn the_element_estimate_tracks_the_real_mesh() {
    for refined in [false, true] {
        let mut p = plate_with_bcs();
        if refined {
            for r in refinement_regions(&p) {
                apply_region(&mut p, &r, r.level).unwrap();
            }
        }
        let est = build::estimate_mesh(&p).unwrap().elements;
        let real = build::sketch_mesh(&p, None).unwrap().n_elems() as f64;
        assert!((est / real - 1.0).abs() < 0.4, "refined {refined}: estimate {est:.0} vs real {real:.0}");
    }
}

#[test]
fn bands_round_trip_and_bad_bands_are_refused() {
    let mut p = plate_with_bcs();
    let support = refinement_regions(&p).into_iter().find(|x| x.kind == RegionKind::Support).unwrap();
    apply_region(&mut p, &support, 2).unwrap();
    assert!(p.to_json().unwrap().contains("\"to\""));
    // JSON text may differ from the stored float by one unit in the last place; the zone itself is unchanged.
    let back = Problem::from_json(&p.to_json().unwrap()).unwrap();
    let (a, b) = (&back.mesh.refine[0], &p.mesh.refine[0]);
    assert!((a.radius - b.radius).abs() < 1e-12 && a.to == b.to && a.level == b.level && a.centre == b.centre);
    let mut q = plate();
    q.mesh.refine.push(RefineRegion { centre: [1.0, 1.0], radius: 1.0, level: 2, to: Some([f64::NAN, 0.0]), blend: None });
    assert!(q.validate_geometry().is_err());
    q.mesh.refine[0] = RefineRegion { centre: [1.0, 1.0], radius: 1.0, level: 2, to: None, blend: Some(0.0) };
    assert!(q.validate_geometry().is_err());
}

#[test]
fn the_error_estimate_adds_zones_where_the_error_is_and_applying_it_lowers_the_error() {
    let mut p = plate_with_bcs();
    p.mesh.adapt_passes = 0;
    p.mesh.size = 0.7;
    p.mesh.hole_factor = 1.0;
    p.mesh.target_error = 0.005;
    let first = fea_problem::solve(&p, None).unwrap();
    let before = first.history.last().unwrap().zz_error;
    let zones = fea_problem::advice::error_zones(&first);
    assert!(!zones.is_empty() && zones.len() <= 3, "{zones:?} (error {before})");
    assert!(zones.iter().any(|z| (z.centre[0] - 6.0).hypot(z.centre[1] - 2.0) < 1.5), "one zone at the hole: {zones:?}");
    assert_eq!(fea_problem::advice::add_zones(&mut p, &zones), zones.len());
    let second = fea_problem::solve(&p, None).unwrap();
    assert!(second.history.last().unwrap().zz_error < before, "{} -> {before}", second.history.last().unwrap().zz_error);
    // Applying the same estimate again adds nothing the zones do not already cover.
    assert!(fea_problem::advice::error_zones(&fea_problem::solve(&p, None).unwrap()).len() < zones.len() + 1);
    // Below the target nothing is asked for.
    p.mesh.target_error = 0.9;
    assert!(fea_problem::advice::error_zones(&fea_problem::solve(&p, None).unwrap()).is_empty());
}
