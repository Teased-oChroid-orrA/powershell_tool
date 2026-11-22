//! Assembly advice (epic Phase B, `docs/epic-assembly-feature-advice.md`): closed-form plates read back through the STEP path,
//! detected by `solid_kernel::features`, and the explained advice over them.
use fea_problem::assembly_advice::{recommend, recommend_step, AssemblyAdvice};
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::features::detect;
use solid_kernel::native_brep::NativeBrep;
use solid_kernel::step::{inventory, Limits};
use solid_kernel::V3;

fn read(b: &NativeBrep) -> AnalyticModel {
    let text = b.to_step_text();
    AnalyticModel::read(&inventory(&text, Limits::default()).unwrap(), Options::default()).unwrap()
}

fn plate(lo: V3, hi: V3, holes: &[([f64; 2], f64)]) -> NativeBrep {
    let mut b = NativeBrep::box_from_corners(lo, hi).unwrap();
    for (c, r) in holes {
        let top = b.planar_faces_through([lo[0], lo[1], hi[2]], [0., 0., 1.])[0];
        b = b.drill(top, [c[0], c[1], hi[2]], *r, None).unwrap();
    }
    b
}

fn advice(plates: &[NativeBrep]) -> Vec<AssemblyAdvice> {
    let models: Vec<AnalyticModel> = plates.iter().map(read).collect();
    recommend(&detect(&models).unwrap())
}

#[test]
fn two_stacked_plates_with_one_bolt_hole_give_a_tie_and_a_fastener_with_the_measured_numbers() {
    // M6 joint: 5.0 mm (tap drill) in the lower plate, 6.6 mm (normal clearance) in the upper
    let items = advice(&[plate([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.5)]), plate([0., 0., 10.], [40., 30., 20.], &[([20., 15.], 3.3)])]);
    assert_eq!(items.len(), 2, "{items:?}");
    let AssemblyAdvice::Tie { a, b, gap, overlap_area, reason } = &items[0] else { panic!("{items:?}") };
    assert_eq!((a.body, b.body), (0, 1));
    assert!(gap.abs() < 1e-9);
    // 40 x 30 contact less the bigger hole's disk (the polygon area reads inside the circle by a chord tolerance)
    let expected = 1200. - std::f64::consts::PI * 3.3 * 3.3;
    assert!((overlap_area - expected).abs() < 1.0, "{overlap_area} vs {expected}");
    assert!(reason.contains("no gap") && reason.contains("Body 1") && reason.contains("body 2"), "{reason}");
    let AssemblyAdvice::Fastener { bodies, faces, size, mismatch, diameters, span, reason } = &items[1] else { panic!("{items:?}") };
    assert_eq!(bodies, &vec![0, 1]);
    assert_eq!(*size, Some("M6"));
    assert!(*mismatch);
    assert!((diameters[0] - 5.0).abs() < 1e-9 && (diameters[1] - 6.6).abs() < 1e-9 && (span - 20.0).abs() < 1e-9, "{diameters:?} {span}");
    assert_eq!(items[1].detail(), "\u{d8}5.000, \u{d8}6.600 mm \u{b7} 20.00 mm");
    assert_eq!(items[0].detail(), format!("no gap \u{b7} {:.0} mm\u{b2}", overlap_area));
    assert_eq!(faces.iter().map(|f| f.body).collect::<std::collections::BTreeSet<_>>().len(), 2);
    assert!(reason.contains("\u{d8}5.000 mm (") && reason.contains("M6 tap drill") && reason.contains("\u{d8}6.600 mm (M6 normal clearance)"), "{reason}");
    assert!(reason.contains("Every bore fits M6") && reason.contains("20.00 mm of axis"), "{reason}");
    assert!(items[1].label().contains("M6") && items[1].faces().len() >= 2);
}

