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

// ---------------------------------------------------------------------------------------------------------------------------
// Contact ties (Phase C, second slice)
// ---------------------------------------------------------------------------------------------------------------------------
/// [`stack`] with its joint a contact of friction `mu`, pushed at the tip (a compression the contact can carry) or pulled (which
/// opens it).
fn contact_stack(mu: f64, push: bool) -> Problem {
    let mut p = stack([1.0, 1.0], 0.0, 0.0);
    p.ties = vec![Tie { friction: Some(mu), ..tie([1.0, 1.0], 0.0, 0.0) }];
    if push {
        p.loads = vec![Load::Traction { edge: "face12".into(), tx: 0.0, ty: 0.0, tz: -PULL }];
    }
    p
}

/// The z displacement of the nodes at height `z`, as (smallest, largest).
fn uz_at(s: &fea_problem::solve::Solved, z: f64) -> (f64, f64) {
    let uz = s.model.mesh.nodes.iter().enumerate().filter(|(_, x)| (x[2] - z).abs() < 1e-9).map(|(i, _)| s.u[3 * i + 2]);
    uz.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

#[test]
fn a_frictionless_contact_carries_a_push_by_the_closed_form_in_linear_and_quadratic_elements() {
    for element in [ElementChoice::Tri3, ElementChoice::Tri6] {
        let mut p = contact_stack(0.0, true);
        p.mesh.element = element;
        let s = solve(&p, None).unwrap();
        // sigma L / E per bar: the joint closes with no overlap and transmits the whole push
        let bar = PULL / p.material.e;
        let (lo, hi) = uz_at(&s, 1.0);
        assert!((lo + bar).abs() < 1e-5 * bar && (hi + bar).abs() < 1e-5 * bar, "{element:?}: both faces of the joint move one bar's shortening: {lo:e} {hi:e} vs {:e}", -bar);
        let (lo, hi) = uz_at(&s, 2.0);
        assert!((lo + 2.0 * bar).abs() < 1e-5 * bar && (hi + 2.0 * bar).abs() < 1e-5 * bar, "{element:?}: {lo:e} {hi:e}");
        let c = &s.summary.contacts[0];
        assert!((c.normal_force / PULL - 1.0).abs() < 1e-5 && c.contact_share > 0.999 && c.tangential_force == 0.0, "{c:?}");
        assert!(s.summary.equilibrium_error < 1e-6, "{element:?}: {}", s.summary.equilibrium_error);
    }
}

#[test]
fn a_frictional_contact_under_a_push_carries_it_and_no_shear() {
    // The overlap that engages a frictional contact from the start (`PairSetup::touch`, 1e-5 of a face) offsets the two faces
    // by that much; a push that shortens the stack by 3.4e-3 reads it as 0.15 %.
    let mut p = contact_stack(0.3, true);
    let push = 5.0e4;
    p.loads = vec![Load::Traction { edge: "face12".into(), tx: 0.0, ty: 0.0, tz: -push }];
    let s = solve(&p, None).unwrap();
    let want = 2.0 * push / p.material.e;
    let (lo, hi) = uz_at(&s, 2.0);
    assert!((lo + want).abs() < 5e-3 * want && (hi + want).abs() < 5e-3 * want, "{lo:e} {hi:e} vs {:e}", -want);
    let c = &s.summary.contacts[0];
    assert!((c.normal_force / push - 1.0).abs() < 1e-4 && c.contact_share > 0.999 && c.slip_share == 0.0, "{c:?}");
    assert!(c.tangential_force < 1e-4 * push, "{c:?}");
    assert!(s.summary.equilibrium_error < 1e-6, "{}", s.summary.equilibrium_error);
}

#[test]
fn a_pull_that_opens_the_contact_is_refused_with_the_reason() {
    let err = solve(&contact_stack(0.0, false), None).unwrap_err();
    assert!(err.contains("contact could not be solved") && err.contains("presses the bodies together"), "{err}");
}

#[test]
fn contact_ties_are_validated_saved_and_kept_apart_from_bonded_ones() {
    let mut p = contact_stack(0.3, true);
    assert!(p.validate().is_ok(), "{:?}", p.validate());
    let back: Problem = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back.ties, p.ties);
    assert!(back.ties[0].is_contact());
    // a bonded tie writes no `friction` key
    assert!(!serde_json::to_string(&tie([1.0, 1.0], 0.0, 0.0)).unwrap().contains("friction"));
    // friction is a coefficient in 0..2
    for mu in [-0.1, 2.5, f64::NAN] {
        p.ties[0].friction = Some(mu);
        assert!(p.validate().unwrap_err().contains("friction must be in 0..2"), "{mu}");
    }
    // one joint of each kind is solved together since Phase C follow-up 2 (the bonded one by a stiff penalty in the contact solve)
    p.ties[0].friction = Some(0.0);
    p.ties.push(tie([1.0, 1.0], 0.0, 0.0));
    assert!(p.validate().is_ok(), "{:?}", p.validate());
    // the faces are named by what they are: a unit change keeps a contact a contact and still resolves
    let mut q = contact_stack(0.3, true);
    q.convert_units(UnitSystem::MmMpa).unwrap();
    assert_eq!(q.ties[0].friction, Some(0.3));
    assert!(q.validate().is_ok(), "{:?}", q.validate());
}

