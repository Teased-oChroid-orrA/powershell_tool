//! Bonded ties between the bodies of an assembly problem (epic `docs/epic-assembly-feature-advice.md`, Phase C): two stacked
//! bars, closed form. With a zero Poisson ratio the stack under a pull is a uniform axial state, so the tip moves `sigma L / E`
//! exactly (not to a mesh tolerance) however the two bodies' meshes meet at the tie.
use fea_problem::assembly_advice::{recommend_step, AssemblyAdvice};
use fea_problem::solid::{flat_face, text_hash, SolidCache};
use fea_problem::{solve, templates, ElementChoice, FaceSig, Geometry, Load, MeshSpec, Problem, Support, Tie, UnitSystem};
use solid_kernel::native_brep::NativeBrep;
use solid_kernel::{SurfaceMesh, V3};

const STEP: &str = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
const PULL: f64 = 50.0;

/// The box `[lo, lo + size]` as six faces tagged `first..first + 6` = x-, x+, y-, y+, z-, z+.
fn box_at(m: &mut SurfaceMesh, lo: V3, size: V3, first: usize) {
    let p = |x: f64, y: f64, z: f64| [lo[0] + x * size[0], lo[1] + y * size[1], lo[2] + z * size[2]];
    let faces: [([V3; 4], usize); 6] = [
        ([p(0.0, 0.0, 0.0), p(0.0, 0.0, 1.0), p(0.0, 1.0, 1.0), p(0.0, 1.0, 0.0)], 0),
        ([p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 1.0, 1.0), p(1.0, 0.0, 1.0)], 1),
        ([p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(1.0, 0.0, 1.0), p(0.0, 0.0, 1.0)], 2),
        ([p(0.0, 1.0, 0.0), p(0.0, 1.0, 1.0), p(1.0, 1.0, 1.0), p(1.0, 1.0, 0.0)], 3),
        ([p(0.0, 0.0, 0.0), p(0.0, 1.0, 0.0), p(1.0, 1.0, 0.0), p(1.0, 0.0, 0.0)], 4),
        ([p(0.0, 0.0, 1.0), p(1.0, 0.0, 1.0), p(1.0, 1.0, 1.0), p(0.0, 1.0, 1.0)], 5),
    ];
    for (q, f) in faces {
        m.push_triangle(q[0], q[1], q[2], first + f);
        m.push_triangle(q[0], q[2], q[3], first + f);
    }
}

/// A unit-footprint 1 x 1 x 1 bar with a second bar of footprint `top` and height 1 standing on it at `x0` (an assembly cache:
/// body 1's faces are `face1`..`face6`, body 2's `face7`..`face12`; the z- face of body 1 is `face5`, the z+ face of body 2
/// `face12`), meshed at 0.5 with no Poisson coupling, held at the base and pulled at the tip.
fn stack(top: [f64; 2], x0: f64, lift: f64) -> Problem {
    let mut m = SurfaceMesh::default();
    box_at(&mut m, [0.0; 3], [1.0, 1.0, 1.0], 0);
    box_at(&mut m, [x0, 0.0, 1.0 + lift], [top[0], top[1], 1.0], 6);
    let mut cache = SolidCache::new(STEP, m, 0.01);
    cache.bodies = vec![6, 6];
    let mut p = templates::blank();
    p.name = "Stacked bars".into();
    p.set_solid(STEP, cache);
    p.material.nu = 0.0;
    p.mesh = MeshSpec { size: 0.5, ..MeshSpec::default() };
    p.supports = vec![Support::fixed("face5")];
    p.loads = vec![Load::Traction { edge: "face12".into(), tx: 0.0, ty: 0.0, tz: PULL }];
    p
}

