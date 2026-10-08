//! The workbench problems against closed-form solutions and their own invariants.

use fea_problem::raster::rasterize;
use fea_problem::solve::{principal, solve, Field};
use fea_problem::templates::templates;
use fea_problem::*;

fn template(name: &str) -> Problem {
    templates().into_iter().find(|(n, _)| *n == name).unwrap().1
}

#[test]
fn every_template_solves_and_balances() {
    for (name, p) in templates() {
        let s = solve(&p, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        let sm = &s.summary;
        eprintln!("{name}: {} nodes, {} elements, vm max {:.1}, zz {:.3}, equilibrium {:.2e}, {:.0} ms", sm.nodes, sm.elements, sm.max_von_mises.value, sm.zz_error, sm.equilibrium_error, sm.solve_ms);
        // A linear solve balances to rounding; a contact solve to its Newton tolerance (1e-4 of the force scale).
        let tol = if p.bushings.is_empty() { 1e-8 } else { 1e-3 };
        assert!(sm.equilibrium_error < tol, "{name}: equilibrium {:e}", sm.equilibrium_error);
        assert!(sm.max_von_mises.value > 0.0 && sm.max_displacement.value > 0.0, "{name}: empty result");
        assert!(s.vtu().unwrap().contains("von_mises"), "{name}");
    }
}

#[test]
fn plate_with_hole_matches_the_net_section_stress_concentration() {
    // Heywood / Peterson, d/W = 0.25: Kt(net) = 3 - 3.14 x + 3.667 x^2 - 1.527 x^3 = 2.420.
    let mut p = template("Plate with a hole");
    p.mesh.size = 0.4;
    let s = solve(&p, None).unwrap();
    let (w, d, sigma) = (4.0, 1.0, 10_000.0);
    let x = d / w;
    // 3.14 is Howland's polynomial coefficient for the stress-concentration fit, not pi.
    #[allow(clippy::approx_constant)]
    let expect = (3.0 - 3.14 * x + 3.667 * x * x - 1.527 * x * x * x) * sigma * w / (w - d);
    let got = s.summary.max_principal.value;
    eprintln!("peak {got:.0} expect {expect:.0}");
    assert!((got / expect - 1.0).abs() < 0.04, "peak {got} vs {expect}");
    // The peak sits on the hole edge, at the section through its centre.
    let at = s.summary.max_principal.at;
    assert!((at[0] - 6.0).abs() < 0.2 && ((at[1] - 2.0).abs() - 0.5).abs() < 0.1, "peak at {at:?}");
}

#[test]
fn cantilever_tip_deflection_is_timoshenko() {
    let p = template("Cantilever beam");
    let s = solve(&p, None).unwrap();
    let (e, nu, pl, l, h) = (29.0e6f64, 0.3f64, 100.0f64, 10.0f64, 1.0f64);
    let (i, g) = (h.powi(3) / 12.0, e / (2.0 * (1.0 + nu)));
    let expect = pl * l.powi(3) / (3.0 * e * i) + pl * l / (5.0 / 6.0 * g * h);
    let uy = s.node_values(Field::Uy);
    let tip = (0..s.model.mesh.nodes.len()).filter(|&n| (s.model.mesh.nodes[n][0] - l).abs() < 1e-9).map(|n| -uy[n]).fold(f64::NEG_INFINITY, f64::max);
    eprintln!("tip {tip:.6e} expect {expect:.6e}");
    assert!((tip / expect - 1.0).abs() < 0.02, "{tip} vs {expect}");
    assert!((s.summary.reaction[1] - 100.0).abs() < 1e-6 * 100.0, "reaction {:?}", s.summary.reaction);
}

#[test]
fn thick_cylinder_matches_lame() {
    let mut p = template("Thick cylinder (axisymmetric)");
    p.mesh.size = 0.12;
    let s = solve(&p, None).unwrap();
    let (a, b, pr, e, nu) = (1.0f64, 2.0f64, 1000.0, 29.0e6, 0.3);
    let k = pr * a * a / (b * b - a * a);
    let ur = |r: f64| (1.0 + nu) / e * k * ((1.0 - 2.0 * nu) * r + b * b / r);
    let ux = s.node_values(Field::Ux);
    for (n, x) in s.model.mesh.nodes.iter().enumerate() {
        assert!((ux[n] / ur(x[0]) - 1.0).abs() < 2e-3, "r = {}: {} vs {}", x[0], ux[n], ur(x[0]));
    }
    let hoop = pr * (b * b + a * a) / (b * b - a * a);
    assert!((s.summary.max_principal.value / hoop - 1.0).abs() < 0.01, "{} vs {hoop}", s.summary.max_principal.value);
    assert!((s.summary.min_principal.value + pr).abs() < 0.02 * pr, "radial {}", s.summary.min_principal.value);
}

#[test]
fn extruded_bar_in_tension_is_exact() {
    let mut p = template("Extruded plate with a hole (3D)");
    if let Geometry::Sketch { holes, .. } = &mut p.geometry {
        holes.clear();
    }
    p.supports = vec![
        Support::roller("left", 0),
        Support::Point { x: 0.0, y: 0.0, z: 0.0, ux: None, uy: Some(0.0), uz: Some(0.0) },
        Support::Point { x: 0.0, y: 2.0, z: 0.0, ux: None, uy: None, uz: Some(0.0) },
    ];
    let s = solve(&p, None).unwrap();
    let ux = s.node_values(Field::Ux);
    let want = 5000.0 * 4.0 / 29.0e6;
    let right = (0..s.model.mesh.nodes.len()).filter(|&n| (s.model.mesh.nodes[n][0] - 4.0).abs() < 1e-9);
    for n in right {
        assert!((ux[n] / want - 1.0).abs() < 1e-6, "{} vs {want}", ux[n]);
    }
    assert!((s.summary.max_von_mises.value / 5000.0 - 1.0).abs() < 1e-4, "{}", s.summary.max_von_mises.value);
}

#[test]
fn bearing_load_has_the_requested_resultant() {
    let p = template("Pin-loaded lug");
    let s = solve(&p, None).unwrap();
    assert!((s.summary.applied[0] + 3000.0).abs() < 1e-6 * 3000.0, "{:?}", s.summary.applied);
    assert!(s.summary.applied[1].abs() < 1e-6 * 3000.0, "{:?}", s.summary.applied);
    // The loaded side of the bore (left, toward -x) carries the compression peak.
    let at = s.summary.min_principal.at;
    assert!(at[0] < 1.0 && (at[1] - 1.0).abs() < 0.3, "peak compression at {at:?}");
}

#[test]
fn adaptive_passes_refine_where_the_error_is() {
    let p = template("L-bracket (adaptive mesh)");
    let s = solve(&p, None).unwrap();
    assert!(s.history.len() >= 2, "{:?}", s.history);
    assert!(s.history.last().unwrap().zz_error < s.history[0].zz_error, "{:?}", s.history);
    assert!(s.history.last().unwrap().elements > s.history[0].elements);
}

#[test]
fn json_round_trips_every_template() {
    for (name, p) in templates() {
        let back = Problem::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(back, p, "{name}");
    }
    assert!(Problem::from_json("{\"hello\": 1}").is_err());
}

#[test]
fn bad_problems_say_what_is_wrong() {
    let mut p = template("Plate with a hole");
    p.supports.clear();
    assert!(solve(&p, None).unwrap_err().contains("support"));
    let mut p = template("Plate with a hole");
    p.supports = vec![Support::fixed("lefft")];
    let e = solve(&p, None).unwrap_err();
    assert!(e.contains("lefft") && e.contains("left"), "{e}");
    let mut p = template("Plate with a hole");
    p.supports = vec![Support::roller("left", 0)];
    let e = solve(&p, None).unwrap_err();
    assert!(e.contains("not fully constrained"), "{e}");
    let mut p = template("Plate with a hole");
    p.mesh.size = 0.001;
    assert!(solve(&p, None).unwrap_err().contains("too fine"));
    let mut p = template("Plate with a hole");
    p.mesh.hole_factor = 0.0;
    assert!(solve(&p, None).is_err());
    let mut p = template("Extruded plate with a hole (3D)");
    p.mesh.element = ElementChoice::Tri6;
    assert!(solve(&p, None).unwrap_err().contains("quadrilateral"));
}

#[test]
fn slot_and_polygon_regions_have_the_right_area() {
    let mut p = template("Plate with a hole");
    p.geometry = Geometry::Sketch { outer: Shape::Slot { cx: 0.0, cy: 0.0, length: 6.0, width: 2.0, angle: 30.0 }, holes: vec![], depth: 1.0, layers: 1 };
    let r = fea_problem::build::region_of(&p).unwrap();
    let area = std::f64::consts::PI + 2.0 * 4.0;
    assert!((r.area() / area - 1.0).abs() < 1e-9, "{} vs {area}", r.area());
    let (lo, hi) = Shape::Slot { cx: 0.0, cy: 0.0, length: 6.0, width: 2.0, angle: 0.0 }.bounds();
    assert_eq!((lo, hi), ([-3.0, -1.0], [3.0, 1.0]));
    assert!(solve_slot_plate().summary.equilibrium_error < 1e-8);
}

fn solve_slot_plate() -> Solved {
    let mut p = template("Plate with a hole");
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 8.0, y1: 3.0 }, holes: vec![Shape::Slot { cx: 4.0, cy: 1.5, length: 2.0, width: 0.8, angle: 0.0 }], depth: 1.0, layers: 1 };
    solve(&p, None).unwrap()
}