#[test]
fn plates_without_holes_give_only_the_tie_and_separated_plates_give_nothing() {
    let touching = advice(&[plate([0., 0., 0.], [40., 30., 10.], &[]), plate([0., 0., 10.], [40., 30., 20.], &[])]);
    assert!(matches!(touching.as_slice(), [AssemblyAdvice::Tie { .. }]), "{touching:?}");
    let apart = advice(&[plate([0., 0., 0.], [40., 30., 10.], &[]), plate([0., 0., 15.], [40., 30., 25.], &[])]);
    assert!(apart.is_empty(), "{apart:?}");
}

#[test]
fn a_step_assembly_reads_every_body_and_a_single_part_gives_no_advice() {
    let step = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../solid-kernel/tests/data/assembly-two-instances.step")).unwrap();
    let items = recommend_step(&step).unwrap();
    // whatever the fixture yields, every key must name one of its two bodies
    assert!(items.iter().flat_map(|i| i.faces()).all(|k| k.body < 2), "{items:?}");
    let single = plate([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.5)]).to_step_text();
    assert!(recommend_step(&single).unwrap().is_empty());
    assert!(recommend_step("not a step file").is_err());
}

fn unmated(items: &[AssemblyAdvice]) -> Vec<&AssemblyAdvice> {
    items.iter().filter(|i| matches!(i, AssemblyAdvice::Unmated { .. })).collect()
}

#[test]
fn a_bolt_sized_hole_with_no_mating_bore_is_advice_with_its_measured_numbers() {
    // lower plate: a 5.0 mm hole (M6 tap drill); upper plate: no hole, so nothing mates it
    let items = advice(&[plate([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.5)]), plate([0., 0., 10.], [40., 30., 20.], &[])]);
    let found = unmated(&items);
    assert_eq!(found.len(), 1, "{items:?}");
    let AssemblyAdvice::Unmated { body, faces, diameter, closure, size, reason } = found[0] else { unreachable!() };
    assert_eq!(*body, 0);
    assert!(!faces.is_empty() && faces.iter().all(|f| f.body == 0));
    assert!((diameter - 5.0).abs() < 1e-9, "{diameter}");
    // 5.0 mm is the M5 nominal, the M6 tap drill and the #10-24 close clearance: the label says so rather than pick one
    assert_eq!(size, "M5 or M6 or #10-24");
    // the lower plate's hole is closed by the upper plate's face in the model, but the hole itself is open at both ends
    assert_eq!(*closure, fea_problem::assembly_advice::Closure::Through);
    assert!(reason.contains("Body 1") && reason.contains("\u{d8}5.000 mm") && reason.contains("10.00 mm deep") && reason.contains("M5 nominal (press / locating)") && reason.contains("M6 tap drill") && reason.contains("#10-24 close clearance"), "{reason}");
    assert_eq!(found[0].detail(), "\u{d8}5.000 mm \u{b7} through");
    assert_eq!(found[0].label(), "Body 1 hole \u{d8}5.000 mm (M5 or M6 or #10-24) has no mate");
    assert_eq!(found[0].faces().len(), faces.len());
    // the order is the detector's: the tie first, the unmated hole after it
    assert!(matches!(items.as_slice(), [AssemblyAdvice::Tie { .. }, AssemblyAdvice::Unmated { .. }]), "{items:?}");
}

#[test]
fn holes_that_are_not_a_standard_size_or_that_have_a_mate_or_stand_alone_are_not_unmated() {
    // 7.3 mm matches no table entry (M6 loose is 7.0, M8 tap is 6.8): noise, not advice
    let odd = advice(&[plate([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 3.65)]), plate([0., 0., 10.], [40., 30., 20.], &[])]);
    assert!(unmated(&odd).is_empty(), "{odd:?}");
    // a mated pair is a fastener site, not two unmated holes
    let mated = advice(&[plate([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.5)]), plate([0., 0., 10.], [40., 30., 20.], &[([20., 15.], 3.3)])]);
    assert!(unmated(&mated).is_empty(), "{mated:?}");
    // one body has nothing to mate with: a standalone part's holes are not advice
    assert!(advice(&[plate([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.5)])]).is_empty());
    // a hole whose neighbour is displaced sideways is not mated by it: both are reported
    let apart = advice(&[plate([0., 0., 0.], [40., 30., 10.], &[([10., 15.], 2.5)]), plate([0., 0., 10.], [40., 30., 20.], &[([30., 15.], 3.3)])]);
    let found = unmated(&apart);
    assert_eq!(found.iter().map(|i| match i { AssemblyAdvice::Unmated { body, .. } => *body, _ => unreachable!() }).collect::<Vec<_>>(), vec![0, 1], "{apart:?}");
}