/// The tie of body 1's top face and body 2's bottom face of [`stack`].
fn tie(top: [f64; 2], x0: f64, gap: f64) -> Tie {
    Tie {
        a: FaceSig { body: 0, centroid: [0.5, 0.5, 1.0], normal: [0.0, 0.0, 1.0], area: 1.0 },
        b: FaceSig { body: 1, centroid: [x0 + 0.5 * top[0], 0.5 * top[1], 1.0], normal: [0.0, 0.0, -1.0], area: top[0] * top[1] },
        gap,
        friction: None,
    }
}

fn tip(p: &Problem) -> (f64, f64) {
    let s = solve(p, None).unwrap();
    assert!(s.summary.equilibrium_error < 1e-6, "{}", s.summary.equilibrium_error);
    assert!((s.summary.reaction[2] + s.summary.applied[2]).abs() < 1e-6 * s.summary.applied[2].abs());
    (s.summary.max_displacement.value, s.summary.equilibrium_error)
}

#[test]
fn a_tied_stack_moves_by_the_closed_form_in_linear_and_quadratic_elements() {
    for element in [ElementChoice::Tri3, ElementChoice::Tri6] {
        let mut p = stack([1.0, 1.0], 0.0, 0.0);
        p.mesh.element = element;
        p.ties = vec![tie([1.0, 1.0], 0.0, 0.0)];
        // sigma L / E for the two bars of 1
        let want = PULL * 2.0 / p.material.e;
        let (got, _) = tip(&p);
        assert!((got / want - 1.0).abs() < 1e-6, "{element:?}: {got} vs {want}");
    }
}

#[test]
fn a_smaller_bar_on_a_larger_one_is_tied_and_balanced() {
    // body 2 is 0.5 x 1 on the middle of body 1's top: its whole face is the slave. Loaded at the tip it is a free column on the
    // base bar, so the tip moves at least its own sigma L / E and the load goes through the tie (reaction = applied).
    let mut p = stack([0.5, 1.0], 0.25, 0.0);
    p.ties = vec![tie([0.5, 1.0], 0.25, 0.0)];
    let (got, _) = tip(&p);
    let own = PULL * 1.0 / p.material.e;
    assert!(got > own && got < 2.0 * own * 1.5, "{got} vs the column's own {own}");
}

#[test]
fn a_stack_without_its_tie_is_refused_as_not_constrained() {
    let p = stack([1.0, 1.0], 0.0, 0.0);
    let err = solve(&p, None).unwrap_err();
    assert!(err.contains("not fully constrained") || err.contains("constrain") || err.contains("rigid"), "{err}");
}

#[test]
fn faces_that_do_not_touch_join_no_node_and_say_so() {
    // body 2 is 0.3 above body 1 while the tie claims no gap
    let mut p = stack([1.0, 1.0], 0.0, 0.3);
    p.ties = vec![Tie { b: FaceSig { centroid: [0.5, 0.5, 1.3], ..tie([1.0, 1.0], 0.0, 0.0).b }, ..tie([1.0, 1.0], 0.0, 0.0) }];
    let err = solve(&p, None).unwrap_err();
    assert!(err.contains("join no node"), "{err}");
    // the same faces tied with the measured gap are joined (a bond across the gap, displacements equal)
    p.ties[0].gap = 0.3;
    assert!(solve(&p, None).is_ok());
}

#[test]
fn ties_that_name_no_real_pair_are_refused_with_the_reason() {
    let mut p = stack([1.0, 1.0], 0.0, 0.0);
    let good = tie([1.0, 1.0], 0.0, 0.0);
    p.ties = vec![Tie { b: FaceSig { body: 0, ..good.b.clone() }, ..good.clone() }];
    assert!(p.validate().unwrap_err().contains("different bodies"));
    p.ties = vec![Tie { a: FaceSig { centroid: [5.0, 5.0, 1.0], ..good.a.clone() }, ..good.clone() }];
    assert!(p.validate().unwrap_err().contains("no flat face of body 1"), "{:?}", p.validate());
    p.ties = vec![Tie { b: FaceSig { body: 7, ..good.b.clone() }, ..good.clone() }];
    assert!(p.validate().unwrap_err().contains("body 8 is not in the assembly"));
    // one solid has nothing to tie
    let mut single = stack([1.0, 1.0], 0.0, 0.0);
    let Geometry::Solid { cache: Some(c), .. } = &mut single.geometry else { unreachable!() };
    c.bodies.clear();
    single.ties = vec![good];
    assert!(single.validate().unwrap_err().contains("one body"));
}

