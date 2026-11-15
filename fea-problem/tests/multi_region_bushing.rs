//! Bushings and pin loads on holes of any region of a sketch (#234). Holes are numbered through the first region's, then each
//! further region's; the checks are differential (a region's bushed hole against the same plate as the only region) and
//! closed-form (Lame fit pressure, the pin load's resultant, equilibrium).
use fea_problem::advice::{self, BcKind};
use fea_problem::{solve, templates::templates, Analysis, ContactPair, Geometry, Load, MeshSpec, Problem, Shape, SketchRegion, Support};

fn bushed() -> Problem {
    let mut p = templates().into_iter().find(|(n, _)| *n == "Lug with an eccentric bushing").unwrap().1;
    // Fixed mesh: an adaptive field would see the other region's hole and differ, which is not what is compared.
    p.mesh = MeshSpec { size: 0.3, hole_factor: 0.2, adapt_passes: 0, ..MeshSpec::default() };
    p
}

/// A small separate plate (one hole) as region 1, the lug as region 2: the lug's bushed hole is hole 2.
fn lug_as_second_region() -> Problem {
    let mut p = bushed();
    let Geometry::Sketch { outer, holes, depth, layers, .. } = p.geometry.clone() else { unreachable!() };
    p.geometry = Geometry::Sketch {
        outer: Shape::Rect { x0: 10.0, y0: 0.0, x1: 12.0, y1: 1.0 },
        holes: vec![Shape::Circle { cx: 11.0, cy: 0.5, r: 0.2 }],
        depth,
        layers,
        regions: vec![SketchRegion { outer, holes }],
    };
    p.supports = vec![Support::fixed("left"), Support::fixed("r2/right")];
    p.loads = vec![Load::Bearing { hole: 2, fx: -1500.0, fy: 0.0 }];
    p.bushings[0].hole = 2;
    p
}

#[test]
fn holes_are_numbered_through_the_regions_and_back() {
    let p = lug_as_second_region();
    let (shape, region, surface) = p.hole_at(2).unwrap();
    assert!(matches!(shape, Shape::Circle { cx, .. } if (*cx - 1.0).abs() < 1e-12));
    assert_eq!((region, surface.as_str()), (1, "r2/hole1"));
    assert_eq!(p.hole_at(1).unwrap().2, "hole1");
    assert!(p.hole_at(0).is_none() && p.hole_at(3).is_none());
    for n in 1..=2 {
        let name = p.hole_at(n).unwrap().2;
        assert!(p.edge_names().contains(&name), "{name} is a surface of the problem");
        assert_eq!(p.hole_number(&name), Some(n));
    }
    for bad in ["hole3", "r2/hole2", "r3/hole1", "r1/hole1", "hole0", "left", "r2/left"] {
        assert_eq!(p.hole_number(bad), None, "{bad}");
    }
    // One region: the numbering is the old one.
    assert_eq!(bushed().hole_at(1).unwrap().2, "hole1");
}

#[test]
fn a_bushing_in_a_later_region_gives_the_same_fit_and_pin_load_as_the_same_plate_alone() {
    let single = solve(&bushed(), None).unwrap();
    let multi = solve(&lug_as_second_region(), None).unwrap();
    let (a, b) = (&single.summary.interfaces[0], &multi.summary.interfaces[0]);
    assert_eq!((a.hole, b.hole), (1, 2));
    eprintln!("alone: mean {:.1} peak {:.1} torque {:.4}; as region 2: mean {:.1} peak {:.1} torque {:.4}", a.mean_pressure, a.peak_pressure, a.torque_capacity, b.mean_pressure, b.peak_pressure, b.torque_capacity);
    for (name, x, y) in [("mean pressure", a.mean_pressure, b.mean_pressure), ("torque capacity", a.torque_capacity, b.torque_capacity), ("normal force", a.normal_force, b.normal_force), ("open arc", a.open_arc_deg, b.open_arc_deg)] {
        assert!((x - y).abs() <= 1e-4 * x.abs().max(1.0), "{name}: alone {x}, as region 2 {y}");
    }
    // The pin load of 1500 lbf is reacted at the lug's clamped end: both plates' reactions add up to it.
    assert!((multi.summary.reaction[0].abs() - 1500.0).abs() < 0.01 * 1500.0, "reaction {:?}", multi.summary.reaction);
    assert!(multi.summary.equilibrium_error < 1e-3, "equilibrium {:e}", multi.summary.equilibrium_error);
}