#[test]
fn principal_stresses() {
    let p = principal(&[100.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert!((p[0] - 100.0).abs() < 1e-5 && p[1].abs() < 1e-5 && p[2].abs() < 1e-5, "{p:?}");
    let p = principal(&[0.0, 0.0, 0.0, 50.0, 0.0, 0.0]);
    assert!((p[0] - 50.0).abs() < 1e-5 && (p[2] + 50.0).abs() < 1e-5 && p[1].abs() < 1e-5, "{p:?}");
    let p = principal(&[7.0, 7.0, 7.0, 0.0, 0.0, 0.0]);
    assert_eq!(p, [7.0; 3]);
    let p = principal(&[10.0, -4.0, 3.0, 2.0, -1.0, 5.0]);
    assert!(p[0] >= p[1] && p[1] >= p[2]);
    assert!((p[0] + p[1] + p[2] - 9.0).abs() < 1e-9);
}

#[test]
fn raster_fills_the_body_and_leaves_the_hole_empty() {
    let p = template("Plate with a hole");
    let s = solve(&p, None).unwrap();
    let vm = s.node_values(Field::VonMises);
    let r = rasterize(&s.model.mesh, Some(&vm), None, 120, 40);
    let filled = r.value.iter().filter(|v| v.is_finite()).count() as f64;
    // Plate 12 x 4 minus the hole, drawn at scale 10 px per unit = 120 x 40 px.
    let area_px = (12.0 * 4.0 - std::f64::consts::PI * 0.25) * r.scale * r.scale;
    assert!((filled / area_px - 1.0).abs() < 0.03, "{filled} vs {area_px}");
    assert!(r.max > r.min && r.max >= s.summary.max_von_mises.value * 0.5, "{} vs {}", r.max, s.summary.max_von_mises.value);
    // The hole centre pixel is empty.
    let (cx, cy) = (((6.0 - r.origin[0]) * r.scale) as usize, ((r.origin[1] - 2.0) * r.scale) as usize);
    assert!(r.at(cx, cy).is_nan());
    assert!(r.edge.iter().any(|&e| e));
    // Deformed shape stays inside the grid.
    let _ = rasterize(&s.model.mesh, Some(&vm), Some((&s.u, 1000.0)), 80, 30);
}

#[test]
fn imported_abaqus_mesh_solves() {
    let inp = "*NODE\n1,0,0\n2,1,0\n3,2,0\n4,0,1\n5,1,1\n6,2,1\n*ELEMENT, TYPE=CPS4\n1,1,2,5,4\n2,2,3,6,5\n*NSET, NSET=LEFT\n1,4\n*NSET, NSET=RIGHT\n3,6\n";
    let mut p = template("Cantilever beam");
    p.geometry = Geometry::Imported { path: "bar.inp".into() };
    p.thickness = 1.0;
    p.supports = vec![Support::fixed("LEFT")];
    p.loads = vec![Load::Force { edge: "RIGHT".into(), fx: 100.0, fy: 0.0, fz: 0.0 }];
    // A node set with no boundary faces between its nodes still loads: its boundary edge is the right end.
    let s = solve(&p, Some(inp)).unwrap();
    assert_eq!(s.summary.elements, 2);
    assert!((s.summary.applied[0] - 100.0).abs() < 1e-9, "{:?}", s.summary.applied);
    let e = solve(&p, None).unwrap_err();
    assert!(e.contains("not been read"), "{e}");
    p.analysis = Analysis::Solid;
    assert!(solve(&p, Some(inp)).unwrap_err().contains("2D mesh"));
}

#[test]
fn the_report_names_the_problem_and_its_results() {
    let s = solve(&template("Cantilever beam"), None).unwrap();
    let r = fea_problem::report::report(&s);
    for want in ["Cantilever beam", "left: fixed", "right: total force", "Equilibrium error", "Max von Mises", "Mesh error (ZZ)"] {
        assert!(r.contains(want), "missing '{want}' in\n{r}");
    }
}

#[test]
fn linear_triangles_carry_a_documented_caution_and_quadratic_elements_do_not() {
    let mut p = template("Plate with a hole");
    p.mesh.size = 0.5;
    p.mesh.element = ElementChoice::Tri3;
    let s = solve(&p, None).unwrap();
    assert!(s.summary.notes.iter().any(|n| n.contains("Linear triangles")), "{:?}", s.summary.notes);
    assert!(fea_problem::report::report(&s).contains("Notes"));
    p.mesh.element = ElementChoice::Quad9;
    assert!(solve(&p, None).unwrap().summary.notes.is_empty());
}

fn lame_pressure_plane_stress(e_h: f64, nu_h: f64, e_b: f64, nu_b: f64, a: f64, ri: f64, delta_radial: f64) -> f64 {
    // Housing: infinite plate (u = p a (1 + nu) / E); bushing: ring (u = -(p a / E)((a^2 + ri^2)/(a^2 - ri^2) - nu)).
    let c = a * (1.0 + nu_h) / e_h + a / e_b * ((a * a + ri * ri) / (a * a - ri * ri) - nu_b);
    delta_radial / c
}

#[test]
fn a_concentric_bushing_in_a_big_plate_gives_the_lame_fit_pressure_and_the_fit_balances() {
    let mut p = template("Lug with an eccentric bushing");
    // A large plate (the Lame housing is infinite), clamped on one side, no pin load: the fit alone.
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 12.0 }, holes: vec![Shape::Circle { cx: 6.0, cy: 6.0, r: 0.5 }], depth: 1.0, layers: 1 };
    p.loads.clear();
    p.supports = vec![Support::fixed("left")];
    p.bushings[0].offset = [0.0, 0.0];
    p.mesh = MeshSpec { size: 1.5, hole_factor: 0.08, ..MeshSpec::default() };
    let b = p.bushings[0].clone();
    let s = solve(&p, None).unwrap();
    let f = &s.summary.interfaces[0];
    let want = lame_pressure_plane_stress(p.material.e, p.material.nu, b.material.e, b.material.nu, 0.5, 0.5 * b.inner_diameter, 0.5 * b.interference);
    eprintln!("fit pressure {:.0} vs Lame {want:.0} (peak {:.0}, open {:.0} deg)", f.mean_pressure, f.peak_pressure, f.open_arc_deg);
    assert!((f.mean_pressure / want - 1.0).abs() < 0.04, "mean {} vs Lame {want}", f.mean_pressure);
    assert!(f.open_arc_deg == 0.0 && f.slip_share < 0.5);
    // Torque capacity of the concentric fit: mu p 2 pi a^2 t.
    let classical = b.friction * f.mean_pressure * 2.0 * std::f64::consts::PI * 0.25 * p.thickness;
    assert!((f.torque_capacity / classical - 1.0).abs() < 0.03, "{} vs {classical}", f.torque_capacity);
    assert!(s.summary.equilibrium_error < 1e-3, "equilibrium {:e}", s.summary.equilibrium_error);
}