#[test]
fn an_assembly_read_from_step_takes_a_contact_from_its_advice() {
    let step = assembly(&[([0., 0., 0.], [20., 10., 5.]), ([0., 0., 5.], [20., 10., 10.])]);
    let mut p = Problem::from_step("Stacked bars", &step).unwrap();
    let advice = recommend_step(&step).unwrap().into_iter().find(|i| matches!(i, AssemblyAdvice::Tie { .. })).unwrap();
    p.apply_contact(&advice, 0.0).unwrap();
    p.apply_contact(&advice, 0.0).unwrap();
    assert_eq!(p.ties.len(), 1);
    assert_eq!(p.ties[0].friction, Some(0.0));
    p.convert_units(UnitSystem::MmMpa).unwrap();
    p.material.nu = 0.0;
    p.mesh = MeshSpec { size: 2.5, ..MeshSpec::default() };
    p.supports = vec![Support::fixed(&face_at(&p, 0, 0.0, -1.0))];
    p.loads = vec![Load::Traction { edge: face_at(&p, 1, 10.0, 1.0), tx: 0.0, ty: 0.0, tz: -10.0 }];
    let s = solve(&p, None).unwrap();
    // 10 MPa through two bars of 5 mm
    let want = 10.0 * 10.0 / p.material.e;
    assert!((s.summary.max_displacement.value / want - 1.0).abs() < 1e-5, "{} vs {want}", s.summary.max_displacement.value);
    assert!((s.summary.contacts[0].normal_force / 2000.0 - 1.0).abs() < 1e-5, "{:?}", s.summary.contacts);
}

// ---------------------------------------------------------------------------------------------------------------------------
// Per-body materials (Phase C follow-up 1): body k > 0 takes `Problem::region_materials[k - 1]`
// ---------------------------------------------------------------------------------------------------------------------------

/// [`stack`] whose second body is `ratio` times as stiff as the first (no Poisson coupling), joined by `friction` (`None` bonded).
fn two_materials(ratio: f64, friction: Option<f64>) -> Problem {
    let mut p = stack([1.0, 1.0], 0.0, 0.0);
    p.ties = vec![Tie { friction, ..tie([1.0, 1.0], 0.0, 0.0) }];
    let mut second = p.material.clone();
    second.name = "second body".into();
    second.e *= ratio;
    p.region_materials = vec![Some(second)];
    p
}

fn node_at(s: &fea_problem::solve::Solved, z: f64) -> usize {
    s.model.mesh.nodes.iter().position(|x| (x[2] - z).abs() < 1e-9).unwrap()
}

#[test]
fn a_bonded_stack_of_two_materials_moves_by_the_series_closed_form() {
    // Uniform axial stress in two bars of unit length: the tip moves sigma (L1 / E1 + L2 / E2), the joint sigma L1 / E1. The
    // state is exact in both element orders, so the check is to rounding.
    for (element, ratio) in [(ElementChoice::Tri3, 3.0), (ElementChoice::Tri6, 3.0), (ElementChoice::Tri3, 0.25)] {
        let mut p = two_materials(ratio, None);
        p.mesh.element = element;
        let (e1, e2) = (p.material.e, p.material.e * ratio);
        let s = solve(&p, None).unwrap();
        let (lo, hi) = uz_at(&s, 1.0);
        let joint = PULL / e1;
        assert!((lo / joint - 1.0).abs() < 1e-6 && (hi / joint - 1.0).abs() < 1e-6, "{element:?} {ratio}: joint {lo:e} {hi:e} vs {joint:e}");
        let want = PULL * (1.0 / e1 + 1.0 / e2);
        let (lo, hi) = uz_at(&s, 2.0);
        assert!((lo / want - 1.0).abs() < 1e-6 && (hi / want - 1.0).abs() < 1e-6, "{element:?} {ratio}: tip {lo:e} {hi:e} vs {want:e}");
        assert!(s.summary.equilibrium_error < 1e-6);
        // the stiffer body is meshed and solved with its own modulus: the energy is U = F^2/2 (L1/E1 + L2/E2) / A
        let energy = 0.5 * PULL * PULL * (1.0 / e1 + 1.0 / e2);
        assert!((s.summary.strain_energy / energy - 1.0).abs() < 1e-6, "{element:?} {ratio}: energy {:e} vs {energy:e}", s.summary.strain_energy);
    }
}

#[test]
fn a_frictionless_contact_of_two_materials_carries_a_push_by_the_series_closed_form() {
    for element in [ElementChoice::Tri3, ElementChoice::Tri6] {
        let mut p = two_materials(3.0, Some(0.0));
        p.mesh.element = element;
        p.loads = vec![Load::Traction { edge: "face12".into(), tx: 0.0, ty: 0.0, tz: -PULL }];
        let (e1, e2) = (p.material.e, p.material.e * 3.0);
        let s = solve(&p, None).unwrap();
        let (lo, hi) = uz_at(&s, 1.0);
        let joint = -PULL / e1;
        assert!((lo / joint - 1.0).abs() < 1e-5 && (hi / joint - 1.0).abs() < 1e-5, "{element:?}: joint {lo:e} {hi:e} vs {joint:e}");
        let want = -PULL * (1.0 / e1 + 1.0 / e2);
        let (lo, hi) = uz_at(&s, 2.0);
        assert!((lo / want - 1.0).abs() < 1e-5 && (hi / want - 1.0).abs() < 1e-5, "{element:?}: tip {lo:e} {hi:e} vs {want:e}");
        assert!((s.summary.contacts[0].normal_force / PULL - 1.0).abs() < 1e-5);
        assert!(s.summary.equilibrium_error < 1e-6);
    }
}