#[test]
fn a_second_regions_concentric_bushing_gives_the_lame_fit_pressure() {
    // A large second region (the Lame housing is infinite) beside a small first one; the fit alone.
    let mut p = lug_as_second_region();
    let Geometry::Sketch { regions, .. } = &mut p.geometry else { unreachable!() };
    regions[0] = SketchRegion { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 12.0 }, holes: vec![Shape::Circle { cx: 6.0, cy: 6.0, r: 0.5 }] };
    p.loads.clear();
    p.supports = vec![Support::fixed("left"), Support::fixed("r2/left")];
    p.bushings[0].offset = [0.0, 0.0];
    p.mesh = MeshSpec { size: 1.5, hole_factor: 0.08, adapt_passes: 0, ..MeshSpec::default() };
    let b = p.bushings[0].clone();
    let f = solve(&p, None).unwrap().summary.interfaces.remove(0);
    let (a, ri, e_h, e_b) = (0.5, 0.5 * b.inner_diameter, p.material.e, b.material.e);
    let c = a * (1.0 + p.material.nu) / e_h + a / e_b * ((a * a + ri * ri) / (a * a - ri * ri) - b.material.nu);
    let want = 0.5 * b.interference / c;
    eprintln!("region 2 fit pressure {:.0} vs Lame {want:.0}", f.mean_pressure);
    assert!((f.mean_pressure / want - 1.0).abs() < 0.04, "mean {} vs Lame {want}", f.mean_pressure);
    assert_eq!(f.hole, 2);
}

#[test]
fn a_pin_load_on_an_unbushed_hole_of_a_later_region_has_the_loads_resultant() {
    let mut p = lug_as_second_region();
    p.bushings.clear();
    p.mesh.adapt_passes = 0;
    let s = solve(&p, None).unwrap();
    assert!((s.summary.applied[0].abs() - 1500.0).abs() < 1e-6 * 1500.0 + 1e-3, "applied {:?}", s.summary.applied);
    assert!((s.summary.reaction[0].abs() - 1500.0).abs() < 1e-6 * 1500.0 + 1e-3, "reaction {:?}", s.summary.reaction);
    assert!(s.summary.equilibrium_error < 1e-8, "equilibrium {:e}", s.summary.equilibrium_error);
    // A hole that does not exist is refused by name.
    p.loads = vec![Load::Bearing { hole: 3, fx: 1.0, fy: 0.0 }];
    assert!(solve(&p, None).unwrap_err().contains("no hole 3"));
}

#[test]
fn bushing_validation_names_what_is_wrong_across_regions() {
    let p = lug_as_second_region();
    p.validate().unwrap();
    // Hole 1 is the small plate's 0.2 in radius hole: the 0.75 in bore leaves no wall there.
    let mut q = p.clone();
    q.bushings[0].hole = 1;
    assert!(q.validate().unwrap_err().contains("wall"));
    let mut q = p.clone();
    q.bushings[0].hole = 3;
    assert!(q.validate().unwrap_err().contains("not a circle"));
    // Two bushings on one hole, and contact pairs with bushings, stay refused.
    let mut q = p.clone();
    q.bushings.push(q.bushings[0].clone());
    assert!(q.validate().unwrap_err().contains("already has a bushing"));
    let mut q = p;
    q.contacts = vec![ContactPair { a: "top".into(), b: "r2/bottom".into(), friction: 0.0 }];
    q.analysis = Analysis::PlaneStress;
    assert!(q.validate().unwrap_err().contains("do not combine with bushings"));
}

#[test]
fn advice_and_refinement_zones_follow_the_holes_of_every_region() {
    let mut p = lug_as_second_region();
    let zones = advice::refinement_regions(&p);
    let at = |x: f64| zones.iter().find(|z| (z.centre[0] - x).abs() < 1e-9 && z.kind == advice::RegionKind::Hole).unwrap_or_else(|| panic!("a hole zone at x = {x}"));
    assert!(at(1.0).loaded, "the bushed, pin-loaded hole of region 2 is the loaded one");
    assert!(at(1.0).reason.contains("hole 2") && at(1.0).reason.contains("region 2"), "{}", at(1.0).reason);
    assert!(!at(11.0).loaded, "the small plate's hole carries nothing");
    // Region 1's hole 1 is not 'loaded' because the pin load of hole 2 exists.
    assert!(at(11.0).reason.contains("hole 1") && !at(11.0).reason.contains("region"), "{}", at(11.0).reason);
    // The zones apply and reach the mesh settings.
    advice::apply_region(&mut p, at(1.0), at(1.0).level).unwrap();
    assert!(p.mesh.refine.iter().any(|z| (z.centre[0] - 1.0).abs() < 1e-9));
    // A pin load recommended on a later region's hole addresses it by its global number; loads stay in step with it.
    p.loads.clear();
    p.bushings.clear();
    advice::add_boundary(&mut p, "r2/hole1", BcKind::Bearing).unwrap();
    assert!(matches!(p.loads[0], Load::Bearing { hole: 2, .. }), "{:?}", p.loads[0]);
    assert!(advice::add_boundary(&mut p, "r2/hole9", BcKind::Bearing).is_err());
    let hints = advice::recommend(&p);
    assert!(hints.iter().all(|a| !matches!(a, advice::Advice::Boundary { kind: BcKind::Bearing, surface, .. } if surface == "r2/hole1")), "that hole already has its pin load");
}