#[test]
fn the_bushed_lug_template_solves_and_the_pin_load_acts_on_the_bushing_bore() {
    let p = template("Lug with an eccentric bushing");
    let s = solve(&p, None).unwrap();
    assert_eq!(s.summary.interfaces.len(), 1);
    assert!(s.summary.interfaces[0].mean_pressure > 1000.0);
    // The 1500 lbf pin load is reacted at the clamped end.
    assert!((s.summary.reaction[0].abs() - 1500.0).abs() < 0.01 * 1500.0, "reaction {:?}", s.summary.reaction);
    assert!(fea_problem::report::report(&s).contains("Interference fits"));
}

#[test]
fn a_bushing_must_sit_in_a_circular_hole_with_a_wall_and_an_interference() {
    let mut p = template("Lug with an eccentric bushing");
    p.bushings[0].interference = 0.0;
    assert!(p.validate().is_err());
    let mut p = template("Lug with an eccentric bushing");
    p.bushings[0].inner_diameter = 0.99;
    assert!(p.validate().unwrap_err().contains("wall"));
    let mut p = template("Lug with an eccentric bushing");
    p.bushings[0].hole = 3;
    assert!(p.validate().is_err());
    let mut p = template("Lug with an eccentric bushing");
    p.analysis = Analysis::Solid;
    assert!(p.validate().is_err());
}