#[test]
fn a_body_material_equal_to_the_problem_material_changes_nothing() {
    // differential: the explicit copy of the material gives the identical solve
    for friction in [None, Some(0.0)] {
        let mut same = two_materials(1.0, friction);
        let mut alone = same.clone();
        alone.region_materials.clear();
        if friction.is_some() {
            for p in [&mut same, &mut alone] {
                p.loads = vec![Load::Traction { edge: "face12".into(), tx: 0.0, ty: 0.0, tz: -PULL }];
            }
        }
        let (a, b) = (solve(&same, None).unwrap(), solve(&alone, None).unwrap());
        assert_eq!(a.u, b.u, "{friction:?}");
    }
}

#[test]
fn strain_fields_read_each_body_with_its_own_modulus() {
    // With nu = 0 the axial strain is sigma / E of the body the node belongs to; the problem's own modulus would read the stiff
    // body's strain three times too large.
    let p = two_materials(3.0, None);
    let s = solve(&p, None).unwrap();
    let strain = s.node_values(fea_problem::solve::Field::MaxPrincipalStrain);
    let (base, tip) = (strain[node_at(&s, 0.0)], strain[node_at(&s, 2.0)]);
    assert!((base / (PULL / p.material.e) - 1.0).abs() < 1e-6, "{base:e}");
    assert!((tip / (PULL / (3.0 * p.material.e)) - 1.0).abs() < 1e-6, "{tip:e}");
    // von Mises stress is the same in both bars (one force through equal sections)
    let vm = s.node_values(fea_problem::solve::Field::VonMises);
    assert!((vm[node_at(&s, 0.0)] / PULL - 1.0).abs() < 1e-6 && (vm[node_at(&s, 2.0)] / PULL - 1.0).abs() < 1e-6);
}

#[test]
fn body_materials_are_validated_scaled_and_saved() {
    let mut p = two_materials(3.0, None);
    assert!(p.validate().is_ok(), "{:?}", p.validate());
    // a body that does not exist
    let mut extra = p.clone();
    extra.region_materials.push(Some(p.material.clone()));
    assert!(extra.validate().unwrap_err().contains("3 body materials for 2 bodies"), "{:?}", extra.validate());
    // a material that is not a material
    let mut bad = p.clone();
    bad.region_materials[0].as_mut().unwrap().e = -1.0;
    assert!(bad.validate().unwrap_err().contains("body 2 material"), "{:?}", bad.validate());
    // a single solid has no second body
    let mut one = stack([1.0, 1.0], 0.0, 0.0);
    let Geometry::Solid { cache: Some(c), .. } = &mut one.geometry else { unreachable!() };
    c.bodies.clear();
    one.region_materials = vec![Some(one.material.clone())];
    assert!(one.validate().unwrap_err().contains("body materials for 1 bodies"), "{:?}", one.validate());
    // the unit change scales the second body's modulus with the first's, and the JSON keeps it
    let back: Problem = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back.region_materials, p.region_materials);
    let (e1, e2) = (p.material.e, p.region_materials[0].as_ref().unwrap().e);
    p.convert_units(UnitSystem::MmMpa).unwrap();
    let k = p.material.e / e1;
    assert!((p.region_materials[0].as_ref().unwrap().e / e2 - k).abs() < 1e-9 * k);
    assert!(p.validate().is_ok(), "{:?}", p.validate());
}

// ---------------------------------------------------------------------------------------------------------------------------
// Bonded and contact ties together (Phase C follow-up 2): the bonded ones are penalty couplings in the contact solve
// ---------------------------------------------------------------------------------------------------------------------------

/// Unit bars of footprint 1 x 1 and height 1 at `(x0, z0)` each, as one assembly (body `k` has faces `face{6k+1}..face{6k+6}`).
fn bars(list: &[(f64, f64)]) -> Problem {
    let mut m = SurfaceMesh::default();
    for (k, &(x0, z0)) in list.iter().enumerate() {
        box_at(&mut m, [x0, 0.0, z0], [1.0, 1.0, 1.0], 6 * k);
    }
    let mut cache = SolidCache::new(STEP, m, 0.01);
    cache.bodies = vec![6; list.len()];
    let mut p = templates::blank();
    p.name = "Bars".into();
    p.set_solid(STEP, cache);
    p.material.nu = 0.0;
    p.mesh = MeshSpec { size: 0.5, ..MeshSpec::default() };
    p
}

fn bottom(k: usize) -> String {
    format!("face{}", 6 * k + 5)
}

fn top(k: usize) -> String {
    format!("face{}", 6 * k + 6)
}

/// The tie of bar `a`'s top face and bar `b`'s bottom face (`friction` `None` bonded).
fn joint(list: &[(f64, f64)], a: usize, b: usize, friction: Option<f64>) -> Tie {
    let ((xa, za), (xb, zb)) = (list[a], list[b]);
    Tie {
        a: FaceSig { body: a, centroid: [xa + 0.5, 0.5, za + 1.0], normal: [0.0, 0.0, 1.0], area: 1.0 },
        b: FaceSig { body: b, centroid: [xb + 0.5, 0.5, zb], normal: [0.0, 0.0, -1.0], area: 1.0 },
        gap: 0.0,
        friction,
    }
}

/// The z displacement of the nodes at height `z` with `x` in `[lo, hi]`, as (smallest, largest).
fn uz_in(s: &fea_problem::solve::Solved, z: f64, lo: f64, hi: f64) -> (f64, f64) {
    let uz = s.model.mesh.nodes.iter().enumerate().filter(|(_, x)| (x[2] - z).abs() < 1e-9 && x[0] >= lo - 1e-9 && x[0] <= hi + 1e-9).map(|(i, _)| s.u[3 * i + 2]);
    uz.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)))
}