#[test]
fn a_counterbore_over_a_mated_bore_is_part_of_the_site_not_an_unmated_hole() {
    // upper plate: an 8.0 mm (M8 nominal) counterbore 3 mm deep over a 6.0 mm (M6 nominal) through hole; lower plate: 5.0 mm
    // (M6 tap drill). The counterbore matches a standard size by itself, so only the stepped-bore rule keeps it out of the advice.
    let mut upper = NativeBrep::box_from_corners([0., 0., 10.], [40., 30., 20.]).unwrap();
    let top = upper.planar_faces_through([0., 0., 20.], [0., 0., 1.])[0];
    upper = upper.drill(top, [20., 15., 20.], 4.0, Some(3.0)).unwrap();
    let floor = upper.planar_faces_through([20., 15., 17.], [0., 0., 1.])[0];
    upper = upper.drill(floor, [20., 15., 17.], 3.0, None).unwrap();
    let lower = plate([0., 0., 0.], [40., 30., 10.], &[([20., 15.], 2.5)]);
    let items = advice(&[lower.clone(), upper.clone()]);
    assert!(unmated(&items).is_empty(), "{items:?}");
    let AssemblyAdvice::Fastener { size, diameters, .. } = items.iter().find(|i| matches!(i, AssemblyAdvice::Fastener { .. })).expect("a site") else { unreachable!() };
    assert_eq!(*size, Some("M6"));
    assert_eq!(diameters.len(), 2, "the counterbore section is not a bore of the site: {diameters:?}");
    // the same counterbored plate with nothing under it: both sections are reported, the bore and the counterbore
    let alone = advice(&[upper, plate([0., 0., -10.], [40., 30., 0.], &[])]);
    let found = unmated(&alone);
    let mut ds: Vec<f64> = found.iter().map(|i| match i { AssemblyAdvice::Unmated { diameter, .. } => *diameter, _ => unreachable!() }).collect();
    ds.sort_by(f64::total_cmp);
    assert_eq!(ds.len(), 2, "{alone:?}");
    assert!((ds[0] - 6.0).abs() < 1e-9 && (ds[1] - 8.0).abs() < 1e-9, "{ds:?}");
}

#[test]
fn a_pin_bridging_bores_that_do_not_touch_is_named_in_the_fastener_reason() {
    let hole = [([20., 15.], 2.5)];
    let pin = NativeBrep::cylinder_z([20., 15., 5.], 2.5, 15.).unwrap();
    // the plates are 5 mm apart; without the pin the two bores are unrelated holes (unmated), not one fastener
    let apart = advice(&[plate([0., 0., 0.], [40., 30., 10.], &hole), plate([0., 0., 15.], [40., 30., 25.], &hole)]);
    assert!(!apart.iter().any(|i| matches!(i, AssemblyAdvice::Fastener { .. })), "{apart:?}");
    let items = advice(&[plate([0., 0., 0.], [40., 30., 10.], &hole), plate([0., 0., 15.], [40., 30., 25.], &hole), pin]);
    let AssemblyAdvice::Fastener { bodies, reason, .. } = items.iter().find(|i| matches!(i, AssemblyAdvice::Fastener { .. })).expect("a fastener item") else { unreachable!() };
    assert_eq!(bodies, &vec![0, 1]);
    assert!(reason.contains("do not touch") && reason.contains("body 3"), "{reason}");
}