#[test]
fn unmodified_templates_with_a_closed_form_agree_with_it_and_edited_ones_are_not_checked() {
    use fea_problem::benchmark::check;
    for name in ["Cantilever beam", "Thick cylinder (axisymmetric)", "Plate with a hole"] {
        let p = template(name);
        let solved = solve(&p, None).unwrap();
        let b = check(&solved).unwrap_or_else(|| panic!("{name} has a benchmark"));
        eprintln!("{}", b.lines().join("\n"));
        assert!(b.passed(), "{name}: {:?}", b.checks);
        // Any edit takes the problem off its benchmark.
        let mut edited = p.clone();
        edited.material.e *= 1.1;
        assert!(check(&solve(&edited, None).unwrap()).is_none(), "{name}");
    }
    assert!(check(&solve(&template("Pin-loaded lug"), None).unwrap()).is_none(), "no closed form for the lug template");
}

/// The same problem in other units: lengths x `a`, stresses and moduli x `b` (forces x `b a^2`, thickness is a length).
fn rescaled(p: &Problem, a: f64, b: f64) -> Problem {
    let shape = |s: &Shape| match s {
        Shape::Rect { x0, y0, x1, y1 } => Shape::Rect { x0: x0 * a, y0: y0 * a, x1: x1 * a, y1: y1 * a },
        Shape::Circle { cx, cy, r } => Shape::Circle { cx: cx * a, cy: cy * a, r: r * a },
        Shape::Slot { cx, cy, length, width, angle } => Shape::Slot { cx: cx * a, cy: cy * a, length: length * a, width: width * a, angle: *angle },
        Shape::Polygon { pts } => Shape::Polygon { pts: pts.iter().map(|q| [q[0] * a, q[1] * a]).collect() },
    };
    let mut q = p.clone();
    q.thickness *= a;
    q.material.e *= b;
    q.material.yield_stress = q.material.yield_stress.map(|y| y * b);
    q.mesh.size *= a;
    if let Geometry::Sketch { outer, holes, depth, .. } = &mut q.geometry {
        *outer = shape(outer);
        *holes = holes.iter().map(shape).collect();
        *depth *= a;
    }
    q.loads = p
        .loads
        .iter()
        .map(|l| match l {
            Load::Pressure { edge, p } => Load::Pressure { edge: edge.clone(), p: p * b },
            Load::Traction { edge, tx, ty, tz } => Load::Traction { edge: edge.clone(), tx: tx * b, ty: ty * b, tz: tz * b },
            Load::Force { edge, fx, fy, fz } => Load::Force { edge: edge.clone(), fx: fx * b * a * a, fy: fy * b * a * a, fz: fz * b * a * a },
            Load::Bearing { hole, fx, fy } => Load::Bearing { hole: *hole, fx: fx * b * a * a, fy: fy * b * a * a },
            other => other.clone(),
        })
        .collect();
    for bu in &mut q.bushings {
        bu.inner_diameter *= a;
        bu.offset = [bu.offset[0] * a, bu.offset[1] * a];
        bu.interference *= a;
        bu.material.e *= b;
        bu.material.yield_stress = bu.material.yield_stress.map(|y| y * b);
    }
    q
}