fn scaled(p: &mut Problem, ratios: &[f64]) {
    p.region_materials = ratios
        .iter()
        .map(|r| {
            let mut m = p.material.clone();
            m.e *= r;
            Some(m)
        })
        .collect();
}

/// The relative distance of `got` from `want`, printed so the penalty's error is on record in the test log.
fn rel(what: &str, got: f64, want: f64) -> f64 {
    let r = (got / want - 1.0).abs();
    eprintln!("{what}: {got:e} vs {want:e}, relative error {r:.2e}");
    r
}

#[test]
fn a_bonded_pull_and_a_contact_push_are_each_carried_by_the_closed_form() {
    // Two independent columns in one problem. Column A (bars 0, 1) is bonded and pulled: the joint carries a tension, which a
    // contact never could. Column B (bars 2, 3) is a frictionless contact and pushed. Each is a uniform axial state with nu = 0.
    let list = [(0.0, 0.0), (0.0, 1.0), (3.0, 0.0), (3.0, 1.0)];
    for element in [ElementChoice::Tri3, ElementChoice::Tri6] {
        let mut p = bars(&list);
        p.mesh.element = element;
        scaled(&mut p, &[2.0, 0.5, 4.0]);
        let e = p.material.e;
        p.ties = vec![joint(&list, 0, 1, None), joint(&list, 2, 3, Some(0.0))];
        p.supports = vec![Support::fixed(&bottom(0)), Support::fixed(&bottom(2))];
        let push = 30.0;
        p.loads = vec![Load::Traction { edge: top(1), tx: 0.0, ty: 0.0, tz: PULL }, Load::Traction { edge: top(3), tx: 0.0, ty: 0.0, tz: -push }];
        let s = solve(&p, None).unwrap();
        let (lo, hi) = uz_in(&s, 1.0, -1.0, 2.0);
        let joint_a = PULL / e;
        assert!(rel("A joint low", lo, joint_a) < 2e-7 && rel("A joint high", hi, joint_a) < 2e-7, "{element:?}");
        let (lo, hi) = uz_in(&s, 2.0, -1.0, 2.0);
        let tip_a = PULL * (1.0 / e + 1.0 / (2.0 * e));
        assert!(rel("A tip low", lo, tip_a) < 2e-7 && rel("A tip high", hi, tip_a) < 2e-7, "{element:?}");
        let (lo, hi) = uz_in(&s, 1.0, 2.0, 5.0);
        let joint_b = -push / (0.5 * e);
        assert!(rel("B joint low", lo, joint_b) < 1e-6 && rel("B joint high", hi, joint_b) < 1e-6, "{element:?}");
        let (lo, hi) = uz_in(&s, 2.0, 2.0, 5.0);
        let tip_b = -push * (1.0 / (0.5 * e) + 1.0 / (4.0 * e));
        assert!(rel("B tip low", lo, tip_b) < 1e-6 && rel("B tip high", hi, tip_b) < 1e-6, "{element:?}");
        let c = &s.summary.contacts[0];
        assert!((c.normal_force / push - 1.0).abs() < 1e-5 && c.contact_share > 0.999, "{c:?}");
        assert!(s.summary.equilibrium_error < 1e-6, "{element:?}: {}", s.summary.equilibrium_error);
        // the reactions are the loads, to the penalty's accuracy
        assert!((s.summary.reaction[2] + s.summary.applied[2]).abs() < 1e-6 * PULL, "{:?} {:?}", s.summary.reaction, s.summary.applied);
        assert!(s.summary.notes.iter().any(|w| w.contains("held by a stiff penalty")), "{:?}", s.summary.notes);
    }
}

#[test]
fn a_column_bonded_then_in_contact_moves_by_the_series_closed_form_and_matches_the_exact_elimination() {
    // Bar 0 fixed at the base, bar 1 bonded to it, bar 2 a frictionless contact on bar 1 and pushed at its top.
    let list = [(0.0, 0.0), (0.0, 1.0), (0.0, 2.0)];
    let mut p = bars(&list);
    scaled(&mut p, &[2.0, 4.0]);
    let e = p.material.e;
    p.ties = vec![joint(&list, 0, 1, None), joint(&list, 1, 2, Some(0.0))];
    p.supports = vec![Support::fixed(&bottom(0))];
    p.loads = vec![Load::Traction { edge: top(2), tx: 0.0, ty: 0.0, tz: -PULL }];
    let s = solve(&p, None).unwrap();
    let want = [-PULL / e, -PULL * (1.0 / e + 0.5 / e), -PULL * (1.0 / e + 0.5 / e + 0.25 / e)];
    for (k, w) in want.iter().enumerate() {
        let (lo, hi) = uz_at(&s, 1.0 + k as f64);
        assert!(rel(&format!("z={}", k + 1), lo, *w) < 2e-6 && rel(&format!("z={}", k + 1), hi, *w) < 2e-6, "z={}: {lo:e} {hi:e} vs {w:e}", k + 1);
    }
    assert!(s.summary.equilibrium_error < 1e-6);
    // differential: the same two bars with only the bonded tie, solved by exact elimination under the force the contact passes on
    let two = [(0.0, 0.0), (0.0, 1.0)];
    let mut q = bars(&two);
    scaled(&mut q, &[2.0]);
    q.ties = vec![joint(&two, 0, 1, None)];
    q.supports = vec![Support::fixed(&bottom(0))];
    q.loads = vec![Load::Traction { edge: top(1), tx: 0.0, ty: 0.0, tz: -PULL }];
    let exact = solve(&q, None).unwrap();
    for z in [1.0, 2.0] {
        let (a, b) = (uz_at(&exact, z).0, uz_at(&s, z).0);
        assert!(rel(&format!("penalty vs elimination z={z}"), b, a) < 2e-6, "z={z}: {b:e} vs {a:e}");
    }
}

