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
    let AssemblyAdvice::Fastener { bodies, faces, size, mismatch, reason } = &items[1] else { panic!("{items:?}") };
    assert_eq!(bodies, &vec![0, 1]);
    assert_eq!(*size, Some("M6"));
    assert!(*mismatch);
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