#[test]
fn the_solution_does_not_depend_on_the_units() {
    // inch / psi -> mm / MPa, and a deliberately awkward pair (metres-ish lengths, kPa-ish stresses).
    for (a, b) in [(25.4, 1.0 / 145.037_7), (1.0e-3, 1.0e6)] {
        for name in ["Plate with a hole", "Thick cylinder (axisymmetric)", "Lug with an eccentric bushing"] {
            let p = template(name);
            let (s0, s1) = (solve(&p, None).unwrap(), solve(&rescaled(&p, a, b), None).unwrap_or_else(|e| panic!("{name} at ({a}, {b}): {e}")));
            let vm = s1.summary.max_von_mises.value / b / s0.summary.max_von_mises.value - 1.0;
            let u = s1.summary.max_displacement.value / a / s0.summary.max_displacement.value - 1.0;
            eprintln!("{name} at ({a}, {b}): von Mises {vm:+.2e}, displacement {u:+.2e}, equilibrium {:.1e}, mesh {}x{} nodes/elements vs {}x{}", s1.summary.equilibrium_error, s1.summary.nodes, s1.summary.elements, s0.summary.nodes, s0.summary.elements);
            // The Delaunay refinement is invariant in exact arithmetic; its float round-off breaks ties differently in other units
            // (9992 against 10171 nodes), so the answers agree to the discretisation error, not to round-off.
            let tol = if p.bushings.is_empty() { 1e-2 } else { 2e-2 };
            assert!(vm.abs() < tol && u.abs() < tol, "{name} at ({a}, {b}): von Mises {vm:e}, displacement {u:e}");
        }
    }
}