#[test]
fn bonded_ties_that_a_contact_solve_cannot_join_are_still_refused_with_the_reason() {
    // a bonded tie between faces that do not touch joins no node, in the contact solve as in the linear one
    let list = [(0.0, 0.0), (0.0, 1.5), (0.0, 2.5)];
    let mut p = bars(&list);
    p.ties = vec![joint(&list, 0, 1, None), joint(&list, 1, 2, Some(0.0))];
    p.supports = vec![Support::fixed(&bottom(0))];
    p.loads = vec![Load::Traction { edge: top(2), tx: 0.0, ty: 0.0, tz: -PULL }];
    let err = solve(&p, None).unwrap_err();
    assert!(err.contains("tie"), "{err}");
}

// ---------------------------------------------------------------------------------------------------------------------------
// Gap-closing contact (Phase C follow-up 3): a body nothing holds, a gap away, is moved to the contact before the solve
// ---------------------------------------------------------------------------------------------------------------------------

/// [`bars`] in a column with `gap` between consecutive bars, a frictional-or-frictionless contact (`mu`) at every joint, the first
/// bar fixed at its base and a push at the top of the last (the weights of the bars' materials are `ratios`).
fn gapped_column(n: usize, gap: f64, mu: f64) -> (Problem, Vec<(f64, f64)>) {
    let list: Vec<(f64, f64)> = (0..n).map(|k| (0.0, k as f64 * (1.0 + gap))).collect();
    let mut p = bars(&list);
    p.ties = (0..n - 1)
        .map(|k| {
            let mut t = joint(&list, k, k + 1, Some(mu));
            t.gap = gap;
            t
        })
        .collect();
    p.supports = vec![Support::fixed(&bottom(0))];
    (p, list)
}

#[test]
fn a_body_a_gap_away_is_moved_to_the_contact_and_carries_the_push_by_the_closed_form() {
    // Bar 1 starts `gap` above bar 0 and is held by nothing but the contact. It travels the gap and then shortens with the stack:
    // its bottom moves `-(sigma L0/E0 + gap)`, its top `-(sigma (L0/E0 + L1/E1) + gap)`.
    let gap = 1.0e-3;
    for element in [ElementChoice::Tri3, ElementChoice::Tri6] {
        let (mut p, list) = gapped_column(2, gap, 0.0);
        p.mesh.element = element;
        scaled(&mut p, &[3.0]);
        let e = p.material.e;
        p.loads = vec![Load::Traction { edge: top(1), tx: 0.0, ty: 0.0, tz: -5.0e4 }];
        let s = solve(&p, None).unwrap();
        let sigma = 5.0e4;
        let (lo, hi) = uz_at(&s, 1.0);
        let joint0 = -sigma / e;
        assert!(rel("lower face of the joint", lo, joint0) < 1e-5 && rel("lower face of the joint", hi, joint0) < 1e-5, "{element:?}");
        let _ = &list;
        let (lo, hi) = uz_at(&s, 1.0 + gap);
        let want = -(sigma / e + gap);
        assert!(rel("upper face of the joint", lo, want) < 1e-6 && rel("upper face of the joint", hi, want) < 1e-6, "{element:?}: {lo:e} {hi:e} vs {want:e}");
        let (lo, hi) = uz_at(&s, 2.0 + gap);
        let want = -(sigma / e + sigma / (3.0 * e) + gap);
        assert!(rel("tip", lo, want) < 1e-6 && rel("tip", hi, want) < 1e-6, "{element:?}: {lo:e} {hi:e} vs {want:e}");
        let c = &s.summary.contacts[0];
        assert!((c.normal_force / sigma - 1.0).abs() < 1e-5 && c.contact_share > 0.999, "{c:?}");
        assert!(s.summary.equilibrium_error < 1e-6, "{element:?}: {}", s.summary.equilibrium_error);
    }
}

#[test]
fn two_gaps_in_a_column_close_one_after_the_other() {
    let gap = 2.0e-3;
    let (mut p, _) = gapped_column(3, gap, 0.0);
    let sigma = 5.0e4;
    p.loads = vec![Load::Traction { edge: top(2), tx: 0.0, ty: 0.0, tz: -sigma }];
    let e = p.material.e;
    let s = solve(&p, None).unwrap();
    // bar k's top moves -((k + 1) sigma/E + k gap)
    for k in 0..3 {
        let z = (k + 1) as f64 * (1.0 + gap) - gap;
        let (lo, hi) = uz_at(&s, z);
        let want = -((k + 1) as f64 * sigma / e + k as f64 * gap);
        assert!(rel(&format!("top of bar {k}"), lo, want) < 1e-5 && rel(&format!("top of bar {k}"), hi, want) < 1e-5, "bar {k}: {lo:e} {hi:e} vs {want:e}");
    }
    // The contact solve stops at a residual of 1e-6 of its force scale (`Tuning::frictionless`), the sum of the element forces of
    // three bars: the base reaction may miss the push by a few of those, measured 1.5e-6 here.
    assert!(s.summary.equilibrium_error < 3e-6, "{} {:?} {:?}", s.summary.equilibrium_error, s.summary.reaction, s.summary.applied);
}