#[test]
fn ties_do_not_combine_with_what_the_solve_cannot_join_them_to() {
    let mut p = stack([1.0, 1.0], 0.0, 0.0);
    p.ties = vec![tie([1.0, 1.0], 0.0, 0.0)];
    p.material.plastic = true;
    p.material.yield_stress = Some(1.0e4);
    p.material.hardening = 1.0e5;
    assert!(p.validate().unwrap_err().contains("linear-elastic"), "{:?}", p.validate());
}

#[test]
fn ties_survive_a_save_and_a_unit_change_and_still_resolve() {
    let mut p = stack([1.0, 1.0], 0.0, 0.0);
    p.ties = vec![tie([1.0, 1.0], 0.0, 0.0)];
    let text = serde_json::to_string(&p).unwrap();
    let back: Problem = serde_json::from_str(&text).unwrap();
    assert_eq!(back.ties, p.ties);
    assert!(back.validate().is_ok());
    // a problem with none writes no `ties` key
    assert!(!serde_json::to_string(&stack([1.0, 1.0], 0.0, 0.0)).unwrap().contains("\"ties\""));
    let before = p.clone();
    p.convert_units(UnitSystem::MmMpa).unwrap();
    let k = 25.4;
    assert!((p.ties[0].a.area - k * k * before.ties[0].a.area).abs() < 1e-9 && (p.ties[0].b.centroid[2] - k * 1.0).abs() < 1e-9);
    assert!(p.validate().is_ok(), "{:?}", p.validate());
}

// ---------------------------------------------------------------------------------------------------------------------------
// From a STEP assembly
// ---------------------------------------------------------------------------------------------------------------------------

fn shift_ids(line: &str, off: u64) -> String {
    let (mut out, mut rest) = (String::new(), line);
    while let Some(i) = rest.find('#') {
        out.push_str(&rest[..=i]);
        let tail = &rest[i + 1..];
        let digits = tail.chars().take_while(|c| c.is_ascii_digit()).count();
        out.push_str(&(tail[..digits].parse::<u64>().unwrap() + off).to_string());
        rest = &tail[digits..];
    }
    out.push_str(rest);
    out
}