#[test]
fn the_loose_body_may_be_the_lower_one() {
    // Bar 1 (the upper) is fixed at its top; bar 0 (the lower) is held by nothing and pushed up into it across the gap.
    let gap = 1.0e-3;
    let (mut p, _) = gapped_column(2, gap, 0.0);
    p.supports = vec![Support::fixed(&top(1))];
    let sigma = 5.0e4;
    p.loads = vec![Load::Traction { edge: bottom(0), tx: 0.0, ty: 0.0, tz: sigma }];
    let e = p.material.e;
    let s = solve(&p, None).unwrap();
    // the fixed top is at z = 2 + gap; bar 0 rises by the gap and its own shortening and bar 1 shortens too
    let (lo, hi) = uz_at(&s, 0.0);
    let want = gap + 2.0 * sigma / e;
    assert!(rel("bottom of the loose bar", lo, want) < 1e-5 && rel("bottom of the loose bar", hi, want) < 1e-5, "{lo:e} {hi:e} vs {want:e}");
    assert!(s.summary.equilibrium_error < 1e-6);
}

#[test]
fn a_gap_with_friction_closes_and_carries_the_push_and_no_shear() {
    let gap = 1.0e-3;
    let (mut p, _) = gapped_column(2, gap, 0.3);
    let sigma = 5.0e4;
    p.loads = vec![Load::Traction { edge: top(1), tx: 0.0, ty: 0.0, tz: -sigma }];
    let e = p.material.e;
    let s = solve(&p, None).unwrap();
    let (lo, hi) = uz_at(&s, 2.0 + gap);
    let want = -(2.0 * sigma / e + gap);
    // the frictional pair keeps its engaging overlap (1e-5 of a face): 0.3 % of this push's shortening
    assert!(rel("tip", lo, want) < 5e-3 && rel("tip", hi, want) < 5e-3, "{lo:e} {hi:e} vs {want:e}");
    let c = &s.summary.contacts[0];
    assert!((c.normal_force / sigma - 1.0).abs() < 1e-4 && c.slip_share == 0.0 && c.tangential_force < 1e-4 * sigma, "{c:?}");
}

#[test]
fn a_gap_that_a_bonded_tie_holds_is_closed_too() {
    // bar 0 fixed, bar 1 bonded to it, bar 2 a gap above bar 1 and held by nothing but the contact
    let gap = 1.0e-3;
    let list = [(0.0, 0.0), (0.0, 1.0), (0.0, 2.0 + gap)];
    let mut p = bars(&list);
    let mut t = joint(&list, 1, 2, Some(0.0));
    t.gap = gap;
    p.ties = vec![joint(&list, 0, 1, None), t];
    p.supports = vec![Support::fixed(&bottom(0))];
    let sigma = 5.0e4;
    p.loads = vec![Load::Traction { edge: top(2), tx: 0.0, ty: 0.0, tz: -sigma }];
    let e = p.material.e;
    let s = solve(&p, None).unwrap();
    let (lo, hi) = uz_at(&s, 3.0 + gap);
    let want = -(3.0 * sigma / e + gap);
    assert!(rel("tip", lo, want) < 2e-6 && rel("tip", hi, want) < 2e-6, "{lo:e} {hi:e} vs {want:e}");
}

#[test]
fn a_pull_after_the_gap_still_opens_the_contact_and_is_refused() {
    // Closing the gap does not make a contact carry a tension: the pull is refused with the reason (a bonded or fastened path
    // carries it).
    let (mut p, _) = gapped_column(2, 1.0e-3, 0.0);
    p.loads = vec![Load::Traction { edge: top(1), tx: 0.0, ty: 0.0, tz: 5.0e4 }];
    let err = solve(&p, None).unwrap_err();
    assert!(err.contains("contact could not be solved") && err.contains("presses the bodies together"), "{err}");
}

// ---------------------------------------------------------------------------------------------------------------------------
// Preloaded fasteners (Phase C follow-up 4): the bolt is a spring between the head's and the nut's bearing faces
// ---------------------------------------------------------------------------------------------------------------------------

use fea_problem::{Fastener, Preload};

/// Two unit bars (`E` and `ratio E`) stacked, bar 0 fixed at its base: that base is the nut's bearing plane (z = 0), the top of bar 1 the
/// head's (z = 2). Every bearing face is inside the bearing diameter, so the bolt acts on the whole face and the state stays uniform.
fn bolted(interface: Option<f64>, ratio: f64, size: &str, preload: Preload) -> (Problem, [f64; 2]) {
    let list = [(0.0, 0.0), (0.0, 1.0)];
    let mut p = bars(&list);
    scaled(&mut p, &[ratio]);
    p.ties = vec![joint(&list, 0, 1, interface)];
    p.supports = vec![Support::fixed(&bottom(0))];
    let mut f = Fastener::new([0.5, 0.5, 0.0], [0.0, 0.0, 1.0], 0.0, 2.0, size, preload);
    f.head_diameter = Some(100.0);
    f.nut_diameter = Some(100.0);
    p.fasteners = vec![f];
    let e = p.material.e;
    (p, [e, ratio * e])
}

/// The bolt's axial stiffness from first principles for `size` and a grip of `grip`: `1 / sum(L / (E A))` over the head's additional
/// length (0.4 d at d), the shank and the thread's (0.4 d3 at the root diameter d3), in the problem's units.
fn bolt_stiffness(p: &Problem, size: &str, grip: f64, e_bolt: f64) -> f64 {
    let (d, pitch) = match size {
        "M6" => (6.0, 1.0),
        "M10" => (10.0, 1.5),
        _ => unreachable!(),
    };
    let mm = 1e-3 / p.units.length_m();
    let (d, d3) = (d * mm, (d - 1.082_532 * pitch) * mm);
    let area = |x: f64| std::f64::consts::FRAC_PI_4 * x * x;
    1.0 / ((0.4 * d + grip) / (e_bolt * area(d)) + 0.4 * d3 / (e_bolt * area(d3)))
}

/// Closed forms of a two-bar stack under a bolt (`k_m` the members' series stiffness): preload `P`, then a pull `F` on the head.
struct Bolted {
    k_b: f64,
    k_m: f64,
}

impl Bolted {
    fn phi(&self) -> f64 {
        self.k_b / (self.k_b + self.k_m)
    }
}

fn bolted_case(interface: Option<f64>, ratio: f64, size: &str, preload: f64, bolt_scale: f64, element: ElementChoice) -> (Problem, Bolted) {
    let (mut p, e) = bolted(interface, ratio, size, Preload::Force(preload));
    p.mesh.element = element;
    let e_bolt = bolt_scale * 200e9 / p.units.stress_pa();
    p.fasteners[0].bolt_modulus = Some(e_bolt);
    let k_m = 1.0 / (1.0 / e[0] + 1.0 / e[1]);
    let k_b = bolt_stiffness(&p, size, 2.0, e_bolt);
    (p, Bolted { k_b, k_m })
}

#[test]
fn a_preload_alone_clamps_the_members_by_the_closed_form_and_the_bolt_carries_it() {
    let preload = 1.0e4;
    for interface in [None, Some(0.0)] {
        for element in [ElementChoice::Tri3, ElementChoice::Tri6] {
            let (p, c) = bolted_case(interface, 0.5, "M6", preload, 10.0, element);
            let s = solve(&p, None).unwrap();
            // the members shorten by P / k_m (series of the two bars), the bolt carries P
            let (lo, hi) = uz_at(&s, 2.0);
            let want = -preload / c.k_m;
            let tol = if interface.is_none() { 1e-5 } else { 1e-6 };
            assert!(rel("head", lo, want) < tol && rel("head", hi, want) < tol, "{interface:?} {element:?}");
            let f = &s.summary.fasteners[0];
            assert!(rel("preload", f.preload, preload) < 1e-12 && rel("stiffness", f.stiffness, c.k_b) < 1e-12, "{f:?}");
            assert!(rel("tension", f.tension, preload) < tol, "{f:?}");
            assert!(s.summary.equilibrium_error < 1e-6, "{}", s.summary.equilibrium_error);
        }
    }
}

#[test]
fn a_pull_adds_phi_of_itself_to_the_bolt_and_takes_the_rest_from_the_clamp() {
    // T = P + Phi F, clamp = P - (1 - Phi) F, head u = -P / k_m + F / (k_b + k_m): below the separation load F = P / (1 - Phi) = P (1 + k_b / k_m), where the clamp reaches zero.
    let preload = 1.0e4;
    for (scale, size) in [(10.0, "M6"), (50.0, "M10")] {
        for interface in [None, Some(0.0)] {
            for element in [ElementChoice::Tri3, ElementChoice::Tri6] {
                let (mut p, c) = bolted_case(interface, 2.0, size, preload, scale, element);
                let pull = 0.8 * preload * (1.0 + c.k_b / c.k_m);
                p.loads = vec![Load::Traction { edge: top(1), tx: 0.0, ty: 0.0, tz: pull }];
                let s = solve(&p, None).unwrap();
                // the contact solve's own tolerance (1e-6 of the force scale) over a clamp of 0.2 P, the bonded penalty's error 1e-5
                let tol = if interface.is_none() { 1e-5 } else { 1e-5 };
                let tension = preload + c.phi() * pull;
                let f = &s.summary.fasteners[0];
                assert!(rel("tension", f.tension, tension) < tol, "{size} {interface:?} {element:?}: {f:?} vs {tension:e} (Phi {:.3})", c.phi());
                let (lo, hi) = uz_at(&s, 2.0);
                let head = -preload / c.k_m + pull / (c.k_b + c.k_m);
                // the head is the small difference of two larger terms (P / k_m and F / (k_b + k_m)): the solve's 1e-6 of the force scale
                // is a bound on its error relative to those terms, not to the difference
                let scale = preload / c.k_m + pull / (c.k_b + c.k_m);
                eprintln!("head, relative to its terms: {:.2e} {:.2e}", (lo - head).abs() / scale, (hi - head).abs() / scale);
                assert!((lo - head).abs() < tol * scale && (hi - head).abs() < tol * scale, "{size} {interface:?} {element:?}");
                if interface.is_some() {
                    let n = s.summary.contacts[0].normal_force;
                    let clamp = preload - (1.0 - c.phi()) * pull;
                    eprintln!("clamp: {n:e} vs {clamp:e}, error {:.2e} of the preload", (n - clamp).abs() / preload);
                    assert!((n - clamp).abs() < tol * preload, "{size} {element:?}");
                }
                assert!(s.summary.equilibrium_error < 1e-6);
            }
        }
    }
}