/// One STEP assembly of the boxes `[lo, hi]`, each a product placed at the origin (the box's own coordinates are the assembly's).
fn assembly(boxes: &[(V3, V3)]) -> String {
    const ROOT: u64 = 9_000_000;
    let (mut data, mut root, mut targets, mut ctx0) = (Vec::new(), Vec::new(), Vec::new(), 0);
    for (k, (lo, hi)) in boxes.iter().enumerate() {
        let off = 100_000 * k as u64;
        let text = NativeBrep::box_from_corners(*lo, *hi).unwrap().to_step_text();
        let body = text.split("DATA;\n").nth(1).unwrap().split("ENDSEC;").next().unwrap();
        let (mut origin, mut ctx) = (0, 0);
        for line in body.lines().filter(|l| !l.is_empty()) {
            if line.starts_with("#10 = ") {
                origin = line.split("(#").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
                ctx = line.rsplit('#').next().unwrap().trim_end_matches(");").parse().unwrap();
            }
            data.push(shift_ids(line, off));
        }
        if k == 0 {
            ctx0 = ctx;
        }
        let b = ROOT + 100 * (k as u64 + 1);
        root.push(format!("#{b} = NEXT_ASSEMBLY_USAGE_OCCURRENCE('{k}','','',#{},#{},$);", ROOT + 3, 5 + off));
        root.push(format!("#{} = PRODUCT_DEFINITION_SHAPE('Placement','Placement of an item',#{b});", b + 1));
        root.push(format!("#{} = ITEM_DEFINED_TRANSFORMATION('','',#{},#{});", b + 2, origin + off, b + 3));
        root.push(format!("#{} = AXIS2_PLACEMENT_3D('',#{},#{},#{});", b + 3, b + 4, b + 5, b + 6));
        root.push(format!("#{} = CARTESIAN_POINT('',(0.,0.,0.));", b + 4));
        root.push(format!("#{} = DIRECTION('',(0.,0.,1.));", b + 5));
        root.push(format!("#{} = DIRECTION('',(1.,0.,0.));", b + 6));
        root.push(format!("#{} = ( REPRESENTATION_RELATIONSHIP('','',#{},#{}) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{}) SHAPE_REPRESENTATION_RELATIONSHIP() );", b + 7, 10 + off, ROOT + 5, b + 2));
        root.push(format!("#{} = CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{},#{});", b + 8, b + 7, b + 1));
        targets.push(format!("#{}", b + 3));
    }
    let head = [
        format!("#{} = PRODUCT('ASSY','ASSY','',(#8));", ROOT + 1),
        format!("#{} = PRODUCT_DEFINITION_FORMATION('','',#{});", ROOT + 2, ROOT + 1),
        format!("#{} = PRODUCT_DEFINITION('design','',#{},#9);", ROOT + 3, ROOT + 2),
        format!("#{} = PRODUCT_DEFINITION_SHAPE('','',#{});", ROOT + 4, ROOT + 3),
        format!("#{} = SHAPE_REPRESENTATION('',(#{},{}),#{ctx0});", ROOT + 5, ROOT + 6, targets.join(",")),
        format!("#{} = AXIS2_PLACEMENT_3D('',#{},#{},#{});", ROOT + 6, ROOT + 7, ROOT + 8, ROOT + 9),
        format!("#{} = CARTESIAN_POINT('',(0.,0.,0.));", ROOT + 7),
        format!("#{} = DIRECTION('',(0.,0.,1.));", ROOT + 8),
        format!("#{} = DIRECTION('',(1.,0.,0.));", ROOT + 9),
        format!("#{} = SHAPE_DEFINITION_REPRESENTATION(#{},#{});", ROOT + 10, ROOT + 4, ROOT + 5),
    ];
    let mut out = String::from("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('ties test'),'2;1');\nFILE_NAME('ties','1979-01-01T00:00:00',('Author'),('fea-problem'),'fea-problem','fea-problem','Unknown');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));\nENDSEC;\nDATA;\n");
    for l in data.iter().chain(&head).chain(&root) {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    out
}

/// The cache's face (1-based name) that is flat, at height `z` and faces `sign` in z, among the faces of body `body`.
fn face_at(p: &Problem, body: usize, z: f64, sign: f64) -> String {
    let Geometry::Solid { cache: Some(c), .. } = &p.geometry else { unreachable!() };
    let hit = c.body_faces()[body].clone().find(|f| flat_face(c, f + 1).is_ok_and(|(m, n)| (m[2] - z).abs() < 1e-6 && n[2] * sign > 0.999)).unwrap();
    format!("face{}", hit + 1)
}

#[test]
fn an_assembly_read_from_step_is_advised_tied_and_solved_by_the_closed_form() {
    // two 20 x 10 x 5 bars stacked in z, a quarter of body 2 hanging off body 1's end
    let step = assembly(&[([0., 0., 0.], [20., 10., 5.]), ([5., 0., 5.], [25., 10., 10.])]);
    let mut p = Problem::from_step("Stacked bars", &step).unwrap();
    let Geometry::Solid { cache: Some(c), .. } = &p.geometry else { panic!("not a solid") };
    assert_eq!(c.bodies.len(), 2, "{:?}", c.bodies);
    assert_eq!(c.body_faces().iter().map(|r| r.len()).sum::<usize>(), c.face_names().len());
    let items = recommend_step(&step).unwrap();
    let advice = items.iter().find(|i| matches!(i, AssemblyAdvice::Tie { .. })).expect("the stacked bars touch: a tie is advised");
    let AssemblyAdvice::Tie { overlap_area, .. } = advice else { unreachable!() };
    assert!((overlap_area - 15.0 * 10.0).abs() < 1e-6, "{overlap_area}");
    assert!(p.ties.is_empty());
    p.apply_tie(advice).unwrap();
    p.apply_tie(advice).unwrap();
    assert_eq!(p.ties.len(), 1, "a tie already stored is not stored twice");
    // the mm / MPa system: 15 x 10 of the 20 x 10 contact face is shared, the smaller face (body 2's, 20 x 10 as well, the
    // equal one) is the slave; body 1 is held, body 2 is pulled
    p.convert_units(UnitSystem::MmMpa).unwrap();
    p.material.nu = 0.0;
    p.mesh = MeshSpec { size: 2.5, ..MeshSpec::default() };
    p.supports = vec![Support::fixed(&face_at(&p, 0, 0.0, -1.0))];
    p.loads = vec![Load::Traction { edge: face_at(&p, 1, 10.0, 1.0), tx: 0.0, ty: 0.0, tz: 10.0 }];
    let s = solve(&p, None).unwrap();
    assert!(s.summary.equilibrium_error < 1e-6, "{}", s.summary.equilibrium_error);
    // The load (10 MPa on 200 mm^2 = 2 kN) goes through 150 mm^2 of tie into the base bar; the top bar is pulled the full 10 MPa
    // over its own 5 mm, and the base bar's share is more than that of a column of its own (its stress is 2000 / 200 = 10 MPa
    // over the 5 mm below the tie; the overhang adds bending): the tip moves at least the two bars' sigma L / E.
    let want = 10.0 * 10.0 / p.material.e;
    assert!(s.summary.max_displacement.value >= 0.99 * want, "{} vs {want}", s.summary.max_displacement.value);
    assert!((s.summary.reaction[2] + 2000.0).abs() < 1e-3 * 2000.0, "{:?}", s.summary.reaction);
}

#[test]
fn only_a_tie_advice_item_can_be_applied() {
    let step = assembly(&[([0., 0., 0.], [20., 10., 5.]), ([0., 0., 5.], [20., 10., 10.])]);
    let mut p = Problem::from_step("Stacked bars", &step).unwrap();
    let unresolved = AssemblyAdvice::Unresolved { a: solid_kernel::features::FaceKey { body: 0, face: 1 }, b: solid_kernel::features::FaceKey { body: 1, face: 1 }, reason: String::new() };
    assert!(p.apply_tie(&unresolved).unwrap_err().contains("only a Tie"));
    // a single part has no assembly to tie
    let single = NativeBrep::box_from_corners([0., 0., 0.], [20., 10., 5.]).unwrap().to_step_text();
    let mut one = Problem::from_step("one", &single).unwrap();
    let tie = recommend_step(&step).unwrap().into_iter().find(|i| matches!(i, AssemblyAdvice::Tie { .. })).unwrap();
    assert!(one.apply_tie(&tie).is_err());
    assert_eq!(text_hash(&step).len(), 16);
    p.apply_tie(&tie).unwrap();
    // the equal bars: tied and pulled, the stack moves sigma L / E
    p.convert_units(UnitSystem::MmMpa).unwrap();
    p.material.nu = 0.0;
    p.mesh = MeshSpec { size: 2.5, ..MeshSpec::default() };
    p.supports = vec![Support::fixed(&face_at(&p, 0, 0.0, -1.0))];
    p.loads = vec![Load::Traction { edge: face_at(&p, 1, 10.0, 1.0), tx: 0.0, ty: 0.0, tz: 10.0 }];
    let s = solve(&p, None).unwrap();
    let want = 10.0 * 10.0 / p.material.e;
    assert!((s.summary.max_displacement.value / want - 1.0).abs() < 1e-6, "{} vs {want}", s.summary.max_displacement.value);
}