#[test]
fn just_below_the_separation_load_the_clamp_is_nearly_gone_and_the_bolt_carries_almost_all_of_the_pull() {
    // Separation is F = P / (1 - Phi): the clamp P - (1 - Phi) F reaches zero and the bolt carries the whole pull (T = F). At 0.95 of
    // it the closed forms still hold (the clamp is 5 % of P). Past it the contact would have to release mid-solve: the contact driver
    // does not converge through that release (`docs/epic-assembly-feature-advice.md`, follow-up 4), so a pull past it is refused.
    let preload = 1.0e4;
    let (mut p, c) = bolted_case(Some(0.0), 1.0, "M6", preload, 10.0, ElementChoice::Tri3);
    let sep = preload / (1.0 - c.phi());
    let pull = 0.95 * sep;
    p.loads = vec![Load::Traction { edge: top(1), tx: 0.0, ty: 0.0, tz: pull }];
    let s = solve(&p, None).unwrap();
    let f = &s.summary.fasteners[0];
    assert!(rel("tension", f.tension, preload + c.phi() * pull) < 1e-6, "{f:?}");
    let clamp = preload - (1.0 - c.phi()) * pull;
    assert!((s.summary.contacts[0].normal_force - clamp).abs() < 1e-5 * preload, "{:?} vs {clamp:e}", s.summary.contacts[0]);
    assert!(clamp > 0.04 * preload && clamp < 0.06 * preload);
}

#[test]
fn a_torque_gives_the_preload_the_joint_solver_finds_and_the_two_modes_agree() {
    let (p, _) = bolted(Some(0.0), 1.0, "M6", Preload::Force(1.0e4));
    let by_force = solve(&p, None).unwrap().summary.fasteners[0].clone();
    // the torque the joint solver needs for that force, given back as the input, returns the force
    let (mut q, _) = bolted(Some(0.0), 1.0, "M6", Preload::Torque(by_force.torque));
    q.fasteners[0].friction = p.fasteners[0].friction;
    let by_torque = solve(&q, None).unwrap().summary.fasteners[0].clone();
    assert!(rel("preload", by_torque.preload, 1.0e4) < 1e-6, "{by_torque:?}");
    assert!(rel("stiffness", by_torque.stiffness, by_force.stiffness) < 1e-12);
    // Shigley's closed form of the same equilibrium, T = P [ d2/2 (l + pi mu d2 sec a) / (pi d2 - mu l sec a) + mu r_m ], with the thread
    // half angle a = 30 deg, lead l = pitch and the mean bearing radius of a uniform pressure on the annulus (the solver's default).
    let mm = 1e-3 / p.units.length_m();
    let (d, pitch, mu) = (6.0 * mm, 1.0 * mm, 0.15);
    let d2 = d - 0.649_519 * pitch;
    let sec = 1.0 / 30f64.to_radians().cos();
    let (ri, ro) = (0.5 * 6.6 * mm, 0.5 * p.fasteners[0].nut_diameter.unwrap());
    let r_m = 2.0 / 3.0 * (ro.powi(3) - ri.powi(3)) / (ro * ro - ri * ri);
    let want = 1.0e4 * (0.5 * d2 * (pitch + std::f64::consts::PI * mu * d2 * sec) / (std::f64::consts::PI * d2 - mu * pitch * sec) + mu * r_m);
    assert!(rel("torque", by_force.torque, want) < 1e-3, "{} vs {want}", by_force.torque);
    // a rougher bolt needs more torque for the same preload
    let (mut rough, _) = bolted(Some(0.0), 1.0, "M6", Preload::Force(1.0e4));
    rough.fasteners[0].friction = 0.3;
    assert!(solve(&rough, None).unwrap().summary.fasteners[0].torque > by_force.torque);
}

#[test]
fn fasteners_are_validated_saved_and_scaled_with_the_units() {
    let (p, _) = bolted(Some(0.0), 1.0, "M6", Preload::Force(1.0e4));
    assert!(p.validate().is_ok(), "{:?}", p.validate());
    let refused = |edit: &dyn Fn(&mut Problem), what: &str| {
        let mut q = p.clone();
        edit(&mut q);
        let err = q.validate().unwrap_err();
        assert!(err.contains(what), "{err}");
    };
    refused(&|q| q.fasteners[0].size = "M7".into(), "not a size of the tables");
    refused(&|q| q.fasteners[0].preload = Preload::Force(0.0), "preload must be positive");
    refused(&|q| q.fasteners[0].head = -1.0, "head plane must lie beyond");
    refused(&|q| q.fasteners[0].head_diameter = Some(-1.0), "bearing diameter must be positive");
    refused(&|q| q.fasteners[0].axis = [0.0; 3], "nonzero axis");
    refused(&|q| q.material.plastic = true, "linear-elastic");
    // a plane that no outer face lies on is named, not ignored
    let mut off = p.clone();
    off.fasteners[0].head = 1.5;
    let err = solve(&off, None).unwrap_err();
    assert!(err.contains("fastener 1 head") && err.contains("no outer face"), "{err}");
    // JSON keeps the fastener, and a unit change scales the lengths, the force and the stiffness consistently: the same physical bolt
    let back: Problem = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back.fasteners, p.fasteners);
    let inch = solve(&p, None).unwrap().summary.fasteners[0].clone();
    let mut mm = p.clone();
    mm.convert_units(UnitSystem::MmMpa).unwrap();
    let si = solve(&mm, None).unwrap().summary.fasteners[0].clone();
    let newtons_per_lbf = UnitSystem::InchPsi.force_n() / UnitSystem::MmMpa.force_n();
    assert!(rel("preload", si.preload, inch.preload * newtons_per_lbf) < 1e-9, "{si:?} {inch:?}");
    assert!(rel("tension", si.tension, inch.tension * newtons_per_lbf) < 1e-6, "{si:?} {inch:?}");
    let per_length = newtons_per_lbf / (UnitSystem::InchPsi.length_m() / UnitSystem::MmMpa.length_m());
    assert!(rel("stiffness", si.stiffness, inch.stiffness * per_length) < 1e-9);
}
